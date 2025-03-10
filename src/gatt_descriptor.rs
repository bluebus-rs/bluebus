#[allow(non_snake_case)]
#[derive(zbus::zvariant::Type, zbus::zvariant::SerializeDict)]
pub struct GattDescriptorProperties {
    UUID: String,
    Characteristic: zbus::zvariant::OwnedObjectPath,
    Device: Vec<u8>,
    Includes: Vec<String>,
    Handle: zbus::zvariant::Optional<u16>,
}

impl Default for GattDescriptorProperties {
    fn default() -> Self {
        Self {
            UUID: String::new(),
            Characteristic: zbus::zvariant::OwnedObjectPath::default(),
            Device: Vec::new(),
            Includes: Vec::new(),
            Handle: zbus::zvariant::Optional::from(Some(0)),
        }
    }
}

#[zbus::proxy(interface = "org.bluez.GattDescriptor1")]
pub trait GattDescriptor {
    fn read(&self, options: std::collections::HashMap<String, String>) -> zbus::Result<String>;
    fn write(
        &self,
        value: Vec<u8>,
        options: std::collections::HashMap<String, String>,
    ) -> zbus::Result<String>;
}
