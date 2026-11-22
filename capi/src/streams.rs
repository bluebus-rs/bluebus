//! Running scans and notification subscriptions. Callback variants run a
//! library thread per handle that pulls the stream and calls C.

use std::ffi::{c_char, c_int, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Mutex;
use std::thread::{JoinHandle, ThreadId};

use bluebus::ble;
use futures::future::BoxFuture;
use futures::{Stream, StreamExt};

use super::bluebus_status::*;
use super::central::bluebus_device;
use super::info::bluebus_device_info;
use super::session::{bluebus_adapter, bluebus_scan_filter, scan_filter};
use super::{
    block_on, bluebus_status, bytes_out, free_handle, guard, into_handle, invalid, millis, obj,
    out, text, Fail, UserData, R,
};

/// A stream that is stopped explicitly (waiting for BlueZ).
pub trait Stoppable: Stream + Unpin + Send + 'static {
    fn stop_stream(self) -> BoxFuture<'static, ble::Result<()>>;
}

impl Stoppable for ble::ScanStream {
    fn stop_stream(self) -> BoxFuture<'static, ble::Result<()>> {
        Box::pin(self.stop())
    }
}

impl Stoppable for ble::NotificationStream {
    fn stop_stream(self) -> BoxFuture<'static, ble::Result<()>> {
        Box::pin(self.stop())
    }
}

/// A library thread feeding stream items to a callback.
pub struct Worker {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<R<()>>>,
    id: ThreadId,
}

impl Worker {
    pub fn spawn<S>(
        name: &str,
        mut stream: S,
        mut on_item: impl FnMut(S::Item) + Send + 'static,
    ) -> R<Worker>
    where
        S: Stoppable,
        S::Item: Send,
    {
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                loop {
                    let next = block_on(async {
                        tokio::select! {
                            _ = &mut rx => None,
                            item = stream.next() => Some(item),
                        }
                    });
                    let Some(Some(item)) = next else { break };
                    if catch_unwind(AssertUnwindSafe(|| on_item(item))).is_err() {
                        break;
                    }
                }
                block_on(stream.stop_stream()).map_err(Fail::from)
            })
            .map_err(|e| Fail::new(BLUEBUS_ERR_INTERNAL, format!("cannot start a thread: {e}")))?;
        Ok(Worker {
            stop: Some(tx),
            id: thread.thread().id(),
            thread: Some(thread),
        })
    }

    /// Stops the thread and waits for it, unless called from the thread
    /// itself (a callback): then it stops after the callback returns.
    pub fn stop(&mut self) -> R<()> {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if std::thread::current().id() == self.id {
            return Ok(());
        }
        match self.thread.take() {
            Some(t) => t
                .join()
                .unwrap_or_else(|_| Err(Fail::new(BLUEBUS_ERR_INTERNAL, "worker thread panicked"))),
            None => Ok(()),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// Scans
// ---------------------------------------------------------------------------

pub const SCAN_FOUND: c_int = 0;
pub const SCAN_UPDATED: c_int = 1;

pub type bluebus_scan_callback =
    Option<unsafe extern "C" fn(*const bluebus_device_info, c_int, *mut c_void)>;

/// `bluebus_scan`: a running callback scan.
pub struct bluebus_scan {
    worker: Mutex<Worker>,
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_adapter_scan_start(
    adapter: *const bluebus_adapter,
    filter: *const bluebus_scan_filter,
    callback: bluebus_scan_callback,
    user: *mut c_void,
    scan: *mut *mut bluebus_scan,
) -> bluebus_status {
    guard(|| {
        let scan = out(scan, "scan")?;
        *scan = ptr::null_mut();
        let a = obj(adapter, "adapter")?;
        let callback = callback.ok_or_else(|| invalid("callback is NULL"))?;
        let filter = scan_filter(filter)?;
        let stream = block_on(a.adapter.scan_stream(filter))?;
        let user = UserData(user);
        let worker = Worker::spawn("bluebus-scan", stream, move |event| {
            let user = user;
            let (kind, device) = match &event {
                ble::ScanEvent::Found(d) => (SCAN_FOUND, d),
                ble::ScanEvent::Updated(d) => (SCAN_UPDATED, d),
                _ => return,
            };
            let info = bluebus_device_info::new(device);
            // SAFETY: the caller of scan_start provided a valid callback.
            unsafe { callback(&info, kind, user.0) };
        })?;
        *scan = into_handle(bluebus_scan {
            worker: Mutex::new(worker),
        });
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_scan_stop(scan: *const bluebus_scan) -> bluebus_status {
    guard(|| lock(&obj(scan, "scan")?.worker).stop())
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_scan_free(scan: *mut bluebus_scan) {
    free_handle(scan)
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

pub type bluebus_notify_callback = Option<unsafe extern "C" fn(*const u8, usize, *mut c_void)>;

pub const WAIT_FOREVER: u32 = u32::MAX;

enum Mode {
    Callback(Mutex<Worker>),
    Pull(Mutex<Option<ble::NotificationStream>>),
}

/// `bluebus_notifications`: a subscription to a characteristic.
pub struct bluebus_notifications {
    mode: Mode,
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_subscribe(
    device: *const bluebus_device,
    uuid: *const c_char,
    callback: bluebus_notify_callback,
    user: *mut c_void,
    notifications: *mut *mut bluebus_notifications,
) -> bluebus_status {
    guard(|| {
        let notifications = out(notifications, "notifications")?;
        *notifications = ptr::null_mut();
        let d = obj(device, "device")?;
        let uuid = text(uuid, "uuid")?;
        let callback = callback.ok_or_else(|| invalid("callback is NULL"))?;
        let stream = block_on(d.device.notifications(uuid))?;
        let user = UserData(user);
        let worker = Worker::spawn("bluebus-notify", stream, move |value: Vec<u8>| {
            let user = user;
            // SAFETY: the caller of subscribe provided a valid callback.
            unsafe { callback(value.as_ptr(), value.len(), user.0) };
        })?;
        *notifications = into_handle(bluebus_notifications {
            mode: Mode::Callback(Mutex::new(worker)),
        });
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_device_notifications(
    device: *const bluebus_device,
    uuid: *const c_char,
    notifications: *mut *mut bluebus_notifications,
) -> bluebus_status {
    guard(|| {
        let notifications = out(notifications, "notifications")?;
        *notifications = ptr::null_mut();
        let d = obj(device, "device")?;
        let stream = block_on(d.device.notifications(text(uuid, "uuid")?))?;
        *notifications = into_handle(bluebus_notifications {
            mode: Mode::Pull(Mutex::new(Some(stream))),
        });
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_notifications_next(
    notifications: *const bluebus_notifications,
    timeout_ms: u32,
    data: *mut *mut u8,
    len: *mut usize,
) -> bluebus_status {
    guard(|| {
        let data = out(data, "data")?;
        let len = out(len, "len")?;
        *data = ptr::null_mut();
        *len = 0;
        let Mode::Pull(stream) = &obj(notifications, "notifications")?.mode else {
            return Err(invalid(
                "bluebus_notifications_next needs a handle from bluebus_device_notifications",
            ));
        };
        let mut stream = lock(stream);
        let Some(s) = stream.as_mut() else {
            return Err(Fail::new(BLUEBUS_ERR_CLOSED, "the subscription is stopped"));
        };
        let value = if timeout_ms == WAIT_FOREVER {
            block_on(s.next())
        } else {
            block_on(async { tokio::time::timeout(millis(timeout_ms), s.next()).await }).map_err(
                |_| {
                    Fail::new(
                        BLUEBUS_ERR_TIMEOUT,
                        format!("timed out: no notification within {timeout_ms} ms"),
                    )
                },
            )?
        };
        match value {
            Some(v) => bytes_out(&v, data, len),
            None => Err(Fail::new(
                BLUEBUS_ERR_CLOSED,
                "the notifications ended (D-Bus connection closed)",
            )),
        }
    })
}

impl bluebus_notifications {
    fn stop(&self) -> R<()> {
        match &self.mode {
            Mode::Callback(w) => lock(w).stop(),
            Mode::Pull(s) => match lock(s).take() {
                Some(s) => Ok(block_on(s.stop())?),
                None => Ok(()),
            },
        }
    }
}

impl Drop for bluebus_notifications {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_notifications_stop(
    notifications: *const bluebus_notifications,
) -> bluebus_status {
    guard(|| obj(notifications, "notifications")?.stop())
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_notifications_free(notifications: *mut bluebus_notifications) {
    free_handle(notifications)
}
