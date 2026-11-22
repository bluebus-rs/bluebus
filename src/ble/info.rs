//! Plain-data snapshots returned by the high-level API.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Str};

/// A snapshot of a local Bluetooth adapter, as returned by
/// [`Session::adapters`](super::Session::adapters).
///
/// ```no_run
/// # async fn f() -> bluebus::ble::Result<()> {
/// let session = bluebus::ble::Session::new().await?;
/// for a in session.adapters().await? {
///     println!("{} {} powered={}", a.name, a.address, a.powered);
/// }
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct AdapterInfo {
    /// Short name, e.g. `"hci0"`.
    pub name: String,
    /// D-Bus object path, e.g. `"/org/bluez/hci0"`.
    pub path: String,
    /// MAC address.
    pub address: String,
    /// Friendly name.
    pub alias: String,
    /// Whether the radio is on.
    pub powered: bool,
    /// Whether a discovery is running (from any client).
    pub discovering: bool,
}

impl AdapterInfo {
    pub(crate) fn from_properties(path: &str, props: &HashMap<String, OwnedValue>) -> Self {
        AdapterInfo {
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            path: path.to_owned(),
            address: props.get("Address").and_then(string).unwrap_or_default(),
            alias: props.get("Alias").and_then(string).unwrap_or_default(),
            powered: props.get("Powered").and_then(boolean).unwrap_or(false),
            discovering: props.get("Discovering").and_then(boolean).unwrap_or(false),
        }
    }
}

/// A snapshot of a remote device: what it advertises and its link state.
///
/// Returned by scans ([`Adapter::scan`](super::Adapter::scan),
/// [`ScanEvent`](super::ScanEvent)), [`Adapter::devices`](super::Adapter::devices)
/// and [`Device::info`](super::Device::info).
///
/// ```no_run
/// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
/// use std::time::Duration;
/// for d in adapter.scan(Default::default(), Duration::from_secs(5)).await? {
///     println!("{} {:?} rssi={:?} uuids={:?}", d.address, d.name, d.rssi, d.uuids);
///     if let Some(data) = d.manufacturer_data.get(&0x004C) {
///         println!("  Apple data: {data:02x?}");
///     }
/// }
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DeviceInfo {
    /// MAC address, upper case (`AA:BB:CC:DD:EE:FF`).
    pub address: String,
    /// `"public"` or `"random"`, when known.
    pub address_type: Option<String>,
    /// Remote name, when known.
    pub name: Option<String>,
    /// Friendly name: the user-set alias, else the name, else an
    /// address-based string.
    pub alias: String,
    /// Signal strength of the last advertisement in dBm (only while seen
    /// by a running discovery).
    pub rssi: Option<i16>,
    /// Advertised TX power in dBm.
    pub tx_power: Option<i16>,
    /// Advertised or resolved service UUIDs (128-bit, lower case).
    pub uuids: Vec<String>,
    /// Manufacturer specific data: company identifier → bytes.
    pub manufacturer_data: HashMap<u16, Vec<u8>>,
    /// Service data: 128-bit service UUID → bytes.
    pub service_data: HashMap<String, Vec<u8>>,
    /// Whether the device is connected.
    pub connected: bool,
    /// Whether the device is paired.
    pub paired: bool,
    /// Whether the device is trusted.
    pub trusted: bool,
    /// Whether the GATT database has been resolved (after connecting).
    pub services_resolved: bool,
}

pub(crate) fn string(v: &OwnedValue) -> Option<String> {
    v.downcast_ref::<Str>().ok().map(|s| s.as_str().to_owned())
}

pub(crate) fn boolean(v: &OwnedValue) -> Option<bool> {
    v.downcast_ref::<bool>().ok()
}

pub(crate) fn strings(v: &OwnedValue) -> Option<Vec<String>> {
    Vec::<String>::try_from(v.try_clone().ok()?).ok()
}

fn byte_map<K: Eq + std::hash::Hash>(v: &OwnedValue) -> HashMap<K, Vec<u8>>
where
    HashMap<K, OwnedValue>: TryFrom<OwnedValue>,
{
    let Some(map) = v
        .try_clone()
        .ok()
        .and_then(|v| HashMap::<K, OwnedValue>::try_from(v).ok())
    else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(k, v)| Some((k, crate::advertising_bytes(&v)?)))
        .collect()
}

impl DeviceInfo {
    /// Builds a snapshot from `org.bluez.Device1` properties. `None` without
    /// an `Address`.
    pub(crate) fn from_properties(props: &HashMap<String, OwnedValue>) -> Option<Self> {
        let address = props.get("Address").and_then(string)?;
        let mut info = DeviceInfo {
            alias: address.clone(),
            address,
            ..Default::default()
        };
        for (name, value) in props {
            info.apply(name, value);
        }
        Some(info)
    }

    /// Applies one changed property.
    pub(crate) fn apply(&mut self, name: &str, v: &OwnedValue) {
        match name {
            "Address" => {
                if let Some(s) = string(v) {
                    self.address = s;
                }
            }
            "Alias" => {
                if let Some(s) = string(v) {
                    self.alias = s;
                }
            }
            "AddressType" => self.address_type = string(v),
            "Name" => self.name = string(v),
            "RSSI" => self.rssi = v.downcast_ref::<i16>().ok(),
            "TxPower" => self.tx_power = v.downcast_ref::<i16>().ok(),
            "UUIDs" => {
                self.uuids = strings(v)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|u| u.to_ascii_lowercase())
                    .collect()
            }
            "ManufacturerData" => self.manufacturer_data = byte_map::<u16>(v),
            "ServiceData" => {
                self.service_data = byte_map::<String>(v)
                    .into_iter()
                    .map(|(k, v)| (k.to_ascii_lowercase(), v))
                    .collect()
            }
            "Connected" => self.connected = boolean(v).unwrap_or(self.connected),
            "Paired" => self.paired = boolean(v).unwrap_or(self.paired),
            "Trusted" => self.trusted = boolean(v).unwrap_or(self.trusted),
            "ServicesResolved" => {
                self.services_resolved = boolean(v).unwrap_or(self.services_resolved)
            }
            _ => {}
        }
    }

    /// Clears a property BlueZ reported as invalidated.
    pub(crate) fn invalidate(&mut self, name: &str) {
        match name {
            "RSSI" => self.rssi = None,
            "TxPower" => self.tx_power = None,
            "Name" => self.name = None,
            "AddressType" => self.address_type = None,
            "UUIDs" => self.uuids.clear(),
            "ManufacturerData" => self.manufacturer_data.clear(),
            "ServiceData" => self.service_data.clear(),
            _ => {}
        }
    }
}

/// A GATT service of a connected device, with its characteristics.
/// Returned by [`Device::services`](super::Device::services).
///
/// ```no_run
/// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
/// for service in device.services().await? {
///     println!("service {}", service.uuid);
///     for c in &service.characteristics {
///         println!("  characteristic {} {:?}", c.uuid, c.flags);
///         for d in &c.descriptors {
///             println!("    descriptor {}", d.uuid);
///         }
///     }
/// }
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ServiceInfo {
    /// 128-bit UUID, lower case.
    pub uuid: String,
    /// BlueZ object path.
    pub path: String,
    /// Characteristics of the service, in handle order.
    pub characteristics: Vec<CharacteristicInfo>,
}

/// A GATT characteristic of a connected device. See [`ServiceInfo`].
///
/// ```no_run
/// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
/// let services = device.services().await?;
/// let readable = services
///     .iter()
///     .flat_map(|s| &s.characteristics)
///     .filter(|c| c.flags.iter().any(|f| f == "read"));
/// for c in readable {
///     println!("{} = {:02x?}", c.uuid, device.read(&c.uuid).await?);
/// }
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct CharacteristicInfo {
    /// 128-bit UUID, lower case.
    pub uuid: String,
    /// BlueZ object path.
    pub path: String,
    /// Flags such as `"read"`, `"write"`, `"write-without-response"`,
    /// `"notify"`, `"indicate"`.
    pub flags: Vec<String>,
    /// Descriptors of the characteristic.
    pub descriptors: Vec<DescriptorInfo>,
}

/// A GATT descriptor of a connected device. See [`ServiceInfo`].
///
/// ```no_run
/// # async fn f(device: bluebus::ble::Device) -> bluebus::ble::Result<()> {
/// let services = device.services().await?;
/// for c in services.iter().flat_map(|s| &s.characteristics) {
///     for d in &c.descriptors {
///         println!("{} / {}: {:?}", c.uuid, d.uuid, d.flags);
///     }
/// }
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DescriptorInfo {
    /// 128-bit UUID, lower case.
    pub uuid: String,
    /// BlueZ object path.
    pub path: String,
    /// Flags such as `"read"`, `"write"`.
    pub flags: Vec<String>,
}

/// Builds the service tree from the flat database of the low-level API.
pub(crate) fn service_tree(db: &crate::GattDatabase) -> Vec<ServiceInfo> {
    db.services
        .iter()
        .map(|s| ServiceInfo {
            uuid: s.uuid.to_ascii_lowercase(),
            path: s.path.clone(),
            characteristics: db
                .characteristics
                .iter()
                .filter(|c| c.parent == s.path)
                .map(|c| CharacteristicInfo {
                    uuid: c.uuid.to_ascii_lowercase(),
                    path: c.path.clone(),
                    flags: c.flags.clone(),
                    descriptors: db
                        .descriptors
                        .iter()
                        .filter(|d| d.parent == c.path)
                        .map(|d| DescriptorInfo {
                            uuid: d.uuid.to_ascii_lowercase(),
                            path: d.path.clone(),
                            flags: d.flags.clone(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        v.into().try_to_owned().unwrap()
    }

    #[test]
    fn device_info_from_properties() {
        let md: HashMap<u16, Value> = HashMap::from([(0x004C, Value::from(vec![1u8, 2]))]);
        let sd: HashMap<&str, Value> = HashMap::from([(
            "0000180F-0000-1000-8000-00805F9B34FB",
            Value::from(vec![9u8]),
        )]);
        let props = HashMap::from([
            ("Address".to_string(), ov("AA:BB:CC:DD:EE:FF")),
            ("Name".to_string(), ov("Sensor")),
            ("RSSI".to_string(), ov(-40i16)),
            ("TxPower".to_string(), ov(4i16)),
            ("Connected".to_string(), ov(true)),
            (
                "UUIDs".to_string(),
                ov(vec!["0000180F-0000-1000-8000-00805F9B34FB"]),
            ),
            ("ManufacturerData".to_string(), ov(md)),
            ("ServiceData".to_string(), ov(sd)),
        ]);
        let mut d = DeviceInfo::from_properties(&props).unwrap();
        assert_eq!(d.alias, "AA:BB:CC:DD:EE:FF");
        assert_eq!(d.name.as_deref(), Some("Sensor"));
        assert_eq!(d.rssi, Some(-40));
        assert_eq!(d.tx_power, Some(4));
        assert!(d.connected && !d.paired);
        assert_eq!(d.uuids, vec!["0000180f-0000-1000-8000-00805f9b34fb"]);
        assert_eq!(d.manufacturer_data[&0x004C], vec![1, 2]);
        assert_eq!(
            d.service_data["0000180f-0000-1000-8000-00805f9b34fb"],
            vec![9]
        );
        d.invalidate("RSSI");
        d.invalidate("ManufacturerData");
        assert_eq!(d.rssi, None);
        assert!(d.manufacturer_data.is_empty());
        assert!(DeviceInfo::from_properties(&HashMap::new()).is_none());
    }

    #[test]
    fn adapter_info_from_properties() {
        let props = HashMap::from([
            ("Address".to_string(), ov("00:11:22:33:44:55")),
            ("Powered".to_string(), ov(true)),
        ]);
        let a = AdapterInfo::from_properties("/org/bluez/hci3", &props);
        assert_eq!(a.name, "hci3");
        assert!(a.powered && !a.discovering);
        assert_eq!(a.alias, "");
    }
}
