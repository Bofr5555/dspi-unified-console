//! The hub's inventory (WP P2IN, PLAN-P2 F1): every DSPi the hub has ever seen,
//! online or not, kept in `<data>/inventory.json`.
//!
//! `device.list` (spec 7.4: handles, a 60 s grace) is unchanged (OD-P2l); the
//! inventory is its own `inventory.*` family, keyed by serial, and outlives a replug
//! and a restart of the hub.
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `inventory.list {serial? or handle?, offset?, limit?}` | every role | `ok {devices, total, next_offset}` |
//! | `inventory.set {serial or handle, location?, note?, tags?}` | Admin | `ok {device}` |
//! | `inventory.forget {serial or handle, with_data?}` | Admin | `ok {serial, forgot}`; `conflict` while online |
//!
//! Each change sends `inventory.changed {v, hub, serial, device}` to every session
//! (`device` `null` when the record went). A firmware change also puts a `firmware`
//! record on the bus ([`BusMessage::Event`], with id 0: not yet stored; the history
//! writer stores it).
//!
//! **Pure state, thin glue.** [`Book`] holds the records and is driven by the caller
//! with the time; [`InventoryService`] feeds it from the registry (on every device
//! event) and from `Facts` on the bus, saves the file and sends the events.
//!
//! **Bounded.** 1024 records ([`MAX_DEVICES`]): past it the offline record seen
//! longest ago goes, with a log line. A location and a note are at most 256
//! characters, tags at most 16 of 32. A page of `inventory.list` is at most 500
//! records and about 512 KiB. The file is read entry by entry: one written whole is
//! one JSON object; a torn or corrupt one is kept aside as `inventory.json.bad-<time>`
//! and the records that can still be read (one per line, as the hub writes them) are
//! kept, with a line in the log. Nothing here panics on the file or on a message.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_proto::ext::{IdentifyStatus, ResetInfo};
use dspi_session::events::{EventKind, EventRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bus::{Bus, BusMessage, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::registry::{DeviceState, RegisteredDevice};
use crate::service::{DeviceEvent, HubService, ServiceConfig, ServiceSetup, Waker};
use crate::session::{Role, Session};

/// The file in the hub's data folder.
pub const FILE_NAME: &str = "inventory.json";
/// The cap `hello.caps` announces.
pub const CAP: &str = "inventory";
/// The event a change sends.
pub const CHANGED_EVENT: &str = "inventory.changed";
/// The messages this family answers.
pub const MESSAGES: [&str; 3] = ["inventory.list", "inventory.set", "inventory.forget"];
/// The most records kept (review S3, OD-P2k).
pub const MAX_DEVICES: usize = 1024;
/// The longest location and note, in characters.
pub const MAX_TEXT_CHARS: usize = 256;
/// The most tags on a record.
pub const MAX_TAGS: usize = 16;
/// The longest tag, in characters.
pub const MAX_TAG_CHARS: usize = 32;
/// The longest serial taken, in bytes (the event store's limit).
pub const MAX_SERIAL_BYTES: usize = 64;
/// A page's default and largest row count (hub-api.md 1.8).
pub const DEFAULT_PAGE: usize = 100;
/// The largest page.
pub const MAX_PAGE: usize = 500;
/// A page stops before its records pass this many bytes of JSON.
const MAX_PAGE_BYTES: usize = 512 * 1024;
/// The largest file read; a larger one is kept aside.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Device-reported strings (name, board, ...) are cut to this many characters.
const MAX_DEVICE_TEXT_CHARS: usize = 64;
/// How often the service reads the bus when nothing woke it.
const TICK: Duration = Duration::from_secs(1);
/// Two reset times further apart than this are two resets.
const RESET_SLACK_S: u64 = 5;
/// The bus queue the service reads.
const BUS_CAPACITY: usize = 1024;
/// The answers' version.
const VERSION: u32 = 1;
/// The file format's version.
const FILE_VERSION: u32 = 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under this lock leaves the book whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Records ─────────────────────────────────────────────────────────────────

/// The last reset the device reported (`EXT_RESET_INFO`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastReset {
    /// `power_on`, `brownout`, `watchdog`, ... ([`dspi_proto::ext::ResetReason::as_str`]).
    pub reason: String,
    /// How long the run before it lasted, seconds, where the device says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_uptime_s: Option<u32>,
    /// When it happened, Unix seconds (the reading's time less the uptime).
    #[serde(default)]
    pub at_s: u64,
}

/// One device of the inventory, as the file keeps it. Times are Unix seconds; the
/// messages show them as ISO-8601 ([`Record::to_json`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The key.
    pub serial: String,
    /// The name `device.list` gave it when last seen.
    #[serde(default)]
    pub name: String,
    /// 0 RP2040, 1 RP2350, 2 STM32.
    #[serde(default)]
    pub platform: u8,
    /// The firmware version, without the beta.
    #[serde(default)]
    pub fw: String,
    /// The beta, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fw_beta: Option<u8>,
    /// Output channels.
    #[serde(default)]
    pub outputs: u8,
    /// `EXT_IDENTIFY`'s `PICO_BOARD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    /// `EXT_IDENTIFY`'s `DSPI_HW_REV`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hw_rev: Option<String>,
    /// `EXT_IDENTIFY`'s chip (`RP2350`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<String>,
    /// `EXT_IDENTIFY`'s chip revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip_rev: Option<u8>,
    /// Set by an admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Set by an admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Set by an admin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// First seen by this hub.
    #[serde(default)]
    pub first_seen_s: u64,
    /// Last seen: when it came online or went offline.
    #[serde(default)]
    pub last_seen_s: u64,
    /// The last reset it reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reset: Option<LastReset>,
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn cut_opt(text: Option<String>, chars: usize) -> Option<String> {
    text.map(|t| cut(&t, chars))
}

/// A serial a message or the file may name: 1 to 64 bytes, no control characters.
pub fn valid_serial(serial: &str) -> bool {
    !serial.is_empty() && serial.len() <= MAX_SERIAL_BYTES && !serial.chars().any(char::is_control)
}

impl Record {
    fn new(serial: &str, now_s: u64) -> Self {
        Self {
            serial: serial.to_owned(),
            name: String::new(),
            platform: 0,
            fw: String::new(),
            fw_beta: None,
            outputs: 0,
            board: None,
            hw_rev: None,
            chip: None,
            chip_rev: None,
            location: None,
            note: None,
            tags: Vec::new(),
            first_seen_s: now_s,
            last_seen_s: now_s,
            last_reset: None,
        }
    }

    /// A record read from the file, made to fit the bounds; `None` for one without a
    /// serial the hub takes.
    fn sanitized(mut self) -> Option<Self> {
        if !valid_serial(&self.serial) {
            return None;
        }
        self.name = cut(&self.name, MAX_DEVICE_TEXT_CHARS);
        self.fw = cut(&self.fw, MAX_DEVICE_TEXT_CHARS);
        self.board = cut_opt(self.board, MAX_DEVICE_TEXT_CHARS);
        self.hw_rev = cut_opt(self.hw_rev, MAX_DEVICE_TEXT_CHARS);
        self.chip = cut_opt(self.chip, MAX_DEVICE_TEXT_CHARS);
        self.location = cut_opt(self.location, MAX_TEXT_CHARS);
        self.note = cut_opt(self.note, MAX_TEXT_CHARS);
        self.tags = self
            .tags
            .into_iter()
            .map(|t| cut(&t, MAX_TAG_CHARS))
            .filter(|t| !t.is_empty())
            .take(MAX_TAGS)
            .collect();
        if let Some(r) = self.last_reset.as_mut() {
            r.reason = cut(&r.reason, MAX_DEVICE_TEXT_CHARS);
        }
        self.last_seen_s = self.last_seen_s.max(self.first_seen_s);
        Some(self)
    }

    /// The record as the messages show it (hub-api.md section 3): with `handle` and
    /// `online` from now, and `last_seen` now while online.
    pub fn to_json(&self, handle: Option<u8>, now_s: u64) -> Value {
        let last_seen = if handle.is_some() {
            now_s.max(self.last_seen_s)
        } else {
            self.last_seen_s
        };
        let mut v = json!({
            "serial": self.serial,
            "handle": handle,
            "online": handle.is_some(),
            "name": self.name,
            "platform": self.platform,
            "fw": self.fw,
            "outputs": self.outputs,
            "location": self.location,
            "note": self.note,
            "tags": self.tags,
            "first_seen": iso8601(self.first_seen_s),
            "last_seen": iso8601(last_seen),
        });
        if let Some(map) = v.as_object_mut() {
            if let Some(beta) = self.fw_beta {
                map.insert("fw_beta".into(), beta.into());
            }
            for (key, val) in [
                ("board", &self.board),
                ("hw_rev", &self.hw_rev),
                ("chip", &self.chip),
            ] {
                if let Some(val) = val {
                    map.insert(key.into(), val.clone().into());
                }
            }
            if let Some(rev) = self.chip_rev {
                map.insert("chip_rev".into(), rev.into());
            }
            if let Some(r) = &self.last_reset {
                let mut reset = json!({"reason": r.reason, "at": iso8601(r.at_s)});
                if let (Some(prev), Some(m)) = (r.prev_uptime_s, reset.as_object_mut()) {
                    m.insert("prev_uptime_s".into(), prev.into());
                }
                map.insert("last_reset".into(), reset);
            }
        }
        v
    }
}

/// What the registry says of an online device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// Its serial.
    pub serial: String,
    /// Its handle.
    pub handle: u8,
    /// The name clients show.
    pub name: String,
    /// 0 RP2040, 1 RP2350, 2 STM32.
    pub platform: u8,
    /// The firmware, without the beta.
    pub fw: String,
    /// The beta, when known.
    pub fw_beta: Option<u8>,
    /// Output channels.
    pub outputs: u8,
}

impl From<&RegisteredDevice> for Seen {
    fn from(d: &RegisteredDevice) -> Self {
        Self {
            serial: d.identity.serial.clone(),
            handle: d.handle,
            name: d.name.clone(),
            platform: d.identity.platform,
            fw: d.identity.firmware.clone(),
            fw_beta: d.identity.firmware_beta,
            outputs: d.identity.outputs,
        }
    }
}

/// What an admin changes with `inventory.set`: each `Some` replaces its field
/// (`Some(None)` clears it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edit {
    /// The location.
    pub location: Option<Option<String>>,
    /// The note.
    pub note: Option<Option<String>>,
    /// The tags.
    pub tags: Option<Vec<String>>,
}

/// Why the book refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InventoryError {
    /// A malformed request or a value past a bound.
    #[error("{0}")]
    BadRequest(String),
    /// A serial the inventory does not hold.
    #[error("no device {0} in the inventory")]
    NotFound(String),
    /// The device is attached (forget).
    #[error("{0} is online")]
    Online(String),
    /// The file system said no.
    #[error("{0}")]
    Io(String),
}

impl InventoryError {
    /// The error code of hub-api.md 1.6.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::Online(_) => "conflict",
            Self::Io(_) => "internal",
        }
    }
}

/// One change to tell the sessions about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A record is new or changed.
    Updated(String),
    /// A record went (forgotten, or dropped past the cap).
    Removed(String),
    /// The firmware differs from the one last seen.
    Firmware {
        /// The device.
        serial: String,
        /// What it ran.
        old: String,
        /// What it runs now.
        new: String,
    },
}

/// The records and which of them are online. Pure: the caller passes the time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    records: BTreeMap<String, Record>,
    online: BTreeMap<String, u8>,
    cap: usize,
}

impl Book {
    /// An empty book holding up to [`MAX_DEVICES`].
    pub fn new() -> Self {
        Self::with_cap(MAX_DEVICES)
    }

    /// An empty book holding up to `cap` records (at least 1).
    pub fn with_cap(cap: usize) -> Self {
        Self {
            records: BTreeMap::new(),
            online: BTreeMap::new(),
            cap: cap.max(1),
        }
    }

    /// How many records it holds.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// One record.
    pub fn get(&self, serial: &str) -> Option<&Record> {
        self.records.get(serial)
    }

    /// The handle of an online device.
    pub fn handle(&self, serial: &str) -> Option<u8> {
        self.online.get(serial).copied()
    }

    /// A record as the messages show it.
    pub fn json(&self, serial: &str, now_s: u64) -> Option<Value> {
        self.records
            .get(serial)
            .map(|r| r.to_json(self.handle(serial), now_s))
    }

    /// Every record: online first, then the one seen last first, then by serial.
    pub fn ordered(&self) -> Vec<&Record> {
        let mut all: Vec<&Record> = self.records.values().collect();
        all.sort_by(|a, b| {
            let on = |r: &Record| self.online.contains_key(&r.serial);
            on(b)
                .cmp(&on(a))
                .then(b.last_seen_s.cmp(&a.last_seen_s))
                .then(a.serial.cmp(&b.serial))
        });
        all
    }

    /// The registry's view now: `online` is every device attached. Devices the book
    /// had online and the registry no longer has are offline from `now_s`.
    pub fn reconcile(&mut self, online: &[Seen], now_s: u64) -> Vec<Change> {
        let mut changes = Vec::new();
        let gone: Vec<String> = self
            .online
            .keys()
            .filter(|s| !online.iter().any(|d| &d.serial == *s))
            .cloned()
            .collect();
        for serial in gone {
            changes.extend(self.offline(&serial, now_s));
        }
        for d in online {
            changes.extend(self.seen(d, now_s));
        }
        changes
    }

    /// A device is online (attached now, or still). Updates what the registry says of
    /// it; `last_seen` moves when it comes online.
    pub fn seen(&mut self, d: &Seen, now_s: u64) -> Vec<Change> {
        if !valid_serial(&d.serial) {
            return Vec::new();
        }
        let mut changes = Vec::new();
        if !self.records.contains_key(&d.serial) {
            changes.extend(self.make_room(&d.serial));
            self.records
                .insert(d.serial.clone(), Record::new(&d.serial, now_s));
        }
        let came_online = self.online.insert(d.serial.clone(), d.handle) != Some(d.handle);
        let Some(r) = self.records.get_mut(&d.serial) else {
            return changes;
        };
        let before = r.clone();
        let fw = cut(&d.fw, MAX_DEVICE_TEXT_CHARS);
        if !r.fw.is_empty() && r.fw != fw {
            changes.push(Change::Firmware {
                serial: d.serial.clone(),
                old: r.fw.clone(),
                new: fw.clone(),
            });
        }
        r.name = cut(&d.name, MAX_DEVICE_TEXT_CHARS);
        r.platform = d.platform;
        r.fw = fw;
        r.fw_beta = d.fw_beta;
        r.outputs = d.outputs;
        if came_online {
            r.last_seen_s = now_s.max(r.first_seen_s);
        }
        if came_online || *r != before {
            changes.push(Change::Updated(d.serial.clone()));
        }
        changes
    }

    /// A device went offline.
    pub fn offline(&mut self, serial: &str, now_s: u64) -> Vec<Change> {
        if self.online.remove(serial).is_none() {
            return Vec::new();
        }
        if let Some(r) = self.records.get_mut(serial) {
            r.last_seen_s = now_s.max(r.last_seen_s);
        }
        vec![Change::Updated(serial.to_owned())]
    }

    /// What the hub learnt of a device (`Facts`): identify and reset info where the
    /// firmware lists them, and its firmware version. A device not in the book is
    /// ignored (it enters with [`Book::seen`]).
    pub fn facts(
        &mut self,
        serial: &str,
        identify: Option<&IdentifyStatus>,
        reset: Option<&ResetInfo>,
        fw: &str,
        now_s: u64,
    ) -> Vec<Change> {
        let Some(r) = self.records.get_mut(serial) else {
            return Vec::new();
        };
        let before = r.clone();
        let mut changes = Vec::new();
        if let Some(id) = identify {
            r.board = Some(cut(&id.board, MAX_DEVICE_TEXT_CHARS)).filter(|b| !b.is_empty());
            r.hw_rev = Some(cut(&id.hw_rev, MAX_DEVICE_TEXT_CHARS)).filter(|h| !h.is_empty());
            r.chip = Some(id.chip.label());
            r.chip_rev = Some(id.chip_rev);
        }
        if let Some(info) = reset {
            let at_s = now_s.saturating_sub(u64::from(info.uptime_s));
            let reason = info.reason.as_str().to_owned();
            let same = r.last_reset.as_ref().is_some_and(|old| {
                old.reason == reason && old.at_s.abs_diff(at_s) <= RESET_SLACK_S
            });
            if !same {
                r.last_reset = Some(LastReset {
                    reason,
                    prev_uptime_s: info.prev_uptime_s,
                    at_s,
                });
            }
        }
        let fw = cut(fw, MAX_DEVICE_TEXT_CHARS);
        if !fw.is_empty() && r.fw != fw {
            if !r.fw.is_empty() {
                changes.push(Change::Firmware {
                    serial: serial.to_owned(),
                    old: r.fw.clone(),
                    new: fw.clone(),
                });
            }
            r.fw = fw;
        }
        if *r != before {
            changes.push(Change::Updated(serial.to_owned()));
        }
        changes
    }

    /// An admin's edit. Validated whole before anything changes.
    pub fn set(&mut self, serial: &str, edit: Edit) -> Result<Vec<Change>, InventoryError> {
        let check_text = |what: &str, t: &Option<Option<String>>| match t {
            Some(Some(text)) if text.chars().count() > MAX_TEXT_CHARS => Err(
                InventoryError::BadRequest(format!("{what} is over {MAX_TEXT_CHARS} characters")),
            ),
            Some(Some(text))
                if text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t') =>
            {
                Err(InventoryError::BadRequest(format!(
                    "{what} has a control character"
                )))
            }
            _ => Ok(()),
        };
        check_text("location", &edit.location)?;
        check_text("note", &edit.note)?;
        let tags = match edit.tags {
            None => None,
            Some(tags) => {
                if tags.len() > MAX_TAGS {
                    return Err(InventoryError::BadRequest(format!(
                        "at most {MAX_TAGS} tags"
                    )));
                }
                let mut out: Vec<String> = Vec::new();
                for t in tags {
                    let t = t.trim().to_owned();
                    if t.is_empty() || t.chars().count() > MAX_TAG_CHARS {
                        return Err(InventoryError::BadRequest(format!(
                            "a tag is 1 to {MAX_TAG_CHARS} characters"
                        )));
                    }
                    if t.chars().any(char::is_control) {
                        return Err(InventoryError::BadRequest(
                            "a tag has a control character".into(),
                        ));
                    }
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
                Some(out)
            }
        };
        let r = self
            .records
            .get_mut(serial)
            .ok_or_else(|| InventoryError::NotFound(serial.to_owned()))?;
        let before = r.clone();
        if let Some(location) = edit.location {
            r.location = location.filter(|l| !l.is_empty());
        }
        if let Some(note) = edit.note {
            r.note = note.filter(|n| !n.is_empty());
        }
        if let Some(tags) = tags {
            r.tags = tags;
        }
        Ok(if *r == before {
            Vec::new()
        } else {
            vec![Change::Updated(serial.to_owned())]
        })
    }

    /// Whether `serial` may be forgotten now: held, and offline.
    pub fn check_forget(&self, serial: &str) -> Result<(), InventoryError> {
        if !self.records.contains_key(serial) {
            return Err(InventoryError::NotFound(serial.to_owned()));
        }
        if self.online.contains_key(serial) {
            return Err(InventoryError::Online(serial.to_owned()));
        }
        Ok(())
    }

    /// Forget an offline device's record.
    pub fn forget(&mut self, serial: &str) -> Result<Record, InventoryError> {
        self.check_forget(serial)?;
        self.records
            .remove(serial)
            .ok_or_else(|| InventoryError::NotFound(serial.to_owned()))
    }

    /// Make room for `incoming`: drop the offline records seen longest ago.
    fn make_room(&mut self, incoming: &str) -> Vec<Change> {
        let mut changes = Vec::new();
        while self.records.len() >= self.cap {
            let oldest = self
                .records
                .values()
                .filter(|r| !self.online.contains_key(&r.serial) && r.serial != incoming)
                .min_by(|a, b| {
                    a.last_seen_s
                        .cmp(&b.last_seen_s)
                        .then(a.serial.cmp(&b.serial))
                })
                .map(|r| r.serial.clone());
            let Some(oldest) = oldest else { break };
            self.records.remove(&oldest);
            log::warn!(
                "hub: inventory full ({} devices): dropped {oldest}, offline the longest",
                self.cap
            );
            changes.push(Change::Removed(oldest));
        }
        changes
    }

    /// Stopping: every online device was last seen now. Nothing goes offline.
    pub fn touch_online(&mut self, now_s: u64) {
        for serial in self.online.keys() {
            if let Some(r) = self.records.get_mut(serial) {
                r.last_seen_s = now_s.max(r.last_seen_s);
            }
        }
    }

    // ── The file ────────────────────────────────────────────────────────────

    /// The file's text: one JSON object, one record per line, so a torn file still
    /// reads line by line.
    pub fn to_file(&self) -> String {
        let mut out = format!("{{\"v\":{FILE_VERSION},\"devices\":[\n");
        let mut first = true;
        for r in self.records.values() {
            let Ok(line) = serde_json::to_string(r) else {
                continue;
            };
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str(&line);
        }
        out.push_str("\n]}\n");
        out
    }

    /// The book from the file's text, and whether all of it read. A whole file reads
    /// as one object; otherwise each line that is a record is read on its own. Past
    /// the cap the records seen last are kept.
    pub fn from_file(text: &str, cap: usize) -> (Self, bool) {
        let mut book = Self::with_cap(cap);
        let mut whole = false;
        let mut records: Vec<Record> = Vec::new();
        if let Ok(v) = serde_json::from_str::<Value>(text)
            && let Some(devices) = v.get("devices").and_then(Value::as_array)
        {
            whole = true;
            for d in devices {
                match serde_json::from_value::<Record>(d.clone())
                    .ok()
                    .and_then(Record::sanitized)
                {
                    Some(r) => records.push(r),
                    None => whole = false,
                }
            }
        } else {
            for line in text.lines() {
                let line = line.trim().trim_end_matches(',');
                if !line.starts_with('{') || line.starts_with("{\"v\"") {
                    continue;
                }
                if let Some(r) = serde_json::from_str::<Record>(line)
                    .ok()
                    .and_then(Record::sanitized)
                {
                    records.push(r);
                }
            }
        }
        records.sort_by_key(|r| std::cmp::Reverse(r.last_seen_s));
        for r in records {
            if book.records.len() >= book.cap {
                whole = false;
                break;
            }
            book.records.entry(r.serial.clone()).or_insert(r);
        }
        (book, whole)
    }
}

// ── Times ───────────────────────────────────────────────────────────────────

/// ISO-8601 UTC with a `Z` (spec 7.1), from seconds since the epoch.
pub fn iso8601(secs: u64) -> String {
    // Howard Hinnant's civil_from_days (as `server.rs`). Every step is within i64 for
    // any u64 of seconds divided down to days, and the divisors are constants.
    #![allow(clippy::arithmetic_side_effects)]
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

// ── Files ───────────────────────────────────────────────────────────────────

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

/// The book from `path`: empty when there is none; a file that does not read whole
/// is kept aside as `inventory.json.bad-<time>` and what could be read is kept.
/// Returns the book and whether it must be written again.
fn load(path: &Path, now_s: u64) -> (Book, bool) {
    let mut bytes = Vec::new();
    let read = fs::File::open(path).and_then(|f| {
        f.take(MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
    });
    match read {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (Book::new(), false),
        Err(e) => {
            log::error!(
                "hub: inventory {} unread ({e}); starting empty",
                path.display()
            );
            set_aside(path, now_s);
            return (Book::new(), true);
        }
    }
    let too_large = u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES;
    let (book, whole) = if too_large {
        (Book::new(), false)
    } else {
        Book::from_file(&String::from_utf8_lossy(&bytes), MAX_DEVICES)
    };
    if whole {
        return (book, false);
    }
    log::error!(
        "hub: inventory {} did not read whole; kept aside, {} device(s) recovered",
        path.display(),
        book.len()
    );
    set_aside(path, now_s);
    (book, true)
}

fn set_aside(path: &Path, now_s: u64) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".bad-{now_s}"));
    if let Err(e) = fs::rename(path, PathBuf::from(&aside)) {
        log::error!("hub: cannot keep {} aside: {e}", path.display());
    }
}

// ── The service ─────────────────────────────────────────────────────────────

struct State {
    book: Book,
    /// The registry must be read again (a device came or went, or the bus dropped).
    resync: bool,
    /// The file is behind the book.
    dirty: bool,
    /// When the bus is read next without a wake.
    next_tick: Instant,
}

/// The `inventory.*` family as a hub service.
pub struct InventoryService {
    state: Mutex<State>,
    sub: Subscription,
    bus: Arc<Bus>,
    clock: Arc<dyn HubClock>,
    wake: Waker,
    data_dir: Option<PathBuf>,
}

/// The service, if the configuration asks for it (the one `registered` line).
pub(crate) fn service(
    config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    if !config.inventory {
        return None;
    }
    Some(Arc::new(InventoryService::new(setup)) as Arc<dyn HubService>)
}

/// What a request names: a serial, by itself or through a handle.
fn address(
    hub: &HubHandle,
    msg: &Value,
    required: bool,
) -> Result<Option<String>, (&'static str, String)> {
    let serial = match msg.get("serial") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if valid_serial(s) => Some(s.clone()),
        Some(_) => return Err(("bad_request", "serial is not a serial".into())),
    };
    let by_handle = match msg.get("handle") {
        None | Some(Value::Null) => None,
        Some(h) => {
            let Some(handle) = h.as_u64().and_then(|h| u8::try_from(h).ok()) else {
                return Err(("bad_request", "handle is not a handle".into()));
            };
            let Some(d) = hub.devices().into_iter().find(|d| d.handle == handle) else {
                return Err(("no_device", format!("no device at handle {handle}")));
            };
            Some(d.identity.serial)
        }
    };
    match (serial, by_handle) {
        (Some(s), Some(h)) if s != h => Err((
            "bad_request",
            "serial and handle name different devices".into(),
        )),
        (Some(s), _) | (None, Some(s)) => Ok(Some(s)),
        (None, None) if required => Err(("bad_request", "no serial".into())),
        (None, None) => Ok(None),
    }
}

fn reply_err(id: &Value, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

fn reply_ok(id: &Value, body: Value) -> String {
    let mut v = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (v.as_object_mut(), body) {
        map.extend(body);
    }
    v.to_string()
}

/// `Some(None)` for `null`, `Some(Some(text))` for a string, `None` if absent.
fn text_field(msg: &Value, key: &str) -> Result<Option<Option<String>>, String> {
    match msg.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(format!("{key} is not text")),
    }
}

fn tags_field(msg: &Value) -> Result<Option<Vec<String>>, String> {
    match msg.get("tags") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(Vec::new())),
        Some(Value::Array(items)) => {
            if items.len() > MAX_TAGS {
                return Err(format!("at most {MAX_TAGS} tags"));
            }
            items
                .iter()
                .map(|t| t.as_str().map(str::to_owned).ok_or("a tag is not text"))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
                .map_err(str::to_owned)
        }
        Some(_) => Err("tags is not a list".into()),
    }
}

fn page_field(msg: &Value, key: &str, default: usize, max: usize) -> Result<usize, String> {
    match msg.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= max)
            .ok_or_else(|| format!("{key} is a whole number up to {max}")),
    }
}

impl InventoryService {
    /// The service: its book from `<data>/inventory.json` (none in process), and its
    /// bus subscription.
    pub fn new(setup: &ServiceSetup<'_>) -> Self {
        let clock = Arc::clone(setup.clock);
        let data_dir = setup.data_dir.map(Path::to_path_buf);
        let (book, dirty) = match &data_dir {
            Some(dir) => load(&dir.join(FILE_NAME), clock.unix_seconds()),
            None => (Book::new(), false),
        };
        Self {
            state: Mutex::new(State {
                book,
                resync: true,
                dirty,
                next_tick: clock.now(),
            }),
            sub: setup.bus.subscribe("inventory", BUS_CAPACITY),
            bus: Arc::clone(setup.bus),
            clock,
            wake: setup.wake.clone(),
            data_dir,
        }
    }

    fn file(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join(FILE_NAME))
    }

    /// Catch up with the registry and the bus, save, and tell everyone. With no hub
    /// lock held.
    fn process(&self, hub: &HubHandle) {
        let now_s = self.clock.unix_seconds();
        let messages = self.sub.drain();
        let dropped = self.sub.take_dropped() > 0;
        let mut changes = Vec::new();
        let (events, firmware) = {
            let mut st = lock(&self.state);
            st.next_tick = self.clock.now().checked_add(TICK).unwrap_or(st.next_tick);
            let device_moved = messages.iter().any(|m| {
                matches!(
                    **m,
                    BusMessage::DeviceOnline { .. }
                        | BusMessage::DeviceOffline { .. }
                        | BusMessage::DeviceRemoved { .. }
                )
            });
            if st.resync || dropped || device_moved {
                st.resync = false;
                let online: Vec<Seen> = hub
                    .devices()
                    .iter()
                    .filter(|d| d.state == DeviceState::Online)
                    .map(Seen::from)
                    .collect();
                changes.extend(st.book.reconcile(&online, now_s));
            }
            for m in &messages {
                if let BusMessage::Facts {
                    serial,
                    identify,
                    reset_info,
                    fw,
                } = &**m
                {
                    changes.extend(st.book.facts(
                        serial,
                        identify.as_ref(),
                        reset_info.as_ref(),
                        fw,
                        now_s,
                    ));
                }
            }
            if !changes.is_empty() {
                st.dirty = true;
            }
            self.save(&mut st);
            self.events_for(&st.book, &changes, hub.hub_id(), now_s)
        };
        for e in events {
            hub.inner.relay.broadcast_event(&e);
        }
        for m in firmware {
            self.bus.publish(m);
        }
        // -- WP P3GP -- a record dropped past the cap (`make_room`) is forgotten as
        // `inventory.forget` forgets one: the groups (P3GR) and Home Assistant hear it.
        for c in &changes {
            if let Change::Removed(serial) = c {
                self.bus.publish(BusMessage::DeviceForgotten {
                    serial: serial.clone(),
                });
            }
        }
    }

    /// The `inventory.changed` texts and the firmware records for `changes`.
    fn events_for(
        &self,
        book: &Book,
        changes: &[Change],
        hub_id: &str,
        now_s: u64,
    ) -> (Vec<String>, Vec<BusMessage>) {
        let mut sent: Vec<&str> = Vec::new();
        let mut events = Vec::new();
        let mut firmware = Vec::new();
        for c in changes {
            let serial = match c {
                Change::Updated(s) | Change::Removed(s) => s.as_str(),
                Change::Firmware { serial, old, new } => {
                    let mut record = EventRecord::new(
                        EventKind::Firmware,
                        self.clock.unix_ms(),
                        format!("Firmware {new} (was {old})"),
                    );
                    record.fw = Some(new.clone());
                    record.serial = serial.clone();
                    firmware.push(BusMessage::Event {
                        serial: serial.clone(),
                        record,
                    });
                    serial.as_str()
                }
            };
            if sent.contains(&serial) {
                continue;
            }
            sent.push(serial);
            events.push(
                json!({
                    "t": CHANGED_EVENT,
                    "v": VERSION,
                    "hub": hub_id,
                    "serial": serial,
                    "device": book.json(serial, now_s),
                })
                .to_string(),
            );
        }
        (events, firmware)
    }

    fn save(&self, st: &mut State) {
        if !st.dirty {
            return;
        }
        let Some(path) = self.file() else {
            st.dirty = false;
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = fs::create_dir_all(dir)
        {
            log::error!("hub: cannot make {}: {e}", dir.display());
            return;
        }
        match write_atomic(&path, &st.book.to_file()) {
            Ok(()) => st.dirty = false,
            // Tried again on the next tick.
            Err(e) => log::error!("hub: cannot save {}: {e}", path.display()),
        }
    }

    fn list(&self, msg: &Value, hub: &HubHandle) -> Result<Value, (&'static str, String)> {
        let serial = address(hub, msg, false)?;
        let now_s = self.clock.unix_seconds();
        let st = lock(&self.state);
        if let Some(serial) = serial {
            let device = st
                .book
                .json(&serial, now_s)
                .ok_or(("not_found", format!("no device {serial} in the inventory")))?;
            return Ok(json!({"devices": [device], "total": 1, "next_offset": null}));
        }
        let offset = page_field(msg, "offset", 0, MAX_DEVICES).map_err(|m| ("bad_request", m))?;
        let limit = page_field(msg, "limit", DEFAULT_PAGE, MAX_PAGE)
            .map_err(|m| ("bad_request", m))?
            .max(1);
        let all = st.book.ordered();
        let total = all.len();
        let mut devices = Vec::new();
        let mut bytes = 0usize;
        for r in all.iter().skip(offset).take(limit) {
            let v = r.to_json(st.book.handle(&r.serial), now_s);
            bytes = bytes.saturating_add(v.to_string().len());
            if bytes > MAX_PAGE_BYTES && !devices.is_empty() {
                break;
            }
            devices.push(v);
        }
        let next = offset.saturating_add(devices.len());
        let next_offset = (next < total).then_some(next);
        Ok(json!({"devices": devices, "total": total, "next_offset": next_offset}))
    }

    fn set(&self, msg: &Value, hub: &HubHandle) -> Result<Value, (&'static str, String)> {
        let Some(serial) = address(hub, msg, true)? else {
            return Err(("bad_request", "no serial".into()));
        };
        let edit = Edit {
            location: text_field(msg, "location").map_err(|m| ("bad_request", m))?,
            note: text_field(msg, "note").map_err(|m| ("bad_request", m))?,
            tags: tags_field(msg).map_err(|m| ("bad_request", m))?,
        };
        let now_s = self.clock.unix_seconds();
        let (device, events) = {
            let mut st = lock(&self.state);
            let changes = st
                .book
                .set(&serial, edit)
                .map_err(|e| (e.code(), e.to_string()))?;
            if !changes.is_empty() {
                st.dirty = true;
                self.save(&mut st);
            }
            let (events, _) = self.events_for(&st.book, &changes, hub.hub_id(), now_s);
            (st.book.json(&serial, now_s), events)
        };
        for e in events {
            hub.inner.relay.broadcast_event(&e);
        }
        Ok(json!({ "device": device }))
    }

    fn forget(&self, msg: &Value, hub: &HubHandle) -> Result<Value, (&'static str, String)> {
        let Some(serial) = address(hub, msg, true)? else {
            return Err(("bad_request", "no serial".into()));
        };
        let with_data = match msg.get("with_data") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(("bad_request", "with_data is true or false".into())),
        };
        // The registry decides online, not the book, which may be a tick behind.
        let attached = hub
            .devices()
            .iter()
            .any(|d| d.identity.serial == serial && d.state == DeviceState::Online);
        if attached {
            return Err(("conflict", format!("{serial} is online")));
        }
        lock(&self.state)
            .book
            .check_forget(&serial)
            .map_err(|e| (e.code(), e.to_string()))?;
        // The device's data first: if a removal fails, the record stays for a retry.
        let mut forgot = Vec::new();
        let mut snapshot_events = Vec::new();
        if let Some(dir) = &self.data_dir {
            let removed = forget_events(dir, &serial).map_err(|e| ("internal", e))?;
            if removed > 0 {
                forgot.push("events");
            }
            if with_data {
                let (removed, events) =
                    forget_snapshots(dir, &serial, hub.hub_id()).map_err(|e| ("internal", e))?;
                snapshot_events = events;
                if removed > 0 {
                    forgot.push("snapshots");
                }
            }
        }
        let now_s = self.clock.unix_seconds();
        let events = {
            let mut st = lock(&self.state);
            st.book
                .forget(&serial)
                .map_err(|e| (e.code(), e.to_string()))?;
            st.dirty = true;
            self.save(&mut st);
            let removed = [Change::Removed(serial.clone())];
            self.events_for(&st.book, &removed, hub.hub_id(), now_s).0
        };
        forgot.insert(0, "record");
        for e in snapshot_events.iter().chain(&events) {
            hub.inner.relay.broadcast_event(e);
        }
        log::info!("hub: inventory forgot {serial} ({})", forgot.join(", "));
        // -- WP P2DFIXA -- for Home Assistant (P2HA), which keeps a device's entities
        // until it is forgotten here.
        self.bus.publish(BusMessage::DeviceForgotten {
            serial: serial.clone(),
        });
        Ok(json!({"serial": serial, "forgot": forgot}))
    }
}

/// Remove `serial`'s history files in `<data>/events/` (P2EVW's `forget`).
fn forget_events(data_dir: &Path, serial: &str) -> Result<u32, String> {
    use dspi_session::events::{EventStore, HUB_LIMITS, HUB_TOTAL_BYTES};
    let dir = data_dir.join("events");
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut store = EventStore::open(dir, HUB_LIMITS, HUB_TOTAL_BYTES)
        .map_err(|e| format!("the history: {e}"))?;
    store
        .forget(serial)
        .map_err(|e| format!("the history of {serial}: {e}"))
}

/// Remove `serial`'s hub snapshots (P2SS's store), with their `snapshot.changed`.
fn forget_snapshots(
    data_dir: &Path,
    serial: &str,
    hub_id: &str,
) -> Result<(usize, Vec<String>), String> {
    let store = crate::snapshots::HubSnapshots::open(data_dir);
    let entries = match store.list(serial) {
        Ok(entries) => entries,
        Err(e) if e.code() == "not_found" => return Ok((0, Vec::new())),
        Err(e) => return Err(format!("the snapshots of {serial}: {e}")),
    };
    let mut removed = 0usize;
    for entry in entries {
        match store.delete(&entry.id) {
            Ok(_) => removed = removed.saturating_add(1),
            Err(e) if e.code() == "not_found" => {}
            Err(e) => return Err(format!("snapshot {}: {e}", entry.id)),
        }
    }
    Ok((removed, store.take_changed(hub_id)))
}

impl HubService for InventoryService {
    fn name(&self) -> &'static str {
        "inventory"
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
        let t = msg.get("t")?.as_str()?;
        if !MESSAGES.contains(&t) {
            return None;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let admin_only = t != "inventory.list";
        if admin_only && session.role() < Role::Admin {
            return Some(reply_err(&id, "denied", "an admin's"));
        }
        // Answer from a book that has seen everything up to now.
        self.process(hub);
        let answer = match t {
            "inventory.list" => self.list(msg, hub),
            "inventory.set" => self.set(msg, hub),
            _ => self.forget(msg, hub),
        };
        Some(match answer {
            Ok(body) => reply_ok(&id, body),
            Err((code, text)) => reply_err(&id, code, &text),
        })
    }

    fn next_due(&self) -> Option<Instant> {
        let st = lock(&self.state);
        if st.resync || st.dirty || !self.sub.is_empty() {
            Some(self.clock.now().min(st.next_tick))
        } else {
            Some(st.next_tick)
        }
    }

    fn run_due(&self, hub: &HubHandle, _now: Instant) {
        self.process(hub);
    }

    fn on_device(&self, event: &DeviceEvent) {
        let _ = event;
        lock(&self.state).resync = true;
        self.wake.wake();
    }

    fn stop(&self) {
        let mut st = lock(&self.state);
        let now_s = self.clock.unix_seconds();
        st.book.touch_online(now_s);
        st.dirty = true;
        self.save(&mut st);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2in;
