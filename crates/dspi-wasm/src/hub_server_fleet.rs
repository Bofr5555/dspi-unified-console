//! The hub's fleet families on the web (WP P3CC), over
//! `dspi_core::hub_server::fleet` (`docs/integrator/hub-api.md` sections 19
//! to 21), through this session's DSPi Link. Records out are the core's,
//! camelCase. A `target` is `{group}` or `{serials}`, one of the two.
//!
//! - `hubGroups`; `hubGroupSet {group: {id?, name, note?, members}}`,
//!   `hubGroupDelete {group}` (Admin).
//! - `hubGroupParamGet {target, path, idx?}`; `hubGroupParamSet {target,
//!   path, idx?, value, offsets?: [{serial, db}]}` (`value` a parameter
//!   value, `{"float": -20}`), `hubGroupPresetLoad {target, slot?, name?,
//!   map?: [{serial, slot?, name?}]}` (Control).
//! - `hubGroupApply {target, snapshot?, name?, document?, guard?,
//!   timeoutMs?}`, `hubGroupApplyConfirm {run}`, `hubGroupApplyCancel {run}`
//!   (Control).
//! - `hubFirmwareImages`; `hubFirmwarePut {name, data}` (`data` the image in
//!   base64), `hubFirmwareDelete {image}` (Admin); `hubRolloutStart {target,
//!   image, whileLive?}`, `hubRollouts {rollout?}`, `hubRolloutCancel
//!   {rollout}`; `hubRolloutStepNotice {step, text?}` (no I/O).
//!
//! Without the family's cap every call answers `unsupported` and sends
//! nothing; in the client read-only mode a write is `read_only`, nothing
//! sent; a `fw.*` call over a plaintext session is the hub's `tls_required`.

use dspi_core::{DspiCore, HubFleetTarget, HubGroup, HubOffset, HubPresetPick, ParamValue};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    // No parameters is every field's default.
    let v = if v.is_null() {
        Value::Object(Default::default())
    } else {
        v
    };
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    Ok(tidy(&r?))
}

#[derive(Deserialize)]
struct GroupSet {
    group: HubGroup,
}

#[derive(Deserialize)]
struct GroupId {
    group: String,
}

#[derive(Deserialize)]
struct ParamGet {
    target: HubFleetTarget,
    path: String,
    #[serde(default)]
    idx: Vec<u32>,
}

#[derive(Deserialize)]
struct ParamSet {
    target: HubFleetTarget,
    path: String,
    #[serde(default)]
    idx: Vec<u32>,
    value: ParamValue,
    #[serde(default)]
    offsets: Vec<HubOffset>,
}

#[derive(Deserialize)]
struct PresetLoad {
    target: HubFleetTarget,
    #[serde(default)]
    slot: Option<u8>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    map: Vec<HubPresetPick>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Apply {
    target: HubFleetTarget,
    #[serde(default)]
    snapshot: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    document: String,
    #[serde(default)]
    guard: bool,
    #[serde(default)]
    timeout_ms: u32,
}

#[derive(Deserialize)]
struct Run {
    run: String,
}

#[derive(Deserialize)]
struct Put {
    name: String,
    data: String,
}

#[derive(Deserialize)]
struct Image {
    image: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Start {
    target: HubFleetTarget,
    image: String,
    #[serde(default)]
    while_live: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Rollout {
    rollout: String,
}

#[derive(Deserialize)]
struct StepNotice {
    step: String,
    #[serde(default)]
    text: String,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "hubGroups" => record(core.hub_groups()),
        "hubGroupSet" => params::<GroupSet>(p).and_then(|x| record(core.hub_group_set(x.group))),
        "hubGroupDelete" => {
            params::<GroupId>(p).and_then(|x| record(core.hub_group_delete(x.group)))
        }
        "hubGroupParamGet" => params::<ParamGet>(p)
            .and_then(|x| record(core.hub_group_param_get(x.target, x.path, x.idx))),
        "hubGroupParamSet" => params::<ParamSet>(p).and_then(|x| {
            record(core.hub_group_param_set(x.target, x.path, x.idx, x.value, x.offsets))
        }),
        "hubGroupPresetLoad" => params::<PresetLoad>(p)
            .and_then(|x| record(core.hub_group_preset_load(x.target, x.slot, x.name, x.map))),
        "hubGroupApply" => params::<Apply>(p).and_then(|x| {
            record(core.hub_group_apply(
                x.target,
                x.snapshot,
                x.name,
                x.document,
                x.guard,
                x.timeout_ms,
            ))
        }),
        "hubGroupApplyConfirm" => {
            params::<Run>(p).and_then(|x| record(core.hub_group_apply_confirm(x.run)))
        }
        "hubGroupApplyCancel" => {
            params::<Run>(p).and_then(|x| record(core.hub_group_apply_cancel(x.run)))
        }
        "hubFirmwareImages" => record(core.hub_firmware_images()),
        "hubFirmwarePut" => {
            params::<Put>(p).and_then(|x| record(core.hub_firmware_put_base64(x.name, &x.data)))
        }
        "hubFirmwareDelete" => {
            params::<Image>(p).and_then(|x| record(core.hub_firmware_delete(x.image)))
        }
        "hubRolloutStart" => params::<Start>(p)
            .and_then(|x| record(core.hub_rollout_start(x.target, x.image, x.while_live))),
        "hubRollouts" => params::<Rollout>(p).and_then(|x| record(core.hub_rollouts(x.rollout))),
        "hubRolloutCancel" => {
            params::<Rollout>(p).and_then(|x| record(core.hub_rollout_cancel(x.rollout)))
        }
        "hubRolloutStepNotice" => params::<StepNotice>(p)
            .map(|x| tidy(&dspi_core::hub_rollout_step_notice(x.step, x.text))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p3cc {
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    /// The same words as the core's own calls, on the web.
    #[test]
    fn wp_p3cc_the_fleet_calls_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let target = json!({"group": "g1"});
        for (m, p) in [
            ("hubGroups", Value::Null),
            (
                "hubGroupSet",
                json!({"group": {"name": "Bar", "members": ["A"]}}),
            ),
            ("hubGroupDelete", json!({"group": "g1"})),
            (
                "hubGroupParamGet",
                json!({"target": target, "path": "vol.master"}),
            ),
            (
                "hubGroupParamSet",
                json!({"target": target, "path": "vol.master", "value": {"float": -20.0},
                    "offsets": [{"serial": "A", "db": -3.0}]}),
            ),
            (
                "hubGroupPresetLoad",
                json!({"target": {"serials": ["A"]}, "slot": 1,
                    "map": [{"serial": "A", "name": "Live"}]}),
            ),
            (
                "hubGroupApply",
                json!({"target": target, "snapshot": "S1", "guard": true, "timeoutMs": 60000}),
            ),
            ("hubGroupApplyConfirm", json!({"run": "a1"})),
            ("hubGroupApplyCancel", json!({"run": "a1"})),
            ("hubFirmwareImages", Value::Null),
            ("hubFirmwarePut", json!({"name": "x.uf2", "data": "AQID"})),
            ("hubFirmwareDelete", json!({"image": "x.uf2"})),
            (
                "hubRolloutStart",
                json!({"target": {"serials": ["A"]}, "image": "x.uf2", "whileLive": true}),
            ),
            ("hubRollouts", Value::Null),
            ("hubRolloutCancel", json!({"rollout": "ro1"})),
        ] {
            let r = run(h, m, p);
            assert_eq!(r["ok"], false, "{m}: {r}");
            assert_eq!(r["code"], "unsupported", "{m}: {r}");
            assert_eq!(r["noticeKey"], "unsupported", "{m}: {r}");
        }
        // Parameters that do not read, and the caller's own mistakes.
        let r = run(
            h,
            "hubGroupParamSet",
            json!({"target": target, "path": "vol.master"}),
        );
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(
            h,
            "hubFirmwarePut",
            json!({"name": "x.uf2", "data": "not base64!"}),
        );
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "hubGroupDelete", json!({"group": ""}));
        assert_eq!(r["code"], "invalid", "{r}");
        let core_says = dspi_core::DspiCore::new()
            .hub_group_delete(String::new())
            .unwrap_err();
        assert_eq!(r["error"], core_says.to_string(), "{r}");
        // The step's notice, no I/O.
        let r = run(h, "hubRolloutStepNotice", json!({"step": "verify"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["key"], "rollout.verify", "{r}");
    }
}
