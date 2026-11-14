//! Low Energy advertising (`org.bluez.LEAdvertisement1`).
//!
//! ```no_run
//! # async fn f() -> zbus::Result<()> {
//! use bluebus::*;
//! let conn = get_system_connection().await?;
//! let adv = LEAdvertisementProperties::new(AdvertisementType::Peripheral)
//!     .with_local_name("bluebus")
//!     .with_service_uuids(["180D"])
//!     .with_manufacturer_data(0xFFFF, vec![1, 2, 3])
//!     .with_discoverable(true);
//! register_advertisement(&conn, adv).await?;
//! // ...
//! unregister_advertisement(&conn).await?;
//! # Ok(()) }
//! ```
//!
//! Unset optional properties are *omitted* from the D-Bus object, as BlueZ
//! requires (an empty `SecondaryChannel`, for example, is rejected).

use std::collections::HashMap;

use zbus::object_server::Interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{fdo, interface};

/// Advertisement type (`Type` property).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvertisementType {
    /// Connectable advertisement.
    Peripheral,
    /// Non-connectable broadcast.
    Broadcast,
}

impl AdvertisementType {
    /// The string BlueZ expects.
    pub fn as_str(self) -> &'static str {
        match self {
            AdvertisementType::Peripheral => "peripheral",
            AdvertisementType::Broadcast => "broadcast",
        }
    }
}

fn bytes_value(bytes: Vec<u8>) -> OwnedValue {
    // `ay` inside a variant is what BlueZ expects for every data map.
    Value::from(bytes)
        .try_to_owned()
        .expect("byte arrays never contain file descriptors")
}

fn clone_map<K: Clone + Eq + std::hash::Hash>(
    map: &HashMap<K, OwnedValue>,
) -> fdo::Result<HashMap<K, OwnedValue>> {
    map.iter()
        .map(|(k, v)| {
            Ok((
                k.clone(),
                v.try_clone()
                    .map_err(|e| fdo::Error::Failed(e.to_string()))?,
            ))
        })
        .collect()
}

fn unset(name: &str) -> fdo::Error {
    fdo::Error::UnknownProperty(format!("{name} is not set"))
}

fn opt<T: Clone>(v: &Option<T>, name: &str) -> fdo::Result<T> {
    v.clone().ok_or_else(|| unset(name))
}

fn non_empty<T: Clone>(v: &[T], name: &str) -> fdo::Result<Vec<T>> {
    if v.is_empty() {
        Err(unset(name))
    } else {
        Ok(v.to_vec())
    }
}

fn non_empty_map<K: Clone + Eq + std::hash::Hash>(
    m: &HashMap<K, OwnedValue>,
    name: &str,
) -> fdo::Result<HashMap<K, OwnedValue>> {
    if m.is_empty() {
        Err(unset(name))
    } else {
        clone_map(m)
    }
}

/// An LE advertisement exported to BlueZ.
///
/// [`Default`] keeps the historical sample advertisement ("MyDevice", heart
/// rate + battery UUIDs, …); use [`LEAdvertisementProperties::new`] and the
/// `with_*` builders to describe your own.
pub struct LEAdvertisementProperties {
    adv_type: String,
    local_name: Option<String>,
    service_uuids: Vec<String>,
    solicit_uuids: Vec<String>,
    manufacturer_data: HashMap<u16, OwnedValue>,
    service_data: HashMap<String, OwnedValue>,
    data: HashMap<u8, OwnedValue>,
    discoverable: Option<bool>,
    discoverable_timeout: Option<u16>,
    includes: Vec<String>,
    duration: Option<u16>,
    timeout: Option<u16>,
    min_interval: Option<u32>,
    max_interval: Option<u32>,
    tx_power: Option<i16>,
    appearance: Option<u16>,
    secondary_channel: Option<String>,
}

impl LEAdvertisementProperties {
    /// An advertisement of the given type with no other property set.
    pub fn new(adv_type: AdvertisementType) -> Self {
        Self {
            adv_type: adv_type.as_str().to_owned(),
            local_name: None,
            service_uuids: Vec::new(),
            solicit_uuids: Vec::new(),
            manufacturer_data: HashMap::new(),
            service_data: HashMap::new(),
            data: HashMap::new(),
            discoverable: None,
            discoverable_timeout: None,
            includes: Vec::new(),
            duration: None,
            timeout: None,
            min_interval: None,
            max_interval: None,
            tx_power: None,
            appearance: None,
            secondary_channel: None,
        }
    }

    /// Local name to advertise (`LocalName`).
    pub fn with_local_name(mut self, name: impl Into<String>) -> Self {
        self.local_name = Some(name.into());
        self
    }
    /// Advertised service UUIDs (16-bit like `"180D"` or 128-bit).
    pub fn with_service_uuids<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        uuids: I,
    ) -> Self {
        self.service_uuids = uuids.into_iter().map(Into::into).collect();
        self
    }
    /// Solicited service UUIDs.
    pub fn with_solicit_uuids<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        uuids: I,
    ) -> Self {
        self.solicit_uuids = uuids.into_iter().map(Into::into).collect();
        self
    }
    /// Adds manufacturer specific data for a company identifier.
    pub fn with_manufacturer_data(mut self, company_id: u16, data: Vec<u8>) -> Self {
        self.manufacturer_data.insert(company_id, bytes_value(data));
        self
    }
    /// Adds service data for a service UUID.
    pub fn with_service_data(mut self, uuid: impl Into<String>, data: Vec<u8>) -> Self {
        self.service_data.insert(uuid.into(), bytes_value(data));
        self
    }
    /// Adds a raw AD structure (`Data`, experimental in BlueZ).
    pub fn with_data(mut self, ad_type: u8, data: Vec<u8>) -> Self {
        self.data.insert(ad_type, bytes_value(data));
        self
    }
    /// Sets the general discoverable flag (peripheral only).
    pub fn with_discoverable(mut self, discoverable: bool) -> Self {
        self.discoverable = Some(discoverable);
        self
    }
    /// Discoverable timeout in seconds.
    pub fn with_discoverable_timeout(mut self, seconds: u16) -> Self {
        self.discoverable_timeout = Some(seconds);
        self
    }
    /// Data BlueZ appends itself: `"tx-power"`, `"appearance"`, `"local-name"`.
    pub fn with_includes<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        includes: I,
    ) -> Self {
        self.includes = includes.into_iter().map(Into::into).collect();
        self
    }
    /// Rotation duration in seconds when several advertisements are active.
    pub fn with_duration(mut self, seconds: u16) -> Self {
        self.duration = Some(seconds);
        self
    }
    /// Seconds after which BlueZ removes the advertisement and calls `Release`.
    pub fn with_timeout(mut self, seconds: u16) -> Self {
        self.timeout = Some(seconds);
        self
    }
    /// Advertising interval range in milliseconds (experimental in BlueZ).
    pub fn with_interval(mut self, min_ms: u32, max_ms: u32) -> Self {
        self.min_interval = Some(min_ms);
        self.max_interval = Some(max_ms);
        self
    }
    /// Requested TX power in dBm (experimental in BlueZ).
    pub fn with_tx_power(mut self, dbm: i16) -> Self {
        self.tx_power = Some(dbm);
        self
    }
    /// GAP appearance value.
    pub fn with_appearance(mut self, appearance: u16) -> Self {
        self.appearance = Some(appearance);
        self
    }
    /// Secondary channel for extended advertising: `"1M"`, `"2M"` or `"Coded"`.
    pub fn with_secondary_channel(mut self, channel: impl Into<String>) -> Self {
        self.secondary_channel = Some(channel.into());
        self
    }
}

#[interface(name = "org.bluez.LEAdvertisement1")]
impl LEAdvertisementProperties {
    #[zbus(property, name = "Type")]
    fn type_(&self) -> fdo::Result<&str> {
        Ok(&self.adv_type)
    }

    #[zbus(property, name = "LocalName")]
    fn local_name(&self) -> fdo::Result<String> {
        opt(&self.local_name, "LocalName")
    }

    #[zbus(property, name = "ServiceUUIDs")]
    fn service_uuids(&self) -> fdo::Result<Vec<String>> {
        non_empty(&self.service_uuids, "ServiceUUIDs")
    }

    #[zbus(property, name = "SolicitUUIDs")]
    fn solicit_uuids(&self) -> fdo::Result<Vec<String>> {
        non_empty(&self.solicit_uuids, "SolicitUUIDs")
    }

    #[zbus(property, name = "ManufacturerData")]
    fn manufacturer_data(&self) -> fdo::Result<HashMap<u16, OwnedValue>> {
        non_empty_map(&self.manufacturer_data, "ManufacturerData")
    }

    #[zbus(property, name = "ServiceData")]
    fn service_data(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        non_empty_map(&self.service_data, "ServiceData")
    }

    #[zbus(property, name = "Data")]
    fn data(&self) -> fdo::Result<HashMap<u8, OwnedValue>> {
        non_empty_map(&self.data, "Data")
    }

    #[zbus(property, name = "Discoverable")]
    fn discoverable(&self) -> fdo::Result<bool> {
        opt(&self.discoverable, "Discoverable")
    }

    #[zbus(property, name = "DiscoverableTimeout")]
    fn discoverable_timeout(&self) -> fdo::Result<u16> {
        opt(&self.discoverable_timeout, "DiscoverableTimeout")
    }

    #[zbus(property, name = "Includes")]
    fn includes(&self) -> fdo::Result<Vec<String>> {
        non_empty(&self.includes, "Includes")
    }

    #[zbus(property, name = "Duration")]
    fn duration(&self) -> fdo::Result<u16> {
        opt(&self.duration, "Duration")
    }

    #[zbus(property, name = "Timeout")]
    fn timeout(&self) -> fdo::Result<u16> {
        opt(&self.timeout, "Timeout")
    }

    #[zbus(property, name = "MinInterval")]
    fn min_interval(&self) -> fdo::Result<u32> {
        opt(&self.min_interval, "MinInterval")
    }

    #[zbus(property, name = "MaxInterval")]
    fn max_interval(&self) -> fdo::Result<u32> {
        opt(&self.max_interval, "MaxInterval")
    }

    #[zbus(property, name = "TxPower")]
    fn tx_power(&self) -> fdo::Result<i16> {
        opt(&self.tx_power, "TxPower")
    }

    #[zbus(property, name = "Appearance")]
    fn appearance(&self) -> fdo::Result<u16> {
        opt(&self.appearance, "Appearance")
    }

    #[zbus(property, name = "SecondaryChannel")]
    fn secondary_channel(&self) -> fdo::Result<String> {
        opt(&self.secondary_channel, "SecondaryChannel")
    }

    /// Called by BlueZ when it removes the advertisement (timeout, adapter
    /// power off, …).
    fn release(&self) -> fdo::Result<()> {
        tracing::debug!("advertisement released by BlueZ");
        Ok(())
    }
}

// zbus derives these helper names from the D-Bus member names; the explicit
// `ServiceUUIDs`/`SolicitUUIDs` names changed them, so keep the 0.1.10 names.
impl LEAdvertisementProperties {
    /// Emits `PropertiesChanged` for `ServiceUUIDs`.
    pub async fn service_uuids_changed(
        &self,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()> {
        self.service_u_u_i_ds_changed(emitter).await
    }

    /// Emits `PropertiesChanged` for `SolicitUUIDs`.
    pub async fn solicit_uuids_changed(
        &self,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()> {
        self.solicit_u_u_i_ds_changed(emitter).await
    }
}

impl Default for LEAdvertisementProperties {
    /// The historical sample advertisement, unchanged since 0.1.0.
    fn default() -> Self {
        Self::new(AdvertisementType::Peripheral)
            .with_local_name("MyDevice")
            .with_service_uuids(["180D", "180F"])
            .with_discoverable(true)
            .with_discoverable_timeout(30)
            .with_includes(["tx-power"])
            .with_duration(60)
            .with_timeout(120)
            .with_interval(100, 200)
            .with_tx_power(-20)
            .with_appearance(0x0040)
            .with_secondary_channel("1M")
    }
}

/// Proxy for `org.bluez.LEAdvertisingManager1` (on the adapter object).
#[zbus::proxy(
    default_service = "org.bluez",
    interface = "org.bluez.LEAdvertisingManager1"
)]
pub trait LEAdvertisingManager1 {
    /// Registers the advertisement exported at `advertisement`.
    fn register_advertisement(
        &self,
        advertisement: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    /// Unregisters a previously registered advertisement.
    fn unregister_advertisement(&self, advertisement: &ObjectPath<'_>) -> zbus::Result<()>;

    /// Number of advertisements currently registered.
    #[zbus(property)]
    fn active_instances(&self) -> zbus::Result<u8>;

    /// Number of additional advertisements that can be registered.
    #[zbus(property)]
    fn supported_instances(&self) -> zbus::Result<u8>;

    /// Supported `Includes` values.
    #[zbus(property)]
    fn supported_includes(&self) -> zbus::Result<Vec<String>>;

    /// Supported `SecondaryChannel` values.
    #[zbus(property)]
    fn supported_secondary_channels(&self) -> zbus::Result<Vec<String>>;

    /// Supported advertising features (e.g. `"CanSetTxPower"`).
    #[zbus(property)]
    fn supported_features(&self) -> zbus::Result<Vec<String>>;
}

/// Exports `advertisement` at [`crate::get_advertisement_path`] and registers
/// it with the adapter at [`crate::get_adapter_path`].
pub async fn register_advertisement(
    conn: &zbus::Connection,
    advertisement: impl Interface + 'static,
) -> zbus::Result<()> {
    register_advertisement_at(
        conn,
        &crate::get_adapter_path(),
        &crate::get_advertisement_path(),
        advertisement,
    )
    .await
}

/// Exports `advertisement` at `adv_path` and registers it with `adapter_path`.
///
/// An advertisement already exported at `adv_path` is unregistered and
/// replaced. If BlueZ rejects the advertisement the object is removed again,
/// so the call can simply be retried.
pub async fn register_advertisement_at<A: Interface + 'static>(
    conn: &zbus::Connection,
    adapter_path: &str,
    adv_path: &str,
    advertisement: A,
) -> zbus::Result<()> {
    let path = ObjectPath::try_from(adv_path)?;
    let manager = LEAdvertisingManager1Proxy::new(conn, adapter_path).await?;
    let server = conn.object_server();

    // Replace a stale advertisement left by a previous registration.
    // `remove` matches by interface name, so any LEAdvertisement1 is found.
    if server.remove::<A, _>(&path).await.unwrap_or(false) {
        let _ = manager.unregister_advertisement(&path).await;
    }
    if !server.at(&path, advertisement).await? {
        return Err(zbus::Error::Failure(format!(
            "could not export the advertisement at {adv_path}"
        )));
    }
    if let Err(e) = crate::object_server_ready::ensure_object_server_ready(conn).await {
        let _ = server.remove::<A, _>(&path).await;
        return Err(e);
    }

    if let Err(e) = manager.register_advertisement(&path, HashMap::new()).await {
        let _ = server.remove::<A, _>(&path).await;
        return Err(e);
    }
    Ok(())
}

/// Unregisters the advertisement at [`crate::get_advertisement_path`] from
/// the adapter at [`crate::get_adapter_path`] and removes the exported object.
pub async fn unregister_advertisement(conn: &zbus::Connection) -> zbus::Result<()> {
    unregister_advertisement_at(
        conn,
        &crate::get_adapter_path(),
        &crate::get_advertisement_path(),
    )
    .await
}

/// Unregisters the advertisement at `adv_path` from `adapter_path` and removes
/// the exported object (also when BlueZ had already released it).
pub async fn unregister_advertisement_at(
    conn: &zbus::Connection,
    adapter_path: &str,
    adv_path: &str,
) -> zbus::Result<()> {
    let path = ObjectPath::try_from(adv_path)?;
    let result = LEAdvertisingManager1Proxy::new(conn, adapter_path)
        .await?
        .unregister_advertisement(&path)
        .await;
    remove_advertisement_object(conn, &path).await;
    result
}

/// Removes whatever `org.bluez.LEAdvertisement1` object is exported at `path`.
async fn remove_advertisement_object(conn: &zbus::Connection, path: &ObjectPath<'_>) {
    // `ObjectServer::remove` matches by interface *name*, so the concrete
    // type only has to share the interface name with the exported one.
    let _ = conn
        .object_server()
        .remove::<LEAdvertisementProperties, _>(path)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_properties_error() {
        let adv = LEAdvertisementProperties::new(AdvertisementType::Broadcast);
        assert_eq!(adv.type_().unwrap(), "broadcast");
        assert!(adv.local_name().is_err());
        assert!(adv.secondary_channel().is_err());
        assert!(adv.service_uuids().is_err());
        assert!(adv.manufacturer_data().is_err());
    }

    #[test]
    fn builder_sets_properties() {
        let adv = LEAdvertisementProperties::new(AdvertisementType::Peripheral)
            .with_local_name("x")
            .with_manufacturer_data(0x004C, vec![1, 2])
            .with_interval(20, 40);
        assert_eq!(adv.local_name().unwrap(), "x");
        let md = adv.manufacturer_data().unwrap();
        assert_eq!(
            Vec::<u8>::try_from(md[&0x004C].try_clone().unwrap()).unwrap(),
            vec![1, 2]
        );
        assert_eq!(adv.min_interval().unwrap(), 20);
        assert_eq!(adv.max_interval().unwrap(), 40);
    }

    #[test]
    fn default_is_unchanged() {
        let adv = LEAdvertisementProperties::default();
        assert_eq!(adv.local_name().unwrap(), "MyDevice");
        assert_eq!(adv.service_uuids().unwrap(), vec!["180D", "180F"]);
        assert_eq!(adv.secondary_channel().unwrap(), "1M");
        assert_eq!(adv.tx_power().unwrap(), -20);
    }
}
