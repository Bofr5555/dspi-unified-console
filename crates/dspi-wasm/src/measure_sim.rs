//! The demo's simulated microphone on the web (WP P5DEMO, PLAN-P5 OD-P5q):
//! on the demo device only, P5SIM's three-way speaker in a room answers the
//! generator and feeds the measurement session from `measureTick`, so the
//! page opens no microphone and pushes nothing (a push is ignored while it is
//! on). Label it with the info's `label` ("Simulated microphone").
//!
//! - `measureSimSet {on, seed}`: on (its world drawn from `seed`, default 1)
//!   or off: `{available, on, label, world, seed, issue}`; `unsupported` over
//!   anything but the demo, `disconnected` with nothing open, `invalid` while
//!   a measurement runs.
//! - `measureSimInfo`: the same record, without I/O.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

#[derive(Deserialize)]
struct SetParams {
    on: bool,
    #[serde(default = "default_seed")]
    seed: u64,
}

fn default_seed() -> u64 {
    1
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureSimSet" => serde_json::from_value::<SetParams>(p)
            .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
            .and_then(|s| {
                core.measure_sim_set(s.on, s.seed)
                    .map(|i| tidy(&i))
                    .map_err(CallError::from)
            }),
        "measureSimInfo" => Ok(tidy(&core.measure_sim_info())),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5demo {
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

    /// The done criterion through the web binding: the demo device measured
    /// with the simulated microphone, the Worker only ticking; a timing
    /// verdict and the document.
    #[test]
    fn wp_p5demo_a_demo_measurement_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let r = run(h, "measureSimSet", json!({ "on": true }));
        assert_eq!(r["ok"], false, "{r}");
        ok(h, "open", json!({ "kind": "demo" }));
        ok(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -20.0 } }),
        );
        let info = ok(h, "measureSimInfo", Value::Null);
        assert_eq!(info["available"], true, "{info}");
        assert_eq!(info["on"], false);
        let info = ok(h, "measureSimSet", json!({ "on": true, "seed": 42 }));
        assert_eq!(info["on"], true);
        assert_eq!(info["seed"], 42);
        assert_eq!(info["label"], "Simulated microphone");
        let setup = json!({ "setup": {
            "kind": "walked",
            "outputs": [{ "output": 0, "role": "full" }, { "output": 1, "role": "full" }],
            "placement": "headphones",
            "coupler": "flatPlate",
            "sweepMs": 1000,
            "gapMs": 300,
            "startLevelOnly": true,
        }});
        assert_eq!(ok(h, "measurePreflight", setup.clone())["clear"], true);
        ok(h, "measureStart", setup);
        let end = Instant::now() + Duration::from_secs(120);
        let mut st = Value::Null;
        while Instant::now() < end {
            st = ok(h, "measureTick", Value::Null);
            if st["active"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(st["phase"], "done", "{st}");
        assert_eq!(st["timing"]["reliable"], true, "{st}");
        let doc = ok(h, "measureResult", Value::Null);
        let doc: Value = serde_json::from_str(doc.as_str().unwrap()).unwrap();
        assert_eq!(doc["meta"]["kind"], "measurement");
        assert_eq!(doc["measurement"]["outputs"].as_array().unwrap().len(), 2);
        let info = ok(h, "measureSimSet", json!({ "on": false }));
        assert_eq!(info["on"], false);
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
