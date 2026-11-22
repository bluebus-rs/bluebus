//! `Session`, `Adapter` and `Device`.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bluebus::ble;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use crate::errors::{check, invalid_argument};
use crate::peripheral::PyPeripheralBuilder;
use crate::streams::{Lazy, PyNotificationStream, PyScanStream};
use crate::types::{PyAdapterInfo, PyDeviceInfo, PyServiceInfo};

/// Seconds (float) → `Duration`.
pub fn seconds(py: Python<'_>, what: &str, secs: f64) -> PyResult<Duration> {
    Duration::try_from_secs_f64(secs).map_err(|_| {
        invalid_argument(
            py,
            format!("{what} must be a non-negative number of seconds, got {secs}"),
        )
    })
}

/// Bytes-like (`bytes`, `bytearray`, `memoryview`) or a sequence of ints.
pub fn bytes_arg(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(b) = value.extract::<Cow<'_, [u8]>>() {
        return Ok(b.into_owned());
    }
    if let Ok(view) = value.call_method0("tobytes") {
        if let Ok(b) = view.extract::<Cow<'_, [u8]>>() {
            return Ok(b.into_owned());
        }
    }
    value.extract::<Vec<u8>>()
}

fn transport(py: Python<'_>, value: Option<&str>) -> PyResult<Option<ble::Transport>> {
    let Some(value) = value else { return Ok(None) };
    match value
        .to_ascii_lowercase()
        .replace(['/', '-', '_'], "")
        .as_str()
    {
        "le" => Ok(Some(ble::Transport::Le)),
        "bredr" | "classic" => Ok(Some(ble::Transport::BrEdr)),
        "auto" => Ok(Some(ble::Transport::Auto)),
        _ => Err(invalid_argument(
            py,
            format!("transport must be \"le\", \"bredr\" or \"auto\", got {value:?}"),
        )),
    }
}

/// Keyword arguments shared by `scan`, `scan_stream` and `find_device`.
#[allow(clippy::too_many_arguments)]
fn scan_filter(
    py: Python<'_>,
    transport_: Option<&str>,
    name: Option<String>,
    name_prefix: Option<String>,
    address: Option<String>,
    service_uuids: Option<Vec<String>>,
    min_rssi: Option<i16>,
    duplicate_data: Option<bool>,
) -> PyResult<ble::ScanFilter> {
    let mut f = ble::ScanFilter::new();
    if let Some(t) = transport(py, transport_)? {
        f = f.transport(t);
    }
    match (name, name_prefix) {
        (Some(_), Some(_)) => {
            return Err(invalid_argument(
                py,
                "give either name or name_prefix, not both",
            ))
        }
        (Some(n), None) => f = f.name(n),
        (None, Some(p)) => f = f.name_prefix(p),
        (None, None) => {}
    }
    if let Some(a) = address {
        f = f.address(a);
    }
    if let Some(u) = service_uuids {
        f = f.service_uuids(u);
    }
    if let Some(r) = min_rssi {
        f = f.min_rssi(r);
    }
    if let Some(d) = duplicate_data {
        f = f.duplicate_data(d);
    }
    Ok(f)
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// The connection to BlueZ on the D-Bus system bus.
///
/// ```python
/// session = await bluebus.Session.new()   # or: await bluebus.Session()
/// adapter = await session.default_adapter()
/// ```
#[pyclass(name = "Session", module = "bluebus", frozen)]
pub struct PySession {
    inner: Arc<OnceLock<ble::Session>>,
}

impl PySession {
    fn get(&self, py: Python<'_>) -> PyResult<ble::Session> {
        self.inner.get().cloned().ok_or_else(|| {
            invalid_argument(
                py,
                "the session is not connected: use `await bluebus.Session()` or `await bluebus.Session.new()`",
            )
        })
    }
}

#[pymethods]
impl PySession {
    /// `bluebus.Session()` must be awaited: `session = await bluebus.Session()`.
    #[new]
    fn py_new() -> Self {
        PySession {
            inner: Arc::new(OnceLock::new()),
        }
    }

    /// Connects to BlueZ (honours `DBUS_SYSTEM_BUS_ADDRESS`).
    #[staticmethod]
    #[pyo3(name = "new")]
    fn connect(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        future_into_py(py, async move {
            let session = check(ble::Session::new().await)?;
            let inner = Arc::new(OnceLock::new());
            let _ = inner.set(session);
            Ok(PySession { inner })
        })
    }

    fn __await__<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let inner = slf.get().inner.clone();
        let this = slf.clone().unbind();
        future_into_py(slf.py(), async move {
            if inner.get().is_none() {
                let session = check(ble::Session::new().await)?;
                let _ = inner.set(session);
            }
            Ok(this)
        })?
        .call_method0("__await__")
    }

    /// The local adapters, sorted by name.
    fn adapters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let s = self.get(py)?;
        future_into_py(py, async move {
            let list = check(s.adapters().await)?;
            Ok(list
                .into_iter()
                .map(PyAdapterInfo::from)
                .collect::<Vec<_>>())
        })
    }

    /// An adapter by name (`"hci1"`), object path or MAC address; `None`
    /// selects the default adapter.
    #[pyo3(signature = (name=None))]
    fn adapter<'py>(&self, py: Python<'py>, name: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        let s = self.get(py)?;
        future_into_py(py, async move {
            let adapter = check(s.adapter(name.as_deref()).await)?;
            Ok(PyAdapter { inner: adapter })
        })
    }

    /// The default adapter (`hci0` if present, else the first one).
    fn default_adapter<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.adapter(py, None)
    }

    /// Registers a pairing agent and makes it BlueZ' default agent.
    ///
    /// `set_agent("auto")` accepts every confirmation ("Just Works");
    /// `set_agent(passkey=123456)` answers passkey (and PIN) requests.
    #[pyo3(signature = (mode=None, *, passkey=None, pin_code=None, capability=None))]
    fn set_agent<'py>(
        &self,
        py: Python<'py>,
        mode: Option<&str>,
        passkey: Option<u32>,
        pin_code: Option<String>,
        capability: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let s = self.get(py)?;
        let capability = match capability {
            None => None,
            Some(c) => Some(
                match c.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
                    "displayonly" => ble::AgentCapability::DisplayOnly,
                    "displayyesno" => ble::AgentCapability::DisplayYesNo,
                    "keyboardonly" => ble::AgentCapability::KeyboardOnly,
                    "noinputnooutput" => ble::AgentCapability::NoInputNoOutput,
                    "keyboarddisplay" => ble::AgentCapability::KeyboardDisplay,
                    _ => {
                        return Err(invalid_argument(
                            py,
                            format!("unknown agent capability {c:?}"),
                        ))
                    }
                },
            ),
        };
        let mode = match mode {
            Some(m) => m.to_ascii_lowercase(),
            None if passkey.is_some() || pin_code.is_some() => "passkey".into(),
            None => "auto".into(),
        };
        match mode.as_str() {
            "auto" => {
                if passkey.is_some() || pin_code.is_some() {
                    return Err(invalid_argument(
                        py,
                        "the \"auto\" agent takes no passkey or pin_code",
                    ));
                }
                let cap = capability.unwrap_or(ble::AgentCapability::NoInputNoOutput);
                future_into_py(py, async move {
                    check(s.set_agent(cap, ble::AutoAcceptAgent).await)
                })
            }
            "passkey" => {
                let Some(key) = passkey else {
                    return Err(invalid_argument(py, "the passkey agent needs passkey=…"));
                };
                if key > 999_999 {
                    return Err(invalid_argument(py, "passkey must be 0…999999"));
                }
                let mut agent = ble::PasskeyAgent::new(key);
                if let Some(pin) = pin_code {
                    agent = agent.with_pin_code(pin);
                }
                let cap = capability.unwrap_or(ble::AgentCapability::KeyboardDisplay);
                future_into_py(py, async move { check(s.set_agent(cap, agent).await) })
            }
            other => Err(invalid_argument(
                py,
                format!("agent mode must be \"auto\" or \"passkey\", got {other:?}"),
            )),
        }
    }

    /// Unregisters the agent set with `set_agent` (no-op if none).
    fn remove_agent<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let s = self.get(py)?;
        future_into_py(py, async move { check(s.remove_agent().await) })
    }

    fn __repr__(&self) -> String {
        match self.inner.get() {
            Some(s) => format!(
                "<bluebus.Session {}>",
                s.connection()
                    .unique_name()
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            ),
            None => "<bluebus.Session (not connected)>".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// A local Bluetooth adapter (`hci0`, …).
#[pyclass(name = "Adapter", module = "bluebus", frozen)]
pub struct PyAdapter {
    pub inner: ble::Adapter,
}

#[pymethods]
impl PyAdapter {
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn address(&self) -> &str {
        self.inner.address()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path()
    }

    /// Current properties.
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let a = self.inner.clone();
        future_into_py(py, async move {
            Ok(PyAdapterInfo::from(check(a.info().await)?))
        })
    }

    fn is_powered<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let a = self.inner.clone();
        future_into_py(py, async move { check(a.is_powered().await) })
    }

    /// Powers the adapter on and waits until BlueZ reports it.
    fn power_on<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let a = self.inner.clone();
        future_into_py(py, async move { check(a.power_on().await) })
    }

    fn power_off<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let a = self.inner.clone();
        future_into_py(py, async move { check(a.power_off().await) })
    }

    /// Scans for `timeout` seconds; returns the matching devices sorted by
    /// address.
    #[pyo3(signature = (timeout=5.0, *, transport=None, name=None, name_prefix=None, address=None, service_uuids=None, min_rssi=None, duplicate_data=None))]
    #[allow(clippy::too_many_arguments)]
    fn scan<'py>(
        &self,
        py: Python<'py>,
        timeout: f64,
        transport: Option<&str>,
        name: Option<String>,
        name_prefix: Option<String>,
        address: Option<String>,
        service_uuids: Option<Vec<String>>,
        min_rssi: Option<i16>,
        duplicate_data: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let duration = seconds(py, "timeout", timeout)?;
        let filter = scan_filter(
            py,
            transport,
            name,
            name_prefix,
            address,
            service_uuids,
            min_rssi,
            duplicate_data,
        )?;
        let a = self.inner.clone();
        future_into_py(py, async move {
            let found = check(a.scan(filter, duration).await)?;
            Ok(found
                .into_iter()
                .map(PyDeviceInfo::from)
                .collect::<Vec<_>>())
        })
    }

    /// An async iterator of `ScanEvent`s; the scan runs until the iterator
    /// is closed (`break`, `aclose()` or `async with`).
    #[pyo3(signature = (*, transport=None, name=None, name_prefix=None, address=None, service_uuids=None, min_rssi=None, duplicate_data=None))]
    #[allow(clippy::too_many_arguments)]
    fn scan_stream(
        &self,
        py: Python<'_>,
        transport: Option<&str>,
        name: Option<String>,
        name_prefix: Option<String>,
        address: Option<String>,
        service_uuids: Option<Vec<String>>,
        min_rssi: Option<i16>,
        duplicate_data: Option<bool>,
    ) -> PyResult<PyScanStream> {
        let filter = scan_filter(
            py,
            transport,
            name,
            name_prefix,
            address,
            service_uuids,
            min_rssi,
            duplicate_data,
        )?;
        let a = self.inner.clone();
        Ok(PyScanStream {
            lazy: Lazy::new(Box::new(move || {
                Box::pin(async move { a.scan_stream(filter).await })
            })),
        })
    }

    /// Scans until a device matches and returns it (`BluebusTimeout` after
    /// `timeout` seconds). A connected device given by `address` is
    /// returned without scanning.
    #[pyo3(signature = (*, name=None, name_prefix=None, address=None, service_uuids=None, min_rssi=None, transport=None, timeout=10.0))]
    #[allow(clippy::too_many_arguments)]
    fn find_device<'py>(
        &self,
        py: Python<'py>,
        name: Option<String>,
        name_prefix: Option<String>,
        address: Option<String>,
        service_uuids: Option<Vec<String>>,
        min_rssi: Option<i16>,
        transport: Option<&str>,
        timeout: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let duration = seconds(py, "timeout", timeout)?;
        let filter = scan_filter(
            py,
            transport,
            name,
            name_prefix,
            address,
            service_uuids,
            min_rssi,
            None,
        )?;
        let a = self.inner.clone();
        future_into_py(py, async move {
            let device = check(a.find_device(filter, duration).await)?;
            Ok(PyDevice { inner: device })
        })
    }

    /// A handle to the device with this MAC address (no I/O; the device
    /// need not be known yet).
    fn device(&self, address: &str) -> PyResult<PyDevice> {
        Ok(PyDevice {
            inner: check(self.inner.device(address))?,
        })
    }

    /// Devices BlueZ knows on this adapter (discovered, paired, …).
    fn devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let a = self.inner.clone();
        future_into_py(py, async move {
            let list = check(a.devices().await)?;
            Ok(list.into_iter().map(PyDeviceInfo::from).collect::<Vec<_>>())
        })
    }

    /// Removes a device (and its pairing) from BlueZ.
    fn remove_device<'py>(&self, py: Python<'py>, address: String) -> PyResult<Bound<'py, PyAny>> {
        let a = self.inner.clone();
        future_into_py(py, async move { check(a.remove_device(&address).await) })
    }

    /// Builder of a local GATT server and advertisement.
    fn peripheral(&self) -> PyPeripheralBuilder {
        PyPeripheralBuilder::new(self.inner.clone())
    }

    fn __repr__(&self) -> String {
        format!(
            "<bluebus.Adapter {} {}>",
            self.inner.name(),
            self.inner.address()
        )
    }
}

// ---------------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------------

/// A remote device: connection, pairing and GATT client. Characteristics
/// are addressed by UUID (`"2a19"`, `"0x2A19"` or the full form).
#[pyclass(name = "Device", module = "bluebus", frozen)]
pub struct PyDevice {
    pub inner: ble::Device,
}

#[pymethods]
impl PyDevice {
    #[getter]
    fn address(&self) -> &str {
        self.inner.address()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path()
    }

    /// Name of the adapter this handle belongs to.
    #[getter]
    fn adapter_name(&self) -> &str {
        self.inner.adapter().name()
    }

    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(
            py,
            async move { Ok(PyDeviceInfo::from(check(d.info().await)?)) },
        )
    }

    /// Connects and waits until the services are resolved.
    #[pyo3(signature = (timeout=20.0))]
    fn connect<'py>(&self, py: Python<'py>, timeout: f64) -> PyResult<Bound<'py, PyAny>> {
        let duration = seconds(py, "timeout", timeout)?;
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.connect(duration).await) })
    }

    fn disconnect<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.disconnect().await) })
    }

    /// `async with device.connected(timeout=20.0):` connects on entry and
    /// disconnects on exit.
    #[pyo3(signature = (timeout=20.0))]
    fn connected(slf: Bound<'_, Self>, timeout: f64) -> PyResult<PyConnection> {
        let duration = seconds(slf.py(), "timeout", timeout)?;
        Ok(PyConnection {
            device: slf.unbind(),
            timeout: duration,
        })
    }

    fn is_connected<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.is_connected().await) })
    }

    /// Pairs (uses the agent set with `Session.set_agent`).
    fn pair<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.pair().await) })
    }

    #[pyo3(signature = (trusted=true))]
    fn trust<'py>(&self, py: Python<'py>, trusted: bool) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.trust(trusted).await) })
    }

    fn rssi<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.rssi().await) })
    }

    /// Battery level in percent (Battery1, else characteristic 0x2A19).
    fn battery_level<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.battery_level().await) })
    }

    /// The GATT services with their characteristics and descriptors.
    fn services<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move {
            let list = check(d.services().await)?;
            Ok(list
                .into_iter()
                .map(PyServiceInfo::from)
                .collect::<Vec<_>>())
        })
    }

    /// Reads a characteristic.
    fn read<'py>(&self, py: Python<'py>, uuid: String) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.read(&uuid).await) })
    }

    /// Writes a characteristic: a write request if `response` (default),
    /// else a write command (write without response).
    #[pyo3(signature = (uuid, data, *, response=true))]
    fn write<'py>(
        &self,
        py: Python<'py>,
        uuid: String,
        data: &Bound<'py, PyAny>,
        response: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let data = bytes_arg(data)?;
        let kind = if response {
            ble::WriteType::WithResponse
        } else {
            ble::WriteType::WithoutResponse
        };
        let d = self.inner.clone();
        future_into_py(py, async move { check(d.write(&uuid, data, kind).await) })
    }

    fn read_descriptor<'py>(
        &self,
        py: Python<'py>,
        characteristic: String,
        descriptor: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let d = self.inner.clone();
        future_into_py(py, async move {
            check(d.read_descriptor(&characteristic, &descriptor).await)
        })
    }

    fn write_descriptor<'py>(
        &self,
        py: Python<'py>,
        characteristic: String,
        descriptor: String,
        data: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let data = bytes_arg(data)?;
        let d = self.inner.clone();
        future_into_py(py, async move {
            check(d.write_descriptor(&characteristic, &descriptor, data).await)
        })
    }

    /// An async iterator of notification values (`bytes`). Subscribes on
    /// `await`, `async with` or the first iteration; unsubscribes on
    /// `aclose()` / exit.
    fn notifications(&self, uuid: String) -> PyNotificationStream {
        let d = self.inner.clone();
        let key = uuid.clone();
        PyNotificationStream {
            lazy: Lazy::new(Box::new(move || {
                Box::pin(async move { d.notifications(&key).await })
            })),
            uuid,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "<bluebus.Device {} on {}>",
            self.inner.address(),
            self.inner.adapter().name()
        )
    }
}

/// Async context manager returned by `Device.connected()`.
#[pyclass(name = "Connection", module = "bluebus", frozen)]
pub struct PyConnection {
    device: Py<PyDevice>,
    timeout: Duration,
}

#[pymethods]
impl PyConnection {
    fn __aenter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let d = self.device.get().inner.clone();
        let device = self.device.clone_ref(py);
        let timeout = self.timeout;
        future_into_py(py, async move {
            check(d.connect(timeout).await)?;
            Ok(device)
        })
    }

    #[pyo3(signature = (exc_type, _exc, _tb))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        exc_type: &Bound<'py, PyAny>,
        _exc: &Bound<'py, PyAny>,
        _tb: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let failing = !exc_type.is_none();
        let d = self.device.get().inner.clone();
        future_into_py(py, async move {
            let r = d.disconnect().await;
            // Do not mask the exception raised in the block.
            if !failing {
                check(r)?;
            }
            Ok(false)
        })
    }
}
