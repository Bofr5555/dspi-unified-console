//! The host-side snapshot store (WP P1SN, PRO1 W3).
//!
//! A **snapshot** is a whole-device state kept by a client: a `.dspipreset`
//! document with `meta.kind = "snapshot"` and a `snapshot` block (the bulk
//! image and the settings outside it; `dspi_core`'s `snapshot.rs` writes and
//! reads that block). This module only files them:
//!
//! - one folder per device serial under the folder the client names, named as
//!   the event log names its files (`s-<serial>` when the serial is ASCII
//!   letters, digits, `-` and `_`, else `x-<the serial's UTF-8 bytes in hex>`);
//! - one document per snapshot, `<id>.dspipreset`, the id made here from the
//!   capture time (ASCII digits and `-`, never a name: a name may hold any
//!   Unicode, a file name may not), unique in the whole store, not only in its
//!   serial's folder, since the calls that take an id look in every folder; an
//!   id two folders hold (copied in by hand) is refused, never guessed
//!   (WP P1BFIX, F3);
//! - an index, `index.json`, `{version, snapshots: [{id, name, note, serial,
//!   savedMs, platform, inputChannels, outputChannels, wireFormat}]}`, so a
//!   list reads one file.
//!
//! The documents are the truth and the index a cache. A list never fails on
//! the index: a torn or hand-edited one (bad JSON, wrong types, unknown or
//! repeated ids, entries whose document is gone) is read entry by entry, what
//! is missing is rebuilt from the documents, and the index is written again
//! when it differed. Every write is atomic (a temporary file, then a rename),
//! so a crash leaves the old file or the new one.
//!
//! The index format is one for every client, so a hub's copy can merge with
//! it later (OD-P1b). On the web there are no files: the core's document calls
//! (capture and recall of a document's text) are used there instead, and the
//! file side of this module is gated off.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The index's file name in each serial's folder.
pub const INDEX_FILE: &str = "index.json";

/// The extension of a snapshot's document.
pub const SNAPSHOT_EXT: &str = "dspipreset";

/// The document's top-level block that carries what a snapshot adds to a
/// preset document.
pub const BLOCK: &str = "snapshot";

/// The index format this build writes.
const INDEX_VERSION: u32 = 1;

/// The longest id this store makes or accepts.
const MAX_ID_LEN: usize = 64;

/// One snapshot in a list: what the index keeps of it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SnapshotEntry {
    /// The store's id (ASCII digits, lowercase letters and `-`).
    pub id: String,
    /// The user's name for it; any Unicode.
    pub name: String,
    /// The user's note; any Unicode.
    pub note: String,
    /// The serial of the device it was taken from (or imported for).
    pub serial: String,
    /// When it was taken, ms since the Unix epoch.
    pub saved_ms: i64,
    /// The platform its document names (`RP2040`, `RP2350`), "" when none.
    pub platform: String,
    /// Input channels its document names, 0 when none.
    pub input_channels: u8,
    /// Output channels its document names, 0 when none.
    pub output_channels: u8,
    /// The wire format of its bulk image (or its document's), 0 when none.
    pub wire_format: u8,
}

/// The folder name of `serial`'s snapshots (see the module documentation).
pub fn dir_name(serial: &str) -> String {
    let safe = !serial.is_empty()
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if safe {
        format!("s-{serial}")
    } else {
        let hex: String = serial.bytes().map(|b| format!("{b:02x}")).collect();
        format!("x-{hex}")
    }
}

/// The serial a folder name names, if it is one of ours.
pub fn serial_of_dir(name: &str) -> Option<String> {
    if let Some(s) = name.strip_prefix("s-") {
        return (!s.is_empty()).then(|| s.to_string());
    }
    let hex = name.strip_prefix("x-")?;
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| {
            hex.get(i..i.saturating_add(2))
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect();
    String::from_utf8(bytes?).ok()
}

/// Whether `id` is one this store could have made: 1 to 64 ASCII digits,
/// lowercase letters and `-`, not starting with `-`. Anything else (a path,
/// `..`, a name) is never looked up.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'-')
}

/// What a document says of itself, as an entry with id `id`. Lenient: a
/// field that is missing or of the wrong type reads as empty.
pub fn entry_from_document(id: &str, doc: &Value) -> SnapshotEntry {
    let meta = doc.get("meta");
    let block = doc.get(BLOCK);
    let text = |v: Option<&Value>, k: &str| {
        v.and_then(|m| m.get(k))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let small = |v: Option<&Value>, k: &str| {
        v.and_then(|m| m.get(k))
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(0)
    };
    let wire = match small(block, "wireFormat") {
        0 => small(meta, "wireFormatVersion"),
        w => w,
    };
    SnapshotEntry {
        id: id.to_string(),
        name: text(meta, "name"),
        note: text(block, "note"),
        serial: text(block, "serial"),
        saved_ms: block
            .and_then(|b| b.get("savedMs"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        platform: text(meta, "platform"),
        input_channels: small(meta, "inputChannelCount"),
        output_channels: small(meta, "outputChannelCount"),
        wire_format: wire,
    }
}

/// The entries an index's text holds, read entry by entry: an entry that is
/// not an object of the right shape, has an id this store would not make, or
/// repeats an earlier id is dropped. Text that is not an index gives none.
pub fn parse_index(text: &str) -> Vec<SnapshotEntry> {
    let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let Some(list) = v.get("snapshots").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<SnapshotEntry> = Vec::new();
    for e in list {
        let Ok(entry) = serde_json::from_value::<SnapshotEntry>(e.clone()) else {
            continue;
        };
        if valid_id(&entry.id) && !out.iter().any(|o| o.id == entry.id) {
            out.push(entry);
        }
    }
    out
}

/// The index's text for `entries`.
pub fn index_text(entries: &[SnapshotEntry]) -> String {
    let v = serde_json::json!({ "version": INDEX_VERSION, "snapshots": entries });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// `entries` in list order: oldest first, then by id.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
fn sorted(mut entries: Vec<SnapshotEntry>) -> Vec<SnapshotEntry> {
    entries.sort_by(|a, b| a.saved_ms.cmp(&b.saved_ms).then_with(|| a.id.cmp(&b.id)));
    entries
}

/// The id for a snapshot taken at `saved_ms`: the time, with `-<n>` added
/// for the `n`th that would otherwise collide.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
fn id_for(saved_ms: i64, n: u32) -> String {
    let t = saved_ms.max(0);
    if n == 0 {
        format!("{t:013}")
    } else {
        format!("{t:013}-{n}")
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use files::SnapshotStore;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod files {
    //! The file side, gated off on the web (no files there).

    use std::collections::BTreeSet;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::{
        BLOCK, INDEX_FILE, SNAPSHOT_EXT, SnapshotEntry, dir_name, entry_from_document, id_for,
        index_text, parse_index, serial_of_dir, sorted, valid_id,
    };

    /// Snapshots filed under one folder, one subfolder per serial.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SnapshotStore {
        root: PathBuf,
    }

    fn not_found(id: &str) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, format!("no snapshot {id}"))
    }

    /// `dir` and its parents; on Unix a directory made here is the user's
    /// alone (0700), as the event log's.
    fn create_dir(dir: &Path) -> io::Result<()> {
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)
    }

    /// Write `text` to `path` atomically: a temporary file beside it, then a
    /// rename over it.
    fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }

    impl SnapshotStore {
        /// A store in `root`, the client's own data folder. Nothing is
        /// touched until the first write.
        pub fn new(root: impl Into<PathBuf>) -> Self {
            Self { root: root.into() }
        }

        /// The folder the store was given.
        pub fn root(&self) -> &Path {
            &self.root
        }

        fn dir(&self, serial: &str) -> PathBuf {
            self.root.join(dir_name(serial))
        }

        fn doc_path(dir: &Path, id: &str) -> PathBuf {
            dir.join(format!("{id}.{SNAPSHOT_EXT}"))
        }

        /// The ids of the documents in `dir`.
        fn files(dir: &Path) -> BTreeSet<String> {
            let Ok(rd) = fs::read_dir(dir) else {
                return BTreeSet::new();
            };
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let id = name.strip_suffix(&format!(".{SNAPSHOT_EXT}"))?;
                    valid_id(id).then(|| id.to_string())
                })
                .collect()
        }

        /// File `doc` as a new snapshot of `serial` taken at `saved_ms`:
        /// an id no folder of the store holds is made, and `meta.kind` and the
        /// `snapshot` block's `id`, `serial` and `savedMs` are set in `doc`
        /// before it is written.
        ///
        /// Errors: the folder or a file could not be written.
        pub fn save(
            &self,
            serial: &str,
            doc: &mut Value,
            saved_ms: i64,
        ) -> io::Result<SnapshotEntry> {
            let dir = self.dir(serial);
            create_dir(&dir)?;
            // -- WP P1BFIX -- F3: free in every serial's folder.
            let mut taken = Self::files(&dir);
            for other in self.serials() {
                taken.extend(Self::files(&self.dir(&other)));
            }
            let listed = self.list(serial);
            let id = (0..u32::MAX)
                .map(|n| id_for(saved_ms, n))
                .find(|id| !taken.contains(id) && !listed.iter().any(|e| &e.id == id))
                .ok_or_else(|| io::Error::other("no free snapshot id"))?;
            let Some(top) = doc.as_object_mut() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a snapshot document is a JSON object",
                ));
            };
            if let Some(meta) = top
                .entry("meta")
                .or_insert_with(|| json!({}))
                .as_object_mut()
            {
                meta.insert("kind".into(), json!("snapshot"));
            }
            let block = top.entry(BLOCK).or_insert_with(|| json!({}));
            if !block.is_object() {
                *block = json!({});
            }
            if let Some(b) = block.as_object_mut() {
                b.insert("id".into(), json!(id));
                b.insert("serial".into(), json!(serial));
                b.insert("savedMs".into(), json!(saved_ms));
            }
            let text = serde_json::to_string_pretty(doc).map_err(io::Error::other)?;
            write_atomic(&Self::doc_path(&dir, &id), &text)?;
            let entry = entry_from_document(&id, doc);
            let mut entries = listed;
            entries.push(entry.clone());
            // The document is filed; a failed index write heals on the next list.
            let _ = write_atomic(&dir.join(INDEX_FILE), &index_text(&sorted(entries)));
            Ok(entry)
        }

        /// `serial`'s snapshots, oldest first. Never fails: a missing folder
        /// is an empty list, and a torn or hand-edited index is repaired from
        /// the documents (and written back when it differed).
        pub fn list(&self, serial: &str) -> Vec<SnapshotEntry> {
            let dir = self.dir(serial);
            let on_disk = Self::files(&dir);
            let index_raw = fs::read_to_string(dir.join(INDEX_FILE)).ok();
            let indexed = index_raw.as_deref().map(parse_index).unwrap_or_default();
            let mut out: Vec<SnapshotEntry> = indexed
                .iter()
                .filter(|e| on_disk.contains(&e.id))
                .cloned()
                .collect();
            for id in &on_disk {
                if out.iter().any(|e| &e.id == id) {
                    continue;
                }
                let doc = fs::read_to_string(Self::doc_path(&dir, id))
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok());
                // A torn document no index names is not a snapshot to show.
                if let Some(doc) = doc.filter(Value::is_object) {
                    let mut e = entry_from_document(id, &doc);
                    if e.serial.is_empty() {
                        e.serial = serial.to_string();
                    }
                    out.push(e);
                }
            }
            let out = sorted(out);
            let text = index_text(&out);
            if (index_raw.is_some() || !on_disk.is_empty())
                && index_raw.as_deref() != Some(text.as_str())
            {
                let _ = write_atomic(&dir.join(INDEX_FILE), &text);
            }
            out
        }

        /// The serials with a snapshot folder, sorted.
        pub fn serials(&self) -> Vec<String> {
            let Ok(rd) = fs::read_dir(&self.root) else {
                return Vec::new();
            };
            let mut out: Vec<String> = rd
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| serial_of_dir(&e.file_name().into_string().ok()?))
                .collect();
            out.sort();
            out
        }

        /// The serial whose folder holds snapshot `id`.
        ///
        /// Errors: `NotFound` for an id no folder holds; `InvalidInput` for
        /// one more than one folder holds (copied in by hand: this store makes
        /// ids unique across folders), so no call acts on another serial's
        /// snapshot by accident (-- WP P1BFIX -- F3).
        fn locate(&self, id: &str) -> io::Result<String> {
            if !valid_id(id) {
                return Err(not_found(id));
            }
            let mut holders = self
                .serials()
                .into_iter()
                .filter(|s| Self::doc_path(&self.dir(s), id).is_file());
            let serial = holders.next().ok_or_else(|| not_found(id))?;
            if let Some(other) = holders.next() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "snapshot {id} is held for more than one device \
                         ({serial}, {other}): remove one of the copies"
                    ),
                ));
            }
            Ok(serial)
        }

        /// Snapshot `id`'s entry and document text.
        ///
        /// Errors: `NotFound` for an id no folder holds; the read's error.
        pub fn read(&self, id: &str) -> io::Result<(SnapshotEntry, String)> {
            let serial = self.locate(id)?;
            let text = fs::read_to_string(Self::doc_path(&self.dir(&serial), id))?;
            let entry = self
                .list(&serial)
                .into_iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| SnapshotEntry {
                    id: id.to_string(),
                    serial: serial.clone(),
                    ..SnapshotEntry::default()
                });
            Ok((entry, text))
        }

        /// Change snapshot `id`'s document with `f` and its entry with `g`,
        /// both written atomically. Errors: `NotFound`; `InvalidData` for a
        /// document that is not a JSON object; the write's error.
        fn update(
            &self,
            id: &str,
            f: impl FnOnce(&mut serde_json::Map<String, Value>),
            g: impl FnOnce(&mut SnapshotEntry),
        ) -> io::Result<SnapshotEntry> {
            let serial = self.locate(id)?;
            let dir = self.dir(&serial);
            let path = Self::doc_path(&dir, id);
            let mut doc: Value = serde_json::from_str(&fs::read_to_string(&path)?)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let top = doc.as_object_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "not a snapshot document")
            })?;
            f(top);
            write_atomic(
                &path,
                &serde_json::to_string_pretty(&doc).map_err(io::Error::other)?,
            )?;
            let mut entries = self.list(&serial);
            let found = entries.iter_mut().find(|e| e.id == id).map(|e| {
                g(e);
                e.clone()
            });
            let _ = write_atomic(&dir.join(INDEX_FILE), &index_text(&entries));
            found.ok_or_else(|| not_found(id))
        }

        /// Rename snapshot `id` (its document's `meta.name` and its entry).
        pub fn rename(&self, id: &str, name: &str) -> io::Result<SnapshotEntry> {
            self.update(
                id,
                |top| {
                    if let Some(meta) = top
                        .entry("meta")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                    {
                        meta.insert("name".into(), json!(name));
                    }
                },
                |e| e.name = name.to_string(),
            )
        }

        /// Set snapshot `id`'s note (its document's `snapshot.note` and its
        /// entry).
        pub fn set_note(&self, id: &str, note: &str) -> io::Result<SnapshotEntry> {
            self.update(
                id,
                |top| {
                    let block = top.entry(BLOCK).or_insert_with(|| json!({}));
                    if !block.is_object() {
                        *block = json!({});
                    }
                    if let Some(b) = block.as_object_mut() {
                        b.insert("note".into(), json!(note));
                    }
                },
                |e| e.note = note.to_string(),
            )
        }

        /// Delete snapshot `id`: its document, then its entry.
        /// Errors: `NotFound`; the removal's error.
        pub fn delete(&self, id: &str) -> io::Result<()> {
            let serial = self.locate(id)?;
            let dir = self.dir(&serial);
            fs::remove_file(Self::doc_path(&dir, id))?;
            // Listing rewrites the index without it.
            let _ = self.list(&serial);
            Ok(())
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1sn;
