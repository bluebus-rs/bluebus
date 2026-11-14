use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// Proxy for a remote device (`org.bluez.Device1`,
/// e.g. `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`).
///
/// Use [`crate::device_path`] to build the path from a MAC address.
/// Properties that BlueZ only sets when known (`Name`, `RSSI`, `TxPower`,
/// `Appearance`, `Icon`, …) return an error when absent.
#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.Device1")]
pub trait Device {
    /// Connects all auto-connectable profiles (or the GATT link for LE).
    fn connect(&self) -> zbus::Result<()>;

    /// Disconnects all profiles and the link.
    fn disconnect(&self) -> zbus::Result<()>;

    /// Connects a single profile by UUID.
    fn connect_profile(&self, uuid: &str) -> zbus::Result<()>;

    /// Disconnects a single profile by UUID.
    fn disconnect_profile(&self, uuid: &str) -> zbus::Result<()>;

    /// Starts pairing. An agent must be registered unless the pairing
    /// method is "Just Works".
    fn pair(&self) -> zbus::Result<()>;

    /// Cancels an ongoing [`pair`](Self::pair).
    fn cancel_pairing(&self) -> zbus::Result<()>;

    /// Whether GATT service discovery has completed.
    #[zbus(property)]
    fn services_resolved(&self) -> zbus::Result<bool>;

    /// Device MAC address.
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;

    /// Address type: `"public"` or `"random"`.
    #[zbus(property)]
    fn address_type(&self) -> zbus::Result<String>;

    /// Remote name (absent until known).
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Friendly name; defaults to the remote name or the address.
    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;

    /// Sets the friendly name. An empty string restores the default.
    #[zbus(property)]
    fn set_alias(&self, alias: &str) -> zbus::Result<()>;

    /// Object path of the adapter this device belongs to.
    #[zbus(property)]
    fn adapter(&self) -> zbus::Result<OwnedObjectPath>;

    /// Class of device (BR/EDR).
    #[zbus(property)]
    fn class(&self) -> zbus::Result<u32>;

    /// GAP appearance (LE).
    #[zbus(property)]
    fn appearance(&self) -> zbus::Result<u16>;

    /// Suggested icon name (freedesktop icon naming spec).
    #[zbus(property)]
    fn icon(&self) -> zbus::Result<String>;

    /// Whether the device is connected.
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;

    /// Whether the device is paired.
    #[zbus(property)]
    fn paired(&self) -> zbus::Result<bool>;

    /// Whether the pairing keys are stored (bonded) *(BlueZ ≥ 5.67)*.
    #[zbus(property)]
    fn bonded(&self) -> zbus::Result<bool>;

    /// Whether the device is trusted (connections authorized without agent).
    #[zbus(property)]
    fn trusted(&self) -> zbus::Result<bool>;

    /// Marks the device as trusted or untrusted.
    #[zbus(property)]
    fn set_trusted(&self, trusted: bool) -> zbus::Result<()>;

    /// Whether connections from the device are rejected.
    #[zbus(property)]
    fn blocked(&self) -> zbus::Result<bool>;

    /// Blocks or unblocks the device.
    #[zbus(property)]
    fn set_blocked(&self, blocked: bool) -> zbus::Result<()>;

    /// Whether the device may wake the host from suspend.
    #[zbus(property)]
    fn wake_allowed(&self) -> zbus::Result<bool>;

    /// Allows or forbids waking the host.
    #[zbus(property)]
    fn set_wake_allowed(&self, allowed: bool) -> zbus::Result<()>;

    /// Whether the device only supports legacy (pre-2.1) pairing.
    #[zbus(property)]
    fn legacy_pairing(&self) -> zbus::Result<bool>;

    /// Service UUIDs offered by the device.
    #[zbus(property, name = "UUIDs")]
    fn uuids(&self) -> zbus::Result<Vec<String>>;

    /// Remote Device ID information in modalias format.
    #[zbus(property)]
    fn modalias(&self) -> zbus::Result<String>;

    /// Signal strength of the last inquiry/advertisement, in dBm.
    #[zbus(property, name = "RSSI")]
    fn rssi(&self) -> zbus::Result<i16>;

    /// Advertised transmit power, in dBm.
    #[zbus(property)]
    fn tx_power(&self) -> zbus::Result<i16>;

    /// Manufacturer specific advertising data: company ID → bytes (`ay` in a variant).
    #[zbus(property)]
    fn manufacturer_data(&self) -> zbus::Result<HashMap<u16, OwnedValue>>;

    /// Service advertising data: UUID → bytes (`ay` in a variant).
    #[zbus(property)]
    fn service_data(&self) -> zbus::Result<HashMap<String, OwnedValue>>;

    /// Advertising data flags.
    #[zbus(property)]
    fn advertising_flags(&self) -> zbus::Result<Vec<u8>>;

    /// Raw advertising data: AD type → bytes.
    #[zbus(property)]
    fn advertising_data(&self) -> zbus::Result<HashMap<u8, OwnedValue>>;
}

/// Proxy for the battery level of a connected device (`org.bluez.Battery1`,
/// at the device path). Present only while the device reports its battery.
#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.Battery1")]
pub trait Battery {
    /// Battery level in percent (0–100).
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<u8>;

    /// Where the level comes from (e.g. `"HFP"`, `"GATT"`).
    #[zbus(property)]
    fn source(&self) -> zbus::Result<String>;
}

/// Creates a [`DeviceProxy`] for `address` on the configured default adapter.
pub async fn device_proxy(
    conn: &zbus::Connection,
    address: &str,
) -> zbus::Result<DeviceProxy<'static>> {
    DeviceProxy::new(
        conn,
        crate::device_path(&crate::get_adapter_path(), address),
    )
    .await
}

/// Extracts the byte payload from a `ManufacturerData`/`ServiceData`/
/// `AdvertisingData` value (a variant holding `ay`).
pub fn advertising_bytes(value: &OwnedValue) -> Option<Vec<u8>> {
    Vec::<u8>::try_from(value.try_clone().ok()?).ok()
}
