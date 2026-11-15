//! The web's Settings page (WP WB6i): the reads and calls the registry's
//! `write` does not cover. Device name, output latency, pins and channel names
//! are `write`s (`dev.name`, `out.latency`, `out.pin`, `ch.name`, ...); these
//! calls add what is read without a path, and the core-voltage flow (WP CG11,
//! `dspi_core::vreg`), whose limits, the 1.20 V rule and the `OVERVOLT` word
//! are the core's alone: nothing here checks a voltage or a word.
//!
//! - `deviceName`: `{supported, name}`, the name as last known (no traffic).
//! - `outputLatency`: `{supported, requestedId, effectiveId, effectiveUs,
//!   pending, options: [OutputLatencyOption]}`; `supported` false (and the
//!   rest empty) on firmware without the setting.
//! - `vregView`, `vregPoll`: the flow's `VregCalView`. Poll on open and about
//!   once a second while the page shows; a poll writes only the next step of
//!   a run the user started.
//! - `vregOpen`, `vregClose`, `vregStart {code, soakS, typed?}`,
//!   `vregKeep {soakS}`, `vregCommit {typed?, token?}`, `vregTrialEnd`,
//!   `vregSet {value, typed?, token?}` (0 is the build default),
//!   `vregTrial {value, typed?}`, `vregAuto {on}`, `vregClear`: the core's
//!   calls, each answering the view after it. `vregCommit` and `vregSet` take
//!   a `confirmToken` of `vreg_save`; a wrong word is `overvolt_word`.
//! - `vregDemoEvent {event, seconds?}`: `hang`, `reboot`, `power_cycle`,
//!   `advance` on the demo device only (`unsupported` elsewhere), for tests.

use dspi_core::{DspiCore, DspiError, VregDemoEvent};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Start {
    code: u8,
    soak_s: u32,
    #[serde(default)]
    typed: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Keep {
    soak_s: u32,
}

#[derive(Deserialize)]
struct Commit {
    #[serde(default)]
    typed: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize)]
struct Manual {
    value: u8,
    #[serde(default)]
    typed: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize)]
struct On {
    on: bool,
}

#[derive(Deserialize)]
struct Demo {
    event: String,
    #[serde(default)]
    seconds: u32,
}

fn device_name(core: &DspiCore) -> CallResult {
    match core.cached_device_name() {
        Ok(name) => Ok(json!({ "supported": true, "name": name })),
        Err(DspiError::Unsupported) => Ok(json!({ "supported": false, "name": "" })),
        Err(e) => Err(e.into()),
    }
}

/// A number field of the latency status, 0 when absent.
fn field(fields: &[dspi_core::NamedValue], name: &str) -> u32 {
    fields
        .iter()
        .find(|f| f.name == name)
        .and_then(|f| f.value.parse().ok())
        .unwrap_or(0)
}

fn output_latency(core: &DspiCore) -> CallResult {
    let status = match core.fetch_output_latency() {
        Ok(s) => s,
        Err(DspiError::Unsupported) => {
            return Ok(json!({
                "supported": false, "requestedId": 0, "effectiveId": 0,
                "effectiveUs": 0, "pending": false, "options": [],
            }));
        }
        Err(e) => return Err(e.into()),
    };
    let options = core.fetch_output_latency_options()?;
    Ok(json!({
        "supported": true,
        "requestedId": field(&status.fields, "requested_id"),
        "effectiveId": field(&status.fields, "effective_id"),
        "effectiveUs": field(&status.fields, "effective_us"),
        "pending": field(&status.fields, "pending") != 0,
        "options": tidy(&options),
    }))
}

/// Run `f`, then answer the flow's view (with the call's error, if any).
fn then_view(core: &DspiCore, r: dspi_core::Result<()>) -> CallResult {
    r?;
    Ok(tidy(&core.core_vreg_cal_view()))
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "deviceName" => device_name(core),
        "outputLatency" => output_latency(core),
        "vregView" => Ok(tidy(&core.core_vreg_cal_view())),
        "vregPoll" => Ok(tidy(&core.core_vreg_cal_poll())),
        "vregOpen" => then_view(core, core.core_vreg_cal_open()),
        "vregClose" => then_view(core, core.core_vreg_cal_close()),
        "vregStart" => params::<Start>(p)
            .and_then(|s| then_view(core, core.core_vreg_cal_start(s.code, s.soak_s, s.typed))),
        "vregKeep" => {
            params::<Keep>(p).and_then(|k| then_view(core, core.core_vreg_cal_keep(k.soak_s)))
        }
        "vregCommit" => params::<Commit>(p)
            .and_then(|c| then_view(core, core.core_vreg_cal_commit_confirmed(c.typed, c.token))),
        "vregTrialEnd" => then_view(core, core.core_vreg_trial_end()),
        "vregSet" => params::<Manual>(p).and_then(|m| {
            then_view(
                core,
                core.core_vreg_manual_set_confirmed(m.value, m.typed, m.token),
            )
        }),
        "vregTrial" => params::<Manual>(p)
            .and_then(|m| then_view(core, core.core_vreg_manual_trial(m.value, m.typed))),
        "vregAuto" => {
            params::<On>(p).and_then(|o| then_view(core, core.core_vreg_manual_auto(o.on)))
        }
        "vregClear" => then_view(core, core.core_vreg_manual_clear()),
        "vregDemoEvent" => params::<Demo>(p).and_then(|d| {
            let event = match d.event.as_str() {
                "hang" => VregDemoEvent::Hang,
                "reboot" => VregDemoEvent::Reboot,
                "power_cycle" => VregDemoEvent::PowerCycle,
                "advance" => VregDemoEvent::AdvanceUptime,
                other => {
                    return Err(CallError::new("invalid", format!("no demo event {other}")));
                }
            };
            core.core_vreg_demo_event(event, d.seconds)?;
            Ok(Value::Null)
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_wb6i {
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

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    fn demo() -> u32 {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        h
    }

    #[test]
    fn wp_wb6i_device_name_and_latency_through_the_core() {
        let h = demo();
        let n = ok(h, "deviceName", Value::Null);
        assert_eq!(n["supported"], true, "{n}");
        ok(
            h,
            "write",
            json!({ "path": "dev.name", "value": { "text": "Stage left" } }),
        );
        assert_eq!(ok(h, "deviceName", Value::Null)["name"], "Stage left");
        assert_eq!(
            ok(h, "state", Value::Null)["device"]["name"],
            "Stage left",
            "the snapshot carries it"
        );

        let l = ok(h, "outputLatency", Value::Null);
        assert_eq!(l["supported"], true, "{l}");
        let options = l["options"].as_array().unwrap();
        assert!(!options.is_empty(), "{l}");
        assert!(options[0]["latencyUs"].as_u64().is_some(), "{l}");
        let id = options
            .iter()
            .map(|o| o["id"].as_u64().unwrap())
            .find(|&i| i != l["requestedId"].as_u64().unwrap())
            .unwrap();
        ok(
            h,
            "write",
            json!({ "path": "out.latency", "value": { "choice": id } }),
        );
        let after = ok(h, "outputLatency", Value::Null);
        assert_eq!(after["requestedId"].as_u64(), Some(id), "{after}");
        // Not a call of this module: left to the others.
        assert_eq!(run(h, "nothingLikeThis", Value::Null)["ok"], false);
    }

    #[test]
    fn wp_wb6i_vreg_flow_on_the_demo_device() {
        let h = demo();
        let v = ok(h, "vregPoll", Value::Null);
        assert_eq!(v["available"], true, "{v}");
        assert_eq!(v["phase"], "idle", "{v}");
        assert_eq!(v["confirmationWord"], "OVERVOLT");
        let v = ok(h, "vregOpen", Value::Null);
        assert_eq!(v["phase"], "review", "{v}");
        let choices = v["choices"].as_array().unwrap().clone();
        assert!(!choices.is_empty(), "{v}");
        // The core's rule, not ours: a choice that needs the word, started without it.
        if let Some(c) = choices.iter().find(|c| c["needsWord"] == true) {
            let r = run(
                h,
                "vregStart",
                json!({ "code": c["code"], "soakS": v["defaultSoakS"] }),
            );
            assert_eq!(r["code"], "overvolt_word", "{r}");
            assert_eq!(r["noticeKey"], "overvolt_word", "{r}");
        }
        // A soak under the minimum: the core refuses it.
        let low = choices[0]["code"].clone();
        let r = run(h, "vregStart", json!({ "code": low, "soakS": 1 }));
        assert_eq!(r["code"], "invalid", "{r}");
        // A save without a token: refused, nothing sent.
        let r = run(h, "vregSet", json!({ "value": 0 }));
        assert_eq!(r["code"], "confirmation_required", "{r}");
        ok(h, "vregClose", Value::Null);
        // Auto and clear answer the view.
        let v = ok(h, "vregAuto", json!({ "on": false }));
        assert!(v["phase"].is_string(), "{v}");
        ok(h, "vregClear", Value::Null);
        // Bad parameters and an unknown demo event.
        assert_eq!(run(h, "vregStart", json!({}))["code"], "invalid");
        assert_eq!(
            run(h, "vregDemoEvent", json!({ "event": "melt" }))["code"],
            "invalid"
        );
        ok(
            h,
            "vregDemoEvent",
            json!({ "event": "advance", "seconds": 5 }),
        );
    }

    #[test]
    fn wp_wb6i_vreg_trial_found_then_ended() {
        let h = demo();
        let v = ok(h, "vregPoll", Value::Null);
        let c = v["choices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["needsWord"] == false)
            .cloned()
            .expect("a choice without the word on the demo");
        // A manual trial shows as a found trial on the next poll.
        ok(h, "vregTrial", json!({ "value": c["code"] }));
        let v = ok(h, "vregPoll", Value::Null);
        assert_eq!(v["phase"], "trial_found", "{v}");
        assert_eq!(v["status"]["trialActive"], true, "{v}");
        ok(h, "vregTrialEnd", Value::Null);
        let mut v = ok(h, "vregPoll", Value::Null);
        for _ in 0..5 {
            if v["phase"] == "ended" {
                break;
            }
            v = ok(h, "vregPoll", Value::Null);
        }
        assert_eq!(v["phase"], "ended", "{v}");
        assert_eq!(v["status"]["trialActive"], false, "{v}");
    }

    #[test]
    fn wp_wb6i_without_a_device() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "deviceName", Value::Null)["code"], "not_open");
        assert_eq!(run(h, "outputLatency", Value::Null)["code"], "not_open");
        let v = ok(h, "vregPoll", Value::Null);
        assert_eq!(v["available"], false, "{v}");
        assert_eq!(run(h, "vregOpen", Value::Null)["ok"], false);
    }
}
