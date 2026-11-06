//! Signal generator and statistics.
//!
//! Owned by WP C10 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// Ok with `supported = false` when the firmware has no generator.
    pub fn siggen_fetch_caps(&self) -> Result<SiggenCaps> {
        todo!("WP C10")
    }

    /// The config the generator runs, as the device sanitised it.
    pub fn siggen_get_config(&self) -> Result<SiggenConfig> {
        todo!("WP C10")
    }

    /// Stage a config (a running generator restarts). The device ACKs even a
    /// refused config: read it back.
    pub fn siggen_set_config(&self, config: SiggenConfig) -> Result<()> {
        todo!("WP C10")
    }

    /// SIGGEN_CTL_START / STOP (fade) / STOP_NOW.
    pub fn siggen_control(&self, action: u16) -> Result<()> {
        todo!("WP C10")
    }

    pub fn siggen_get_status(&self) -> Result<SiggenStatus> {
        todo!("WP C10")
    }

    /// One REQ_GET_STATUS u32 counter (STAT_*).
    pub fn fetch_stat(&self, which: u16) -> Result<u32> {
        todo!("WP C10")
    }

    pub fn fetch_buffer_stats(&self) -> Result<BufferStats> {
        todo!("WP C10")
    }

    pub fn reset_buffer_stats(&self) -> Result<()> {
        todo!("WP C10")
    }

    pub fn fetch_spdif_rx_status(&self) -> Result<SpdifRxStatus> {
        todo!("WP C10")
    }

    /// 24 bytes of IEC 60958 channel status.
    pub fn fetch_spdif_rx_channel_status(&self) -> Result<Vec<u8>> {
        todo!("WP C10")
    }

    /// GPIO of S/PDIF receiver `index`; Err if it has none.
    pub fn fetch_spdif_rx_pin(&self, index: u8) -> Result<u8> {
        todo!("WP C10")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C10"]
    fn wp_c10_is_implemented() {
        todo!("WP C10")
    }
}
