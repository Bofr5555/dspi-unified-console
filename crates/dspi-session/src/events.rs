//! The event log (WP P0M, PRO1 M1, the client half): a persistent,
//! bounded log per device of what happened to it, kept as JSON lines.
//!
//! Lives in the session so `dspi-core` (which feeds it for C, UniFFI and wasm),
//! the Terminal and later the hub can share the record, the file format and
//! the store. The hub half (a shared log written by the hub, many devices in
//! one folder) is [`store::EventStore`] (WP P2EVW), which takes the serial
//! with each record.
//!
//! # The file format
//!
//! One file per device in the client's data directory, named by its serial:
//! `events-s-<serial>.jsonl` when the serial is ASCII letters, digits, `-` and
//! `_` only, else `events-x-<the serial's UTF-8 bytes in hex>.jsonl`. Each
//! line is one [`EventRecord`] in JSON, ending in `\n`, oldest first. When a
//! file would grow past [`Limits::max_file_bytes`] it is rotated: `.1.jsonl` is
//! the newest older file, up to [`Limits::rotated_files`]; the oldest goes.
//! A device's log therefore stays under `max_file_bytes * (rotated_files +
//! 1)` (1 MiB with the defaults), and one line under [`Limits::max_line_bytes`].
//!
//! Reading tolerates anything: a line that is not a record (a torn write, a
//! hand edit, a newer build's unknown kind is still read, as
//! [`EventKind::Unknown`]) is skipped and counted, a file that grew far past
//! its bound is read from its tail only, and an unreadable file reads as
//! empty. Nothing in a file can make a read panic or fail.
//!
//! Ids rise per device across files and runs, up to [`MAX_ID`]: a new store
//! reads the newest record's id when a device opens. A reader pages with the
//! last id it saw.
//!
//! **Several writers.** Each append takes an exclusive advisory lock on
//! `<stem>.lock` beside the files (`flock`, `LockFileEx`) for its id, its size
//! check, a rotation and the write, and settles the id there as one past the
//! newest in the files. So two writers of one serial in one directory (a hub
//! and a Terminal on one machine, two cores in one process) never write the
//! same id or rotate under each other. The rule for any writer of this format:
//! append only under that lock, with the id taken under it. Readers take no
//! lock; a torn last line is skipped. A file system without advisory locks
//! (some network shares) fails the append, and the record is kept in memory.
//!
//! On Unix the directory is created 0700 and the files 0600: the log holds
//! serials, firmware versions and notification text, the user's alone.
//!
//! # Coalescing
//!
//! A record with a [`EventRecord::coalesce`] key (a slider dragged through
//! fifty notifications) is held back: the next record with the same key within
//! [`COALESCE_MS`] of the first replaces it and counts one more in
//! [`EventRecord::count`]. Anything else, a read, an export or closing the
//! device writes it out.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
#![warn(missing_docs)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use dspi_transport::clock::Instant;
use serde::{Deserialize, Serialize};

/// The record format's version, written as `v` on every line.
pub const RECORD_VERSION: u8 = 1;

/// How long a coalescing record waits for a repeat, ms.
pub const COALESCE_MS: i64 = 1000;

/// What a record is about. Written in `snake_case`; a kind this build does
/// not know reads as [`EventKind::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A decoded device notification (`text` is the monitor's line).
    Notification,
    /// A connection opened (`fw`, `uptime_s` when known, `source` the transport).
    Connected,
    /// A connection closed (`source` the reason).
    Disconnected,
    /// A channel clipped (`channel`); one record per clip episode.
    Clip,
    /// The chip temperature crossed a band edge (`value` in °C).
    Temperature,
    /// A starvation, underrun or ring counter stepped (`source` the counter,
    /// `value` its total, `count` the steps since the last record).
    Counter,
    /// The device restarted (`source` the reason when known).
    Reset,
    /// The firmware differs from the last connection's (`fw` the new one).
    Firmware,
    /// A document was imported (`lines` the changes).
    Import,
    /// A preset was recalled (`channel` the slot, `lines` the changes).
    Recall,
    // -- WP SW1 --
    /// The audio link (not DSPi Link): lost, locked, re-anchored (`source`
    /// which, `value` the alink frame's low 32 bits), or an apply-at-frame
    /// result (`source` `apply`). Written `audio_link` (the file format's
    /// value, kept since ALINK1); `alink` also reads as this kind.
    #[serde(rename = "audio_link", alias = "alink")]
    Alink,
    // -- WP P2EVW --
    /// The device stopped answering while attached (`value` the seconds it
    /// has been silent).
    Unresponsive,
    /// An alarm was raised, acknowledged or cleared (`source` its code, `by`
    /// who acknowledged it).
    Alarm,
    /// A schedule rule ran (`source` its id, `by` the rule's name, `lines`
    /// the results).
    Schedule,
    /// Access to the device changed hands: a lock taken or released, a role
    /// change (`by` the client).
    Access,
    /// Records were lost before they reached the log (`count` how many).
    EventsDropped,
    /// A kind from a newer build.
    #[default]
    #[serde(other)]
    Unknown,
}

impl EventKind {
    /// The name written in the file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notification => "notification",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Clip => "clip",
            Self::Temperature => "temperature",
            Self::Counter => "counter",
            Self::Reset => "reset",
            Self::Firmware => "firmware",
            Self::Import => "import",
            Self::Recall => "recall",
            Self::Alink => "audio_link",
            // -- WP P2EVW --
            Self::Unresponsive => "unresponsive",
            Self::Alarm => "alarm",
            Self::Schedule => "schedule",
            Self::Access => "access",
            Self::EventsDropped => "events_dropped",
            Self::Unknown => "unknown",
        }
    }
}

fn record_version() -> u8 {
    RECORD_VERSION
}

fn one() -> u32 {
    1
}

fn is_one(n: &u32) -> bool {
    *n == 1
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One line of the event log. Every field but `kind` and `text` is optional in
/// the file; absent ones read as their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    /// The record format's version ([`RECORD_VERSION`]).
    #[serde(default = "record_version")]
    pub v: u8,
    /// Rising per device; set by the store.
    #[serde(default)]
    pub id: u64,
    /// When, ms since the Unix epoch.
    #[serde(default)]
    pub t: i64,
    /// What happened.
    #[serde(default)]
    pub kind: EventKind,
    /// The device's serial; set by the store.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// One readable line.
    #[serde(default)]
    pub text: String,
    /// Who or what caused it: a notification's source (`HOST`, `PRESET`,
    /// `UART`, ...), a transport, a counter's name, a reset reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The DSPi Link session that caused it, when another one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<u32>,
    // -- WP P2EVW --
    /// Who caused it, by name: the client of the session that wrote (its
    /// client name), or the rule (an alarm rule, `Schedule: <name>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// This client caused it (its own write coming back).
    #[serde(default, skip_serializing_if = "is_false")]
    pub own: bool,
    /// A wire channel, or a preset slot for a recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
    /// A number: °C, a counter's total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// The firmware version, on connects and firmware changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fw: Option<String>,
    /// The device's uptime in seconds, where it reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_s: Option<u32>,
    /// Detail lines: an import's or a recall's changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    /// How many occurrences this record stands for (coalesced repeats, counter
    /// steps); 1 when absent.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count: u32,
    /// Coalescing key; never written.
    #[serde(skip)]
    pub coalesce: Option<String>,
}

impl EventRecord {
    /// A record of `kind` saying `text`, at `t` ms since the epoch.
    pub fn new(kind: EventKind, t: i64, text: impl Into<String>) -> Self {
        Self {
            v: RECORD_VERSION,
            id: 0,
            t,
            kind,
            serial: String::new(),
            text: text.into(),
            source: None,
            session: None,
            // -- WP P2EVW --
            by: None,
            own: false,
            channel: None,
            value: None,
            fw: None,
            uptime_s: None,
            lines: Vec::new(),
            count: 1,
            coalesce: None,
        }
    }
}

/// Where else records go: the hub's shared event log implements this (the hub
/// half of M1). Called with each record as it is written, with the store's
/// lock held: do not call back into the core from it, and do not block.
pub trait EventSink: Send + Sync {
    /// One record, final (ids and coalescing done).
    fn record(&self, record: &EventRecord);
}

/// The store's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// A file is rotated before it grows past this.
    pub max_file_bytes: u64,
    /// Older files kept per device.
    pub rotated_files: u32,
    /// A longer line is cut (its `lines`, then its `text`).
    pub max_line_bytes: usize,
    /// Records kept in memory (all devices), for a store with no directory.
    pub memory_records: usize,
}

impl Limits {
    /// 256 KiB per file, three older files, 4 KiB per line, 1000 in memory.
    pub const DEFAULT: Self = Self {
        max_file_bytes: 256 * 1024,
        rotated_files: 3,
        max_line_bytes: 4096,
        memory_records: 1000,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What a device's log said about its last connection, when it opened.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LastSeen {
    /// The firmware of the last record that named one.
    pub fw: Option<String>,
    /// The last record with an uptime: when, and the uptime then.
    pub uptime: Option<(i64, u32)>,
}

/// What a [`LogReader`] found of a device's files and memory, for
/// [`EventLog::open_device_with`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    /// The last connection.
    pub last: LastSeen,
    /// The highest id in use.
    pub max_id: u64,
}

/// A page of records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadBatch {
    /// Oldest first.
    pub records: Vec<EventRecord>,
    /// The id to pass next time: the last record's, or `after_id`.
    pub last_id: u64,
    /// Lines that were not records.
    pub skipped: u32,
}

/// The open device's part of the store.
struct Open {
    serial: String,
    next_id: u64,
}

/// The event log of every device this client sees.
///
/// Not thread-safe by itself; the core keeps it behind a mutex.
pub struct EventLog {
    limits: Limits,
    dir: Option<PathBuf>,
    open: Option<Open>,
    pending: Option<EventRecord>,
    memory: VecDeque<EventRecord>,
    /// Ids for devices with no file, per serial.
    memory_ids: HashMap<String, u64>,
    sink: Option<Arc<dyn EventSink>>,
    write_errors: u32,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(Limits::DEFAULT)
    }
}

impl EventLog {
    /// An empty store kept in memory until [`EventLog::set_dir`].
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            dir: None,
            open: None,
            pending: None,
            memory: VecDeque::new(),
            memory_ids: HashMap::new(),
            sink: None,
            write_errors: 0,
        }
    }

    /// Keep the log in files under `dir` (created if missing), or in
    /// memory only with `None`. An open device carries on in the new place.
    ///
    /// Errors: the directory cannot be created; `Unsupported` on the web,
    /// which has no files. The store is unchanged then.
    pub fn set_dir(&mut self, dir: Option<PathBuf>) -> std::io::Result<()> {
        if let Some(d) = &dir {
            files::create_dir(d)?;
        }
        self.flush_pending();
        self.dir = dir;
        if let Some(serial) = self.open.take().map(|o| o.serial) {
            self.open_device(&serial);
        }
        Ok(())
    }

    /// The directory in use, if any.
    pub fn dir(&self) -> Option<&PathBuf> {
        self.dir.as_ref()
    }

    /// Records that could not be written to a file since the store began (they
    /// were kept in memory instead).
    pub fn write_errors(&self) -> u32 {
        self.write_errors
    }

    /// Send every record to `sink` too, or stop with `None`.
    pub fn set_sink(&mut self, sink: Option<Arc<dyn EventSink>>) {
        self.sink = sink;
    }

    /// Make `serial` the device records go to, and say what its log
    /// remembers of its last connection. Reads files: a caller with a lock
    /// around the store uses [`EventLog::reader`] and
    /// [`EventLog::open_device_with`] instead, reading with the lock released.
    pub fn open_device(&mut self, serial: &str) -> LastSeen {
        let seen = self.reader(serial).seen();
        self.open_device_with(serial, seen)
    }

    /// [`EventLog::open_device`] with what a [`LogReader`] found.
    pub fn open_device_with(&mut self, serial: &str, seen: Seen) -> LastSeen {
        self.close_device();
        let memory_id = self.memory_ids.get(serial).copied().unwrap_or(0);
        self.open = Some(Open {
            serial: serial.to_string(),
            next_id: next_after(seen.max_id.max(memory_id)),
        });
        seen.last
    }

    /// What it takes to read `serial`'s log without the store: the
    /// directory, the bounds and a copy of its records in memory. A held-back
    /// record is written out first.
    pub fn reader(&mut self, serial: &str) -> LogReader {
        self.flush_pending();
        LogReader {
            dir: self.dir.clone(),
            limits: self.limits,
            serial: serial.to_string(),
            memory: self
                .memory
                .iter()
                .filter(|r| r.serial == serial)
                .cloned()
                .collect(),
        }
    }

    /// Write out what is held back and stop recording for the open device.
    pub fn close_device(&mut self) {
        self.flush_pending();
        self.open = None;
    }

    /// The open device's serial.
    pub fn device(&self) -> Option<&str> {
        self.open.as_ref().map(|o| o.serial.as_str())
    }

    /// Add `record` to the open device's log. Dropped with no device open.
    pub fn record(&mut self, record: EventRecord) {
        if self.open.is_none() {
            return;
        }
        if let Some(p) = &mut self.pending {
            let same = p.coalesce.is_some() && p.coalesce == record.coalesce;
            if same && record.t.saturating_sub(p.t) < COALESCE_MS && record.t >= p.t {
                let count = p.count.saturating_add(record.count);
                let t = p.t;
                *p = EventRecord { t, count, ..record };
                return;
            }
        }
        self.flush_pending();
        if record.coalesce.is_some() {
            self.pending = Some(record);
        } else {
            self.write(record);
        }
    }

    /// Write out a held-back record whose window has passed at `now_ms`.
    pub fn tick(&mut self, now_ms: i64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| now_ms.saturating_sub(p.t) >= COALESCE_MS)
        {
            self.flush_pending();
        }
    }

    fn flush_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.write(p);
        }
    }

    fn write(&mut self, mut record: EventRecord) {
        let Some(open) = &mut self.open else {
            return;
        };
        record.v = RECORD_VERSION;
        record.id = open.next_id;
        record.serial.clone_from(&open.serial);
        record.coalesce = None;
        let mut on_disk = false;
        if let Some(dir) = &self.dir {
            // The id is settled under the file's lock: another writer of
            // this serial in this directory may have moved it on.
            match files::append(dir, &mut record, &self.limits) {
                Ok(()) => on_disk = true,
                Err(_) => self.write_errors = self.write_errors.saturating_add(1),
            }
        }
        open.next_id = next_after(record.id);
        if !on_disk {
            encode(&mut record, self.limits.max_line_bytes);
            self.memory_ids.insert(record.serial.clone(), record.id);
            self.memory.push_back(record.clone());
            while self.memory.len() > self.limits.memory_records {
                self.memory.pop_front();
            }
        }
        if let Some(sink) = &self.sink {
            sink.record(&record);
        }
    }

    /// [`LogReader::read`] of `serial`, reading files with the store held.
    pub fn read(&mut self, serial: &str, after_id: u64, limit: usize) -> ReadBatch {
        self.reader(serial).read(after_id, limit)
    }

    /// [`LogReader::export`] of `serial`, reading files with the store held.
    pub fn export(&mut self, serial: &str) -> String {
        self.reader(serial).export()
    }

    /// The serials with an event log: files in the directory, the open device,
    /// and those in memory. Sorted.
    pub fn devices(&self) -> Vec<String> {
        let mut out = self.devices_in_memory();
        out.extend(self.dir.as_deref().map(files::devices).unwrap_or_default());
        out.sort();
        out.dedup();
        out
    }

    /// The open device and the serials in memory, unsorted: with
    /// [`EventLog::devices_in`] of [`EventLog::dir`], what
    /// [`EventLog::devices`] lists, with the directory listed outside a lock.
    pub fn devices_in_memory(&self) -> Vec<String> {
        let mut out: Vec<String> = self.memory.iter().map(|r| r.serial.clone()).collect();
        out.extend(self.open.as_ref().map(|o| o.serial.clone()));
        out
    }

    /// The serials with files in `dir`, unsorted.
    pub fn devices_in(dir: &std::path::Path) -> Vec<String> {
        files::devices(dir)
    }

    // -- WP P0E --
    /// `serial`'s log in `dir` as JSON lines, oldest first, in the file
    /// format: read only, with no store, no lock file and nothing created,
    /// for a diagnostics bundle of another program's folder. "" when there
    /// is none.
    pub fn export_in(dir: &std::path::Path, serial: &str) -> String {
        LogReader {
            dir: Some(dir.to_path_buf()),
            limits: Limits::DEFAULT,
            serial: serial.to_string(),
            memory: Vec::new(),
        }
        .export()
    }
}

/// A read of one device's log taken out of the store ([`EventLog::reader`]),
/// so files are read and parsed with no lock held. A writer appending
/// meanwhile is harmless: a torn last line is skipped.
#[derive(Debug, Clone)]
pub struct LogReader {
    dir: Option<PathBuf>,
    limits: Limits,
    serial: String,
    memory: Vec<EventRecord>,
}

impl LogReader {
    /// The records with an id above `after_id`, oldest first, at most
    /// `limit`: the oldest such when paging (`after_id` above 0), the newest
    /// `limit` for a first read (`after_id` 0). From the files and memory.
    pub fn read(&self, after_id: u64, limit: usize) -> ReadBatch {
        let (all, skipped) = self.all();
        let mut records: Vec<EventRecord> = all.into_iter().filter(|r| r.id > after_id).collect();
        if after_id == 0 {
            let drop = records.len().saturating_sub(limit);
            records.drain(..drop);
        } else {
            records.truncate(limit);
        }
        ReadBatch {
            last_id: records.last().map_or(after_id, |r| r.id),
            records,
            skipped,
        }
    }

    /// The whole log as JSON lines, oldest first, in the file format (bad
    /// lines left out).
    pub fn export(&self) -> String {
        let (all, _) = self.all();
        let mut out = String::new();
        for mut r in all {
            out.push_str(&encode(&mut r, usize::MAX));
        }
        out
    }

    /// What the log remembers of the last connection, and its highest id.
    pub fn seen(&self) -> Seen {
        let (all, _) = self.all();
        Seen {
            max_id: all.iter().map(|r| r.id).max().unwrap_or(0),
            last: LastSeen {
                fw: all.iter().rev().find_map(|r| r.fw.clone()),
                uptime: all.iter().rev().find_map(|r| r.uptime_s.map(|u| (r.t, u))),
            },
        }
    }

    /// File records, then memory ones, in id order.
    fn all(&self) -> (Vec<EventRecord>, u32) {
        let (mut all, skipped) = match &self.dir {
            Some(dir) => files::read(dir, &self.serial, &self.limits),
            None => (Vec::new(), 0),
        };
        all.extend(self.memory.iter().cloned());
        all.sort_by_key(|r| r.id);
        (all, skipped)
    }
}

/// The highest id a record may carry: 2^53 - 1, the largest integer a
/// JavaScript number holds exactly, so the web reads every id. A line with a
/// larger one is not a record ([`parse_line`]), so a corrupt or hand-edited
/// line cannot push the numbering to its end.
pub const MAX_ID: u64 = (1 << 53) - 1;

/// The id after `id`, never past [`MAX_ID`].
fn next_after(id: u64) -> u64 {
    id.saturating_add(1).min(MAX_ID)
}

/// `record` as one line within `max` bytes: its detail lines are dropped,
/// then its text cut, until it fits.
fn encode(record: &mut EventRecord, max: usize) -> String {
    let line = |r: &EventRecord| {
        let mut s = serde_json::to_string(r).unwrap_or_default();
        s.push('\n');
        s
    };
    let mut s = line(record);
    while s.len() > max && record.lines.pop().is_some() {
        s = line(record);
    }
    // Escapes can make the text longer on the line than in memory: cut until
    // it fits or nothing is left to cut.
    while s.len() > max && !record.text.is_empty() {
        let over = s.len().saturating_sub(max);
        let mut cut = record
            .text
            .len()
            .saturating_sub(over.saturating_add("…".len()));
        while cut > 0 && !record.text.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        record.text.truncate(cut);
        if cut > 0 {
            record.text.push('…');
        }
        s = line(record);
    }
    s
}

/// One line of a file as a record, if it is one.
pub fn parse_line(line: &str) -> Option<EventRecord> {
    let line = line.trim();
    // An object only: serde would take a JSON array as a record too.
    if !line.starts_with('{') {
        return None;
    }
    serde_json::from_str::<EventRecord>(line)
        .ok()
        .filter(|r| r.id <= MAX_ID)
}

/// The file name stem for `serial` (see the module documentation).
pub fn file_stem(serial: &str) -> String {
    let safe = !serial.is_empty()
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if safe {
        format!("events-s-{serial}")
    } else {
        let hex: String = serial.bytes().map(|b| format!("{b:02x}")).collect();
        format!("events-x-{hex}")
    }
}

/// The serial a file stem names, if it is one of ours. Only the file side
/// reads stems, and the web has none.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
fn serial_of(stem: &str) -> Option<String> {
    if let Some(s) = stem.strip_prefix("events-s-") {
        return (!s.is_empty()).then(|| s.to_string());
    }
    let hex = stem.strip_prefix("events-x-")?;
    if hex.len() % 2 != 0 {
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

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod files {
    //! The file side, gated off on the web (no files there).

    use std::fs::{self, OpenOptions};
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};

    use super::{EventRecord, Limits, encode, file_stem, next_after, parse_line, serial_of};

    /// `dir` and its parents; on Unix a directory made here is the user's
    /// alone (0700). An existing one keeps its mode.
    pub(super) fn create_dir(dir: &Path) -> io::Result<()> {
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)
    }

    /// For appending to a log file, created as the user's alone on Unix (0600).
    fn open_append(p: &Path) -> io::Result<fs::File> {
        let mut o = OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(p)
    }

    /// The current file (`n` 0) or the `n`th older one.
    fn path(dir: &Path, serial: &str, n: u32) -> PathBuf {
        let stem = file_stem(serial);
        if n == 0 {
            dir.join(format!("{stem}.jsonl"))
        } else {
            dir.join(format!("{stem}.{n}.jsonl"))
        }
    }

    /// The advisory lock file of `serial`'s log (`<stem>.lock`).
    fn lock_path(dir: &Path, serial: &str) -> PathBuf {
        dir.join(format!("{}.lock", file_stem(serial)))
    }

    /// Append `record` to `serial`'s current file, rotating first if it would
    /// not fit, all under an exclusive advisory lock on `<stem>.lock`
    /// (`flock` on Unix, `LockFileEx` on Windows), so two writers of one serial
    /// in one directory (two processes, or two stores in one) take turns.
    /// Under the lock the record's id becomes the higher of its own and one
    /// past the newest id in the files, so ids never collide.
    pub(super) fn append(dir: &Path, record: &mut EventRecord, limits: &Limits) -> io::Result<()> {
        let serial = record.serial.clone();
        let lock = open_append(&lock_path(dir, &serial))?;
        lock.lock()?;
        let result = append_locked(dir, &serial, record, limits);
        let _ = lock.unlock();
        result
    }

    fn append_locked(
        dir: &Path,
        serial: &str,
        record: &mut EventRecord,
        limits: &Limits,
    ) -> io::Result<()> {
        let tail = (0..=limits.rotated_files.min(1))
            .find_map(|n| last_id(&path(dir, serial, n), limits))
            .unwrap_or(0);
        record.id = record.id.max(next_after(tail));
        let line = encode(record, limits.max_line_bytes);
        let current = path(dir, serial, 0);
        let size = fs::metadata(&current).map_or(0, |m| m.len());
        let len = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let mut torn = false;
        if size > 0 && size.saturating_add(len) > limits.max_file_bytes {
            rotate(dir, serial, limits.rotated_files)?;
        } else if size > 0 {
            // A crash, or another writer, may have left the end torn: the
            // record then starts on a line of its own. One byte to read.
            torn = ends_torn(&current);
        }
        let mut f = open_append(&current)?;
        let mut out = String::with_capacity(line.len().saturating_add(1));
        if torn {
            out.push('\n');
        }
        out.push_str(&line);
        f.write_all(out.as_bytes())
    }

    /// The newest valid id in a file's last few lines.
    fn last_id(p: &Path, limits: &Limits) -> Option<u64> {
        let cap = u64::try_from(limits.max_line_bytes)
            .unwrap_or(u64::MAX)
            .saturating_mul(2)
            .max(4096);
        let (text, _) = read_tail(p, cap)?;
        text.split('\n').rev().find_map(parse_line).map(|r| r.id)
    }

    /// Whether the file's last byte is not a line end.
    fn ends_torn(p: &Path) -> bool {
        let Ok(mut f) = fs::File::open(p) else {
            return false;
        };
        if f.seek(SeekFrom::End(-1)).is_err() {
            return false;
        }
        let mut b = [0u8; 1];
        f.read_exact(&mut b).is_ok() && b != *b"\n"
    }

    /// `.n` to `.n+1` from the oldest down, the oldest beyond `keep` removed,
    /// the current file to `.1`.
    fn rotate(dir: &Path, serial: &str, keep: u32) -> io::Result<()> {
        if keep == 0 {
            return fs::remove_file(path(dir, serial, 0));
        }
        let oldest = path(dir, serial, keep);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for n in (1..keep).rev() {
            let from = path(dir, serial, n);
            if from.exists() {
                fs::rename(&from, path(dir, serial, n.saturating_add(1)))?;
            }
        }
        fs::rename(path(dir, serial, 0), path(dir, serial, 1))
    }

    /// Every record in `serial`'s files, oldest file first, and how many lines
    /// were not records. Each file is read from its last `4 * max_file_bytes`
    /// at most, so a file grown out of bounds cannot fill memory.
    pub(super) fn read(dir: &Path, serial: &str, limits: &Limits) -> (Vec<EventRecord>, u32) {
        let mut out = Vec::new();
        let mut skipped = 0u32;
        let cap = limits.max_file_bytes.saturating_mul(4).max(4096);
        for n in (0..=limits.rotated_files).rev() {
            let Some((text, cut)) = read_tail(&path(dir, serial, n), cap) else {
                continue;
            };
            let mut lines = text.split('\n');
            if cut {
                // The first line was entered in the middle.
                lines.next();
            }
            for line in lines {
                if line.trim().is_empty() {
                    continue;
                }
                match parse_line(line) {
                    Some(r) => out.push(r),
                    None => skipped = skipped.saturating_add(1),
                }
            }
        }
        (out, skipped)
    }

    /// The file's last `cap` bytes as text, and whether it was cut.
    fn read_tail(p: &Path, cap: u64) -> Option<(String, bool)> {
        let mut f = fs::File::open(p).ok()?;
        let size = f.metadata().ok()?.len();
        let cut = size > cap;
        if cut {
            f.seek(SeekFrom::Start(size.saturating_sub(cap))).ok()?;
        }
        let mut bytes = Vec::new();
        f.take(cap).read_to_end(&mut bytes).ok()?;
        Some((String::from_utf8_lossy(&bytes).into_owned(), cut))
    }

    /// The serials with a current or rotated file in `dir`.
    pub(super) fn devices(dir: &Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let stem = name.strip_suffix(".jsonl")?;
                // A rotated file: `<stem>.<n>`; stems hold no dot.
                let stem = stem.split('.').next()?;
                serial_of(stem)
            })
            .collect()
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod files {
    //! The web has no files: the log stays in memory.

    use std::io;
    use std::path::Path;

    use super::{EventRecord, Limits};

    pub(super) fn create_dir(_: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub(super) fn append(_: &Path, _: &mut EventRecord, _: &Limits) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub(super) fn read(_: &Path, _: &str, _: &Limits) -> (Vec<EventRecord>, u32) {
        (Vec::new(), 0)
    }

    pub(super) fn devices(_: &Path) -> Vec<String> {
        Vec::new()
    }
}

// ── Watchers: what turns polled values into records ───────────────────

/// RP2040 and RP2350 temperature edges, °C: at or above [`TEMP_WARM_C`] is
/// warm, at or above [`TEMP_HOT_C`] hot. A band is left only
/// [`TEMP_HYSTERESIS_C`] below its edge, so a reading on an edge does not flap.
/// Both chips are specified to 85 °C ambient; the edges leave room before that.
pub const TEMP_WARM_C: f64 = 65.0;
/// See [`TEMP_WARM_C`].
pub const TEMP_HOT_C: f64 = 75.0;
/// See [`TEMP_WARM_C`].
pub const TEMP_HYSTERESIS_C: f64 = 3.0;

/// The band edges for one platform ([`TempEdges::for_platform`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempEdges {
    /// At or above: warm, °C.
    pub warm_c: f64,
    /// At or above: hot, °C.
    pub hot_c: f64,
    /// How far below an edge a band is left, °C.
    pub hysteresis_c: f64,
}

impl TempEdges {
    /// RP2040 and RP2350: 65 / 75 °C, 3 °C hysteresis.
    pub const RP2: Self = Self {
        warm_c: TEMP_WARM_C,
        hot_c: TEMP_HOT_C,
        hysteresis_c: TEMP_HYSTERESIS_C,
    };

    /// STM32H723: 80 / 95 °C, 3 °C hysteresis. The ordinary grade (suffix 6)
    /// is specified to a junction temperature of 105 °C (DS13313, general
    /// operating conditions; suffix 3 parts go higher), and an H7 die at full
    /// clock commonly sits at 55-70 °C, so the RP2 edges would read "warm" in
    /// normal use. Not yet checked against the current DS13313 revision.
    pub const STM32H723: Self = Self {
        warm_c: 80.0,
        hot_c: 95.0,
        hysteresis_c: 3.0,
    };

    /// The edges for `DspState.platform_id` (0 RP2040, 1 RP2350, 2
    /// STM32H723); an unknown platform gets the RP2 ones.
    pub fn for_platform(platform_id: u8) -> Self {
        match platform_id {
            2 => Self::STM32H723,
            _ => Self::RP2,
        }
    }

    fn band(&self, c: f64, below: f64) -> Band {
        if c >= self.hot_c - below {
            Band::Hot
        } else if c >= self.warm_c - below {
            Band::Warm
        } else {
            Band::Normal
        }
    }
}

impl Default for TempEdges {
    fn default() -> Self {
        Self::RP2
    }
}

/// A counter step is recorded at most this often per counter, ms; steps in
/// between are added to the next record's `count`.
pub const COUNTER_INTERVAL_MS: i64 = 30_000;

/// An uptime this much lower than the last one plus the time since (seconds)
/// is a restart. Covers clock drift and the poll's own delay.
pub const UPTIME_SLACK_S: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Band {
    Normal,
    Warm,
    Hot,
}

impl Band {
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warm => "warm",
            Self::Hot => "hot",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CounterSeen {
    value: u64,
    last_record: Option<i64>,
    unrecorded: u64,
}

/// One connection's view of the polled values, which turns their changes into
/// records. Start a new one per connection: the first reading of anything is
/// its baseline.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    edges: TempEdges,
    band: Option<Band>,
    counters: HashMap<String, CounterSeen>,
    /// The last uptime reading, against the monotonic clock.
    uptime: Option<(Instant, u32)>,
    hangs: Option<u8>,
}

impl Watch {
    /// A watch with nothing seen yet, with the RP2040/RP2350 temperature edges.
    pub fn new() -> Self {
        Self::default()
    }

    /// A watch for `DspState.platform_id`'s temperature edges.
    pub fn for_platform(platform_id: u8) -> Self {
        Self {
            edges: TempEdges::for_platform(platform_id),
            ..Self::default()
        }
    }

    /// A temperature reading in centi-°C (`REQ_GET_STATUS` 16) at `t`. A
    /// record when the band changed, or on the first reading when it is not
    /// normal.
    pub fn temperature(&mut self, centi_c: i32, t: i64) -> Option<EventRecord> {
        let c = f64::from(centi_c) / 100.0;
        let e = self.edges;
        let band = match self.band {
            None => e.band(c, 0.0),
            Some(old) => {
                let up = e.band(c, 0.0);
                if up > old {
                    up
                } else {
                    e.band(c, e.hysteresis_c).max(up).min(old)
                }
            }
        };
        let old = self.band.replace(band);
        if old == Some(band) || (old.is_none() && band == Band::Normal) {
            return None;
        }
        let mut r = EventRecord::new(
            EventKind::Temperature,
            t,
            format!("Temperature {c:.1} °C: {}", band.name()),
        );
        r.value = Some(c);
        r.source = Some(band.name().to_string());
        Some(r)
    }

    /// A cumulative counter's reading (`name` a stable key, `label` what the
    /// text calls it). A record when it rose, at most once per
    /// [`COUNTER_INTERVAL_MS`]; a counter that went down was reset with the
    /// device and becomes the new baseline.
    pub fn counter(&mut self, name: &str, label: &str, value: u64, t: i64) -> Option<EventRecord> {
        let Some(seen) = self.counters.get_mut(name) else {
            self.counters.insert(
                name.to_string(),
                CounterSeen {
                    value,
                    last_record: None,
                    unrecorded: 0,
                },
            );
            return None;
        };
        if value <= seen.value {
            if value < seen.value {
                // Reset with the device: steps from before do not carry over.
                seen.unrecorded = 0;
            }
            seen.value = value;
            return None;
        }
        let steps = value
            .saturating_sub(seen.value)
            .saturating_add(seen.unrecorded);
        seen.value = value;
        if seen
            .last_record
            .is_some_and(|last| t.saturating_sub(last) < COUNTER_INTERVAL_MS)
        {
            seen.unrecorded = steps;
            return None;
        }
        seen.unrecorded = 0;
        seen.last_record = Some(t);
        let mut r = EventRecord::new(
            EventKind::Counter,
            t,
            format!("{label} +{steps} (total {value})"),
        );
        r.source = Some(name.to_string());
        r.value = Some(value as f64);
        r.count = u32::try_from(steps).unwrap_or(u32::MAX);
        Some(r)
    }

    /// The device's uptime and hang count read at `now` (the monotonic clock,
    /// so a jump of the wall clock is no restart), recorded at `t` (ms since
    /// the epoch) (the core-voltage extension's status; FW5's readout later).
    /// A [`EventKind::Reset`] record when the uptime fell short of the last one
    /// plus the time since, or the hang count rose.
    pub fn uptime(
        &mut self,
        uptime_s: u32,
        hang_count: Option<u8>,
        now: Instant,
        t: i64,
    ) -> Option<EventRecord> {
        let restarted = self.uptime.is_some_and(|(then, was)| {
            let elapsed_s = now.saturating_duration_since(then).as_secs();
            fell_short(was, elapsed_s, uptime_s)
        });
        self.uptime = Some((now, uptime_s));
        let hung = match (self.hangs, hang_count) {
            (Some(old), Some(new)) => new > old,
            _ => false,
        };
        if hang_count.is_some() {
            self.hangs = hang_count;
        }
        if !restarted && !hung {
            return None;
        }
        let why = if hung { "watchdog (a hang)" } else { "unknown" };
        let mut r = EventRecord::new(
            EventKind::Reset,
            t,
            format!("The device restarted ({why}); up {uptime_s} s"),
        );
        r.source = Some(if hung { "hang" } else { "unknown" }.to_string());
        r.uptime_s = Some(uptime_s);
        Some(r)
    }
}

/// Whether an uptime of `now_s` at `t` means a restart since `last` (when,
/// uptime then), by the wall clock: across connections and runs, where only
/// the wall clock is shared. Within a connection [`Watch::uptime`] uses the
/// monotonic clock.
pub fn restarted(last: (i64, u32), now_s: u32, t: i64) -> bool {
    let (then, was) = last;
    let elapsed_s = u64::try_from(t.saturating_sub(then).max(0) / 1000).unwrap_or(0);
    fell_short(was, elapsed_s, now_s)
}

/// An uptime of `now_s` after `elapsed_s` from one of `was`: short by more
/// than [`UPTIME_SLACK_S`].
fn fell_short(was: u32, elapsed_s: u64, now_s: u32) -> bool {
    let expected = u64::from(was).saturating_add(elapsed_s);
    let slack = u64::try_from(UPTIME_SLACK_S).unwrap_or(0);
    u64::from(now_s).saturating_add(slack) < expected
}

// ── WP FWX's events ───────────────────────────────────────────────────

/// Why the device restarted, as FW5's readout will report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetReason {
    /// Power was applied.
    PowerOn,
    /// The watchdog fired.
    Watchdog,
    /// Software asked for it (a reboot command, the bootloader).
    Software,
    /// The supply dipped.
    Brownout,
    /// Something the chip does not say.
    Unknown,
}

impl ResetReason {
    /// The name used as a record's `source`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PowerOn => "power_on",
            Self::Watchdog => "watchdog",
            Self::Software => "software",
            Self::Brownout => "brownout",
            Self::Unknown => "unknown",
        }
    }
}

/// An event from the extension channel's notifications (WP FWX, PRO1 FW1 and
/// FW5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtEvent {
    /// Clip flags set: one bit per wire channel, as `clip_flags` (an event
    /// may carry several channels at once).
    Clip {
        /// The channels.
        mask: u32,
    },
    /// The device restarted.
    Reset {
        /// Why.
        reason: ResetReason,
    },
    // -- WP SW1 --
    /// The audio link's state changed (0xE1, `EXT_NOTIFY_FEATURES` bit 2).
    AlinkState(dspi_proto::ext::alink::AlinkStateEvent),
    /// An apply-at-frame op ran (0xE2, bit 3).
    ApplyResult(dspi_proto::ext::alink::ApplyResultEvent),
    // -- WP AUSB1 --
    /// A USB-direct state change, gap or re-anchor (0xE3, bit 4).
    AlinkUsb(dspi_proto::ext::alink_usb::AlinkUsbEvent),
}

/// Decode a notification the session could not (`Event::Unknown { id,
/// bytes }`) as an extension event, given the device's `EXT_NOTIFY_FEATURES`
/// flags (WP FWX, wired by WP CS1): a clip event (0xE0) of a device that
/// announces them (bit 1) joins the meters' clip hold (and so the log's
/// de-duplication). One event may name several channels: `mask` has them
/// all. `None` for anything else, a device without the bit included.
///
/// FW5 sends no reset event: the reason comes from the `EXT_RESET_INFO`
/// readout at connect, so [`ExtEvent::Reset`] is not decoded from a
/// notification yet.
///
/// WP SW1: the audio link's 0xE1 and 0xE2, each behind its own feature bit
/// (`dspi_proto::ext::alink::alink_event`).
pub fn decode_ext_event(notify_features: Option<u32>, id: u8, bytes: &[u8]) -> Option<ExtEvent> {
    use dspi_proto::ext::alink::{AlinkEvent, alink_event};
    if let Some(c) = dspi_proto::ext::clip_event(notify_features, id, bytes) {
        return Some(ExtEvent::Clip { mask: c.mask });
    }
    // -- WP AUSB1 --
    if let Some(e) = dspi_proto::ext::alink_usb::alink_usb_event(notify_features, id, bytes) {
        return Some(ExtEvent::AlinkUsb(e));
    }
    // -- WP SW1 --
    alink_event(notify_features, id, bytes).map(|e| match e {
        AlinkEvent::State(s) => ExtEvent::AlinkState(s),
        AlinkEvent::Apply(a) => ExtEvent::ApplyResult(a),
    })
}

// -- WP SW1 --
/// The event-log record for an audio-link event at `t`: a loss, a lock, a
/// re-anchor (resync) or an apply-at-frame result, kind [`EventKind::Alink`].
/// `None` for the other extension events.
pub fn alink_record(e: &ExtEvent, t: i64) -> Option<EventRecord> {
    use dspi_proto::ext::alink::{AlinkEventReason, AlinkState, ApplyResult};
    let (source, text, frame) = match e {
        ExtEvent::AlinkState(s) => {
            let class = match s.class {
                1..=3 => format!(", L{}", s.class),
                _ => String::new(),
            };
            let (source, what) = match (s.reason, s.state) {
                (AlinkEventReason::Los, _) => ("lost", "lost (LOS): outputs muted"),
                (AlinkEventReason::HeaderErrors, _) => {
                    ("lost", "lost (bad headers): outputs muted")
                }
                (AlinkEventReason::CommaLost, _) => {
                    ("lost", "lost (no frame start): outputs muted")
                }
                (AlinkEventReason::Reanchor, _) => ("resync", "re-anchored (a short silence)"),
                (AlinkEventReason::VcxoLimit, _) => ("resync", "out of clock range: re-anchored"),
                (AlinkEventReason::Disabled, _) => ("off", "switched off"),
                (_, AlinkState::Locked) => ("locked", "locked"),
                (_, AlinkState::Holdover) => ("lost", "lost: outputs muted"),
                (_, AlinkState::Searching) => ("searching", "searching"),
                (_, AlinkState::Off) => ("off", "off"),
                (_, AlinkState::Other(_)) => ("unknown", "changed state"),
            };
            (source, format!("Audio link {what}{class}"), s.frame_low)
        }
        ExtEvent::ApplyResult(a) => {
            let how = match a.result {
                ApplyResult::Applied => "applied",
                ApplyResult::Late => "applied late (its frame had passed)",
                ApplyResult::Cancelled => "cancelled",
                ApplyResult::Refused => "refused",
                ApplyResult::Other(_) => "ended",
            };
            (
                "apply",
                format!("Apply at frame: {} {how}", a.op.as_str()),
                a.frame_low,
            )
        }
        // -- WP AUSB1 --
        ExtEvent::AlinkUsb(u) => {
            use dspi_proto::ext::alink_usb::{UsbEventReason as R, UsbState as S};
            let (source, what) = match (u.reason, u.state) {
                (R::SofLost, _) => ("lost", "lost its USB time base: anchoring again"),
                (R::HostRange, S::Rejected) => ("lost", "rejected the host (beyond 295 ppm)"),
                (R::HostRange, _) => ("locked", "host back in range"),
                (R::Reanchor, _) => ("resync", "re-anchored (a short silence)"),
                (R::Gap, _) => ("gap", "missed USB packets (a faded gap)"),
                (R::Disabled, _) => ("off", "switched off"),
                (R::Config, _) => ("config", "cannot phase-lock: running rate-locked"),
                (R::Holdover, _) => ("holdover", "no SOF: holding its timeline"),
                (R::Resumed, _) => ("locked", "SOF back: same timeline"),
                (_, S::PhaseLocked) => ("locked", "phase-locked"),
                (_, S::RateLocked) => ("locked", "rate-locked"),
                (_, S::PullIn) => ("searching", "pulling in to the host"),
                (_, S::Anchoring) => ("searching", "anchoring"),
                (_, S::NoSof) => ("lost", "no SOF"),
                (_, S::Off) => ("off", "off"),
                (_, _) => ("unknown", "changed state"),
            };
            (
                source,
                format!("Audio link USB-direct {what}"),
                u.sample_low,
            )
        }
        ExtEvent::Clip { .. } | ExtEvent::Reset { .. } => return None,
    };
    let mut r = EventRecord::new(EventKind::Alink, t, text);
    r.source = Some(source.to_string());
    r.value = Some(f64::from(frame));
    Some(r)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0m;

// -- WP P2EVW --
/// The multi-device writer (the hub's shared log); files only, so not on the
/// web.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub mod store;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use store::{EventStore, EventStoreError, HUB_LIMITS, HUB_TOTAL_BYTES};
