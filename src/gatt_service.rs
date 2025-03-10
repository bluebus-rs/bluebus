#[allow(non_snake_case)]
#[derive(zbus::zvariant::Type, zbus::zvariant::SerializeDict)]
struct GattServiceProperties {
    UUID: String,
    Primary: bool,
    Device: zbus::zvariant::Optional<zbus::zvariant::OwnedObjectPath>,
    Includes: zbus::zvariant::Optional<Vec<zbus::zvariant::OwnedObjectPath>>,
    Handle: zbus::zvariant::Optional<u16>,
}

impl Default for GattServiceProperties {
    fn default() -> Self {
        Self {
            UUID: String::new(),
            Primary: false,
            Device: zbus::zvariant::Optional::from(None),
            Includes: zbus::zvariant::Optional::from(Some(Vec::new())),
            Handle: zbus::zvariant::Optional::from(Some(0)),
        }
    }
}

#[zbus::proxy(interface = "org.bluez.GattService1")]
pub trait GattService {
    fn UUID(&self) -> zbus::Result<String>;
}
