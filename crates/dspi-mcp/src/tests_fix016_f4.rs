//! FIX016 F4 (REVG F4, reviewer A's A5): the question sheet names an output
//! by a name alone only when the user's confirmed speaker layout gives it;
//! a channel name, which an agent may set without a question, keeps the
//! output's number: "Sub (quieter) · Out 3".

use serde_json::{Value, json};

use crate::policy::{ConfirmMode, ScreenWords, question_output_name, refusal_words};
use crate::tests_wp_ffix::server_of;
use crate::tests_wp_mcp5::{call_at, demo, layout, ok_at};

fn sheet(s: &crate::Server, id: u64) -> Value {
    s.waiting_confirmations()
        .into_iter()
        .find(|c| c["confirmation_id"] == json!(id))
        .unwrap()
}

fn held(r: &Value) -> u64 {
    assert_eq!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{r}"
    );
    r["structuredContent"]["confirmation_id"].as_u64().unwrap()
}

#[test]
fn fix016_f4_an_agents_channel_name_keeps_the_output_number_on_the_sheet() {
    // The reviewer's probe: no confirmed layout, Out 3 renamed by the agent
    // (not asked under ask-risky), then raised
    let (set, _id, core) = demo();
    core.set_output_gain(2, -20.0).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    ok_at(
        &mut s,
        c,
        0,
        "dspi_set_channel_name",
        json!({"output": 3, "name": "Sub (quieter)"}),
    );
    let r = call_at(
        &mut s,
        c,
        0,
        "dspi_set_gain",
        json!({"output": 3, "gain_db": 0.0}),
    );
    let sh = sheet(&s, held(&r));
    let risk = &sh["risks"][0];
    assert_eq!(risk["channel"], json!("Sub (quieter) \u{b7} Out 3"), "{sh}");
    assert!(
        risk["said"]
            .as_str()
            .unwrap()
            .starts_with("Sub (quieter) \u{b7} Out 3: gain"),
        "{sh}"
    );
    assert!(
        sh["batch"][0]["said"]
            .as_str()
            .unwrap()
            .starts_with("Sub (quieter) \u{b7} Out 3"),
        "{sh}"
    );
}

#[test]
fn fix016_f4_a_confirmed_layouts_name_still_reads_alone() {
    let (set, _id, core) = demo();
    layout(&core, true);
    core.set_output_gain(0, -20.0).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let r = call_at(
        &mut s,
        c,
        0,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -3.0}),
    );
    let sh = sheet(&s, held(&r));
    let channel = sh["risks"][0]["channel"].as_str().unwrap().to_owned();
    assert!(!channel.contains("Out 1"), "{sh}");
    assert!(!channel.is_empty(), "{sh}");
}

#[test]
fn fix016_f4_the_rule_and_the_refusals() {
    // A name from the confirmed layout, the core's own labels, any other
    assert_eq!(
        question_output_name("Left tweeter", 1, &[1]),
        "Left tweeter"
    );
    assert_eq!(question_output_name("Out 2", 1, &[]), "Out 2");
    assert_eq!(
        question_output_name("2 \u{b7} None", 1, &[]),
        "2 \u{b7} None"
    );
    assert_eq!(
        question_output_name("Tweeter", 1, &[]),
        "Tweeter \u{b7} Out 2"
    );
    // refusal_words and held_said read the same words
    let words = ScreenWords {
        channels: dspi_core::ChannelWords {
            inputs: vec![],
            outputs: vec![
                "Out 1".into(),
                question_output_name("Woofer (safe)", 1, &[]),
            ],
            from_layout: vec![],
        },
        presets: vec![],
    };
    let said = refusal_words(
        "Out 2: a tweeter keeps a high-pass at 1000 Hz",
        Some(&words),
    );
    assert!(said.contains("Woofer (safe) \u{b7} Out 2"), "{said}");
    let held = crate::tools::library::held_said(
        &["Out 2 crossover 1 frequency: 600 \u{2192} 1000 Hz".into()],
        Some(&words),
    );
    assert!(held.contains("Woofer (safe) \u{b7} Out 2"), "{held}");
}
