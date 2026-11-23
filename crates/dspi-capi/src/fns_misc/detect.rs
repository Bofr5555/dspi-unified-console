//! C functions: the speaker detection (WP ADET0, AUTODES), over
//! `dspi_core`'s `measure` module: on a device whose speakers nobody has set
//! up, which outputs reach a driver and which drive a tweeter, played quietly
//! from the top down; the speaker layout written as each output completes.
//!
//! The setup is JSON (`DetectSetup`, camelCase: `{outputs, placement,
//! coupler, floorHz, micDocument, splCapDb, captureRateHz}`; every field
//! optional). Every reply is the detection's state `{ok, stage ("idle",
//! "ramp", "shape", "descent", "done", "stopped"), running, output,
//! outputsDone, outputsTotal, floorHz, segmentHz, levelDbfs, session,
//! detection, issue, actor}`, its `detection` `{atMs, by, placement, floorHz,
//! microphone, outputs, stopped, summary, deviceOutputs}` with per output
//! `{output, class ("tweeter", "midrange", "woofer", "sub", "fullRange",
//! "nothing", "unknown", "off"), confidence ("high", "medium", "low"), role,
//! floorHz, heard ("top", "aboveFloor", "belowFloor", "never"), levelDbfs,
//! topFallDb, lowTurnDb, fLowHz, lowestHz, stoppedAtHz, note, probedMs}`, or
//! a failure.
//!
//! While it runs, the client calls `dspi_detect_tick` about every 50 ms off
//! the UI thread (instead of `dspi_measure_tick`) and pushes capture with
//! `dspi_measure_capture_push` whenever `session.wantsCapture`.

use std::ffi::c_char;

use dspi_core::{DetectSetup, DspiError, Result};

use super::{done_reply, json_reply, text};
use crate::FfiCore;
use crate::fns_device::arg;

/// The setup from its JSON text.
fn setup(p: *const c_char) -> Result<DetectSetup> {
    let t = arg(p, "setup")?;
    serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
        msg: format!("the detection setup: {e}"),
    })
}

/// Start the speaker detection: the state, or a failure (`invalid` naming
/// the blockers or `detect.confirmed_layout:`, `read_only`, `denied`,
/// `locked`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_start(
    core: *mut FfiCore,
    setup_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(setup(setup_json).and_then(|s| c.detect_start(s)))
    })
}

/// Drive the detection (about every 50 ms, off the UI thread): its state
/// (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.detect_tick())))
}

/// The detection's state, without I/O (safe from the measure callback).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_state(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.detect_state())))
}

/// The user's Stop: the state (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_stop(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.detect_stop())))
}

/// Forget a finished detection's state: `{ok}`, or `invalid` while one runs.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_clear(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| done_reply(c.detect_clear()))
}

/// Whether AI agents may detect the speakers under "never ask": `{ok,
/// value}` (Settings › AI Agents; off by default).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_agents_allowed(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.detect_agents_allowed()))
    })
}

/// The user's switch for [`dspi_detect_agents_allowed`]: `{ok}`, or
/// `confirmation_required` inside an agent's scope.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_detect_set_agents_allowed(
    core: *mut FfiCore,
    allowed: bool,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(c.detect_set_agents_allowed(allowed))
    })
}

#[cfg(test)]
mod tests_wp_adet0 {
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
    fn wp_adet0_the_c_calls_carry_the_detection() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let s = read(|b, l| dspi_detect_state(h, b, l));
        assert_eq!(s["stage"], "idle", "{s}");
        // The demo's volume is too high to measure: refused, nothing runs.
        let setup = CString::new(r#"{"outputs":[0,1]}"#).unwrap();
        let r = read(|b, l| dspi_detect_start(h, setup.as_ptr(), b, l));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "invalid");
        let bad = CString::new(r#"{"floorHz":500}"#).unwrap();
        let r = read(|b, l| dspi_detect_start(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let bad = CString::new("{").unwrap();
        let r = read(|b, l| dspi_detect_start(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid");
        let s = read(|b, l| dspi_detect_tick(h, b, l));
        assert_eq!(s["running"], false, "{s}");
        let s = read(|b, l| dspi_detect_stop(h, b, l));
        assert_eq!(s["ok"], true);
        let r = read(|b, l| dspi_detect_clear(h, b, l));
        assert_eq!(r["ok"], true, "{r}");
        // The never-ask switch: off by default, the user's to set.
        let r = read(|b, l| dspi_detect_agents_allowed(h, b, l));
        assert_eq!(r["value"], false, "{r}");
        let r = read(|b, l| dspi_detect_set_agents_allowed(h, true, b, l));
        assert_eq!(r["ok"], true, "{r}");
        let r = read(|b, l| dspi_detect_agents_allowed(h, b, l));
        assert_eq!(r["value"], true, "{r}");
        dspi_core_free(h);
    }
}
