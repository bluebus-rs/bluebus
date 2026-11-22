use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{Stream, StreamExt};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedValue, Value};

use super::error::is_unknown_object;
use super::info::service_tree;
use super::uuid::normalize_uuid;
use super::{Adapter, DeviceInfo, Error, Result, ServiceInfo};
use crate::{
    BatteryProxy, DeviceProxy, GattCharacteristic1Proxy, GattDatabase, GattDescriptor1Proxy,
    GattOptions, DEVICE_INTERFACE,
};

const BATTERY_LEVEL_UUID: &str = "00002a19-0000-1000-8000-00805f9b34fb";
const CHARACTERISTIC_INTERFACE: &str = "org.bluez.GattCharacteristic1";

/// How [`Device::write`] writes a characteristic.
///
/// ```no_run
/// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
/// use bluebus::ble::WriteType;
/// device.write("2a39", vec![1], WriteType::WithResponse).await?;
/// device.write("6e400002-b5a3-f393-e0a9-e50e24dcca9e", b"hi".to_vec(), WriteType::WithoutResponse).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteType {
    /// Write request: the remote acknowledges it (BlueZ type `"request"`).
    #[default]
    WithResponse,
    /// Write command: no acknowledgement (BlueZ type `"command"`); the
    /// characteristic must have the `write-without-response` flag.
    WithoutResponse,
}

/// A remote Bluetooth device: connection, pairing and GATT client.
///
/// Get one from [`Adapter::device`] or [`Adapter::find_device`]. Cheap to
/// clone; clones share the cached GATT database.
///
/// Characteristics and descriptors are addressed by UUID, in short
/// (`"2a19"`, `"0x2A19"`) or full form. If several characteristics share a
/// UUID, the first one (in handle order) is used.
///
/// ```no_run
/// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
/// use std::time::Duration;
/// let device = adapter.device("AA:BB:CC:DD:EE:FF")?;
/// device.connect(Duration::from_secs(20)).await?;
/// let level = device.read("2a19").await?;
/// println!("battery {}%", level[0]);
/// device.disconnect().await?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Device {
    inner: Arc<DeviceInner>,
}

struct DeviceInner {
    adapter: Adapter,
    path: String,
    address: String,
    /// GATT database of the current connection.
    gatt: tokio::sync::Mutex<Option<GattDatabase>>,
    /// Characteristic path → number of live notification streams.
    notify: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("address", &self.inner.address)
            .field("adapter", &self.inner.adapter.name())
            .finish()
    }
}

impl Device {
    pub(crate) fn new(adapter: Adapter, address: String) -> Self {
        Device {
            inner: Arc::new(DeviceInner {
                path: crate::device_path(adapter.path(), &address),
                adapter,
                address,
                gatt: tokio::sync::Mutex::new(None),
                notify: Arc::default(),
            }),
        }
    }

    /// MAC address (upper case).
    ///
    /// ```no_run
    /// # fn f(device: bluebus::ble::Device) {
    /// println!("{}", device.address());
    /// # }
    /// ```
    pub fn address(&self) -> &str {
        &self.inner.address
    }

    /// D-Bus object path of the device.
    ///
    /// ```no_run
    /// # fn f(device: bluebus::ble::Device) {
    /// println!("{}", device.path()); // /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF
    /// # }
    /// ```
    pub fn path(&self) -> &str {
        &self.inner.path
    }

    /// The adapter the device belongs to.
    ///
    /// ```no_run
    /// # fn f(device: bluebus::ble::Device) {
    /// println!("{}", device.adapter().name());
    /// # }
    /// ```
    pub fn adapter(&self) -> &Adapter {
        &self.inner.adapter
    }

    fn conn(&self) -> &zbus::Connection {
        self.inner.adapter.conn()
    }

    fn not_found(&self) -> Error {
        Error::NotFound(format!(
            "device {} on {}",
            self.inner.address,
            self.inner.adapter.name()
        ))
    }

    /// Maps "no such object" to [`Error::NotFound`].
    fn device_error(&self, e: zbus::Error) -> Error {
        if is_unknown_object(&e) {
            self.not_found()
        } else {
            Error::Bus(e)
        }
    }

    async fn proxy(&self) -> Result<DeviceProxy<'static>> {
        Ok(DeviceProxy::builder(self.conn())
            .path(self.inner.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?)
    }

    /// A fresh snapshot of the device (name, RSSI, advertised data, link
    /// state).
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// let info = device.info().await?;
    /// println!("{} {:?} connected={}", info.address, info.name, info.connected);
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::NotFound`] if BlueZ does not know the device.
    pub async fn info(&self) -> Result<DeviceInfo> {
        let props = super::get_all(self.conn(), &self.inner.path, DEVICE_INTERFACE)
            .await
            .map_err(|e| match e {
                Error::Bus(e) => self.device_error(e),
                e => e,
            })?;
        DeviceInfo::from_properties(&props).ok_or_else(|| self.not_found())
    }

    /// Connects and waits until BlueZ has resolved the GATT services, all
    /// within `timeout`. Returns at once if already connected and resolved.
    /// On timeout the pending connection is cancelled.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// use std::time::Duration;
    /// device.connect(Duration::from_secs(20)).await?;
    /// assert!(device.is_connected().await?);
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::Timeout`]; [`Error::NotFound`] for an unknown device;
    /// [`Error::Bus`] with BlueZ' reason (`org.bluez.Error.Failed`, …).
    pub async fn connect(&self, timeout: Duration) -> Result<()> {
        let proxy = self.proxy().await?;
        let attempt = async {
            let mut changes =
                super::properties_changed(self.conn(), &self.inner.path, DEVICE_INTERFACE).await?;
            let info = self.info().await?;
            if info.connected && info.services_resolved {
                return Ok(());
            }
            *self.inner.gatt.lock().await = None;
            if !info.connected {
                if let Err(e) = proxy.connect().await {
                    let already = super::error::bus_error_name(&e).as_deref()
                        == Some("org.bluez.Error.AlreadyConnected");
                    if !already {
                        return Err(self.device_error(e));
                    }
                }
            }
            loop {
                let info = self.info().await?;
                if info.services_resolved {
                    return Ok(());
                }
                if !info.connected {
                    return Err(Error::Bus(zbus::Error::Failure(format!(
                        "{} disconnected while resolving services",
                        self.inner.address
                    ))));
                }
                if changes.next().await.is_none() {
                    return Err(Error::Bus(zbus::Error::Failure(
                        "D-Bus connection closed".into(),
                    )));
                }
            }
        };
        match tokio::time::timeout(timeout, attempt).await {
            Ok(result) => result,
            Err(_) => {
                let _ = tokio::time::timeout(Duration::from_secs(5), proxy.disconnect()).await;
                Err(Error::Timeout(format!(
                    "connecting to {} (with service discovery) took longer than {timeout:?}",
                    self.inner.address
                )))
            }
        }
    }

    /// Disconnects. Does nothing if the device is not connected.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// device.disconnect().await?;
    /// # Ok(()) }
    /// ```
    pub async fn disconnect(&self) -> Result<()> {
        *self.inner.gatt.lock().await = None;
        if !self.info().await?.connected {
            return Ok(());
        }
        self.proxy()
            .await?
            .disconnect()
            .await
            .map_err(|e| self.device_error(e))
    }

    /// Whether the device is connected.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// println!("connected: {}", device.is_connected().await?);
    /// # Ok(()) }
    /// ```
    pub async fn is_connected(&self) -> Result<bool> {
        Ok(self.info().await?.connected)
    }

    /// Pairs with the device. Does nothing if already paired.
    ///
    /// Unless the pairing is "Just Works", an agent must answer BlueZ'
    /// questions: see [`Session::set_agent`](super::Session::set_agent).
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session, device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// use bluebus::ble::{AgentCapability, AutoAcceptAgent};
    /// session.set_agent(AgentCapability::NoInputNoOutput, AutoAcceptAgent).await?;
    /// device.pair().await?;
    /// device.trust(true).await?;
    /// # Ok(()) }
    /// ```
    pub async fn pair(&self) -> Result<()> {
        if self.info().await?.paired {
            return Ok(());
        }
        self.proxy()
            .await?
            .pair()
            .await
            .map_err(|e| self.device_error(e))
    }

    /// Marks the device as trusted (connections are accepted without asking
    /// the agent) or untrusted.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// device.trust(true).await?;
    /// # Ok(()) }
    /// ```
    pub async fn trust(&self, trusted: bool) -> Result<()> {
        self.proxy()
            .await?
            .set_trusted(trusted)
            .await
            .map_err(|e| self.device_error(e))
    }

    /// Signal strength of the last advertisement (dBm), while a discovery
    /// sees the device.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// if let Some(rssi) = device.rssi().await? {
    ///     println!("{rssi} dBm");
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn rssi(&self) -> Result<Option<i16>> {
        Ok(self.info().await?.rssi)
    }

    /// Battery level in percent: BlueZ' `Battery1` interface if present,
    /// else the Battery Level characteristic (`0x2A19`).
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// println!("battery {}%", device.battery_level().await?);
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::NotFound`] if the device reports no battery level.
    pub async fn battery_level(&self) -> Result<u8> {
        let battery = BatteryProxy::builder(self.conn())
            .path(self.inner.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        if let Ok(level) = battery.percentage().await {
            return Ok(level);
        }
        self.read(BATTERY_LEVEL_UUID)
            .await?
            .first()
            .copied()
            .ok_or_else(|| Error::NotFound("battery level (empty value)".into()))
    }

    async fn database(&self, refresh: bool) -> Result<GattDatabase> {
        let mut cache = self.inner.gatt.lock().await;
        if refresh || cache.is_none() {
            *cache = Some(crate::discover_gatt_database(self.conn(), &self.inner.path).await?);
        }
        Ok(cache.clone().unwrap_or_default())
    }

    /// The GATT services of the connected device with their characteristics
    /// and descriptors. Cached until the next connect/disconnect.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// for s in device.services().await? {
    ///     println!("{} ({} characteristics)", s.uuid, s.characteristics.len());
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn services(&self) -> Result<Vec<ServiceInfo>> {
        let mut db = self.database(false).await?;
        if db.services.is_empty() {
            db = self.database(true).await?;
        }
        Ok(service_tree(&db))
    }

    async fn characteristic_path(&self, uuid: &str, refresh: bool) -> Result<String> {
        let mut db = self.database(refresh).await?;
        if !refresh && db.characteristic_by_uuid(uuid).is_none() {
            db = self.database(true).await?;
        }
        db.characteristic_by_uuid(uuid)
            .map(|c| c.path.clone())
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "characteristic {uuid} on {} (is it connected?)",
                    self.inner.address
                ))
            })
    }

    async fn descriptor_path(&self, chr: &str, desc: &str, refresh: bool) -> Result<String> {
        let chr_path = self.characteristic_path(chr, refresh).await?;
        let find = |db: &GattDatabase| {
            db.descriptors
                .iter()
                .find(|d| d.parent == chr_path && d.uuid.eq_ignore_ascii_case(desc))
                .map(|d| d.path.clone())
        };
        let mut found = find(&self.database(false).await?);
        if found.is_none() && !refresh {
            found = find(&self.database(true).await?);
        }
        found.ok_or_else(|| Error::NotFound(format!("descriptor {desc} of characteristic {chr}")))
    }

    /// Runs `op` on the object resolved by `resolve`; if the object vanished
    /// (stale cache after a reconnect), refreshes the database and retries once.
    async fn with_path<T, R, RF, F, Fut>(&self, resolve: R, op: F) -> Result<T>
    where
        R: Fn(bool) -> RF,
        RF: Future<Output = Result<String>>,
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let path = resolve(false).await?;
        match op(path).await {
            Err(Error::Bus(e)) if is_unknown_object(&e) => op(resolve(true).await?).await,
            other => other,
        }
    }

    async fn with_characteristic<T, F, Fut>(&self, uuid: &str, op: F) -> Result<T>
    where
        F: Fn(GattCharacteristic1Proxy<'static>) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let uuid = normalize_uuid(uuid)?;
        let conn = self.conn().clone();
        self.with_path(
            |refresh| self.characteristic_path(&uuid, refresh),
            |path| {
                let conn = conn.clone();
                let op = &op;
                async move { op(characteristic_proxy(&conn, path).await?).await }
            },
        )
        .await
    }

    async fn with_descriptor<T, F, Fut>(&self, chr: &str, desc: &str, op: F) -> Result<T>
    where
        F: Fn(GattDescriptor1Proxy<'static>) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let chr = normalize_uuid(chr)?;
        let desc = normalize_uuid(desc)?;
        let conn = self.conn().clone();
        self.with_path(
            |refresh| self.descriptor_path(&chr, &desc, refresh),
            |path| {
                let conn = conn.clone();
                let op = &op;
                async move {
                    let proxy = GattDescriptor1Proxy::builder(&conn)
                        .destination(crate::BLUEZ_SERVICE)?
                        .path(path)?
                        .cache_properties(CacheProperties::No)
                        .build()
                        .await?;
                    op(proxy).await
                }
            },
        )
        .await
    }

    /// Reads a characteristic.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// let name = device.read("2a00").await?; // Device Name
    /// println!("{}", String::from_utf8_lossy(&name));
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::NotFound`] if the device has no such characteristic;
    /// [`Error::InvalidArgument`] for a malformed UUID.
    pub async fn read(&self, uuid: &str) -> Result<Vec<u8>> {
        self.with_characteristic(uuid, |chr| async move {
            Ok(chr.read_value(GattOptions::new()).await?)
        })
        .await
    }

    /// Writes a characteristic.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// use bluebus::ble::WriteType;
    /// device.write("2a06", vec![0x02], WriteType::WithoutResponse).await?; // Alert Level
    /// # Ok(()) }
    /// ```
    pub async fn write(
        &self,
        uuid: &str,
        data: impl Into<Vec<u8>>,
        write_type: WriteType,
    ) -> Result<()> {
        let data = data.into();
        let kind = match write_type {
            WriteType::WithResponse => "request",
            WriteType::WithoutResponse => "command",
        };
        self.with_characteristic(uuid, |chr| {
            let data = data.clone();
            async move {
                let options = GattOptions::from([(
                    "type".to_string(),
                    OwnedValue::try_from(Value::from(kind))?,
                )]);
                Ok(chr.write_value(data, options).await?)
            }
        })
        .await
    }

    /// Reads a descriptor of a characteristic.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// let description = device.read_descriptor("2a19", "2901").await?;
    /// # Ok(()) }
    /// ```
    pub async fn read_descriptor(&self, characteristic: &str, descriptor: &str) -> Result<Vec<u8>> {
        self.with_descriptor(characteristic, descriptor, |d| async move {
            Ok(d.read_value(GattOptions::new()).await?)
        })
        .await
    }

    /// Writes a descriptor of a characteristic.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// device.write_descriptor("2a19", "2901", b"level".to_vec()).await?;
    /// # Ok(()) }
    /// ```
    pub async fn write_descriptor(
        &self,
        characteristic: &str,
        descriptor: &str,
        data: impl Into<Vec<u8>>,
    ) -> Result<()> {
        let data = data.into();
        self.with_descriptor(characteristic, descriptor, |d| {
            let data = data.clone();
            async move { Ok(d.write_value(data, GattOptions::new()).await?) }
        })
        .await
    }

    /// Enables notifications (or indications) on a characteristic and
    /// returns every received value, in order. Dropping the stream (or
    /// [`NotificationStream::stop`]) disables them again once no other
    /// stream of this device uses the characteristic.
    ///
    /// Note that BlueZ also reports the result of a [`read`](Self::read) of
    /// the same characteristic as a value change.
    ///
    /// ```no_run
    /// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
    /// use futures::StreamExt;
    /// let mut heart_rate = device.notifications("2a37").await?;
    /// while let Some(value) = heart_rate.next().await {
    ///     println!("{value:02x?}");
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn notifications(&self, uuid: &str) -> Result<NotificationStream> {
        let notify = self.inner.notify.clone();
        self.with_characteristic(uuid, |chr| {
            let notify = notify.clone();
            async move {
                let path = chr.inner().path().to_string();
                let rule = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .sender(crate::BLUEZ_SERVICE)?
                    .interface("org.freedesktop.DBus.Properties")?
                    .member("PropertiesChanged")?
                    .path(path.clone())?
                    .arg(0, CHARACTERISTIC_INTERFACE)?
                    .build();
                // Subscribe before enabling so that no early value is lost.
                let signals =
                    zbus::MessageStream::for_match_rule(rule, chr.inner().connection(), Some(1024))
                        .await?;
                {
                    let mut streams = notify.lock().await;
                    let count = streams.entry(path.clone()).or_insert(0);
                    if *count == 0 {
                        chr.start_notify().await?;
                    }
                    *count += 1;
                }
                let values = signals.filter_map(|msg| async move {
                    let msg = msg.ok()?;
                    let (_, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
                        msg.body().deserialize().ok()?;
                    Vec::<u8>::try_from(changed.get("Value")?.try_clone().ok()?).ok()
                });
                Ok(NotificationStream {
                    values: Box::pin(values),
                    release: Some(Release {
                        chr,
                        path,
                        notify,
                        runtime: tokio::runtime::Handle::current(),
                    }),
                })
            }
        })
        .await
    }
}

async fn characteristic_proxy(
    conn: &zbus::Connection,
    path: String,
) -> Result<GattCharacteristic1Proxy<'static>> {
    Ok(GattCharacteristic1Proxy::builder(conn)
        .destination(crate::BLUEZ_SERVICE)?
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?)
}

/// Values notified by a characteristic, as a [`Stream`] of byte vectors.
/// Created by [`Device::notifications`].
///
/// ```no_run
/// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
/// use futures::StreamExt;
/// let mut values = device.notifications("2a19").await?;
/// let first = values.next().await;
/// values.stop().await?;
/// # Ok(()) }
/// ```
pub struct NotificationStream {
    values: Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>,
    release: Option<Release>,
}

impl std::fmt::Debug for NotificationStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationStream")
            .field("characteristic", &self.release.as_ref().map(|r| &r.path))
            .finish()
    }
}

struct Release {
    chr: GattCharacteristic1Proxy<'static>,
    path: String,
    notify: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
    runtime: tokio::runtime::Handle,
}

impl Release {
    async fn run(self) -> Result<()> {
        let mut streams = self.notify.lock().await;
        let count = streams.entry(self.path.clone()).or_insert(1);
        *count = count.saturating_sub(1);
        if *count == 0 {
            streams.remove(&self.path);
            if let Err(e) = self.chr.stop_notify().await {
                let e = zbus::Error::from(e);
                // A vanished object means the device disconnected: nothing to stop.
                if !is_unknown_object(&e) {
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }
}

impl NotificationStream {
    /// Disables notifications (unless another stream of the same device
    /// still uses the characteristic) and waits for BlueZ to confirm.
    ///
    /// ```no_run
    /// # async fn f(stream: bluebus::ble::NotificationStream) -> bluebus::ble::Result<()> {
    /// stream.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn stop(mut self) -> Result<()> {
        match self.release.take() {
            Some(r) => r.run().await,
            None => Ok(()),
        }
    }
}

impl Stream for NotificationStream {
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Vec<u8>>> {
        self.values.as_mut().poll_next(cx)
    }
}

impl Drop for NotificationStream {
    fn drop(&mut self) {
        if let Some(r) = self.release.take() {
            let runtime = r.runtime.clone();
            runtime.spawn(async move {
                let _ = r.run().await;
            });
        }
    }
}
