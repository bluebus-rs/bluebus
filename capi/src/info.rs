//! Plain data handed to C: device, adapter and GATT attribute descriptions,
//! and the characteristic flag bits.

use std::ffi::{c_char, c_int, CString};
use std::ptr;

use bluebus::ble;

use super::{c_string, quiet};

// ---------------------------------------------------------------------------
// Owned C strings and arrays
// ---------------------------------------------------------------------------

pub fn leak_str(s: &str) -> *const c_char {
    c_string(s).into_raw()
}

pub fn leak_opt_str(s: Option<&str>) -> *const c_char {
    s.map_or(ptr::null(), leak_str)
}

pub unsafe fn free_str(p: *const c_char) {
    if !p.is_null() {
        drop(CString::from_raw(p as *mut c_char));
    }
}

/// A Rust vector as a C array (`NULL`, 0 when empty).
pub fn leak_vec<T>(v: Vec<T>) -> (*const T, usize) {
    if v.is_empty() {
        return (ptr::null(), 0);
    }
    let len = v.len();
    (Box::into_raw(v.into_boxed_slice()) as *const T, len)
}

pub unsafe fn take_vec<T>(p: *const T, len: usize) -> Vec<T> {
    if p.is_null() || len == 0 {
        return Vec::new();
    }
    Box::from_raw(ptr::slice_from_raw_parts_mut(p as *mut T, len)).into_vec()
}

fn leak_bytes(v: &[u8]) -> (*const u8, usize) {
    leak_vec(v.to_vec())
}

// ---------------------------------------------------------------------------
// Characteristic flags
// ---------------------------------------------------------------------------

/// `BLUEBUS_FLAG_*` bit → BlueZ flag string.
pub const FLAGS: &[(u32, &str)] = &[
    (1 << 0, "broadcast"),
    (1 << 1, "read"),
    (1 << 2, "write-without-response"),
    (1 << 3, "write"),
    (1 << 4, "notify"),
    (1 << 5, "indicate"),
    (1 << 6, "authenticated-signed-writes"),
    (1 << 7, "extended-properties"),
    (1 << 8, "reliable-write"),
    (1 << 9, "writable-auxiliaries"),
    (1 << 10, "encrypt-read"),
    (1 << 11, "encrypt-write"),
    (1 << 12, "encrypt-notify"),
    (1 << 13, "encrypt-indicate"),
    (1 << 14, "encrypt-authenticated-read"),
    (1 << 15, "encrypt-authenticated-write"),
    (1 << 16, "secure-read"),
    (1 << 17, "secure-write"),
    (1 << 18, "authorize"),
];

pub fn flags_to_bits(flags: &[String]) -> u32 {
    flags
        .iter()
        .filter_map(|f| FLAGS.iter().find(|(_, s)| s == f).map(|(b, _)| *b))
        .fold(0, |a, b| a | b)
}

/// The BlueZ flag strings of a bitmask, or the unknown bits.
pub fn bits_to_flags(bits: u32) -> Result<Vec<String>, u32> {
    let known = FLAGS.iter().fold(0, |a, (b, _)| a | b);
    if bits & !known != 0 {
        return Err(bits & !known);
    }
    Ok(FLAGS
        .iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, s)| (*s).to_owned())
        .collect())
}

// ---------------------------------------------------------------------------
// Device information
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct bluebus_manufacturer_data {
    pub company_id: u16,
    pub data: *const u8,
    pub len: usize,
}

#[repr(C)]
pub struct bluebus_service_data {
    pub uuid: *const c_char,
    pub data: *const u8,
    pub len: usize,
}

#[repr(C)]
pub struct bluebus_device_info {
    pub address: *const c_char,
    pub address_type: *const c_char,
    pub name: *const c_char,
    pub alias: *const c_char,
    pub rssi: i16,
    pub tx_power: i16,
    pub has_rssi: bool,
    pub has_tx_power: bool,
    pub connected: bool,
    pub paired: bool,
    pub trusted: bool,
    pub services_resolved: bool,
    pub uuids: *const *const c_char,
    pub uuid_count: usize,
    pub manufacturer_data: *const bluebus_manufacturer_data,
    pub manufacturer_data_count: usize,
    pub service_data: *const bluebus_service_data,
    pub service_data_count: usize,
}

impl bluebus_device_info {
    pub fn new(d: &ble::DeviceInfo) -> Self {
        let uuids: Vec<*const c_char> = d.uuids.iter().map(|u| leak_str(u)).collect();
        let (uuids, uuid_count) = leak_vec(uuids);
        let mut mfr: Vec<_> = d.manufacturer_data.iter().collect();
        mfr.sort_by_key(|(id, _)| **id);
        let mfr: Vec<_> = mfr
            .into_iter()
            .map(|(id, v)| {
                let (data, len) = leak_bytes(v);
                bluebus_manufacturer_data {
                    company_id: *id,
                    data,
                    len,
                }
            })
            .collect();
        let (manufacturer_data, manufacturer_data_count) = leak_vec(mfr);
        let mut svc: Vec<_> = d.service_data.iter().collect();
        svc.sort_by(|a, b| a.0.cmp(b.0));
        let svc: Vec<_> = svc
            .into_iter()
            .map(|(uuid, v)| {
                let (data, len) = leak_bytes(v);
                bluebus_service_data {
                    uuid: leak_str(uuid),
                    data,
                    len,
                }
            })
            .collect();
        let (service_data, service_data_count) = leak_vec(svc);
        bluebus_device_info {
            address: leak_str(&d.address),
            address_type: leak_opt_str(d.address_type.as_deref()),
            name: leak_opt_str(d.name.as_deref()),
            alias: leak_str(&d.alias),
            rssi: d.rssi.unwrap_or(0),
            tx_power: d.tx_power.unwrap_or(0),
            has_rssi: d.rssi.is_some(),
            has_tx_power: d.tx_power.is_some(),
            connected: d.connected,
            paired: d.paired,
            trusted: d.trusted,
            services_resolved: d.services_resolved,
            uuids,
            uuid_count,
            manufacturer_data,
            manufacturer_data_count,
            service_data,
            service_data_count,
        }
    }

    unsafe fn release(&mut self) {
        free_str(self.address);
        free_str(self.address_type);
        free_str(self.name);
        free_str(self.alias);
        for u in take_vec(self.uuids, self.uuid_count) {
            free_str(u);
        }
        for m in take_vec(self.manufacturer_data, self.manufacturer_data_count) {
            drop(take_vec(m.data, m.len));
        }
        for s in take_vec(self.service_data, self.service_data_count) {
            free_str(s.uuid);
            drop(take_vec(s.data, s.len));
        }
    }
}

impl Drop for bluebus_device_info {
    fn drop(&mut self) {
        // SAFETY: every pointer was created by `new` and is freed only here.
        unsafe { self.release() }
    }
}

pub fn device_infos(v: &[ble::DeviceInfo]) -> (*mut bluebus_device_info, usize) {
    let (p, n) = leak_vec(v.iter().map(bluebus_device_info::new).collect());
    (p as *mut _, n)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_info_free(infos: *mut bluebus_device_info, count: usize) {
    quiet((), || drop(take_vec(infos, count)))
}

// ---------------------------------------------------------------------------
// Adapter information
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct bluebus_adapter_info {
    pub name: *const c_char,
    pub path: *const c_char,
    pub address: *const c_char,
    pub alias: *const c_char,
    pub powered: bool,
    pub discovering: bool,
}

impl Drop for bluebus_adapter_info {
    fn drop(&mut self) {
        // SAFETY: created by `adapter_infos`, freed only here.
        unsafe {
            free_str(self.name);
            free_str(self.path);
            free_str(self.address);
            free_str(self.alias);
        }
    }
}

pub fn adapter_infos(v: &[ble::AdapterInfo]) -> (*mut bluebus_adapter_info, usize) {
    let infos = v
        .iter()
        .map(|a| bluebus_adapter_info {
            name: leak_str(&a.name),
            path: leak_str(&a.path),
            address: leak_str(&a.address),
            alias: leak_str(&a.alias),
            powered: a.powered,
            discovering: a.discovering,
        })
        .collect();
    let (p, n) = leak_vec(infos);
    (p as *mut _, n)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_info_free(infos: *mut bluebus_adapter_info, count: usize) {
    quiet((), || drop(take_vec(infos, count)))
}

// ---------------------------------------------------------------------------
// GATT attributes (flattened service tree)
// ---------------------------------------------------------------------------

pub const ATTR_SERVICE: c_int = 0;
pub const ATTR_CHARACTERISTIC: c_int = 1;
pub const ATTR_DESCRIPTOR: c_int = 2;

#[repr(C)]
pub struct bluebus_gatt_attribute {
    pub kind: c_int,
    pub uuid: *const c_char,
    pub path: *const c_char,
    pub service_uuid: *const c_char,
    pub characteristic_uuid: *const c_char,
    pub flags: u32,
}

impl Drop for bluebus_gatt_attribute {
    fn drop(&mut self) {
        // SAFETY: created by `gatt_attributes`, freed only here.
        unsafe {
            free_str(self.uuid);
            free_str(self.path);
            free_str(self.service_uuid);
            free_str(self.characteristic_uuid);
        }
    }
}

pub fn gatt_attributes(services: &[ble::ServiceInfo]) -> (*mut bluebus_gatt_attribute, usize) {
    let mut out = Vec::new();
    for s in services {
        out.push(bluebus_gatt_attribute {
            kind: ATTR_SERVICE,
            uuid: leak_str(&s.uuid),
            path: leak_str(&s.path),
            service_uuid: leak_str(&s.uuid),
            characteristic_uuid: ptr::null(),
            flags: 0,
        });
        for c in &s.characteristics {
            out.push(bluebus_gatt_attribute {
                kind: ATTR_CHARACTERISTIC,
                uuid: leak_str(&c.uuid),
                path: leak_str(&c.path),
                service_uuid: leak_str(&s.uuid),
                characteristic_uuid: ptr::null(),
                flags: flags_to_bits(&c.flags),
            });
            for d in &c.descriptors {
                out.push(bluebus_gatt_attribute {
                    kind: ATTR_DESCRIPTOR,
                    uuid: leak_str(&d.uuid),
                    path: leak_str(&d.path),
                    service_uuid: leak_str(&s.uuid),
                    characteristic_uuid: leak_str(&c.uuid),
                    flags: flags_to_bits(&d.flags),
                });
            }
        }
    }
    let (p, n) = leak_vec(out);
    (p as *mut _, n)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_gatt_attributes_free(
    attributes: *mut bluebus_gatt_attribute,
    count: usize,
) {
    quiet((), || drop(take_vec(attributes, count)))
}
