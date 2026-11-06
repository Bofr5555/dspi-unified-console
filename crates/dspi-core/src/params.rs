//! DSP parameters: EQ, volume, loudness, crossfeed, delays, leveller, matrix, outputs, names.
//!
//! Owned by WP C3 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// PEQ band 0..9 of wire channel `ch`.
    pub fn set_filter(&self, ch: u8, band: u8, params: FilterParams) -> Result<()> {
        todo!("WP C3")
    }

    /// Crossover band 0..3 of an output's wire channel `ch`.
    pub fn set_crossover(&self, ch: u8, xband: u8, params: FilterParams) -> Result<()> {
        todo!("WP C3")
    }

    /// `band`: PEQ 0..9 or crossover 20..23.
    pub fn set_band_bypass(&self, ch: u8, band: u8, bypass: bool) -> Result<()> {
        todo!("WP C3")
    }

    /// Legacy preamp: every input to the same value.
    pub fn set_preamp(&self, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_input_preamp(&self, input: u8, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_bypass(&self, enabled: bool) -> Result<()> {
        todo!("WP C3")
    }

    /// dB: −128 = mute, otherwise −127..0.
    pub fn set_master_volume(&self, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    /// 0 = independent of presets, 1 = saved with presets.
    pub fn set_master_volume_mode(&self, mode: u8) -> Result<()> {
        todo!("WP C3")
    }

    /// FLASH WRITE. Refused { PRESET_* } on failure.
    pub fn save_master_volume(&self) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_user_volume(&self, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_user_mute(&self, muted: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_delay(&self, ch: u8, ms: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_loudness(&self, enabled: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_loudness_ref(&self, spl: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_loudness_intensity(&self, pct: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_loudness_mask(&self, mask: u16) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_crossfeed(&self, enabled: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_crossfeed_preset(&self, preset: u8) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_crossfeed_freq(&self, freq: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_crossfeed_feed(&self, feed: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_crossfeed_itd(&self, enabled: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_crossfeed_outputs(&self, pair_mask: u8) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_leveller_enabled(&self, enabled: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_leveller_amount(&self, pct: f32) -> Result<()> {
        todo!("WP C3")
    }

    /// 0 slow, 1 medium, 2 fast.
    pub fn set_leveller_speed(&self, speed: u8) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_leveller_max_gain(&self, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_leveller_lookahead(&self, enabled: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_leveller_gate(&self, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_leveller_masks(&self, detector: u8, apply: u8) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_input_source(&self, source: u8) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_matrix_route(
        &self,
        input: u8,
        output: u8,
        enabled: bool,
        gain: f32,
        invert: bool,
    ) -> Result<()> {
        todo!("WP C3")
    }

    /// Returns the resulting state: the firmware may refuse (PDM / Core 1 conflict).
    pub fn set_output_enable(&self, output: u8, enabled: bool) -> Result<bool> {
        todo!("WP C3")
    }

    pub fn set_output_gain(&self, output: u8, db: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_output_mute(&self, output: u8, muted: bool) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_output_delay(&self, output: u8, ms: f32) -> Result<()> {
        todo!("WP C3")
    }

    pub fn set_channel_name(&self, ch: u8, name: String) -> Result<()> {
        todo!("WP C3")
    }

    /// Reads the name from the device.
    pub fn get_channel_name(&self, ch: u8) -> Result<String> {
        todo!("WP C3")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C3"]
    fn wp_c3_is_implemented() {
        todo!("WP C3")
    }
}
