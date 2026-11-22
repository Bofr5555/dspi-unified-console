//! Text the agent reads but nobody vetted (MCPDES 3.5).
//!
//! Channel, preset, snapshot, measurement, device and speaker names, notes,
//! history labels and an MCP client's own name are typed by a person or come
//! from a device or a file. Any of them may say "ignore previous instructions".
//! The rules:
//!
//! 1. Such text goes only into result fields named `*_name` or `note`, never
//!    into a tool description, the server's instructions, a prompt's text or an
//!    error message. Errors say "output 4", not its name.
//! 2. It passes [`name`] or [`note`] first: control characters (newlines and
//!    tabs included), the line and paragraph separators and every Unicode
//!    format character (`Cf`: the bidi overrides and isolates, zero-width
//!    characters, the BOM, tag characters) are dropped, the ends trimmed, and
//!    the length capped on a character boundary.
//! 3. The descriptions and `dspi://guide` say names are data, never
//!    instructions; the policy, not the model, is the real barrier.
//!
//! Not done: NFC normalisation. It needs Unicode's composition tables, which
//! no crate in the workspace carries (MCP1 adds none); a decomposed name reads
//! the same and cannot reorder text.

use serde_json::Value;

/// The longest name kept, in bytes (the firmware's `CHANNEL_NAME_LEN`).
pub const NAME_MAX_BYTES: usize = dspi_core::consts::CHANNEL_NAME_LEN;

/// The longest note kept, in bytes.
pub const NOTE_MAX_BYTES: usize = 200;

/// The longest MCP client name kept for the actor label, in bytes.
pub const CLIENT_MAX_BYTES: usize = 48;

/// Whether `c` is dropped from untrusted text.
fn dropped(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') || dspi_core::actor::is_format(c)
}

/// `text` cleaned (rule 2) and cut to at most `max` bytes.
pub fn clean(text: &str, max: usize) -> String {
    let kept: String = text.chars().filter(|c| !dropped(*c)).collect();
    let trimmed = kept.trim();
    let mut out = String::with_capacity(trimmed.len().min(max));
    for c in trimmed.chars() {
        if out.len().saturating_add(c.len_utf8()) > max {
            break;
        }
        out.push(c);
    }
    // A cut can leave a trailing space.
    out.truncate(out.trim_end().len());
    out
}

/// A name: [`clean`] to [`NAME_MAX_BYTES`].
pub fn name(text: &str) -> String {
    clean(text, NAME_MAX_BYTES)
}

/// A note or a longer label: [`clean`] to [`NOTE_MAX_BYTES`].
pub fn note(text: &str) -> String {
    clean(text, NOTE_MAX_BYTES)
}

/// The history label for an MCP client that calls itself `client`
/// ("AI · Claude Code"); "AI · agent" for a client that names nothing usable.
pub fn actor_label(client: &str) -> String {
    let client = clean(client, CLIENT_MAX_BYTES);
    if client.is_empty() {
        "AI · agent".to_owned()
    } else {
        format!("AI · {client}")
    }
}

/// Every string in `value` (keys too) with the dropped characters removed and
/// no length cap: for a whole document an agent reads as a resource, whose
/// names sit in the document's own fields.
pub fn scrub_json(value: Value) -> Value {
    let strip = |s: &str| -> String { s.chars().filter(|c| !dropped(*c)).collect() };
    match value {
        Value::String(s) => Value::String(strip(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(scrub_json).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (strip(&k), scrub_json(v)))
                .collect(),
        ),
        other => other,
    }
}
