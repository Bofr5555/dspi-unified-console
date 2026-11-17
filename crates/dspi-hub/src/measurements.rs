//! The hub's shared store of measurement documents (WP P5HUB, PLAN-P5 T11 and section
//! 1.5; `hub-api.md` section 22).
//!
//! The hub keeps the three measurement kinds of `dspi_measure::doc` (`measurement`,
//! `mic`, `target`) in `<data>/measurements/`, in the clients' own format:
//! [`dspi_session::measure_store`] (a folder per serial for measurements, `mic/` and
//! `target/`, `<id>.dspipreset` documents, an `index.json` cache per folder, atomic
//! 0600 writes, the corrupt-file rule). The snapshot store's rules apply (`snapshots.rs`):
//! **the hub's copy wins**, the id is the client's (the hub never makes one), and a filed
//! document is never overwritten (`conflict`).
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `measure.doc.list {id, kind, serial?}` | every role | `ok {kind, serial, docs: [entry]}`, oldest first |
//! | `measure.doc.get {id, doc, seq?}` | every role | `ok {doc: entry, document}`, or one part (below) |
//! | `measure.doc.put {id, document}` or `{id, seq, last, part}` | Control, Admin | `ok {doc: entry, created}`; a part before the last `ok {seq, received}` |
//! | `measure.doc.rename {id, doc, name}` | Control, Admin | `ok {doc: entry}` |
//! | `measure.doc.note {id, doc, note}` | Control, Admin | `ok {doc: entry}` |
//! | `measure.doc.delete {id, doc}` | Admin | `ok {kind, serial, doc: id}` |
//!
//! `doc` is a document's store id; an entry is the store index's own object (`{id, kind,
//! name, note, serial, savedMs, bytes}`, `serial` "" for a `mic` or `target`). Each
//! change queues one event `measure.doc.changed {v, hub, kind, serial?, doc, change}`
//! (`change`: `put`, `rename`, `note`, `delete`; `serial` only for a measurement), sent to
//! every session after the answer. The family's cap is [`CAP`].
//!
//! **Large documents.** A DSPi Link frame is at most 64 KiB (`hello.limits.max_frame`); a
//! measurement may be 1 MiB. So:
//! - `measure.doc.get` answers the whole document (`document`, its compact JSON text) when
//!   that answer fits one frame; otherwise, and whenever the request names a `seq`, one
//!   **part**: `{doc, seq, last, part, size, sha256}`: `part` a slice of the compact text
//!   (cut at a character, at most [`PART_BYTES`] once escaped as a JSON string), `size`
//!   the whole compact text's bytes and `sha256` its hash (lower-case hex), the same in
//!   every part, so a client can tell a document that changed between two parts. Ask for
//!   the next part with `{doc, seq: n + 1}` until `last`.
//! - `measure.doc.put` takes the document whole (`document`, its text or the JSON object),
//!   or in parts: `{seq, last, part}` from `seq` 0, in order, one upload per session (a
//!   `seq` 0 starts it again), at most [`MAX_UPLOADS`] open on the hub (`busy` past it),
//!   dropped when idle for [`UPLOAD_TIMEOUT`] or when its session ends; the text is at
//!   most 1 MiB and is read as a document only when whole. An out-of-order part drops the
//!   upload (`bad_request`).
//!
//! **Checks.** Every document is read with its kind's reader (`dspi_measure::doc`, through
//! [`AnyDoc`]) before it is filed: another kind, schema or shape, or past its bound (1 MiB
//! a measurement, 256 KiB a mic or target, measured on the filed, pretty-printed form) is
//! `bad_request`; so is a document without a store id, and a measurement that names no
//! device. An id the hub holds with other content, as another kind or for another device,
//! is `conflict`, never an overwrite; the same document again is `ok` with `created:
//! false`. 100 measurements a device, 32 microphones, 64 targets and 64 MiB in all; a cap
//! reached is `conflict`. Names (at most 256 characters) and notes (4096) take any
//! Unicode; longer is `bad_request`. A stored document that no longer reads is set aside
//! by the store (`<id>.dspipreset.bad-<time>`) and is `not_found` from then on.
//!
//! **What the hub never does:** act on a document (it writes no device from one and
//! measures nothing: the microphone is the client's, OD-P5f), make an id, or log a
//! document's content (a log line names the message and the store's error only).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use dspi_measure::doc::{DocKind, MAX_MEASUREMENT_BYTES, MAX_NAME_CHARS, MAX_NOTE_CHARS};
use dspi_session::measure_store::{AnyDoc, MeasureEntry, MeasureStore, valid_id};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Role, Session, SessionId};

/// The folder under the hub's data folder.
pub const DIR_NAME: &str = "measurements";

/// The cap `hello.caps` announces for this family.
pub const CAP: &str = "hub_measure";

/// The event a change queues.
pub const CHANGED_EVENT: &str = "measure.doc.changed";

/// The messages this module answers.
pub const MESSAGES: [&str; 6] = [
    "measure.doc.list",
    "measure.doc.get",
    "measure.doc.put",
    "measure.doc.rename",
    "measure.doc.note",
    "measure.doc.delete",
];

/// The answers' version.
const VERSION: u32 = 1;

/// A document's compact text up to this many bytes may be put whole (as the snapshot
/// family's wire cap: `max_frame` 65 536 less 4 KiB); a larger one comes in parts.
pub const MAX_WHOLE_BYTES: usize = 61_440;

/// The most bytes one part's slice takes once escaped as a JSON string. With an entry
/// (at most about 27 KB escaped: a 256-character name, a 4096-character note) and the
/// message around it, a part's answer stays within one 64 KiB frame.
pub const PART_BYTES: usize = 32_768;

/// Uploads in parts open on the hub at once (one per session); `busy` past it.
pub const MAX_UPLOADS: usize = 8;

/// An upload with no part for this long is dropped.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// The largest frame the hub sends or takes ([`crate::server::MAX_FRAME`]).
const MAX_FRAME: usize = crate::server::MAX_FRAME as usize;

/// The longest serial accepted, in bytes.
const MAX_SERIAL_BYTES: usize = 64;

/// How much larger than the largest document a whole `put`'s text may be before it is
/// parsed (a client may send it with any indentation).
const MAX_INPUT_FACTOR: usize = 4;

/// Change events kept for the service; the oldest is dropped beyond it.
const MAX_PENDING_EVENTS: usize = 64;

/// Why a store call was refused. [`MeasureDocError::code`] is the wire code.
#[derive(Debug, thiserror::Error)]
pub enum MeasureDocError {
    /// The request or document is not what the call takes.
    #[error("{0}")]
    BadRequest(String),
    /// No document with this id.
    #[error("{0}")]
    NotFound(String),
    /// A cap, or an id the hub holds otherwise.
    #[error("{0}")]
    Conflict(String),
    /// Too many uploads open on the hub.
    #[error("{0}")]
    Busy(String),
    /// The disk.
    #[error("store: {0}")]
    Io(#[from] io::Error),
}

impl MeasureDocError {
    /// The error code on the wire (`hub-api.md` section 1.6; no new code).
    pub fn code(&self) -> &'static str {
        match self {
            MeasureDocError::BadRequest(_) => "bad_request",
            MeasureDocError::NotFound(_) => "not_found",
            MeasureDocError::Conflict(_) => "conflict",
            MeasureDocError::Busy(_) => "busy",
            MeasureDocError::Io(_) => "internal",
        }
    }
}

fn bad(msg: impl Into<String>) -> MeasureDocError {
    MeasureDocError::BadRequest(msg.into())
}

/// A store error of a call that names document `id`.
fn read_err(id: &str, e: io::Error) -> MeasureDocError {
    match e.kind() {
        io::ErrorKind::NotFound => MeasureDocError::NotFound(format!("no document {id}")),
        // Two folders hold it (copied in by hand): acted on by no call.
        io::ErrorKind::InvalidInput => MeasureDocError::Conflict(e.to_string()),
        // Damaged, and set aside by the store's corrupt-file rule.
        io::ErrorKind::InvalidData => {
            log::warn!("hub measurements: document {id} was damaged and set aside");
            MeasureDocError::NotFound(format!(
                "document {id} no longer reads and was set aside on the hub"
            ))
        }
        _ => MeasureDocError::Io(e),
    }
}

/// A store error of a put.
fn put_err(e: io::Error) -> MeasureDocError {
    match e.kind() {
        io::ErrorKind::InvalidData => MeasureDocError::BadRequest(e.to_string()),
        io::ErrorKind::QuotaExceeded => MeasureDocError::Conflict(e.to_string()),
        _ => MeasureDocError::Io(e),
    }
}

/// A serial a message may name: 1 to 64 bytes, no control characters.
fn check_serial(serial: &str) -> Result<(), MeasureDocError> {
    if serial.is_empty() || serial.len() > MAX_SERIAL_BYTES || serial.chars().any(char::is_control)
    {
        return Err(bad(format!(
            "serial is 1 to {MAX_SERIAL_BYTES} bytes without control characters"
        )));
    }
    Ok(())
}

/// The bytes `c` takes in a JSON string.
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if u32::from(c) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `text` cut into parts at character boundaries, each at most `budget` bytes once
/// escaped as a JSON string (and never empty): the byte ranges, in order. An empty text
/// is one empty part.
pub fn split_parts(text: &str, budget: usize) -> Vec<(usize, usize)> {
    let budget = budget.max(6);
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut used = 0usize;
    for (i, c) in text.char_indices() {
        let n = escaped_len(c);
        if used.saturating_add(n) > budget && i > start {
            out.push((start, i));
            start = i;
            used = 0;
        }
        used = used.saturating_add(n);
    }
    out.push((start, text.len()));
    out
}

/// The lower-case hex SHA-256 of `text`.
pub fn sha256_hex(text: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
}

/// What a `put` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put {
    /// The document's entry.
    pub entry: MeasureEntry,
    /// False when the hub already held this very document.
    pub created: bool,
}

/// An upload in parts.
struct Upload {
    next_seq: u64,
    text: String,
    touched: Instant,
    session: Weak<Session>,
}

/// The hub's measurement store over the client store's format.
pub struct HubMeasurements {
    store: MeasureStore,
    /// Serialises the store's calls (several connections call in) and holds the
    /// pending change events.
    pending: Mutex<Vec<Value>>,
    /// Uploads in parts, by session. A leaf lock, never held with `pending`.
    uploads: Mutex<BTreeMap<SessionId, Upload>>,
}

impl std::fmt::Debug for HubMeasurements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubMeasurements")
            .field("root", &self.store.root())
            .finish_non_exhaustive()
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl HubMeasurements {
    /// The store in `<data_dir>/measurements/`. Nothing is touched until the first write.
    pub fn open(data_dir: &Path) -> Self {
        Self::with_root(data_dir.join(DIR_NAME))
    }

    /// The store in `root` itself (tests).
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            store: MeasureStore::new(root),
            pending: Mutex::new(Vec::new()),
            uploads: Mutex::new(BTreeMap::new()),
        }
    }

    /// The folder the documents are in.
    pub fn root(&self) -> &Path {
        self.store.root()
    }

    /// The documents of `kind`, oldest first: a measurement's of device `serial`
    /// (required), every microphone or target (`serial` ignored). Never fails on disk: a
    /// missing folder is an empty list, a torn index is rebuilt.
    pub fn list(
        &self,
        kind: DocKind,
        serial: Option<&str>,
    ) -> Result<Vec<MeasureEntry>, MeasureDocError> {
        let serial = match kind {
            DocKind::Measurement => {
                let s = serial
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| bad("serial is required for measurements"))?;
                check_serial(s)?;
                Some(s)
            }
            DocKind::Mic | DocKind::Target => None,
        };
        let _g = guard(&self.pending);
        Ok(self.store.list(Some(kind), serial))
    }

    /// Document `id`'s entry and its text in the compact form the DSPi Link carries.
    pub fn get(&self, id: &str) -> Result<(MeasureEntry, String), MeasureDocError> {
        if !valid_id(id) {
            return Err(MeasureDocError::NotFound(format!("no document {id}")));
        }
        let (entry, text) = {
            let _g = guard(&self.pending);
            self.store.read(id).map_err(|e| read_err(id, e))?
        };
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| MeasureDocError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
        Ok((entry, v.to_string()))
    }

    /// File `text`, a client's document, under the id it carries (see the module
    /// documentation for the checks and refusals).
    pub fn put(&self, text: &str) -> Result<Put, MeasureDocError> {
        let limit = MAX_MEASUREMENT_BYTES.saturating_mul(MAX_INPUT_FACTOR);
        if text.len() > limit {
            return Err(bad(format!(
                "a measurement document is at most {MAX_MEASUREMENT_BYTES} bytes; this text is {}",
                text.len()
            )));
        }
        let d = AnyDoc::parse(text).map_err(|e| {
            bad(format!(
                "not a measurement, microphone or target document: {e}"
            ))
        })?;
        let id = d.id().to_string();
        if !valid_id(&id) {
            return Err(bad(
                "the document carries no store id (measurement.id, mic.id or target.id); \
                 the hub never makes one",
            ));
        }
        if d.kind() == DocKind::Measurement {
            check_serial(d.serial())
                .map_err(|_| bad("a measurement names its device (measurement.serial)"))?;
        }
        // The filed form (validated, pretty-printed): every bound is measured on it.
        let filed = d.to_text().map_err(|e| bad(e.to_string()))?;

        let mut pending = guard(&self.pending);
        match self.store.read(&id) {
            Ok((held, held_text)) => {
                let same =
                    held.kind == d.kind().word() && held.serial == d.serial() && held_text == filed;
                if !same {
                    let why = if held.kind != d.kind().word() {
                        format!("the hub holds {id} as a {} document", held.kind)
                    } else if held.serial != d.serial() {
                        format!("the hub holds {id} for another device")
                    } else {
                        format!("the hub holds a different document {id}; it is never overwritten")
                    };
                    return Err(MeasureDocError::Conflict(why));
                }
                return Ok(Put {
                    entry: held,
                    created: false,
                });
            }
            // Free, or set aside just now by the corrupt-file rule.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::InvalidData
                ) => {}
            Err(e) => return Err(read_err(&id, e)),
        }
        let (entry, _) = self.store.put(&filed, 0).map_err(put_err)?;
        if entry.id != id {
            // The store gives a taken id a new one; under this lock the id was free, so
            // this is a store the hub does not own alone. Undo, never keep a made id.
            let _ = self.store.delete(&entry.id);
            return Err(MeasureDocError::Conflict(format!(
                "document {id} could not be filed under its own id"
            )));
        }
        push_event(&mut pending, &entry, "put");
        Ok(Put {
            entry,
            created: true,
        })
    }

    /// Rename document `id` (any Unicode, at most 256 characters).
    pub fn rename(&self, id: &str, name: &str) -> Result<MeasureEntry, MeasureDocError> {
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(bad(format!(
                "a name is at most {MAX_NAME_CHARS} characters"
            )));
        }
        self.update(id, "rename", |s| s.rename(id, name))
    }

    /// Set document `id`'s note (any Unicode, at most 4096 characters).
    pub fn set_note(&self, id: &str, note: &str) -> Result<MeasureEntry, MeasureDocError> {
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(bad(format!(
                "a note is at most {MAX_NOTE_CHARS} characters"
            )));
        }
        self.update(id, "note", |s| s.set_note(id, note))
    }

    fn update(
        &self,
        id: &str,
        change: &str,
        f: impl FnOnce(&MeasureStore) -> io::Result<MeasureEntry>,
    ) -> Result<MeasureEntry, MeasureDocError> {
        if !valid_id(id) {
            return Err(MeasureDocError::NotFound(format!("no document {id}")));
        }
        let mut pending = guard(&self.pending);
        let entry = f(&self.store).map_err(|e| match e.kind() {
            // A rename or note that would take the document past its bound.
            io::ErrorKind::InvalidData if self.store.read(id).is_ok() => MeasureDocError::Conflict(
                format!("document {id} would be past its bound with this change"),
            ),
            _ => read_err(id, e),
        })?;
        push_event(&mut pending, &entry, change);
        Ok(entry)
    }

    /// Delete document `id`. Answers the entry it had.
    pub fn delete(&self, id: &str) -> Result<MeasureEntry, MeasureDocError> {
        if !valid_id(id) {
            return Err(MeasureDocError::NotFound(format!("no document {id}")));
        }
        let mut pending = guard(&self.pending);
        let (entry, _) = self.store.read(id).map_err(|e| read_err(id, e))?;
        self.store.delete(id).map_err(|e| read_err(id, e))?;
        push_event(&mut pending, &entry, "delete");
        Ok(entry)
    }

    /// The `measure.doc.changed` events queued since the last call, oldest first (at most
    /// 64 are kept), as the text to send to every session, with `hub` the hub's id.
    pub fn take_changed(&self, hub: &str) -> Vec<String> {
        std::mem::take(&mut *guard(&self.pending))
            .into_iter()
            .map(|mut e| {
                if let Some(m) = e.as_object_mut() {
                    m.insert("hub".into(), json!(hub));
                }
                e.to_string()
            })
            .collect()
    }

    /// Uploads that are open now (tests and diagnostics).
    pub fn uploads_open(&self) -> usize {
        guard(&self.uploads).len()
    }

    /// One part of an upload of `session`'s, at `now`. `Ok(None)` while more parts are
    /// expected (the bytes received so far in the answer), `Ok(Some(text))` with the
    /// whole text after the last.
    fn upload_part(
        &self,
        session: &Arc<Session>,
        now: Instant,
        seq: u64,
        part: &str,
        last: bool,
    ) -> Result<Result<String, usize>, MeasureDocError> {
        let sid = session.id();
        let mut ups = guard(&self.uploads);
        ups.retain(|_, u| {
            u.session.strong_count() > 0
                && now.saturating_duration_since(u.touched) < UPLOAD_TIMEOUT
        });
        if seq == 0 {
            ups.remove(&sid);
            if ups.len() >= MAX_UPLOADS {
                return Err(MeasureDocError::Busy(format!(
                    "{MAX_UPLOADS} uploads are open on the hub; try again later"
                )));
            }
            ups.insert(
                sid,
                Upload {
                    next_seq: 0,
                    text: String::new(),
                    touched: now,
                    session: Arc::downgrade(session),
                },
            );
        }
        let Some(u) = ups.get_mut(&sid) else {
            return Err(bad(
                "no upload is open in this session (start with seq 0; it may have timed out)",
            ));
        };
        let wrong = if u.next_seq != seq {
            Some(format!(
                "part {seq} is out of order: {} was next",
                u.next_seq
            ))
        } else if u.text.len().saturating_add(part.len()) > MAX_MEASUREMENT_BYTES {
            Some(format!(
                "a document is at most {MAX_MEASUREMENT_BYTES} bytes"
            ))
        } else {
            None
        };
        if let Some(why) = wrong {
            ups.remove(&sid);
            return Err(bad(format!("{why}; the upload was dropped")));
        }
        u.text.push_str(part);
        u.next_seq = seq.saturating_add(1);
        u.touched = now;
        if !last {
            return Ok(Err(u.text.len()));
        }
        Ok(Ok(ups.remove(&sid).map(|u| u.text).unwrap_or_default()))
    }

    /// Answer a `measure.doc.*` message for `session` at `now` (the hub clock's): the
    /// whole reply text, or `None` for any other message.
    pub fn handle_message(
        &self,
        session: &Arc<Session>,
        now: Instant,
        msg: &Value,
    ) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if !MESSAGES.contains(&t) {
            return None;
        }
        let Some(id) = msg.get("id").filter(|id| id.is_i64() || id.is_u64()) else {
            return Some(err(None, "bad_request", "id is required"));
        };
        let need = match t {
            "measure.doc.list" | "measure.doc.get" => Role::Viewer,
            "measure.doc.delete" => Role::Admin,
            _ => Role::Control,
        };
        if session.role() < need {
            return Some(err(Some(id), "denied", "this role may not"));
        }
        let text = |k: &str| msg.get(k).and_then(Value::as_str);
        let doc = text("doc");
        let result = match t {
            "measure.doc.list" => match text("kind").and_then(DocKind::from_word) {
                None => Err(bad("kind is measurement, mic or target")),
                Some(kind) => {
                    let serial = text("serial");
                    self.list(kind, serial).map(|docs| {
                        let serial = match kind {
                            DocKind::Measurement => serial.unwrap_or_default(),
                            _ => "",
                        };
                        json!({"kind": kind.word(), "serial": serial, "docs": docs})
                    })
                }
            },
            "measure.doc.get" => match (doc, msg.get("seq")) {
                (None, _) => Err(bad("doc is required")),
                (Some(_), Some(s)) if !s.is_u64() => Err(bad("seq is a whole number from 0")),
                (Some(d), seq) => self.get_answer(id, d, seq.and_then(Value::as_u64)),
            },
            "measure.doc.put" => self.put_message(session, now, msg),
            "measure.doc.rename" => match (doc, text("name")) {
                (Some(d), Some(name)) => self.rename(d, name).map(|e| json!({"doc": e})),
                _ => Err(bad("doc and name are required")),
            },
            "measure.doc.note" => match (doc, text("note")) {
                (Some(d), Some(note)) => self.set_note(d, note).map(|e| json!({"doc": e})),
                _ => Err(bad("doc and note are required")),
            },
            // "measure.doc.delete", the last of MESSAGES.
            _ => match doc {
                None => Err(bad("doc is required")),
                Some(d) => self
                    .delete(d)
                    .map(|e| json!({"kind": e.kind, "serial": e.serial, "doc": e.id})),
            },
        };
        Some(match result {
            Ok(body) => ok(id, body),
            Err(e) => {
                if let MeasureDocError::Io(io) = &e {
                    log::warn!("hub measurements: {t}: {io}");
                }
                err(Some(id), e.code(), &e.to_string())
            }
        })
    }

    /// `measure.doc.get`'s answer body: the whole document when it fits one frame and no
    /// part was asked for, else part `seq` (0 when none).
    fn get_answer(
        &self,
        req: &Value,
        doc: &str,
        seq: Option<u64>,
    ) -> Result<Value, MeasureDocError> {
        let (entry, compact) = self.get(doc)?;
        if seq.is_none() && compact.len() <= MAX_FRAME {
            let whole = json!({"doc": entry, "document": compact});
            if ok(req, whole.clone()).len() <= MAX_FRAME {
                return Ok(whole);
            }
        }
        let parts = split_parts(&compact, PART_BYTES);
        let seq = seq.unwrap_or(0);
        let n = usize::try_from(seq).unwrap_or(usize::MAX);
        let Some(&(a, b)) = parts.get(n) else {
            return Err(bad(format!(
                "seq {seq} is past the last part ({} parts)",
                parts.len()
            )));
        };
        let part = compact.get(a..b).unwrap_or_default();
        Ok(json!({
            "doc": entry,
            "seq": seq,
            "last": n.saturating_add(1) >= parts.len(),
            "part": part,
            "size": compact.len(),
            "sha256": sha256_hex(&compact),
        }))
    }

    /// `measure.doc.put`'s answer body, whole or by parts.
    fn put_message(
        &self,
        session: &Arc<Session>,
        now: Instant,
        msg: &Value,
    ) -> Result<Value, MeasureDocError> {
        if let Some(part) = msg.get("part") {
            let part = part
                .as_str()
                .ok_or_else(|| bad("part is a slice of the document's text"))?;
            let seq = msg
                .get("seq")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad("seq is a whole number from 0"))?;
            let last = match msg.get("last") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(bad("last is true or false")),
            };
            return match self.upload_part(session, now, seq, part, last)? {
                Err(received) => Ok(json!({"seq": seq, "received": received})),
                Ok(text) => {
                    let received = text.len();
                    self.put(&text).map(|p| {
                        json!({"doc": p.entry, "created": p.created,
                               "seq": seq, "received": received})
                    })
                }
            };
        }
        let text = match msg.get("document") {
            Some(Value::String(s)) => s.clone(),
            Some(v @ Value::Object(_)) => v.to_string(),
            _ => {
                return Err(bad(
                    "document is required (its text or the object), or its parts",
                ));
            }
        };
        self.put(&text)
            .map(|p| json!({"doc": p.entry, "created": p.created}))
    }
}

fn push_event(pending: &mut Vec<Value>, entry: &MeasureEntry, change: &str) {
    if pending.len() >= MAX_PENDING_EVENTS {
        pending.remove(0);
    }
    let mut e = json!({"t": CHANGED_EVENT, "v": VERSION, "kind": entry.kind,
        "doc": entry.id, "change": change});
    if !entry.serial.is_empty()
        && let Some(m) = e.as_object_mut()
    {
        m.insert("serial".into(), json!(entry.serial));
    }
    pending.push(e);
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

fn ok(id: &Value, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

// ── The service ─────────────────────────────────────────────────────────────

/// The `measure.doc.*` family, each change broadcast to every session as
/// `measure.doc.changed` after the answer is made.
struct MeasureService {
    store: HubMeasurements,
    clock: Arc<dyn HubClock>,
}

impl HubService for MeasureService {
    fn name(&self) -> &'static str {
        "measurements"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let answer = self.store.handle_message(session, self.clock.now(), msg)?;
        for event in self.store.take_changed(hub.hub_id()) {
            hub.inner.relay.broadcast_event(&event);
        }
        Some(answer)
    }
}

/// The service, on a hub with a data folder and [`ServiceConfig::measure`] on.
pub(crate) fn service(
    config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    if !config.measure {
        return None;
    }
    let dir = setup.data_dir?;
    Some(Arc::new(MeasureService {
        store: HubMeasurements::open(dir),
        clock: Arc::clone(setup.clock),
    }))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5hub;
