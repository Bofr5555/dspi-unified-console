//! Effects: psybass, limiter, upmixer, subharmonic synth, tube.
//!
//! Owned by WP C4 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// `param`: PSYBASS_PARAM_*.
    pub fn set_psybass_param(&self, param: u8, value: f32) -> Result<()> {
        todo!("WP C4")
    }

    pub fn set_psybass_mask(&self, mask: u16) -> Result<()> {
        todo!("WP C4")
    }

    pub fn set_psybass_enabled(&self, enabled: bool) -> Result<()> {
        todo!("WP C4")
    }

    /// `output` = LIMITER_ALL_OUTPUTS (0xFF) for every output; `param` LIMITER_PARAM_*.
    pub fn set_limiter_param(&self, output: u8, param: u8, value: f32) -> Result<()> {
        todo!("WP C4")
    }

    /// Gain reduction (dB, > 0 while limiting) per output, MAX_OUTPUTS entries.
    pub fn fetch_limiter_meter(&self) -> Result<Vec<f32>> {
        todo!("WP C4")
    }

    /// UPMIX_PARAM_*; RP2350 only.
    pub fn set_upmix_param(&self, id: u8, value: f32) -> Result<()> {
        todo!("WP C4")
    }

    pub fn fetch_upmix_status(&self) -> Result<UpmixStatus> {
        todo!("WP C4")
    }

    /// SUBHARM_PARAM_*.
    pub fn set_subharm_param(&self, id: u8, value: f32) -> Result<()> {
        todo!("WP C4")
    }

    pub fn set_subharm_solo(&self, solo: bool) -> Result<()> {
        todo!("WP C4")
    }

    /// Headroom the subharm settings cost, dB.
    pub fn fetch_subharm_headroom(&self) -> Result<f32> {
        todo!("WP C4")
    }

    /// Synthesized sub peak per output 0..1, MAX_OUTPUTS entries.
    pub fn fetch_subharm_meter(&self) -> Result<Vec<f32>> {
        todo!("WP C4")
    }

    /// TUBE_PARAM_*.
    pub fn set_tube_param(&self, idx: u8, value: f32) -> Result<()> {
        todo!("WP C4")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C4"]
    fn wp_c4_is_implemented() {
        todo!("WP C4")
    }
}
