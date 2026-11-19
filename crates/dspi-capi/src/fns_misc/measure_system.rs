//! C functions: the system measurement across units (WP ATCORE, PLAN-AT
//! 1.4 and 1.5), over `dspi_core`'s `measure::system` module.
//!
//! Arguments and replies are JSON in the core's records (camelCase): the
//! setup `MeasureSystemSetup` (`{units: [{serial, outputs: [{output, role,
//! speaker, inverted}]}], passes, anchorSerial, anchorOutput, micDocument,
//! sweepMs, gapMs, ceilingDb, splCapDb, captureRateHz, leadMs, leaseMs,
//! name}`), a unit `MeasureSystemUnit`, the checks `[MeasureSystemUnitCheck]`
//! (each from `dspi_measure_system_unit_check` on that unit's own core),
//! the state `MeasureSystemState`, the result, the proposal and what an
//! apply did (`MeasureSystemApplied`), or a failure.
//!
//! While a session runs, the client calls `dspi_measure_system_tick` about
//! every 50 ms off the UI thread and pushes capture with
//! `dspi_measure_system_capture_push` whenever the state wants it.

use std::ffi::c_char;

use dspi_core::{DspiError, MeasureSystemSetup, MeasureSystemUnit, MeasureSystemUnitCheck, Result};

use super::measure::c_floats;
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

/// Judge this core's own device as `unit_json` of `setup_json`: the check
/// to hand to the core on the hub session, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_unit_check(
    core: *mut FfiCore,
    setup_json: *const c_char,
    unit_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = json_arg::<MeasureSystemSetup>(setup_json, "setup").and_then(|s| {
            let u: MeasureSystemUnit = json_arg(unit_json, "unit")?;
            c.measure_system_unit_check(s, u)
        });
        json_reply(r)
    })
}

/// What stands between `setup_json` and a system measurement, each item
/// naming its unit, from `checks_json`: the preflight, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_preflight(
    core: *mut FfiCore,
    setup_json: *const c_char,
    checks_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = json_arg::<MeasureSystemSetup>(setup_json, "setup").and_then(|s| {
            let checks: Vec<MeasureSystemUnitCheck> = json_arg(checks_json, "checks")?;
            c.measure_system_preflight(s, checks)
        });
        json_reply(r)
    })
}

/// Start a system measurement: the state, or a failure (`invalid`,
/// `unsupported` for a hub without `system_measure`, `denied`, the hub's
/// code).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_start(
    core: *mut FfiCore,
    setup_json: *const c_char,
    checks_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = json_arg::<MeasureSystemSetup>(setup_json, "setup").and_then(|s| {
            let checks: Vec<MeasureSystemUnitCheck> = json_arg(checks_json, "checks")?;
            c.measure_system_start(s, checks)
        });
        json_reply(r)
    })
}

/// Push `count` mono samples of the microphone's capture from frame
/// `first_frame` at `rate_hz` (as `dspi_measure_capture_push`): the state,
/// or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_capture_push(
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
                .and_then(|b| c.measure_system_capture_push(first_frame, b, rate_hz, clipped)),
        )
    })
}

/// Drive the system session (about every 50 ms, off the UI thread): the
/// state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_system_tick())))
}

/// The system session's state, without I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_state(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_system_state())))
}

/// The user's Stop: the state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_stop(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_system_stop())))
}

/// The finished measurement, or `invalid` while none is ready.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_result(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_system_result()))
}

/// The alignment the result proposes, or `invalid` while none is ready.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_proposal(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_system_proposal()))
}

/// Start the summed check of every inter-unit transition, from each
/// unit's fresh check in `checks_json` (the preflight is judged again and
/// each output ramped again): the state, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_sum_check(
    core: *mut FfiCore,
    checks_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = json_arg::<Vec<MeasureSystemUnitCheck>>(checks_json, "checks")
            .and_then(|checks| c.measure_system_sum_check(checks));
        json_reply(r)
    })
}

/// Apply proposal `proposal_id` (Admin): what was done, or a failure
/// (`denied`, `invalid`, the hub's code).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_apply(
    core: *mut FfiCore,
    proposal_id: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_system_apply(proposal_id))
    })
}

/// Repeat the summed runs after an apply, from each unit's fresh check
/// in `checks_json` (as `dspi_measure_system_sum_check`): the state, or a
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_verify(
    core: *mut FfiCore,
    checks_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = json_arg::<Vec<MeasureSystemUnitCheck>>(checks_json, "checks")
            .and_then(|checks| c.measure_system_verify(checks));
        json_reply(r)
    })
}

/// Where the applied alignment stands.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_applied(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.measure_system_applied()))
    })
}

/// Keep the applied alignment: what stands, or `invalid`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_keep(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_system_keep()))
}

/// Revert the applied alignment (Admin): what was done, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_system_revert(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_system_revert()))
}

#[cfg(test)]
mod tests_wp_atcore {
    use std::ffi::CString;

    use serde_json::{Value, json};

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
    fn wp_atcore_the_c_calls_carry_the_system_session() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let setup = json!({"units": [
            {"serial": "A", "outputs": [{"output": 0}]},
            {"serial": "B", "outputs": [{"output": 0}]}
        ]});
        let s = CString::new(setup.to_string()).unwrap();
        let u =
            CString::new(json!({"serial": "A", "outputs": [{"output": 0}]}).to_string()).unwrap();
        // The demo device is not unit A: refused in words.
        let r = read(|b, l| dspi_measure_system_unit_check(h, s.as_ptr(), u.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        // No hub session: the preflight says it needs the DSPi hub.
        let none = CString::new("[]").unwrap();
        let r = read(|b, l| dspi_measure_system_preflight(h, s.as_ptr(), none.as_ptr(), b, l));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["blockers"][0]["key"], "measure.system.needs_hub", "{r}");
        let r = read(|b, l| dspi_measure_system_start(h, s.as_ptr(), none.as_ptr(), b, l));
        assert_eq!(r["ok"], false, "{r}");
        let bad = CString::new("{").unwrap();
        let r = read(|b, l| dspi_measure_system_start(h, bad.as_ptr(), none.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let st = read(|b, l| dspi_measure_system_tick(h, b, l));
        assert_eq!(st["phase"], "idle", "{st}");
        assert_eq!(
            read(|b, l| dspi_measure_system_state(h, b, l))["active"],
            false
        );
        assert_eq!(
            read(|b, l| dspi_measure_system_stop(h, b, l))["phase"],
            "idle"
        );
        let r = read(|b, l| {
            dspi_measure_system_capture_push(h, 0, std::ptr::null(), 0, 48_000, false, b, l)
        });
        assert_eq!(r["ok"], true, "{r}");
        for r in [
            read(|b, l| dspi_measure_system_result(h, b, l)),
            read(|b, l| dspi_measure_system_proposal(h, b, l)),
            read(|b, l| dspi_measure_system_sum_check(h, c"[]".as_ptr(), b, l)),
            read(|b, l| dspi_measure_system_apply(h, 1, b, l)),
            read(|b, l| dspi_measure_system_verify(h, c"[]".as_ptr(), b, l)),
            read(|b, l| dspi_measure_system_keep(h, b, l)),
            read(|b, l| dspi_measure_system_revert(h, b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
        }
        assert_eq!(
            read(|b, l| dspi_measure_system_applied(h, b, l))["stage"],
            "none"
        );
        dspi_core_free(h);
    }
}
