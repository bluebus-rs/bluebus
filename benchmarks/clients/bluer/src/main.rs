//! bench-bluer <hciN> <reads> <writes> <notifications>: same scenario as bench-bluebus.

use std::time::{Duration, Instant};

use bluer::gatt::remote::{Characteristic, CharacteristicWriteRequest};
use bluer::gatt::WriteOp;
use bluer::{AdapterEvent, DiscoveryFilter, DiscoveryTransport};
use futures::{pin_mut, StreamExt};

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
async fn main() -> bluer::Result<()> {
    let t0 = Instant::now();
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (reads, writes, notes): (usize, usize, u32) =
        (a[1].parse().unwrap(), a[2].parse().unwrap(), a[3].parse().unwrap());
    let session = bluer::Session::new().await?;
    let adapter = session.adapter(&a[0])?;
    adapter.set_powered(true).await?;
    let startup = t0.elapsed();

    let t = Instant::now();
    adapter
        .set_discovery_filter(DiscoveryFilter {
            transport: DiscoveryTransport::Le,
            duplicate_data: true,
            ..Default::default()
        })
        .await?;
    let device = {
        let events = adapter.discover_devices().await?;
        pin_mut!(events);
        loop {
            if let Some(AdapterEvent::DeviceAdded(addr)) = events.next().await {
                let d = adapter.device(addr)?;
                if d.name().await?.as_deref() == Some(NAME) {
                    break d;
                }
            }
        }
    };
    let discover = t.elapsed();
    // Dropping the stream stops discovery in the background; wait for it,
    // as the other clients stop discovery before connecting.
    while adapter.is_discovering().await? {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let t = Instant::now();
    device.connect().await?;
    let mut chars: Vec<(String, Characteristic)> = Vec::new();
    for s in device.services().await? {
        for c in s.characteristics().await? {
            chars.push((c.uuid().await?.to_string(), c));
        }
    }
    let connect = t.elapsed();
    let chr = |u: &str| chars.iter().find(|(x, _)| x == u).unwrap().1.clone();
    let req = CharacteristicWriteRequest {
        op_type: WriteOp::Request,
        ..Default::default()
    };

    let cpu_rw = cpu_ms();
    let read_chr = chr(READ);
    let mut read_t = Vec::with_capacity(reads);
    for _ in 0..reads {
        let t = Instant::now();
        let v = read_chr.read().await?;
        read_t.push(t.elapsed());
        assert_eq!(v.len(), 20);
    }
    let write_chr = chr(WRITE);
    let mut write_t = Vec::with_capacity(writes);
    for _ in 0..writes {
        let t = Instant::now();
        write_chr.write_ext(&[1; 20], &req).await?;
        write_t.push(t.elapsed());
    }

    let cpu_rw = cpu_ms() - cpu_rw;
    let note_chr = chr(NOTIFY);
    let acquire = a.get(4).map(String::as_str) == Some("acquire");
    let stream = if acquire {
        let reader = note_chr.notify_io().await?;
        futures::stream::unfold(reader, |r| async move { r.recv().await.ok().map(|v| (v, r)) })
            .boxed()
    } else {
        note_chr.notify().await?.boxed()
    };
    pin_mut!(stream);
    let t = Instant::now();
    note_chr.write_ext(&notes.to_le_bytes(), &req).await?;
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

    let t = Instant::now();
    device.disconnect().await?;
    let disconnect = t.elapsed();

    read_t.sort();
    write_t.sort();
    let pct = |v: &[Duration], p: f64| ms(v[((v.len() - 1) as f64 * p) as usize]);
    println!(
        "{{\"lib\":\"bluer\",\"mode\":\"{}\",\"startup_ms\":{:.2},\"discover_ms\":{:.1},\"connect_ms\":{:.1},\
\"read_p50_ms\":{:.3},\"read_p95_ms\":{:.3},\"write_p50_ms\":{:.3},\"write_p95_ms\":{:.3},\
\"notify_received\":{got},\"notify_ms\":{:.1},\"disconnect_ms\":{:.1},\"cpu_rw_ms\":{:.2},\"cpu_notify_ms\":{:.2}}}",
        if acquire { "acquire" } else { "start" },
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
