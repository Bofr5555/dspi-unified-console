//! The volume of a room or a zone of the installation on the web (WP FEAT2):
//! the core's `system_volume_*` calls as JSON (the shapes of the C ABI's
//! `fns_misc/system_volume.rs`).
//!
//! - `systemVolumePlan {room or zone, volume?, levelDb or stepDb}` → `{ask,
//!   name, number, of, units: [...]}`; asks the hub, writes nothing.
//! - `systemVolumeSet {plan}` → the plan, each unit's row settled.

use dspi_core::DspiCore;
use dspi_core::link::{SystemVolumeAsk, SystemVolumePlan};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

#[derive(Debug, Deserialize)]
struct SetParams {
    plan: SystemVolumePlan,
}

fn params<T: DeserializeOwned>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// The call `method` names, if it is one of these.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "systemVolumePlan" => params::<SystemVolumeAsk>(p).and_then(|a| {
            core.system_volume_plan(a)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "systemVolumeSet" => params::<SetParams>(p).and_then(|s| {
            core.system_volume_set(s.plan)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests_wp_feat2 {
    use serde_json::{Value, json};

    use crate::calls::{call, session_new};
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
    fn wp_feat2_web_volume_calls_off_a_hub() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let ask = json!({"room": "bar", "levelDb": -20.0});
        assert_eq!(run(h, "systemVolumePlan", ask)["code"], "not_open");
        for bad in [
            json!({"room": "bar"}),
            json!({"room": "bar", "zone": "in", "levelDb": -20.0}),
            json!({"room": "bar", "levelDb": 3.0}),
            json!({"room": "bar", "levelDb": "loud"}),
        ] {
            assert_eq!(run(h, "systemVolumePlan", bad)["code"], "invalid");
        }
        // A plan with nothing to write sends nothing.
        let plan = json!({"ask": {"room": "bar", "levelDb": -20.0}, "units": [
            {"unitId": "a", "serial": "S-A", "state": "unchanged", "beforeDb": -20.0}]});
        let r = run(h, "systemVolumeSet", json!({ "plan": plan }));
        assert_eq!(r["value"]["units"][0]["state"], "unchanged", "{r}");
        let mut loud = plan.clone();
        loud["units"][0]["state"] = json!("ready");
        loud["units"][0]["targetDb"] = json!(6.0);
        let r = run(h, "systemVolumeSet", json!({ "plan": loud }));
        assert_eq!(r["code"], "invalid", "{r}");
    }
}
