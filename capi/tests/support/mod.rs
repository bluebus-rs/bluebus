//! The mock BlueZ of the bluebus tests (`tests/common`), run on its own
//! runtime so that the blocking C functions can be called from plain
//! `#[test]` functions.
#![allow(dead_code)]

#[path = "../../../tests/common/mod.rs"]
pub mod common;

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use zbus::zvariant::OwnedValue;

#[allow(unused_imports)]
pub use common::{MockBluez, ADAPTER0, CHR_UUID, DESC_UUID, SVC_UUID};

pub const HEADSET: &str = "11:22:33:44:55:66";
pub const ANON: &str = "AA:BB:CC:DD:EE:FF";

/// A mock BlueZ on a private bus, with `DBUS_SYSTEM_BUS_ADDRESS` pointing at
/// it. Holds a global lock: tests using it run one at a time.
pub struct Mock {
    // Field order: the mock goes before its runtime.
    pub bz: MockBluez,
    pub rt: tokio::runtime::Runtime,
    _serial: MutexGuard<'static, ()>,
}

static SERIAL: Mutex<()> = Mutex::new(());

impl Mock {
    pub fn start() -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let bz = rt.block_on(MockBluez::start());
        // Only one test runs at a time (SERIAL) and the library reads the
        // variable only while a test calls it.
        std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", &bz.bus.address);
        Mock {
            bz,
            rt,
            _serial: serial,
        }
    }

    pub fn address(&self) -> String {
        self.bz.bus.address.clone()
    }

    pub fn device_path(&self, address: &str) -> String {
        bluebus::device_path(ADAPTER0, address)
    }

    /// The battery level characteristic of a connected mock device.
    pub fn chr_path(&self, address: &str) -> String {
        format!("{}/service0001/char0002", self.device_path(address))
    }

    pub fn remote_notify(&self, address: &str, value: Vec<u8>) {
        let path = self.chr_path(address);
        self.rt
            .block_on(common::remote_notify(&self.bz.server, &path, value));
    }

    pub fn remote_notifying(&self, address: &str) -> bool {
        let path = self.chr_path(address);
        self.rt
            .block_on(common::remote_notifying(&self.bz.server, &path))
    }

    pub fn set_rssi(&self, address: &str, rssi: i16) {
        let path = self.device_path(address);
        self.rt
            .block_on(common::set_device_rssi(&self.bz.server, &path, rssi));
    }

    /// (owner, object path) of the local characteristic `uuid` registered
    /// by a peripheral.
    pub fn local_characteristic(&self, uuid: &str) -> Option<(String, String)> {
        let st = self.bz.state.lock().unwrap();
        for ((owner, _), objects) in &st.gatt_apps {
            for (path, ifaces) in objects {
                let Some(chr) = ifaces.get("org.bluez.GattCharacteristic1") else {
                    continue;
                };
                let u = String::try_from(chr["UUID"].try_clone().unwrap()).unwrap();
                if u == uuid {
                    return Some((owner.clone(), path.to_string()));
                }
            }
        }
        None
    }

    /// A remote central writing a local characteristic through "BlueZ".
    pub fn remote_write(&self, uuid: &str, value: &[u8]) {
        let (owner, path) = self.local_characteristic(uuid).expect("characteristic");
        let server = self.bz.server.clone();
        let value = value.to_vec();
        self.rt.block_on(async move {
            let proxy = zbus::Proxy::new(&server, owner, path, "org.bluez.GattCharacteristic1")
                .await
                .unwrap();
            let () = proxy
                .call("WriteValue", &(value, HashMap::<String, OwnedValue>::new()))
                .await
                .unwrap();
        });
    }

    /// A remote central reading a local characteristic.
    pub fn remote_read(&self, uuid: &str) -> Vec<u8> {
        let (owner, path) = self.local_characteristic(uuid).expect("characteristic");
        let server = self.bz.server.clone();
        self.rt.block_on(async move {
            let proxy = zbus::Proxy::new(&server, owner, path, "org.bluez.GattCharacteristic1")
                .await
                .unwrap();
            proxy
                .call("ReadValue", &(HashMap::<String, OwnedValue>::new(),))
                .await
                .unwrap()
        })
    }
}

/// Polls `check` until it is true (panics after 5 s).
pub fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
