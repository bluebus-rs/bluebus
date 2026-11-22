//! Functions that open their own system-bus connection, pointed at the mock
//! through `DBUS_SYSTEM_BUS_ADDRESS`. Kept in a separate test binary because
//! it changes the process environment.

mod common;

#[tokio::test]
async fn path_less_helpers_use_the_system_bus() {
    let bz = common::MockBluez::start().await;
    // SAFETY: this binary has a single test, nothing reads the env concurrently.
    unsafe { std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", &bz.bus.address) };

    let conn = bluebus::get_system_connection().await.unwrap();
    let adapter = bluebus::adapter_proxy(&conn).await.unwrap();
    adapter.set_powered(true).await.unwrap();
    adapter.start_discovery().await.unwrap();

    let devices = bluebus::list_devices().await;
    assert_eq!(devices.len(), 2);
    assert_eq!(bluebus::list_system_devices().await.unwrap().len(), 2);

    bluebus::set_adapter_path(common::ADAPTER1);
    let devices = bluebus::list_devices().await;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].alias, "Other");
    bluebus::set_adapter_path(bluebus::DEFAULT_ADAPTER_PATH);

    // The high-level session honours DBUS_SYSTEM_BUS_ADDRESS too.
    let session = bluebus::ble::Session::new().await.unwrap();
    assert_eq!(session.adapters().await.unwrap().len(), 2);
    assert_eq!(session.default_adapter().await.unwrap().name(), "hci0");

    drop(bz);
    // Errors become an empty list in the infallible variant.
    assert!(bluebus::list_devices().await.is_empty());
    assert!(bluebus::list_system_devices().await.is_err());
    assert!(matches!(
        bluebus::ble::Session::new().await,
        Err(bluebus::ble::Error::Bus(_))
    ));
}
