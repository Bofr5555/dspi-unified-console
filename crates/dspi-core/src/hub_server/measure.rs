//! The hub's measurement documents (WP P5HUB, the fifth section of
//! `hub_server`; PLAN-P5 T11): typed calls for `measure.doc.*`
//! (`docs/integrator/hub-api.md` section 22) and the merged list of this
//! console's store and the hub's.
//!
//! - [`DspiCore::hub_measure_docs`] (`measure.doc.list`, every role),
//!   [`DspiCore::hub_measure_doc_get`] (`measure.doc.get`, every role; a large
//!   document is fetched part by part and checked whole),
//!   [`DspiCore::hub_measure_doc_put`] (Control, Admin; read and bounded here
//!   first, sent whole or in parts), [`DspiCore::hub_measure_doc_rename`],
//!   [`DspiCore::hub_measure_doc_note`] (Control, Admin) and
//!   [`DspiCore::hub_measure_doc_delete`] (Admin).
//! - [`DspiCore::measure_docs_merged`]: this console's documents
//!   ([`DspiCore::measure_docs`]) and the hub's, **the hub's winning** on an
//!   id both hold with different content (the local copy is listed beside it,
//!   never overwritten), as [`DspiCore::snapshot_list_merged`] does.
//! - `measure.doc.changed` reaches the app through the same
//!   [`super::HubServerListener`] ([`super::HubServerEvent::measure_doc`],
//!   [`super::HubServerEvent::measure_kind`]).
//!
//! As the other sections: [`DspiError::Unsupported`] with nothing sent
//! without the cap `hub_measure` (an upstream Weeb Labs hub, or a console on
//! no hub: the documents then stay in this console's own store),
//! [`DspiError::Disconnected`] while the hub session is down, the hub's
//! refusals typed (`bad_request` as [`DspiError::InvalidArgument`],
//! `denied`, `not_found`/`conflict`/`busy` as [`DspiError::Link`]), and the
//! client read-only mode lets out only the reads (`measure.doc.list`,
//! `measure.doc.get`). Each call is one or more round trips: off the UI
//! thread. No document content is ever logged.

use serde::Serialize;
#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use serde_json::{Map, Value};

#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use super::{cut, text};
use crate::measure_doc::MeasureDocInfo;
#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use crate::{DspiCore, DspiError, Result};
#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use dspi_measure::doc::{DocKind, MAX_MEASUREMENT_BYTES, MAX_NOTE_CHARS};
#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use dspi_session::measure_store::{AnyDoc, id_for, valid_id};

/// Cap: `measure.doc.*` (hub-api.md section 22).
pub const CAP_HUB_MEASURE: &str = "hub_measure";

/// A document's compact text up to this many bytes is put whole (as the
/// snapshot family's wire cap); a larger one goes in parts.
pub const MEASURE_WIRE_WHOLE_BYTES: usize = 61_440;

/// The most bytes one part of a put takes once escaped as a JSON string
/// (the hub's own part size).
pub const MEASURE_WIRE_PART_BYTES: usize = 32_768;

/// Parts of one `get` read at most: a 1 MiB document in parts of at least
/// a few bytes each, bounded.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const PARTS_MAX: u64 = 65_536;

/// Documents taken from one list at most (the hub's largest folder holds 100).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const DOCS_MAX: usize = 1024;

/// A measurement document on the hub with its text
/// ([`DspiCore::hub_measure_doc_get`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubMeasureDocument {
    /// Its entry on the hub.
    pub doc: MeasureDocInfo,
    /// The document's text (compact JSON), read and checked by its kind's
    /// reader.
    pub document: String,
}

/// A document filed on the hub ([`DspiCore::hub_measure_doc_put`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubMeasureDocPut {
    /// Its entry on the hub.
    pub doc: MeasureDocInfo,
    /// False: the hub already held this document with the same content.
    pub created: bool,
}

/// One row of the merged list ([`DspiCore::measure_docs_merged`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedMeasureDoc {
    /// The entry: the hub's where it holds the id (the hub's wins).
    pub doc: MeasureDocInfo,
    /// [`super::WHERE_LOCAL`], [`super::WHERE_HUB`] or [`super::WHERE_BOTH`]
    /// (`where` in JSON).
    #[serde(rename = "where")]
    pub location: String,
    /// This console's own copy of an id the hub holds with different
    /// content: listed beside the hub's, never overwritten by it.
    pub local_copy: bool,
}

/// The documents of both sides ([`DspiCore::measure_docs_merged`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDocsMerged {
    /// By kind (measurement, mic, target), then oldest first; on an id both
    /// sides hold with different content the hub's row comes first, then the
    /// local copy.
    pub entries: Vec<MergedMeasureDoc>,
    /// The hub's list is in it (a hub with `hub_measure` answered).
    pub hub_listed: bool,
    /// Why the hub's list is not in it, as a notice row; "" when it is, or
    /// when there is no such hub.
    pub hub_notice_key: String,
}

// ── Reading the hub's answers ─────────────────────────────────────────────

/// An entry of the hub's (the store index's own object), bounded.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn doc_from(v: &Value) -> MeasureDocInfo {
    MeasureDocInfo {
        id: text(v.get("id")),
        kind: text(v.get("kind")),
        name: text(v.get("name")),
        note: v
            .get("note")
            .and_then(Value::as_str)
            .map(|n| cut(n, MAX_NOTE_CHARS))
            .unwrap_or_default(),
        serial: text(v.get("serial")),
        saved_ms: v.get("savedMs").and_then(Value::as_i64).unwrap_or(0),
        bytes: v.get("bytes").and_then(Value::as_u64).unwrap_or(0),
    }
}

/// The bytes `c` takes in a JSON string.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if u32::from(c) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `text` cut into parts at character boundaries, each at most `budget`
/// bytes once escaped as a JSON string and never empty: the slices, in
/// order (the hub's own rule, `dspi_hub::measurements::split_parts`).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn split_parts(text: &str, budget: usize) -> Vec<&str> {
    let budget = budget.max(6);
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut used = 0usize;
    for (i, c) in text.char_indices() {
        let n = escaped_len(c);
        if used.saturating_add(n) > budget && i > start {
            out.push(text.get(start..i).unwrap_or_default());
            start = i;
            used = 0;
        }
        used = used.saturating_add(n);
    }
    out.push(text.get(start..).unwrap_or_default());
    out
}

/// The lower-case hex SHA-256 of `text`.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A broken answer of the hub's.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn protocol(msg: impl Into<String>) -> DspiError {
    DspiError::Link {
        code: "protocol".into(),
        msg: msg.into(),
    }
}

/// `text`, a document of the family, ready for the hub: read and bounded
/// by its kind's reader, a measurement without a serial given `serial`, a
/// document without a store id given the one a store would make first
/// (from its time: a measurement's `savedMs`, else `now_ms`). The compact
/// JSON text the hub files.
///
/// Errors: [`DspiError::InvalidArgument`] for a text that is not such a
/// document, or past its bound.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn hub_ready(text: &str, serial: &str, now_ms: i64) -> Result<(DocKind, Value)> {
    let mut d =
        AnyDoc::parse(text).map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
    if let AnyDoc::Measurement(m) = &mut d
        && m.measurement.serial.is_empty()
    {
        m.measurement.serial = serial.to_owned();
    }
    if !valid_id(d.id()) {
        let at = d.saved_ms().filter(|t| *t > 0).unwrap_or(now_ms);
        let id = id_for(at, 0);
        match &mut d {
            AnyDoc::Measurement(m) => m.measurement.id = id,
            AnyDoc::Mic(m) => m.mic.id = id,
            AnyDoc::Target(t) => t.target.id = id,
        }
    }
    let filed = d
        .to_text()
        .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
    let v: Value = serde_json::from_str(&filed)
        .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
    Ok((d.kind(), v))
}

/// Two entries of one id are the same document: every field the index keeps
/// agrees (the size is the filed form's, so content that differs anywhere
/// but keeps the same size and names is the same capture).
fn same(a: &MeasureDocInfo, b: &MeasureDocInfo) -> bool {
    a == b
}

/// Both sides' lists as one: the hub's wins on an id both hold.
pub(crate) fn merge(local: Vec<MeasureDocInfo>, hub: Vec<MeasureDocInfo>) -> Vec<MergedMeasureDoc> {
    use super::{WHERE_BOTH, WHERE_HUB, WHERE_LOCAL};
    let row = |doc: MeasureDocInfo, location: &str, local_copy: bool| MergedMeasureDoc {
        doc,
        location: location.into(),
        local_copy,
    };
    let mut out: Vec<MergedMeasureDoc> = Vec::with_capacity(local.len().saturating_add(hub.len()));
    let mut local: Vec<Option<MeasureDocInfo>> = local.into_iter().map(Some).collect();
    for h in hub {
        let mine = local
            .iter_mut()
            .find(|l| l.as_ref().is_some_and(|l| l.id == h.id))
            .and_then(Option::take);
        match mine {
            Some(l) if same(&l, &h) => out.push(row(h, WHERE_BOTH, false)),
            Some(l) => {
                out.push(row(h, WHERE_HUB, false));
                out.push(row(l, WHERE_LOCAL, true));
            }
            None => out.push(row(h, WHERE_HUB, false)),
        }
    }
    out.extend(
        local
            .into_iter()
            .flatten()
            .map(|l| row(l, WHERE_LOCAL, false)),
    );
    let rank = |k: &str| {
        DocKind::ALL
            .iter()
            .position(|d| d.word() == k)
            .unwrap_or(usize::MAX)
    };
    // Stable: the hub's row stays before its local copy.
    out.sort_by(|a, b| {
        (rank(&a.doc.kind), a.doc.saved_ms, &a.doc.id).cmp(&(
            rank(&b.doc.kind),
            b.doc.saved_ms,
            &b.doc.id,
        ))
    });
    out
}

// ── The calls ──────────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The hub's documents of `kind` (`measurement`, `mic`, `target`; ""
    /// every kind), oldest first within a kind (`measure.doc.list`): the
    /// measurements of device `serial` ("" the open device's; with kind ""
    /// and no device open, none), every microphone and target. Every role.
    /// Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown kind;
    /// [`DspiError::Unsupported`] without the `hub_measure` cap (nothing
    /// sent); [`DspiError::Disconnected`] (also for kind `measurement`, serial
    /// "" and no device open); the hub's refusal.
    pub fn hub_measure_docs(&self, kind: String, serial: String) -> Result<Vec<MeasureDocInfo>> {
        let kind = kind_word(&kind)?;
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_MEASURE)?;
            let mut out: Vec<MeasureDocInfo> = Vec::new();
            for k in DocKind::ALL {
                if kind.is_some_and(|want| want != k) {
                    continue;
                }
                let mut f = Map::new();
                f.insert("kind".into(), k.word().into());
                if k == DocKind::Measurement {
                    match self.hub_serial(serial.clone()) {
                        Ok(s) => {
                            f.insert("serial".into(), s.into());
                        }
                        Err(_) if kind.is_none() => continue,
                        Err(e) => return Err(e),
                    }
                }
                let reply = self.hub_ask("measure.doc.list", f)?;
                if let Some(a) = reply.get("docs").and_then(Value::as_array) {
                    out.extend(a.iter().take(DOCS_MAX).map(doc_from));
                }
            }
            Ok(out)
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (kind, serial);
            Err(DspiError::Unsupported)
        }
    }

    /// Document `id` on the hub with its text (`measure.doc.get`): a large
    /// one part by part, the whole checked against the hub's size and hash
    /// and read by its kind's reader. Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`; `protocol` for an answer
    /// that does not make the document it announced; as
    /// [`DspiCore::hub_measure_docs`].
    pub fn hub_measure_doc_get(&self, id: String) -> Result<HubMeasureDocument> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_MEASURE)?;
            let ask = |seq: Option<u64>| {
                let mut f = Map::new();
                f.insert("doc".into(), id.clone().into());
                if let Some(s) = seq {
                    f.insert("seq".into(), s.into());
                }
                self.hub_ask("measure.doc.get", f)
            };
            let first = ask(None)?;
            let doc = first
                .get("doc")
                .map(doc_from)
                .unwrap_or_else(|| doc_from(&Value::Null));
            let document = match (first.get("document"), first.get("part")) {
                (Some(Value::String(s)), _) => s.clone(),
                (Some(v @ Value::Object(_)), _) => v.to_string(),
                (None, Some(_)) => {
                    let size = first.get("size").and_then(Value::as_u64).unwrap_or(0);
                    let hash = text(first.get("sha256"));
                    if size > u64::try_from(MAX_MEASUREMENT_BYTES).unwrap_or(u64::MAX) {
                        return Err(protocol(format!(
                            "the hub announced a document of {size} bytes"
                        )));
                    }
                    let mut whole = String::new();
                    let mut reply = first;
                    let mut seq: u64 = 0;
                    loop {
                        if reply.get("seq").and_then(Value::as_u64) != Some(seq)
                            || text(reply.get("sha256")) != hash
                        {
                            return Err(protocol(
                                "the document changed on the hub while it was read; read it again",
                            ));
                        }
                        let part = reply
                            .get("part")
                            .and_then(Value::as_str)
                            .ok_or_else(|| protocol("a part of the hub's answer is missing"))?;
                        if u64::try_from(whole.len().saturating_add(part.len())).unwrap_or(u64::MAX)
                            > size
                        {
                            return Err(protocol("the hub's parts are longer than announced"));
                        }
                        whole.push_str(part);
                        if reply.get("last").and_then(Value::as_bool).unwrap_or(true) {
                            break;
                        }
                        seq = seq.saturating_add(1);
                        if seq >= PARTS_MAX {
                            return Err(protocol("too many parts"));
                        }
                        reply = ask(Some(seq))?;
                    }
                    if u64::try_from(whole.len()).unwrap_or(u64::MAX) != size
                        || sha256_hex(&whole) != hash
                    {
                        return Err(protocol(
                            "the document read from the hub is not the one it announced",
                        ));
                    }
                    whole
                }
                _ => return Err(protocol("the hub's answer has no document")),
            };
            // Read as the hub read it: a document that is not one never
            // reaches a client.
            AnyDoc::parse(&document)
                .map_err(|e| protocol(format!("the hub's document does not read: {e}")))?;
            Ok(HubMeasureDocument { doc, document })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            Err(DspiError::Unsupported)
        }
    }

    /// File `document` (a measurement, microphone or target document's text)
    /// on the hub (`measure.doc.put`), under the id it carries. It is read
    /// and bounded here first (a text that is not such a document is refused
    /// with nothing sent); a measurement without a serial takes the open
    /// device's, a document without a store id the one a store would make
    /// first (from its time). Sent whole up to 61 440 bytes compact, in parts
    /// past it. The same document again is `created: false`. Control or
    /// Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a text that is not such a
    /// document or past its bound (nothing sent), or the hub's
    /// `bad_request`; [`DspiError::Link`] `conflict` (the hub holds another
    /// document with this id, or a cap is reached) or `busy` (too many
    /// uploads on the hub); [`DspiError::Denied`] for a Viewer;
    /// [`DspiError::ReadOnly`]; as [`DspiCore::hub_measure_docs`].
    pub fn hub_measure_doc_put(&self, document: String) -> Result<HubMeasureDocPut> {
        #[cfg(feature = "link")]
        {
            // Read first: what is not a document is refused here whatever the hub.
            AnyDoc::parse(&document)
                .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
            self.hub_cap(CAP_HUB_MEASURE)?;
            let serial = self.hub_serial(String::new()).unwrap_or_default();
            let (_, doc) = hub_ready(&document, &serial, crate::events::now_ms())?;
            let compact = doc.to_string();
            let reply = if compact.len() <= MEASURE_WIRE_WHOLE_BYTES {
                let mut f = Map::new();
                f.insert("document".into(), doc);
                self.hub_ask("measure.doc.put", f)?
            } else {
                let parts = split_parts(&compact, MEASURE_WIRE_PART_BYTES);
                let n = parts.len();
                let mut last = Value::Null;
                for (seq, part) in parts.into_iter().enumerate() {
                    let mut f = Map::new();
                    f.insert("seq".into(), seq.into());
                    f.insert("last".into(), (seq.saturating_add(1) >= n).into());
                    f.insert("part".into(), part.into());
                    last = self.hub_ask("measure.doc.put", f)?;
                }
                last
            };
            Ok(HubMeasureDocPut {
                doc: reply
                    .get("doc")
                    .map(doc_from)
                    .unwrap_or_else(|| doc_from(&Value::Null)),
                created: reply
                    .get("created")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            })
        }
        #[cfg(not(feature = "link"))]
        {
            AnyDoc::parse(&document)
                .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
            Err(DspiError::Unsupported)
        }
    }

    /// Rename document `id` on the hub (`measure.doc.rename`; any Unicode, at
    /// most 256 characters). Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`; [`DspiError::InvalidArgument`]
    /// past 256 characters (the hub's `bad_request`); as
    /// [`DspiCore::hub_measure_doc_put`].
    pub fn hub_measure_doc_rename(&self, id: String, name: String) -> Result<MeasureDocInfo> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_MEASURE)?;
            let mut f = Map::new();
            f.insert("doc".into(), id.into());
            f.insert("name".into(), name.into());
            let reply = self.hub_ask("measure.doc.rename", f)?;
            Ok(reply
                .get("doc")
                .map(doc_from)
                .unwrap_or_else(|| doc_from(&Value::Null)))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (id, name);
            Err(DspiError::Unsupported)
        }
    }

    /// Set document `id`'s note on the hub (`measure.doc.note`; any Unicode,
    /// at most 4096 characters). Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_measure_doc_rename`].
    pub fn hub_measure_doc_note(&self, id: String, note: String) -> Result<MeasureDocInfo> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_MEASURE)?;
            let mut f = Map::new();
            f.insert("doc".into(), id.into());
            f.insert("note".into(), note.into());
            let reply = self.hub_ask("measure.doc.note", f)?;
            Ok(reply
                .get("doc")
                .map(doc_from)
                .unwrap_or_else(|| doc_from(&Value::Null)))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (id, note);
            Err(DspiError::Unsupported)
        }
    }

    /// Delete document `id` from the hub (`measure.doc.delete`). Admin of the
    /// hub only. This console's own copy, if any, stays. Slow: off the UI
    /// thread.
    ///
    /// Errors: [`DspiError::Denied`] below Admin; as
    /// [`DspiCore::hub_measure_doc_rename`].
    pub fn hub_measure_doc_delete(&self, id: String) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_MEASURE)?;
            let mut f = Map::new();
            f.insert("doc".into(), id.into());
            self.hub_ask("measure.doc.delete", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            Err(DspiError::Unsupported)
        }
    }

    /// The documents of `kind` (as [`DspiCore::measure_docs`]) on both sides:
    /// this console's store and, on a DSPi Link session to a hub that keeps
    /// them, the hub's ([`DspiCore::hub_measure_docs`]; its measurements are
    /// device `serial`'s, "" the open device's). Each row says where it is
    /// (`local`, `hub` or `both`). **The hub's wins:** an id both sides hold
    /// with different content (any field of the entry differs) is listed as
    /// the hub's, and this console's as a local copy beside it; nothing is
    /// overwritten. A hub that cannot be read leaves the local rows, with
    /// `hub_notice_key` saying why. A round trip on a hub: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown kind.
    pub fn measure_docs_merged(&self, kind: String, serial: String) -> Result<MeasureDocsMerged> {
        let local = self.measure_docs(kind.clone(), serial.clone())?;
        let (hub, hub_listed, hub_notice_key) = match self.hub_measure_docs(kind, serial) {
            Ok(list) => (list, true, String::new()),
            Err(DspiError::Unsupported) => (Vec::new(), false, String::new()),
            Err(e) => (Vec::new(), false, e.notice_key()),
        };
        Ok(MeasureDocsMerged {
            entries: merge(local, hub),
            hub_listed,
            hub_notice_key,
        })
    }
}

/// A kind word ("" every kind).
fn kind_word(word: &str) -> Result<Option<DocKind>> {
    if word.trim().is_empty() {
        return Ok(None);
    }
    DocKind::from_word(word)
        .map(Some)
        .ok_or_else(|| DspiError::InvalidArgument {
            msg: format!("no document kind \"{word}\" (measurement, mic or target)"),
        })
}

#[cfg(test)]
#[path = "tests_wp_p5hub.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5hub;
