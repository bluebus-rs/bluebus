//! Async iterators over `ble::ScanStream` and `ble::NotificationStream`.
//!
//! The Rust stream is opened lazily (on the first `__anext__`) or eagerly
//! with `await stream` / `async with stream`. `aclose()` interrupts a
//! pending `__anext__` and stops the stream (StopDiscovery / StopNotify);
//! dropping the Python object without closing stops it in the background.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bluebus::ble;
use futures::future::BoxFuture;
use futures::{Stream, StreamExt};
use pyo3::exceptions::PyStopAsyncIteration;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use crate::errors::check;
use crate::types::PyScanEvent;

/// A stream that can be stopped asynchronously.
pub trait Stoppable: Stream + Send + Unpin + 'static {
    fn stop(self) -> BoxFuture<'static, ble::Result<()>>;
}

impl Stoppable for ble::ScanStream {
    fn stop(self) -> BoxFuture<'static, ble::Result<()>> {
        Box::pin(ble::ScanStream::stop(self))
    }
}

impl Stoppable for ble::NotificationStream {
    fn stop(self) -> BoxFuture<'static, ble::Result<()>> {
        Box::pin(ble::NotificationStream::stop(self))
    }
}

type Opener<S> = Box<dyn FnOnce() -> BoxFuture<'static, ble::Result<S>> + Send>;

pub struct Lazy<S: Stoppable> {
    opener: Mutex<Option<Opener<S>>>,
    stream: tokio::sync::Mutex<Option<S>>,
    closed: AtomicBool,
    close: tokio::sync::Notify,
}

impl<S: Stoppable> Lazy<S> {
    pub fn new(opener: Opener<S>) -> Arc<Self> {
        Arc::new(Lazy {
            opener: Mutex::new(Some(opener)),
            stream: tokio::sync::Mutex::new(None),
            closed: AtomicBool::new(false),
            close: tokio::sync::Notify::new(),
        })
    }

    fn take_opener(&self) -> Option<Opener<S>> {
        self.opener.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Opens the stream if this was not done yet.
    async fn open(&self, slot: &mut Option<S>) -> ble::Result<()> {
        if slot.is_none() {
            if let Some(opener) = self.take_opener() {
                *slot = Some(opener().await?);
            }
        }
        Ok(())
    }

    pub async fn start(self: Arc<Self>) -> PyResult<()> {
        let mut slot = self.stream.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        check(self.open(&mut slot).await)
    }

    /// The next item, or `None` once the stream ended or was closed.
    pub async fn next(self: Arc<Self>) -> PyResult<Option<S::Item>> {
        let closing = self.close.notified();
        tokio::pin!(closing);
        closing.as_mut().enable();
        let mut slot = self.stream.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Ok(None);
        }
        check(self.open(&mut slot).await)?;
        let Some(stream) = slot.as_mut() else {
            return Ok(None);
        };
        tokio::select! {
            item = stream.next() => Ok(item),
            _ = &mut closing => Ok(None),
        }
    }

    pub async fn close(self: Arc<Self>) -> PyResult<()> {
        self.closed.store(true, Ordering::SeqCst);
        self.close.notify_waiters();
        self.take_opener();
        let stream = self.stream.lock().await.take();
        match stream {
            Some(s) => check(s.stop().await),
            None => Ok(()),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

fn stop_iteration() -> PyErr {
    PyStopAsyncIteration::new_err(())
}

/// Async iterator of `ScanEvent`s returned by `Adapter.scan_stream()`.
///
/// ```python
/// async for event in adapter.scan_stream(transport="le"):
///     print(event.kind, event.device.address)
///     break  # or: await stream.aclose()
/// ```
#[pyclass(name = "ScanStream", module = "bluebus", frozen)]
pub struct PyScanStream {
    pub lazy: Arc<Lazy<ble::ScanStream>>,
}

#[pymethods]
impl PyScanStream {
    fn __aiter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lazy = self.lazy.clone();
        future_into_py(py, async move {
            match lazy.next().await? {
                Some(event) => Ok(PyScanEvent::from(event)),
                None => Err(stop_iteration()),
            }
        })
    }

    /// Starts the scan now (otherwise the first `__anext__` starts it).
    fn start<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let lazy = slf.get().lazy.clone();
        let this = slf.clone().unbind();
        future_into_py(slf.py(), async move {
            lazy.start().await?;
            Ok(this)
        })
    }

    fn __await__<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        Self::start(slf)?.call_method0("__await__")
    }

    fn __aenter__<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        Self::start(slf)
    }

    #[pyo3(signature = (*_args))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _args: &Bound<'py, pyo3::types::PyTuple>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.aclose(py)
    }

    /// Stops the scan (StopDiscovery) and ends the iteration.
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lazy = self.lazy.clone();
        future_into_py(py, lazy.close())
    }

    #[getter]
    fn closed(&self) -> bool {
        self.lazy.is_closed()
    }

    fn __repr__(&self) -> String {
        format!(
            "<bluebus.ScanStream closed={}>",
            crate::py_bool(self.lazy.is_closed())
        )
    }
}

/// Async iterator of notification values (`bytes`) returned by
/// `Device.notifications()`.
///
/// ```python
/// async with device.notifications("2a37") as values:  # subscribed here
///     async for value in values:
///         print(value.hex())
/// ```
#[pyclass(name = "NotificationStream", module = "bluebus", frozen)]
pub struct PyNotificationStream {
    pub lazy: Arc<Lazy<ble::NotificationStream>>,
    pub uuid: String,
}

#[pymethods]
impl PyNotificationStream {
    fn __aiter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lazy = self.lazy.clone();
        future_into_py(py, async move {
            match lazy.next().await? {
                Some(value) => Ok(value),
                None => Err(stop_iteration()),
            }
        })
    }

    /// Subscribes now (otherwise the first `__anext__` subscribes).
    fn start<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let lazy = slf.get().lazy.clone();
        let this = slf.clone().unbind();
        future_into_py(slf.py(), async move {
            lazy.start().await?;
            Ok(this)
        })
    }

    fn __await__<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        Self::start(slf)?.call_method0("__await__")
    }

    fn __aenter__<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        Self::start(slf)
    }

    #[pyo3(signature = (*_args))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _args: &Bound<'py, pyo3::types::PyTuple>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.aclose(py)
    }

    /// Unsubscribes (StopNotify once the last stream of the characteristic
    /// is closed) and ends the iteration.
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lazy = self.lazy.clone();
        future_into_py(py, lazy.close())
    }

    #[getter]
    fn uuid(&self) -> &str {
        &self.uuid
    }

    #[getter]
    fn closed(&self) -> bool {
        self.lazy.is_closed()
    }

    fn __repr__(&self) -> String {
        format!(
            "<bluebus.NotificationStream uuid='{}' closed={}>",
            self.uuid,
            crate::py_bool(self.lazy.is_closed())
        )
    }
}
