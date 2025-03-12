#[allow(non_snake_case)]
#[derive(zbus::zvariant::Type, zbus::zvariant::SerializeDict)]
struct LEAdvertisementProperties {
    Type: String,
    LocalName: zbus::zvariant::Optional<String>,
    ServiceUUIDs: Vec<String>,
    SolicitUUIDs: Vec<String>,
    ManufacturerData: std::collections::HashMap<u16, zbus::zvariant::OwnedValue>,
    ServiceData: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    Data: std::collections::HashMap<u8, zbus::zvariant::OwnedValue>,
    Discoverable: zbus::zvariant::Optional<bool>,
    DiscoverableTimeout: zbus::zvariant::Optional<u16>,
    Includes: Vec<String>,
    Duration: zbus::zvariant::Optional<u16>,
    Timeout: zbus::zvariant::Optional<u16>,
    MinInterval: zbus::zvariant::Optional<u32>,
    MaxInterval: zbus::zvariant::Optional<u32>,
    TxPower: zbus::zvariant::Optional<i16>,
    Appearance: zbus::zvariant::Optional<u16>,
    SecondaryChannel: zbus::zvariant::Optional<String>,
}

impl Default for LEAdvertisementProperties {
    fn default() -> Self {
        Self {
            Type: String::new(),
            LocalName: zbus::zvariant::Optional::from(Some(String::new())),
            ServiceUUIDs: Vec::new(),
            SolicitUUIDs: Vec::new(),
            ManufacturerData: std::collections::HashMap::new(),
            ServiceData: std::collections::HashMap::new(),
            Data: std::collections::HashMap::new(),
            Discoverable: zbus::zvariant::Optional::from(Some(false)),
            DiscoverableTimeout: zbus::zvariant::Optional::from(Some(0)),
            Includes: Vec::new(),
            Duration: zbus::zvariant::Optional::from(Some(0)),
            Timeout: zbus::zvariant::Optional::from(Some(0)),
            MinInterval: zbus::zvariant::Optional::from(Some(0)),
            MaxInterval: zbus::zvariant::Optional::from(Some(0)),
            TxPower: zbus::zvariant::Optional::from(Some(0)),
            Appearance: zbus::zvariant::Optional::from(Some(0)),
            SecondaryChannel: zbus::zvariant::Optional::from(Some(String::new())),
        }
    }
}

#[zbus::proxy(interface = "org.bluez.LEAdvertisement1")]
trait LEAdvertisement {
    fn Release(&self) -> zbus::Result<()>;
}

pub async fn register_advertisement(conn: &zbus::Connection) -> zbus::Result<()> {
    let adapter_path = crate::paths::get_adapter_path();
    let adv_path = &crate::paths::get_advertisement_path();
    let bus_name = zbus::names::BusName::try_from("org.bluez")?;
    let advertisement = LEAdvertisementProperties::default();

    let adapter_proxy = zbus::Proxy::new(
        conn,
        bus_name,
        adapter_path,
        "org.bluez.LEAdvertisingManager1",
    )
    .await?;

    adapter_proxy
        .call_method("RegisterAdvertisement", &(adv_path, &advertisement))
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
