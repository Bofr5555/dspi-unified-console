//! WP UB5 (UXP B5, D2): one agent's questions raised together are one sheet.
//! A question joins its agent's open group while the group's newest question
//! is less than 0.4 s old and its first less than 1.5 s; then the group
//! seals, the app shows it, and it never takes another question: one Allow
//! over the sheet answers exactly the questions it showed.

use serde_json::{Map, Value, json};

use crate::confirm::{ConfirmState, Confirmations, GROUP_MS, GROUP_QUIET_MS};
use crate::policy::{ConfirmMode, Risk, RiskKind};
use crate::tests_wp_ffix::{demo, server_of};
use crate::{ClientId, Server};

/// `tool` with `args` by `c` at `now_ms` on the server's clock.
fn call_at(s: &mut Server, c: ClientId, now_ms: u64, tool: &str, args: Value) -> Value {
    let params = json!({"name": tool, "arguments": args, "_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "ub5-agent", "version": "1"},
    }});
    let r = s
        .handle_value(
            c,
            now_ms,
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params}),
        )
        .unwrap();
    r.get("result").cloned().unwrap_or_else(|| panic!("{r}"))
}

/// Out `output` (from 1) louder, held: its confirmation's id.
fn raise_at(s: &mut Server, c: ClientId, now_ms: u64, output: u8) -> u64 {
    let r = call_at(
        s,
        c,
        now_ms,
        "dspi_set_gain",
        json!({"output": output, "gain_db": -3.0}),
    );
    assert_eq!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{r}"
    );
    r["structuredContent"]["confirmation_id"].as_u64().unwrap()
}

/// What the app shows at `now_ms`, as AiHost does: the sealed questions,
/// and the first one's group as one sheet (their ids).
fn sheet_at(s: &mut Server, now_ms: u64) -> Vec<u64> {
    let rows: Vec<Value> = s
        .confirmations_at(now_ms)
        .into_iter()
        .filter(|c| c["sealed"] == json!(true))
        .collect();
    let Some(front) = rows.first() else {
        return Vec::new();
    };
    rows.iter()
        .filter(|c| c["group"] == front["group"])
        .map(|c| c["confirmation_id"].as_u64().unwrap())
        .collect()
}

fn state(s: &Server, id: u64) -> ConfirmState {
    s.confirmation(id).unwrap().state
}

#[test]
fn wp_ub5_questions_raised_together_are_one_sheet_once_sealed() {
    let (set, core) = demo("ub5-together");
    for o in 0..3 {
        core.set_output_gain(o, -20.0).unwrap();
    }
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let a = raise_at(&mut s, c, 1_000, 1);
    let b = raise_at(&mut s, c, 1_200, 2);
    // Not shown while it may still grow
    assert!(sheet_at(&mut s, 1_300).is_empty());
    let rows = s.confirmations_at(1_300);
    assert_eq!(rows[0]["group"], rows[1]["group"]);
    assert_eq!(rows[0]["seals_ms"], json!(1_200 + GROUP_QUIET_MS));
    // Sealed: one sheet with both
    assert_eq!(sheet_at(&mut s, 1_200 + GROUP_QUIET_MS), vec![a, b]);
    // One Allow over the sheet answers both, each with its own grant
    for id in [a, b] {
        s.answer_confirmation(id, true, 1_700).unwrap();
    }
    assert!((core.get_state().output_gain_db[0] + 3.0).abs() < 0.01);
    assert!((core.get_state().output_gain_db[1] + 3.0).abs() < 0.01);
}

#[test]
fn wp_ub5_an_allow_never_covers_a_question_raised_after_the_sheet_came_up() {
    let (set, core) = demo("ub5-later");
    for o in 0..3 {
        core.set_output_gain(o, -20.0).unwrap();
    }
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let a = raise_at(&mut s, c, 1_000, 1);
    // The app reads the sheet once the group sealed
    let shown = sheet_at(&mut s, 1_000 + GROUP_QUIET_MS);
    assert_eq!(shown, vec![a]);
    // A question that reaches the server afterwards, even one stamped
    // before the read (a call that waited for the server), within the
    // group's time: a group, and a sheet, of its own
    let late = raise_at(&mut s, c, 1_000 + GROUP_QUIET_MS - 50, 2);
    let rows = s.confirmations_at(1_000 + GROUP_QUIET_MS);
    let group_of = |id: u64| {
        rows.iter()
            .find(|r| r["confirmation_id"] == json!(id))
            .unwrap()["group"]
            .clone()
    };
    assert_ne!(group_of(late), group_of(a));
    // The person allows the sheet they saw: exactly its questions
    for id in &shown {
        s.answer_confirmation(*id, true, 1_500).unwrap();
    }
    assert_eq!(state(&s, a), ConfirmState::Allowed);
    assert_eq!(state(&s, late), ConfirmState::Waiting);
    assert!((core.get_state().output_gain_db[0] + 3.0).abs() < 0.01);
    assert!((core.get_state().output_gain_db[1] + 20.0).abs() < 0.01);
    // It comes up on its own once sealed
    assert_eq!(sheet_at(&mut s, 1_000 + 2 * GROUP_QUIET_MS), vec![late]);
}

fn louder(output: u8) -> Vec<Risk> {
    vec![Risk {
        kind: RiskKind::Louder,
        what: format!("Out {} gain up", output + 1),
        output: Some(output),
        keys: Vec::new(),
        said: String::new(),
        channel: String::new(),
    }]
}

fn open(t: &mut Confirmations, client: u64, tool: &'static str, risks: Vec<Risk>, at: u64) -> u64 {
    t.open(
        ClientId(client),
        "AI · test",
        tool,
        None,
        risks,
        Map::new(),
        at,
    )
}

#[test]
fn wp_ub5_a_group_takes_questions_for_at_most_its_time() {
    let mut t = Confirmations::default();
    let first = open(&mut t, 1, "dspi_set_gain", louder(0), 0);
    let mut last = 0;
    // Each within the quiet time of the one before: they join, up to the cap
    for at in (GROUP_QUIET_MS - 10..GROUP_MS).step_by(usize::try_from(GROUP_QUIET_MS - 10).unwrap())
    {
        let id = open(&mut t, 1, "dspi_set_gain", louder(1), at);
        assert_eq!(t.get(id).unwrap().group, first, "at {at}");
        last = at;
    }
    assert_eq!(t.seals_at(first), (last + GROUP_QUIET_MS).min(GROUP_MS));
    // Past the group's time: a new one, however close to the one before
    let late = open(&mut t, 1, "dspi_set_gain", louder(2), GROUP_MS);
    assert_eq!(t.get(late).unwrap().group, late);
    // Nothing is sealed until its time; then all of the group at once
    t.seal_due(GROUP_MS - 1);
    assert!(!t.get(first).unwrap().sealed);
    t.seal_due(GROUP_MS);
    assert!(
        t.waiting()
            .iter()
            .filter(|c| c.group == first)
            .all(|c| c.sealed)
    );
    assert!(!t.get(late).unwrap().sealed);
}

#[test]
fn wp_ub5_a_detection_and_the_roles_are_sheets_of_their_own() {
    let mut t = Confirmations::default();
    let detect = vec![Risk {
        kind: RiskKind::Detect,
        what: "Speaker detection".into(),
        output: None,
        keys: Vec::new(),
        said: String::new(),
        channel: String::new(),
    }];
    let d = open(&mut t, 1, "dspi_detect_speakers", detect, 0);
    // Shown at once, and nothing joins it
    assert!(t.get(d).unwrap().sealed);
    let g = open(&mut t, 1, "dspi_set_gain", louder(0), 10);
    assert_eq!(t.get(g).unwrap().group, g);
    let roles = open(
        &mut t,
        1,
        crate::tools::roles::SET_ROLES_NAME,
        vec![Risk {
            kind: RiskKind::SpeakerRoles,
            what: "Speaker roles from your words".into(),
            output: None,
            keys: Vec::new(),
            said: String::new(),
            channel: String::new(),
        }],
        20,
    );
    assert_eq!(t.get(roles).unwrap().group, roles);
    assert!(t.get(roles).unwrap().sealed);
}

#[test]
fn wp_ub5_two_agents_never_share_a_sheet() {
    let mut t = Confirmations::default();
    let a = open(&mut t, 1, "dspi_set_gain", louder(0), 0);
    let b = open(&mut t, 2, "dspi_set_gain", louder(1), 10);
    assert_eq!(t.get(a).unwrap().group, a);
    assert_eq!(t.get(b).unwrap().group, b);
}

#[test]
fn wp_ub5_the_group_is_the_apps_alone() {
    let (set, core) = demo("ub5-agent");
    core.set_output_gain(0, -20.0).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let id = raise_at(&mut s, c, 1_000, 1);
    let r = call_at(
        &mut s,
        c,
        1_100,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    let text = r.to_string();
    assert!(
        !text.contains("\"group\"") && !text.contains("sealed"),
        "{text}"
    );
}
