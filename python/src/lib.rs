//! Python bindings of `bluebus::ble` (native module `bluebus._bluebus`).
//!
//! Every I/O method returns an asyncio awaitable backed by a tokio future
//! (`pyo3-async-runtimes`); the Python package `bluebus` re-exports the
//! classes and defines the exceptions (`bluebus/exceptions.py`).

use pyo3::prelude::*;

mod central;
mod errors;
mod peripheral;
mod streams;
mod types;

/// `True` / `False` for reprs.
pub(crate) fn py_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

/// Full lower-case form of a 16-, 32- or 128-bit UUID (`"2a19"` →
/// `"00002a19-0000-1000-8000-00805f9b34fb"`).
#[pyfunction]
fn normalize_uuid(uuid: &str) -> PyResult<String> {
    errors::check(bluebus::ble::normalize_uuid(uuid))
}

#[pymodule]
fn _bluebus(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(normalize_uuid, m)?)?;
    m.add_class::<central::PySession>()?;
    m.add_class::<central::PyAdapter>()?;
    m.add_class::<central::PyDevice>()?;
    m.add_class::<central::PyConnection>()?;
    m.add_class::<streams::PyScanStream>()?;
    m.add_class::<streams::PyNotificationStream>()?;
    m.add_class::<types::PyAdapterInfo>()?;
    m.add_class::<types::PyDeviceInfo>()?;
    m.add_class::<types::PyServiceInfo>()?;
    m.add_class::<types::PyCharacteristicInfo>()?;
    m.add_class::<types::PyDescriptorInfo>()?;
    m.add_class::<types::PyScanEvent>()?;
    m.add_class::<peripheral::PyCharacteristic>()?;
    m.add_class::<peripheral::PyService>()?;
    m.add_class::<peripheral::PyAdvertisement>()?;
    m.add_class::<peripheral::PyPeripheralBuilder>()?;
    m.add_class::<peripheral::PyPeripheral>()?;
    Ok(())
}
