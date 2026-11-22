//! Tests of the high-level `bluebus::ble` API against the mock BlueZ daemon
//! on a private D-Bus bus (needs `dbus-daemon`).

mod common;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bluebus::ble::{
    normalize_uuid, uuid_from_u16, Advertisement, AgentCapability, AgentError, AgentHandler,
    AgentResult, AutoAcceptAgent, Characteristic, Error, PasskeyAgent, ScanEvent, ScanFilter,
    Service, Session, Transport, WriteType,
};
use common::*;
use futures::StreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const HEADSET: &str = "11:22:33:44:55:66";
const ANON: &str = "AA:BB:CC:DD:EE:FF";

async fn session(bz: &MockBluez) -> Session {
    Session::with_connection(bz.client().await)
}

/// Polls `check` until it returns true (fails the test after 5 s).
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    timeout(what, async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
}

fn dbus_name(e: &Error) -> String {
    e.dbus_error_name().unwrap_or_else(|| e.to_string())
}

/// Powered hci0 with both nearby devices discovered.
async fn discovered(bz: &MockBluez) -> (Session, bluebus::ble::Adapter) {
    let s = session(bz).await;
    let adapter = s.default_adapter().await.unwrap();
    adapter.power_on().await.unwrap();
    let found = adapter
        .scan(ScanFilter::new(), Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    (s, adapter)
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_lists_and_opens_adapters() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    assert!(s.connection().unique_name().is_some());
    assert!(format!("{s:?}").contains("Session"));

    let adapters = s.adapters().await.unwrap();
    let names: Vec<&str> = adapters.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["hci0", "hci1"]);
    assert_eq!(adapters[0].path, ADAPTER0);
    assert_eq!(adapters[0].address, "00:AA:00:AA:00:AA");
    assert_eq!(adapters[0].alias, "mockhost");
    assert!(!adapters[0].powered && !adapters[0].discovering);

    let default = s.default_adapter().await.unwrap();
    assert_eq!(default.name(), "hci0");
    assert_eq!(s.adapter(None).await.unwrap().path(), ADAPTER0);
    for key in ["hci1", ADAPTER1, "00:bb:00:bb:00:bb"] {
        let a = s.adapter(Some(key)).await.unwrap();
        assert_eq!(
            (a.name(), a.path(), a.address()),
            ("hci1", ADAPTER1, "00:BB:00:BB:00:BB")
        );
    }
    assert!(format!("{default:?}").contains("hci0"));
    assert!(default.session().connection().unique_name().is_some());
    let e = s.adapter(Some("hci9")).await.unwrap_err();
    assert!(
        matches!(&e, Error::NotFound(m) if m.contains("hci9")),
        "{e}"
    );
}

#[tokio::test]
async fn agents_answer_pairing() {
    let bz = MockBluez::start().await;
    let (s, adapter) = discovered(&bz).await;
    let device = adapter.device(HEADSET).unwrap();

    // Without an agent the mock pairs "Just Works".
    s.remove_agent().await.unwrap();

    struct Refuse;
    impl AgentHandler for Refuse {
        async fn request_confirmation(&self, _: OwnedObjectPath, _: u32) -> AgentResult<()> {
            Err(AgentError::Rejected("no".into()))
        }
    }
    s.set_agent(AgentCapability::DisplayYesNo, Refuse)
        .await
        .unwrap();
    {
        let st = bz.state.lock().unwrap();
        assert_eq!(st.agents.len(), 1);
        assert_eq!(st.agents[0].2, "DisplayYesNo");
        assert!(st.default_agent.is_some());
    }
    let e = device.pair().await.unwrap_err();
    assert_eq!(dbus_name(&e), "org.bluez.Error.AuthenticationRejected");
    assert!(!device.info().await.unwrap().paired);

    // Replacing the agent unregisters the previous one.
    s.set_agent(AgentCapability::NoInputNoOutput, AutoAcceptAgent)
        .await
        .unwrap();
    assert_eq!(bz.state.lock().unwrap().agents.len(), 1);
    device.pair().await.unwrap();
    assert!(device.info().await.unwrap().paired);
    assert_eq!(bz.state.lock().unwrap().pair_requests.len(), 2);
    // Already paired: no new request.
    device.pair().await.unwrap();
    assert_eq!(bz.state.lock().unwrap().pair_requests.len(), 2);

    s.set_agent(AgentCapability::KeyboardDisplay, PasskeyAgent::new(123456))
        .await
        .unwrap();
    assert_eq!(bz.state.lock().unwrap().agents[0].2, "KeyboardDisplay");
    s.remove_agent().await.unwrap();
    assert!(bz.state.lock().unwrap().agents.is_empty());
    assert!(bz.state.lock().unwrap().default_agent.is_none());
    s.remove_agent().await.unwrap();
}

#[tokio::test]
async fn agent_helpers_answer_requests() {
    let dev = OwnedObjectPath::try_from("/org/bluez/hci0/dev_11_22_33_44_55_66").unwrap();
    let agent = PasskeyAgent::new(42);
    assert_eq!(agent.passkey(), 42);
    assert_eq!(agent.pin_code(), "000042");
    assert_eq!(agent.request_passkey(dev.clone()).await.unwrap(), 42);
    assert_eq!(agent.request_pin_code(dev.clone()).await.unwrap(), "000042");
    assert!(agent.request_confirmation(dev.clone(), 1).await.is_ok());
    let agent = agent.with_pin_code("1234");
    assert_eq!(agent.request_pin_code(dev.clone()).await.unwrap(), "1234");

    let auto = AutoAcceptAgent;
    assert!(auto.request_confirmation(dev.clone(), 1).await.is_ok());
    assert!(auto.request_authorization(dev.clone()).await.is_ok());
    assert!(auto
        .authorize_service(dev.clone(), "180f".into())
        .await
        .is_ok());
    assert!(auto.request_passkey(dev).await.is_err());
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adapter_power() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();
    assert!(!adapter.is_powered().await.unwrap());
    adapter.power_on().await.unwrap();
    assert!(adapter.is_powered().await.unwrap());
    assert!(adapter.info().await.unwrap().powered);
    adapter.power_on().await.unwrap(); // already on
    adapter.power_off().await.unwrap();
    assert!(!adapter.is_powered().await.unwrap());

    // Scanning needs power, like BlueZ.
    let e = adapter.scan_stream(ScanFilter::new()).await.unwrap_err();
    assert_eq!(dbus_name(&e), "org.bluez.Error.NotReady");
    assert!(bz
        .state
        .lock()
        .unwrap()
        .discovery_filter
        .as_ref()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn scan_collects_matching_devices() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();
    adapter.power_on().await.unwrap();

    let found = adapter
        .scan(ScanFilter::new(), Duration::from_millis(100))
        .await
        .unwrap();
    let addresses: Vec<&str> = found.iter().map(|d| d.address.as_str()).collect();
    assert_eq!(addresses, [HEADSET, ANON]);
    let headset = &found[0];
    assert_eq!(headset.name.as_deref(), Some("Headset"));
    assert_eq!(headset.alias, "Headset");
    assert_eq!(headset.address_type.as_deref(), Some("public"));
    assert_eq!(headset.rssi, Some(-48));
    assert_eq!(headset.tx_power, Some(4));
    assert!(headset.uuids.contains(&SVC_UUID.to_string()));
    assert_eq!(headset.manufacturer_data[&0x004C], vec![0x02, 0x15]);
    assert_eq!(headset.service_data[SVC_UUID], vec![0x57]);
    assert!(!headset.connected && !headset.paired && !headset.trusted);
    assert_eq!(found[1].name, None);
    assert!(!adapter.info().await.unwrap().discovering, "scan stopped");

    // BlueZ-side filter keys plus local filtering.
    let filter = ScanFilter::new()
        .transport(Transport::Le)
        .service_uuid("0x180F")
        .min_rssi(-60)
        .duplicate_data(true);
    let mut events = adapter.scan_stream(filter.clone()).await.unwrap();
    {
        let st = bz.state.lock().unwrap();
        let mut keys: Vec<&str> = st
            .discovery_filter
            .as_ref()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["DuplicateData", "RSSI", "Transport", "UUIDs"]);
        let uuids: Vec<String> = st.discovery_filter.as_ref().unwrap()["UUIDs"]
            .try_clone()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(uuids, [SVC_UUID]);
    }
    let only = timeout("filtered Found", events.next()).await.unwrap();
    assert_eq!(only.device().address, HEADSET);
    events.stop().await.unwrap();
    assert!(bz
        .state
        .lock()
        .unwrap()
        .discovery_filter
        .as_ref()
        .unwrap()
        .is_empty());
    let found = adapter
        .scan(filter, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].address, HEADSET);

    for (i, filter) in [
        ScanFilter::new().name("Headset"),
        ScanFilter::new().name_prefix("Head"),
        ScanFilter::new().address("11:22:33:44:55:66"),
        // Every mock device advertises 0x180F.
        ScanFilter::new()
            .service_uuids(["180d", "180f"])
            .min_rssi(-50),
    ]
    .into_iter()
    .enumerate()
    {
        let found = adapter
            .scan(filter.clone(), Duration::from_millis(50))
            .await
            .unwrap();
        assert_eq!(found.len(), 1, "filter #{i}");
        assert!(filter.matches(&found[0]));
    }
    let both = adapter
        .scan(
            ScanFilter::new().service_uuid("180f"),
            Duration::from_millis(50),
        )
        .await
        .unwrap();
    assert_eq!(both.len(), 2);
    let none = adapter
        .scan(
            ScanFilter::new().service_uuid("180d"),
            Duration::from_millis(50),
        )
        .await
        .unwrap();
    assert!(none.is_empty());
    let none = adapter
        .scan(ScanFilter::new().name("Nobody"), Duration::from_millis(50))
        .await
        .unwrap();
    assert!(none.is_empty());

    let e = adapter
        .scan(ScanFilter::new().service_uuid("heart"), Duration::ZERO)
        .await
        .unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "{e}");
    let e = adapter
        .scan(ScanFilter::new().address("nope"), Duration::ZERO)
        .await
        .unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "{e}");
}

#[tokio::test]
async fn scan_stream_events_and_guard() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();
    adapter.power_on().await.unwrap();

    let mut events = adapter
        .scan_stream(ScanFilter::new().transport(Transport::Le))
        .await
        .unwrap();
    assert!(format!("{events:?}").contains("ScanStream"));
    {
        let st = bz.state.lock().unwrap();
        let f = st.discovery_filter.as_ref().unwrap();
        assert_eq!(
            String::try_from(f["Transport"].try_clone().unwrap()).unwrap(),
            "le"
        );
    }
    let mut found = Vec::new();
    for _ in 0..2 {
        match timeout("Found", events.next()).await.unwrap() {
            ScanEvent::Found(d) => found.push(d.address),
            other => panic!("unexpected {other:?}"),
        }
    }
    found.sort();
    assert_eq!(found, [HEADSET, ANON]);
    assert!(adapter.info().await.unwrap().discovering);

    set_device_rssi(&bz.server, &bluebus::device_path(ADAPTER0, HEADSET), -30).await;
    let event = timeout("Updated", events.next()).await.unwrap();
    assert!(matches!(&event, ScanEvent::Updated(_)));
    assert_eq!(event.device().rssi, Some(-30));
    assert_eq!(event.clone().into_device().address, HEADSET);

    // Dropping the stream stops discovery and resets the filter.
    drop(events);
    eventually("discovery stopped on drop", || async {
        !adapter.info().await.unwrap().discovering
    })
    .await;
    assert!(bz
        .state
        .lock()
        .unwrap()
        .discovery_filter
        .as_ref()
        .unwrap()
        .is_empty());

    // Drop + immediate restart: the pending stop cannot cancel the new scan.
    let events = adapter.scan_stream(ScanFilter::new()).await.unwrap();
    drop(events);
    let mut events = adapter.scan_stream(ScanFilter::new()).await.unwrap();
    // Devices already seen by the running discovery are reported at once.
    let event = timeout("initial Found", events.next()).await.unwrap();
    assert!(matches!(event, ScanEvent::Found(_)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(adapter.info().await.unwrap().discovering);
    events.stop().await.unwrap();
    assert!(!adapter.info().await.unwrap().discovering);

    // A second concurrent scan on the same session is refused by BlueZ.
    let first = adapter.scan_stream(ScanFilter::new()).await.unwrap();
    let e = adapter.scan_stream(ScanFilter::new()).await.unwrap_err();
    assert_eq!(dbus_name(&e), "org.bluez.Error.InProgress");
    first.stop().await.unwrap();
}

#[tokio::test]
async fn find_device_and_known_devices() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();
    adapter.power_on().await.unwrap();

    let device = adapter
        .find_device(ScanFilter::new().name("Headset"), Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(device.address(), HEADSET);
    assert!(!adapter.info().await.unwrap().discovering);

    let e = adapter
        .find_device(ScanFilter::new().name("Nobody"), Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Timeout(_)), "{e}");
    assert!(!adapter.info().await.unwrap().discovering);

    // A connected device is returned without scanning.
    device.connect(Duration::from_secs(2)).await.unwrap();
    adapter.power_off().await.unwrap(); // a scan would fail now
    let again = adapter
        .find_device(
            ScanFilter::new().address(HEADSET.to_lowercase()),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(again.address(), HEADSET);

    let known = adapter.devices().await.unwrap();
    assert_eq!(known.len(), 2);
    assert!(known[0].connected);
    let other = s
        .adapter(Some("hci1"))
        .await
        .unwrap()
        .devices()
        .await
        .unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].name.as_deref(), Some("Other"));

    adapter.remove_device(&ANON.to_lowercase()).await.unwrap();
    assert_eq!(adapter.devices().await.unwrap().len(), 1);
    let e = adapter.remove_device(ANON).await.unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e}");
    let e = adapter.remove_device("garbage").await.unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "{e}");
}

// ---------------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------------

#[tokio::test]
async fn device_handles_and_link() {
    let bz = MockBluez::start().await;
    let (_s, adapter) = discovered(&bz).await;

    assert!(matches!(
        adapter.device("not-an-address"),
        Err(Error::InvalidArgument(_))
    ));
    let unknown = adapter.device("01:02:03:04:05:07").unwrap();
    assert!(matches!(unknown.info().await, Err(Error::NotFound(_))));
    assert!(matches!(
        unknown.connect(Duration::from_secs(1)).await,
        Err(Error::NotFound(_))
    ));

    let device = adapter.device("11-22-33-44-55-66").unwrap();
    assert_eq!(device.address(), HEADSET);
    assert_eq!(device.path(), bluebus::device_path(ADAPTER0, HEADSET));
    assert_eq!(device.adapter().name(), "hci0");
    assert!(format!("{device:?}").contains(HEADSET));
    assert_eq!(device.rssi().await.unwrap(), Some(-48));
    assert!(!device.is_connected().await.unwrap());

    device.disconnect().await.unwrap(); // not connected: no-op
    device.connect(Duration::from_secs(2)).await.unwrap();
    let info = device.info().await.unwrap();
    assert!(info.connected && info.services_resolved);
    assert!(device.is_connected().await.unwrap());
    device.connect(Duration::from_secs(2)).await.unwrap(); // already connected

    device.trust(true).await.unwrap();
    assert!(device.info().await.unwrap().trusted);
    device.trust(false).await.unwrap();
    assert!(!device.info().await.unwrap().trusted);

    device.disconnect().await.unwrap();
    assert!(!device.is_connected().await.unwrap());

    // Connecting a blocked device fails with BlueZ' error.
    let proxy = bluebus::DeviceProxy::new(adapter.session().connection(), device.path().to_owned())
        .await
        .unwrap();
    proxy.set_blocked(true).await.unwrap();
    let e = device.connect(Duration::from_secs(2)).await.unwrap_err();
    assert_eq!(dbus_name(&e), "org.bluez.Error.Failed");
}

#[tokio::test]
async fn device_gatt_client() {
    let bz = MockBluez::start().await;
    let (_s, adapter) = discovered(&bz).await;
    let device = adapter.device(HEADSET).unwrap();
    assert!(
        matches!(device.read("2a19").await, Err(Error::NotFound(_))),
        "no services before connecting"
    );
    device.connect(Duration::from_secs(2)).await.unwrap();
    let dev_path = device.path().to_owned();
    let chr_path = format!("{dev_path}/service0001/char0002");

    let services = device.services().await.unwrap();
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].uuid, SVC_UUID);
    assert_eq!(services[0].path, format!("{dev_path}/service0001"));
    let chr = &services[0].characteristics[0];
    assert_eq!(chr.uuid, CHR_UUID);
    assert_eq!(chr.path, chr_path);
    assert_eq!(chr.flags, ["read", "write", "notify"]);
    assert_eq!(chr.descriptors[0].uuid, DESC_UUID);
    assert_eq!(chr.descriptors[0].flags, ["read", "write"]);

    // Every UUID spelling resolves to the same characteristic.
    for (i, uuid) in ["2a19", "0x2A19", CHR_UUID, &CHR_UUID.to_uppercase()]
        .into_iter()
        .enumerate()
    {
        assert_eq!(device.read(uuid).await.unwrap(), vec![87], "spelling #{i}");
    }
    assert_eq!(device.battery_level().await.unwrap(), 87, "Battery1");

    device
        .write("2a19", vec![1, 2], WriteType::WithResponse)
        .await
        .unwrap();
    device
        .write("2a19", b"x".as_slice(), WriteType::WithoutResponse)
        .await
        .unwrap();
    {
        let st = bz.state.lock().unwrap();
        let types: Vec<Option<&str>> = st
            .remote_writes
            .iter()
            .map(|(_, _, t)| t.as_deref())
            .collect();
        assert_eq!(types, [Some("request"), Some("command")]);
        assert_eq!(st.remote_writes[0].0, chr_path);
        assert_eq!(st.remote_writes[0].1, vec![1, 2]);
    }
    assert_eq!(device.read("2a19").await.unwrap(), b"x");
    assert_eq!(WriteType::default(), WriteType::WithResponse);

    assert_eq!(
        device.read_descriptor("2a19", "2902").await.unwrap(),
        vec![0, 0]
    );
    device
        .write_descriptor("0x2a19", "0x2902", vec![1, 0])
        .await
        .unwrap();
    assert_eq!(
        device.read_descriptor("2a19", "2902").await.unwrap(),
        vec![1, 0]
    );

    assert!(matches!(device.read("2a00").await, Err(Error::NotFound(_))));
    assert!(matches!(
        device.read_descriptor("2a19", "2901").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        device.read("not a uuid").await,
        Err(Error::InvalidArgument(_))
    ));

    // Battery level falls back to the 0x2A19 characteristic.
    bz.server
        .object_server()
        .remove::<MockBattery, _>(dev_path.as_str())
        .await
        .unwrap();
    device
        .write("2a19", vec![55], WriteType::WithResponse)
        .await
        .unwrap();
    assert_eq!(device.battery_level().await.unwrap(), 55, "GATT fallback");

    // The cached database is refreshed when an object vanished.
    let server = bz.server.object_server();
    server
        .remove::<MockRemoteDescriptor, _>(format!("{chr_path}/desc0003").as_str())
        .await
        .unwrap();
    server
        .remove::<MockRemoteCharacteristic, _>(chr_path.as_str())
        .await
        .unwrap();
    let moved = format!("{dev_path}/service0001/char0009");
    server
        .at(
            moved.as_str(),
            MockRemoteCharacteristic {
                service: format!("{dev_path}/service0001"),
                value: vec![9],
                notifying: false,
                state: bz.state.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(device.read("2a19").await.unwrap(), vec![9]);
    assert_eq!(
        device.services().await.unwrap()[0].characteristics[0].path,
        moved
    );
}

#[tokio::test]
async fn device_notifications() {
    let bz = MockBluez::start().await;
    let (_s, adapter) = discovered(&bz).await;
    let device = adapter.device(HEADSET).unwrap();
    device.connect(Duration::from_secs(2)).await.unwrap();
    let chr_path = format!("{}/service0001/char0002", device.path());

    let mut first = device.notifications("2a19").await.unwrap();
    assert!(format!("{first:?}").contains("char0002"));
    assert!(remote_notifying(&bz.server, &chr_path).await);
    // A second subscriber shares the BlueZ subscription (no InProgress).
    let mut second = device.clone().notifications("0x2A19").await.unwrap();

    for v in [vec![1u8], vec![2], vec![3]] {
        remote_notify(&bz.server, &chr_path, v).await;
    }
    for expected in [vec![1u8], vec![2], vec![3]] {
        assert_eq!(timeout("first", first.next()).await.unwrap(), expected);
        assert_eq!(timeout("second", second.next()).await.unwrap(), expected);
    }

    // Dropping one stream keeps notifications on for the other.
    drop(first);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(remote_notifying(&bz.server, &chr_path).await);
    remote_notify(&bz.server, &chr_path, vec![4]).await;
    assert_eq!(timeout("second", second.next()).await.unwrap(), vec![4]);
    second.stop().await.unwrap();
    assert!(!remote_notifying(&bz.server, &chr_path).await);

    // Drop of the last stream stops notifications in the background.
    let third = device.notifications("2a19").await.unwrap();
    assert!(remote_notifying(&bz.server, &chr_path).await);
    drop(third);
    let server = bz.server.clone();
    let path = chr_path.clone();
    eventually("StopNotify on drop", move || {
        let server = server.clone();
        let path = path.clone();
        async move { !remote_notifying(&server, &path).await }
    })
    .await;

    assert!(matches!(
        device.notifications("2a37").await,
        Err(Error::NotFound(_))
    ));
}

// ---------------------------------------------------------------------------
// Peripheral
// ---------------------------------------------------------------------------

fn app_objects(bz: &MockBluez) -> Vec<(String, bluebus::ObjectsMap)> {
    let st = bz.state.lock().unwrap();
    st.gatt_apps
        .iter()
        .map(|((_, app), objects)| {
            let copy = objects
                .iter()
                .map(|(p, i)| {
                    (
                        p.clone(),
                        i.iter()
                            .map(|(k, v)| {
                                (
                                    k.clone(),
                                    v.iter()
                                        .map(|(n, x)| (n.clone(), x.try_clone().unwrap()))
                                        .collect(),
                                )
                            })
                            .collect(),
                    )
                })
                .collect();
            (app.clone(), copy)
        })
        .collect()
}

fn find_object(
    objects: &bluebus::ObjectsMap,
    iface: &str,
    uuid: &str,
) -> (String, HashMap<String, OwnedValue>) {
    objects
        .iter()
        .find_map(|(p, i)| {
            let props = i.get(iface)?;
            let u = String::try_from(props["UUID"].try_clone().unwrap()).unwrap();
            (u == uuid).then(|| {
                (
                    p.to_string(),
                    props
                        .iter()
                        .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
                        .collect(),
                )
            })
        })
        .unwrap_or_else(|| panic!("{iface}: characteristic not exported"))
}

#[tokio::test]
async fn peripheral_gatt_server_and_advertisement() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();
    let owner = s.connection().unique_name().unwrap().to_string();

    let writes: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let control = Characteristic::new("12345678-1234-5678-1234-56789ABCDEF1")
        .write()
        .write_without_response()
        .on_write({
            let writes = writes.clone();
            move |v| writes.lock().unwrap().push(v)
        });
    assert!(format!("{control:?}").contains("on_write: true"));
    let builder = adapter
        .peripheral()
        .service(
            Service::new("180f").characteristic(
                Characteristic::new("2A19")
                    .read()
                    .notify()
                    .flag("read") // duplicate flags are ignored
                    .value(vec![100])
                    .descriptor("2901", b"Level".to_vec()),
            ),
        )
        .service(
            Service::new("12345678-1234-5678-1234-56789abcdef0")
                .primary(false)
                .characteristic(control)
                .characteristic(Characteristic::new("2a05").indicate()),
        )
        .advertise(
            Advertisement::new()
                .local_name("bluebus-test")
                .service_uuid("180f")
                .manufacturer_data(0xFFFF, vec![0xE2, 0xE0])
                .service_data("180f", vec![100]),
        );
    assert!(format!("{builder:?}").contains("bluebus-test"));
    let peripheral = builder.start().await.unwrap();
    assert!(peripheral.is_running());
    assert!(format!("{peripheral:?}").contains("Peripheral"));

    // What BlueZ read from the application.
    let apps = app_objects(&bz);
    assert_eq!(apps.len(), 1);
    let objects = &apps[0].1;
    let (svc_path, svc) = find_object(objects, "org.bluez.GattService1", SVC_UUID);
    assert!(bool::try_from(&*svc["Primary"]).unwrap());
    let (_, secondary) = find_object(
        objects,
        "org.bluez.GattService1",
        "12345678-1234-5678-1234-56789abcdef0",
    );
    assert!(!bool::try_from(&*secondary["Primary"]).unwrap());
    let (chr_path, chr) = find_object(objects, "org.bluez.GattCharacteristic1", CHR_UUID);
    assert!(chr_path.starts_with(&svc_path));
    assert_eq!(
        Vec::<String>::try_from(chr["Flags"].try_clone().unwrap()).unwrap(),
        ["read", "notify"]
    );
    assert_eq!(
        Vec::<u8>::try_from(chr["Value"].try_clone().unwrap()).unwrap(),
        [100]
    );
    let (_, desc) = find_object(objects, "org.bluez.GattDescriptor1", &uuid_from_u16(0x2901));
    assert_eq!(
        desc["Characteristic"]
            .downcast_ref::<zbus::zvariant::ObjectPath>()
            .unwrap()
            .as_str(),
        chr_path
    );
    let (ctl_path, ctl) = find_object(
        objects,
        "org.bluez.GattCharacteristic1",
        "12345678-1234-5678-1234-56789abcdef1",
    );
    assert_eq!(
        Vec::<String>::try_from(ctl["Flags"].try_clone().unwrap()).unwrap(),
        ["write", "write-without-response"]
    );
    {
        let st = bz.state.lock().unwrap();
        assert_eq!(st.advertisements.len(), 1);
        let adv = st.advertisements.values().next().unwrap();
        assert_eq!(
            String::try_from(adv["Type"].try_clone().unwrap()).unwrap(),
            "peripheral"
        );
        assert_eq!(
            String::try_from(adv["LocalName"].try_clone().unwrap()).unwrap(),
            "bluebus-test"
        );
        assert_eq!(
            Vec::<String>::try_from(adv["ServiceUUIDs"].try_clone().unwrap()).unwrap(),
            [SVC_UUID]
        );
        assert!(bool::try_from(&*adv["Discoverable"]).unwrap());
        assert!(adv.contains_key("ManufacturerData") && adv.contains_key("ServiceData"));
    }

    // A remote central (through BlueZ) reads, writes and subscribes.
    let remote = |path: &str| {
        let server = bz.server.clone();
        let owner = owner.clone();
        let path = path.to_owned();
        async move {
            zbus::Proxy::new(&server, owner, path, "org.bluez.GattCharacteristic1")
                .await
                .unwrap()
        }
    };
    let level = remote(&chr_path).await;
    let v: Vec<u8> = level
        .call("ReadValue", &(HashMap::<String, OwnedValue>::new(),))
        .await
        .unwrap();
    assert_eq!(v, [100]);
    let ctl_remote = remote(&ctl_path).await;
    let () = ctl_remote
        .call(
            "WriteValue",
            &(b"go".to_vec(), HashMap::<String, OwnedValue>::new()),
        )
        .await
        .unwrap();
    assert_eq!(*writes.lock().unwrap(), [b"go".to_vec()]);
    assert_eq!(
        peripheral
            .value("12345678-1234-5678-1234-56789abcdef1")
            .await
            .unwrap(),
        b"go"
    );

    peripheral.set_value("2a19", vec![90]).await.unwrap();
    assert_eq!(peripheral.value("0x2A19").await.unwrap(), [90]);
    let v: Vec<u8> = level
        .call("ReadValue", &(HashMap::<String, OwnedValue>::new(),))
        .await
        .unwrap();
    assert_eq!(v, [90]);

    assert!(!peripheral.is_notifying("2a19").await.unwrap());
    let props = zbus::fdo::PropertiesProxy::builder(&bz.server)
        .destination(owner.as_str())
        .unwrap()
        .path(chr_path.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let mut changes = props.receive_properties_changed().await.unwrap();
    let () = level.call("StartNotify", &()).await.unwrap();
    assert!(peripheral.is_notifying("2a19").await.unwrap());
    peripheral.notify("2a19", vec![80]).await.unwrap();
    let got = timeout("notification", async {
        while let Some(sig) = changes.next().await {
            let args = sig.args().unwrap();
            if let Some(v) = args.changed_properties().get("Value") {
                return Vec::<u8>::try_from(v.try_to_owned().unwrap()).unwrap();
            }
        }
        unreachable!()
    })
    .await;
    assert_eq!(got, [80]);

    assert!(matches!(
        peripheral.notify("2a37", vec![1]).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        peripheral.value("bad").await,
        Err(Error::InvalidArgument(_))
    ));

    // stop() unregisters both; a second stop is a no-op.
    peripheral.stop().await.unwrap();
    assert!(!peripheral.is_running());
    assert!(bz.state.lock().unwrap().gatt_apps.is_empty());
    assert!(bz.state.lock().unwrap().advertisements.is_empty());
    peripheral.stop().await.unwrap();
    assert!(matches!(
        peripheral.set_value("2a19", vec![1]).await,
        Err(Error::InvalidArgument(_))
    ));
    // Objects are gone and write callbacks removed.
    let e = level
        .call::<_, _, Vec<u8>>("ReadValue", &(HashMap::<String, OwnedValue>::new(),))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("Unknown"), "{e}");
}

#[tokio::test]
async fn peripherals_run_sequentially_and_together() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();
    let start = |name: &'static str| {
        adapter
            .peripheral()
            .service(Service::new("180f").characteristic(Characteristic::new("2a19").read()))
            .advertise(Advertisement::new().local_name(name))
            .start()
    };

    let mut seen_apps = Vec::new();
    let mut seen_advs = Vec::new();
    for name in ["one", "two", "three"] {
        let p = start(name).await.unwrap();
        {
            let st = bz.state.lock().unwrap();
            seen_apps.extend(st.gatt_apps.keys().map(|(_, p)| p.clone()));
            seen_advs.extend(st.advertisements.keys().map(|(_, p)| p.clone()));
        }
        p.stop().await.unwrap();
    }
    seen_apps.dedup();
    seen_advs.dedup();
    assert_eq!(seen_apps.len(), 3, "unique application paths");
    assert_eq!(seen_advs.len(), 3, "unique advertisement paths");

    // Two at once, then drop cleans up in the background.
    let a = start("a").await.unwrap();
    let b = start("b").await.unwrap();
    assert_eq!(bz.state.lock().unwrap().gatt_apps.len(), 2);
    assert_eq!(bz.state.lock().unwrap().advertisements.len(), 2);
    let a2 = a.clone();
    drop(a);
    assert_eq!(bz.state.lock().unwrap().gatt_apps.len(), 2, "clone alive");
    drop(a2);
    drop(b);
    let state = bz.state.clone();
    eventually("cleanup on drop", move || {
        let state = state.clone();
        async move {
            let st = state.lock().unwrap();
            st.gatt_apps.is_empty() && st.advertisements.is_empty()
        }
    })
    .await;

    // Advertisement only (a beacon) and GATT only.
    let beacon = adapter
        .peripheral()
        .advertise(
            Advertisement::new()
                .broadcast()
                .discoverable(true)
                .manufacturer_data(0x004C, vec![0x02, 0x15]),
        )
        .start()
        .await
        .unwrap();
    {
        let st = bz.state.lock().unwrap();
        assert!(st.gatt_apps.is_empty());
        let adv = st.advertisements.values().next().unwrap();
        assert_eq!(
            String::try_from(adv["Type"].try_clone().unwrap()).unwrap(),
            "broadcast"
        );
        assert!(!adv.contains_key("Discoverable"));
    }
    beacon.stop().await.unwrap();
    let gatt_only = adapter
        .peripheral()
        .service(Service::new("180f").characteristic(Characteristic::new("2a19").read()))
        .start()
        .await
        .unwrap();
    assert!(bz.state.lock().unwrap().advertisements.is_empty());
    assert_eq!(bz.state.lock().unwrap().gatt_apps.len(), 1);
    gatt_only.stop().await.unwrap();
}

#[tokio::test]
async fn peripheral_errors_leave_nothing_registered() {
    let bz = MockBluez::start().await;
    let s = session(&bz).await;
    let adapter = s.default_adapter().await.unwrap();

    let e = adapter.peripheral().start().await.unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "{e}");
    let e = adapter
        .peripheral()
        .service(Service::new("180f").characteristic(Characteristic::new("2a19")))
        .start()
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::InvalidArgument(m) if m.contains("flags")),
        "{e}"
    );
    let e = adapter
        .peripheral()
        .service(Service::new("battery"))
        .start()
        .await
        .unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "{e}");
    let e = adapter
        .peripheral()
        .advertise(Advertisement::new().service_uuid("nope"))
        .start()
        .await
        .unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "{e}");

    // BlueZ rejects an invalid flag: the advertisement is not registered.
    let e = adapter
        .peripheral()
        .service(Service::new("180f").characteristic(Characteristic::new("2a19").flag("bogus")))
        .advertise(Advertisement::new().local_name("x"))
        .start()
        .await
        .unwrap_err();
    assert_eq!(dbus_name(&e), "org.bluez.Error.InvalidArguments");
    {
        let st = bz.state.lock().unwrap();
        assert!(st.gatt_apps.is_empty() && st.advertisements.is_empty());
    }

    // hci1 of the mock has no GattManager1/LEAdvertisingManager1.
    let hci1 = s.adapter(Some("hci1")).await.unwrap();
    let e = hci1
        .peripheral()
        .service(Service::new("180f").characteristic(Characteristic::new("2a19").read()))
        .start()
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Bus(_)), "{e}");

    // After the failures a valid peripheral still starts.
    let ok = adapter
        .peripheral()
        .service(Service::new("180f").characteristic(Characteristic::new("2a19").read()))
        .start()
        .await
        .unwrap();
    ok.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// Errors and UUIDs
// ---------------------------------------------------------------------------

#[test]
fn errors_display_and_convert() {
    use std::error::Error as _;
    let bus = Error::from(zbus::fdo::Error::UnknownObject("gone".into()));
    assert_eq!(
        bus.dbus_error_name().as_deref(),
        Some("org.freedesktop.DBus.Error.UnknownObject")
    );
    assert!(bus.source().is_some());
    assert!(bus.to_string().contains("gone"));
    let variant = Error::from(zbus::zvariant::Error::IncorrectType);
    assert!(matches!(variant, Error::Bus(_)));
    assert_eq!(variant.dbus_error_name(), None);
    let plain = Error::from(zbus::Error::Failure("boom".into()));
    assert!(plain.to_string().contains("boom"));

    let t = Error::Timeout("connect".into());
    assert_eq!(t.to_string(), "timed out: connect");
    assert!(t.source().is_none());
    assert_eq!(Error::NotFound("x".into()).to_string(), "not found: x");
    assert_eq!(
        Error::InvalidArgument("y".into()).to_string(),
        "invalid argument: y"
    );
    assert!(format!("{t:?}").contains("Timeout"));
}

#[test]
fn uuid_normalisation() {
    let full = "0000180f-0000-1000-8000-00805f9b34fb";
    for s in [
        "180f",
        "180F",
        "0x180f",
        "0X180F",
        "0000180f",
        full,
        &full.to_uppercase(),
    ] {
        assert_eq!(normalize_uuid(s).unwrap(), full, "{s}");
    }
    assert_eq!(uuid_from_u16(0x180f), full);
    assert_eq!(bluebus::ble::uuid_from_u32(0x0000180f), full, "32-bit form");
    assert!(full.ends_with(bluebus::ble::BASE_UUID_SUFFIX));
    assert!(matches!(
        normalize_uuid("180g"),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn filters_and_builders_are_plain_values() {
    let f = ScanFilter::new()
        .transport(Transport::Auto)
        .name("a")
        .min_rssi(-1);
    assert_eq!(f.clone(), f);
    assert_ne!(f, ScanFilter::default());
    let adv = Advertisement::new().local_name("n");
    assert_eq!(adv.clone(), adv);
    assert_ne!(adv, Advertisement::default());
    let svc = Service::new("180f").characteristic(Characteristic::new("2a19").read());
    assert!(format!("{svc:?}").contains("2a19"));
}
