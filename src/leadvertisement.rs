use std::collections::HashMap;
use zbus;
use zbus::object_server::Interface;
use zbus::zvariant::{ObjectPath, Optional, OwnedValue, Value};
use zbus::{fdo, interface};

#[allow(non_snake_case)]
pub struct LEAdvertisementProperties {
    Type: String,
    LocalName: Optional<String>,
    ServiceUUIDs: Vec<String>,
    SolicitUUIDs: Vec<String>,
    ManufacturerData: HashMap<u16, OwnedValue>,
    ServiceData: HashMap<String, OwnedValue>,
    Data: HashMap<u8, OwnedValue>,
    Discoverable: Optional<bool>,
    DiscoverableTimeout: Optional<u16>,
    Includes: Vec<String>,
    Duration: Optional<u16>,
    Timeout: Optional<u16>,
    MinInterval: Optional<u32>,
    MaxInterval: Optional<u32>,
    TxPower: Optional<i16>,
    Appearance: Optional<u16>,
    SecondaryChannel: Optional<String>,
}

#[interface(name = "org.bluez.LEAdvertisement1")]
impl LEAdvertisementProperties {
    #[zbus(property)]
    fn type_(&self) -> fdo::Result<&str> {
        Ok(&self.Type)
    }

    #[zbus(property)]
    fn local_name(&self) -> fdo::Result<Optional<String>> {
        Ok(self.LocalName.clone())
    }

    #[zbus(property)]
    fn service_uuids(&self) -> fdo::Result<Vec<String>> {
        Ok(self.ServiceUUIDs.clone())
    }

    #[zbus(property)]
    fn solicit_uuids(&self) -> fdo::Result<Vec<String>> {
        Ok(self.SolicitUUIDs.clone())
    }

    #[zbus(property)]
    fn manufacturer_data(&self) -> fdo::Result<HashMap<u16, OwnedValue>> {
        Ok(self.ManufacturerData.clone())
    }

    #[zbus(property)]
    fn service_data(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        Ok(self.ServiceData.clone())
    }

    #[zbus(property)]
    fn data(&self) -> fdo::Result<HashMap<u8, OwnedValue>> {
        Ok(self.Data.clone())
    }

    #[zbus(property)]
    fn discoverable(&self) -> fdo::Result<Optional<bool>> {
        Ok(self.Discoverable.clone())
    }

    #[zbus(property)]
    fn discoverable_timeout(&self) -> fdo::Result<Optional<u16>> {
        Ok(self.DiscoverableTimeout.clone())
    }

    #[zbus(property)]
    fn includes(&self) -> fdo::Result<Vec<String>> {
        Ok(self.Includes.clone())
    }

    #[zbus(property)]
    fn duration(&self) -> fdo::Result<Optional<u16>> {
        Ok(self.Duration.clone())
    }

    #[zbus(property)]
    fn timeout(&self) -> fdo::Result<Optional<u16>> {
        Ok(self.Timeout.clone())
    }

    #[zbus(property)]
    fn min_interval(&self) -> fdo::Result<Optional<u32>> {
        Ok(self.MinInterval.clone())
    }

    #[zbus(property)]
    fn max_interval(&self) -> fdo::Result<Optional<u32>> {
        Ok(self.MaxInterval.clone())
    }

    #[zbus(property)]
    fn tx_power(&self) -> fdo::Result<Optional<i16>> {
        Ok(self.TxPower.clone())
    }

    #[zbus(property)]
    fn appearance(&self) -> fdo::Result<Optional<u16>> {
        Ok(self.Appearance.clone())
    }

    #[zbus(property)]
    fn secondary_channel(&self) -> fdo::Result<Optional<String>> {
        Ok(self.SecondaryChannel.clone())
    }

    fn release(&self) -> fdo::Result<()> {
        println!("Advertisement released.");
        Ok(())
    }
}

impl Default for LEAdvertisementProperties {
    fn default() -> Self {
        Self {
            Type: "peripheral".to_string(), // Must be "broadcast" or "peripheral"
            LocalName: Optional::from(Some("MyDevice".to_string())), // Optional local name
            ServiceUUIDs: vec!["180D".to_string(), "180F".to_string()], // List of service UUIDs
            SolicitUUIDs: Vec::new(),       // Optional solicit UUIDs
            ManufacturerData: HashMap::new(), // Optional manufacturer data
            ServiceData: HashMap::new(),    // Optional service data
            Data: HashMap::new(),           // Optional data
            Discoverable: Optional::from(Some(true)), // Optional discoverable flag
            DiscoverableTimeout: Optional::from(Some(30)), // Optional discoverable timeout
            Includes: vec!["tx-power".to_string()], // Optional includes
            Duration: Optional::from(Some(60)), // Optional duration
            Timeout: Optional::from(Some(120)), // Optional timeout
            MinInterval: Optional::from(Some(100)), // Optional min interval
            MaxInterval: Optional::from(Some(200)), // Optional max interval
            TxPower: Optional::from(Some(-20)), // Optional TX power
            Appearance: Optional::from(Some(0x0040)), // Optional appearance (e.g., 0x0040 for HID)
            SecondaryChannel: Optional::from(Some("1M".to_string())), // Optional secondary channel
        }
    }
}

#[zbus::proxy(interface = "org.bluez.LEAdvertisement1")]
trait LEAdvertisement {
    fn Release(&self) -> zbus::Result<()>;
}

pub async fn register_advertisement(
    conn: &zbus::Connection,
    advertisement: impl Interface + 'static,
) -> zbus::Result<()> {
    let adapter_path = crate::paths::get_adapter_path();
    let adv_path = crate::paths::get_advertisement_path();
    let bus_name = zbus::names::BusName::try_from("org.bluez")?;

    // Convert the advertisement path to an ObjectPath
    let adv_object_path = ObjectPath::try_from(adv_path.as_str())?;

    // Create a proxy for the LEAdvertisingManager1 interface
    let adapter_proxy = zbus::Proxy::new(
        conn,
        bus_name,
        adapter_path,
        "org.bluez.LEAdvertisingManager1",
    )
    .await?;

    // Register the advertisement on the D-Bus
    conn.object_server()
        .at(adv_object_path.clone(), advertisement)
        .await?;

    // Prepare the options dictionary (a{sv})
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("Type", Value::new("peripheral")); // Example option

    // Call the RegisterAdvertisement method
    adapter_proxy
        .call_method("RegisterAdvertisement", &(adv_object_path.clone(), options))
        .await?;

    Ok(())
}

pub async fn unregister_advertisement(conn: &zbus::Connection) -> zbus::Result<()> {
    let adapter_path = crate::paths::get_adapter_path();
    let adv_path = &crate::paths::get_advertisement_path();
    let bus_name = zbus::names::BusName::try_from("org.bluez")?;

    let adapter_proxy = zbus::Proxy::new(
        conn,
        bus_name,
        adapter_path,
        "org.bluez.LEAdvertisingManager1",
    )
    .await?;
    adapter_proxy
        .call_method("UnRegisterAdvertisement", &(adv_path))
        .await?;

    Ok(())
}
