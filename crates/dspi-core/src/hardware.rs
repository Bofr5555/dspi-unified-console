//! Hardware: pins, outputs, I2S/S/PDIF/ADAT, clocks, DAC mute, LG, control interfaces, Core 1, bootloader.
//!
//! Owned by WP C6 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// Refused { PIN_CONFIG_* } on a rejected pin.
    pub fn set_output_pin(&self, slot: u8, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn fetch_output_pin(&self, slot: u8) -> Result<u8> {
        todo!("WP C6")
    }

    /// 0 = output config stored independently, 1 = saved with presets.
    pub fn set_output_config_mode(&self, mode: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// FLASH WRITE. Refused { PRESET_* }.
    pub fn save_output_config(&self) -> Result<()> {
        todo!("WP C6")
    }

    /// 0 = S/PDIF, 1 = I2S.
    pub fn set_output_type(&self, slot: u8, kind: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// role 0 = master/shared, 1 = slave pair.
    pub fn set_i2s_bck_pin(&self, role: u8, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_i2s_clock_pin_mode(&self, mode: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_mck_enabled(&self, enabled: bool) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_mck_pin(&self, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// 0 = 128×fs, 1 = 256×fs.
    pub fn set_mck_multiplier(&self, mult: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_adat_out_enabled(&self, enabled: bool) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_adat_out_pin(&self, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_spdif_rx_pin(&self, index: u8, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// index 1..3 (input 1 is always on).
    pub fn set_spdif_input_enabled(&self, index: u8, enabled: bool) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_i2s_rx_pin(&self, pair: u8, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// 2, 4, 6 or 8.
    pub fn set_i2s_input_channels(&self, channels: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_adat_input_enabled(&self, enabled: bool) -> Result<()> {
        todo!("WP C6")
    }

    /// 0xFF clears the pin.
    pub fn set_adat_input_pin(&self, pin: u8) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_adat_input_clock_mode(&self, mode: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// I2S/ADAT-master input rate: 0 = 44.1k, 1 = 48k, 2 = 96k (others clamp
    /// to 48k). REQ_SET_INPUT_RATE 0xED is a payload SET with no status byte:
    /// Ok means the transfer completed. The firmware stores the rate and
    /// applies it live only while it is the rate authority (I2S or ADAT input
    /// in master clock mode).
    pub fn set_input_rate(&self, index: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// Hz the pipeline runs at now (0 if unknown): the LIVE rate (firmware
    /// `audio_state.freq`, REQ_GET_INPUT_RATE 0xEE word 0, same as status
    /// wValue 15) for every source, not the configured I2S rate (word 1,
    /// ignored; that one is `DspState.i2s_input_rate`).
    pub fn fetch_input_rate(&self) -> Result<u32> {
        todo!("WP C6")
    }

    /// I2S clock mode, 0 master / 1 slave. REQ_SET_I2S_CLOCK_MODE 0x88 is a
    /// payload SET with no status byte (Ok = transfer completed); the
    /// firmware applies it deferred in its main loop.
    pub fn set_i2s_clock_mode(&self, mode: u8) -> Result<()> {
        todo!("WP C6")
    }

    /// `which`: 0 = I2S slave lock, 1 = ADAT input lock.
    pub fn fetch_input_lock(&self, which: u8) -> Result<InputLockStatus> {
        todo!("WP C6")
    }

    pub fn fetch_adat_out_status(&self) -> Result<AdatOutStatus> {
        todo!("WP C6")
    }

    /// Applied later; read back with `fetch_dac_mute` ~150 ms after.
    pub fn set_dac_mute(
        &self,
        enabled: bool,
        active_low: bool,
        pin: u8,
        hold_ms: u16,
        release_ms: u16,
    ) -> Result<()> {
        todo!("WP C6")
    }

    /// Re-reads the DAC mute config into the state.
    pub fn fetch_dac_mute(&self) -> Result<()> {
        todo!("WP C6")
    }

    /// Pulse the mute output ~1 s. Refused { 3 } = feature disabled.
    pub fn test_dac_mute(&self) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_lg_sound_sync(&self, enabled: bool) -> Result<()> {
        todo!("WP C6")
    }

    pub fn fetch_lg_status(&self) -> Result<LgStatus> {
        todo!("WP C6")
    }

    /// Re-read UART/I2C configs + status into the state.
    pub fn fetch_ctrl_ifaces(&self) -> Result<()> {
        todo!("WP C6")
    }

    /// Outcome readable via `fetch_ctrl_ifaces` ~250 ms later.
    pub fn set_uart(&self, config: UartConfig) -> Result<()> {
        todo!("WP C6")
    }

    pub fn set_i2c(&self, config: I2cConfig) -> Result<()> {
        todo!("WP C6")
    }

    /// Reboots the device into its USB bootloader (it disappears). Always Ok,
    /// as in the core: the transfer result is ignored because the device
    /// drops off the bus (also Ok without a device).
    pub fn enter_bootloader(&self) -> Result<()> {
        todo!("WP C6")
    }

    /// 0 idle, 1 PDM, 2 EQ worker.
    pub fn fetch_core1_mode(&self) -> Result<u8> {
        todo!("WP C6")
    }

    /// Whether enabling `output` would conflict with Core 1's mode.
    pub fn check_core1_conflict(&self, output: u8) -> Result<bool> {
        todo!("WP C6")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C6"]
    fn wp_c6_is_implemented() {
        todo!("WP C6")
    }
}
