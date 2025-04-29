use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};


/// Type alias representing a mapping of managed D-Bus objects.
type ObjectsMap = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

#[zbus::proxy(
    default_service = "org.bluez",
    interface = "org.freedesktop.DBus.ObjectManager",
    default_path = "/"
)]
pub trait ObjectManager {
    /// Retrieves a map of all managed objects.
    /// 
    /// # Returns
    /// A `Result` containing a map of object paths to their interfaces and properties.
    fn get_managed_objects(&self) -> zbus::Result<ObjectsMap>;

    /// Signal emitted when new interfaces are added to an object.
    /// It is suggested to use this function in a tokio::thread.
    /// 
    /// # Arguments
    /// * `object_path` - The path of the object where interfaces were added.
    /// * `interfaces` - A map of interface names to their properties.
    /// 
    /// # Returns
    /// A `Result` indicating success or failure.
    #[zbus(signal)]
    fn interfaces_added(
        &self,
        object_path: OwnedObjectPath,
        interfaces: HashMap<String, HashMap<String, OwnedValue>>,
    ) -> zbus::Result<()>;

    /// Signal emitted when interfaces are removed from an object.
    /// It is suggested to use this function in a tokio::thread.
    /// 
    /// # Arguments
    /// * `object_path` - The path of the object where interfaces were removed.
    /// * `interfaces` - A list of removed interface names.
    /// 
    /// # Returns
    /// A `Result` indicating success or failure.
    #[zbus(signal)]
    fn interfaces_removed(
        &self,
        object_path: OwnedObjectPath,
        interfaces: Vec<String>,
    ) -> zbus::Result<()>;
}

/// Prints the object paths of all managed objects.
/// 
/// # Arguments
/// * `objects` - A reference to the managed objects map.
pub fn print_key_pairs(objects: &ObjectsMap) {
    for (path, _interfaces) in objects.iter() {
        println!("Object Path: {}", path);
    }
}

/// Prints the available Bluetooth devices based on managed objects.
/// 
/// # Arguments
/// * `objects` - A reference to the managed objects map.
pub fn print_avaiable_devices(objects: &ObjectsMap) {
    for (path, _interfaces) in objects.iter() {
        println!("Object Path: {}", path);
    }
}
