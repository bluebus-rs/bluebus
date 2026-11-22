//! Mapping of `bluebus::ble::Error` to the Python exceptions defined in
//! `bluebus/exceptions.py`.

use bluebus::ble;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

static BLUEBUS_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static TIMEOUT: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static NOT_FOUND: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static INVALID_ARGUMENT: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static DBUS_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

const MODULE: &str = "bluebus.exceptions";

fn class<'py>(
    py: Python<'py>,
    cell: &'static PyOnceLock<Py<PyType>>,
    name: &str,
) -> PyResult<&'py Bound<'py, PyType>> {
    cell.import(py, MODULE, name)
}

fn instantiate(
    py: Python<'_>,
    cell: &'static PyOnceLock<Py<PyType>>,
    name: &str,
    args: (String, Option<String>),
) -> PyErr {
    match class(py, cell, name).and_then(|cls| match args.1 {
        Some(n) => cls.call1((args.0, n)),
        None if name == "DBusError" => cls.call1((args.0, py.None())),
        None => cls.call1((args.0,)),
    }) {
        Ok(value) => PyErr::from_value(value),
        Err(e) => e,
    }
}

/// Converts a `ble::Error` (with the interpreter attached).
pub fn to_pyerr_with(py: Python<'_>, e: ble::Error) -> PyErr {
    let message = e.to_string();
    match e {
        ble::Error::Timeout(_) => instantiate(py, &TIMEOUT, "BluebusTimeout", (message, None)),
        ble::Error::NotFound(_) => instantiate(py, &NOT_FOUND, "NotFound", (message, None)),
        ble::Error::InvalidArgument(_) => {
            instantiate(py, &INVALID_ARGUMENT, "InvalidArgument", (message, None))
        }
        ble::Error::Bus(_) => {
            let name = e.dbus_error_name();
            instantiate(py, &DBUS_ERROR, "DBusError", (message, name))
        }
        _ => instantiate(py, &BLUEBUS_ERROR, "BluebusError", (message, None)),
    }
}

/// Converts a `ble::Error`, attaching to the interpreter for the duration
/// of the conversion (errors only, so this stays off the hot path).
pub fn to_pyerr(e: ble::Error) -> PyErr {
    Python::attach(|py| to_pyerr_with(py, e))
}

/// `ble::Result<T>` → `PyResult<T>`.
pub fn check<T>(r: ble::Result<T>) -> PyResult<T> {
    r.map_err(to_pyerr)
}

/// A `bluebus.InvalidArgument` raised from argument parsing.
pub fn invalid_argument(py: Python<'_>, message: impl Into<String>) -> PyErr {
    to_pyerr_with(py, ble::Error::InvalidArgument(message.into()))
}
