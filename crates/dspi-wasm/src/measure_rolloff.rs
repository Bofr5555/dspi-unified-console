//! The roll-off search on the web (WP ROLL1, owner 2026-11-22): the core's
//! `measure_rolloff_*` calls as JSON. While it runs, the Worker pushes the
//! microphone's blocks with `measureCapturePush` whenever
//! `session.wantsCapture` and calls `measureRolloffTick` on its own timer,
//! about every 50 ms, instead of `measureTick`; nothing here runs a thread.
//! Shapes are the core's records (camelCase): `MeasureRolloffSetup`,
//! `MeasureRolloffState` (its `result` a `MeasureRolloffResult`).
//!
//! - `measureRolloffStart {setup}`: the state, or a failure.
//! - `measureRolloffTick`, `measureRolloffState`, `measureRolloffStop`: the
//!   state.
//! - `measureRolloffClear`: null, or a failure while one runs.

use dspi_core::{DspiCore, MeasureRolloffSetup};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// `measureRolloffStart`'s parameters.
#[derive(Debug, Deserialize)]
struct Start {
    setup: MeasureRolloffSetup,
}

fn reply<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    r.map(|v| tidy(&v)).map_err(CallError::from)
}

/// The roll-off search's calls; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureRolloffStart" => serde_json::from_value::<Start>(p)
            .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
            .and_then(|q| reply(core.measure_rolloff_start(q.setup))),
        "measureRolloffTick" => Ok(tidy(&core.measure_rolloff_tick())),
        "measureRolloffState" => Ok(tidy(&core.measure_rolloff_state())),
        "measureRolloffStop" => Ok(tidy(&core.measure_rolloff_stop())),
        "measureRolloffClear" => reply(core.measure_rolloff_clear()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_roll1 {
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

    #[test]
    fn wp_roll1_the_search_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let st = run(h, "measureRolloffState", Value::Null);
        assert_eq!(st["value"]["stage"], "idle", "{st}");
        // The demo's volume is too high: refused; bad parameters too.
        let r = run(
            h,
            "measureRolloffStart",
            json!({ "setup": { "output": 2, "role": "high", "sweepMs": 1000 } }),
        );
        assert_eq!(r["ok"], false, "{r}");
        let r = run(h, "measureRolloffStart", json!({ "setup": 3 }));
        assert_eq!(r["ok"], false, "{r}");
        let st = run(h, "measureRolloffTick", Value::Null);
        assert_eq!(st["value"]["running"], false, "{st}");
        assert_eq!(run(h, "measureRolloffStop", Value::Null)["ok"], true);
        assert_eq!(run(h, "measureRolloffClear", Value::Null)["ok"], true);
        assert_eq!(run(h, "close", Value::Null)["ok"], true);
        session_free(h).unwrap();
    }
}
