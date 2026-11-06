//! Extension channel.
//!
//! Owned by WP C8 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// K340 firmware fork (third_party/patches/0002): the outputs of every
    /// crossfeed pair at once, 4 bytes (`DspState.crossfeed_pair_map`
    /// encoding). `InvalidArgument` (no USB traffic) for a map the firmware
    /// would refuse (see `crossfeed_pair_map_valid`); stored normalized.
    pub fn set_crossfeed_pair_map(&self, map: Vec<u8>) -> Result<()> {
        todo!("WP C8")
    }

    /// Reads the live pair map (4 bytes) into the state. An error from a
    /// connected device means the firmware does not have the feature (it
    /// STALLs the request); the state then keeps the bulk image's zero map.
    pub fn fetch_crossfeed_pair_map(&self) -> Result<Vec<u8>> {
        todo!("WP C8")
    }

    /// What the extension channel offers on the connected device (probed on
    /// connect; empty on stock firmware).
    pub fn ext_status(&self) -> Result<ExtStatus> {
        todo!("WP C8")
    }

    /// Read `EXT_CORE_VREG`.
    pub fn fetch_core_vreg(&self) -> Result<CoreVregInfo> {
        todo!("WP C8")
    }

    /// `EXT_CORE_VREG` write: `op` 0 set (value = VREG code, 0 = build
    /// default), 1 trial, 2 end trial, 3 auto (value 0/1), 4 clear
    /// (`dspi_proto::ext::VregOp`).
    pub fn core_vreg_op(&self, op: u8, value: u8) -> Result<()> {
        let _ = (op, value);
        todo!("WP C8")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C8"]
    fn wp_c8_is_implemented() {
        todo!("WP C8")
    }
}
