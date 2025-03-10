/// Establishes a system D-Bus connection for Bluetooth operations.
///
/// This function returns a new connection to the system bus, which
/// is required for interacting with Bluetooth adapters and devices.
///
/// # Returns
/// * `Connection` - An established connection to the system bus.
///
/// # Panics
/// This function will panic if the connection cannot be established.
pub async fn get_system_connection() -> Result<zbus::Connection, zbus::Error> {
    zbus::Connection::system().await
}
