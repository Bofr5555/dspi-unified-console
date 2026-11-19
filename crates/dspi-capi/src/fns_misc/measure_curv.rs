//! C functions: the passive microphone RTA and a measurement document's
//! curves (WP P5CURV), over `dspi_core`'s `measure_rta_passive_*` and the
//! free `measure_doc_curves`.
//!
//! The passive RTA's setup is JSON (`MeasureRtaPassiveSetup`, camelCase:
//! `{micDocument, captureRateHz, averaging ("fast", "slow", "infinite"),
//! bandsPerOctave}`; every field optional). Its state is `{ok, active,
//! captureRateHz, bandsPerOctave, averaging, frames, nonfiniteSamples, held,
//! spl}`, its frame `{ok, state, rta}` (`rta` as the live frame's: `{current,
//! held, compare, frames, averaging}`, each curve `{freqsHz, magDb, valid,
//! spl}`). None of these touches the device: allowed read-only and for a
//! Viewer.
//!
//! The curves' options are JSON (`{smoothing, target, targetDocument}`), the
//! reply `{ok, smoothing, timingReliable, outputs: [{output, role, speaker,
//! measured, target, spreadDb}]}` on the standard grid, or a failure.

use std::ffi::c_char;

use dspi_core::{
    DspiError, MeasureDocCurvesOptions, MeasureRtaAveraging, MeasureRtaPassiveSetup, Result,
};

use super::{guard, json_reply, text};
use crate::FfiCore;
use crate::fns_device::{arg, copy_text};

/// A JSON argument as `T`.
fn json_arg<T: serde::de::DeserializeOwned>(p: *const c_char, name: &str) -> Result<T> {
    let t = arg(p, name)?;
    serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
        msg: format!("{name}: {e}"),
    })
}

/// Start the passive RTA of `setup_json`: the state, or a failure
/// (`invalid`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_start(
    core: *mut FfiCore,
    setup_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            json_arg::<MeasureRtaPassiveSetup>(setup_json, "setup")
                .and_then(|s| c.measure_rta_passive_start(s)),
        )
    })
}

/// `count` floats at `p`, copied; `None` for a null or misaligned pointer
/// with a count (as `dspi_measure_capture_push` reads them).
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

/// Push `count` mono samples (full scale ±1) at `rate_hz` into the passive
/// RTA; `samples` points at `count` floats (null with `count` 0). The state,
/// or a failure (`invalid` for another rate than the started one, a block
/// over 1 Mi samples, a null or misaligned pointer with a count).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_push(
    core: *mut FfiCore,
    samples: *const f32,
    count: u32,
    rate_hz: u32,
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
                .and_then(|b| c.measure_rta_passive_push(b, rate_hz)),
        )
    })
}

/// The passive RTA's frame, without I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_frame(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.measure_rta_passive_frame()))
    })
}

/// The passive RTA's state, without I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_state(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.measure_rta_passive_state()))
    })
}

/// Hold (`on` nonzero) the spectrum for comparison, or drop the held copy:
/// the state, or a failure (`invalid`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_hold(
    core: *mut FfiCore,
    on: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.measure_rta_passive_hold(on != 0))
    })
}

/// The averaging preset (`averaging` a JSON string: `"fast"`, `"slow"`,
/// `"infinite"`): the state, or a failure (`invalid`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_averaging(
    core: *mut FfiCore,
    averaging: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            json_arg::<MeasureRtaAveraging>(averaging, "averaging")
                .and_then(|a| c.measure_rta_passive_averaging(a)),
        )
    })
}

/// Stop the passive RTA: the state (never a failure).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_rta_passive_stop(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(Ok(c.measure_rta_passive_stop()))
    })
}

/// A measurement document's curves (`options_json` null: none smoothed, no
/// target), or a failure. No handle needed. Returns the bytes needed
/// including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_curves(
    doc: *const c_char,
    options_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let options = if options_json.is_null() {
            Ok(MeasureDocCurvesOptions::default())
        } else {
            json_arg::<MeasureDocCurvesOptions>(options_json, "options")
        };
        copy_text(
            &json_reply(arg(doc, "doc").and_then(|d| {
                options.and_then(|o| dspi_core::measure_doc_curves(d.to_owned(), o))
            })),
            buf,
            len,
        )
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5curv {
    use std::ffi::CString;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> serde_json::Value {
        let mut buf = vec![0u8; 1 << 20];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len(), "{n}");
        let s = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        serde_json::from_str(s.to_str().unwrap()).unwrap()
    }

    #[test]
    fn wp_p5curv_the_passive_rta_and_the_curves_through_c() {
        let core = dspi_core_new();
        let setup = CString::new(r#"{"captureRateHz":48000,"averaging":"infinite"}"#).unwrap();
        let st = reply(|b, l| dspi_measure_rta_passive_start(core, setup.as_ptr(), b, l));
        assert_eq!(st["ok"], true, "{st}");
        assert_eq!(st["active"], true);
        let block: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 0.1309).sin() * 0.5)
            .collect();
        let st = reply(|b, l| {
            dspi_measure_rta_passive_push(core, block.as_ptr(), block.len() as u32, 48_000, b, l)
        });
        assert!(st["frames"].as_u64().unwrap() > 0, "{st}");
        let f = reply(|b, l| dspi_measure_rta_passive_frame(core, b, l));
        assert!(
            f["rta"]["current"]["magDb"].as_array().unwrap().len() > 100,
            "{f}"
        );
        let h = reply(|b, l| dspi_measure_rta_passive_hold(core, 1, b, l));
        assert_eq!(h["held"], true, "{h}");
        let fast = CString::new(r#""fast""#).unwrap();
        let a = reply(|b, l| dspi_measure_rta_passive_averaging(core, fast.as_ptr(), b, l));
        assert_eq!(a["averaging"], "fast", "{a}");
        let bad =
            reply(|b, l| dspi_measure_rta_passive_push(core, block.as_ptr(), 10, 44_100, b, l));
        assert_eq!(bad["ok"], false, "{bad}");
        let st = reply(|b, l| dspi_measure_rta_passive_stop(core, b, l));
        assert_eq!(st["active"], false);
        assert_eq!(
            reply(|b, l| dspi_measure_rta_passive_state(core, b, l))["active"],
            false
        );
        dspi_core_free(core);

        // Not a measurement: a failure, as JSON.
        let doc = CString::new("{}").unwrap();
        let r = reply(|b, l| dspi_measure_doc_curves(doc.as_ptr(), std::ptr::null(), b, l));
        assert_eq!(r["ok"], false, "{r}");
    }
}
