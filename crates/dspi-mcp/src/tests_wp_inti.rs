//! WP INTI: FUNC2b's and FUNC2c's preset-load fixes together. Under an
//! installer's limits profile, a load the tweeter rule refuses goes back to
//! the slot active before (FUNC2b) and tells nothing of the profile's hold;
//! a load the profile held says so (FUNC2c `limits_held`), and a later load
//! the profile held nothing of carries no older rows. The load guard's
//! silent master (-127 dB) is back each time.

use serde_json::json;

use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{LR4_HP, blank, confirm, demo, err, hp, n_in, ok, server_of};

#[test]
fn wp_inti_a_refused_load_and_a_held_load_under_a_limits_profile() {
    let (set, core) = demo("inti-load");
    let tweeter = n_in(&core) + 1;
    let save = |slot: u8, name: &str| {
        let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
        core.save_preset_confirmed(slot, Some(token)).unwrap();
        core.set_preset_name(slot, name.into()).unwrap();
    };
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    core.set_output_gain(0, 6.0).unwrap();
    // Slot 9 (8): the tweeter at 850 Hz and Out 1 at +6 dB: refused.
    core.set_crossover(tweeter, 0, hp(LR4_HP, 850.0)).unwrap();
    save(8, "Test");
    // Slot 3 (2): the tweeter protected, Out 1 at +6 dB: the profile holds it.
    core.set_crossover(tweeter, 0, hp(LR4_HP, 3_000.0)).unwrap();
    save(2, "Lunch");
    // Slot 2 (1): nothing for the profile to hold, the active one.
    core.set_output_gain(0, 0.0).unwrap();
    save(1, "Dinner");
    core.set_master_volume(-12.0).unwrap();
    core.limits_apply_profile(
        json!({"schemaVersion": 1, "meta": {"kind": "limits", "name": "Limits profile"},
               "limits": {"outputs": [{"output": 0, "maxGainDb": 0.0}]}})
        .to_string(),
    )
    .unwrap();
    assert!(core.limits_status().active);
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let level = |core: &dspi_core::DspiCore| core.get_state().master_volume_db;
    assert!((level(&core) - -12.0).abs() < 0.01);

    // Refused: back on Dinner, its settings, the master as it was, no hold told.
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 8}));
    assert!(
        e.contains("drives a tweeter") && e.contains("put back"),
        "{e}"
    );
    assert!(!e.contains("limits_held") && !e.contains("still 9"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 1, "left on the refused slot");
    assert!((st.xover[usize::from(tweeter)][0].freq - 3_000.0).abs() < 1.0);
    assert!(
        st.output_gain_db[0].abs() < 0.01,
        "{}",
        st.output_gain_db[0]
    );
    assert!((level(&core) - -12.0).abs() < 0.01, "{}", level(&core));

    // Held: Lunch is loaded, its +6 dB held to 0 dB, and the agent is told.
    let r = ok(&mut s, c, "dspi_preset_load", json!({"slot": 2}));
    let held = r["limits_held"].as_array().cloned().unwrap_or_default();
    assert!(
        held.iter()
            .any(|x| x.as_str().is_some_and(|x| x.starts_with("Out 1 gain"))),
        "{r}"
    );
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 2);
    assert!(st.output_gain_db[0] <= 0.01, "{}", st.output_gain_db[0]);
    assert!((level(&core) - -12.0).abs() < 0.5, "{}", level(&core));

    // Refused again from Lunch: back on Lunch, still held.
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 8}));
    assert!(e.contains("put back") && !e.contains("limits_held"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 2, "left on the refused slot");
    assert!(st.output_gain_db[0] <= 0.01, "{}", st.output_gain_db[0]);

    // Dinner holds nothing: no older rows.
    let r = ok(&mut s, c, "dspi_preset_load", json!({"slot": 1}));
    assert!(r.get("limits_held").is_none(), "{r}");
    assert_eq!(core.get_state().active_preset_slot, 1);
    assert!((level(&core) - -12.0).abs() < 0.5, "{}", level(&core));
}

/// Review RF2: an agent's grant for test signals on a device covers that
/// device's own jobs only. A system measurement, verify or sum check across a
/// hub's units (required on the same `hub/<serial>` device) is still asked,
/// its question grants nothing, and its own Allow lets the re-run pass.
#[test]
fn wp_inti_a_device_grant_does_not_cover_test_signals_across_units() {
    use crate::policy::{self, JudgingCell, RiskKind};
    for mode in [ConfirmMode::AskRisky, ConfirmMode::AskEverything] {
        let cell = JudgingCell::default();
        policy::lock(&cell).mode = Some(mode);
        // Without a grant, a one-device signal is asked, and that question grants.
        assert!(policy::require(&cell, RiskKind::TestSignal, "one device", &[1, 0]).is_err());
        assert!(crate::grant::asks_grant(&policy::lock(&cell).asked));
        policy::lock(&cell).asked.clear();
        // With the agent's grant on this device: the device's own jobs pass.
        policy::lock(&cell).grant.push(crate::grant::standing_key());
        assert!(policy::require(&cell, RiskKind::TestSignal, "one device", &[1, 0]).is_ok());
        assert!(policy::lock(&cell).asked.is_empty());
        // Across units: held, and its question opens no grant.
        assert!(policy::require_system_signal(&cell, "4 units", &[7, 0, 1]).is_err());
        let asked = policy::lock(&cell).asked.clone();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(asked[0].kind, RiskKind::TestSignal);
        assert!(!crate::grant::asks_grant(&asked));
        // The user's Allow of that question: the re-run passes.
        let key = asked[0].keys[0].clone();
        let mut j = policy::lock(&cell);
        j.asked.clear();
        j.grant.push(key);
        drop(j);
        assert!(policy::require_system_signal(&cell, "4 units", &[7, 0, 1]).is_ok());
    }
}

/// Review RF5: a preset load is judged with the levels too. A stored preset
/// that raises the gain of a tweeter the user left playing unprotected is
/// louder there: refused and put back under every setting, as a gain the
/// agent sets itself would be (ODEC, owner answer 1).
#[test]
fn wp_inti_a_preset_that_raises_an_unprotected_tweeters_gain_is_refused() {
    for mode in [ConfirmMode::NeverAsk, ConfirmMode::AskRisky] {
        let (set, core) = demo(&format!("inti-rf5-{}", mode.as_str()));
        let save = |slot: u8| {
            let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
            core.save_preset_confirmed(slot, Some(token)).unwrap();
        };
        blank(&core, 1);
        core.set_matrix_route(0, 1, true, -6.0, false).unwrap();
        // Slot 4 (3): the same, 6 dB louder on the tweeter.
        core.set_output_gain(1, -4.0).unwrap();
        save(3);
        // Slot 2 (1): the active one, as the user left it.
        core.set_output_gain(1, -10.0).unwrap();
        save(1);
        confirm(&core);
        let mut s = server_of(&set, mode);
        let c = s.connect();
        let e = match mode {
            ConfirmMode::NeverAsk => err(&mut s, c, "dspi_preset_load", json!({"slot": 3})),
            _ => {
                // Asked as a recall; the user's Allow does not lift the rule.
                let r =
                    crate::tests_wp_ffix::call(&mut s, c, "dspi_preset_load", json!({"slot": 3}));
                let id = r["structuredContent"]["confirmation_id"].as_u64().unwrap();
                s.answer_confirmation(id, true, 1_000).unwrap();
                ok(
                    &mut s,
                    c,
                    "dspi_get_confirmation",
                    json!({"confirmation_id": id}),
                )
                .to_string()
            }
        };
        assert!(
            e.contains("drives a tweeter") && e.contains("put back"),
            "{}: {e}",
            mode.as_str()
        );
        let st = core.get_state();
        assert!(
            (st.output_gain_db[1] - -10.0).abs() < 0.01,
            "{}",
            st.output_gain_db[1]
        );
        assert_eq!(st.active_preset_slot, 1);
    }
}

/// Review RF6: on a tweeter the user left playing unprotected, a PEQ boost
/// (any amount, on the output or on an input that feeds it) is louder too:
/// refused under every setting. A cut still passes.
#[test]
fn wp_inti_an_eq_boost_on_an_unprotected_tweeter_is_refused() {
    use dspi_core::FilterParams;
    let peak = |gain: f32| FilterParams {
        filter_type: 3,
        bypass: false,
        freq: 5_000.0,
        q: 1.0,
        gain,
        qp: 0.707,
    };
    for mode in [
        ConfirmMode::NeverAsk,
        ConfirmMode::AskRisky,
        ConfirmMode::AskEverything,
    ] {
        let (set, core) = demo(&format!("inti-rf6-{}", mode.as_str()));
        blank(&core, 1);
        core.set_matrix_route(0, 1, true, -6.0, false).unwrap();
        confirm(&core);
        let mut s = server_of(&set, mode);
        let c = s.connect();
        let tweeter = n_in(&core) + 1;
        for (on, gain) in [("output", 3.0), ("output", 6.0), ("input", 2.0)] {
            let n = if on == "output" { 1 } else { 0 };
            let e = err(
                &mut s,
                c,
                "dspi_set_peq_band",
                json!({on: n, "slot": 2, "type": "peaking", "freq_hz": 5000, "q": 1, "gain_db": gain}),
            );
            assert!(
                e.contains("drives a tweeter")
                    && e.contains("makes it louder")
                    && e.contains("boost"),
                "{}: {on} {gain} dB: {e}",
                mode.as_str()
            );
        }
        let st = core.get_state();
        assert_ne!(st.filters[usize::from(tweeter)][2], peak(3.0));
        if mode == ConfirmMode::NeverAsk {
            ok(
                &mut s,
                c,
                "dspi_set_peq_band",
                json!({"output": 1, "slot": 2, "type": "peaking", "freq_hz": 5000, "q": 1, "gain_db": -3}),
            );
        }
    }
}

/// Owner 2026-11-23 (FUNC2b/c question 3): an agent's save asks one question
/// that names the preset (no rename question after it), and a save across
/// several devices asks one question for all of them.
#[test]
fn wp_inti_a_named_save_on_several_devices_is_one_question() {
    let (set, core) = demo("inti-save");
    let other_id = set.open(dspi_core::DeviceTarget::Demo).unwrap();
    let other = set.core(&other_id).unwrap();
    other.fetch_all().unwrap();
    let here = set
        .list()
        .into_iter()
        .find(|d| d.id != other_id)
        .unwrap()
        .id;
    let mut s = server_of(&set, ConfirmMode::AskEverything);
    let c = s.connect();
    let r = crate::tests_wp_ffix::call(
        &mut s,
        c,
        "dspi_preset_save",
        json!({"device": here.as_str(), "slot": 3, "name": "Film night",
               "devices": [here.as_str(), other_id.as_str()]}),
    );
    let id = r["structuredContent"]["confirmation_id"].as_u64().unwrap();
    let sheet = s.confirmation(id).unwrap();
    assert_eq!(sheet.risks.len(), 1, "{:?}", sheet.risks);
    let what = &sheet.risks[0].what;
    assert!(
        what.contains("preset 4")
            && what.contains("\u{201c}Film night\u{201d}")
            && what.contains("on 2 devices"),
        "{what}"
    );
    s.answer_confirmation(id, true, 1_000).unwrap();
    let w = ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(w["result"]["isError"], json!(false), "{w}");
    assert!(s.waiting_confirmations().is_empty(), "no second question");
    for core in [&core, &other] {
        let st = core.get_state();
        assert!(st.preset_occupied >> 3 & 1 == 1, "slot 4 saved");
        assert_eq!(st.preset_names[3], "Film night");
        assert_eq!(st.active_preset_slot, 3);
    }
    // One device, named: one question too.
    let r = crate::tests_wp_ffix::call(
        &mut s,
        c,
        "dspi_preset_save",
        json!({"device": here.as_str(), "slot": 4, "name": "Day"}),
    );
    let id = r["structuredContent"]["confirmation_id"].as_u64().unwrap();
    assert_eq!(s.confirmation(id).unwrap().risks.len(), 1);
    s.answer_confirmation(id, true, 1_000).unwrap();
    ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(core.get_state().preset_names[4], "Day");
    // A device that is not open: refused before anything is asked.
    let e = err(
        &mut s,
        c,
        "dspi_preset_save",
        json!({"device": here.as_str(), "slot": 5, "devices": ["demo:99"]}),
    );
    assert!(e.contains("no open device"), "{e}");
}
