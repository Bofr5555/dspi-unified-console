//! C functions: the meter ballistics and the event history (WP P0M), over
//! `dspi_core`'s `meter_ballistics` and `event_log_*`.
//!
//! The history crosses as JSON lines in the history's file format: one record
//! per line, `{"v":1,"id":..,"t":..,"kind":"..","serial":"..","text":".."...}`
//! (`dspi_session::events`). A text reply too long for `buf` writes nothing and
//! returns the length needed, so ask with a null `buf` and call again.

use std::ffi::c_char;

use dspi_core::DspiCore;

use super::{c_str, copy_text, fetch_value, flag, text};
use crate::types::MeterBallistics;
use crate::{FfiCore, with_core};

/// The meter ballistics' timing into `out`. False for a null handle or an
/// unusable `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_meter_ballistics(core: *mut FfiCore, out: *mut MeterBallistics) -> bool {
    fetch_value(core, out, |c| {
        Ok(MeterBallistics::from(c.meter_ballistics()))
    })
}

/// Change the meter ballistics' timing (`SystemStatus.peak_hold`,
/// `clip_hold`) for every later `dspi_fetch_status`. A negative or non-finite
/// `decay_per_s` reads as 0; a `clip_hold_ms` of 0 holds until
/// `dspi_clear_clips`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_meter_ballistics(
    core: *mut FfiCore,
    hold_ms: u32,
    decay_per_s: f32,
    clip_hold_ms: u32,
) {
    with_core(core, (), |h| {
        h.core.set_meter_ballistics(dspi_core::MeterBallistics {
            hold_ms,
            decay_per_s,
            clip_hold_ms,
        });
    });
}

/// Start every channel's `SystemStatus.peak_max` again.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_reset_meter_max(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.reset_meter_max());
}

/// Keep the event history in files under `dir` (the app's data directory),
/// or in memory only for "" or null. Blocking file I/O. False when the
/// directory cannot be created (`dspi_last_error`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_event_log_set_dir(core: *mut FfiCore, dir: *const c_char) -> bool {
    let dir = c_str(dir).unwrap_or("").to_owned();
    flag(core, |c| c.event_log_set_dir(dir))
}

/// `serial`'s history records with ids above `after_id` into `buf` as JSON
/// lines, oldest first, as many whole lines as fit and at most `limit`: with
/// `after_id` 0 the newest, otherwise the next after it. A null or empty
/// `serial` is the open device. Returns the id to pass next time: the last
/// line written, or past a record too long for the buffer at all, which is
/// dropped. With nothing new, a null buffer, a length of 0 or a null handle it
/// returns `after_id`. Reads files: not on the UI thread.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_event_log_read(
    core: *mut FfiCore,
    serial: *const c_char,
    after_id: u64,
    limit: u32,
    buf: *mut c_char,
    buf_len: u32,
) -> u64 {
    if buf.is_null() || buf_len == 0 {
        return after_id;
    }
    let serial = c_str(serial).unwrap_or("").to_owned();
    with_core(core, after_id, |h| {
        let page = h.core.event_log_page(serial, after_id, limit);
        let room = usize::try_from(buf_len).map_or(usize::MAX, |n| n.saturating_sub(1));
        let mut lines = String::new();
        let mut next = after_id;
        for record in &page.records {
            let mut line = serde_json::to_string(record).unwrap_or_default();
            line.push('\n');
            if line.len() <= room {
                if lines.len().saturating_add(line.len()) > room {
                    break;
                }
                lines.push_str(&line);
            }
            next = record.id;
        }
        copy_text(&lines, buf, buf_len);
        next
    })
}

/// `serial`'s whole event history as JSON lines (the file format), oldest
/// first; a null or empty `serial` is the open device. Returns the bytes
/// needed, NUL included; 1 (an empty text) for a device with no history.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_event_log_export(
    core: *mut FfiCore,
    serial: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let serial = c_str(serial).unwrap_or("").to_owned();
    text(core, buf, len, |c| c.event_log_export(serial))
}

/// The serials with an event history, as a JSON array of strings, sorted.
/// Returns the bytes needed, NUL included.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_event_log_devices(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c: &DspiCore| {
        serde_json::to_string(&c.event_log_devices()).unwrap_or_else(|_| "[]".into())
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0m;
