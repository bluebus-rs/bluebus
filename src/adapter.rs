use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, Value};

/// Proxy for a BlueZ adapter (`org.bluez.Adapter1`, e.g. `/org/bluez/hci0`).
///
/// The adapter path has no default; use [`adapter_proxy`] for the configured
/// default adapter, or `AdapterProxy::new(&conn, "/org/bluez/hci1")`.
///
/// Properties marked *(BlueZ ≥ x)* return an `UnknownProperty` error on
/// older daemons.
///
/// Property getters read zbus' cache, which BlueZ updates through
/// `PropertiesChanged` *after* a setter returns. To act on the new value,
/// wait for it with the matching `receive_*_changed` stream (see
/// [`set_adapter_powered`]) or build the proxy with
/// `.cache_properties(zbus::proxy::CacheProperties::No)`.
#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.Adapter1")]
pub trait Adapter {
    /// Starts device discovery. Discovered devices appear as `Device1` objects.
    fn start_discovery(&self) -> zbus::Result<()>;

    /// Stops a discovery session started by this client.
    fn stop_discovery(&self) -> zbus::Result<()>;

    /// Removes a device and its pairing information.
    fn remove_device(&self, device: OwnedObjectPath) -> zbus::Result<()>;

    /// Sets the discovery filter for this client's next discovery session.
    ///
    /// Keys: `UUIDs` (`as`), `RSSI` (`n`), `Pathloss` (`q`), `Transport`
    /// (`"auto"`, `"bredr"`, `"le"`), `DuplicateData` (`b`), `Discoverable`
    /// (`b`), `Pattern` (`s`). An empty map resets the filter.
    /// See [`DiscoveryFilter`] for a typed builder.
    fn set_discovery_filter(&self, filter: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    /// Returns the discovery filter keys supported by this adapter.
    fn get_discovery_filters(&self) -> zbus::Result<Vec<String>>;

    /// Connects to a device that has not been discovered (experimental,
    /// requires `bluetoothd --experimental`). Keys: `Address`, `AddressType`.
    fn connect_device(&self, properties: HashMap<&str, Value<'_>>)
        -> zbus::Result<OwnedObjectPath>;

    /// Adapter MAC address.
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;

    /// Address type: `"public"` or `"random"`.
    #[zbus(property)]
    fn address_type(&self) -> zbus::Result<String>;

    /// System name (hostname based) of the adapter.
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Friendly name; writable.
    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;

    /// Sets the friendly name. An empty string resets it to the system name.
    #[zbus(property)]
    fn set_alias(&self, alias: &str) -> zbus::Result<()>;

    /// Class of device.
    #[zbus(property)]
    fn class(&self) -> zbus::Result<u32>;

    /// Whether the adapter radio is on.
    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;

    /// Powers the adapter on or off. Fails with `org.bluez.Error.Blocked`
    /// (or `Failed`) when the radio is rfkill-blocked.
    #[zbus(property)]
    fn set_powered(&self, powered: bool) -> zbus::Result<()>;

    /// Power state: `"on"`, `"off"`, `"off-enabling"`, `"on-disabling"`,
    /// `"off-blocked"` *(BlueZ ≥ 5.73)*.
    #[zbus(property)]
    fn power_state(&self) -> zbus::Result<String>;

    /// Whether the adapter is visible to other devices.
    #[zbus(property)]
    fn discoverable(&self) -> zbus::Result<bool>;

    /// Makes the adapter visible or hidden.
    #[zbus(property)]
    fn set_discoverable(&self, discoverable: bool) -> zbus::Result<()>;

    /// Seconds after which discoverable mode is turned off (0 = never).
    #[zbus(property)]
    fn discoverable_timeout(&self) -> zbus::Result<u32>;

    /// Sets the discoverable timeout in seconds (0 = never).
    #[zbus(property)]
    fn set_discoverable_timeout(&self, seconds: u32) -> zbus::Result<()>;

    /// Whether incoming pairing is accepted.
    #[zbus(property)]
    fn pairable(&self) -> zbus::Result<bool>;

    /// Enables or disables incoming pairing.
    #[zbus(property)]
    fn set_pairable(&self, pairable: bool) -> zbus::Result<()>;

    /// Seconds after which pairable mode is turned off (0 = never).
    #[zbus(property)]
    fn pairable_timeout(&self) -> zbus::Result<u32>;

    /// Sets the pairable timeout in seconds (0 = never).
    #[zbus(property)]
    fn set_pairable_timeout(&self, seconds: u32) -> zbus::Result<()>;

    /// Whether a discovery session is running (from any client).
    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;

    /// 128-bit UUIDs of the local services.
    #[zbus(property, name = "UUIDs")]
    fn uuids(&self) -> zbus::Result<Vec<String>>;

    /// Remote Device ID information in modalias format.
    #[zbus(property)]
    fn modalias(&self) -> zbus::Result<String>;

    /// Supported roles: `"central"`, `"peripheral"`, `"central-peripheral"`.
    #[zbus(property)]
    fn roles(&self) -> zbus::Result<Vec<String>>;

    /// Enabled experimental features (UUIDs) *(BlueZ ≥ 5.59)*.
    #[zbus(property)]
    fn experimental_features(&self) -> zbus::Result<Vec<String>>;

    /// Controller manufacturer ID *(BlueZ ≥ 5.69)*.
    #[zbus(property)]
    fn manufacturer(&self) -> zbus::Result<u16>;

    /// Bluetooth core version of the controller *(BlueZ ≥ 5.69)*.
    #[zbus(property)]
    fn version(&self) -> zbus::Result<u8>;
}

/// Creates an [`AdapterProxy`] for the configured default adapter
/// ([`crate::get_adapter_path`]).
pub async fn adapter_proxy(conn: &zbus::Connection) -> zbus::Result<AdapterProxy<'static>> {
    AdapterProxy::new(conn, crate::get_adapter_path()).await
}

/// Powers the adapter on or off and waits (up to `timeout`) until BlueZ
/// reports the new state, so that `adapter.powered()` is up to date.
pub async fn set_adapter_powered(
    adapter: &AdapterProxy<'_>,
    powered: bool,
    timeout: std::time::Duration,
) -> zbus::Result<()> {
    use futures::StreamExt;
    let mut changes = adapter.receive_powered_changed().await;
    adapter.set_powered(powered).await?;
    let wait = async {
        if adapter.powered().await.ok() == Some(powered) {
            return Ok(());
        }
        while let Some(change) = changes.next().await {
            if change.get().await.ok() == Some(powered) {
                return Ok(());
            }
        }
        Err(zbus::Error::Failure("property stream ended".into()))
    };
    tokio::time::timeout(timeout, wait).await.map_err(|_| {
        zbus::Error::Failure(format!(
            "adapter did not report Powered={powered} within {timeout:?}"
        ))
    })?
}

/// Returns the object paths of all adapters (`/org/bluez/hci0`, …), sorted.
pub async fn list_adapters(conn: &zbus::Connection) -> zbus::Result<Vec<String>> {
    let objects = crate::ObjectManagerProxy::new(conn)
        .await?
        .get_managed_objects()
        .await?;
    let mut adapters: Vec<String> = objects
        .into_iter()
        .filter(|(_, ifaces)| ifaces.contains_key("org.bluez.Adapter1"))
        .map(|(path, _)| path.to_string())
        .collect();
    adapters.sort();
    Ok(adapters)
}

/// Transport selector of a [`DiscoveryFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// BR/EDR and LE (interleaved).
    Auto,
    /// Classic Bluetooth only.
    BrEdr,
    /// Low Energy only.
    Le,
}

impl Transport {
    /// The string BlueZ expects.
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Auto => "auto",
            Transport::BrEdr => "bredr",
            Transport::Le => "le",
        }
    }
}

/// Typed builder for [`AdapterProxy::set_discovery_filter`].
///
/// ```no_run
/// # async fn f(adapter: &bluebus::AdapterProxy<'_>) -> zbus::Result<()> {
/// let filter = bluebus::DiscoveryFilter::new()
///     .transport(bluebus::Transport::Le)
///     .rssi(-80)
///     .uuids(["0000180d-0000-1000-8000-00805f9b34fb"]);
/// adapter.set_discovery_filter(filter.to_dict()).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoveryFilter {
    pub uuids: Option<Vec<String>>,
    pub rssi: Option<i16>,
    pub pathloss: Option<u16>,
    pub transport: Option<Transport>,
    pub duplicate_data: Option<bool>,
    pub discoverable: Option<bool>,
    pub pattern: Option<String>,
}

impl DiscoveryFilter {
    /// An empty filter (resets any previous filter).
    pub fn new() -> Self {
        Self::default()
    }
    /// Only report devices advertising one of these service UUIDs.
    pub fn uuids<I: IntoIterator<Item = S>, S: Into<String>>(mut self, uuids: I) -> Self {
        self.uuids = Some(uuids.into_iter().map(Into::into).collect());
        self
    }
    /// Minimum RSSI in dBm. Mutually exclusive with `pathloss` in BlueZ.
    pub fn rssi(mut self, rssi: i16) -> Self {
        self.rssi = Some(rssi);
        self
    }
    /// Maximum path loss in dB.
    pub fn pathloss(mut self, pathloss: u16) -> Self {
        self.pathloss = Some(pathloss);
        self
    }
    /// Restricts discovery to a transport.
    pub fn transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }
    /// Report every advertisement, not only changes.
    pub fn duplicate_data(mut self, enabled: bool) -> Self {
        self.duplicate_data = Some(enabled);
        self
    }
    /// Make the adapter discoverable while discovering.
    pub fn discoverable(mut self, enabled: bool) -> Self {
        self.discoverable = Some(enabled);
        self
    }
    /// Only report devices whose address or name starts with this prefix.
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }

    /// Converts to the dictionary expected by `SetDiscoveryFilter`.
    pub fn to_dict(&self) -> HashMap<&'static str, Value<'static>> {
        let mut d = HashMap::new();
        if let Some(u) = &self.uuids {
            d.insert("UUIDs", Value::from(u.clone()));
        }
        if let Some(r) = self.rssi {
            d.insert("RSSI", Value::from(r));
        }
        if let Some(p) = self.pathloss {
            d.insert("Pathloss", Value::from(p));
        }
        if let Some(t) = self.transport {
            d.insert("Transport", Value::from(t.as_str()));
        }
        if let Some(b) = self.duplicate_data {
            d.insert("DuplicateData", Value::from(b));
        }
        if let Some(b) = self.discoverable {
            d.insert("Discoverable", Value::from(b));
        }
        if let Some(p) = &self.pattern {
            d.insert("Pattern", Value::from(p.clone()));
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_filter_dict() {
        let d = DiscoveryFilter::new()
            .transport(Transport::Le)
            .rssi(-70)
            .uuids(["180d"])
            .to_dict();
        assert_eq!(d.len(), 3);
        assert_eq!(d["Transport"], Value::from("le"));
        assert_eq!(d["RSSI"], Value::from(-70i16));
        assert_eq!(d["UUIDs"].value_signature().to_string(), "as");
        assert!(DiscoveryFilter::new().to_dict().is_empty());
    }
}
