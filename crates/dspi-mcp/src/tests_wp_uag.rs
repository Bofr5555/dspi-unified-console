//! WP UAG (UXP A6): an agent's question in the screen's words. Each risk
//! carries the app's words (`said`: channels and presets as the screens name
//! them, no "(flash)"), and the sheet gets the whole call (`batch`), the
//! asked rows first and folded, then what the same Allow also does. None of
//! it reaches an agent.

use serde_json::{Map, Value, json};

use crate::ClientId;
use crate::confirm::Confirmations;

use crate::policy::{ConfirmMode, RiskKind, ScreenWords, said_of};
use crate::tests_wp_ffix::{call, demo, ok, server_of};

/// Out 2 the user's "Right woofer", Out 4 their "Sub", the rest full range.
fn layout(core: &dspi_core::DspiCore) {
    use dspi_core::measure::MeasureRole;
    let outputs = (0..core.get_state().num_output_channels)
        .map(|n| dspi_core::SpeakerLayoutOutput {
            output: n,
            role: match n {
                1 => MeasureRole::Low,
                3 => MeasureRole::Sub,
                _ => MeasureRole::Full,
            },
            speaker: match n {
                1 => "R".into(),
                3 => "Sub".into(),
                _ => format!("Spk{n}"),
            },
            group: 0,
            source: Default::default(),
        })
        .collect();
    core.speaker_layout_set(dspi_core::SpeakerLayout {
        outputs,
        confirmed_by_user: true,
        ..dspi_core::SpeakerLayout::default()
    })
    .unwrap();
}

fn held_id(r: &Value) -> u64 {
    assert_eq!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{r}"
    );
    r["structuredContent"]["confirmation_id"].as_u64().unwrap()
}

/// The app's sheet for confirmation `id`.
fn sheet(s: &crate::Server, id: u64) -> Value {
    s.waiting_confirmations()
        .into_iter()
        .find(|c| c["confirmation_id"] == json!(id))
        .unwrap()
}

#[test]
fn wp_uag_the_question_names_the_channel_and_shows_the_whole_call() {
    let (set, core) = demo("uag-batch");
    layout(&core);
    core.set_output_gain(1, -5.0).unwrap();
    core.set_output_gain(3, 0.0).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let held = call(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [
            {"op": "set_gain", "output": 2, "gain_db": -2.0},
            {"op": "set_gain", "output": 4, "gain_db": -6.0},
        ]}),
    );
    let id = held_id(&held);
    let sh = sheet(&s, id);
    let risk = &sh["risks"][0];
    assert_eq!(risk["kind"], json!("louder"), "{sh}");
    // The agent's words keep the number; the app's name the speaker
    assert!(
        risk["what"].as_str().unwrap().starts_with("Out 2 gain up"),
        "{sh}"
    );
    assert_eq!(
        risk["said"],
        json!("Right woofer: gain \u{2212}5.0 dB \u{2192} \u{2212}2.0 dB"),
        "{sh}"
    );
    assert_eq!(risk["channel"], json!("Right woofer"), "{sh}");
    let batch = sh["batch"].as_array().unwrap();
    assert_eq!(batch.len(), 2, "{sh}");
    assert_eq!(batch[0]["risky"], json!(true));
    assert_eq!(batch[0]["said"], risk["said"]);
    // The quieter output, not asked about, is on the sheet too
    assert_eq!(batch[1]["risky"], json!(false));
    assert_eq!(
        batch[1]["said"],
        json!("Sub: gain 0 \u{2192} \u{2212}6 dB"),
        "{sh}"
    );
}

#[test]
fn wp_uag_said_never_reaches_an_agent() {
    let (set, core) = demo("uag-agent");
    layout(&core);
    core.set_output_gain(1, -5.0).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let held = call(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 2, "gain_db": -2.0}),
    );
    let id = held_id(&held);
    assert!(sheet(&s, id)["risks"][0]["said"].is_string());
    let text = held.to_string();
    assert!(!text.contains("said") && !text.contains("woofer"), "{text}");
    for _ in 0..2 {
        let r = ok(
            &mut s,
            c,
            "dspi_get_confirmation",
            json!({"confirmation_id": id}),
        );
        let text = r.to_string();
        assert!(
            !text.contains("said") && !text.contains("batch") && !text.contains("Right woofer"),
            "{text}"
        );
        assert!(
            r["risks"][0]["what"].as_str().unwrap().contains("Out 2"),
            "{r}"
        );
        let _ = s.answer_confirmation(id, true, 0);
    }
    let act = ok(&mut s, c, "dspi_get_activity", json!({}));
    assert!(!act.to_string().contains("Right woofer"), "{act}");
}

#[test]
fn wp_uag_a_flash_write_names_its_preset_without_flash() {
    let (set, core) = demo("uag-preset");
    core.set_preset_name(1, "Lunch".into()).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let held = call(
        &mut s,
        c,
        "dspi_preset_set_startup",
        json!({"mode": "slot", "slot": 2}),
    );
    let id = held_id(&held);
    let sh = sheet(&s, id);
    assert_eq!(sh["risks"][0]["kind"], json!("flash_write"), "{sh}");
    assert_eq!(
        sh["risks"][0]["said"],
        json!("Start with preset 2 \u{201c}Lunch\u{201d}"),
        "{sh}"
    );
    // The agent's words are as they were
    assert_eq!(sh["risks"][0]["what"], json!("Start with preset 2 (flash)"));
}

#[test]
fn wp_uag_identical_rows_of_several_channels_fold() {
    let mk = |o: u8, said: &str| crate::policy::Risk {
        kind: RiskKind::Louder,
        what: format!("Turn off Out {}'s limiter", o + 1),
        output: Some(o),
        keys: Vec::new(),
        said: said.to_owned(),
        channel: String::new(),
    };
    let mut t = Confirmations::default();
    let batch = |t: &mut Confirmations, risks| {
        let id = t.open(
            ClientId(1),
            "AI · test",
            "dspi_set_limiter",
            None,
            risks,
            Map::new(),
            0,
        );
        t.set_rest(id, vec!["Sub: gain 0 \u{2192} \u{2212}6 dB".into()]);
        t.get(id).unwrap().to_json()["batch"].clone()
    };
    let b = batch(
        &mut t,
        vec![
            mk(0, "Left woofer: limiter off"),
            mk(1, "Right woofer: limiter off"),
            mk(2, "Centre woofer: gain 0 \u{2192} 3 dB"),
        ],
    );
    assert_eq!(
        b[0]["said"],
        json!("Left woofer, Right woofer: limiter off"),
        "{b}"
    );
    assert_eq!(
        b[1]["said"],
        json!("Centre woofer: gain 0 \u{2192} 3 dB"),
        "{b}"
    );
    assert_eq!(
        b[2],
        json!({"said": "Sub: gain 0 \u{2192} \u{2212}6 dB", "kind": null, "risky": false})
    );
    let many = (0..9)
        .map(|o| mk(o, &format!("Out {}: limiter off", o + 1)))
        .collect();
    let b = batch(&mut t, many);
    assert_eq!(b.as_array().unwrap().len(), 2, "{b}");
    assert_eq!(b[0]["said"], json!("9 outputs: limiter off"), "{b}");
}

#[test]
fn wp_uag_said_of_reads_as_the_screen() {
    let words = ScreenWords {
        channels: dspi_core::ChannelWords {
            inputs: vec!["L".into(), "R".into()],
            outputs: vec!["Left woofer".into(), "Right tweeter".into()],
            from_layout: vec![0, 1],
        },
        presets: vec![String::new(), "Lunch".into()],
    };
    let w = Some(&words);
    let cases = [
        (
            RiskKind::Louder,
            "Out 2 gain up from -5.0 dB to -2.0 dB",
            "Right tweeter: gain \u{2212}5.0 dB \u{2192} \u{2212}2.0 dB",
        ),
        (
            RiskKind::Louder,
            "Turn off Out 1's limiter",
            "Left woofer: limiter off",
        ),
        (
            RiskKind::Louder,
            "Route In 2 to Out 1 at +0.0 dB",
            "Route R to Left woofer at +0.0 dB",
        ),
        (
            RiskKind::DelayStep,
            "Out 1 delay from 1.00 to 30.00 ms (a step over 20 ms)",
            "Left woofer: delay 1.00 \u{2192} 30.00 ms (a step over 20 ms)",
        ),
        (
            RiskKind::FlashWrite,
            "Rename preset 2 (flash)",
            "Rename preset 2 \u{201c}Lunch\u{201d}",
        ),
        (
            RiskKind::FlashWrite,
            "Save the settings to preset 2 as \u{201c}Dinner\u{201d} (flash)",
            "Save the settings to preset 2 as \u{201c}Dinner\u{201d}",
        ),
        (
            RiskKind::Recall,
            "Load preset 1 (every setting replaced)",
            "Load preset 1 (every setting replaced)",
        ),
        (RiskKind::Unmute, "Unmute Out 2", "Unmute Right tweeter"),
    ];
    for (kind, what, said) in cases {
        assert_eq!(said_of(kind, what, w), said, "{what}");
    }
    // No device: the server's words, cleaned
    assert_eq!(
        said_of(RiskKind::FlashWrite, "Rename preset 2 (flash)", None),
        "Rename preset 2"
    );
}

// ---------------------------------------------------------------- UXP A7

/// The newest activity entry of `tool`.
fn last_of(s: &crate::Server, tool: &str) -> crate::activity::ActivityEntry {
    s.activity(20)
        .into_iter()
        .rev()
        .find(|e| e.tool == tool)
        .unwrap()
}

#[test]
fn wp_uag_a_refusal_says_why_to_the_person_and_nothing_new_to_the_agent() {
    let (set, core) = demo("uag-refused");
    layout(&core);
    let n_in = core.get_state().num_input_channels;
    // The user's 2 kHz high-pass on Out 3: a tweeter's, whatever its row says
    core.set_crossover(
        n_in + 2,
        0,
        crate::tests_wp_ffix::hp(crate::tests_wp_ffix::LR4_HP, 2_000.0),
    )
    .unwrap();
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let e = crate::tests_wp_ffix::err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 3, "high_pass": null}),
    );
    let entry = last_of(&s, "dspi_set_crossover");
    assert_eq!(entry.outcome, crate::activity::Outcome::Error);
    assert!(entry.journal_ids.is_empty());
    assert_eq!(entry.said, "Kept Spk2's high-pass at 1 kHz", "{entry:?}");
    // The agent's words are the rule's, as before; the person's never reach it
    assert_eq!(entry.detail, e);
    assert!(e.starts_with("Out 3 has a high-pass"), "{e}");
    let act = ok(&mut s, c, "dspi_get_activity", json!({}));
    assert!(!act.to_string().contains("Kept Spk2"), "{act}");

    // The installer lock
    core.installer_pin_set("4711".into()).unwrap();
    core.installer_lock_cover(vec!["channels".into()]).unwrap();
    core.installer_lock().unwrap();
    crate::tests_wp_ffix::err(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 5, "gain_db": -3.0}),
    );
    assert_eq!(
        last_of(&s, "dspi_set_gain").said,
        "The installer lock is on"
    );
    // A change that runs keeps the core's words for what it did
    ok(
        &mut s,
        c,
        "dspi_set_mute",
        json!({"output": 5, "muted": true}),
    );
    assert!(last_of(&s, "dspi_set_mute").said.contains("muted"));
}

#[test]
fn wp_uag_refusal_words_name_what_the_rule_kept() {
    use crate::policy::refusal_words;
    let words = ScreenWords {
        channels: dspi_core::ChannelWords {
            inputs: vec!["L".into()],
            outputs: vec!["Sub".into(), "Right tweeter".into(), "Left mid".into()],
            from_layout: vec![0, 1, 2],
        },
        presets: Vec::new(),
    };
    let w = Some(&words);
    for (why, said) in [
        (
            "Out 2 drives a tweeter in the user's speaker setup (confirmed), so it must keep a \
             high-pass at 1000 Hz or above, 12 dB/octave or steeper, whenever it can play: this \
             removes or weakens its high-pass.",
            "Kept Right tweeter's high-pass at 1 kHz",
        ),
        (
            "Out 3 drives a midrange in the user's speaker setup (confirmed), so it must keep a \
             high-pass at 80 Hz or above",
            "Kept Left mid's high-pass at 80 Hz",
        ),
        (
            "Out 2: a wholesale change (a recall, a preset load, an import) may not remove or \
             lower a tweeter's high-pass; the user can do it in the app.",
            "Kept Right tweeter's high-pass",
        ),
        (
            "Out 1: the speaker detection heard nothing from it, so an agent may not route to it.",
            "Nothing was heard from Sub: only you can include it, in Settings › Speakers",
        ),
    ] {
        assert_eq!(refusal_words(why, w), said, "{why}");
    }
}

#[test]
fn wp_uag_a_quiet_load_is_one_sentence_naming_what_the_limits_held() {
    let words = ScreenWords {
        channels: dspi_core::ChannelWords {
            inputs: vec!["L".into()],
            outputs: vec![
                "Sub".into(),
                "Left tweeter".into(),
                "Sub 2".into(),
                "Right tweeter".into(),
            ],
            from_layout: vec![0, 1, 2, 3],
        },
        presets: Vec::new(),
    };
    let held = [
        "Out 2 crossover 1 frequency: 600 \u{2192} 1000 Hz".to_owned(),
        "Out 4 crossover 1 frequency: 600 \u{2192} 1000 Hz".to_owned(),
    ];
    assert_eq!(
        crate::tools::library::held_said(&held, Some(&words)),
        "your limits kept Left tweeter and Right tweeter at 1 kHz"
    );
    let mixed = [
        held[0].clone(),
        "Out 1 limiter threshold: -3 \u{2192} -6 dB".to_owned(),
    ];
    assert_eq!(
        crate::tools::library::held_said(&mixed, Some(&words)),
        "your limits held Left tweeter crossover 1 frequency: 600 \u{2192} 1000 Hz; Sub limiter \
         threshold: -3 \u{2192} -6 dB"
    );
}

#[test]
fn wp_uag_a_preset_load_says_its_name_once() {
    let (set, core) = demo("uag-load");
    core.set_preset_name(1, "Lunch".into()).unwrap();
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok(&mut s, c, "dspi_preset_save", json!({"slot": 2}));
    ok(&mut s, c, "dspi_preset_load", json!({"slot": 2}));
    let e = last_of(&s, "dspi_preset_load");
    assert!(e.said.starts_with("Loaded preset 2"), "{e:?}");
}
