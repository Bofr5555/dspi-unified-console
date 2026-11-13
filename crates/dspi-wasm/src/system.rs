//! The hub's audio-link system model (WP SW3, `docs/integrator/audio-link.md`):
//! the `system.*` requests of the hub this session is on, for the web console's
//! System view, scene bar, status rail and "Clock and links" panel. They go on
//! the session's own Link connection (`DspiCore::link_hub_request`), so the hub
//! judges each by this session's role: reads for every role, scenes, the system
//! mute and acknowledging an alarm for control, the model and the alignment for
//! admin. Not the unit's own audio link calls ([`crate::audio_link`]).
//!
//! `hubSystem {t, ...}` sends one of [`ALLOWED`] with the other fields beside
//! it and answers the hub's `ok` without `t` and `id`. A refusal keeps the
//! hub's meaning: `denied` (the role; the message names what), `rate_limited`
//! (`system.status` more than once a second), `invalid` (`bad_request`), else
//! `link` with the hub's words. `hubRole` is the role the hub gave this session.

use dspi_core::{DspiCore, DspiError};
use serde_json::{Value, json};

use crate::calls::CallError;

type CallResult = Result<Value, CallError>;

/// The requests a page may send: the hub's `system.*`, and nothing else.
pub const ALLOWED: [&str; 8] = [
    "system.get",
    "system.set",
    "system.status",
    "system.align",
    "system.scene.apply",
    "system.mute",
    "system.alarms",
    "system.alarm.ack",
];

/// The page's code for the hub's (or the client's) `code`.
fn page_code(code: &str) -> &'static str {
    match code {
        "denied" => "denied",
        "rate_limited" => "rate_limited",
        "bad_request" => "invalid",
        // -- fixup sw3x -- `system.set` with a stale revision
        "conflict" => "conflict",
        "timeout" => "transfer",
        _ => "link",
    }
}

/// The page's error for a hub's refusal.
fn hub_error(e: DspiError) -> CallError {
    // -- WP P1AFIX -- (review p1a F3) the core error's notice key, like every
    // other core failure (`From<DspiError>`)
    let notice_key = Some(e.notice_key());
    match e {
        DspiError::Link { code, msg } => {
            // `msg` is the client's "hub refused: <code> <words>": the words alone.
            let page = page_code(&code);
            let words = msg
                .strip_prefix("hub refused: ")
                .and_then(|m| m.strip_prefix(code.as_str()))
                .map(str::trim_start)
                .filter(|w| !w.is_empty())
                .unwrap_or(&msg);
            CallError {
                notice_key,
                ..CallError::new(page, words)
            }
        }
        other => other.into(),
    }
}

/// `hubSystem`: `params` is `{t, ...fields}`.
pub fn request(core: &DspiCore, params: Value) -> CallResult {
    let Value::Object(mut fields) = params else {
        return Err(CallError::new("invalid", "hubSystem takes {t, ...}"));
    };
    let t = match fields.remove("t") {
        Some(Value::String(t)) if ALLOWED.contains(&t.as_str()) => t,
        Some(Value::String(t)) => {
            return Err(CallError::new(
                "invalid",
                format!("not a system request: {t}"),
            ));
        }
        _ => return Err(CallError::new("invalid", "hubSystem needs t")),
    };
    fields.remove("id");
    let mut answer = core.link_hub_request(&t, fields).map_err(hub_error)?;
    if let Value::Object(m) = &mut answer {
        m.remove("t");
        m.remove("id");
    }
    Ok(answer)
}

// -- WP WB8 --
/// `sceneOutcome {scene, report, model, heldFrame?, rateHz?}`: what a recall
/// the page sent (`hubSystem` `system.scene.apply`) did, in the core's words
/// (`dspi_core::link::system::scene_recall_outcome`, the outcome every client
/// shows): `LinkSceneRecall`. No traffic.
fn scene_outcome(params: Value) -> CallResult {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct P {
        scene: String,
        report: Value,
        #[serde(default)]
        model: Value,
        #[serde(default)]
        held_frame: Option<u64>,
        #[serde(default)]
        rate_hz: Option<u32>,
    }
    let p: P = serde_json::from_value(params)
        .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))?;
    if !p.report.is_object() {
        return Err(CallError::new(
            "invalid",
            "sceneOutcome needs the hub's report",
        ));
    }
    let outcome = dspi_core::link::system::scene_recall_outcome(
        &p.scene,
        &p.report,
        &p.model,
        p.held_frame,
        p.rate_hz,
    );
    Ok(crate::calls::tidy(&outcome))
}

/// Dispatch one of this module's methods; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, params: Value) -> Option<CallResult> {
    Some(match method {
        "hubSystem" => request(core, params),
        "hubRole" => Ok(json!({ "role": core.link_hub_role() })),
        // -- WP WB8 --
        "sceneOutcome" => scene_outcome(params),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_sw3 {
    use dspi_link::client::set_caller_driven_on_this_thread;
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
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

    // -- WP WB8 -- a refused recall in the core's words, per the sender's answer
    #[test]
    fn wp_wb8_scene_outcome_words_by_the_senders_answer() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let model = json!({ "rateHz": 48000, "scenes": [{ "id": "s2", "name": "Closed" }] });
        let report = |result: &str| {
            json!({
                "plan": { "frame": 10_000, "leadFrames": 4800, "path": "superframe", "systemMute": true },
                "frameAfterWrites": 5300,
                "onTime": false,
                "writes": [{ "serial": "S", "op": "superframe", "result": result }],
            })
        };
        let outcome = |result: &str, held: Value| {
            let r = run(
                h,
                "sceneOutcome",
                json!({ "scene": "s2", "report": report(result), "model": model, "heldFrame": held }),
            );
            assert_eq!(r["ok"], true, "{r}");
            r["value"].clone()
        };
        let v = outcome("refused", json!(10_100));
        assert_eq!(v["taken"], false);
        assert_eq!(v["heldFrame"], 10_100);
        assert_eq!(
            v["message"],
            "Closed was refused: a scene is already scheduled on the sender for link frame 10100, \
             100 ms after this recall. The sender takes one scene at a time; recall Closed again \
             after that. System mute unchanged."
        );
        let v = outcome("refused", Value::Null);
        assert!(
            v["message"].as_str().unwrap().contains("still holds one"),
            "{v}"
        );
        for (result, words) in [
            ("timeout", "the sender did not answer in time"),
            (
                "locked",
                "another console is applying changes to the sender",
            ),
            ("bad_frame", "the sender's answer was"),
        ] {
            let v = outcome(result, json!(10_100));
            let m = v["message"].as_str().unwrap();
            assert!(m.starts_with("Closed was not recalled: "), "{m}");
            assert!(m.contains(words), "{result}: {m}");
            assert!(!m.contains("already scheduled"), "{m}");
            assert_eq!(v["heldFrame"], Value::Null);
        }
        let v = outcome("applied", Value::Null);
        assert_eq!(v["taken"], true);
        let r = run(h, "sceneOutcome", json!({ "scene": "s2", "report": 3 }));
        assert_eq!(r["code"], "invalid", "{r}");
    }

    #[test]
    fn wp_sw3_only_system_requests_and_only_on_a_hub() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        // No hub: not open, nothing sent.
        let r = run(h, "hubSystem", json!({ "t": "system.get" }));
        assert_eq!(r["code"], "not_open", "{r}");
        assert_eq!(run(h, "hubRole", Value::Null)["value"]["role"], "");
        // Not a system request, or none at all: refused before anything goes out.
        for bad in [
            json!({ "t": "auth.pair", "pin": "1" }),
            json!({ "t": 3 }),
            json!({}),
            json!([1]),
        ] {
            let r = run(h, "hubSystem", bad.clone());
            assert_eq!(r["code"], "invalid", "{bad}: {r}");
        }
    }

    #[test]
    fn wp_sw3_a_request_reaches_the_hub_and_its_refusal_comes_back() {
        guard::reset_for_test();
        set_caller_driven_on_this_thread(true);
        let hub = FakeHub::start(HubConfig {
            auth: Auth::None,
            ..HubConfig::default()
        })
        .unwrap();
        let h = session_new().unwrap();
        let r = run(
            h,
            "linkConnect",
            json!({ "url": format!("ws://{}", hub.addr()) }),
        );
        assert_eq!(r["ok"], true, "{r}");
        // dspi-link's fake hub has no system model: the hub's own refusal
        // (`unknown_type`, naming the request), after a round trip.
        let r = run(h, "hubSystem", json!({ "t": "system.status" }));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "link", "{r}");
        assert!(
            r["error"].as_str().unwrap().contains("system.status"),
            "{r}"
        );
        // -- WP P1AFIX -- (F3) the hub's refusal carries the notice key too
        assert_eq!(r["noticeKey"], "dlink", "{r}");
        assert_eq!(super::page_code("bad_request"), "invalid");
        assert_eq!(super::page_code("denied"), "denied");
        assert_eq!(super::page_code("conflict"), "conflict");
        assert_eq!(run(h, "close", Value::Null)["ok"], true);
        set_caller_driven_on_this_thread(false);
    }
}
