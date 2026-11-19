//! The system measurement across units on the web (WP ATCORE, PLAN-AT 1.4
//! and 1.5): the core's `measure_system_*` calls as JSON. The Worker on the
//! hub session runs the session; each unit's own Worker answers
//! `measureSystemUnitCheck` for its unit. While a session runs, the Worker
//! pushes the microphone's blocks with `measureSystemCapturePush` and calls
//! `measureSystemTick` on its own timer, about every 50 ms; nothing here runs
//! a thread. Shapes are the core's records (camelCase).
//!
//! - `measureSystemUnitCheck {setup, unit}`: this device as one unit.
//! - `measureSystemPreflight {setup, checks}`, `measureSystemStart {setup,
//!   checks}`: the preflight, the state.
//! - `measureSystemCapturePush {firstFrame, samples, rateHz, clipped?}`,
//!   `measureSystemTick`, `measureSystemState`, `measureSystemStop`: the
//!   state.
//! - `measureSystemResult`, `measureSystemProposal`, `measureSystemSumCheck
//!   {checks}`, `measureSystemApply {proposalId}`, `measureSystemVerify
//!   {checks}` (each unit's fresh check: a summed session judges the
//!   preflight and ramps again), `measureSystemApplied`,
//!   `measureSystemKeep`, `measureSystemRevert`.

use dspi_core::{DspiCore, MeasureSystemSetup, MeasureSystemUnit, MeasureSystemUnitCheck};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};
use crate::measure::PushParams;

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// `measureSystemUnitCheck`'s parameters.
#[derive(Debug, Deserialize)]
struct UnitCheck {
    setup: MeasureSystemSetup,
    unit: MeasureSystemUnit,
}

/// `measureSystemPreflight` and `measureSystemStart`'s parameters.
#[derive(Debug, Deserialize)]
struct Start {
    setup: MeasureSystemSetup,
    #[serde(default)]
    checks: Vec<MeasureSystemUnitCheck>,
}

/// Each unit's fresh check, for a summed session.
#[derive(Debug, Deserialize)]
struct Checks {
    #[serde(default)]
    checks: Vec<MeasureSystemUnitCheck>,
}

/// A proposal's id.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Id {
    proposal_id: u32,
}

fn reply<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    r.map(|v| tidy(&v)).map_err(CallError::from)
}

/// The system measurement's calls; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureSystemUnitCheck" => params::<UnitCheck>(p)
            .and_then(|q| reply(core.measure_system_unit_check(q.setup, q.unit))),
        "measureSystemPreflight" => {
            params::<Start>(p).and_then(|q| reply(core.measure_system_preflight(q.setup, q.checks)))
        }
        "measureSystemStart" => {
            params::<Start>(p).and_then(|q| reply(core.measure_system_start(q.setup, q.checks)))
        }
        "measureSystemCapturePush" => params::<PushParams>(p).and_then(|q| {
            reply(core.measure_system_capture_push(q.first_frame, q.samples, q.rate_hz, q.clipped))
        }),
        "measureSystemTick" => Ok(tidy(&core.measure_system_tick())),
        "measureSystemState" => Ok(tidy(&core.measure_system_state())),
        "measureSystemStop" => Ok(tidy(&core.measure_system_stop())),
        "measureSystemResult" => reply(core.measure_system_result()),
        "measureSystemProposal" => reply(core.measure_system_proposal()),
        "measureSystemSumCheck" => {
            params::<Checks>(p).and_then(|q| reply(core.measure_system_sum_check(q.checks)))
        }
        "measureSystemApply" => {
            params::<Id>(p).and_then(|q| reply(core.measure_system_apply(q.proposal_id)))
        }
        "measureSystemVerify" => {
            params::<Checks>(p).and_then(|q| reply(core.measure_system_verify(q.checks)))
        }
        "measureSystemApplied" => Ok(tidy(&core.measure_system_applied())),
        "measureSystemKeep" => reply(core.measure_system_keep()),
        "measureSystemRevert" => reply(core.measure_system_revert()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_atcore {
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
    fn wp_atcore_the_system_session_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let setup = json!({"units": [
            {"serial": "A", "outputs": [{"output": 0}]},
            {"serial": "B", "outputs": [{"output": 0}]}
        ]});
        // No hub session here: the notice, and a refused start.
        let pre = ok(
            h,
            "measureSystemPreflight",
            json!({"setup": setup, "checks": []}),
        );
        assert_eq!(
            pre["blockers"][0]["key"], "measure.system.needs_hub",
            "{pre}"
        );
        let r = run(h, "measureSystemStart", json!({"setup": setup}));
        assert_eq!(r["ok"], false, "{r}");
        let r = run(
            h,
            "measureSystemUnitCheck",
            json!({"setup": setup, "unit": setup["units"][0]}),
        );
        assert_eq!(r["ok"], false, "the demo device is not unit A: {r}");
        assert_eq!(
            run(h, "measureSystemStart", json!({"setup": 3}))["ok"],
            false
        );
        assert_eq!(ok(h, "measureSystemTick", Value::Null)["phase"], "idle");
        assert_eq!(ok(h, "measureSystemState", Value::Null)["active"], false);
        assert_eq!(ok(h, "measureSystemStop", Value::Null)["phase"], "idle");
        let push = json!({"firstFrame": 0, "samples": [0.0, 0.0], "rateHz": 48000});
        assert_eq!(ok(h, "measureSystemCapturePush", push)["phase"], "idle");
        for (m, p) in [
            ("measureSystemResult", Value::Null),
            ("measureSystemProposal", Value::Null),
            ("measureSystemSumCheck", json!({"checks": []})),
            ("measureSystemApply", json!({ "proposalId": 1 })),
            ("measureSystemVerify", json!({"checks": []})),
            ("measureSystemKeep", Value::Null),
            ("measureSystemRevert", Value::Null),
        ] {
            assert_eq!(run(h, m, p)["ok"], false, "{m}");
        }
        assert_eq!(ok(h, "measureSystemApplied", Value::Null)["stage"], "none");
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
