use crate::ObjectManagerProxy;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub address: String,
    pub alias: String,
    pub connected: bool,
    pub paired: bool,
    pub address_type: Option<String>,
    pub connectable: Option<bool>,
    pub discoverable: Option<bool>,
    pub discoverable_timeout: Option<u32>,
    pub discovering: Option<bool>,
    pub name: Option<String>,
    pub pairable: Option<bool>,
    pub pairable_timeout: Option<u32>,
    pub power_state: Option<String>,
    pub powered: Option<bool>,
    pub rssi: Option<i16>,
}

/// Lists all devices in the system.
pub async fn list_devices() -> Vec<DeviceInfo> {
    list_system_devices().await
}

pub async fn list_system_devices() -> Vec<DeviceInfo> {
    let conn = crate::get_system_connection().await.unwrap();
    let proxy = ObjectManagerProxy::new(&conn).await.unwrap();
    let objects = proxy.get_managed_objects().await.unwrap();

    let mut devices = Vec::new();
    let adapter_path = crate::get_adapter_path();
    
    for (path, interface) in objects {
        if path.starts_with(&format!("{}/dev", adapter_path)) {
            if let Some(device) = interface.get("org.bluez.Device1") {
                if let Some(address) = device.get("Address") {
                    let addr = address
                        .downcast_ref::<zbus::zvariant::Str>()
                        .ok()
                        .map(|s| s.as_str().to_owned())
                        .unwrap_or_default()
                        .to_string();

                    let alias = device
                        .get("Alias")
                        .and_then(|alias| alias.downcast_ref::<zbus::zvariant::Str>().ok())
                        .map(|s| s.as_str().to_owned())
                        .unwrap_or(addr.to_string())
                        .to_string();

                    let connected = device
                        .get("Connected")
                        .and_then(|v| v.downcast_ref::<bool>().ok())
                        .unwrap_or(false);

                    let paired = device
                        .get("Paired")
                        .and_then(|v| v.downcast_ref::<bool>().ok())
                        .unwrap_or(false);

                    let address_type = device
                        .get("AddressType")
                        .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                        .map(|s| s.as_str().to_owned());

                    let connectable = device
                        .get("Connectable")
                        .and_then(|v| v.downcast_ref::<bool>().ok());

                    let discoverable = device
                        .get("Discoverable")
                        .and_then(|v| v.downcast_ref::<bool>().ok());

                    let discoverable_timeout = device
                        .get("DiscoverableTimeout")
                        .and_then(|v| v.downcast_ref::<u32>().ok());

                    let discovering = device
                        .get("Discovering")
                        .and_then(|v| v.downcast_ref::<bool>().ok());

                    let name = device
                        .get("Name")
                        .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                        .map(|s| s.as_str().to_owned());

                    let pairable = device
                        .get("Pairable")
                        .and_then(|v| v.downcast_ref::<bool>().ok());

                    let pairable_timeout = device
                        .get("PairableTimeout")
                        .and_then(|v| v.downcast_ref::<u32>().ok());

                    let power_state = device
                        .get("PowerState")
                        .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                        .map(|s| s.as_str().to_owned());

                    let powered = device
                        .get("Powered")
                        .and_then(|v| v.downcast_ref::<bool>().ok());

                    let rssi = device
                        .get("RSSI")
                        .and_then(|v| v.downcast_ref::<i16>().ok());
                    
                    let device_info = DeviceInfo {
                        address: addr,
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

                    devices.push(device_info);
                }
            }
        }
    }
    
    devices
}
