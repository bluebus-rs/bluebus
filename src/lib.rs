//! # Bluetooth D-Bus API for Rust
//!
//! This library provides an interface to interact with Bluetooth devices
//! using the BlueZ D-Bus API. It enables communication with Bluetooth adapters,
//! devices, GATT profiles, and Low Energy advertisements.
//!
//! ## Features
//! - **Device Discovery**: Scan and list nearby Bluetooth devices.
//! - **Device Management**: Connect, pair, unpair, and trust devices.
//! - **GATT Support**: Read/write characteristics and descriptors.
//! - **LE Advertisement**: Control Bluetooth Low Energy advertisements.
//!
//! ## Example
//! Power the default adapter on, scan for three seconds and list devices:
//!
//! ```no_run
//! #[tokio::main]
//! async fn main() -> zbus::Result<()> {
//!     let conn = bluebus::get_system_connection().await?;
//!     let adapter = bluebus::adapter_proxy(&conn).await?;
//!
//!     if !adapter.powered().await? {
//!         adapter.set_powered(true).await?;
//!     }
//!
//!     adapter.start_discovery().await?;
//!     tokio::time::sleep(std::time::Duration::from_secs(3)).await;
//!     adapter.stop_discovery().await?;
//!
//!     for device in bluebus::list_devices_with_connection(&conn).await? {
//!         println!("{} {} rssi={:?}", device.address, device.alias, device.rssi);
//!     }
//!     Ok(())
//! }
//! ```
//!
//! See [`Monitor`] for live add/remove/change events, [`register_agent`] for
//! pairing, [`register_gatt_application`] for a GATT server and
//! [`register_advertisement`] for LE advertising.
//!
//! ## Modules
//!
//! - [`ble`] – **high-level, easy API** (sessions, scanning, GATT client and
//!   server by UUID); start here.
//! - [`adapter`] – adapters: power, discovery, filters.
//! - [`device`] – remote devices: connect, pair, trust, battery.
//! - [`agent`] / [`agent_manager`] – pairing agents.
//! - [`gatt`] – GATT client proxies and GATT server objects.
//! - [`leadvertisement`] – LE advertisements.
//! - [`monitor`] – device events.
//! - [`cache`] – device snapshots ([`DeviceInfo`]).
//! - [`object_manager`] – BlueZ object tree.
//! - [`paths`] – well-known names and configurable paths.
//!
//! All items except those of [`ble`] are re-exported at the crate root.

/// Manages Bluetooth adapter interactions.
pub mod adapter;
/// Handles Bluetooth authentication agents.
pub mod agent;
/// Manages agent registrations.
pub mod agent_manager;
pub mod ble;
/// Caches Bluetooth device states.
pub mod cache;
/// Handles Bluetooth connections.
pub mod connection;
/// Controls Bluetooth devices.
pub mod device;
/// Interfaces with GATT characteristics and services.
pub mod gatt;
/// Manages Bluetooth Low Energy advertisements.
pub mod leadvertisement;
/// Listens for Bluetooth device events.
pub mod monitor;
/// Manages D-Bus objects.
pub mod object_manager;
mod object_server_ready;
/// Defines Bluetooth system paths.
pub mod paths;

// Re-export modules for easier access.
pub use adapter::*;
pub use agent::*;
pub use agent_manager::*;
pub use cache::*;
pub use connection::*;
pub use device::*;
pub use gatt::*;
pub use leadvertisement::*;
pub use monitor::*;
pub use object_manager::*;
pub use paths::*;

// Compile every example of the README as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
