//! A blocking MQTT 3.1.1 client on one thread (WP P2MQC, OD-P2c).
//!
//! [`MqttClient::start`] spawns the thread and returns at once. The thread connects over
//! TCP, with TLS when [`MqttConfig::tls`] says so (WP SECOUT, PLAN-SEC 4.8: the shared
//! connector [`crate::outbound::tls`], the handshake done and the broker verified before a
//! byte of CONNECT goes out), sends CONNECT with the will and the credentials,
//! subscribes to every filter asked for so far, and then sends what [`MqttClient::publish`]
//! queued, answers QoS 1 publishes with PUBACK, keeps the connection alive with PINGREQ and
//! hands what arrives to the caller's handler. When the connection ends it reconnects with
//! a doubling backoff and subscribes again; QoS 1 publishes not yet acknowledged are sent
//! again with the dup flag. The backoff starts again from its minimum only after a
//! connection has stayed up for `stable_after`. A packet from the broker over `max_packet`
//! is read past and counted ([`MqttStatus::oversized`]), not a reason to reconnect.
//! [`MqttClient::stop`] is bounded by about `connect_timeout`, whatever the broker does.
//!
//! Bounded everything: the outgoing queue keeps at most `queue_cap` publishes and drops the
//! oldest when full (counted in [`MqttStatus::dropped`]); at most [`MAX_INFLIGHT`] QoS 1
//! publishes wait for their PUBACK; the receive buffer never holds more than one packet of
//! `max_packet` bytes plus one read; at most [`MAX_SUBSCRIPTIONS`] filters.
//!
//! The password is never written to a log, an event, an error or a `Debug` (OD-P2e), nor
//! is a client key. Without TLS the password goes only to a loopback broker, unless
//! [`MqttTls::allow_plaintext_password`] says otherwise (PLAN-SEC 4.8, OD-SECo).
//!
//! Lock order: the client's one lock is a leaf, never held across socket I/O or the handler.

use std::collections::VecDeque;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustls::ClientConfig;
use serde_json::{Map, Value};

use super::codec::{self, Connect, DecodeError, Packet, Publish, QoS, Will};
use crate::outbound::tls::{self, ClientIdentity, Stream, TlsError, Trust};

/// The MQTT port without TLS.
pub const DEFAULT_MQTT_PORT: u16 = 1883;
// -- WP SECOUT --
/// The MQTT port with TLS.
pub const DEFAULT_MQTTS_PORT: u16 = 8883;
/// QoS 1 publishes sent and not yet acknowledged, at most.
pub const MAX_INFLIGHT: usize = 16;
/// Topic filters a client subscribes to, at most.
pub const MAX_SUBSCRIPTIONS: usize = 256;
/// Filters per SUBSCRIBE packet when the client subscribes again after a reconnect.
const FILTERS_PER_SUBSCRIBE: usize = 16;
/// SUBSCRIBEs waiting for their SUBACK that the client remembers, at most.
const MAX_PENDING_SUBACKS: usize = 64;
/// One read from the socket.
const READ_CHUNK: usize = 4096;
/// Bytes of publishes written per pass before the thread looks at `stop` and the socket
/// again (at least one publish per pass).
const MAX_BATCH_BYTES: usize = 256 * 1024;
/// How long [`MqttClient::stop`] waits for the thread to send DISCONNECT by itself before
/// it shuts the socket under it.
const STOP_GRACE: Duration = Duration::from_millis(500);

/// How to reach the broker and how the client behaves. Its `Debug` shows whether a
/// password is set, never the password.
#[derive(Clone)]
pub struct MqttConfig {
    /// The broker's host name or address.
    pub host: String,
    /// The broker's port ([`DEFAULT_MQTT_PORT`]).
    pub port: u16,
    /// The client identifier.
    pub client_id: String,
    /// The user name, if the broker wants one.
    pub username: Option<String>,
    /// The password; MQTT 3.1.1 sends one only with a user name.
    pub password: Option<String>,
    /// Keep-alive (whole seconds, at most 65 535; zero turns it off).
    pub keep_alive: Duration,
    /// The last will, published by the broker when the client goes away unannounced.
    pub will: Option<Will>,
    /// Publishes the outgoing queue keeps while the broker is away; the oldest go first.
    pub queue_cap: usize,
    /// The longest packet sent or taken, fixed header included.
    pub max_packet: usize,
    /// For the TCP connect and for the CONNACK, and the socket's write timeout.
    pub connect_timeout: Duration,
    /// The first wait after a failed or ended connection.
    pub backoff_min: Duration,
    /// The longest wait; each failure doubles the wait up to this.
    pub backoff_max: Duration,
    /// How long a connection must stay up before the backoff starts again from
    /// `backoff_min`. A broker that accepts and then closes (a client-id takeover, an ACL
    /// kick) therefore meets a growing wait, not a reconnect every `backoff_min`.
    pub stable_after: Duration,
    /// How often the thread looks at the queue while connected (the socket's read timeout).
    pub tick: Duration,
    // -- WP SECOUT --
    /// TLS to the broker, and whether the password may go out without it.
    pub tls: MqttTls,
}

impl MqttConfig {
    /// A configuration with the defaults: no credentials, no will, keep-alive 30 s, 256
    /// queued publishes, 256 KiB packets, 5 s connect timeout, backoff 1 s to 60 s, reset
    /// after a connection has stayed up 30 s.
    pub fn new(host: impl Into<String>, port: u16, client_id: impl Into<String>) -> MqttConfig {
        MqttConfig {
            host: host.into(),
            port,
            client_id: client_id.into(),
            username: None,
            password: None,
            keep_alive: Duration::from_secs(30),
            will: None,
            queue_cap: 256,
            max_packet: 256 * 1024,
            connect_timeout: Duration::from_secs(5),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            stable_after: Duration::from_secs(30),
            tick: Duration::from_millis(25),
            tls: MqttTls::default(),
        }
    }
}

impl fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field(
                "password",
                &if self.password.is_some() {
                    "set"
                } else {
                    "not set"
                },
            )
            .field("keep_alive", &self.keep_alive)
            .field("will", &self.will)
            .field("queue_cap", &self.queue_cap)
            .field("max_packet", &self.max_packet)
            .field("connect_timeout", &self.connect_timeout)
            .field("backoff_min", &self.backoff_min)
            .field("backoff_max", &self.backoff_max)
            .field("stable_after", &self.stable_after)
            .field("tick", &self.tick)
            .field("tls", &self.tls)
            .finish()
    }
}

// ── TLS settings (WP SECOUT) ────────────────────────────────────────────────

/// Whether the client speaks TLS to the broker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MqttTlsMode {
    /// Plain TCP.
    #[default]
    Off,
    /// TLS 1.2 or 1.3, the broker verified ([`MqttTls`]).
    On,
}

/// TLS to the broker (PLAN-SEC 4.8): `mqtt.json`'s `tls` object, `{mode: off|on, ca_pem?,
/// pin_sha256?, client_cert_pem?, client_key_pem?, client_cert_file?, client_key_file?,
/// allow_plaintext_password?}`. Its `Debug` shows whether a client key is set, never the key.
///
/// With `mode: on` the broker is verified with the `webpki-roots` CAs and its host name,
/// or with `ca_pem` instead of the roots, or with `pin_sha256` instead of both (OD-SECf);
/// a client certificate is sent if one is set (inline PEM, or PEM files whose key file is
/// not readable by group or others). With `mode: off` the password goes only to a
/// loopback broker unless `allow_plaintext_password` is set.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct MqttTls {
    /// Off or on.
    pub mode: MqttTlsMode,
    /// CA or self-signed certificates to trust instead of the `webpki-roots` CAs.
    pub ca_pem: Option<String>,
    /// The broker's key, `sha256//<base64>` of its SubjectPublicKeyInfo.
    pub pin_sha256: Option<String>,
    /// The client certificate chain, PEM.
    pub client_cert_pem: Option<String>,
    /// The client certificate's private key, PEM. Never shown.
    pub client_key_pem: Option<String>,
    /// The client certificate chain as a PEM file, instead of `client_cert_pem`.
    pub client_cert_file: Option<PathBuf>,
    /// The client key as a PEM file (0600), instead of `client_key_pem`.
    pub client_key_file: Option<PathBuf>,
    /// Without TLS, send the password to a broker that is not on loopback too.
    pub allow_plaintext_password: bool,
}

fn shown(set: bool) -> &'static str {
    if set { "set" } else { "not set" }
}

impl fmt::Debug for MqttTls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttTls")
            .field("mode", &self.mode)
            .field("ca_pem", &shown(self.ca_pem.is_some()))
            .field("pin_sha256", &self.pin_sha256)
            .field("client_cert_pem", &shown(self.client_cert_pem.is_some()))
            .field("client_key_pem", &shown(self.client_key_pem.is_some()))
            .field("client_cert_file", &self.client_cert_file)
            .field("client_key_file", &self.client_key_file)
            .field("allow_plaintext_password", &self.allow_plaintext_password)
            .finish()
    }
}

/// What a broker configured before TLS is told, once, when it is kept sending its password
/// in clear (OD-SECo).
pub const MQTT_MIGRATED_NOTICE: &str = "The MQTT broker was set up before the hub had TLS and is not on this machine. Its password still goes out unencrypted (\"allow_plaintext_password\": true was set so it keeps working). Turn TLS on (port 8883) to encrypt it.";

/// A PEM file, at most [`tls::MAX_PEM_BYTES`]; a key file must not be readable by group or
/// others (Unix).
fn read_pem_file(what: &str, path: &Path, secret: bool) -> Result<String, MqttError> {
    let bad = |why: String| MqttError::Config(format!("{what} {}: {why}", path.display()));
    let file = std::fs::File::open(path).map_err(|e| bad(e.to_string()))?;
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .map_err(|e| bad(e.to_string()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(bad(format!(
                "readable by others (mode {:o}); make it 0600",
                mode & 0o777
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = secret;
    let mut text = String::new();
    file.take((tls::MAX_PEM_BYTES as u64).saturating_add(1))
        .read_to_string(&mut text)
        .map_err(|_| bad("not a PEM text file".into()))?;
    if text.len() > tls::MAX_PEM_BYTES {
        return Err(bad(format!("longer than {} bytes", tls::MAX_PEM_BYTES)));
    }
    Ok(text)
}

/// A broker host that is loopback without a lookup: a loopback literal or `localhost`.
pub fn host_is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(tls::is_loopback)
}

impl MqttTls {
    /// The broker's port when the settings give none: 8883 with TLS, 1883 without.
    pub fn default_port(&self) -> u16 {
        match self.mode {
            MqttTlsMode::On => DEFAULT_MQTTS_PORT,
            MqttTlsMode::Off => DEFAULT_MQTT_PORT,
        }
    }

    /// Whether the connection is TLS.
    pub fn encrypted(&self) -> bool {
        self.mode == MqttTlsMode::On
    }

    /// `mqtt.json`'s `tls` as read at start, for the broker `host`. Absent (or `null`): a
    /// file written before TLS (OD-P2f), so plain TCP, and a broker not on loopback keeps
    /// getting its password with `allow_plaintext_password` set and the notice
    /// [`MQTT_MIGRATED_NOTICE`] answered, once: the caller writes [`MqttTls::to_json`]
    /// back, so the next start reads the field (OD-SECo).
    pub fn load(
        value: Option<&Value>,
        host: &str,
    ) -> Result<(MqttTls, Option<&'static str>), MqttError> {
        match value {
            None | Some(Value::Null) => {
                let migrate = !host_is_loopback(host);
                let tls = MqttTls {
                    allow_plaintext_password: migrate,
                    ..MqttTls::default()
                };
                Ok((tls, migrate.then_some(MQTT_MIGRATED_NOTICE)))
            }
            Some(v) => MqttTls::parse(v, None).map(|t| (t, None)),
        }
    }

    /// `mqtt.set`'s `tls`, against the current settings: `client_key_pem: "set"` keeps the
    /// current key (so `mqtt.get`'s view can be sent back as it is).
    pub fn update(value: &Value, current: &MqttTls) -> Result<MqttTls, MqttError> {
        MqttTls::parse(value, Some(current))
    }

    fn parse(value: &Value, current: Option<&MqttTls>) -> Result<MqttTls, MqttError> {
        let obj = value
            .as_object()
            .ok_or_else(|| MqttError::Config("tls is not an object".into()))?;
        let text = |key: &str| -> Result<Option<String>, MqttError> {
            match obj.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(MqttError::Config(format!("tls.{key} is not a string"))),
            }
        };
        let mode = match text("mode")?.as_deref() {
            None | Some("off") => MqttTlsMode::Off,
            Some("on") => MqttTlsMode::On,
            Some(_) => return Err(MqttError::Config("tls.mode must be off or on".into())),
        };
        let allow_plaintext_password = match obj.get("allow_plaintext_password") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                return Err(MqttError::Config(
                    "tls.allow_plaintext_password is not a bool".into(),
                ));
            }
        };
        let client_key_pem = match text("client_key_pem")?.as_deref() {
            Some("set") => current.and_then(|c| c.client_key_pem.clone()),
            Some("not set") => None,
            _ => text("client_key_pem")?,
        };
        let pin_sha256 = text("pin_sha256")?
            .map(|p| tls::parse_pin(&p).map(|pin| tls::pin_text(&pin)))
            .transpose()
            .map_err(|e| MqttError::Config(e.to_string()))?;
        let tls = MqttTls {
            mode,
            ca_pem: text("ca_pem")?,
            pin_sha256,
            client_cert_pem: text("client_cert_pem")?,
            client_key_pem,
            client_cert_file: text("client_cert_file")?.map(PathBuf::from),
            client_key_file: text("client_key_file")?.map(PathBuf::from),
            allow_plaintext_password,
        };
        tls.check()?;
        Ok(tls)
    }

    /// The settings' own rules: TLS fields only with `mode: on`; one trust; a client
    /// certificate and its key together, inline or as files; every PEM parses.
    pub fn check(&self) -> Result<(), MqttError> {
        let cfg = |why: &str| MqttError::Config(why.to_owned());
        let any_tls = self.ca_pem.is_some()
            || self.pin_sha256.is_some()
            || self.client_cert_pem.is_some()
            || self.client_key_pem.is_some()
            || self.client_cert_file.is_some()
            || self.client_key_file.is_some();
        if self.mode == MqttTlsMode::Off {
            if any_tls {
                return Err(cfg(
                    "tls: ca_pem, pin_sha256 and client certificates need mode on",
                ));
            }
            return Ok(());
        }
        if self.allow_plaintext_password {
            return Err(cfg("tls.allow_plaintext_password is for mode off only"));
        }
        if self.client_cert_pem.is_some() && self.client_cert_file.is_some()
            || self.client_key_pem.is_some() && self.client_key_file.is_some()
        {
            return Err(cfg(
                "tls: a client certificate or key inline or as a file, not both",
            ));
        }
        let cert = self.client_cert_pem.is_some() || self.client_cert_file.is_some();
        let key = self.client_key_pem.is_some() || self.client_key_file.is_some();
        if cert != key {
            return Err(cfg(
                "tls: a client certificate needs its key, and a key its certificate",
            ));
        }
        Trust::from_settings(self.ca_pem.as_deref(), self.pin_sha256.as_deref())
            .map_err(|e| MqttError::Config(e.to_string()))?;
        if let Some(pem) = &self.client_cert_pem {
            tls::parse_certs("client_cert_pem", pem)
                .map_err(|e| MqttError::Config(e.to_string()))?;
        }
        if let Some(pem) = &self.client_key_pem {
            tls::parse_key("client_key_pem", pem).map_err(|e| MqttError::Config(e.to_string()))?;
        }
        Ok(())
    }

    /// The client configuration for `mode: on` (`None` for off). Reads the PEM files.
    pub fn client_config(&self) -> Result<Option<Arc<ClientConfig>>, MqttError> {
        self.check()?;
        if self.mode == MqttTlsMode::Off {
            return Ok(None);
        }
        let config_err = |e: TlsError| MqttError::Config(e.to_string());
        let trust = Trust::from_settings(self.ca_pem.as_deref(), self.pin_sha256.as_deref())
            .map_err(config_err)?;
        let cert = match (&self.client_cert_pem, &self.client_cert_file) {
            (Some(pem), _) => Some(pem.clone()),
            (None, Some(path)) => Some(read_pem_file("client_cert_file", path, false)?),
            (None, None) => None,
        };
        let key = match (&self.client_key_pem, &self.client_key_file) {
            (Some(pem), _) => Some(pem.clone()),
            (None, Some(path)) => Some(read_pem_file("client_key_file", path, true)?),
            (None, None) => None,
        };
        let identity = match (cert, key) {
            (Some(c), Some(k)) => Some(ClientIdentity::from_pem(&c, &k).map_err(config_err)?),
            _ => None,
        };
        tls::client_config(&trust, identity.as_ref(), &[])
            .map(Some)
            .map_err(config_err)
    }

    /// The `tls` object as `mqtt.json` keeps it (0600, the client key included).
    pub fn to_json(&self) -> Value {
        self.json(false)
    }

    /// The `tls` object for `mqtt.get` and the diagnostics bundle: the client key as `set`
    /// or `not set`, never the key.
    pub fn view_json(&self) -> Value {
        self.json(true)
    }

    fn json(&self, view: bool) -> Value {
        let mut m = Map::new();
        let mode = match self.mode {
            MqttTlsMode::Off => "off",
            MqttTlsMode::On => "on",
        };
        m.insert("mode".into(), mode.into());
        let mut put = |key: &str, v: &Option<String>| {
            if let Some(v) = v {
                m.insert(key.into(), v.clone().into());
            }
        };
        put("ca_pem", &self.ca_pem);
        put("pin_sha256", &self.pin_sha256);
        put("client_cert_pem", &self.client_cert_pem);
        if view {
            m.insert(
                "client_key_pem".into(),
                shown(self.client_key_pem.is_some()).into(),
            );
        } else if let Some(k) = &self.client_key_pem {
            m.insert("client_key_pem".into(), k.clone().into());
        }
        for (key, path) in [
            ("client_cert_file", &self.client_cert_file),
            ("client_key_file", &self.client_key_file),
        ] {
            if let Some(p) = path {
                m.insert(key.into(), p.display().to_string().into());
            }
        }
        m.insert(
            "allow_plaintext_password".into(),
            self.allow_plaintext_password.into(),
        );
        if view {
            m.insert("encrypted".into(), self.encrypted().into());
        }
        Value::Object(m)
    }
}

/// What the client tells its handler. The handler runs on the client's thread: it should
/// hand the event on and return, and it must not drop the [`MqttClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttEvent {
    /// The broker accepted the connection.
    Connected {
        /// The broker kept a session for this client.
        session_present: bool,
    },
    /// A connection ended, or could not be made; the client tries again after its backoff.
    Disconnected {
        /// Why, for the log and `mqtt.status`. Never holds the password.
        reason: String,
    },
    /// A message on a subscribed topic.
    Message {
        /// Its topic.
        topic: String,
        /// Its payload.
        payload: Vec<u8>,
        /// The broker's retained message for the topic, sent on subscribing.
        retain: bool,
    },
    /// The broker refused a subscription (SUBACK 0x80).
    SubscribeRefused {
        /// The filter.
        filter: String,
    },
}

/// The client's state, for `mqtt.status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MqttStatus {
    /// Connected and accepted by the broker.
    pub connected: bool,
    /// Publishes waiting in the queue.
    pub queued: usize,
    /// QoS 1 publishes sent and not yet acknowledged.
    pub inflight: usize,
    /// Publishes dropped because the queue was full.
    pub dropped: u64,
    /// Connections the broker accepted since the start.
    pub connects: u64,
    /// Why the last connection ended or failed.
    pub last_error: Option<String>,
    /// Packets from the broker over `max_packet`, read past and dropped (the connection
    /// stays up, so an oversized retained message cannot cause a reconnect storm).
    pub oversized: u64,
    // -- WP SECOUT --
    /// The current connection is TLS.
    pub encrypted: bool,
}

/// Why the client did not take a call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MqttError {
    /// The configuration cannot make a valid CONNECT.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// Not a topic name to publish to (empty, a wildcard, NUL, too long).
    #[error("invalid topic name")]
    InvalidTopic,
    /// Not a valid topic filter.
    #[error("invalid topic filter")]
    InvalidFilter,
    /// The publish would be longer than `max_packet`.
    #[error("publish of {len} bytes is over the limit of {max}")]
    TooLarge {
        /// Its encoded length.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// Already [`MAX_SUBSCRIPTIONS`] filters.
    #[error("too many subscriptions")]
    TooManySubscriptions,
    /// The thread could not start.
    #[error("could not start the client thread: {0}")]
    Spawn(String),
}

/// The outgoing queue: bounded, dropping the oldest when full.
#[derive(Debug)]
struct OutQueue {
    cap: usize,
    items: VecDeque<Publish>,
    dropped: u64,
}

impl OutQueue {
    fn new(cap: usize) -> OutQueue {
        OutQueue {
            cap,
            items: VecDeque::new(),
            dropped: 0,
        }
    }

    fn push(&mut self, p: Publish) {
        if self.cap == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        while self.items.len() >= self.cap {
            self.items.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.items.push_back(p);
    }
}

/// The wait before the next connection attempt: doubling from `min` to `max`.
#[derive(Debug)]
struct Backoff {
    min: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    fn new(min: Duration, max: Duration) -> Backoff {
        let max = max.max(min);
        Backoff {
            min,
            max,
            next: min,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = self.next.saturating_mul(2).min(self.max);
        d
    }

    fn reset(&mut self) {
        self.next = self.min;
    }
}

#[derive(Debug)]
struct Shared {
    queue: OutQueue,
    subs: Vec<(String, QoS)>,
    new_subs: Vec<(String, QoS)>,
    stop: bool,
    status: MqttStatus,
    /// A clone of the connected socket, for [`MqttClient::stop`] to shut down under a
    /// write that a slow broker keeps alive (under TLS too: the TCP socket beneath).
    stream: Option<TcpStream>,
}

struct Inner {
    state: Mutex<Shared>,
    wake: Condvar,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The hub's MQTT client: a handle on its thread. Dropping it disconnects and stops it.
pub struct MqttClient {
    inner: Arc<Inner>,
    thread: Option<JoinHandle<()>>,
    max_packet: usize,
}

impl fmt::Debug for MqttClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttClient")
            .field("status", &self.status())
            .finish()
    }
}

impl MqttClient {
    /// Checks the configuration and starts the client's thread, which connects and keeps
    /// connecting. `handler` gets every [`MqttEvent`] on that thread.
    pub fn start(
        config: MqttConfig,
        handler: impl FnMut(MqttEvent) + Send + 'static,
    ) -> Result<MqttClient, MqttError> {
        if u16::try_from(config.keep_alive.as_secs()).is_err() {
            return Err(MqttError::Config("keep-alive over 65 535 s".into()));
        }
        if config.password.is_some() && config.username.is_none() {
            return Err(MqttError::Config("a password needs a user name".into()));
        }
        if config.max_packet > codec::MAX_REMAINING_LENGTH {
            return Err(MqttError::Config(
                "max_packet over the protocol's limit".into(),
            ));
        }
        if config.tick.is_zero() {
            return Err(MqttError::Config("tick must not be zero".into()));
        }
        // The CONNECT must encode, and fit.
        let connect = connect_packet(&config);
        let len = codec::encode(&connect)
            .map_err(|e| MqttError::Config(e.to_string()))?
            .len();
        if len > config.max_packet {
            return Err(MqttError::Config("CONNECT over max_packet".into()));
        }
        // -- WP SECOUT --
        let tls_config = config.tls.client_config()?;
        let inner = Arc::new(Inner {
            state: Mutex::new(Shared {
                queue: OutQueue::new(config.queue_cap),
                subs: Vec::new(),
                new_subs: Vec::new(),
                stop: false,
                status: MqttStatus::default(),
                stream: None,
            }),
            wake: Condvar::new(),
        });
        let max_packet = config.max_packet;
        let worker = Worker {
            backoff: Backoff::new(config.backoff_min, config.backoff_max),
            cfg: config,
            inner: Arc::clone(&inner),
            handler: Box::new(handler),
            inflight: VecDeque::new(),
            pending_subacks: VecDeque::new(),
            next_id: 1,
            tls_config,
        };
        let thread = std::thread::Builder::new()
            .name("dspi-mqtt".into())
            .spawn(move || worker.run())
            .map_err(|e| MqttError::Spawn(e.to_string()))?;
        Ok(MqttClient {
            inner,
            thread: Some(thread),
            max_packet,
        })
    }

    /// Queues a publish. It goes out when connected; while the broker is away the queue
    /// keeps the newest `queue_cap` publishes.
    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: QoS,
        retain: bool,
    ) -> Result<(), MqttError> {
        if !codec::valid_topic(topic) {
            return Err(MqttError::InvalidTopic);
        }
        let p = Publish {
            topic: topic.to_owned(),
            payload: payload.to_vec(),
            qos,
            retain,
            dup: false,
            packet_id: 0,
        };
        let len = publish_len(&p);
        if len > self.max_packet {
            return Err(MqttError::TooLarge {
                len,
                max: self.max_packet,
            });
        }
        let mut s = self.inner.lock();
        s.queue.push(p);
        s.status.queued = s.queue.items.len();
        s.status.dropped = s.queue.dropped;
        Ok(())
    }

    /// Subscribes to `filter`, now if connected and again after every reconnect. Asking for
    /// a filter already subscribed with the same QoS does nothing.
    pub fn subscribe(&self, filter: &str, qos: QoS) -> Result<(), MqttError> {
        if !codec::valid_filter(filter) {
            return Err(MqttError::InvalidFilter);
        }
        let mut s = self.inner.lock();
        if let Some(entry) = s.subs.iter_mut().find(|(f, _)| f == filter) {
            if entry.1 == qos {
                return Ok(());
            }
            entry.1 = qos;
        } else {
            if s.subs.len() >= MAX_SUBSCRIPTIONS {
                return Err(MqttError::TooManySubscriptions);
            }
            s.subs.push((filter.to_owned(), qos));
        }
        s.new_subs.retain(|(f, _)| f != filter);
        s.new_subs.push((filter.to_owned(), qos));
        Ok(())
    }

    /// The client's state now.
    pub fn status(&self) -> MqttStatus {
        self.inner.lock().status.clone()
    }

    /// Sends DISCONNECT if connected (so the broker does not publish the will) and stops
    /// the thread; waits for it unless called on that thread.
    ///
    /// Bounded: the thread looks at `stop` every `tick` and between publishes; if it has
    /// not finished within half a second (a broker that does not read), the socket is shut
    /// down under it. A TCP connect in progress ends within `connect_timeout`, a name
    /// lookup is waited for at most `tick`. So `stop` returns within about
    /// `connect_timeout`, whatever the broker does.
    pub fn stop(&mut self) {
        self.inner.lock().stop = true;
        self.inner.wake.notify_all();
        let Some(t) = self.thread.take() else { return };
        if t.thread().id() == std::thread::current().id() {
            return;
        }
        let grace = Instant::now().checked_add(STOP_GRACE);
        while !t.is_finished() && grace.is_some_and(|g| Instant::now() < g) {
            std::thread::sleep(Duration::from_millis(5));
        }
        if !t.is_finished()
            && let Some(stream) = self.inner.lock().stream.take()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let _ = t.join();
    }
}

impl Drop for MqttClient {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A publish's encoded length, without encoding it.
fn publish_len(p: &Publish) -> usize {
    let id = match p.qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 2,
    };
    let rem = p
        .topic
        .len()
        .saturating_add(2)
        .saturating_add(id)
        .saturating_add(p.payload.len());
    let header = match rem {
        0..=127 => 2usize,
        128..=16_383 => 3,
        16_384..=2_097_151 => 4,
        _ => 5,
    };
    rem.saturating_add(header)
}

fn connect_packet(cfg: &MqttConfig) -> Packet {
    Packet::Connect(Connect {
        client_id: cfg.client_id.clone(),
        keep_alive: u16::try_from(cfg.keep_alive.as_secs()).unwrap_or(u16::MAX),
        clean_session: true,
        will: cfg.will.clone(),
        username: cfg.username.clone(),
        password: cfg.password.as_ref().map(|p| p.as_bytes().to_vec()),
    })
}

/// Why a connection ended. Its text goes to events and the status; it never holds the
/// password.
#[derive(Debug, thiserror::Error)]
enum Dropped {
    #[error("{0}")]
    Io(String),
    #[error("the broker closed the connection")]
    Closed,
    #[error("the broker refused the connection: {}", codec::connack_reason(*.0))]
    Refused(u8),
    #[error("no CONNACK in time")]
    ConnackTimeout,
    #[error("no answer to PINGREQ in time")]
    KeepAlive,
    #[error("protocol error: {0}")]
    Protocol(&'static str),
    #[error("{0}")]
    Decode(#[from] DecodeError),
    #[error("stopped")]
    Stopped,
    // -- WP SECOUT --
    #[error(
        "refused to send the password unencrypted to {0}: turn TLS on, or set tls.allow_plaintext_password"
    )]
    PlaintextPassword(String),
    #[error("{0}")]
    Tls(#[from] TlsError),
}

impl From<std::io::Error> for Dropped {
    fn from(e: std::io::Error) -> Dropped {
        Dropped::Io(e.to_string())
    }
}

/// The bytes read and not yet decoded: at most one unfinished packet plus one read.
struct RxBuf {
    buf: Vec<u8>,
    max_packet: usize,
    /// Bytes of an oversized packet still to read past.
    skip: usize,
    /// Oversized packets read past on this connection.
    skipped: u64,
}

impl RxBuf {
    /// One read; a timeout is not an error.
    fn fill(&mut self, stream: &mut Stream) -> Result<(), Dropped> {
        let mut chunk = [0u8; READ_CHUNK];
        match stream.read(&mut chunk) {
            Ok(0) => Err(Dropped::Closed),
            Ok(n) => {
                let got = chunk.get(..n).unwrap_or(&[]);
                let dropped = self.skip.min(got.len());
                self.skip = self.skip.saturating_sub(dropped);
                self.buf
                    .extend_from_slice(got.get(dropped..).unwrap_or(&[]));
                Ok(())
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The next whole packet. A packet over `max_packet` is read past, not kept: its fixed
    /// header gives its length, so its bytes are dropped as they arrive and the next packet
    /// is read as usual. Anything else that is not a packet ends the connection.
    fn take(&mut self) -> Result<Option<Packet>, Dropped> {
        loop {
            if self.skip > 0 {
                return Ok(None);
            }
            match codec::decode(&self.buf, self.max_packet) {
                Ok(Some((p, used))) => {
                    // `decode` never reports more than it was given.
                    self.buf.drain(..used.min(self.buf.len()));
                    return Ok(Some(p));
                }
                Ok(None) => return Ok(None),
                Err(DecodeError::TooLarge { len, max }) => {
                    log::warn!(
                        "mqtt: dropped a packet of {len} bytes from the broker (limit {max})"
                    );
                    self.skipped = self.skipped.saturating_add(1);
                    let here = len.min(self.buf.len());
                    self.buf.drain(..here);
                    self.skip = len.saturating_sub(here);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

struct Worker {
    cfg: MqttConfig,
    inner: Arc<Inner>,
    handler: Box<dyn FnMut(MqttEvent) + Send>,
    /// QoS 1 publishes sent and not acknowledged, oldest first; kept across reconnects.
    inflight: VecDeque<Publish>,
    /// SUBSCRIBE packet ids waiting for SUBACK, with their filters.
    pending_subacks: VecDeque<(u16, Vec<String>)>,
    next_id: u16,
    backoff: Backoff,
    /// TLS to the broker, made once at start; `None` for plain TCP.
    tls_config: Option<Arc<ClientConfig>>,
}

impl Worker {
    fn run(mut self) {
        loop {
            if self.inner.lock().stop {
                return;
            }
            let result = self.session();
            {
                let mut s = self.inner.lock();
                s.stream = None;
                s.status.encrypted = false;
            }
            let reason = match result {
                Ok(()) => return,
                Err(e) => e.to_string(),
            };
            {
                let mut s = self.inner.lock();
                if s.stop {
                    s.status.connected = false;
                    return;
                }
                s.status.connected = false;
                s.status.last_error = Some(reason.clone());
            }
            log::info!("mqtt: {}:{}: {reason}", self.cfg.host, self.cfg.port);
            (self.handler)(MqttEvent::Disconnected { reason });
            let delay = self.backoff.next_delay();
            let until = Instant::now().checked_add(delay);
            let mut s = self.inner.lock();
            while !s.stop {
                let left = until.map_or(delay, |u| u.saturating_duration_since(Instant::now()));
                if left.is_zero() {
                    break;
                }
                s = self
                    .inner
                    .wake
                    .wait_timeout(s, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
    }

    fn stopping(&self) -> bool {
        self.inner.lock().stop
    }

    /// The broker's addresses. An IP address needs no lookup; a name is looked up on a
    /// short-lived thread of its own, because `to_socket_addrs` has no timeout: the client
    /// waits for it at most `connect_timeout` and looks at `stop` every `tick` meanwhile. A
    /// lookup that hangs is left to the system resolver's own timeout and its answer dropped.
    fn resolve(&self, deadline: Option<Instant>) -> Result<Vec<SocketAddr>, Dropped> {
        if let Ok(ip) = self.cfg.host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, self.cfg.port)]);
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let target = (self.cfg.host.clone(), self.cfg.port);
        std::thread::Builder::new()
            .name("dspi-mqtt-dns".into())
            .spawn(move || {
                let r = target
                    .to_socket_addrs()
                    .map(|a| a.collect::<Vec<_>>())
                    .map_err(|e| e.to_string());
                let _ = tx.send(r);
            })
            .map_err(|e| Dropped::Io(format!("name lookup: {e}")))?;
        loop {
            if self.stopping() {
                return Err(Dropped::Stopped);
            }
            match rx.recv_timeout(self.cfg.tick) {
                Ok(Ok(addrs)) => return Ok(addrs),
                Ok(Err(e)) => return Err(Dropped::Io(format!("{}: {e}", self.cfg.host))),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Dropped::Io(format!(
                        "{}: name lookup failed",
                        self.cfg.host
                    )));
                }
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(Dropped::Io(format!(
                    "{}: name lookup timed out",
                    self.cfg.host
                )));
            }
        }
    }

    /// A connection to the broker, all addresses together within `connect_timeout`, the
    /// TLS handshake included. Without TLS and with a password, only loopback addresses are
    /// tried unless `allow_plaintext_password` is set: nothing is sent to the others.
    fn open(&self) -> Result<Stream, Dropped> {
        let deadline = Instant::now().checked_add(self.cfg.connect_timeout);
        let mut addrs = self.resolve(deadline)?;
        // -- WP SECOUT --
        let guard_password = self.tls_config.is_none()
            && self.cfg.password.is_some()
            && !self.cfg.tls.allow_plaintext_password;
        if guard_password {
            let first = addrs.first().map(SocketAddr::ip);
            addrs.retain(|a| tls::is_loopback(a.ip()));
            if addrs.is_empty() {
                let shown = first.map_or_else(|| self.cfg.host.clone(), |ip| ip.to_string());
                return Err(Dropped::PlaintextPassword(shown));
            }
        }
        let mut last = Dropped::Io(format!("{}: no address", self.cfg.host));
        let mut tcp = None;
        for addr in addrs {
            if self.stopping() {
                return Err(Dropped::Stopped);
            }
            let left = deadline.map_or(self.cfg.connect_timeout, |d| {
                d.saturating_duration_since(Instant::now())
            });
            if left.is_zero() {
                return Err(Dropped::Io(format!("{}: connect timed out", self.cfg.host)));
            }
            match TcpStream::connect_timeout(&addr, left) {
                Ok(s) => {
                    tcp = Some(s);
                    break;
                }
                Err(e) => last = e.into(),
            }
        }
        let Some(tcp) = tcp else { return Err(last) };
        match &self.tls_config {
            None => Ok(Stream::Plain(tcp)),
            Some(config) => {
                let by = deadline.unwrap_or_else(Instant::now);
                Ok(tls::connect(tcp, Arc::clone(config), &self.cfg.host, by)?)
            }
        }
    }

    /// DISCONNECT (so the broker does not publish the will), then close.
    fn goodbye(stream: &mut Stream) -> Result<(), Dropped> {
        let _ = Self::send(stream, &Packet::Disconnect);
        stream.close();
        Ok(())
    }

    fn send(stream: &mut Stream, packet: &Packet) -> Result<(), Dropped> {
        let bytes =
            codec::encode(packet).map_err(|_| Dropped::Protocol("a packet did not encode"))?;
        stream.write_all(&bytes)?;
        // Under TLS, what was encrypted goes out here.
        stream.flush()?;
        Ok(())
    }

    fn take_id(&mut self) -> u16 {
        for _ in 0..=u16::MAX {
            let id = self.next_id;
            self.next_id = self.next_id.checked_add(1).unwrap_or(1);
            let used = self.inflight.iter().any(|p| p.packet_id == id)
                || self.pending_subacks.iter().any(|(p, _)| *p == id);
            if id != 0 && !used {
                return id;
            }
        }
        // Unreachable: at most MAX_INFLIGHT + MAX_PENDING_SUBACKS ids are in use.
        1
    }

    fn subscribe(&mut self, stream: &mut Stream, subs: &[(String, QoS)]) -> Result<(), Dropped> {
        for chunk in subs.chunks(FILTERS_PER_SUBSCRIBE) {
            let packet_id = self.take_id();
            Self::send(
                stream,
                &Packet::Subscribe {
                    packet_id,
                    filters: chunk.to_vec(),
                },
            )?;
            while self.pending_subacks.len() >= MAX_PENDING_SUBACKS {
                self.pending_subacks.pop_front();
            }
            self.pending_subacks
                .push_back((packet_id, chunk.iter().map(|(f, _)| f.clone()).collect()));
        }
        Ok(())
    }

    /// One connection, until it fails (`Err`) or the client stops (`Ok`).
    fn session(&mut self) -> Result<(), Dropped> {
        let mut stream = self.open()?;
        stream.tcp().set_read_timeout(Some(self.cfg.tick))?;
        stream
            .tcp()
            .set_write_timeout(Some(self.cfg.connect_timeout))?;
        stream.tcp().set_nodelay(true)?;
        {
            let mut s = self.inner.lock();
            if s.stop {
                return Ok(());
            }
            s.stream = stream.tcp().try_clone().ok();
        }
        let mut rx = RxBuf {
            buf: Vec::new(),
            max_packet: self.cfg.max_packet,
            skip: 0,
            skipped: 0,
        };
        Self::send(&mut stream, &connect_packet(&self.cfg))?;

        let deadline = Instant::now().checked_add(self.cfg.connect_timeout);
        let session_present = loop {
            if self.inner.lock().stop {
                stream.close();
                return Ok(());
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(Dropped::ConnackTimeout);
            }
            rx.fill(&mut stream)?;
            match rx.take()? {
                Some(Packet::Connack {
                    session_present,
                    return_code: 0,
                }) => break session_present,
                Some(Packet::Connack { return_code, .. }) => {
                    return Err(Dropped::Refused(return_code));
                }
                Some(_) => return Err(Dropped::Protocol("a packet before CONNACK")),
                None => {}
            }
        };

        // The backoff starts again only once this connection has stayed up (`stable_after`).
        let connected_at = Instant::now();
        let mut stable = false;
        let subs = {
            let mut s = self.inner.lock();
            s.status.connected = true;
            s.status.encrypted = stream.encrypted();
            s.status.connects = s.status.connects.saturating_add(1);
            s.status.last_error = None;
            s.new_subs.clear();
            s.subs.clone()
        };
        (self.handler)(MqttEvent::Connected { session_present });
        self.pending_subacks.clear();
        self.subscribe(&mut stream, &subs)?;
        for p in self.inflight.iter_mut() {
            p.dup = true;
        }
        for p in self.inflight.clone() {
            Self::send(&mut stream, &Packet::Publish(p))?;
        }

        let keep_alive = Duration::from_secs(self.cfg.keep_alive.as_secs());
        let mut last_sent = Instant::now();
        let mut ping_sent: Option<Instant> = None;
        loop {
            let (stop, new_subs) = {
                let mut s = self.inner.lock();
                (s.stop, std::mem::take(&mut s.new_subs))
            };
            if stop {
                return Self::goodbye(&mut stream);
            }
            if !new_subs.is_empty() {
                self.subscribe(&mut stream, &new_subs)?;
                last_sent = Instant::now();
            }
            // One publish at a time, `stop` looked at before each, at most MAX_BATCH_BYTES
            // per pass, so neither a long queue nor a slow broker holds the thread.
            let mut written: usize = 0;
            while written < MAX_BATCH_BYTES {
                let next = {
                    let mut s = self.inner.lock();
                    if s.stop {
                        drop(s);
                        return Self::goodbye(&mut stream);
                    }
                    let room = self.inflight.len() < MAX_INFLIGHT;
                    let p = match s.queue.items.front() {
                        Some(front) if front.qos == QoS::AtMostOnce || room => {
                            s.queue.items.pop_front()
                        }
                        _ => None,
                    };
                    s.status.queued = s.queue.items.len();
                    p
                };
                let Some(mut p) = next else { break };
                written = written.saturating_add(publish_len(&p));
                if p.qos == QoS::AtLeastOnce {
                    p.packet_id = self.take_id();
                    self.inflight.push_back(p.clone());
                }
                Self::send(&mut stream, &Packet::Publish(p))?;
                last_sent = Instant::now();
            }
            {
                let mut s = self.inner.lock();
                s.status.inflight = self.inflight.len();
                s.status.oversized = s.status.oversized.saturating_add(rx.skipped);
                rx.skipped = 0;
            }
            if !stable && connected_at.elapsed() >= self.cfg.stable_after {
                stable = true;
                self.backoff.reset();
            }

            let now = Instant::now();
            if !keep_alive.is_zero() {
                match ping_sent {
                    Some(t) if now.saturating_duration_since(t) >= keep_alive => {
                        return Err(Dropped::KeepAlive);
                    }
                    None if now.saturating_duration_since(last_sent) >= keep_alive => {
                        Self::send(&mut stream, &Packet::Pingreq)?;
                        last_sent = now;
                        ping_sent = Some(now);
                    }
                    _ => {}
                }
            }

            rx.fill(&mut stream)?;
            while let Some(packet) = rx.take()? {
                match packet {
                    Packet::Publish(p) => {
                        if p.qos == QoS::AtLeastOnce {
                            Self::send(
                                &mut stream,
                                &Packet::Puback {
                                    packet_id: p.packet_id,
                                },
                            )?;
                            last_sent = Instant::now();
                        }
                        (self.handler)(MqttEvent::Message {
                            topic: p.topic,
                            payload: p.payload,
                            retain: p.retain,
                        });
                    }
                    Packet::Puback { packet_id } => {
                        self.inflight.retain(|p| p.packet_id != packet_id);
                    }
                    Packet::Suback {
                        packet_id,
                        return_codes,
                    } => {
                        let pos = self
                            .pending_subacks
                            .iter()
                            .position(|(id, _)| *id == packet_id);
                        if let Some((_, filters)) = pos.and_then(|i| self.pending_subacks.remove(i))
                        {
                            for (filter, code) in filters.into_iter().zip(return_codes) {
                                if code == 0x80 {
                                    (self.handler)(MqttEvent::SubscribeRefused { filter });
                                }
                            }
                        }
                    }
                    Packet::Pingresp => ping_sent = None,
                    Packet::Connack { .. } => return Err(Dropped::Protocol("a second CONNACK")),
                    Packet::Connect(_)
                    | Packet::Subscribe { .. }
                    | Packet::Pingreq
                    | Packet::Disconnect => {
                        return Err(Dropped::Protocol("a client packet from the broker"));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2mqc {
    use super::*;

    fn publish(n: u8) -> Publish {
        Publish {
            topic: "t".into(),
            payload: vec![n],
            qos: QoS::AtMostOnce,
            retain: false,
            dup: false,
            packet_id: 0,
        }
    }

    #[test]
    fn wp_p2mqc_queue_drops_the_oldest() {
        let mut q = OutQueue::new(3);
        for n in 0..5 {
            q.push(publish(n));
        }
        let left: Vec<u8> = q.items.iter().map(|p| p.payload[0]).collect();
        assert_eq!(left, vec![2, 3, 4]);
        assert_eq!(q.dropped, 2);
        let mut none = OutQueue::new(0);
        none.push(publish(1));
        assert_eq!((none.items.len(), none.dropped), (0, 1));
    }

    #[test]
    fn wp_p2mqc_backoff_doubles_to_the_cap_and_resets() {
        let s = Duration::from_secs;
        let mut b = Backoff::new(s(1), s(5));
        let got: Vec<Duration> = (0..5).map(|_| b.next_delay()).collect();
        assert_eq!(got, vec![s(1), s(2), s(4), s(5), s(5)]);
        b.reset();
        assert_eq!(b.next_delay(), s(1));
    }

    #[test]
    fn wp_p2mqc_publish_len_matches_encode() {
        for (topic_len, payload_len, qos) in [
            (1usize, 0usize, QoS::AtMostOnce),
            (3, 122, QoS::AtMostOnce),
            (3, 123, QoS::AtMostOnce),
            (10, 16_370, QoS::AtLeastOnce),
            (10, 16_371, QoS::AtLeastOnce),
            (5, 2_097_200, QoS::AtMostOnce),
        ] {
            let p = Publish {
                topic: "x".repeat(topic_len),
                payload: vec![0; payload_len],
                qos,
                retain: false,
                dup: false,
                packet_id: if qos == QoS::AtLeastOnce { 1 } else { 0 },
            };
            assert_eq!(
                publish_len(&p),
                codec::encode(&Packet::Publish(p.clone())).unwrap().len()
            );
        }
    }

    #[test]
    fn wp_p2mqc_oversized_packet_is_read_past() {
        let big = codec::encode(&Packet::Publish(Publish {
            topic: "set/big".into(),
            payload: vec![7; 2000],
            qos: QoS::AtMostOnce,
            retain: true,
            dup: false,
            packet_id: 0,
        }))
        .unwrap();
        let ok = codec::encode(&Packet::Pingresp).unwrap();
        // Whole in the buffer: dropped, and the next packet read.
        let mut rx = RxBuf {
            buf: [big.clone(), ok.clone()].concat(),
            max_packet: 1024,
            skip: 0,
            skipped: 0,
        };
        assert_eq!(rx.take().unwrap(), Some(Packet::Pingresp));
        assert_eq!((rx.skipped, rx.skip, rx.buf.len()), (1, 0, 0));
        // Only its head in: the rest is still to skip, nothing is kept.
        let mut rx = RxBuf {
            buf: big[..100].to_vec(),
            max_packet: 1024,
            skip: 0,
            skipped: 0,
        };
        assert_eq!(rx.take().unwrap(), None);
        assert_eq!((rx.skip, rx.buf.len()), (big.len() - 100, 0));
    }

    #[test]
    fn wp_p2mqc_config_debug_hides_password() {
        let mut c = MqttConfig::new("broker", DEFAULT_MQTT_PORT, "hub");
        c.username = Some("u".into());
        c.password = Some("hunter2".into());
        let d = format!("{c:?}");
        assert!(!d.contains("hunter2"));
        assert!(d.contains("\"set\""));
    }

    #[test]
    fn wp_p2mqc_start_refuses_bad_config() {
        let mut c = MqttConfig::new("127.0.0.1", 1, "hub");
        c.password = Some("p".into());
        assert!(matches!(
            MqttClient::start(c, |_| {}),
            Err(MqttError::Config(_))
        ));
        let mut c = MqttConfig::new("127.0.0.1", 1, "hub");
        c.keep_alive = Duration::from_secs(70_000);
        assert!(matches!(
            MqttClient::start(c, |_| {}),
            Err(MqttError::Config(_))
        ));
    }
}
