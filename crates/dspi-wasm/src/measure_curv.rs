//! The passive microphone RTA and a measurement document's curves on the web
//! (WP P5CURV): the core's `measure_rta_passive_*` calls and the free
//! `measure_doc_curves` as JSON. The passive RTA never touches the device, so
//! the Viewer's and the read-only page's screens offer it; the Worker pushes
//! the microphone's blocks with `measureRtaPassivePush` (no session, no
//! tick). Shapes are the core's records (camelCase).
//!
//! - `measureRtaPassiveStart {setup: {micDocument, captureRateHz, averaging,
//!   bandsPerOctave}}`, `measureRtaPassivePush {samples, rateHz}`,
//!   `measureRtaPassiveHold {on}`, `measureRtaPassiveAveraging {averaging}`,
//!   `measureRtaPassiveStop`, `measureRtaPassiveState`: the state `{active,
//!   captureRateHz, bandsPerOctave, averaging, frames, nonfiniteSamples, held,
//!   spl}`; `measureRtaPassiveFrame`: `{state, rta}` (`rta` as the live
//!   frame's).
//! - `measureDocCurves {text, options: {smoothing, target, targetDocument}}`:
//!   `{smoothing, timingReliable, outputs: [{output, role, speaker, measured,
//!   target, spreadDb}]}` on the standard grid (1/96 octave).
//!
//! A refusal is code `invalid` with the reason in words.

use dspi_core::{DspiCore, MeasureDocCurvesOptions, MeasureRtaAveraging, MeasureRtaPassiveSetup};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

#[derive(Debug, Deserialize)]
struct Start {
    setup: MeasureRtaPassiveSetup,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Push {
    samples: Vec<f32>,
    rate_hz: u32,
}

#[derive(Debug, Deserialize)]
struct Hold {
    on: bool,
}

#[derive(Debug, Deserialize)]
struct Avg {
    averaging: MeasureRtaAveraging,
}

#[derive(Debug, Deserialize)]
struct Curves {
    text: String,
    #[serde(default)]
    options: MeasureDocCurvesOptions,
}

/// The calls of this module; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureRtaPassiveStart" => params::<Start>(p).and_then(|s| {
            core.measure_rta_passive_start(s.setup)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureRtaPassivePush" => params::<Push>(p).and_then(|q| {
            core.measure_rta_passive_push(q.samples, q.rate_hz)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureRtaPassiveFrame" => Ok(tidy(&core.measure_rta_passive_frame())),
        "measureRtaPassiveState" => Ok(tidy(&core.measure_rta_passive_state())),
        "measureRtaPassiveHold" => params::<Hold>(p).and_then(|q| {
            core.measure_rta_passive_hold(q.on)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureRtaPassiveAveraging" => params::<Avg>(p).and_then(|q| {
            core.measure_rta_passive_averaging(q.averaging)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureRtaPassiveStop" => Ok(tidy(&core.measure_rta_passive_stop())),
        "measureDocCurves" => params::<Curves>(p).and_then(|q| {
            dspi_core::measure_doc_curves(q.text, q.options)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5curv {
    use serde_json::{Value, json};

    use crate::calls::{call, session_free, session_new};
    use crate::guard;

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    /// The passive RTA in the page's read-only mode on the demo device, as
    /// the Worker drives it; the curves' refusal for a non-measurement.
    #[test]
    fn wp_p5curv_the_passive_rta_through_the_calls_read_only() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        ok(h, "setClientReadOnly", json!({ "on": true }));
        let st = ok(
            h,
            "measureRtaPassiveStart",
            json!({ "setup": { "captureRateHz": 48000, "averaging": "infinite" } }),
        );
        assert_eq!(st["active"], true, "{st}");
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut st = Value::Null;
        for _ in 0..60 {
            let samples: Vec<f32> = (0..960)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (((seed >> 40) as f64 / (1u64 << 24) as f64) * 0.2 - 0.1) as f32
                })
                .collect();
            st = ok(
                h,
                "measureRtaPassivePush",
                json!({ "samples": samples, "rateHz": 48000 }),
            );
        }
        assert!(st["frames"].as_u64().unwrap() > 0, "{st}");
        let f = ok(h, "measureRtaPassiveFrame", Value::Null);
        assert!(
            f["rta"]["current"]["magDb"].as_array().unwrap().len() > 100,
            "{f}"
        );
        assert_eq!(
            ok(h, "measureRtaPassiveHold", json!({ "on": true }))["held"],
            true
        );
        let a = ok(
            h,
            "measureRtaPassiveAveraging",
            json!({ "averaging": "slow" }),
        );
        assert_eq!(a["averaging"], "slow");
        assert_eq!(ok(h, "measureRtaPassiveStop", Value::Null)["active"], false);
        assert_eq!(
            ok(h, "measureRtaPassiveState", Value::Null)["active"],
            false
        );
        // Still read-only: a write is refused.
        let w = run(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -20.0 } }),
        );
        assert_eq!(w["ok"], false, "{w}");
        // Bad parameters and a non-measurement are `invalid`.
        assert_eq!(
            run(h, "measureRtaPassivePush", json!({}))["code"],
            "invalid"
        );
        let r = run(h, "measureDocCurves", json!({ "text": "{}" }));
        assert_eq!(r["code"], "invalid", "{r}");
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
