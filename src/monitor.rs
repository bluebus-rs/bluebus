use futures::StreamExt;
use tokio::sync::mpsc;
use std::collections::HashMap;

/// Monitors Bluetooth device connections and disconnections.
/// This struct listens for events related to devices being added or removed from the system.
pub struct Monitor {
    /// Shared reference to the D-Bus connection.
    connection: std::sync::Arc<zbus::Connection>,
    /// Proxy for managing object events.
    manager: std::sync::Arc<crate::ObjectManagerProxy<'static>>,
    device_added_tx: mpsc::Sender<crate::cache::DeviceInfo>,
    device_removed_tx: mpsc::Sender<crate::cache::DeviceInfo>,
    device_changed_tx: mpsc::Sender<crate::cache::DeviceInfo>,
    /// Local device cache for monitoring
    devices: std::sync::Arc<tokio::sync::RwLock<HashMap<String, crate::cache::DeviceInfo>>>,
}

impl Monitor {
    /// Creates a new monitor instance.
    ///
    /// # Arguments
    /// * `connection` - Shared D-Bus connection.
    /// * `manager` - Proxy to the object manager.
    ///
    /// # Returns
    /// A new `Monitor` instance.
    pub async fn new(
        connection: std::sync::Arc<zbus::Connection>,
        manager: std::sync::Arc<crate::ObjectManagerProxy<'static>>,
        device_added_tx: mpsc::Sender<crate::cache::DeviceInfo>,
        device_removed_tx: mpsc::Sender<crate::cache::DeviceInfo>,
        device_changed_tx: mpsc::Sender<crate::cache::DeviceInfo>,
    ) -> Self {
        Self {
            connection,
            manager,
            device_added_tx,
            device_removed_tx,
            device_changed_tx,
            devices: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Monitors the removal of Bluetooth devices.
    /// This listens for D-Bus signals when devices are removed and logs the event.
    pub async fn monitor_device_removed(&self) {
        let mut interfaces_removed = self.manager.receive_interfaces_removed().await.unwrap();
        let devices = self.devices.clone();
        
        loop {
            if let Some(signal) = interfaces_removed.next().await {
                let args = signal.args().unwrap();
                let path = args.object_path().as_str();

                if let Some(removed_dev) = devices.write().await.remove(path) {
                    let _ = self.device_removed_tx.send(removed_dev).await;
                }
            }
        }
    }

    /// Monitors the addition of new Bluetooth devices.
    /// This listens for D-Bus signals when new devices are added and logs the event.
    pub async fn monitor_device_added(&self) {
        let mut interfaces_added = self.manager.receive_interfaces_added().await.unwrap();
        let devices = self.devices.clone();
        
        loop {
            if let Some(signal) = interfaces_added.next().await {
                let args = signal.args().unwrap();
                if let Some(interfaces) = args.interfaces().get("org.bluez.Device1") {
                    if let Some(address) = interfaces.get("Address") {
                        let addr = address
                            .downcast_ref::<zbus::zvariant::Str>()
                            .ok()
                            .map(|s| s.as_str().to_owned())
                            .unwrap_or_default()
                            .to_string();

                        let alias = interfaces
                            .get("Alias")
                            .and_then(|alias| alias.downcast_ref::<zbus::zvariant::Str>().ok())
                            .map(|s| s.as_str().to_owned())
                            .unwrap_or(addr.to_string())
                            .to_string();

                        let connected = interfaces
                            .get("Connected")
                            .and_then(|v| v.downcast_ref::<bool>().ok())
                            .unwrap_or(false);

                        let paired = interfaces
                            .get("Paired")
                            .and_then(|v| v.downcast_ref::<bool>().ok())
                            .unwrap_or(false);

                        let address_type = interfaces
                            .get("AddressType")
                            .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                            .map(|s| s.as_str().to_owned());

                        let connectable = interfaces
                            .get("Connectable")
                            .and_then(|v| v.downcast_ref::<bool>().ok());

                        let discoverable = interfaces
                            .get("Discoverable")
                            .and_then(|v| v.downcast_ref::<bool>().ok());

                        let discoverable_timeout = interfaces
                            .get("DiscoverableTimeout")
                            .and_then(|v| v.downcast_ref::<u32>().ok());

                        let discovering = interfaces
                            .get("Discovering")
                            .and_then(|v| v.downcast_ref::<bool>().ok());

                        let name = interfaces
                            .get("Name")
                            .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                            .map(|s| s.as_str().to_owned());

                        let pairable = interfaces
                            .get("Pairable")
                            .and_then(|v| v.downcast_ref::<bool>().ok());

                        let pairable_timeout = interfaces
                            .get("PairableTimeout")
                            .and_then(|v| v.downcast_ref::<u32>().ok());

                        let power_state = interfaces
                            .get("PowerState")
                            .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                            .map(|s| s.as_str().to_owned());

                        let powered = interfaces
                            .get("Powered")
                            .and_then(|v| v.downcast_ref::<bool>().ok());

                        let rssi = interfaces
                            .get("RSSI")
                            .and_then(|v| v.downcast_ref::<i16>().ok());

                        let path = args.object_path().to_string();

                        let new_device = crate::cache::DeviceInfo {
                            address: addr.to_string(),
                            alias,
                            connected,
                            paired,
                            address_type,
                            connectable,
                            discoverable,
                            discoverable_timeout,
                            discovering,
                            name,
                            pairable,
                            pairable_timeout,
                            power_state,
                            powered,
                            rssi,
                        };
                        
                        devices.write().await.insert(path.clone(), new_device.clone());
                        
                        let _ = self.monitor_device_properties(
                            self.connection.clone(),
                            std::sync::Arc::new(path),
                            devices.clone(),
                            self.device_changed_tx.clone(),
                        )
                        .await;
                        let _ = self.device_added_tx.send(new_device).await;
                    }
                }
            }
        }
    }

    /// Monitors property changes of Bluetooth devices.
    /// Updates internal cache when a device property (e.g., alias, connection status) changes.
    async fn monitor_device_properties(
        &self,
        conn: std::sync::Arc<zbus::Connection>,
        object_path: std::sync::Arc<String>,
        devices: std::sync::Arc<tokio::sync::RwLock<HashMap<String, crate::cache::DeviceInfo>>>,
        device_changed_tx: mpsc::Sender<crate::cache::DeviceInfo>,
    ) -> zbus::Result<()> {
        let path_clone = object_path.clone();

        tokio::spawn(async move {
            let props = zbus::fdo::PropertiesProxy::builder(&conn)
                .destination("org.bluez")
                .unwrap()
                .path(object_path.as_str())
                .unwrap()
                .build()
                .await
                .unwrap();

            let mut props_changed = props.receive_properties_changed().await.unwrap();

            while let Some(signal) = props_changed.next().await {
                let args = signal.args().unwrap();
                let interface_name = args.interface_name().to_string();
                let changed_props = args.changed_properties();

                if interface_name == "org.bluez.Device1" {
                    if let Some(mut device) = devices.read().await.get(object_path.as_str()).cloned() {
                        let mut changed = false;
                        
                        if let Some(new_value) = changed_props.get("Connected") {
                            if let Ok(val) = new_value.downcast_ref::<bool>() {
                                device.connected = val;
                                changed = true;
                            }
                        }

                        if let Some(new_value) = changed_props.get("Paired") {
                            if let Ok(val) = new_value.downcast_ref::<bool>() {
                                device.paired = val;
                                changed = true;
                            }
                        }

                        if let Some(new_value) = changed_props.get("Alias") {
                            if let Ok(val) = new_value.downcast_ref::<zbus::zvariant::Str>() {
                                device.alias = val.as_str().to_owned();
                                changed = true;
                            }
                        }

                        if changed {
                            devices.write().await.insert(path_clone.to_string(), device.clone());
                            let _ = device_changed_tx.send(device).await;
                        }
                    }
                }
            }
        });

        Ok(())
    }
}
