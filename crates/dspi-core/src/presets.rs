//! Presets, flash, baseline and diff.
//!
//! Owned by WP C5 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// FLASH WRITE (waits for the deferred save). Refused { PRESET_* }.
    pub fn save_preset(&self, slot: u8) -> Result<()> {
        todo!("WP C5")
    }

    /// Load and re-read every parameter. Refused { PRESET_* }.
    pub fn load_preset(&self, slot: u8) -> Result<()> {
        todo!("WP C5")
    }

    /// FLASH WRITE: live state -> `dest`, re-save `source` (stays active).
    pub fn copy_preset(&self, source: u8, dest: u8) -> Result<()> {
        todo!("WP C5")
    }

    /// FLASH WRITE.
    pub fn delete_preset(&self, slot: u8) -> Result<()> {
        todo!("WP C5")
    }

    pub fn set_preset_name(&self, slot: u8, name: String) -> Result<()> {
        todo!("WP C5")
    }

    pub fn get_preset_name(&self, slot: u8) -> Result<String> {
        todo!("WP C5")
    }

    pub fn get_preset_directory(&self) -> Result<PresetDirectory> {
        todo!("WP C5")
    }

    /// mode 0 = load `default_slot` at boot, 1 = last active slot.
    pub fn set_preset_startup(&self, mode: u8, default_slot: u8) -> Result<()> {
        todo!("WP C5")
    }

    /// FLASH WRITE. Refused { FLASH_* }.
    pub fn save_params(&self) -> Result<()> {
        todo!("WP C5")
    }

    /// Revert to saved (reloads the active preset). Refused { FLASH_* }.
    pub fn load_params(&self) -> Result<()> {
        todo!("WP C5")
    }

    /// FLASH WRITE. Refused { FLASH_* }.
    pub fn factory_reset(&self) -> Result<()> {
        todo!("WP C5")
    }

    /// The live state now matches the active preset.
    pub fn capture_baseline(&self) {
        todo!("WP C5")
    }

    pub fn preset_dirty(&self) -> bool {
        todo!("WP C5")
    }

    /// What differs from the active preset, one sentence each.
    pub fn preset_changes(&self) -> Vec<String> {
        todo!("WP C5")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C5"]
    fn wp_c5_is_implemented() {
        todo!("WP C5")
    }
}
