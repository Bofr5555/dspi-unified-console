//! Reading the device into the mirror.
//!
//! Owned by WP C2 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// Identify the firmware and read every parameter. Err only when the device
    /// could not be talked to; check `DspState.compat` for compatibility.
    pub fn fetch_all(&self) -> Result<()> {
        todo!("WP C2")
    }

    pub fn refresh_params(&self) -> Result<()> {
        todo!("WP C2")
    }

    /// Write the whole cached state in one chunked bulk transfer. Like the
    /// core, `Disconnected` also when no image has been read yet (nothing to
    /// apply: call `fetch_all` first).
    pub fn apply_all_params(&self) -> Result<()> {
        todo!("WP C2")
    }

    pub fn fetch_status(&self) -> Result<SystemStatus> {
        todo!("WP C2")
    }

    /// Copy of the cached state (the C ABI returns a borrowed pointer).
    pub fn get_state(&self) -> DspState {
        todo!("WP C2")
    }

    pub fn max_delay_ms(&self) -> f32 {
        todo!("WP C2")
    }

    pub fn clear_clips(&self) -> Result<()> {
        todo!("WP C2")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C2"]
    fn wp_c2_is_implemented() {
        todo!("WP C2")
    }
}
