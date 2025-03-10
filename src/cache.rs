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
pub fn list_devices() -> Vec<DeviceInfo> {
    let devices = DEVICES_CACHE.read().expect("Failed to acquire read lock");
    devices.values().cloned().collect()
}

/// Clears all devices from the cache.
pub fn clear_devices() {
    let mut devices = DEVICES_CACHE.write().expect("Failed to acquire write lock");
    devices.clear();
}
