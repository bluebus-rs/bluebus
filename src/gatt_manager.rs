#[zbus::proxy(interface = "org.bluez.GattManager1")]
pub trait GattManager {
    fn register_application(
        &self,
        application: &str,
        options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::Result<()>;
    fn unregister_application(&self, application: &str) -> zbus::Result<()>;
}

pub async fn register_gatt_application(conn: &zbus::Connection) -> zbus::Result<()> {
    let adapter_path = crate::paths::get_adapter_path();
    let application_path = &crate::paths::get_gatt_application_path();
    let bus_name = zbus::names::BusName::try_from("org.bluez")?;
    let options: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
        std::collections::HashMap::new();

    let gatt_manager = GattManagerProxy::new(conn, bus_name, adapter_path).await?;
    gatt_manager
        .register_application(application_path, options)
        .await?;
    Ok(())
}

pub async fn unregister_gatt_application(conn: &zbus::Connection) -> zbus::Result<()> {
    let adapter_path = crate::paths::get_adapter_path();
    let application_path = &crate::paths::get_gatt_application_path();
    let bus_name = zbus::names::BusName::try_from("org.bluez")?;

    let gatt_manager = GattManagerProxy::new(conn, bus_name, adapter_path).await?;
    gatt_manager
        .unregister_application(application_path)
        .await?;
    Ok(())
}
