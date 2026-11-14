//! Smoke test against the real BlueZ daemon and adapter.
//!
//! ```sh
//! cargo run --example hardware_smoke            # default adapter
//! cargo run --example hardware_smoke -- hci1    # another adapter
//! ```
//!
//! Non-destructive: it does not pair, connect or remove devices, does not
//! replace the default agent, and restores the adapter's power state.

use std::sync::Arc;
use std::time::Duration;

use bluebus::*;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(hci) = std::env::args().nth(1) {
        set_adapter_path(&format!("/org/bluez/{hci}"));
    }
    let adapter_path = get_adapter_path();
    let conn = get_system_connection().await?;
    let mut failures = 0;
    macro_rules! check {
        ($name:expr, $e:expr) => {
            match $e {
                Ok(v) => {
                    println!("  ok   {:<38} {:?}", $name, v);
                    Some(v)
                }
                Err(e) => {
                    failures += 1;
                    println!("  FAIL {:<38} {}", $name, e);
                    None
                }
            }
        };
    }

    println!("adapters: {:?}", list_adapters(&conn).await?);
    let adapter = adapter_proxy(&conn).await?;
    let was_powered = adapter.powered().await?;

    println!("\n[adapter {adapter_path}]");
    check!("Address", adapter.address().await);
    check!(
        "Name / Alias",
        Ok::<_, zbus::Error>((adapter.name().await?, adapter.alias().await?))
    );
    check!("Class", adapter.class().await.map(|c| format!("{c:#08x}")));
    check!("AddressType", adapter.address_type().await);
    check!("UUIDs (count)", adapter.uuids().await.map(|u| u.len()));
    check!("Roles", adapter.roles().await);
    match adapter.power_state().await {
        Ok(v) => println!("  ok   {:<38} {:?}", "PowerState", v),
        Err(_) => println!(
            "  info {:<38} not provided by this BlueZ version",
            "PowerState"
        ),
    }
    check!(
        "set_adapter_powered(true)",
        set_adapter_powered(&adapter, true, Duration::from_secs(10)).await
    );
    check!(
        "Discoverable/Pairable",
        Ok::<_, zbus::Error>((adapter.discoverable().await?, adapter.pairable().await?))
    );
    check!("GetDiscoveryFilters", adapter.get_discovery_filters().await);

    println!("\n[agent]");
    check!(
        "register_agent (not default)",
        register_agent(
            &conn,
            "/bluebus/smoke/agent",
            AgentCapability::NoInputNoOutput,
            Agent::new("0000".into(), 0),
            false
        )
        .await
    );
    check!(
        "unregister_agent",
        unregister_agent(&conn, "/bluebus/smoke/agent").await
    );

    println!("\n[discovery + monitor, 6 s]");
    let conn_arc = Arc::new(conn.clone());
    let manager = Arc::new(ObjectManagerProxy::new(&conn).await?);
    let (added_tx, mut added) = mpsc::channel(256);
    let (removed_tx, mut removed) = mpsc::channel(256);
    let (changed_tx, mut changed) = mpsc::channel(256);
    let monitor = Arc::new(Monitor::new(conn_arc, manager, added_tx, removed_tx, changed_tx).await);
    let runner = tokio::spawn({
        let m = monitor.clone();
        async move { m.run().await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    check!(
        "SetDiscoveryFilter(auto, dup)",
        adapter
            .set_discovery_filter(
                DiscoveryFilter::new()
                    .transport(Transport::Auto)
                    .duplicate_data(true)
                    .to_dict()
            )
            .await
    );
    check!("StartDiscovery", adapter.start_discovery().await);
    tokio::time::sleep(Duration::from_secs(6)).await;
    check!("StopDiscovery", adapter.stop_discovery().await);
    check!(
        "SetDiscoveryFilter(reset)",
        adapter.set_discovery_filter(Default::default()).await
    );
    let (mut a, mut r, mut c) = (0, 0, 0);
    while added.try_recv().is_ok() {
        a += 1;
    }
    while removed.try_recv().is_ok() {
        r += 1;
    }
    while changed.try_recv().is_ok() {
        c += 1;
    }
    println!(
        "  monitor events: added={a} removed={r} changed={c}, cached devices={}",
        monitor.devices().await.len()
    );
    runner.abort();

    let devices = check!(
        "list_adapter_devices_with_paths",
        list_adapter_devices_with_paths(&conn, &adapter_path)
            .await
            .map(|d| d.len())
    );
    if devices.is_some() {
        for (path, d) in list_adapter_devices_with_paths(&conn, &adapter_path)
            .await?
            .iter()
            .take(15)
        {
            println!(
                "       {} {:<24} rssi={:<5} paired={} connected={}",
                d.address,
                d.alias,
                d.rssi.map(|r| r.to_string()).unwrap_or("-".into()),
                d.paired,
                d.connected
            );
            let _ = path;
        }
    }
    // Read-only look at paired devices.
    for (path, d) in list_adapter_devices_with_paths(&conn, &adapter_path)
        .await?
        .into_iter()
        .filter(|(_, d)| d.paired)
    {
        let dev = DeviceProxy::new(&conn, path.clone()).await?;
        println!("  paired {} ({})", d.alias, d.address);
        check!(
            "    Device.Adapter",
            dev.adapter().await.map(|p| p.to_string())
        );
        check!(
            "    Device.Trusted/Bonded",
            Ok::<_, zbus::Error>((dev.trusted().await?, dev.bonded().await.ok()))
        );
        check!(
            "    Device.UUIDs (count)",
            dev.uuids().await.map(|u| u.len())
        );
        check!(
            "    Device.Icon",
            dev.icon()
                .await
                .or_else(|_| Ok::<_, zbus::Error>("<none>".into()))
        );
    }

    println!("\n[GATT server]");
    let app = "/bluebus/smoke/app";
    let svc = format!("{app}/service0");
    let chr = format!("{svc}/char0");
    register_service(
        &conn,
        &svc,
        GattService {
            uuid: "a7f0e000-1b5e-4a8f-9d6c-6c7b5d1b0001".into(),
            primary: true,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await?;
    register_characteristic(
        &conn,
        &chr,
        GattCharacteristic::new(
            "a7f0e000-1b5e-4a8f-9d6c-6c7b5d1b0002",
            svc.clone(),
            &["read", "write", "notify"],
        )
        .with_value(*b"bluebus"),
    )
    .await?;
    register_descriptor(
        &conn,
        &format!("{chr}/desc0"),
        GattDescriptor {
            uuid: "2901".into(),
            characteristic: chr.clone(),
            value: b"smoke".to_vec(),
            flags: vec!["read".into()],
            handle: None,
        },
    )
    .await?;
    check!(
        "RegisterApplication",
        register_gatt_application_at(&conn, &adapter_path, app).await
    );
    check!(
        "notify_characteristic_value",
        notify_characteristic_value(&conn, &chr, b"v2".to_vec()).await
    );
    check!(
        "UnregisterApplication",
        unregister_gatt_application_at(&conn, &adapter_path, app).await
    );

    println!("\n[LE advertising]");
    let mgr = LEAdvertisingManager1Proxy::new(&conn, adapter_path.as_str()).await?;
    check!("SupportedInstances", mgr.supported_instances().await);
    check!("SupportedIncludes", mgr.supported_includes().await);
    check!(
        "SupportedSecondaryChannels",
        mgr.supported_secondary_channels().await
    );
    let adv_path = "/bluebus/smoke/adv0";
    let adv = LEAdvertisementProperties::new(AdvertisementType::Peripheral)
        .with_local_name("bluebus-smoke")
        .with_service_uuids(["a7f0e000-1b5e-4a8f-9d6c-6c7b5d1b0001"])
        .with_manufacturer_data(0xFFFF, vec![0xB1, 0x0E])
        .with_discoverable(true)
        .with_includes(["tx-power"]);
    check!(
        "RegisterAdvertisement (custom)",
        register_advertisement_at(&conn, &adapter_path, adv_path, adv).await
    );
    check!(
        "ActiveInstances",
        LEAdvertisingManager1Proxy::builder(&conn)
            .path(adapter_path.as_str())?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?
            .active_instances()
            .await
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    check!(
        "UnregisterAdvertisement",
        unregister_advertisement_at(&conn, &adapter_path, adv_path).await
    );
    check!(
        "re-RegisterAdvertisement (broadcast)",
        register_advertisement_at(
            &conn,
            &adapter_path,
            adv_path,
            LEAdvertisementProperties::new(AdvertisementType::Broadcast)
                .with_manufacturer_data(0xFFFF, vec![1])
        )
        .await
    );
    check!(
        "UnregisterAdvertisement",
        unregister_advertisement_at(&conn, &adapter_path, adv_path).await
    );
    match register_advertisement(&conn, LEAdvertisementProperties::default()).await {
        Ok(()) => {
            println!("  ok   RegisterAdvertisement (Default)");
            check!(
                "UnregisterAdvertisement (Default)",
                unregister_advertisement(&conn).await
            );
        }
        Err(e) => {
            println!("  info RegisterAdvertisement (Default)    {e}  (controller/BlueZ dependent)")
        }
    }

    if !was_powered {
        check!(
            "restore Powered=false",
            set_adapter_powered(&adapter, false, Duration::from_secs(10)).await
        );
    }
    println!(
        "\n{}",
        if failures == 0 {
            "ALL CHECKS PASSED".to_string()
        } else {
            format!("{failures} CHECK(S) FAILED")
        }
    );
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
