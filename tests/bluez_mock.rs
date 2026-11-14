//! End-to-end tests of every bluebus feature against a mock BlueZ daemon
//! on a private D-Bus bus. Run with `cargo test` (needs `dbus-daemon`).

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bluebus::*;
use common::*;
use futures::StreamExt;
use tokio::sync::mpsc;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

fn err_name(e: &zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        zbus::Error::FDO(e) => format!("fdo:{e}"),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adapter_power_properties_and_discovery() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;

    assert_eq!(
        list_adapters(&conn).await.unwrap(),
        vec![ADAPTER0, ADAPTER1]
    );

    let adapter = adapter_proxy(&conn).await.unwrap();
    assert_eq!(adapter.inner().path().as_str(), ADAPTER0);
    assert_eq!(adapter.address().await.unwrap(), "00:AA:00:AA:00:AA");
    assert_eq!(adapter.address_type().await.unwrap(), "public");
    assert_eq!(adapter.name().await.unwrap(), "mockhost");
    assert_eq!(adapter.class().await.unwrap(), 0x7c010c);
    assert_eq!(
        adapter.uuids().await.unwrap().len(),
        1,
        "UUIDs property name"
    );
    assert_eq!(
        adapter.roles().await.unwrap(),
        vec!["central", "peripheral"]
    );
    assert_eq!(adapter.modalias().await.unwrap(), "usb:v1D6Bp0246d0548");
    assert_eq!(adapter.manufacturer().await.unwrap(), 2);
    assert_eq!(adapter.version().await.unwrap(), 12);
    assert!(adapter.experimental_features().await.unwrap().is_empty());

    // Discovery on a powered-off adapter fails like BlueZ.
    let e = adapter.start_discovery().await.unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.NotReady");

    set_adapter_powered(&adapter, true, std::time::Duration::from_secs(5))
        .await
        .unwrap();
    assert!(adapter.powered().await.unwrap(), "cache updated");
    // Read the other properties back without zbus' cache.
    let adapter = AdapterProxy::builder(&conn)
        .path(ADAPTER0)
        .unwrap()
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .unwrap();
    assert_eq!(adapter.power_state().await.unwrap(), "on");

    adapter.set_alias("bluebus-test").await.unwrap();
    assert_eq!(adapter.alias().await.unwrap(), "bluebus-test");
    adapter.set_discoverable(true).await.unwrap();
    assert!(adapter.discoverable().await.unwrap());
    adapter.set_discoverable_timeout(60).await.unwrap();
    assert_eq!(adapter.discoverable_timeout().await.unwrap(), 60);
    adapter.set_pairable(false).await.unwrap();
    assert!(!adapter.pairable().await.unwrap());
    adapter.set_pairable_timeout(30).await.unwrap();
    assert_eq!(adapter.pairable_timeout().await.unwrap(), 30);

    // Discovery filter: the typed builder produces exactly what BlueZ accepts.
    assert!(adapter
        .get_discovery_filters()
        .await
        .unwrap()
        .contains(&"Transport".to_string()));
    let filter = DiscoveryFilter::new()
        .transport(Transport::Le)
        .rssi(-80)
        .uuids([SVC_UUID])
        .duplicate_data(false)
        .discoverable(true)
        .pattern("11:22");
    adapter
        .set_discovery_filter(filter.to_dict())
        .await
        .unwrap();
    assert_eq!(
        bz.state
            .lock()
            .unwrap()
            .discovery_filter
            .as_ref()
            .unwrap()
            .len(),
        6
    );
    adapter.set_discovery_filter(HashMap::new()).await.unwrap();

    adapter.start_discovery().await.unwrap();
    assert!(adapter.discovering().await.unwrap());
    let devices = list_adapter_devices_with_paths(&conn, ADAPTER0)
        .await
        .unwrap();
    assert_eq!(devices.len(), 2);
    let (path, headset) = &devices[0];
    assert_eq!(path, "/org/bluez/hci0/dev_11_22_33_44_55_66");
    assert_eq!(headset.address, "11:22:33:44:55:66");
    assert_eq!(headset.alias, "Headset");
    assert_eq!(headset.name.as_deref(), Some("Headset"));
    assert_eq!(headset.rssi, Some(-48));
    assert_eq!(headset.trusted, Some(false));
    assert_eq!(devices[1].1.name, None, "nameless device");

    // Devices of another adapter are not listed.
    let other = list_adapter_devices(&conn, ADAPTER1).await.unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].address, "01:02:03:04:05:06");
    assert_eq!(
        list_devices_with_connection(&conn).await.unwrap().len(),
        2,
        "default adapter = hci0"
    );

    // Only devices are printed / formatted, not adapters or services.
    let objects = ObjectManagerProxy::new(&conn)
        .await
        .unwrap()
        .get_managed_objects()
        .await
        .unwrap();
    let lines = format_available_devices(&objects);
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| l.contains("/dev_")));
    print_available_devices(&objects);
    print_key_pairs(&objects);

    adapter.stop_discovery().await.unwrap();
    assert!(!adapter.discovering().await.unwrap());
    let e = adapter.stop_discovery().await.unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.Failed");

    // Remove a device.
    let dev = OwnedObjectPath::try_from(devices[1].0.clone()).unwrap();
    adapter.remove_device(dev.clone()).await.unwrap();
    assert_eq!(
        list_adapter_devices(&conn, ADAPTER0).await.unwrap().len(),
        1
    );
    let e = adapter.remove_device(dev).await.unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.DoesNotExist");
}

// ---------------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------------

#[tokio::test]
async fn device_connect_trust_block_profiles_battery() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    let adapter = adapter_proxy(&conn).await.unwrap();
    adapter.set_powered(true).await.unwrap();
    adapter.start_discovery().await.unwrap();

    assert_eq!(
        device_proxy(&conn, "11:22:33:44:55:66")
            .await
            .unwrap()
            .inner()
            .path()
            .as_str(),
        "/org/bluez/hci0/dev_11_22_33_44_55_66"
    );
    let dev = DeviceProxy::builder(&conn)
        .path(device_path(ADAPTER0, "11:22:33:44:55:66"))
        .unwrap()
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .unwrap();
    assert_eq!(dev.address().await.unwrap(), "11:22:33:44:55:66");
    assert_eq!(dev.address_type().await.unwrap(), "public");
    assert_eq!(dev.name().await.unwrap(), "Headset");
    assert_eq!(dev.adapter().await.unwrap().as_str(), ADAPTER0);
    assert_eq!(dev.rssi().await.unwrap(), -48, "RSSI property name");
    assert_eq!(dev.tx_power().await.unwrap(), 4);
    assert_eq!(dev.class().await.unwrap(), 0x240404);
    assert_eq!(dev.appearance().await.unwrap(), 0x0941);
    assert_eq!(dev.icon().await.unwrap(), "audio-headset");
    assert_eq!(dev.modalias().await.unwrap(), "bluetooth:v004Cp0001d0001");
    assert!(!dev.legacy_pairing().await.unwrap());
    assert_eq!(dev.uuids().await.unwrap().len(), 2, "UUIDs property name");
    assert_eq!(dev.advertising_flags().await.unwrap(), vec![0x06]);
    let md = dev.manufacturer_data().await.unwrap();
    assert_eq!(advertising_bytes(&md[&0x004C]), Some(vec![0x02, 0x15]));
    let sd = dev.service_data().await.unwrap();
    assert_eq!(advertising_bytes(&sd[SVC_UUID]), Some(vec![0x57]));
    let ad = dev.advertising_data().await.unwrap();
    assert_eq!(advertising_bytes(&ad[&0xFF]), Some(vec![0x4C, 0x00]));

    // A device without a name reports an error for Name, but has an alias.
    let anon = device_proxy(&conn, "aa:bb:cc:dd:ee:ff").await.unwrap();
    assert!(anon.name().await.is_err());
    assert_eq!(anon.alias().await.unwrap(), "AA-BB-CC-DD-EE-FF");

    dev.set_alias("My Headset").await.unwrap();
    assert_eq!(dev.alias().await.unwrap(), "My Headset");
    dev.set_trusted(true).await.unwrap();
    assert!(dev.trusted().await.unwrap());
    dev.set_wake_allowed(true).await.unwrap();
    assert!(dev.wake_allowed().await.unwrap());

    dev.set_blocked(true).await.unwrap();
    assert!(dev.blocked().await.unwrap());
    assert_eq!(
        err_name(&dev.connect().await.unwrap_err()),
        "org.bluez.Error.Failed"
    );
    dev.set_blocked(false).await.unwrap();

    assert!(!dev.connected().await.unwrap());
    dev.connect().await.unwrap();
    assert!(dev.connected().await.unwrap());
    assert!(dev.services_resolved().await.unwrap());

    dev.connect_profile(SVC_UUID).await.unwrap();
    dev.disconnect_profile(SVC_UUID).await.unwrap();
    let e = dev
        .connect_profile("00001234-0000-1000-8000-00805f9b34fb")
        .await
        .unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.NotAvailable");

    let battery = BatteryProxy::new(&conn, dev.inner().path().to_owned())
        .await
        .unwrap();
    assert_eq!(battery.percentage().await.unwrap(), 87);
    assert_eq!(battery.source().await.unwrap(), "GATT");

    assert_eq!(
        err_name(&dev.cancel_pairing().await.unwrap_err()),
        "org.bluez.Error.DoesNotExist"
    );

    dev.disconnect().await.unwrap();
    assert!(!dev.connected().await.unwrap());

    let snapshot = list_adapter_devices(&conn, ADAPTER0).await.unwrap();
    let headset = snapshot
        .iter()
        .find(|d| d.address == "11:22:33:44:55:66")
        .unwrap();
    assert_eq!(headset.alias, "My Headset");
    assert_eq!(headset.trusted, Some(true));
}

// ---------------------------------------------------------------------------
// Agent / pairing
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Recorder {
    confirmations: Arc<Mutex<Vec<(String, u32)>>>,
    reject: bool,
}

impl AgentHandler for Recorder {
    async fn request_confirmation(&self, device: OwnedObjectPath, passkey: u32) -> AgentResult<()> {
        // Handlers run on the caller's tokio runtime: tokio APIs work here.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        self.confirmations
            .lock()
            .unwrap()
            .push((device.to_string(), passkey));
        if self.reject {
            Err(AgentError::Rejected("user said no".into()))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn pairing_through_registered_agent() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    let adapter = adapter_proxy(&conn).await.unwrap();
    adapter.set_powered(true).await.unwrap();
    adapter.start_discovery().await.unwrap();

    let rec = Recorder::default();
    register_agent(
        &conn,
        "/bluebus/agent",
        AgentCapability::DisplayYesNo,
        rec.clone(),
        true,
    )
    .await
    .unwrap();
    {
        let st = bz.state.lock().unwrap();
        assert_eq!(st.agents.len(), 1);
        assert_eq!(st.agents[0].2, "DisplayYesNo");
        assert_eq!(
            st.default_agent.as_ref().unwrap().1.as_str(),
            "/bluebus/agent"
        );
    }

    let dev = device_proxy(&conn, "11:22:33:44:55:66").await.unwrap();
    dev.pair().await.unwrap();
    assert!(dev.paired().await.unwrap());
    assert!(dev.bonded().await.unwrap());
    assert_eq!(
        rec.confirmations.lock().unwrap().as_slice(),
        &[("/org/bluez/hci0/dev_11_22_33_44_55_66".to_string(), 123456)]
    );
    assert_eq!(
        err_name(&dev.pair().await.unwrap_err()),
        "org.bluez.Error.AlreadyExists"
    );

    // Registering a second agent at the same path is refused locally.
    assert!(register_agent(
        &conn,
        "/bluebus/agent",
        AgentCapability::NoInputNoOutput,
        Recorder::default(),
        false
    )
    .await
    .is_err());

    unregister_agent(&conn, "/bluebus/agent").await.unwrap();
    assert!(bz.state.lock().unwrap().agents.is_empty());
    assert!(bz.state.lock().unwrap().default_agent.is_none());

    // A rejecting agent makes pairing fail.
    let rejecting = Recorder {
        reject: true,
        ..Default::default()
    };
    register_agent(
        &conn,
        "/bluebus/agent",
        AgentCapability::KeyboardDisplay,
        rejecting.clone(),
        true,
    )
    .await
    .unwrap();
    let other = device_proxy(&conn, "AA:BB:CC:DD:EE:FF").await.unwrap();
    let e = other.pair().await.unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.AuthenticationRejected");
    assert!(!other.paired().await.unwrap());
    assert_eq!(rejecting.confirmations.lock().unwrap().len(), 1);
    unregister_agent(&conn, "/bluebus/agent").await.unwrap();

    // Unregistering twice reports BlueZ' error but still cleans up.
    assert_eq!(
        err_name(&unregister_agent(&conn, "/bluebus/agent").await.unwrap_err()),
        "org.bluez.Error.DoesNotExist"
    );
}

#[tokio::test]
async fn legacy_agent_struct_answers_bluez_calls() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    register_agent(
        &conn,
        "/legacy/agent",
        AgentCapability::KeyboardDisplay,
        Agent::new("0000".into(), 424242),
        true,
    )
    .await
    .unwrap();

    // Act as BlueZ: call every Agent1 method on the exported object.
    let owner = conn.unique_name().unwrap().to_string();
    let agent = zbus::Proxy::new(
        &bz.server,
        owner.as_str(),
        "/legacy/agent",
        "org.bluez.Agent1",
    )
    .await
    .unwrap();
    let dev = OwnedObjectPath::try_from("/org/bluez/hci0/dev_11_22_33_44_55_66").unwrap();

    let pin: String = agent.call("RequestPinCode", &(dev.clone(),)).await.unwrap();
    assert_eq!(pin, "0000");
    let passkey: u32 = agent.call("RequestPasskey", &(dev.clone(),)).await.unwrap();
    assert_eq!(passkey, 424242);
    let () = agent
        .call("DisplayPinCode", &(dev.clone(), "1234"))
        .await
        .unwrap();
    let () = agent
        .call("DisplayPasskey", &(dev.clone(), 1u32, 2u16))
        .await
        .unwrap();
    let () = agent
        .call("RequestConfirmation", &(dev.clone(), 5u32))
        .await
        .unwrap();
    let () = agent
        .call("RequestAuthorization", &(dev.clone(),))
        .await
        .unwrap();
    let () = agent
        .call("AuthorizeService", &(dev.clone(), SVC_UUID))
        .await
        .unwrap();
    let () = agent.call("Cancel", &()).await.unwrap();
    let () = agent.call("Release", &()).await.unwrap();

    // The inherent methods of the old API are still callable directly.
    let legacy = Agent::new("9999".into(), 1);
    assert_eq!(legacy.request_pin_code(&dev).await.unwrap(), "9999");
    legacy.request_confirmation(&dev, 1).unwrap();
}

#[tokio::test]
async fn default_agent_handler_rejects_secrets() {
    struct Defaults;
    impl AgentHandler for Defaults {}

    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    register_agent(
        &conn,
        "/d/agent",
        AgentCapability::NoInputNoOutput,
        Defaults,
        false,
    )
    .await
    .unwrap();
    let owner = conn.unique_name().unwrap().to_string();
    let agent = zbus::Proxy::new(&bz.server, owner.as_str(), "/d/agent", "org.bluez.Agent1")
        .await
        .unwrap();
    let dev = OwnedObjectPath::try_from("/org/bluez/hci0/dev_11_22_33_44_55_66").unwrap();
    let e = agent
        .call::<_, _, String>("RequestPinCode", &(dev.clone(),))
        .await
        .unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.Rejected");
    let () = agent.call("RequestAuthorization", &(dev,)).await.unwrap();
}

#[tokio::test]
async fn agent_registration_failure_is_rolled_back() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    register_agent(
        &conn,
        "/a/one",
        AgentCapability::NoInputNoOutput,
        Agent::new("0".into(), 0),
        false,
    )
    .await
    .unwrap();
    // The mock (like BlueZ) allows one agent per client: this one is refused…
    let e = register_agent(
        &conn,
        "/a/two",
        AgentCapability::NoInputNoOutput,
        Agent::new("0".into(), 0),
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.AlreadyExists");
    // …and its object was removed, so the path is free again.
    unregister_agent(&conn, "/a/one").await.unwrap();
    register_agent(
        &conn,
        "/a/two",
        AgentCapability::NoInputNoOutput,
        Agent::new("0".into(), 0),
        false,
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// GATT client
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gatt_client_read_write_notify() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    let adapter = adapter_proxy(&conn).await.unwrap();
    adapter.set_powered(true).await.unwrap();
    adapter.start_discovery().await.unwrap();
    let dev = device_proxy(&conn, "11:22:33:44:55:66").await.unwrap();
    dev.connect().await.unwrap();

    let dev_path = dev.inner().path().to_string();
    let db = discover_gatt_database(&conn, &dev_path).await.unwrap();
    assert_eq!(db.services.len(), 1);
    assert_eq!(db.characteristics.len(), 1);
    assert_eq!(db.descriptors.len(), 1);
    assert_eq!(db.services[0].uuid, SVC_UUID);
    assert_eq!(db.services[0].parent, dev_path);
    let chr_attr = db.characteristic_by_uuid(&CHR_UUID.to_uppercase()).unwrap();
    assert_eq!(chr_attr.flags, vec!["read", "write", "notify"]);
    assert_eq!(chr_attr.parent, db.services[0].path);
    assert_eq!(db.descriptors[0].parent, chr_attr.path);

    // Service proxy
    let svc = gatt_service_proxy(&conn, db.services[0].path.as_str())
        .await
        .unwrap();
    assert_eq!(svc.uuid().await.unwrap(), SVC_UUID, "UUID property name");
    assert!(svc.primary().await.unwrap());
    assert_eq!(svc.device_path().await.unwrap().as_str(), dev_path);
    assert!(svc.include_paths().await.unwrap().is_empty());
    assert_eq!(Option::<u16>::from(svc.handle().await.unwrap()), Some(1));

    // Characteristic proxy
    let chr = gatt_characteristic_proxy(&conn, chr_attr.path.clone())
        .await
        .unwrap();
    assert_eq!(chr.uuid().await.unwrap(), CHR_UUID);
    assert_eq!(
        chr.service_path().await.unwrap().as_str(),
        db.services[0].path
    );
    assert_eq!(chr.flags().await.unwrap(), vec!["read", "write", "notify"]);
    assert_eq!(
        Option::<u16>::from(chr.mtu().await.unwrap()),
        Some(247),
        "MTU property name"
    );
    assert_eq!(Option::<u16>::from(chr.handle().await.unwrap()), Some(2));
    assert_eq!(chr.read_value(HashMap::new()).await.unwrap(), vec![87]);

    chr.write_value(
        vec![1, 2, 3],
        HashMap::from([(
            "type".to_string(),
            OwnedValue::from(zbus::zvariant::Str::from("request")),
        )]),
    )
    .await
    .unwrap();
    let offset = HashMap::from([("offset".to_string(), OwnedValue::from(1u16))]);
    assert_eq!(chr.read_value(offset).await.unwrap(), vec![2, 3]);

    // Notifications
    let mut notes = characteristic_notifications(&chr).await.unwrap();
    assert!(
        timeout("notifying", async {
            loop {
                if chr.notifying().await.unwrap() {
                    break true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
    );
    remote_notify(&bz.server, &chr_attr.path, vec![42]).await;
    remote_notify(&bz.server, &chr_attr.path, vec![43]).await;
    // Every notification arrives, in order, and nothing else.
    assert_eq!(
        timeout("notification 1", notes.next()).await.unwrap(),
        vec![42]
    );
    assert_eq!(
        timeout("notification 2", notes.next()).await.unwrap(),
        vec![43]
    );
    chr.stop_notify().await.unwrap();

    // Descriptor proxy
    let desc = gatt_descriptor_proxy(&conn, db.descriptors[0].path.as_str())
        .await
        .unwrap();
    assert_eq!(desc.uuid().await.unwrap(), DESC_UUID);
    assert_eq!(desc.characteristic().await.unwrap().as_str(), chr_attr.path);
    assert_eq!(desc.flags().await.unwrap(), vec!["read", "write"]);
    assert_eq!(desc.handle().await.unwrap(), 3);
    desc.write_value(vec![1, 0], HashMap::new()).await.unwrap();
    assert_eq!(desc.read_value(HashMap::new()).await.unwrap(), vec![1, 0]);
}

// ---------------------------------------------------------------------------
// GATT server
// ---------------------------------------------------------------------------

const APP: &str = "/test/gatt_app";

async fn export_app(conn: &zbus::Connection) -> (String, String, String) {
    let svc = format!("{APP}/service0");
    let chr = format!("{svc}/char0");
    let desc = format!("{chr}/desc0");
    register_service(
        conn,
        &svc,
        GattService {
            uuid: SVC_UUID.into(),
            primary: true,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await
    .unwrap();
    register_characteristic(
        conn,
        &chr,
        GattCharacteristic {
            uuid: CHR_UUID.into(),
            service: svc.clone(),
            value: Some(b"hello".to_vec()),
            write_acquired: false,
            notify_acquired: false,
            notifying: false,
            flags: vec!["read".into(), "write".into(), "notify".into()],
            handle: None,
            mtu: None,
        },
    )
    .await
    .unwrap();
    register_descriptor(
        conn,
        &desc,
        GattDescriptor {
            uuid: "2901".into(),
            characteristic: chr.clone(),
            value: b"Level".to_vec(),
            flags: vec!["read".into()],
            handle: None,
        },
    )
    .await
    .unwrap();
    (svc, chr, desc)
}

#[tokio::test]
async fn gatt_server_application_lifecycle() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    let (svc, chr, desc) = export_app(&conn).await;

    type Writes = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
    let writes: Writes = Arc::default();
    {
        let writes = writes.clone();
        set_write_handler(&chr, move |path, value| {
            writes
                .lock()
                .unwrap()
                .push((path.to_owned(), value.to_vec()))
        });
    }

    // BlueZ validates the tree it reads through ObjectManager.
    register_gatt_application_at(&conn, ADAPTER0, APP)
        .await
        .unwrap();
    {
        let st = bz.state.lock().unwrap();
        let objects = st.gatt_apps.values().next().unwrap();
        let svc_props =
            &objects.iter().find(|(p, _)| p.as_str() == svc).unwrap().1["org.bluez.GattService1"];
        assert!(svc_props.contains_key("UUID") && svc_props.contains_key("Primary"));
        assert!(!svc_props.contains_key("Handle"), "unset Handle is omitted");
        let chr_props = &objects.iter().find(|(p, _)| p.as_str() == chr).unwrap().1
            ["org.bluez.GattCharacteristic1"];
        assert_eq!(chr_props["Service"].value_signature().to_string(), "o");
        assert_eq!(chr_props["Value"].value_signature().to_string(), "ay");
        assert!(!chr_props.contains_key("WriteAcquired"));
        assert!(objects
            .iter()
            .any(|(p, i)| p.as_str() == desc && i.contains_key("org.bluez.GattDescriptor1")));
    }
    // Registering twice is reported by BlueZ.
    assert_eq!(
        err_name(
            &register_gatt_application_at(&conn, ADAPTER0, APP)
                .await
                .unwrap_err()
        ),
        "org.bluez.Error.AlreadyExists"
    );

    // Act as a remote central through BlueZ.
    let owner = conn.unique_name().unwrap().to_string();
    let remote = zbus::Proxy::new(
        &bz.server,
        owner.as_str(),
        chr.as_str(),
        "org.bluez.GattCharacteristic1",
    )
    .await
    .unwrap();
    let v: Vec<u8> = remote
        .call("ReadValue", &(HashMap::<String, OwnedValue>::new(),))
        .await
        .unwrap();
    assert_eq!(v, b"hello");
    let v: Vec<u8> = remote
        .call(
            "ReadValue",
            &(HashMap::from([(
                "offset".to_string(),
                OwnedValue::from(3u16),
            )]),),
        )
        .await
        .unwrap();
    assert_eq!(v, b"lo");
    let e = remote
        .call::<_, _, Vec<u8>>(
            "ReadValue",
            &(HashMap::from([(
                "offset".to_string(),
                OwnedValue::from(9u16),
            )]),),
        )
        .await
        .unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.InvalidOffset");

    let () = remote
        .call(
            "WriteValue",
            &(b"abc".to_vec(), HashMap::<String, OwnedValue>::new()),
        )
        .await
        .unwrap();
    let () = remote
        .call(
            "WriteValue",
            &(
                b"Z".to_vec(),
                HashMap::from([("offset".to_string(), OwnedValue::from(1u16))]),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        local_characteristic_value(&conn, &chr)
            .await
            .unwrap()
            .unwrap(),
        b"aZ"
    );
    assert_eq!(
        *writes.lock().unwrap(),
        vec![
            (chr.clone(), b"abc".to_vec()),
            (chr.clone(), b"aZ".to_vec())
        ]
    );
    remove_write_handler(&chr);

    // BlueZ assigns handles through Properties.Set.
    let props = zbus::fdo::PropertiesProxy::builder(&bz.server)
        .destination(owner.as_str())
        .unwrap()
        .path(chr.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let iface = zbus::names::InterfaceName::try_from("org.bluez.GattCharacteristic1").unwrap();
    props
        .set(
            iface.clone(),
            "Handle",
            zbus::zvariant::Value::from(0x002au16),
        )
        .await
        .unwrap();
    assert_eq!(
        u16::try_from(props.get(iface.clone(), "Handle").await.unwrap()).unwrap(),
        0x2a
    );

    // Notifications reach BlueZ as PropertiesChanged on Value.
    let mut changes = props.receive_properties_changed().await.unwrap();
    let () = remote.call("StartNotify", &()).await.unwrap();
    notify_characteristic_value(&conn, &chr, vec![7, 7])
        .await
        .unwrap();
    let got = timeout("Value notification", async {
        while let Some(sig) = changes.next().await {
            let args = sig.args().unwrap();
            if let Some(v) = args.changed_properties().get("Value") {
                return Vec::<u8>::try_from(v.try_to_owned().unwrap()).unwrap();
            }
        }
        unreachable!()
    })
    .await;
    assert_eq!(got, vec![7, 7]);
    let () = remote.call("StopNotify", &()).await.unwrap();

    let remote_desc = zbus::Proxy::new(
        &bz.server,
        owner.as_str(),
        desc.as_str(),
        "org.bluez.GattDescriptor1",
    )
    .await
    .unwrap();
    let v: Vec<u8> = remote_desc
        .call("ReadValue", &(HashMap::<String, OwnedValue>::new(),))
        .await
        .unwrap();
    assert_eq!(v, b"Level");

    unregister_gatt_application_at(&conn, ADAPTER0, APP)
        .await
        .unwrap();
    assert!(bz.state.lock().unwrap().gatt_apps.is_empty());
    // Objects are kept, so the app can be registered again.
    register_gatt_application_at(&conn, ADAPTER0, APP)
        .await
        .unwrap();
}

#[tokio::test]
async fn gatt_server_rejects_invalid_tree() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    // A characteristic pointing to a service that does not exist.
    register_characteristic(
        &conn,
        &format!("{APP}/service9/char0"),
        GattCharacteristic::new(CHR_UUID, format!("{APP}/service9"), &["read"]),
    )
    .await
    .unwrap();
    register_service(
        &conn,
        &format!("{APP}/service0"),
        GattService {
            uuid: SVC_UUID.into(),
            primary: true,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await
    .unwrap();
    let e = register_gatt_application_at(&conn, ADAPTER0, APP)
        .await
        .unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.InvalidArguments");
}

// ---------------------------------------------------------------------------
// LE advertising
// ---------------------------------------------------------------------------

fn adv_props(bz: &MockBluez) -> HashMap<String, OwnedValue> {
    let st = bz.state.lock().unwrap();
    assert_eq!(st.advertisements.len(), 1);
    st.advertisements
        .values()
        .next()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
        .collect()
}

#[tokio::test]
async fn advertisement_register_unregister_reregister() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    let path = "/test/adv0";

    let adv = LEAdvertisementProperties::new(AdvertisementType::Peripheral)
        .with_local_name("bluebus")
        .with_service_uuids(["180D"])
        .with_manufacturer_data(0xFFFF, vec![1, 2, 3])
        .with_service_data("180D", vec![9])
        .with_discoverable(true)
        .with_includes(["tx-power"]);
    register_advertisement_at(&conn, ADAPTER0, path, adv)
        .await
        .unwrap();

    let props = adv_props(&bz);
    let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
    keys.sort();
    // Exact BlueZ names, and nothing that was not set.
    assert_eq!(
        keys,
        [
            "Discoverable",
            "Includes",
            "LocalName",
            "ManufacturerData",
            "ServiceData",
            "ServiceUUIDs",
            "Type"
        ]
    );
    assert_eq!(
        String::try_from(props["Type"].try_clone().unwrap()).unwrap(),
        "peripheral"
    );

    let mgr = LEAdvertisingManager1Proxy::new(&conn, ADAPTER0)
        .await
        .unwrap();
    assert_eq!(mgr.active_instances().await.unwrap(), 1);
    assert_eq!(mgr.supported_instances().await.unwrap(), 4);
    assert!(mgr
        .supported_includes()
        .await
        .unwrap()
        .contains(&"tx-power".to_string()));
    assert_eq!(mgr.supported_secondary_channels().await.unwrap().len(), 3);
    assert_eq!(
        mgr.supported_features().await.unwrap(),
        vec!["CanSetTxPower"]
    );

    unregister_advertisement_at(&conn, ADAPTER0, path)
        .await
        .unwrap();
    assert!(bz.state.lock().unwrap().advertisements.is_empty());
    // Unregistering removed the object too: registering again works.
    register_advertisement_at(
        &conn,
        ADAPTER0,
        path,
        LEAdvertisementProperties::new(AdvertisementType::Broadcast),
    )
    .await
    .unwrap();
    assert_eq!(adv_props(&bz).len(), 1, "only Type");
    // Registering over an active advertisement replaces it.
    register_advertisement_at(
        &conn,
        ADAPTER0,
        path,
        LEAdvertisementProperties::new(AdvertisementType::Peripheral).with_local_name("v2"),
    )
    .await
    .unwrap();
    assert_eq!(
        String::try_from(adv_props(&bz)["LocalName"].try_clone().unwrap()).unwrap(),
        "v2"
    );
    unregister_advertisement_at(&conn, ADAPTER0, path)
        .await
        .unwrap();
    assert_eq!(
        err_name(
            &unregister_advertisement_at(&conn, ADAPTER0, path)
                .await
                .unwrap_err()
        ),
        "org.bluez.Error.DoesNotExist"
    );
}

#[tokio::test]
async fn default_advertisement_and_global_paths() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    // The historical default advertisement passes BlueZ validation.
    register_advertisement(&conn, LEAdvertisementProperties::default())
        .await
        .unwrap();
    let props = adv_props(&bz);
    assert_eq!(
        bz.state
            .lock()
            .unwrap()
            .advertisements
            .keys()
            .next()
            .unwrap()
            .1,
        get_advertisement_path()
    );
    for key in [
        "ServiceUUIDs",
        "SecondaryChannel",
        "MinInterval",
        "MaxInterval",
        "TxPower",
        "Appearance",
        "Timeout",
        "Duration",
    ] {
        assert!(props.contains_key(key), "{key} missing");
    }
    unregister_advertisement(&conn).await.unwrap();
    assert!(bz.state.lock().unwrap().advertisements.is_empty());
}

#[tokio::test]
async fn rejected_advertisement_is_cleaned_up() {
    let bz = MockBluez::start().await;
    let conn = bz.client().await;
    let path = "/test/adv_bad";
    let bad =
        LEAdvertisementProperties::new(AdvertisementType::Peripheral).with_secondary_channel("3M");
    let e = register_advertisement_at(&conn, ADAPTER0, path, bad)
        .await
        .unwrap_err();
    assert_eq!(err_name(&e), "org.bluez.Error.InvalidArguments");
    let bad = LEAdvertisementProperties::new(AdvertisementType::Peripheral).with_interval(200, 100);
    assert!(register_advertisement_at(&conn, ADAPTER0, path, bad)
        .await
        .is_err());
    register_advertisement_at(
        &conn,
        ADAPTER0,
        path,
        LEAdvertisementProperties::new(AdvertisementType::Peripheral),
    )
    .await
    .unwrap();
    // Path validation
    assert!(register_advertisement_at(
        &conn,
        ADAPTER0,
        "not a path",
        LEAdvertisementProperties::default()
    )
    .await
    .is_err());
}

// ---------------------------------------------------------------------------
// Monitor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn monitor_reports_add_change_remove() {
    let bz = MockBluez::start().await;
    let conn = Arc::new(bz.client().await);
    let manager = Arc::new(ObjectManagerProxy::new(&conn).await.unwrap());
    let (added_tx, mut added) = mpsc::channel(16);
    let (removed_tx, mut removed) = mpsc::channel(16);
    let (changed_tx, mut changed) = mpsc::channel(16);
    let monitor =
        Arc::new(Monitor::new(conn.clone(), manager, added_tx, removed_tx, changed_tx).await);
    let runner = tokio::spawn({
        let m = monitor.clone();
        async move { m.run().await }
    });

    // The pre-existing device of hci1 is seeded silently.
    timeout("seeding", async {
        while monitor.devices().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(monitor
        .devices()
        .await
        .contains_key("/org/bluez/hci1/dev_01_02_03_04_05_06"));

    let adapter = adapter_proxy(&conn).await.unwrap();
    adapter.set_powered(true).await.unwrap();
    adapter.start_discovery().await.unwrap();
    let mut seen = vec![
        timeout("added #1", added.recv()).await.unwrap().address,
        timeout("added #2", added.recv()).await.unwrap().address,
    ];
    seen.sort();
    assert_eq!(seen, ["11:22:33:44:55:66", "AA:BB:CC:DD:EE:FF"]);

    // Property changes of a pre-existing device are reported (needs seeding).
    let old = DeviceProxy::new(&conn, "/org/bluez/hci1/dev_01_02_03_04_05_06")
        .await
        .unwrap();
    old.set_trusted(true).await.unwrap();
    let d = timeout("changed (seeded device)", changed.recv())
        .await
        .unwrap();
    assert_eq!(
        (d.address.as_str(), d.trusted),
        ("01:02:03:04:05:06", Some(true))
    );

    // Several changes in a row: the 0.1.10 watcher dead-locked on the first.
    let dev = device_proxy(&conn, "11:22:33:44:55:66").await.unwrap();
    dev.set_alias("Renamed").await.unwrap();
    let d = timeout("changed alias", changed.recv()).await.unwrap();
    assert_eq!(d.alias, "Renamed");
    dev.connect().await.unwrap();
    let d = timeout("changed connected", changed.recv()).await.unwrap();
    assert!(d.connected);
    dev.set_trusted(true).await.unwrap();
    let d = timeout("changed trusted", changed.recv()).await.unwrap();
    assert_eq!(d.trusted, Some(true));

    // Removing only Battery1 from a device must not report the device as removed.
    bz.server
        .object_server()
        .remove::<MockBattery, _>(dev.inner().path().as_str())
        .await
        .unwrap();
    adapter
        .remove_device(OwnedObjectPath::from(
            ObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap(),
        ))
        .await
        .unwrap();
    let d = timeout("removed", removed.recv()).await.unwrap();
    assert_eq!(
        d.address, "AA:BB:CC:DD:EE:FF",
        "Battery1 removal was misreported as device removal"
    );

    // Removing the connected device (with GATT children) reports it once.
    adapter
        .remove_device(OwnedObjectPath::from(dev.inner().path().to_owned()))
        .await
        .unwrap();
    let d = timeout("removed #2", removed.recv()).await.unwrap();
    assert_eq!(d.alias, "Renamed");
    assert!(!monitor
        .devices()
        .await
        .contains_key(dev.inner().path().as_str()));
    assert!(removed.try_recv().is_err());

    runner.abort();
}

#[tokio::test]
async fn monitor_returns_when_bus_goes_away() {
    let bz = MockBluez::start().await;
    let conn = Arc::new(bz.client().await);
    let manager = Arc::new(ObjectManagerProxy::new(&conn).await.unwrap());
    let (tx, _rx) = mpsc::channel(1);
    let monitor = Monitor::new(conn, manager, tx.clone(), tx.clone(), tx).await;
    let run = monitor.run();
    drop(bz); // kills the bus daemon
              // 0.1.10 spun forever on a closed stream; now `run` finishes.
    timeout("monitor to stop", run).await;
}
