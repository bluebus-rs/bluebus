//! bench-btleplug <hciN> <reads> <writes> <notifications>: same scenario as bench-bluebus.

use std::time::{Duration, Instant};

use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use futures::StreamExt;
use uuid::Uuid;

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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = Instant::now();
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (reads, writes, notes): (usize, usize, u32) =
        (a[1].parse()?, a[2].parse()?, a[3].parse()?);
    let manager = Manager::new().await?;
    let mut adapter = None;
    for ad in manager.adapters().await? {
        if ad.adapter_info().await?.starts_with(&format!("{} ", a[0])) {
            adapter = Some(ad);
        }
    }
    let adapter = adapter.ok_or("adapter not found")?;
    let startup = t0.elapsed();

    let t = Instant::now();
    let mut events = adapter.events().await?;
    adapter.start_scan(ScanFilter::default()).await?;
    let peripheral = loop {
        if let Some(CentralEvent::DeviceDiscovered(id)) = events.next().await {
            let p = adapter.peripheral(&id).await?;
            if p.properties().await?.and_then(|p| p.local_name).as_deref() == Some(NAME) {
                break p;
            }
        }
    };
    let discover = t.elapsed();
    adapter.stop_scan().await?;

    let t = Instant::now();
    peripheral.connect().await?;
    peripheral.discover_services().await?;
    let connect = t.elapsed();
    let chars = peripheral.characteristics();
    let chr = |u: &str| {
        let u = Uuid::parse_str(u).unwrap();
        chars.iter().find(|c| c.uuid == u).unwrap().clone()
    };

    let cpu_rw = cpu_ms();
    let read_chr = chr(READ);
    let mut read_t = Vec::with_capacity(reads);
    for _ in 0..reads {
        let t = Instant::now();
        let v = peripheral.read(&read_chr).await?;
        read_t.push(t.elapsed());
        assert_eq!(v.len(), 20);
    }
    let write_chr = chr(WRITE);
    let mut write_t = Vec::with_capacity(writes);
    for _ in 0..writes {
        let t = Instant::now();
        peripheral.write(&write_chr, &[1; 20], WriteType::WithResponse).await?;
        write_t.push(t.elapsed());
    }

    let cpu_rw = cpu_ms() - cpu_rw;
    let note_chr = chr(NOTIFY);
    peripheral.subscribe(&note_chr).await?;
    let mut stream = peripheral.notifications().await?;
    let t = Instant::now();
    peripheral
        .write(&note_chr, &notes.to_le_bytes(), WriteType::WithResponse)
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
    peripheral.unsubscribe(&note_chr).await?;

    let t = Instant::now();
    peripheral.disconnect().await?;
    let disconnect = t.elapsed();

    read_t.sort();
    write_t.sort();
    let pct = |v: &[Duration], p: f64| ms(v[((v.len() - 1) as f64 * p) as usize]);
    println!(
        "{{\"lib\":\"btleplug\",\"startup_ms\":{:.2},\"discover_ms\":{:.1},\"connect_ms\":{:.1},\
\"read_p50_ms\":{:.3},\"read_p95_ms\":{:.3},\"write_p50_ms\":{:.3},\"write_p95_ms\":{:.3},\
\"notify_received\":{got},\"notify_ms\":{:.1},\"disconnect_ms\":{:.1},\"cpu_rw_ms\":{:.2},\"cpu_notify_ms\":{:.2}}}",
        ms(startup),
        ms(discover),
        ms(connect),
        pct(&read_t, 0.5),
        pct(&read_t, 0.95),
        pct(&write_t, 0.5),
        pct(&write_t, 0.95),
        ms(last),
        ms(disconnect),
        cpu_rw,
        cpu_n
    );
    Ok(())
}
