use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use zbus::zvariant::OwnedValue;

use super::uuid::{normalize_address, normalize_uuid};
use super::{Adapter, DeviceInfo, Result, Session, Transport};
use crate::{DiscoveryFilter, ObjectsMap, DEVICE_INTERFACE};

#[derive(Debug, Clone, PartialEq, Eq)]
enum NameMatch {
    Exact(String),
    Prefix(String),
}

/// Which devices a scan reports. Every criterion is optional; an empty
/// filter reports every device.
///
/// Transport, service UUIDs, minimum RSSI and duplicate data are also passed
/// to BlueZ as a discovery filter; name and address are checked locally.
///
/// ```
/// use bluebus::ble::{ScanFilter, Transport};
/// let filter = ScanFilter::new()
///     .transport(Transport::Le)
///     .name_prefix("Sensor")
///     .service_uuid("181a")
///     .min_rssi(-85)
///     .duplicate_data(false);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanFilter {
    transport: Option<Transport>,
    name: Option<NameMatch>,
    address: Option<String>,
    service_uuids: Vec<String>,
    min_rssi: Option<i16>,
    duplicate_data: Option<bool>,
}

impl ScanFilter {
    /// An empty filter: every device matches.
    ///
    /// ```
    /// let any = bluebus::ble::ScanFilter::new();
    /// assert_eq!(any, Default::default());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Restricts discovery to LE, BR/EDR or both ([`Transport::Auto`]).
    ///
    /// ```
    /// use bluebus::ble::{ScanFilter, Transport};
    /// let le_only = ScanFilter::new().transport(Transport::Le);
    /// ```
    pub fn transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Only devices whose name (or alias) is exactly `name`.
    ///
    /// ```
    /// let f = bluebus::ble::ScanFilter::new().name("Thermometer");
    /// ```
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(NameMatch::Exact(name.into()));
        self
    }

    /// Only devices whose name (or alias) starts with `prefix`.
    ///
    /// ```
    /// let f = bluebus::ble::ScanFilter::new().name_prefix("Polar H10");
    /// ```
    pub fn name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.name = Some(NameMatch::Prefix(prefix.into()));
        self
    }

    /// Only the device with this MAC address (case-insensitive).
    ///
    /// ```
    /// let f = bluebus::ble::ScanFilter::new().address("aa:bb:cc:dd:ee:ff");
    /// ```
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Only devices advertising this service UUID (short or full form). May
    /// be called several times: a device matches if it advertises any of them.
    ///
    /// ```
    /// let f = bluebus::ble::ScanFilter::new().service_uuid("180d").service_uuid("0x180f");
    /// ```
    pub fn service_uuid(mut self, uuid: impl Into<String>) -> Self {
        self.service_uuids.push(uuid.into());
        self
    }

    /// Adds several service UUIDs; see [`service_uuid`](Self::service_uuid).
    ///
    /// ```
    /// let f = bluebus::ble::ScanFilter::new().service_uuids(["180d", "180f"]);
    /// ```
    pub fn service_uuids<I, S>(mut self, uuids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.service_uuids.extend(uuids.into_iter().map(Into::into));
        self
    }

    /// Only devices received with at least this RSSI (dBm).
    ///
    /// ```
    /// let near = bluebus::ble::ScanFilter::new().min_rssi(-60);
    /// ```
    pub fn min_rssi(mut self, rssi: i16) -> Self {
        self.min_rssi = Some(rssi);
        self
    }

    /// `true` reports every received advertisement as an
    /// [`ScanEvent::Updated`] event (BlueZ `DuplicateData`), not only changes.
    ///
    /// ```
    /// let f = bluebus::ble::ScanFilter::new().duplicate_data(true);
    /// ```
    pub fn duplicate_data(mut self, enabled: bool) -> Self {
        self.duplicate_data = Some(enabled);
        self
    }

    /// Whether `device` satisfies the local criteria (name, address, service
    /// UUIDs, minimum RSSI). The transport is enforced by BlueZ only.
    ///
    /// ```no_run
    /// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
    /// let filter = bluebus::ble::ScanFilter::new().name_prefix("Sensor");
    /// let known: Vec<_> = adapter.devices().await?.into_iter().filter(|d| filter.matches(d)).collect();
    /// # Ok(()) }
    /// ```
    pub fn matches(&self, device: &DeviceInfo) -> bool {
        if let Some(address) = &self.address {
            if !address.trim().eq_ignore_ascii_case(&device.address) {
                return false;
            }
        }
        match &self.name {
            Some(NameMatch::Exact(n)) => {
                if device.name.as_deref() != Some(n.as_str()) && device.alias != *n {
                    return false;
                }
            }
            Some(NameMatch::Prefix(p)) => {
                let name_ok = device.name.as_deref().is_some_and(|n| n.starts_with(p));
                if !name_ok && !device.alias.starts_with(p.as_str()) {
                    return false;
                }
            }
            None => {}
        }
        if !self.service_uuids.is_empty() {
            let wanted = |u: &String| {
                let u = normalize_uuid(u).unwrap_or_else(|_| u.to_ascii_lowercase());
                device.uuids.contains(&u) || device.service_data.contains_key(&u)
            };
            if !self.service_uuids.iter().any(wanted) {
                return false;
            }
        }
        if let Some(min) = self.min_rssi {
            if !device.rssi.is_some_and(|r| r >= min) {
                return false;
            }
        }
        true
    }

    pub(crate) fn address_filter(&self) -> Option<&str> {
        self.address.as_deref()
    }

    /// Checks and normalises UUIDs and the address.
    fn validated(mut self) -> Result<Self> {
        self.service_uuids = self
            .service_uuids
            .iter()
            .map(|u| normalize_uuid(u))
            .collect::<Result<_>>()?;
        if let Some(a) = &self.address {
            self.address = Some(normalize_address(a)?);
        }
        Ok(self)
    }

    fn discovery_filter(&self) -> DiscoveryFilter {
        let mut f = DiscoveryFilter::new();
        f.transport = self.transport;
        if !self.service_uuids.is_empty() {
            f.uuids = Some(self.service_uuids.clone());
        }
        f.rssi = self.min_rssi;
        f.duplicate_data = self.duplicate_data;
        f
    }
}

/// An event of a [`ScanStream`].
///
/// ```no_run
/// # fn f(event: bluebus::ble::ScanEvent) {
/// use bluebus::ble::ScanEvent;
/// match event {
///     ScanEvent::Found(d) => println!("new device {}", d.address),
///     ScanEvent::Updated(d) => println!("{} rssi={:?}", d.address, d.rssi),
///     _ => {}
/// }
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanEvent {
    /// A device matched the filter for the first time in this scan.
    Found(DeviceInfo),
    /// A device already reported changed (RSSI, advertised data, …).
    Updated(DeviceInfo),
}

/// A running discovery, as a [`Stream`] of [`ScanEvent`]s. Created by
/// [`Adapter::scan_stream`]. Dropping it stops the discovery in the
/// background; [`stop`](Self::stop) does it and waits.
///
/// ```no_run
/// # async fn f(adapter: bluebus::ble::Adapter) -> bluebus::ble::Result<()> {
/// use futures::StreamExt;
/// let mut scan = adapter.scan_stream(Default::default()).await?;
/// if let Some(event) = scan.next().await {
///     println!("{:?}", event.device());
/// }
/// scan.stop().await?;
/// # Ok(()) }
/// ```
pub struct ScanStream {
    events: mpsc::Receiver<ScanEvent>,
    task: JoinHandle<()>,
    stopper: Option<Stopper>,
}

impl std::fmt::Debug for ScanStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanStream").finish_non_exhaustive()
    }
}

/// Everything needed to stop the discovery.
struct Stopper {
    adapter: Adapter,
    runtime: tokio::runtime::Handle,
}

impl Stopper {
    async fn stop(adapter: Adapter) -> Result<()> {
        let proxy = adapter.proxy().await?;
        let stopped = proxy.stop_discovery().await;
        let _ = proxy.set_discovery_filter(HashMap::new()).await;
        match stopped {
            Ok(()) => Ok(()),
            // Already stopped (adapter powered off, …).
            Err(e)
                if matches!(
                    super::error::bus_error_name(&e).as_deref(),
                    Some("org.bluez.Error.Failed" | "org.bluez.Error.NotReady")
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl ScanStream {
    /// Stops the discovery and waits until BlueZ confirmed it.
    ///
    /// ```no_run
    /// # async fn f(scan: bluebus::ble::ScanStream) -> bluebus::ble::Result<()> {
    /// scan.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn stop(mut self) -> Result<()> {
        self.task.abort();
        match self.stopper.take() {
            Some(s) => Stopper::stop(s.adapter).await,
            None => Ok(()),
        }
    }
}

impl Stream for ScanStream {
    type Item = ScanEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ScanEvent>> {
        self.events.poll_recv(cx)
    }
}

impl Drop for ScanStream {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(s) = self.stopper.take() {
            let session: Session = s.adapter.session().clone();
            let path = s.adapter.path().to_owned();
            let task = s.runtime.spawn(async move {
                let _ = Stopper::stop(s.adapter).await;
            });
            session.set_pending_scan_stop(path, task);
        }
    }
}

fn is_adapter_device(path: &str, adapter_path: &str) -> bool {
    path.strip_prefix(adapter_path)
        .and_then(|rest| rest.strip_prefix("/dev_"))
        .is_some_and(|rest| !rest.contains('/'))
}

/// Devices of `adapter_path` among the managed objects: path → snapshot.
pub(crate) fn adapter_devices(
    objects: &ObjectsMap,
    adapter_path: &str,
) -> HashMap<String, DeviceInfo> {
    objects
        .iter()
        .filter(|(path, _)| is_adapter_device(path.as_str(), adapter_path))
        .filter_map(|(path, ifaces)| {
            let info = DeviceInfo::from_properties(ifaces.get(DEVICE_INTERFACE)?)?;
            Some((path.to_string(), info))
        })
        .collect()
}

type PropsChanged = (String, HashMap<String, OwnedValue>, Vec<String>);

pub(crate) async fn start(adapter: &Adapter, filter: ScanFilter) -> Result<ScanStream> {
    let filter = filter.validated()?;
    let conn = adapter.conn().clone();
    let adapter_path = adapter.path().to_owned();
    if let Some(pending) = adapter.session().take_pending_scan_stop(&adapter_path) {
        let _ = pending.await;
    }

    // Subscribe before starting so that nothing is missed.
    let manager = crate::ObjectManagerProxy::new(&conn).await?;
    let mut added = manager.receive_interfaces_added().await?;
    let mut removed = manager.receive_interfaces_removed().await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(crate::BLUEZ_SERVICE)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace(adapter_path.clone())?
        .arg(0, DEVICE_INTERFACE)?
        .build();
    let mut changed = zbus::MessageStream::for_match_rule(rule, &conn, Some(1024)).await?;
    let mut known = adapter_devices(&manager.get_managed_objects().await?, &adapter_path);

    let proxy = adapter.proxy().await?;
    proxy
        .set_discovery_filter(filter.discovery_filter().to_dict())
        .await?;
    if let Err(e) = proxy.start_discovery().await {
        let _ = proxy.set_discovery_filter(HashMap::new()).await;
        return Err(e.into());
    }

    let (tx, events) = mpsc::channel(256);
    let task = tokio::spawn(async move {
        let mut reported: HashSet<String> = HashSet::new();
        // Sends an event if the device matches. `false` once nobody listens.
        async fn report(
            path: &str,
            known: &HashMap<String, DeviceInfo>,
            reported: &mut HashSet<String>,
            filter: &ScanFilter,
            tx: &mpsc::Sender<ScanEvent>,
        ) -> bool {
            let Some(info) = known.get(path) else {
                return true;
            };
            if !filter.matches(info) {
                return true;
            }
            let event = if reported.insert(path.to_owned()) {
                ScanEvent::Found(info.clone())
            } else {
                ScanEvent::Updated(info.clone())
            };
            tx.send(event).await.is_ok()
        }

        // Devices already seen by a running discovery (they have an RSSI).
        let mut initial: Vec<String> = known
            .iter()
            .filter(|(_, d)| d.rssi.is_some())
            .map(|(p, _)| p.clone())
            .collect();
        initial.sort();
        for path in initial {
            if !report(&path, &known, &mut reported, &filter, &tx).await {
                return;
            }
        }

        loop {
            let path = tokio::select! {
                Some(signal) = added.next() => {
                    let Ok(args) = signal.args() else { continue };
                    let path = args.object_path().to_string();
                    if !is_adapter_device(&path, &adapter_path) {
                        continue;
                    }
                    let Some(info) = args
                        .interfaces()
                        .get(DEVICE_INTERFACE)
                        .and_then(DeviceInfo::from_properties)
                    else {
                        continue;
                    };
                    known.insert(path.clone(), info);
                    path
                }
                Some(signal) = removed.next() => {
                    if let Ok(args) = signal.args() {
                        if args.interfaces().iter().any(|i| i == DEVICE_INTERFACE) {
                            let path = args.object_path().to_string();
                            known.remove(&path);
                            reported.remove(&path);
                        }
                    }
                    continue;
                }
                Some(msg) = changed.next() => {
                    let Ok(msg) = msg else { continue };
                    let Some(path) = msg.header().path().map(|p| p.to_string()) else {
                        continue;
                    };
                    if !is_adapter_device(&path, &adapter_path) {
                        continue;
                    }
                    let Ok((_, props, invalidated)) = msg.body().deserialize::<PropsChanged>()
                    else {
                        continue;
                    };
                    if !known.contains_key(&path) {
                        // Appeared before we subscribed but after the snapshot.
                        match super::get_all(&conn, &path, DEVICE_INTERFACE)
                            .await
                            .ok()
                            .and_then(|p| DeviceInfo::from_properties(&p))
                        {
                            Some(info) => {
                                known.insert(path.clone(), info);
                            }
                            None => continue,
                        }
                    }
                    let info = known.get_mut(&path).expect("inserted above");
                    for (name, value) in &props {
                        info.apply(name, value);
                    }
                    for name in &invalidated {
                        info.invalidate(name);
                    }
                    path
                }
                else => return,
            };
            if !report(&path, &known, &mut reported, &filter, &tx).await {
                return;
            }
        }
    });
    Ok(ScanStream {
        events,
        task,
        stopper: Some(Stopper {
            adapter: adapter.clone(),
            runtime: tokio::runtime::Handle::current(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: Option<&str>, rssi: Option<i16>) -> DeviceInfo {
        DeviceInfo {
            address: "AA:BB:CC:DD:EE:FF".into(),
            name: name.map(Into::into),
            alias: name.unwrap_or("AA-BB-CC-DD-EE-FF").into(),
            rssi,
            uuids: vec!["0000180f-0000-1000-8000-00805f9b34fb".into()],
            ..Default::default()
        }
    }

    #[test]
    fn filter_matching() {
        let d = device(Some("Sensor 1"), Some(-50));
        assert!(ScanFilter::new().matches(&d));
        assert!(ScanFilter::new().name("Sensor 1").matches(&d));
        assert!(!ScanFilter::new().name("Sensor").matches(&d));
        assert!(ScanFilter::new().name_prefix("Sensor").matches(&d));
        assert!(!ScanFilter::new().name_prefix("Other").matches(&d));
        assert!(ScanFilter::new().address("aa:bb:cc:dd:ee:ff").matches(&d));
        assert!(!ScanFilter::new().address("aa:bb:cc:dd:ee:00").matches(&d));
        assert!(ScanFilter::new().service_uuid("180F").matches(&d));
        assert!(ScanFilter::new()
            .service_uuids(["180d", "0x180f"])
            .matches(&d));
        assert!(!ScanFilter::new().service_uuid("180d").matches(&d));
        assert!(ScanFilter::new().min_rssi(-50).matches(&d));
        assert!(!ScanFilter::new().min_rssi(-49).matches(&d));
        assert!(!ScanFilter::new()
            .min_rssi(-100)
            .matches(&device(None, None)));
        assert!(!ScanFilter::new().name("x").matches(&device(None, None)));
    }

    #[test]
    fn filter_to_bluez() {
        let f = ScanFilter::new()
            .transport(Transport::Le)
            .service_uuid("180f")
            .min_rssi(-70)
            .duplicate_data(true)
            .name("ignored by BlueZ")
            .validated()
            .unwrap();
        let d = f.discovery_filter();
        assert_eq!(d.transport, Some(Transport::Le));
        assert_eq!(
            d.uuids,
            Some(vec!["0000180f-0000-1000-8000-00805f9b34fb".to_string()])
        );
        assert_eq!(d.rssi, Some(-70));
        assert_eq!(d.duplicate_data, Some(true));
        assert_eq!(d.pattern, None);
        assert!(ScanFilter::new().discovery_filter().to_dict().is_empty());
        assert!(ScanFilter::new().service_uuid("nope").validated().is_err());
        assert!(ScanFilter::new().address("nope").validated().is_err());
    }

    #[test]
    fn device_paths() {
        assert!(is_adapter_device(
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF",
            "/org/bluez/hci0"
        ));
        assert!(!is_adapter_device(
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/service0001",
            "/org/bluez/hci0"
        ));
        assert!(!is_adapter_device(
            "/org/bluez/hci10/dev_AA_BB_CC_DD_EE_FF",
            "/org/bluez/hci1"
        ));
    }
}
