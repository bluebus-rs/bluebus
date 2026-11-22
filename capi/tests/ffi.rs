//! The C functions, called from Rust against the mock BlueZ. The mock is
//! reached through `DBUS_SYSTEM_BUS_ADDRESS`, so this binary has its own
//! process environment (and serialises its tests, see `support::Mock`).

#[path = "../src/lib.rs"]
#[allow(dead_code, unused_imports)]
mod ffi;
mod support;

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::mem::MaybeUninit;
use std::ptr::{self, null, null_mut};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ffi::bluebus_status::{self, *};
use ffi::*;
use support::*;

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(bluebus_last_error()) }
        .to_string_lossy()
        .into_owned()
}

fn last_dbus_name() -> Option<String> {
    let p = bluebus_last_dbus_error_name();
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

#[track_caller]
fn ok(status: bluebus_status) {
    assert_eq!(status, BLUEBUS_OK, "{}", last_error());
}

unsafe fn string(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
}

unsafe fn take_bytes(data: *mut u8, len: usize) -> Vec<u8> {
    let v = if data.is_null() {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data, len).to_vec()
    };
    bluebus_bytes_free(data);
    v
}

struct Infos(Vec<(String, Option<String>, Option<i16>)>);

unsafe fn take_infos(p: *mut bluebus_device_info, n: usize) -> Infos {
    let v = (0..n)
        .map(|k| {
            let d = &*p.add(k);
            (
                string(d.address).unwrap(),
                string(d.name),
                d.has_rssi.then_some(d.rssi),
            )
        })
        .collect();
    bluebus_device_info_free(p, n);
    Infos(v)
}

/// Session, powered hci0, and the headset found and connected.
struct Central {
    session: *mut bluebus_session,
    adapter: *mut bluebus_adapter,
    device: *mut bluebus_device,
}

impl Central {
    unsafe fn new(connect: bool) -> Self {
        let mut session = null_mut();
        ok(bluebus_session_new(&mut session));
        let mut adapter = null_mut();
        ok(bluebus_session_default_adapter(session, &mut adapter));
        ok(bluebus_adapter_power_on(adapter));
        let name = cs("Headset");
        let filter = bluebus_scan_filter {
            name: name.as_ptr(),
            ..zero_filter()
        };
        let mut device = null_mut();
        ok(bluebus_adapter_find_device(
            adapter,
            &filter,
            5000,
            &mut device,
        ));
        if connect {
            ok(bluebus_device_connect(device, 5000));
        }
        Central {
            session,
            adapter,
            device,
        }
    }
}

impl Drop for Central {
    fn drop(&mut self) {
        unsafe {
            bluebus_device_free(self.device);
            bluebus_adapter_free(self.adapter);
            bluebus_session_free(self.session);
        }
    }
}

fn zero_filter() -> bluebus_scan_filter {
    bluebus_scan_filter {
        transport: 0,
        name: null(),
        name_prefix: null(),
        address: null(),
        service_uuids: null(),
        service_uuid_count: 0,
        min_rssi: 0,
        has_min_rssi: false,
        duplicate_data: false,
        has_duplicate_data: false,
    }
}

// ---------------------------------------------------------------------------

#[test]
fn version_status_and_uuid() {
    let v = unsafe { CStr::from_ptr(bluebus_version()) }
        .to_str()
        .unwrap();
    assert_eq!(v, env!("CARGO_PKG_VERSION"));
    let s = unsafe { CStr::from_ptr(bluebus_status_str(BLUEBUS_ERR_TIMEOUT as c_int)) };
    assert_eq!(s.to_str().unwrap(), "timeout");
    let s = unsafe { CStr::from_ptr(bluebus_status_str(1234)) };
    assert_eq!(s.to_str().unwrap(), "unknown status");

    let mut buf = [0 as c_char; 37];
    unsafe {
        ok(bluebus_normalize_uuid(
            cs("0x2A19").as_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
        ));
        assert_eq!(
            CStr::from_ptr(buf.as_ptr()).to_str().unwrap(),
            "00002a19-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            bluebus_normalize_uuid(cs("2a19").as_ptr(), buf.as_mut_ptr(), 10),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            bluebus_normalize_uuid(cs("xyz").as_ptr(), buf.as_mut_ptr(), buf.len()),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert!(last_error().contains("xyz"), "{}", last_error());
        assert_eq!(last_dbus_name(), None);
    }
}

#[test]
fn null_pointers_are_rejected() {
    unsafe {
        assert_eq!(
            bluebus_session_new(null_mut()),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert!(last_error().contains("session is NULL"), "{}", last_error());
        let mut adapter = null_mut();
        assert_eq!(
            bluebus_session_default_adapter(null(), &mut adapter),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert!(adapter.is_null());
        assert_eq!(
            bluebus_adapter_power_on(null()),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            bluebus_device_connect(null(), 10),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        let (mut data, mut len) = (null_mut(), 0usize);
        assert_eq!(
            bluebus_device_read(null(), cs("2a19").as_ptr(), &mut data, &mut len),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            bluebus_notifications_next(null(), 0, &mut data, &mut len),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert_eq!(bluebus_scan_stop(null()), BLUEBUS_ERR_INVALID_ARGUMENT);
        assert_eq!(
            bluebus_peripheral_stop(null()),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert!(!bluebus_peripheral_is_running(null()));
        assert!(bluebus_adapter_name(null()).is_null());
        assert!(bluebus_device_address(null()).is_null());
        // Free functions accept NULL.
        bluebus_session_free(null_mut());
        bluebus_adapter_free(null_mut());
        bluebus_device_free(null_mut());
        bluebus_scan_free(null_mut());
        bluebus_notifications_free(null_mut());
        bluebus_peripheral_builder_free(null_mut());
        bluebus_peripheral_free(null_mut());
        bluebus_bytes_free(null_mut());
        bluebus_device_info_free(null_mut(), 3);
        bluebus_adapter_info_free(null_mut(), 0);
        bluebus_gatt_attributes_free(null_mut(), 1);
    }
}

#[test]
fn errors_are_per_thread() {
    unsafe {
        assert_eq!(
            bluebus_session_new(null_mut()),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
    }
    let other = std::thread::spawn(last_error).join().unwrap();
    assert_eq!(other, "");
    assert!(!last_error().is_empty());
}

#[test]
fn session_and_adapters() {
    let mock = Mock::start();
    unsafe {
        let mut session = null_mut();
        ok(bluebus_session_new(&mut session));

        // MaybeUninit out-parameters: the C call fills them.
        let (mut infos, mut n) = (MaybeUninit::uninit(), 0usize);
        ok(bluebus_session_adapters(
            session,
            infos.as_mut_ptr(),
            &mut n,
        ));
        let infos = infos.assume_init();
        assert_eq!(n, 2);
        let names: Vec<_> = (0..n)
            .map(|k| string((*infos.add(k)).name).unwrap())
            .collect();
        assert_eq!(names, ["hci0", "hci1"]);
        assert_eq!(string((*infos).path).unwrap(), ADAPTER0);
        assert_eq!(string((*infos).address).unwrap(), "00:AA:00:AA:00:AA");
        assert!(!(*infos).powered);
        bluebus_adapter_info_free(infos, n);

        let mut a = null_mut();
        ok(bluebus_session_default_adapter(session, &mut a));
        assert_eq!(string(bluebus_adapter_name(a)).unwrap(), "hci0");
        assert_eq!(string(bluebus_adapter_path(a)).unwrap(), ADAPTER0);
        let mut b = null_mut();
        ok(bluebus_session_adapter(
            session,
            cs("00:bb:00:bb:00:bb").as_ptr(),
            &mut b,
        ));
        assert_eq!(string(bluebus_adapter_name(b)).unwrap(), "hci1");
        assert_eq!(
            string(bluebus_adapter_address(b)).unwrap(),
            "00:BB:00:BB:00:BB"
        );

        let mut c = null_mut();
        assert_eq!(
            bluebus_session_adapter(session, cs("hci9").as_ptr(), &mut c),
            BLUEBUS_ERR_NOT_FOUND
        );
        assert!(c.is_null());
        assert!(last_error().contains("hci9"), "{}", last_error());

        // Power and a D-Bus error with its name.
        let mut powered = true;
        ok(bluebus_adapter_is_powered(b, &mut powered));
        assert!(!powered);
        let (mut found, mut count) = (null_mut(), 0usize);
        assert_eq!(
            bluebus_adapter_scan(b, null(), 10, &mut found, &mut count),
            BLUEBUS_ERR_DBUS
        );
        assert_eq!(
            last_dbus_name().as_deref(),
            Some("org.bluez.Error.NotReady")
        );
        ok(bluebus_adapter_power_on(b));
        ok(bluebus_adapter_is_powered(b, &mut powered));
        assert!(powered);
        ok(bluebus_adapter_power_off(b));
        ok(bluebus_adapter_is_powered(b, &mut powered));
        assert!(!powered);

        // Known devices of hci1, and removal.
        ok(bluebus_adapter_devices(b, &mut found, &mut count));
        let Infos(list) = take_infos(found, count);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "01:02:03:04:05:06");
        assert_eq!(list[0].1.as_deref(), Some("Other"));
        ok(bluebus_adapter_remove_device(
            b,
            cs("01:02:03:04:05:06").as_ptr(),
        ));
        ok(bluebus_adapter_devices(b, &mut found, &mut count));
        assert_eq!(count, 0);
        assert!(found.is_null());
        assert_eq!(
            bluebus_adapter_remove_device(b, cs("not an address").as_ptr()),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );

        bluebus_adapter_free(a);
        bluebus_adapter_free(b);
        bluebus_session_free(session);
    }
    drop(mock);
}

#[test]
fn session_without_bus_fails() {
    let _mock = Mock::start();
    std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent/bluebus");
    unsafe {
        let mut session = null_mut();
        assert_eq!(bluebus_session_new(&mut session), BLUEBUS_ERR_DBUS);
        assert!(session.is_null());
        assert!(!last_error().is_empty());
    }
}

#[test]
fn blocking_scan_and_filters() {
    let _mock = Mock::start();
    unsafe {
        let mut session = null_mut();
        ok(bluebus_session_new(&mut session));
        let mut adapter = null_mut();
        ok(bluebus_session_default_adapter(session, &mut adapter));
        ok(bluebus_adapter_power_on(adapter));

        let (mut found, mut n) = (MaybeUninit::uninit(), 0usize);
        ok(bluebus_adapter_scan(
            adapter,
            null(),
            100,
            found.as_mut_ptr(),
            &mut n,
        ));
        let mut found = found.assume_init();
        assert_eq!(n, 2);
        {
            let d = &*found;
            assert_eq!(string(d.address).unwrap(), HEADSET);
            assert_eq!(string(d.name).as_deref(), Some("Headset"));
            assert_eq!(string(d.alias).as_deref(), Some("Headset"));
            assert!(d.has_rssi);
            assert_eq!(d.rssi, -48);
            assert!(!d.connected && !d.paired);
            let anon = &*found.add(1);
            assert_eq!(string(anon.address).unwrap(), ANON);
            assert!(anon.name.is_null());
        }
        bluebus_device_info_free(found, n);

        let prefix = cs("Head");
        let filter = bluebus_scan_filter {
            name_prefix: prefix.as_ptr(),
            transport: 3,
            ..zero_filter()
        };
        ok(bluebus_adapter_scan(
            adapter, &filter, 100, &mut found, &mut n,
        ));
        let Infos(list) = take_infos(found, n);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, HEADSET);

        let filter = bluebus_scan_filter {
            min_rssi: -60,
            has_min_rssi: true,
            has_duplicate_data: true,
            duplicate_data: false,
            ..zero_filter()
        };
        ok(bluebus_adapter_scan(
            adapter, &filter, 100, &mut found, &mut n,
        ));
        assert_eq!(take_infos(found, n).0.len(), 1);

        let name = cs("Headset");
        let both = bluebus_scan_filter {
            name: name.as_ptr(),
            name_prefix: prefix.as_ptr(),
            ..zero_filter()
        };
        assert_eq!(
            bluebus_adapter_scan(adapter, &both, 10, &mut found, &mut n),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        let bad = bluebus_scan_filter {
            transport: 9,
            ..zero_filter()
        };
        assert_eq!(
            bluebus_adapter_scan(adapter, &bad, 10, &mut found, &mut n),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        let uuids = [null::<c_char>()];
        let null_uuid = bluebus_scan_filter {
            service_uuids: uuids.as_ptr(),
            service_uuid_count: 1,
            ..zero_filter()
        };
        assert_eq!(
            bluebus_adapter_scan(adapter, &null_uuid, 10, &mut found, &mut n),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );

        // find_device times out when nothing matches.
        let nobody = cs("Nobody");
        let filter = bluebus_scan_filter {
            name: nobody.as_ptr(),
            ..zero_filter()
        };
        let mut device = null_mut();
        assert_eq!(
            bluebus_adapter_find_device(adapter, &filter, 100, &mut device),
            BLUEBUS_ERR_TIMEOUT
        );
        assert!(device.is_null());

        bluebus_adapter_free(adapter);
        bluebus_session_free(session);
    }
}

struct ScanLog {
    events: Mutex<Vec<(String, c_int)>>,
    scan: AtomicPtr<bluebus_scan>,
    stops: AtomicUsize,
}

unsafe extern "C" fn on_scan(d: *const bluebus_device_info, kind: c_int, user: *mut c_void) {
    let log = &*(user as *const ScanLog);
    log.events
        .lock()
        .unwrap()
        .push((string((*d).address).unwrap(), kind));
    // Stop the scan from its own callback once the handle is known and an
    // update arrived.
    let scan = log.scan.load(Ordering::SeqCst);
    if kind == 1 && !scan.is_null() {
        assert_eq!(bluebus_scan_stop(scan), BLUEBUS_OK);
        log.stops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn callback_scan() {
    let mock = Mock::start();
    unsafe {
        let mut session = null_mut();
        ok(bluebus_session_new(&mut session));
        let mut adapter = null_mut();
        ok(bluebus_session_default_adapter(session, &mut adapter));
        ok(bluebus_adapter_power_on(adapter));

        let log = Arc::new(ScanLog {
            events: Mutex::new(Vec::new()),
            scan: AtomicPtr::new(null_mut()),
            stops: AtomicUsize::new(0),
        });
        let mut scan = null_mut();
        assert_eq!(
            bluebus_adapter_scan_start(adapter, null(), None, null_mut(), &mut scan),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        ok(bluebus_adapter_scan_start(
            adapter,
            null(),
            Some(on_scan),
            Arc::as_ptr(&log) as *mut c_void,
            &mut scan,
        ));
        eventually("two devices found", || {
            log.events.lock().unwrap().len() >= 2
        });
        {
            let mut ev = log.events.lock().unwrap().clone();
            ev.sort();
            assert_eq!(ev, [(HEADSET.into(), 0), (ANON.into(), 0)]);
        }
        // A second scan on the same adapter and session is refused.
        let mut second = null_mut();
        assert_eq!(
            bluebus_adapter_scan_start(
                adapter,
                null(),
                Some(on_scan),
                Arc::as_ptr(&log) as *mut c_void,
                &mut second
            ),
            BLUEBUS_ERR_DBUS
        );
        assert_eq!(
            last_dbus_name().as_deref(),
            Some("org.bluez.Error.InProgress")
        );

        // An update; the callback stops its own scan.
        log.scan.store(scan, Ordering::SeqCst);
        mock.set_rssi(HEADSET, -30);
        eventually("stop from the callback", || {
            log.stops.load(Ordering::SeqCst) == 1
        });
        ok(bluebus_scan_stop(scan)); // waits for the thread; twice is fine
        ok(bluebus_scan_stop(scan));
        let mut discovering = true;
        eventually("discovery stopped", || {
            let (mut infos, mut n) = (MaybeUninit::uninit(), 0usize);
            ok(bluebus_session_adapters(
                session,
                infos.as_mut_ptr(),
                &mut n,
            ));
            let infos = infos.assume_init();
            discovering = (*infos).discovering;
            bluebus_adapter_info_free(infos, n);
            !discovering
        });
        let n = log.events.lock().unwrap().len();
        mock.set_rssi(HEADSET, -20);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(log.events.lock().unwrap().len(), n, "no events after stop");
        bluebus_scan_free(scan);

        // A new scan works after the old one is freed.
        let fresh = Arc::new(ScanLog {
            events: Mutex::new(Vec::new()),
            scan: AtomicPtr::new(null_mut()),
            stops: AtomicUsize::new(0),
        });
        ok(bluebus_adapter_scan_start(
            adapter,
            null(),
            Some(on_scan),
            Arc::as_ptr(&fresh) as *mut c_void,
            &mut scan,
        ));
        eventually("devices again", || !fresh.events.lock().unwrap().is_empty());
        bluebus_scan_free(scan); // stops without bluebus_scan_stop

        bluebus_adapter_free(adapter);
        bluebus_session_free(session);
    }
}

#[test]
fn connect_read_write_services() {
    let mock = Mock::start();
    unsafe {
        let c = Central::new(false);
        assert_eq!(string(bluebus_device_address(c.device)).unwrap(), HEADSET);
        let mut connected = true;
        ok(bluebus_device_is_connected(c.device, &mut connected));
        assert!(!connected);
        ok(bluebus_device_connect(c.device, 5000));
        ok(bluebus_device_is_connected(c.device, &mut connected));
        assert!(connected);

        let mut info = MaybeUninit::uninit();
        ok(bluebus_device_get_info(c.device, info.as_mut_ptr()));
        let mut info = info.assume_init();
        assert!((*info).connected && (*info).services_resolved);
        assert_eq!(string((*info).address_type).as_deref(), Some("public"));
        bluebus_device_info_free(info, 1);

        let (mut data, mut len) = (null_mut(), 0usize);
        ok(bluebus_device_read(
            c.device,
            cs("2a19").as_ptr(),
            &mut data,
            &mut len,
        ));
        assert_eq!(take_bytes(data, len), [87]);
        let mut level = 0u8;
        ok(bluebus_device_battery_level(c.device, &mut level));
        assert_eq!(level, 87);
        let (mut rssi, mut has) = (0i16, false);
        ok(bluebus_device_rssi(c.device, &mut rssi, &mut has));
        assert!(has);
        assert_eq!(rssi, -48);

        assert_eq!(
            bluebus_device_read(c.device, cs("2a37").as_ptr(), &mut data, &mut len),
            BLUEBUS_ERR_NOT_FOUND
        );
        assert_eq!(
            bluebus_device_read(c.device, cs("zz").as_ptr(), &mut data, &mut len),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );

        ok(bluebus_device_write(
            c.device,
            cs("2a19").as_ptr(),
            [1, 2].as_ptr(),
            2,
            true,
        ));
        ok(bluebus_device_write(
            c.device,
            cs("2a19").as_ptr(),
            [3].as_ptr(),
            1,
            false,
        ));
        assert_eq!(
            bluebus_device_write(c.device, cs("2a19").as_ptr(), null(), 4, true),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        {
            let st = mock.bz.state.lock().unwrap();
            let w: Vec<_> = st
                .remote_writes
                .iter()
                .map(|(_, v, t)| (v.clone(), t.clone()))
                .collect();
            assert_eq!(
                w,
                [
                    (vec![1, 2], Some("request".to_owned())),
                    (vec![3], Some("command".to_owned()))
                ]
            );
        }
        ok(bluebus_device_read(
            c.device,
            cs("2a19").as_ptr(),
            &mut data,
            &mut len,
        ));
        assert_eq!(take_bytes(data, len), [3]);

        ok(bluebus_device_read_descriptor(
            c.device,
            cs("2a19").as_ptr(),
            cs("2902").as_ptr(),
            &mut data,
            &mut len,
        ));
        assert_eq!(take_bytes(data, len), [0, 0]);
        ok(bluebus_device_write_descriptor(
            c.device,
            cs("2a19").as_ptr(),
            cs("2902").as_ptr(),
            [1, 0].as_ptr(),
            2,
        ));

        let (mut attrs, mut n) = (null_mut(), 0usize);
        ok(bluebus_device_services(c.device, &mut attrs, &mut n));
        let list: Vec<_> = (0..n)
            .map(|k| {
                let a = &*attrs.add(k);
                (
                    a.kind,
                    string(a.uuid).unwrap(),
                    string(a.service_uuid).unwrap(),
                    string(a.characteristic_uuid),
                    a.flags,
                )
            })
            .collect();
        bluebus_gatt_attributes_free(attrs, n);
        let rwn = (1 << 1) | (1 << 3) | (1 << 4);
        assert_eq!(
            list,
            [
                (0, SVC_UUID.into(), SVC_UUID.into(), None, 0),
                (1, CHR_UUID.into(), SVC_UUID.into(), None, rwn),
                (
                    2,
                    DESC_UUID.into(),
                    SVC_UUID.into(),
                    Some(CHR_UUID.into()),
                    list[2].4
                ),
            ]
        );

        // Pairing with the auto-accept agent, trust.
        assert_eq!(
            bluebus_session_set_agent_passkey(c.session, 1_000_000),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        ok(bluebus_session_set_agent_passkey(c.session, 123456));
        ok(bluebus_session_set_agent_auto(c.session));
        assert_eq!(mock.bz.state.lock().unwrap().agents.len(), 1);
        ok(bluebus_device_pair(c.device));
        assert_eq!(mock.bz.state.lock().unwrap().pair_requests.len(), 1);
        ok(bluebus_device_trust(c.device, true));
        ok(bluebus_device_get_info(c.device, &mut info));
        assert!((*info).paired && (*info).trusted);
        bluebus_device_info_free(info, 1);
        ok(bluebus_device_pair(c.device)); // already paired: no-op
        assert_eq!(mock.bz.state.lock().unwrap().pair_requests.len(), 1);
        ok(bluebus_session_remove_agent(c.session));
        ok(bluebus_session_remove_agent(c.session));
        assert!(mock.bz.state.lock().unwrap().agents.is_empty());

        ok(bluebus_device_disconnect(c.device));
        ok(bluebus_device_is_connected(c.device, &mut connected));
        assert!(!connected);

        // Unknown device handle: created, but operations fail.
        let mut ghost = null_mut();
        ok(bluebus_adapter_device(
            c.adapter,
            cs("01:01:01:01:01:01").as_ptr(),
            &mut ghost,
        ));
        assert_eq!(bluebus_device_connect(ghost, 100), BLUEBUS_ERR_NOT_FOUND);
        bluebus_device_free(ghost);
        assert_eq!(
            bluebus_adapter_device(c.adapter, cs("nope").as_ptr(), &mut ghost),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
    }
}

struct Received(Mutex<Vec<Vec<u8>>>);

unsafe extern "C" fn on_value(data: *const u8, len: usize, user: *mut c_void) {
    let r = &*(user as *const Received);
    r.0.lock()
        .unwrap()
        .push(std::slice::from_raw_parts(data, len).to_vec());
}

#[test]
fn notifications_callback_and_pull() {
    let mock = Mock::start();
    unsafe {
        let c = Central::new(true);
        let received = Arc::new(Received(Mutex::new(Vec::new())));
        let mut sub = null_mut();
        ok(bluebus_device_subscribe(
            c.device,
            cs("2a19").as_ptr(),
            Some(on_value),
            Arc::as_ptr(&received) as *mut c_void,
            &mut sub,
        ));
        assert!(mock.remote_notifying(HEADSET));
        mock.remote_notify(HEADSET, vec![50]);
        mock.remote_notify(HEADSET, vec![49]);
        eventually("two notifications", || {
            received.0.lock().unwrap().len() == 2
        });
        assert_eq!(*received.0.lock().unwrap(), [vec![50], vec![49]]);

        // A pull subscription on the same characteristic shares it.
        let mut pull = null_mut();
        ok(bluebus_device_notifications(
            c.device,
            cs("2a19").as_ptr(),
            &mut pull,
        ));
        let (mut data, mut len) = (null_mut(), 0usize);
        assert_eq!(
            bluebus_notifications_next(pull, 20, &mut data, &mut len),
            BLUEBUS_ERR_TIMEOUT
        );
        mock.remote_notify(HEADSET, vec![48]);
        ok(bluebus_notifications_next(pull, 2000, &mut data, &mut len));
        assert_eq!(take_bytes(data, len), [48]);
        eventually("callback got it too", || {
            received.0.lock().unwrap().len() == 3
        });
        // Queued while nobody waits.
        mock.remote_notify(HEADSET, vec![47]);
        std::thread::sleep(std::time::Duration::from_millis(50));
        ok(bluebus_notifications_next(
            pull,
            BLUEBUS_WAIT_FOREVER_U32,
            &mut data,
            &mut len,
        ));
        assert_eq!(take_bytes(data, len), [47]);
        assert_eq!(
            bluebus_notifications_next(sub, 0, &mut data, &mut len),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );

        // Stopping the callback subscription keeps notifications on for the
        // pull one; stopping both turns them off.
        ok(bluebus_notifications_stop(sub));
        ok(bluebus_notifications_stop(sub));
        let n = received.0.lock().unwrap().len();
        assert!(mock.remote_notifying(HEADSET));
        mock.remote_notify(HEADSET, vec![46]);
        ok(bluebus_notifications_next(pull, 2000, &mut data, &mut len));
        assert_eq!(take_bytes(data, len), [46]);
        assert_eq!(received.0.lock().unwrap().len(), n);
        ok(bluebus_notifications_stop(pull));
        assert!(!mock.remote_notifying(HEADSET));
        assert_eq!(
            bluebus_notifications_next(pull, 10, &mut data, &mut len),
            BLUEBUS_ERR_CLOSED
        );
        bluebus_notifications_free(sub);
        bluebus_notifications_free(pull);

        // Free without stop turns them off too.
        ok(bluebus_device_notifications(
            c.device,
            cs("2a19").as_ptr(),
            &mut pull,
        ));
        assert!(mock.remote_notifying(HEADSET));
        bluebus_notifications_free(pull);
        eventually("notify off", || !mock.remote_notifying(HEADSET));

        assert_eq!(
            bluebus_device_subscribe(c.device, cs("2a19").as_ptr(), None, null_mut(), &mut sub),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
    }
}

const BLUEBUS_WAIT_FOREVER_U32: u32 = u32::MAX;

#[test]
fn handles_are_thread_safe() {
    let _mock = Mock::start();
    unsafe {
        let c = Central::new(true);
        let device = c.device as usize;
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        let (mut data, mut len) = (null_mut(), 0usize);
                        let s = bluebus_device_read(
                            device as *const bluebus_device,
                            c"2a19".as_ptr(),
                            &mut data,
                            &mut len,
                        );
                        assert_eq!(s, BLUEBUS_OK);
                        assert_eq!(take_bytes(data, len), [87]);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }
}

struct Writes(Mutex<Vec<(usize, Vec<u8>)>>);

unsafe extern "C" fn on_write_a(data: *const u8, len: usize, user: *mut c_void) {
    let w = &*(user as *const Writes);
    w.0.lock()
        .unwrap()
        .push((0, std::slice::from_raw_parts(data, len).to_vec()));
}

unsafe extern "C" fn on_write_b(data: *const u8, len: usize, user: *mut c_void) {
    let w = &*(user as *const Writes);
    w.0.lock()
        .unwrap()
        .push((1, std::slice::from_raw_parts(data, len).to_vec()));
}

const SVC: &str = "12345678-1234-5678-1234-56789abcdef0";
const CTL: &str = "12345678-1234-5678-1234-56789abcdef1";

#[test]
fn peripheral() {
    let mock = Mock::start();
    unsafe {
        let mut session = null_mut();
        ok(bluebus_session_new(&mut session));
        let mut adapter = null_mut();
        ok(bluebus_session_default_adapter(session, &mut adapter));
        ok(bluebus_adapter_power_on(adapter));

        let mut b = null_mut();
        ok(bluebus_peripheral_builder_new(adapter, &mut b));
        let mut p = null_mut();
        assert_eq!(
            bluebus_peripheral_builder_start(b, &mut p),
            BLUEBUS_ERR_INVALID_ARGUMENT,
            "nothing to publish"
        );
        ok(bluebus_peripheral_builder_add_service(
            b,
            cs("180f").as_ptr(),
            true,
        ));
        assert_eq!(
            bluebus_peripheral_builder_add_service(b, cs("0x180F").as_ptr(), true),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        ok(bluebus_peripheral_builder_add_service(
            b,
            cs(SVC).as_ptr(),
            true,
        ));
        let writes = Arc::new(Writes(Mutex::new(Vec::new())));
        let user = Arc::as_ptr(&writes) as *mut c_void;
        let read_notify = (1 << 1) | (1 << 4);
        ok(bluebus_peripheral_builder_add_characteristic(
            b,
            cs("180f").as_ptr(),
            cs("2a19").as_ptr(),
            read_notify | (1 << 3),
            [100].as_ptr(),
            1,
            Some(on_write_a),
            user,
        ));
        ok(bluebus_peripheral_builder_add_characteristic(
            b,
            cs(SVC).as_ptr(),
            cs(CTL).as_ptr(),
            (1 << 3) | (1 << 2),
            null(),
            0,
            Some(on_write_b),
            user,
        ));
        ok(bluebus_peripheral_builder_add_descriptor(
            b,
            cs("2a19").as_ptr(),
            cs("2901").as_ptr(),
            b"level".as_ptr(),
            5,
        ));
        assert_eq!(
            bluebus_peripheral_builder_add_characteristic(
                b,
                cs("1234").as_ptr(),
                cs("2a00").as_ptr(),
                2,
                null(),
                0,
                None,
                null_mut()
            ),
            BLUEBUS_ERR_NOT_FOUND
        );
        assert_eq!(
            bluebus_peripheral_builder_add_characteristic(
                b,
                cs("180f").as_ptr(),
                cs("2a00").as_ptr(),
                1 << 31,
                null(),
                0,
                None,
                null_mut()
            ),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            bluebus_peripheral_builder_add_descriptor(
                b,
                cs("2a99").as_ptr(),
                cs("2901").as_ptr(),
                null(),
                0
            ),
            BLUEBUS_ERR_NOT_FOUND
        );
        let uuids = [cs("180f"), cs(SVC)];
        let uuid_ptrs: Vec<*const c_char> = uuids.iter().map(|u| u.as_ptr()).collect();
        ok(bluebus_peripheral_builder_advertise(
            b,
            cs("bluebus-c").as_ptr(),
            uuid_ptrs.as_ptr(),
            uuid_ptrs.len(),
        ));
        ok(bluebus_peripheral_builder_advertise_manufacturer_data(
            b,
            0xffff,
            [1, 2, 3].as_ptr(),
            3,
        ));

        ok(bluebus_peripheral_builder_start(b, &mut p));
        assert!(bluebus_peripheral_is_running(p));
        {
            let st = mock.bz.state.lock().unwrap();
            assert_eq!(st.gatt_apps.len(), 1);
            assert_eq!(st.advertisements.len(), 1);
            let adv = st.advertisements.values().next().unwrap();
            assert_eq!(
                String::try_from(adv["LocalName"].try_clone().unwrap()).unwrap(),
                "bluebus-c"
            );
            assert!(adv.contains_key("ManufacturerData"));
        }
        assert_eq!(mock.remote_read(CHR_UUID), [100]);

        // Remote writes reach the C callbacks (on the library thread).
        mock.remote_write(CTL, b"go");
        mock.remote_write(CHR_UUID, &[7]);
        eventually("two writes", || writes.0.lock().unwrap().len() == 2);
        assert_eq!(
            *writes.0.lock().unwrap(),
            [(1, b"go".to_vec()), (0, vec![7])]
        );
        let (mut data, mut len) = (null_mut(), 0usize);
        ok(bluebus_peripheral_value(
            p,
            cs(CTL).as_ptr(),
            &mut data,
            &mut len,
        ));
        assert_eq!(take_bytes(data, len), b"go");

        ok(bluebus_peripheral_set_value(
            p,
            cs("2a19").as_ptr(),
            [90].as_ptr(),
            1,
        ));
        assert_eq!(mock.remote_read(CHR_UUID), [90]);
        ok(bluebus_peripheral_notify(
            p,
            cs("2a19").as_ptr(),
            [80].as_ptr(),
            1,
        ));
        ok(bluebus_peripheral_value(
            p,
            cs("0x2A19").as_ptr(),
            &mut data,
            &mut len,
        ));
        assert_eq!(take_bytes(data, len), [80]);
        let mut notifying = true;
        ok(bluebus_peripheral_is_notifying(
            p,
            cs("2a19").as_ptr(),
            &mut notifying,
        ));
        assert!(!notifying);
        assert_eq!(
            bluebus_peripheral_notify(p, cs("2a37").as_ptr(), [1].as_ptr(), 1),
            BLUEBUS_ERR_NOT_FOUND
        );

        // The builder can start a second peripheral.
        let mut p2 = null_mut();
        ok(bluebus_peripheral_builder_start(b, &mut p2));
        assert_eq!(mock.bz.state.lock().unwrap().gatt_apps.len(), 2);
        bluebus_peripheral_free(p2);
        eventually("second one gone", || {
            mock.bz.state.lock().unwrap().gatt_apps.len() == 1
        });

        ok(bluebus_peripheral_stop(p));
        ok(bluebus_peripheral_stop(p));
        assert!(!bluebus_peripheral_is_running(p));
        {
            let st = mock.bz.state.lock().unwrap();
            assert!(st.gatt_apps.is_empty() && st.advertisements.is_empty());
        }
        assert_eq!(
            bluebus_peripheral_set_value(p, cs("2a19").as_ptr(), [1].as_ptr(), 1),
            BLUEBUS_ERR_INVALID_ARGUMENT
        );
        bluebus_peripheral_free(p);
        bluebus_peripheral_builder_free(b);

        // An advertisement-only peripheral (broadcast data, no services).
        ok(bluebus_peripheral_builder_new(adapter, &mut b));
        ok(bluebus_peripheral_builder_advertise(b, null(), null(), 0));
        ok(bluebus_peripheral_builder_start(b, &mut p));
        {
            let st = mock.bz.state.lock().unwrap();
            assert!(st.gatt_apps.is_empty());
            assert_eq!(st.advertisements.len(), 1);
        }
        bluebus_peripheral_free(p);
        bluebus_peripheral_builder_free(b);
        assert!(mock.bz.state.lock().unwrap().advertisements.is_empty());

        bluebus_adapter_free(adapter);
        bluebus_session_free(session);
    }
    let _ = ptr::null::<u8>();
}
