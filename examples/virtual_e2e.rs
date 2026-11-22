//! End-to-end test between two adapters driven by the real BlueZ daemon:
//! one acts as a peripheral (GATT server + advertisement), the other as a
//! central (discovery, connect, GATT read/write/notify, pairing).
//!
//! Intended for two virtual controllers created by BlueZ' `btvirt`:
//!
//! ```sh
//! docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2
//! cargo run --example virtual_e2e -- hci1 hci2
//! docker stop bluebus-vhci
//! ```
//!
//! During pairing the test agent is briefly the default agent (BlueZ asks
//! the default agent on the responding side); it is unregistered afterwards
//! and BlueZ falls back to the previous default agent.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bluebus::*;
use futures::StreamExt;
use tokio::sync::mpsc;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

const SVC: &str = "b1eb05e0-0000-4000-8000-00000000e2e0";
const CHR: &str = "b1eb05e0-0000-4000-8000-00000000e2e1";
const NAME: &str = "bluebus-e2e";

#[derive(Clone, Default)]
struct E2eAgent {
    calls: Arc<Mutex<Vec<String>>>,
}

impl AgentHandler for E2eAgent {
    async fn request_confirmation(&self, device: OwnedObjectPath, passkey: u32) -> AgentResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("RequestConfirmation {device} {passkey:06}"));
        Ok(())
    }
    async fn request_authorization(&self, device: OwnedObjectPath) -> AgentResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("RequestAuthorization {device}"));
        Ok(())
    }
    async fn authorize_service(&self, device: OwnedObjectPath, uuid: String) -> AgentResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("AuthorizeService {device} {uuid}"));
        Ok(())
    }
}

async fn within<T>(
    what: &str,
    secs: u64,
    fut: impl std::future::Future<Output = T>,
) -> Result<T, String> {
    tokio::time::timeout(Duration::from_secs(secs), fut)
        .await
        .map_err(|_| format!("timeout: {what}"))
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let p_hci = args.next().unwrap_or_else(|| "hci1".into());
    let c_hci = args.next().unwrap_or_else(|| "hci2".into());
    match run(
        &format!("/org/bluez/{p_hci}"),
        &format!("/org/bluez/{c_hci}"),
    )
    .await
    {
        Ok(n) => println!("\nALL {n} END-TO-END CHECKS PASSED"),
        Err(e) => {
            println!("\nFAILED: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(p_path: &str, c_path: &str) -> Result<usize, String> {
    let mut n = 0usize;
    macro_rules! ok {
        ($what:expr, $e:expr) => {{
            let v = $e;
            n += 1;
            println!("  ok  {:<52} {}", $what, v);
        }};
    }
    let e = |e: zbus::Error| e.to_string();

    // Two independent applications: peripheral and central.
    let pconn = get_system_connection().await.map_err(e)?;
    let cconn = get_system_connection().await.map_err(e)?;
    let periph = AdapterProxy::builder(&pconn)
        .path(p_path)
        .map_err(e)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(e)?;
    let central = AdapterProxy::new(&cconn, c_path.to_owned())
        .await
        .map_err(e)?;
    let p_addr = periph.address().await.map_err(e)?;
    println!(
        "peripheral {p_path} ({p_addr})  central {c_path} ({})",
        central.address().await.map_err(e)?
    );

    println!("\n[setup]");
    set_adapter_powered(&periph, true, Duration::from_secs(10))
        .await
        .map_err(e)?;
    set_adapter_powered(&central, true, Duration::from_secs(10))
        .await
        .map_err(e)?;
    ok!("both adapters powered", "");
    // Forget the peer from a previous run.
    let dev_path = device_path(c_path, &p_addr);
    let _ = central
        .remove_device(OwnedObjectPath::try_from(dev_path.clone()).unwrap())
        .await;
    let _ = periph
        .remove_device(
            OwnedObjectPath::try_from(device_path(p_path, &central.address().await.map_err(e)?))
                .unwrap(),
        )
        .await;

    // Peripheral: GATT application.
    let app = "/bluebus/e2e/app";
    let svc_path = format!("{app}/service0");
    let chr_path = format!("{svc_path}/char0");
    register_service(
        &pconn,
        &svc_path,
        GattService {
            uuid: SVC.into(),
            primary: true,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await
    .map_err(|x| x.to_string())?;
    register_characteristic(
        &pconn,
        &chr_path,
        GattCharacteristic::new(CHR, svc_path.clone(), &["read", "write", "notify"])
            .with_value(*b"hello"),
    )
    .await
    .map_err(|x| x.to_string())?;
    register_descriptor(
        &pconn,
        &format!("{chr_path}/desc0"),
        GattDescriptor {
            uuid: "2901".into(),
            characteristic: chr_path.clone(),
            value: b"e2e".to_vec(),
            flags: vec!["read".into()],
            handle: None,
        },
    )
    .await
    .map_err(|x| x.to_string())?;
    let writes: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    {
        let writes = writes.clone();
        set_write_handler(&chr_path, move |_, v| {
            writes.lock().unwrap().push(v.to_vec())
        });
    }
    register_gatt_application_at(&pconn, p_path, app)
        .await
        .map_err(e)?;
    ok!("peripheral: RegisterApplication", "");

    // Peripheral: advertisement.
    let adv_path = "/bluebus/e2e/adv0";
    let adv = LEAdvertisementProperties::new(AdvertisementType::Peripheral)
        .with_local_name(NAME)
        .with_service_uuids([SVC])
        .with_manufacturer_data(0xFFFF, vec![0xE2, 0xE0])
        .with_discoverable(true);
    register_advertisement_at(&pconn, p_path, adv_path, adv)
        .await
        .map_err(e)?;
    ok!("peripheral: RegisterAdvertisement", "");

    // Central: monitor + filtered LE discovery.
    println!("\n[discovery]");
    let manager = Arc::new(ObjectManagerProxy::new(&cconn).await.map_err(e)?);
    let (added_tx, mut added) = mpsc::channel(64);
    let (removed_tx, mut removed) = mpsc::channel(64);
    let (changed_tx, mut changed) = mpsc::channel(256);
    let monitor = Arc::new(
        Monitor::new(
            Arc::new(cconn.clone()),
            manager,
            added_tx,
            removed_tx,
            changed_tx,
        )
        .await,
    );
    let runner = tokio::spawn({
        let m = monitor.clone();
        async move { m.run().await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    central
        .set_discovery_filter(
            DiscoveryFilter::new()
                .transport(Transport::Le)
                .uuids([SVC])
                .to_dict(),
        )
        .await
        .map_err(e)?;
    central.start_discovery().await.map_err(e)?;
    let found = within("device added", 20, async {
        while let Some(d) = added.recv().await {
            if d.address == p_addr {
                return Some(d);
            }
        }
        None
    })
    .await?
    .ok_or("monitor stopped")?;
    ok!(
        "central: Monitor reported peripheral",
        format!(
            "{} alias={:?} rssi={:?}",
            found.address, found.alias, found.rssi
        )
    );
    central.stop_discovery().await.map_err(e)?;
    central
        .set_discovery_filter(Default::default())
        .await
        .map_err(e)?;

    let dev = DeviceProxy::builder(&cconn)
        .path(dev_path.clone())
        .map_err(e)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(e)?;
    let md = dev.manufacturer_data().await.map_err(e)?;
    let md = md.get(&0xFFFF).and_then(advertising_bytes);
    if md != Some(vec![0xE2, 0xE0]) {
        return Err(format!("manufacturer data {md:?}"));
    }
    ok!("central: advertised ManufacturerData", "ffff → e2e0");
    let uuids = dev.uuids().await.map_err(e)?;
    if !uuids.iter().any(|u| u.eq_ignore_ascii_case(SVC)) {
        return Err(format!("service UUID not advertised: {uuids:?}"));
    }
    ok!("central: advertised ServiceUUIDs", SVC);
    let alias = dev.alias().await.map_err(e)?;
    ok!("central: advertised LocalName", alias);

    // Connect + GATT.
    println!("\n[connection + GATT client]");
    within("Connect", 30, dev.connect()).await?.map_err(e)?;
    ok!("central: Connect", "");
    within("ServicesResolved", 30, async {
        while !dev.services_resolved().await.unwrap_or(false) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    ok!("central: ServicesResolved", "");
    let saw_connected = within("Monitor Connected", 5, async {
        while let Some(d) = changed.recv().await {
            if d.address == p_addr && d.connected {
                return true;
            }
        }
        false
    })
    .await?;
    ok!("central: Monitor reported Connected=true", saw_connected);

    let db = discover_gatt_database(&cconn, &dev_path).await.map_err(e)?;
    let chr_attr = db
        .characteristic_by_uuid(CHR)
        .ok_or("characteristic not found on the remote")?
        .clone();
    ok!(
        "central: discover_gatt_database",
        format!(
            "{} services, {} characteristics, {} descriptors",
            db.services.len(),
            db.characteristics.len(),
            db.descriptors.len()
        )
    );
    let chr = gatt_characteristic_proxy(&cconn, chr_attr.path.clone())
        .await
        .map_err(e)?;
    let flags = chr.flags().await.map_err(|x| x.to_string())?;
    ok!("central: characteristic Flags", format!("{flags:?}"));
    let v = chr
        .read_value(Default::default())
        .await
        .map_err(|x| x.to_string())?;
    if v != b"hello" {
        return Err(format!("read {v:?}"));
    }
    ok!("central: ReadValue", String::from_utf8_lossy(&v));
    chr.write_value(b"world".to_vec(), Default::default())
        .await
        .map_err(|x| x.to_string())?;
    let local = local_characteristic_value(&pconn, &chr_path)
        .await
        .map_err(e)?;
    if local.as_deref() != Some(b"world".as_slice())
        || writes.lock().unwrap().last().map(Vec::as_slice) != Some(b"world".as_slice())
    {
        return Err(format!("peripheral did not see the write: {local:?}"));
    }
    ok!(
        "central: WriteValue → peripheral value + write handler",
        "world"
    );
    let v = chr
        .read_value(Default::default())
        .await
        .map_err(|x| x.to_string())?;
    ok!(
        "central: ReadValue after write",
        String::from_utf8_lossy(&v)
    );
    let mtu = chr
        .mtu()
        .await
        .map(Option::<u16>::from)
        .map_err(|x| x.to_string())?;
    ok!("central: MTU", format!("{mtu:?}"));
    if let Some(desc) = db
        .descriptors
        .iter()
        .find(|d| d.parent == chr_attr.path && d.uuid.starts_with("00002901"))
    {
        let d = gatt_descriptor_proxy(&cconn, desc.path.as_str())
            .await
            .map_err(e)?;
        let v = d
            .read_value(Default::default())
            .await
            .map_err(|x| x.to_string())?;
        ok!(
            "central: descriptor 0x2901 ReadValue",
            String::from_utf8_lossy(&v)
        );
    }

    // Notifications.
    let mut notes = characteristic_notifications(&chr).await.map_err(e)?;
    within("Notifying on peripheral", 10, async {
        loop {
            let iface = pconn
                .object_server()
                .interface::<_, GattCharacteristic>(chr_path.as_str())
                .await
                .unwrap();
            if iface.get().await.notifying {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    ok!("central: StartNotify → peripheral Notifying=true", "");
    for (i, value) in [b"n1".to_vec(), b"n2".to_vec(), b"n3".to_vec()]
        .into_iter()
        .enumerate()
    {
        notify_characteristic_value(&pconn, &chr_path, value.clone())
            .await
            .map_err(e)?;
        let got = within("notification", 10, notes.next())
            .await?
            .ok_or("stream ended")?;
        if got != value {
            return Err(format!("notification {i}: {got:?}"));
        }
    }
    ok!(
        "peripheral notify → central stream (3 in order)",
        "n1 n2 n3"
    );
    chr.stop_notify().await.map_err(|x| x.to_string())?;
    drop(notes);

    // Notifications through AcquireNotify (a socket instead of D-Bus).
    let (mut notes, mtu) = characteristic_notifications_acquired(&chr)
        .await
        .map_err(e)?;
    within("Notifying on peripheral (acquired)", 10, async {
        loop {
            let iface = pconn
                .object_server()
                .interface::<_, GattCharacteristic>(chr_path.as_str())
                .await
                .unwrap();
            if iface.get().await.notifying {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    for value in [b"a1".to_vec(), b"a2".to_vec(), b"a3".to_vec()] {
        notify_characteristic_value(&pconn, &chr_path, value.clone())
            .await
            .map_err(e)?;
        let got = within("acquired notification", 10, notes.next())
            .await?
            .ok_or("socket closed")?;
        if got != value {
            return Err(format!("acquired notification: {got:?}"));
        }
    }
    ok!(
        "AcquireNotify socket → central stream (3 in order)",
        format!("a1 a2 a3, mtu {mtu}")
    );
    drop(notes); // closing the socket stops notifications

    // Pairing (Just Works) with our agent as default on both sides.
    println!("\n[pairing]");
    let agent = E2eAgent::default();
    register_agent(
        &cconn,
        "/bluebus/e2e/agent",
        AgentCapability::NoInputNoOutput,
        agent.clone(),
        true,
    )
    .await
    .map_err(e)?;
    ok!("register_agent (default)", "");
    let pair = within("Pair", 30, dev.pair()).await?;
    let pair_res = match pair {
        Ok(()) => "ok".to_string(),
        Err(err) => format!("{err}"),
    };
    let paired = dev.paired().await.map_err(e)?;
    unregister_agent(&cconn, "/bluebus/e2e/agent")
        .await
        .map_err(e)?;
    if !paired {
        return Err(format!(
            "Pair: {pair_res}; agent calls {:?}",
            agent.calls.lock().unwrap()
        ));
    }
    ok!(
        "central: Pair → Paired=true",
        format!("agent calls: {:?}", agent.calls.lock().unwrap())
    );
    ok!("central: Bonded", format!("{:?}", dev.bonded().await.ok()));
    dev.set_trusted(true).await.map_err(e)?;
    ok!("central: Trusted=true", dev.trusted().await.map_err(e)?);
    dev.set_alias("e2e peer").await.map_err(e)?;
    ok!("central: Alias", dev.alias().await.map_err(e)?);
    let snapshot = list_adapter_devices(&cconn, c_path)
        .await
        .map_err(e)?
        .into_iter()
        .find(|d| d.address == p_addr)
        .ok_or("missing in list")?;
    ok!(
        "central: list_adapter_devices snapshot",
        format!(
            "paired={} trusted={:?} alias={}",
            snapshot.paired, snapshot.trusted, snapshot.alias
        )
    );

    // Disconnect + remove.
    println!("\n[teardown]");
    within("Disconnect", 20, dev.disconnect())
        .await?
        .map_err(e)?;
    within("Connected=false", 10, async {
        while dev.connected().await.unwrap_or(true) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    ok!("central: Disconnect", "");
    central
        .remove_device(OwnedObjectPath::try_from(dev_path.clone()).unwrap())
        .await
        .map_err(e)?;
    let gone = within("Monitor removed", 10, async {
        while let Some(d) = removed.recv().await {
            if d.address == p_addr {
                return true;
            }
        }
        false
    })
    .await?;
    ok!("central: RemoveDevice → Monitor reported removal", gone);
    runner.abort();

    unregister_advertisement_at(&pconn, p_path, adv_path)
        .await
        .map_err(e)?;
    ok!("peripheral: UnregisterAdvertisement", "");
    unregister_gatt_application_at(&pconn, p_path, app)
        .await
        .map_err(e)?;
    ok!("peripheral: UnregisterApplication", "");
    let c_addr = central.address().await.map_err(e)?;
    let _ = periph
        .remove_device(OwnedObjectPath::try_from(device_path(p_path, &c_addr)).unwrap())
        .await;
    Ok(n)
}
