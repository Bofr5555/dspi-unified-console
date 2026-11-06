//! DSPi Link sessions.
//!
//! Owned by WP C15 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use std::sync::Arc;

use crate::callbacks::LinkListener;
#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// Open a DSPi Link session to `url` (`link_ws_url`), replacing any
    /// previous one. `token` = the token stored for this hub's id ("" =
    /// none). Blocks for the hello and authentication (up to ~15 s). When
    /// `needs_pairing`, ask for the PIN and call `link_pair`; otherwise the
    /// device list is ready (`link_devices`).
    pub fn link_connect(
        &self,
        url: String,
        client_name: String,
        app_version: String,
        token: String,
    ) -> Result<LinkHello> {
        todo!("WP C15")
    }

    /// Pair with the PIN the user typed (role `control`, as the macOS
    /// Console asks). Returns the token to store against the hub id ("" when
    /// the hub issued none). Link { code: "denied" } for a wrong PIN.
    pub fn link_pair(&self, pin: String) -> Result<String> {
        todo!("WP C15")
    }

    /// The hub's devices (empty before authentication).
    pub fn link_devices(&self) -> Vec<LinkDevice> {
        todo!("WP C15")
    }

    pub fn link_status(&self) -> LinkStatus {
        todo!("WP C15")
    }

    /// Bind the core to the hub's device `handle` (as `open_fd` does for
    /// USB): starts the notify listener over Link, resets the state. Then
    /// `fetch_all` as usual (single-shot bulk, the hub's snapshot first).
    pub fn link_select_device(&self, handle: u32, serial: String) -> Result<()> {
        todo!("WP C15")
    }

    /// Close the Link session (normal close) and drop a device bound to it.
    pub fn link_close(&self) {
        todo!("WP C15")
    }

    /// Session events (devices changed, closed). Same rules as the other listeners.
    pub fn set_link_listener(&self, listener: Option<Arc<dyn LinkListener>>) {
        todo!("WP C15")
    }
}

/// The current DSPi Link session (WP C15).
pub(crate) struct LinkSession {}

/// Parse a hub address typed by the user (`host`, `host:port`).
pub fn link_parse_address(text: String) -> Result<LinkAddress> {
    todo!("WP C15")
}

/// The WebSocket URL for a hub.
pub fn link_ws_url(host: String, port: u16, path: String) -> String {
    todo!("WP C15")
}

/// `GET /dspi/v1/info` (spec 3.2).
pub fn link_fetch_info(host: String, port: u16) -> Result<LinkInfo> {
    todo!("WP C15")
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C15"]
    fn wp_c15_is_implemented() {
        todo!("WP C15")
    }
}
