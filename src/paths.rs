//! Well-known BlueZ names and the configurable object paths used by the
//! convenience functions of this crate.
//!
//! The three mutable globals ([`ADAPTER_PATH`], [`GATT_APPLICATION_PATH`],
//! [`ADVERTISEMENT_PATH`]) are kept for backward compatibility. Every
//! function that relies on them also has an explicit variant taking the path
//! as an argument (for example [`crate::register_advertisement_at`]), which is
//! the recommended way to work with more than one adapter.

use std::sync::{Arc, Mutex, MutexGuard};

/// D-Bus well-known name of the BlueZ daemon.
pub const BLUEZ_SERVICE: &str = "org.bluez";
/// Object path of the BlueZ root object (hosts `AgentManager1`).
pub const BLUEZ_ROOT_PATH: &str = "/org/bluez";
/// Default adapter object path.
pub const DEFAULT_ADAPTER_PATH: &str = "/org/bluez/hci0";
/// Default object path at which a local GATT application is exported.
pub const DEFAULT_GATT_APPLICATION_PATH: &str = "/org/bluez/gatt_application";
/// Default object path at which a local LE advertisement is exported.
pub const DEFAULT_ADVERTISEMENT_PATH: &str = "/org/bluez/diagnify/adv/0";

lazy_static::lazy_static! {
    /// Adapter used by the path-less helpers. Defaults to [`DEFAULT_ADAPTER_PATH`].
    pub static ref ADAPTER_PATH: Arc<Mutex<String>> =
        Arc::new(Mutex::new(String::from(DEFAULT_ADAPTER_PATH)));
}

lazy_static::lazy_static! {
    /// Root path of the GATT application. Defaults to [`DEFAULT_GATT_APPLICATION_PATH`].
    pub static ref GATT_APPLICATION_PATH: Arc<Mutex<String>> =
        Arc::new(Mutex::new(String::from(DEFAULT_GATT_APPLICATION_PATH)));
}

lazy_static::lazy_static! {
    /// Path of the LE advertisement object. Defaults to [`DEFAULT_ADVERTISEMENT_PATH`].
    pub static ref ADVERTISEMENT_PATH: Arc<Mutex<String>> =
        Arc::new(Mutex::new(String::from(DEFAULT_ADVERTISEMENT_PATH)));
}

/// Locks a path mutex, recovering the value if a previous holder panicked.
/// A `String` cannot be left in an inconsistent state, so poisoning is harmless.
fn lock(m: &Mutex<String>) -> MutexGuard<'_, String> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the adapter path used by the path-less helpers.
pub fn get_adapter_path() -> String {
    lock(&ADAPTER_PATH).clone()
}

/// Sets the adapter path used by the path-less helpers (e.g. `/org/bluez/hci1`).
pub fn set_adapter_path(new_value: &str) {
    *lock(&ADAPTER_PATH) = new_value.to_string();
}

/// Returns the GATT application root path.
pub fn get_gatt_application_path() -> String {
    lock(&GATT_APPLICATION_PATH).clone()
}

/// Sets the GATT application root path.
pub fn set_gatt_application_path(new_value: &str) {
    *lock(&GATT_APPLICATION_PATH) = new_value.to_string();
}

/// Returns the LE advertisement object path.
pub fn get_advertisement_path() -> String {
    lock(&ADVERTISEMENT_PATH).clone()
}

/// Sets the LE advertisement object path.
pub fn set_advertisement_path(new_value: &str) {
    *lock(&ADVERTISEMENT_PATH) = new_value.to_string();
}

/// Builds the BlueZ object path of a device from its adapter path and MAC
/// address: `("/org/bluez/hci0", "AA:BB:CC:DD:EE:FF")` →
/// `"/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF"`.
///
/// The address is upper-cased, as BlueZ does.
pub fn device_path(adapter_path: &str, address: &str) -> String {
    format!(
        "{}/dev_{}",
        adapter_path.trim_end_matches('/'),
        address.to_ascii_uppercase().replace(':', "_")
    )
}

/// Extracts the MAC address from a BlueZ device object path, the inverse of
/// [`device_path`]. Works for child objects too (services, characteristics).
/// Returns `None` if the path does not contain a well-formed `dev_` element.
pub fn address_from_device_path(path: &str) -> Option<String> {
    let element = path.split('/').find_map(|e| e.strip_prefix("dev_"))?;
    let parts: Vec<&str> = element.split('_').collect();
    let well_formed = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    well_formed.then(|| parts.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_path_round_trip() {
        let p = device_path("/org/bluez/hci0", "aa:bb:cc:dd:ee:0f");
        assert_eq!(p, "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_0F");
        assert_eq!(
            address_from_device_path(&p).as_deref(),
            Some("AA:BB:CC:DD:EE:0F")
        );
        assert_eq!(
            address_from_device_path(&format!("{p}/service000a/char000b")).as_deref(),
            Some("AA:BB:CC:DD:EE:0F")
        );
        assert_eq!(
            device_path("/org/bluez/hci1/", "00:11:22:33:44:55"),
            "/org/bluez/hci1/dev_00_11_22_33_44_55"
        );
    }

    #[test]
    fn malformed_device_paths() {
        assert_eq!(address_from_device_path("/org/bluez/hci0"), None);
        assert_eq!(address_from_device_path("/org/bluez/hci0/dev_AA_BB"), None);
        assert_eq!(
            address_from_device_path("/org/bluez/hci0/dev_GG_BB_CC_DD_EE_FF"),
            None
        );
    }

    #[test]
    fn poisoned_mutex_is_recovered() {
        let m = Mutex::new(String::from("x"));
        let _ = std::panic::catch_unwind(|| {
            let _g = m.lock().unwrap();
            panic!("poison");
        });
        assert!(m.is_poisoned());
        assert_eq!(*lock(&m), "x");
    }
}
