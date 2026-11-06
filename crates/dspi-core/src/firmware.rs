//! Firmware files and the bootloader: check a `.uf2` before flashing, find
//! the board's bootloader drive, copy the image to it.
//!
//! The Qt app does the drive work in C++ today (`FirmwareUpdater.cpp`) and the
//! Android app in Kotlin; this gives every core-based app (Qt, Terminal,
//! later iOS for the check only) one implementation. Entering the bootloader
//! is `DspiCore::enter_bootloader` (WP C6).
//!
//! Owned by WP C11 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use crate::Result;
use crate::records::{BootloaderCounts, BootloaderDrive, Uf2Info};

/// Check a UF2 image: block structure, magic numbers, family id, the flash
/// range it covers, and the DSPi version string inside it.
pub fn uf2_inspect(image: Vec<u8>) -> Uf2Info {
    todo!("WP C11")
}

/// Boards in BOOTSEL mode on the bus (USB `2E8A:0003` RP2040, `2E8A:000F`
/// RP2350). 0/0 where the bus cannot be enumerated (Android).
pub fn bootloader_counts() -> BootloaderCounts {
    todo!("WP C11")
}

/// Mounted bootloader drives (`INFO_UF2.TXT` at the root): Linux
/// `/proc/self/mountinfo` + `/run/media`, macOS `/Volumes`, Windows drive
/// letters.
pub fn bootloader_drives() -> Vec<BootloaderDrive> {
    todo!("WP C11")
}

/// Write `image` to `drive` as `firmware.uf2`, synced to the medium. Refuses
/// an image whose family does not match the drive's board. The board reboots
/// into the new firmware when the copy completes.
pub fn flash_to_drive(image: Vec<u8>, drive: BootloaderDrive) -> Result<()> {
    todo!("WP C11")
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C11"]
    fn a_dspi_image_inspects_as_valid() {
        todo!("WP C11")
    }
}
