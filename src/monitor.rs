use futures::StreamExt;

/// Monitors Bluetooth device connections and disconnections.
/// This struct listens for events related to devices being added or removed from the system.
pub struct Monitor {
    /// Shared reference to the D-Bus connection.
    connection: std::sync::Arc<zbus::Connection>,
    /// Proxy for managing object events.
    manager: std::sync::Arc<crate::ObjectManagerProxy<'static>>,
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
    ) -> Self {
        Self {
            connection,
            manager,
        }
    }

    /// Monitors the removal of Bluetooth devices.
    /// This listens for D-Bus signals when devices are removed and logs the event.
    pub async fn monitor_device_removed(&self) {
        let mut interfaces_removed = self.manager.receive_interfaces_removed().await.unwrap();
        loop {
            if let Some(signal) = interfaces_removed.next().await {
                let args = signal.args().unwrap();

                if let Some(removed_dev) = crate::remove_device(args.object_path().as_str()) {
                    println!(
                        "❌[DEL][DEV]\tAddress({})\tAlias({})\t Interface Path: ({})",
                        removed_dev.address,
                        removed_dev.alias,
                        args.object_path()
                    );
                }
            }
        }
    }

    /// Monitors the addition of new Bluetooth devices.
    /// This listens for D-Bus signals when new devices are added and logs the event.
    pub async fn monitor_device_added(&self) {
        let mut interfaces_added = self.manager.receive_interfaces_added().await.unwrap();
        loop {
            if let Some(signal) = interfaces_added.next().await {
                let args = signal.args().unwrap();
                if let Some(interfaces) = args.interfaces().get("org.bluez.Device1") {
                    let address = interfaces.get("Address").unwrap().to_string();
                    let alias = interfaces.get("Alias").unwrap().to_string();
                    let connected = interfaces
                        .get("Connected")
                        .unwrap()
                        .downcast_ref::<bool>()
                        .unwrap();
                    let paired = interfaces
                        .get("Paired")
                        .unwrap()
                        .downcast_ref::<bool>()
                        .unwrap();
                    let path = args.object_path().to_string();

                    println!(
                        "✅[NEW][DEV]\tAddress({})\tAlias({})\t Interface Path: ({})",
                        address,
                        alias,
                        args.object_path()
                    );

                    let new_device = crate::cache::DeviceInfo {
                        address: address.to_string(),
                        alias,
                        connected,
                        paired,
                    };
                    crate::add_or_update_device(path, &new_device);
                    self.monitor_device_properties(
                        self.connection.clone(),
                        std::sync::Arc::new(args.object_path.to_string()),
                    )
                    .await
                    .unwrap();
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
    ) -> zbus::Result<()> {
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
                    if let Some(mut device) = crate::get_device(object_path.as_str()) {
                        if let Some(new_value) = changed_props.get("Connected") {
                            let val = new_value.downcast_ref::<bool>().unwrap();
                            device.connected = val;
                        }
                        if let Some(new_value) = changed_props.get("Paired") {
                            let val = new_value.downcast_ref::<bool>().unwrap();
                            device.paired = val;
                        }
                        if let Some(new_value) = changed_props.get("Alias") {
                            let val = new_value.downcast_ref::<String>().unwrap();
                            device.alias = val;
                        }
                        crate::add_or_update_device(object_path.to_string(), &device);
                        println!(
                            "🔄[UPT][DEV]\tAddress({})\tAlias({})\t Interface Path: ({})",
                            device.address, device.alias, object_path
                        );
                    }
                }
            }
        });
        Ok(())
    }
}
