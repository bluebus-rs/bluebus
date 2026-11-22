//! bench-bluebus <hciN> <reads> <writes> <notifications>
//! Prints one JSON line with the phase timings (see ../../README.md).

use std::time::{Duration, Instant};

use bluebus::*;
use futures::StreamExt;
use zbus::proxy::CacheProperties;

const SVC: &str = "b1eb0000-0000-4000-8000-00000000be00";
const READ: &str = "b1eb0000-0000-4000-8000-00000000be01";
const WRITE: &str = "b1eb0000-0000-4000-8000-00000000be02";
const NOTIFY: &str = "b1eb0000-0000-4000-8000-00000000be03";
const NAME: &str = "bench-periph";

/// CPU time of this process so far (all threads, from /proc, nanosecond resolution).
fn cpu_ms() -> f64 {
    let mut ns = 0u64;
    for t in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        if let Ok(s) = std::fs::read_to_string(t.path().join("schedstat")) {
            ns += s.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
        }
    }
    ns as f64 / 1e6
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let t0 = Instant::now();
    let a: Vec<String> = std::env::args().skip(1).collect();
    let adapter_path = format!("/org/bluez/{}", a[0]);
    let (reads, writes, notes): (usize, usize, u32) =
        (a[1].parse().unwrap(), a[2].parse().unwrap(), a[3].parse().unwrap());

    let conn = get_system_connection().await?;
    let adapter = AdapterProxy::new(&conn, adapter_path.clone()).await?;
    set_adapter_powered(&adapter, true, Duration::from_secs(10)).await?;
    let startup = t0.elapsed();

    // Discovery: until a device named NAME shows up.
    let t = Instant::now();
    let manager = ObjectManagerProxy::new(&conn).await?;
    let mut added = manager.receive_interfaces_added().await?;
    adapter
        .set_discovery_filter(DiscoveryFilter::new().transport(Transport::Le).duplicate_data(true).to_dict())
        .await?;
    adapter.start_discovery().await?;
    let dev_path = loop {
        let sig = added.next().await.expect("bus closed");
        let args = sig.args()?;
        let path = args.object_path().to_string();
        if !path.starts_with(&adapter_path) {
            continue;
        }
        if let Some(props) = args.interfaces().get("org.bluez.Device1") {
            let name = props
                .get("Name")
                .and_then(|v| <&str>::try_from(v).ok().map(str::to_owned));
            if name.as_deref() == Some(NAME) {
                break path;
            }
        }
    };
    let discover = t.elapsed();
    adapter.stop_discovery().await?;

    // Connect and resolve services.
    let t = Instant::now();
    let dev = DeviceProxy::builder(&conn)
        .path(dev_path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    dev.connect().await?;
    let mut resolved = DeviceProxy::new(&conn, dev_path.clone())
        .await?
        .receive_services_resolved_changed()
        .await;
    while !dev.services_resolved().await? {
        resolved.next().await;
    }
    let db = discover_gatt_database(&conn, &dev_path).await?;
    let connect = t.elapsed();
    let chr = |uuid: &str| db.characteristic_by_uuid(uuid).unwrap().path.clone();
    let _ = SVC;

    let cpu_rw = cpu_ms();
    let read_chr = gatt_characteristic_proxy(&conn, chr(READ)).await?;
    let mut read_us = Vec::with_capacity(reads);
    for _ in 0..reads {
        let t = Instant::now();
        let v = read_chr.read_value(Default::default()).await?;
        read_us.push(t.elapsed());
        assert_eq!(v.len(), 20);
    }

    let write_chr = gatt_characteristic_proxy(&conn, chr(WRITE)).await?;
    let mut write_us = Vec::with_capacity(writes);
    let mut opts = std::collections::HashMap::new();
    opts.insert("type".to_string(), zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from("request")).unwrap());
    for _ in 0..writes {
        let t = Instant::now();
        write_chr.write_value(vec![1; 20], opts.clone()).await?;
        write_us.push(t.elapsed());
    }

    let cpu_rw = cpu_ms() - cpu_rw;
    let note_chr = gatt_characteristic_proxy(&conn, chr(NOTIFY)).await?;
    let acquire = a.get(4).map(String::as_str) == Some("acquire");
    let mut stream = if acquire {
        characteristic_notifications_acquired(&note_chr).await?.0.boxed()
    } else {
        characteristic_notifications(&note_chr).await?.boxed()
    };
    let t = Instant::now();
    note_chr
        .write_value(notes.to_le_bytes().to_vec(), opts.clone())
        .await?;
    let cpu_n = cpu_ms();
    let mut got = 0u32;
    let mut last = t.elapsed();
    while got < notes {
        match tokio::time::timeout(Duration::from_secs(3), stream.next()).await {
            Ok(Some(_)) => {
                got += 1;
                last = t.elapsed();
            }
            _ => break,
        }
    }
    let cpu_n = cpu_ms() - cpu_n;
    drop(stream);
    if !acquire {
        note_chr.stop_notify().await?;
    }

    let t = Instant::now();
    dev.disconnect().await?;
    let disconnect = t.elapsed();

    read_us.sort();
    write_us.sort();
    let pct = |v: &[Duration], p: f64| ms(v[((v.len() - 1) as f64 * p) as usize]);
    println!(
        "{{\"lib\":\"bluebus\",\"mode\":\"{}\",\"startup_ms\":{:.2},\"discover_ms\":{:.1},\"connect_ms\":{:.1},\
\"read_p50_ms\":{:.3},\"read_p95_ms\":{:.3},\"write_p50_ms\":{:.3},\"write_p95_ms\":{:.3},\
\"notify_received\":{got},\"notify_ms\":{:.1},\"disconnect_ms\":{:.1},\"cpu_rw_ms\":{:.2},\"cpu_notify_ms\":{:.2}}}",
        if acquire { "acquire" } else { "start" },
        ms(startup),
        ms(discover),
        ms(connect),
        pct(&read_us, 0.5),
        pct(&read_us, 0.95),
        pct(&write_us, 0.5),
        pct(&write_us, 0.95),
        ms(last),
        ms(disconnect),
        cpu_rw,
        cpu_n
    );
    Ok(())
}
