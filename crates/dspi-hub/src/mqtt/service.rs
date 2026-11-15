//! MQTT on the hub (WP P2MQ, PLAN-P2 I6 and M4, `docs/integrator/hub-api.md` section 13.1).
//!
//! Off by default. The broker setting lives in `<data>/mqtt.json` (Admin, 0600). While it
//! is on, the hub keeps one [`MqttClient`] connected and publishes:
//!
//! | Topic | Payload | QoS, retained |
//! |---|---|---|
//! | `<base>/status` | `online` on every connection; the will `offline`, and `offline` on a clean stop | 1, retained |
//! | `<base>/<serial>/event` | an alarm (hub-api.md section 7's object, `"kind": "alarm"`) | 1 |
//! | `<base>/<serial>/event` | a history record, `{"kind": "event", "record": {...}}` | 0 |
//! | `<base>/hub/event` | an alarm about no one device (the audio link's) | 1 |
//!
//! `<base>` is `base_topic`, `dspi/<hub id>` by default. Every payload carries `v` and
//! `hub` (hub-api.md section 1.1).
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `mqtt.get` | Admin | `settings` (the password as `set` / `not set`) |
//! | `mqtt.set {settings}` | Admin | `settings`; in part, a field left out keeps its value |
//! | `mqtt.status` | Admin | `state`, `msg?`, `since`, `queued`, `dropped`, `encrypted` |
//!
//! **Threads.** The client has its own thread (P2MQC); its handler only puts the
//! connection's ups and downs on a small bounded queue of this module ([`EventQueue`]),
//! never touches the client. A pump thread of this module owns the client: it reads that
//! queue (and publishes `online` on every connection), reads the hub's bus (`Alarm` and
//! `Event` only) and publishes. Neither waits on the broker: a publish is queued in the
//! client, whose queue drops its oldest when the broker is away. With MQTT off there is
//! no thread and no bus subscription.
//!
//! **No writes.** P2MQ subscribes to nothing, so nothing that arrives over MQTT reaches a
//! device. Home Assistant's `set` topics (P2HA) act as an API key's principal (OD-P2c),
//! never as the hub; `ha` is kept here for P2HA and is not used by this module.
//!
//! **TLS (PLAN-SEC 4.8; SECOUT's [`MqttTls`], wired by WP MQWIRE).** `tls` in `mqtt.json`
//! is read with [`MqttTls::load`]: a file written before TLS with a broker that is not on
//! loopback keeps sending its password (`allow_plaintext_password`) with a one-time notice,
//! written back at once so it comes once, logged and shown by `mqtt.get` and `mqtt.status`
//! for this run. `mqtt.set` changes it with [`MqttTls::update`], `mqtt.get` shows
//! [`MqttTls::view_json`] (the client key as `set` / `not set`). Without a `port` the port
//! is [`MqttTls::default_port`] (8883 with TLS, 1883 without). `mqtt.status`'s `encrypted`
//! is the client's ([`MqttStatus::encrypted`]).
//!
//! **Secrets.** The password is kept only in the 0600 file; it is never in an answer, a
//! log line or a `Debug` (OD-P2e), and the diagnostics bundle shows it as `set` / `not set`.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::client::{MqttClient, MqttConfig, MqttEvent, MqttStatus, MqttTls};
use super::codec::{self, QoS, Will};
use crate::bus::{Bus, BusMessage, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::outbound::iso8601;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Role, Session};
use crate::system::alarms::AlarmState;

/// The family's cap in `hello.caps`.
pub const CAP: &str = "mqtt";
/// The settings' file in the hub's data folder.
pub const FILE_NAME: &str = "mqtt.json";
const TEMP_NAME: &str = "mqtt.json.tmp";
/// The largest `mqtt.json` read; a larger one is set aside.
pub const MAX_FILE: u64 = 64 * 1024;
/// A broker's host name or address, in bytes.
pub const MAX_BROKER_BYTES: usize = 253;
/// A client id, in bytes.
pub const MAX_CLIENT_ID_BYTES: usize = 64;
/// A user name or a password, in bytes.
pub const MAX_CREDENTIAL_BYTES: usize = 256;
/// The base topic, in bytes.
pub const MAX_BASE_TOPIC_BYTES: usize = 128;
/// Connection ups and downs the handler keeps for the pump; the oldest go past it.
pub const EVENT_QUEUE_CAP: usize = 64;
/// Bus messages the pump holds; it reads promptly, and `Alarm`/`Event` only reach it.
pub const BUS_CAPACITY: usize = 1024;
/// Publishes the client keeps while the broker is away.
pub const CLIENT_QUEUE_CAP: usize = 256;
/// The status topic's payloads.
pub const ONLINE: &[u8] = b"online";
/// The will, and what a clean stop publishes.
pub const OFFLINE: &[u8] = b"offline";
/// The topic level of an alarm that is about no one device.
pub const HUB_LEVEL: &str = "hub";

const VERSION: u32 = 1;
/// How often the pump looks at the stop flag and the client's events.
const PUMP_TICK: Duration = Duration::from_millis(50);
/// Bus messages published per pump pass, at most.
const PUMP_BATCH: usize = 256;
/// How long a clean stop waits for `offline` to reach the broker.
const FLUSH_WAIT: Duration = Duration::from_secs(1);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Settings ────────────────────────────────────────────────────────────────

/// `ha`: Home Assistant (P2HA's). Kept, checked for shape, not used here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HaSetting {
    /// Discovery and `set` topics on.
    pub enabled: bool,
    /// The API key (its `cid`) whose principal `set` topics act as.
    pub key: Option<u32>,
}

impl HaSetting {
    fn to_json(self) -> Value {
        json!({"enabled": self.enabled, "key": self.key})
    }
}

/// The broker setting, as `mqtt.json` keeps it. Its `Debug` never shows the password.
#[derive(Clone, Default, PartialEq)]
pub struct MqttSettings {
    /// On: the hub connects and publishes.
    pub enabled: bool,
    /// The broker's host name or address; empty for none.
    pub broker: String,
    /// The broker's port; `None` for [`MqttTls::default_port`].
    pub port: Option<u16>,
    /// The client id; `None` for the default, `dspi` and the hub's id.
    pub client_id: Option<String>,
    /// The user name, if the broker wants one.
    pub username: Option<String>,
    /// The password; only with a user name.
    pub password: Option<String>,
    /// The base topic; `None` for the default, `dspi/<hub id>`.
    pub base_topic: Option<String>,
    /// Home Assistant (P2HA).
    pub ha: HaSetting,
    /// TLS to the broker (PLAN-SEC 4.8).
    pub tls: MqttTls,
}

fn shown(set: bool) -> &'static str {
    if set { "set" } else { "not set" }
}

impl std::fmt::Debug for MqttSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MqttSettings")
            .field("enabled", &self.enabled)
            .field("broker", &self.broker)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field("password", &shown(self.password.is_some()))
            .field("base_topic", &self.base_topic)
            .field("ha", &self.ha)
            .field("tls", &self.tls)
            .finish()
    }
}

/// Only letters, digits, `-` and `_`, for a topic level or a client id made from the
/// hub's id.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

impl MqttSettings {
    /// The base topic in use: the setting, or `dspi/<hub id>`.
    pub fn base(&self, hub_id: &str) -> String {
        match &self.base_topic {
            Some(b) => b.clone(),
            None => {
                let id = plain(hub_id);
                if id.is_empty() {
                    "dspi/hub".to_owned()
                } else {
                    format!("dspi/{id}")
                }
            }
        }
    }

    /// The client id in use: the setting, or `dspi` and the hub's id's first letters and
    /// digits, 23 characters at most (what every MQTT 3.1.1 broker takes).
    pub fn client(&self, hub_id: &str) -> String {
        match &self.client_id {
            Some(c) => c.clone(),
            None => {
                let id: String = hub_id
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(19)
                    .collect();
                format!("dspi{id}")
            }
        }
    }

    /// Whether the hub should be connected.
    pub fn active(&self) -> bool {
        self.enabled && !self.broker.is_empty()
    }

    /// The port in use: the setting, or 8883 with TLS and 1883 without.
    pub fn port_in_use(&self) -> u16 {
        self.port.unwrap_or_else(|| self.tls.default_port())
    }

    /// What `mqtt.get` shows: the values in use, the password and the client key as
    /// `set` / `not set`.
    pub fn view(&self, hub_id: &str) -> Value {
        json!({
            "enabled": self.enabled,
            "broker": self.broker,
            "port": self.port_in_use(),
            "client_id": self.client(hub_id),
            "username": self.username,
            "password": shown(self.password.is_some()),
            "base_topic": self.base(hub_id),
            "ha": self.ha.to_json(),
            "tls": self.tls.view_json(),
        })
    }

    /// The file's object, with the password and `tls` as they are.
    fn to_file(&self) -> Value {
        json!({
            "v": VERSION,
            "enabled": self.enabled,
            "broker": self.broker,
            "port": self.port,
            "client_id": self.client_id,
            "username": self.username,
            "password": self.password,
            "base_topic": self.base_topic,
            "ha": self.ha.to_json(),
            "tls": self.tls.to_json(),
        })
    }

    /// Whether `other` makes the same connection: a change of `ha` alone does not
    /// reconnect.
    fn same_connection(&self, other: &MqttSettings) -> bool {
        let mut a = self.clone();
        a.ha = other.ha;
        a.active() == other.active() && a == *other
    }

    /// The client's configuration: the will `offline`, retained, on `<base>/status`.
    pub fn client_config(&self, hub_id: &str) -> MqttConfig {
        let mut c = MqttConfig::new(self.broker.clone(), self.port_in_use(), self.client(hub_id));
        c.tls = self.tls.clone();
        c.username = self.username.clone();
        c.password = self.password.clone();
        c.queue_cap = CLIENT_QUEUE_CAP;
        c.will = Some(Will {
            topic: status_topic(&self.base(hub_id)),
            payload: OFFLINE.to_vec(),
            qos: QoS::AtLeastOnce,
            retain: true,
        });
        c
    }
}

/// `<base>/status`.
pub fn status_topic(base: &str) -> String {
    format!("{base}/status")
}

/// `<base>/<serial>/event`, or `<base>/hub/event` without a serial; `None` for a serial
/// that is no topic level (empty, `/`, a wildcard, a control character, too long).
pub fn event_topic(base: &str, serial: Option<&str>) -> Option<String> {
    let level = match serial {
        None => HUB_LEVEL,
        Some(s) => {
            let fine = !s.is_empty()
                && s.len() <= 64
                && !s
                    .chars()
                    .any(|c| matches!(c, '/' | '+' | '#') || c.is_control());
            if !fine {
                return None;
            }
            s
        }
    };
    Some(format!("{base}/{level}/event"))
}

fn no_control(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

fn check_broker(s: &str) -> Result<String, String> {
    let ok = s.len() <= MAX_BROKER_BYTES
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'));
    if ok {
        Ok(s.to_owned())
    } else {
        Err("broker: a host name or an address".into())
    }
}

fn check_client_id(s: &str) -> Result<String, String> {
    let ok = !s.is_empty()
        && s.len() <= MAX_CLIENT_ID_BYTES
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if ok {
        Ok(s.to_owned())
    } else {
        Err(format!(
            "client_id: 1 to {MAX_CLIENT_ID_BYTES} of A-Z a-z 0-9 . - _"
        ))
    }
}

fn check_credential(field: &str, s: &str) -> Result<String, String> {
    if s.is_empty() || s.len() > MAX_CREDENTIAL_BYTES || !no_control(s) {
        return Err(format!(
            "{field}: 1 to {MAX_CREDENTIAL_BYTES} bytes, no control characters"
        ));
    }
    Ok(s.to_owned())
}

fn check_base_topic(s: &str) -> Result<String, String> {
    let ok = s.len() <= MAX_BASE_TOPIC_BYTES
        && codec::valid_topic(s)
        && no_control(s)
        && !s.split('/').any(str::is_empty)
        && !s.starts_with('$');
    if ok {
        Ok(s.to_owned())
    } else {
        Err(format!(
            "base_topic: a topic name of at most {MAX_BASE_TOPIC_BYTES} bytes, no wildcards, \
             no empty level, not starting with $"
        ))
    }
}

fn opt_string(
    v: &Value,
    field: &str,
    check: impl Fn(&str) -> Result<String, String>,
) -> Result<Option<String>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => check(s).map(Some),
        _ => Err(format!("{field}: a string or null")),
    }
}

fn parse_port(v: &Value) -> Result<u16, String> {
    v.as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| "port: 1 to 65535".to_owned())
}

fn parse_ha(v: &Value, current: HaSetting) -> Result<HaSetting, String> {
    let Some(map) = v.as_object() else {
        return Err("ha: an object {enabled, key}".into());
    };
    let mut ha = current;
    if let Some(e) = map.get("enabled") {
        ha.enabled = e.as_bool().ok_or("ha.enabled: true or false")?;
    }
    if let Some(k) = map.get("key") {
        ha.key = match k {
            Value::Null => None,
            _ => Some(
                k.as_u64()
                    .and_then(|k| u32::try_from(k).ok())
                    .ok_or("ha.key: an API key's cid, or null")?,
            ),
        };
    }
    Ok(ha)
}

/// Where a field's value comes from: `mqtt.set` (where `password` `"set"` keeps it and
/// `"not set"` clears it, so `mqtt.get`'s answer can be sent back) or the file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Request,
    File,
}

/// Apply one field to `s`.
fn apply_field(s: &mut MqttSettings, key: &str, v: &Value, from: Source) -> Result<(), String> {
    match key {
        "enabled" => s.enabled = v.as_bool().ok_or("enabled: true or false")?,
        "broker" => {
            s.broker = match v {
                Value::Null => String::new(),
                Value::String(b) => check_broker(b)?,
                _ => return Err("broker: a string".into()),
            }
        }
        "port" => {
            s.port = match v {
                Value::Null => None,
                _ => Some(parse_port(v)?),
            }
        }
        "client_id" => s.client_id = opt_string(v, key, check_client_id)?,
        "username" => s.username = opt_string(v, key, |u| check_credential(key, u))?,
        "password" => {
            s.password = match (v, from) {
                (Value::String(p), Source::Request) if p == "set" => s.password.take(),
                (Value::String(p), Source::Request) if p == "not set" => None,
                _ => opt_string(v, key, |p| check_credential(key, p))?,
            }
        }
        "base_topic" => s.base_topic = opt_string(v, key, check_base_topic)?,
        "ha" => s.ha = parse_ha(v, s.ha)?,
        // From the file, `tls` is read once every other field is (`decode`, which needs
        // the broker for the migration).
        "tls" if from == Source::Request => {
            s.tls = MqttTls::update(v, &s.tls).map_err(|e| format!("tls: {e}"))?;
        }
        // `v`, anything unknown: ignored.
        _ => {}
    }
    Ok(())
}

/// The whole setting's own rules, after every field.
fn check_whole(s: &MqttSettings) -> Result<(), String> {
    if s.password.is_some() && s.username.is_none() {
        return Err("password: needs a username (MQTT 3.1.1)".into());
    }
    if s.enabled && s.broker.is_empty() {
        return Err("broker: needed to turn MQTT on".into());
    }
    Ok(())
}

/// `mqtt.set`'s `settings` applied to `current`: in part, all or nothing.
pub fn apply_request(current: &MqttSettings, patch: &Value) -> Result<MqttSettings, String> {
    let Some(map) = patch.as_object() else {
        return Err("settings: an object".into());
    };
    let mut s = current.clone();
    // The order of the fields in the request does not matter: `password` before
    // `username` is the same as after.
    for (k, v) in map {
        apply_field(&mut s, k, v, Source::Request)?;
    }
    check_whole(&s)?;
    Ok(s)
}

/// What reading the file found.
struct Decoded {
    settings: MqttSettings,
    /// A field that was bad (its rule, never its value).
    bad: Option<String>,
    /// [`MqttTls::load`]'s one-time notice: the file must be written back.
    notice: Option<&'static str>,
}

/// The file's settings, field by field: a bad field keeps its default and is named.
fn decode(bytes: &[u8]) -> Decoded {
    let mut s = MqttSettings::default();
    let map = match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => map,
        Ok(_) => {
            return Decoded {
                settings: s,
                bad: Some("not an object".into()),
                notice: None,
            };
        }
        Err(e) => {
            return Decoded {
                settings: s,
                bad: Some(e.to_string()),
                notice: None,
            };
        }
    };
    let mut bad = None;
    for (k, v) in &map {
        if let Err(why) = apply_field(&mut s, k, v, Source::File) {
            bad = Some(why);
        }
    }
    // -- WP MQWIRE -- `tls` last: a file without it (written before TLS) is migrated for
    // its broker (OD-SECo). With no broker or no password there is nothing to keep
    // working, so no flag and no notice.
    let mut notice = None;
    let tls = map.get("tls").filter(|v| !v.is_null());
    if tls.is_some() || (!s.broker.is_empty() && s.password.is_some()) {
        match MqttTls::load(tls, &s.broker) {
            Ok((t, n)) => {
                s.tls = t;
                notice = n;
            }
            Err(e) => bad = Some(format!("tls: {e}")),
        }
    }
    if let Err(why) = check_whole(&s) {
        // A password without a user name cannot be sent; MQTT stays off until fixed.
        s.enabled = false;
        if s.username.is_none() {
            s.password = None;
        }
        bad = Some(why);
    }
    Decoded {
        settings: s,
        bad,
        notice,
    }
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

/// Read the settings. A file that is not whole is set aside as `.bad-<unix seconds>` and
/// its good fields written back; never a panic, never a failed start.
fn load(dir: &Path, unix_seconds: u64) -> (MqttSettings, Option<&'static str>) {
    let path = dir.join(FILE_NAME);
    let Decoded {
        settings,
        bad,
        notice,
    } = match read_limited(&path) {
        Ok(bytes) => decode(&bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return (MqttSettings::default(), None);
        }
        Err(e) => Decoded {
            settings: MqttSettings::default(),
            bad: Some(e.to_string()),
            notice: None,
        },
    };
    if let Some(n) = notice {
        log::warn!("hub: {n}");
    }
    if let Some(why) = bad {
        let aside = dir.join(format!("{FILE_NAME}.bad-{unix_seconds}"));
        // `why` names a field and its rule, never a value.
        log::warn!(
            "hub: {} set aside as {} ({why}); the good settings kept",
            path.display(),
            aside.display()
        );
        if let Err(e) = fs::rename(&path, &aside) {
            log::warn!("hub: cannot set {} aside: {e}", path.display());
        }
        if let Err(e) = save(dir, &settings) {
            log::warn!("hub: cannot write {}: {e}", path.display());
        }
    } else if notice.is_some() {
        // -- WP MQWIRE -- written back with the flag, so the next start reads it and the
        // notice comes once (OD-SECo).
        if let Err(e) = save(dir, &settings) {
            log::warn!("hub: cannot write {}: {e}", path.display());
        }
    }
    (settings, notice)
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

/// Write the settings atomically, 0600.
fn save(dir: &Path, settings: &MqttSettings) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(&settings.to_file()).map_err(io::Error::other)?;
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

// ── Status ──────────────────────────────────────────────────────────────────

/// The connection as `mqtt.status` tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Off, or no broker.
    Off,
    /// Trying to connect, or to connect again after a lost connection.
    Connecting,
    /// Connected and accepted.
    Connected,
    /// The broker refused the connection (a wrong password, a refused client id), or the
    /// setting cannot make a connection; the client keeps trying after its backoff.
    Failed,
}

impl LinkState {
    /// Its word in `mqtt.status`.
    pub fn word(self) -> &'static str {
        match self {
            LinkState::Off => "off",
            LinkState::Connecting => "connecting",
            LinkState::Connected => "connected",
            LinkState::Failed => "failed",
        }
    }
}

/// `mqtt.status`'s state: pure, fed the client's events and counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView {
    /// The state.
    pub state: LinkState,
    /// Why, for a person (a refusal, a lost connection). Never the password.
    pub msg: Option<String>,
    /// When `state` last changed, Unix seconds.
    pub since: u64,
    /// Publishes waiting in the client.
    pub queued: usize,
    /// Publishes the client dropped (a full queue) since it started.
    pub dropped: u64,
    /// The connection now is TLS ([`MqttStatus::encrypted`]).
    pub encrypted: bool,
    /// The one-time migration notice of this run (OD-SECo), if the file was migrated.
    pub notice: Option<&'static str>,
}

impl StatusView {
    fn new(at: u64) -> Self {
        Self {
            state: LinkState::Off,
            msg: None,
            since: at,
            queued: 0,
            dropped: 0,
            encrypted: false,
            notice: None,
        }
    }

    /// Move to `state`; `since` changes only when the state does.
    pub fn set(&mut self, state: LinkState, msg: Option<String>, at: u64) {
        if self.state != state {
            self.since = at;
        }
        if state != LinkState::Connected {
            self.encrypted = false;
        }
        self.state = state;
        self.msg = msg;
    }

    /// What one client event means.
    pub fn on_event(&mut self, event: &MqttEvent, at: u64) {
        match event {
            MqttEvent::Connected { .. } => self.set(LinkState::Connected, None, at),
            MqttEvent::Disconnected { reason } => {
                // P2MQC's `Dropped::Refused`: "the broker refused the connection: <why>".
                // -- WP MQWIRE -- SECOUT's `PlaintextPassword` is a setting to fix as well.
                let state = if reason.contains("refused the connection")
                    || reason.contains("refused to send the password")
                {
                    LinkState::Failed
                } else {
                    LinkState::Connecting
                };
                self.set(state, Some(reason.clone()), at);
            }
            MqttEvent::Message { .. } | MqttEvent::SubscribeRefused { .. } => {}
        }
    }

    /// The client's counts.
    pub fn counts(&mut self, s: &MqttStatus) {
        self.queued = s.queued;
        self.dropped = s.dropped;
        self.encrypted = s.encrypted;
    }

    fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("state".into(), self.state.word().into());
        if let Some(msg) = &self.msg {
            m.insert("msg".into(), msg.clone().into());
        }
        m.insert("since".into(), iso8601(self.since).into());
        m.insert("queued".into(), self.queued.into());
        m.insert("dropped".into(), self.dropped.into());
        // -- WP MQWIRE -- the client's: true while the connection is TLS.
        m.insert("encrypted".into(), self.encrypted.into());
        if let Some(n) = self.notice {
            m.insert("notice".into(), n.into());
        }
        m
    }
}

/// The handler's bounded queue: the client's thread puts, the pump takes. The oldest go
/// past [`EVENT_QUEUE_CAP`].
#[derive(Debug, Default)]
pub struct EventQueue(Mutex<VecDeque<MqttEvent>>);

impl EventQueue {
    /// Called on the client's thread: never blocks on anything but this leaf lock.
    pub fn push(&self, event: MqttEvent) {
        // P2MQ subscribes to nothing; a message (P2HA's later) is not this queue's.
        if !matches!(
            event,
            MqttEvent::Connected { .. } | MqttEvent::Disconnected { .. }
        ) {
            return;
        }
        let mut q = lock(&self.0);
        while q.len() >= EVENT_QUEUE_CAP {
            q.pop_front();
        }
        q.push_back(event);
    }

    /// Everything queued, oldest first.
    pub fn drain(&self) -> Vec<MqttEvent> {
        lock(&self.0).drain(..).collect()
    }
}

// ── What is published ───────────────────────────────────────────────────────

/// The alarm object (hub-api.md section 7), with `kind`, `v` and `hub`.
fn alarm_payload(m: &BusMessage, hub: &str) -> Option<(Option<String>, Value)> {
    let BusMessage::Alarm {
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
    } = m
    else {
        return None;
    };
    let state_word = match state {
        AlarmState::Raised => "raised",
        AlarmState::Acknowledged => "acknowledged",
        AlarmState::Cleared => "cleared",
    };
    let mut v = json!({
        "v": VERSION,
        "hub": hub,
        "kind": "alarm",
        "id": id,
        "source": source,
        "code": code,
        "subject": subject,
        "severity": severity,
        "message": message.chars().take(1024).collect::<String>(),
        "state": state_word,
        "at": iso8601(*at),
        "raised_at": iso8601(*raised_at),
        "count": count,
    });
    if let Some(map) = v.as_object_mut() {
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
    Some((serial.clone(), v))
}

/// What one bus message becomes: topic, payload and QoS, or nothing.
pub fn publication(m: &BusMessage, base: &str, hub: &str) -> Option<(String, Vec<u8>, QoS)> {
    match m {
        BusMessage::Alarm { .. } => {
            let (serial, v) = alarm_payload(m, hub)?;
            let topic = event_topic(base, serial.as_deref())?;
            Some((topic, v.to_string().into_bytes(), QoS::AtLeastOnce))
        }
        BusMessage::Event { serial, record } => {
            let topic = event_topic(base, Some(serial))?;
            let v = json!({
                "v": VERSION,
                "hub": hub,
                "kind": "event",
                "serial": serial,
                "record": record,
            });
            Some((topic, v.to_string().into_bytes(), QoS::AtMostOnce))
        }
        _ => None,
    }
}

fn wanted(m: &BusMessage) -> bool {
    matches!(m, BusMessage::Alarm { .. } | BusMessage::Event { .. })
}

// ── The service ─────────────────────────────────────────────────────────────

/// One running connection: the pump thread, which owns the client.
struct Runtime {
    stop: Arc<AtomicBool>,
    pump: JoinHandle<()>,
}

impl Runtime {
    /// Stop the pump; it publishes `offline`, waits for it a moment and stops the client.
    fn end(self) {
        self.stop.store(true, Ordering::SeqCst);
        if self.pump.join().is_err() {
            log::error!("hub: the MQTT pump panicked");
        }
    }
}

struct Shared {
    clock: Arc<dyn HubClock>,
    bus: Arc<Bus>,
    data_dir: Option<PathBuf>,
    settings: Mutex<MqttSettings>,
    /// A leaf: written by the pump, read by `mqtt.status`.
    status: Arc<Mutex<StatusView>>,
    /// Held while a connection starts or ends, so two `mqtt.set`s do not race.
    runtime: Mutex<Option<Runtime>>,
    hub_id: OnceLock<String>,
    stopped: AtomicBool,
    /// The first `run_due` has started what the file asked for.
    begun: AtomicBool,
    /// When the service was made: its first due time (a fixed time, never later than the
    /// timer's `now`), so the hub's id is learnt and a saved setting connects.
    born: Instant,
}

/// The `mqtt` family: the broker setting and the hub's one connection.
pub struct MqttService {
    shared: Arc<Shared>,
}

/// The service, for `service.rs`'s list. Always there: with MQTT off it does nothing.
pub fn service(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Option<Arc<dyn HubService>> {
    let _ = config;
    Some(Arc::new(MqttService::new(
        setup.data_dir,
        Arc::clone(setup.clock),
        Arc::clone(setup.bus),
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

struct Pump {
    client: MqttClient,
    events: Arc<EventQueue>,
    sub: Subscription,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<StatusView>>,
    clock: Arc<dyn HubClock>,
    base: String,
    hub: String,
}

impl Pump {
    fn run(self) {
        let status_topic = status_topic(&self.base);
        while !self.stop.load(Ordering::SeqCst) {
            for e in self.events.drain() {
                if matches!(e, MqttEvent::Connected { .. })
                    && let Err(err) =
                        self.client
                            .publish(&status_topic, ONLINE, QoS::AtLeastOnce, true)
                {
                    log::warn!("hub: MQTT status not published: {err}");
                }
                lock(&self.status).on_event(&e, self.clock.unix_seconds());
            }
            if let Some(first) = self.sub.recv_timeout(PUMP_TICK) {
                self.send(&first);
                for _ in 1..PUMP_BATCH {
                    let Some(m) = self.sub.try_recv() else { break };
                    self.send(&m);
                }
            }
            let lost = self.sub.take_dropped();
            if lost > 0 {
                log::warn!("hub: MQTT lost {lost} bus message(s) to a full queue");
            }
            lock(&self.status).counts(&self.client.status());
        }
        self.goodbye(&status_topic);
    }

    fn send(&self, m: &BusMessage) {
        let Some((topic, payload, qos)) = publication(m, &self.base, &self.hub) else {
            return;
        };
        if let Err(e) = self.client.publish(&topic, &payload, qos, false) {
            log::warn!("hub: an MQTT publish to {topic} was refused: {e}");
        }
    }

    /// A clean stop: `offline` (the broker drops the will on DISCONNECT), given a moment
    /// to go out, then the client.
    fn goodbye(mut self, status_topic: &str) {
        if self.client.status().connected {
            let _ = self
                .client
                .publish(status_topic, OFFLINE, QoS::AtLeastOnce, true);
            let until = Instant::now().checked_add(FLUSH_WAIT);
            while until.is_some_and(|u| Instant::now() < u) {
                let s = self.client.status();
                if !s.connected || (s.queued == 0 && s.inflight == 0) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.client.stop();
    }
}

impl MqttService {
    /// The service, its setting read from `data_dir` (`None`: kept for this run only).
    /// Nothing connects until the hub's id is known (the first `run_due`, or `mqtt.set`).
    pub fn new(data_dir: Option<&Path>, clock: Arc<dyn HubClock>, bus: Arc<Bus>) -> Self {
        let (settings, notice) = data_dir.map_or_else(
            || (MqttSettings::default(), None),
            |d| load(d, clock.unix_seconds()),
        );
        let born = clock.now();
        let mut view = StatusView::new(clock.unix_seconds());
        view.notice = notice;
        let status = Arc::new(Mutex::new(view));
        Self {
            shared: Arc::new(Shared {
                clock,
                bus,
                data_dir: data_dir.map(Path::to_owned),
                settings: Mutex::new(settings),
                status,
                runtime: Mutex::new(None),
                hub_id: OnceLock::new(),
                stopped: AtomicBool::new(false),
                begun: AtomicBool::new(false),
                born,
            }),
        }
    }

    /// The state now, as `mqtt.status` tells it.
    pub fn status(&self) -> StatusView {
        lock(&self.shared.status).clone()
    }

    /// End the running connection, if any, and start one for the setting now if it is
    /// on. On the caller's thread: ending takes at most about the client's connect timeout
    /// and [`FLUSH_WAIT`].
    fn restart(&self) {
        let shared = &self.shared;
        let mut rt = lock(&shared.runtime);
        if let Some(old) = rt.take() {
            old.end();
        }
        let now = shared.clock.unix_seconds();
        let settings = lock(&shared.settings).clone();
        if shared.stopped.load(Ordering::SeqCst) || !settings.active() {
            lock(&shared.status).set(LinkState::Off, None, now);
            return;
        }
        let Some(hub_id) = shared.hub_id.get() else {
            return;
        };
        let base = settings.base(hub_id);
        let events = Arc::new(EventQueue::default());
        let handler_events = Arc::clone(&events);
        // Subscribe before connecting, so nothing raised meanwhile is missed.
        let sub = shared.bus.subscribe_filtered("mqtt", BUS_CAPACITY, wanted);
        let client = match MqttClient::start(settings.client_config(hub_id), move |e| {
            handler_events.push(e);
        }) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("hub: MQTT not started: {e}");
                lock(&shared.status).set(LinkState::Failed, Some(e.to_string()), now);
                return;
            }
        };
        {
            let mut st = lock(&shared.status);
            st.set(LinkState::Connecting, None, now);
            st.queued = 0;
            st.dropped = 0;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let pump = Pump {
            client,
            events,
            sub,
            stop: Arc::clone(&stop),
            status: Arc::clone(&shared.status),
            clock: Arc::clone(&shared.clock),
            base,
            hub: hub_id.clone(),
        };
        match thread::Builder::new()
            .name("hub-mqtt".into())
            .spawn(move || pump.run())
        {
            Ok(pump) => *rt = Some(Runtime { stop, pump }),
            Err(e) => {
                // The closure, and the client in it, are dropped: the client stops.
                log::error!("hub: the MQTT pump did not start: {e}");
                lock(&shared.status).set(LinkState::Failed, Some("no thread".into()), now);
            }
        }
    }

    fn get(&self, hub: &HubHandle, id: &Value) -> String {
        let view = lock(&self.shared.settings).view(hub.hub_id());
        let mut m = Map::new();
        m.insert("settings".into(), view);
        // -- WP MQWIRE --
        if let Some(n) = lock(&self.shared.status).notice {
            m.insert("notice".into(), n.into());
        }
        reply_ok(id, m)
    }

    fn set(&self, hub: &HubHandle, id: &Value, msg: &Value) -> String {
        let Some(patch) = msg.get("settings") else {
            return reply_err(id, "bad_request", "settings: the fields to change");
        };
        let _ = self.shared.hub_id.set(hub.hub_id().to_owned());
        let (changed, active, view) = {
            let mut current = lock(&self.shared.settings);
            let next = match apply_request(&current, patch) {
                Ok(n) => n,
                Err(why) => return reply_err(id, "bad_request", &why),
            };
            if let Some(dir) = &self.shared.data_dir
                && let Err(e) = save(dir, &next)
            {
                log::error!("hub: cannot write {}: {e}", dir.join(FILE_NAME).display());
                return reply_err(id, "internal", "the settings could not be saved");
            }
            let changed = !next.same_connection(&current);
            // -- WP MQWIRE -- the migration notice is answered once TLS is set again.
            if next.tls != current.tls {
                lock(&self.shared.status).notice = None;
            }
            *current = next;
            (changed, current.active(), current.view(hub.hub_id()))
        };
        // Lock order: `runtime`, then `settings` (as `restart` takes them); never the
        // other way round, so the settings' lock is let go first.
        let reconnect = changed || (active && lock(&self.shared.runtime).is_none());
        self.shared.begun.store(true, Ordering::SeqCst);
        if reconnect {
            self.restart();
        }
        let mut m = Map::new();
        m.insert("settings".into(), view);
        reply_ok(id, m)
    }
}

impl HubService for MqttService {
    fn name(&self) -> &'static str {
        "mqtt"
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
        if !matches!(t, "mqtt.get" | "mqtt.set" | "mqtt.status") {
            return None;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        if session.role() != Role::Admin {
            return Some(reply_err(&id, "denied", "the MQTT broker is an admin's"));
        }
        Some(match t {
            "mqtt.get" => self.get(hub, &id),
            "mqtt.set" => self.set(hub, &id, msg),
            _ => reply_ok(&id, self.status().to_json()),
        })
    }

    fn next_due(&self) -> Option<Instant> {
        let s = &self.shared;
        if s.stopped.load(Ordering::SeqCst) || s.begun.load(Ordering::SeqCst) {
            return None;
        }
        lock(&s.settings).active().then_some(s.born)
    }

    fn run_due(&self, hub: &HubHandle, now: Instant) {
        let _ = now;
        let _ = self.shared.hub_id.set(hub.hub_id().to_owned());
        if !self.shared.begun.swap(true, Ordering::SeqCst) {
            self.restart();
        }
    }

    fn stop(&self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        let old = lock(&self.shared.runtime).take();
        if let Some(old) = old {
            old.end();
        }
        lock(&self.shared.status).set(LinkState::Off, None, self.shared.clock.unix_seconds());
    }
}

impl Drop for MqttService {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(old) = lock(&self.shared.runtime).take() {
            old.end();
        }
    }
}

#[cfg(test)]
#[path = "tests_wp_p2mq.rs"]
mod tests_wp_p2mq;
