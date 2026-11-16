//! C functions: the measurement session (WP P5SES, PLAN-P5 sections 1.1 to
//! 1.5), over `dspi_core`'s `measure` module.
//!
//! The setup is JSON (`MeasureSetup`, camelCase: `{kind, outputs: [{output,
//! role, speaker, group, inverted}], placement, count, coupler, micDocument,
//! sweepMs, gapMs, passes, pinkMs, ceilingDb, splCapDb, captureRateHz,
//! name}`; every field optional but `outputs`). Every reply is JSON: the
//! state `{ok, phase, kind, active, wantsCapture, captureRateHz, position,
//! positions, run, runs, output, levelDb, rampLevels, runLevelDb, frames,
//! generatorStartFrame, generatorStarts, progress, issue, timingReliable,
//! resultReady}`, the preflight `{ok, clear, blockers, warnings}`, or a
//! failure. `dspi_measure_result` answers the document's text.
//!
//! The client pushes its microphone's blocks with `dspi_measure_capture_push`
//! and calls `dspi_measure_tick` about every 50 ms off the UI thread.

use std::ffi::{c_char, c_void};
use std::sync::Arc;

use dspi_core::{DspiError, MeasureListener, MeasureSetup, Result};

use super::{document_reply, done_reply, json_reply, text};
use crate::fns_device::arg;
use crate::{FfiCore, with_core};

/// The setup from its JSON text.
fn setup(p: *const c_char) -> Result<MeasureSetup> {
    let t = arg(p, "setup")?;
    serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
        msg: format!("the measurement setup: {e}"),
    })
}

/// `count` floats at `p`, copied; `None` for a null or misaligned pointer
/// with a count.
fn c_floats(p: *const f32, count: u32) -> Option<Vec<f32>> {
    if count == 0 {
        return Some(Vec::new());
    }
    if p.is_null() || !p.is_aligned() {
        return None;
    }
    let n = usize::try_from(count).unwrap_or(0);
    // SAFETY: a non-null, aligned buffer of `count` floats; the C caller's
    // contract.
    Some(unsafe { std::slice::from_raw_parts(p, n) }.to_vec())
}

/// A C wake-up callback for the measurement listener.
struct CMeasure {
    callback: extern "C" fn(user_data: *mut c_void),
    user_data: *mut c_void,
}

// SAFETY: the callback is called on the threads that push, tick or stop, which
// its documentation tells the caller; `user_data` is only handed back to it.
unsafe impl Send for CMeasure {}
// SAFETY: as for Send.
unsafe impl Sync for CMeasure {}

impl MeasureListener for CMeasure {
    fn on_measure(&self) {
        (self.callback)(self.user_data);
    }
}

/// Set the callback that says the session's state changed (call
/// `dspi_measure_state`); null unsets it. Called on the thread that pushed,
/// ticked or stopped, with no core lock held.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_set_callback(
    core: *mut FfiCore,
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    with_core(core, (), |h| {
        let listener = callback.map(|callback| -> Arc<dyn MeasureListener> {
            Arc::new(CMeasure {
                callback,
                user_data,
            })
        });
        h.core.measure_set_listener(listener);
    });
}

/// The preflight of the setup `setup` (JSON): `{ok, clear, blockers,
/// warnings}`, each issue `{key, output, message}`; writes nothing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_preflight(
    core: *mut FfiCore,
    setup_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(setup(setup_json).and_then(|s| c.measure_preflight(s)))
    })
}

/// Start a measurement of `setup` (JSON): the state, or a failure
/// (`invalid` naming the blockers, `read_only`, `denied`, `locked`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_start(
    core: *mut FfiCore,
    setup_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(setup(setup_json).and_then(|s| c.measure_start(s)))
    })
}

/// Push `count` mono samples (full scale ±1) of the microphone's capture
/// from frame `first_frame` at `rate_hz`; `clipped` when the converter
/// clipped. `samples` points at `count` floats (null with `count` 0). The
/// state, or a failure for a block over 1 Mi samples or a null or misaligned
/// pointer with a count.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_capture_push(
    core: *mut FfiCore,
    first_frame: u64,
    samples: *const f32,
    count: u32,
    rate_hz: u32,
    clipped: bool,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let block = c_floats(samples, count);
    text(core, buf, len, |c| {
        json_reply(
            block
                .ok_or_else(|| DspiError::InvalidArgument {
                    msg: "samples is null or misaligned".into(),
                })
                .and_then(|b| c.measure_capture_push(first_frame, b, rate_hz, clipped)),
        )
    })
}

/// Drive the session (about every 50 ms, off the UI thread): the state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_tick())))
}

/// The microphone is at the next position: the state, or `invalid` when the
/// session is not waiting for one.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_next_position(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_next_position()))
}

/// The user's Stop: the state (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_stop(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_stop())))
}

/// The session's state, without I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_state(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_state())))
}

/// The finished measurement as a `measurement` document's text, or a
/// failure (`invalid` while none is ready).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_result(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| document_reply(c.measure_result()))
}

/// Forget a finished session: `{ok}`, or `invalid` while one is active.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_clear(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| done_reply(c.measure_clear()))
}

#[cfg(test)]
mod tests_wp_p5ses {
    use std::ffi::CString;
    use std::ptr::null_mut;

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
    fn wp_p5ses_the_c_calls_carry_the_session() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let setup = CString::new(r#"{"outputs":[{"output":0}],"sweepMs":1000}"#).unwrap();
        // The demo's volume is too high to measure: the blockers in words.
        let p = read(|b, l| dspi_measure_preflight(h, setup.as_ptr(), b, l));
        assert_eq!(p["ok"], true);
        assert_eq!(p["clear"], false);
        assert!(
            p["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["key"] == "measure.volume_high"),
            "{p}"
        );
        let s = read(|b, l| dspi_measure_start(h, setup.as_ptr(), b, l));
        assert_eq!(s["ok"], false);
        assert_eq!(s["code"], "invalid");
        // No session: the state is idle, a push is ignored, no result.
        let s = read(|b, l| dspi_measure_state(h, b, l));
        assert_eq!(s["phase"], "idle");
        let samples = [0f32; 16];
        let s =
            read(|b, l| dspi_measure_capture_push(h, 0, samples.as_ptr(), 16, 48_000, false, b, l));
        assert_eq!(s["ok"], true);
        let r = read(|b, l| dspi_measure_result(h, b, l));
        assert_eq!(r["code"], "invalid");
        let bad = CString::new("{").unwrap();
        let r = read(|b, l| dspi_measure_start(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
        // A null pointer with a count is refused.
        let r =
            read(|b, l| dspi_measure_capture_push(h, 0, std::ptr::null(), 4, 48_000, false, b, l));
        assert_eq!(r["code"], "invalid");
        let s = read(|b, l| dspi_measure_stop(h, b, l));
        assert_eq!(s["ok"], true);
        dspi_measure_set_callback(h, None, null_mut());
        dspi_core_free(h);
    }
}
