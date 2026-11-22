//! C functions: the roll-off search (WP ROLL1, owner 2026-11-22), over
//! `dspi_core`'s `measure` module: where an output's response drops off at
//! the low end, measured from the top down; on a tweeter never below 1 kHz.
//!
//! The setup is JSON (`MeasureRolloffSetup`, camelCase: `{output, role,
//! rolesFromLayout, speaker, placement, micDocument, startHz, sweepMs,
//! ceilingDb, splCapDb, captureRateHz, name, startLevelOnly}`; every field
//! but `output` optional). Every reply is the search's state `{ok, stage
//! ("idle", "measuring", "done", "stopped"), running, output, role, floorHz,
//! segment, segments, segmentStartHz, session, result, issue, actor}`, its
//! `result` `{output, role, end ("rolledOff", "floor"), stopDb, referenceDb,
//! referenceHz, rolloffHz, lowestHz, lowEndDb, note, freqsHz, magDb, spl,
//! document}`, or a failure.
//!
//! While it runs, the client calls `dspi_measure_rolloff_tick` about every
//! 50 ms off the UI thread (instead of `dspi_measure_tick`) and pushes
//! capture with `dspi_measure_capture_push` whenever `session.wantsCapture`.

use std::ffi::c_char;

use dspi_core::{DspiError, MeasureRolloffSetup, Result};

use super::{done_reply, json_reply, text};
use crate::FfiCore;
use crate::fns_device::arg;

/// The setup from its JSON text.
fn setup(p: *const c_char) -> Result<MeasureRolloffSetup> {
    let t = arg(p, "setup")?;
    serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
        msg: format!("the roll-off setup: {e}"),
    })
}

/// Start the roll-off search of `setup_json`'s output: the state, or a
/// failure (`invalid` naming the blockers or `measure.layout_unconfirmed:`,
/// `read_only`, `denied`, `locked`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rolloff_start(
    core: *mut FfiCore,
    setup_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(setup(setup_json).and_then(|s| c.measure_rolloff_start(s)))
    })
}

/// Drive the search (about every 50 ms, off the UI thread): its state (never
/// a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rolloff_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_rolloff_tick())))
}

/// The search's state, without I/O (safe from the measure callback).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rolloff_state(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.measure_rolloff_state()))
    })
}

/// The user's Stop: the state (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rolloff_stop(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_rolloff_stop())))
}

/// Forget a finished search: `{ok}`, or `invalid` while one runs.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rolloff_clear(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| done_reply(c.measure_rolloff_clear()))
}

#[cfg(test)]
mod tests_wp_roll1 {
    use std::ffi::CString;

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    fn read(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 1 << 16];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len());
        let s = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        serde_json::from_str(s.to_str().unwrap()).unwrap()
    }

    #[test]
    fn wp_roll1_the_c_calls_carry_the_search() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let s = read(|b, l| dspi_measure_rolloff_state(h, b, l));
        assert_eq!(s["stage"], "idle", "{s}");
        // The demo's volume is too high to measure: refused, nothing runs.
        let setup = CString::new(r#"{"output":2,"role":"high","sweepMs":1000}"#).unwrap();
        let r = read(|b, l| dspi_measure_rolloff_start(h, setup.as_ptr(), b, l));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "invalid");
        let bad = CString::new(r#"{"output":2,"startHz":15000}"#).unwrap();
        let r = read(|b, l| dspi_measure_rolloff_start(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
        let bad = CString::new("{").unwrap();
        let r = read(|b, l| dspi_measure_rolloff_start(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
        let s = read(|b, l| dspi_measure_rolloff_tick(h, b, l));
        assert_eq!(s["running"], false, "{s}");
        let s = read(|b, l| dspi_measure_rolloff_stop(h, b, l));
        assert_eq!(s["ok"], true);
        let r = read(|b, l| dspi_measure_rolloff_clear(h, b, l));
        assert_eq!(r["ok"], true, "{r}");
        dspi_core_free(h);
    }
}
