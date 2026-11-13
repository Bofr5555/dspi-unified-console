//! C functions: the audio link (WP SW1, the fork's `extension_channel_spec.md`
//! 2.8), over `DspiCore`'s `audio_link_*`, `*_audio_link_*`,
//! `fetch_audio_link_frame_counter` and `apply_at_frame`. Only a unit whose
//! `EXT_LIST` names `EXT_LINK_INFO` answers; others reply `unsupported`
//! with nothing sent. Not DSPi Link (`fns_link.rs`).

use std::ffi::c_char;

use dspi_core::DspiCore;

use super::{c_bytes, json_reply, status, text};
use crate::fns_device::flag;
use crate::{FfiCore, with_core};

/// The matrix inputs a unit takes from the link.
const SLOTS: usize = 8;

/// The unit's audio link as the connect probe found it, in JSON, with no
/// traffic: `value` null without a link, else `role`, `roleCode`,
/// `protocolVersion`, `classMask`, `maxLinks`, `maxChannels`,
/// `slotsPerUnit`, `rxPin`, `txPin`, `featureBits`, `dLocalMin`,
/// `dLocalMax`, `linkInput`, `stateEvents`, `applyEvents`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_audio_link_info(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.audio_link_info()))
}

/// Read `EXT_LINK_INFO` again (its delay limits follow the output latency
/// option), in JSON as [`dspi_audio_link_info`]'s fields, or a failure
/// (`unsupported` without the link).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_audio_link_info(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_audio_link_info()))
}

/// The link's status in JSON (`state`, `stateCode`, `linkClass`, `rateCode`,
/// `channels`, `hops`, `losPin`, `senderId`, `flags`, `frame`, `lockCount`,
/// `lossCount`, `crc8Errors`, `crc16Errors`, `bitSlips`, `superframeErrors`,
/// `vcxoPpm`, `phaseErrorFrames`, `fillFrames`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_audio_link_status(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_audio_link_status()))
}

/// The link slot of each of the 8 matrix inputs (0xFF silent) into `out` of
/// `out_len` bytes. False for a buffer under 8 bytes, with nothing written,
/// or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_audio_link_slots(
    core: *mut FfiCore,
    out: *mut u8,
    out_len: u32,
) -> bool {
    let fits = usize::try_from(out_len).is_ok_and(|n| n >= SLOTS);
    if out.is_null() || !fits {
        return false;
    }
    with_core(core, false, |h| {
        let Ok(slots) = h.core.fetch_audio_link_slots() else {
            return false;
        };
        for (i, byte) in slots.into_iter().take(SLOTS).enumerate() {
            // SAFETY: `out` holds `out_len` >= 8 bytes and `i < 8`.
            unsafe { out.add(i).write(byte) };
        }
        true
    })
}

/// Write the 8 slots (`slots_len` must be 8; 0..63 a link channel, 0xFF
/// silence). Returns a status byte; 0xFF for a null or short buffer without
/// asking the device, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_audio_link_slots(
    core: *mut FfiCore,
    slots: *const u8,
    slots_len: u32,
) -> u8 {
    if slots.is_null() {
        return 0xFF;
    }
    let bytes = c_bytes(slots, slots_len);
    status(core, |c| c.set_audio_link_slots(bytes))
}

/// The phase-lock mode and delay in JSON (`mode`, `dLocal`, `hopsOffset`,
/// `flags`, `state`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_audio_link_phase(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_audio_link_phase()))
}

/// Set the phase-lock `mode` (0 fill servo, 1 phase lock), the fixed delay
/// `d_local` in link frames and `hops_offset` (-64..64, normally 0).
/// Returns a status byte, 0xFF on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_audio_link_phase(
    core: *mut FfiCore,
    mode: u8,
    d_local: u16,
    hops_offset: i16,
) -> u8 {
    status(core, |c| c.set_audio_link_phase(mode, d_local, hops_offset))
}

/// The link frame counter in JSON (`frame`, `timeUs`, `outputFrames`), or a
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_audio_link_frame_counter(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.fetch_audio_link_frame_counter())
    })
}

/// Queue `op` (0 cancel all, 1 mute, 2 unmute, 3 preset load, 4 master
/// volume, 5 output delay, 6 rate, 7 click, 8 sync pulse) with `arg` and
/// `value` for link frame `frame`. Returns a status byte, 0xFF on failure
/// (arguments out of range, a full queue, no lock).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_at_frame(
    core: *mut FfiCore,
    frame: u64,
    op: u8,
    arg: u8,
    value: i16,
) -> u8 {
    status(core, |c| c.apply_at_frame(frame, op, arg, value))
}

/// The apply queue in JSON (`pending`, `lastOp`, `lastOpCode`, `lastResult`,
/// `lastResultCode`, `lastFrame`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_apply_at_frame_status(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.fetch_apply_at_frame_status())
    })
}

/// Clear the link's error and lock counters. Returns a status byte, 0xFF on
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_reset_audio_link_stats(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::reset_audio_link_stats)
}

/// Open the demo device as a unit receiving the audio link, in place of any
/// open device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_open_demo_audio_link(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::open_demo_audio_link)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_sw1;
