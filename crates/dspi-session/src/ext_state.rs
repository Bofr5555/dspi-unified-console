//! What the extension channel says about the connected device, probed once
//! on connect and kept with the device state.
//!
//! Owned by WP S3 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::{CoreVregStatus, PairMap};
use dspi_transport::Transport;

/// The extension channel's view of a device. `supported` is empty on stock
/// firmware (`EXT_LIST` stalls there), and then everything else is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtState {
    /// Sub-command ids from `EXT_LIST`, 0 excluded.
    pub supported: Vec<u16>,
    /// `EXT_CROSSFEED_PAIR_MAP`, when supported.
    pub pair_map: Option<PairMap>,
    /// `EXT_CORE_VREG`, when supported. Re-read on demand (it changes with
    /// uptime and the trial state).
    pub core_vreg: Option<CoreVregStatus>,
}

impl ExtState {
    /// Probe: `EXT_LIST`, then each supported sub-command this build knows.
    /// A stall is "not supported" and gives the empty state, never an error;
    /// only a transport failure (disconnect, timeout) is returned.
    pub fn probe(t: &mut dyn Transport) -> Result<Self, dspi_transport::TransportError> {
        let _ = t;
        todo!("WP S3: probe EXT_LIST and the known sub-commands")
    }

    pub fn has(&self, id: u16) -> bool {
        self.supported.contains(&id)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP S3"]
    fn stock_firmware_probes_as_empty() {
        todo!("WP S3")
    }

    #[test]
    #[ignore = "WP S3"]
    fn a_k340_probes_the_pair_map_and_core_voltage() {
        todo!("WP S3")
    }
}
