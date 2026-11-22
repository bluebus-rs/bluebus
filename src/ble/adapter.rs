use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

use super::uuid::normalize_address;
use super::{
    Device, DeviceInfo, Error, PeripheralBuilder, Result, ScanEvent, ScanFilter, ScanStream,
    Session,
};
use crate::AdapterProxy;

const ADAPTER_INTERFACE: &str = "org.bluez.Adapter1";
const POWER_TIMEOUT: Duration = Duration::from_secs(10);

/// A local Bluetooth adapter (`hci0`, …): power, scanning, devices and the
/// local peripheral role.
///
/// Get one from [`Session::adapter`] or [`Session::default_adapter`].
/// Cheap to clone.
///
/// ```no_run
/// # async fn f() -> bluebus::ble::Result<()> {
/// use std::time::Duration;
/// use bluebus::ble::{ScanFilter, Session, Transport};
///
/// let adapter = Session::new().await?.default_adapter().await?;
/// adapter.power_on().await?;
/// let found = adapter
///     .scan(ScanFilter::new().transport(Transport::Le), Duration::from_secs(5))
///     .await?;
/// for d in found {
///     println!("{} {:?} {:?}", d.address, d.name, d.rssi);
/// }
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Adapter {
    inner: Arc<AdapterInner>,
}

struct AdapterInner {
    session: Session,
    path: String,
    name: String,
    address: String,
}

impl std::fmt::Debug for Adapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adapter")
            .field("name", &self.inner.name)
            .field("address", &self.inner.address)
            .finish()
    }
}

impl Adapter {
    pub(crate) fn new(session: Session, info: &super::AdapterInfo) -> Self {
        Adapter {
            inner: Arc::new(AdapterInner {
                session,
                path: info.path.clone(),
                name: info.name.clone(),
                address: info.address.clone(),
            }),
        }
    }

    /// Short name, e.g. `"hci0"`.
    ///
    /// ```no_run
    /// # fn f(adapter: bluebus::ble::Adapter) {
    /// assert!(adapter.name().starts_with("hci"));
    /// # }
    /// ```
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// D-Bus object path, e.g. `"/org/bluez/hci0"`.
    ///
    /// ```no_run
    /// # fn f(adapter: bluebus::ble::Adapter) {
    /// println!("{}", adapter.path());
    /// # }
    /// ```
    pub fn path(&self) -> &str {
        &self.inner.path
    }

    /// MAC address of the adapter.
    ///
    /// ```no_run
    /// # fn f(adapter: bluebus::ble::Adapter) {
    /// println!("{}", adapter.address());
    /// # }
    /// ```
    pub fn address(&self) -> &str {
        &self.inner.address
    }

    /// The session this adapter belongs to.
    ///
    /// ```no_run
    /// # fn f(adapter: bluebus::ble::Adapter) {
    /// let conn = adapter.session().connection();
    /// # }
    /// ```
    pub fn session(&self) -> &Session {
        &self.inner.session
    }

    pub(crate) fn conn(&self) -> &zbus::Connection {
        self.inner.session.connection()
    }

    pub(crate) async fn proxy(&self) -> Result<AdapterProxy<'static>> {
        Ok(AdapterProxy::builder(self.conn())
            .path(self.inner.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?)
    }

    /// A fresh snapshot of the adapter state.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// let info = adapter.info().await?;
    /// println!("{} powered={} discovering={}", info.alias, info.powered, info.discovering);
    /// # Ok(()) }
    /// ```
    pub async fn info(&self) -> Result<super::AdapterInfo> {
        let props = super::get_all(self.conn(), &self.inner.path, ADAPTER_INTERFACE).await?;
        Ok(super::AdapterInfo::from_properties(
            &self.inner.path,
            &props,
        ))
    }

    /// Whether the radio is on.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// if !adapter.is_powered().await? {
    ///     adapter.power_on().await?;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn is_powered(&self) -> Result<bool> {
        Ok(self.proxy().await?.powered().await?)
    }

    /// Turns the radio on and waits (up to 10 s) until BlueZ reports it on.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// adapter.power_on().await?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::Bus`] with `org.bluez.Error.Blocked`/`Failed` when the radio
    /// is rfkill-blocked; [`Error::Timeout`] if the state never changes.
    pub async fn power_on(&self) -> Result<()> {
        self.set_powered(true).await
    }

    /// Turns the radio off and waits (up to 10 s) until BlueZ reports it off.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// adapter.power_off().await?;
    /// # Ok(()) }
    /// ```
    pub async fn power_off(&self) -> Result<()> {
        self.set_powered(false).await
    }

    async fn set_powered(&self, on: bool) -> Result<()> {
        let proxy = self.proxy().await?;
        let mut changes =
            super::properties_changed(self.conn(), &self.inner.path, ADAPTER_INTERFACE).await?;
        if proxy.powered().await? == on {
            return Ok(());
        }
        proxy.set_powered(on).await?;
        let wait = async {
            loop {
                if proxy.powered().await? == on {
                    return Ok(());
                }
                if changes.next().await.is_none() {
                    return Err(Error::Bus(zbus::Error::Failure(
                        "D-Bus connection closed".into(),
                    )));
                }
            }
        };
        tokio::time::timeout(POWER_TIMEOUT, wait)
            .await
            .map_err(|_| {
                Error::Timeout(format!(
                    "{} did not report Powered={on} within {POWER_TIMEOUT:?}",
                    self.inner.name
                ))
            })?
    }

    /// Scans for `duration` and returns every device that matched `filter`
    /// (latest snapshot of each, sorted by address).
    ///
    /// The adapter must be powered ([`power_on`](Self::power_on)).
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// use std::time::Duration;
    /// use bluebus::ble::ScanFilter;
    /// let sensors = adapter
    ///     .scan(ScanFilter::new().name_prefix("Sensor").min_rssi(-80), Duration::from_secs(5))
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn scan(&self, filter: ScanFilter, duration: Duration) -> Result<Vec<DeviceInfo>> {
        let mut stream = self.scan_stream(filter).await?;
        let mut found: HashMap<String, DeviceInfo> = HashMap::new();
        let deadline = tokio::time::sleep(duration);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                event = stream.next() => match event {
                    Some(event) => {
                        let device = event.into_device();
                        found.insert(device.address.clone(), device);
                    }
                    None => break,
                },
            }
        }
        stream.stop().await?;
        let mut devices: Vec<DeviceInfo> = found.into_values().collect();
        devices.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(devices)
    }

    /// Starts a discovery and returns a stream of [`ScanEvent`]s for the
    /// devices matching `filter`. Discovery stops when the stream is dropped
    /// (or with [`ScanStream::stop`]).
    ///
    /// Run one scan per adapter and session at a time.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// use futures::StreamExt;
    /// use bluebus::ble::{ScanEvent, ScanFilter};
    /// let mut events = adapter.scan_stream(ScanFilter::new().service_uuid("180d")).await?;
    /// while let Some(event) = events.next().await {
    ///     match event {
    ///         ScanEvent::Found(d) => println!("found {} {:?}", d.address, d.name),
    ///         ScanEvent::Updated(d) => println!("updated {} rssi={:?}", d.address, d.rssi),
    ///         _ => {}
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for a malformed filter; [`Error::Bus`] if
    /// BlueZ refuses to start discovery (`org.bluez.Error.NotReady` when the
    /// adapter is off, `InProgress` when this session already scans).
    pub async fn scan_stream(&self, filter: ScanFilter) -> Result<ScanStream> {
        super::scan::start(self, filter).await
    }

    /// Scans until a device matches `filter` and returns it, or fails with
    /// [`Error::Timeout`]. A connected device whose address is in the
    /// filter is returned without scanning.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// use std::time::Duration;
    /// use bluebus::ble::ScanFilter;
    /// let device = adapter
    ///     .find_device(ScanFilter::new().name("Thermometer"), Duration::from_secs(10))
    ///     .await?;
    /// device.connect(Duration::from_secs(20)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn find_device(&self, filter: ScanFilter, timeout: Duration) -> Result<Device> {
        if let Some(address) = filter.address_filter() {
            let device = self.device(address)?;
            if let Ok(info) = device.info().await {
                if info.connected && filter.matches(&info) {
                    return Ok(device);
                }
            }
        }
        let mut stream = self.scan_stream(filter).await?;
        let first = tokio::time::timeout(timeout, stream.next()).await;
        stream.stop().await?;
        match first {
            Ok(Some(event)) => self.device(&event.device().address),
            Ok(None) => Err(Error::Bus(zbus::Error::Failure("scan stream ended".into()))),
            Err(_) => Err(Error::Timeout(format!(
                "no device matching the filter found within {timeout:?}"
            ))),
        }
    }

    /// A handle to the device with this MAC address on this adapter. Does not
    /// check that BlueZ knows the device; operations on an unknown device
    /// fail with [`Error::NotFound`]. Use [`find_device`](Self::find_device)
    /// to discover it first.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// let device = adapter.device("AA:BB:CC:DD:EE:FF")?;
    /// println!("connected: {}", device.is_connected().await?);
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for a malformed address.
    pub fn device(&self, address: &str) -> Result<Device> {
        let address = normalize_address(address)?;
        Ok(Device::new(self.clone(), address))
    }

    /// Devices known to BlueZ on this adapter (discovered, paired or
    /// connected), sorted by address.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// for d in adapter.devices().await? {
    ///     println!("{} {} paired={}", d.address, d.alias, d.paired);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let objects = crate::ObjectManagerProxy::new(self.conn())
            .await?
            .get_managed_objects()
            .await?;
        let mut devices: Vec<DeviceInfo> = super::scan::adapter_devices(&objects, &self.inner.path)
            .into_values()
            .collect();
        devices.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(devices)
    }

    /// Removes a device and its pairing information from BlueZ.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// adapter.remove_device("AA:BB:CC:DD:EE:FF").await?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::NotFound`] if BlueZ does not know the device.
    pub async fn remove_device(&self, address: &str) -> Result<()> {
        let address = normalize_address(address)?;
        let path = OwnedObjectPath::try_from(crate::device_path(&self.inner.path, &address))?;
        match self.proxy().await?.remove_device(path).await {
            Ok(()) => Ok(()),
            Err(e)
                if super::error::bus_error_name(&e).as_deref()
                    == Some("org.bluez.Error.DoesNotExist")
                    || super::error::is_unknown_object(&e) =>
            {
                Err(Error::NotFound(format!(
                    "device {address} on {}",
                    self.inner.name
                )))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Starts describing a local GATT server and/or advertisement on this
    /// adapter (BLE peripheral role). See [`PeripheralBuilder`].
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// use bluebus::ble::{Advertisement, Characteristic, Service};
    /// let peripheral = adapter
    ///     .peripheral()
    ///     .service(Service::new("180f").characteristic(Characteristic::new("2a19").read().notify().value(vec![100])))
    ///     .advertise(Advertisement::new().local_name("my-battery").service_uuid("180f"))
    ///     .start()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn peripheral(&self) -> PeripheralBuilder {
        PeripheralBuilder::new(self.clone())
    }
}

impl ScanEvent {
    /// The device snapshot carried by the event.
    ///
    /// ```no_run
    /// # fn f(event: bluebus::ble::ScanEvent) {
    /// println!("{}", event.device().address);
    /// # }
    /// ```
    pub fn device(&self) -> &DeviceInfo {
        match self {
            ScanEvent::Found(d) | ScanEvent::Updated(d) => d,
        }
    }

    /// Consumes the event and returns its device snapshot.
    ///
    /// ```no_run
    /// # fn f(event: bluebus::ble::ScanEvent) {
    /// let device: bluebus::ble::DeviceInfo = event.into_device();
    /// # }
    /// ```
    pub fn into_device(self) -> DeviceInfo {
        match self {
            ScanEvent::Found(d) | ScanEvent::Updated(d) => d,
        }
    }
}
