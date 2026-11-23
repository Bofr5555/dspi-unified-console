//! WP ODEC: the owner's answers of 2026-11-23. A level raise on a confirmed
//! tweeter the user left playing unprotected is refused (1); an agent keeps a
//! tune only after a passed verify (2); no agent preset load while the master
//! volume is saved with presets (3); one question grants test signals on a
//! device for ten minutes (4); after Stop the agent is told the user stopped
//! it (5); the agent never reads that the computer's audio is simulated (7).

use dspi_core::measure::{MeasureTuneStage, MeasureTuneState, MeasureTuneVerify};
use serde_json::json;

use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{blank, confirm, demo as ffix_demo, err, n_in, ok, server_of};
use crate::tests_wp_mcp5::{call_at, demo, err_at, held_at, layout, ok_at, server};

// ------------------------------------------------------------ 1

#[test]
fn wp_odec_no_level_raise_on_a_tweeter_left_playing_unprotected_under_any_setting() {
    for mode in [
        ConfirmMode::NeverAsk,
        ConfirmMode::AskRisky,
        ConfirmMode::AskEverything,
    ] {
        let (set, core) = ffix_demo(&format!("raise-{}", mode.as_str()));
        blank(&core, 1);
        core.set_matrix_route(0, 1, true, -6.0, false).unwrap();
        core.set_output_gain(1, -10.0).unwrap();
        core.set_master_volume(-20.0).unwrap();
        confirm(&core);
        let mut s = server_of(&set, mode);
        let c = s.connect();
        for (tool, args, what) in [
            (
                "dspi_set_gain",
                json!({"output": 1, "gain_db": -4}),
                "gain up",
            ),
            (
                "dspi_set_volume",
                json!({"volume": "master", "db": -10}),
                "Master volume up",
            ),
        ] {
            let e = err(&mut s, c, tool, args);
            assert!(
                e.contains("drives a tweeter") && e.contains("makes it louder") && e.contains(what),
                "{}: {tool}: {e}",
                mode.as_str()
            );
        }
        let st = core.get_state();
        assert!((st.output_gain_db[1] - -10.0).abs() < 0.01);
        assert!((st.master_volume_db - -20.0).abs() < 0.01);
        // Turning it down still passes (never asked under "never ask").
        if mode == ConfirmMode::NeverAsk {
            ok(
                &mut s,
                c,
                "dspi_set_gain",
                json!({"output": 1, "gain_db": -16}),
            );
            ok(&mut s, c, "dspi_set_mute", json!({"output": 1, "muted": true}));
        }
    }
}

#[test]
fn wp_odec_a_protected_tweeter_may_be_raised_and_other_outputs_are_not_held_to_it() {
    let (set, core) = ffix_demo("raise-protected");
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, -6.0, false).unwrap();
    core.set_crossover(
        n_in(&core) + 1,
        0,
        crate::tests_wp_ffix::hp(crate::tests_wp_ffix::LR4_HP, 2_000.0),
    )
    .unwrap();
    core.set_output_gain(1, -10.0).unwrap();
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -6}),
    );
    // A woofer's gain with a tweeter elsewhere protected: as before.
    ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -3}),
    );
}

// ------------------------------------------------------------ 2

fn tune(stage: MeasureTuneStage, verify: Option<bool>) -> MeasureTuneState {
    MeasureTuneState {
        stage,
        verify: verify.map(|passed| MeasureTuneVerify {
            passed,
            ..MeasureTuneVerify::default()
        }),
        ..MeasureTuneState::default()
    }
}

#[test]
fn wp_odec_an_agent_keeps_a_tune_only_after_a_passed_verify() {
    use crate::tools::measure::{verify_failed, verify_passed};
    let applied = tune(MeasureTuneStage::Applied, None);
    assert!(!verify_passed(&applied) && !verify_failed(&applied));
    assert!(!verify_passed(&tune(MeasureTuneStage::Verified, Some(false))));
    assert!(verify_passed(&tune(MeasureTuneStage::Verified, Some(true))));
    // Over MCP: with no tune verified, the Keep is refused in words.
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let e = err_at(&mut s, c, 1_000, "dspi_tune_keep", json!({}));
    assert!(
        e.contains("no passed verify") && e.contains("only the user"),
        "{e}"
    );
}

// ------------------------------------------------------------ 3

#[test]
fn wp_odec_no_agent_preset_load_while_the_master_volume_is_saved_with_presets() {
    for mode in [ConfirmMode::NeverAsk, ConfirmMode::AskRisky] {
        let (set, core) = ffix_demo(&format!("vol-with-presets-{}", mode.as_str()));
        let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
        core.save_preset_confirmed(1, Some(token)).unwrap();
        core.set_master_volume_mode(1).unwrap();
        assert_eq!(core.get_state().master_volume_mode, 1);
        let mut s = server_of(&set, mode);
        let c = s.connect();
        // Refused at once, never asked.
        for (tool, args) in [
            ("dspi_preset_load", json!({"slot": 1})),
            ("dspi_revert_to_saved", json!({})),
        ] {
            let e = err(&mut s, c, tool, args);
            assert!(
                e.contains("saves the master volume with each preset") && e.contains("app"),
                "{tool}: {e}"
            );
        }
        assert!(s.waiting_confirmations().is_empty());
        // Independent of presets again: as before.
        core.set_master_volume_mode(0).unwrap();
        if mode == ConfirmMode::NeverAsk {
            ok(&mut s, c, "dspi_preset_load", json!({"slot": 1}));
        }
    }
}

// ------------------------------------------------------------ 4

const START: &str = "dspi_measure_start";

fn start_args() -> serde_json::Value {
    json!({"outputs": [0], "sweep_ms": 1000, "passes": 2})
}

#[test]
fn wp_odec_one_question_grants_test_signals_on_a_device_for_ten_minutes() {
    for mode in [ConfirmMode::AskRisky, ConfirmMode::AskEverything] {
        let (set, id, core) = demo();
        layout(&core, true);
        let mut s = server(&set, mode, true);
        let c = s.connect();
        let mut now = 1_000;
        let cid = held_at(&mut s, c, now, START, start_args());
        let sheet = s.confirmation(cid).unwrap().to_json();
        assert_eq!(sheet["grant"]["kind"], json!("test_signals"), "{sheet}");
        assert_eq!(sheet["grant"]["minutes"], json!(10));
        assert_eq!(sheet["grant"]["device"], json!(id.as_str()));
        s.answer_confirmation(cid, true, now).unwrap();
        let grants = s.grants();
        assert_eq!(grants.len(), 1, "{grants:?}");
        assert_eq!(grants[0]["device"], json!(id.as_str()));
        assert_eq!(s.work()["grants"], json!(grants));
        // The agent stops its request and starts again: not asked.
        ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
        let r = ok_at(&mut s, c, now, START, start_args());
        assert_ne!(r["status"], json!("awaiting_confirmation"), "{r}");
        ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
        // A louder change is still asked, one at a time.
        core.set_output_gain(0, -10.0).unwrap();
        held_at(
            &mut s,
            c,
            now,
            "dspi_set_gain",
            json!({"output": 0, "gain_db": -6}),
        );
        // Another agent has no grant.
        let other = s.connect_as("other");
        held_at(&mut s, other, now, START, start_args());
        // Ten minutes on, it is asked again (and the agent is told).
        now += crate::grant::GRANT_MS;
        s.tick(now);
        assert!(s.grants().is_empty());
        let r = call_at(&mut s, c, now, START, start_args());
        assert_eq!(
            r["structuredContent"]["status"],
            json!("awaiting_confirmation"),
            "{r}"
        );
        // The agent is told with its next result (a held one waits).
        let r = call_at(&mut s, c, now, "dspi_list_devices", json!({}));
        assert!(
            r.to_string().contains("ended (ten minutes)"),
            "the agent is told: {r}"
        );
    }
}

#[test]
fn wp_odec_the_user_revokes_a_grant_and_never_ask_grants_nothing() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::AskRisky, true);
    let c = s.connect();
    let now = 1_000;
    let cid = held_at(&mut s, c, now, START, start_args());
    s.answer_confirmation(cid, true, now).unwrap();
    ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    let gid = s.grants()[0]["id"].as_u64().unwrap();
    assert!(s.revoke_grant(gid));
    assert!(!s.revoke_grant(gid));
    let r = call_at(&mut s, c, now, START, start_args());
    assert_eq!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation")
    );
    let r = call_at(&mut s, c, now, "dspi_list_devices", json!({}));
    assert!(r.to_string().contains("revoked your permission"), "{r}");
    // A question that is not about test signals grants nothing.
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::AskRisky, true);
    let c = s.connect();
    core.set_output_gain(0, -10.0).unwrap();
    let cid = held_at(
        &mut s,
        c,
        now,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -6}),
    );
    assert_eq!(s.confirmation(cid).unwrap().to_json()["grant"], json!(null));
    s.answer_confirmation(cid, true, now).unwrap();
    assert!(s.grants().is_empty());
    // Never ask: nothing is asked, so nothing is granted.
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    ok_at(&mut s, c, now, START, start_args());
    assert!(s.grants().is_empty());
}

// ------------------------------------------------------------ 5

#[test]
fn wp_odec_after_stop_the_agent_is_told_the_user_stopped_it() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect_as("Claude Code");
    ok_at(&mut s, c, 1_000, "dspi_list_devices", json!({}));
    let labels = s.stopped_labels();
    assert_eq!(labels.len(), 1, "{labels:?}");
    // The next run: the same agent's first result says so, once.
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    s.note_stopped(labels);
    let other = s.connect_as("Someone else");
    let r = call_at(&mut s, other, 1_000, "dspi_list_devices", json!({}));
    assert!(!r.to_string().contains("pressed Stop"), "{r}");
    let c = s.connect_as("Claude Code");
    let r = call_at(&mut s, c, 1_000, "dspi_list_devices", json!({}));
    let block = &r["structuredContent"]["user_activity"];
    assert!(
        block["note"].as_str().unwrap().contains("the user pressed Stop"),
        "{r}"
    );
    assert_eq!(block["events"][0]["kind"], json!("stopped"));
    let r = call_at(&mut s, c, 1_000, "dspi_list_devices", json!({}));
    assert!(!r.to_string().contains("pressed Stop"), "{r}");
    // Not told yet, it is handed on to the run after.
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    s.note_stopped(vec!["AI · Late".into()]);
    assert!(
        s.stopped_labels().contains(&"AI · Late".to_owned()),
        "{:?}",
        s.stopped_labels()
    );
}

// ------------------------------------------------------------ 7

#[cfg(feature = "sim-bus")]
#[test]
fn wp_odec_the_host_audio_reading_names_nothing_simulated() {
    let audio = crate::host_audio::sim_bus::from_sim(dspi_core::SimHostAudio {
        os: "linux".into(),
        audio_system: "PipeWire 1.6".into(),
        outputs: Vec::new(),
    });
    assert!(audio.sim);
    assert_eq!(audio.audio_system, "PipeWire 1.6");
    let v = crate::tools::host::report(&audio, &[]);
    assert!(!v.to_string().to_lowercase().contains("simulat"), "{v}");
}
