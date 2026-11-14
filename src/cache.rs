//! Snapshot of the devices known to BlueZ.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Str, Value};

use crate::ObjectManagerProxy;

/// D-Bus interface name of BlueZ devices.
pub const DEVICE_INTERFACE: &str = "org.bluez.Device1";

/// A plain-data snapshot of a BlueZ `org.bluez.Device1` object.
///
/// `address`, `alias`, `connected`, `paired`, `address_type`, `name`,
/// `trusted` and `rssi` map to `Device1` properties. The remaining optional
/// fields (`connectable`, `discoverable`, `discoverable_timeout`,
/// `discovering`, `pairable`, `pairable_timeout`, `power_state`, `powered`)
/// are *adapter* properties that BlueZ never exposes on devices; they are
/// kept for backward compatibility and are filled only if a property with
/// that name is present.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    pub address: String,
    pub alias: String,
    pub connected: bool,
    pub paired: bool,
    pub address_type: Option<String>,
    pub connectable: Option<bool>,
    pub discoverable: Option<bool>,
    pub discoverable_timeout: Option<u32>,
    pub discovering: Option<bool>,
    pub name: Option<String>,
    pub pairable: Option<bool>,
    pub pairable_timeout: Option<u32>,
    pub power_state: Option<String>,
    pub powered: Option<bool>,
    pub trusted: Option<bool>,
    pub rssi: Option<i16>,
}

fn as_string(v: &Value<'_>) -> Option<String> {
    v.downcast_ref::<Str>().ok().map(|s| s.as_str().to_owned())
}

fn as_bool(v: &Value<'_>) -> Option<bool> {
    v.downcast_ref::<bool>().ok()
}

fn as_u32(v: &Value<'_>) -> Option<u32> {
    v.downcast_ref::<u32>().ok()
}

impl DeviceInfo {
    /// Builds a snapshot from the property map of an `org.bluez.Device1`
    /// interface (as returned by `GetManagedObjects` or `InterfacesAdded`).
    ///
    /// Returns `None` if the mandatory `Address` property is missing.
    /// `Alias` falls back to the address, as BlueZ itself does.
    pub fn from_properties(props: &HashMap<String, OwnedValue>) -> Option<Self> {
        let address = props.get("Address").and_then(|v| as_string(v))?;
        let mut info = DeviceInfo {
            alias: address.clone(),
            address,
            ..Default::default()
        };
        for (name, value) in props {
            info.apply_property(name, value);
        }
        Some(info)
    }

    /// Updates a single field from a `Device1` property. Returns `true` if
    /// the property is known and the stored value actually changed.
    pub fn apply_property(&mut self, name: &str, value: &Value<'_>) -> bool {
        fn set<T: PartialEq>(slot: &mut T, new: Option<T>) -> bool {
            match new {
                Some(new) if *slot != new => {
                    *slot = new;
                    true
                }
                _ => false,
            }
        }
        fn set_opt<T: PartialEq>(slot: &mut Option<T>, new: Option<T>) -> bool {
            if new.is_some() && *slot != new {
                *slot = new;
                true
            } else {
                false
            }
        }
        match name {
            "Address" => set(&mut self.address, as_string(value)),
            "Alias" => set(&mut self.alias, as_string(value)),
            "Connected" => set(&mut self.connected, as_bool(value)),
            "Paired" => set(&mut self.paired, as_bool(value)),
            "AddressType" => set_opt(&mut self.address_type, as_string(value)),
            "Connectable" => set_opt(&mut self.connectable, as_bool(value)),
            "Discoverable" => set_opt(&mut self.discoverable, as_bool(value)),
            "DiscoverableTimeout" => set_opt(&mut self.discoverable_timeout, as_u32(value)),
            "Discovering" => set_opt(&mut self.discovering, as_bool(value)),
            "Name" => set_opt(&mut self.name, as_string(value)),
            "Pairable" => set_opt(&mut self.pairable, as_bool(value)),
            "PairableTimeout" => set_opt(&mut self.pairable_timeout, as_u32(value)),
            "PowerState" => set_opt(&mut self.power_state, as_string(value)),
            "Powered" => set_opt(&mut self.powered, as_bool(value)),
            "Trusted" => set_opt(&mut self.trusted, as_bool(value)),
            "RSSI" => set_opt(&mut self.rssi, value.downcast_ref::<i16>().ok()),
            _ => false,
        }
    }

    /// Clears a property that BlueZ reported as invalidated (for example
    /// `RSSI` when the device goes out of range). Returns `true` on change.
    pub fn invalidate_property(&mut self, name: &str) -> bool {
        fn clear<T>(slot: &mut Option<T>) -> bool {
            slot.take().is_some()
        }
        match name {
            "RSSI" => clear(&mut self.rssi),
            "Name" => clear(&mut self.name),
            "AddressType" => clear(&mut self.address_type),
            "Trusted" => clear(&mut self.trusted),
            _ => false,
        }
    }
}

/// Lists the devices of the default adapter ([`crate::get_adapter_path`]).
///
/// Opens a new system bus connection and returns an empty list on any
/// error. Prefer [`list_devices_with_connection`] to reuse a connection and
/// see errors.
pub async fn list_devices() -> Vec<DeviceInfo> {
    list_system_devices().await.unwrap_or_default()
}

/// Lists the devices of the default adapter ([`crate::get_adapter_path`])
/// over a new system bus connection.
pub async fn list_system_devices(
) -> Result<Vec<DeviceInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let conn = crate::get_system_connection().await?;
    Ok(list_devices_with_connection(&conn).await?)
}

/// Lists the devices of the default adapter ([`crate::get_adapter_path`])
/// using an existing connection.
pub async fn list_devices_with_connection(
    conn: &zbus::Connection,
) -> zbus::Result<Vec<DeviceInfo>> {
    list_adapter_devices(conn, &crate::get_adapter_path()).await
}

/// Lists the devices that belong to `adapter_path` (e.g. `/org/bluez/hci1`).
pub async fn list_adapter_devices(
    conn: &zbus::Connection,
    adapter_path: &str,
) -> zbus::Result<Vec<DeviceInfo>> {
    Ok(list_adapter_devices_with_paths(conn, adapter_path)
        .await?
        .into_iter()
        .map(|(_, info)| info)
        .collect())
}

/// Like [`list_adapter_devices`] but also returns each device's object path,
/// which is what [`crate::DeviceProxy`] needs. Sorted by path.
pub async fn list_adapter_devices_with_paths(
    conn: &zbus::Connection,
    adapter_path: &str,
) -> zbus::Result<Vec<(String, DeviceInfo)>> {
    let objects = ObjectManagerProxy::new(conn)
        .await?
        .get_managed_objects()
        .await?;
    let adapter_path = adapter_path.trim_end_matches('/');

    let mut devices: Vec<(String, DeviceInfo)> = objects
        .into_iter()
        .filter_map(|(path, interfaces)| {
            let props = interfaces.get(DEVICE_INTERFACE)?;
            // `Adapter` is authoritative; fall back to the path layout.
            let belongs = match props.get("Adapter") {
                Some(adapter) => adapter
                    .downcast_ref::<zbus::zvariant::ObjectPath>()
                    .map(|p| p.as_str() == adapter_path)
                    .unwrap_or(false),
                None => path.as_str().starts_with(&format!("{adapter_path}/dev_")),
            };
            if !belongs {
                return None;
            }
            DeviceInfo::from_properties(props).map(|info| (path.to_string(), info))
        })
        .collect();
    devices.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ov<'a, T: Into<Value<'a>>>(v: T) -> OwnedValue {
        v.into().try_to_owned().unwrap()
    }

    #[test]
    fn parses_device_properties() {
        let props = HashMap::from([
            ("Address".to_string(), ov("AA:BB:CC:DD:EE:FF")),
            ("Connected".to_string(), ov(true)),
            ("RSSI".to_string(), ov(-42i16)),
            ("Name".to_string(), ov("Speaker")),
            ("Trusted".to_string(), ov(false)),
            ("AddressType".to_string(), ov("public")),
        ]);
        let info = DeviceInfo::from_properties(&props).unwrap();
        assert_eq!(info.address, "AA:BB:CC:DD:EE:FF");
        assert_eq!(
            info.alias, "AA:BB:CC:DD:EE:FF",
            "alias falls back to address"
        );
        assert!(info.connected);
        assert!(!info.paired);
        assert_eq!(info.rssi, Some(-42));
        assert_eq!(info.name.as_deref(), Some("Speaker"));
        assert_eq!(info.trusted, Some(false));
        assert_eq!(info.address_type.as_deref(), Some("public"));
    }

    #[test]
    fn missing_address_is_rejected() {
        let props = HashMap::from([("Alias".to_string(), ov("x"))]);
        assert!(DeviceInfo::from_properties(&props).is_none());
    }

    #[test]
    fn apply_reports_only_real_changes() {
        let mut info = DeviceInfo {
            address: "A".into(),
            ..Default::default()
        };
        assert!(info.apply_property("Connected", &Value::from(true)));
        assert!(!info.apply_property("Connected", &Value::from(true)));
        assert!(!info.apply_property("Connected", &Value::from("wrong type")));
        assert!(!info.apply_property("UnknownProp", &Value::from(1u8)));
        assert!(info.apply_property("RSSI", &Value::from(-60i16)));
        assert!(info.invalidate_property("RSSI"));
        assert!(!info.invalidate_property("RSSI"));
        assert_eq!(info.rssi, None);
    }
}
