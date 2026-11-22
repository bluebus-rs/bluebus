//! C API of bluebus (`include/bluebus.h`), over the high-level
//! [`bluebus::ble`] module.
//!
//! Every function blocks the calling thread; the library runs its own tokio
//! runtime. Handles are boxed Rust values behind opaque pointers, every
//! entry point catches panics, and errors are reported as a
//! [`bluebus_status`] plus a thread-local message.
//!
//! This file is also compiled into the test binaries in `capi/tests`
//! (`#[path = "../src/lib.rs"] mod ffi;`), so the modules refer to each other
//! through `super::`, never `crate::`.
#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)] // the safety contract is in bluebus.h

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;
use std::time::Duration;

use bluebus::ble;

pub mod central;
pub mod info;
pub mod peripheral;
pub mod session;
pub mod streams;

pub use self::central::*;
pub use self::info::*;
pub use self::peripheral::*;
pub use self::session::*;
pub use self::streams::*;

/// Result of every fallible function (`enum bluebus_status`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum bluebus_status {
    BLUEBUS_OK = 0,
    BLUEBUS_ERR_TIMEOUT = 1,
    BLUEBUS_ERR_NOT_FOUND = 2,
    BLUEBUS_ERR_INVALID_ARGUMENT = 3,
    BLUEBUS_ERR_DBUS = 4,
    BLUEBUS_ERR_CLOSED = 5,
    BLUEBUS_ERR_INTERNAL = 6,
}

use bluebus_status::*;

/// A failed call: the status and the message stored for `bluebus_last_error`.
#[derive(Debug)]
pub struct Fail {
    pub status: bluebus_status,
    pub message: String,
    pub dbus_name: Option<String>,
}

pub type R<T> = Result<T, Fail>;

impl Fail {
    pub fn new(status: bluebus_status, message: impl Into<String>) -> Self {
        Fail {
            status,
            message: message.into(),
            dbus_name: None,
        }
    }
}

pub fn invalid(message: impl Into<String>) -> Fail {
    Fail::new(BLUEBUS_ERR_INVALID_ARGUMENT, message)
}

impl From<ble::Error> for Fail {
    fn from(e: ble::Error) -> Self {
        let status = match &e {
            ble::Error::Bus(_) => BLUEBUS_ERR_DBUS,
            ble::Error::Timeout(_) => BLUEBUS_ERR_TIMEOUT,
            ble::Error::NotFound(_) => BLUEBUS_ERR_NOT_FOUND,
            ble::Error::InvalidArgument(_) => BLUEBUS_ERR_INVALID_ARGUMENT,
            _ => BLUEBUS_ERR_INTERNAL,
        };
        Fail {
            status,
            message: e.to_string(),
            dbus_name: e.dbus_error_name(),
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
    static LAST_DBUS_NAME: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// A C string without interior NUL bytes.
pub fn c_string(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn record(e: Fail) -> bluebus_status {
    LAST_ERROR.with(|m| *m.borrow_mut() = c_string(&e.message));
    LAST_DBUS_NAME.with(|n| *n.borrow_mut() = e.dbus_name.as_deref().map(c_string));
    e.status
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".into()
    }
}

/// Runs an entry point: catches panics and records errors.
pub fn guard(f: impl FnOnce() -> R<()>) -> bluebus_status {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => BLUEBUS_OK,
        Ok(Err(e)) => record(e),
        Err(p) => record(Fail::new(
            BLUEBUS_ERR_INTERNAL,
            format!("internal error: {}", panic_message(&*p)),
        )),
    }
}

/// Runs an entry point that cannot fail (free functions, getters).
pub fn quiet<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// The library's runtime (two worker threads, created on first use).
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("bluebus-rt")
            .enable_all()
            .build()
            .expect("cannot create the bluebus runtime")
    })
}

/// Blocks the calling thread on `f`.
pub fn block_on<F>(f: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        // block_on would panic; run it on a scoped helper thread instead.
        return std::thread::scope(|s| {
            s.spawn(|| runtime().block_on(f))
                .join()
                .unwrap_or_else(|p| std::panic::resume_unwind(p))
        });
    }
    runtime().block_on(f)
}

pub fn millis(ms: u32) -> Duration {
    Duration::from_millis(u64::from(ms))
}

// ---------------------------------------------------------------------------
// Pointer helpers
// ---------------------------------------------------------------------------

/// An output pointer; set to its zero value up front.
pub unsafe fn out<'a, T>(p: *mut T, what: &str) -> R<&'a mut T> {
    p.as_mut().ok_or_else(|| invalid(format!("{what} is NULL")))
}

/// A handle or input struct.
pub unsafe fn obj<'a, T>(p: *const T, what: &str) -> R<&'a T> {
    p.as_ref().ok_or_else(|| invalid(format!("{what} is NULL")))
}

/// A required UTF-8 string.
pub unsafe fn text<'a>(p: *const c_char, what: &str) -> R<&'a str> {
    opt_text(p, what)?.ok_or_else(|| invalid(format!("{what} is NULL")))
}

/// An optional UTF-8 string (`NULL` is `None`).
pub unsafe fn opt_text<'a>(p: *const c_char, what: &str) -> R<Option<&'a str>> {
    if p.is_null() {
        return Ok(None);
    }
    CStr::from_ptr(p)
        .to_str()
        .map(Some)
        .map_err(|_| invalid(format!("{what} is not valid UTF-8")))
}

/// A list of `count` strings.
pub unsafe fn text_list(p: *const *const c_char, count: usize, what: &str) -> R<Vec<String>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if p.is_null() {
        return Err(invalid(format!("{what} is NULL but its count is {count}")));
    }
    std::slice::from_raw_parts(p, count)
        .iter()
        .map(|s| text(*s, what).map(str::to_owned))
        .collect()
}

/// `len` input bytes (`data` may be NULL when `len` is 0).
pub unsafe fn bytes_in<'a>(data: *const u8, len: usize, what: &str) -> R<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    if data.is_null() {
        return Err(invalid(format!("{what} is NULL but its length is {len}")));
    }
    Ok(std::slice::from_raw_parts(data, len))
}

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    #[link_name = "free"]
    fn c_free(p: *mut c_void);
}

/// Hands bytes to C in `malloc` memory (`NULL` for an empty value).
pub unsafe fn bytes_out(value: &[u8], data: &mut *mut u8, len: &mut usize) -> R<()> {
    *data = std::ptr::null_mut();
    *len = 0;
    if value.is_empty() {
        return Ok(());
    }
    let p = malloc(value.len()) as *mut u8;
    if p.is_null() {
        return Err(Fail::new(BLUEBUS_ERR_INTERNAL, "out of memory"));
    }
    std::ptr::copy_nonoverlapping(value.as_ptr(), p, value.len());
    *data = p;
    *len = value.len();
    Ok(())
}

pub fn into_handle<T>(v: T) -> *mut T {
    Box::into_raw(Box::new(v))
}

pub unsafe fn free_handle<T>(p: *mut T) {
    if !p.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(p))));
    }
}

/// A `void* user` pointer handed back to C callbacks.
#[derive(Clone, Copy)]
pub struct UserData(pub *mut c_void);
// SAFETY: the pointer is only passed back to the C callback; the caller of
// the registering function promised that this is fine from any thread.
unsafe impl Send for UserData {}
unsafe impl Sync for UserData {}

// ---------------------------------------------------------------------------
// Errors and version
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn bluebus_last_error() -> *const c_char {
    quiet(c"".as_ptr(), || LAST_ERROR.with(|m| m.borrow().as_ptr()))
}

#[no_mangle]
pub extern "C" fn bluebus_last_dbus_error_name() -> *const c_char {
    quiet(std::ptr::null(), || {
        LAST_DBUS_NAME.with(|n| n.borrow().as_ref().map_or(std::ptr::null(), |s| s.as_ptr()))
    })
}

#[no_mangle]
pub extern "C" fn bluebus_status_str(status: c_int) -> *const c_char {
    // Taken as an int: C may pass any value.
    let s = match status {
        0 => c"ok",
        1 => c"timeout",
        2 => c"not found",
        3 => c"invalid argument",
        4 => c"D-Bus error",
        5 => c"closed",
        6 => c"internal error",
        _ => c"unknown status",
    };
    s.as_ptr()
}

#[no_mangle]
pub extern "C" fn bluebus_version() -> *const c_char {
    static VERSION: &CStr =
        match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(v) => v,
            Err(_) => c"",
        };
    VERSION.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_bytes_free(data: *mut u8) {
    if !data.is_null() {
        c_free(data as *mut c_void);
    }
}

#[no_mangle]
pub unsafe extern "C" fn bluebus_normalize_uuid(
    uuid: *const c_char,
    out_uuid: *mut c_char,
    size: usize,
) -> bluebus_status {
    guard(|| {
        let full = ble::normalize_uuid(text(uuid, "uuid")?)?;
        if out_uuid.is_null() || size < full.len() + 1 {
            return Err(invalid("out_uuid needs room for 37 bytes"));
        }
        std::ptr::copy_nonoverlapping(full.as_ptr(), out_uuid as *mut u8, full.len());
        *out_uuid.add(full.len()) = 0;
        Ok(())
    })
}
