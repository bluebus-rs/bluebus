use zbus::zvariant::OwnedObjectPath;

use super::{AgentHandler, AgentResult};

/// A pairing agent that accepts every confirmation and authorization
/// request ("Just Works" and numeric comparison without asking). PIN code
/// and passkey requests are rejected.
///
/// Register it with [`Session::set_agent`](super::Session::set_agent) and
/// [`AgentCapability::NoInputNoOutput`](super::AgentCapability::NoInputNoOutput).
///
/// ```no_run
/// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
/// use bluebus::ble::{AgentCapability, AutoAcceptAgent};
/// session.set_agent(AgentCapability::NoInputNoOutput, AutoAcceptAgent).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoAcceptAgent;

impl AgentHandler for AutoAcceptAgent {}

/// A pairing agent answering with a fixed passkey and PIN code; it also
/// accepts every confirmation and authorization request.
///
/// ```no_run
/// # async fn f(session: bluebus::ble::Session) -> bluebus::ble::Result<()> {
/// use bluebus::ble::{AgentCapability, PasskeyAgent};
/// let agent = PasskeyAgent::new(123456).with_pin_code("0000");
/// session.set_agent(AgentCapability::KeyboardDisplay, agent).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyAgent {
    passkey: u32,
    pin_code: String,
}

impl PasskeyAgent {
    /// An agent answering passkey requests with `passkey` (0–999999) and
    /// legacy PIN code requests with the same digits (`"123456"`).
    ///
    /// ```
    /// let agent = bluebus::ble::PasskeyAgent::new(42);
    /// assert_eq!(agent.passkey(), 42);
    /// assert_eq!(agent.pin_code(), "000042");
    /// ```
    pub fn new(passkey: u32) -> Self {
        PasskeyAgent {
            passkey,
            pin_code: format!("{passkey:06}"),
        }
    }

    /// Uses another PIN code for legacy (pre-2.1) pairing.
    ///
    /// ```
    /// let agent = bluebus::ble::PasskeyAgent::new(1).with_pin_code("1234");
    /// assert_eq!(agent.pin_code(), "1234");
    /// ```
    pub fn with_pin_code(mut self, pin_code: impl Into<String>) -> Self {
        self.pin_code = pin_code.into();
        self
    }

    /// The passkey given to BlueZ.
    ///
    /// ```
    /// assert_eq!(bluebus::ble::PasskeyAgent::new(7).passkey(), 7);
    /// ```
    pub fn passkey(&self) -> u32 {
        self.passkey
    }

    /// The PIN code given to BlueZ.
    ///
    /// ```
    /// assert_eq!(bluebus::ble::PasskeyAgent::new(7).pin_code(), "000007");
    /// ```
    pub fn pin_code(&self) -> &str {
        &self.pin_code
    }
}

impl AgentHandler for PasskeyAgent {
    async fn request_pin_code(&self, _device: OwnedObjectPath) -> AgentResult<String> {
        Ok(self.pin_code.clone())
    }

    async fn request_passkey(&self, _device: OwnedObjectPath) -> AgentResult<u32> {
        Ok(self.passkey)
    }
}
