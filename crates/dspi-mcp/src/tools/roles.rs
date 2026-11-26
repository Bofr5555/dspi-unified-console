//! Speaker roles from the user's words (WP ROLES, owner 2026-11-23: "Yes
//! that is certainly needed"; this reverses MPLAN 2.2's "an agent never sets
//! the layout").
//!
//! - **One tool**, [`SET_ROLES`]: the agent passes the table it read from
//!   the user's words (output, role, speaker label) and quotes the words.
//!   The core keeps it as a **stated** layout (`dspi_core::LayoutState::
//!   Stated`): trusted as a detected one is (every tweeter rule applies to a
//!   stated tweeter), an output it does not name unknown and treated as a
//!   tweeter, and shown to the user as "set by AI from your words, not
//!   checked" until they confirm or correct it in Settings › Speakers.
//! - **One question** under "ask on everything" and "ask for risky changes":
//!   the whole table ([`crate::policy::require_roles`]). Under "never ask" it
//!   is refused unless the user switched on Settings › AI Agents › Agents may
//!   set speaker roles from my words (off by default).
//! - **Over a confirmed setup** only with the user's words on what changed
//!   (`what_changed`), and still the one question.
//! - -- WP REVDFIX -- B1, **the ratchet**: a tweeter in the user's own
//!   rows stays a tweeter for agents until the user changes it in Settings ›
//!   Speakers, whatever a statement says (`dspi_core::SpeakerLayout::
//!   user_tweeter_held`); under "never ask" a table that makes one something
//!   else is refused, and the question shows each of the user's own rows the
//!   table changes.
//! - **Trust but check**: before an agent's stimulus plays below the floor
//!   of an output stated as a midrange, woofer, sub or full range, the app
//!   checks its top end (the detection's stage B, above the floor only;
//!   `tools::measure`): if it holds like a tweeter's, nothing below the
//!   floor plays there and the request's note says so for the agent to
//!   relay.

use dspi_core::SpeakerLayoutOutput;
use dspi_core::measure::MeasureRole;
use serde_json::{Map, Value, json};

use crate::policy;
use crate::registry::{Call, Tool, ToolClass, ToolError, ToolResult};
use crate::tools::edit::schema;
use crate::tools::read::{object_schema, shown, shown_all, state};
use crate::untrusted;

/// The tools.
pub static TOOLS: &[&Tool] = &[&SET_ROLES];

/// The tool's name (the confirmation sheet shows its table).
pub const SET_ROLES_NAME: &str = "dspi_set_speaker_roles";

/// A speaker label, characters at most.
const MAX_LABEL_CHARS: usize = 32;
/// The words shown on the confirmation sheet, characters at most.
const SHOWN_WORDS_CHARS: usize = 300;

/// The refusal over a confirmed setup without the user's words on what
/// changed.
pub const ROLES_CONFIRMED_TEXT: &str = "the user has confirmed the speaker setup of this device, \
     so it is set again only when the user's own words say what changed (for example \"I \
     rewired output 5\"): quote them in `what_changed` (the user is still asked). Otherwise the \
     user corrects it in Settings › Speakers. Nothing was changed.";

/// The role words the tool takes, and the role each means.
const ROLE_WORDS: &[(&str, MeasureRole)] = &[
    ("tweeter", MeasureRole::High),
    ("midrange", MeasureRole::Mid),
    ("woofer", MeasureRole::Low),
    ("sub", MeasureRole::Sub),
    ("full_range", MeasureRole::Full),
];

/// A role in the sheet's words.
fn role_text(role: MeasureRole) -> &'static str {
    match role {
        MeasureRole::High => "tweeter",
        MeasureRole::Mid => "midrange",
        MeasureRole::Low => "woofer",
        MeasureRole::Sub => "subwoofer",
        MeasureRole::Full => "full range",
    }
}

/// A role as its tool word (`role` in the table).
fn role_tool_word(role: MeasureRole) -> &'static str {
    ROLE_WORDS
        .iter()
        .find(|(_, r)| *r == role)
        .map_or("full_range", |(w, _)| w)
}

/// One stated row, checked.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    output: u8,
    role: MeasureRole,
    speaker: String,
}

/// The table in `args` (`rows`), checked against a device of `n_out`
/// outputs: every output once and present, a known role, a short label.
fn rows_of(args: &Map<String, Value>, n_out: u8) -> Result<Vec<Row>, String> {
    let list = args
        .get("rows")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
        .ok_or("`rows` names no output: one row per output the user named")?;
    if list.len() > 16 {
        return Err("at most 16 rows".into());
    }
    let mut rows: Vec<Row> = Vec::new();
    for v in list {
        let o = v.as_object().ok_or("each row is {output, role, speaker}")?;
        if let Some(k) = o
            .keys()
            .find(|k| !matches!(k.as_str(), "output" | "role" | "speaker"))
        {
            return Err(format!("a row has no field `{k}` (output, role, speaker)"));
        }
        // -- WP SHOWFIX -- from 1, as the app shows them (Out 1 is 1); kept from 0
        let given = o
            .get("output")
            .and_then(Value::as_u64)
            .filter(|n| *n >= 1)
            .ok_or("each row's `output` is a number from 1 (Out 1 is 1)")?;
        let output = crate::tools::read::from_shown(given)
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| *n < n_out)
            .ok_or_else(|| {
                format!(
                    "the device has no output {given} (it has outputs 1..{n_out}, counted from \
                     1 as on the screen: Out 1 is 1)"
                )
            })?;
        if rows.iter().any(|r| r.output == output) {
            return Err(format!("Out {given} is in the table twice"));
        }
        let word = o.get("role").and_then(Value::as_str).unwrap_or("");
        let role = ROLE_WORDS
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, r)| *r)
            .ok_or("each row's `role` is tweeter, midrange, woofer, sub or full_range")?;
        let speaker = untrusted::clean(
            o.get("speaker").and_then(Value::as_str).unwrap_or(""),
            MAX_LABEL_CHARS,
        );
        rows.push(Row {
            output,
            role,
            speaker,
        });
    }
    rows.sort_by_key(|r| r.output);
    Ok(rows)
}

/// The layout's rows: one alignment group per speaker label, in the order
/// the labels first come.
fn layout_rows(rows: &[Row]) -> Vec<SpeakerLayoutOutput> {
    let mut labels: Vec<&str> = Vec::new();
    rows.iter()
        .map(|r| {
            let group = match labels.iter().position(|l| *l == r.speaker) {
                Some(g) => g,
                None => {
                    labels.push(&r.speaker);
                    labels.len().saturating_sub(1)
                }
            };
            SpeakerLayoutOutput {
                output: r.output,
                role: r.role,
                speaker: r.speaker.clone(),
                group: u8::try_from(group).unwrap_or(u8::MAX),
                ..SpeakerLayoutOutput::default()
            }
        })
        .collect()
}

/// The table as the confirmation sheet shows it (`roles` beside the
/// risks): output (from 0: the app adds 1), side, role; plus the quoted words. `None` when
/// `args` hold no table.
pub fn table_json(args: &Map<String, Value>) -> Option<Value> {
    let rows = rows_of(args, 16).ok()?;
    let words = args
        .get("user_words")
        .and_then(Value::as_str)
        .map(|w| untrusted::clean(w, SHOWN_WORDS_CHARS));
    let changed = args
        .get("what_changed")
        .and_then(Value::as_str)
        .map(|w| untrusted::clean(w, SHOWN_WORDS_CHARS))
        .filter(|w| !w.is_empty());
    Some(json!({
        "rows": rows.iter().map(|r| json!({
            "output": r.output,
            "speaker": r.speaker,
            "role": role_tool_word(r.role),
            "role_text": role_text(r.role),
        })).collect::<Vec<_>>(),
        "user_words": words,
        "what_changed": changed,
    }))
}

/// "Out 1–6" style words for a list of outputs (from 0).
fn outs_words(outs: &[u8]) -> String {
    let n: Vec<String> = outs
        .iter()
        .map(|o| u16::from(*o).saturating_add(1).to_string())
        .collect();
    match n.as_slice() {
        [] => String::new(),
        [one] => format!("Out {one}"),
        _ => format!("Out {}", n.join(", ")),
    }
}

// -- WP REVDFIX -- B1
/// What the table does to the user's own rows (`own`, those a statement
/// replaces): (output, role before, role after) for each it changes, by
/// output; `None` after for one the table leaves out.
fn own_changes(
    own: &[SpeakerLayoutOutput],
    rows: &[Row],
) -> Vec<(u8, MeasureRole, Option<MeasureRole>)> {
    own.iter()
        .filter_map(|o| {
            let now = rows.iter().find(|r| r.output == o.output).map(|r| r.role);
            (now != Some(o.role)).then_some((o.output, o.role, now))
        })
        .collect()
}

// -- WP REVDFIX -- B1
/// The user's own tweeters the table states as something else (an output it
/// leaves out stays unknown, a tweeter).
fn demoted(changes: &[(u8, MeasureRole, Option<MeasureRole>)]) -> Vec<u8> {
    changes
        .iter()
        .filter(|(_, was, now)| {
            *was == MeasureRole::High && now.is_some_and(|n| n != MeasureRole::High)
        })
        .map(|(o, _, _)| *o)
        .collect()
}

// -- WP REVDFIX -- B1
/// The refusal under "never ask" of a table that makes the user's own
/// tweeters `outs` something else.
fn demote_refusal(outs: &[u8]) -> String {
    format!(
        "{} {} a tweeter in the user's own speaker setup: under \"never ask\" an agent's \
         statement may not make {} something else. Nothing was changed. State {} a tweeter or \
         leave {} out, or ask the user to change the setup in Settings › Speakers.",
        outs_words(outs),
        if outs.len() == 1 { "is" } else { "are" },
        if outs.len() == 1 { "it" } else { "them" },
        if outs.len() == 1 { "it" } else { "them" },
        if outs.len() == 1 { "it" } else { "them" },
    )
}

/// The question: the whole table, in the user's words; (-- WP REVDFIX --)
/// with what it changes in the user's own rows, row by row.
fn question(
    rows: &[Row],
    unnamed: &[u8],
    words: &str,
    changed: Option<&str>,
    own: &[(u8, MeasureRole, Option<MeasureRole>)],
) -> String {
    let table: Vec<String> = rows
        .iter()
        .map(|r| {
            let side = if r.speaker.is_empty() {
                String::new()
            } else {
                format!(" · {}", r.speaker)
            };
            format!(
                "Out {}{side} · {}",
                u16::from(r.output).saturating_add(1),
                role_text(r.role)
            )
        })
        .collect();
    // The table, then (after a line break, which the sheet shows apart) the rest
    let mut text = format!("Speaker roles from your words: {}.\n", table.join("; "));
    // -- WP REVDFIX -- B1: each of the user's own rows it changes
    if !own.is_empty() {
        let moves: Vec<String> = own
            .iter()
            .map(|(o, was, now)| {
                format!(
                    "Out {} {} \u{2192} {}",
                    u16::from(*o).saturating_add(1),
                    role_text(*was),
                    now.map_or("not named (unknown, treated as a tweeter)", role_text)
                )
            })
            .collect();
        text.push_str(&format!(
            "It changes your own setup: {}. ",
            moves.join("; ")
        ));
        if !demoted(own).is_empty() {
            text.push_str(
                "A tweeter you set stays a tweeter for agents until you change it in Settings › \
                 Speakers. ",
            );
        }
    }
    if !unnamed.is_empty() {
        text.push_str(&format!(
            "{} not named: unknown, treated as tweeters. ",
            outs_words(unnamed)
        ));
    }
    if let Some(c) = changed {
        text.push_str(&format!(
            "This replaces your confirmed setup (what changed, in your words: \u{201c}{}\u{201d}). ",
            untrusted::clean(c, SHOWN_WORDS_CHARS)
        ));
    }
    text.push_str(&format!(
        "Your words as quoted: \u{201c}{}\u{201d}. Not checked: before anything plays below 1 kHz \
         on a midrange, woofer or sub, the app checks that its top end is not a tweeter's.",
        untrusted::clean(words, SHOWN_WORDS_CHARS)
    ));
    text
}

/// The grant's binding: exactly this table (output, role pairs).
fn binding(rows: &[Row]) -> Vec<u8> {
    rows.iter()
        .flat_map(|r| {
            let role = ROLE_WORDS
                .iter()
                .position(|(_, x)| *x == r.role)
                .and_then(|p| u8::try_from(p).ok())
                .unwrap_or(u8::MAX);
            [r.output, role]
        })
        .collect()
}

/// What the agent does next.
const NEXT_TEXT: &str = "The roles are set as the user stated them and are not checked: the \
     safety rules use them now (a tweeter keeps a high-pass at 1 kHz or above, 12 dB/octave or \
     steeper), and an output not in the table is unknown, treated as a tweeter. Before an \
     agent's test signal plays below 1 kHz on an output stated as a midrange, woofer, sub or \
     full range, the app checks its top end with a quiet sweep above 1 kHz (measure before \
     setting its low-pass: one that hides the top end leaves it unchecked); if the top falls, \
     the stated role holds; if it holds like a tweeter's, nothing below 1 kHz plays there, it is \
     a tweeter to the safety rules from then on, and the request's note says so: tell the user. \
     The user can confirm or correct the roles in Settings › Speakers. Measure next \
     (dspi_measure_start).";

static SET_ROLES: Tool = Tool {
    name: SET_ROLES_NAME,
    title: "Set the speaker roles from the user's words",
    description: "Set which output drives what (tweeter, midrange, woofer, sub, full range) \
                  from what the USER told you, e.g. \"L and R front, three way, tweeter, mid, \
                  woofer, left then right, outputs 1-6\" = rows Out 1 L tweeter, Out 2 L \
                  midrange, Out 3 L woofer, Out 4 R tweeter, Out 5 R midrange, Out 6 R woofer. \
                  Outputs are numbered from 1, as on the screen (Out 1 is 1). Never guess roles the user did \
                  not state: if they did not say, run dspi_detect_speakers or ask them. Quote \
                  their words verbatim in `user_words`. One question shows the user the whole \
                  table (under \"never ask\" only with the user's switch, agent_may.set_roles). \
                  The roles then guard the outputs (roles_from: speaker_setup_stated; every \
                  tweeter rule applies); outputs you leave out are unknown and treated as \
                  tweeters; before a test signal plays below 1 kHz on a stated midrange, \
                  woofer or sub the app checks its top end and plays nothing lower if it looks \
                  like a tweeter. Over a confirmed setup only with the user's words on what changed \
                  (`what_changed`). A tweeter the user set stays a tweeter for agents until the \
                  user changes it in Settings › Speakers (user_tweeters_kept; under \"never \
                  ask\" a table that makes one something else is refused). The user can confirm \
                  or correct the roles in Settings › Speakers.",
    class: ToolClass::Write,
    device: true,
    input_schema: || {
        let mut m = Map::new();
        m.insert(
            "rows".into(),
            json!({"type": "array", "minItems": 1, "maxItems": 16,
                   "description": "One row per output the user named",
                   "items": {"type": "object", "additionalProperties": false,
                             "required": ["output", "role", "speaker"],
                             "properties": {
                                 "output": {"type": "integer", "minimum": 1,
                                            "description": "The output, from 1 (Out 1 is 1)"},
                                 "role": {"type": "string",
                                          "enum": ["tweeter", "midrange", "woofer", "sub", "full_range"],
                                          "description": "What it drives, as the user said"},
                                 "speaker": {"type": "string", "maxLength": MAX_LABEL_CHARS,
                                             "description": "The speaker or side it belongs to \
                                                             (L, R, C, Sub, or the user's word); \
                                                             outputs of one speaker share it"}}}}),
        );
        m.insert(
            "user_words".into(),
            json!({"type": "string", "minLength": 1,
                   "maxLength": dspi_core::speaker_layout::MAX_WORDS_CHARS,
                   "description": "The user's words the table comes from, quoted verbatim"}),
        );
        m.insert(
            "what_changed".into(),
            json!({"type": "string",
                   "maxLength": dspi_core::speaker_layout::MAX_WORDS_CHARS,
                   "description": "Over a confirmed speaker setup only: the user's own words on \
                                   what changed, quoted verbatim"}),
        );
        schema(m, &["rows", "user_words"])
    },
    output_schema: object_schema,
    handler: |call, args| set_roles(call, args),
};

fn set_roles(call: &Call<'_>, args: &Map<String, Value>) -> ToolResult {
    let core = call.core()?;
    let st = state(core)?;
    let n_out = st.num_output_channels;
    let rows =
        rows_of(args, n_out).map_err(|e| ToolError::new(format!("{e}. Nothing was changed.")))?;
    let words = args
        .get("user_words")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .ok_or_else(|| {
            ToolError::new("quote the user's words in `user_words`. Nothing was changed.")
        })?;
    let changed = args
        .get("what_changed")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|w| !w.is_empty());
    let before = core.speaker_layout().map_err(ToolError::from)?;
    let confirmed = before
        .as_ref()
        .is_some_and(|l| l.state() == dspi_core::LayoutState::Confirmed);
    if confirmed && changed.is_none() {
        return Err(ToolError::new(ROLES_CONFIRMED_TEXT));
    }
    let unnamed: Vec<u8> = (0..n_out)
        .filter(|o| !rows.iter().any(|r| r.output == *o))
        .collect();
    // -- WP REVDFIX -- B1: the user's own rows it replaces, and their tweeters
    // it would demote (refused under "never ask"; they stay tweeters for
    // agents in any case)
    let own = before
        .as_ref()
        .map(dspi_core::speaker_layout::replaced_by_statement)
        .unwrap_or_default();
    let changes = own_changes(&own, &rows);
    let demotes = Some(demoted(&changes))
        .filter(|d| !d.is_empty())
        .map(|d| demote_refusal(&d));
    let what = question(
        &rows,
        &unnamed,
        words,
        changed.filter(|_| confirmed),
        &changes,
    );
    policy::require_roles(&call.judging, &what, &binding(&rows), demotes.as_deref())?;
    let kept = core
        .speaker_roles_state(
            layout_rows(&rows),
            words.to_owned(),
            changed.filter(|_| confirmed).unwrap_or("").to_owned(),
        )
        .map_err(ToolError::from)?;
    Ok(json!({
        "ok": true,
        "device": call.device_text(),
        "roles_from": "speaker_setup_stated",
        "rows": kept.outputs.iter().map(|r| json!({
            "output": shown(r.output),
            "role": role_tool_word(r.role),
            "speaker": untrusted::name(&r.speaker),
            "group": r.group,
        })).collect::<Vec<_>>(),
        "unknown_treated_as_tweeter": shown_all(&unnamed),
        // -- WP REVDFIX -- B1
        "user_tweeters_kept": shown_all(&demoted(&changes)),
        "next": NEXT_TEXT,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wp_roles_the_owners_sentence_reads_as_its_table() {
        // -- WP SHOWFIX -- the agent's outputs from 1 (Out 1 is 1), kept from 0
        let args = json!({"rows": [
            {"output": 4, "role": "tweeter", "speaker": "R"},
            {"output": 1, "role": "tweeter", "speaker": "L"},
            {"output": 2, "role": "midrange", "speaker": "L"},
            {"output": 3, "role": "woofer", "speaker": "L"},
            {"output": 5, "role": "midrange", "speaker": "R"},
            {"output": 6, "role": "woofer", "speaker": "R"},
        ], "user_words": "L and r front speakers"});
        let rows = rows_of(args.as_object().unwrap(), 8).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.output).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5]
        );
        let l = layout_rows(&rows);
        assert_eq!(
            l.iter().map(|r| r.group).collect::<Vec<_>>(),
            [0, 0, 0, 1, 1, 1]
        );
        let q = question(&rows, &[6, 7], "L and r front speakers", None, &[]);
        assert!(q.starts_with("Speaker roles from your words: Out 1 · L · tweeter; Out 2 · L · midrange; Out 3 · L · woofer; Out 4 · R · tweeter"), "{q}");
        assert!(
            q.contains("Out 7, 8 not named: unknown, treated as tweeters"),
            "{q}"
        );
        // Refused: twice, out of range (0 and 9 on 8 outputs), an unknown role, an extra field
        for bad in [
            json!({"rows": [{"output": 1, "role": "tweeter", "speaker": "L"}, {"output": 1, "role": "woofer", "speaker": "L"}]}),
            json!({"rows": [{"output": 9, "role": "tweeter", "speaker": "L"}]}),
            json!({"rows": [{"output": 0, "role": "tweeter", "speaker": "L"}]}),
            json!({"rows": [{"output": 1, "role": "high", "speaker": "L"}]}),
            json!({"rows": [{"output": 1, "role": "tweeter", "speaker": "L", "group": 2}]}),
            json!({"rows": []}),
        ] {
            assert!(rows_of(bad.as_object().unwrap(), 8).is_err(), "{bad}");
        }
        // The binding is the table: another role is another grant
        let other = vec![Row {
            role: MeasureRole::Mid,
            ..rows[0].clone()
        }];
        assert_ne!(binding(&rows[..1]), binding(&other));
    }
}
