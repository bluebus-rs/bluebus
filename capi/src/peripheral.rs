//! Local GATT server and advertisement.

use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::{mpsc, Mutex};
use std::thread::{JoinHandle, ThreadId};

use bluebus::ble;

use super::bluebus_status::*;
use super::info::bits_to_flags;
use super::session::bluebus_adapter;
use super::{
    block_on, bluebus_status, bytes_in, bytes_out, free_handle, guard, into_handle, invalid, obj,
    out, quiet, text, text_list, Fail, UserData, R,
};

pub type bluebus_write_callback = Option<unsafe extern "C" fn(*const u8, usize, *mut c_void)>;

struct CharSpec {
    uuid: String,
    flags: Vec<String>,
    value: Vec<u8>,
    on_write: Option<(
        unsafe extern "C" fn(*const u8, usize, *mut c_void),
        UserData,
    )>,
    descriptors: Vec<(String, Vec<u8>)>,
}

struct ServiceSpec {
    uuid: String,
    primary: bool,
    characteristics: Vec<CharSpec>,
}

#[derive(Default)]
struct Spec {
    services: Vec<ServiceSpec>,
    advertisement: Option<ble::Advertisement>,
}

/// `bluebus_peripheral_builder`: services and advertisement to publish.
pub struct bluebus_peripheral_builder {
    adapter: ble::Adapter,
    spec: Mutex<Spec>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_new(
    adapter: *const bluebus_adapter,
    builder: *mut *mut bluebus_peripheral_builder,
) -> bluebus_status {
    guard(|| {
        let builder = out(builder, "builder")?;
        *builder = ptr::null_mut();
        let a = obj(adapter, "adapter")?;
        *builder = into_handle(bluebus_peripheral_builder {
            adapter: a.adapter.clone(),
            spec: Mutex::new(Spec::default()),
        });
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_free(builder: *mut bluebus_peripheral_builder) {
    free_handle(builder)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_add_service(
    builder: *const bluebus_peripheral_builder,
    uuid: *const c_char,
    primary: bool,
) -> bluebus_status {
    guard(|| {
        let b = obj(builder, "builder")?;
        let uuid = ble::normalize_uuid(text(uuid, "uuid")?)?;
        let mut spec = lock(&b.spec);
        if spec.services.iter().any(|s| s.uuid == uuid) {
            return Err(invalid(format!("service {uuid} was already added")));
        }
        spec.services.push(ServiceSpec {
            uuid,
            primary,
            characteristics: Vec::new(),
        });
        Ok(())
    })
}

fn service<'a>(spec: &'a mut Spec, uuid: &str) -> R<&'a mut ServiceSpec> {
    let uuid = ble::normalize_uuid(uuid)?;
    spec.services
        .iter_mut()
        .find(|s| s.uuid == uuid)
        .ok_or_else(|| {
            Fail::new(
                BLUEBUS_ERR_NOT_FOUND,
                format!("service {uuid} was not added to the builder"),
            )
        })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_add_characteristic(
    builder: *const bluebus_peripheral_builder,
    service_uuid: *const c_char,
    uuid: *const c_char,
    flags: u32,
    value: *const u8,
    value_len: usize,
    on_write: bluebus_write_callback,
    user: *mut c_void,
) -> bluebus_status {
    guard(|| {
        let b = obj(builder, "builder")?;
        let service_uuid = text(service_uuid, "service_uuid")?;
        let uuid = ble::normalize_uuid(text(uuid, "uuid")?)?;
        let flags = bits_to_flags(flags)
            .map_err(|bits| invalid(format!("unknown characteristic flag bits {bits:#x}")))?;
        let value = bytes_in(value, value_len, "value")?.to_vec();
        let mut spec = lock(&b.spec);
        let svc = service(&mut spec, service_uuid)?;
        svc.characteristics.push(CharSpec {
            uuid,
            flags,
            value,
            on_write: on_write.map(|cb| (cb, UserData(user))),
            descriptors: Vec::new(),
        });
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_add_descriptor(
    builder: *const bluebus_peripheral_builder,
    characteristic_uuid: *const c_char,
    uuid: *const c_char,
    value: *const u8,
    value_len: usize,
) -> bluebus_status {
    guard(|| {
        let b = obj(builder, "builder")?;
        let chr = ble::normalize_uuid(text(characteristic_uuid, "characteristic_uuid")?)?;
        let uuid = text(uuid, "uuid")?.to_owned();
        let value = bytes_in(value, value_len, "value")?.to_vec();
        let mut spec = lock(&b.spec);
        let c = spec
            .services
            .iter_mut()
            .flat_map(|s| s.characteristics.iter_mut())
            .find(|c| c.uuid == chr)
            .ok_or_else(|| {
                Fail::new(
                    BLUEBUS_ERR_NOT_FOUND,
                    format!("characteristic {chr} was not added to the builder"),
                )
            })?;
        c.descriptors.push((uuid, value));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_advertise(
    builder: *const bluebus_peripheral_builder,
    local_name: *const c_char,
    service_uuids: *const *const c_char,
    service_uuid_count: usize,
) -> bluebus_status {
    guard(|| {
        let b = obj(builder, "builder")?;
        let mut adv = ble::Advertisement::new();
        if let Some(name) = super::opt_text(local_name, "local_name")? {
            adv = adv.local_name(name);
        }
        for u in text_list(service_uuids, service_uuid_count, "service_uuids")? {
            adv = adv.service_uuid(u);
        }
        lock(&b.spec).advertisement = Some(adv);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_advertise_manufacturer_data(
    builder: *const bluebus_peripheral_builder,
    company_id: u16,
    data: *const u8,
    len: usize,
) -> bluebus_status {
    guard(|| {
        let b = obj(builder, "builder")?;
        let data = bytes_in(data, len, "data")?.to_vec();
        let mut spec = lock(&b.spec);
        let adv = spec.advertisement.take().unwrap_or_default();
        spec.advertisement = Some(adv.manufacturer_data(company_id, data));
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Running peripheral
// ---------------------------------------------------------------------------

enum Msg {
    Write(usize, Vec<u8>),
    Quit,
}

/// Calls the C write callbacks on one library thread.
struct Dispatcher {
    tx: mpsc::Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    id: ThreadId,
}

impl Dispatcher {
    fn stop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if std::thread::current().id() == self.id {
            return; // inside a callback: the thread ends after it returns
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// `bluebus_peripheral`: a started GATT server / advertisement.
pub struct bluebus_peripheral {
    peripheral: ble::Peripheral,
    dispatcher: Mutex<Option<Dispatcher>>,
}

impl Drop for bluebus_peripheral {
    fn drop(&mut self) {
        let _ = block_on(self.peripheral.stop());
        if let Some(mut d) = lock(&self.dispatcher).take() {
            d.stop();
        }
    }
}

type Callbacks = Vec<(
    unsafe extern "C" fn(*const u8, usize, *mut c_void),
    UserData,
)>;

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_builder_start(
    builder: *const bluebus_peripheral_builder,
    peripheral: *mut *mut bluebus_peripheral,
) -> bluebus_status {
    guard(|| {
        let peripheral = out(peripheral, "peripheral")?;
        *peripheral = ptr::null_mut();
        let b = obj(builder, "builder")?;
        let (tx, rx) = mpsc::channel::<Msg>();
        let mut callbacks: Callbacks = Vec::new();
        let mut rust = b.adapter.peripheral();
        {
            let spec = lock(&b.spec);
            for s in &spec.services {
                let mut service = ble::Service::new(s.uuid.clone()).primary(s.primary);
                for c in &s.characteristics {
                    let mut chr = ble::Characteristic::new(c.uuid.clone()).value(c.value.clone());
                    for f in &c.flags {
                        chr = chr.flag(f.clone());
                    }
                    for (uuid, value) in &c.descriptors {
                        chr = chr.descriptor(uuid.clone(), value.clone());
                    }
                    if let Some(cb) = c.on_write {
                        let index = callbacks.len();
                        callbacks.push(cb);
                        let tx = tx.clone();
                        chr = chr.on_write(move |value| {
                            let _ = tx.send(Msg::Write(index, value));
                        });
                    }
                    service = service.characteristic(chr);
                }
                rust = rust.service(service);
            }
            if let Some(adv) = &spec.advertisement {
                rust = rust.advertise(adv.clone());
            }
        }
        let dispatcher = if callbacks.is_empty() {
            None
        } else {
            let thread = std::thread::Builder::new()
                .name("bluebus-peripheral".into())
                .spawn(move || dispatch(rx, callbacks))
                .map_err(|e| {
                    Fail::new(BLUEBUS_ERR_INTERNAL, format!("cannot start a thread: {e}"))
                })?;
            Some(Dispatcher {
                tx,
                id: thread.thread().id(),
                thread: Some(thread),
            })
        };
        let started = block_on(rust.start());
        let p = match started {
            Ok(p) => p,
            Err(e) => {
                if let Some(mut d) = dispatcher {
                    d.stop();
                }
                return Err(e.into());
            }
        };
        *peripheral = into_handle(bluebus_peripheral {
            peripheral: p,
            dispatcher: Mutex::new(dispatcher),
        });
        Ok(())
    })
}

fn dispatch(rx: mpsc::Receiver<Msg>, callbacks: Callbacks) {
    while let Ok(Msg::Write(index, value)) = rx.recv() {
        let (cb, user) = callbacks[index];
        let _ = std::panic::catch_unwind(|| {
            // SAFETY: the caller of add_characteristic provided the callback.
            unsafe { cb(value.as_ptr(), value.len(), user.0) }
        });
    }
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_free(peripheral: *mut bluebus_peripheral) {
    free_handle(peripheral)
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_stop(
    peripheral: *const bluebus_peripheral,
) -> bluebus_status {
    guard(|| Ok(block_on(obj(peripheral, "peripheral")?.peripheral.stop())?))
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_is_running(
    peripheral: *const bluebus_peripheral,
) -> bool {
    quiet(false, || {
        peripheral
            .as_ref()
            .is_some_and(|p| p.peripheral.is_running())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_set_value(
    peripheral: *const bluebus_peripheral,
    uuid: *const c_char,
    data: *const u8,
    len: usize,
) -> bluebus_status {
    guard(|| {
        let p = obj(peripheral, "peripheral")?;
        let uuid = text(uuid, "uuid")?;
        let value = bytes_in(data, len, "data")?.to_vec();
        Ok(block_on(p.peripheral.set_value(uuid, value))?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_notify(
    peripheral: *const bluebus_peripheral,
    uuid: *const c_char,
    data: *const u8,
    len: usize,
) -> bluebus_status {
    guard(|| {
        let p = obj(peripheral, "peripheral")?;
        let uuid = text(uuid, "uuid")?;
        let value = bytes_in(data, len, "data")?.to_vec();
        Ok(block_on(p.peripheral.notify(uuid, value))?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_value(
    peripheral: *const bluebus_peripheral,
    uuid: *const c_char,
    data: *mut *mut u8,
    len: *mut usize,
) -> bluebus_status {
    guard(|| {
        let data = out(data, "data")?;
        let len = out(len, "len")?;
        *data = ptr::null_mut();
        *len = 0;
        let p = obj(peripheral, "peripheral")?;
        let value = block_on(p.peripheral.value(text(uuid, "uuid")?))?;
        bytes_out(&value, data, len)
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_peripheral_is_notifying(
    peripheral: *const bluebus_peripheral,
    uuid: *const c_char,
    notifying: *mut bool,
) -> bluebus_status {
    guard(|| {
        let notifying = out(notifying, "notifying")?;
        let p = obj(peripheral, "peripheral")?;
        *notifying = block_on(p.peripheral.is_notifying(text(uuid, "uuid")?))?;
        Ok(())
    })
}
