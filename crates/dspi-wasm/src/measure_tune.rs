//! The auto-tune on the web (WP P5TUNE, PLAN-P5 section 1.4): the core's
//! `measure_layout_proposal` and `measure_tune_*` calls as JSON. The
//! measurement is a document's text (the web keeps documents on the hub or
//! as files) or a hub document's id. While the summed check or the verify
//! runs, the Worker pushes the microphone's blocks with `measureCapturePush`
//! and calls `measureTuneTick` on its own timer, about every 50 ms, instead
//! of `measureTick` (also while a tune is applied, for the guard's timer);
//! nothing here runs a thread. Shapes are the core's records (camelCase):
//! `MeasureLayout`, `MeasureTuneOptions`, `MeasureTuneProposal`,
//! `MeasureTuneState`, `MeasureTuneKeep`, the guard and the A/B compare.
//!
//! - `measureLayoutProposal`: the layout.
//! - `measureTunePropose {measurement, layout, options?}`: the proposal.
//! - `measureTuneProposal`: the current proposal, or null.
//! - `measureTuneSumCheck {proposalId}`, `measureTuneTick`,
//!   `measureTuneState`, `measureTuneVerify`, `measureTuneStop`: the state.
//! - `measureTuneApply {proposalId, guardS?}`, `measureTuneRevert`: the guard.
//! - `measureTuneKeep`: Keep's record (`measurement` holds the document with
//!   the tune's row: file it).
//! - `measureTuneAb`: the A/B compare.

use dspi_core::{DspiCore, MeasureLayout, MeasureTuneOptions};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// `measureTunePropose`'s parameters.
#[derive(Debug, Deserialize)]
struct Propose {
    measurement: String,
    layout: MeasureLayout,
    #[serde(default)]
    options: MeasureTuneOptions,
}

/// A proposal's id.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Id {
    proposal_id: u32,
}

/// `measureTuneApply`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Apply {
    proposal_id: u32,
    #[serde(default)]
    guard_s: u32,
}

fn reply<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    r.map(|v| tidy(&v)).map_err(CallError::from)
}

/// The auto-tune's calls; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureLayoutProposal" => reply(core.measure_layout_proposal()),
        "measureTunePropose" => params::<Propose>(p)
            .and_then(|q| reply(core.measure_tune_propose(q.measurement, q.layout, q.options))),
        "measureTuneProposal" => Ok(tidy(&core.measure_tune_proposal())),
        "measureTuneSumCheck" => {
            params::<Id>(p).and_then(|q| reply(core.measure_tune_sum_check(q.proposal_id)))
        }
        "measureTuneTick" => Ok(tidy(&core.measure_tune_tick())),
        "measureTuneState" => Ok(tidy(&core.measure_tune_state())),
        "measureTuneApply" => params::<Apply>(p)
            .and_then(|q| reply(core.measure_tune_apply(q.proposal_id, q.guard_s))),
        "measureTuneVerify" => reply(core.measure_tune_verify()),
        "measureTuneKeep" => reply(core.measure_tune_keep()),
        "measureTuneRevert" => reply(core.measure_tune_revert()),
        "measureTuneAb" => reply(core.measure_tune_ab()),
        "measureTuneStop" => Ok(tidy(&core.measure_tune_stop())),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p5tune {
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

    #[test]
    fn wp_p5tune_the_tune_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let layout = ok(h, "measureLayoutProposal", Value::Null);
        assert!(layout["outputs"].is_array(), "{layout}");
        // Not a measurement: refused; bad parameters too.
        let r = run(
            h,
            "measureTunePropose",
            json!({ "measurement": "{\"schemaVersion\":1}", "layout": layout }),
        );
        assert_eq!(r["ok"], false, "{r}");
        let r = run(h, "measureTunePropose", json!({ "layout": 3 }));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(ok(h, "measureTuneProposal", Value::Null), Value::Null);
        let st = ok(h, "measureTuneTick", Value::Null);
        assert_eq!(st["stage"], "idle", "{st}");
        assert_eq!(ok(h, "measureTuneState", Value::Null)["running"], false);
        for (m, p) in [
            ("measureTuneSumCheck", json!({ "proposalId": 1 })),
            ("measureTuneApply", json!({ "proposalId": 1 })),
            ("measureTuneVerify", Value::Null),
            ("measureTuneKeep", Value::Null),
            ("measureTuneRevert", Value::Null),
            ("measureTuneAb", Value::Null),
        ] {
            assert_eq!(run(h, m, p)["ok"], false, "{m}");
        }
        assert_eq!(ok(h, "measureTuneStop", Value::Null)["stage"], "idle");
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
