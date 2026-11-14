use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// Every object exported by BlueZ: path → interface name → property name → value.
pub type ObjectsMap = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// Proxy for BlueZ's `org.freedesktop.DBus.ObjectManager` (at `/`).
///
/// ```no_run
/// # async fn f() -> zbus::Result<()> {
/// let conn = bluebus::get_system_connection().await?;
/// let objects = bluebus::ObjectManagerProxy::new(&conn).await?.get_managed_objects().await?;
/// bluebus::print_available_devices(&objects);
/// # Ok(()) }
/// ```
#[zbus::proxy(
    default_service = "org.bluez",
    interface = "org.freedesktop.DBus.ObjectManager",
    default_path = "/"
)]
pub trait ObjectManager {
    /// Retrieves all objects BlueZ exports, with their interfaces and properties.
    fn get_managed_objects(&self) -> zbus::Result<ObjectsMap>;

    /// Emitted when an object (e.g. a newly discovered device) gains interfaces.
    #[zbus(signal)]
    fn interfaces_added(
        &self,
        object_path: OwnedObjectPath,
        interfaces: HashMap<String, HashMap<String, OwnedValue>>,
    ) -> zbus::Result<()>;

    /// Emitted when an object loses interfaces (e.g. a device is removed).
    #[zbus(signal)]
    fn interfaces_removed(
        &self,
        object_path: OwnedObjectPath,
        interfaces: Vec<String>,
    ) -> zbus::Result<()>;
}

/// Prints the object path of every managed object, sorted.
pub fn print_key_pairs(objects: &ObjectsMap) {
    let mut paths: Vec<&OwnedObjectPath> = objects.keys().collect();
    paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for path in paths {
        println!("Object Path: {}", path);
    }
}

/// Formats one line per Bluetooth device (`org.bluez.Device1` objects only),
/// sorted by object path: `"<path> <address> <alias>"`.
pub fn format_available_devices(objects: &ObjectsMap) -> Vec<String> {
    let mut devices: Vec<(&OwnedObjectPath, crate::DeviceInfo)> = objects
        .iter()
        .filter_map(|(path, ifaces)| {
            let info = crate::DeviceInfo::from_properties(ifaces.get(crate::DEVICE_INTERFACE)?)?;
            Some((path, info))
        })
        .collect();
    devices.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    devices
        .into_iter()
        .map(|(path, d)| format!("{} {} {}", path, d.address, d.alias))
        .collect()
}

/// Prints the Bluetooth devices among the managed objects.
///
/// Before 0.1.11 this printed every object path, including adapters,
/// services and characteristics; now only devices are printed.
pub fn print_available_devices(objects: &ObjectsMap) {
    for line in format_available_devices(objects) {
        println!("Device: {line}");
    }
}

/// Misspelled alias of [`print_available_devices`], kept for compatibility.
#[deprecated(since = "0.1.11", note = "use `print_available_devices`")]
pub fn print_avaiable_devices(objects: &ObjectsMap) {
    print_available_devices(objects)
}
