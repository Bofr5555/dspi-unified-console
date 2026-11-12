//! C functions: identify and the reset readout (WP FWX, PRO1 FW4 and FW5),
//! over `DspiCore::identify`, `fetch_identify` and `fetch_reset_info`. Only
//! firmware that lists `EXT_IDENTIFY` / `EXT_RESET_INFO` (wp/FWX) answers;
//! others reply `unsupported` with nothing sent.

use std::ffi::c_char;

use super::{json_reply, status, text};
use crate::FfiCore;

/// The `tone_output` of [`dspi_identify`] for no tone (0xFF in C).
const IDENTIFY_NO_TONE: u8 = 0xFF;

/// Blink the device's LED for `seconds` (0 stops it; the firmware shortens
/// more than 120 to 120). `tone_output` is an output for the quiet identify
/// tone (1 kHz, -40 dBFS, 400 ms, through that output's whole chain), or
/// 0xFF for none; pass 0xFF unless the user asked for sound. Returns a
/// status byte, 0xFF on failure (unsupported, a tone output past the
/// outputs, a read-only connection, a refusal).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_identify(core: *mut FfiCore, seconds: u8, tone_output: u8) -> u8 {
    let tone = (tone_output != IDENTIFY_NO_TONE).then_some(tone_output);
    status(core, |c| c.identify(seconds, tone))
}

/// The identify state and the board, in JSON (`remainingS`,
/// `lastToneOutput`, `chip`, `chipRev`, `chipId`, `board`, `hwRev`), or a
/// failure (`unsupported` without it).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_identify(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_identify()))
}

/// Why the device last restarted, in JSON (`reason`, `reasonCode`,
/// `description`, `uptimeS`, `prevUptimeS` (null after a power-on),
/// `hangCount`, `chip`, `chipReset`, `watchdogReason`), or a failure
/// (`unsupported` without it).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_reset_info(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_reset_info()))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_fwx;
