#[zbus::proxy(default_service = "org.bluez", interface = "org.bluez.AgentManager1")]
pub trait AgentManager {
    fn register_agent(
        &self,
        path: &zbus::zvariant::OwnedObjectPath,
        capability: String,
    ) -> zbus::Result<()>;
    fn unregister_agent(&self, path: &zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;
    fn request_default_agent(&self, path: &zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;
}
