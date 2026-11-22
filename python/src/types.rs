//! Read-only data classes: snapshots returned by the API.

use std::collections::HashMap;
use std::fmt::Write as _;

use bluebus::ble;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::py_bool;

/// Python-style repr of an optional string.
fn opt_str(v: &Option<String>) -> String {
    match v {
        Some(s) => format!("{s:?}").replacen('"', "'", 2),
        None => "None".into(),
    }
}

fn quoted(s: &str) -> String {
    format!("{s:?}").replacen('"', "'", 2)
}

fn opt_num<T: std::fmt::Display>(v: &Option<T>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => "None".into(),
    }
}

fn bytes_repr(b: &[u8]) -> String {
    let mut s = String::from("b'");
    for &c in b {
        match c {
            b'\\' => s.push_str("\\\\"),
            b'\'' => s.push_str("\\'"),
            0x20..=0x7e => s.push(c as char),
            _ => {
                let _ = write!(s, "\\x{c:02x}");
            }
        }
    }
    s.push('\'');
    s
}

/// A local Bluetooth adapter (snapshot).
#[pyclass(name = "AdapterInfo", module = "bluebus", frozen, eq)]
#[derive(Clone, PartialEq)]
pub struct PyAdapterInfo {
    /// Name such as `"hci0"`.
    #[pyo3(get)]
    name: String,
    /// D-Bus object path (`/org/bluez/hci0`).
    #[pyo3(get)]
    path: String,
    /// MAC address.
    #[pyo3(get)]
    address: String,
    #[pyo3(get)]
    alias: String,
    #[pyo3(get)]
    powered: bool,
    #[pyo3(get)]
    discovering: bool,
}

impl From<ble::AdapterInfo> for PyAdapterInfo {
    fn from(a: ble::AdapterInfo) -> Self {
        PyAdapterInfo {
            name: a.name,
            path: a.path,
            address: a.address,
            alias: a.alias,
            powered: a.powered,
            discovering: a.discovering,
        }
    }
}

#[pymethods]
impl PyAdapterInfo {
    fn __repr__(&self) -> String {
        format!(
            "AdapterInfo(name={}, address={}, alias={}, powered={}, discovering={})",
            quoted(&self.name),
            quoted(&self.address),
            quoted(&self.alias),
            py_bool(self.powered),
            py_bool(self.discovering)
        )
    }
}

/// A remote device as seen by BlueZ (snapshot).
#[pyclass(name = "DeviceInfo", module = "bluebus", frozen, eq)]
#[derive(Clone, PartialEq)]
pub struct PyDeviceInfo {
    #[pyo3(get)]
    address: String,
    #[pyo3(get)]
    address_type: Option<String>,
    #[pyo3(get)]
    name: Option<String>,
    #[pyo3(get)]
    alias: String,
    #[pyo3(get)]
    rssi: Option<i16>,
    #[pyo3(get)]
    tx_power: Option<i16>,
    #[pyo3(get)]
    uuids: Vec<String>,
    manufacturer_data: HashMap<u16, Vec<u8>>,
    service_data: HashMap<String, Vec<u8>>,
    #[pyo3(get)]
    connected: bool,
    #[pyo3(get)]
    paired: bool,
    #[pyo3(get)]
    trusted: bool,
    #[pyo3(get)]
    services_resolved: bool,
}

impl From<ble::DeviceInfo> for PyDeviceInfo {
    fn from(d: ble::DeviceInfo) -> Self {
        PyDeviceInfo {
            address: d.address,
            address_type: d.address_type,
            name: d.name,
            alias: d.alias,
            rssi: d.rssi,
            tx_power: d.tx_power,
            uuids: d.uuids,
            manufacturer_data: d.manufacturer_data,
            service_data: d.service_data,
            connected: d.connected,
            paired: d.paired,
            trusted: d.trusted,
            services_resolved: d.services_resolved,
        }
    }
}

#[pymethods]
impl PyDeviceInfo {
    /// Company identifier → payload.
    #[getter]
    fn manufacturer_data<'py>(&self, py: Python<'py>) -> HashMap<u16, Bound<'py, PyBytes>> {
        self.manufacturer_data
            .iter()
            .map(|(k, v)| (*k, PyBytes::new(py, v)))
            .collect()
    }

    /// Service UUID (full form) → payload.
    #[getter]
    fn service_data<'py>(&self, py: Python<'py>) -> HashMap<String, Bound<'py, PyBytes>> {
        self.service_data
            .iter()
            .map(|(k, v)| (k.clone(), PyBytes::new(py, v)))
            .collect()
    }

    fn __repr__(&self) -> String {
        let mut manufacturer: Vec<_> = self.manufacturer_data.iter().collect();
        manufacturer.sort();
        let manufacturer = manufacturer
            .iter()
            .map(|(k, v)| format!("0x{k:04x}: {}", bytes_repr(v)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "DeviceInfo(address={}, name={}, rssi={}, manufacturer_data={{{manufacturer}}}, \
             connected={}, paired={})",
            quoted(&self.address),
            opt_str(&self.name),
            opt_num(&self.rssi),
            py_bool(self.connected),
            py_bool(self.paired)
        )
    }
}

/// A discovered GATT descriptor.
#[pyclass(name = "DescriptorInfo", module = "bluebus", frozen, eq)]
#[derive(Clone, PartialEq)]
pub struct PyDescriptorInfo {
    #[pyo3(get)]
    uuid: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    flags: Vec<String>,
}

#[pymethods]
impl PyDescriptorInfo {
    fn __repr__(&self) -> String {
        format!(
            "DescriptorInfo(uuid={}, flags={:?})",
            quoted(&self.uuid),
            self.flags
        )
        .replace('"', "'")
    }
}

/// A discovered GATT characteristic.
#[pyclass(name = "CharacteristicInfo", module = "bluebus", frozen, eq)]
#[derive(Clone, PartialEq)]
pub struct PyCharacteristicInfo {
    #[pyo3(get)]
    uuid: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    flags: Vec<String>,
    #[pyo3(get)]
    descriptors: Vec<PyDescriptorInfo>,
}

#[pymethods]
impl PyCharacteristicInfo {
    fn __repr__(&self) -> String {
        format!(
            "CharacteristicInfo(uuid={}, flags={:?}, descriptors={})",
            quoted(&self.uuid),
            self.flags,
            self.descriptors.len()
        )
        .replace('"', "'")
    }
}

/// A discovered GATT service with its characteristics.
#[pyclass(name = "ServiceInfo", module = "bluebus", frozen, eq)]
#[derive(Clone, PartialEq)]
pub struct PyServiceInfo {
    #[pyo3(get)]
    uuid: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    characteristics: Vec<PyCharacteristicInfo>,
}

#[pymethods]
impl PyServiceInfo {
    fn __repr__(&self) -> String {
        format!(
            "ServiceInfo(uuid={}, characteristics={})",
            quoted(&self.uuid),
            self.characteristics.len()
        )
    }
}

impl From<ble::ServiceInfo> for PyServiceInfo {
    fn from(s: ble::ServiceInfo) -> Self {
        PyServiceInfo {
            uuid: s.uuid,
            path: s.path,
            characteristics: s
                .characteristics
                .into_iter()
                .map(|c| PyCharacteristicInfo {
                    uuid: c.uuid,
                    path: c.path,
                    flags: c.flags,
                    descriptors: c
                        .descriptors
                        .into_iter()
                        .map(|d| PyDescriptorInfo {
                            uuid: d.uuid,
                            path: d.path,
                            flags: d.flags,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// An event of `Adapter.scan_stream`: `kind` is `"found"` (first report of
/// a device) or `"updated"` (new RSSI, name, advertising data, …).
#[pyclass(name = "ScanEvent", module = "bluebus", frozen, eq)]
#[derive(Clone, PartialEq)]
pub struct PyScanEvent {
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    device: PyDeviceInfo,
}

impl From<ble::ScanEvent> for PyScanEvent {
    fn from(e: ble::ScanEvent) -> Self {
        let kind = if matches!(e, ble::ScanEvent::Found(_)) {
            "found"
        } else {
            "updated"
        };
        PyScanEvent {
            kind,
            device: e.into_device().into(),
        }
    }
}

#[pymethods]
impl PyScanEvent {
    fn __repr__(&self) -> String {
        format!(
            "ScanEvent(kind={}, device={})",
            quoted(self.kind),
            self.device.__repr__()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reprs() {
        assert_eq!(bytes_repr(b"a\x01'\\"), r"b'a\x01\'\\'");
        assert_eq!(opt_str(&Some("x".into())), "'x'");
        assert_eq!(opt_str(&None), "None");
        assert_eq!(opt_num(&Some(-4)), "-4");
    }
}
