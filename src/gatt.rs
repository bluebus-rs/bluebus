use zbus::object_server::Interface;
use zbus::{Connection, fdo, interface, proxy};
use zbus::zvariant::Optional;

#[proxy(name = "org.bluez.GattService1")]
pub trait GattService1 {
    #[zbus(property)]
    fn uuid(&self) -> fdo::Result<String>;

    #[zbus(property)]
    fn primary(&self) -> fdo::Result<bool>;

    #[zbus(property)]
    fn device(&self) -> fdo::Result<Optional<String>>;

    #[zbus(property)]
    fn includes(&self) -> fdo::Result<Vec<String>>;

    #[zbus(property)]
    fn handle(&self) -> fdo::Result<Optional<u16>>;
}

pub struct GattService {
    pub uuid: String,
    pub primary: bool,
    pub device: Option<String>,
    pub includes: Vec<String>,
    pub handle: Option<u16>,
}

#[interface(name = "org.bluez.GattService1")]
impl GattService {
    fn uuid(&self) -> fdo::Result<String> {
        Ok(self.uuid.clone())
    }

    fn primary(&self) -> fdo::Result<bool> {
        Ok(self.primary)
    }

    fn device(&self) -> fdo::Result<Optional<String>> {
        Ok(Optional::from(self.device.clone()))
    }

    fn includes(&self) -> fdo::Result<Vec<String>> {
        Ok(self.includes.clone())
    }

    fn handle(&self) -> fdo::Result<Optional<u16>> {
        Ok(Optional::from(self.handle))
    }
}

#[proxy(interface = "org.bluez.GattCharacteristic1")]
trait GattCharacteristic1 {
    async fn read_value(
        &self,
        options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> fdo::Result<Vec<u8>>;

    async fn write_value(
        &mut self,
        value: Vec<u8>,
        options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> fdo::Result<()>;

    async fn start_notify(&mut self) -> fdo::Result<()>;

    async fn stop_notify(&mut self) -> fdo::Result<()>;

    #[zbus(property)]
    fn uuid(&self) -> fdo::Result<String>;

    #[zbus(property)]
    fn service(&self) -> fdo::Result<String>;

    #[zbus(property)]
    fn value(&self) -> fdo::Result<Optional<Vec<u8>>>;

    #[zbus(property)]
    fn write_acquired(&self) -> fdo::Result<bool>;

    #[zbus(property)]
    fn notify_acquired(&self) -> fdo::Result<bool>;

    #[zbus(property)]
    fn notifying(&self) -> fdo::Result<bool>;

    #[zbus(property)]
    fn flags(&self) -> fdo::Result<Vec<String>>;

    #[zbus(property)]
    fn handle(&self) -> fdo::Result<Optional<u16>>;

    #[zbus(property)]
    fn mtu(&self) -> fdo::Result<Optional<u16>>;
}

pub struct GattCharacteristic {
    pub uuid: String,
    pub service: String,
    pub value: Option<Vec<u8>>,
    pub write_acquired: bool,
    pub notify_acquired: bool,
    pub notifying: bool,
    pub flags: Vec<String>,
    pub handle: Option<u16>,
    pub mtu: Option<u16>,
}

#[interface(name = "org.bluez.GattCharacteristic1")]
impl GattCharacteristic {
    async fn read_value(
        &self,
        _options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> fdo::Result<Vec<u8>> {
        Ok(self.value.clone().unwrap_or_default())
    }

    async fn write_value(
        &mut self,
        value: Vec<u8>,
        _options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> fdo::Result<()> {
        self.value = Some(value);
        Ok(())
    }

    async fn start_notify(&mut self) -> fdo::Result<()> {
        self.notifying = true;
        Ok(())
    }

    async fn stop_notify(&mut self) -> fdo::Result<()> {
        self.notifying = false;
        Ok(())
    }

    fn uuid(&self) -> fdo::Result<String> {
        Ok(self.uuid.clone())
    }

    fn service(&self) -> fdo::Result<String> {
        Ok(self.service.clone())
    }

    fn value(&self) -> fdo::Result<Optional<Vec<u8>>> {
        Ok(Optional::from(self.value.clone()))
    }

    fn write_acquired(&self) -> fdo::Result<bool> {
        Ok(self.write_acquired)
    }

    fn notify_acquired(&self) -> fdo::Result<bool> {
        Ok(self.notify_acquired)
    }

    fn notifying(&self) -> fdo::Result<bool> {
        Ok(self.notifying)
    }

    fn flags(&self) -> fdo::Result<Vec<String>> {
        Ok(self.flags.clone())
    }

    fn handle(&self) -> fdo::Result<Optional<u16>> {
        Ok(Optional::from(self.handle))
    }

    fn mtu(&self) -> fdo::Result<Optional<u16>> {
        Ok(Optional::from(self.mtu))
    }
}

pub async fn register_service(
    connection: &Connection,
    service_path: &str,
    service: impl Interface + 'static,
) -> fdo::Result<()> {
    connection.object_server().at(service_path, service).await?;
    Ok(())
}

pub async fn register_characteristic(
    connection: &Connection,
    characteristic_path: &str,
    characteristic: impl Interface + 'static,
) -> fdo::Result<()> {
    connection
        .object_server()
        .at(characteristic_path, characteristic)
        .await?;
    Ok(())
}
