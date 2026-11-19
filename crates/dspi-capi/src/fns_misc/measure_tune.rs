//! C functions: the auto-tune (WP P5TUNE, PLAN-P5 section 1.4), over
//! `dspi_core`'s `measure` module.
//!
//! The layout is JSON (`MeasureLayout`, camelCase: `{outputs: [{output, role
//! ("full", "sub", "low", "mid", "high"), speaker, group, name, feedInput,
//! feedGainDb, feedInverted}], template, notes}`), the options too
//! (`MeasureTuneOptions`: `{targetDocument, micDocument, keepCrossover,
//! keepDelays, keepLevels, keepEq, windowOctaves, fitMinHz, fitMaxHz}`; every
//! field optional). Every reply is JSON: the layout, a proposal `{ok, id,
//! serial, measurementId, measurementName, writes: [{output, kind, timing,
//! words, params: [{path, indices, value}]}], curves, transitions, held,
//! withdrawn, warnings, notes, timingReliable, sumCheck}`, the tune's state
//! `{ok, stage, running, proposalId, run, runs, session, guard, verify,
//! issue, snapshotBeforeId, snapshotAfterId}`, the guard, Keep's
//! `{ok, guard, snapshotBeforeId, snapshotBefore, snapshotAfterId,
//! snapshotAfter, measurementId, measurement, notes}`, an A/B compare, or a
//! failure.
//!
//! While the summed check or the verify runs, the client calls
//! `dspi_measure_tune_tick` about every 50 ms off the UI thread (instead of
//! `dspi_measure_tick`; also while a tune is applied, for the guard's timer)
//! and pushes capture with `dspi_measure_capture_push` whenever the session
//! wants it.

use std::ffi::c_char;

use dspi_core::{DspiError, MeasureLayout, MeasureTuneOptions, Result};

use super::{json_reply, text};
use crate::FfiCore;
use crate::fns_device::arg;

/// A JSON argument as `T`; null or "" is `T`'s default.
fn json_or_default<T: serde::de::DeserializeOwned + Default>(
    p: *const c_char,
    name: &str,
) -> Result<T> {
    if p.is_null() {
        return Ok(T::default());
    }
    let t = arg(p, name)?;
    if t.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
        msg: format!("{name}: {e}"),
    })
}

/// The layout the device's state suggests: the layout, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_layout_proposal(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_layout_proposal()))
}

/// Propose a tune of `layout_json` from `measurement` (a filed
/// measurement's id or a document's text) with `options_json` (null or ""
/// for the defaults): the proposal, or a failure (`invalid` naming why).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_propose(
    core: *mut FfiCore,
    measurement: *const c_char,
    layout_json: *const c_char,
    options_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = arg(measurement, "measurement").and_then(|m| {
            let layout: MeasureLayout = json_or_default(layout_json, "layout")?;
            let options: MeasureTuneOptions = json_or_default(options_json, "options")?;
            c.measure_tune_propose(m.to_owned(), layout, options)
        });
        json_reply(r)
    })
}

/// The current proposal: `{ok, value: null}` with none.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_proposal(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.measure_tune_proposal()))
    })
}

/// Start the summed check of proposal `proposal_id`: the tune's state, or a
/// failure (`invalid`, `read_only`, `denied`, `locked`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_sum_check(
    core: *mut FfiCore,
    proposal_id: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_tune_sum_check(proposal_id))
    })
}

/// Drive the tune (about every 50 ms, off the UI thread): its state (never a
/// failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_tune_tick())))
}

/// The tune's state, without I/O (safe from the measure callback).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_state(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_tune_state())))
}

/// Apply proposal `proposal_id` guarded for `guard_s` seconds (0: the planned
/// verify plus a margin, at most 590 s): the
/// guard, or a failure (`invalid` naming the blocker, `read_only`,
/// `denied`, the setters' refusals).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_apply(
    core: *mut FfiCore,
    proposal_id: u32,
    guard_s: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_tune_apply(proposal_id, guard_s))
    })
}

/// Verify the applied tune inside its guard: the tune's state, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_verify(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_tune_verify()))
}

/// Keep the applied tune: Keep's reply, or a failure. After a verify that did
/// not pass the first Keep fails (`invalid`, `measure.tune.keep_unverified`):
/// show it, and keep only with a second Keep the user confirms.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_keep(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_tune_keep()))
}

/// Revert the applied tune: the guard, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_revert(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_tune_revert()))
}

/// Start the A/B compare of the kept tune: the compare, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_ab(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_tune_ab()))
}

/// Stop the tune's runs (and forget it unless applied): its state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tune_stop(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_tune_stop())))
}

#[cfg(test)]
mod tests_wp_p5tune {
    use std::ffi::CString;

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    fn read(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 1 << 20];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len());
        let s = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        serde_json::from_str(s.to_str().unwrap()).unwrap()
    }

    #[test]
    fn wp_p5tune_the_c_calls_carry_the_tune() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let l = read(|b, l| dspi_measure_layout_proposal(h, b, l));
        assert_eq!(l["ok"], true, "{l}");
        assert!(l["outputs"].is_array());
        // Not a measurement: refused in words; a bad layout too.
        let m = CString::new("{\"schemaVersion\":1}").unwrap();
        let lay = CString::new(l.to_string()).unwrap();
        let r = read(|b, len| {
            dspi_measure_tune_propose(h, m.as_ptr(), lay.as_ptr(), std::ptr::null(), b, len)
        });
        assert_eq!(r["code"], "invalid", "{r}");
        let bad = CString::new("{\"outputs\": 3}").unwrap();
        let r = read(|b, len| {
            dspi_measure_tune_propose(h, m.as_ptr(), bad.as_ptr(), std::ptr::null(), b, len)
        });
        assert_eq!(r["code"], "invalid", "{r}");
        let r = read(|b, len| {
            dspi_measure_tune_propose(h, std::ptr::null(), lay.as_ptr(), std::ptr::null(), b, len)
        });
        assert_eq!(r["code"], "invalid", "{r}");
        // Nothing proposed: the state is idle, every step refused.
        let p = read(|b, len| dspi_measure_tune_proposal(h, b, len));
        assert_eq!(
            (p["ok"].clone(), p["value"].clone()),
            (Value::Bool(true), Value::Null)
        );
        let st = read(|b, len| dspi_measure_tune_tick(h, b, len));
        assert_eq!(st["stage"], "idle", "{st}");
        assert_eq!(st["running"], false);
        let st = read(|b, len| dspi_measure_tune_state(h, b, len));
        assert_eq!(st["ok"], true);
        for r in [
            read(|b, len| dspi_measure_tune_sum_check(h, 1, b, len)),
            read(|b, len| dspi_measure_tune_apply(h, 1, 0, b, len)),
            read(|b, len| dspi_measure_tune_verify(h, b, len)),
            read(|b, len| dspi_measure_tune_keep(h, b, len)),
            read(|b, len| dspi_measure_tune_revert(h, b, len)),
            read(|b, len| dspi_measure_tune_ab(h, b, len)),
        ] {
            assert_eq!(r["code"], "invalid", "{r}");
        }
        let st = read(|b, len| dspi_measure_tune_stop(h, b, len));
        assert_eq!(st["stage"], "idle");
        dspi_core_free(h);
    }
}
