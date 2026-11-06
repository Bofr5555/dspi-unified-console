//! The hub's typed errors.

/// Why a hub operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HubError {
    /// The hub has been stopped, or is stopping.
    #[error("the hub has stopped")]
    Stopped,
    /// No device with that serial or handle is online.
    #[error("no such device on the hub")]
    NoDevice,
    /// A device with this serial is already attached.
    #[error("device {0} is already attached")]
    AlreadyAttached(String),
    /// All 255 handles are taken (spec 2: 0..254).
    #[error("every device handle is in use")]
    NoHandle,
    /// The device did not answer its identity reads.
    #[error("the device did not identify itself: {0}")]
    Identity(String),
    /// This hub already has an app attached; it has one local session.
    #[error("an app is already attached to this hub")]
    AppAttached,
    /// A thread or a socket could not be set up.
    #[error("{0}")]
    Io(String),
}
