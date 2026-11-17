//! The live response and direct adjustments on the web (WP P5LIVE, PLAN-P5
//! T9 and T5): the core's `measure_live_*` calls as JSON. The Worker pushes
//! the microphone's blocks with `measureCapturePush` as for a measurement and
//! calls `measureLiveTick` on its own timer, about every 50 ms, instead of
//! `measureTick`; nothing here runs a thread. Shapes are the core's records
//! (camelCase): `MeasureLiveSetup`, `MeasureLiveState`, `MeasureLiveFrame`,
//! `MeasureLiveProposal`, `MeasureLiveBand`.

use dspi_core::{DspiCore, MeasureLiveBand, MeasureLiveSetup, MeasureRtaAveraging};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// `measureLiveStart`'s parameters.
#[derive(Debug, Deserialize)]
struct Start {
    setup: MeasureLiveSetup,
}

/// `measureLivePredict`'s parameters.
#[derive(Debug, Deserialize)]
struct Predict {
    output: u8,
    bands: Vec<MeasureLiveBand>,
}

/// `measureLiveCutPeak`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CutPeak {
    output: u8,
    freq_hz: f64,
}

/// `measureLiveFitRange`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FitRange {
    output: u8,
    f1_hz: f64,
    f2_hz: f64,
}

/// `measureLiveApply`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Apply {
    proposal_id: u32,
}

/// `measureLiveHold`'s parameters.
#[derive(Debug, Deserialize)]
struct Hold {
    on: bool,
}

/// `measureLiveAveraging`'s parameters.
#[derive(Debug, Deserialize)]
struct Avg {
    averaging: MeasureRtaAveraging,
}

/// The live view's calls; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureLiveStart" => params::<Start>(p).and_then(|s| {
            core.measure_live_start(s.setup)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureLiveTick" => Ok(tidy(&core.measure_live_tick())),
        "measureLiveFrame" => Ok(tidy(&core.measure_live_frame())),
        "measureLiveState" => Ok(tidy(&core.measure_live_state())),
        "measureLiveStop" => Ok(tidy(&core.measure_live_stop())),
        "measureLivePredict" => params::<Predict>(p).and_then(|q| {
            core.measure_live_predict(q.output, q.bands)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureLiveCutPeak" => params::<CutPeak>(p).and_then(|q| {
            core.measure_live_cut_peak(q.output, q.freq_hz)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureLiveFitRange" => params::<FitRange>(p).and_then(|q| {
            core.measure_live_fit_range(q.output, q.f1_hz, q.f2_hz)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureLiveApply" => params::<Apply>(p).and_then(|q| {
            core.measure_live_apply(q.proposal_id)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureLiveDiscard" => Ok(tidy(&core.measure_live_discard())),
        "measureLiveHold" => params::<Hold>(p).and_then(|q| {
            core.measure_live_hold(q.on)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "measureLiveAveraging" => params::<Avg>(p).and_then(|q| {
            core.measure_live_averaging(q.averaging)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5live {
    use std::time::{Duration, Instant};

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

    /// The live pink RTA driven by the caller's calls, as the Worker drives
    /// it: the demo device, a microphone that hears noise while the
    /// generator plays, two runs, the frame with its RTA, hold, stop.
    #[test]
    fn wp_p5live_a_live_rta_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        ok(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -20.0 } }),
        );
        let setup = json!({ "setup": {
            "output": 0,
            "mode": "pinkRta",
            "pinkMs": 1000,
            "averaging": "infinite",
        }});
        let st = ok(h, "measureLiveStart", setup.clone());
        assert_eq!(st["active"], true, "{st}");
        // A second start is busy.
        assert_eq!(run(h, "measureLiveStart", setup)["code"], "invalid");
        let rate = 48_000u64;
        let t0 = Instant::now();
        let mut next = 5_000u64;
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let end = t0 + Duration::from_secs(60);
        let mut st = Value::Null;
        while Instant::now() < end {
            let due = 5_000 + (t0.elapsed().as_secs_f64() * rate as f64) as u64;
            while next < due {
                let level = st["session"]["levelDb"].as_f64();
                let amp = level.map_or(0.0, |l| 10f64.powf((l - 20.0) / 20.0));
                let samples: Vec<f32> = (0..960)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        let n = ((seed >> 40) as f64 / (1u64 << 24) as f64) * 2.0 - 1.0;
                        (amp * n + 5e-5 * n) as f32
                    })
                    .collect();
                ok(
                    h,
                    "measureCapturePush",
                    json!({ "firstFrame": next, "samples": samples, "rateHz": rate }),
                );
                next += 960;
            }
            st = ok(h, "measureLiveTick", Value::Null);
            if st["active"] == false || st["runs"].as_u64().unwrap_or(0) >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(st["active"], true, "{st}");
        assert!(st["runs"].as_u64().unwrap() >= 2, "re-armed: {st}");
        let f = ok(h, "measureLiveFrame", Value::Null);
        assert_eq!(f["mode"], "pinkRta");
        assert!(f["rta"]["frames"].as_u64().unwrap() > 0, "{}", f["rta"]);
        assert!(f["rta"]["current"]["magDb"].as_array().unwrap().len() > 10);
        assert!(f["target"]["freqsHz"].as_array().unwrap().len() > 100);
        ok(h, "measureLiveHold", json!({ "on": true }));
        ok(h, "measureLiveAveraging", json!({ "averaging": "fast" }));
        let f = ok(h, "measureLiveFrame", Value::Null);
        assert!(f["rta"]["held"].is_object());
        // No pending change: an apply is refused.
        let r = run(h, "measureLiveApply", json!({ "proposalId": 7 }));
        assert_eq!(r["code"], "invalid");
        let st = ok(h, "measureLiveStop", Value::Null);
        assert_eq!(st["active"], false);
        assert_eq!(ok(h, "measureLiveState", Value::Null)["active"], false);
        assert_eq!(ok(h, "measureState", Value::Null)["active"], false);
        // Bad parameters are the binding's `invalid`.
        let r = run(h, "measureLiveCutPeak", json!({ "output": 0 }));
        assert_eq!(r["code"], "invalid", "{r}");
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
