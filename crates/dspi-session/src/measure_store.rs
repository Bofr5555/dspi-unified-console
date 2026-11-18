//! The host-side store of measurement documents (WP P5DOCK, PLAN-P5 T11 and
//! section 1.6).
//!
//! The three kinds of `dspi_measure::doc` (`measurement`, `mic`, `target`) are
//! `.dspipreset` schema-1 documents. This module only files them, by the
//! snapshot store's rules (`snapshot_store.rs`, WP P1SN):
//!
//! - **folders:** one per device serial for measurements, named as the
//!   snapshot store names its folders (`s-<serial>`, or `x-<hex>`); `mic/` for
//!   the microphones and `target/` for the targets (neither can be a serial's
//!   folder name);
//! - **one document per file**, `<id>.dspipreset`; the id is the store's
//!   (ASCII digits, lowercase letters and `-`, never a name), **unique in the
//!   whole store**: a document's own id is kept when it is free, otherwise one
//!   is made from its time; an id two folders hold (copied in by hand) is
//!   refused, never guessed. A put never overwrites a filed document;
//! - **an index** per folder, `index.json` (`{version, documents: [{id, kind,
//!   name, note, serial, savedMs, bytes}]}`), a cache: a torn or hand-edited
//!   one is read entry by entry and rebuilt from the documents;
//! - **atomic writes** (a temporary file, then a rename), files 0600 and
//!   folders 0700 on Unix;
//! - **the corrupt-file rule:** a document that no longer reads as its
//!   folder's kind (torn, hand-edited past its bounds, of another kind) is set
//!   aside as `<id>.dspipreset.bad-<unix seconds>` and is no longer listed or
//!   served; nothing else is touched;
//! - **bounds** (PLAN-P5 section 3): 100 measurements per device, 32
//!   microphones, 64 targets, 64 MiB of documents in all; each document within
//!   its kind's own bound (1 MiB a measurement, 256 KiB a mic or target).
//!
//! Every document is validated with its kind's reader before it is filed, and
//! again when it is read back. On the web there are no files: the core's
//! document calls give and take document text there, and the file side of
//! this module is gated off (as the snapshot store's).

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_measure::doc::{self, DocKind, MeasurementDoc, MicDoc, TargetDoc};
use dspi_measure::types::MeasureError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::snapshot_store::{dir_name, id_for, serial_of_dir, valid_id};

/// The index's file name in each folder.
pub const INDEX_FILE: &str = "index.json";
/// The extension of a document.
pub const DOC_EXT: &str = "dspipreset";
/// The microphones' folder.
pub const MIC_DIR: &str = "mic";
/// The targets' folder.
pub const TARGET_DIR: &str = "target";
/// The most measurements filed for one device.
pub const MAX_MEASUREMENTS_PER_DEVICE: usize = 100;
/// The most microphones filed.
pub const MAX_MICS: usize = 32;
/// The most targets filed.
pub const MAX_TARGETS: usize = 64;
/// The most bytes of documents in the whole store.
pub const MAX_STORE_BYTES: u64 = 64 << 20;
/// The largest index this store reads; a larger one is rebuilt.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
const MAX_INDEX_BYTES: u64 = 1 << 20;
/// The index format this build writes.
const INDEX_VERSION: u32 = 1;

/// One document in a list: what the index keeps of it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureEntry {
    /// The store's id (ASCII digits, lowercase letters and `-`).
    pub id: String,
    /// The kind's word: `measurement`, `mic` or `target`.
    pub kind: String,
    /// The document's name (`meta.name`); any Unicode.
    pub name: String,
    /// The user's note (the block's `note`); any Unicode.
    pub note: String,
    /// The measured device's serial (measurements only, else "").
    pub serial: String,
    /// When it was measured (a measurement's `savedMs`) or filed (the others:
    /// the time its id was made from), ms since the Unix epoch.
    pub saved_ms: i64,
    /// The document's size, bytes.
    pub bytes: u64,
}

/// The most entries any one folder may list (the largest bound above).
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
fn max_count(kind: DocKind) -> usize {
    match kind {
        DocKind::Measurement => MAX_MEASUREMENTS_PER_DEVICE,
        DocKind::Mic => MAX_MICS,
        DocKind::Target => MAX_TARGETS,
    }
}

/// A document of the measurement family, read and validated by its kind's
/// reader.
#[derive(Debug, Clone, PartialEq)]
pub enum AnyDoc {
    /// A `measurement` document.
    Measurement(Box<MeasurementDoc>),
    /// A `mic` document.
    Mic(Box<MicDoc>),
    /// A `target` document.
    Target(Box<TargetDoc>),
}

impl AnyDoc {
    /// Reads any of the three kinds: its kind from `meta.kind`, then that
    /// kind's bounded reader and validation.
    ///
    /// Errors: [`MeasureError::Invalid`] for a text that is not a document
    /// of the family (a preset, a snapshot, not JSON, past 1 MiB) or that the
    /// kind's reader refuses; [`MeasureError::Parse`] as the reader's.
    pub fn parse(text: &str) -> Result<AnyDoc, MeasureError> {
        match doc::doc_kind(text) {
            Some(DocKind::Measurement) => {
                MeasurementDoc::from_text(text).map(|d| AnyDoc::Measurement(Box::new(d)))
            }
            Some(DocKind::Mic) => MicDoc::from_text(text).map(|d| AnyDoc::Mic(Box::new(d))),
            Some(DocKind::Target) => {
                TargetDoc::from_text(text).map(|d| AnyDoc::Target(Box::new(d)))
            }
            None => Err(MeasureError::Invalid(
                "not a measurement, microphone or target document".into(),
            )),
        }
    }

    /// Its kind.
    pub fn kind(&self) -> DocKind {
        match self {
            AnyDoc::Measurement(_) => DocKind::Measurement,
            AnyDoc::Mic(_) => DocKind::Mic,
            AnyDoc::Target(_) => DocKind::Target,
        }
    }

    /// The store id it carries ("" when none).
    pub fn id(&self) -> &str {
        match self {
            AnyDoc::Measurement(d) => &d.measurement.id,
            AnyDoc::Mic(d) => &d.mic.id,
            AnyDoc::Target(d) => &d.target.id,
        }
    }

    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
    fn set_id(&mut self, id: &str) {
        let slot = match self {
            AnyDoc::Measurement(d) => &mut d.measurement.id,
            AnyDoc::Mic(d) => &mut d.mic.id,
            AnyDoc::Target(d) => &mut d.target.id,
        };
        *slot = id.to_string();
    }

    /// Its name (`meta.name`).
    pub fn name(&self) -> &str {
        match self {
            AnyDoc::Measurement(d) => &d.meta.name,
            AnyDoc::Mic(d) => &d.meta.name,
            AnyDoc::Target(d) => &d.meta.name,
        }
    }

    /// Sets its name, cut to `doc::MAX_NAME_CHARS` characters.
    pub fn set_name(&mut self, name: &str) {
        let name = cut(name, doc::MAX_NAME_CHARS);
        match self {
            AnyDoc::Measurement(d) => d.meta.name = name,
            AnyDoc::Mic(d) => d.meta.name = name,
            AnyDoc::Target(d) => d.meta.name = name,
        }
    }

    /// Its note (the block's `note`).
    pub fn note(&self) -> &str {
        match self {
            AnyDoc::Measurement(d) => &d.measurement.note,
            AnyDoc::Mic(d) => &d.mic.note,
            AnyDoc::Target(d) => &d.target.note,
        }
    }

    /// Sets its note, cut to `doc::MAX_NOTE_CHARS` characters.
    pub fn set_note(&mut self, note: &str) {
        let note = cut(note, doc::MAX_NOTE_CHARS);
        match self {
            AnyDoc::Measurement(d) => d.measurement.note = note,
            AnyDoc::Mic(d) => d.mic.note = note,
            AnyDoc::Target(d) => d.target.note = note,
        }
    }

    /// The measured device's serial (a measurement's; "" for the others).
    pub fn serial(&self) -> &str {
        match self {
            AnyDoc::Measurement(d) => &d.measurement.serial,
            AnyDoc::Mic(_) | AnyDoc::Target(_) => "",
        }
    }

    /// A measurement's `savedMs`; `None` for the others.
    pub fn saved_ms(&self) -> Option<i64> {
        match self {
            AnyDoc::Measurement(d) => Some(d.measurement.saved_ms),
            AnyDoc::Mic(_) | AnyDoc::Target(_) => None,
        }
    }

    /// The document's text as a store files it (validated, within its
    /// kind's bound).
    ///
    /// Errors: the kind's writer's.
    pub fn to_text(&self) -> Result<String, MeasureError> {
        match self {
            AnyDoc::Measurement(d) => d.to_text(),
            AnyDoc::Mic(d) => d.to_text(),
            AnyDoc::Target(d) => d.to_text(),
        }
    }

    /// Its entry, filed as `id` in a document of `bytes` bytes.
    pub fn entry(&self, id: &str, bytes: u64) -> MeasureEntry {
        MeasureEntry {
            id: id.to_string(),
            kind: self.kind().word().to_string(),
            name: self.name().to_string(),
            note: self.note().to_string(),
            serial: self.serial().to_string(),
            saved_ms: self.saved_ms().unwrap_or_else(|| saved_ms_of_id(id)),
            bytes,
        }
    }
}

/// `s` cut to at most `max` characters.
fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The time an id of this store was made from (its leading digits, ms), 0
/// for an id that does not start with one.
pub fn saved_ms_of_id(id: &str) -> i64 {
    let digits: String = id
        .chars()
        .take_while(char::is_ascii_digit)
        .take(18)
        .collect();
    digits.parse().unwrap_or(0)
}

/// The folder of a document of `kind` (and `serial`, for a measurement).
/// `None` for a measurement that names no device.
pub fn folder_name(kind: DocKind, serial: &str) -> Option<String> {
    match kind {
        DocKind::Measurement => (!serial.is_empty()).then(|| dir_name(serial)),
        DocKind::Mic => Some(MIC_DIR.to_string()),
        DocKind::Target => Some(TARGET_DIR.to_string()),
    }
}

/// The kind and serial of a folder name, if it is one of the store's.
pub fn folder_kind(name: &str) -> Option<(DocKind, String)> {
    match name {
        MIC_DIR => Some((DocKind::Mic, String::new())),
        TARGET_DIR => Some((DocKind::Target, String::new())),
        other => serial_of_dir(other).map(|s| (DocKind::Measurement, s)),
    }
}

/// The entries an index's text holds, read entry by entry: an entry that is
/// not an object of the right shape, has an id this store would not make,
/// is of another kind than `kind`, or repeats an earlier id is dropped. Text
/// that is not an index gives none.
pub fn parse_index(text: &str, kind: DocKind) -> Vec<MeasureEntry> {
    let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let Some(list) = v.get("documents").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<MeasureEntry> = Vec::new();
    for e in list {
        let Ok(entry) = serde_json::from_value::<MeasureEntry>(e.clone()) else {
            continue;
        };
        if valid_id(&entry.id) && entry.kind == kind.word() && !out.iter().any(|o| o.id == entry.id)
        {
            out.push(entry);
        }
    }
    out
}

/// The index's text for `entries`.
pub fn index_text(entries: &[MeasureEntry]) -> String {
    let v = serde_json::json!({ "version": INDEX_VERSION, "documents": entries });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The order of a list: by kind (measurement, mic, target), then oldest
/// first, then by id.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
fn sort(entries: &mut [MeasureEntry]) {
    let rank = |k: &str| {
        DocKind::ALL
            .iter()
            .position(|d| d.word() == k)
            .unwrap_or(usize::MAX)
    };
    entries.sort_by(|a, b| {
        rank(&a.kind)
            .cmp(&rank(&b.kind))
            .then_with(|| a.saved_ms.cmp(&b.saved_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use files::MeasureStore;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod files {
    //! The file side, gated off on the web (no files there).

    use std::collections::BTreeSet;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::path::{Path, PathBuf};

    use dspi_measure::doc::DocKind;

    use super::{
        AnyDoc, DOC_EXT, INDEX_FILE, MAX_INDEX_BYTES, MAX_STORE_BYTES, MeasureEntry, folder_kind,
        folder_name, id_for, index_text, max_count, parse_index, sort, valid_id,
    };

    /// Measurement, microphone and target documents filed under one folder.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MeasureStore {
        root: PathBuf,
    }

    fn not_found(id: &str) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, format!("no document {id}"))
    }

    fn invalid_data(msg: impl std::fmt::Display) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
    }

    fn full(msg: String) -> io::Error {
        io::Error::new(io::ErrorKind::QuotaExceeded, msg)
    }

    /// `dir` and its parents; on Unix a directory made here is the user's
    /// alone (0700).
    fn create_dir(dir: &Path) -> io::Result<()> {
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)
    }

    /// Write `text` to `path` atomically: a temporary file beside it (0600 on
    /// Unix), then a rename over it.
    fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let written = (|| {
            let mut o = fs::OpenOptions::new();
            o.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
            let mut f = o.open(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()
        })();
        written
            .and_then(|()| fs::rename(&tmp, path))
            .inspect_err(|_| {
                let _ = fs::remove_file(&tmp);
            })
    }

    /// The text of `path`, refused past `max` bytes without reading it all.
    fn read_bounded(path: &Path, max: u64) -> io::Result<String> {
        let f = fs::File::open(path)?;
        let mut text = String::new();
        f.take(max.saturating_add(1)).read_to_string(&mut text)?;
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > max {
            return Err(invalid_data(format!("larger than {max} bytes")));
        }
        Ok(text)
    }

    fn unix_seconds() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    impl MeasureStore {
        /// A store in `root`, the client's own data folder. Nothing is
        /// touched until the first write.
        pub fn new(root: impl Into<PathBuf>) -> Self {
            Self { root: root.into() }
        }

        /// The folder the store was given.
        pub fn root(&self) -> &Path {
            &self.root
        }

        fn doc_path(dir: &Path, id: &str) -> PathBuf {
            dir.join(format!("{id}.{DOC_EXT}"))
        }

        /// The ids of the documents in `dir`.
        fn files(dir: &Path) -> BTreeSet<String> {
            let Ok(rd) = fs::read_dir(dir) else {
                return BTreeSet::new();
            };
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let id = name.strip_suffix(&format!(".{DOC_EXT}"))?;
                    valid_id(id).then(|| id.to_string())
                })
                .collect()
        }

        /// Every folder of the store: its kind, serial ("" for mic and
        /// target) and path, sorted by name.
        fn folders(&self) -> Vec<(DocKind, String, PathBuf)> {
            let Ok(rd) = fs::read_dir(&self.root) else {
                return Vec::new();
            };
            let mut out: Vec<(String, DocKind, String)> = rd
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let (kind, serial) = folder_kind(&name)?;
                    Some((name, kind, serial))
                })
                .collect();
            out.sort_by(|a, b| a.0.cmp(&b.0));
            out.into_iter()
                .map(|(name, kind, serial)| (kind, serial, self.root.join(name)))
                .collect()
        }

        /// Set a damaged document aside (the corrupt-file rule):
        /// `<id>.dspipreset.bad-<unix seconds>`. Best effort.
        fn set_aside(dir: &Path, id: &str) -> PathBuf {
            let from = Self::doc_path(dir, id);
            let to = dir.join(format!("{id}.{DOC_EXT}.bad-{}", unix_seconds()));
            // A failed rename leaves the file where it is: it is refused again
            // on the next read, never served.
            let _ = fs::rename(&from, &to);
            to
        }

        /// The document `id` in `dir` of `kind`, read and validated; a damaged
        /// one is set aside and refused (`InvalidData`).
        fn read_doc(dir: &Path, kind: DocKind, id: &str) -> io::Result<(AnyDoc, String)> {
            let max = u64::try_from(kind.max_bytes()).unwrap_or(u64::MAX);
            let text = match read_bounded(&Self::doc_path(dir, id), max) {
                Ok(t) => t,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(e),
                Err(e) => {
                    let to = Self::set_aside(dir, id);
                    return Err(invalid_data(format!(
                        "document {id} could not be read ({e}); it was set aside as {}",
                        to.display()
                    )));
                }
            };
            match AnyDoc::parse(&text) {
                Ok(d) if d.kind() == kind => Ok((d, text)),
                Ok(d) => {
                    let to = Self::set_aside(dir, id);
                    Err(invalid_data(format!(
                        "document {id} is a {} document in the {} folder; it was set aside as {}",
                        d.kind().word(),
                        kind.word(),
                        to.display()
                    )))
                }
                Err(e) => {
                    let to = Self::set_aside(dir, id);
                    Err(invalid_data(format!(
                        "document {id} is damaged ({e}); it was set aside as {}",
                        to.display()
                    )))
                }
            }
        }

        /// The entries of one folder, oldest first. A torn or hand-edited
        /// index is repaired from the documents (and written back when it
        /// differed); a document the index does not name and that does not
        /// read is set aside.
        fn list_dir(&self, kind: DocKind, serial: &str, dir: &Path) -> Vec<MeasureEntry> {
            let on_disk = Self::files(dir);
            let index_path = dir.join(INDEX_FILE);
            let index_raw = fs::metadata(&index_path)
                .ok()
                .filter(|m| m.len() <= MAX_INDEX_BYTES)
                .and_then(|_| fs::read_to_string(&index_path).ok());
            let indexed = index_raw
                .as_deref()
                .map(|t| parse_index(t, kind))
                .unwrap_or_default();
            let mut out: Vec<MeasureEntry> = indexed
                .into_iter()
                .filter(|e| on_disk.contains(&e.id))
                .collect();
            for id in &on_disk {
                if out.iter().any(|e| &e.id == id) {
                    continue;
                }
                if let Ok((d, text)) = Self::read_doc(dir, kind, id) {
                    let mut e = d.entry(id, u64::try_from(text.len()).unwrap_or(u64::MAX));
                    if kind == DocKind::Measurement && e.serial.is_empty() {
                        e.serial = serial.to_string();
                    }
                    out.push(e);
                }
            }
            sort(&mut out);
            let text = index_text(&out);
            if (index_raw.is_some() || !on_disk.is_empty())
                && index_raw.as_deref() != Some(text.as_str())
            {
                let _ = write_atomic(&index_path, &text);
            }
            out
        }

        /// The documents of `kind` (`None`: every kind), oldest first within
        /// a kind; measurements of device `serial` only when one is given
        /// (`None` or "": every device's). Never fails: a missing folder is
        /// an empty list, a damaged index is rebuilt.
        pub fn list(&self, kind: Option<DocKind>, serial: Option<&str>) -> Vec<MeasureEntry> {
            let serial = serial.filter(|s| !s.is_empty());
            let mut out: Vec<MeasureEntry> = self
                .folders()
                .into_iter()
                .filter(|(k, s, _)| {
                    kind.is_none_or(|want| want == *k)
                        && (*k != DocKind::Measurement || serial.is_none_or(|want| want == s))
                })
                .flat_map(|(k, s, dir)| self.list_dir(k, &s, &dir))
                .collect();
            sort(&mut out);
            out
        }

        /// The folder holding document `id`, and its kind and serial.
        ///
        /// Errors: `NotFound` for an id no folder holds (or one this store
        /// would not make); `InvalidInput` for one more than one folder holds
        /// (copied in by hand), so no call acts on the wrong copy.
        fn locate(&self, id: &str) -> io::Result<(DocKind, String, PathBuf)> {
            if !valid_id(id) {
                return Err(not_found(id));
            }
            let mut holders = self
                .folders()
                .into_iter()
                .filter(|(_, _, dir)| Self::doc_path(dir, id).is_file());
            let first = holders.next().ok_or_else(|| not_found(id))?;
            if let Some((_, _, other)) = holders.next() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "document {id} is held in more than one folder ({}, {}): remove one of the copies",
                        first.2.display(),
                        other.display()
                    ),
                ));
            }
            Ok(first)
        }

        /// Document `id`'s entry and text (as filed).
        ///
        /// Errors: `NotFound`; `InvalidInput` (two folders); `InvalidData`
        /// for a damaged document, which is set aside.
        pub fn read(&self, id: &str) -> io::Result<(MeasureEntry, String)> {
            let (kind, serial, dir) = self.locate(id)?;
            let (d, text) = Self::read_doc(&dir, kind, id)?;
            let entry = self
                .list_dir(kind, &serial, &dir)
                .into_iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| d.entry(id, u64::try_from(text.len()).unwrap_or(u64::MAX)));
            Ok((entry, text))
        }

        /// The bytes of every document filed.
        fn total_bytes(&self) -> u64 {
            self.folders()
                .iter()
                .flat_map(|(_, _, dir)| {
                    Self::files(dir)
                        .into_iter()
                        .map(move |id| Self::doc_path(dir, &id))
                })
                .filter_map(|p| fs::metadata(p).ok())
                .fold(0u64, |a, m| a.saturating_add(m.len()))
        }

        /// File `text` (a `measurement`, `mic` or `target` document) as a new
        /// document. Its own id is kept when it is one this store would make
        /// and no folder holds it; otherwise an id is made from its time (a
        /// measurement's `savedMs`, else `now_ms`). Never overwrites.
        ///
        /// Returns its entry and the text filed (with its id).
        ///
        /// Errors: `InvalidData` for a text that is not a valid document of
        /// the family, or a measurement that names no device;
        /// `QuotaExceeded` past a bound (the folder's count, or the store's
        /// 64 MiB); the write's error.
        pub fn put(&self, text: &str, now_ms: i64) -> io::Result<(MeasureEntry, String)> {
            let mut d = AnyDoc::parse(text).map_err(invalid_data)?;
            let kind = d.kind();
            let folder = folder_name(kind, d.serial())
                .ok_or_else(|| invalid_data("the measurement names no device (serial)"))?;
            let dir = self.root.join(&folder);
            let listed = self.list_dir(kind, d.serial(), &dir);
            if listed.len() >= max_count(kind) {
                return Err(full(match kind {
                    DocKind::Measurement => format!(
                        "this device already has {} measurements: delete one first",
                        listed.len()
                    ),
                    DocKind::Mic => format!(
                        "{} microphones are filed already: delete one first",
                        listed.len()
                    ),
                    DocKind::Target => format!(
                        "{} targets are filed already: delete one first",
                        listed.len()
                    ),
                }));
            }
            let mut taken: BTreeSet<String> = BTreeSet::new();
            for (_, _, other) in self.folders() {
                taken.extend(Self::files(&other));
            }
            taken.extend(listed.iter().map(|e| e.id.clone()));
            let own = d.id().to_string();
            let id = if valid_id(&own) && !taken.contains(&own) {
                own
            } else {
                let at = d.saved_ms().filter(|t| *t > 0).unwrap_or(now_ms);
                (0..u32::MAX)
                    .map(|n| id_for(at, n))
                    .find(|id| !taken.contains(id))
                    .ok_or_else(|| io::Error::other("no free document id"))?
            };
            d.set_id(&id);
            let filed = d.to_text().map_err(invalid_data)?;
            let bytes = u64::try_from(filed.len()).unwrap_or(u64::MAX);
            let total = self.total_bytes().saturating_add(bytes);
            if total > MAX_STORE_BYTES {
                return Err(full(format!(
                    "the measurement store would hold {total} bytes (at most {MAX_STORE_BYTES}): delete a document first"
                )));
            }
            create_dir(&dir)?;
            write_atomic(&Self::doc_path(&dir, &id), &filed)?;
            let entry = d.entry(&id, bytes);
            let mut entries = listed;
            entries.push(entry.clone());
            sort(&mut entries);
            // The document is filed; a failed index write heals on the next list.
            let _ = write_atomic(&dir.join(INDEX_FILE), &index_text(&entries));
            Ok((entry, filed))
        }

        /// Change document `id` with `f`, write it back atomically and return
        /// its new entry.
        fn update(&self, id: &str, f: impl FnOnce(&mut AnyDoc)) -> io::Result<MeasureEntry> {
            let (kind, serial, dir) = self.locate(id)?;
            let (mut d, _) = Self::read_doc(&dir, kind, id)?;
            f(&mut d);
            let text = d.to_text().map_err(invalid_data)?;
            write_atomic(&Self::doc_path(&dir, id), &text)?;
            let entry = d.entry(id, u64::try_from(text.len()).unwrap_or(u64::MAX));
            let mut entries: Vec<MeasureEntry> = self
                .list_dir(kind, &serial, &dir)
                .into_iter()
                .filter(|e| e.id != id)
                .collect();
            entries.push(entry.clone());
            sort(&mut entries);
            let _ = write_atomic(&dir.join(INDEX_FILE), &index_text(&entries));
            Ok(entry)
        }

        /// Rename document `id` (its `meta.name`, cut to 256 characters).
        /// Errors as [`MeasureStore::read`], and the write's.
        pub fn rename(&self, id: &str, name: &str) -> io::Result<MeasureEntry> {
            self.update(id, |d| d.set_name(name))
        }

        /// Set document `id`'s note (cut to 4096 characters). Errors as
        /// [`MeasureStore::rename`].
        pub fn set_note(&self, id: &str, note: &str) -> io::Result<MeasureEntry> {
            self.update(id, |d| d.set_note(note))
        }

        // -- WP P5TUNE -- a tune's row on a filed measurement (the update path:
        // `put` never replaces a document)
        /// Add `record` to measurement `id`'s `tunes` (the oldest dropped past
        /// [`dspi_measure::doc::MAX_TUNES`]) and write it back atomically, its
        /// id unchanged.
        ///
        /// Errors: `InvalidInput` for a document that is not a measurement;
        /// as [`MeasureStore::rename`].
        pub fn add_tune(
            &self,
            id: &str,
            record: dspi_measure::doc::TuneRecord,
        ) -> io::Result<MeasureEntry> {
            let (kind, _, _) = self.locate(id)?;
            if kind != DocKind::Measurement {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "only a measurement keeps tunes",
                ));
            }
            self.update(id, |d| {
                if let AnyDoc::Measurement(m) = d {
                    let t = &mut m.measurement.tunes;
                    while t.len() >= dspi_measure::doc::MAX_TUNES {
                        t.remove(0);
                    }
                    t.push(record);
                }
            })
        }

        /// Delete document `id`: its file, then its entry.
        /// Errors: `NotFound`; `InvalidInput` (two folders); the removal's.
        pub fn delete(&self, id: &str) -> io::Result<()> {
            let (kind, serial, dir) = self.locate(id)?;
            fs::remove_file(Self::doc_path(&dir, id))?;
            // Listing rewrites the index without it.
            let _ = self.list_dir(kind, &serial, &dir);
            Ok(())
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5dock;
