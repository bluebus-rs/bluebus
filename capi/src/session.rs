//! Sessions, adapters, the pairing agent and scan filters.

use std::ffi::{c_char, c_int, CString};
use std::ptr;

use bluebus::ble;

use super::central::bluebus_device;
use super::info::{adapter_infos, bluebus_adapter_info, bluebus_device_info, device_infos};
use super::{
    block_on, bluebus_status, c_string, free_handle, guard, into_handle, invalid, millis, obj,
    opt_text, out, quiet, text, text_list, R,
};

/// `bluebus_session`: the connection to BlueZ.
pub struct bluebus_session {
    pub session: ble::Session,
}

/// `bluebus_adapter`: a local adapter.
pub struct bluebus_adapter {
    pub adapter: ble::Adapter,
    name: CString,
    path: CString,
    address: CString,
}

impl bluebus_adapter {
    pub fn new(adapter: ble::Adapter) -> Self {
        bluebus_adapter {
            name: c_string(adapter.name()),
            path: c_string(adapter.path()),
            address: c_string(adapter.address()),
            adapter,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_new(session: *mut *mut bluebus_session) -> bluebus_status {
    guard(|| {
        let session = out(session, "session")?;
        *session = ptr::null_mut();
        let s = block_on(ble::Session::new())?;
        *session = into_handle(bluebus_session { session: s });
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_free(session: *mut bluebus_session) {
    free_handle(session)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_adapters(
    session: *const bluebus_session,
    adapters: *mut *mut bluebus_adapter_info,
    count: *mut usize,
) -> bluebus_status {
    guard(|| {
        let adapters = out(adapters, "adapters")?;
        let count = out(count, "count")?;
        *adapters = ptr::null_mut();
        *count = 0;
        let s = obj(session, "session")?;
        let list = block_on(s.session.adapters())?;
        (*adapters, *count) = adapter_infos(&list);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_adapter(
    session: *const bluebus_session,
    name: *const c_char,
    adapter: *mut *mut bluebus_adapter,
) -> bluebus_status {
    guard(|| {
        let adapter = out(adapter, "adapter")?;
        *adapter = ptr::null_mut();
        let s = obj(session, "session")?;
        let name = opt_text(name, "name")?;
        let a = block_on(s.session.adapter(name))?;
        *adapter = into_handle(bluebus_adapter::new(a));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_default_adapter(
    session: *const bluebus_session,
    adapter: *mut *mut bluebus_adapter,
) -> bluebus_status {
    bluebus_session_adapter(session, ptr::null(), adapter)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_set_agent_auto(
    session: *const bluebus_session,
) -> bluebus_status {
    guard(|| {
        let s = obj(session, "session")?;
        block_on(
            s.session
                .set_agent(ble::AgentCapability::NoInputNoOutput, ble::AutoAcceptAgent),
        )?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_set_agent_passkey(
    session: *const bluebus_session,
    passkey: u32,
) -> bluebus_status {
    guard(|| {
        let s = obj(session, "session")?;
        if passkey > 999_999 {
            return Err(invalid(format!("passkey {passkey} is not in 0..=999999")));
        }
        block_on(s.session.set_agent(
            ble::AgentCapability::KeyboardDisplay,
            ble::PasskeyAgent::new(passkey),
        ))?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_session_remove_agent(
    session: *const bluebus_session,
) -> bluebus_status {
    guard(|| {
        let s = obj(session, "session")?;
        block_on(s.session.remove_agent())?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_free(adapter: *mut bluebus_adapter) {
    free_handle(adapter)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_name(adapter: *const bluebus_adapter) -> *const c_char {
    quiet(ptr::null(), || {
        adapter.as_ref().map_or(ptr::null(), |a| a.name.as_ptr())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_path(adapter: *const bluebus_adapter) -> *const c_char {
    quiet(ptr::null(), || {
        adapter.as_ref().map_or(ptr::null(), |a| a.path.as_ptr())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_address(adapter: *const bluebus_adapter) -> *const c_char {
    quiet(ptr::null(), || {
        adapter.as_ref().map_or(ptr::null(), |a| a.address.as_ptr())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_power_on(
    adapter: *const bluebus_adapter,
) -> bluebus_status {
    guard(|| Ok(block_on(obj(adapter, "adapter")?.adapter.power_on())?))
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_power_off(
    adapter: *const bluebus_adapter,
) -> bluebus_status {
    guard(|| Ok(block_on(obj(adapter, "adapter")?.adapter.power_off())?))
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_is_powered(
    adapter: *const bluebus_adapter,
    powered: *mut bool,
) -> bluebus_status {
    guard(|| {
        let powered = out(powered, "powered")?;
        *powered = block_on(obj(adapter, "adapter")?.adapter.is_powered())?;
        Ok(())
    })
}

/// `struct bluebus_scan_filter`; all-zero means "every device".
#[repr(C)]
pub struct bluebus_scan_filter {
    pub transport: c_int,
    pub name: *const c_char,
    pub name_prefix: *const c_char,
    pub address: *const c_char,
    pub service_uuids: *const *const c_char,
    pub service_uuid_count: usize,
    pub min_rssi: i16,
    pub has_min_rssi: bool,
    pub duplicate_data: bool,
    pub has_duplicate_data: bool,
}

pub const TRANSPORT_DEFAULT: c_int = 0;
pub const TRANSPORT_AUTO: c_int = 1;
pub const TRANSPORT_BREDR: c_int = 2;
pub const TRANSPORT_LE: c_int = 3;

/// Converts a C filter (`NULL` = no filter).
pub unsafe fn scan_filter(f: *const bluebus_scan_filter) -> R<ble::ScanFilter> {
    let mut filter = ble::ScanFilter::new();
    let Some(f) = f.as_ref() else {
        return Ok(filter);
    };
    filter = match f.transport {
        TRANSPORT_DEFAULT => filter,
        TRANSPORT_AUTO => filter.transport(ble::Transport::Auto),
        TRANSPORT_BREDR => filter.transport(ble::Transport::BrEdr),
        TRANSPORT_LE => filter.transport(ble::Transport::Le),
        t => return Err(invalid(format!("unknown transport {t}"))),
    };
    let name = opt_text(f.name, "filter name")?;
    let prefix = opt_text(f.name_prefix, "filter name_prefix")?;
    match (name, prefix) {
        (Some(_), Some(_)) => {
            return Err(invalid("set either name or name_prefix in a scan filter"));
        }
        (Some(n), None) => filter = filter.name(n),
        (None, Some(p)) => filter = filter.name_prefix(p),
        (None, None) => {}
    }
    if let Some(a) = opt_text(f.address, "filter address")? {
        filter = filter.address(a);
    }
    filter = filter.service_uuids(text_list(
        f.service_uuids,
        f.service_uuid_count,
        "filter service_uuids",
    )?);
    if f.has_min_rssi {
        filter = filter.min_rssi(f.min_rssi);
    }
    if f.has_duplicate_data {
        filter = filter.duplicate_data(f.duplicate_data);
    }
    Ok(filter)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_scan(
    adapter: *const bluebus_adapter,
    filter: *const bluebus_scan_filter,
    duration_ms: u32,
    devices: *mut *mut bluebus_device_info,
    count: *mut usize,
) -> bluebus_status {
    guard(|| {
        let devices = out(devices, "devices")?;
        let count = out(count, "count")?;
        *devices = ptr::null_mut();
        *count = 0;
        let a = obj(adapter, "adapter")?;
        let filter = scan_filter(filter)?;
        let found = block_on(a.adapter.scan(filter, millis(duration_ms)))?;
        (*devices, *count) = device_infos(&found);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_find_device(
    adapter: *const bluebus_adapter,
    filter: *const bluebus_scan_filter,
    timeout_ms: u32,
    device: *mut *mut bluebus_device,
) -> bluebus_status {
    guard(|| {
        let device = out(device, "device")?;
        *device = ptr::null_mut();
        let a = obj(adapter, "adapter")?;
        let filter = scan_filter(filter)?;
        let d = block_on(a.adapter.find_device(filter, millis(timeout_ms)))?;
        *device = into_handle(bluebus_device::new(d));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_device(
    adapter: *const bluebus_adapter,
    address: *const c_char,
    device: *mut *mut bluebus_device,
) -> bluebus_status {
    guard(|| {
        let device = out(device, "device")?;
        *device = ptr::null_mut();
        let a = obj(adapter, "adapter")?;
        let d = a.adapter.device(text(address, "address")?)?;
        *device = into_handle(bluebus_device::new(d));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_devices(
    adapter: *const bluebus_adapter,
    devices: *mut *mut bluebus_device_info,
    count: *mut usize,
) -> bluebus_status {
    guard(|| {
        let devices = out(devices, "devices")?;
        let count = out(count, "count")?;
        *devices = ptr::null_mut();
        *count = 0;
        let list = block_on(obj(adapter, "adapter")?.adapter.devices())?;
        (*devices, *count) = device_infos(&list);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_remove_device(
    adapter: *const bluebus_adapter,
    address: *const c_char,
) -> bluebus_status {
    guard(|| {
        let a = obj(adapter, "adapter")?;
        block_on(a.adapter.remove_device(text(address, "address")?))?;
        Ok(())
    })
}
