/// Defines the `Adapter` trait for interfacing with Bluetooth adapters via D-Bus.
/// Provides methods to control and retrieve information about the adapter.
#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.Adapter1")]
pub trait Adapter {
    /// Starts Bluetooth device discovery.
    fn start_discovery(&self) -> zbus::Result<()>;

    /// Stops Bluetooth device discovery.
    fn stop_discovery(&self) -> zbus::Result<()>;

    /// Removes a previously paired Bluetooth device.
    ///
    /// # Arguments
    /// * `device` - The object path of the device to be removed.
    fn remove_device(&self, device: zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;

    /// Retrieves the Bluetooth adapter's unique address (MAC address).
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;

    /// Retrieves the name of the Bluetooth adapter.
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Retrieves the alias name of the adapter, which can be customized.
    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;

    /// Sets a new alias name for the adapter.
    ///
    /// # Arguments
    /// * `alias` - The new alias to set.
    #[zbus(property)]
    fn set_alias(&self, alias: &str) -> zbus::Result<()>;

    /// Checks if the adapter is powered on.
    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;

    /// Enables or disables the Bluetooth adapter.
    ///
    /// # Arguments
    /// * `powered` - `true` to power on, `false` to power off.
    #[zbus(property)]
    fn set_powered(&self, powered: bool) -> zbus::Result<()>;

    /// Checks if the adapter is in discoverable mode.
    #[zbus(property)]
    fn discoverable(&self) -> zbus::Result<bool>;

    /// Sets the adapter's discoverable mode.
    ///
    /// # Arguments
    /// * `discoverable` - `true` to make discoverable, `false` to hide.
    #[zbus(property)]
    fn set_discoverable(&self, discoverable: bool) -> zbus::Result<()>;
}
