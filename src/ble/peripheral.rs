use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use zbus::fdo;

use super::uuid::normalize_uuid;
use super::{Adapter, Error, Result};
use crate::{
    AdvertisementType, GattCharacteristic, GattDescriptor, GattService, LEAdvertisementProperties,
};

type WriteCallback = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

/// A characteristic of a local GATT [`Service`].
///
/// ```
/// use bluebus::ble::Characteristic;
/// let level = Characteristic::new("2a19")
///     .read()
///     .notify()
///     .value(vec![100])
///     .descriptor("2901", b"Battery level".to_vec());
/// let control = Characteristic::new("12345678-1234-5678-1234-56789abcdef1")
///     .write()
///     .write_without_response()
///     .on_write(|value| println!("written: {value:?}"));
/// ```
#[derive(Clone)]
pub struct Characteristic {
    uuid: String,
    flags: Vec<String>,
    value: Vec<u8>,
    on_write: Option<WriteCallback>,
    descriptors: Vec<(String, Vec<u8>)>,
}

impl std::fmt::Debug for Characteristic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Characteristic")
            .field("uuid", &self.uuid)
            .field("flags", &self.flags)
            .field("value", &self.value)
            .field("on_write", &self.on_write.is_some())
            .field("descriptors", &self.descriptors)
            .finish()
    }
}

impl Characteristic {
    /// A characteristic with this UUID (short or full form), no flags and an
    /// empty value. Add at least one flag.
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a37").notify();
    /// ```
    pub fn new(uuid: impl Into<String>) -> Self {
        Characteristic {
            uuid: uuid.into(),
            flags: Vec::new(),
            value: Vec::new(),
            on_write: None,
            descriptors: Vec::new(),
        }
    }

    /// Adds a BlueZ flag, such as `"encrypt-read"` or `"authorize"`.
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a19").read().flag("encrypt-read");
    /// ```
    pub fn flag(mut self, flag: impl Into<String>) -> Self {
        let flag = flag.into();
        if !self.flags.contains(&flag) {
            self.flags.push(flag);
        }
        self
    }

    /// Centrals may read the value (`"read"`).
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a19").read();
    /// ```
    pub fn read(self) -> Self {
        self.flag("read")
    }

    /// Centrals may write with response (`"write"`).
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a06").write();
    /// ```
    pub fn write(self) -> Self {
        self.flag("write")
    }

    /// Centrals may write without response (`"write-without-response"`).
    ///
    /// Note: BlueZ then also acknowledges *write requests* to this
    /// characteristic before the application has processed them, so a central
    /// may see its write complete shortly before [`Peripheral::value`] (and
    /// the [`on_write`](Self::on_write) callback) reflect it.
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a06").write_without_response();
    /// ```
    pub fn write_without_response(self) -> Self {
        self.flag("write-without-response")
    }

    /// Centrals may subscribe to notifications (`"notify"`).
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a37").notify();
    /// ```
    pub fn notify(self) -> Self {
        self.flag("notify")
    }

    /// Centrals may subscribe to indications (`"indicate"`).
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a05").indicate();
    /// ```
    pub fn indicate(self) -> Self {
        self.flag("indicate")
    }

    /// The initial value.
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a00").read().value(b"my device".to_vec());
    /// ```
    pub fn value(mut self, value: impl Into<Vec<u8>>) -> Self {
        self.value = value.into();
        self
    }

    /// Called with the new value after every write by a central.
    ///
    /// The callback runs on the D-Bus dispatcher: keep it short and do not
    /// block; spawn a task for slow work (for example to call
    /// [`Peripheral::notify`]).
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a06")
    ///     .write()
    ///     .on_write(|value| println!("alert level {value:?}"));
    /// ```
    pub fn on_write(mut self, callback: impl Fn(Vec<u8>) + Send + Sync + 'static) -> Self {
        self.on_write = Some(Arc::new(callback));
        self
    }

    /// Adds a read-only descriptor, such as `"2901"` (user description).
    ///
    /// ```
    /// let c = bluebus::ble::Characteristic::new("2a19").read().descriptor("2901", b"Level".to_vec());
    /// ```
    pub fn descriptor(mut self, uuid: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        self.descriptors.push((uuid.into(), value.into()));
        self
    }
}

/// A local GATT service, served by a [`Peripheral`].
///
/// ```
/// use bluebus::ble::{Characteristic, Service};
/// let battery = Service::new("180f").characteristic(Characteristic::new("2a19").read().value(vec![90]));
/// ```
#[derive(Debug, Clone)]
pub struct Service {
    uuid: String,
    primary: bool,
    characteristics: Vec<Characteristic>,
}

impl Service {
    /// A primary service with this UUID (short or full form).
    ///
    /// ```
    /// let s = bluebus::ble::Service::new("12345678-1234-5678-1234-56789abcdef0");
    /// ```
    pub fn new(uuid: impl Into<String>) -> Self {
        Service {
            uuid: uuid.into(),
            primary: true,
            characteristics: Vec::new(),
        }
    }

    /// Marks the service as primary (default) or secondary.
    ///
    /// ```
    /// let s = bluebus::ble::Service::new("180a").primary(false);
    /// ```
    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    /// Adds a characteristic.
    ///
    /// ```
    /// use bluebus::ble::{Characteristic, Service};
    /// let s = Service::new("180f").characteristic(Characteristic::new("2a19").read());
    /// ```
    pub fn characteristic(mut self, characteristic: Characteristic) -> Self {
        self.characteristics.push(characteristic);
        self
    }
}

/// An LE advertisement sent by a [`Peripheral`].
///
/// Connectable and discoverable by default.
///
/// ```
/// use bluebus::ble::Advertisement;
/// let adv = Advertisement::new()
///     .local_name("my-sensor")
///     .service_uuid("181a")
///     .manufacturer_data(0xFFFF, vec![1, 2, 3])
///     .service_data("181a", vec![0x2a]);
/// let beacon = Advertisement::new().broadcast().manufacturer_data(0x004C, vec![0x02, 0x15]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    local_name: Option<String>,
    service_uuids: Vec<String>,
    manufacturer_data: BTreeMap<u16, Vec<u8>>,
    service_data: BTreeMap<String, Vec<u8>>,
    discoverable: bool,
    connectable: bool,
}

impl Default for Advertisement {
    fn default() -> Self {
        Advertisement {
            local_name: None,
            service_uuids: Vec::new(),
            manufacturer_data: BTreeMap::new(),
            service_data: BTreeMap::new(),
            discoverable: true,
            connectable: true,
        }
    }
}

impl Advertisement {
    /// A connectable, discoverable advertisement with no data.
    ///
    /// ```
    /// let adv = bluebus::ble::Advertisement::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// The local name to advertise.
    ///
    /// ```
    /// let adv = bluebus::ble::Advertisement::new().local_name("thermo");
    /// ```
    pub fn local_name(mut self, name: impl Into<String>) -> Self {
        self.local_name = Some(name.into());
        self
    }

    /// Adds an advertised service UUID (short or full form).
    ///
    /// ```
    /// let adv = bluebus::ble::Advertisement::new().service_uuid("180d").service_uuid("180f");
    /// ```
    pub fn service_uuid(mut self, uuid: impl Into<String>) -> Self {
        self.service_uuids.push(uuid.into());
        self
    }

    /// Sets manufacturer specific data for a company identifier.
    ///
    /// ```
    /// let adv = bluebus::ble::Advertisement::new().manufacturer_data(0xFFFF, vec![0xCA, 0xFE]);
    /// ```
    pub fn manufacturer_data(mut self, company_id: u16, data: impl Into<Vec<u8>>) -> Self {
        self.manufacturer_data.insert(company_id, data.into());
        self
    }

    /// Sets service data for a service UUID.
    ///
    /// ```
    /// let adv = bluebus::ble::Advertisement::new().service_data("feaa", vec![0x10, 0x00]);
    /// ```
    pub fn service_data(mut self, uuid: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        self.service_data.insert(uuid.into(), data.into());
        self
    }

    /// Whether to set the LE General Discoverable flag (default `true`;
    /// ignored for [`broadcast`](Self::broadcast) advertisements).
    ///
    /// ```
    /// let adv = bluebus::ble::Advertisement::new().discoverable(false);
    /// ```
    pub fn discoverable(mut self, discoverable: bool) -> Self {
        self.discoverable = discoverable;
        self
    }

    /// Makes the advertisement non-connectable (a beacon).
    ///
    /// ```
    /// let beacon = bluebus::ble::Advertisement::new().broadcast();
    /// ```
    pub fn broadcast(mut self) -> Self {
        self.connectable = false;
        self
    }

    fn to_properties(&self) -> Result<LEAdvertisementProperties> {
        let kind = if self.connectable {
            AdvertisementType::Peripheral
        } else {
            AdvertisementType::Broadcast
        };
        let mut adv = LEAdvertisementProperties::new(kind);
        if let Some(name) = &self.local_name {
            adv = adv.with_local_name(name.clone());
        }
        if !self.service_uuids.is_empty() {
            let uuids = self
                .service_uuids
                .iter()
                .map(|u| normalize_uuid(u))
                .collect::<Result<Vec<_>>>()?;
            adv = adv.with_service_uuids(uuids);
        }
        for (id, data) in &self.manufacturer_data {
            adv = adv.with_manufacturer_data(*id, data.clone());
        }
        for (uuid, data) in &self.service_data {
            adv = adv.with_service_data(normalize_uuid(uuid)?, data.clone());
        }
        if self.connectable && self.discoverable {
            adv = adv.with_discoverable(true);
        }
        Ok(adv)
    }
}

/// Describes a local BLE peripheral: GATT services and/or an advertisement.
/// Created by [`Adapter::peripheral`]; [`start`](Self::start) publishes it.
///
/// ```no_run
/// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
/// use bluebus::ble::{Advertisement, Characteristic, Service};
/// let peripheral = adapter
///     .peripheral()
///     .service(
///         Service::new("12345678-1234-5678-1234-56789abcdef0").characteristic(
///             Characteristic::new("12345678-1234-5678-1234-56789abcdef1")
///                 .read()
///                 .write()
///                 .notify()
///                 .value(b"hello".to_vec())
///                 .on_write(|v| println!("written {v:?}")),
///         ),
///     )
///     .advertise(Advertisement::new().local_name("bluebus").service_uuid("12345678-1234-5678-1234-56789abcdef0"))
///     .start()
///     .await?;
/// peripheral.notify("12345678-1234-5678-1234-56789abcdef1", b"world".to_vec()).await?;
/// peripheral.stop().await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct PeripheralBuilder {
    adapter: Adapter,
    services: Vec<Service>,
    advertisement: Option<Advertisement>,
}

impl PeripheralBuilder {
    pub(crate) fn new(adapter: Adapter) -> Self {
        PeripheralBuilder {
            adapter,
            services: Vec::new(),
            advertisement: None,
        }
    }

    /// Adds a GATT service.
    ///
    /// ```no_run
    /// # fn f(adapter: bluebus::ble::Adapter) {
    /// use bluebus::ble::{Characteristic, Service};
    /// let builder = adapter.peripheral().service(Service::new("180f").characteristic(Characteristic::new("2a19").read()));
    /// # }
    /// ```
    pub fn service(mut self, service: Service) -> Self {
        self.services.push(service);
        self
    }

    /// Advertises while the peripheral runs.
    ///
    /// ```no_run
    /// # fn f(adapter: bluebus::ble::Adapter) {
    /// use bluebus::ble::Advertisement;
    /// let builder = adapter.peripheral().advertise(Advertisement::new().local_name("beacon").broadcast());
    /// # }
    /// ```
    pub fn advertise(mut self, advertisement: Advertisement) -> Self {
        self.advertisement = Some(advertisement);
        self
    }

    /// Exports the GATT objects, registers the application and the
    /// advertisement with BlueZ, and returns the running [`Peripheral`].
    /// Each started peripheral uses its own object paths, so several can run
    /// at once or one after another.
    ///
    /// ```no_run
    /// # async fn f(builder: bluebus::ble::PeripheralBuilder) -> bluebus::ble::Result<()> {
    /// let peripheral = builder.start().await?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for a malformed UUID, a characteristic
    /// without flags, or nothing to publish; [`Error::Bus`] if BlueZ rejects
    /// the application or advertisement. Nothing stays registered on error.
    pub async fn start(self) -> Result<Peripheral> {
        if self.services.is_empty() && self.advertisement.is_none() {
            return Err(Error::InvalidArgument(
                "a peripheral needs at least one service or an advertisement".into(),
            ));
        }
        let advertisement = self
            .advertisement
            .as_ref()
            .map(Advertisement::to_properties)
            .transpose()?;
        let conn = self.adapter.conn().clone();
        let adapter_path = self.adapter.path().to_owned();
        let mut res = Resources {
            conn: conn.clone(),
            adapter_path: adapter_path.clone(),
            app: None,
            app_registered: false,
            services: Vec::new(),
            characteristics: Vec::new(),
            descriptors: Vec::new(),
            advertisement: None,
        };
        let result = async {
            if !self.services.is_empty() {
                let app = super::unique_path("app");
                res.app = Some(app.clone());
                for (i, service) in self.services.iter().enumerate() {
                    export_service(&mut res, &app, i, service).await?;
                }
                crate::register_gatt_application_at(&conn, &adapter_path, &app).await?;
                res.app_registered = true;
            }
            if let Some(adv) = advertisement {
                let path = super::unique_path("adv");
                crate::register_advertisement_at(&conn, &adapter_path, &path, adv).await?;
                res.advertisement = Some(path);
            }
            Ok::<(), Error>(())
        }
        .await;
        if let Err(e) = result {
            let _ = res.release().await;
            return Err(e);
        }
        Ok(Peripheral {
            inner: Arc::new(PeripheralInner {
                conn,
                characteristics: res
                    .characteristics
                    .iter()
                    .map(|(uuid, path)| (uuid.clone(), path.clone()))
                    .collect(),
                resources: Mutex::new(Some(res)),
                runtime: tokio::runtime::Handle::try_current().ok(),
            }),
        })
    }
}

async fn export_service(res: &mut Resources, app: &str, i: usize, service: &Service) -> Result<()> {
    let svc_path = format!("{app}/service{i}");
    let svc_uuid = normalize_uuid(&service.uuid)?;
    crate::register_service(
        &res.conn,
        &svc_path,
        GattService {
            uuid: svc_uuid,
            primary: service.primary,
            device: None,
            includes: vec![],
            handle: None,
        },
    )
    .await?;
    res.services.push(svc_path.clone());
    for (j, chr) in service.characteristics.iter().enumerate() {
        let chr_uuid = normalize_uuid(&chr.uuid)?;
        if chr.flags.is_empty() {
            return Err(Error::InvalidArgument(format!(
                "characteristic {chr_uuid} has no flags (add .read(), .write(), .notify(), …)"
            )));
        }
        let chr_path = format!("{svc_path}/char{j}");
        let mut exported = GattCharacteristic::new(chr_uuid.clone(), svc_path.clone(), &[]);
        exported.flags = chr.flags.clone();
        exported.value = Some(chr.value.clone());
        crate::register_characteristic(&res.conn, &chr_path, exported).await?;
        res.characteristics.push((chr_uuid, chr_path.clone()));
        if let Some(callback) = &chr.on_write {
            let callback = callback.clone();
            crate::set_write_handler(&chr_path, move |_, value| callback(value.to_vec()));
        }
        for (k, (uuid, value)) in chr.descriptors.iter().enumerate() {
            let desc_path = format!("{chr_path}/desc{k}");
            crate::register_descriptor(
                &res.conn,
                &desc_path,
                GattDescriptor {
                    uuid: normalize_uuid(uuid)?,
                    characteristic: chr_path.clone(),
                    value: value.clone(),
                    flags: vec!["read".into()],
                    handle: None,
                },
            )
            .await?;
            res.descriptors.push(desc_path);
        }
    }
    Ok(())
}

/// What a peripheral registered, to be released by `stop` or on drop.
struct Resources {
    conn: zbus::Connection,
    adapter_path: String,
    app: Option<String>,
    app_registered: bool,
    services: Vec<String>,
    /// (uuid, path)
    characteristics: Vec<(String, String)>,
    descriptors: Vec<String>,
    advertisement: Option<String>,
}

impl Resources {
    fn remove_write_handlers(&self) {
        for (_, path) in &self.characteristics {
            crate::remove_write_handler(path);
        }
    }

    /// Unregisters and unexports everything; returns the first error.
    async fn release(self) -> Result<()> {
        self.remove_write_handlers();
        let mut first: Option<Error> = None;
        let mut note = |r: zbus::Result<()>| {
            if let Err(e) = r {
                first.get_or_insert(e.into());
            }
        };
        if let Some(adv) = &self.advertisement {
            note(crate::unregister_advertisement_at(&self.conn, &self.adapter_path, adv).await);
        }
        if let (Some(app), true) = (&self.app, self.app_registered) {
            note(crate::unregister_gatt_application_at(&self.conn, &self.adapter_path, app).await);
        }
        let server = self.conn.object_server();
        for path in &self.descriptors {
            let _ = server.remove::<GattDescriptor, _>(path.as_str()).await;
        }
        for (_, path) in &self.characteristics {
            let _ = server.remove::<GattCharacteristic, _>(path.as_str()).await;
        }
        for path in &self.services {
            let _ = server.remove::<GattService, _>(path.as_str()).await;
        }
        if let Some(app) = &self.app {
            let _ = server.remove::<fdo::ObjectManager, _>(app.as_str()).await;
        }
        match first {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// A running local peripheral (GATT server and/or advertisement), started
/// by [`PeripheralBuilder::start`]. Cheap to clone.
///
/// [`stop`](Self::stop) unregisters everything; when the last clone is
/// dropped without `stop`, the same happens in the background.
///
/// ```no_run
/// # async fn f(peripheral: bluebus::ble::Peripheral) -> bluebus::ble::Result<()> {
/// // Update the value centrals read, and notify subscribers.
/// peripheral.notify("2a19", vec![42]).await?;
/// println!("current: {:?}", peripheral.value("2a19").await?);
/// peripheral.stop().await?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Peripheral {
    inner: Arc<PeripheralInner>,
}

struct PeripheralInner {
    conn: zbus::Connection,
    /// (uuid, path)
    characteristics: Vec<(String, String)>,
    resources: Mutex<Option<Resources>>,
    runtime: Option<tokio::runtime::Handle>,
}

impl std::fmt::Debug for Peripheral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peripheral")
            .field("characteristics", &self.inner.characteristics)
            .finish_non_exhaustive()
    }
}

impl Drop for PeripheralInner {
    fn drop(&mut self) {
        let Some(res) = self
            .resources
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        res.remove_write_handlers();
        if let Some(runtime) = &self.runtime {
            runtime.spawn(async move {
                let _ = res.release().await;
            });
        }
    }
}

impl Peripheral {
    fn resources(&self) -> std::sync::MutexGuard<'_, Option<Resources>> {
        self.inner
            .resources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn path_of(&self, uuid: &str) -> Result<String> {
        let uuid = normalize_uuid(uuid)?;
        if self.resources().is_none() {
            return Err(Error::InvalidArgument("the peripheral is stopped".into()));
        }
        self.inner
            .characteristics
            .iter()
            .find(|(u, _)| *u == uuid)
            .map(|(_, p)| p.clone())
            .ok_or_else(|| Error::NotFound(format!("local characteristic {uuid}")))
    }

    async fn interface(
        &self,
        uuid: &str,
    ) -> Result<zbus::object_server::InterfaceRef<GattCharacteristic>> {
        let path = self.path_of(uuid)?;
        Ok(self
            .inner
            .conn
            .object_server()
            .interface::<_, GattCharacteristic>(path.as_str())
            .await?)
    }

    /// Whether the peripheral is still registered (not stopped).
    ///
    /// ```no_run
    /// # fn f(peripheral: bluebus::ble::Peripheral) {
    /// assert!(peripheral.is_running());
    /// # }
    /// ```
    pub fn is_running(&self) -> bool {
        self.resources().is_some()
    }

    /// Replaces the value of a local characteristic without notifying.
    ///
    /// ```no_run
    /// # async fn f(peripheral: bluebus::ble::Peripheral) -> bluebus::ble::Result<()> {
    /// peripheral.set_value("2a19", vec![80]).await?;
    /// # Ok(()) }
    /// ```
    pub async fn set_value(&self, uuid: &str, value: impl Into<Vec<u8>>) -> Result<()> {
        self.interface(uuid).await?.get_mut().await.value = Some(value.into());
        Ok(())
    }

    /// Replaces the value of a local characteristic and, if a central has
    /// subscribed, sends it as a notification (or indication).
    ///
    /// ```no_run
    /// # async fn f(peripheral: bluebus::ble::Peripheral) -> bluebus::ble::Result<()> {
    /// peripheral.notify("2a37", vec![0x00, 72]).await?;
    /// # Ok(()) }
    /// ```
    pub async fn notify(&self, uuid: &str, value: impl Into<Vec<u8>>) -> Result<()> {
        let path = self.path_of(uuid)?;
        Ok(crate::notify_characteristic_value(&self.inner.conn, &path, value.into()).await?)
    }

    /// The current value of a local characteristic (after remote writes).
    ///
    /// ```no_run
    /// # async fn f(peripheral: bluebus::ble::Peripheral) -> bluebus::ble::Result<()> {
    /// let value = peripheral.value("2a06").await?;
    /// # Ok(()) }
    /// ```
    pub async fn value(&self, uuid: &str) -> Result<Vec<u8>> {
        Ok(self
            .interface(uuid)
            .await?
            .get()
            .await
            .value
            .clone()
            .unwrap_or_default())
    }

    /// Whether a central has subscribed to notifications of a local
    /// characteristic.
    ///
    /// ```no_run
    /// # async fn f(peripheral: bluebus::ble::Peripheral) -> bluebus::ble::Result<()> {
    /// if peripheral.is_notifying("2a37").await? {
    ///     peripheral.notify("2a37", vec![0, 60]).await?;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn is_notifying(&self, uuid: &str) -> Result<bool> {
        Ok(self.interface(uuid).await?.get().await.notifying)
    }

    /// Unregisters the advertisement and the GATT application and removes
    /// the exported objects. Stopping twice is a no-op.
    ///
    /// ```no_run
    /// # async fn f(peripheral: bluebus::ble::Peripheral) -> bluebus::ble::Result<()> {
    /// peripheral.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn stop(&self) -> Result<()> {
        let res = self.resources().take();
        match res {
            Some(res) => res.release().await,
            None => Ok(()),
        }
    }
}
