//! C functions: the alink's USB-direct units (WP AUSB1, the fork's
//! `extension_channel_spec.md` 2.9), over `DspiCore`'s `*_alink_usb*`,
//! `fetch_frame_counter_time_base` and `apply_at_time_base`. Only a unit
//! whose `EXT_LIST` names `EXT_ALINK_USB` answers; others reply
//! `unsupported` with nothing sent.

use std::ffi::c_char;

use dspi_core::DspiCore;

use super::{json_reply, status, text};
use crate::FfiCore;
use crate::fns_device::flag;

/// Whether the unit has the USB-direct block, as the connect probe found it
/// (no traffic). False without one or without a device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_usb_direct_supported(core: *mut FfiCore) -> bool {
    crate::with_core(core, false, |h| {
        h.core.usb_direct_supported().unwrap_or(false)
    })
}

/// `EXT_ALINK_USB` in JSON (`mode`, `modeName`, `autoReanchor`, `d`,
/// `dBelowMin`, `dAboveMax`, `rateUnsupported`, `state`, `stateCode`), or a
/// failure (`unsupported` without the block).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_usb(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_alink_usb()))
}

/// Write `EXT_ALINK_USB`: `mode` 0 off, 1 rate lock, 2 phase lock;
/// `auto_reanchor`; `d` samples (170-399 at 48 kHz in mode 2). A change
/// re-anchors. Returns a status byte, 0xFF on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_alink_usb(
    core: *mut FfiCore,
    mode: u8,
    auto_reanchor: bool,
    d: u16,
) -> u8 {
    status(core, |c| c.set_alink_usb(mode, auto_reanchor, d))
}

/// `EXT_ALINK_USB_STATUS` in JSON (`state`, `stateCode`, `phaseState`,
/// `rateCode`, `rateHz`, `hwTimestamp`, `stream`, `nominalFeedback`,
/// `timeBase`, `holdover`, `hostPpm`, `vcxoPpm`, `phaseErrorSamples`,
/// `sample`, `busFrame`, `dMin`, `dMax`, `fill`, `streamStartBusFrame`,
/// `streamAgeFrames`, `streamShift`, `gapCount`, `anchors`, `reanchors`,
/// `missedSamples`, `lateSamples`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_usb_status(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_alink_usb_status()))
}

/// `EXT_ALINK_USB_DIAG` in JSON (`lastCause`, `rides`, `holdovers`, `ends`
/// as `{cause, count}`, `suspends`, `resumes`, `busResets`, `unplugs`,
/// `lastFrames`, `lastUs`, `lastDevUs`, `longestRideUs`, `maxPassUs`,
/// `maxIsrQuietUs`, `maxIsrLateUs`, `holdoverUs`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_usb_diag(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_alink_usb_diag()))
}

/// Clear `EXT_ALINK_USB_DIAG`. Returns a status byte, 0xFF on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_clear_alink_usb_diag(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::clear_alink_usb_diag)
}

/// `EXT_FRAME_COUNTER` with its time base in JSON (`frame`, `timeUs`,
/// `outputFrames`, `timeBase`, `timeBaseCode`, `busFrame`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_frame_counter_time_base(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.fetch_frame_counter_time_base())
    })
}

/// Queue `op` with `arg` and `value` on time base `time_base` (0 the alink
/// frame, 1 the USB sample index, 2 a bus frame in bits 0-10 and the sample in
/// bits 16-23) at `frame`. Returns a status byte, 0xFF on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_at_time_base(
    core: *mut FfiCore,
    time_base: u8,
    frame: u64,
    op: u8,
    arg: u8,
    value: i16,
) -> u8 {
    status(core, |c| {
        c.apply_at_time_base(time_base, frame, op, arg, value)
    })
}

/// Open the demo device as an alink USB-direct unit, in place of any open
/// device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_open_demo_usb_direct(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::open_demo_usb_direct)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_ausb1;
