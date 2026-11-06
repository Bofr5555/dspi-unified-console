//! DSPi Link: one device on a hub, as a [`Transport`].
//!
//! Ported from the Android console's `dspi-ffi/src/link.rs`. Status mapping
//! (Link spec 8.1) onto [`TransportError`]: ERROR/BLOCKED -> `Stalled` (the
//! RTA bands-all fallback depends on it), TIMEOUT -> `TimedOut`, NO_DEVICE ->
//! `Disconnected`, DENIED -> `Denied`, LOCKED -> `Locked`, the rest ->
//! `Remote`. Bulk is single-shot (`caps().chunked_bulk == false`); the first
//! bulk read may come from the hub's snapshot. Notifications carry the
//! writer's session id as their origin.
//!
//! Owned by WP T5 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_link::LinkClient;

use crate::{
    DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps, TransportError,
};

/// Map a Link CMD status onto a transport error (`None` for OK).
pub fn map_status(status: u8, opcode: u8) -> Option<TransportError> {
    let _ = (status, opcode);
    todo!("WP T5")
}

pub struct LinkTransport {
    #[allow(dead_code)]
    client: LinkClient,
    #[allow(dead_code)]
    handle: u8,
    descriptor: DeviceDescriptor,
}

impl LinkTransport {
    /// Device `handle` on the hub `client` is connected to. Link handles are
    /// one byte on the wire (spec 7.4); the embedder API takes a `u32` for
    /// the bindings and refuses anything above 255 before calling this.
    pub fn new(client: LinkClient, handle: u8, serial: &str) -> Self {
        let _ = (client, handle, serial);
        todo!("WP T5")
    }
}

impl Transport for LinkTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let _ = (opcode, value, len);
        todo!("WP T5")
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        let _ = (opcode, value, data);
        todo!("WP T5")
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        todo!("WP T5: the hub's max_payload")
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        todo!("WP T5")
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: crate::TransportKind::Link,
            chunked_bulk: false,
            remote: true,
        }
    }

    fn session_id(&self) -> Option<u16> {
        todo!("WP T5")
    }

    fn lock(&mut self, reason: &str, timeout: std::time::Duration) -> Result<()> {
        let _ = (reason, timeout);
        todo!("WP T5")
    }

    fn unlock(&mut self) {
        todo!("WP T5")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP T5"]
    fn link_statuses_map_onto_transport_errors() {
        todo!("WP T5")
    }

    #[test]
    #[ignore = "WP T5"]
    fn a_device_on_the_fake_hub_reads_its_bulk_image() {
        todo!("WP T5")
    }
}
