//! C functions: the audio link (WP SW1, the fork's `extension_channel_spec.md`
//! 2.8), over `DspiCore`'s `alink_*`, `*_alink_*`,
//! `fetch_alink_frame_counter` and `apply_at_frame`. Only a unit whose
//! `EXT_LIST` names `EXT_ALINK_INFO` answers; others reply `unsupported`
//! with nothing sent. Not DSPi Link (`fns_link.rs`).

use std::ffi::c_char;

use dspi_core::DspiCore;

use super::{c_bytes, json_reply, status, text};
use crate::fns_device::flag;
use crate::{FfiCore, with_core};

/// The matrix inputs a unit takes from the alink.
const SLOTS: usize = 8;

/// The unit's audio link as the connect probe found it, in JSON, with no
/// traffic: `value` null without an alink, else `role`, `roleCode`,
/// `protocolVersion`, `classMask`, `maxAlinks`, `maxChannels`,
/// `slotsPerUnit`, `rxPin`, `txPin`, `featureBits`, `dLocalMin`,
/// `dLocalMax`, `alinkInput`, `stateEvents`, `applyEvents`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_alink_info(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.alink_info()))
}

/// Read `EXT_ALINK_INFO` again (its delay limits follow the output latency
/// option), in JSON as [`dspi_alink_info`]'s fields, or a failure
/// (`unsupported` without the alink).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_info(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_alink_info()))
}

/// The alink's status in JSON (`state`, `stateCode`, `alinkClass`, `rateCode`,
/// `channels`, `hops`, `losPin`, `senderId`, `flags`, `frame`, `lockCount`,
/// `lossCount`, `crc8Errors`, `crc16Errors`, `bitSlips`, `superframeErrors`,
/// `vcxoPpm`, `phaseErrorFrames`, `fillFrames`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_status(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_alink_status()))
}

/// The audio link slot of each of the 8 matrix inputs (0xFF silent) into `out` of
/// `out_len` bytes. False for a buffer under 8 bytes, with nothing written,
/// or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_slots(core: *mut FfiCore, out: *mut u8, out_len: u32) -> bool {
    let fits = usize::try_from(out_len).is_ok_and(|n| n >= SLOTS);
    if out.is_null() || !fits {
        return false;
    }
    with_core(core, false, |h| {
        let Ok(slots) = h.core.fetch_alink_slots() else {
            return false;
        };
        for (i, byte) in slots.into_iter().take(SLOTS).enumerate() {
            // SAFETY: `out` holds `out_len` >= 8 bytes and `i < 8`.
            unsafe { out.add(i).write(byte) };
        }
        true
    })
}

/// Write the 8 slots (`slots_len` must be 8; 0..63 an audio link channel, 0xFF
/// silence). Returns a status byte; 0xFF for a null or short buffer without
/// asking the device, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_alink_slots(core: *mut FfiCore, slots: *const u8, slots_len: u32) -> u8 {
    if slots.is_null() {
        return 0xFF;
    }
    let bytes = c_bytes(slots, slots_len);
    status(core, |c| c.set_alink_slots(bytes))
}

/// The phase-lock mode and delay in JSON (`mode`, `dLocal`, `hopsOffset`,
/// `flags`, `state`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_phase(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_alink_phase()))
}

/// Set the phase-lock `mode` (0 fill servo, 1 phase lock), the fixed delay
/// `d_local` in alink frames and `hops_offset` (-64..64, normally 0).
/// Returns a status byte, 0xFF on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_alink_phase(
    core: *mut FfiCore,
    mode: u8,
    d_local: u16,
    hops_offset: i16,
) -> u8 {
    status(core, |c| c.set_alink_phase(mode, d_local, hops_offset))
}

/// The alink frame counter in JSON (`frame`, `timeUs`, `outputFrames`), or a
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_alink_frame_counter(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.fetch_alink_frame_counter())
    })
}

/// Queue `op` (0 cancel all, 1 mute, 2 unmute, 3 preset load, 4 master
/// volume, 5 output delay, 6 rate, 7 click, 8 sync pulse; -- WP ATCORE --
/// 9 generator start: the staged finite config's sample 0 at that frame,
/// `arg` 0 and `value` 0, refused unless staged since the last stop and
/// at least the arm lead ahead; 10 generator stop, `arg` 0 fade or 1 now)
/// with `arg` and `value` for alink frame `frame`. Returns a status byte,
/// 0xFF on failure (arguments out of range, a full queue, no lock).
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

/// Clear the alink's error and lock counters. Returns a status byte, 0xFF on
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_reset_alink_stats(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::reset_alink_stats)
}

/// Open the demo device as a unit receiving the audio link, in place of any
/// open device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_open_demo_alink(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::open_demo_alink)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_sw1;
