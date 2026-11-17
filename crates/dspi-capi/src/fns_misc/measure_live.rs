//! C functions: the live response and direct adjustments (WP P5LIVE, PLAN-P5
//! T9 and T5), over `dspi_core`'s `measure` module.
//!
//! The setup is JSON (`MeasureLiveSetup`, camelCase: `{output, mode ("sweep"
//! or "pinkRta"), role, placement, coupler, micDocument, targetDocument,
//! smoothing, sweepMs, gapMs, pinkMs, ceilingDb, splCapDb, captureRateHz,
//! averaging ("fast", "slow", "infinite"), bandsPerOctave}`; every field
//! optional). Every reply is JSON: the live state `{ok, active, mode, output,
//! seq, runs, levelDb, pending, issue, session}`, the frame `{ok, active,
//! mode, output, seq, measured, target, predicted, rta, pending, issue,
//! session}` (each curve `{freqsHz, magDb, valid, spl}`), a proposal `{ok,
//! id, adjust, output, label, changes, gainBeforeDb, gainAfterDb,
//! rmsBeforeDb, rmsAfterDb, notes, issue, predicted}`, or a failure.
//!
//! While live, the client calls `dspi_measure_live_tick` about every 50 ms
//! off the UI thread (instead of `dspi_measure_tick`) and pushes capture with
//! `dspi_measure_capture_push` whenever the session wants it.

use std::ffi::c_char;

use dspi_core::{DspiError, MeasureLiveBand, MeasureLiveSetup, MeasureRtaAveraging, Result};

use super::{json_reply, text};
use crate::FfiCore;
use crate::fns_device::arg;

/// A JSON argument as `T`.
fn json_arg<T: serde::de::DeserializeOwned>(p: *const c_char, name: &str) -> Result<T> {
    let t = arg(p, name)?;
    serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
        msg: format!("{name}: {e}"),
    })
}

/// Start the live view of `setup` (JSON): the live state, or a failure
/// (`invalid` naming the blockers, `read_only`, `denied`, `locked`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_start(
    core: *mut FfiCore,
    setup_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            json_arg::<MeasureLiveSetup>(setup_json, "setup").and_then(|s| c.measure_live_start(s)),
        )
    })
}

/// Drive the live view (about every 50 ms, off the UI thread): the live
/// state (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_live_tick())))
}

/// What the live view shows: the frame, without I/O (safe from the measure
/// callback).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_frame(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_live_frame())))
}

/// The live state, without I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_state(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_live_state())))
}

/// Stop the live view: the live state (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_stop(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_live_stop())))
}

/// Preview `bands_json` (a JSON array of `{band, filterType, bypass, freq, q,
/// gain, qp}`, PEQ bands of `output`) as the pending change: the proposal,
/// or a failure. An empty array drops the pending change.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_predict(
    core: *mut FfiCore,
    output: u8,
    bands_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            json_arg::<Vec<MeasureLiveBand>>(bands_json, "bands")
                .and_then(|b| c.measure_live_predict(output, b)),
        )
    })
}

/// "Cut this peak" at `freq_hz` on `output`: the proposal (its `issue` says
/// why nothing is proposed), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_cut_peak(
    core: *mut FfiCore,
    output: u8,
    freq_hz: f64,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_live_cut_peak(output, freq_hz))
    })
}

/// "Fit this range" `f1_hz..f2_hz` on `output`: the proposal, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_fit_range(
    core: *mut FfiCore,
    output: u8,
    f1_hz: f64,
    f2_hz: f64,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_live_fit_range(output, f1_hz, f2_hz))
    })
}

/// Write the pending change `proposal_id` as one undo step: the live state,
/// or a failure (`invalid` for no such change or a changed EQ, the setters'
/// refusals).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_apply(
    core: *mut FfiCore,
    proposal_id: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_live_apply(proposal_id))
    })
}

/// Drop the pending change: the live state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_discard(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_live_discard())))
}

/// Hold the RTA's spectrum (`on`) or drop the held copy: the live state, or
/// `invalid` with no RTA or nothing to hold.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_hold(
    core: *mut FfiCore,
    on: bool,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_live_hold(on)))
}

/// The RTA's averaging, `"fast"`, `"slow"` or `"infinite"`: the live state,
/// or `invalid`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_live_averaging(
    core: *mut FfiCore,
    averaging: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let a = arg(averaging, "averaging").and_then(|w| {
            serde_json::from_value::<MeasureRtaAveraging>(serde_json::Value::String(
                w.trim().to_owned(),
            ))
            .map_err(|_| DspiError::InvalidArgument {
                msg: "averaging: fast, slow or infinite".into(),
            })
        });
        json_reply(a.and_then(|a| c.measure_live_averaging(a)))
    })
}

#[cfg(test)]
mod tests_wp_p5live {
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
    fn wp_p5live_the_c_calls_carry_the_live_view() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        // The demo's volume is too high to measure: refused, in words.
        let setup = CString::new(r#"{"output":0,"sweepMs":1000,"mode":"sweep"}"#).unwrap();
        let s = read(|b, l| dspi_measure_live_start(h, setup.as_ptr(), b, l));
        assert_eq!(s["ok"], false, "{s}");
        assert_eq!(s["code"], "invalid");
        let bad = CString::new(r#"{"mode":"loud"}"#).unwrap();
        let s = read(|b, l| dspi_measure_live_start(h, bad.as_ptr(), b, l));
        assert_eq!(s["code"], "invalid");
        // Nothing running: the frame and state are idle; adjustments refused.
        let f = read(|b, l| dspi_measure_live_frame(h, b, l));
        assert_eq!(
            (f["ok"].clone(), f["active"].clone()),
            (Value::Bool(true), Value::Bool(false))
        );
        let st = read(|b, l| dspi_measure_live_tick(h, b, l));
        assert_eq!(st["active"], false);
        assert_eq!(st["session"]["phase"], "idle");
        let r = read(|b, l| dspi_measure_live_cut_peak(h, 0, 1_000.0, b, l));
        assert_eq!(r["code"], "invalid");
        let r = read(|b, l| dspi_measure_live_fit_range(h, 0, 20.0, 300.0, b, l));
        assert_eq!(r["code"], "invalid");
        let bands =
            CString::new(r#"[{"band":0,"filterType":1,"freq":1000,"q":1,"gain":-3}]"#).unwrap();
        let r = read(|b, l| dspi_measure_live_predict(h, 0, bands.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
        let r = read(|b, l| dspi_measure_live_apply(h, 1, b, l));
        assert_eq!(r["code"], "invalid");
        let r = read(|b, l| dspi_measure_live_hold(h, true, b, l));
        assert_eq!(r["code"], "invalid");
        let word = CString::new("loud").unwrap();
        let r = read(|b, l| dspi_measure_live_averaging(h, word.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
        let r = read(|b, l| dspi_measure_live_averaging(h, std::ptr::null(), b, l));
        assert_eq!(r["code"], "invalid");
        let s = read(|b, l| dspi_measure_live_discard(h, b, l));
        assert_eq!(s["ok"], true);
        let s = read(|b, l| dspi_measure_live_state(h, b, l));
        assert_eq!(s["active"], false);
        let s = read(|b, l| dspi_measure_live_stop(h, b, l));
        assert_eq!(s["ok"], true);
        dspi_core_free(h);
    }
}
