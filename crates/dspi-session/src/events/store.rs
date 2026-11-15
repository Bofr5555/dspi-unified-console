//! The multi-device event writer (WP P2EVW, PRO1 M1's hub half): one writer
//! for the logs of many devices in one folder.
//!
//! [`EventLog`](super::EventLog) records for one open device at a time, as a
//! client does. The hub has many devices at once, so [`EventStore`] takes the
//! serial with every record. Everything else is the client's format, so a
//! client's `EventLog`, [`EventLog::export_in`](super::EventLog::export_in)
//! and the diagnostics bundle read the hub's folder unchanged:
//!
//! - the same file names (`events-s-<serial>.jsonl`, `.1.jsonl` ...), the same
//!   rotation at [`Limits::max_file_bytes`] keeping
//!   [`Limits::rotated_files`] older files;
//! - the same lock rule: each append takes the exclusive advisory lock on
//!   `<stem>.lock` and settles the id there as one past the newest in the
//!   files, so a client writing the same serial in the same folder never
//!   writes the same id;
//! - the same id rule: rising per device across files and runs, up to
//!   [`MAX_ID`](super::MAX_ID).
//!
//! On top, a **total cap** over the folder ([`HUB_TOTAL_BYTES`], OD-P2k): when
//! the logs in it grow past it, whole files are removed, the oldest rotated
//! file first (by its last write); a device's current file only when no
//! rotated file is left. [`EventStore::forget`] removes one device's files.
//!
//! Records are written as given: [`EventRecord::coalesce`] is ignored (the
//! writer coalesces before, if it wants to). A record that cannot be written is
//! not kept: the call says why and [`EventStore::write_errors`] counts it.
//!
//! Files only: the module is not built for the web.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{
    EventRecord, Limits, LogReader, RECORD_VERSION, ReadBatch, encode, file_stem, files,
    next_after, serial_of,
};

/// The hub's per-device bounds (OD-P2k): 512 KiB per file and three older
/// files, so about 2 MiB (some 10 000 records) per device.
pub const HUB_LIMITS: Limits = Limits {
    max_file_bytes: 512 * 1024,
    rotated_files: 3,
    max_line_bytes: Limits::DEFAULT.max_line_bytes,
    memory_records: 0,
};

/// The hub's cap over every device's log together (OD-P2k): 128 MiB.
pub const HUB_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

/// The longest serial the store takes, in bytes. A serial that is not plain
/// ASCII is written in hex in the file name (twice as long), and a file name
/// must stay well inside the usual 255-byte limit. Real serials are 16 hex
/// digits.
pub const MAX_SERIAL_BYTES: usize = 64;

/// Serials whose next id the store remembers between appends; past it the
/// memory starts again (the files' newest id still rules under the lock).
const MAX_TRACKED: usize = 4096;

/// Why the store refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum EventStoreError {
    /// The serial is empty or longer than [`MAX_SERIAL_BYTES`].
    #[error("not a serial the event store takes")]
    BadSerial,
    /// The file system said no (the folder, a lock, a write, a removal).
    #[error("event store: {0}")]
    Io(#[from] io::Error),
}

/// One log file of the folder, as a scan finds it.
#[derive(Debug, Clone)]
struct LogFile {
    serial: String,
    /// 0 the current file, else the `n`th older one.
    n: u32,
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

/// The event logs of many devices in one folder (the hub's `<data>/events/`).
///
/// Not thread-safe by itself; the hub keeps it behind a mutex, off device
/// workers (appends do file I/O).
#[derive(Debug)]
pub struct EventStore {
    dir: PathBuf,
    limits: Limits,
    total_bytes: u64,
    /// The bytes the folder's logs hold: scanned at open and after an
    /// eviction, counted up per append in between (other writers are not
    /// counted until the next scan).
    used: u64,
    next_ids: HashMap<String, u64>,
    write_errors: u32,
    evicted_files: u64,
}

impl EventStore {
    /// The store in `dir` (created if missing, 0700 on Unix) with `limits` per
    /// device and `total_bytes` over all of them. The hub passes
    /// [`HUB_LIMITS`] and [`HUB_TOTAL_BYTES`] unless its configuration says
    /// otherwise. A folder already over the total is trimmed now.
    ///
    /// Errors: the folder cannot be created.
    pub fn open(dir: PathBuf, limits: Limits, total_bytes: u64) -> Result<Self, EventStoreError> {
        files::create_dir(&dir)?;
        let mut store = Self {
            dir,
            limits,
            total_bytes,
            used: 0,
            next_ids: HashMap::new(),
            write_errors: 0,
            evicted_files: 0,
        };
        store.enforce_total()?;
        Ok(store)
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The per-device bounds.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The cap over all devices, bytes.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// The bytes the logs hold, as last scanned plus this store's appends.
    pub fn used_bytes(&self) -> u64 {
        self.used
    }

    /// Records that could not be written since the store opened.
    pub fn write_errors(&self) -> u32 {
        self.write_errors
    }

    /// Files the total cap removed since the store opened.
    pub fn evicted_files(&self) -> u64 {
        self.evicted_files
    }

    /// Append `record` to `serial`'s log, and give it back as written: its id,
    /// the serial, the format's version, and cut to
    /// [`Limits::max_line_bytes`]. Rotates the device's file first when the
    /// line would not fit, and trims the folder when it grew past the total.
    ///
    /// Errors: a serial the store does not take; the lock or the write failed
    /// (the record is lost and counted in [`EventStore::write_errors`]). A
    /// failed trim after a good write is not an error of this record: it is
    /// tried again on the next append.
    pub fn append(
        &mut self,
        serial: &str,
        mut record: EventRecord,
    ) -> Result<EventRecord, EventStoreError> {
        check_serial(serial)?;
        record.v = RECORD_VERSION;
        record.serial = serial.to_string();
        record.coalesce = None;
        // At least one past this store's last id for the serial; under the lock
        // `files::append` raises it past the files' newest (another writer).
        record.id = self.next_ids.get(serial).copied().unwrap_or(1).max(1);
        if let Err(e) = files::append(&self.dir, &mut record, &self.limits) {
            self.write_errors = self.write_errors.saturating_add(1);
            return Err(e.into());
        }
        if self.next_ids.len() >= MAX_TRACKED && !self.next_ids.contains_key(serial) {
            self.next_ids.clear();
        }
        self.next_ids
            .insert(serial.to_string(), next_after(record.id));
        // The line as written (`files::append` already cut the record to fit).
        let written = encode(&mut record.clone(), self.limits.max_line_bytes);
        let len = u64::try_from(written.len()).unwrap_or(u64::MAX);
        self.used = self.used.saturating_add(len);
        if self.used > self.total_bytes {
            let _ = self.enforce_total();
        }
        Ok(record)
    }

    /// The records of `serial` with an id above `after_id`, oldest first, at
    /// most `limit` (the newest `limit` when `after_id` is 0), as
    /// [`LogReader::read`]. Takes no lock; a torn last line is skipped.
    pub fn read(&self, serial: &str, after_id: u64, limit: usize) -> ReadBatch {
        self.reader(serial).read(after_id, limit)
    }

    /// `serial`'s whole log as JSON lines in the file format, oldest first.
    pub fn export(&self, serial: &str) -> String {
        self.reader(serial).export()
    }

    /// The serials with a log in the folder (this store's or another
    /// writer's), sorted.
    pub fn devices(&self) -> Vec<String> {
        let mut out = files::devices(&self.dir);
        out.sort();
        out.dedup();
        out
    }

    /// Remove every file of `serial`'s log (current, rotated, its lock file),
    /// whoever wrote them; the device's ids start again at 1. Returns how many
    /// log files went. A client writing the same serial at that moment may
    /// start a new file afterwards.
    ///
    /// Errors: a serial the store does not take; a removal failed (what was
    /// removed stays removed).
    pub fn forget(&mut self, serial: &str) -> Result<u32, EventStoreError> {
        check_serial(serial)?;
        self.next_ids.remove(serial);
        let mine: Vec<LogFile> = scan(&self.dir)
            .into_iter()
            .filter(|f| f.serial == serial)
            .collect();
        let mut removed = 0u32;
        let result = with_lock(&self.dir, serial, || {
            for f in &mine {
                match fs::remove_file(&f.path) {
                    Ok(()) => removed = removed.saturating_add(1),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        });
        // The lock file last, once it is released.
        let lock = lock_path(&self.dir, serial);
        let lock_result = match fs::remove_file(&lock) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        self.used = scan(&self.dir)
            .iter()
            .map(|f| f.len)
            .fold(0, u64::saturating_add);
        result?;
        lock_result?;
        Ok(removed)
    }

    /// Scan the folder and remove whole files until its logs fit the total:
    /// the oldest rotated file first (by last write; a higher number first on
    /// a tie), a current file only when no rotated one is left. Each removal
    /// is made under the device's lock. Returns how many files went. Run by
    /// [`EventStore::append`] when the count passes the total, and at open.
    ///
    /// Errors: a lock or a removal failed; files removed before stay removed.
    pub fn enforce_total(&mut self) -> Result<u32, EventStoreError> {
        let mut logs = scan(&self.dir);
        let mut total = logs.iter().map(|f| f.len).fold(0, u64::saturating_add);
        self.used = total;
        // Removal order: rotated before current, then oldest first.
        logs.sort_by(|a, b| {
            (a.n == 0)
                .cmp(&(b.n == 0))
                .then(a.modified.cmp(&b.modified))
                .then(b.n.cmp(&a.n))
                .then(a.path.cmp(&b.path))
        });
        let mut removed = 0u32;
        for f in logs {
            if total <= self.total_bytes {
                break;
            }
            let gone = with_lock(&self.dir, &f.serial, || match fs::remove_file(&f.path) {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e),
            });
            match gone {
                Ok(true) => {
                    removed = removed.saturating_add(1);
                    self.evicted_files = self.evicted_files.saturating_add(1);
                    total = total.saturating_sub(f.len);
                }
                Ok(false) => total = total.saturating_sub(f.len),
                Err(e) => {
                    self.used = total;
                    return Err(e.into());
                }
            }
        }
        self.used = total;
        Ok(removed)
    }

    fn reader(&self, serial: &str) -> LogReader {
        LogReader {
            dir: Some(self.dir.clone()),
            limits: self.limits,
            serial: serial.to_string(),
            memory: Vec::new(),
        }
    }
}

fn check_serial(serial: &str) -> Result<(), EventStoreError> {
    if serial.is_empty() || serial.len() > MAX_SERIAL_BYTES {
        return Err(EventStoreError::BadSerial);
    }
    Ok(())
}

/// The advisory lock file of `serial`'s log, as `files` names it.
fn lock_path(dir: &Path, serial: &str) -> PathBuf {
    dir.join(format!("{}.lock", file_stem(serial)))
}

/// Run `f` holding `serial`'s exclusive advisory lock (the one every writer
/// of the format appends under).
fn with_lock<T>(dir: &Path, serial: &str, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let lock = o.open(lock_path(dir, serial))?;
    lock.lock()?;
    let result = f();
    let _ = lock.unlock();
    result
}

/// Every log file in `dir` (ours by name: `<stem>.jsonl`, `<stem>.<n>.jsonl`),
/// with its size and last write. Anything else in the folder is left alone.
fn scan(dir: &Path) -> Vec<LogFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let rest = name.strip_suffix(".jsonl")?;
            let (stem, n) = match rest.split_once('.') {
                None => (rest, 0),
                Some((stem, n)) => (stem, n.parse::<u32>().ok().filter(|n| *n > 0)?),
            };
            let serial = serial_of(stem)?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some(LogFile {
                serial,
                n,
                path: e.path(),
                len: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2evw;
