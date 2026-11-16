//! The measurement session on the web (WP P5SES, PLAN-P5 sections 1.1 to
//! 1.5): the core's `measure_*` calls as JSON. The Worker pushes the
//! microphone's blocks (`measureCapturePush`, from the capture worklet's
//! MessagePort) and calls `measureTick` on its own timer, about every 50 ms;
//! nothing here runs a thread. Shapes are the core's records (camelCase):
//! `MeasureSetup`, `MeasurePreflight`, `MeasureState`.

use dspi_core::{DspiCore, MeasureSetup};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

/// `measurePreflight` and `measureStart`'s parameters: the setup.
#[derive(Debug, Deserialize)]
pub struct SetupParams {
    pub setup: MeasureSetup,
}

/// `measureCapturePush`'s parameters: one block of the microphone's capture.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushParams {
    /// The block's first frame, the worklet's `currentFrame`.
    pub first_frame: u64,
    /// Mono samples, full scale ±1.
    pub samples: Vec<f32>,
    /// The capture's rate, Hz.
    pub rate_hz: u32,
    /// The converter clipped.
    #[serde(default)]
    pub clipped: bool,
}

/// `measurePreflight`: `{clear, blockers, warnings}`.
pub fn preflight(core: &DspiCore, p: SetupParams) -> Result<Value, CallError> {
    Ok(tidy(&core.measure_preflight(p.setup)?))
}

/// `measureStart`: the state.
pub fn start(core: &DspiCore, p: SetupParams) -> Result<Value, CallError> {
    Ok(tidy(&core.measure_start(p.setup)?))
}

/// `measureCapturePush`: the state.
pub fn push(core: &DspiCore, p: PushParams) -> Result<Value, CallError> {
    Ok(tidy(&core.measure_capture_push(
        p.first_frame,
        p.samples,
        p.rate_hz,
        p.clipped,
    )?))
}

/// `measureTick`: the state.
pub fn tick(core: &DspiCore) -> Value {
    tidy(&core.measure_tick())
}

/// `measureNextPosition`: the state.
pub fn next_position(core: &DspiCore) -> Result<Value, CallError> {
    Ok(tidy(&core.measure_next_position()?))
}

/// `measureStop`: the state.
pub fn stop(core: &DspiCore) -> Value {
    tidy(&core.measure_stop())
}

/// `measureState`: the state.
pub fn state(core: &DspiCore) -> Value {
    tidy(&core.measure_state())
}

/// `measureResult`: the `measurement` document's text.
pub fn result(core: &DspiCore) -> Result<Value, CallError> {
    Ok(Value::String(core.measure_result()?))
}

/// `measureClear`: forget a finished session.
pub fn clear(core: &DspiCore) -> Result<Value, CallError> {
    core.measure_clear()?;
    Ok(Value::Null)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5ses {
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

    /// A whole session driven by the caller's calls, as the Worker drives
    /// it: the demo device, a microphone that hears a tone while the
    /// generator plays, pink noise to the end, the document.
    #[test]
    fn wp_p5ses_a_whole_session_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        ok(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -20.0 } }),
        );
        let setup = json!({ "setup": {
            "kind": "pink",
            "outputs": [{ "output": 0 }],
            "pinkMs": 1000,
        }});
        let p = ok(h, "measurePreflight", setup.clone());
        assert_eq!(p["clear"], true, "{p}");
        let st = ok(h, "measureStart", setup);
        assert_eq!(st["active"], true);
        // The caller's loop: push what the clock says, tick.
        let rate = 48_000u64;
        let t0 = Instant::now();
        let mut next = 5_000u64;
        let mut phase = 0.0f64;
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let end = t0 + Duration::from_secs(60);
        let mut st = Value::Null;
        while Instant::now() < end {
            let due = 5_000 + (t0.elapsed().as_secs_f64() * rate as f64) as u64;
            while next < due {
                let level = st["levelDb"].as_f64();
                let amp = level.map_or(0.0, |l| 10f64.powf((l - 20.0) / 20.0));
                let samples: Vec<f32> = (0..960)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        phase += std::f64::consts::TAU * 1_000.0 / rate as f64;
                        let n = ((seed >> 40) as f64 / (1u64 << 24) as f64) * 2.0 - 1.0;
                        (amp * phase.sin() + 5e-5 * n) as f32
                    })
                    .collect();
                ok(
                    h,
                    "measureCapturePush",
                    json!({ "firstFrame": next, "samples": samples, "rateHz": rate }),
                );
                next += 960;
            }
            st = ok(h, "measureTick", Value::Null);
            if st["active"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(st["phase"], "done", "{st}");
        let doc = ok(h, "measureResult", Value::Null);
        let doc: Value = serde_json::from_str(doc.as_str().unwrap()).unwrap();
        assert_eq!(doc["meta"]["kind"], "measurement");
        assert_eq!(doc["measurement"]["stimulus"]["kind"], "pink");
        assert_eq!(ok(h, "measureState", Value::Null)["resultReady"], true);
        ok(h, "measureClear", Value::Null);
        assert_eq!(ok(h, "measureState", Value::Null)["phase"], "idle");
        // A setup that is not one is the binding's `invalid`.
        let r = run(h, "measureStart", json!({ "setup": { "outputs": [] } }));
        assert_eq!(r["code"], "invalid", "{r}");
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
