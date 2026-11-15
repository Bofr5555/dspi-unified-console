//! The hub's shared snapshot store (WP P2SS, PRO phase 2, OD-P1b).
//!
//! The hub keeps a shared copy of each device's snapshots in `<data>/snapshots/`, in the
//! clients' own format: [`dspi_session::snapshot_store`] (one folder per serial,
//! `<id>.dspipreset` documents, an `index.json` cache). A client's list can therefore merge
//! with the hub's entry by entry (P2CC1); **the hub's copy wins** on an id both hold with
//! different content, and the hub never overwrites one of its snapshots silently.
//!
//! [`HubSnapshots`] is the store, [`HubSnapshots::handle_message`] answers the
//! `snapshot.*` messages below. It is **not registered as a service yet**: P2PA registers
//! it beside the `api` family (and adds `snapshot.recall`, which needs the hub's core).
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `snapshot.list {id, serial}` | every role | `ok {serial, snapshots: [entry]}`, oldest first |
//! | `snapshot.get {id, snapshot}` | every role | `ok {snapshot: entry, document: text}` |
//! | `snapshot.put {id, serial, document}` | Control, Admin | `ok {snapshot: entry, created}` |
//! | `snapshot.rename {id, snapshot, name}` | Control, Admin | `ok {snapshot: entry}` |
//! | `snapshot.note {id, snapshot, note}` | Control, Admin | `ok {snapshot: entry}` |
//! | `snapshot.delete {id, snapshot}` | Admin | `ok {serial, snapshot: id}` |
//!
//! `id` is the request id, as in every hub message; `snapshot` is a snapshot's store id
//! (as `alarm` is an alarm's in `system.alarm.ack`). An entry is the index's own object
//! (`{id, name, note, serial, savedMs, platform, inputChannels, outputChannels,
//! wireFormat}`). Every answer carries `"v": 1`. Each change queues one event
//! `snapshot.changed {v, hub, serial, snapshot, change}` (`change`: `put`, `rename`, `note`,
//! `delete`) that the registrar drains with [`HubSnapshots::take_changed`], passing the
//! hub's id for `hub`. The family's cap is [`CAP`], `hub_snapshots` (`snapshot` is spec
//! 7.5's device cache).
//!
//! **`snapshot.put`** takes a document (its text, or the JSON object) as a client's store
//! files it: `meta.kind = "snapshot"`, a `snapshot` block naming the same serial as the
//! message and the client's id (capture-time ids are unique, so one snapshot keeps one id on
//! both sides). Refusals: `bad_request` for a document that is not such a snapshot or names
//! another device's serial; `conflict` for the caps (OD-P2k: [`Caps`]), for an id the hub
//! holds with different content or for another device, never an overwrite. The same
//! document put again is `ok` with `created: false`.
//!
//! A document is filed pretty-printed (the form the client store's rename and note write),
//! and every cap is measured on that form, so a rename never makes a filed snapshot
//! unreadable. Bounded: a document is at most 256 KiB, a name 256 characters, a note 4096, a serial 64
//! bytes; 200 snapshots per device and 64 MiB in all. Nothing here panics on a message or
//! on what is on disk: a torn `index.json` is rebuilt from the documents by the client
//! store's own `list`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dspi_session::snapshot_store::{
    self, BLOCK, INDEX_FILE, SNAPSHOT_EXT, SnapshotEntry, SnapshotStore, valid_id,
};
use serde_json::{Value, json};

use crate::session::Role;

/// The folder under the hub's data folder.
pub const DIR_NAME: &str = "snapshots";

/// The answers' version (the P1HA/P1HL convention).
const VERSION: u32 = 1;

/// The messages this module answers. `snapshot.recall` is P2PA's.
pub const MESSAGES: [&str; 6] = [
    "snapshot.list",
    "snapshot.get",
    "snapshot.put",
    "snapshot.rename",
    "snapshot.note",
    "snapshot.delete",
];

/// The cap `hello.caps` announces for this family once registered.
// -- WP P2SS -- review p2a F1: not "snapshot", which every hub already announces for
// spec 7.5's device snapshot cache.
pub const CAP: &str = "hub_snapshots";

/// The event a change queues.
pub const CHANGED_EVENT: &str = "snapshot.changed";

/// The longest name kept, in characters.
pub const MAX_NAME_CHARS: usize = 256;
/// The longest note kept, in characters.
pub const MAX_NOTE_CHARS: usize = 4096;
/// The longest serial accepted, in bytes.
pub const MAX_SERIAL_BYTES: usize = 64;
/// How much larger than the cap a document's text may be before it is parsed (its
/// stored, pretty-printed form is what the cap measures).
const MAX_INPUT_FACTOR: u64 = 4;
/// Change events kept for the registrar; the oldest is dropped beyond it.
const MAX_PENDING_EVENTS: usize = 64;

/// The retention caps (OD-P2k).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Snapshots per device.
    pub per_device: usize,
    /// Bytes per document.
    pub doc_bytes: u64,
    /// Bytes of documents in all.
    pub total_bytes: u64,
}

impl Default for Caps {
    /// 200 per device, 256 KiB each, 64 MiB in all.
    fn default() -> Self {
        Self {
            per_device: 200,
            doc_bytes: 256 * 1024,
            total_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Why a store call was refused. [`SnapshotError::code`] is the wire code.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// The request or document is not what the call takes.
    #[error("{0}")]
    BadRequest(String),
    /// No snapshot with this id.
    #[error("no snapshot {0}")]
    NotFound(String),
    /// A cap, or an id the hub holds otherwise.
    #[error("{0}")]
    Conflict(String),
    /// The disk.
    #[error("store: {0}")]
    Io(#[from] io::Error),
}

impl SnapshotError {
    /// The error code on the wire.
    pub fn code(&self) -> &'static str {
        match self {
            SnapshotError::BadRequest(_) => "bad_request",
            SnapshotError::NotFound(_) => "not_found",
            SnapshotError::Conflict(_) => "conflict",
            SnapshotError::Io(_) => "internal",
        }
    }
}

/// What a `put` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put {
    /// The snapshot's entry.
    pub entry: SnapshotEntry,
    /// False when the hub already held this very document.
    pub created: bool,
}

/// The hub's snapshot store over the client store's format.
#[derive(Debug)]
pub struct HubSnapshots {
    store: SnapshotStore,
    caps: Caps,
    /// Serialises the store's writes (several connections call in) and holds the
    /// pending change events.
    pending: Mutex<Vec<Value>>,
}

fn bad(msg: impl Into<String>) -> SnapshotError {
    SnapshotError::BadRequest(msg.into())
}

fn map_io(id: &str, e: io::Error) -> SnapshotError {
    match e.kind() {
        io::ErrorKind::NotFound => SnapshotError::NotFound(id.to_string()),
        // The client store's answer for an id two folders hold.
        io::ErrorKind::InvalidInput => SnapshotError::Conflict(e.to_string()),
        _ => SnapshotError::Io(e),
    }
}

/// `dir` and its parents, the user's alone on Unix (0700).
fn create_dir(dir: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// On Unix, `path` the user's alone (0600). The client store writes with the
/// process's umask; the hub's files are 0600 (PLAN-P2, "Store files").
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.is_file() {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Write `text` to `path` atomically (a temporary file, then a rename), 0600 on Unix.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    {
        use std::io::Write;
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// A serial a message may name: 1 to 64 bytes, no control characters.
fn check_serial(serial: &str) -> Result<(), SnapshotError> {
    if serial.is_empty() || serial.len() > MAX_SERIAL_BYTES || serial.chars().any(char::is_control)
    {
        return Err(bad(format!(
            "serial is 1 to {MAX_SERIAL_BYTES} bytes without control characters"
        )));
    }
    Ok(())
}

impl HubSnapshots {
    /// The store in `<data_dir>/snapshots/` with the default caps. Nothing is touched
    /// until the first write.
    pub fn open(data_dir: &Path) -> Self {
        Self::with_caps(data_dir.join(DIR_NAME), Caps::default())
    }

    /// The store in `root` itself, with `caps` (tests, and Admin-adjusted caps).
    pub fn with_caps(root: impl Into<PathBuf>, caps: Caps) -> Self {
        Self {
            store: SnapshotStore::new(root),
            caps,
            pending: Mutex::new(Vec::new()),
        }
    }

    /// The folder the documents are in.
    pub fn root(&self) -> &Path {
        self.store.root()
    }

    /// The caps in force.
    pub fn caps(&self) -> Caps {
        self.caps
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Value>> {
        // A panic elsewhere while holding it left only a list of strings behind.
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn dir(&self, serial: &str) -> PathBuf {
        self.root().join(snapshot_store::dir_name(serial))
    }

    fn doc_path(&self, serial: &str, id: &str) -> PathBuf {
        self.dir(serial).join(format!("{id}.{SNAPSHOT_EXT}"))
    }

    /// The serial whose folder holds `id`, if exactly one does.
    fn holder(&self, id: &str) -> Result<Option<String>, SnapshotError> {
        let mut found = self
            .store
            .serials()
            .into_iter()
            .filter(|s| self.doc_path(s, id).is_file());
        let first = found.next();
        if let (Some(a), Some(b)) = (&first, found.next()) {
            return Err(SnapshotError::Conflict(format!(
                "snapshot {id} is held for more than one device ({a}, {b})"
            )));
        }
        Ok(first)
    }

    /// Bytes of documents in the whole store.
    fn total_bytes(&self) -> u64 {
        let ext = format!(".{SNAPSHOT_EXT}");
        let mut total: u64 = 0;
        for serial in self.store.serials() {
            let Ok(rd) = fs::read_dir(self.dir(&serial)) else {
                continue;
            };
            for e in rd.flatten() {
                let is_doc = e.file_name().to_str().is_some_and(|n| n.ends_with(&ext));
                if is_doc && let Ok(m) = e.metadata() {
                    total = total.saturating_add(m.len());
                }
            }
        }
        total
    }

    /// `serial`'s snapshots, oldest first. Never fails: a missing folder is an empty
    /// list and a torn index is rebuilt from the documents.
    pub fn list(&self, serial: &str) -> Result<Vec<SnapshotEntry>, SnapshotError> {
        check_serial(serial)?;
        let _g = self.lock();
        let out = self.store.list(serial);
        restrict(&self.dir(serial).join(INDEX_FILE));
        Ok(out)
    }

    /// Snapshot `id`'s entry and document text.
    pub fn get(&self, id: &str) -> Result<(SnapshotEntry, String), SnapshotError> {
        if !valid_id(id) {
            return Err(SnapshotError::NotFound(id.to_string()));
        }
        let _g = self.lock();
        let serial = self
            .holder(id)?
            .ok_or_else(|| SnapshotError::NotFound(id.to_string()))?;
        // Every document the hub files or rewrites is at most the cap in the form it is
        // stored in (review p2a F4); a larger one was not put here.
        let len = fs::metadata(self.doc_path(&serial, id))
            .map_err(|e| map_io(id, e))?
            .len();
        if len > self.caps.doc_bytes {
            return Err(SnapshotError::Conflict(format!(
                "snapshot {id}'s document is {len} bytes, over the cap"
            )));
        }
        self.store.read(id).map_err(|e| map_io(id, e))
    }

    /// File `text`, a client's snapshot document of `serial`, under the id its `snapshot`
    /// block names (see the module documentation for the checks and refusals).
    pub fn put(&self, serial: &str, text: &str) -> Result<Put, SnapshotError> {
        check_serial(serial)?;
        // A bound before parsing only; the cap is measured on the stored form below.
        let raw = u64::try_from(text.len()).unwrap_or(u64::MAX);
        if raw > self.caps.doc_bytes.saturating_mul(MAX_INPUT_FACTOR) {
            return Err(SnapshotError::Conflict(format!(
                "a snapshot document is at most {} bytes; this one is {raw}",
                self.caps.doc_bytes
            )));
        }
        let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| bad(format!("document is not JSON: {e}")))?;
        if !doc.is_object() {
            return Err(bad("a snapshot document is a JSON object"));
        }
        let kind = doc
            .get("meta")
            .and_then(|m| m.get("kind"))
            .and_then(Value::as_str);
        if kind != Some("snapshot") {
            return Err(bad("document is not a snapshot (meta.kind)"));
        }
        let Some(block) = doc.get(BLOCK).filter(|b| b.is_object()) else {
            return Err(bad("document has no snapshot block"));
        };
        let doc_serial = block.get("serial").and_then(Value::as_str).unwrap_or("");
        if doc_serial != serial {
            return Err(bad(format!(
                "document is a snapshot of {doc_serial:?}, not of {serial:?}"
            )));
        }
        let id = block.get("id").and_then(Value::as_str).unwrap_or("");
        if !valid_id(id) {
            return Err(bad("document's snapshot.id is not a store id"));
        }
        let id = id.to_string();
        // -- WP P2SS -- review p2a F4: filed pretty-printed, the form the client store's
        // rename and note write, and every cap measured on that form.
        let stored =
            serde_json::to_string_pretty(&doc).map_err(|e| bad(format!("document: {e}")))?;
        let len = u64::try_from(stored.len()).unwrap_or(u64::MAX);
        if len > self.caps.doc_bytes {
            return Err(SnapshotError::Conflict(format!(
                "a snapshot document is at most {} bytes; this one is {len}",
                self.caps.doc_bytes
            )));
        }

        let mut pending = self.lock();
        match self.holder(&id)? {
            Some(held) if held != serial => {
                return Err(SnapshotError::Conflict(format!(
                    "snapshot {id} is held for another device"
                )));
            }
            Some(_) => {
                let path = self.doc_path(serial, &id);
                let same = fs::metadata(&path)
                    .ok()
                    .filter(|m| m.len() <= self.caps.doc_bytes)
                    .and_then(|_| fs::read_to_string(&path).ok())
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .is_some_and(|held| held == doc);
                if !same {
                    return Err(SnapshotError::Conflict(format!(
                        "the hub holds a different snapshot {id}; it is never overwritten"
                    )));
                }
                let entry = self
                    .store
                    .list(serial)
                    .into_iter()
                    .find(|e| e.id == id)
                    .unwrap_or_else(|| snapshot_store::entry_from_document(&id, &doc));
                return Ok(Put {
                    entry,
                    created: false,
                });
            }
            None => {}
        }
        let count = self.store.list(serial).len();
        if count >= self.caps.per_device {
            return Err(SnapshotError::Conflict(format!(
                "the hub keeps at most {} snapshots of a device; delete one first",
                self.caps.per_device
            )));
        }
        let total = self.total_bytes();
        if total.saturating_add(len) > self.caps.total_bytes {
            return Err(SnapshotError::Conflict(format!(
                "the hub's snapshots are at most {} bytes in all",
                self.caps.total_bytes
            )));
        }
        create_dir(&self.dir(serial))?;
        write_atomic(&self.doc_path(serial, &id), &stored)?;
        // The client store's list adds the new document to the index from its text.
        let entry = self
            .store
            .list(serial)
            .into_iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| snapshot_store::entry_from_document(&id, &doc));
        restrict(&self.dir(serial).join(INDEX_FILE));
        push_event(&mut pending, serial, &id, "put");
        Ok(Put {
            entry,
            created: true,
        })
    }

    /// The serial holding `id`, when its document with `change` applied (as the client
    /// store's rename or note applies it, then pretty-printed) stays within the cap.
    fn editable(
        &self,
        id: &str,
        change: impl FnOnce(&mut serde_json::Map<String, Value>),
    ) -> Result<String, SnapshotError> {
        if !valid_id(id) {
            return Err(SnapshotError::NotFound(id.to_string()));
        }
        let serial = self
            .holder(id)?
            .ok_or_else(|| SnapshotError::NotFound(id.to_string()))?;
        let path = self.doc_path(&serial, id);
        let len = fs::metadata(&path).map_err(|e| map_io(id, e))?.len();
        let over = || {
            SnapshotError::Conflict(format!(
                "snapshot {id}'s document would be over {} bytes",
                self.caps.doc_bytes
            ))
        };
        if len > self.caps.doc_bytes {
            return Err(over());
        }
        let mut doc: Value = serde_json::from_str(&fs::read_to_string(&path)?)
            .map_err(|e| SnapshotError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
        let top = doc.as_object_mut().ok_or_else(|| {
            SnapshotError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a snapshot document",
            ))
        })?;
        change(top);
        let after = serde_json::to_string_pretty(&doc).map_err(io::Error::other)?;
        if u64::try_from(after.len()).unwrap_or(u64::MAX) > self.caps.doc_bytes {
            return Err(over());
        }
        Ok(serial)
    }

    /// Rename snapshot `id` (any Unicode, at most 256 characters).
    pub fn rename(&self, id: &str, name: &str) -> Result<SnapshotEntry, SnapshotError> {
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(bad(format!(
                "a name is at most {MAX_NAME_CHARS} characters"
            )));
        }
        let mut pending = self.lock();
        let serial = self.editable(id, |top| {
            let meta = top.entry("meta").or_insert_with(|| json!({}));
            if let Some(m) = meta.as_object_mut() {
                m.insert("name".into(), json!(name));
            }
        })?;
        let entry = self.store.rename(id, name).map_err(|e| map_io(id, e))?;
        self.restrict_all(&serial, id);
        push_event(&mut pending, &serial, id, "rename");
        Ok(entry)
    }

    /// Set snapshot `id`'s note (any Unicode, at most 4096 characters).
    pub fn set_note(&self, id: &str, note: &str) -> Result<SnapshotEntry, SnapshotError> {
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(bad(format!(
                "a note is at most {MAX_NOTE_CHARS} characters"
            )));
        }
        let mut pending = self.lock();
        let serial = self.editable(id, |top| {
            let block = top.entry(BLOCK).or_insert_with(|| json!({}));
            if !block.is_object() {
                *block = json!({});
            }
            if let Some(b) = block.as_object_mut() {
                b.insert("note".into(), json!(note));
            }
        })?;
        let entry = self.store.set_note(id, note).map_err(|e| map_io(id, e))?;
        self.restrict_all(&serial, id);
        push_event(&mut pending, &serial, id, "note");
        Ok(entry)
    }

    /// Delete snapshot `id`. Answers the serial that held it.
    pub fn delete(&self, id: &str) -> Result<String, SnapshotError> {
        if !valid_id(id) {
            return Err(SnapshotError::NotFound(id.to_string()));
        }
        let mut pending = self.lock();
        let serial = self
            .holder(id)?
            .ok_or_else(|| SnapshotError::NotFound(id.to_string()))?;
        self.store.delete(id).map_err(|e| map_io(id, e))?;
        restrict(&self.dir(&serial).join(INDEX_FILE));
        push_event(&mut pending, &serial, id, "delete");
        Ok(serial)
    }

    fn restrict_all(&self, serial: &str, id: &str) {
        restrict(&self.doc_path(serial, id));
        restrict(&self.dir(serial).join(INDEX_FILE));
    }

    /// The `snapshot.changed` events queued since the last call, oldest first (at most 64
    /// are kept), as the text to send to every session: `{t, v, hub, serial, snapshot,
    /// change}`, with `hub` the hub's id the registrar passes (every hub event carries
    /// it, PLAN-P2 section 3; review p2a F7).
    pub fn take_changed(&self, hub: &str) -> Vec<String> {
        std::mem::take(&mut *self.lock())
            .into_iter()
            .map(|mut e| {
                if let Some(m) = e.as_object_mut() {
                    m.insert("hub".into(), json!(hub));
                }
                e.to_string()
            })
            .collect()
    }

    /// Answer a `snapshot.*` message of this family for a session of `role`. `None` for
    /// any other message (`snapshot.recall` included: P2PA's).
    pub fn handle_message(&self, role: Role, msg: &Value) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if !MESSAGES.contains(&t) {
            return None;
        }
        let id = msg.get("id").filter(|id| id.is_i64() || id.is_u64());
        let Some(id) = id else {
            return Some(err(None, "bad_request", "id is required"));
        };
        let need = match t {
            "snapshot.list" | "snapshot.get" => Role::Viewer,
            "snapshot.delete" => Role::Admin,
            _ => Role::Control,
        };
        if role < need {
            return Some(err(Some(id), "denied", "this role may not"));
        }
        let text = |k: &str| msg.get(k).and_then(Value::as_str);
        let snap = text("snapshot");
        let result = match t {
            "snapshot.list" => match text("serial") {
                None => Err(bad("serial is required")),
                Some(serial) => self
                    .list(serial)
                    .map(|list| json!({"serial": serial, "snapshots": list})),
            },
            "snapshot.get" => match snap {
                None => Err(bad("snapshot is required")),
                Some(s) => self
                    .get(s)
                    .map(|(entry, doc)| json!({"snapshot": entry, "document": doc})),
            },
            "snapshot.put" => {
                let document = match msg.get("document") {
                    Some(Value::String(s)) => Ok(s.clone()),
                    Some(v @ Value::Object(_)) => {
                        serde_json::to_string(v).map_err(|e| bad(format!("document: {e}")))
                    }
                    _ => Err(bad("document is required (its text, or the object)")),
                };
                match (text("serial"), document) {
                    (None, _) => Err(bad("serial is required")),
                    (_, Err(e)) => Err(e),
                    (Some(serial), Ok(doc)) => self
                        .put(serial, &doc)
                        .map(|p| json!({"snapshot": p.entry, "created": p.created})),
                }
            }
            "snapshot.rename" => match (snap, text("name")) {
                (Some(s), Some(name)) => self.rename(s, name).map(|e| json!({"snapshot": e})),
                _ => Err(bad("snapshot and name are required")),
            },
            "snapshot.note" => match (snap, text("note")) {
                (Some(s), Some(note)) => self.set_note(s, note).map(|e| json!({"snapshot": e})),
                _ => Err(bad("snapshot and note are required")),
            },
            // "snapshot.delete", the last of MESSAGES.
            _ => match snap {
                None => Err(bad("snapshot is required")),
                Some(s) => self
                    .delete(s)
                    .map(|serial| json!({"serial": serial, "snapshot": s})),
            },
        };
        Some(match result {
            Ok(body) => ok(id, body),
            Err(e) => {
                if let SnapshotError::Io(io) = &e {
                    log::warn!("hub snapshots: {t}: {io}");
                }
                err(Some(id), e.code(), &e.to_string())
            }
        })
    }
}

fn push_event(pending: &mut Vec<Value>, serial: &str, id: &str, change: &str) {
    if pending.len() >= MAX_PENDING_EVENTS {
        pending.remove(0);
    }
    pending.push(
        json!({"t": CHANGED_EVENT, "v": VERSION, "serial": serial, "snapshot": id, "change": change}),
    );
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

fn ok(id: &Value, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2ss;
