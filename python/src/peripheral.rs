//! Local GATT server and advertisement: `Characteristic`, `Service`,
//! `Advertisement`, `PeripheralBuilder`, `Peripheral`.
//!
//! `on_write` callbacks: BlueZ' write reaches the zbus object server, which
//! hands the value to a channel and returns at once. A dedicated thread
//! takes the values in order and, with the interpreter attached, schedules
//! `bluebus._callbacks.dispatch` on the asyncio loop that started the
//! peripheral (`loop.call_soon_threadsafe`). The callback therefore runs on
//! the event loop thread like any asyncio callback; exceptions are logged
//! (logger `"bluebus"`), coroutines are scheduled as tasks. Neither the
//! tokio runtime nor the D-Bus dispatcher ever waits for Python.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use bluebus::ble;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyBytes;
use pyo3_async_runtimes::tokio::future_into_py;

use crate::central::bytes_arg;
use crate::errors::{check, invalid_argument};

// ---------------------------------------------------------------------------
// Write callbacks
// ---------------------------------------------------------------------------

struct Job {
    callback: Arc<Py<PyAny>>,
    event_loop: Option<Arc<Py<PyAny>>>,
    value: Vec<u8>,
}

static DISPATCH: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn run_job(job: Job) {
    Python::attach(|py| {
        let result = DISPATCH
            .import(py, "bluebus._callbacks", "dispatch")
            .and_then(|dispatch| {
                let event_loop = match &job.event_loop {
                    Some(l) => l.bind(py).clone(),
                    None => py.None().into_bound(py),
                };
                dispatch.call1((
                    event_loop,
                    job.callback.bind(py),
                    PyBytes::new(py, &job.value),
                ))
            });
        if let Err(e) = result {
            // `dispatch` itself catches the callback's exceptions; this is
            // a failure to schedule it at all.
            e.print(py);
        }
        drop(job);
    });
}

fn callback_sender() -> &'static mpsc::Sender<Job> {
    static SENDER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("bluebus-callbacks".into())
            .spawn(move || {
                for job in rx {
                    run_job(job);
                }
            })
            .expect("spawn the bluebus callback thread");
        tx
    })
}

fn write_callback(
    callback: Arc<Py<PyAny>>,
    event_loop: Option<Arc<Py<PyAny>>>,
) -> impl Fn(Vec<u8>) + Send + Sync + 'static {
    move |value| {
        let _ = callback_sender().send(Job {
            callback: callback.clone(),
            event_loop: event_loop.clone(),
            value,
        });
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// A local GATT characteristic.
///
/// ```python
/// bluebus.Characteristic("2a19", read=True, notify=True, value=b"\x64")
/// bluebus.Characteristic("2a06", write=True, on_write=lambda v: print(v))
/// ```
#[pyclass(name = "Characteristic", module = "bluebus")]
pub struct PyCharacteristic {
    #[pyo3(get)]
    uuid: String,
    #[pyo3(get)]
    flags: Vec<String>,
    value: Vec<u8>,
    on_write: Option<Arc<Py<PyAny>>>,
    descriptors: Vec<(String, Vec<u8>)>,
}

#[pymethods]
impl PyCharacteristic {
    #[new]
    #[pyo3(signature = (uuid, *, read=false, write=false, write_without_response=false, notify=false, indicate=false, flags=None, value=None, on_write=None, descriptors=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        uuid: String,
        read: bool,
        write: bool,
        write_without_response: bool,
        notify: bool,
        indicate: bool,
        flags: Option<Vec<String>>,
        value: Option<&Bound<'_, PyAny>>,
        on_write: Option<Py<PyAny>>,
        descriptors: Option<BTreeMap<String, Py<PyAny>>>,
    ) -> PyResult<Self> {
        let mut all = Vec::new();
        for (on, flag) in [
            (read, "read"),
            (write, "write"),
            (write_without_response, "write-without-response"),
            (notify, "notify"),
            (indicate, "indicate"),
        ] {
            if on {
                all.push(flag.to_string());
            }
        }
        for flag in flags.unwrap_or_default() {
            if !all.contains(&flag) {
                all.push(flag);
            }
        }
        if let Some(cb) = &on_write {
            if !cb.bind(py).is_callable() {
                return Err(invalid_argument(py, "on_write must be callable"));
            }
        }
        let mut descs = Vec::new();
        for (uuid, v) in descriptors.unwrap_or_default() {
            descs.push((uuid, bytes_arg(v.bind(py))?));
        }
        Ok(PyCharacteristic {
            uuid,
            flags: all,
            value: match value {
                Some(v) => bytes_arg(v)?,
                None => Vec::new(),
            },
            on_write: on_write.map(Arc::new),
            descriptors: descs,
        })
    }

    /// Initial value.
    #[getter]
    fn value<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.value)
    }

    /// Adds a read-only descriptor; returns the characteristic.
    fn descriptor<'py>(
        mut slf: PyRefMut<'py, Self>,
        uuid: String,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let value = bytes_arg(value)?;
        slf.descriptors.push((uuid, value));
        Ok(slf)
    }

    fn __repr__(&self) -> String {
        format!(
            "Characteristic('{}', flags={:?}, on_write={})",
            self.uuid,
            self.flags,
            if self.on_write.is_some() {
                "…"
            } else {
                "None"
            }
        )
        .replace('"', "'")
    }
}

impl PyCharacteristic {
    fn build(&self, event_loop: &Option<Arc<Py<PyAny>>>) -> ble::Characteristic {
        let mut c = ble::Characteristic::new(self.uuid.clone()).value(self.value.clone());
        for flag in &self.flags {
            c = c.flag(flag.clone());
        }
        if let Some(cb) = &self.on_write {
            c = c.on_write(write_callback(cb.clone(), event_loop.clone()));
        }
        for (uuid, value) in &self.descriptors {
            c = c.descriptor(uuid.clone(), value.clone());
        }
        c
    }
}

/// A local GATT service.
///
/// ```python
/// bluebus.Service("180f").characteristic(bluebus.Characteristic("2a19", read=True))
/// ```
#[pyclass(name = "Service", module = "bluebus")]
pub struct PyService {
    #[pyo3(get)]
    uuid: String,
    #[pyo3(get)]
    primary: bool,
    characteristics: Vec<Py<PyCharacteristic>>,
}

#[pymethods]
impl PyService {
    #[new]
    #[pyo3(signature = (uuid, characteristics=None, *, primary=true))]
    fn new(
        uuid: String,
        characteristics: Option<Vec<Py<PyCharacteristic>>>,
        primary: bool,
    ) -> Self {
        PyService {
            uuid,
            primary,
            characteristics: characteristics.unwrap_or_default(),
        }
    }

    /// Adds a characteristic; returns the service.
    fn characteristic<'py>(
        mut slf: PyRefMut<'py, Self>,
        characteristic: Py<PyCharacteristic>,
    ) -> PyRefMut<'py, Self> {
        slf.characteristics.push(characteristic);
        slf
    }

    #[getter]
    fn characteristics(&self, py: Python<'_>) -> Vec<Py<PyCharacteristic>> {
        self.characteristics
            .iter()
            .map(|c| c.clone_ref(py))
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "Service('{}', characteristics={})",
            self.uuid,
            self.characteristics.len()
        )
    }
}

impl PyService {
    fn build(&self, py: Python<'_>, event_loop: &Option<Arc<Py<PyAny>>>) -> ble::Service {
        let mut s = ble::Service::new(self.uuid.clone()).primary(self.primary);
        for c in &self.characteristics {
            s = s.characteristic(c.borrow(py).build(event_loop));
        }
        s
    }
}

/// LE advertising data.
///
/// ```python
/// bluebus.Advertisement(local_name="my-sensor", service_uuids=["180f"],
///                       manufacturer_data={0xFFFF: b"\x01\x02"})
/// ```
#[pyclass(name = "Advertisement", module = "bluebus", frozen)]
pub struct PyAdvertisement {
    #[pyo3(get)]
    local_name: Option<String>,
    #[pyo3(get)]
    service_uuids: Vec<String>,
    manufacturer_data: BTreeMap<u16, Vec<u8>>,
    service_data: BTreeMap<String, Vec<u8>>,
    #[pyo3(get)]
    discoverable: bool,
    #[pyo3(get)]
    connectable: bool,
}

#[pymethods]
impl PyAdvertisement {
    #[new]
    #[pyo3(signature = (*, local_name=None, service_uuids=None, manufacturer_data=None, service_data=None, discoverable=true, connectable=true))]
    fn new(
        py: Python<'_>,
        local_name: Option<String>,
        service_uuids: Option<Vec<String>>,
        manufacturer_data: Option<BTreeMap<u16, Py<PyAny>>>,
        service_data: Option<BTreeMap<String, Py<PyAny>>>,
        discoverable: bool,
        connectable: bool,
    ) -> PyResult<Self> {
        let mut manufacturer = BTreeMap::new();
        for (k, v) in manufacturer_data.unwrap_or_default() {
            manufacturer.insert(k, bytes_arg(v.bind(py))?);
        }
        let mut service = BTreeMap::new();
        for (k, v) in service_data.unwrap_or_default() {
            service.insert(k, bytes_arg(v.bind(py))?);
        }
        Ok(PyAdvertisement {
            local_name,
            service_uuids: service_uuids.unwrap_or_default(),
            manufacturer_data: manufacturer,
            service_data: service,
            discoverable,
            connectable,
        })
    }

    #[getter]
    fn manufacturer_data<'py>(&self, py: Python<'py>) -> BTreeMap<u16, Bound<'py, PyBytes>> {
        self.manufacturer_data
            .iter()
            .map(|(k, v)| (*k, PyBytes::new(py, v)))
            .collect()
    }

    #[getter]
    fn service_data<'py>(&self, py: Python<'py>) -> BTreeMap<String, Bound<'py, PyBytes>> {
        self.service_data
            .iter()
            .map(|(k, v)| (k.clone(), PyBytes::new(py, v)))
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "Advertisement(local_name={}, service_uuids={:?}, connectable={})",
            match &self.local_name {
                Some(n) => format!("'{n}'"),
                None => "None".into(),
            },
            self.service_uuids,
            if self.connectable { "True" } else { "False" }
        )
        .replace('"', "'")
    }
}

impl PyAdvertisement {
    fn build(&self) -> ble::Advertisement {
        let mut a = ble::Advertisement::new().discoverable(self.discoverable);
        if let Some(n) = &self.local_name {
            a = a.local_name(n.clone());
        }
        for u in &self.service_uuids {
            a = a.service_uuid(u.clone());
        }
        for (k, v) in &self.manufacturer_data {
            a = a.manufacturer_data(*k, v.clone());
        }
        for (k, v) in &self.service_data {
            a = a.service_data(k.clone(), v.clone());
        }
        if !self.connectable {
            a = a.broadcast();
        }
        a
    }
}

/// Returned by `Adapter.peripheral()`; `await builder.start()` exports the
/// services and registers the advertisement.
#[pyclass(name = "PeripheralBuilder", module = "bluebus")]
pub struct PyPeripheralBuilder {
    adapter: ble::Adapter,
    services: Vec<Py<PyService>>,
    advertisement: Option<Py<PyAdvertisement>>,
}

impl PyPeripheralBuilder {
    pub fn new(adapter: ble::Adapter) -> Self {
        PyPeripheralBuilder {
            adapter,
            services: Vec::new(),
            advertisement: None,
        }
    }
}

#[pymethods]
impl PyPeripheralBuilder {
    /// Adds a service; returns the builder.
    fn service<'py>(mut slf: PyRefMut<'py, Self>, service: Py<PyService>) -> PyRefMut<'py, Self> {
        slf.services.push(service);
        slf
    }

    /// Sets the advertisement; returns the builder.
    fn advertise<'py>(
        mut slf: PyRefMut<'py, Self>,
        advertisement: Py<PyAdvertisement>,
    ) -> PyRefMut<'py, Self> {
        slf.advertisement = Some(advertisement);
        slf
    }

    /// Exports the GATT application and registers the advertisement.
    /// `on_write` callbacks run on the event loop running this call.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let event_loop = pyo3_async_runtimes::get_running_loop(py)
            .ok()
            .map(|l| Arc::new(l.unbind()));
        let mut builder = self.adapter.peripheral();
        for s in &self.services {
            builder = builder.service(s.borrow(py).build(py, &event_loop));
        }
        if let Some(a) = &self.advertisement {
            builder = builder.advertise(a.get().build());
        }
        future_into_py(py, async move {
            let peripheral = check(builder.start().await)?;
            Ok(PyPeripheral { inner: peripheral })
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "<bluebus.PeripheralBuilder on {} services={} advertisement={}>",
            self.adapter.name(),
            self.services.len(),
            crate::py_bool(self.advertisement.is_some())
        )
    }
}

/// A running local GATT server and/or advertisement. Stops on `stop()`,
/// on `async with` exit, or (in the background) when garbage collected.
#[pyclass(name = "Peripheral", module = "bluebus", frozen)]
pub struct PyPeripheral {
    inner: ble::Peripheral,
}

#[pymethods]
impl PyPeripheral {
    /// Sets the value of a local characteristic and notifies subscribers.
    fn notify<'py>(
        &self,
        py: Python<'py>,
        uuid: String,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let value = bytes_arg(value)?;
        let p = self.inner.clone();
        future_into_py(py, async move { check(p.notify(&uuid, value).await) })
    }

    /// Sets the value returned to reads, without notifying.
    fn set_value<'py>(
        &self,
        py: Python<'py>,
        uuid: String,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let value = bytes_arg(value)?;
        let p = self.inner.clone();
        future_into_py(py, async move { check(p.set_value(&uuid, value).await) })
    }

    /// The current value (the last written or set one).
    fn value<'py>(&self, py: Python<'py>, uuid: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        future_into_py(py, async move { check(p.value(&uuid).await) })
    }

    /// Whether a central subscribed to the characteristic.
    fn is_notifying<'py>(&self, py: Python<'py>, uuid: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        future_into_py(py, async move { check(p.is_notifying(&uuid).await) })
    }

    /// `False` after `stop()`.
    #[getter]
    fn running(&self) -> bool {
        self.inner.is_running()
    }

    /// Unregisters the advertisement and the GATT application.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        future_into_py(py, async move { check(p.stop().await) })
    }

    fn __aenter__<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let this = slf.clone().unbind();
        future_into_py(slf.py(), async move { Ok(this) })
    }

    #[pyo3(signature = (*_args))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _args: &Bound<'py, pyo3::types::PyTuple>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        future_into_py(py, async move {
            check(p.stop().await)?;
            Ok(false)
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "<bluebus.Peripheral running={}>",
            crate::py_bool(self.inner.is_running())
        )
    }
}
