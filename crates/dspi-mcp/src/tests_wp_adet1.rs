//! WP ADET1: the speaker detection for agents (AUTODES 3–6). The tool and
//! its one grouped question with the warning, the grant that covers a second
//! detection, Revoke and Stop ending it, "never ask" only with the user's
//! switch, the refusal on a confirmed layout, the `detect` block of
//! `dspi_measure_status`, the overview's fields, the policy's trusted
//! (detected) roles and the `nothing` rule, end to end on the demo DSPi with
//! its simulated microphone.

use std::sync::Arc;

use dspi_core::measure::{MeasurePlacement, MeasureRole};
use dspi_core::{
    CoreSet, DetectClass, DetectConfidence, DetectHeard, DetectedOutput, Detection, DspiCore,
    RoleSource, SpeakerLayout, SpeakerLayoutOutput,
};
use serde_json::{Value, json};

use crate::policy::{AgentPolicy, ConfirmMode};
use crate::rate::RateLimit;
use crate::tests_wp_mcp5::{
    app_drives, app_takes, call_at, demo, err_at, held_at, layout, ok_at, status,
};
use crate::tools::measure::DETECT_WARNING;
use crate::{Server, ServerConfig};

const DETECT: &str = "dspi_detect_speakers";

/// A server whose policy the test keeps, to flip the user's switch.
fn server(set: &Arc<CoreSet>, mode: ConfirmMode) -> (Server, Arc<AgentPolicy>) {
    let policy = Arc::new(AgentPolicy::new(mode));
    let config = ServerConfig {
        policy: Arc::clone(&policy) as Arc<dyn crate::Policy>,
        write_limit: RateLimit {
            burst: 1000,
            per_s: 1.0,
        },
        read_limit: RateLimit {
            burst: 100_000,
            per_s: 1.0,
        },
        measuring: true,
        ..ServerConfig::default()
    };
    (Server::new(Arc::clone(set), config), policy)
}

fn row(output: u8, role: MeasureRole, source: RoleSource) -> SpeakerLayoutOutput {
    SpeakerLayoutOutput {
        output,
        role,
        speaker: "L".into(),
        group: 0,
        source,
    }
}

fn found(output: u8, class: DetectClass, role: Option<MeasureRole>) -> DetectedOutput {
    DetectedOutput {
        output,
        class,
        confidence: DetectConfidence::High,
        role,
        floor_hz: 1_000.0,
        heard: if class == DetectClass::Nothing {
            DetectHeard::Never
        } else {
            DetectHeard::Top
        },
        probed_ms: 1_700_000_000_000,
        ..DetectedOutput::default()
    }
}

/// The user's switches of the demo's Out 4 (output 3): off, muted, routed
/// from nothing; then a layout as the detection leaves it: outputs 0-2 the
/// left three-way (detected), nothing heard on output 3.
fn detected_layout(core: &DspiCore) {
    let st = core.get_state();
    core.set_output_enable(3, false).unwrap();
    core.set_output_mute(3, true).unwrap();
    for i in 0..st.num_input_channels {
        core.set_matrix_route(i, 3, false, 0.0, false).unwrap();
    }
    core.speaker_layout_set(SpeakerLayout {
        outputs: vec![
            row(0, MeasureRole::Low, RoleSource::Detected),
            row(1, MeasureRole::Mid, RoleSource::Detected),
            row(2, MeasureRole::High, RoleSource::Detected),
        ],
        detected: Some(Detection {
            at_ms: 1_700_000_000_000,
            by: "AI · Claude Code".into(),
            placement: MeasurePlacement::ListeningPosition,
            floor_hz: 1_000.0,
            outputs: vec![
                found(0, DetectClass::Woofer, Some(MeasureRole::Low)),
                found(1, DetectClass::Midrange, Some(MeasureRole::Mid)),
                found(2, DetectClass::Tweeter, Some(MeasureRole::High)),
                found(3, DetectClass::Nothing, None),
            ],
            summary: "1 tweeter, 1 midrange, 1 woofer; nothing heard on Out 4.".into(),
            device_outputs: st.num_output_channels,
            ..Detection::default()
        }),
        ..SpeakerLayout::default()
    })
    .unwrap();
    assert_eq!(
        core.speaker_layout().unwrap().unwrap().state(),
        dspi_core::LayoutState::AutoDetected
    );
}

fn overview(s: &mut Server, c: crate::ClientId) -> Value {
    ok_at(s, c, 0, "dspi_get_overview", json!({}))
}

// ------------------------------------------------------------ the policy

#[test]
fn wp_adet1_detected_roles_are_trusted_under_every_setting() {
    for mode in [
        ConfirmMode::AskEverything,
        ConfirmMode::AskRisky,
        ConfirmMode::NeverAsk,
    ] {
        let (set, _id, core) = demo();
        detected_layout(&core);
        let (mut s, _) = server(&set, mode);
        let c = s.connect();
        // The detected tweeter keeps its high-pass: the hard rule, worded.
        let e = err_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 3, "high_pass": null}),
        );
        assert!(
            e.contains("drives a tweeter") && e.contains("detected by AI, not yet confirmed"),
            "{mode:?}: {e}"
        );
        // The measuring tools take the detected layout as a confirmed one's.
        let p = ok_at(
            &mut s,
            c,
            0,
            "dspi_measure_preflight",
            json!({"outputs": [1, 2, 3]}),
        );
        let roles: Vec<&str> = p["roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["low", "mid", "high"], "{p}");
    }
    // A confirmed layout still says so.
    let (set, _id, core) = demo();
    layout(&core, true);
    let (mut s, _) = server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let e = err_at(
        &mut s,
        c,
        0,
        "dspi_set_crossover",
        json!({"output": 3, "high_pass": null}),
    );
    assert!(e.contains("speaker setup (confirmed)"), "{e}");
    // An unconfirmed one gives no roles: nothing to measure on.
    layout(&core, false);
    let e = err_at(&mut s, c, 0, "dspi_measure_preflight", json!({}));
    assert!(e.contains("dspi_detect_speakers"), "{e}");
}

#[test]
fn wp_adet1_nothing_heard_is_never_routed_enabled_or_unmuted_by_an_agent() {
    for mode in [
        ConfirmMode::AskEverything,
        ConfirmMode::AskRisky,
        ConfirmMode::NeverAsk,
    ] {
        let (set, _id, core) = demo();
        detected_layout(&core);
        let (mut s, _) = server(&set, mode);
        let c = s.connect();
        for (tool, args, what) in [
            (
                "dspi_set_output_enabled",
                json!({"output": 4, "enabled": true}),
                "switch it on",
            ),
            (
                "dspi_set_mute",
                json!({"output": 4, "muted": false}),
                "unmute it",
            ),
            (
                "dspi_set_route",
                json!({"input": 1, "output": 4, "on": true}),
                "route an input to it",
            ),
        ] {
            let e = err_at(&mut s, c, 0, tool, args);
            assert!(
                e.contains("Out 4") && e.contains("heard nothing") && e.contains(what),
                "{mode:?} {tool}: {e}"
            );
        }
        let st = core.get_state();
        assert!(
            !st.output_enabled[3] && st.output_muted[3],
            "nothing changed"
        );
        // Quieter is fine: muting what plays never is the rule's concern.
        let r = call_at(
            &mut s,
            c,
            0,
            "dspi_set_mute",
            json!({"output": 4, "muted": true}),
        );
        assert_eq!(r["isError"], json!(false), "{r}");
        // The overview says so.
        let o = overview(&mut s, c);
        assert_eq!(o["outputs"][3]["reaches"], json!("nothing"), "{o}");
        assert_eq!(o["outputs"][3]["role"], json!(null));
    }
    // The user includes it (a row of theirs): the rule no longer holds.
    let (set, _id, core) = demo();
    detected_layout(&core);
    let mut l = core.speaker_layout().unwrap().unwrap();
    l.outputs.push(row(3, MeasureRole::Low, RoleSource::User));
    core.speaker_layout_set(l).unwrap();
    let (mut s, _) = server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok_at(
        &mut s,
        c,
        0,
        "dspi_set_output_enabled",
        json!({"output": 4, "enabled": true}),
    );
    assert!(core.get_state().output_enabled[3]);
}

#[test]
fn wp_adet1_the_overview_says_where_each_role_came_from() {
    let (set, _id, core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    assert_eq!(overview(&mut s, c)["roles_from"], json!("none"));
    detected_layout(&core);
    let o = overview(&mut s, c);
    assert_eq!(o["roles_from"], json!("speaker_setup_auto_detected"), "{o}");
    let out2 = &o["outputs"][2];
    assert_eq!(out2["role"], json!("high"));
    assert_eq!(out2["role_source"], json!("detected"));
    assert_eq!(out2["confidence"], json!("high"));
    assert_eq!(out2["reaches"], json!("driver"));
    assert_eq!(out2["detected_at_ms"], json!(1_700_000_000_000_i64));
    assert_eq!(o["agent_may"]["detect"], json!(true), "{o}");
    // The user confirms it: their word, no confidence of the detection's.
    let mut l = core.speaker_layout().unwrap().unwrap();
    l.confirmed_by_user = true;
    l.outputs[2].source = RoleSource::User;
    core.speaker_layout_set(l).unwrap();
    let o = overview(&mut s, c);
    assert_eq!(o["roles_from"], json!("speaker_setup_confirmed"));
    assert_eq!(o["outputs"][2]["role_source"], json!("user"));
    assert_eq!(o["outputs"][2]["confidence"], json!(null));
    // An app that does not measure: no detection either.
    let (mut s, _) = {
        let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskRisky));
        let config = ServerConfig {
            policy: Arc::clone(&policy) as Arc<dyn crate::Policy>,
            measuring: false,
            ..ServerConfig::default()
        };
        (Server::new(Arc::clone(&set), config), policy)
    };
    let c = s.connect();
    assert_eq!(overview(&mut s, c)["agent_may"]["detect"], json!(false));
}

// ------------------------------------------------------------- the tool

#[test]
fn wp_adet1_the_detection_is_refused_on_a_confirmed_layout_and_without_the_switch() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let (mut s, policy) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let e = err_at(&mut s, c, 0, DETECT, json!({}));
    assert!(
        e.contains("confirmed") && e.contains("Settings › Speakers"),
        "{e}"
    );
    // Never ask: refused, and the overview says so, until the user's switch.
    layout(&core, false);
    policy.set_mode(ConfirmMode::NeverAsk);
    assert_eq!(overview(&mut s, c)["agent_may"]["detect"], json!(false));
    let e = err_at(&mut s, c, 0, DETECT, json!({"outputs": [3]}));
    assert!(e.contains("Agents may detect speakers"), "{e}");
    assert!(s.measure_requests().is_empty());
    policy.set_agent_detect(true);
    assert_eq!(overview(&mut s, c)["agent_may"]["detect"], json!(true));
    let r = ok_at(&mut s, c, 0, DETECT, json!({"outputs": [3]}));
    assert_eq!(r["status"], json!("waiting_for_app"), "{r}");
    assert!(s.grants().is_empty(), "never ask grants nothing");
}

#[test]
fn wp_adet1_no_level_argument_and_the_floor_only_rises() {
    let (set, _id, _core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    s.stop_agents(0);
    for args in [
        json!({"max_level_dbfs": -10}),
        json!({"level_dbfs": -10}),
        json!({"roles": [{"output": 0, "role": "low"}]}),
    ] {
        let e = err_at(&mut s, c, 0, DETECT, args.clone());
        assert!(e.starts_with("no argument"), "{args}: {e}");
    }
    for hz in [500, 9_000] {
        let e = err_at(&mut s, c, 0, DETECT, json!({"floor_hz": hz}));
        assert!(e.contains("floor"), "{hz}: {e}");
    }
    assert!(s.measure_requests().is_empty());
}

#[test]
fn wp_adet1_one_grouped_question_with_the_warning_and_a_grant_for_detections() {
    let (set, id, core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let now = 1_000;
    let cid = held_at(&mut s, c, now, DETECT, json!({"outputs": [1, 2, 3]}));
    let sheet = s.confirmation(cid).unwrap().to_json();
    let kinds: Vec<&str> = sheet["risks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["test_signal", "detect"], "{sheet}");
    let detect = sheet["risks"][1]["what"].as_str().unwrap();
    assert!(
        detect.contains("Out 1–3")
            && detect.contains("above 1 kHz")
            && detect.contains("−24 dBFS")
            && detect.contains("−50 dBFS")
            && detect.contains("treated as a tweeter"),
        "{detect}"
    );
    assert_eq!(sheet["warning"], json!(DETECT_WARNING));
    assert_eq!(sheet["grant"]["kind"], json!("test_signals"));
    assert_eq!(sheet["grant"]["detect"], json!(true));
    assert_eq!(sheet["grant"]["device"], json!(id.as_str()));
    s.answer_confirmation(cid, true, now).unwrap();
    let grants = s.grants();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0]["detect"], json!(true), "{grants:?}");
    let reqs = s.measure_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0]["kind"], json!("detect"));
    assert_eq!(reqs[0]["loop"], json!("detect"));
    // A second detection by the same agent inside the grant is not asked.
    ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    let r = ok_at(
        &mut s,
        c,
        now,
        DETECT,
        json!({"outputs": [3], "floor_hz": 2500}),
    );
    assert!(r["what"].as_str().unwrap().contains("above 2.5 kHz"), "{r}");
    ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    // Another agent is asked.
    let other = s.connect_as("other");
    held_at(&mut s, other, now, DETECT, json!({}));
    // Stop ends every grant: asked again.
    s.stop_agents(now);
    assert!(s.grants().is_empty());
    held_at(&mut s, c, now, DETECT, json!({}));

    // A grant from a measurement's question does not cover a detection.
    let (set, _id, core2) = demo();
    layout(&core2, true);
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let cid = held_at(
        &mut s,
        c,
        now,
        "dspi_measure_start",
        json!({"outputs": [1], "sweep_ms": 1000, "passes": 2}),
    );
    s.answer_confirmation(cid, true, now).unwrap();
    assert_eq!(s.grants()[0]["detect"], json!(false));
    ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    layout(&core2, false);
    let cid = held_at(&mut s, c, now, DETECT, json!({"outputs": [3]}));
    // Allowed: the grant now covers detections too.
    s.answer_confirmation(cid, true, now).unwrap();
    assert_eq!(s.grants()[0]["detect"], json!(true));
    let _ = core;
}

#[test]
fn wp_adet1_a_revoke_stops_a_running_detection_at_once() {
    let (set, _id, core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let now = 1_000;
    let cid = held_at(&mut s, c, now, DETECT, json!({"outputs": [3]}));
    s.answer_confirmation(cid, true, now).unwrap();
    let started = app_takes(&mut s, &core);
    assert_eq!(started.len(), 1, "{started:?}");
    assert!(started[0].get("error").is_none(), "{started:?}");
    for _ in 0..5 {
        core.detect_tick();
    }
    let st = status(&mut s, c, now);
    assert_eq!(st["detect"]["by_this_agent"], json!(true), "{st}");
    assert_eq!(st["request"]["state"], json!("running"), "{st}");
    let gid = s.grants()[0]["id"].as_u64().unwrap();
    assert!(s.revoke_grant(gid));
    let d = core.detect_state();
    assert_eq!(d.stage, dspi_core::DetectStage::Stopped, "{d:?}");
    assert!(!d.running);
    let st = status(&mut s, c, now);
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    assert!(
        st["request"]["note"].as_str().unwrap().contains("revoked"),
        "{st}"
    );
    assert_eq!(st["detect"]["stage"], json!("stopped"), "{st}");
    assert_eq!(st["detect"]["result"]["stopped"], json!(true), "{st}");
}

#[test]
fn wp_adet1_the_demo_simulation_detects_a_tweeter_end_to_end() {
    let (set, _id, core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let mut now = 1_000;
    assert_eq!(overview(&mut s, c)["roles_from"], json!("none"));
    let cid = held_at(&mut s, c, now, DETECT, json!({"outputs": [3]}));
    s.answer_confirmation(cid, true, now).unwrap();
    // Busy while it waits.
    let e = err_at(&mut s, c, now, DETECT, json!({"outputs": [3]}));
    assert!(e.contains("waits for the app"), "{e}");
    let started = app_takes(&mut s, &core);
    assert_eq!(started[0]["ticket"]["loop"], json!("detect"), "{started:?}");
    assert!(started[0].get("error").is_none(), "{started:?}");
    let agent = s.actor_of(c).unwrap();
    assert_eq!(core.detect_state().actor, agent);
    // A measurement is busy while it runs.
    let e = err_at(&mut s, c, now, "dspi_measure_start", json!({}));
    assert!(e.contains("a speaker detection is running"), "{e}");
    app_drives(&core, "detect", 300);
    now += 5_000;
    let st = status(&mut s, c, now);
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    assert!(
        st["request"]["note"].as_str().unwrap().contains("complete"),
        "{st}"
    );
    let res = &st["detect"]["result"];
    assert_eq!(st["detect"]["stage"], json!("done"), "{st}");
    assert_eq!(res["layout_state"], json!("auto_detected"), "{st}");
    let out = &res["outputs"][0];
    assert_eq!(out["output"], json!(3), "Out 3");
    assert_eq!(out["class"], json!("tweeter"), "{st}");
    assert_eq!(out["role"], json!("high"));
    assert!(out["evidence"]["heard_at"].is_string(), "{st}");
    assert!(res["summary"].as_str().unwrap().contains("tweeter"), "{st}");
    assert!(
        res["next"]
            .as_str()
            .unwrap()
            .contains("Settings › Speakers")
    );
    // The overview and the measuring tools take it.
    let o = overview(&mut s, c);
    assert_eq!(o["roles_from"], json!("speaker_setup_auto_detected"), "{o}");
    assert_eq!(o["outputs"][2]["role_source"], json!("detected"));
    assert_eq!(o["outputs"][2]["reaches"], json!("driver"));
    let p = ok_at(
        &mut s,
        c,
        now,
        "dspi_measure_preflight",
        json!({"outputs": [3]}),
    );
    assert_eq!(p["roles"][0]["role"], json!("high"), "{p}");
    // Another detection on an auto-detected layout is allowed (the grant).
    let r = ok_at(&mut s, c, now, DETECT, json!({"outputs": [3]}));
    assert_eq!(r["status"], json!("waiting_for_app"));
}

#[test]
fn wp_adet1_the_tool_is_listed_before_the_options_closed_and_not_agent_proof() {
    let (set, _id, _core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "adet1-agent", "version": "1"}}}}),
        )
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let at = names.iter().position(|n| *n == DETECT).unwrap();
    assert_eq!(names[at + 1], "dspi_get_measure_options", "{names:?}");
    assert_eq!(names[at - 1], "dspi_system_measure_revert");
    let t = &tools[at];
    assert_eq!(
        t["inputSchema"]["additionalProperties"],
        json!(false),
        "{t}"
    );
    let props: Vec<&String> = t["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert!(
        !props
            .iter()
            .any(|p| p.contains("role") || p.contains("level")),
        "{props:?}"
    );
    assert!(!DETECT.contains("speaker_layout"));
    #[cfg(feature = "link")]
    for bad in crate::tests_wp_mcp2c::AGENT_PROOF {
        assert!(!DETECT.contains(bad), "{bad}");
    }
}
