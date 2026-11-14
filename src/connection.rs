/// Opens a new connection to the D-Bus system bus, where BlueZ lives.
///
/// Every call opens a *new* connection; clone the returned
/// [`zbus::Connection`] (cheap, reference counted) to share it instead.
///
/// The bus address can be overridden with the standard
/// `DBUS_SYSTEM_BUS_ADDRESS` environment variable.
///
/// # Errors
/// Returns an error if the system bus is unreachable or authentication fails.
pub async fn get_system_connection() -> Result<zbus::Connection, zbus::Error> {
    zbus::Connection::system().await
}
