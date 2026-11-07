//! The network front: WebSocket and HTTP on one port (spec 4). Owned by WP HB1.
//!
//! HUB.md section 2 gives the threads: one acceptor, and per connection a reader
//! (frames in, [`crate::HubHandle::submit`]) and a writer (the session's outbox out).
//! The server holds no protocol state of its own beyond a session's handshake.
//!
//! **Origin (spec 4):** a browser sends `Origin` with every upgrade, a native client
//! none. An upgrade whose `Origin` is not the hub itself is refused with HTTP 403.
//! The comparison is against [`OwnNames`], never the request's `Host`, which a
//! hostile page controls through DNS rebinding.

use crate::auth::AccessMode;
use crate::error::HubError;
use crate::hub::HubHandle;
use std::path::PathBuf;

/// What the network side of a hub is configured with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConfig {
    /// The port; 0 lets the OS choose. [`dspi_link::DEFAULT_PORT`] by default.
    pub port: u16,
    /// PIN by default. Open only if the user asks.
    pub access: AccessMode,
    /// Serve the web UI on this port while sharing (WP HB5).
    pub web: bool,
    /// Where the hub keeps its [`crate::store::HubStore`].
    pub store_dir: PathBuf,
}

impl NetworkConfig {
    /// The defaults: the standard port, PIN pairing, no web UI.
    pub fn new(store_dir: PathBuf) -> Self {
        Self {
            port: dspi_link::DEFAULT_PORT,
            access: AccessMode::default(),
            web: false,
            store_dir,
        }
    }
}

/// The names that count as the hub itself, for the `Origin` check: its interfaces'
/// addresses, its mDNS `.local` name and loopback, all on its bound port. Built per
/// upgrade, since interfaces change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnNames {
    /// Host names and literal addresses, lower case, without brackets.
    pub hosts: Vec<String>,
    /// The bound port.
    pub port: u16,
}

/// Whether a WebSocket upgrade with this `Origin` header may go on. No header (a
/// native client) passes; `null`, a foreign site, another port or a malformed value
/// does not.
pub fn origin_allowed(origin: Option<&str>, own: &OwnNames) -> bool {
    let _ = (origin, own);
    todo!("WP HB1")
}

/// A running server.
#[derive(Debug)]
pub struct Server {}

impl Server {
    /// Bind and start accepting. Binds LAN interfaces only and refuses peers off the
    /// local network (spec 6.4). A port in use is [`HubError::Io`].
    pub fn start(hub: HubHandle, config: &NetworkConfig) -> Result<Self, HubError> {
        let _ = (hub, config);
        todo!("WP HB1")
    }

    /// The port actually bound.
    pub fn port(&self) -> u16 {
        todo!("WP HB1")
    }

    /// Stop accepting. Existing connections go on until their outbox closes.
    pub fn stop_accepting(&self) {
        todo!("WP HB1")
    }

    /// Join the connections, whose outboxes the hub has closed with 4003. A peer that
    /// will not take the close frame is cut off after a second.
    pub fn join(self) {
        todo!("WP HB1")
    }
}
