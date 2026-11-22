//! Remote devices: connection, pairing and the GATT client.

use std::ffi::{c_char, CString};
use std::ptr;

use bluebus::ble;

use super::info::{bluebus_device_info, bluebus_gatt_attribute, gatt_attributes, leak_vec};
use super::{
    block_on, bluebus_status, bytes_in, bytes_out, c_string, free_handle, guard, millis, obj, out,
    quiet, text,
};

/// `bluebus_device`: a remote device on one adapter.
pub struct bluebus_device {
    pub device: ble::Device,
    address: CString,
}

impl bluebus_device {
    pub fn new(device: ble::Device) -> Self {
        bluebus_device {
            address: c_string(device.address()),
            device,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_free(device: *mut bluebus_device) {
    free_handle(device)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_address(device: *const bluebus_device) -> *const c_char {
    quiet(ptr::null(), || {
        device.as_ref().map_or(ptr::null(), |d| d.address.as_ptr())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_get_info(
    device: *const bluebus_device,
    info: *mut *mut bluebus_device_info,
) -> bluebus_status {
    guard(|| {
        let info = out(info, "info")?;
        *info = ptr::null_mut();
        let d = block_on(obj(device, "device")?.device.info())?;
        *info = leak_vec(vec![bluebus_device_info::new(&d)]).0 as *mut _;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_connect(
    device: *const bluebus_device,
    timeout_ms: u32,
) -> bluebus_status {
    guard(|| {
        Ok(block_on(
            obj(device, "device")?.device.connect(millis(timeout_ms)),
        )?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_disconnect(
    device: *const bluebus_device,
) -> bluebus_status {
    guard(|| Ok(block_on(obj(device, "device")?.device.disconnect())?))
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_is_connected(
    device: *const bluebus_device,
    connected: *mut bool,
) -> bluebus_status {
    guard(|| {
        let connected = out(connected, "connected")?;
        *connected = block_on(obj(device, "device")?.device.is_connected())?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_pair(device: *const bluebus_device) -> bluebus_status {
    guard(|| Ok(block_on(obj(device, "device")?.device.pair())?))
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_trust(
    device: *const bluebus_device,
    trusted: bool,
) -> bluebus_status {
    guard(|| Ok(block_on(obj(device, "device")?.device.trust(trusted))?))
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_rssi(
    device: *const bluebus_device,
    rssi: *mut i16,
    has_rssi: *mut bool,
) -> bluebus_status {
    guard(|| {
        let rssi = out(rssi, "rssi")?;
        let has_rssi = out(has_rssi, "has_rssi")?;
        let v = block_on(obj(device, "device")?.device.rssi())?;
        *rssi = v.unwrap_or(0);
        *has_rssi = v.is_some();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_battery_level(
    device: *const bluebus_device,
    percent: *mut u8,
) -> bluebus_status {
    guard(|| {
        let percent = out(percent, "percent")?;
        *percent = block_on(obj(device, "device")?.device.battery_level())?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_services(
    device: *const bluebus_device,
    attributes: *mut *mut bluebus_gatt_attribute,
    count: *mut usize,
) -> bluebus_status {
    guard(|| {
        let attributes = out(attributes, "attributes")?;
        let count = out(count, "count")?;
        *attributes = ptr::null_mut();
        *count = 0;
        let services = block_on(obj(device, "device")?.device.services())?;
        (*attributes, *count) = gatt_attributes(&services);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_read(
    device: *const bluebus_device,
    uuid: *const c_char,
    data: *mut *mut u8,
    len: *mut usize,
) -> bluebus_status {
    guard(|| {
        let data = out(data, "data")?;
        let len = out(len, "len")?;
        *data = ptr::null_mut();
        *len = 0;
        let d = obj(device, "device")?;
        let value = block_on(d.device.read(text(uuid, "uuid")?))?;
        bytes_out(&value, data, len)
    })
}

fn write_type(with_response: bool) -> ble::WriteType {
    if with_response {
        ble::WriteType::WithResponse
    } else {
        ble::WriteType::WithoutResponse
    }
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_write(
    device: *const bluebus_device,
    uuid: *const c_char,
    data: *const u8,
    len: usize,
    with_response: bool,
) -> bluebus_status {
    guard(|| {
        let d = obj(device, "device")?;
        let uuid = text(uuid, "uuid")?;
        let value = bytes_in(data, len, "data")?.to_vec();
        block_on(d.device.write(uuid, value, write_type(with_response)))?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_read_descriptor(
    device: *const bluebus_device,
    characteristic_uuid: *const c_char,
    descriptor_uuid: *const c_char,
    data: *mut *mut u8,
    len: *mut usize,
) -> bluebus_status {
    guard(|| {
        let data = out(data, "data")?;
        let len = out(len, "len")?;
        *data = ptr::null_mut();
        *len = 0;
        let d = obj(device, "device")?;
        let value = block_on(d.device.read_descriptor(
            text(characteristic_uuid, "characteristic_uuid")?,
            text(descriptor_uuid, "descriptor_uuid")?,
        ))?;
        bytes_out(&value, data, len)
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_write_descriptor(
    device: *const bluebus_device,
    characteristic_uuid: *const c_char,
    descriptor_uuid: *const c_char,
    data: *const u8,
    len: usize,
) -> bluebus_status {
    guard(|| {
        let d = obj(device, "device")?;
        let chr = text(characteristic_uuid, "characteristic_uuid")?;
        let desc = text(descriptor_uuid, "descriptor_uuid")?;
        let value = bytes_in(data, len, "data")?.to_vec();
        block_on(d.device.write_descriptor(chr, desc, value))?;
        Ok(())
    })
}
