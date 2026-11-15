//! Notifications out: webhooks and push (WP P2WH, PLAN-P2 M4, `docs/integrator/hub-api.md`
//! section 8).
//!
//! Opt-in targets in `<data>/outbound.json` (Admin); none by default, and with none the
//! service runs no thread and does not listen on the bus. A target is a **webhook** (a
//! JSON POST, signed) or a **push** (an ntfy-style plain-text POST of the text). Each
//! target picks the kinds it wants ([`KINDS`]).
//!
//! **Transport (WP SECOUT, PLAN-SEC 4.8, OD-SECe/f).** `https://` is the norm, verified
//! with the `webpki-roots` CAs, a target's own `ca_pem`, or its `pin_sha256` ([`tls`]).
//! `http://` only with `"plaintext": true`, and only to a private, loopback or link-local
//! address, checked on the resolved address of every send; `outbound.get` shows such a
//! target as `"encrypted": false`. Targets written before TLS (`http://` without the
//! flag) load as `plaintext: true` with a one-time notice, kept in the file and shown as
//! `notice` until the next `outbound.set`.
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `outbound.get` | Admin | `targets` (secrets as `set` / `not set`), each with `queued`, `dropped`, `last` |
//! | `outbound.set {targets}` | Admin | `targets`, and `secrets {<id>: <secret>}` for the new ones, in this answer only |
//! | `outbound.test {target}` | Admin | `delivered`, `status?`, `msg?`: one delivery, now |
//!
//! **Where deliveries come from.** A dispatcher thread reads the hub's bus
//! ([`BusMessage::Alarm`] raised or cleared, [`BusMessage::ScheduleRan`]) and queues a
//! delivery for each enabled target that wants its kind. Queuing never blocks: a queue
//! holds [`QUEUE_CAP`] deliveries and drops its oldest past that, counting each in
//! `dropped`. Each target has its own worker thread, so a dead or slow target holds up
//! only itself, never the bus, a device's worker or another target.
//!
//! **Retries.** A failed delivery (no connection, a timeout, a status other than 2xx)
//! is tried again after the target's backoff, 1 s doubling to 5 min, reset by a success;
//! after [`MAX_ATTEMPTS`] tries it is dropped and counted. Nothing is kept across a
//! restart but the targets.
//!
//! **Signature.** Every POST carries `X-DSPi-Timestamp: <unix seconds>` (the time of
//! that try); a target with a secret also `X-DSPi-Signature: sha256=<hex HMAC-SHA256(
//! secret, timestamp + "." + body)>` ([`hmac`]), keyed with the secret's text. Secrets
//! are made by the hub (`"secret": "new"`, 32 random bytes as 64 hex characters), shown
//! once in `outbound.set`'s answer and kept only in the 0600 file.
//!
//! **Bounds.** 16 targets; 64 queued per target; a body's text 1024 characters and at
//! most 64 schedule results; the reply read to its status line within 8 KiB; 5 s to
//! connect and 10 s in all ([`http::HttpLimits`]). A file that cannot be read is set
//! aside as `outbound.json.bad-<time>`; its good entries are kept.
//!
//! Pure state ([`Outbox`], [`Notice`]) and thin glue ([`OutboundService`]): the state is
//! tested without threads or sockets.

pub mod hmac;
pub mod http;
// -- WP SECOUT --
pub mod tls;

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::bus::{Bus, BusMessage, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Role, Session};
use crate::system::alarms::AlarmState;
use http::{HttpLimits, HttpUrl, Post, PostError};
use tls::{TlsError, Trust};

/// The family's cap in `hello.caps`.
pub const CAP: &str = "outbound";
/// The targets' file in the hub's data folder.
pub const FILE_NAME: &str = "outbound.json";
const TEMP_NAME: &str = "outbound.json.tmp";
/// Targets at most.
pub const MAX_TARGETS: usize = 16;
/// Deliveries queued per target at most; the oldest goes past it.
pub const QUEUE_CAP: usize = 64;
/// Tries of one delivery before it is dropped.
pub const MAX_ATTEMPTS: u32 = 10;
/// The first backoff after a failure.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
/// The longest backoff.
pub const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// A target's name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// A target's id, in bytes (`[A-Za-z0-9_-]`).
pub const MAX_ID_BYTES: usize = 32;
/// The kinds a target can pick.
pub const KINDS: [&str; 5] = [
    "alarm.raised",
    "alarm.cleared",
    "restart",
    "schedule.ran",
    "schedule.failed",
];
/// The kind of `outbound.test`'s delivery.
pub const TEST_KIND: &str = "test";
/// What a target written before TLS is told once it has been kept as `plaintext: true`.
pub const MIGRATED_NOTICE: &str = "This target uses http://, which sends unencrypted. It was set up before the hub had TLS and was kept working as \"plaintext\": true. Plain http:// goes only to private, loopback or link-local addresses; anything else is refused. Change the URL to https:// to encrypt it.";

const VERSION: u32 = 1;
const MAX_FILE: u64 = 256 * 1024;
const MAX_TEXT_CHARS: usize = 1024;
const MAX_BODY_RESULTS: usize = 64;
/// The bus carries every device notification too; the dispatcher reads promptly, and a
/// deep queue keeps a burst from pushing an alarm out.
const BUS_CAPACITY: usize = crate::bus::MAX_CAPACITY;
/// How often the dispatcher looks at the stop flag.
const DISPATCH_POLL: Duration = Duration::from_millis(250);
/// A worker with nothing to do still looks again this often.
const IDLE_WAIT: Duration = Duration::from_secs(60);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Targets ─────────────────────────────────────────────────────────────────

/// What a target is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// The JSON body, signed.
    Webhook,
    /// The text alone (`text/plain`), as ntfy takes it.
    Push,
}

impl TargetKind {
    fn name(self) -> &'static str {
        match self {
            TargetKind::Webhook => "webhook",
            TargetKind::Push => "push",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            TargetKind::Webhook => "application/json",
            TargetKind::Push => "text/plain; charset=utf-8",
        }
    }
}

/// One target as `outbound.json` keeps it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Its id (`target` in `outbound.test`).
    pub id: String,
    /// For people.
    pub name: String,
    /// Webhook or push.
    pub kind: TargetKind,
    /// `https://`, or `http://` with [`Target::plaintext`].
    pub url: String,
    /// Off: nothing is queued for it.
    pub enabled: bool,
    /// What it is sent, of [`KINDS`].
    pub kinds: Vec<String>,
    /// The signing secret, 64 hex characters; never in an answer but the one that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    // -- WP SECOUT --
    /// `http://` is allowed (to private, loopback or link-local addresses only, OD-SECe).
    #[serde(default)]
    pub plaintext: bool,
    /// For `https://`: trust these CA or self-signed certificates instead of the
    /// `webpki-roots` CAs (OD-SECf).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<String>,
    /// For `https://`: the receiver's key, `sha256//<base64>` of its SubjectPublicKeyInfo;
    /// replaces the CA and host name check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_sha256: Option<String>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("url", &self.url)
            .field("enabled", &self.enabled)
            .field("kinds", &self.kinds)
            .field("secret", &secret_shown(self.secret.as_ref()))
            .field("plaintext", &self.plaintext)
            .field("ca_pem", &secret_shown(self.ca_pem.as_ref()))
            .field("pin_sha256", &secret_shown(self.pin_sha256.as_ref()))
            .finish()
    }
}

fn secret_shown(secret: Option<&String>) -> &'static str {
    if secret.is_some() { "set" } else { "not set" }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == 64
        && secret
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A stored target's own checks; its parsed URL.
fn check_stored(t: &Target) -> Result<HttpUrl, String> {
    if !valid_id(&t.id) {
        return Err(format!("a bad id {:?}", t.id));
    }
    if t.name.chars().count() > MAX_NAME_CHARS {
        return Err("a name too long".into());
    }
    if let Some(k) = t.kinds.iter().find(|k| !KINDS.contains(&k.as_str())) {
        return Err(format!("an unknown kind {k:?}"));
    }
    if t.secret.as_deref().is_some_and(|s| !valid_secret(s)) {
        return Err("a malformed secret".into());
    }
    let url = http::parse_url(&t.url).map_err(|e| e.to_string())?;
    check_transport(
        &url,
        t.plaintext,
        t.ca_pem.as_deref(),
        t.pin_sha256.as_deref(),
    )?;
    Ok(url)
}

// -- WP SECOUT --
/// OD-SECe/f for one target: `https://` with at most one of `ca_pem` and `pin_sha256`, or
/// `http://` with `plaintext` and neither. The trust it makes. Where an `http://` body may
/// go is checked on every send ([`http::post`]).
fn check_transport(
    url: &HttpUrl,
    plaintext: bool,
    ca_pem: Option<&str>,
    pin: Option<&str>,
) -> Result<Trust, String> {
    if url.tls {
        if plaintext {
            return Err("plaintext is for http:// URLs only".into());
        }
        return Trust::from_settings(ca_pem, pin).map_err(|e| e.to_string());
    }
    if !plaintext {
        return Err("http:// sends unencrypted: use https://, or set \"plaintext\": true for a receiver on a private, loopback or link-local address".into());
    }
    if ca_pem.is_some() || pin.is_some() {
        return Err("ca_pem and pin_sha256 are for https:// URLs only".into());
    }
    Ok(Trust::Roots)
}

/// `outbound.set`'s further rule: an `http://` IP literal that is not private, loopback or
/// link-local is refused at once (a stored one is kept, and refused on every send).
fn check_public_literal(url: &HttpUrl) -> Result<(), String> {
    match url.host.parse::<std::net::IpAddr>() {
        Ok(ip) if !url.tls && !tls::plaintext_allowed(ip) => Err(format!(
            "http:// to {ip} would cross the internet unencrypted: plaintext goes only to private, loopback or link-local addresses; use https://"
        )),
        _ => Ok(()),
    }
}

/// The trust of a target that passed [`check_transport`].
fn trust_of(t: &Target) -> Result<Trust, TlsError> {
    Trust::from_settings(t.ca_pem.as_deref(), t.pin_sha256.as_deref())
}

/// A target written before TLS (`http://` without `plaintext: true`) is kept working as
/// `plaintext: true` (OD-SECo's rule for webhooks): its id, if it was.
fn migrate(entry: &mut Value) -> Option<String> {
    let obj = entry.as_object_mut()?;
    let url = obj.get("url")?.as_str()?;
    if url.get(..7).map(str::to_ascii_lowercase).as_deref() != Some("http://") {
        return None;
    }
    if obj.get("plaintext") == Some(&Value::Bool(true)) {
        return None;
    }
    obj.insert("plaintext".into(), Value::Bool(true));
    Some(obj.get("id")?.as_str()?.to_owned())
}

fn new_secret() -> Result<String, crate::HubError> {
    let bytes: [u8; 32] = crate::store::random_bytes()?;
    Ok(HEXLOWER.encode(&bytes))
}

// ── The file ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct FileData {
    v: u32,
    targets: Vec<Value>,
}

fn read_limited(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FILE.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too large"));
    }
    Ok(bytes)
}

/// The good entries of `bytes`, whether any was bad, and the ids migrated to
/// `plaintext: true`.
fn decode(bytes: &[u8]) -> (Vec<(Target, HttpUrl)>, Option<String>, Vec<String>) {
    let data: FileData = match serde_json::from_slice(bytes) {
        Ok(d) => d,
        Err(e) => return (Vec::new(), Some(e.to_string()), Vec::new()),
    };
    let mut good: Vec<(Target, HttpUrl)> = Vec::new();
    let mut bad = None;
    let mut migrated = Vec::new();
    for (i, mut entry) in data.targets.into_iter().enumerate() {
        let was = migrate(&mut entry);
        let checked = serde_json::from_value::<Target>(entry)
            .map_err(|e| e.to_string())
            .and_then(|t| check_stored(&t).map(|u| (t, u)));
        match checked {
            Ok((t, _)) if good.iter().any(|(g, _)| g.id == t.id) => {
                bad = Some(format!("target {i}: the id {:?} twice", t.id));
            }
            Ok(_) if good.len() >= MAX_TARGETS => {
                bad = Some(format!("more than {MAX_TARGETS} targets"));
            }
            Ok(pair) => {
                if let Some(id) = was {
                    migrated.push(id);
                }
                good.push(pair);
            }
            Err(why) => bad = Some(format!("target {i}: {why}")),
        }
    }
    (good, bad, migrated)
}

/// Read the targets. A file that is not whole is set aside as `.bad-<unix seconds>`
/// and its good entries written back; never a panic, never a failed start.
#[cfg(test)]
fn load(dir: &Path, unix_seconds: u64) -> Vec<(Target, HttpUrl)> {
    load_migrating(dir, unix_seconds).0
}

/// [`load`], and the ids of the targets written before TLS that were kept as
/// `plaintext: true` (the file is written back with the flag, so this happens once).
fn load_migrating(dir: &Path, unix_seconds: u64) -> (Vec<(Target, HttpUrl)>, Vec<String>) {
    let path = dir.join(FILE_NAME);
    let (good, bad, migrated) = match read_limited(&path) {
        Ok(bytes) => decode(&bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => (Vec::new(), Some(e.to_string()), Vec::new()),
    };
    // -- WP SECOUT --
    for id in &migrated {
        log::warn!(
            "hub: outbound target {id:?} uses http://, set up before TLS: kept as \"plaintext\": true (unencrypted, private addresses only); change it to https:// to encrypt it"
        );
    }
    if bad.is_none() && !migrated.is_empty() {
        let targets: Vec<Target> = good.iter().map(|(t, _)| t.clone()).collect();
        if let Err(e) = save(dir, &targets) {
            log::warn!("hub: cannot write {}: {e}", path.display());
        }
    }
    if let Some(why) = bad {
        let aside = dir.join(format!("{FILE_NAME}.bad-{unix_seconds}"));
        log::warn!(
            "hub: {} set aside as {} ({why}); {} target(s) kept",
            path.display(),
            aside.display(),
            good.len()
        );
        if let Err(e) = fs::rename(&path, &aside) {
            log::warn!("hub: cannot set {} aside: {e}", path.display());
        }
        let targets: Vec<Target> = good.iter().map(|(t, _)| t.clone()).collect();
        if let Err(e) = save(dir, &targets) {
            log::warn!("hub: cannot write {}: {e}", path.display());
        }
    }
    (good, migrated)
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// Write the targets atomically, 0600.
fn save(dir: &Path, targets: &[Target]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let data = json!({"v": VERSION, "targets": targets});
    let bytes = serde_json::to_vec_pretty(&data).map_err(io::Error::other)?;
    let temp = dir.join(TEMP_NAME);
    let mut file = private_file(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, dir.join(FILE_NAME))?;
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

// ── What is sent ────────────────────────────────────────────────────────────

/// ISO-8601 UTC with a `Z` (spec 7.1), from seconds since the epoch. The same as
/// `server.rs`'s, which is private there.
pub fn iso8601(secs: u64) -> String {
    // Howard Hinnant's civil_from_days. Every step is within i64 for any u64 of seconds
    // divided down to days, and the divisors are constants.
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

fn clip_text(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

/// One thing to tell the targets, made from a bus message.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// The kinds it is, best first: a target gets it once, as the first it wants.
    pub kinds: Vec<&'static str>,
    /// The device, where it is about one.
    pub serial: Option<String>,
    /// When it happened, Unix seconds.
    pub at: u64,
    /// One line for a person; what a push target is sent.
    pub text: String,
    /// The alarm object (hub-api.md section 7), for an alarm.
    pub alarm: Option<Value>,
    /// `{rule, name, by?, results, results_total}`, for a schedule.
    pub schedule: Option<Value>,
}

impl Notice {
    /// What a bus message tells the targets, if anything.
    pub fn from_bus(message: &BusMessage) -> Option<Self> {
        match message {
            BusMessage::Alarm {
                id,
                source,
                code,
                subject,
                serial,
                severity,
                state,
                message,
                at,
                raised_at,
                cleared_at,
                acknowledged_by,
                count,
            } => {
                let (kinds, word) = match state {
                    AlarmState::Raised if code == "device_restarted" => {
                        (vec!["restart", "alarm.raised"], "Alarm")
                    }
                    AlarmState::Raised => (vec!["alarm.raised"], "Alarm"),
                    AlarmState::Cleared => (vec!["alarm.cleared"], "Cleared"),
                    AlarmState::Acknowledged => return None,
                };
                let mut alarm = json!({
                    "id": id,
                    "source": source,
                    "code": code,
                    "subject": subject,
                    "severity": severity,
                    "message": clip_text(message),
                    "state": state,
                    "raised_at": iso8601(*raised_at),
                    "count": count,
                });
                if let Some(map) = alarm.as_object_mut() {
                    if let Some(s) = serial {
                        map.insert("serial".into(), s.clone().into());
                    }
                    if let Some(c) = cleared_at {
                        map.insert("cleared_at".into(), iso8601(*c).into());
                    }
                    if let Some(by) = acknowledged_by {
                        map.insert("acknowledged_by".into(), by.clone().into());
                    }
                }
                let about = serial.as_deref().unwrap_or(subject);
                Some(Self {
                    kinds,
                    serial: serial.clone(),
                    at: *at,
                    text: clip_text(&format!("{word} on {about}: {message}")),
                    alarm: Some(alarm),
                    schedule: None,
                })
            }
            BusMessage::ScheduleRan {
                rule,
                name,
                at,
                by,
                results,
            } => {
                let ok = results.iter().filter(|r| r.ok).count();
                let failed = ok < results.len();
                let kinds = if failed {
                    vec!["schedule.failed", "schedule.ran"]
                } else {
                    vec!["schedule.ran"]
                };
                let shown: Vec<_> = results.iter().take(MAX_BODY_RESULTS).collect();
                let mut schedule = json!({
                    "rule": rule,
                    "name": name,
                    "results": shown,
                    "results_total": results.len(),
                });
                if let (Some(by), Some(map)) = (by, schedule.as_object_mut()) {
                    map.insert("by".into(), by.clone().into());
                }
                let verb = if failed { "failed" } else { "ran" };
                Some(Self {
                    kinds,
                    serial: None,
                    at: *at,
                    text: clip_text(&format!(
                        "Schedule {name} {verb}: {ok} of {} actions went through",
                        results.len()
                    )),
                    alarm: None,
                    schedule: Some(schedule),
                })
            }
            _ => None,
        }
    }

    /// `outbound.test`'s delivery.
    pub fn test(at: u64) -> Self {
        Self {
            kinds: vec![TEST_KIND],
            serial: None,
            at,
            text: "A test from the DSPi hub".into(),
            alarm: None,
            schedule: None,
        }
    }

    /// A webhook's body: `{v, hub, kind, serial?, at, text, alarm?, schedule?}`.
    pub fn body(&self, hub: &str, kind: &str) -> String {
        let mut m = Map::new();
        m.insert("v".into(), VERSION.into());
        m.insert("hub".into(), hub.into());
        m.insert("kind".into(), kind.into());
        if let Some(s) = &self.serial {
            m.insert("serial".into(), s.clone().into());
        }
        m.insert("at".into(), iso8601(self.at).into());
        m.insert("text".into(), self.text.clone().into());
        if let Some(a) = &self.alarm {
            m.insert("alarm".into(), a.clone());
        }
        if let Some(s) = &self.schedule {
            m.insert("schedule".into(), s.clone());
        }
        Value::Object(m).to_string()
    }

    /// What a target of `kind` is sent, as `kind`.
    fn payload(&self, target: TargetKind, hub: &str, kind: &str) -> Arc<str> {
        match target {
            TargetKind::Webhook => Arc::from(self.body(hub, kind)),
            TargetKind::Push => Arc::from(self.text.as_str()),
        }
    }
}

// ── The state ───────────────────────────────────────────────────────────────

/// How one try went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A 2xx.
    Delivered(u16),
    /// Another status.
    Refused(u16),
    /// No status: no connection, a timeout, not HTTP.
    Failed(String),
}

impl Outcome {
    /// From what [`http::post`] answered.
    pub fn from_post(result: Result<u16, PostError>) -> Self {
        match result {
            Ok(s) if (200..300).contains(&s) => Outcome::Delivered(s),
            Ok(s) => Outcome::Refused(s),
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }

    /// Whether it went through.
    pub fn ok(&self) -> bool {
        matches!(self, Outcome::Delivered(_))
    }

    fn status(&self) -> Option<u16> {
        match self {
            Outcome::Delivered(s) | Outcome::Refused(s) => Some(*s),
            Outcome::Failed(_) => None,
        }
    }

    fn msg(&self) -> Option<String> {
        match self {
            Outcome::Delivered(_) => None,
            Outcome::Refused(s) => Some(format!("the receiver answered {s}")),
            Outcome::Failed(why) => Some(why.clone()),
        }
    }
}

/// A target's last try, as `outbound.get` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Last {
    /// When, ISO-8601.
    pub at: String,
    /// Whether it went through.
    pub ok: bool,
    /// The receiver's status, if it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Why not, for a person.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

impl Last {
    fn of(outcome: &Outcome, at: u64) -> Self {
        Self {
            at: iso8601(at),
            ok: outcome.ok(),
            status: outcome.status(),
            msg: outcome.msg(),
        }
    }
}

/// One queued delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The kind it is sent as.
    pub kind: &'static str,
    /// What is sent.
    pub body: Arc<str>,
    /// Tries so far.
    pub attempts: u32,
}

/// A delivery to make now, with where to.
#[derive(Debug, Clone)]
pub struct Job {
    /// The target's slot.
    pub epoch: u64,
    /// Where.
    pub url: HttpUrl,
    /// Webhook or push.
    pub target: TargetKind,
    /// Its secret, to sign with.
    pub secret: Option<String>,
    /// What.
    pub delivery: Delivery,
    // -- WP SECOUT --
    /// What an `https://` delivery trusts.
    pub trust: Result<Trust, TlsError>,
}

/// What a worker is to do.
#[derive(Debug)]
pub enum Take {
    /// Its target is gone: stop.
    Gone,
    /// Nothing now; look again at this time (hub clock), or when woken.
    Wait(Option<Instant>),
    /// Deliver this.
    Ready(Job),
}

#[derive(Debug)]
struct Slot {
    target: Target,
    url: HttpUrl,
    /// Which slot, for its worker: a target removed and added again is a new one.
    epoch: u64,
    queue: VecDeque<Delivery>,
    dropped: u64,
    in_flight: bool,
    not_before: Option<Instant>,
    backoff: Duration,
    last: Option<Last>,
    worker: bool,
    // -- WP SECOUT --
    /// A one-time notice for the Admin (a target kept as `plaintext: true` on load),
    /// shown by `outbound.get` until the next `outbound.set`.
    notice: Option<String>,
}

/// What `outbound.test` needs of a target: where, what, its secret and its trust.
type Found = (HttpUrl, TargetKind, Option<String>, Result<Trust, TlsError>);

/// Every target's queue, backoff and last result. No threads, no sockets, no clock of
/// its own: the caller passes the time.
#[derive(Debug, Default)]
pub struct Outbox {
    slots: Vec<Slot>,
    next_epoch: u64,
}

impl Outbox {
    /// The targets, as kept.
    pub fn targets(&self) -> Vec<Target> {
        self.slots.iter().map(|s| s.target.clone()).collect()
    }

    /// Whether there is no target.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Replace the targets. A target whose id stays keeps its queue, counts and backoff;
    /// one that is now disabled loses its queue; one left out is gone (its worker stops).
    /// A kept target whose kind changed drops its queue (counted in `dropped`: bodies made
    /// for the other kind are never sent), and one whose destination changed (its URL or
    /// what it trusts) starts its backoff again, so a corrected URL is tried at once.
    pub fn set_targets(&mut self, targets: Vec<(Target, HttpUrl)>) {
        let mut old = std::mem::take(&mut self.slots);
        for (target, url) in targets {
            let slot = match old.iter().position(|s| s.target.id == target.id) {
                Some(i) => {
                    let mut s = old.swap_remove(i);
                    // -- WP SECOUT --
                    if s.target.kind != target.kind {
                        let n = u64::try_from(s.queue.len()).unwrap_or(u64::MAX);
                        s.dropped = s.dropped.saturating_add(n);
                        s.queue.clear();
                    }
                    let moved = s.target.url != target.url
                        || s.target.plaintext != target.plaintext
                        || s.target.ca_pem != target.ca_pem
                        || s.target.pin_sha256 != target.pin_sha256;
                    if moved {
                        s.backoff = BACKOFF_FIRST;
                        s.not_before = None;
                    }
                    s.notice = None;
                    s.target = target;
                    s.url = url;
                    if !s.target.enabled {
                        s.queue.clear();
                    }
                    s
                }
                None => {
                    self.next_epoch = self.next_epoch.wrapping_add(1);
                    Slot {
                        target,
                        url,
                        epoch: self.next_epoch,
                        queue: VecDeque::new(),
                        dropped: 0,
                        in_flight: false,
                        not_before: None,
                        backoff: BACKOFF_FIRST,
                        last: None,
                        worker: false,
                        notice: None,
                    }
                }
            };
            self.slots.push(slot);
        }
    }

    /// Queue `notice` for every enabled target that wants one of its kinds; answers
    /// how many were queued. Past [`QUEUE_CAP`] a queue drops its oldest.
    pub fn enqueue(&mut self, notice: &Notice, hub: &str) -> usize {
        let mut queued = 0usize;
        for s in self.slots.iter_mut().filter(|s| s.target.enabled) {
            let Some(kind) = notice
                .kinds
                .iter()
                .copied()
                .find(|k| s.target.kinds.iter().any(|w| w == k))
            else {
                continue;
            };
            while s.queue.len() >= QUEUE_CAP {
                s.queue.pop_front();
                s.dropped = s.dropped.saturating_add(1);
            }
            s.queue.push_back(Delivery {
                kind,
                body: notice.payload(s.target.kind, hub, kind),
                attempts: 0,
            });
            queued = queued.saturating_add(1);
        }
        queued
    }

    /// The next delivery of the slot `epoch`, if it is due at `now`.
    pub fn take(&mut self, epoch: u64, now: Instant) -> Take {
        let Some(s) = self.slots.iter_mut().find(|s| s.epoch == epoch) else {
            return Take::Gone;
        };
        if s.in_flight || s.queue.is_empty() {
            return Take::Wait(None);
        }
        if let Some(due) = s.not_before.filter(|due| *due > now) {
            return Take::Wait(Some(due));
        }
        let Some(delivery) = s.queue.pop_front() else {
            return Take::Wait(None);
        };
        s.in_flight = true;
        Take::Ready(Job {
            epoch,
            url: s.url.clone(),
            target: s.target.kind,
            secret: s.target.secret.clone(),
            delivery,
            trust: trust_of(&s.target),
        })
    }

    /// A try of `job` ended with `outcome` at `now` (hub clock; `at` Unix seconds). A
    /// failure backs the target off and queues the delivery again at the front, unless
    /// it has had [`MAX_ATTEMPTS`] or the queue filled meanwhile (then it is dropped).
    pub fn finish(&mut self, job: Job, outcome: &Outcome, now: Instant, at: u64) {
        let Some(s) = self.slots.iter_mut().find(|s| s.epoch == job.epoch) else {
            return;
        };
        s.in_flight = false;
        s.last = Some(Last::of(outcome, at));
        if outcome.ok() {
            s.backoff = BACKOFF_FIRST;
            s.not_before = None;
            return;
        }
        s.not_before = Some(now.checked_add(s.backoff).unwrap_or(now));
        s.backoff = s.backoff.saturating_mul(2).min(BACKOFF_MAX);
        let mut delivery = job.delivery;
        delivery.attempts = delivery.attempts.saturating_add(1);
        // A body made for the kind the target had when it was taken is not sent as another.
        let other_kind = job.target != s.target.kind;
        if delivery.attempts >= MAX_ATTEMPTS
            || s.queue.len() >= QUEUE_CAP
            || !s.target.enabled
            || other_kind
        {
            s.dropped = s.dropped.saturating_add(1);
        } else {
            s.queue.push_front(delivery);
        }
    }

    /// Show `notice` on the target `id` until the next `outbound.set`.
    pub fn set_notice(&mut self, id: &str, notice: &str) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.target.id == id) {
            s.notice = Some(notice.to_owned());
        }
    }

    /// `outbound.test`'s result, as the target's last.
    pub fn record(&mut self, id: &str, outcome: &Outcome, at: u64) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.target.id == id) {
            s.last = Some(Last::of(outcome, at));
        }
    }

    /// The earliest time a backed-off target with work and a worker is due, later than
    /// `after`.
    pub fn next_due(&self, after: Option<Instant>) -> Option<Instant> {
        self.slots
            .iter()
            .filter(|s| s.worker && !s.in_flight && !s.queue.is_empty())
            .filter_map(|s| s.not_before)
            .filter(|due| after.is_none_or(|a| *due > a))
            .min()
    }

    /// Queued (and in flight) and dropped, of the target `id`.
    pub fn counts(&self, id: &str) -> Option<(usize, u64)> {
        self.slots.iter().find(|s| s.target.id == id).map(|s| {
            (
                s.queue.len().saturating_add(usize::from(s.in_flight)),
                s.dropped,
            )
        })
    }

    /// The queued deliveries of the target `id`, oldest first.
    pub fn queued(&self, id: &str) -> Vec<Delivery> {
        self.slots
            .iter()
            .find(|s| s.target.id == id)
            .map(|s| s.queue.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// What `outbound.get` shows: no secret.
    pub fn view(&self) -> Vec<Value> {
        self.slots
            .iter()
            .map(|s| {
                let t = &s.target;
                let mut v = json!({
                    "id": t.id,
                    "name": t.name,
                    "kind": t.kind.name(),
                    "url": t.url,
                    "enabled": t.enabled,
                    "kinds": t.kinds,
                    "secret": secret_shown(t.secret.as_ref()),
                    "queued": s.queue.len().saturating_add(usize::from(s.in_flight)),
                    "dropped": s.dropped,
                    "last": s.last,
                });
                // -- WP SECOUT --
                if let Some(m) = v.as_object_mut() {
                    m.insert("encrypted".into(), s.url.tls.into());
                    m.insert("plaintext".into(), t.plaintext.into());
                    if s.url.tls {
                        let trust = trust_of(t).map_or("invalid", |tr| tr.name());
                        m.insert("trust".into(), trust.into());
                    }
                    if let Some(pem) = &t.ca_pem {
                        m.insert("ca_pem".into(), pem.clone().into());
                    }
                    if let Some(pin) = &t.pin_sha256 {
                        m.insert("pin_sha256".into(), pin.clone().into());
                    }
                    if let Some(n) = &s.notice {
                        m.insert("notice".into(), n.clone().into());
                    }
                }
                v
            })
            .collect()
    }

    /// Slots that have no worker yet, now marked as having one.
    fn needing_workers(&mut self) -> Vec<u64> {
        self.slots
            .iter_mut()
            .filter(|s| !s.worker)
            .map(|s| {
                s.worker = true;
                s.epoch
            })
            .collect()
    }

    fn worker_failed(&mut self, epoch: u64) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.epoch == epoch) {
            s.worker = false;
        }
    }

    fn find(&self, id: &str) -> Option<Found> {
        self.slots.iter().find(|s| s.target.id == id).map(|s| {
            (
                s.url.clone(),
                s.target.kind,
                s.target.secret.clone(),
                trust_of(&s.target),
            )
        })
    }
}

// ── `outbound.set` ──────────────────────────────────────────────────────────

enum SecretAsk {
    Keep,
    New,
    Clear,
}

struct Entry {
    id: Option<String>,
    name: Option<String>,
    kind: TargetKind,
    url: String,
    parsed: HttpUrl,
    enabled: bool,
    kinds: Vec<String>,
    secret: SecretAsk,
    plaintext: bool,
    ca_pem: Option<String>,
    pin_sha256: Option<String>,
}

fn parse_entry(e: &Value) -> Result<Entry, String> {
    let obj = e.as_object().ok_or("not an object")?;
    let text = |key: &str| -> Result<Option<String>, String> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{key} is not a string")),
        }
    };
    let id = text("id")?;
    if let Some(id) = &id
        && !valid_id(id)
    {
        return Err(format!(
            "id must be 1 to {MAX_ID_BYTES} of A-Z, a-z, 0-9, _ and -"
        ));
    }
    let name = text("name")?.map(|n| n.trim().to_owned());
    if name
        .as_deref()
        .is_some_and(|n| n.chars().count() > MAX_NAME_CHARS)
    {
        return Err(format!("name is longer than {MAX_NAME_CHARS} characters"));
    }
    let kind = match text("kind")?.as_deref() {
        Some("webhook") => TargetKind::Webhook,
        Some("push") => TargetKind::Push,
        _ => return Err("kind must be webhook or push".into()),
    };
    let url = text("url")?.ok_or("url is missing")?;
    let parsed = http::parse_url(&url).map_err(|e| format!("url: {e}"))?;
    let enabled = match obj.get("enabled") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("enabled is not a bool".into()),
    };
    let kinds = match obj.get("kinds") {
        None => KINDS.iter().map(|k| (*k).to_owned()).collect(),
        Some(Value::Array(list)) => {
            let mut kinds: Vec<String> = Vec::new();
            for k in list {
                let k = k
                    .as_str()
                    .filter(|k| KINDS.contains(k))
                    .ok_or_else(|| format!("kinds: each of {}", KINDS.join(", ")))?;
                if !kinds.iter().any(|have| have == k) {
                    kinds.push(k.to_owned());
                }
            }
            kinds
        }
        Some(_) => return Err("kinds is not a list".into()),
    };
    let secret = match obj.get("secret") {
        None => SecretAsk::Keep,
        Some(Value::String(s)) if s == "set" => SecretAsk::Keep,
        Some(Value::String(s)) if s == "new" => SecretAsk::New,
        Some(Value::Null) => SecretAsk::Clear,
        Some(Value::String(s)) if s == "not set" => SecretAsk::Clear,
        Some(_) => {
            return Err("secret is made by the hub: \"new\", \"set\" (keep) or \"not set\"".into());
        }
    };
    // -- WP SECOUT --
    let plaintext = match obj.get("plaintext") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("plaintext is not a bool".into()),
    };
    let ca_pem = text("ca_pem")?.filter(|p| !p.trim().is_empty());
    let pin_sha256 = text("pin_sha256")?
        .filter(|p| !p.trim().is_empty())
        .map(|p| tls::parse_pin(&p).map(|pin| tls::pin_text(&pin)))
        .transpose()
        .map_err(|e| e.to_string())?;
    check_transport(&parsed, plaintext, ca_pem.as_deref(), pin_sha256.as_deref())
        .and_then(|_| check_public_literal(&parsed))
        .map_err(|why| format!("url: {why}"))?;
    Ok(Entry {
        id,
        name,
        kind,
        url,
        parsed,
        enabled,
        kinds,
        secret,
        plaintext,
        ca_pem,
        pin_sha256,
    })
}

/// The new targets for `outbound.set`'s list, against the current ones; and the new
/// secrets by id. `Err((index, why))` is a `bad_request`.
type Planned = (Vec<(Target, HttpUrl)>, Map<String, Value>);

fn plan(list: &[Value], current: &[Target]) -> Result<Planned, (Option<usize>, String)> {
    if list.len() > MAX_TARGETS {
        return Err((None, format!("at most {MAX_TARGETS} targets")));
    }
    let mut entries = Vec::with_capacity(list.len());
    for (i, e) in list.iter().enumerate() {
        entries.push(parse_entry(e).map_err(|why| (Some(i), why))?);
    }
    let mut used: Vec<String> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(id) = &e.id {
            if used.contains(id) {
                return Err((Some(i), format!("the id {id:?} twice")));
            }
            used.push(id.clone());
        }
    }
    let mut targets = Vec::with_capacity(entries.len());
    let mut secrets = Map::new();
    for e in entries {
        let id = match e.id {
            Some(id) => id,
            None => {
                let id = (1..=MAX_TARGETS.saturating_add(list.len()))
                    .map(|n| format!("t{n}"))
                    .find(|id| !used.contains(id))
                    .unwrap_or_else(|| "t0".into());
                used.push(id.clone());
                id
            }
        };
        let secret = match e.secret {
            SecretAsk::Keep => current
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.secret.clone()),
            SecretAsk::Clear => None,
            SecretAsk::New => {
                let s = new_secret().map_err(|err| (None, err.to_string()))?;
                secrets.insert(id.clone(), s.clone().into());
                Some(s)
            }
        };
        let name = e
            .name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone());
        targets.push((
            Target {
                id,
                name,
                kind: e.kind,
                url: e.url,
                enabled: e.enabled,
                kinds: e.kinds,
                secret,
                plaintext: e.plaintext,
                ca_pem: e.ca_pem,
                pin_sha256: e.pin_sha256,
            },
            e.parsed,
        ));
    }
    Ok((targets, secrets))
}

// ── The service ─────────────────────────────────────────────────────────────

struct Shared {
    clock: Arc<dyn HubClock>,
    bus: Arc<Bus>,
    data_dir: Option<PathBuf>,
    limits: HttpLimits,
    state: Mutex<Outbox>,
    ready: Condvar,
    stopped: AtomicBool,
    hub_id: OnceLock<String>,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
    /// The hub clock's time at the last `run_due`, so a due time it has woken the
    /// workers for is not due again.
    last_run: Mutex<Option<Instant>>,
    /// When the service was made: its first due time while the hub's id is unknown
    /// (a fixed time, so it is not later than the timer's `now`).
    born: Instant,
}

/// The `outbound` family: the targets, their queues and the threads that deliver.
pub struct OutboundService {
    shared: Arc<Shared>,
}

/// The service, for `service.rs`'s list. Always on: with no target it does nothing.
pub fn service(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Option<Arc<dyn HubService>> {
    let _ = config;
    Some(Arc::new(OutboundService::new(
        setup.data_dir,
        Arc::clone(setup.clock),
        Arc::clone(setup.bus),
        HttpLimits::default(),
    )))
}

fn reply_ok(id: &Value, mut fields: Map<String, Value>) -> String {
    fields.insert("t".into(), "ok".into());
    fields.insert("id".into(), id.clone());
    fields.insert("v".into(), VERSION.into());
    Value::Object(fields).to_string()
}

fn reply_err(id: &Value, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

fn deliver(job: &Job, at: u64, limits: &HttpLimits) -> Outcome {
    // -- WP SECOUT --
    let config = if job.url.tls {
        let made = job
            .trust
            .clone()
            .and_then(|trust| tls::client_config(&trust, None, &[http::ALPN]));
        match made {
            Ok(c) => Some(c),
            Err(e) => return Outcome::Failed(e.to_string()),
        }
    } else {
        None
    };
    let body = job.delivery.body.as_bytes();
    let mut headers = vec![("X-DSPi-Timestamp", at.to_string())];
    if let Some(secret) = &job.secret {
        headers.push(("X-DSPi-Signature", hmac::signature(secret, at, body)));
    }
    let post = Post {
        url: &job.url,
        tls: config.as_ref(),
        content_type: job.target.content_type(),
        headers: &headers,
        body,
    };
    Outcome::from_post(http::post(&post, limits))
}

impl OutboundService {
    /// The service, its targets read from `data_dir` (`None`: kept for this run only).
    /// No thread starts until the hub's id is known and a target exists.
    pub fn new(
        data_dir: Option<&Path>,
        clock: Arc<dyn HubClock>,
        bus: Arc<Bus>,
        limits: HttpLimits,
    ) -> Self {
        let mut outbox = Outbox::default();
        if let Some(dir) = data_dir {
            let (targets, migrated) = load_migrating(dir, clock.unix_seconds());
            outbox.set_targets(targets);
            for id in migrated {
                outbox.set_notice(&id, MIGRATED_NOTICE);
            }
        }
        let born = clock.now();
        Self {
            shared: Arc::new(Shared {
                clock,
                bus,
                data_dir: data_dir.map(Path::to_owned),
                limits,
                state: Mutex::new(outbox),
                ready: Condvar::new(),
                stopped: AtomicBool::new(false),
                hub_id: OnceLock::new(),
                dispatcher: Mutex::new(None),
                last_run: Mutex::new(None),
                born,
            }),
        }
    }

    /// Wake every worker, to look at its queue and the clock again.
    pub fn kick(&self) {
        self.shared.ready.notify_all();
    }

    /// Queued (and in flight) and dropped, of the target `id`.
    pub fn counts(&self, id: &str) -> Option<(usize, u64)> {
        lock(&self.shared.state).counts(id)
    }

    /// Start what is not running: the dispatcher once a target exists, a worker per target.
    fn ensure_running(&self, hub_id: &str) {
        let shared = &self.shared;
        let _ = shared.hub_id.set(hub_id.to_owned());
        if shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        let epochs = {
            let mut st = lock(&shared.state);
            if st.is_empty() {
                return;
            }
            st.needing_workers()
        };
        for epoch in epochs {
            let s = Arc::clone(shared);
            let spawned = thread::Builder::new()
                .name("hub-outbound-target".into())
                .spawn(move || worker(&s, epoch));
            if let Err(e) = spawned {
                log::error!("hub: an outbound worker did not start: {e}");
                lock(&shared.state).worker_failed(epoch);
            }
        }
        let mut dispatcher = lock(&shared.dispatcher);
        if dispatcher.is_none() {
            let sub = shared.bus.subscribe("outbound", BUS_CAPACITY);
            let s = Arc::clone(shared);
            match thread::Builder::new()
                .name("hub-outbound".into())
                .spawn(move || dispatch(&s, &sub))
            {
                Ok(t) => *dispatcher = Some(t),
                Err(e) => log::error!("hub: the outbound dispatcher did not start: {e}"),
            }
        }
    }

    fn get(&self, id: &Value) -> String {
        let targets = lock(&self.shared.state).view();
        let mut m = Map::new();
        m.insert("targets".into(), targets.into());
        reply_ok(id, m)
    }

    fn set(&self, hub: &HubHandle, id: &Value, msg: &Value) -> String {
        let Some(list) = msg.get("targets").and_then(Value::as_array) else {
            return reply_err(id, "bad_request", "targets: the whole list of targets");
        };
        let mut st = lock(&self.shared.state);
        let (targets, secrets) = match plan(list, &st.targets()) {
            Ok(p) => p,
            Err((index, why)) => {
                let msg = match index {
                    Some(i) => format!("targets[{i}]: {why}"),
                    None => why,
                };
                let mut v =
                    json!({"t": "err", "id": id, "v": VERSION, "code": "bad_request", "msg": msg});
                if let (Some(i), Some(map)) = (index, v.as_object_mut()) {
                    map.insert("target".into(), i.into());
                }
                return v.to_string();
            }
        };
        if let Some(dir) = &self.shared.data_dir {
            let plain: Vec<Target> = targets.iter().map(|(t, _)| t.clone()).collect();
            if let Err(e) = save(dir, &plain) {
                log::error!("hub: cannot write {}: {e}", dir.join(FILE_NAME).display());
                return reply_err(id, "internal", "the targets could not be saved");
            }
        }
        st.set_targets(targets);
        let view = st.view();
        drop(st);
        self.ensure_running(hub.hub_id());
        self.kick();
        let mut m = Map::new();
        m.insert("targets".into(), view.into());
        m.insert("secrets".into(), Value::Object(secrets));
        reply_ok(id, m)
    }

    fn test(&self, hub: &HubHandle, id: &Value, msg: &Value) -> String {
        let Some(target) = msg.get("target").and_then(Value::as_str) else {
            return reply_err(id, "bad_request", "target: a target's id");
        };
        let Some((url, kind, secret, trust)) = lock(&self.shared.state).find(target) else {
            return reply_err(id, "not_found", "no such target");
        };
        let at = self.shared.clock.unix_seconds();
        let notice = Notice::test(at);
        let job = Job {
            epoch: 0,
            url,
            target: kind,
            secret,
            delivery: Delivery {
                kind: TEST_KIND,
                body: notice.payload(kind, hub.hub_id(), TEST_KIND),
                attempts: 0,
            },
            trust,
        };
        // On the asker's connection alone, at most the total limit.
        let outcome = deliver(&job, at, &self.shared.limits);
        lock(&self.shared.state).record(target, &outcome, self.shared.clock.unix_seconds());
        let mut m = Map::new();
        m.insert("delivered".into(), outcome.ok().into());
        if let Some(s) = outcome.status() {
            m.insert("status".into(), s.into());
        }
        if let Some(why) = outcome.msg() {
            m.insert("msg".into(), why.into());
        }
        reply_ok(id, m)
    }
}

impl HubService for OutboundService {
    fn name(&self) -> &'static str {
        "outbound"
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
        if !matches!(t, "outbound.get" | "outbound.set" | "outbound.test") {
            return None;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        if session.role() != Role::Admin {
            return Some(reply_err(
                &id,
                "denied",
                "notification targets are an admin's",
            ));
        }
        Some(match t {
            "outbound.get" => self.get(&id),
            "outbound.set" => self.set(hub, &id, msg),
            _ => self.test(hub, &id, msg),
        })
    }

    fn next_due(&self) -> Option<Instant> {
        let shared = &self.shared;
        if shared.stopped.load(Ordering::SeqCst) {
            return None;
        }
        let st = lock(&shared.state);
        if shared.hub_id.get().is_none() && !st.is_empty() {
            // Targets from the file: start once the hub's id is known (`run_due`).
            return Some(shared.born);
        }
        st.next_due(*lock(&shared.last_run))
    }

    fn run_due(&self, hub: &HubHandle, now: Instant) {
        *lock(&self.shared.last_run) = Some(now);
        self.ensure_running(hub.hub_id());
        self.kick();
    }

    fn stop(&self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.kick();
        let dispatcher = lock(&self.shared.dispatcher).take();
        if let Some(t) = dispatcher {
            let _ = t.join();
        }
        // The workers end on their own: at once if idle, else after their try (at most
        // the total limit). Joining them would hold the hub's stop up for that.
    }
}

impl Drop for OutboundService {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.shared.ready.notify_all();
    }
}

/// The dispatcher: from the bus into the queues. Never waits on a target.
fn dispatch(shared: &Shared, sub: &Subscription) {
    while !shared.stopped.load(Ordering::SeqCst) {
        if let Some(m) = sub.recv_timeout(DISPATCH_POLL)
            && let Some(notice) = Notice::from_bus(&m)
        {
            let hub = shared.hub_id.get().map_or("", String::as_str);
            if lock(&shared.state).enqueue(&notice, hub) > 0 {
                shared.ready.notify_all();
            }
        }
        let lost = sub.take_dropped();
        if lost > 0 {
            log::warn!("hub: outbound lost {lost} bus message(s) to a full queue");
        }
    }
}

/// One target's worker: take, deliver, finish, until the target is gone or the hub stops.
fn worker(shared: &Shared, epoch: u64) {
    loop {
        let job = {
            let mut st = lock(&shared.state);
            loop {
                if shared.stopped.load(Ordering::SeqCst) {
                    return;
                }
                let now = shared.clock.now();
                match st.take(epoch, now) {
                    Take::Gone => return,
                    Take::Ready(job) => break job,
                    Take::Wait(due) => {
                        let wait = due
                            .map_or(IDLE_WAIT, |d| d.saturating_duration_since(now))
                            .max(Duration::from_millis(1));
                        st = shared
                            .ready
                            .wait_timeout(st, wait)
                            .map(|(g, _)| g)
                            .unwrap_or_else(|p| p.into_inner().0);
                    }
                }
            }
        };
        let outcome = deliver(&job, shared.clock.unix_seconds(), &shared.limits);
        if !outcome.ok() {
            log::info!(
                "hub: an outbound delivery to {} failed: {}",
                job.url.host_header,
                outcome.msg().unwrap_or_default()
            );
        }
        let (now, at) = (shared.clock.now(), shared.clock.unix_seconds());
        lock(&shared.state).finish(job, &outcome, now, at);
    }
}

#[cfg(test)]
mod tests_wp_p2wh;
// -- WP SECOUT --
#[cfg(test)]
mod tests_wp_secout;
