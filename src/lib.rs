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
//! ## Example Usage
//! Here’s how you can scan for Bluetooth devices and list them:
//!
//! ```no_run
//! 
//! #[tokio::main]
//! async fn main() -> zbus::Result<()> {
//!     let conn = bluebus::get_system_connection().await.unwrap();
//!     let adapter = bluebus::AdapterProxy::builder(&conn)
//!         .path(bluebus::ADAPTER_PATH)?.build().await?;
//!      
//!     if !adapter.powered().await? {
//!         adapter.set_powered(true).await?;
//!     }
//!
//!     adapter.start_discovery().await?;
//!     tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
//!     adapter.stop_discovery().await?;
//!
//!     let monitor = Arc::new(bluebus::Monitor::new(conn.clone(), manager).await);
//!     let monitor_clone = Arc::clone(&monitor);
//!
//!     bluebus::print_avaiable_devices(&objects);
//!
//!     tokio::spawn(async move {
//!         monitor.monitor_device_added().await;
//!     });
//!
//!     tokio::spawn(async move {
//!         monitor_clone.monitor_device_removed().await;
//!     });
//!     Ok(())
//! }
//! ```
//!
//! ## Modules Overview
//! 
//! - [`adapter`] - Manages Bluetooth adapters (enables/disables, scans devices).
//! - [`agent`] - Handles pairing and authentication requests.
//! - [`device`] - Manages Bluetooth device connections.
//! - [`gatt_service`] - Interfaces with Bluetooth GATT services.
//! - [`monitor`] - Monitors Bluetooth events like device additions/removals.
//! - [`object_manager`] - Handles D-Bus object management.
//!
//! ## Installation
//! Add the following to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! bluetooth_lib = "0.1"
//! ```
//!

/// Manages Bluetooth adapter interactions.
pub mod adapter;
/// Handles Bluetooth authentication agents.
pub mod agent;
/// Manages agent registrations.
pub mod agent_manager;
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
