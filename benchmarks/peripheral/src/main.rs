//! The GATT server every benchmark client talks to (same server for all).
//!
//! bench-peripheral <hciN>
//!
//! Service BENCH_SVC with three characteristics:
//! - READ: read → 20 bytes
//! - WRITE: write / write-without-response, value discarded
//! - NOTIFY: notify + write; writing a u32 (little endian) N makes the
//!   server send N notifications of 20 bytes (first 4 bytes = sequence).

use std::time::Duration;

use bluebus::*;
use zbus::proxy::CacheProperties;

const SVC: &str = "b1eb0000-0000-4000-8000-00000000be00";
const READ: &str = "b1eb0000-0000-4000-8000-00000000be01";
const WRITE: &str = "b1eb0000-0000-4000-8000-00000000be02";
const NOTIFY: &str = "b1eb0000-0000-4000-8000-00000000be03";
const NAME: &str = "bench-periph";

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let hci = std::env::args().nth(1).unwrap_or_else(|| "hci1".into());
    let adapter_path = format!("/org/bluez/{hci}");
    let conn = get_system_connection().await?;
    let adapter = AdapterProxy::builder(&conn)
        .path(adapter_path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    set_adapter_powered(&adapter, true, Duration::from_secs(10)).await?;

    let app = "/bluebus/bench/app";
    let svc = format!("{app}/service0");
    let read = format!("{svc}/char0");
    let write = format!("{svc}/char1");
    let notify = format!("{svc}/char2");
    register_service(
        &conn,
        &svc,
        GattService {
            uuid: SVC.into(),
            primary: true,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await
    .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let chars = [
        (&read, READ, &["read"][..], (0u8..20).collect::<Vec<_>>()),
        (&write, WRITE, &["write", "write-without-response"][..], vec![0]),
        (&notify, NOTIFY, &["notify", "write"][..], vec![0; 20]),
    ];
    for (path, uuid, flags, value) in chars {
        register_characteristic(
            &conn,
            path,
            GattCharacteristic::new(uuid, svc.clone(), flags).with_value(value),
        )
        .await
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    }
    set_write_handler(&write, |_, _| {});

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u32>();
    set_write_handler(&notify, move |_, v| {
        if v.len() >= 4 {
            let _ = tx.send(u32::from_le_bytes([v[0], v[1], v[2], v[3]]));
        }
    });
    {
        let conn = conn.clone();
        let notify = notify.clone();
        tokio::spawn(async move {
            while let Some(n) = rx.recv().await {
                for i in 0..n {
                    let mut value = vec![0u8; 20];
                    value[..4].copy_from_slice(&i.to_le_bytes());
                    if notify_characteristic_value(&conn, &notify, value).await.is_err() {
                        break;
                    }
                }
            }
        });
    }
    register_gatt_application_at(&conn, &adapter_path, app).await?;
    register_advertisement_at(
        &conn,
        &adapter_path,
        "/bluebus/bench/adv0",
        LEAdvertisementProperties::new(AdvertisementType::Peripheral)
            .with_local_name(NAME)
            .with_service_uuids([SVC])
            .with_discoverable(true),
    )
    .await?;
    println!("READY {}", adapter.address().await?);
    std::future::pending::<()>().await;
    Ok(())
}
