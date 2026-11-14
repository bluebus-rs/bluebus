//! Device add/remove/change notifications.
//!
//! ```no_run
//! # async fn f() -> zbus::Result<()> {
//! use std::sync::Arc;
//! use tokio::sync::mpsc;
//!
//! let conn = Arc::new(bluebus::get_system_connection().await?);
//! let manager = Arc::new(bluebus::ObjectManagerProxy::new(&conn).await?);
//! let (added_tx, mut added) = mpsc::channel(32);
//! let (removed_tx, mut removed) = mpsc::channel(32);
//! let (changed_tx, mut changed) = mpsc::channel(32);
//! let monitor = Arc::new(bluebus::Monitor::new(conn, manager, added_tx, removed_tx, changed_tx).await);
//! tokio::spawn({ let m = monitor.clone(); async move { m.run().await } });
//! loop {
//!     tokio::select! {
//!         Some(d) = added.recv() => println!("added {} {}", d.address, d.alias),
//!         Some(d) = removed.recv() => println!("removed {}", d.address),
//!         Some(d) = changed.recv() => println!("changed {} rssi={:?}", d.address, d.rssi),
//!     }
//! }
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio::sync::{mpsc, OnceCell, RwLock};
use tokio::task::JoinHandle;
use zbus::zvariant::OwnedValue;

use crate::cache::{DeviceInfo, DEVICE_INTERFACE};

type DeviceCache = Arc<RwLock<HashMap<String, DeviceInfo>>>;

/// Watches BlueZ for devices being added, removed or changing properties
/// and reports [`DeviceInfo`] snapshots on the given channels.
///
/// Devices that already exist when monitoring starts are loaded into the
/// cache silently, so their later changes and removal are reported too.
pub struct Monitor {
    connection: Arc<zbus::Connection>,
    manager: Arc<crate::ObjectManagerProxy<'static>>,
    device_added_tx: mpsc::Sender<DeviceInfo>,
    device_removed_tx: mpsc::Sender<DeviceInfo>,
    device_changed_tx: mpsc::Sender<DeviceInfo>,
    /// Object path → last known snapshot.
    devices: DeviceCache,
    started: OnceCell<()>,
    property_task: Mutex<Option<JoinHandle<()>>>,
}

impl Monitor {
    /// Creates a monitor. Nothing is watched until one of the `monitor_*`
    /// methods or [`run`](Self::run) is awaited.
    pub async fn new(
        connection: Arc<zbus::Connection>,
        manager: Arc<crate::ObjectManagerProxy<'static>>,
        device_added_tx: mpsc::Sender<DeviceInfo>,
        device_removed_tx: mpsc::Sender<DeviceInfo>,
        device_changed_tx: mpsc::Sender<DeviceInfo>,
    ) -> Self {
        Self {
            connection,
            manager,
            device_added_tx,
            device_removed_tx,
            device_changed_tx,
            devices: Arc::new(RwLock::new(HashMap::new())),
            started: OnceCell::new(),
            property_task: Mutex::new(None),
        }
    }

    /// Runs [`monitor_device_added`](Self::monitor_device_added) and
    /// [`monitor_device_removed`](Self::monitor_device_removed) concurrently
    /// until the bus connection closes.
    pub async fn run(&self) {
        tokio::join!(self.monitor_device_added(), self.monitor_device_removed());
    }

    /// Snapshot of the currently known devices, keyed by object path.
    pub async fn devices(&self) -> HashMap<String, DeviceInfo> {
        self.devices.read().await.clone()
    }

    /// Reports devices removed by BlueZ on the "removed" channel.
    /// Returns when the signal stream ends (connection closed).
    pub async fn monitor_device_removed(&self) {
        let mut removed = match self.manager.receive_interfaces_removed().await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::warn!("cannot subscribe to InterfacesRemoved: {e}");
                return;
            }
        };
        self.ensure_started().await;

        while let Some(signal) = removed.next().await {
            let Ok(args) = signal.args() else { continue };
            // Ignore removal of other interfaces (Battery1, MediaControl1, …)
            // from a device that is still present.
            if !args.interfaces().iter().any(|i| i == DEVICE_INTERFACE) {
                continue;
            }
            let device = self
                .devices
                .write()
                .await
                .remove(args.object_path().as_str());
            if let Some(device) = device {
                let _ = self.device_removed_tx.send(device).await;
            }
        }
    }

    /// Reports newly discovered devices on the "added" channel and starts
    /// reporting property changes on the "changed" channel.
    /// Returns when the signal stream ends (connection closed).
    pub async fn monitor_device_added(&self) {
        let mut added = match self.manager.receive_interfaces_added().await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::warn!("cannot subscribe to InterfacesAdded: {e}");
                return;
            }
        };
        self.ensure_started().await;

        while let Some(signal) = added.next().await {
            let Ok(args) = signal.args() else { continue };
            let Some(device) = args
                .interfaces()
                .get(DEVICE_INTERFACE)
                .and_then(DeviceInfo::from_properties)
            else {
                continue;
            };
            self.devices
                .write()
                .await
                .insert(args.object_path().to_string(), device.clone());
            let _ = self.device_added_tx.send(device).await;
        }
    }

    /// Loads existing devices and starts the property watcher, once.
    async fn ensure_started(&self) {
        self.started
            .get_or_init(|| async {
                // Subscribe before loading so no change is lost in between.
                match property_changes(&self.connection).await {
                    Ok(stream) => {
                        let task = tokio::spawn(watch_properties(
                            stream,
                            self.devices.clone(),
                            self.device_changed_tx.clone(),
                        ));
                        *self.property_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
                    }
                    Err(e) => tracing::warn!("cannot subscribe to PropertiesChanged: {e}"),
                }
                match self.manager.get_managed_objects().await {
                    Ok(objects) => {
                        let mut devices = self.devices.write().await;
                        for (path, ifaces) in objects {
                            if let Some(info) = ifaces
                                .get(DEVICE_INTERFACE)
                                .and_then(DeviceInfo::from_properties)
                            {
                                devices.entry(path.to_string()).or_insert(info);
                            }
                        }
                    }
                    Err(e) => tracing::warn!("cannot load existing devices: {e}"),
                }
            })
            .await;
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(task) = self
            .property_task
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

/// One subscription for `PropertiesChanged` of every BlueZ device.
async fn property_changes(conn: &zbus::Connection) -> zbus::Result<zbus::MessageStream> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(crate::BLUEZ_SERVICE)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace(crate::BLUEZ_ROOT_PATH)?
        .arg(0, DEVICE_INTERFACE)?
        .build();
    zbus::MessageStream::for_match_rule(rule, conn, Some(256)).await
}

async fn watch_properties(
    mut stream: zbus::MessageStream,
    devices: DeviceCache,
    changed_tx: mpsc::Sender<DeviceInfo>,
) {
    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let Some(path) = header.path() else { continue };
        let Ok((iface, changed, invalidated)) =
            msg.body()
                .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
        else {
            continue;
        };
        if iface != DEVICE_INTERFACE {
            continue;
        }
        // Update under the lock, send after releasing it.
        let snapshot = {
            let mut devices = devices.write().await;
            let Some(device) = devices.get_mut(path.as_str()) else {
                continue;
            };
            let mut any = false;
            for (name, value) in &changed {
                any |= device.apply_property(name, value);
            }
            for name in &invalidated {
                any |= device.invalidate_property(name);
            }
            any.then(|| device.clone())
        };
        if let Some(device) = snapshot {
            let _ = changed_tx.send(device).await;
        }
    }
}
