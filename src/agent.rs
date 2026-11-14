//! Pairing agents (`org.bluez.Agent1`).
//!
//! BlueZ asks a registered agent for PIN codes, passkeys and confirmations
//! while pairing. Implement [`AgentHandler`] (every method has a default)
//! and register it with [`register_agent`]:
//!
//! ```no_run
//! # async fn f() -> zbus::Result<()> {
//! use bluebus::*;
//! struct AutoAccept;
//! impl AgentHandler for AutoAccept {}
//!
//! let conn = get_system_connection().await?;
//! register_agent(&conn, "/bluebus/agent", AgentCapability::NoInputNoOutput, AutoAccept, true).await?;
//! # Ok(()) }
//! ```
//!
//! The legacy [`Agent`] struct (fixed PIN/passkey, accepts every request)
//! implements [`AgentHandler`] too.

use std::future::Future;
use std::sync::Arc;

use zbus::zvariant::{ObjectPath, OwnedObjectPath};
use zbus::{fdo, interface};

/// Errors an agent returns to BlueZ.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// The user rejected the request (`org.bluez.Error.Rejected`).
    Rejected(String),
    /// The request was canceled (`org.bluez.Error.Canceled`).
    Canceled(String),
}

impl From<fdo::Error> for AgentError {
    fn from(e: fdo::Error) -> Self {
        AgentError::ZBus(e.into())
    }
}

/// Result type of agent callbacks.
pub type AgentResult<T> = Result<T, AgentError>;

/// Input/output capability announced to BlueZ; it selects the pairing method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentCapability {
    DisplayOnly,
    DisplayYesNo,
    KeyboardOnly,
    #[default]
    NoInputNoOutput,
    KeyboardDisplay,
}

impl AgentCapability {
    /// The string BlueZ expects.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentCapability::DisplayOnly => "DisplayOnly",
            AgentCapability::DisplayYesNo => "DisplayYesNo",
            AgentCapability::KeyboardOnly => "KeyboardOnly",
            AgentCapability::NoInputNoOutput => "NoInputNoOutput",
            AgentCapability::KeyboardDisplay => "KeyboardDisplay",
        }
    }
}

impl std::fmt::Display for AgentCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Callbacks of a pairing agent. The defaults reject requests that need an
/// answer from the user (PIN, passkey) and accept confirmations and
/// authorizations, which is the "Just Works" behavior.
pub trait AgentHandler: Send + Sync + 'static {
    /// BlueZ unregistered the agent.
    fn release(&self) -> impl Future<Output = AgentResult<()>> + Send {
        async { Ok(()) }
    }

    /// Return the PIN code (1–16 alphanumeric characters) for legacy pairing.
    fn request_pin_code(
        &self,
        device: OwnedObjectPath,
    ) -> impl Future<Output = AgentResult<String>> + Send {
        let _ = device;
        async { Err(AgentError::Rejected("PIN code not available".into())) }
    }

    /// Show `pincode` to the user (to be typed on the remote device).
    fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
    ) -> impl Future<Output = AgentResult<()>> + Send {
        let _ = (device, pincode);
        async { Ok(()) }
    }

    /// Return the 6-digit passkey (0–999999).
    fn request_passkey(
        &self,
        device: OwnedObjectPath,
    ) -> impl Future<Output = AgentResult<u32>> + Send {
        let _ = device;
        async { Err(AgentError::Rejected("passkey not available".into())) }
    }

    /// Show `passkey` to the user; `entered` is the number of typed digits.
    fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        entered: u16,
    ) -> impl Future<Output = AgentResult<()>> + Send {
        let _ = (device, passkey, entered);
        async { Ok(()) }
    }

    /// Confirm that `passkey` matches the one shown on the remote device.
    /// Return `Err(AgentError::Rejected(..))` to refuse.
    fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> impl Future<Output = AgentResult<()>> + Send {
        let _ = (device, passkey);
        async { Ok(()) }
    }

    /// Authorize an incoming pairing request ("Just Works").
    fn request_authorization(
        &self,
        device: OwnedObjectPath,
    ) -> impl Future<Output = AgentResult<()>> + Send {
        let _ = device;
        async { Ok(()) }
    }

    /// Authorize a connection to the service `uuid`.
    fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
    ) -> impl Future<Output = AgentResult<()>> + Send {
        let _ = (device, uuid);
        async { Ok(()) }
    }

    /// The pending request was canceled by BlueZ.
    fn cancel(&self) -> impl Future<Output = AgentResult<()>> + Send {
        async { Ok(()) }
    }
}

/// D-Bus adapter that exports an [`AgentHandler`] as `org.bluez.Agent1`.
///
/// zbus dispatches incoming calls on its own executor. When created inside
/// a tokio runtime, the handler futures are run on that runtime, so
/// handlers may freely use tokio APIs (timers, channels, `spawn`).
pub struct AgentService<H: AgentHandler> {
    handler: Arc<H>,
    runtime: Option<tokio::runtime::Handle>,
}

impl<H: AgentHandler> AgentService<H> {
    /// Wraps a handler, capturing the current tokio runtime if there is one.
    pub fn new(handler: H) -> Self {
        Self {
            handler: Arc::new(handler),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    /// The wrapped handler.
    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Runs a handler call on the captured runtime (or inline without one).
    async fn dispatch<T, F, Fut>(&self, call: F) -> Result<T, AgentError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<H>) -> Fut,
        Fut: Future<Output = Result<T, AgentError>> + Send + 'static,
    {
        let fut = call(self.handler.clone());
        match &self.runtime {
            Some(rt) => rt.spawn(fut).await.map_err(|e| {
                AgentError::ZBus(zbus::Error::Failure(format!("agent handler panicked: {e}")))
            })?,
            None => fut.await,
        }
    }
}

#[interface(name = "org.bluez.Agent1")]
impl<H: AgentHandler> AgentService<H> {
    async fn release(&self) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.release().await }).await
    }

    async fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, AgentError> {
        self.dispatch(|h| async move { h.request_pin_code(device).await })
            .await
    }

    async fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
    ) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.display_pin_code(device, pincode).await })
            .await
    }

    async fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, AgentError> {
        self.dispatch(|h| async move { h.request_passkey(device).await })
            .await
    }

    async fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        entered: u16,
    ) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.display_passkey(device, passkey, entered).await })
            .await
    }

    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.request_confirmation(device, passkey).await })
            .await
    }

    async fn request_authorization(&self, device: OwnedObjectPath) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.request_authorization(device).await })
            .await
    }

    async fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
    ) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.authorize_service(device, uuid).await })
            .await
    }

    async fn cancel(&self) -> Result<(), AgentError> {
        self.dispatch(|h| async move { h.cancel().await }).await
    }
}

/// Exports `handler` at `path`, registers it with BlueZ and, if
/// `make_default`, makes it the default agent (needed to answer pairing
/// requests initiated by remote devices).
///
/// If BlueZ rejects the registration, the exported object is removed again
/// so the call can be retried.
pub async fn register_agent<H: AgentHandler>(
    conn: &zbus::Connection,
    path: &str,
    capability: AgentCapability,
    handler: H,
    make_default: bool,
) -> zbus::Result<()> {
    let object_path = OwnedObjectPath::from(ObjectPath::try_from(path)?);
    if !conn
        .object_server()
        .at(&object_path, AgentService::new(handler))
        .await?
    {
        return Err(zbus::Error::Failure(format!(
            "an agent is already exported at {path}"
        )));
    }
    let result = async {
        crate::object_server_ready::ensure_object_server_ready(conn).await?;
        let manager = crate::AgentManagerProxy::new(conn, crate::BLUEZ_ROOT_PATH).await?;
        manager
            .register_agent(&object_path, capability.as_str().to_owned())
            .await?;
        if make_default {
            if let Err(e) = manager.request_default_agent(&object_path).await {
                let _ = manager.unregister_agent(&object_path).await;
                return Err(e);
            }
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        remove_agent_object(conn, &object_path).await;
    }
    result
}

/// Unregisters the agent at `path` from BlueZ and removes the exported
/// object (also when BlueZ no longer knew the agent).
pub async fn unregister_agent(conn: &zbus::Connection, path: &str) -> zbus::Result<()> {
    let object_path = OwnedObjectPath::from(ObjectPath::try_from(path)?);
    let manager = crate::AgentManagerProxy::new(conn, crate::BLUEZ_ROOT_PATH).await?;
    let result = manager.unregister_agent(&object_path).await;
    remove_agent_object(conn, &object_path).await;
    result
}

async fn remove_agent_object(conn: &zbus::Connection, path: &ObjectPath<'_>) {
    // `remove` matches by interface name, so any handler type works here.
    let _ = conn
        .object_server()
        .remove::<AgentService<Agent>, _>(path)
        .await;
}

/// A simple agent with a fixed PIN code and passkey that accepts every
/// confirmation and authorization request.
///
/// Its methods are callable directly; to make BlueZ use it, register it:
/// `register_agent(&conn, path, capability, Agent::new(pin, passkey), true)`.
/// Pairing secrets are no longer printed to stdout (since 0.1.11); events
/// are logged at `debug` level through `tracing` without the secrets.
pub struct Agent {
    /// The PIN code used for pairing.
    pincode: String,
    /// The passkey used for authentication.
    passkey: u32,
}

impl Agent {
    /// Creates a new agent with a given PIN code and passkey.
    pub fn new(pincode: String, passkey: u32) -> Self {
        Self { pincode, passkey }
    }

    /// Releases the agent, indicating it is no longer in use.
    pub async fn release(&self) -> zbus::fdo::Result<()> {
        tracing::debug!("agent released");
        Ok(())
    }

    /// Returns the configured PIN code.
    pub async fn request_pin_code(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
    ) -> zbus::fdo::Result<String> {
        tracing::debug!(%device, "RequestPinCode");
        Ok(self.pincode.clone())
    }

    /// Called when a PIN code must be shown to the user.
    pub async fn display_pin_code(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
        pincode: &str,
    ) -> zbus::fdo::Result<()> {
        let _ = pincode;
        tracing::debug!(%device, "DisplayPinCode");
        Ok(())
    }

    /// Returns the configured passkey.
    pub async fn request_passkey(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
    ) -> zbus::fdo::Result<u32> {
        tracing::debug!(%device, "RequestPasskey");
        Ok(self.passkey)
    }

    /// Called when a passkey must be shown to the user.
    pub async fn display_passkey(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
        passkey: u32,
        entered: u16,
    ) -> zbus::fdo::Result<()> {
        let _ = passkey;
        tracing::debug!(%device, entered, "DisplayPasskey");
        Ok(())
    }

    /// Accepts the confirmation request.
    pub fn request_confirmation(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
        passkey: u32,
    ) -> zbus::fdo::Result<()> {
        let _ = passkey;
        tracing::debug!(%device, "RequestConfirmation");
        Ok(())
    }

    /// Accepts the authorization request.
    pub async fn request_authorization(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
    ) -> zbus::fdo::Result<()> {
        tracing::debug!(%device, "RequestAuthorization");
        Ok(())
    }

    /// Accepts the service authorization request.
    pub async fn authorize_service(
        &self,
        device: &zbus::zvariant::OwnedObjectPath,
        uuid: &str,
    ) -> zbus::fdo::Result<()> {
        tracing::debug!(%device, uuid, "AuthorizeService");
        Ok(())
    }

    /// Called when the pending request was canceled.
    pub async fn cancel(&self) -> zbus::fdo::Result<()> {
        tracing::debug!("Cancel");
        Ok(())
    }
}

impl AgentHandler for Agent {
    async fn release(&self) -> AgentResult<()> {
        Ok(Agent::release(self).await?)
    }
    async fn request_pin_code(&self, device: OwnedObjectPath) -> AgentResult<String> {
        Ok(Agent::request_pin_code(self, &device).await?)
    }
    async fn display_pin_code(&self, device: OwnedObjectPath, pincode: String) -> AgentResult<()> {
        Ok(Agent::display_pin_code(self, &device, &pincode).await?)
    }
    async fn request_passkey(&self, device: OwnedObjectPath) -> AgentResult<u32> {
        Ok(Agent::request_passkey(self, &device).await?)
    }
    async fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        entered: u16,
    ) -> AgentResult<()> {
        Ok(Agent::display_passkey(self, &device, passkey, entered).await?)
    }
    async fn request_confirmation(&self, device: OwnedObjectPath, passkey: u32) -> AgentResult<()> {
        Ok(Agent::request_confirmation(self, &device, passkey)?)
    }
    async fn request_authorization(&self, device: OwnedObjectPath) -> AgentResult<()> {
        Ok(Agent::request_authorization(self, &device).await?)
    }
    async fn authorize_service(&self, device: OwnedObjectPath, uuid: String) -> AgentResult<()> {
        Ok(Agent::authorize_service(self, &device, &uuid).await?)
    }
    async fn cancel(&self) -> AgentResult<()> {
        Ok(Agent::cancel(self).await?)
    }
}
