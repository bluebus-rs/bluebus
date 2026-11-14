//! Code written against the 0.1.10 API must keep compiling (and the parts
//! that worked must keep working). Compile-time check: nothing here talks
//! to a bus unless called from a test.
#![allow(deprecated, dead_code, unused_variables, unused_mut)]

use std::collections::HashMap;
use std::sync::Arc;

use bluebus::*;
use tokio::sync::mpsc;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

async fn old_style_usage(conn: zbus::Connection) -> zbus::Result<()> {
    // Globals and their accessors.
    *ADAPTER_PATH.lock().unwrap() = "/org/bluez/hci0".to_string();
    set_adapter_path("/org/bluez/hci0");
    let p: String = get_adapter_path();
    set_gatt_application_path(&get_gatt_application_path());
    set_advertisement_path(&get_advertisement_path());
    let _ = GATT_APPLICATION_PATH.lock().unwrap().clone();
    let _ = ADVERTISEMENT_PATH.lock().unwrap().clone();

    // Proxies built the 0.1.10 way.
    let adapter = AdapterProxy::builder(&conn)
        .path(get_adapter_path())?
        .build()
        .await?;
    let adapter2 = AdapterProxy::new(&conn, "/org/bluez/hci0").await?;
    let _: bool = adapter.powered().await?;
    adapter.set_powered(true).await?;
    adapter.start_discovery().await?;
    adapter.stop_discovery().await?;
    adapter
        .remove_device(OwnedObjectPath::try_from("/org/bluez/hci0/dev_00_00_00_00_00_00").unwrap())
        .await?;
    let _: String = adapter.address().await?;
    let _: String = adapter.name().await?;
    adapter.set_alias("x").await?;
    adapter.set_discoverable(true).await?;

    let dev = DeviceProxy::new(&conn, "/org/bluez/hci0/dev_00_00_00_00_00_00").await?;
    dev.connect().await?;
    dev.pair().await?;
    dev.cancel_pairing().await?;
    dev.set_trusted(true).await?;
    dev.set_blocked(false).await?;
    let _: (bool, bool, bool, String, String) = (
        dev.connected().await?,
        dev.paired().await?,
        dev.services_resolved().await?,
        dev.address().await?,
        dev.name().await?,
    );
    dev.disconnect().await?;

    let objects = ObjectManagerProxy::new(&conn)
        .await?
        .get_managed_objects()
        .await?;
    print_key_pairs(&objects);
    print_avaiable_devices(&objects);

    let mgr = AgentManagerProxy::new(&conn, "/org/bluez").await?;
    let path = OwnedObjectPath::try_from("/agent").unwrap();
    mgr.register_agent(&path, "NoInputNoOutput".to_string())
        .await?;
    mgr.request_default_agent(&path).await?;
    mgr.unregister_agent(&path).await?;

    let agent = Agent::new("0000".to_string(), 123456);
    let _: String = agent.request_pin_code(&path).await?;
    let _: u32 = agent.request_passkey(&path).await?;
    agent.display_pin_code(&path, "1").await?;
    agent.display_passkey(&path, 1, 1).await?;
    agent.request_confirmation(&path, 1)?;
    agent.request_authorization(&path).await?;
    agent.authorize_service(&path, "uuid").await?;
    agent.cancel().await?;
    agent.release().await?;

    // GATT client, including the old `&mut self` calling style.
    let mut chr = GattCharacteristic1Proxy::new(
        &conn,
        "org.bluez",
        "/org/bluez/hci0/dev_00_00_00_00_00_00/service0001/char0002",
    )
    .await?;
    let opts: HashMap<String, OwnedValue> = HashMap::new();
    let _: Vec<u8> = chr.read_value(opts.clone()).await?;
    chr.write_value(vec![1], opts).await?;
    chr.start_notify().await?;
    chr.stop_notify().await?;
    let _ = (
        chr.uuid().await?,
        chr.service().await?,
        chr.value().await?,
        chr.flags().await?,
        chr.handle().await?,
        chr.mtu().await?,
    );
    let _ = (
        chr.write_acquired().await?,
        chr.notify_acquired().await?,
        chr.notifying().await?,
    );
    let svc = GattService1Proxy::new(
        &conn,
        "org.bluez",
        "/org/bluez/hci0/dev_00_00_00_00_00_00/service0001",
    )
    .await?;
    let _ = (
        svc.uuid().await?,
        svc.primary().await?,
        svc.device().await?,
        svc.includes().await?,
        svc.handle().await?,
    );

    // GATT server objects built with struct literals.
    register_service(
        &conn,
        "/app/s0",
        GattService {
            uuid: "180f".into(),
            primary: true,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await?;
    register_characteristic(
        &conn,
        "/app/s0/c0",
        GattCharacteristic {
            uuid: "2a19".into(),
            service: "/app/s0".into(),
            value: None,
            write_acquired: false,
            notify_acquired: false,
            notifying: false,
            flags: vec!["read".into()],
            handle: None,
            mtu: None,
        },
    )
    .await?;

    // Advertising.
    register_advertisement(&conn, LEAdvertisementProperties::default()).await?;
    unregister_advertisement(&conn).await?;

    // Device snapshots.
    let _: Vec<DeviceInfo> = list_devices().await;
    let _ = list_system_devices()
        .await
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let info = DeviceInfo {
        address: String::new(),
        alias: String::new(),
        connected: false,
        paired: false,
        address_type: None,
        connectable: None,
        discoverable: None,
        discoverable_timeout: None,
        discovering: None,
        name: None,
        pairable: None,
        pairable_timeout: None,
        power_state: None,
        powered: None,
        trusted: None,
        rssi: None,
    };

    // Monitor with the 0.1.10 constructor and methods.
    let conn = Arc::new(conn);
    let manager = Arc::new(ObjectManagerProxy::new(&conn).await?);
    let (tx, _rx) = mpsc::channel::<DeviceInfo>(8);
    let monitor = Arc::new(Monitor::new(conn.clone(), manager, tx.clone(), tx.clone(), tx).await);
    let m = monitor.clone();
    tokio::spawn(async move { m.monitor_device_added().await });
    tokio::spawn(async move { monitor.monitor_device_removed().await });

    let _c: zbus::Connection = get_system_connection().await?;
    Ok(())
}

#[test]
fn compiles() {
    let _ = old_style_usage;
}
