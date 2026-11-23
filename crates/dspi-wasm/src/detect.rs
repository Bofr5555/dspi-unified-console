//! The speaker detection on the web (WP ADET0, AUTODES): the core's
//! `detect_*` calls as JSON. While it runs, the Worker pushes the
//! microphone's blocks with `measureCapturePush` whenever
//! `session.wantsCapture` and calls `detectTick` on its own timer, about
//! every 50 ms, instead of `measureTick`; nothing here runs a thread. Shapes
//! are the core's records (camelCase): `DetectSetup`, `DetectState` (its
//! `detection` a `Detection`). The web has no Detect button in v1 (AUTODES
//! 5.2); the calls are here so a later page can use them.
//!
//! - `detectStart {setup}`: the state, or a failure.
//! - `detectTick`, `detectState`, `detectStop`: the state.
//! - `detectClear`: null, or a failure while one runs.
//! - `detectAgentsAllowed`: the never-ask switch; `detectSetAgentsAllowed
//!   {allowed}`: null.

use dspi_core::{DetectSetup, DspiCore};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// `detectStart`'s parameters.
#[derive(Debug, Deserialize)]
struct Start {
    setup: DetectSetup,
}

/// `detectSetAgentsAllowed`'s parameters.
#[derive(Debug, Deserialize)]
struct Allowed {
    allowed: bool,
}

fn reply<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    r.map(|v| tidy(&v)).map_err(CallError::from)
}

fn bad(e: serde_json::Error) -> CallError {
    CallError::new("invalid", format!("bad parameters: {e}"))
}

/// The speaker detection's calls; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "detectStart" => serde_json::from_value::<Start>(p)
            .map_err(bad)
            .and_then(|q| reply(core.detect_start(q.setup))),
        "detectTick" => Ok(tidy(&core.detect_tick())),
        "detectState" => Ok(tidy(&core.detect_state())),
        "detectStop" => Ok(tidy(&core.detect_stop())),
        "detectClear" => reply(core.detect_clear()),
        "detectAgentsAllowed" => Ok(Value::Bool(core.detect_agents_allowed())),
        "detectSetAgentsAllowed" => serde_json::from_value::<Allowed>(p)
            .map_err(bad)
            .and_then(|q| reply(core.detect_set_agents_allowed(q.allowed))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_adet0 {
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
    fn wp_adet0_the_detection_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let st = run(h, "detectState", Value::Null);
        assert_eq!(st["value"]["stage"], "idle", "{st}");
        // The demo's volume is too high: refused; bad parameters too.
        let r = run(h, "detectStart", json!({ "setup": { "outputs": [0, 1] } }));
        assert_eq!(r["ok"], false, "{r}");
        let r = run(h, "detectStart", json!({ "setup": 3 }));
        assert_eq!(r["ok"], false, "{r}");
        let st = run(h, "detectTick", Value::Null);
        assert_eq!(st["value"]["running"], false, "{st}");
        assert_eq!(run(h, "detectStop", Value::Null)["ok"], true);
        assert_eq!(run(h, "detectClear", Value::Null)["ok"], true);
        let r = run(h, "detectAgentsAllowed", Value::Null);
        assert_eq!(r["value"], false, "{r}");
        let r = run(h, "detectSetAgentsAllowed", json!({ "allowed": true }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(run(h, "detectAgentsAllowed", Value::Null)["value"], true);
        assert_eq!(run(h, "close", Value::Null)["ok"], true);
        session_free(h).unwrap();
    }
}
