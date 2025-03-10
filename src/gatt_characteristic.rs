#[allow(non_snake_case)]
#[derive(zbus::zvariant::Type, zbus::zvariant::SerializeDict)]
pub struct GattCharacteristicProperties {
    UUID: String,
    Service: zbus::zvariant::OwnedObjectPath,
    Flags: Vec<String>,
    Value: Vec<u8>,
    WriteAcquired: zbus::zvariant::Optional<bool>,
    NotifyAcquired: zbus::zvariant::Optional<bool>,
    Notifying: zbus::zvariant::Optional<bool>,
    Handle: zbus::zvariant::Optional<u16>,
    MTU: zbus::zvariant::Optional<u16>,
}

impl Default for GattCharacteristicProperties {
    fn default() -> Self {
        Self {
            UUID: String::new(),
            Service: zbus::zvariant::OwnedObjectPath::default(),
            Flags: Vec::new(),
            Value: Vec::new(),
            WriteAcquired: zbus::zvariant::Optional::from(Some(false)),
            NotifyAcquired: zbus::zvariant::Optional::from(Some(false)),
            Notifying: zbus::zvariant::Optional::from(Some(false)),
            Handle: zbus::zvariant::Optional::from(Some(0)),
            MTU: zbus::zvariant::Optional::from(Some(0)),
        }
    }
}

#[zbus::proxy(interface = "org.bluez.GattCharacteristic1")]
pub trait GattCharacteristic {
    fn ReadValue(
        &self,
        options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::Result<Vec<u8>>;
    fn WriteValue(
        &self,
        value: Vec<u8>,
        options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::Result<()>;
    fn StartNotify(&self) -> zbus::Result<()>;
    fn StopNotify(&self) -> zbus::Result<()>;
}
