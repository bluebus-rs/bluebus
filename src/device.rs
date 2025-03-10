/// Defines the `Device` trait for interfacing with Bluetooth devices via D-Bus.
/// Provides methods for connecting, pairing, and managing device properties.
#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.Device1")]
pub trait Device {
    /// Initiates a connection to the Bluetooth device.
    fn connect(&self) -> zbus::Result<()>;

    /// Disconnects the Bluetooth device.
    fn disconnect(&self) -> zbus::Result<()>;

    /// Pairs the Bluetooth device with the adapter.
    fn pair(&self) -> zbus::Result<()>;

    /// Cancels an ongoing pairing process.
    fn cancel_pairing(&self) -> zbus::Result<()>;

    /// Checks if the device's services have been resolved.
    #[zbus(property)]
    fn services_resolved(&self) -> zbus::Result<bool>;

    /// Retrieves the Bluetooth device's unique address (MAC address).
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;

    /// Retrieves the name of the Bluetooth device.
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Checks if the device is currently connected.
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;

    /// Checks if the device is paired.
    #[zbus(property)]
    fn paired(&self) -> zbus::Result<bool>;

    /// Checks if the device is marked as trusted.
    #[zbus(property)]
    fn trusted(&self) -> zbus::Result<bool>;

    /// Sets the device as trusted or untrusted.
    ///
    /// # Arguments
    /// * `trusted` - `true` to mark as trusted, `false` to unmark.
    #[zbus(property)]
    fn set_trusted(&self, trusted: bool) -> zbus::Result<()>;

    /// Checks if the device is blocked.
    #[zbus(property)]
    fn blocked(&self) -> zbus::Result<bool>;

    /// Blocks or unblocks the device.
    ///
    /// # Arguments
    /// * `blocked` - `true` to block, `false` to unblock.
    #[zbus(property)]
    fn set_blocked(&self, blocked: bool) -> zbus::Result<()>;
}
