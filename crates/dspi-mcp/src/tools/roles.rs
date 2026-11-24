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
use crate::tools::read::{object_schema, state};
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
        let output = o
            .get("output")
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .ok_or("each row's `output` is a number from 0")?;
        if output >= n_out {
            return Err(format!(
                "the device has no output {output} (outputs are numbered from 0: Out 1 is 0)"
            ));
        }
        if rows.iter().any(|r| r.output == output) {
            return Err(format!("output {output} is in the table twice"));
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
/// risks): output (from 0), side, role; plus the quoted words. `None` when
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

/// The question: the whole table, in the user's words.
fn question(rows: &[Row], unnamed: &[u8], words: &str, changed: Option<&str>) -> String {
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
     steeper), and an output not in the table is unknown, treated as a tweeter. An output stated \
     as a midrange, woofer, sub or full range is guarded as a tweeter until its top end is \
     checked: no route, unmute or switch-on without a high-pass at 1 kHz or above (its filters \
     are free while it cannot play). Measure next (dspi_measure_start), before routing and \
     before setting low-passes: it checks each such output's top end first with a quiet sweep \
     above 1 kHz; if the top falls, the stated role holds; if it holds like a tweeter's, nothing \
     below 1 kHz plays there and the request's note says so: tell the user. The user can \
     confirm or correct the roles in Settings › Speakers.";

static SET_ROLES: Tool = Tool {
    name: SET_ROLES_NAME,
    title: "Set the speaker roles from the user's words",
    description: "Set which output drives what (tweeter, midrange, woofer, sub, full range) \
                  from what the USER told you, e.g. \"L and R front, three way, tweeter, mid, \
                  woofer, left then right, outputs 1-6\" = rows Out 1 L tweeter, Out 2 L \
                  midrange, Out 3 L woofer, Out 4 R tweeter, Out 5 R midrange, Out 6 R woofer. \
                  Outputs are numbered from 0 here (Out 1 is 0). Never guess roles the user did \
                  not state: if they did not say, run dspi_detect_speakers or ask them. Quote \
                  their words verbatim in `user_words`. One question shows the user the whole \
                  table (under \"never ask\" only with the user's switch, agent_may.set_roles). \
                  The roles then guard the outputs (roles_from: speaker_setup_stated; every \
                  tweeter rule applies); outputs you leave out are unknown and treated as \
                  tweeters; a stated midrange, woofer or sub is guarded as a tweeter until a \
                  measurement has checked its top end (measure before routing it), and plays \
                  nothing below 1 kHz if it looks like a tweeter. Over a confirmed setup only with the user's words on what changed \
                  (`what_changed`). The user can confirm or correct the roles in Settings › \
                  Speakers.",
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
                                 "output": {"type": "integer", "minimum": 0,
                                            "description": "The output, from 0 (Out 1 is 0)"},
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
    let confirmed = core
        .speaker_layout()
        .map_err(ToolError::from)?
        .is_some_and(|l| l.state() == dspi_core::LayoutState::Confirmed);
    if confirmed && changed.is_none() {
        return Err(ToolError::new(ROLES_CONFIRMED_TEXT));
    }
    let unnamed: Vec<u8> = (0..n_out)
        .filter(|o| !rows.iter().any(|r| r.output == *o))
        .collect();
    let what = question(&rows, &unnamed, words, changed.filter(|_| confirmed));
    policy::require_roles(&call.judging, &what, &binding(&rows))?;
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
            "output": r.output,
            "role": role_tool_word(r.role),
            "speaker": untrusted::name(&r.speaker),
            "group": r.group,
        })).collect::<Vec<_>>(),
        "unknown_treated_as_tweeter": unnamed,
        "next": NEXT_TEXT,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wp_roles_the_owners_sentence_reads_as_its_table() {
        let args = json!({"rows": [
            {"output": 3, "role": "tweeter", "speaker": "R"},
            {"output": 0, "role": "tweeter", "speaker": "L"},
            {"output": 1, "role": "midrange", "speaker": "L"},
            {"output": 2, "role": "woofer", "speaker": "L"},
            {"output": 4, "role": "midrange", "speaker": "R"},
            {"output": 5, "role": "woofer", "speaker": "R"},
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
        let q = question(&rows, &[6, 7], "L and r front speakers", None);
        assert!(q.starts_with("Speaker roles from your words: Out 1 · L · tweeter; Out 2 · L · midrange; Out 3 · L · woofer; Out 4 · R · tweeter"), "{q}");
        assert!(
            q.contains("Out 7, 8 not named: unknown, treated as tweeters"),
            "{q}"
        );
        // Refused: twice, out of range, an unknown role, an extra field
        for bad in [
            json!({"rows": [{"output": 0, "role": "tweeter", "speaker": "L"}, {"output": 0, "role": "woofer", "speaker": "L"}]}),
            json!({"rows": [{"output": 8, "role": "tweeter", "speaker": "L"}]}),
            json!({"rows": [{"output": 0, "role": "high", "speaker": "L"}]}),
            json!({"rows": [{"output": 0, "role": "tweeter", "speaker": "L", "group": 2}]}),
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
