//! Snapshots: whole-device states kept host-side (WP P1SN, PRO1 W3).
//!
//! A snapshot is a `.dspipreset` document (P1DF's family, `meta.kind =
//! "snapshot"`) with a `snapshot` block for what a preset document does not
//! hold:
//!
//! - `image`: the bulk image itself, hex, when the device's wire format is
//!   this build's (a recall sends exactly it, one `0xA1`); `wireFormat` its
//!   format;
//! - the device-global settings outside the image: `deviceName`,
//!   `sourceNames` (the input sources' names), `outputLatency` (the requested
//!   option) and `controlSurface` (bindings and their names, IR commands,
//!   groups, macros, the display's configuration and pages), each only when
//!   the firmware has it;
//! - `id`, `serial`, `note`, `savedMs`: the store's (`snapshot_store.rs`).
//!
//! Channel names and the crossfeed pair map are in this build's image, so
//! they travel with it. The control interfaces (UART, I2C) and the preset
//! slots are not part of a snapshot: one is how the device is reached, the
//! other is the device's own storage.
//!
//! **Capture** reads the settings outside the image, then takes the mirror
//! (no transfer for the image). **Recall** checks first (platform, channel
//! count, wire format, a writable connection), then, under the device lock
//! (CG24), writes the settings outside the image that differ, then the image
//! as one bulk write, last, so it ends the undo history (`docs/integrator/undo.md`),
//! and records a recall with its changes in the event log. A connection error
//! stops the recall where it is; nothing is queued or replayed. A snapshot
//! from another platform or channel count is refused with the reason; an image
//! of another wire format is read through `wire_compat` and refused unless a
//! forward map exists (FW9: none does yet). A snapshot with no image (taken
//! from firmware whose image this build reads only through `wire_compat`) is
//! recalled through the import's own path, every block.
//!
//! Only snapshots are recalled or filed: a document whose `meta.kind` is not
//! `"snapshot"`, or that has no `snapshot` block (a preset, a template), is
//! refused with the reason, by the recall, the document recall and the store's
//! import alike. Such a document is imported with `config_import`, which asks
//! for its confirmation token (P1TK); a recall asks for none (WP P1BFIX, F1).
//!
//! The store is files (`dspi_session::snapshot_store`), in the folder the
//! client names with [`DspiCore::snapshot_set_dir`]; the web, which has no
//! files, uses the document calls ([`DspiCore::snapshot_capture_document`],
//! [`DspiCore::snapshot_recall_document`]) with downloads and uploads.

use std::path::PathBuf;
use std::sync::Mutex;

use dspi_proto::Platform;
use dspi_proto::generated::BULK_SIZE;
use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
use dspi_proto::wire_compat::{self, WireRead};
use dspi_session::events::EventKind;
use dspi_session::preset_file::MetaKind;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use dspi_session::snapshot_store::{SnapshotEntry, SnapshotStore};
use serde_json::{Map, Value, json};

use crate::presets::Holder;
use crate::records::DspState;
use crate::{DspiCore, DspiError, Result, state};

/// What other sessions are told while a recall holds the device lock.
const RECALL_REASON: &str = "Recalling a snapshot";

/// How long a recall may hold the device lock (the longest a caller may).
const RECALL_LEASE_MS: u32 = 60_000;

/// The longest name and note kept, in characters.
const MAX_NAME_CHARS: usize = 200;
const MAX_NOTE_CHARS: usize = 4000;

/// The snapshot block's key (`dspi_session::snapshot_store::BLOCK`).
const BLOCK: &str = "snapshot";

/// The folder the client named for snapshots.
// The web has no files: the slot is never read there.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
#[derive(Debug, Default)]
pub(crate) struct StoreSlot(Mutex<Option<PathBuf>>);

/// One snapshot in a list ([`DspiCore::snapshot_list`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    /// The store's id, for the other calls.
    pub id: String,
    /// The user's name; any Unicode.
    pub name: String,
    /// The user's note; any Unicode.
    pub note: String,
    /// The serial of the device it was taken from.
    pub serial: String,
    /// When it was taken, ms since the Unix epoch.
    pub saved_ms: i64,
    /// The platform it was taken on (`RP2040`, `RP2350`), "" when unknown.
    pub platform: String,
    /// The device's input channels then, 0 when unknown.
    pub input_channels: u8,
    /// The device's output channels then, 0 when unknown.
    pub output_channels: u8,
    /// The wire format of its image, 0 when unknown.
    pub wire_format: u8,
}

/// What a recall did ([`DspiCore::snapshot_recall`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRecall {
    /// The snapshot's name.
    pub name: String,
    /// Whether the snapshot's own image was written (false: a document
    /// without one, written through the import's path).
    pub image: bool,
    /// What changed, in the unsaved-changes list's words, then the settings
    /// outside the image that were rewritten.
    pub changes: Vec<String>,
    /// What was not written, and why (a feature this firmware lacks, a part
    /// the device refused, a value held to the device's range).
    pub notes: Vec<String>,
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
impl From<SnapshotEntry> for SnapshotInfo {
    fn from(e: SnapshotEntry) -> Self {
        Self {
            id: e.id,
            name: e.name,
            note: e.note,
            serial: e.serial,
            saved_ms: e.saved_ms,
            platform: e.platform,
            input_channels: e.input_channels,
            output_channels: e.output_channels,
            wire_format: e.wire_format,
        }
    }
}

// ─── Small helpers ──────────────────────────────────────────────────────

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let t = text.as_bytes();
    if !t.len().is_multiple_of(2) {
        return None;
    }
    t.chunks(2)
        .map(|p| {
            std::str::from_utf8(p)
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect()
}

/// `v`'s value at `path`, or null.
fn at<'v>(v: &'v Value, path: &[&str]) -> &'v Value {
    path.iter()
        .try_fold(v, |v, k| v.get(k))
        .unwrap_or(&Value::Null)
}

/// Whether a JSON reply or snapshot says `key: true`.
fn is_true(v: &Value, key: &str) -> bool {
    v.get(key) == Some(&Value::Bool(true))
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

// -- WP P1BFIX -- F1
/// The refusal of a document that is not a snapshot (see the module
/// documentation): recalling it would be an import without the import's
/// confirmation token.
pub(crate) const NOT_A_SNAPSHOT: &str = "not a snapshot — import it instead";

/// Refuse a document that is not a snapshot: `meta.kind` other than
/// `"snapshot"`, or no `snapshot` block.
fn check_snapshot(doc: &Value) -> Result<()> {
    let kind = at(doc, &["meta", "kind"]).as_str().unwrap_or_default();
    let block = doc.get(BLOCK).is_some_and(Value::is_object);
    if kind == MetaKind::Snapshot.word() && block {
        return Ok(());
    }
    let what = match kind {
        "" => "a document without a kind".to_owned(),
        k if k == MetaKind::Snapshot.word() => "a snapshot without its snapshot block".to_owned(),
        k => format!("a {k} document"),
    };
    Err(invalid(format!("{NOT_A_SNAPSHOT} ({what})")))
}

/// A store error as the API's.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn io_error(e: std::io::Error) -> DspiError {
    if e.kind() == std::io::ErrorKind::NotFound {
        invalid(format!("snapshot not found ({e})"))
    } else if e.kind() == std::io::ErrorKind::InvalidInput {
        // -- WP P1BFIX -- F3: an id two devices' folders hold.
        invalid(e.to_string())
    } else {
        DspiError::Io {
            msg: format!("the snapshot store: {e}"),
        }
    }
}

/// Errors that end a recall where it is: the device is gone or not ours to
/// write. Anything else is noted and the recall goes on.
fn stops(e: &DspiError) -> bool {
    matches!(
        e,
        DspiError::Disconnected
            | DspiError::ReadOnly
            | DspiError::Locked
            | DspiError::Denied
            | DspiError::Link { .. }
            // -- WP P1DKFIX -- a hub's lock or its refusal of an image ends it too
            | DspiError::InstallerLock { .. }
            | DspiError::ImageRefused { .. }
    )
}

/// Only an identical format may be written: the forward map from an image
/// of wire format `from` to one of `to`, which exists only for `from == to`
/// until the firmware moves past V32 (FW9).
fn forward_map(from: u8, to: u8) -> Option<fn(Vec<u8>) -> Vec<u8>> {
    (from == to).then_some(|image| image)
}

/// The settings a snapshot document carries outside its preset blocks.
#[derive(Debug, Default)]
struct Plan {
    name: String,
    image: Option<Vec<u8>>,
    device_name: Option<String>,
    source_names: Option<Vec<String>>,
    output_latency: Option<u8>,
    control_surface: Option<Map<String, Value>>,
}

impl Plan {
    fn of(doc: &Value) -> Result<Self> {
        let block = doc.get(BLOCK);
        let get = |k: &str| block.and_then(|b| b.get(k)).filter(|v| !v.is_null());
        let image = match get("image") {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(unhex)
                    .filter(|i| !i.is_empty())
                    .ok_or_else(|| invalid("this snapshot's image is damaged"))?,
            ),
        };
        Ok(Self {
            name: doc
                .get("meta")
                .and_then(|m| m.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            image,
            device_name: get("deviceName").and_then(Value::as_str).map(str::to_owned),
            source_names: get("sourceNames").and_then(Value::as_array).map(|a| {
                a.iter()
                    .map(|n| n.as_str().unwrap_or_default().to_owned())
                    .collect()
            }),
            output_latency: get("outputLatency")
                .and_then(Value::as_u64)
                .and_then(|n| u8::try_from(n).ok()),
            control_surface: get("controlSurface").and_then(Value::as_object).cloned(),
        })
    }
}

/// How the event log names a recall of `plan`.
fn recall_name(plan: &Plan) -> String {
    if plan.name.is_empty() {
        "a snapshot".to_string()
    } else {
        format!("snapshot \"{}\"", plan.name)
    }
}

/// Refuse a snapshot of another platform or channel count than `s`, when its
/// `meta` says.
fn check_shape(doc: &Value, s: &DspState) -> Result<()> {
    let meta = doc.get("meta");
    let platform = Platform::from_id(s.platform_id).name();
    if let Some(p) = meta.and_then(|m| m.get("platform")).and_then(Value::as_str)
        && !p.is_empty()
        && !p.eq_ignore_ascii_case(&platform)
    {
        return Err(invalid(format!(
            "this snapshot was taken on an {p} device; this device is an {platform}, so nothing was sent"
        )));
    }
    let count = |k: &str| {
        meta.and_then(|m| m.get(k))
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
    };
    let (ins, outs) = (count("inputChannelCount"), count("outputChannelCount"));
    if ins.is_some_and(|n| n != s.num_input_channels)
        || outs.is_some_and(|n| n != s.num_output_channels)
    {
        return Err(invalid(format!(
            "this snapshot has {} inputs and {} outputs; this device has {} and {}, so nothing was sent",
            ins.unwrap_or(s.num_input_channels),
            outs.unwrap_or(s.num_output_channels),
            s.num_input_channels,
            s.num_output_channels
        )));
    }
    Ok(())
}

/// The image to write for `image`, or why there is none: another wire format
/// with no forward map, or bytes that are not an image.
fn writable_image(image: &[u8]) -> Result<Vec<u8>> {
    let to = WIRE_FORMAT_VERSION as u8;
    let from = image.first().copied().unwrap_or(0);
    if from != to {
        // Read through wire_compat, so the reason names what it is.
        let read = wire_compat::normalize(image)
            .map_err(|_| invalid("this snapshot's image is damaged"))?
            .read;
        let Some(map) = forward_map(from, to) else {
            let when = if read == WireRead::Older {
                "an older"
            } else {
                "a newer"
            };
            return Err(invalid(format!(
                "this snapshot was taken on {when} firmware (wire format {from}); this build \
                 cannot convert it to wire format {to}, so nothing was sent"
            )));
        };
        return Ok(map(image.to_vec()));
    }
    if image.len() != BULK_SIZE {
        return Err(invalid("this snapshot's image is damaged"));
    }
    Ok(image.to_vec())
}

/// The parts of the control-surface configuration a snapshot keeps, from
/// `cs_snapshot`'s JSON.
fn cs_part(s: &Value) -> Map<String, Value> {
    let max_pages = at(s, &["display", "maxPages"])
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    let pages: Vec<Value> = at(s, &["display", "pages"])
        .as_array()
        .map(|p| p.iter().take(max_pages).cloned().collect())
        .unwrap_or_default();
    let mut m = Map::new();
    for k in ["bindings", "names", "ir", "groups", "macros"] {
        m.insert(k.into(), at(s, &[k]).clone());
    }
    m.insert(
        "displayConfig".into(),
        at(s, &["display", "config"]).clone(),
    );
    m.insert("displayPages".into(), Value::Array(pages));
    m
}

/// The operations that turn control-surface configuration `now` into `want`
/// for one kind (`key` in [`cs_part`], `op` and its payload key).
fn cs_ops(
    want: &Map<String, Value>,
    now: &Map<String, Value>,
    key: &str,
    op: &str,
    field: &str,
) -> Vec<Value> {
    let empty = Vec::new();
    let w = want.get(key).and_then(Value::as_array).unwrap_or(&empty);
    let n = now.get(key).and_then(Value::as_array).unwrap_or(&empty);
    let limit = if key == "names" {
        now.get("bindings")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    } else {
        n.len()
    };
    w.iter()
        .zip(n)
        .take(limit)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, _))| {
            let mut o = Map::new();
            o.insert("op".into(), json!(op));
            o.insert("index".into(), json!(i));
            o.insert(field.into(), a.clone());
            Value::Object(o)
        })
        .collect()
}

// ─── The API ────────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The open device's state as a snapshot document (`.dspipreset` text,
    /// `meta.kind = "snapshot"`), not filed: for the web, which keeps
    /// snapshots as downloads, and for a client's own store. `name` and
    /// `note` (any Unicode; cut to 200 and 4000 characters) go in with it.
    /// Reads the settings outside the image (the control surfaces' whole
    /// configuration among them); the image is the mirror.
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::Transfer`] before
    /// the device's settings were read, or for a failed read;
    /// [`DspiError::Unsupported`] for firmware whose image this build cannot
    /// map.
    pub fn snapshot_capture_document(&self, name: String, note: String) -> Result<String> {
        let (doc, _) = self.capture_value(&name, &note)?;
        serde_json::to_string_pretty(&doc).map_err(|e| DspiError::Io {
            msg: format!("the snapshot could not be written: {e}"),
        })
    }

    /// Recall a snapshot document's text (see
    /// [`DspiCore::snapshot_recall`]): for the web and for a document the
    /// client keeps itself. Only a snapshot (`meta.kind = "snapshot"` with a
    /// `snapshot` block): any other document is refused with
    /// [`DspiError::InvalidArgument`] ("not a snapshot — import it instead"),
    /// nothing sent; it goes through `config_import` and its token.
    pub fn snapshot_recall_document(&self, text: String) -> Result<SnapshotRecall> {
        let doc = crate::files::parse_config(&text).map_err(invalid)?;
        check_snapshot(&doc)?; // -- WP P1BFIX -- F1
        self.recall_value(&doc)
    }

    /// Keep snapshots in files under `dir`, the client's own data folder
    /// (created if missing): one folder per device serial, one document per
    /// snapshot and an index. "" forgets the folder.
    ///
    /// Errors: [`DspiError::Io`] when the folder cannot be created;
    /// [`DspiError::Unsupported`] on the web (no files there).
    pub fn snapshot_set_dir(&self, dir: String) -> Result<()> {
        self.set_store_dir(dir)
    }

    /// Capture the open device as a new snapshot in the store (see
    /// [`DspiCore::snapshot_capture_document`]). Errors as there, and
    /// [`DspiError::Io`] with no folder named or a failed write.
    pub fn snapshot_capture(&self, name: String, note: String) -> Result<SnapshotInfo> {
        self.capture_into_store(&name, &note)
    }

    /// The snapshots of device `serial` ("" the open device), oldest first.
    /// Never fails: no folder, no device or no snapshots is an empty list, and
    /// a damaged index is rebuilt from the documents.
    pub fn snapshot_list(&self, serial: String) -> Vec<SnapshotInfo> {
        self.list_store(serial)
    }

    /// Recall snapshot `id` on the open device: the settings outside the
    /// image that differ (device and source names, output latency, control
    /// surfaces), then the image as one bulk write, under the device lock.
    /// The undo history before it ends; the event log records a recall with
    /// its changes. Live, not stored in a preset slot. Never guarded and
    /// never queued: on a lost connection it stops where it is.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown id, a document
    /// that is not a snapshot, a snapshot of another platform or channel
    /// count, or an image of another wire format this build cannot convert
    /// (nothing is sent);
    /// [`DspiError::Disconnected`], [`DspiError::ReadOnly`],
    /// [`DspiError::Locked`] and the transfer errors of a bulk write;
    /// [`DspiError::Io`] for an unreadable store. When the image fails after
    /// settings outside it were written, the error's words (and a Recall
    /// event) say what was written and what was not; a hub's refusal of the
    /// image then comes as [`DspiError::Link`] with code `locked` or
    /// `denied`.
    pub fn snapshot_recall(&self, id: String) -> Result<SnapshotRecall> {
        let text = self.read_store(&id)?;
        self.snapshot_recall_document(text)
    }

    /// Rename snapshot `id`; any Unicode, cut to 200 characters.
    /// Errors: [`DspiError::InvalidArgument`] for an unknown id;
    /// [`DspiError::Io`].
    pub fn snapshot_rename(&self, id: String, name: String) -> Result<SnapshotInfo> {
        self.update_store(&id, Some(cut(&name, MAX_NAME_CHARS)), None)
    }

    /// Set snapshot `id`'s note; any Unicode, cut to 4000 characters.
    /// Errors as [`DspiCore::snapshot_rename`].
    pub fn snapshot_set_note(&self, id: String, note: String) -> Result<SnapshotInfo> {
        self.update_store(&id, None, Some(cut(&note, MAX_NOTE_CHARS)))
    }

    /// Delete snapshot `id`. Errors as [`DspiCore::snapshot_rename`].
    pub fn snapshot_delete(&self, id: String) -> Result<()> {
        self.delete_from_store(&id)
    }

    /// Snapshot `id`'s document, to save as a file. Errors as
    /// [`DspiCore::snapshot_rename`].
    pub fn snapshot_export(&self, id: String) -> Result<String> {
        self.read_store(&id)
    }

    /// File a snapshot document (`meta.kind = "snapshot"` with a `snapshot`
    /// block, e.g. one exported from another console) in the store, under the
    /// open device's serial (else the document's), named `name` ("" keeps the
    /// document's) with `note`. A snapshot without an image is recalled
    /// through the import's path.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for text that is not a
    /// document this build reads, or a document that is not a snapshot
    /// ("not a snapshot — import it instead": a preset or template goes
    /// through `config_import`); [`DspiError::Disconnected`] with no device
    /// and no serial in the document; [`DspiError::Io`].
    pub fn snapshot_import(
        &self,
        text: String,
        name: String,
        note: String,
    ) -> Result<SnapshotInfo> {
        self.import_into_store(&text, &name, &note)
    }
}

// -- WP P1CFIX -- F3
/// The settings a snapshot document `doc` would recall, built onto `base`
/// (the mirror): its image decoded, or, without one, its blocks as the
/// import builds them. `None` for a damaged image.
pub(crate) fn state_of_document(base: &DspState, doc: &Value) -> Option<DspState> {
    let plan = Plan::of(doc).ok()?;
    match plan.image {
        Some(image) => {
            let mut s = base.clone();
            state::decode_bulk(&image, &mut s).ok()?;
            Some(s)
        }
        None => Some(crate::files::built_whole(base, doc)),
    }
}

impl DspiCore {
    // -- WP P1CFIX -- F3
    /// [`DspiCore::snapshot_capture_document`] with `adjust` applied to the
    /// mirror's copy before it is written into the document (the image and
    /// the blocks alike); the device and the mirror are untouched.
    pub(crate) fn snapshot_capture_document_adjusted(
        &self,
        name: String,
        note: String,
        adjust: &dyn Fn(&mut DspState),
    ) -> Result<String> {
        let (doc, _) = self.capture_value_adjusted(&name, &note, adjust)?;
        serde_json::to_string_pretty(&doc).map_err(|e| DspiError::Io {
            msg: format!("the snapshot could not be written: {e}"),
        })
    }

    /// The snapshot document of the open device, and its serial.
    fn capture_value(&self, name: &str, note: &str) -> Result<(Value, String)> {
        self.capture_value_adjusted(name, note, &|_| {})
    }

    /// [`DspiCore::capture_value`], the exported settings passed through
    /// `adjust` first (WP P1CFIX, F3).
    fn capture_value_adjusted(
        &self,
        name: &str,
        note: &str,
        adjust: &dyn Fn(&mut DspState),
    ) -> Result<(Value, String)> {
        self.ensure_probed();
        // The settings outside the image first: each read takes the lock.
        let latency = match self.fetch_output_latency() {
            Ok(l) => l
                .fields
                .iter()
                .find(|f| f.name == "requested_id")
                .and_then(|f| f.value.parse::<u8>().ok()),
            Err(DspiError::Unsupported) => None,
            Err(e) => return Err(e),
        };
        let device_name = match self.cached_device_name() {
            Ok(n) => Some(n),
            Err(DspiError::Unsupported) => None,
            Err(e) => return Err(e),
        };
        let mut sources = Vec::new();
        for i in 0..=u8::MAX {
            match self.cached_input_source_name(i) {
                Ok(n) => sources.push(n),
                Err(DspiError::Unsupported | DspiError::InvalidArgument { .. }) => break,
                Err(e) => return Err(e),
            }
        }
        let cs = match self.cs_fetch() {
            Ok(()) => {
                let s: Value = serde_json::from_str(&self.cs_snapshot()).unwrap_or_default();
                is_true(&s, "supported").then(|| cs_part(&s))
            }
            Err(DspiError::Unsupported) => None,
            Err(e) => return Err(e),
        };

        let saved_ms = crate::events::now_ms();
        let mut inner = self.lock();
        let conn = inner.conn()?;
        let serial = conn.serial.clone();
        let exact = conn.wire == Some(WireRead::Exact);
        let (mut s, map) = crate::files::exportable(&mut inner)?;
        drop(inner);
        adjust(&mut s); // -- WP P1CFIX -- F3
        let name = cut(name, MAX_NAME_CHARS);
        let text = crate::files::export_document(
            &s,
            &name,
            concat!("dspi-core ", env!("CARGO_PKG_VERSION")),
            0,
            map,
            MetaKind::Snapshot,
        );
        let mut doc: Value = serde_json::from_str(&text).map_err(|e| DspiError::Io {
            msg: format!("the snapshot could not be written: {e}"),
        })?;
        let mut block = Map::new();
        block.insert("serial".into(), json!(serial));
        block.insert("note".into(), json!(cut(note, MAX_NOTE_CHARS)));
        block.insert("savedMs".into(), json!(saved_ms));
        block.insert("wireFormat".into(), json!(s.format_version));
        if exact {
            block.insert("image".into(), json!(hex(&state::encode_bulk(&s))));
        }
        if let Some(n) = device_name {
            block.insert("deviceName".into(), json!(n));
        }
        if !sources.is_empty() {
            block.insert("sourceNames".into(), json!(sources));
        }
        if let Some(l) = latency {
            block.insert("outputLatency".into(), json!(l));
        }
        if let Some(cs) = cs {
            block.insert("controlSurface".into(), Value::Object(cs));
        }
        if let Some(top) = doc.as_object_mut() {
            top.insert(BLOCK.into(), Value::Object(block));
        }
        Ok((doc, serial))
    }

    /// [`DspiCore::snapshot_recall_document`] for a parsed document.
    fn recall_value(&self, doc: &Value) -> Result<SnapshotRecall> {
        let began = dspi_transport::clock::Instant::now(); // -- WP ECHO1 --
        self.ensure_probed();
        // -- WP P1EFIX -- no echo queued before the image lands after it
        self.settle_notifications();
        let plan = Plan::of(doc)?;
        // Every check before anything is sent.
        let (generation, image, had_caller) = {
            let mut inner = self.lock();
            inner.writable_conn()?;
            check_shape(doc, &inner.state)?;
            let image = plan.image.as_deref().map(writable_image).transpose()?;
            let conn = inner.conn()?;
            let had = conn.held_lock.as_ref().map(|h| h.holder) == Some(Holder::Caller);
            (conn.generation, image, had)
        };
        // Held, so flows inside (a control-surface macro) do not ask again.
        self.device_lock_acquire(RECALL_REASON.into(), RECALL_LEASE_MS)?;
        let result = self.recall_locked(doc, &plan, image, generation);
        self.whole_image_ended(began); // -- WP ECHO1 --
        if !had_caller {
            self.device_lock_release();
        }
        result
    }

    /// Whether the connection a recall began on is still the open one.
    fn same_connection(&self, generation: u64) -> Result<()> {
        match self.lock().conn.as_ref() {
            Some(c) if c.generation == generation => Ok(()),
            _ => Err(DspiError::Disconnected),
        }
    }

    fn recall_locked(
        &self,
        doc: &Value,
        plan: &Plan,
        image: Option<Vec<u8>>,
        generation: u64,
    ) -> Result<SnapshotRecall> {
        let mut extra = Vec::new();
        let mut notes = Vec::new();
        // The settings outside the image first: the image, written last,
        // then ends the undo history over all of it.
        self.same_connection(generation)?;
        self.recall_names(plan, &mut extra, &mut notes)?;
        self.same_connection(generation)?;
        self.recall_latency(plan, &mut extra, &mut notes)?;
        self.same_connection(generation)?;
        self.recall_surfaces(plan, &mut extra, &mut notes)?;

        let with_image = image.is_some();
        // -- WP P1BFIX -- F4: a failed last write leaves the parts before it
        // written; say which, in the event log and the error.
        let (before, after, separate) = match self.recall_whole(doc, image, generation, &mut notes)
        {
            Ok(w) => w,
            Err(e) => return Err(self.partial_recall(plan, with_image, &extra, e)),
        };
        // The output configuration kept apart from presets goes one by one.
        let after = if separate && crate::files::io_changed(&before, &after) {
            notes.extend(self.push_output_config_lines(&before, &after));
            self.lock().state.clone()
        } else {
            after
        };
        let sources = self
            .lock()
            .conn
            .as_ref()
            .map(|c| c.source_names.to_vec())
            .unwrap_or_default();
        let mut changes = crate::presets::changes(&before, &after, &sources);
        changes.extend(extra);
        let name = recall_name(plan);
        self.events.applied(
            EventKind::Recall,
            None,
            &format!("Recalled {name}"),
            changes.clone(),
        );
        Ok(SnapshotRecall {
            name: plan.name.clone(),
            image: with_image,
            changes,
            notes,
        })
    }

    /// The recall's last write: the image as one bulk write, or, for a
    /// snapshot without one, every block through the import's path. The
    /// state before and after, and whether the output configuration is kept
    /// apart from presets.
    fn recall_whole(
        &self,
        doc: &Value,
        image: Option<Vec<u8>>,
        generation: u64,
        notes: &mut Vec<String>,
    ) -> Result<(DspState, DspState, bool)> {
        Ok({
            let mut guard = self.lock();
            let inner = &mut *guard;
            if inner.conn.as_ref().map(|c| c.generation) != Some(generation) {
                return Err(DspiError::Disconnected);
            }
            match image {
                Some(image) => {
                    let before = inner.state.clone();
                    let mut s = before.clone();
                    state::decode_bulk(&image, &mut s)
                        .map_err(|_| invalid("this snapshot's image is damaged"))?;
                    // -- WP P1LM -- held to the device's limits profile, and said so
                    let held = crate::limits::hold(inner, &mut s);
                    notes.extend(crate::limits::held_line(&held));
                    inner.state = s;
                    if let Err(e) = crate::connect::apply_all_params_locked(inner) {
                        // -- WP BULKFIX -- as the import does
                        if crate::connect::bulk_left_muted(inner) {
                            drop(guard);
                            self.log_left_muted("Recalling a snapshot", &e);
                            return Err(crate::connect::with_why(e));
                        }
                        inner.state = before;
                        return Err(e);
                    }
                    let separate = inner.state.output_config_mode == 0;
                    (before, inner.state.clone(), separate)
                }
                None => match self.write_whole_document(inner, doc) {
                    Ok(w) => {
                        notes.extend(w.notes);
                        (w.before, w.after, w.separate)
                    }
                    Err(e) => {
                        let muted = crate::connect::bulk_left_muted(inner);
                        drop(guard);
                        if muted {
                            self.log_left_muted("Recalling a snapshot", &e);
                        }
                        return Err(e);
                    }
                },
            }
        })
    }

    // -- WP P1BFIX -- F4
    /// A recall whose last write (the image, or the blocks of a snapshot
    /// without one) failed after the settings outside it were written: one
    /// Recall event naming what was written (`written`, in the
    /// unsaved-changes list's words) and what was not, and `e` carrying the
    /// same words. The variant stays where it carries words (`Transfer`,
    /// `Link`, `Io`, `InvalidArgument`); a hub's `Locked` or `Denied` becomes
    /// `Link` with that code, a device's `Refused` a `Transfer`, so the words
    /// reach the caller; `Disconnected` stays as it is (the event has them).
    fn partial_recall(
        &self,
        plan: &Plan,
        with_image: bool,
        written: &[String],
        e: DspiError,
    ) -> DspiError {
        let name = recall_name(plan);
        let not = if with_image {
            "its image (every setting in it) was not written"
        } else {
            "its settings were not written"
        };
        let did = if written.is_empty() {
            "nothing else was changed".to_owned()
        } else {
            format!("written before it: {}", written.join("; "))
        };
        let tail = format!("{name}: {not} ({e}); {did}");
        let what = format!("partly recalled {tail}");
        let mut r = dspi_session::events::EventRecord::new(
            EventKind::Recall,
            crate::events::now_ms(),
            format!("Partly recalled {tail}"),
        );
        r.lines = written.to_vec();
        r.count = u32::try_from(written.len()).unwrap_or(u32::MAX).max(1);
        self.events.record_all(vec![r]);
        match e {
            DspiError::Transfer { msg } => DspiError::Transfer {
                msg: format!("{msg}; {what}"),
            },
            DspiError::Io { msg } => DspiError::Io {
                msg: format!("{msg}; {what}"),
            },
            DspiError::InvalidArgument { msg } => DspiError::InvalidArgument {
                msg: format!("{msg}; {what}"),
            },
            DspiError::Link { code, msg } => DspiError::Link {
                code,
                msg: format!("{msg}; {what}"),
            },
            DspiError::Locked => DspiError::Link {
                code: "locked".into(),
                msg: what,
            },
            DspiError::Denied => DspiError::Link {
                code: "denied".into(),
                msg: what,
            },
            DspiError::Refused { .. } => DspiError::Transfer { msg: what },
            other => other,
        }
    }

    /// Note `e` for `what`, or stop the recall on a connection error.
    fn noted(e: DspiError, what: &str, notes: &mut Vec<String>) -> Result<()> {
        if stops(&e) {
            return Err(e);
        }
        notes.push(match e {
            DspiError::Unsupported => format!("{what}: not supported by this firmware"),
            e => format!("{what}: not written ({e})"),
        });
        Ok(())
    }

    fn recall_names(
        &self,
        plan: &Plan,
        extra: &mut Vec<String>,
        notes: &mut Vec<String>,
    ) -> Result<()> {
        if let Some(want) = &plan.device_name {
            match self.cached_device_name() {
                Ok(now) if &now == want => {}
                Ok(now) => match self.set_device_name(want.clone()) {
                    Ok(()) => extra.push(format!("Device name: \"{now}\" to \"{want}\"")),
                    Err(e) => Self::noted(e, "Device name", notes)?,
                },
                Err(e) => Self::noted(e, "Device name", notes)?,
            }
        }
        for (i, want) in plan.source_names.iter().flatten().enumerate() {
            let Ok(source) = u8::try_from(i) else { break };
            let what = format!("Input source {} name", i.saturating_add(1));
            match self.cached_input_source_name(source) {
                Ok(now) if &now == want => {}
                Ok(now) => match self.set_input_source_name(source, want.clone()) {
                    Ok(()) => extra.push(format!("{what}: \"{now}\" to \"{want}\"")),
                    Err(e) => Self::noted(e, &what, notes)?,
                },
                // A source this device lacks: the shape check passed, so a
                // newer firmware's longer list.
                Err(DspiError::InvalidArgument { .. }) => break,
                Err(e) => {
                    Self::noted(e, "Input source names", notes)?;
                    break;
                }
            }
        }
        Ok(())
    }

    fn recall_latency(
        &self,
        plan: &Plan,
        extra: &mut Vec<String>,
        notes: &mut Vec<String>,
    ) -> Result<()> {
        let Some(want) = plan.output_latency else {
            return Ok(());
        };
        let now = self.fetch_output_latency().map(|l| {
            l.fields
                .iter()
                .find(|f| f.name == "requested_id")
                .and_then(|f| f.value.parse::<u8>().ok())
        });
        match now {
            Ok(Some(now)) if now == want => Ok(()),
            Ok(now) => match self.set_output_latency(want) {
                Ok(()) => {
                    let was = now.map_or_else(|| "?".to_string(), |n| n.to_string());
                    extra.push(format!("Output latency: option {was} to {want}"));
                    Ok(())
                }
                Err(e) => Self::noted(e, "Output latency", notes),
            },
            Err(e) => Self::noted(e, "Output latency", notes),
        }
    }

    /// The control-surface configuration as it stands, fetched if it was not.
    fn surfaces_now(&self) -> Result<Option<Map<String, Value>>> {
        let read = |c: &Self| serde_json::from_str::<Value>(&c.cs_snapshot()).unwrap_or_default();
        let mut s = read(self);
        if !is_true(&s, "supported") {
            self.cs_fetch()?;
            s = read(self);
        }
        Ok(is_true(&s, "supported").then(|| cs_part(&s)))
    }

    fn recall_surfaces(
        &self,
        plan: &Plan,
        extra: &mut Vec<String>,
        notes: &mut Vec<String>,
    ) -> Result<()> {
        const WHAT: &str = "Control surfaces";
        let Some(want) = &plan.control_surface else {
            return Ok(());
        };
        let mut now = match self.surfaces_now() {
            Ok(Some(n)) => n,
            Ok(None) => return Self::noted(DspiError::Unsupported, WHAT, notes),
            Err(e) => return Self::noted(e, WHAT, notes),
        };
        // Groups and macros before the bindings that may name them; the
        // bindings twice, so one that frees a GPIO another wants goes first;
        // the display last.
        let kinds: [(&str, &str, &str); 8] = [
            ("groups", "group", "group"),
            ("macros", "macro", "macro"),
            ("ir", "ir", "command"),
            ("bindings", "binding", "binding"),
            ("bindings", "binding", "binding"),
            ("names", "name", "name"),
            ("displayConfig", "displayConfig", "config"),
            ("displayPages", "displayPage", "page"),
        ];
        let mut written = 0usize;
        for (key, op, field) in kinds {
            let ops = if key == "displayConfig" {
                let w = want.get(key).filter(|c| c.is_object());
                w.filter(|&c| Some(c) != now.get(key))
                    .map(|c| {
                        let mut o = Map::new();
                        o.insert("op".into(), json!(op));
                        o.insert(field.into(), c.clone());
                        vec![Value::Object(o)]
                    })
                    .unwrap_or_default()
            } else {
                cs_ops(want, &now, key, op, field)
            };
            for o in ops {
                if self.apply_cs(&o)? {
                    written = written.saturating_add(1);
                }
            }
            // The editor's copy is re-read by every write: no traffic here.
            now = self.surfaces_now()?.unwrap_or_default();
        }
        if written > 0 {
            extra.push(format!(
                "Control surfaces: {written} item{} rewritten",
                if written == 1 { "" } else { "s" }
            ));
        }
        // What still differs was refused by the device.
        for (key, what) in [
            ("groups", "group"),
            ("macros", "macro"),
            ("ir", "IR command"),
            ("bindings", "binding"),
            ("names", "binding name"),
            ("displayPages", "display page"),
        ] {
            let left = cs_ops(want, &now, key, "", "").len();
            if left > 0 {
                notes.push(format!(
                    "{WHAT}: {left} {what}{} not written (refused by the device)",
                    if left == 1 { "" } else { "s" }
                ));
            }
        }
        if want.get("displayConfig").is_some_and(Value::is_object)
            && want.get("displayConfig") != now.get("displayConfig")
        {
            notes.push(format!(
                "{WHAT}: the display settings were not written (refused by the device)"
            ));
        }
        Ok(())
    }

    /// Send one control-surface operation; true when it was applied. A lost
    /// device stops the recall; a refusal shows in what still differs.
    fn apply_cs(&self, op: &Value) -> Result<bool> {
        let reply: Value = serde_json::from_str(&self.cs_apply(op.to_string())).unwrap_or_default();
        if is_true(&reply, "ok") {
            return Ok(true);
        }
        if !self.is_connected() {
            return Err(DspiError::Disconnected);
        }
        log::debug!("snapshot recall: {op} not applied: {reply}");
        Ok(false)
    }
}

// ─── The store ──────────────────────────────────────────────────────────

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
impl DspiCore {
    fn store(&self) -> Result<SnapshotStore> {
        let dir = self
            .snapshots
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        dir.map(SnapshotStore::new).ok_or_else(|| DspiError::Io {
            msg: "no folder has been named for snapshots".into(),
        })
    }

    // -- WP P1AQ --
    /// Whether a folder is named for snapshots ([`DspiCore::snapshot_set_dir`]).
    /// Read-only; no file is touched.
    pub(crate) fn snapshot_dir_named(&self) -> bool {
        self.snapshots
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    fn set_store_dir(&self, dir: String) -> Result<()> {
        let dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
        if let Some(d) = &dir {
            std::fs::create_dir_all(d).map_err(|e| DspiError::Io {
                msg: format!("the snapshots' folder: {e}"),
            })?;
        }
        *self.snapshots.0.lock().unwrap_or_else(|p| p.into_inner()) = dir;
        Ok(())
    }

    fn capture_into_store(&self, name: &str, note: &str) -> Result<SnapshotInfo> {
        let store = self.store()?;
        let (mut doc, serial) = self.capture_value(name, note)?;
        let saved = at(&doc, &[BLOCK, "savedMs"]).as_i64().unwrap_or(0);
        store
            .save(&serial, &mut doc, saved)
            .map(SnapshotInfo::from)
            .map_err(io_error)
    }

    fn list_store(&self, serial: String) -> Vec<SnapshotInfo> {
        let Ok(store) = self.store() else {
            return Vec::new();
        };
        let serial = if serial.is_empty() {
            match self.lock().conn.as_ref() {
                Some(c) => c.serial.clone(),
                None => return Vec::new(),
            }
        } else {
            serial
        };
        store
            .list(&serial)
            .into_iter()
            .map(SnapshotInfo::from)
            .collect()
    }

    fn read_store(&self, id: &str) -> Result<String> {
        self.store()?.read(id).map(|(_, t)| t).map_err(io_error)
    }

    fn update_store(
        &self,
        id: &str,
        name: Option<String>,
        note: Option<String>,
    ) -> Result<SnapshotInfo> {
        let store = self.store()?;
        let e = match (name, note) {
            (Some(n), _) => store.rename(id, &n),
            (None, Some(n)) => store.set_note(id, &n),
            (None, None) => store.read(id).map(|(e, _)| e),
        };
        e.map(SnapshotInfo::from).map_err(io_error)
    }

    fn delete_from_store(&self, id: &str) -> Result<()> {
        self.store()?.delete(id).map_err(io_error)
    }

    fn import_into_store(&self, text: &str, name: &str, note: &str) -> Result<SnapshotInfo> {
        let store = self.store()?;
        let mut doc = crate::files::parse_config(text).map_err(invalid)?;
        check_snapshot(&doc)?; // -- WP P1BFIX -- F1
        let open = self.lock().conn.as_ref().map(|c| c.serial.clone());
        let serial = open
            .or_else(|| {
                at(&doc, &[BLOCK, "serial"])
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .ok_or(DspiError::Disconnected)?;
        if let Some(top) = doc.as_object_mut() {
            if !name.is_empty()
                && let Some(meta) = top
                    .entry("meta")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
            {
                meta.insert("name".into(), json!(cut(name, MAX_NAME_CHARS)));
            }
            let block = top.entry(BLOCK).or_insert_with(|| json!({}));
            if !block.is_object() {
                *block = json!({});
            }
            if let Some(b) = block.as_object_mut() {
                b.insert("note".into(), json!(cut(note, MAX_NOTE_CHARS)));
            }
        }
        store
            .save(&serial, &mut doc, crate::events::now_ms())
            .map(SnapshotInfo::from)
            .map_err(io_error)
    }
}

/// The web has no files: the document calls are its store.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl DspiCore {
    // -- WP P1AQ --
    /// No folder is ever named on the web.
    pub(crate) fn snapshot_dir_named(&self) -> bool {
        false
    }
    fn set_store_dir(&self, _dir: String) -> Result<()> {
        Err(DspiError::Unsupported)
    }
    fn capture_into_store(&self, _name: &str, _note: &str) -> Result<SnapshotInfo> {
        Err(DspiError::Unsupported)
    }
    fn list_store(&self, _serial: String) -> Vec<SnapshotInfo> {
        Vec::new()
    }
    fn read_store(&self, _id: &str) -> Result<String> {
        Err(DspiError::Unsupported)
    }
    fn update_store(
        &self,
        _id: &str,
        _name: Option<String>,
        _note: Option<String>,
    ) -> Result<SnapshotInfo> {
        Err(DspiError::Unsupported)
    }
    fn delete_from_store(&self, _id: &str) -> Result<()> {
        Err(DspiError::Unsupported)
    }
    fn import_into_store(&self, _text: &str, _name: &str, _note: &str) -> Result<SnapshotInfo> {
        Err(DspiError::Unsupported)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1sn;

// -- WP P1BFIX --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1bfix;
