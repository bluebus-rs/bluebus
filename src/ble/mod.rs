//! High-level, easy BLE API.
//!
//! A small set of cloneable handles over the low-level BlueZ proxies of
//! this crate, in the spirit of Python's *bleak*:
//!
//! * [`Session`]: the connection to BlueZ; lists and opens adapters, sets
//!   the pairing agent.
//! * [`Adapter`]: power, [`scan`](Adapter::scan) /
//!   [`scan_stream`](Adapter::scan_stream) with a [`ScanFilter`],
//!   [`find_device`](Adapter::find_device), known devices.
//! * [`Device`]: connect (waits for service discovery), pair, trust, GATT
//!   [`read`](Device::read) / [`write`](Device::write) /
//!   [`notifications`](Device::notifications) by UUID.
//! * [`Peripheral`]: a local GATT server and advertisement, built with
//!   [`Adapter::peripheral`].
//!
//! Every handle is `Clone + Send + Sync + 'static`, results are plain data
//! ([`DeviceInfo`], [`ServiceInfo`], …) and every error is an [`Error`].
//! UUIDs may be given in short (`"2a19"`, `"0x2A19"`) or full form
//! ([`normalize_uuid`]). The types live in this module only; the crate root
//! keeps the low-level API (where `Adapter` and `Device` are zbus proxies).
//!
//! # Central
//!
//! ```no_run
//! use std::time::Duration;
//! use bluebus::ble::{ScanFilter, Session, WriteType};
//! use futures::StreamExt;
//!
//! # async fn run() -> bluebus::ble::Result<()> {
//! let session = Session::new().await?;
//! let adapter = session.default_adapter().await?;
//! adapter.power_on().await?;
//!
//! let device = adapter
//!     .find_device(ScanFilter::new().name_prefix("Sensor"), Duration::from_secs(10))
//!     .await?;
//! device.connect(Duration::from_secs(20)).await?;
//! println!("battery: {}%", device.battery_level().await?);
//!
//! device.write("2a06", vec![1], WriteType::WithoutResponse).await?;
//! let mut values = device.notifications("2a37").await?;
//! while let Some(v) = values.next().await {
//!     println!("heart rate: {}", v[1]);
//! }
//! # Ok(()) }
//! ```
//!
//! # Peripheral
//!
//! ```no_run
//! use bluebus::ble::{Advertisement, Characteristic, Service, Session};
//!
//! # async fn run() -> bluebus::ble::Result<()> {
//! let adapter = Session::new().await?.default_adapter().await?;
//! let peripheral = adapter
//!     .peripheral()
//!     .service(Service::new("180f").characteristic(
//!         Characteristic::new("2a19").read().notify().value(vec![100]),
//!     ))
//!     .advertise(Advertisement::new().local_name("bluebus").service_uuid("180f"))
//!     .start()
//!     .await?;
//! peripheral.notify("2a19", vec![99]).await?;
//! peripheral.stop().await?;
//! # Ok(()) }
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use zbus::names::InterfaceName;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

mod adapter;
mod agent;
mod device;
mod error;
mod info;
mod peripheral;
mod scan;
mod session;
mod uuid;

pub use self::adapter::Adapter;
pub use self::agent::{AutoAcceptAgent, PasskeyAgent};
pub use self::device::{Device, NotificationStream, WriteType};
pub use self::error::{Error, Result};
pub use self::info::{AdapterInfo, CharacteristicInfo, DescriptorInfo, DeviceInfo, ServiceInfo};
pub use self::peripheral::{Advertisement, Characteristic, Peripheral, PeripheralBuilder, Service};
pub use self::scan::{ScanEvent, ScanFilter, ScanStream};
pub use self::session::Session;
pub use self::uuid::{normalize_uuid, uuid_from_u16, uuid_from_u32, BASE_UUID_SUFFIX};
pub use crate::{AgentCapability, AgentError, AgentHandler, AgentResult, Transport};

/// A process-wide unique object path for objects exported by this module.
pub(crate) fn unique_path(kind: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "/bluebus/ble/p{}/{kind}{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// `GetAll` on a BlueZ object, without caching.
pub(crate) async fn get_all(
    conn: &zbus::Connection,
    path: &str,
    interface: &'static str,
) -> Result<HashMap<String, OwnedValue>> {
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(crate::BLUEZ_SERVICE)?
        .path(path.to_owned())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    Ok(props
        .get_all(InterfaceName::from_static_str_unchecked(interface))
        .await?)
}

/// Subscribes to `PropertiesChanged` of `interface` on the BlueZ object `path`.
pub(crate) async fn properties_changed(
    conn: &zbus::Connection,
    path: &str,
    interface: &'static str,
) -> Result<zbus::MessageStream> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(crate::BLUEZ_SERVICE)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(path.to_owned())?
        .arg(0, interface)?
        .build();
    Ok(zbus::MessageStream::for_match_rule(rule, conn, Some(64)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_handle<T: Clone + Send + Sync + 'static>() {}
    fn assert_data<T: Send + Sync + 'static>() {}

    #[test]
    fn handles_are_shareable() {
        assert_handle::<Session>();
        assert_handle::<Adapter>();
        assert_handle::<Device>();
        assert_handle::<Peripheral>();
        assert_handle::<PeripheralBuilder>();
        assert_handle::<Characteristic>();
        assert_handle::<Service>();
        assert_handle::<Advertisement>();
        assert_handle::<ScanFilter>();
        assert_handle::<DeviceInfo>();
        assert_handle::<ServiceInfo>();
        assert_data::<Error>();
        fn assert_send<T: Send + 'static>() {}
        assert_send::<ScanStream>();
        assert_send::<NotificationStream>();
    }

    #[test]
    fn unique_paths_differ() {
        let a = unique_path("app");
        let b = unique_path("app");
        assert_ne!(a, b);
        assert!(zbus::zvariant::ObjectPath::try_from(a.as_str()).is_ok());
    }
}
