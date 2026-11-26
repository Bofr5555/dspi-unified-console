//! FIX016 R2 (REVG R2, B10; owner: "one Allow answers exactly the rows
//! shown"): a group takes a question only while its whole sheet, asked and
//! unasked rows and the devices' headings, fits the rows a sheet shows
//! ([`GROUP_SHEET_ROWS`]); a question past it is the next sheet. The sheet
//! lists every asked row before any unasked one, and an answer to a question
//! whose group is not sealed (not shown yet) is refused.

use serde_json::{Value, json};

use crate::confirm::{AnswerError, ConfirmState, GROUP_QUIET_MS, GROUP_SHEET_ROWS};
use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{demo, server_of};
use crate::{ClientId, Server};

fn call_at(s: &mut Server, c: ClientId, now_ms: u64, tool: &str, args: Value) -> Value {
    let params = json!({"name": tool, "arguments": args, "_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "fix016-agent", "version": "1"},
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

/// `dspi_apply_changes` of `(output from 1, gain)` pairs, held: its id.
fn apply_at(s: &mut Server, c: ClientId, now_ms: u64, gains: &[(u8, f64)]) -> u64 {
    let changes: Vec<Value> = gains
        .iter()
        .map(|(o, g)| json!({"op": "set_gain", "output": o, "gain_db": g}))
        .collect();
    let r = call_at(
        s,
        c,
        now_ms,
        "dspi_apply_changes",
        json!({ "changes": changes }),
    );
    assert_eq!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{r}"
    );
    r["structuredContent"]["confirmation_id"].as_u64().unwrap()
}

fn group_of(rows: &[Value], id: u64) -> Value {
    rows.iter()
        .find(|r| r["confirmation_id"] == json!(id))
        .unwrap()["group"]
        .clone()
}

/// The sheets shown at `now_ms`: each group's `sheet`, by group, oldest
/// first.
fn sheets(s: &mut Server, now_ms: u64) -> Vec<(Value, Vec<Value>)> {
    let mut out: Vec<(Value, Vec<Value>)> = Vec::new();
    for r in s.confirmations_at(now_ms) {
        assert_eq!(r["sealed"], json!(true), "{r}");
        if !out.iter().any(|(g, _)| *g == r["group"]) {
            out.push((r["group"].clone(), r["sheet"].as_array().unwrap().clone()));
        }
    }
    out
}

fn risky_of(sheet: &[Value], id: u64) -> bool {
    sheet
        .iter()
        .any(|r| r["risky"] == json!(true) && r["confirmation_id"] == json!(id))
}

#[test]
fn fix016_r2_a_risky_row_past_the_sheet_goes_to_the_next_sheet() {
    // The reviewer's probe: three parallel dspi_apply_changes, each one
    // louder change plus six quieter ones, in ask-risky (one sheet of 22
    // rows before: the third call's risky row was row 15, behind "and 10
    // more")
    let (set, core) = demo("fix016-r2-probe");
    for o in 0..9 {
        core.set_output_gain(o, -20.0).unwrap();
    }
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let quieter: Vec<(u8, f64)> = (4..=9).map(|o| (o, -30.0)).collect();
    let ids: Vec<u64> = (1..=3u8)
        .map(|k| {
            let mut gains = vec![(k, -3.0)];
            gains.extend(quieter.iter().copied());
            apply_at(&mut s, c, 1_000 + u64::from(k) * 10, &gains)
        })
        .collect();
    let rows = s.confirmations_at(1_030 + GROUP_QUIET_MS);
    let shown = sheets(&mut s, 1_030 + GROUP_QUIET_MS);
    // Every sheet fits the rows a sheet shows: nothing folded away
    for (g, sheet) in &shown {
        assert!(sheet.len() <= GROUP_SHEET_ROWS, "{g}: {sheet:?}");
    }
    // The third call's risky row is not on the first sheet: on a next one
    let first = &shown[0].1;
    assert!(risky_of(first, ids[0]), "{first:?}");
    assert!(!risky_of(first, ids[2]), "{first:?}");
    assert_ne!(group_of(&rows, ids[2]), group_of(&rows, ids[0]));
    assert!(
        shown.iter().skip(1).any(|(_, sh)| risky_of(sh, ids[2])),
        "{shown:?}"
    );
    // On each sheet every asked row comes before any unasked one
    for (_, sheet) in &shown {
        let first_rest = sheet
            .iter()
            .position(|r| r["risky"] == json!(false) && r["head"] != json!(true))
            .unwrap_or(sheet.len());
        assert!(
            sheet
                .iter()
                .enumerate()
                .all(|(i, r)| r["risky"] != json!(true) || i < first_rest),
            "{sheet:?}"
        );
    }
}

#[test]
fn fix016_r2_a_group_takes_questions_while_its_sheet_fits() {
    let (set, core) = demo("fix016-r2-cap");
    for o in 0..9 {
        core.set_output_gain(o, -20.0).unwrap();
    }
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    // Four asked rows each (different values, so they do not fold)
    let a = apply_at(
        &mut s,
        c,
        1_000,
        &[(1, -3.0), (2, -4.0), (3, -5.0), (4, -6.0)],
    );
    let b = apply_at(
        &mut s,
        c,
        1_010,
        &[(5, -3.0), (6, -4.0), (7, -5.0), (8, -6.0)],
    );
    // 1 heading + 8 rows: three more fill the sheet exactly
    let fits = apply_at(&mut s, c, 1_020, &[(9, -3.0), (1, -2.0), (2, -2.5)]);
    // One more row would pass it: the next sheet
    let late = apply_at(&mut s, c, 1_030, &[(3, -2.0)]);
    let rows = s.confirmations_at(1_030 + GROUP_QUIET_MS);
    assert_eq!(group_of(&rows, a), group_of(&rows, b));
    assert_eq!(group_of(&rows, a), group_of(&rows, fits));
    assert_ne!(group_of(&rows, late), group_of(&rows, a));
    let shown = sheets(&mut s, 1_030 + GROUP_QUIET_MS);
    assert_eq!(shown[0].1.len(), GROUP_SHEET_ROWS, "{:?}", shown[0].1);
    assert!(risky_of(&shown[1].1, late));
    assert!(!risky_of(&shown[0].1, late));
}

#[test]
fn fix016_r2_an_answer_to_an_unsealed_question_is_refused() {
    let (set, core) = demo("fix016-r2-unsealed");
    core.set_output_gain(0, -20.0).unwrap();
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let id = apply_at(&mut s, c, 1_000, &[(1, -3.0)]);
    // Not shown yet: its group may still grow
    assert_eq!(
        s.answer_confirmation(id, true, 1_100),
        Err(AnswerError::Unsealed)
    );
    assert_eq!(s.confirmation(id).unwrap().state, ConfirmState::Waiting);
    assert!((core.get_state().output_gain_db[0] + 20.0).abs() < 0.01);
    // Once sealed it is the app's to answer
    s.answer_confirmation(id, true, 1_000 + GROUP_QUIET_MS)
        .unwrap();
    assert!((core.get_state().output_gain_db[0] + 3.0).abs() < 0.01);
}
