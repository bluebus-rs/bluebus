/// Proxy for `org.bluez.AgentManager1`, located at [`crate::BLUEZ_ROOT_PATH`]
/// (`AgentManagerProxy::new(&conn, "/org/bluez")`).
///
/// [`crate::register_agent`] wraps exporting an agent and registering it.
#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.AgentManager1")]
pub trait AgentManager {
    /// Registers the agent exported at `path`. `capability` is one of
    /// `DisplayOnly`, `DisplayYesNo`, `KeyboardOnly`, `NoInputNoOutput`,
    /// `KeyboardDisplay` (see [`crate::AgentCapability`]).
    fn register_agent(
        &self,
        path: &zbus::zvariant::OwnedObjectPath,
        capability: String,
    ) -> zbus::Result<()>;

    /// Unregisters the agent exported at `path`.
    fn unregister_agent(&self, path: &zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;

    /// Makes the agent at `path` the system default agent.
    fn request_default_agent(&self, path: &zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;
}
