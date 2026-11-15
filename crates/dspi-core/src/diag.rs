//! The diagnostics bundle (WP P0E, PRO1 E2): one zip a user saves and
//! attaches to a support request by hand. Nothing here sends anything
//! anywhere: the bytes go to the path the caller names and nowhere else.
//!
//! What is in it (see [`DspiCore::diagnostics_zip`]):
//!
//! | File | What |
//! |---|---|
//! | `README.txt` | what this is, what is in it, what was left out |
//! | `versions.json` | this core, the client, the OS, the device's firmware |
//! | `device/dump.json` | the open device: serial, transport, probe (`uconsole dump --json`'s capabilities), extensions, the mirror |
//! | `device/stats.json` | the status counters, buffers, ring health, clock diagnostics |
//! | `device/interrupts.txt` | the notification monitor's lines (Interrupt Monitor) |
//! | `events/<serial>.jsonl` | each device's event log (P0M), in its file format |
//! | `hub/hub-store.json` | the hub's store with tokens, hashes and secrets removed |
//! | `hub/inventory.json` | the hub's inventory (WP P2IN), secrets removed the same way |
//! | `hub/alarms.json` | the hub's alarm rules (WP P2AL), secrets removed the same way |
//! | `hub/outbound.json` | the hub's notification targets (WP P2WH, entry WP P2AL): each secret, CA and pin as `set` or `not set`, each URL to its host |
//! | `system/os.json` | OS facts: name, architecture, release, the udev rule on Linux |
//! | `system/usb.json` | DSPis and bootloaders on the bus, by enumeration only |
//! | `client/<name>` | files the client adds (`uconsole doctor`'s report, a log) |
//! | `notes.txt` | what could not be read, and why |
//!
//! Every section is read on its own: one that fails (no device, a Link
//! viewer refused a read) leaves a line in `notes.txt` and the rest is
//! still written. Device reads are the ones a client's Stats window makes;
//! nothing is written to the device, nothing on the bus is opened, and no
//! read holds the core's lock past its own call.
//!
//! The hub store is filtered by key, not copied: every value whose key
//! names a token, a hash, a secret, a PIN, a password or a key is replaced
//! with [`REMOVED`], and a file that is not JSON is left out entirely.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::records::{DiagFile, DiagOptions};
use crate::{DspiCore, DspiError, Result, consts::*};

mod zip;

/// What a removed value reads as in the bundle.
pub const REMOVED: &str = "(removed)";
/// The hub store's file name in its directory (dspi-hub `store.rs`).
const HUB_STORE_FILE: &str = "hub-store.json";
/// The largest hub store read: the hub's own limit.
const HUB_STORE_MAX: u64 = 1 << 20;
/// The largest client file kept; a longer one is cut there with a note.
const CLIENT_FILE_MAX: usize = 4 << 20;
/// Parts of a key whose value never goes into a bundle (compared in lower
/// case, `_` and `-` removed).
const SECRET_KEYS: &[&str] = &[
    "token",
    "secret",
    "sha256",
    "hash",
    "pin",
    "password",
    "passwd",
    "passphrase",
    "key",
    "credential",
    "cookie",
    "auth",
];
/// Keys that contain one of [`SECRET_KEYS`] and are not secrets: the hub's
/// access mode is `pin`, `open` or `paired`, which a supporter needs.
const NOT_SECRET_KEYS: &[&str] = &["access"];

/// The status counters a bundle reads, by name (`STAT_*`).
const COUNTERS: &[(&str, u16)] = &[
    ("pdm_ring_overruns", STAT_PDM_RING_OVERRUNS),
    ("pdm_ring_underruns", STAT_PDM_RING_UNDERRUNS),
    ("pdm_dma_overruns", STAT_PDM_DMA_OVERRUNS),
    ("pdm_dma_underruns", STAT_PDM_DMA_UNDERRUNS),
    ("spdif_overruns", STAT_SPDIF_OVERRUNS),
    ("spdif_underruns", STAT_SPDIF_UNDERRUNS),
    ("clock_hz", STAT_CLOCK_HZ),
    ("core_mv", STAT_CORE_MV),
    ("sample_rate", STAT_SAMPLE_RATE),
    ("temperature", STAT_TEMPERATURE),
    ("starvation_total", STAT_STARVATION_TOTAL),
    ("starvation_first", STAT_STARVATION_FIRST),
    ("usb_ring_overruns", STAT_USB_RING_OVERRUNS),
];

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Write a diagnostics bundle to `path`: one zip with versions, the open
    /// device's dump, stats and monitor lines, every event log, the hub
    /// store at `options.hub_store_dir` with its tokens and secrets removed,
    /// the OS and USB facts, and `options.files`. Nothing is sent anywhere.
    /// Returns the names of the files in the zip.
    ///
    /// Reads the device (a Stats window's reads) and files: not on the UI
    /// thread. The zip is written beside `path` first and renamed over it,
    /// so a failure leaves no half file.
    ///
    /// Errors: [`DspiError::Io`] when the file cannot be written;
    /// [`DspiError::Unsupported`] where there is no file system (the web,
    /// which saves [`DspiCore::diagnostics_zip`]'s bytes itself). What the
    /// bundle could not read is in its `notes.txt`, never an error.
    pub fn diagnostics_save(&self, path: String, options: DiagOptions) -> Result<Vec<String>> {
        let (bytes, names) = self.diagnostics_build(options)?;
        write_atomically(Path::new(&path), &bytes).map_err(|e| {
            if e.kind() == std::io::ErrorKind::Unsupported {
                DspiError::Unsupported
            } else {
                DspiError::Io {
                    msg: format!("the diagnostics file {path}: {e}"),
                }
            }
        })?;
        Ok(names)
    }

    /// A file name for a bundle saved now: `dspi-diagnostics-` and the UTC
    /// date and time, `.zip`. For a Save dialog's suggestion. Never fails.
    pub fn diagnostics_file_name(&self) -> String {
        let (y, mo, d, h, mi, s) = utc(crate::events::now_ms());
        format!("dspi-diagnostics-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}Z.zip")
    }
}

impl DspiCore {
    /// [`DspiCore::diagnostics_save`]'s zip as bytes, for a caller that
    /// saves it itself (the web). Errors: [`DspiError::Io`] for a bundle past
    /// 4 GiB, which no device makes.
    pub fn diagnostics_zip(&self, options: DiagOptions) -> Result<Vec<u8>> {
        self.diagnostics_build(options).map(|(bytes, _)| bytes)
    }

    fn diagnostics_build(&self, options: DiagOptions) -> Result<(Vec<u8>, Vec<String>)> {
        let now = crate::events::now_ms();
        let scrub = Scrub::from_env();
        let mut b = Bundle::default();

        b.json("versions.json", &self.diag_versions(&options.app, now));
        let connected = self.is_connected();
        if connected {
            let dump = self.diag_dump(&mut b.notes);
            b.json("device/dump.json", &dump);
            let stats = self.diag_stats(&mut b.notes);
            b.json("device/stats.json", &stats);
        } else {
            b.notes
                .push("device: none open, so no dump and no stats".into());
        }
        b.text("device/interrupts.txt", &self.diag_interrupts());
        self.diag_events(&mut b);
        diag_hub_store(&mut b, &options.hub_store_dir);
        // -- WP P2IN --
        diag_hub_data(&mut b, &options.hub_store_dir, HUB_INVENTORY_FILE);
        // -- WP P2AL --
        diag_hub_data(&mut b, &options.hub_store_dir, HUB_ALARMS_FILE);
        diag_hub_outbound(&mut b, &options.hub_store_dir);
        b.json("system/os.json", &os_facts());
        b.json("system/usb.json", &self.diag_usb(&scrub));
        diag_client_files(&mut b, options.files, &scrub);

        let mut readme = readme(now, connected);
        let _ = writeln!(readme, "  README.txt");
        for (name, _) in &b.files {
            let _ = writeln!(readme, "  {name}");
        }
        let _ = writeln!(readme, "  notes.txt");
        let notes = if b.notes.is_empty() {
            "Nothing was left out.\n".to_owned()
        } else {
            b.notes.iter().fold(String::new(), |mut s, n| {
                let _ = writeln!(s, "{}", scrub.apply(n));
                s
            })
        };

        let mut zip = zip::ZipWriter::new(utc(now));
        let too_large = |_| DspiError::Io {
            msg: "the diagnostics bundle is past what a zip holds".into(),
        };
        zip.add("README.txt", readme.as_bytes())
            .map_err(too_large)?;
        let mut names = vec!["README.txt".to_owned()];
        for (name, data) in &b.files {
            zip.add(name, data).map_err(too_large)?;
            names.push(name.clone());
        }
        zip.add("notes.txt", notes.as_bytes()).map_err(too_large)?;
        names.push("notes.txt".into());
        Ok((zip.finish().map_err(too_large)?, names))
    }

    fn diag_versions(&self, app: &str, now: i64) -> Value {
        let info = crate::core_info();
        let state = self.get_state();
        let firmware = (self.is_connected()).then(|| {
            let mut v = format!("{}.{}.{}", state.fw_major, state.fw_minor, state.fw_patch);
            if state.fw_beta != 0 {
                let _ = write!(v, " beta {}", state.fw_beta);
            }
            v
        });
        let (y, mo, d, h, mi, s) = utc(now);
        json!({
            "created": format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
            "client": app,
            "core": info.ffi_version,
            "wireFormat": info.wire_format_version,
            "minWireFormat": info.min_wire_format_version,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
            "device": firmware.map(|fw| json!({
                "firmware": fw,
                "wireFormat": state.format_version,
                "platformId": state.platform_id,
            })),
        })
    }

    fn diag_dump(&self, notes: &mut Vec<String>) -> Value {
        let state = self.get_state();
        let mut inner = self.lock();
        let Some(conn) = inner.conn.as_mut() else {
            notes.push("device: closed while the bundle was read".into());
            return Value::Null;
        };
        let serial = conn.serial.clone();
        let transport = format!("{:?}", conn.caps.kind).to_lowercase();
        let read_only = !conn.writable();
        let extensions = conn.ext.supported.clone();
        let caps = conn
            .session()
            .ok()
            .map(|s| serde_json::to_value(s.capabilities()));
        drop(inner);
        let state = match serde_json::to_value(state) {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!(
                    "device/dump.json: the mirror would not serialise: {e}"
                ));
                Value::Null
            }
        };
        let caps = match caps {
            Some(Ok(v)) => v,
            Some(Err(e)) => {
                notes.push(format!(
                    "device/dump.json: the probe would not serialise: {e}"
                ));
                Value::Null
            }
            None => {
                notes.push("device/dump.json: no probe (a raw connection)".into());
                Value::Null
            }
        };
        json!({
            "serial": serial,
            "transport": transport,
            "readOnly": read_only,
            "capabilities": caps,
            "extensions": extensions,
            "state": state,
        })
    }

    fn diag_stats(&self, notes: &mut Vec<String>) -> Value {
        let mut counters = Map::new();
        for &(name, id) in COUNTERS {
            let v = match self.fetch_stat(id) {
                Ok(v) => json!(v),
                Err(e) => {
                    notes.push(format!("device/stats.json: {name}: {e}"));
                    Value::Null
                }
            };
            counters.insert(name.into(), v);
        }
        let mut section = |what: &str, r: Result<Value>| match r {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!("device/stats.json: {what}: {e}"));
                Value::Null
            }
        };
        let to_json = |v: std::result::Result<Value, serde_json::Error>| {
            v.map_err(|e| DspiError::Io { msg: e.to_string() })
        };
        let buffers = section(
            "buffers",
            self.fetch_buffer_stats()
                .and_then(|b| to_json(serde_json::to_value(b))),
        );
        let ring = section(
            "ringHealth",
            self.fetch_ring_health()
                .and_then(|r| to_json(serde_json::to_value(r))),
        );
        let clock = section(
            "clockDiag",
            self.fetch_clock_diag()
                .and_then(|c| to_json(serde_json::to_value(c))),
        );
        json!({
            "counters": counters,
            "buffers": buffers,
            "ringHealth": ring,
            "clockDiag": clock,
            "readOnlyRefusals": self.read_only_refusals(),
        })
    }

    fn diag_interrupts(&self) -> String {
        let batch = self.monitor_read(0);
        if batch.entries.is_empty() {
            return "No notifications this run.\n".into();
        }
        let mut out = String::new();
        for e in &batch.entries {
            let (y, mo, d, h, mi, s) = utc(e.time_ms);
            let ms = e.time_ms.rem_euclid(1000);
            let _ = writeln!(
                out,
                "{:>6}  {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{ms:03}Z  {}",
                e.id, e.text
            );
        }
        out
    }

    fn diag_events(&self, b: &mut Bundle) {
        let devices = self.event_log_devices();
        if devices.is_empty() {
            b.notes
                .push("events: no event log yet (no device seen this run, no folder set)".into());
        }
        for serial in devices {
            let text = self.event_log_export(serial.clone());
            let name = format!("events/{}.jsonl", file_part(&serial));
            b.text(&name, &text);
        }
    }

    fn diag_usb(&self, scrub: &Scrub) -> Value {
        let devices: Vec<Value> = self
            .scan_quiet()
            .into_iter()
            .map(|d| json!({ "serial": d.serial, "locationId": d.location_id }))
            .collect();
        let boot = crate::bootloader_counts();
        let drives: Vec<Value> = crate::bootloader_drives()
            .into_iter()
            .map(|d| json!({ "path": scrub.apply(&d.path), "platform": d.platform }))
            .collect();
        json!({
            "enumeration": cfg!(all(feature = "usb", not(target_os = "android"))),
            "devices": devices,
            "inaccessibleAtLastScan": self.inaccessible_devices(),
            "bootloaders": { "rp2040": boot.rp2040, "rp2350": boot.rp2350 },
            "bootloaderDrives": drives,
        })
    }
}

// -- review p0e N4 (the coordinator's decision; the owner may reverse it) --
/// Takes the user out of paths: the home folder becomes `~`, and the user
/// name as a whole path segment (`/run/media/<user>/RPI-RP2`) `<user>`.
/// Only path-shaped text is touched, so a serial or a name that happens to
/// contain the user name is left alone.
pub(crate) struct Scrub {
    home: Option<String>,
    user: Option<String>,
}

impl Scrub {
    /// This process's home folder and user name, where the platform has
    /// them (`HOME`/`USERPROFILE`, `USER`/`USERNAME`); none on the web.
    fn from_env() -> Self {
        let var = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| std::env::var(n).ok())
                .filter(|v| !v.is_empty())
        };
        Self::new(var(&["HOME", "USERPROFILE"]), var(&["USER", "USERNAME"]))
    }

    pub(crate) fn new(home: Option<String>, user: Option<String>) -> Self {
        // A home of "/" or "C:\\" would turn every path into "~...".
        let home = home
            .map(|h| h.trim_end_matches(['/', '\\']).to_owned())
            .filter(|h| h.chars().filter(|c| *c != ':').count() > 2);
        let user = user.filter(|u| !u.contains(['/', '\\']));
        Self { home, user }
    }

    pub(crate) fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        if let Some(home) = &self.home {
            out = replace_prefixed(&out, home, "~");
        }
        if let Some(user) = &self.user {
            for sep in ['/', '\\'] {
                out = out.replace(&format!("{sep}{user}{sep}"), &format!("{sep}<user>{sep}"));
            }
        }
        out
    }
}

/// `text` with `from` replaced by `to` where `from` is followed by a path
/// separator or ends a word: `/home/bob` must not turn `/home/bobby` into
/// `~by`.
fn replace_prefixed(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let (before, tail) = rest.split_at(at);
        let after = tail.get(from.len()..).unwrap_or("");
        out.push_str(before);
        let ends = after.chars().next().is_none_or(|c| {
            matches!(c, '/' | '\\') || !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
        });
        out.push_str(if ends { to } else { from });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The files so far, in order, and what could not be read.
#[derive(Default)]
struct Bundle {
    files: Vec<(String, Vec<u8>)>,
    notes: Vec<String>,
}

impl Bundle {
    fn text(&mut self, name: &str, text: &str) {
        self.add(name, text.as_bytes().to_vec());
    }

    fn json(&mut self, name: &str, value: &Value) {
        match serde_json::to_vec_pretty(value) {
            Ok(mut v) => {
                v.push(b'\n');
                self.add(name, v);
            }
            Err(e) => self.notes.push(format!("{name}: {e}")),
        }
    }

    /// Add under `name`, or under `name` with `-2`, `-3`... when taken.
    fn add(&mut self, name: &str, data: Vec<u8>) {
        let taken = |n: &str| {
            self.files.iter().any(|(f, _)| f == n) || n == "README.txt" || n == "notes.txt"
        };
        let mut unique = name.to_owned();
        let mut i = 2u32;
        while taken(&unique) {
            unique = match name.rsplit_once('.') {
                Some((stem, ext))
                    if !stem.ends_with('/')
                        && ext.len() <= 5
                        && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
                {
                    format!("{stem}-{i}.{ext}")
                }
                _ => format!("{name}-{i}"),
            };
            i = i.saturating_add(1);
        }
        self.files.push((unique, data));
    }
}

/// The hub store under `dir` with its secrets removed, as `hub/hub-store.json`.
fn diag_hub_store(b: &mut Bundle, dir: &str) {
    if dir.is_empty() {
        b.notes.push("hub: no store named".into());
        return;
    }
    let path = Path::new(dir).join(HUB_STORE_FILE);
    let bytes = match read_limited(&path, HUB_STORE_MAX) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            b.notes.push(format!("hub: no store at {}", path.display()));
            return;
        }
        Err(e) => {
            b.notes
                .push(format!("hub: the store at {} unread: {e}", path.display()));
            return;
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut v) => {
            redact(&mut v);
            b.json("hub/hub-store.json", &v);
        }
        // Not ours to judge: it could hold anything, so none of it goes in.
        Err(_) => b.notes.push(format!(
            "hub: the store at {} is not JSON and was left out",
            path.display()
        )),
    }
}

// -- WP P2IN --
/// The hub's inventory file in its data folder (dspi-hub `inventory.rs`).
const HUB_INVENTORY_FILE: &str = "inventory.json";
/// The largest phase-2 hub data file read (the inventory's own limit is 8 MiB).
const HUB_DATA_MAX: u64 = 8 << 20;

/// A phase-2 data file of the hub under `dir` (PLAN-P2, review S6), redacted as the
/// store is, as `hub/<file>`. No folder named, or no such file: nothing, and no note
/// (most hubs have no such file); unreadable or not JSON: a note, and none of it.
fn diag_hub_data(b: &mut Bundle, dir: &str, file: &str) {
    if dir.is_empty() {
        return;
    }
    let path = Path::new(dir).join(file);
    let bytes = match read_limited(&path, HUB_DATA_MAX) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            b.notes.push(format!("hub: {} unread: {e}", path.display()));
            return;
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut v) => {
            redact(&mut v);
            b.json(&format!("hub/{file}"), &v);
        }
        Err(_) => b.notes.push(format!(
            "hub: {} is not JSON and was left out",
            path.display()
        )),
    }
}

// -- WP P2AL --
/// The hub's alarm rules (dspi-hub `device_alarms.rs`): no secret in them, redacted anyway.
const HUB_ALARMS_FILE: &str = "alarms.json";
/// The hub's notification targets (dspi-hub `outbound/`).
const HUB_OUTBOUND_FILE: &str = "outbound.json";
/// Keys of a target whose value is shown only as `set` or `not set` (the webhook secret;
/// the TLS trust and client keys PLAN-SEC adds).
const SET_OR_NOT_KEYS: &[&str] = &[
    "secret",
    "ca_pem",
    "pin_sha256",
    "client_cert_pem",
    "client_key_pem",
];

/// `outbound.json` under `dir` as `hub/outbound.json`: every value under
/// [`SET_OR_NOT_KEYS`] as `set` or `not set` (PLAN-P2 S6, PLAN-SEC), every other secret
/// key as the store's, and each `url` cut to its scheme and host (a path or a query can
/// carry a token). Unreadable or not JSON: a note, and none of it.
fn diag_hub_outbound(b: &mut Bundle, dir: &str) {
    if dir.is_empty() {
        return;
    }
    let path = Path::new(dir).join(HUB_OUTBOUND_FILE);
    let bytes = match read_limited(&path, HUB_DATA_MAX) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            b.notes.push(format!("hub: {} unread: {e}", path.display()));
            return;
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut v) => {
            redact_outbound(&mut v);
            b.json(&format!("hub/{HUB_OUTBOUND_FILE}"), &v);
        }
        Err(_) => b.notes.push(format!(
            "hub: {} is not JSON and was left out",
            path.display()
        )),
    }
}

/// [`redact`], with [`SET_OR_NOT_KEYS`] shown as `set` or `not set` and URLs cut.
pub(crate) fn redact_outbound(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if SET_OR_NOT_KEYS.contains(&k.as_str()) {
                    let set = match child {
                        Value::Null => false,
                        Value::String(s) => !s.is_empty() && s != "not set",
                        _ => true,
                    };
                    *child = Value::String(if set { "set" } else { "not set" }.into());
                } else if k == "url" {
                    if let Value::String(s) = child {
                        *child = Value::String(url_host(s));
                    } else {
                        *child = Value::String(REMOVED.into());
                    }
                } else if is_secret_key(k) {
                    *child = Value::String(REMOVED.into());
                } else {
                    redact_outbound(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_outbound),
        _ => {}
    }
}

/// `scheme://host[:port]` of a URL, without credentials, path or query; "(removed)"
/// past it, so a supporter sees where the hub sends and not what a URL may hide.
fn url_host(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return REMOVED.into();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let more = rest.len() > authority.len();
    let scheme: String = scheme.chars().take(16).collect();
    let host: String = host.chars().take(255).collect();
    if more {
        format!("{scheme}://{host}/{REMOVED}")
    } else {
        format!("{scheme}://{host}")
    }
}

/// Replace every value under a secret key, at any depth, with [`REMOVED`].
pub(crate) fn redact(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if is_secret_key(k) {
                    *child = Value::String(REMOVED.into());
                } else {
                    redact(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let k: String = key
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    !NOT_SECRET_KEYS.contains(&k.as_str()) && SECRET_KEYS.iter().any(|s| k.contains(s))
}

fn diag_client_files(b: &mut Bundle, files: Vec<DiagFile>, scrub: &Scrub) {
    for f in files {
        let mut text = scrub.apply(&f.text);
        if text.len() > CLIENT_FILE_MAX {
            let mut cut = CLIENT_FILE_MAX;
            while !text.is_char_boundary(cut) {
                cut = cut.saturating_sub(1);
            }
            text.truncate(cut);
            b.notes
                .push(format!("client/{}: cut at 4 MiB", file_part(&f.name)));
        }
        b.text(&format!("client/{}", file_part(&f.name)), &text);
    }
}

/// `name` as one path component: letters, digits, `.`, `_` and `-` kept,
/// anything else `_`, no leading dot, never empty, at most 80 characters.
fn file_part(name: &str) -> String {
    let s: String = name
        .chars()
        .take(80)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_start_matches('.').to_owned();
    if s.is_empty() { "unnamed".into() } else { s }
}

fn read_limited(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(std::io::Error::other("too large"));
    }
    Ok(bytes)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// What a supporter asks first about the machine. File reads only, and
/// only where the file is the platform's own (Linux); no program is run.
fn os_facts() -> Value {
    let mut facts = Map::new();
    facts.insert("os".into(), json!(std::env::consts::OS));
    facts.insert("arch".into(), json!(std::env::consts::ARCH));
    facts.insert("family".into(), json!(std::env::consts::FAMILY));
    #[cfg(not(target_family = "wasm"))]
    facts.insert(
        "cpus".into(),
        json!(std::thread::available_parallelism().map_or(0, |n| n.get())),
    );
    #[cfg(target_os = "linux")]
    {
        let first_line = |p: &str| {
            std::fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(str::to_owned))
        };
        facts.insert(
            "kernel".into(),
            json!(first_line("/proc/sys/kernel/osrelease")),
        );
        let pretty = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_owned())
            });
        facts.insert("distribution".into(), json!(pretty));
        // `uconsole doctor`'s check: without the rule a user cannot claim
        // the device.
        let udev = "/etc/udev/rules.d/70-dspi.rules";
        facts.insert(
            "udevRule".into(),
            json!({ "path": udev, "present": Path::new(udev).exists() }),
        );
    }
    Value::Object(facts)
}

fn readme(now: i64, connected: bool) -> String {
    let (y, mo, d, h, mi, s) = utc(now);
    format!(
        "DSPi diagnostics bundle\n\
         =======================\n\
         \n\
         Written {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC by the DSPi core {}.\n\
         {}\n\
         \n\
         Nothing in this file was sent anywhere. It was written only where it\n\
         was saved; share it yourself if you want to.\n\
         \n\
         Left out on purpose: the hub's client tokens and their hashes, PINs,\n\
         and every other value whose name marks it as a secret (they read\n\
         \"{REMOVED}\"). In paths, this computer's home folder reads \"~\" and\n\
         the user name \"<user>\" (notes.txt, bootloader drives, the client's\n\
         files). Device serials, device and client names, and the rest of\n\
         each path are kept, because a supporter needs them to read the rest.\n\
         \n\
         notes.txt lists what could not be read.\n\
         \n\
         Files:\n",
        env!("CARGO_PKG_VERSION"),
        if connected {
            "A device was open."
        } else {
            "No device was open."
        },
    )
}

/// UTC calendar fields of `ms` since the epoch: `(year, month, day, hour,
/// minute, second)`. Howard Hinnant's `civil_from_days`.
pub(crate) fn utc(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let field = |v: i64| u32::try_from(v).unwrap_or(0);
    let (h, mi, s) = (
        field(tod.div_euclid(3600)),
        field(tod.rem_euclid(3600).div_euclid(60)),
        field(tod.rem_euclid(60)),
    );
    // Days since 0000-03-01; every step stays far inside i64 for any i64
    // millisecond count, so the plain operators cannot overflow here.
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = doe
        .saturating_sub(doe.div_euclid(1460))
        .saturating_add(doe.div_euclid(36_524))
        .saturating_sub(doe.div_euclid(146_096))
        .div_euclid(365);
    let doy = doe.saturating_sub(
        yoe.saturating_mul(365)
            .saturating_add(yoe.div_euclid(4))
            .saturating_sub(yoe.div_euclid(100)),
    );
    let mp = doy.saturating_mul(5).saturating_add(2).div_euclid(153);
    let d = doy
        .saturating_sub(mp.saturating_mul(153).saturating_add(2).div_euclid(5))
        .saturating_add(1);
    let m = if mp < 10 {
        mp.saturating_add(3)
    } else {
        mp.saturating_sub(9)
    };
    let y = yoe
        .saturating_add(era.saturating_mul(400))
        .saturating_add(i64::from(m <= 2));
    (y, field(m), field(d), h, mi, s)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0e;
// -- WP P2IN --
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_p2in;
// -- WP P2AL --
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_p2al;
