use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;

use super::{Adapter, AdapterInfo, AgentCapability, AgentHandler, Error, Result};

/// Entry point of the high-level API: a connection to BlueZ.
///
/// Cheap to clone (reference counted); every [`Adapter`] and
/// [`Device`](super::Device) keeps its session alive.
///
/// ```no_run
/// # async fn f() -> bluebus::ble::Result<()> {
/// use bluebus::ble::Session;
/// let session = Session::new().await?;
/// let adapter = session.default_adapter().await?;
/// println!("using {} ({})", adapter.name(), adapter.address());
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    conn: zbus::Connection,
    /// Object path of the agent registered through [`Session::set_agent`].
    agent: tokio::sync::Mutex<Option<String>>,
    /// Adapter path → task stopping a scan whose stream was dropped. A new
    /// scan waits for it, so the late `StopDiscovery` cannot cancel it.
    pending_scan_stops: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("unique_name", &self.inner.conn.unique_name())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Connects to BlueZ on the D-Bus system bus. The bus address can be
    /// overridden with the `DBUS_SYSTEM_BUS_ADDRESS` environment variable.
    ///
    /// ```no_run
    /// # async fn f() -> bluebus::ble::Result<()> {
    /// let session = bluebus::ble::Session::new().await?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::Bus`] if the system bus is unreachable.
    pub async fn new() -> Result<Self> {
        Ok(Self::with_connection(crate::get_system_connection().await?))
    }

    /// Wraps an existing D-Bus connection (for example a private bus in
    /// tests, or a connection shared with the low-level API).
    ///
    /// ```no_run
    /// # async fn f() -> bluebus::ble::Result<()> {
    /// let conn = bluebus::get_system_connection().await?;
    /// let session = bluebus::ble::Session::with_connection(conn);
    /// # Ok(()) }
    /// ```
    pub fn with_connection(conn: zbus::Connection) -> Self {
        Session {
            inner: Arc::new(SessionInner {
                conn,
                agent: tokio::sync::Mutex::new(None),
                pending_scan_stops: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The underlying D-Bus connection, to mix in the low-level API.
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session) -> zbus::Result<()> {
    /// let paths = bluebus::list_adapters(session.connection()).await?;
    /// # Ok(()) }
    /// ```
    pub fn connection(&self) -> &zbus::Connection {
        &self.inner.conn
    }

    /// Lists the local adapters, sorted by name (`hci0`, `hci1`, …).
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
    /// for adapter in session.adapters().await? {
    ///     println!("{} {}", adapter.name, adapter.address);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn adapters(&self) -> Result<Vec<AdapterInfo>> {
        let objects = crate::ObjectManagerProxy::new(&self.inner.conn)
            .await?
            .get_managed_objects()
            .await?;
        let mut adapters: Vec<AdapterInfo> = objects
            .iter()
            .filter_map(|(path, ifaces)| {
                let props = ifaces.get("org.bluez.Adapter1")?;
                Some(AdapterInfo::from_properties(path.as_str(), props))
            })
            .collect();
        adapters.sort_by(|a, b| (a.path.len(), &a.path).cmp(&(b.path.len(), &b.path)));
        Ok(adapters)
    }

    /// Opens an adapter by name (`"hci1"`), object path (`"/org/bluez/hci1"`)
    /// or MAC address. `None` selects the default adapter: `hci0` if present,
    /// else the first one.
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
    /// let hci1 = session.adapter(Some("hci1")).await?;
    /// let default = session.adapter(None).await?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// [`Error::NotFound`] if no adapter matches.
    pub async fn adapter(&self, name: Option<&str>) -> Result<Adapter> {
        let adapters = self.adapters().await?;
        let found = match name {
            None => adapters
                .iter()
                .find(|a| a.name == "hci0")
                .or_else(|| adapters.first()),
            Some(want) => adapters
                .iter()
                .find(|a| a.name == want || a.path == want || a.address.eq_ignore_ascii_case(want)),
        };
        match found {
            Some(info) => Ok(Adapter::new(self.clone(), info)),
            None => Err(Error::NotFound(match name {
                Some(n) => format!("Bluetooth adapter {n:?}"),
                None => "no Bluetooth adapter available".into(),
            })),
        }
    }

    /// The default adapter; same as `adapter(None)`.
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
    /// let adapter = session.default_adapter().await?;
    /// adapter.power_on().await?;
    /// # Ok(()) }
    /// ```
    pub async fn default_adapter(&self) -> Result<Adapter> {
        self.adapter(None).await
    }

    /// Registers `handler` as this session's pairing agent and makes it
    /// BlueZ' default agent, replacing an agent set earlier on this session.
    ///
    /// The agent answers pairing requests for [`Device::pair`](super::Device::pair)
    /// and for pairings initiated by remote devices. It stays registered
    /// until [`remove_agent`](Self::remove_agent) or until the connection
    /// closes.
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
    /// use bluebus::ble::{AgentCapability, AutoAcceptAgent, PasskeyAgent};
    /// session.set_agent(AgentCapability::NoInputNoOutput, AutoAcceptAgent).await?;
    /// // or answer passkey requests with a fixed value:
    /// session.set_agent(AgentCapability::KeyboardDisplay, PasskeyAgent::new(123456)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn set_agent<H: AgentHandler>(
        &self,
        capability: AgentCapability,
        handler: H,
    ) -> Result<()> {
        let mut agent = self.inner.agent.lock().await;
        if let Some(old) = agent.take() {
            let _ = crate::unregister_agent(&self.inner.conn, &old).await;
        }
        let path = super::unique_path("agent");
        crate::register_agent(&self.inner.conn, &path, capability, handler, true).await?;
        *agent = Some(path);
        Ok(())
    }

    /// Unregisters the agent set with [`set_agent`](Self::set_agent). Does
    /// nothing if none is set.
    ///
    /// ```no_run
    /// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
    /// session.remove_agent().await?;
    /// # Ok(()) }
    /// ```
    pub async fn remove_agent(&self) -> Result<()> {
        match self.inner.agent.lock().await.take() {
            Some(path) => Ok(crate::unregister_agent(&self.inner.conn, &path).await?),
            None => Ok(()),
        }
    }

    pub(crate) fn take_pending_scan_stop(&self, adapter_path: &str) -> Option<JoinHandle<()>> {
        self.inner
            .pending_scan_stops
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(adapter_path)
    }

    pub(crate) fn set_pending_scan_stop(&self, adapter_path: String, task: JoinHandle<()>) {
        self.inner
            .pending_scan_stops
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(adapter_path, task);
    }
}
