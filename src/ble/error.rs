use std::fmt;

/// The error type of the [`ble`](crate::ble) API.
///
/// ```no_run
/// # async fn f() {
/// use bluebus::ble::{Error, Session};
/// match Session::new().await {
///     Ok(_) => {}
///     Err(Error::Bus(e)) => eprintln!("D-Bus failure: {e}"),
///     Err(e) => eprintln!("{e}"),
/// }
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A D-Bus or BlueZ error (for example `org.bluez.Error.NotReady`).
    /// [`Error::dbus_error_name`] returns the D-Bus error name.
    Bus(zbus::Error),
    /// An operation did not complete in time. The text says what was awaited.
    Timeout(String),
    /// An adapter, device, characteristic or descriptor does not exist.
    NotFound(String),
    /// An argument is malformed (UUID, MAC address, empty service, …).
    InvalidArgument(String),
}

/// Result type of the [`ble`](crate::ble) API.
///
/// ```
/// fn parse(uuid: &str) -> bluebus::ble::Result<String> {
///     bluebus::ble::normalize_uuid(uuid)
/// }
/// assert!(parse("2a19").is_ok());
/// ```
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// The D-Bus error name of a [`Error::Bus`] error, such as
    /// `"org.bluez.Error.NotReady"` or `"org.freedesktop.DBus.Error.UnknownObject"`.
    ///
    /// ```
    /// use bluebus::ble::Error;
    /// let e = Error::Bus(zbus::fdo::Error::UnknownObject("gone".into()).into());
    /// assert_eq!(e.dbus_error_name().as_deref(), Some("org.freedesktop.DBus.Error.UnknownObject"));
    /// assert_eq!(Error::Timeout("x".into()).dbus_error_name(), None);
    /// ```
    pub fn dbus_error_name(&self) -> Option<String> {
        match self {
            Error::Bus(e) => bus_error_name(e),
            _ => None,
        }
    }
}

pub(crate) fn bus_error_name(e: &zbus::Error) -> Option<String> {
    use zbus::DBusError;
    match e {
        zbus::Error::MethodError(name, _, _) => Some(name.to_string()),
        zbus::Error::FDO(e) => Some(e.name().to_string()),
        _ => None,
    }
}

/// Whether `e` means that the object (or its interface) is gone.
pub(crate) fn is_unknown_object(e: &zbus::Error) -> bool {
    matches!(
        bus_error_name(e).as_deref(),
        Some(
            "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        )
    ) || matches!(e, zbus::Error::InterfaceNotFound)
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Bus(e) => write!(f, "{e}"),
            Error::Timeout(what) => write!(f, "timed out: {what}"),
            Error::NotFound(what) => write!(f, "not found: {what}"),
            Error::InvalidArgument(what) => write!(f, "invalid argument: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Bus(e) => Some(e),
            _ => None,
        }
    }
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Self {
        Error::Bus(e)
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(e: zbus::fdo::Error) -> Self {
        Error::Bus(e.into())
    }
}

impl From<zbus::zvariant::Error> for Error {
    fn from(e: zbus::zvariant::Error) -> Self {
        Error::Bus(e.into())
    }
}
