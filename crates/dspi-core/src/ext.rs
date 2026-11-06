//! Extension channel.
//!
//! Owned by WP C8 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// `EXT_CROSSFEED_PAIR_MAP` (Bofr5555/DSPi fork, the K340 build): the
    /// outputs of every crossfeed pair at once, 4 bytes
    /// (`DspState.crossfeed_pair_map` encoding). `InvalidArgument`, with no
    /// traffic, for a map the firmware would refuse
    /// ([`crate::crossfeed_pair_map_valid`]); `Unsupported` when the device
    /// has no such extension; stored normalized.
    pub fn set_crossfeed_pair_map(&self, map: Vec<u8>) -> Result<()> {
        todo!("WP C8")
    }

    /// Read the live pair map (4 bytes) into the state. `Unsupported` when
    /// the firmware has no such extension (it stalls); the state then keeps
    /// the all-default map (zeros). The bulk image does not carry the map.
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
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C8"]
    fn wp_c8_is_implemented() {
        todo!("WP C8")
    }
}
