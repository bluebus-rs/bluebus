use crate::ObjectManagerProxy;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub address: String,
    pub alias: String,
    pub connected: bool,
    pub paired: bool,
}

lazy_static::lazy_static! {
    pub static ref DEVICES_CACHE:  std::sync::Arc<std::sync::RwLock<std::collections::HashMap::<String, DeviceInfo>>>=
    std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::<String, DeviceInfo>::new()));
}

/// Adds or updates a device in the cache.
pub fn add_or_update_device(path: String, device: &DeviceInfo) {
    let mut devices = DEVICES_CACHE.write().expect("Failed to acquire write lock");
    devices.insert(path, device.clone());
}

/// Retrieves a device from the cache by its address.
pub fn get_device(path: &str) -> Option<DeviceInfo> {
    let devices = DEVICES_CACHE.read().expect("Failed to acquire read lock");
    devices.get(path).cloned()
}

/// Removes a device from the cache by its address.
pub fn remove_device(path: &str) -> Option<DeviceInfo> {
    let mut devices = DEVICES_CACHE.write().expect("Failed to acquire write lock");
    devices.remove(path)
}

/// Lists all devices in the cache.
pub async fn list_devices() -> Vec<DeviceInfo> {
    list_system_devices().await;
    let devices = DEVICES_CACHE.read().expect("Failed to acquire read lock");
    devices.values().cloned().collect()
}

/// Clears all devices from the cache.
pub fn clear_devices() {
    let mut devices = DEVICES_CACHE.write().expect("Failed to acquire write lock");
    devices.clear();
}

pub async fn list_system_devices() {
    let conn = crate::get_system_connection().await.unwrap();
    let proxy = ObjectManagerProxy::new(&conn).await.unwrap();
    let objects = proxy.get_managed_objects().await.unwrap();

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
                    let path = path.to_string();
                    let device_info = DeviceInfo {
                        address: addr,
                        alias,
                        connected,
                        paired,
                    };

                    crate::add_or_update_device(path, &device_info);
                }
            }
        }
    }
}
