//! Test harness: a private `dbus-daemon` plus a mock `org.bluez` service.
//!
//! The mock follows the BlueZ D-Bus API contract (doc/*.rst in the BlueZ
//! tree): object layout, property names and signatures, and the validation
//! BlueZ performs when it reads advertisements, GATT applications and agents.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::message::Header;
use zbus::names::{InterfaceName, OwnedUniqueName};
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{fdo, interface, Connection};

pub const ADAPTER0: &str = "/org/bluez/hci0";
pub const ADAPTER1: &str = "/org/bluez/hci1";

pub const SVC_UUID: &str = "0000180f-0000-1000-8000-00805f9b34fb";
pub const CHR_UUID: &str = "00002a19-0000-1000-8000-00805f9b34fb";
pub const DESC_UUID: &str = "00002902-0000-1000-8000-00805f9b34fb";

/// Fails a test instead of hanging forever.
pub async fn timeout<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

// ---------------------------------------------------------------------------
// Private bus
// ---------------------------------------------------------------------------

pub struct TestBus {
    child: Child,
    dir: std::path::PathBuf,
    pub address: String,
}

impl TestBus {
    pub fn start() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bluebus-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                dir.display()
            ),
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .arg("--nofork")
            .arg("--print-address=1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon must be installed to run the integration tests");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Self {
            child,
            dir,
            address: line.trim().to_owned(),
        }
    }

    pub async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// Errors (org.bluez.Error.*)
// ---------------------------------------------------------------------------

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum BluezError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotReady(String),
    Failed(String),
    DoesNotExist(String),
    AlreadyExists(String),
    InvalidArguments(String),
    NotAvailable(String),
    AuthenticationRejected(String),
    InProgress(String),
}

// ---------------------------------------------------------------------------
// Shared mock state (inspected by the tests)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct State {
    /// (owner, path, capability)
    pub agents: Vec<(OwnedUniqueName, OwnedObjectPath, String)>,
    pub default_agent: Option<(OwnedUniqueName, OwnedObjectPath)>,
    /// (owner, app path) → objects read through GetManagedObjects
    pub gatt_apps: HashMap<(String, String), bluebus::ObjectsMap>,
    /// (owner, adv path) → properties read through GetAll
    pub advertisements: HashMap<(String, String), HashMap<String, OwnedValue>>,
    pub discovery_filter: Option<HashMap<String, OwnedValue>>,
    pub pair_requests: Vec<String>,
}

pub type SharedState = Arc<Mutex<State>>;

fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    v.into().try_to_owned().unwrap()
}

// ---------------------------------------------------------------------------
// org.bluez.AgentManager1 at /org/bluez
// ---------------------------------------------------------------------------

pub struct MockAgentManager {
    pub state: SharedState,
}

#[interface(name = "org.bluez.AgentManager1")]
impl MockAgentManager {
    fn register_agent(
        &self,
        agent: OwnedObjectPath,
        capability: String,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), BluezError> {
        const CAPS: [&str; 6] = [
            "DisplayOnly",
            "DisplayYesNo",
            "KeyboardOnly",
            "NoInputNoOutput",
            "KeyboardDisplay",
            "",
        ];
        if !CAPS.contains(&capability.as_str()) {
            return Err(BluezError::InvalidArguments(format!(
                "invalid capability {capability}"
            )));
        }
        let owner: OwnedUniqueName = hdr.sender().unwrap().to_owned().into();
        let mut st = self.state.lock().unwrap();
        if st.agents.iter().any(|(o, _, _)| *o == owner) {
            return Err(BluezError::AlreadyExists("agent already registered".into()));
        }
        st.agents.push((owner, agent, capability));
        Ok(())
    }

    fn unregister_agent(
        &self,
        agent: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), BluezError> {
        let owner = hdr.sender().unwrap().to_string();
        let mut st = self.state.lock().unwrap();
        let before = st.agents.len();
        st.agents
            .retain(|(o, p, _)| !(o.as_str() == owner && *p == agent));
        if st.agents.len() == before {
            return Err(BluezError::DoesNotExist("no such agent".into()));
        }
        if st
            .default_agent
            .as_ref()
            .is_some_and(|(o, p)| o.as_str() == owner && *p == agent)
        {
            st.default_agent = None;
        }
        Ok(())
    }

    fn request_default_agent(
        &self,
        agent: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), BluezError> {
        let owner = hdr.sender().unwrap().to_string();
        let mut st = self.state.lock().unwrap();
        let found = st
            .agents
            .iter()
            .find(|(o, p, _)| o.as_str() == owner && *p == agent)
            .map(|(o, p, _)| (o.clone(), p.clone()));
        match found {
            Some(a) => {
                st.default_agent = Some(a);
                Ok(())
            }
            None => Err(BluezError::DoesNotExist("agent not registered".into())),
        }
    }
}

// ---------------------------------------------------------------------------
// org.bluez.Adapter1 at /org/bluez/hciN
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DeviceSpec {
    pub address: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
}

pub struct MockAdapter {
    pub path: String,
    pub address: String,
    pub alias: String,
    pub powered: bool,
    pub discoverable: bool,
    pub discoverable_timeout: u32,
    pub pairable: bool,
    pub pairable_timeout: u32,
    pub discovering: bool,
    /// Devices "found" by StartDiscovery.
    pub nearby: Vec<DeviceSpec>,
    pub state: SharedState,
}

#[interface(name = "org.bluez.Adapter1")]
impl MockAdapter {
    async fn start_discovery(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if !self.powered {
            return Err(BluezError::NotReady("Resource Not Ready".into()));
        }
        if self.discovering {
            return Err(BluezError::InProgress(
                "Operation already in progress".into(),
            ));
        }
        self.discovering = true;
        self.discovering_changed(&emitter).await?;
        for spec in self.nearby.clone() {
            add_device(server, &self.path, &spec, self.state.clone()).await;
        }
        Ok(())
    }

    async fn stop_discovery(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if !self.discovering {
            return Err(BluezError::Failed("No discovery started".into()));
        }
        self.discovering = false;
        self.discovering_changed(&emitter).await?;
        Ok(())
    }

    async fn remove_device(
        &self,
        device: OwnedObjectPath,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(), BluezError> {
        if !device.as_str().starts_with(&format!("{}/dev_", self.path)) {
            return Err(BluezError::DoesNotExist("Does Not Exist".into()));
        }
        // Children first, like BlueZ.
        for child in [
            "service0001/char0002/desc0003",
            "service0001/char0002",
            "service0001",
        ] {
            let p = format!("{}/{child}", device.as_str());
            let _ = server.remove::<MockRemoteDescriptor, _>(p.as_str()).await;
            let _ = server
                .remove::<MockRemoteCharacteristic, _>(p.as_str())
                .await;
            let _ = server.remove::<MockRemoteService, _>(p.as_str()).await;
        }
        let _ = server.remove::<MockBattery, _>(device.as_str()).await;
        match server.remove::<MockDevice, _>(device.as_str()).await {
            Ok(true) => Ok(()),
            _ => Err(BluezError::DoesNotExist("Does Not Exist".into())),
        }
    }

    fn set_discovery_filter(&self, filter: HashMap<String, OwnedValue>) -> Result<(), BluezError> {
        let expected: HashMap<&str, &str> = HashMap::from([
            ("UUIDs", "as"),
            ("RSSI", "n"),
            ("Pathloss", "q"),
            ("Transport", "s"),
            ("DuplicateData", "b"),
            ("Discoverable", "b"),
            ("Pattern", "s"),
        ]);
        for (k, v) in &filter {
            match expected.get(k.as_str()) {
                Some(sig) if v.value_signature().to_string() == *sig => {}
                _ => {
                    return Err(BluezError::InvalidArguments(format!(
                        "bad filter key {k}: {}",
                        v.value_signature()
                    )))
                }
            }
        }
        if let Some(t) = filter.get("Transport") {
            let t: String = t.try_clone().unwrap().try_into().unwrap();
            if !["auto", "bredr", "le"].contains(&t.as_str()) {
                return Err(BluezError::InvalidArguments(format!("bad transport {t}")));
            }
        }
        self.state.lock().unwrap().discovery_filter = Some(filter);
        Ok(())
    }

    fn get_discovery_filters(&self) -> Vec<String> {
        [
            "UUIDs",
            "RSSI",
            "Pathloss",
            "Transport",
            "DuplicateData",
            "Discoverable",
            "Pattern",
        ]
        .map(String::from)
        .to_vec()
    }

    #[zbus(property)]
    fn address(&self) -> String {
        self.address.clone()
    }
    #[zbus(property)]
    fn address_type(&self) -> String {
        "public".into()
    }
    #[zbus(property)]
    fn name(&self) -> String {
        "mockhost".into()
    }
    #[zbus(property)]
    fn alias(&self) -> String {
        self.alias.clone()
    }
    #[zbus(property)]
    fn set_alias(&mut self, alias: String) {
        self.alias = if alias.is_empty() {
            "mockhost".into()
        } else {
            alias
        };
    }
    #[zbus(property)]
    fn class(&self) -> u32 {
        0x7c010c
    }
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.powered
    }
    #[zbus(property)]
    fn set_powered(&mut self, powered: bool) {
        self.powered = powered;
        if !powered {
            self.discovering = false;
        }
    }
    #[zbus(property)]
    fn power_state(&self) -> String {
        if self.powered { "on" } else { "off" }.into()
    }
    #[zbus(property)]
    fn discoverable(&self) -> bool {
        self.discoverable
    }
    #[zbus(property)]
    fn set_discoverable(&mut self, v: bool) {
        self.discoverable = v;
    }
    #[zbus(property)]
    fn discoverable_timeout(&self) -> u32 {
        self.discoverable_timeout
    }
    #[zbus(property)]
    fn set_discoverable_timeout(&mut self, v: u32) {
        self.discoverable_timeout = v;
    }
    #[zbus(property)]
    fn pairable(&self) -> bool {
        self.pairable
    }
    #[zbus(property)]
    fn set_pairable(&mut self, v: bool) {
        self.pairable = v;
    }
    #[zbus(property)]
    fn pairable_timeout(&self) -> u32 {
        self.pairable_timeout
    }
    #[zbus(property)]
    fn set_pairable_timeout(&mut self, v: u32) {
        self.pairable_timeout = v;
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.discovering
    }
    #[zbus(property, name = "UUIDs")]
    fn uuids(&self) -> Vec<String> {
        vec!["0000110e-0000-1000-8000-00805f9b34fb".into()]
    }
    #[zbus(property)]
    fn modalias(&self) -> String {
        "usb:v1D6Bp0246d0548".into()
    }
    #[zbus(property)]
    fn roles(&self) -> Vec<String> {
        vec!["central".into(), "peripheral".into()]
    }
    #[zbus(property)]
    fn experimental_features(&self) -> Vec<String> {
        vec![]
    }
    #[zbus(property)]
    fn manufacturer(&self) -> u16 {
        2
    }
    #[zbus(property)]
    fn version(&self) -> u8 {
        12
    }
}

// ---------------------------------------------------------------------------
// org.bluez.GattManager1 / LEAdvertisingManager1 at /org/bluez/hciN
// ---------------------------------------------------------------------------

pub struct MockGattManager {
    pub state: SharedState,
}

const CHR_FLAGS: [&str; 17] = [
    "broadcast",
    "read",
    "write-without-response",
    "write",
    "notify",
    "indicate",
    "authenticated-signed-writes",
    "extended-properties",
    "reliable-write",
    "writable-auxiliaries",
    "encrypt-read",
    "encrypt-write",
    "encrypt-authenticated-read",
    "encrypt-authenticated-write",
    "secure-read",
    "secure-write",
    "authorize",
];

fn str_prop(props: &HashMap<String, OwnedValue>, key: &str, sig: &str) -> Result<(), String> {
    match props.get(key) {
        Some(v) if v.value_signature().to_string() == sig => Ok(()),
        Some(v) => Err(format!(
            "{key} has signature {}, expected {sig}",
            v.value_signature()
        )),
        None => Err(format!("missing {key}")),
    }
}

fn path_prop(props: &HashMap<String, OwnedValue>, key: &str) -> Result<String, String> {
    str_prop(props, key, "o")?;
    Ok(props[key].downcast_ref::<ObjectPath>().unwrap().to_string())
}

/// Validation performed by BlueZ' gatt-database.c on a registered app.
fn validate_gatt_app(objects: &bluebus::ObjectsMap) -> Result<(), String> {
    let mut services = 0;
    let has = |path: &str, iface: &str| {
        objects
            .iter()
            .any(|(p, i)| p.as_str() == path && i.contains_key(iface))
    };
    for (path, ifaces) in objects {
        if let Some(p) = ifaces.get("org.bluez.GattService1") {
            services += 1;
            str_prop(p, "UUID", "s")?;
            str_prop(p, "Primary", "b")?;
            if p.contains_key("Includes") {
                str_prop(p, "Includes", "ao")?;
            }
        }
        if let Some(p) = ifaces.get("org.bluez.GattCharacteristic1") {
            str_prop(p, "UUID", "s")?;
            let svc = path_prop(p, "Service")?;
            if !has(&svc, "org.bluez.GattService1") {
                return Err(format!("{path}: Service {svc} not found"));
            }
            str_prop(p, "Flags", "as")?;
            let flags: Vec<String> = p["Flags"].try_clone().unwrap().try_into().unwrap();
            if flags.is_empty() || flags.iter().any(|f| !CHR_FLAGS.contains(&f.as_str())) {
                return Err(format!("{path}: invalid flags {flags:?}"));
            }
            if p.contains_key("WriteAcquired") || p.contains_key("NotifyAcquired") {
                return Err(format!("{path}: Acquire* advertised but not implemented"));
            }
        }
        if let Some(p) = ifaces.get("org.bluez.GattDescriptor1") {
            str_prop(p, "UUID", "s")?;
            let chr = path_prop(p, "Characteristic")?;
            if !has(&chr, "org.bluez.GattCharacteristic1") {
                return Err(format!("{path}: Characteristic {chr} not found"));
            }
            str_prop(p, "Flags", "as")?;
        }
    }
    if services == 0 {
        return Err("No object received".into());
    }
    Ok(())
}

#[interface(name = "org.bluez.GattManager1")]
impl MockGattManager {
    async fn register_application(
        &self,
        application: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> Result<(), BluezError> {
        let owner = hdr.sender().unwrap().to_string();
        let key = (owner.clone(), application.to_string());
        if self.state.lock().unwrap().gatt_apps.contains_key(&key) {
            return Err(BluezError::AlreadyExists("Already Exists".into()));
        }
        let om = fdo::ObjectManagerProxy::builder(conn)
            .destination(owner.clone())?
            .path(application.clone())?
            .build()
            .await?;
        let objects = om
            .get_managed_objects()
            .await
            .map_err(|e| BluezError::Failed(format!("GetManagedObjects failed: {e}")))?;
        let objects: bluebus::ObjectsMap = objects
            .into_iter()
            .map(|(p, i)| (p, i.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
            .collect();
        validate_gatt_app(&objects).map_err(BluezError::InvalidArguments)?;
        self.state.lock().unwrap().gatt_apps.insert(key, objects);
        Ok(())
    }

    fn unregister_application(
        &self,
        application: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), BluezError> {
        let key = (hdr.sender().unwrap().to_string(), application.to_string());
        match self.state.lock().unwrap().gatt_apps.remove(&key) {
            Some(_) => Ok(()),
            None => Err(BluezError::DoesNotExist("Does Not Exist".into())),
        }
    }
}

pub struct MockAdvertisingManager {
    pub state: SharedState,
}

/// Validation performed by BlueZ' advertising.c (parse_* functions).
fn validate_advertisement(props: &HashMap<String, OwnedValue>) -> Result<(), String> {
    let sigs: HashMap<&str, &str> = HashMap::from([
        ("Type", "s"),
        ("ServiceUUIDs", "as"),
        ("SolicitUUIDs", "as"),
        ("ManufacturerData", "a{qv}"),
        ("ServiceData", "a{sv}"),
        ("Data", "a{yv}"),
        ("Discoverable", "b"),
        ("DiscoverableTimeout", "q"),
        ("Includes", "as"),
        ("LocalName", "s"),
        ("Appearance", "q"),
        ("Duration", "q"),
        ("Timeout", "q"),
        ("SecondaryChannel", "s"),
        ("MinInterval", "u"),
        ("MaxInterval", "u"),
        ("TxPower", "n"),
    ]);
    for (k, v) in props {
        if let Some(sig) = sigs.get(k.as_str()) {
            if v.value_signature().to_string() != *sig {
                return Err(format!("{k}: signature {} != {sig}", v.value_signature()));
            }
        }
    }
    let s = |k: &str| {
        props
            .get(k)
            .map(|v| String::try_from(v.try_clone().unwrap()).unwrap())
    };
    match s("Type").as_deref() {
        Some("broadcast") | Some("peripheral") => {}
        other => return Err(format!("invalid Type {other:?}")),
    }
    if let Some(ch) = s("SecondaryChannel") {
        if !["1M", "2M", "Coded"].contains(&ch.as_str()) {
            return Err(format!("invalid SecondaryChannel {ch:?}"));
        }
    }
    if let Some(inc) = props.get("Includes") {
        let inc: Vec<String> = inc.try_clone().unwrap().try_into().unwrap();
        if inc
            .iter()
            .any(|i| !["tx-power", "appearance", "local-name", "rsi"].contains(&i.as_str()))
        {
            return Err(format!("invalid Includes {inc:?}"));
        }
    }
    let u = |k: &str| props.get(k).and_then(|v| v.downcast_ref::<u32>().ok());
    if let (Some(min), Some(max)) = (u("MinInterval"), u("MaxInterval")) {
        if min > max {
            return Err("MinInterval > MaxInterval".into());
        }
    }
    for (_, v) in props
        .get("ManufacturerData")
        .map(|m| HashMap::<u16, OwnedValue>::try_from(m.try_clone().unwrap()).unwrap())
        .unwrap_or_default()
    {
        if v.value_signature().to_string() != "ay" {
            return Err("ManufacturerData values must be ay".into());
        }
    }
    Ok(())
}

#[interface(name = "org.bluez.LEAdvertisingManager1")]
impl MockAdvertisingManager {
    async fn register_advertisement(
        &self,
        advertisement: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> Result<(), BluezError> {
        let owner = hdr.sender().unwrap().to_string();
        let key = (owner.clone(), advertisement.to_string());
        if self.state.lock().unwrap().advertisements.contains_key(&key) {
            return Err(BluezError::AlreadyExists("Already Exists".into()));
        }
        let props = fdo::PropertiesProxy::builder(conn)
            .destination(owner.clone())?
            .path(advertisement.clone())?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let all = props
            .get_all(InterfaceName::from_static_str_unchecked(
                "org.bluez.LEAdvertisement1",
            ))
            .await
            .map_err(|e| BluezError::Failed(format!("GetAll failed: {e}")))?;
        validate_advertisement(&all).map_err(|e| {
            BluezError::InvalidArguments(format!("Failed to parse advertisement: {e}"))
        })?;
        self.state.lock().unwrap().advertisements.insert(key, all);
        Ok(())
    }

    fn unregister_advertisement(
        &self,
        advertisement: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), BluezError> {
        let key = (hdr.sender().unwrap().to_string(), advertisement.to_string());
        match self.state.lock().unwrap().advertisements.remove(&key) {
            Some(_) => Ok(()),
            None => Err(BluezError::DoesNotExist("Does Not Exist".into())),
        }
    }

    #[zbus(property)]
    fn active_instances(&self) -> u8 {
        self.state.lock().unwrap().advertisements.len() as u8
    }
    #[zbus(property)]
    fn supported_instances(&self) -> u8 {
        5 - self.state.lock().unwrap().advertisements.len() as u8
    }
    #[zbus(property)]
    fn supported_includes(&self) -> Vec<String> {
        vec!["tx-power".into(), "appearance".into(), "local-name".into()]
    }
    #[zbus(property)]
    fn supported_secondary_channels(&self) -> Vec<String> {
        vec!["1M".into(), "2M".into(), "Coded".into()]
    }
    #[zbus(property)]
    fn supported_features(&self) -> Vec<String> {
        vec!["CanSetTxPower".into()]
    }
}

// ---------------------------------------------------------------------------
// Devices: org.bluez.Device1 + Battery1, and their remote GATT database
// ---------------------------------------------------------------------------

pub struct MockDevice {
    pub path: String,
    pub adapter: String,
    pub spec: DeviceSpec,
    pub alias: Option<String>,
    pub connected: bool,
    pub paired: bool,
    pub trusted: bool,
    pub blocked: bool,
    pub wake_allowed: bool,
    pub services_resolved: bool,
    pub state: SharedState,
}

pub async fn add_device(
    server: &ObjectServer,
    adapter: &str,
    spec: &DeviceSpec,
    state: SharedState,
) -> String {
    let path = bluebus::device_path(adapter, &spec.address);
    let dev = MockDevice {
        path: path.clone(),
        adapter: adapter.to_owned(),
        spec: spec.clone(),
        alias: None,
        connected: false,
        paired: false,
        trusted: false,
        blocked: false,
        wake_allowed: false,
        services_resolved: false,
        state,
    };
    server.at(path.as_str(), dev).await.unwrap();
    path
}

#[interface(name = "org.bluez.Device1")]
impl MockDevice {
    async fn connect(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if self.blocked {
            return Err(BluezError::Failed("device is blocked".into()));
        }
        if self.connected {
            return Err(BluezError::AlreadyExists("Already Connected".into()));
        }
        self.connected = true;
        self.connected_changed(&emitter).await?;
        // Resolve the remote GATT database (Battery Service).
        let svc = format!("{}/service0001", self.path);
        let chr = format!("{svc}/char0002");
        let desc = format!("{chr}/desc0003");
        server
            .at(
                svc.as_str(),
                MockRemoteService {
                    device: self.path.clone(),
                },
            )
            .await?;
        server
            .at(
                chr.as_str(),
                MockRemoteCharacteristic {
                    service: svc.clone(),
                    value: vec![87],
                    notifying: false,
                },
            )
            .await?;
        server
            .at(
                desc.as_str(),
                MockRemoteDescriptor {
                    characteristic: chr.clone(),
                    value: vec![0, 0],
                },
            )
            .await?;
        server
            .at(self.path.as_str(), MockBattery { percentage: 87 })
            .await?;
        self.services_resolved = true;
        self.services_resolved_changed(&emitter).await?;
        Ok(())
    }

    async fn disconnect(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if !self.connected {
            return Err(BluezError::Failed("Not Connected".into()));
        }
        self.connected = false;
        self.services_resolved = false;
        self.connected_changed(&emitter).await?;
        self.services_resolved_changed(&emitter).await?;
        Ok(())
    }

    fn connect_profile(&self, uuid: String) -> Result<(), BluezError> {
        if !self.connected {
            return Err(BluezError::Failed("Not Connected".into()));
        }
        if !self.uuids().contains(&uuid.to_lowercase()) {
            return Err(BluezError::NotAvailable(
                "Operation currently not available".into(),
            ));
        }
        Ok(())
    }

    fn disconnect_profile(&self, uuid: String) -> Result<(), BluezError> {
        self.connect_profile(uuid)
    }

    /// Simulates numeric comparison: asks the default agent to confirm 123456.
    async fn pair(
        &mut self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if self.paired {
            return Err(BluezError::AlreadyExists("Already Exists".into()));
        }
        let agent = self.state.lock().unwrap().default_agent.clone();
        if let Some((owner, path)) = agent {
            let proxy =
                zbus::Proxy::new(conn, owner.as_str(), path.as_str(), "org.bluez.Agent1").await?;
            let device = OwnedObjectPath::try_from(self.path.clone()).unwrap();
            let reply: zbus::Result<()> = proxy
                .call("RequestConfirmation", &(device, 123456u32))
                .await;
            self.state
                .lock()
                .unwrap()
                .pair_requests
                .push(self.path.clone());
            if let Err(e) = reply {
                return Err(BluezError::AuthenticationRejected(format!(
                    "agent refused: {e}"
                )));
            }
        }
        self.paired = true;
        self.paired_changed(&emitter).await?;
        Ok(())
    }

    fn cancel_pairing(&self) -> Result<(), BluezError> {
        Err(BluezError::DoesNotExist("no pairing in progress".into()))
    }

    #[zbus(property)]
    fn address(&self) -> String {
        self.spec.address.clone()
    }
    #[zbus(property)]
    fn address_type(&self) -> String {
        "public".into()
    }
    #[zbus(property)]
    fn name(&self) -> fdo::Result<String> {
        self.spec
            .name
            .clone()
            .ok_or_else(|| fdo::Error::UnknownProperty("Name".into()))
    }
    #[zbus(property)]
    fn alias(&self) -> String {
        self.alias
            .clone()
            .or_else(|| self.spec.name.clone())
            .unwrap_or_else(|| self.spec.address.replace(':', "-"))
    }
    #[zbus(property)]
    fn set_alias(&mut self, alias: String) {
        self.alias = (!alias.is_empty()).then_some(alias);
    }
    #[zbus(property)]
    fn adapter(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(self.adapter.clone()).unwrap()
    }
    #[zbus(property)]
    fn class(&self) -> u32 {
        0x240404
    }
    #[zbus(property)]
    fn appearance(&self) -> u16 {
        0x0941
    }
    #[zbus(property)]
    fn icon(&self) -> String {
        "audio-headset".into()
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.connected
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        self.paired
    }
    #[zbus(property)]
    fn bonded(&self) -> bool {
        self.paired
    }
    #[zbus(property)]
    fn trusted(&self) -> bool {
        self.trusted
    }
    #[zbus(property)]
    fn set_trusted(&mut self, v: bool) {
        self.trusted = v;
    }
    #[zbus(property)]
    fn blocked(&self) -> bool {
        self.blocked
    }
    #[zbus(property)]
    fn set_blocked(&mut self, v: bool) {
        self.blocked = v;
    }
    #[zbus(property)]
    fn wake_allowed(&self) -> bool {
        self.wake_allowed
    }
    #[zbus(property)]
    fn set_wake_allowed(&mut self, v: bool) {
        self.wake_allowed = v;
    }
    #[zbus(property)]
    fn legacy_pairing(&self) -> bool {
        false
    }
    #[zbus(property, name = "UUIDs")]
    fn uuids(&self) -> Vec<String> {
        vec![
            "0000110b-0000-1000-8000-00805f9b34fb".into(),
            SVC_UUID.into(),
        ]
    }
    #[zbus(property)]
    fn modalias(&self) -> String {
        "bluetooth:v004Cp0001d0001".into()
    }
    #[zbus(property, name = "RSSI")]
    fn rssi(&self) -> fdo::Result<i16> {
        self.spec
            .rssi
            .ok_or_else(|| fdo::Error::UnknownProperty("RSSI".into()))
    }
    #[zbus(property)]
    fn tx_power(&self) -> i16 {
        4
    }
    #[zbus(property)]
    fn manufacturer_data(&self) -> HashMap<u16, OwnedValue> {
        HashMap::from([(0x004C, ov(vec![0x02u8, 0x15]))])
    }
    #[zbus(property)]
    fn service_data(&self) -> HashMap<String, OwnedValue> {
        HashMap::from([(SVC_UUID.to_string(), ov(vec![0x57u8]))])
    }
    #[zbus(property)]
    fn advertising_flags(&self) -> Vec<u8> {
        vec![0x06]
    }
    #[zbus(property)]
    fn advertising_data(&self) -> HashMap<u8, OwnedValue> {
        HashMap::from([(0xFF, ov(vec![0x4Cu8, 0x00]))])
    }
    #[zbus(property)]
    fn services_resolved(&self) -> bool {
        self.services_resolved
    }
}

pub struct MockBattery {
    pub percentage: u8,
}

#[interface(name = "org.bluez.Battery1")]
impl MockBattery {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        self.percentage
    }
    #[zbus(property)]
    fn source(&self) -> String {
        "GATT".into()
    }
}

pub struct MockRemoteService {
    pub device: String,
}

#[interface(name = "org.bluez.GattService1")]
impl MockRemoteService {
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        SVC_UUID.into()
    }
    #[zbus(property)]
    fn primary(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn device(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(self.device.clone()).unwrap()
    }
    #[zbus(property)]
    fn includes(&self) -> Vec<OwnedObjectPath> {
        vec![]
    }
    #[zbus(property)]
    fn handle(&self) -> u16 {
        1
    }
}

pub struct MockRemoteCharacteristic {
    pub service: String,
    pub value: Vec<u8>,
    pub notifying: bool,
}

fn offset(options: &HashMap<String, OwnedValue>) -> usize {
    options
        .get("offset")
        .and_then(|v| v.downcast_ref::<u16>().ok())
        .unwrap_or(0) as usize
}

#[interface(name = "org.bluez.GattCharacteristic1")]
impl MockRemoteCharacteristic {
    fn read_value(&self, options: HashMap<String, OwnedValue>) -> Result<Vec<u8>, BluezError> {
        self.value
            .get(offset(&options)..)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| BluezError::Failed("Invalid offset".into()))
    }

    async fn write_value(
        &mut self,
        value: Vec<u8>,
        options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if let Some(t) = options.get("type") {
            let t: String = t.try_clone().unwrap().try_into().unwrap();
            if !["command", "request", "reliable"].contains(&t.as_str()) {
                return Err(BluezError::InvalidArguments(format!("bad write type {t}")));
            }
        }
        self.value = value;
        self.value_changed(&emitter).await?;
        Ok(())
    }

    async fn start_notify(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        if self.notifying {
            return Err(BluezError::InProgress("Already notifying".into()));
        }
        self.notifying = true;
        self.notifying_changed(&emitter).await?;
        Ok(())
    }

    async fn stop_notify(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        self.notifying = false;
        self.notifying_changed(&emitter).await?;
        Ok(())
    }

    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        CHR_UUID.into()
    }
    #[zbus(property)]
    fn service(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(self.service.clone()).unwrap()
    }
    #[zbus(property)]
    fn value(&self) -> Vec<u8> {
        self.value.clone()
    }
    #[zbus(property)]
    fn notifying(&self) -> bool {
        self.notifying
    }
    #[zbus(property)]
    fn flags(&self) -> Vec<String> {
        vec!["read".into(), "write".into(), "notify".into()]
    }
    #[zbus(property)]
    fn handle(&self) -> u16 {
        2
    }
    #[zbus(property, name = "MTU")]
    fn mtu(&self) -> u16 {
        247
    }
}

/// Simulates the remote device sending a notification.
pub async fn remote_notify(server: &Connection, chr_path: &str, value: Vec<u8>) {
    let iface = server
        .object_server()
        .interface::<_, MockRemoteCharacteristic>(chr_path)
        .await
        .unwrap();
    iface.get_mut().await.value = value;
    iface
        .get()
        .await
        .value_changed(iface.signal_emitter())
        .await
        .unwrap();
}

pub struct MockRemoteDescriptor {
    pub characteristic: String,
    pub value: Vec<u8>,
}

#[interface(name = "org.bluez.GattDescriptor1")]
impl MockRemoteDescriptor {
    fn read_value(&self, _options: HashMap<String, OwnedValue>) -> Vec<u8> {
        self.value.clone()
    }
    fn write_value(&mut self, value: Vec<u8>, _options: HashMap<String, OwnedValue>) {
        self.value = value;
    }
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        DESC_UUID.into()
    }
    #[zbus(property)]
    fn characteristic(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(self.characteristic.clone()).unwrap()
    }
    #[zbus(property)]
    fn value(&self) -> Vec<u8> {
        self.value.clone()
    }
    #[zbus(property)]
    fn flags(&self) -> Vec<String> {
        vec!["read".into(), "write".into()]
    }
    #[zbus(property)]
    fn handle(&self) -> u16 {
        3
    }
}

// ---------------------------------------------------------------------------
// Assembling the mock daemon
// ---------------------------------------------------------------------------

pub struct MockBluez {
    pub bus: TestBus,
    /// Connection owning `org.bluez`.
    pub server: Connection,
    pub state: SharedState,
}

pub fn adapter(
    path: &str,
    address: &str,
    nearby: Vec<DeviceSpec>,
    state: SharedState,
) -> MockAdapter {
    MockAdapter {
        path: path.into(),
        address: address.into(),
        alias: "mockhost".into(),
        powered: false,
        discoverable: false,
        discoverable_timeout: 180,
        pairable: true,
        pairable_timeout: 0,
        discovering: false,
        nearby,
        state,
    }
}

pub fn spec(address: &str, name: Option<&str>, rssi: Option<i16>) -> DeviceSpec {
    DeviceSpec {
        address: address.into(),
        name: name.map(Into::into),
        rssi,
    }
}

impl MockBluez {
    /// hci0 (powered off) finds two devices; hci1 has one known device.
    pub async fn start() -> Self {
        let bus = TestBus::start();
        let state = SharedState::default();
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name("org.bluez")
            .unwrap()
            .serve_at("/", fdo::ObjectManager)
            .unwrap()
            .serve_at(
                "/org/bluez",
                MockAgentManager {
                    state: state.clone(),
                },
            )
            .unwrap()
            .serve_at(
                ADAPTER0,
                adapter(
                    ADAPTER0,
                    "00:AA:00:AA:00:AA",
                    vec![
                        spec("11:22:33:44:55:66", Some("Headset"), Some(-48)),
                        spec("AA:BB:CC:DD:EE:FF", None, Some(-90)),
                    ],
                    state.clone(),
                ),
            )
            .unwrap()
            .serve_at(
                ADAPTER0,
                MockGattManager {
                    state: state.clone(),
                },
            )
            .unwrap()
            .serve_at(
                ADAPTER0,
                MockAdvertisingManager {
                    state: state.clone(),
                },
            )
            .unwrap()
            .serve_at(
                ADAPTER1,
                adapter(ADAPTER1, "00:BB:00:BB:00:BB", vec![], state.clone()),
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        add_device(
            server.object_server(),
            ADAPTER1,
            &spec("01:02:03:04:05:06", Some("Other"), None),
            state.clone(),
        )
        .await;
        Self { bus, server, state }
    }

    pub async fn client(&self) -> Connection {
        self.bus.connect().await
    }
}
