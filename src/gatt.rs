//! GATT client proxies and a GATT server (peripheral) implementation.
//!
//! # Client
//! [`GattService1Proxy`], [`GattCharacteristic1Proxy`] and
//! [`GattDescriptor1Proxy`] talk to the services of a connected remote
//! device. These proxies have no default destination; use
//! [`gatt_characteristic_proxy`] & co, or pass `"org.bluez"` explicitly.
//!
//! # Server
//! Export [`GattService`], [`GattCharacteristic`] and [`GattDescriptor`]
//! objects *below* the application path, then call
//! [`register_gatt_application`]:
//!
//! ```no_run
//! # async fn f() -> zbus::Result<()> {
//! use bluebus::*;
//! let conn = get_system_connection().await?;
//! let app = get_gatt_application_path();
//! let svc = format!("{app}/service0");
//! let chr = format!("{svc}/char0");
//! register_service(&conn, &svc, GattService {
//!     uuid: "12345678-1234-5678-1234-56789abcdef0".into(),
//!     primary: true, device: None, includes: vec![], handle: None,
//! }).await?;
//! register_characteristic(&conn, &chr, GattCharacteristic {
//!     uuid: "12345678-1234-5678-1234-56789abcdef1".into(),
//!     service: svc.clone(),
//!     value: Some(b"hello".to_vec()),
//!     write_acquired: false, notify_acquired: false, notifying: false,
//!     flags: vec!["read".into(), "write".into(), "notify".into()],
//!     handle: None, mtu: None,
//! }).await?;
//! register_gatt_application(&conn).await?;
//! // Later: push a notification to subscribed centrals.
//! notify_characteristic_value(&conn, &chr, b"world".to_vec()).await?;
//! # Ok(()) }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::{Stream, StreamExt};
use zbus::message::Header;
use zbus::object_server::{Interface, SignalEmitter};
use zbus::zvariant::{ObjectPath, Optional, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{fdo, interface, proxy, Connection};

/// Options dictionary passed to `ReadValue`/`WriteValue` (`a{sv}`).
pub type GattOptions = HashMap<String, OwnedValue>;

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

/// Proxy for a remote GATT service (`org.bluez.GattService1`).
#[proxy(interface = "org.bluez.GattService1")]
pub trait GattService1 {
    /// 128-bit service UUID.
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> fdo::Result<String>;

    /// Whether this is a primary service.
    #[zbus(property)]
    fn primary(&self) -> fdo::Result<bool>;

    /// Owning device, as a string. Prefer [`device_path`](Self::device_path).
    #[zbus(property)]
    fn device(&self) -> fdo::Result<Optional<String>>;

    /// Owning device object path.
    #[zbus(property, name = "Device")]
    fn device_path(&self) -> fdo::Result<OwnedObjectPath>;

    /// Included services, as strings. Prefer [`include_paths`](Self::include_paths).
    #[zbus(property)]
    fn includes(&self) -> fdo::Result<Vec<String>>;

    /// Included service object paths.
    #[zbus(property, name = "Includes")]
    fn include_paths(&self) -> fdo::Result<Vec<OwnedObjectPath>>;

    /// Attribute handle.
    #[zbus(property)]
    fn handle(&self) -> fdo::Result<Optional<u16>>;
}

/// Proxy for a remote GATT characteristic (`org.bluez.GattCharacteristic1`).
#[proxy(interface = "org.bluez.GattCharacteristic1")]
pub trait GattCharacteristic1 {
    /// Reads the value. Options: `offset` (`q`), `mtu` (`q`), `device` (`o`).
    fn read_value(&self, options: GattOptions) -> fdo::Result<Vec<u8>>;

    /// Writes the value. Options: `offset` (`q`), `type` (`"command"`,
    /// `"request"`, `"reliable"`), `mtu`, `device`, `link`, `prepare-authorize`.
    fn write_value(&self, value: Vec<u8>, options: GattOptions) -> fdo::Result<()>;

    /// Acquires a socket for write-without-response. Returns the fd and MTU.
    fn acquire_write(&self, options: GattOptions) -> fdo::Result<(OwnedFd, u16)>;

    /// Acquires a socket for notifications. Returns the fd and MTU.
    fn acquire_notify(&self, options: GattOptions) -> fdo::Result<(OwnedFd, u16)>;

    /// Enables notifications/indications; values arrive as `Value` changes.
    /// See [`characteristic_notifications`].
    fn start_notify(&self) -> fdo::Result<()>;

    /// Disables notifications/indications.
    fn stop_notify(&self) -> fdo::Result<()>;

    /// Confirms an indication received through `AcquireNotify`.
    fn confirm(&self) -> fdo::Result<()>;

    /// 128-bit characteristic UUID.
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> fdo::Result<String>;

    /// Owning service, as a string. Prefer [`service_path`](Self::service_path).
    #[zbus(property)]
    fn service(&self) -> fdo::Result<String>;

    /// Owning service object path.
    #[zbus(property, name = "Service")]
    fn service_path(&self) -> fdo::Result<OwnedObjectPath>;

    /// Cached value (updated by reads and notifications).
    #[zbus(property)]
    fn value(&self) -> fdo::Result<Optional<Vec<u8>>>;

    /// Whether a write socket is acquired.
    #[zbus(property)]
    fn write_acquired(&self) -> fdo::Result<bool>;

    /// Whether a notify socket is acquired.
    #[zbus(property)]
    fn notify_acquired(&self) -> fdo::Result<bool>;

    /// Whether notifications/indications are enabled.
    #[zbus(property)]
    fn notifying(&self) -> fdo::Result<bool>;

    /// Characteristic flags (`"read"`, `"write"`, `"notify"`, …).
    #[zbus(property)]
    fn flags(&self) -> fdo::Result<Vec<String>>;

    /// Attribute handle.
    #[zbus(property)]
    fn handle(&self) -> fdo::Result<Optional<u16>>;

    /// Exchanged ATT MTU.
    #[zbus(property, name = "MTU")]
    fn mtu(&self) -> fdo::Result<Optional<u16>>;
}

/// Proxy for a remote GATT descriptor (`org.bluez.GattDescriptor1`).
#[proxy(interface = "org.bluez.GattDescriptor1")]
pub trait GattDescriptor1 {
    /// Reads the descriptor value. Options: `offset`, `mtu`, `device`, `link`.
    fn read_value(&self, options: GattOptions) -> fdo::Result<Vec<u8>>;

    /// Writes the descriptor value. Options: `offset`, `device`, `link`, `prepare-authorize`.
    fn write_value(&self, value: Vec<u8>, options: GattOptions) -> fdo::Result<()>;

    /// 128-bit descriptor UUID.
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> fdo::Result<String>;

    /// Owning characteristic object path.
    #[zbus(property)]
    fn characteristic(&self) -> fdo::Result<OwnedObjectPath>;

    /// Cached value.
    #[zbus(property)]
    fn value(&self) -> fdo::Result<Vec<u8>>;

    /// Descriptor flags.
    #[zbus(property)]
    fn flags(&self) -> fdo::Result<Vec<String>>;

    /// Attribute handle.
    #[zbus(property)]
    fn handle(&self) -> fdo::Result<u16>;
}

/// Proxy for `org.bluez.GattManager1` (on the adapter object).
#[proxy(default_service = "org.bluez", interface = "org.bluez.GattManager1")]
pub trait GattManager1 {
    /// Registers a local GATT application rooted at `application`, which must
    /// implement `org.freedesktop.DBus.ObjectManager`.
    fn register_application(
        &self,
        application: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    /// Unregisters a previously registered application.
    fn unregister_application(&self, application: &ObjectPath<'_>) -> zbus::Result<()>;
}

/// Creates a [`GattService1Proxy`] for a BlueZ service object.
pub async fn gatt_service_proxy<'a>(
    conn: &Connection,
    path: impl TryInto<ObjectPath<'a>, Error = zbus::zvariant::Error>,
) -> zbus::Result<GattService1Proxy<'a>> {
    GattService1Proxy::new(conn, crate::BLUEZ_SERVICE, path.try_into()?).await
}

/// Creates a [`GattCharacteristic1Proxy`] for a BlueZ characteristic object.
pub async fn gatt_characteristic_proxy<'a>(
    conn: &Connection,
    path: impl TryInto<ObjectPath<'a>, Error = zbus::zvariant::Error>,
) -> zbus::Result<GattCharacteristic1Proxy<'a>> {
    GattCharacteristic1Proxy::new(conn, crate::BLUEZ_SERVICE, path.try_into()?).await
}

/// Creates a [`GattDescriptor1Proxy`] for a BlueZ descriptor object.
pub async fn gatt_descriptor_proxy<'a>(
    conn: &Connection,
    path: impl TryInto<ObjectPath<'a>, Error = zbus::zvariant::Error>,
) -> zbus::Result<GattDescriptor1Proxy<'a>> {
    GattDescriptor1Proxy::new(conn, crate::BLUEZ_SERVICE, path.try_into()?).await
}

/// A GATT attribute discovered on a remote device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GattAttribute {
    /// Object path of the attribute.
    pub path: String,
    /// 128-bit UUID.
    pub uuid: String,
    /// Object path of the parent (device for services, service for
    /// characteristics, characteristic for descriptors).
    pub parent: String,
    /// Flags (empty for services).
    pub flags: Vec<String>,
}

/// The GATT database of a remote device, as resolved by BlueZ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GattDatabase {
    pub services: Vec<GattAttribute>,
    pub characteristics: Vec<GattAttribute>,
    pub descriptors: Vec<GattAttribute>,
}

impl GattDatabase {
    /// Finds the first characteristic with this UUID (case-insensitive).
    pub fn characteristic_by_uuid(&self, uuid: &str) -> Option<&GattAttribute> {
        self.characteristics
            .iter()
            .find(|c| c.uuid.eq_ignore_ascii_case(uuid))
    }
}

/// Lists services, characteristics and descriptors below `device_path`.
/// The device must be connected and `ServicesResolved` must be `true`.
pub async fn discover_gatt_database(
    conn: &Connection,
    device_path: &str,
) -> zbus::Result<GattDatabase> {
    fn attr(
        path: &OwnedObjectPath,
        props: &HashMap<String, OwnedValue>,
        parent_key: &str,
    ) -> Option<GattAttribute> {
        let get_str = |k: &str| {
            props
                .get(k)
                .and_then(|v| v.downcast_ref::<zbus::zvariant::Str>().ok())
                .map(|s| s.to_string())
        };
        let parent = props
            .get(parent_key)
            .and_then(|v| v.downcast_ref::<ObjectPath>().ok())
            .map(|p| p.to_string())
            .unwrap_or_default();
        let flags = props
            .get("Flags")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<String>::try_from(v).ok())
            .unwrap_or_default();
        Some(GattAttribute {
            path: path.to_string(),
            uuid: get_str("UUID")?,
            parent,
            flags,
        })
    }

    let prefix = format!("{}/", device_path.trim_end_matches('/'));
    let objects = crate::ObjectManagerProxy::new(conn)
        .await?
        .get_managed_objects()
        .await?;
    let mut db = GattDatabase::default();
    for (path, ifaces) in &objects {
        if !path.as_str().starts_with(&prefix) {
            continue;
        }
        if let Some(a) = ifaces
            .get("org.bluez.GattService1")
            .and_then(|p| attr(path, p, "Device"))
        {
            db.services.push(a);
        }
        if let Some(a) = ifaces
            .get("org.bluez.GattCharacteristic1")
            .and_then(|p| attr(path, p, "Service"))
        {
            db.characteristics.push(a);
        }
        if let Some(a) = ifaces
            .get("org.bluez.GattDescriptor1")
            .and_then(|p| attr(path, p, "Characteristic"))
        {
            db.descriptors.push(a);
        }
    }
    for list in [
        &mut db.services,
        &mut db.characteristics,
        &mut db.descriptors,
    ] {
        list.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(db)
}

/// Enables notifications on a remote characteristic and returns a stream
/// with every received value, in order.
///
/// Built on the raw `PropertiesChanged` signals rather than zbus' property
/// cache stream, which only yields the latest value and would drop
/// notifications that arrive back to back. Dropping the stream does not call
/// `StopNotify`; call [`GattCharacteristic1Proxy::stop_notify`] when done.
pub async fn characteristic_notifications(
    proxy: &GattCharacteristic1Proxy<'static>,
) -> zbus::Result<impl Stream<Item = Vec<u8>> + Unpin + 'static> {
    let inner = proxy.inner();
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(inner.destination().to_owned())?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(inner.path().to_owned())?
        .arg(0, "org.bluez.GattCharacteristic1")?
        .build();
    // Subscribe before enabling so that no early notification is lost.
    let signals = zbus::MessageStream::for_match_rule(rule, inner.connection(), Some(1024)).await?;
    proxy.start_notify().await?;
    Ok(Box::pin(signals.filter_map(|msg| async move {
        let msg = msg.ok()?;
        let (_, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
            msg.body().deserialize().ok()?;
        Vec::<u8>::try_from(changed.get("Value")?.try_clone().ok()?).ok()
    })))
}

// ---------------------------------------------------------------------------
// Server side
// ---------------------------------------------------------------------------

fn unknown_property(name: &str) -> fdo::Error {
    fdo::Error::UnknownProperty(format!("{name} is not set"))
}

fn to_object_path(s: &str, what: &str) -> fdo::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(s.to_owned()).map_err(|e| {
        fdo::Error::InvalidArgs(format!("{what} {s:?} is not a valid object path: {e}"))
    })
}

/// Reads the optional `offset` option (`q`) of a ReadValue/WriteValue call.
fn offset_option(options: &GattOptions) -> usize {
    options
        .get("offset")
        .and_then(|v| v.downcast_ref::<u16>().ok())
        .map(usize::from)
        .unwrap_or(0)
}

/// `org.bluez.Error.InvalidOffset`, as BlueZ expects it.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum GattError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// Read or write offset past the end of the value.
    InvalidOffset(String),
    /// Operation not allowed by the attribute flags.
    NotPermitted(String),
    /// Generic failure.
    Failed(String),
}

fn read_with_offset(value: &[u8], options: &GattOptions) -> Result<Vec<u8>, GattError> {
    let offset = offset_option(options);
    value.get(offset..).map(<[u8]>::to_vec).ok_or_else(|| {
        GattError::InvalidOffset(format!("offset {offset} > length {}", value.len()))
    })
}

fn write_with_offset(
    current: &mut Vec<u8>,
    data: Vec<u8>,
    options: &GattOptions,
) -> Result<(), GattError> {
    let offset = offset_option(options);
    if offset == 0 {
        *current = data;
        return Ok(());
    }
    if offset > current.len() {
        return Err(GattError::InvalidOffset(format!(
            "offset {offset} > length {}",
            current.len()
        )));
    }
    current.truncate(offset);
    current.extend_from_slice(&data);
    Ok(())
}

/// Callback invoked when a remote central writes a local characteristic or
/// descriptor. Receives the object path and the full new value.
pub type WriteHandler = Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

lazy_static::lazy_static! {
    static ref WRITE_HANDLERS: Mutex<HashMap<String, WriteHandler>> = Mutex::new(HashMap::new());
}

/// Installs a callback run after every remote write to the local
/// characteristic or descriptor at `path`. Replaces any previous handler.
pub fn set_write_handler(path: &str, handler: impl Fn(&str, &[u8]) + Send + Sync + 'static) {
    WRITE_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_owned(), Arc::new(handler));
}

/// Removes the write callback of `path`.
pub fn remove_write_handler(path: &str) {
    WRITE_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(path);
}

fn run_write_handler(header: &Header<'_>, value: &[u8]) {
    let Some(path) = header.path() else { return };
    // Clone the handler out so the lock is not held while it runs.
    let handler = WRITE_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path.as_str())
        .cloned();
    if let Some(handler) = handler {
        handler(path.as_str(), value);
    }
}

/// A local GATT service exported to BlueZ (`org.bluez.GattService1`).
///
/// `includes` and `device` must be valid object paths; `handle` is assigned
/// by BlueZ when `None`.
pub struct GattService {
    pub uuid: String,
    pub primary: bool,
    pub device: Option<String>,
    pub includes: Vec<String>,
    pub handle: Option<u16>,
}

#[interface(name = "org.bluez.GattService1")]
impl GattService {
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        self.uuid.clone()
    }

    #[zbus(property)]
    fn primary(&self) -> bool {
        self.primary
    }

    #[zbus(property)]
    fn device(&self) -> fdo::Result<OwnedObjectPath> {
        match &self.device {
            Some(d) => to_object_path(d, "Device"),
            None => Err(unknown_property("Device")),
        }
    }

    #[zbus(property)]
    fn includes(&self) -> fdo::Result<Vec<OwnedObjectPath>> {
        self.includes
            .iter()
            .map(|p| to_object_path(p, "Includes"))
            .collect()
    }

    #[zbus(property)]
    fn handle(&self) -> fdo::Result<u16> {
        self.handle.ok_or_else(|| unknown_property("Handle"))
    }

    #[zbus(property)]
    fn set_handle(&mut self, handle: u16) {
        self.handle = Some(handle);
    }
}

/// A local GATT characteristic exported to BlueZ (`org.bluez.GattCharacteristic1`).
///
/// * `service` — object path of the parent [`GattService`].
/// * `flags` — e.g. `"read"`, `"write"`, `"write-without-response"`, `"notify"`, `"indicate"`.
/// * `value` — served to `ReadValue` and replaced by `WriteValue`.
/// * `write_acquired` / `notify_acquired` / `mtu` — not exported: the
///   socket based `Acquire*` methods are not implemented, and advertising
///   them would make BlueZ bypass `WriteValue`.
pub struct GattCharacteristic {
    pub uuid: String,
    pub service: String,
    pub value: Option<Vec<u8>>,
    pub write_acquired: bool,
    pub notify_acquired: bool,
    pub notifying: bool,
    pub flags: Vec<String>,
    pub handle: Option<u16>,
    pub mtu: Option<u16>,
}

impl GattCharacteristic {
    /// Creates a characteristic with no value and the given flags.
    pub fn new(uuid: impl Into<String>, service: impl Into<String>, flags: &[&str]) -> Self {
        Self {
            uuid: uuid.into(),
            service: service.into(),
            value: None,
            write_acquired: false,
            notify_acquired: false,
            notifying: false,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            handle: None,
            mtu: None,
        }
    }

    /// Sets the initial value.
    pub fn with_value(mut self, value: impl Into<Vec<u8>>) -> Self {
        self.value = Some(value.into());
        self
    }
}

#[interface(name = "org.bluez.GattCharacteristic1")]
impl GattCharacteristic {
    async fn read_value(&self, options: GattOptions) -> Result<Vec<u8>, GattError> {
        read_with_offset(self.value.as_deref().unwrap_or_default(), &options)
    }

    async fn write_value(
        &mut self,
        value: Vec<u8>,
        options: GattOptions,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), GattError> {
        let current = self.value.get_or_insert_with(Vec::new);
        write_with_offset(current, value, &options)?;
        run_write_handler(&header, self.value.as_deref().unwrap_or_default());
        if self.notifying {
            self.value_changed(&emitter).await?;
        }
        Ok(())
    }

    async fn start_notify(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        if !self.notifying {
            self.notifying = true;
            self.notifying_changed(&emitter).await?;
        }
        Ok(())
    }

    async fn stop_notify(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        if self.notifying {
            self.notifying = false;
            self.notifying_changed(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        self.uuid.clone()
    }

    #[zbus(property)]
    fn service(&self) -> fdo::Result<OwnedObjectPath> {
        to_object_path(&self.service, "Service")
    }

    #[zbus(property)]
    fn value(&self) -> fdo::Result<Vec<u8>> {
        self.value.clone().ok_or_else(|| unknown_property("Value"))
    }

    #[zbus(property)]
    fn notifying(&self) -> bool {
        self.notifying
    }

    #[zbus(property)]
    fn flags(&self) -> Vec<String> {
        self.flags.clone()
    }

    #[zbus(property)]
    fn handle(&self) -> fdo::Result<u16> {
        self.handle.ok_or_else(|| unknown_property("Handle"))
    }

    #[zbus(property)]
    fn set_handle(&mut self, handle: u16) {
        self.handle = Some(handle);
    }
}

/// A local GATT descriptor exported to BlueZ (`org.bluez.GattDescriptor1`).
pub struct GattDescriptor {
    pub uuid: String,
    /// Object path of the parent [`GattCharacteristic`].
    pub characteristic: String,
    pub value: Vec<u8>,
    /// e.g. `"read"`, `"write"`.
    pub flags: Vec<String>,
    pub handle: Option<u16>,
}

#[interface(name = "org.bluez.GattDescriptor1")]
impl GattDescriptor {
    async fn read_value(&self, options: GattOptions) -> Result<Vec<u8>, GattError> {
        read_with_offset(&self.value, &options)
    }

    async fn write_value(
        &mut self,
        value: Vec<u8>,
        options: GattOptions,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), GattError> {
        write_with_offset(&mut self.value, value, &options)?;
        run_write_handler(&header, &self.value);
        Ok(())
    }

    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        self.uuid.clone()
    }

    #[zbus(property)]
    fn characteristic(&self) -> fdo::Result<OwnedObjectPath> {
        to_object_path(&self.characteristic, "Characteristic")
    }

    #[zbus(property)]
    fn flags(&self) -> Vec<String> {
        self.flags.clone()
    }

    #[zbus(property)]
    fn handle(&self) -> fdo::Result<u16> {
        self.handle.ok_or_else(|| unknown_property("Handle"))
    }

    #[zbus(property)]
    fn set_handle(&mut self, handle: u16) {
        self.handle = Some(handle);
    }
}

/// Exports a service object (any `Interface`, normally [`GattService`]).
pub async fn register_service(
    connection: &Connection,
    service_path: &str,
    service: impl Interface + 'static,
) -> fdo::Result<()> {
    connection.object_server().at(service_path, service).await?;
    Ok(())
}

/// Exports a characteristic object (normally [`GattCharacteristic`]).
pub async fn register_characteristic(
    connection: &Connection,
    characteristic_path: &str,
    characteristic: impl Interface + 'static,
) -> fdo::Result<()> {
    connection
        .object_server()
        .at(characteristic_path, characteristic)
        .await?;
    Ok(())
}

/// Exports a descriptor object (normally [`GattDescriptor`]).
pub async fn register_descriptor(
    connection: &Connection,
    descriptor_path: &str,
    descriptor: impl Interface + 'static,
) -> fdo::Result<()> {
    connection
        .object_server()
        .at(descriptor_path, descriptor)
        .await?;
    Ok(())
}

/// Registers the GATT application at [`crate::get_gatt_application_path`]
/// with the adapter at [`crate::get_adapter_path`].
///
/// Export all services/characteristics/descriptors (below the application
/// path) *before* calling this: BlueZ reads the whole tree once.
pub async fn register_gatt_application(conn: &Connection) -> zbus::Result<()> {
    register_gatt_application_at(
        conn,
        &crate::get_adapter_path(),
        &crate::get_gatt_application_path(),
    )
    .await
}

/// Registers a GATT application rooted at `app_path` with `adapter_path`.
/// Exports the required `ObjectManager` at `app_path` if not yet present.
pub async fn register_gatt_application_at(
    conn: &Connection,
    adapter_path: &str,
    app_path: &str,
) -> zbus::Result<()> {
    let app = ObjectPath::try_from(app_path)?;
    // Returns Ok(false) if an ObjectManager is already exported there.
    conn.object_server().at(&app, fdo::ObjectManager).await?;
    crate::object_server_ready::ensure_object_server_ready(conn).await?;
    GattManager1Proxy::new(conn, adapter_path)
        .await?
        .register_application(&app, HashMap::new())
        .await
}

/// Unregisters the GATT application at [`crate::get_gatt_application_path`].
/// The exported objects are kept and can be registered again.
pub async fn unregister_gatt_application(conn: &Connection) -> zbus::Result<()> {
    unregister_gatt_application_at(
        conn,
        &crate::get_adapter_path(),
        &crate::get_gatt_application_path(),
    )
    .await
}

/// Unregisters the GATT application rooted at `app_path` from `adapter_path`.
pub async fn unregister_gatt_application_at(
    conn: &Connection,
    adapter_path: &str,
    app_path: &str,
) -> zbus::Result<()> {
    let app = ObjectPath::try_from(app_path)?;
    GattManager1Proxy::new(conn, adapter_path)
        .await?
        .unregister_application(&app)
        .await
}

/// Updates the value of the local characteristic at `path` and, if a
/// central has enabled notifications, sends it as a notification/indication.
pub async fn notify_characteristic_value(
    conn: &Connection,
    path: &str,
    value: Vec<u8>,
) -> zbus::Result<()> {
    let iface = conn
        .object_server()
        .interface::<_, GattCharacteristic>(path)
        .await?;
    let notifying = {
        let mut chr = iface.get_mut().await;
        chr.value = Some(value);
        chr.notifying
    };
    if notifying {
        iface
            .get()
            .await
            .value_changed(iface.signal_emitter())
            .await?;
    }
    Ok(())
}

/// Returns the current value of the local characteristic at `path`.
pub async fn local_characteristic_value(
    conn: &Connection,
    path: &str,
) -> zbus::Result<Option<Vec<u8>>> {
    let iface = conn
        .object_server()
        .interface::<_, GattCharacteristic>(path)
        .await?;
    let value = iface.get().await.value.clone();
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(offset: u16) -> GattOptions {
        HashMap::from([("offset".to_string(), OwnedValue::from(offset))])
    }

    #[test]
    fn offsets() {
        assert_eq!(
            read_with_offset(b"hello", &GattOptions::new()).unwrap(),
            b"hello"
        );
        assert_eq!(read_with_offset(b"hello", &opts(2)).unwrap(), b"llo");
        assert_eq!(read_with_offset(b"hello", &opts(5)).unwrap(), b"");
        assert!(matches!(
            read_with_offset(b"hello", &opts(6)),
            Err(GattError::InvalidOffset(_))
        ));

        let mut v = b"hello".to_vec();
        write_with_offset(&mut v, b"p!".to_vec(), &opts(3)).unwrap();
        assert_eq!(v, b"help!");
        write_with_offset(&mut v, b"x".to_vec(), &GattOptions::new()).unwrap();
        assert_eq!(v, b"x");
        assert!(write_with_offset(&mut v, b"y".to_vec(), &opts(9)).is_err());
    }
}
