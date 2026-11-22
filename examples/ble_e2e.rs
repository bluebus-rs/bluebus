//! End-to-end test of the high-level `bluebus::ble` API between two adapters
//! driven by the real BlueZ daemon: one acts as a peripheral (GATT server +
//! advertisement), the other as a central (scan, connect, GATT
//! read/write/notify, pairing). Mirrors `virtual_e2e.rs`, which uses the
//! low-level API.
//!
//! Intended for two virtual controllers created by BlueZ' `btvirt`:
//!
//! ```sh
//! docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2
//! cargo run --example ble_e2e -- hci1 hci2   # peripheral central
//! docker stop bluebus-vhci
//! ```
//!
//! During pairing the central's agent is briefly BlueZ' default agent; it is
//! removed afterwards and BlueZ falls back to the previous default agent.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bluebus::ble::{
    Advertisement, AgentCapability, AutoAcceptAgent, Characteristic, Error, ScanEvent, ScanFilter,
    Service, Session, Transport, WriteType,
};
use futures::StreamExt;

const SVC: &str = "b1eb05e0-0000-4000-8000-00000000e2e0";
const CHR: &str = "b1eb05e0-0000-4000-8000-00000000e2e1";
const NAME: &str = "bluebus-ble-e2e";

async fn within<T>(what: &str, secs: u64, fut: impl Future<Output = T>) -> Result<T, String> {
    tokio::time::timeout(Duration::from_secs(secs), fut)
        .await
        .map_err(|_| format!("timeout: {what}"))
}

/// Polls `check` every 50 ms until it returns `Ok(true)`.
async fn until<F, Fut>(what: &str, secs: u64, mut check: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, Error>>,
{
    within(what, secs, async {
        loop {
            if check().await.map_err(|e| e.to_string())? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let p_hci = args.next().unwrap_or_else(|| "hci1".into());
    let c_hci = args.next().unwrap_or_else(|| "hci2".into());
    match run(&p_hci, &c_hci).await {
        Ok(n) => println!("\nALL {n} END-TO-END CHECKS PASSED"),
        Err(e) => {
            println!("\nFAILED: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(p_hci: &str, c_hci: &str) -> Result<usize, String> {
    let mut n = 0usize;
    macro_rules! ok {
        ($what:expr, $e:expr) => {{
            let v = $e;
            n += 1;
            println!("  ok  {:<56} {}", $what, v);
        }};
    }
    let e = |e: Error| e.to_string();

    // Two independent applications (two D-Bus connections).
    let p_session = Session::new().await.map_err(e)?;
    let c_session = Session::new().await.map_err(e)?;
    let periph = p_session.adapter(Some(p_hci)).await.map_err(e)?;
    let central = c_session.adapter(Some(c_hci)).await.map_err(e)?;
    let p_addr = periph.address().to_owned();
    let c_addr = central.address().to_owned();
    println!(
        "peripheral {} ({p_addr})  central {} ({c_addr})",
        periph.name(),
        central.name()
    );

    println!("\n[setup]");
    periph.power_on().await.map_err(e)?;
    central.power_on().await.map_err(e)?;
    ok!("Adapter::power_on (both)", "");
    // Forget the peer from a previous run.
    let _ = central.remove_device(&p_addr).await;
    let _ = periph.remove_device(&c_addr).await;

    let writes: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let peripheral = periph
        .peripheral()
        .service(
            Service::new(SVC).characteristic(
                Characteristic::new(CHR)
                    .read()
                    .write()
                    .write_without_response()
                    .notify()
                    .value(b"hello".to_vec())
                    .descriptor("2901", b"e2e".to_vec())
                    .on_write({
                        let writes = writes.clone();
                        move |v| writes.lock().unwrap().push(v)
                    }),
            ),
        )
        .service(
            Service::new("180f")
                .characteristic(Characteristic::new("2a19").read().notify().value(vec![77])),
        )
        .advertise(
            Advertisement::new()
                .local_name(NAME)
                .service_uuid(SVC)
                .manufacturer_data(0xFFFF, vec![0xE2, 0xE0]),
        )
        .start()
        .await
        .map_err(e)?;
    ok!(
        "peripheral: PeripheralBuilder::start (GATT + advertisement)",
        ""
    );

    println!("\n[discovery]");
    let mut scan = central
        .scan_stream(ScanFilter::new().transport(Transport::Le).service_uuid(SVC))
        .await
        .map_err(e)?;
    let found = within("ScanEvent::Found", 30, async {
        while let Some(event) = scan.next().await {
            if let ScanEvent::Found(d) = event {
                if d.address == p_addr {
                    return Some(d);
                }
            }
        }
        None
    })
    .await?
    .ok_or("scan stream ended")?;
    scan.stop().await.map_err(e)?;
    ok!(
        "central: scan_stream → Found",
        format!(
            "{} alias={:?} rssi={:?}",
            found.address, found.alias, found.rssi
        )
    );
    if found.manufacturer_data.get(&0xFFFF) != Some(&vec![0xE2, 0xE0]) {
        return Err(format!("manufacturer data {:?}", found.manufacturer_data));
    }
    ok!("central: advertised manufacturer_data", "ffff → e2e0");
    if !found.uuids.iter().any(|u| u == SVC) {
        return Err(format!("service UUID not advertised: {:?}", found.uuids));
    }
    ok!("central: advertised service uuids", SVC);

    let device = central
        .find_device(
            ScanFilter::new()
                .transport(Transport::Le)
                .address(p_addr.clone()),
            Duration::from_secs(30),
        )
        .await
        .map_err(e)?;
    let info = device.info().await.map_err(e)?;
    ok!(
        "central: find_device(address)",
        format!("name={:?} rssi={:?}", info.name, info.rssi)
    );
    let named = central
        .scan(
            ScanFilter::new().transport(Transport::Le).name(NAME),
            Duration::from_secs(3),
        )
        .await
        .map_err(e)?;
    ok!(
        "central: scan(name filter, 3 s)",
        format!("{} device(s)", named.len())
    );

    println!("\n[connection + GATT client]");
    device.connect(Duration::from_secs(30)).await.map_err(e)?;
    ok!(
        "central: Device::connect (+ ServicesResolved)",
        device.is_connected().await.map_err(e)?
    );
    let services = device.services().await.map_err(e)?;
    let chr = services
        .iter()
        .flat_map(|s| &s.characteristics)
        .find(|c| c.uuid == CHR)
        .ok_or("characteristic not found on the remote")?;
    ok!(
        "central: services()",
        format!(
            "{} services, CHR flags {:?}, {} descriptor(s)",
            services.len(),
            chr.flags,
            chr.descriptors.len()
        )
    );
    let v = device.read(CHR).await.map_err(e)?;
    if v != b"hello" {
        return Err(format!("read {v:?}"));
    }
    ok!("central: read", String::from_utf8_lossy(&v));
    device
        .write(CHR, b"world".to_vec(), WriteType::WithResponse)
        .await
        .map_err(e)?;
    // BlueZ acknowledges writes to a characteristic that also allows
    // write-without-response before the application has processed them.
    until("write arrives at the peripheral", 10, || async {
        Ok(peripheral.value(CHR).await? == b"world")
    })
    .await?;
    if writes.lock().unwrap().last().map(Vec::as_slice) != Some(b"world") {
        return Err(format!("on_write not called: {:?}", writes.lock().unwrap()));
    }
    ok!(
        "central: write(WithResponse) → peripheral value + on_write",
        "world"
    );
    device
        .write(CHR, b"cmd".to_vec(), WriteType::WithoutResponse)
        .await
        .map_err(e)?;
    until("write without response arrives", 10, || async {
        Ok(peripheral.value(CHR).await? == b"cmd")
    })
    .await?;
    ok!("central: write(WithoutResponse) → peripheral value", "cmd");
    let d = device.read_descriptor(CHR, "2901").await.map_err(e)?;
    ok!(
        "central: read_descriptor(0x2901)",
        String::from_utf8_lossy(&d)
    );
    ok!(
        "central: battery_level",
        format!("{}%", device.battery_level().await.map_err(e)?)
    );

    let mut notes = device.notifications(CHR).await.map_err(e)?;
    until("peripheral sees the subscription", 10, || {
        peripheral.is_notifying(CHR)
    })
    .await?;
    ok!("central: notifications → peripheral is_notifying", "");
    for value in [b"n1".to_vec(), b"n2".to_vec(), b"n3".to_vec()] {
        peripheral.notify(CHR, value.clone()).await.map_err(e)?;
        let got = within("notification", 10, notes.next())
            .await?
            .ok_or("stream ended")?;
        if got != value {
            return Err(format!("notification {got:?}, expected {value:?}"));
        }
    }
    ok!(
        "peripheral notify → central stream (3 in order)",
        "n1 n2 n3"
    );
    notes.stop().await.map_err(e)?;
    until("peripheral sees the unsubscription", 10, || async {
        Ok(!peripheral.is_notifying(CHR).await?)
    })
    .await?;
    ok!("central: NotificationStream::stop → is_notifying=false", "");
    ok!(
        "central: rssi",
        format!("{:?}", device.rssi().await.map_err(e)?)
    );

    println!("\n[pairing]");
    c_session
        .set_agent(AgentCapability::NoInputNoOutput, AutoAcceptAgent)
        .await
        .map_err(e)?;
    ok!("Session::set_agent(AutoAcceptAgent)", "");
    let paired = within("pair", 30, device.pair()).await?;
    c_session.remove_agent().await.map_err(e)?;
    paired.map_err(e)?;
    let info = device.info().await.map_err(e)?;
    if !info.paired {
        return Err("Paired=false after pair()".into());
    }
    ok!("central: pair → paired", info.paired);
    device.trust(true).await.map_err(e)?;
    ok!(
        "central: trust(true)",
        device.info().await.map_err(e)?.trusted
    );

    println!("\n[teardown]");
    device.disconnect().await.map_err(e)?;
    until("disconnected", 10, || async {
        Ok(!device.is_connected().await?)
    })
    .await?;
    ok!("central: disconnect", "");
    central.remove_device(&p_addr).await.map_err(e)?;
    let still = central
        .devices()
        .await
        .map_err(e)?
        .iter()
        .any(|d| d.address == p_addr);
    if still {
        return Err("device still known after remove_device".into());
    }
    ok!("central: remove_device", "");
    peripheral.stop().await.map_err(e)?;
    ok!("peripheral: stop (unregister GATT + advertisement)", "");

    // Several peripherals one after another in the same process.
    for i in 0..2 {
        let p = periph
            .peripheral()
            .service(Service::new(SVC).characteristic(Characteristic::new(CHR).read()))
            .advertise(Advertisement::new().local_name(format!("{NAME}-{i}")))
            .start()
            .await
            .map_err(e)?;
        p.stop().await.map_err(e)?;
    }
    ok!(
        "peripheral: start/stop twice more (unique object paths)",
        ""
    );
    let _ = periph.remove_device(&c_addr).await;
    Ok(n)
}
