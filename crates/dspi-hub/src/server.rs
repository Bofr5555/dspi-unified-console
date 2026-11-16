//! The network front: WebSocket and HTTP on one port (spec 4). Owned by WP HB1.
//!
//! HUB.md section 2 gives the threads: one acceptor, and per connection a reader
//! (frames in, [`crate::HubHandle::submit`]) and a writer (the session's outbox out).
//! The server holds no protocol state of its own beyond a session's handshake.
//!
//! **Origin (spec 4):** a browser sends `Origin` with every upgrade, a native client
//! none. An upgrade whose `Origin` is not the hub itself is refused with HTTP 403.
//! The comparison is against [`OwnNames`], never the request's `Host`, which a
//! hostile page controls through DNS rebinding.
//!
//! **Everyone on the LAN can reach this, before any authentication.** What each of them
//! can cost is bounded here:
//!
//! | Cost | Bound |
//! |---|---|
//! | Sockets and threads | Two budgets (WP HB9). Plain HTTP, from accept until a request upgrades: [`ServerLimits::max_http`], of which [`ServerLimits::max_http_per_peer`] from one address; past either an idle socket (the address's own, else the fullest address's) is closed to make room, and only if none is idle the answer is HTTP 503. WebSockets, from the upgrade: [`ServerLimits::max_connections`], of which [`ServerLimits::max_preauth_per_peer`] unauthenticated from one address; past either the upgrade is answered 503. A 503 at accept is on a short-lived thread, at most [`ServerLimits::max_refusing`] at a time; past that, closed unread |
//! | A socket that says nothing | Its first request's head within [`ServerLimits::head_timeout`], a later one's start within [`ServerLimits::keepalive_idle`], and any head within [`ServerLimits::request_timeout`] of its first byte, however slowly it trickles; reclaimed sooner while it waits if room is needed; authenticated within [`ServerLimits::auth_timeout`] of the upgrade, or close 4001 |
//! | Memory per socket | A head of [`MAX_HEAD`] bytes; a frame or message of [`MAX_FRAME`], refused from its header before it is buffered |
//! | A peer that stops reading | One write's [`ServerLimits::write_stall`]; an HTTP response's whole [`ServerLimits::response_timeout`]. Behind that, the outbox's limits (HUB.md section 6) |
//! | A peer that stops answering | [`ServerLimits::dead_peer`], close 1001. A frame trickled and never finished does not count as an answer |
//! | Guessing | [`crate::auth`]'s limits, and a token is not guessable |
//!
//! -- WP SECSRV -- **TLS on the same port (PLAN-SEC 4.1, 4.6, 4.7).** With [`TlsMode::On`]
//! or [`TlsMode::Require`] a connection's first byte decides: `0x16`, a TLS handshake
//! record, is TLS 1.3 (dspi-link's [`TlsPipe`], the hub's key from
//! [`crate::tls_identity`]); anything else is plain HTTP. Everything on the port (the
//! WebSocket, `/dspi/v1/info`, the pages, REST) is served over both. The handshake is
//! part of the first request's [`ServerLimits::head_timeout`], on a socket counted in
//! the plain HTTP budget from accept, and reclaimable like any other there: a slow or
//! half-open handshake is a slowloris like any other. [`TlsMode::Off`] serves the
//! protocol as before, and answers a ClientHello with a plain `400` at once.
//!
//! | Over plaintext | `off` | `on` | `on` + upstream compatibility | `require` |
//! |---|---|---|---|---|
//! | pages, `/info`, a token on the WebSocket | yes | yes | yes | `426`; the WebSocket upgraded, then closed 4003 `tls_required` |
//! | `auth.pair` with a PIN | yes | refused, `tls_required` | yes | no |
//! | REST with an API key | yes | only a key minted `plaintext_ok` | every key | no |
//!
//! **`Host`.** Over plaintext every request's `Host` must be an IP address, a name
//! with no dot, or one of the hub's own names ([`OwnNames`], plus
//! [`NetworkConfig::host_names`]): a page that a DNS name was rebound under cannot read
//! `/info` or the assets (421). Over TLS a rebound name fails the browser's certificate
//! check, and a native client pins the key, so any name the client used is fine.
//!
//! Lock level: the connection table is a leaf. Nothing else is taken under it, and
//! nothing blocks under it. -- WP SECSRV -- The TLS state's locks are leaves too, the
//! identity's taken before the config's.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dspi_link::discovery::AdvertiseScope;
use dspi_link::frames::{CmdResponse, Frame, NotifyFrame, PollFrame, kind};
use dspi_link::messages::{PROTO_MAJOR, PROTO_MINOR, PollSpec};
use dspi_link::socket::{Pipe, server_pipes, server_ws_config};
// -- WP SECSRV --
use dspi_link::status;
use dspi_link::tls::{self as link_tls, SpkiHash, TlsPipe};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Map, Value, json};
use socket2::{Domain, Protocol, Socket, Type};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role as WsRole};
use tungstenite::{Message, WebSocket};

use crate::auth::{AccessMode, AuthError, AuthStore, peer_key};
use crate::error::HubError;
use crate::hub::{HubHandle, MAX_INFLIGHT};
use crate::polls::{POLL_BUDGET_BPS, POLL_MAX_HZ};
use crate::router::MAX_PAYLOAD;
use crate::session::{CLOSE_REVOKED, CLOSE_SHUTTING_DOWN, Outbound, Outbox, Pop, Role, Session};
use crate::store::{parse_role, role_name, unix_seconds};
// -- WP SECSRV --
use crate::tls_identity::{TlsIdentity, TlsIdentityEvent, TlsNames};
use std::time::SystemTime;

/// `limits.max_frame`: the largest WebSocket frame and message taken (spec 4).
pub const MAX_FRAME: u32 = 65536;
/// The largest HTTP request head read. Past it the answer is 431.
pub const MAX_HEAD: usize = 8192;
/// Headers per request. Past it the answer is 400.
const MAX_HEADERS: usize = 64;

const SUBPROTOCOL: &str = "dspi-link-1";
const WS_PATH: &str = "/dspi/v1";
const INFO_PATH: &str = "/dspi/v1/info";
/// Polling the listener, which has no portable way to be woken. A new client waits this
/// long at most, and it leaves no way for `stop_accepting` to hang.
const ACCEPT_TICK: Duration = Duration::from_millis(50);

/// Close: protocol error (spec 5). A client does not reconnect.
const CLOSE_PROTOCOL: u16 = 4000;
/// Close: not authenticated in time (spec 5).
const CLOSE_AUTH_TIMEOUT: u16 = 4001;
/// Close: nothing heard for too long (spec 5).
const CLOSE_KEEPALIVE: u16 = 1001;
/// Close: normal (spec 5).
const CLOSE_NORMAL: u16 = 1000;
/// Close reasons travel in a control frame, 125 bytes with its code.
const MAX_CLOSE_REASON: usize = 123;

// -- WP SECSRV --
/// The close reason of a plaintext WebSocket on a [`TlsMode::Require`] hub, with code
/// 4003: a client that knows it reads the word before the colon.
pub const TLS_REQUIRED_REASON: &str = "tls_required: this hub accepts encrypted connections only";
/// The error code of a request refused for want of TLS (`auth.pair`, REST).
pub const TLS_REQUIRED: &str = "tls_required";
/// How often the certificate is looked at for its end (and a rotation's announcement
/// for its end), besides a change of the names it carries.
const TLS_REFRESH_EVERY: Duration = Duration::from_secs(3600);
/// How often the advertised name is compared with the certificate's.
const TLS_NAMES_EVERY: Duration = Duration::from_secs(1);
/// How long the hub's own names are kept before the interfaces are listed again.
const OWN_NAMES_FOR: Duration = Duration::from_secs(1);

// -- WP SECSRV --
/// TLS on the Link port (PLAN-SEC 4.1; `uconsole hub --tls`). The library's default is
/// [`TlsMode::Off`], so an embedder chooses; `uconsole hub` defaults to
/// [`TlsMode::Require`] (OD-SECb, reconsidered 2026-11-15) and to `Off` on loopback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// Plaintext only, the protocol as before, and no key is made. The first byte is
    /// still peeked (not taken), so a ClientHello is answered `400` at once instead of
    /// waiting out the head timeout.
    #[default]
    Off,
    /// TLS and plaintext both, by a connection's first byte. Over plaintext a PIN
    /// pairing and an API key not minted `plaintext_ok` are refused, unless
    /// [`NetworkConfig::upstream_compat`].
    On,
    /// TLS only. Plaintext HTTP is answered `426`; a plaintext WebSocket upgrade is
    /// completed and closed at once with 4003 [`TLS_REQUIRED_REASON`].
    Require,
}

impl TlsMode {
    /// `off`, `on` or `require`, as `--tls` takes it.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "require" => Some(Self::Require),
            _ => None,
        }
    }

    /// As [`Self::parse`] reads it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Require => "require",
        }
    }
}

// -- WP SECSRV --
/// What the hub serves over which transport: a [`TlsMode`] and the upstream
/// compatibility switch, checked together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TlsPolicy {
    /// [`NetworkConfig::tls`].
    pub mode: TlsMode,
    /// [`NetworkConfig::upstream_compat`].
    pub upstream_compat: bool,
}

impl TlsPolicy {
    /// Whether TLS connections are taken.
    pub fn serves_tls(self) -> bool {
        self.mode != TlsMode::Off
    }

    /// Whether plaintext connections get past the `426`.
    pub fn serves_plaintext(self) -> bool {
        self.mode != TlsMode::Require
    }

    /// Whether `auth.pair` is taken over plaintext.
    pub fn plaintext_pairing(self) -> bool {
        match self.mode {
            TlsMode::Off => true,
            TlsMode::On => self.upstream_compat,
            TlsMode::Require => false,
        }
    }

    /// Whether every API key is taken over plaintext, not only one minted `plaintext_ok`.
    pub fn plaintext_api_keys(self) -> bool {
        self.plaintext_pairing()
    }

    /// Whether this is a downgrade the user must be shown [`downgrade_warning`] for.
    /// `loopback`: the hub is bound to loopback, where nothing crosses a network.
    pub fn is_downgraded(self, loopback: bool) -> bool {
        !loopback && (self.mode == TlsMode::Off || self.upstream_compat)
    }
}

// -- WP SECSRV --
/// What upstream compatibility mode turns off, exactly, one line each: for every
/// client's hub settings (SECPAIR, SECWEB, SECQT, SECAN, SECSW) and `uconsole hub`.
pub const UPSTREAM_COMPAT_EFFECTS: &[&str] = &[
    "TLS is no longer required: the hub also serves unencrypted DSPi Link (ws:// and \
     http://) on its port, so upstream DSPi Link clients can connect",
    "on those connections nothing is encrypted: anyone on this network can read the \
     traffic, including PINs, tokens and API keys, and change it in transit",
    "the hub's key is not pinned on them: an unencrypted client cannot tell this hub \
     from another machine posing as it",
    "pairing with a PIN is allowed without encryption (otherwise refused), so a \
     listener can take the token a new client is given",
    "every REST API key works without encryption, not only keys minted plaintext_ok",
    "the hub does not advertise that it requires TLS (no tls=1 in its DNS-SD record)",
];

// -- WP SECSRV --
/// What `--tls off` on a network address turns off, one line each.
pub const TLS_OFF_EFFECTS: &[&str] = &[
    "nothing on this port is encrypted: anyone on this network can read the traffic, \
     including PINs, tokens and API keys, and change it in transit",
    "no client can verify this hub's key (no key pinning): another machine can pose as it",
    "pairing and every REST API key work without encryption",
];

// -- WP SECSRV --
/// The large warning a downgraded hub is started with ([`TlsPolicy::is_downgraded`]),
/// listing exactly which protections are off. `None` when nothing is downgraded.
pub fn downgrade_warning(policy: TlsPolicy, loopback: bool) -> Option<String> {
    if !policy.is_downgraded(loopback) {
        return None;
    }
    let (title, effects) = if policy.mode == TlsMode::Off {
        ("TLS IS OFF", TLS_OFF_EFFECTS)
    } else {
        ("UPSTREAM COMPATIBILITY MODE IS ON", UPSTREAM_COMPAT_EFFECTS)
    };
    let rule = "!".repeat(72);
    let mut text = format!("{rule}\n!! SECURITY DOWNGRADED: {title}\n!!\n");
    for effect in effects {
        text.push_str("!!  - ");
        text.push_str(effect);
        text.push('\n');
    }
    text.push_str("!!\n!! Use it only on a network you trust, and only while you need it.\n");
    text.push_str(&rule);
    Some(text)
}

/// What one socket may cost. The defaults are the spec's where it names one. Tests
/// shorten the times.
///
/// -- WP HB9 -- Two budgets. A socket counts as plain HTTP from accept until its request
/// upgrades: the page's files, `/dspi/v1/info`, a browser's speculative connects. Only
/// at the upgrade does it move into the WebSocket budget, so no number of page loads can
/// keep a Link client or the app out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLimits {
    /// WebSockets at once, authenticated or not. Each costs up to two threads, and up to
    /// `max_inflight` queued commands. Past it the upgrade is answered HTTP 503.
    pub max_connections: usize,
    /// Unauthenticated WebSockets from one address (an IPv6 /64 counts as one), so one
    /// machine cannot hold every slot. Past it the upgrade is answered HTTP 503.
    pub max_preauth_per_peer: usize,
    // -- WP HB9 --
    /// Plain HTTP sockets at once (not yet upgraded), one thread each. More than two
    /// addresses' worth, so one dual-stack machine (two addresses) cannot fill it.
    pub max_http: usize,
    /// Plain HTTP sockets from one address. A browser opens up to six per host and
    /// pre-connects more, per window or profile. Past it the address's own idlest
    /// socket is closed to make room; only if none is idle, HTTP 503.
    pub max_http_per_peer: usize,
    /// Threads answering 503 at once. Past it a socket is closed unanswered.
    pub max_refusing: usize,
    /// From accept to the whole head of a socket's first request.
    pub head_timeout: Duration,
    // -- WP HB9 --
    /// From a request's first byte to its whole head, however slowly it trickles.
    pub request_timeout: Duration,
    /// How long a kept-alive socket may wait for its next request.
    pub keepalive_idle: Duration,
    /// From a response's first byte to its last, however steadily the peer takes it.
    /// Each write must also progress within [`Self::write_stall`].
    pub response_timeout: Duration,
    /// From the upgrade to a successful authentication (spec 5).
    pub auth_timeout: Duration,
    /// Between pings to an authenticated peer (spec 4).
    pub ping_interval: Duration,
    /// Silence before a peer counts as dead (spec 4).
    pub dead_peer: Duration,
    /// How long a write may make no progress (spec 5, close 1011).
    pub write_stall: Duration,
    /// How long a peer gets to take a close frame.
    pub close_grace: Duration,
    /// The reader's wake-up, to check the limits above.
    pub read_tick: Duration,
    // -- WP P2RS -- the REST mirror's (`rest.rs`)
    /// The largest REST request body taken, by its `Content-Length`. Past it: 413,
    /// unread.
    pub rest_max_body: usize,
    /// From the end of a REST request's head to the last byte of its body, however
    /// slowly it trickles. Past it: 408.
    pub rest_body_timeout: Duration,
    /// REST requests per second one key may make, on average.
    pub rest_rate: u32,
    /// REST requests one key may make at once after a quiet spell. Past it and the
    /// rate: 429.
    pub rest_burst: u32,
    /// How long a key's session stays open with no request: its parameter cores go
    /// with it.
    pub rest_idle: Duration,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            max_connections: 16,
            max_preauth_per_peer: 4,
            // -- WP HB9 --
            max_http: 48,
            max_http_per_peer: 16,
            max_refusing: 4,
            head_timeout: Duration::from_secs(10),
            // -- WP HB9 --
            request_timeout: Duration::from_secs(3),
            keepalive_idle: Duration::from_secs(5),
            response_timeout: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(10),
            dead_peer: Duration::from_secs(30),
            write_stall: Duration::from_secs(10),
            close_grace: Duration::from_secs(1),
            read_tick: Duration::from_secs(1),
            // -- WP P2RS --
            rest_max_body: crate::rest::MAX_BODY,
            rest_body_timeout: Duration::from_secs(3),
            rest_rate: 10,
            rest_burst: 20,
            rest_idle: Duration::from_secs(60),
        }
    }
}

/// What the network side of a hub is configured with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConfig {
    /// The port; 0 lets the OS choose. [`dspi_link::DEFAULT_PORT`] by default.
    pub port: u16,
    /// PIN by default. Open only if the user asks.
    pub access: AccessMode,
    /// Serve the web UI on this port while sharing (WP HB5).
    pub web: bool,
    /// Where the hub keeps its [`crate::store::HubStore`].
    pub store_dir: PathBuf,
    // -- WP HB1 --
    /// The address to listen on. By default every interface, IPv6 and IPv4, which with
    /// the peer check is what "LAN interfaces" comes to (spec 6.4). Tests use loopback.
    pub bind: IpAddr,
    /// The mDNS `.local` name of this hub, for the `Origin` check (WP HB5).
    pub mdns_name: Option<String>,
    /// What one socket may cost.
    pub limits: ServerLimits,
    // -- WP HB5 --
    /// Whether and where the hub says it is there.
    pub advertise: Advertise,
    // -- WP TM26 --
    /// Bind, and accept nothing until [`NetworkControl::start_accepting`]: the port is
    /// the hub's while the embedder attaches its devices, so no client is told "no
    /// device" by a hub that is still starting. A client that connects meanwhile waits
    /// in the listen backlog.
    pub hold_accepts: bool,
    // -- WP WB7 --
    /// The web console, served at [`crate::web::CONSOLE_PATH`] beside the page at `/`
    /// while `web` is on. `None`: not served (404).
    pub console: Option<Arc<crate::web::Console>>,
    // -- WP P2RS --
    /// The REST mirror under `/api/v1/` (`uconsole hub --rest`). Off by default, and
    /// then `/api/v1/` answers 404.
    pub rest: bool,
    // -- WP SECSRV --
    /// TLS on the port (`--tls`). [`TlsMode::Off`] by default: see [`TlsMode`]. The
    /// key and certificate are kept under `store_dir` ([`crate::tls_identity`]).
    pub tls: TlsMode,
    /// Upstream compatibility (`--upstream-compat`, owner-approved, off by default):
    /// with [`TlsMode::On`], PIN pairing and every API key over plaintext too, for
    /// upstream DSPi Link clients. [`UPSTREAM_COMPAT_EFFECTS`] lists what it turns off;
    /// the embedder shows [`downgrade_warning`]. Refused with [`TlsMode::Require`].
    pub upstream_compat: bool,
    /// More names the hub answers to over plaintext (the `Host` rule), lower case:
    /// a DNS name it is reached by that is not its mDNS name (`--host-name`).
    pub host_names: Vec<String>,
}

// -- WP SECSRV --
/// The TLS mode [`NetworkConfig::new`] starts with: [`TlsMode::Off`], or under the
/// test-only feature `test-tls-mode`, `DSPI_HUB_TEST_TLS` (`off`, `on`, `compat`: on
/// with upstream compatibility, `require`), so the existing suites run against a hub
/// that sniffs every connection. The existing suites are meant for `off` and `compat`,
/// where their plaintext clients pair and use REST as before (their assertions on
/// `/info`'s `tls`, an `https` own Origin and a ClientHello follow the mode). In `on`
/// and `require`, plaintext pairing and REST are refused by design, so every test that
/// pairs over plaintext fails there.
fn default_tls() -> (TlsMode, bool) {
    #[cfg(feature = "test-tls-mode")]
    {
        match std::env::var("DSPI_HUB_TEST_TLS").as_deref() {
            Ok("on") => return (TlsMode::On, false),
            Ok("compat") => return (TlsMode::On, true),
            Ok("require") => return (TlsMode::Require, false),
            _ => {}
        }
    }
    (TlsMode::Off, false)
}

// -- WP HB5 --
/// Whether and where a hub advertises itself (spec 3.1). Needs feature `mdns`; a build
/// without it advertises nothing, whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Advertise {
    /// On every interface. A hub bound to loopback stays silent, as nobody else could
    /// reach it, so a hub in a test cannot reach the network by this.
    #[default]
    Lan,
    /// On the loopback interface alone, whatever the hub is bound to. For tests.
    LoopbackOnly,
    /// Not at all: reachable by a typed address.
    Off,
}

impl NetworkConfig {
    // -- WP HB5 --
    /// Where this configuration advertises. `None` for not at all.
    pub(crate) fn advertise_scope(&self) -> Option<AdvertiseScope> {
        if !crate::advertise::AVAILABLE {
            return None;
        }
        match self.advertise {
            Advertise::Lan => (!self.bind.is_loopback()).then_some(AdvertiseScope::AllInterfaces),
            Advertise::LoopbackOnly => Some(AdvertiseScope::LoopbackOnly),
            Advertise::Off => None,
        }
    }

    // -- WP SECSRV --
    /// Whether the DNS-SD record says TLS (spec 3.1's `tls=1`; SECDISC's
    /// `HubAdvert.tls`): whenever TLS is served (`on`, `require`), but not in upstream
    /// compatibility mode, where an upstream client would read it as "use `wss://`".
    pub(crate) fn advertises_tls(&self) -> bool {
        self.tls != TlsMode::Off && !self.upstream_compat
    }

    /// The defaults: the standard port, PIN pairing, no web UI.
    pub fn new(store_dir: PathBuf) -> Self {
        Self {
            port: dspi_link::DEFAULT_PORT,
            access: AccessMode::default(),
            web: false,
            store_dir,
            bind: IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            mdns_name: None,
            limits: ServerLimits::default(),
            // -- WP HB5 --
            advertise: Advertise::default(),
            // -- WP TM26 --
            hold_accepts: false,
            // -- WP WB7 --
            console: None,
            // -- WP P2RS --
            rest: false,
            // -- WP SECSRV --
            tls: default_tls().0,
            upstream_compat: default_tls().1,
            host_names: Vec::new(),
        }
    }
}

/// The names that count as the hub itself, for the `Origin` check: its interfaces'
/// addresses, its mDNS `.local` name and loopback, all on its bound port. Built per
/// upgrade, since interfaces change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnNames {
    /// Host names and literal addresses, lower case, without brackets.
    pub hosts: Vec<String>,
    /// The bound port.
    pub port: u16,
}

impl OwnNames {
    fn current(port: u16, mdns_name: Option<&str>) -> Self {
        let mut hosts: Vec<String> = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
        // A failed listing leaves loopback only, which refuses more, never less.
        match if_addrs::get_if_addrs() {
            Ok(interfaces) => hosts.extend(interfaces.iter().map(|i| i.ip().to_string())),
            Err(e) => log::warn!("hub: cannot list interfaces: {e}"),
        }
        hosts.extend(mdns_name.map(str::to_ascii_lowercase));
        Self { hosts, port }
    }
}

/// `host` and `port` from `http://host[:port]`, the exact shape a browser sends, or
/// `None` for anything else: another scheme (`https://` a hub does not serve is not
/// its own page), a path, credentials, a port spelt oddly. Host lower case, no brackets.
fn parse_http_origin(origin: &str) -> Option<(String, u16)> {
    parse_origin(origin, "http://", 80)
}

// -- WP SECSRV --
/// [`parse_http_origin`] for `scheme` (`http://` or `https://`) and its default port.
fn parse_origin(origin: &str, scheme: &str, default_port: u16) -> Option<(String, u16)> {
    let rest = origin
        .get(..scheme.len())
        .filter(|s| s.eq_ignore_ascii_case(scheme))
        .and_then(|_| origin.get(scheme.len()..))?;
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let ip: Ipv6Addr = host.parse().ok()?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':')?),
        };
        (ip.to_string(), port)
    } else {
        match rest.split_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), Some(port)),
            None => (rest.to_ascii_lowercase(), None),
        }
    };
    let host_is_plain = host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
    if host.is_empty() || !host_is_plain {
        return None;
    }
    let port = match port {
        None => default_port,
        // Digits only (u16's parser takes `+11915`), and as a browser would write it.
        Some(p) if p.bytes().all(|b| b.is_ascii_digit()) && !p.starts_with('0') => {
            p.parse().ok()?
        }
        Some(_) => return None,
    };
    Some((host, port))
}

/// Whether a WebSocket upgrade with this `Origin` header may go on. No header (a
/// native client) passes; `null`, a foreign site, another port or a malformed value
/// does not. The scheme must be `http`, the only one a hub serves.
pub fn origin_allowed(origin: Option<&str>, own: &OwnNames) -> bool {
    origin_allowed_with(origin, own, false)
}

// -- WP SECSRV --
/// [`origin_allowed`] on a hub that also serves TLS (`https`): its own page is then
/// `https://<own name>:<port>` too (PLAN-SEC 4.6). A foreign `https` origin is refused
/// as a foreign `http` one is.
pub fn origin_allowed_with(origin: Option<&str>, own: &OwnNames, https: bool) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let ours = |(host, port): (String, u16)| {
        port == own.port && own.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host))
    };
    parse_http_origin(origin).is_some_and(ours)
        || (https && parse_origin(origin, "https://", 443).is_some_and(ours))
}

// -- WP SECSRV --
/// The host of a `Host` header (`name[:port]`, `[v6][:port]`), lower case, no brackets;
/// `None` when it is not that shape.
fn host_of(authority: &str) -> Option<String> {
    parse_http_origin(&format!("http://{}", authority.trim())).map(|(host, _)| host)
}

// -- WP SECSRV --
/// Whether a plaintext request's `Host` may be answered (DNS rebinding, PLAN-SEC 4.6):
/// none at all (not a browser: every browser sends one), an IP address (a page cannot
/// be rebound under one), a name with no dot (no public DNS name is one), or one of
/// the hub's own names. A repeated or malformed one is refused.
///
/// `own` is called only for a dotted name (review secb N2).
fn host_allowed(request: &httparse::Request<'_, '_>, own: impl FnOnce() -> OwnNames) -> bool {
    let Ok(host) = single_header(request, "Host") else {
        return false;
    };
    let Some(host) = host else {
        return true;
    };
    host_of(host).is_some_and(|name| {
        name.parse::<IpAddr>().is_ok()
            || !name.contains('.')
            || own().hosts.iter().any(|h| h.eq_ignore_ascii_case(&name))
    })
}

/// Whether a peer may connect at all: on the hub's own network, not arriving through
/// a router's port forward (spec 6.4). Private and link-local ranges, CGNAT (where a
/// Tailscale peer arrives from, which the spec recommends), loopback, and anything
/// inside the subnet of one of this machine's interfaces (a LAN on public addresses,
/// which is usual for IPv6).
pub(crate) fn is_local_peer(peer: IpAddr) -> bool {
    match peer.to_canonical() {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || (a == 100 && (64..=127).contains(&b))
                || in_local_subnet(peer.to_canonical())
        }
        IpAddr::V6(ip) => {
            let first = ip.segments().first().copied().unwrap_or(0);
            ip.is_loopback()
                || (first & 0xFFC0) == 0xFE80 // link-local
                || (first & 0xFE00) == 0xFC00 // unique local
                || in_local_subnet(IpAddr::V6(ip))
        }
    }
}

fn in_local_subnet(peer: IpAddr) -> bool {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return false;
    };
    interfaces.iter().any(|i| match (&i.addr, peer) {
        (if_addrs::IfAddr::V4(a), IpAddr::V4(p)) => {
            let mask = u32::from(a.netmask);
            u32::from(a.ip) & mask == u32::from(p) & mask
        }
        (if_addrs::IfAddr::V6(a), IpAddr::V6(p)) => {
            let mask = u128::from(a.netmask);
            u128::from(a.ip) & mask == u128::from(p) & mask
        }
        _ => false,
    })
}

/// The listener, IPv6 with IPv4 on the same socket where the address is `::`. A host
/// without IPv6 gets IPv4 only.
fn bind(addr: IpAddr, port: u16) -> io::Result<TcpListener> {
    let listen = |ip: IpAddr| -> io::Result<TcpListener> {
        let domain = if ip.is_ipv6() {
            Domain::IPV6
        } else {
            Domain::IPV4
        };
        let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        if ip.is_ipv6() {
            socket.set_only_v6(false)?;
        }
        // So a restarted hub gets its port back. On Windows the same option would let
        // another program take the port from a running one, and it is not needed there.
        #[cfg(unix)]
        socket.set_reuse_address(true)?;
        socket.bind(&SocketAddr::new(ip, port).into())?;
        socket.listen(128)?;
        Ok(socket.into())
    };
    if addr == IpAddr::V6(Ipv6Addr::UNSPECIFIED) {
        match listen(addr) {
            Err(e) if e.kind() != io::ErrorKind::AddrInUse => {
                log::info!("hub: no IPv6 ({e}), listening on IPv4 only");
                listen(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            }
            result => result,
        }
    } else {
        listen(addr)
    }
}

fn after(now: Instant, wait: Duration) -> Instant {
    now.checked_add(wait).unwrap_or(now)
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data consistent before it can panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Being answered 503. Not a connection for the limits.
    Refusing,
    // -- WP HB9 --
    /// Plain HTTP: not upgraded (yet). [`ServerLimits::max_http`]'s.
    Http(Http),
    /// A WebSocket without a session yet.
    PreAuth,
    Session,
}

// -- WP HB9 --
/// Where a plain HTTP socket is, for choosing one to close when room is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Http {
    /// Waiting for a request to start, since then: a browser's spare or kept-alive
    /// socket. Closed first.
    Idle(Instant),
    /// A request's head has started, since then. Closed next.
    Head(Instant),
    /// Being answered. Never closed for room: bounded by the response's own times.
    Busy,
}

impl Http {
    /// Whether and how readily it may be closed to make room: lower goes first.
    fn reclaim_order(self) -> Option<(u8, Instant)> {
        match self {
            Self::Idle(since) => Some((0, since)),
            Self::Head(since) => Some((1, since)),
            Self::Busy => None,
        }
    }
}

impl Stage {
    fn is_http(self) -> bool {
        matches!(self, Self::Http(_))
    }

    fn is_websocket(self) -> bool {
        matches!(self, Self::PreAuth | Self::Session)
    }
}

struct Entry {
    /// The address, as limits count it.
    peer: IpAddr,
    stage: Stage,
    // -- WP HB9 -- Closed to make room: no longer counted, and its thread stops.
    reclaimed: bool,
    /// To cut it off from outside.
    tcp: TcpStream,
    tx: Option<Arc<WsTx>>,
    session: Option<Arc<Session>>,
    thread: Option<JoinHandle<()>>,
    done: bool,
}

#[derive(Default)]
struct Table {
    next_id: u64,
    conns: BTreeMap<u64, Entry>,
}

struct Shared {
    hub: HubHandle,
    auth: Arc<AuthStore>,
    hub_id: String,
    web: bool,
    mdns_name: Option<String>,
    port: u16,
    limits: ServerLimits,
    stopping: AtomicBool,
    table: Mutex<Table>,
    changed: Condvar,
    // -- WP TM26 -- False while [`NetworkConfig::hold_accepts`] holds them.
    accepting: AtomicBool,
    // -- WP WB7 --
    console: Option<Arc<crate::web::Console>>,
    // -- WP P2RS -- the REST mirror's keys and their sessions
    rest: crate::rest::Rest,
    // -- WP SECSRV --
    policy: TlsPolicy,
    /// The key, its certificate and the rustls config made of them, when TLS is served.
    tls: Option<TlsState>,
    /// [`NetworkConfig::host_names`].
    host_names: Vec<String>,
    /// [`Shared::own_names`]'s last answer and when. A leaf lock.
    own_names: Mutex<Option<(Instant, OwnNames)>>,
}

// -- WP SECSRV --
/// The hub's TLS identity while the server runs. Locks: leaves, `identity` before
/// `config` when both are taken.
struct TlsState {
    identity: Mutex<TlsIdentity>,
    /// What a new connection is handshaken with: made again when the certificate or the
    /// key changes. Existing connections keep theirs.
    config: Mutex<Arc<ServerConfig>>,
    /// The names the certificate was last checked against.
    names: Mutex<TlsNames>,
    /// When the names and the certificate's end are looked at next.
    next_names: Mutex<Instant>,
    next_refresh: Mutex<Instant>,
    /// What loading the identity did, at the start (a new key, a replaced one).
    at_start: Vec<TlsIdentityEvent>,
}

// -- WP SECSRV --
/// The rustls config for an identity: its certificate and key.
fn tls_config(identity: &TlsIdentity) -> Result<Arc<ServerConfig>, HubError> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        identity.private_key_der().to_vec(),
    ));
    let chain = vec![CertificateDer::from(identity.certificate_der().to_vec())];
    link_tls::server_config(chain, key)
        .map_err(|e| HubError::Io(format!("the hub's TLS key cannot be used: {e}")))
}

// -- WP SECSRV --
/// What the hub's TLS is now, for an embedder: `uconsole hub`'s start line, its pairing
/// line (SECPAIR), the advertisement (SECDISC). Public data only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsInfo {
    /// What is served over which transport.
    pub policy: TlsPolicy,
    /// The key's pin while TLS is served.
    pub pin: Option<SpkiHash>,
    /// The certificate, PEM (`uconsole hub tls cert`).
    pub certificate_pem: Option<String>,
    /// What loading the identity did at the start: a first key, a replaced one (every
    /// client will report that this hub's key changed), a certificate issued again.
    pub at_start: Vec<TlsIdentityEvent>,
}

/// The way to a running server, for a binding: the pairing window, the client list,
/// revoking. Clonable, and valid after the server has stopped.
#[derive(Clone)]
pub struct NetworkControl {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for NetworkControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkControl")
            .field("port", &self.shared.port)
            .finish_non_exhaustive()
    }
}

impl NetworkControl {
    /// The port actually bound.
    pub fn port(&self) -> u16 {
        self.shared.port
    }

    // -- WP SECSRV --
    /// The hub's TLS: its policy, its key's pin and certificate, and what the start did.
    pub fn tls(&self) -> TlsInfo {
        let shared = &self.shared;
        let (pin, certificate_pem, at_start) = match &shared.tls {
            Some(tls) => {
                let identity = guard(&tls.identity);
                (
                    Some(identity.pin()),
                    Some(identity.certificate_pem().to_owned()),
                    tls.at_start.clone(),
                )
            }
            None => (None, None, Vec::new()),
        };
        TlsInfo {
            policy: shared.policy,
            pin,
            certificate_pem,
            at_start,
        }
    }

    // -- WP SECSRV --
    /// A new TLS key (`uconsole hub tls rotate`, Admin on the hub's machine): the old
    /// one signs it unless `forget_old` (PLAN-SEC 4.2), new connections get it at once,
    /// and the event goes to the embedder too. `HubError::Io` when TLS is off or a file
    /// cannot be written.
    pub fn rotate_tls_key(&self, forget_old: bool) -> Result<TlsIdentityEvent, HubError> {
        let shared = &self.shared;
        let Some(tls) = &shared.tls else {
            return Err(HubError::Io("TLS is off on this hub".into()));
        };
        let names = guard(&tls.names).clone();
        let event = guard(&tls.identity).rotate(&names, SystemTime::now(), forget_old)?;
        shared.reload_tls();
        shared.hub.emit(crate::hub::HubEvent::Tls(event.clone()));
        Ok(event)
    }

    // -- WP TM26 --
    /// Start accepting clients, on a server started with
    /// [`NetworkConfig::hold_accepts`]. Without it, and when called again, nothing
    /// changes.
    pub fn start_accepting(&self) {
        self.shared.accepting.store(true, Ordering::SeqCst);
    }

    /// Clients and pairing: [`AuthStore::begin_pairing`], [`AuthStore::clients`].
    /// To revoke or re-role, use [`Self::revoke`] and [`Self::set_role`], which also
    /// reach the client's live sessions.
    pub fn auth(&self) -> &AuthStore {
        &self.shared.auth
    }

    /// Sockets the limits count now, plain HTTP and WebSocket, authenticated or not.
    pub fn connections(&self) -> usize {
        guard(&self.shared.table)
            .conns
            .values()
            .filter(|e| e.counts() && e.stage != Stage::Refusing)
            .count()
    }

    // -- WP HB9 --
    /// WebSockets the limits count now, authenticated or not: [`ServerLimits::max_connections`]'s.
    pub fn websockets(&self) -> usize {
        guard(&self.shared.table)
            .conns
            .values()
            .filter(|e| e.counts() && e.stage.is_websocket())
            .count()
    }

    /// Whether this client has a live session.
    pub fn is_online(&self, cid: u32) -> bool {
        !self.shared.sessions_of(cid).is_empty()
    }

    /// Forget a client: its token stops working and its connections close with 4002,
    /// at once (spec 6.2). False if there is no such client.
    pub fn revoke(&self, cid: u32) -> bool {
        // Store first: a connection listing its session after this finds the client
        // gone (`Connection::open_session`), and one before it is found below.
        let known = self.shared.auth.revoke(cid);
        if known {
            self.shared.close_client(cid);
        }
        known
    }

    /// Change a client's role: its next command is judged by the new one (spec 6.2).
    /// False if there is no such client.
    pub fn set_role(&self, cid: u32, role: Role) -> bool {
        let known = self.shared.auth.set_role(cid, role);
        if known {
            for session in self.shared.sessions_of(cid) {
                session.set_role(role);
            }
        }
        known
    }
}

impl Entry {
    // -- WP HB9 --
    fn counts(&self) -> bool {
        !self.done && !self.reclaimed
    }
}

// -- WP SECSRV --
impl Shared {
    /// The names that count as the hub itself now: [`OwnNames::current`] with the name
    /// the responder holds (after a conflict the old one is somebody else's, so only the
    /// configured one until it runs, and after) and [`NetworkConfig::host_names`].
    ///
    /// Kept for [`OWN_NAMES_FOR`]: listing the interfaces costs a system call, and a
    /// page load asks many times a second (review secb N2).
    fn own_names(&self) -> OwnNames {
        let now = Instant::now();
        if let Some((at, own)) = guard(&self.own_names).as_ref()
            && now.saturating_duration_since(*at) < OWN_NAMES_FOR
        {
            return own.clone();
        }
        let advertised = self.hub.advertised().map(|names| names.host);
        let name = advertised.as_deref().or(self.mdns_name.as_deref());
        let mut own = OwnNames::current(self.port, name);
        own.hosts.extend(self.host_names.iter().cloned());
        *guard(&self.own_names) = Some((now, own.clone()));
        own
    }

    /// The names the certificate is for: the advertised (or configured) mDNS name. No
    /// address: none is configured as static here (PLAN-SEC 4.2).
    fn tls_names(&self) -> TlsNames {
        let advertised = self.hub.advertised().map(|names| names.host);
        TlsNames::new(advertised.as_deref().or(self.mdns_name.as_deref()), &[])
    }

    /// The config a new TLS connection is handshaken with.
    fn tls_config(&self) -> Option<Arc<ServerConfig>> {
        self.tls.as_ref().map(|tls| Arc::clone(&guard(&tls.config)))
    }

    /// Make the rustls config again from the identity (a new certificate or key). A
    /// config that cannot be made leaves the last one, with a log line.
    fn reload_tls(&self) {
        let Some(tls) = &self.tls else {
            return;
        };
        let made = tls_config(&guard(&tls.identity));
        match made {
            Ok(config) => *guard(&tls.config) = config,
            Err(e) => log::error!("hub: {e}; the previous certificate is still served"),
        }
    }

    /// On the acceptor's tick: issue the certificate again when the advertised name
    /// changed or it nears its end (and end a rotation's announcement), then serve it.
    /// Cheap when nothing is due.
    fn tls_tick(&self) {
        let Some(tls) = &self.tls else {
            return;
        };
        let now = Instant::now();
        {
            let mut next = guard(&tls.next_names);
            if now < *next {
                return;
            }
            *next = after(now, TLS_NAMES_EVERY);
        }
        let names = self.tls_names();
        let renamed = *guard(&tls.names) != names;
        let due = {
            let mut next = guard(&tls.next_refresh);
            let due = now >= *next;
            if due {
                *next = after(now, TLS_REFRESH_EVERY);
            }
            due
        };
        if !renamed && !due {
            return;
        }
        guard(&tls.names).clone_from(&names);
        let refreshed = guard(&tls.identity).refresh(&names, SystemTime::now());
        match refreshed {
            Ok(Some(event)) => {
                self.reload_tls();
                self.hub.emit(crate::hub::HubEvent::Tls(event));
            }
            Ok(None) => {}
            Err(e) => log::warn!("hub: the TLS certificate could not be checked: {e}"),
        }
    }
}

impl Shared {
    fn sessions_of(&self, cid: u32) -> Vec<Arc<Session>> {
        let mut sessions: Vec<Arc<Session>> = guard(&self.table)
            .conns
            .values()
            .filter(|e| !e.done)
            .filter_map(|e| e.session.as_ref())
            .filter(|s| s.client_id() == Some(cid))
            .map(Arc::clone)
            .collect();
        // -- WP P2RS -- an API key's REST session: revoked and re-roled with the rest
        sessions.extend(self.rest.session_of(cid));
        sessions
    }

    fn close_client(&self, cid: u32) {
        for session in self.sessions_of(cid) {
            session.outbox().close_with(CLOSE_REVOKED, "revoked");
            self.hub.close_session(session.id());
        }
    }

    fn set_tx(&self, id: u64, tx: &Arc<WsTx>) {
        if let Some(e) = guard(&self.table).conns.get_mut(&id) {
            e.tx = Some(Arc::clone(tx));
        }
    }

    // -- WP HB9 --
    /// Where a plain HTTP socket is now. False if it was closed to make room, when its
    /// thread is to stop.
    fn set_http(&self, id: u64, http: Http) -> bool {
        let mut table = guard(&self.table);
        match table.conns.get_mut(&id) {
            Some(e) if !e.reclaimed => {
                e.stage = Stage::Http(http);
                true
            }
            _ => false,
        }
    }

    /// Move a plain HTTP socket into the WebSocket budget, at its upgrade. False when
    /// there is no room there, or it was closed to make room.
    fn promote(&self, id: u64) -> bool {
        let mut table = guard(&self.table);
        let Some(peer) = table
            .conns
            .get(&id)
            .filter(|e| !e.reclaimed)
            .map(|e| e.peer)
        else {
            return false;
        };
        let live = || table.conns.values().filter(|e| e.counts());
        let websockets = live().filter(|e| e.stage.is_websocket()).count();
        let from_peer = live()
            .filter(|e| e.stage == Stage::PreAuth && e.peer == peer)
            .count();
        if websockets >= self.limits.max_connections
            || from_peer >= self.limits.max_preauth_per_peer
        {
            return false;
        }
        if let Some(e) = table.conns.get_mut(&id) {
            e.stage = Stage::PreAuth;
        }
        true
    }

    fn set_session(&self, id: u64, session: &Arc<Session>) {
        if let Some(e) = guard(&self.table).conns.get_mut(&id) {
            e.stage = Stage::Session;
            e.session = Some(Arc::clone(session));
        }
    }

    fn mark_done(&self, id: u64) {
        if let Some(e) = guard(&self.table).conns.get_mut(&id) {
            e.done = true;
            e.session = None;
            e.tx = None;
        }
        self.changed.notify_all();
    }

    /// Join the threads that have finished. Their joins do not wait: all that is left of
    /// them is to return.
    fn reap(&self) {
        let finished: Vec<JoinHandle<()>> = {
            let mut table = guard(&self.table);
            let ids: Vec<u64> = table
                .conns
                .iter()
                .filter(|(_, e)| e.done && e.thread.is_some())
                .map(|(id, _)| *id)
                .collect();
            ids.iter()
                .filter_map(|id| table.conns.remove(id))
                .filter_map(|e| e.thread)
                .collect()
        };
        for thread in finished {
            let _ = thread.join();
        }
    }

    /// A new socket: count it, and give it a thread. Never blocks: a flood costs this
    /// thread nothing but the accepts.
    fn admit(self: &Arc<Self>, tcp: TcpStream, peer: SocketAddr) {
        let opened = Instant::now();
        if !is_local_peer(peer.ip()) {
            log::debug!("hub: refused a peer off the local network");
            return;
        }
        // Some systems pass the listener's mode on to what it accepts.
        if tcp.set_nonblocking(false).is_err() || tcp.set_nodelay(true).is_err() {
            return;
        }
        let Ok(cut_off) = tcp.try_clone() else {
            return;
        };
        let key = peer_key(peer.ip());
        let (id, stage) = {
            let mut table = guard(&self.table);
            let refusing = table
                .conns
                .values()
                .filter(|e| e.counts() && e.stage == Stage::Refusing)
                .count();
            // -- WP HB9 -- Every socket starts as plain HTTP; the upgrade moves it on.
            let stage = if make_http_room(&mut table, key, &self.limits) {
                Stage::Http(Http::Idle(opened))
            } else if refusing < self.limits.max_refusing {
                Stage::Refusing
            } else {
                return;
            };
            let id = table.next_id;
            table.next_id = id.wrapping_add(1);
            table.conns.insert(
                id,
                Entry {
                    peer: key,
                    stage,
                    reclaimed: false,
                    tcp: cut_off,
                    tx: None,
                    session: None,
                    thread: None,
                    done: false,
                },
            );
            (id, stage)
        };
        let shared = Arc::clone(self);
        let name = match stage {
            Stage::Refusing => format!("hub-refuse-{id}"),
            _ => format!("hub-conn-rx-{id}"),
        };
        let spawned = thread::Builder::new().name(name).spawn(move || {
            // Marks it done however this ends, a panic included.
            let _done = DoneGuard {
                shared: &shared,
                id,
            };
            match stage {
                Stage::Refusing => refuse(&shared, tcp, opened),
                _ => serve(&shared, id, tcp, peer.ip(), opened),
            }
        });
        let mut table = guard(&self.table);
        match spawned {
            Ok(thread) => {
                if let Some(e) = table.conns.get_mut(&id) {
                    e.thread = Some(thread);
                }
            }
            Err(e) => {
                log::warn!("hub: no thread for a connection: {e}");
                table.conns.remove(&id);
            }
        }
    }
}

// -- WP HB9 --
/// Room in the plain HTTP budget for one more socket from `peer`, closing an idle one if
/// that is what it takes: the address's own when it is at its share, else one of the
/// fullest address's. False if every socket that would have to go is being answered.
fn make_http_room(table: &mut Table, peer: IpAddr, limits: &ServerLimits) -> bool {
    let mut per_peer: BTreeMap<IpAddr, usize> = BTreeMap::new();
    for e in table
        .conns
        .values()
        .filter(|e| e.counts() && e.stage.is_http())
    {
        let n = per_peer.entry(e.peer).or_default();
        *n = n.saturating_add(1);
    }
    let own = per_peer.get(&peer).copied().unwrap_or(0);
    let total: usize = per_peer.values().sum();
    let from: Option<IpAddr> = if own >= limits.max_http_per_peer {
        Some(peer)
    } else if total >= limits.max_http {
        None
    } else {
        return true;
    };
    // The idlest, from the address at its share, or from the fullest address that has one.
    let victim = table
        .conns
        .iter()
        .filter(|(_, e)| e.counts() && from.is_none_or(|p| e.peer == p))
        .filter_map(|(id, e)| match e.stage {
            Stage::Http(http) => http.reclaim_order().map(|order| (*id, e.peer, order)),
            _ => None,
        })
        .min_by_key(|(_, p, order)| {
            let fullness = per_peer.get(p).copied().unwrap_or(0);
            (std::cmp::Reverse(fullness), *order)
        })
        .map(|(id, ..)| id);
    let Some(entry) = victim.and_then(|id| table.conns.get_mut(&id)) else {
        return false;
    };
    entry.reclaimed = true;
    // Ends its read at once, on its own thread, which then marks it done.
    let _ = entry.tcp.shutdown(Shutdown::Both);
    true
}

struct DoneGuard<'a> {
    shared: &'a Shared,
    id: u64,
}

impl Drop for DoneGuard<'_> {
    fn drop(&mut self) {
        self.shared.mark_done(self.id);
    }
}

fn accept_loop(shared: &Arc<Shared>, listener: &TcpListener) {
    while !shared.stopping.load(Ordering::SeqCst) {
        // -- WP TM26 --
        if !shared.accepting.load(Ordering::SeqCst) {
            thread::sleep(ACCEPT_TICK);
            continue;
        }
        match listener.accept() {
            Ok((tcp, peer)) => {
                shared.reap();
                // -- WP P2RS --
                shared.rest.sweep(&shared.hub, &shared.limits);
                // -- WP SECSRV --
                shared.tls_tick();
                shared.admit(tcp, peer);
            }
            Err(e) => {
                // Out of descriptors looks the same to this loop: wait and go on.
                if e.kind() != io::ErrorKind::WouldBlock {
                    log::debug!("hub: accept: {e}");
                }
                shared.reap();
                // -- WP P2RS -- idle keys' sessions close; their outboxes are emptied
                shared.rest.sweep(&shared.hub, &shared.limits);
                // -- WP SECSRV --
                shared.tls_tick();
                thread::sleep(ACCEPT_TICK);
            }
        }
    }
}

// -- WP P2RS --
/// The byte stream one plain HTTP request is read from and answered on (PLAN-SEC section
/// 6, P2-C): a `TcpStream`, or -- WP SECSRV -- a [`TlsPipe`] over one, so the head and
/// body reads, the REST mirror and the answers are the same over both.
pub(crate) trait HubStream: Read + Write {
    /// As [`TcpStream::set_read_timeout`]: `None` waits for ever. -- WP SECSRV -- A read
    /// ends within it however slowly a TLS record trickles in.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
    /// As [`TcpStream::set_write_timeout`].
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    /// End the sending half, behind what was written (TLS: its `close_notify` first).
    fn shutdown_write(&mut self) -> io::Result<()>;
    /// The peer's address, as the limits and the failure ledger count it.
    fn peer_ip(&self) -> io::Result<IpAddr>;
}

// -- WP P2RS --
impl HubStream for TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }

    fn shutdown_write(&mut self) -> io::Result<()> {
        self.shutdown(Shutdown::Write)
    }

    fn peer_ip(&self) -> io::Result<IpAddr> {
        self.peer_addr().map(|a| a.ip())
    }
}

// -- WP SECSRV --
impl HubStream for TlsPipe {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        // The deadline bounds a whole read, which may take several socket reads for
        // one record; the socket's own timeout alone would restart with each.
        self.set_deadline(timeout.map(|t| after(Instant::now(), t)));
        TlsPipe::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.tcp().set_write_timeout(timeout)
    }

    fn shutdown_write(&mut self) -> io::Result<()> {
        let notified = self.send_close_notify();
        self.tcp().shutdown(Shutdown::Write).and(notified)
    }

    fn peer_ip(&self) -> io::Result<IpAddr> {
        self.peer_addr().map(|a| a.ip())
    }
}

// -- WP SECSRV --
/// A stream an upgrade turns into a WebSocket's two halves.
trait IntoWire: HubStream + Sized {
    /// The socket under it, for its timeouts and to cut it off.
    fn raw_tcp(&self) -> &TcpStream;
    /// The reader's and the writer's halves, buffered (a frame goes out whole).
    fn into_wires(self) -> io::Result<(Wire, Wire)>;
}

impl IntoWire for TcpStream {
    fn raw_tcp(&self) -> &TcpStream {
        self
    }

    fn into_wires(self) -> io::Result<(Wire, Wire)> {
        let (reader, writer) = server_pipes(self)?;
        Ok((Wire::Plain(reader), Wire::Plain(writer)))
    }
}

impl IntoWire for TlsPipe {
    fn raw_tcp(&self) -> &TcpStream {
        self.tcp()
    }

    fn into_wires(mut self) -> io::Result<(Wire, Wire)> {
        let mut writer = self.try_clone()?;
        // The socket's read timeout (the reader's tick) is what bounds a read now.
        self.set_deadline(None);
        self.set_buffered(true);
        writer.set_buffered(true);
        Ok((Wire::Tls(self), Wire::Tls(writer)))
    }
}

// -- WP SECSRV --
/// One half of a WebSocket's byte stream: plain, or TLS on the same port.
enum Wire {
    Plain(Pipe),
    Tls(TlsPipe),
}

impl Wire {
    /// A write failed part way: see [`Pipe::is_broken`].
    fn is_broken(&self) -> bool {
        match self {
            Self::Plain(p) => p.is_broken(),
            Self::Tls(t) => t.is_broken(),
        }
    }

    /// The socket under it.
    fn raw_tcp(&mut self) -> &TcpStream {
        match self {
            Self::Plain(p) => p.tcp_mut(),
            Self::Tls(t) => t.tcp(),
        }
    }

    /// TLS: `close_notify` behind what was sent. Plain: nothing.
    fn close_notify(&mut self) {
        if let Self::Tls(t) = self {
            let _ = t.send_close_notify();
        }
    }
}

impl Read for Wire {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(p) => p.read(buf),
            Self::Tls(t) => t.read(buf),
        }
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(p) => p.write(buf),
            Self::Tls(t) => t.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(p) => p.flush(),
            Self::Tls(t) => t.flush(),
        }
    }
}

/// Read up to the end of the next HTTP head: `(head, what came behind it)`, starting
/// with `buf`, what came behind the last one. Bounded in bytes and in time: the request
/// must start by `start_by`, and its head end within `request_timeout` of its first byte
/// and by `cap`, however slowly it trickles (WP HB9). `started` hears of the first byte;
/// false from it ends the read.
fn read_head<S: HubStream + ?Sized>(
    tcp: &mut S,
    mut buf: Vec<u8>,
    start_by: Instant,
    cap: Option<Instant>,
    request_timeout: Duration,
    mut started: impl FnMut() -> bool,
) -> Result<(Vec<u8>, Vec<u8>), HeadError> {
    let head_by = |first_byte: Instant| {
        let end = after(first_byte, request_timeout);
        cap.map_or(end, |cap| end.min(cap))
    };
    let mut deadline = start_by;
    if !buf.is_empty() {
        if !started() {
            return Err(HeadError::Gone);
        }
        deadline = head_by(Instant::now());
    }
    let mut chunk = [0u8; 1024];
    let mut scanned: usize = 0;
    loop {
        // Back over the seam: the end of the head can straddle two reads.
        let from = scanned.saturating_sub(3);
        let end = buf
            .get(from..)
            .and_then(|tail| tail.windows(4).position(|w| w == b"\r\n\r\n"))
            .map(|at| from.saturating_add(at).saturating_add(4));
        scanned = buf.len();
        match end {
            Some(end) if end <= MAX_HEAD => {
                let tail = buf.split_off(end);
                return Ok((buf, tail));
            }
            Some(_) => return Err(HeadError::TooLarge),
            None if buf.len() > MAX_HEAD => return Err(HeadError::TooLarge),
            None => {}
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(HeadError::Gone);
        }
        tcp.set_read_timeout(Some(left))
            .map_err(|_| HeadError::Gone)?;
        let n = match tcp.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(HeadError::Gone),
            Ok(n) => n,
        };
        if buf.is_empty() {
            if !started() {
                return Err(HeadError::Gone);
            }
            deadline = head_by(Instant::now());
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
    }
}

enum HeadError {
    /// Timed out or closed: nobody to answer.
    Gone,
    TooLarge,
}

/// Write all of it by `cap`, however slowly the peer takes it, each write making
/// progress within `stall`.
fn write_by<S: HubStream + ?Sized>(
    tcp: &mut S,
    mut bytes: &[u8],
    stall: Duration,
    cap: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let left = cap.saturating_duration_since(Instant::now()).min(stall);
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        tcp.set_write_timeout(Some(left))?;
        match tcp.write(bytes)? {
            0 => return Err(io::ErrorKind::WriteZero.into()),
            n => bytes = bytes.get(n..).unwrap_or_default(),
        }
    }
    Ok(())
}

/// Read and discard until the peer closes or the deadline, so that what it has sent
/// does not turn our close into a reset, which could cost it what it has not read yet.
fn drain<S: HubStream + ?Sized>(tcp: &mut S, deadline: Instant) {
    let mut sink = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || tcp.set_read_timeout(Some(left)).is_err() {
            return;
        }
        if matches!(tcp.read(&mut sink), Ok(0) | Err(_)) {
            return;
        }
    }
}

/// One whole HTTP answer, written by `until` with each write progressing within
/// [`ServerLimits::write_stall`]. With `keep` the connection stays open for the next
/// request, and the result says whether it may; without, it closes behind the answer.
fn respond<S: HubStream + ?Sized>(
    shared: &Shared,
    tcp: &mut S,
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    keep: bool,
    until: Instant,
) -> bool {
    // -- WP HB9 -- kept alive when the request allows it
    let connection = if keep {
        let idle = shared.limits.keepalive_idle.as_secs().max(1);
        format!("Keep-Alive: timeout={idle}")
    } else {
        "Connection: close".to_owned()
    };
    let mut head = format!(
        "HTTP/1.1 {status}\r\n{connection}\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let stall = shared.limits.write_stall;
    if write_by(tcp, head.as_bytes(), stall, until).is_err()
        || write_by(tcp, body, stall, until).is_err()
    {
        return false;
    }
    if keep {
        return true;
    }
    let _ = tcp.shutdown_write();
    drain(tcp, after(Instant::now(), shared.limits.close_grace));
    false
}

const TEXT: (&str, &str) = ("Content-Type", "text/plain; charset=utf-8");
const BUSY: &[u8] = b"The hub has no room for another session.";

/// The answer for a socket past the limits. On its own short-lived thread, so that a
/// legitimate client hears 503 (`LinkError::HubBusy`, spec 4) and not a reset: closing
/// with its request unread would be one.
fn refuse(shared: &Shared, tcp: TcpStream, opened: Instant) {
    let deadline = after(opened, shared.limits.close_grace);
    // -- WP SECSRV -- a TLS client hears its 503 over TLS, within the same grace
    match sniff(shared, &tcp, deadline) {
        Sniffed::Plain => refuse_on(shared, tcp, deadline),
        Sniffed::Tls(config) => {
            if let Ok(mut pipe) = link_tls::accept(tcp, config, deadline) {
                pipe.set_deadline(None);
                refuse_on(shared, pipe, deadline);
            }
        }
        Sniffed::TlsOff => refuse_tls_off(shared, tcp),
        Sniffed::Gone => {}
    }
}

fn refuse_on<S: HubStream>(shared: &Shared, mut tcp: S, deadline: Instant) {
    let grace = shared.limits.close_grace;
    let _ = read_head(
        &mut tcp,
        Vec::new(),
        deadline,
        Some(deadline),
        grace,
        || true,
    );
    let headers = [TEXT, ("Retry-After", "5")];
    let status = "503 Service Unavailable";
    respond(shared, &mut tcp, status, &headers, BUSY, false, deadline);
}

// -- WP SECSRV --
/// What a connection's first byte says.
enum Sniffed {
    /// Plain HTTP, or TLS is not served: nothing was read.
    Plain,
    /// A TLS handshake record, to be handshaken with this config.
    Tls(Arc<ServerConfig>),
    /// Closed, cut off or silent until `by`.
    Gone,
    /// A TLS handshake on a hub that serves none ([`TlsMode::Off`]).
    TlsOff,
}

// -- WP SECSRV --
/// Wait for the first byte, by `by`, without taking it (with TLS off too: a
/// ClientHello there is answered at once).
fn sniff(shared: &Shared, tcp: &TcpStream, by: Instant) -> Sniffed {
    let config = shared
        .policy
        .serves_tls()
        .then(|| shared.tls_config())
        .flatten();
    let left = by.saturating_duration_since(Instant::now());
    if left.is_zero() || tcp.set_read_timeout(Some(left)).is_err() {
        return Sniffed::Gone;
    }
    match (link_tls::sniff_tls(tcp), config) {
        (Ok(true), Some(config)) => Sniffed::Tls(config),
        (Ok(true), None) => Sniffed::TlsOff,
        (Ok(false), _) => Sniffed::Plain,
        (Err(_), _) => Sniffed::Gone,
    }
}

// -- WP SECSRV --
/// [`Sniffed::TlsOff`]: a plain `400` at once, not after waiting for a head that never
/// comes, so a client trying TLS first fails now rather than at its own timeout
/// (SECCORE). Its handshake then fails on the answer.
fn refuse_tls_off(shared: &Shared, mut tcp: TcpStream) {
    log::debug!("hub: a TLS handshake on a hub with TLS off");
    let until = after(Instant::now(), shared.limits.write_stall);
    let why = b"Not HTTP: this hub does not speak TLS.";
    respond(
        shared,
        &mut tcp,
        "400 Bad Request",
        &[TEXT],
        why,
        false,
        until,
    );
}

// -- WP HB9 --
/// What a socket does after a request's answer.
enum Next {
    /// Waits for the next request: kept alive.
    Keep,
    Close,
    /// Becomes a WebSocket, already counted in that budget. The `Sec-WebSocket-Accept`.
    Upgrade(String),
}

fn kept(alive: bool) -> Next {
    if alive { Next::Keep } else { Next::Close }
}

/// Whether the connection may carry another request after this one's answer: HTTP/1.1
/// asking for it by `Connection: keep-alive`, as browsers do, and no body this server
/// would have to skip. Without the word it closes as before, for a client that reads
/// its answer to the end of the stream.
fn keep_alive(request: &httparse::Request<'_, '_>) -> bool {
    let Ok(connection) = single_header(request, "Connection") else {
        return false;
    };
    let no_body = match single_header(request, "Content-Length") {
        Ok(None) => true,
        Ok(Some(length)) => length.trim() == "0",
        Err(()) => false,
    };
    let chunked = request
        .headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("Transfer-Encoding"));
    request.version == Some(1)
        && request.method == Some("GET")
        && has_token(connection, "keep-alive")
        && !has_token(connection, "close")
        && no_body
        && !chunked
}

/// The whole life of a socket, on its reader thread: plain HTTP requests, kept alive
/// while the peer and the answer allow, until one upgrades or the socket closes.
///
/// -- WP SECSRV -- First, when TLS is served, its first byte: a TLS handshake, which
/// must end within the first request's [`ServerLimits::head_timeout`], then the same
/// requests over it. The socket counts as a request being read from that byte on.
fn serve(shared: &Arc<Shared>, id: u64, tcp: TcpStream, peer: IpAddr, opened: Instant) {
    let first_by = after(opened, shared.limits.head_timeout);
    match sniff(shared, &tcp, first_by) {
        Sniffed::Plain => serve_on(shared, id, tcp, false, peer, first_by),
        Sniffed::Tls(config) => {
            if !shared.set_http(id, Http::Head(Instant::now())) {
                return;
            }
            match link_tls::accept(tcp, config, first_by) {
                Ok(mut pipe) => {
                    // read_head sets its own deadlines from here
                    pipe.set_deadline(None);
                    serve_on(shared, id, pipe, true, peer, first_by);
                }
                Err(e) => log::debug!("hub: a TLS handshake failed: {e}"),
            }
        }
        Sniffed::TlsOff => refuse_tls_off(shared, tcp),
        Sniffed::Gone => {}
    }
}

/// [`serve`] on a plain or TLS stream (`encrypted`), from its first request on.
fn serve_on<S: IntoWire>(
    shared: &Arc<Shared>,
    id: u64,
    mut tcp: S,
    encrypted: bool,
    peer: IpAddr,
    first_by: Instant,
) {
    let limits = &shared.limits;
    let mut carry = Vec::new();
    let mut first = true;
    loop {
        let (start_by, cap) = if first {
            (first_by, Some(first_by))
        } else {
            // Idle again, for the next request: the first to go when room is needed.
            if !shared.set_http(id, Http::Idle(Instant::now())) {
                return;
            }
            (after(Instant::now(), limits.keepalive_idle), None)
        };
        first = false;
        let started = || shared.set_http(id, Http::Head(Instant::now()));
        let read = read_head(
            &mut tcp,
            carry,
            start_by,
            cap,
            limits.request_timeout,
            started,
        );
        // Being answered from here on: bounded by the response's own times.
        if !shared.set_http(id, Http::Busy) {
            return;
        }
        let until = after(Instant::now(), limits.response_timeout);
        let (head, tail) = match read {
            Ok(read) => read,
            Err(HeadError::Gone) => return,
            Err(HeadError::TooLarge) => {
                let status = "431 Request Header Fields Too Large";
                respond(
                    shared,
                    &mut tcp,
                    status,
                    &[TEXT],
                    b"Too large.",
                    false,
                    until,
                );
                return;
            }
        };
        // -- WP P2RS -- `tail`: a REST body starts there, and what it leaves is carried
        let mut tail = tail;
        match answer(shared, id, &mut tcp, encrypted, &head, &mut tail, until) {
            Next::Keep => carry = tail,
            Next::Close => return,
            Next::Upgrade(accept) => {
                upgrade(shared, id, tcp, encrypted, tail, &accept, peer);
                return;
            }
        }
    }
}

/// The answer to one plain HTTP request.
///
/// -- WP P2RS -- Over any [`HubStream`]; `tail` is what came behind the head, where a
/// REST request's body starts. An upgrade is answered by the caller, on the `TcpStream`.
///
/// -- WP SECSRV -- `encrypted`: it came over TLS. Over plaintext the `Host` is checked
/// first (DNS rebinding), and a [`TlsMode::Require`] hub answers `426` to all but a
/// WebSocket upgrade, which [`upgrade`] closes with 4003.
fn answer<S: HubStream + ?Sized>(
    shared: &Shared,
    id: u64,
    tcp: &mut S,
    encrypted: bool,
    head: &[u8],
    tail: &mut Vec<u8>,
    until: Instant,
) -> Next {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    if !matches!(request.parse(head), Ok(httparse::Status::Complete(_))) {
        let status = "400 Bad Request";
        respond(shared, tcp, status, &[TEXT], b"Not HTTP.", false, until);
        return Next::Close;
    }
    let path = request
        .path
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    // -- WP SECSRV -- every route, before anything is said about the hub (an upgrade's
    // `Origin` is judged first, in `check_upgrade`, so a page learns as little as it can)
    let misdirected = !encrypted && !host_allowed(&request, || shared.own_names());
    if misdirected && path != WS_PATH {
        return misdirected_page(shared, tcp, until);
    }
    if path == WS_PATH {
        match check_upgrade(shared, &request, encrypted) {
            // -- WP SECSRV --
            Ok(_) if misdirected => return misdirected_page(shared, tcp, until),
            // -- WP HB9 -- Into the WebSocket budget here, and only here.
            Ok(accept) if shared.promote(id) => return Next::Upgrade(accept),
            Ok(_) => {
                let headers = [TEXT, ("Retry-After", "5")];
                let status = "503 Service Unavailable";
                respond(shared, tcp, status, &headers, BUSY, false, until);
            }
            Err((status, why)) => {
                respond(shared, tcp, status, &[TEXT], why.as_bytes(), false, until);
            }
        }
        return Next::Close;
    }
    // -- WP SECSRV -- TLS only: the rest of what a plaintext request could reach
    if !encrypted && !shared.policy.serves_plaintext() {
        return tls_required_page(shared, tcp, &request, until);
    }
    // -- WP P2RS -- `/api/v1/`: the REST mirror (`rest.rs`), `404` while it is off.
    if crate::rest::is_rest_path(path) {
        let reply = shared.rest.answer(
            &crate::rest::RestCtx {
                hub: &shared.hub,
                auth: &shared.auth,
                hub_id: &shared.hub_id,
                limits: &shared.limits,
                // -- WP SECSRV --
                encrypted,
                any_key_in_plaintext: shared.policy.plaintext_api_keys(),
            },
            tcp,
            &request,
            tail,
        );
        let headers: Vec<(&str, &str)> = reply
            .headers
            .iter()
            .map(|(n, v)| (*n, v.as_str()))
            .collect();
        let keep = reply.keep && keep_alive(&request);
        return kept(respond(
            shared,
            tcp,
            reply.status,
            &headers,
            &reply.body,
            keep,
            until,
        ));
    }
    if request.method != Some("GET") {
        let status = "405 Method Not Allowed";
        let headers = [TEXT, ("Allow", "GET")];
        respond(shared, tcp, status, &headers, b"GET only.", false, until);
        return Next::Close;
    }
    let keep = keep_alive(&request);
    if path == INFO_PATH {
        let body = shared.info();
        let headers = [
            ("Content-Type", "application/json"),
            ("Access-Control-Allow-Origin", "*"),
        ];
        return kept(respond(
            shared,
            tcp,
            "200 OK",
            &headers,
            body.as_bytes(),
            keep,
            until,
        ));
    }
    // -- WP WB7 -- The console, beside the page at `/`, with web on only.
    if shared.web && crate::web::is_console_path(path) {
        return kept(serve_console(
            shared, tcp, encrypted, &request, path, keep, until,
        ));
    }
    // `/dspi/` is the protocol's (spec 4.2), and the web UI is only served when on.
    let asset = (shared.web && !path.starts_with("/dspi/"))
        .then(|| crate::web::asset(path))
        .flatten();
    // -- WP WB7 -- `/hub` to `/hub/`, where upstream's relative URLs resolve.
    if let Some(to) = shared.web.then(|| crate::web::redirect(path)).flatten() {
        let headers = [TEXT, ("Location", to)];
        let status = "301 Moved Permanently";
        return kept(respond(
            shared, tcp, status, &headers, b"Moved.", keep, until,
        ));
    }
    let alive = match asset {
        Some(asset) => {
            // -- WP HB5 -- A hub's update changes these under the same paths.
            // -- WP WB7 -- ...but for upstream's hashed asset names; and a policy per page.
            // -- WP SECSRV -- `wss://` from an https page
            let hub = own_authority(shared, &request, encrypted);
            let csp = crate::web::page_csp_for(path, hub, encrypted);
            let headers = [
                ("Content-Type", asset.content_type),
                ("X-Content-Type-Options", "nosniff"),
                ("Cache-Control", crate::web::cache_control(path)),
                ("Content-Security-Policy", csp.as_str()),
                ("Referrer-Policy", "no-referrer"),
            ];
            respond(shared, tcp, "200 OK", &headers, asset.body, keep, until)
        }
        None => respond(
            shared,
            tcp,
            "404 Not Found",
            &[TEXT],
            b"Not found.",
            keep,
            until,
        ),
    };
    kept(alive)
}

// -- WP WB7 --
/// One request under the console's path. Whether the connection stays open.
fn serve_console<S: HubStream + ?Sized>(
    shared: &Shared,
    tcp: &mut S,
    encrypted: bool,
    request: &httparse::Request<'_, '_>,
    path: &str,
    keep: bool,
    until: Instant,
) -> bool {
    use crate::web::ConsoleAnswer;
    let Some(console) = shared.console.as_deref() else {
        let why = b"No web console on this hub.";
        return respond(shared, tcp, "404 Not Found", &[TEXT], why, keep, until);
    };
    // -- WP SECSRV -- `wss://` from an https page
    let hub = own_authority(shared, request, encrypted);
    let csp = crate::web::console_csp_for(hub, encrypted);
    let if_none_match = single_header(request, "If-None-Match").ok().flatten();
    // The same as the page at `/`, plus the policy and a validator: the file names
    // carry no hash, so a browser asks again each time and gets 304 when unchanged.
    let common = [
        ("X-Content-Type-Options", "nosniff"),
        ("Cache-Control", "no-cache"),
        ("Content-Security-Policy", csp.as_str()),
        ("Referrer-Policy", "no-referrer"),
    ];
    match console.answer_for(path, hub, encrypted, if_none_match) {
        ConsoleAnswer::Redirect => {
            let headers = [TEXT, ("Location", crate::web::CONSOLE_PATH)];
            let status = "301 Moved Permanently";
            respond(
                shared,
                tcp,
                status,
                &headers,
                b"See /console/.",
                keep,
                until,
            )
        }
        ConsoleAnswer::NotModified { etag } => {
            let mut headers = common.to_vec();
            headers.push(("ETag", etag));
            respond(shared, tcp, "304 Not Modified", &headers, b"", keep, until)
        }
        ConsoleAnswer::File {
            content_type,
            body,
            etag,
        } => {
            let mut headers = common.to_vec();
            headers.push(("Content-Type", content_type));
            headers.extend(etag.map(|e| ("ETag", e)));
            respond(shared, tcp, "200 OK", &headers, &body, keep, until)
        }
        ConsoleAnswer::NotFound => respond(
            shared,
            tcp,
            "404 Not Found",
            &[TEXT],
            b"Not found.",
            keep,
            until,
        ),
    }
}

/// The request's `Host`, when it names this hub as the `Origin` rule does: the only
/// address a page from here may open a socket to. `None` for anything else.
///
/// -- WP SECSRV -- Over TLS, any well-formed `Host` on the hub's port: a name the hub's
/// certificate was accepted for in the browser (a rebound one fails that check).
fn own_authority<'a>(
    shared: &Shared,
    request: &httparse::Request<'_, 'a>,
    encrypted: bool,
) -> Option<&'a str> {
    let host = single_header(request, "Host").ok().flatten()?.trim();
    if encrypted {
        let on_port = parse_origin(&format!("https://{host}"), "https://", 443)
            .is_some_and(|(_, port)| port == shared.port);
        return on_port.then_some(host);
    }
    origin_allowed(Some(&format!("http://{host}")), &shared.own_names()).then_some(host)
}

// -- WP SECSRV --
/// The answer to a plaintext request for a name that is not the hub's (DNS rebinding).
fn misdirected_page<S: HubStream + ?Sized>(shared: &Shared, tcp: &mut S, until: Instant) -> Next {
    log::debug!("hub: refused a request for a name that is not the hub's");
    let status = "421 Misdirected Request";
    let why = b"This hub does not answer to that name: use its address or its .local name.";
    respond(shared, tcp, status, &[TEXT], why, false, until);
    Next::Close
}

// -- WP SECSRV --
/// The `426` page of a [`TlsMode::Require`] hub: what to open instead, and the key's
/// fingerprint to compare with the hub's own display.
fn tls_required_page<S: HubStream + ?Sized>(
    shared: &Shared,
    tcp: &mut S,
    request: &httparse::Request<'_, '_>,
    until: Instant,
) -> Next {
    let fingerprint = shared
        .tls
        .as_ref()
        .map(|tls| guard(&tls.identity).fingerprint())
        .unwrap_or_default();
    let at = own_authority(shared, request, false)
        .map(|host| format!("https://{host}/"))
        .unwrap_or_else(|| "https:// and this hub's address".to_owned());
    let body = format!(
        "This DSPi hub accepts encrypted connections only (TLS).\n\
         Open {at} instead, or update the client.\n\
         Check that the hub's key fingerprint is {fingerprint}\n"
    );
    let headers = [TEXT, ("Upgrade", "TLS/1.3")];
    let status = "426 Upgrade Required";
    respond(shared, tcp, status, &headers, body.as_bytes(), false, until);
    Next::Close
}

/// One header's value, all of it: `Err` when it is there more than once or is not text,
/// since a caller must not read either as "absent".
fn single_header<'a>(
    request: &httparse::Request<'_, 'a>,
    name: &str,
) -> Result<Option<&'a str>, ()> {
    let mut found = None;
    for h in request
        .headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case(name))
    {
        if found.is_some() {
            return Err(());
        }
        found = Some(std::str::from_utf8(h.value).map_err(|_| ())?);
    }
    Ok(found)
}

fn has_token(value: Option<&str>, token: &str) -> bool {
    value.is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
}

/// Everything a WebSocket upgrade must have. `Ok` is the `Sec-WebSocket-Accept` value.
fn check_upgrade(
    shared: &Shared,
    request: &httparse::Request<'_, '_>,
    encrypted: bool,
) -> Result<String, (&'static str, &'static str)> {
    const FORBIDDEN: &str = "403 Forbidden";
    const BAD: &str = "400 Bad Request";
    // First, so a web page learns as little as it can from the answer.
    let origin = single_header(request, "Origin").map_err(|()| (FORBIDDEN, "Origin refused."))?;
    let allowed = match origin {
        None => true,
        // No page to be the origin of, so nothing a browser sends can be right.
        Some(_) if !shared.web => false,
        // -- WP HB5 -- The name the responder holds now: after a conflict the old one is
        // somebody else's. The configured one until it runs, and after.
        // -- WP SECSRV -- `https://` too where TLS is served; over TLS also the page
        // of the very name it was asked at (`own_authority`'s rule)
        Some(o) => {
            let https = shared.policy.serves_tls();
            origin_allowed_with(origin, &shared.own_names(), https)
                || (encrypted
                    && own_authority(shared, request, true)
                        .is_some_and(|host| o.eq_ignore_ascii_case(&format!("https://{host}"))))
        }
    };
    if !allowed {
        log::debug!("hub: refused a WebSocket from a foreign origin");
        return Err((FORBIDDEN, "Origin refused."));
    }
    let header = |name| single_header(request, name).map_err(|()| (BAD, "Repeated header."));
    let proper = request.method == Some("GET")
        && request.version == Some(1)
        && has_token(header("Connection")?, "upgrade")
        && has_token(header("Upgrade")?, "websocket")
        && header("Sec-WebSocket-Version")? == Some("13");
    if !proper {
        return Err((BAD, "A WebSocket upgrade is required here."));
    }
    // Spec 4 lets a hub refuse this, and the bridge does. Everything here sends it.
    if !has_token(header("Sec-WebSocket-Protocol")?, SUBPROTOCOL) {
        return Err((BAD, "Sub-protocol dspi-link-1 is required."));
    }
    let key = header("Sec-WebSocket-Key")?.ok_or((BAD, "No Sec-WebSocket-Key."))?;
    Ok(tungstenite::handshake::derive_accept_key(key.as_bytes()))
}

/// -- WP SECSRV -- Over a plain or TLS stream (`encrypted`). A plaintext one on a
/// [`TlsMode::Require`] hub is closed at once with 4003 [`TLS_REQUIRED_REASON`], so an
/// older client shows a reason rather than a failed handshake.
fn upgrade<S: IntoWire>(
    shared: &Arc<Shared>,
    id: u64,
    mut tcp: S,
    encrypted: bool,
    tail: Vec<u8>,
    accept: &str,
    peer: IpAddr,
) {
    // -- WP HB9 -- The pre-auth clock starts here, where the WebSocket budget's count does.
    let upgraded = Instant::now();
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: {SUBPROTOCOL}\r\n\r\n"
    );
    let stall = shared.limits.write_stall;
    if write_by(&mut tcp, response.as_bytes(), stall, after(upgraded, stall)).is_err() {
        return;
    }
    let limits = &shared.limits;
    // -- WP SECSRV -- on the socket: a TLS half's reads go by its timeout from here
    let raw = tcp.raw_tcp();
    let set = raw
        .set_write_timeout(Some(limits.write_stall))
        .and(raw.set_read_timeout(Some(limits.read_tick)));
    let halves = raw
        .try_clone()
        .and_then(|cut_off| Ok((cut_off, tcp.into_wires()?)));
    let (Ok(()), Ok((cut_off, (reader, writer)))) = (set, halves) else {
        return;
    };
    let config = server_ws_config(MAX_FRAME as usize);
    let reader = WebSocket::from_partially_read(reader, tail, WsRole::Server, Some(config));
    let writer = WebSocket::from_raw_socket(writer, WsRole::Server, Some(config));
    let tx = Arc::new(WsTx {
        ws: Mutex::new(writer),
        tcp: cut_off,
        close_grace: limits.close_grace,
        closing: OnceLock::new(),
    });
    shared.set_tx(id, &tx);
    let mut connection = Connection {
        shared: Arc::clone(shared),
        id,
        peer,
        tx,
        session: None,
        writer: None,
        // -- fixup sw2x --
        last_system_status: None,
        // -- WP SECSRV --
        encrypted,
    };
    if !encrypted && !shared.policy.serves_plaintext() {
        log::debug!("hub: closed a plaintext WebSocket: TLS is required");
        connection
            .tx
            .close(CLOSE_SHUTTING_DOWN, TLS_REQUIRED_REASON);
    }
    connection.run(reader, upgraded);
}

/// The sending half of a WebSocket. The writer thread sends through it once there is
/// one; before that the reader does, since a socket with no session has no thread to
/// spare for a writer.
struct WsTx {
    // -- WP SECSRV -- plain or TLS
    ws: Mutex<WebSocket<Wire>>,
    tcp: TcpStream,
    close_grace: Duration,
    closing: OnceLock<Instant>,
}

impl WsTx {
    fn send(&self, message: Message) -> Result<(), tungstenite::Error> {
        guard(&self.ws).send(message)
    }

    /// Send a close frame, best effort, and half-close. The reader then gives the peer
    /// [`Self::close_deadline`] to answer. If a stalled write holds the socket there
    /// is no use for a close frame, and it would be cut off a frame in part way.
    fn close(&self, code: u16, reason: &str) {
        if self.closing.set(Instant::now()).is_err() {
            return;
        }
        let _ = self.tcp.set_write_timeout(Some(self.close_grace));
        let ws = match self.ws.try_lock() {
            Ok(ws) => Some(ws),
            Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        };
        let Some(mut ws) = ws else {
            self.abort();
            return;
        };
        let mut cut = reason.len().min(MAX_CLOSE_REASON);
        while !reason.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        let frame = CloseFrame {
            code: CloseCode::from(code),
            reason: reason.get(..cut).unwrap_or_default().into(),
        };
        let _ = ws.close(Some(frame));
        let _ = ws.flush();
        // -- WP SECSRV --
        ws.get_mut().close_notify();
        let _ = self.tcp.shutdown(Shutdown::Write);
    }

    /// Whether a close has been sent, and until when the peer may answer it.
    fn close_deadline(&self) -> Option<Instant> {
        self.closing.get().map(|at| after(*at, self.close_grace))
    }

    /// Cut the connection, which ends a read or write blocked on it, on any thread.
    fn abort(&self) {
        let _ = self.tcp.shutdown(Shutdown::Both);
    }
}

fn encode(item: Outbound, poll_seq: &mut u16) -> Option<Message> {
    let frame = match item {
        Outbound::Notify {
            seq,
            handle,
            origin,
            packet,
        } => Frame::Notify(NotifyFrame {
            seq,
            handle,
            origin,
            packet: packet.to_vec(),
        }),
        Outbound::Resync { handle, reason } => Frame::Resync { handle, reason },
        Outbound::Poll {
            handle,
            slot,
            payload,
        } => {
            let seq = *poll_seq;
            *poll_seq = seq.wrapping_add(1);
            Frame::Poll(PollFrame {
                seq,
                handle,
                slot,
                payload,
            })
        }
        Outbound::Response(response) => Frame::CmdResponse(response),
        Outbound::Event(json) => return Some(Message::text(json)),
        Outbound::Close { .. } => return None,
    };
    Some(Message::binary(frame.encode()))
}

/// `hub-conn-tx-<id>`: the session's outbox onto the socket, and the pings.
fn writer_loop(tx: &WsTx, outbox: &Outbox, ping_interval: Duration) {
    let mut poll_seq = 0u16;
    let mut next_ping = after(Instant::now(), ping_interval);
    loop {
        let wait = next_ping.saturating_duration_since(Instant::now());
        let sent = match outbox.pop(wait) {
            Pop::Item(Outbound::Close { code, reason }) => {
                tx.close(code, &reason);
                return;
            }
            Pop::Item(item) => match encode(item, &mut poll_seq) {
                Some(message) => tx.send(message),
                None => Ok(()),
            },
            Pop::Idle => Ok(()),
            Pop::Closed => {
                tx.close(CLOSE_NORMAL, "");
                return;
            }
        };
        let sent = sent.and_then(|()| {
            let now = Instant::now();
            if now < next_ping {
                return Ok(());
            }
            next_ping = after(now, ping_interval);
            tx.send(Message::Ping(Vec::new().into()))
        });
        if let Err(e) = sent {
            // Spec 5's 1011 for a peer not reading can rarely be sent: the stalled write
            // may have cut a frame in two. So it ends here, and the reader goes with it.
            log::debug!("hub: a connection's write failed: {e}");
            tx.abort();
            return;
        }
    }
}

/// One upgraded socket, on its reader thread. Dropping it ends everything the
/// connection had, on a panic too: session (locks, polls), socket, writer.
struct Connection {
    shared: Arc<Shared>,
    id: u64,
    peer: IpAddr,
    tx: Arc<WsTx>,
    session: Option<Arc<Session>>,
    writer: Option<JoinHandle<()>>,
    // -- fixup sw2x --
    /// The last `system.status` this connection's session was answered.
    last_system_status: Option<Instant>,
    // -- WP SECSRV --
    /// It came over TLS. SECPAIR's `tls_only` mark and `ok.hub_key` go by it.
    encrypted: bool,
}

// -- fixup sw2x --
/// A session may ask `system.status` at most this often: each one costs every
/// unit's worker five or more transfers.
pub const SYSTEM_STATUS_INTERVAL: Duration = Duration::from_secs(1);

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            self.shared.hub.close_session(session.id());
        }
        self.tx.abort();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

impl Connection {
    fn run(&mut self, mut reader: WebSocket<Wire>, upgraded: Instant) {
        let limits = self.shared.limits.clone();
        let auth_deadline = after(upgraded, limits.auth_timeout);
        let mut last_heard = Instant::now();
        while self.tx.close_deadline().is_none() {
            match reader.read() {
                Ok(message) => {
                    last_heard = Instant::now();
                    // A close from the peer has its reply queued; the next read ends it.
                    self.on_message(message);
                }
                // Our own failed pong looks like this too: see `Pipe::is_broken`.
                Err(tungstenite::Error::Io(e))
                    if is_timeout(&e) && !reader.get_ref().is_broken() => {}
                Err(
                    tungstenite::Error::Capacity(_)
                    | tungstenite::Error::Protocol(_)
                    | tungstenite::Error::Utf8(_),
                ) => {
                    log::debug!("hub: a client broke the framing");
                    self.tx.close(CLOSE_PROTOCOL, "protocol error");
                }
                Err(_) => return,
            }
            let now = Instant::now();
            if self.session.is_none() {
                if now >= auth_deadline {
                    self.tx.close(CLOSE_AUTH_TIMEOUT, "authentication timeout");
                }
            } else if now.saturating_duration_since(last_heard) >= limits.dead_peer {
                self.tx.close(CLOSE_KEEPALIVE, "keepalive failure");
            }
        }
        if let Some(deadline) = self.tx.close_deadline() {
            // -- WP SECSRV -- the bytes on the socket, decrypted or not
            if let Ok(mut tcp) = reader.get_mut().raw_tcp().try_clone() {
                drain(&mut tcp, deadline);
            }
        }
    }

    fn on_message(&mut self, message: Message) {
        match message {
            Message::Text(text) => self.on_text(text.as_str()),
            Message::Binary(bytes) => self.on_binary(&bytes),
            _ => {}
        }
    }

    /// The session, unless it has been closed under this connection (revoked, the hub
    /// stopping): then nothing more of this client's may reach a device.
    fn live_session(&self) -> Option<Arc<Session>> {
        self.session
            .as_ref()
            .filter(|s| !s.outbox().is_closed())
            .map(Arc::clone)
    }

    fn on_binary(&mut self, bytes: &[u8]) {
        // Before authentication binary frames are discarded (spec 5).
        let Some(session) = self.live_session() else {
            return;
        };
        let answer = |tag, status| {
            session
                .outbox()
                .push_reliable(Outbound::Response(CmdResponse {
                    tag,
                    status,
                    payload: Vec::new(),
                }));
        };
        let Some(tag) = bytes.get(2..4).map(|t| {
            u16::from_le_bytes([
                t.first().copied().unwrap_or(0),
                t.get(1).copied().unwrap_or(0),
            ])
        }) else {
            return;
        };
        match Frame::decode(bytes) {
            Ok(Frame::CmdRequest(request)) => {
                let done = crate::Completion::to_outbox(request.tag, Arc::clone(session.outbox()));
                self.shared.hub.submit(&session, request, done);
            }
            // `fw.install` is not implemented, so no transfer can be open.
            Ok(Frame::FwData { xfer, .. }) => answer(xfer, status::UNSUPPORTED),
            // Spec 8: a request type this hub does not know gets this under its own tag.
            Ok(Frame::Unknown { kind, tag }) if kind < 0x80 => answer(tag, status::UNSUPPORTED),
            // Hub-to-client types from a client, and reserved ones: ignored.
            Ok(_) => {}
            // Only a request has anyone waiting for an answer.
            Err(_) if bytes.first() == Some(&kind::CMD_REQUEST) => answer(tag, status::BAD_FRAME),
            Err(_) => {}
        }
    }

    fn on_text(&mut self, text: &str) {
        // Not JSON, or not one of ours: protocol error (spec 5), as Swift's.
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            self.tx.close(CLOSE_PROTOCOL, "malformed message");
            return;
        };
        let Some(message) = value.as_object() else {
            self.tx.close(CLOSE_PROTOCOL, "malformed message");
            return;
        };
        let Some(t) = message.get("t").and_then(Value::as_str) else {
            self.tx.close(CLOSE_PROTOCOL, "malformed message");
            return;
        };
        let id = message.get("id").and_then(Value::as_u64);
        match t {
            "hello" => self.on_hello(),
            "auth.token" => self.on_auth_token(id, message),
            "auth.pair" => self.on_auth_pair(id, message),
            _ => match self.live_session() {
                Some(session) => {
                    // -- WP P2RS -- the chain in one place, which the REST mirror runs too
                    let answer = control_plane(&self.shared.hub, &session, &value);
                    match answer {
                        Some(answer) => self.reply(answer),
                        None => self.on_request(t, id, message, &session),
                    }
                }
                None if self.session.is_some() => {}
                None => self.reply(err(id, "unauthenticated", None)),
            },
        }
    }

    /// A reply, in order with everything else the session is sent, and before it has
    /// a session straight on the socket.
    fn reply(&self, text: String) {
        match &self.session {
            Some(session) => session.outbox().push_reliable(Outbound::Event(text)),
            None => {
                if self.tx.send(Message::text(text)).is_err() {
                    self.tx.abort();
                }
            }
        }
    }

    fn on_hello(&mut self) {
        self.reply(self.shared.hello());
        if self.shared.auth.mode() == AccessMode::Open && self.session.is_none() {
            // Open access: the session opens with this answer, and no auth message follows
            // (spec 6.1).
            self.open_session(Role::Admin, None, |_| None);
        }
    }

    fn already_authenticated(&self, id: Option<u64>) {
        self.reply(err(id, "bad_request", Some("already authenticated")));
    }

    fn on_auth_token(&mut self, id: Option<u64>, message: &Map<String, Value>) {
        if self.shared.auth.mode() == AccessMode::Open {
            return self.open_access_ok(id);
        }
        if self.session.is_some() {
            return self.already_authenticated(id);
        }
        let Some(token) = message.get("token").and_then(Value::as_str) else {
            return self.reply(err(id, "bad_request", Some("no token")));
        };
        match self
            .shared
            .auth
            .verify_token(token, self.peer, Instant::now())
        {
            // -- WP P2CFIXA -- an API key is for REST and MQTT, never a DSPi Link session
            Ok(authed) if authed.kind == crate::auth::ClientKind::Api => self.reply(err(
                id,
                "unauthenticated",
                Some("an API key opens no DSPi Link session"),
            )),
            Ok(authed) => {
                let shared = Arc::clone(&self.shared);
                self.open_session(authed.role, Some(authed.cid), move |session| {
                    Some(auth_ok(&shared, id, session, None))
                });
            }
            Err(e) => self.reply(auth_err(id, e)),
        }
    }

    fn on_auth_pair(&mut self, id: Option<u64>, message: &Map<String, Value>) {
        if self.shared.auth.mode() == AccessMode::Open {
            return self.open_access_ok(id);
        }
        if self.session.is_some() {
            return self.already_authenticated(id);
        }
        // -- WP SECSRV -- a PIN and the token it earns never cross in the clear on a hub
        // that speaks TLS (OD-SECk), unless upstream compatibility is on. Before the PIN
        // is looked at: no guess counts, and the window stays open.
        if !self.encrypted && !self.shared.policy.plaintext_pairing() {
            return self.reply(err(
                id,
                TLS_REQUIRED,
                Some("pairing needs an encrypted connection: update the client"),
            ));
        }
        let Some(pin) = message.get("pin").and_then(Value::as_str) else {
            return self.reply(err(id, "bad_request", Some("no pin")));
        };
        let requested = match message.get("role") {
            None | Some(Value::Null) => None,
            Some(role) => match role.as_str().and_then(parse_role) {
                Some(role) => Some(role),
                None => return self.reply(err(id, "bad_request", Some("unknown role"))),
            },
        };
        let name = message.get("name").and_then(Value::as_str).unwrap_or("");
        match self
            .shared
            .auth
            .pair(pin, name, requested, self.peer, Instant::now())
        {
            Ok(authed) => {
                let shared = Arc::clone(&self.shared);
                self.open_session(authed.role, Some(authed.cid), move |session| {
                    Some(auth_ok(&shared, id, session, authed.token.as_deref()))
                });
            }
            Err(e) => self.reply(auth_err(id, e)),
        }
    }

    /// A client on an open hub may still ask for its session id, with any PIN or token
    /// (spec 6.1).
    fn open_access_ok(&mut self, id: Option<u64>) {
        if self.session.is_none() {
            self.open_session(Role::Admin, None, |_| None);
        }
        if let Some(session) = self.live_session() {
            self.reply(auth_ok(&self.shared, id, &session, None));
        }
    }

    /// Open the session this connection has earned, and its writer. `answer` makes the
    /// first thing to send, so nothing the session is queued goes out ahead of it, which
    /// would reach the client before it knows its own session id (`origin`).
    fn open_session(
        &mut self,
        role: Role,
        cid: Option<u32>,
        answer: impl FnOnce(&Session) -> Option<String>,
    ) {
        let shared = Arc::clone(&self.shared);
        let session = match shared.hub.open_session(role, cid) {
            Ok(session) => session,
            Err(_) => {
                self.tx.close(CLOSE_SHUTTING_DOWN, "hub shutting down");
                return;
            }
        };
        self.session = Some(Arc::clone(&session));
        shared.set_session(self.id, &session);
        // Listed first, then asked: a revocation or a role change either finds the session
        // in the table, or has already happened and is seen here.
        if let Some(cid) = cid {
            match shared.auth.role_of(cid) {
                Some(role) => session.set_role(role),
                None => {
                    self.tx.close(CLOSE_REVOKED, "revoked");
                    return;
                }
            }
        }
        if let Some(text) = answer(&session)
            && self.tx.send(Message::text(text)).is_err()
        {
            self.tx.abort();
            return;
        }
        let (tx, outbox) = (Arc::clone(&self.tx), Arc::clone(session.outbox()));
        let ping = shared.limits.ping_interval;
        let spawned = thread::Builder::new()
            .name(format!("hub-conn-tx-{}", self.id))
            .spawn(move || writer_loop(&tx, &outbox, ping));
        match spawned {
            Ok(writer) => self.writer = Some(writer),
            Err(e) => {
                log::warn!("hub: no thread for a connection: {e}");
                self.tx.close(1011, "out of resources");
            }
        }
    }

    fn on_request(
        &mut self,
        t: &str,
        id: Option<u64>,
        message: &Map<String, Value>,
        session: &Arc<Session>,
    ) {
        let handle = message
            .get("handle")
            .and_then(Value::as_u64)
            .map(|h| u8::try_from(h).ok());
        let admin = session.role() == Role::Admin;
        let denied = || err(id, "denied", Some("admin role required"));
        let answer = match t {
            "device.list" => {
                let devices: Vec<Value> = self
                    .shared
                    .hub
                    .devices()
                    .iter()
                    .map(|d| d.to_json())
                    .collect();
                ok(id, json!({"devices": devices}))
            }
            "device.snapshot" => self.snapshot(id, handle),
            "poll.subscribe" => self.poll_subscribe(id, handle, message, session),
            "poll.unsubscribe" => self.poll_unsubscribe(id, handle, message, session),
            "device.rename" => {
                // Answered by the device's worker, or at once.
                if let Some(refused) = self.rename(id, handle, message, session) {
                    self.reply(refused);
                }
                return;
            }
            "auth.list" if !admin => denied(),
            "auth.list" => ok(id, json!({"clients": self.client_list()})),
            "auth.revoke" | "auth.set_role" if !admin => denied(),
            "auth.revoke" => match self.auth_revoke(id, message) {
                Some(answer) => answer,
                None => return,
            },
            "auth.set_role" => self.auth_set_role(id, message),
            // -- WP P2RS -- an API key, shown in this answer only
            "auth.mint" if !admin => denied(),
            "auth.mint" => crate::rest::mint(&self.shared.auth, id, message, session.role()),
            // `fw.install` is later work (PLAN-CLIENTS), and the hub messages are not in
            // HB1's entry. -- WP P2SV -- `hub.stats` and `hub.rename` reach here only when
            // no service answered them (the services run first, in `on_text`).
            "hub.stats" | "hub.rename" if !admin => denied(),
            "fw.install" | "hub.stats" | "hub.rename" => err(id, "unsupported", None),
            // -- WP SW2 --
            t if t.starts_with("system.") => self.system(t, id, message, session.role()),
            _ => err(id, "unknown_type", None),
        };
        self.reply(answer);
    }

    // -- WP SW2 --
    /// The audio link's system model (`docs/integrator/audio-link.md`). Reading is
    /// for every role; acknowledging an alarm and applying a scene for control
    /// and admin; changing the model and writing the alignment for admin, as link
    /// configuration is an admin action (`extension_channel_spec.md` 2.8). The
    /// reads and writes block on the devices, on this connection alone.
    fn system(
        &mut self,
        t: &str,
        id: Option<u64>,
        message: &Map<String, Value>,
        role: Role,
    ) -> String {
        let hub = &self.shared.hub;
        let admin_only = matches!(t, "system.set" | "system.align");
        // -- fixup sw2x -- a scene and the system mute are then judged op by op
        // against the policy (`HubHandle::system_apply_scene`).
        let control = matches!(t, "system.scene.apply" | "system.alarm.ack" | "system.mute");
        if (admin_only && role != Role::Admin) || (control && role == Role::Viewer) {
            let need = if admin_only { "admin" } else { "control" };
            return err(id, "denied", Some(&format!("{need} role required")));
        }
        let json = |v: Result<Value, serde_json::Error>| match v {
            Ok(body) => ok(id, body),
            Err(_) => err(id, "internal", None),
        };
        match t {
            "system.get" => {
                let (model, revision) = hub.system_model();
                // -- fixup sw3x -- the role too: a role changed live reaches no client otherwise.
                json(
                    serde_json::to_value(model).map(
                        |m| json!({"model": m, "revision": revision, "role": role_name(role)}),
                    ),
                )
            }
            "system.set" => {
                let Some(model) = message.get("model").cloned() else {
                    return err(id, "bad_request", Some("no model"));
                };
                let model = match serde_json::from_value(model) {
                    Ok(m) => m,
                    Err(e) => return err(id, "bad_request", Some(&e.to_string())),
                };
                // -- fixup sw3x -- `revision`: the one the model was edited from.
                let expected = message.get("revision").and_then(Value::as_u64);
                match hub.system_set_model_at(model, expected) {
                    // -- WP HWLK1F -- a unit without the audio link: kept, with a warning
                    Ok(revision) => ok(
                        id,
                        json!({"revision": revision, "warnings": hub.system_alink_warnings()}),
                    ),
                    Err(crate::system::ModelRefused::Invalid(e)) => {
                        err(id, "bad_request", Some(&e.to_string()))
                    }
                    Err(e @ crate::system::ModelRefused::Stale { .. }) => {
                        err(id, "conflict", Some(&e.to_string()))
                    }
                    Err(e @ crate::system::ModelRefused::NotSaved(_)) => {
                        err(id, "internal", Some(&e.to_string()))
                    }
                }
            }
            "system.status" => {
                // -- fixup sw2x -- at most one a second per session.
                let now = Instant::now();
                if let Some(last) = self.last_system_status
                    && now.saturating_duration_since(last) < SYSTEM_STATUS_INTERVAL
                {
                    return err(
                        id,
                        "rate_limited",
                        Some("system.status at most once a second per session"),
                    );
                }
                self.last_system_status = Some(now);
                // -- fixup sw3x -- with this session's role now (see system.get)
                json(serde_json::to_value(hub.system_status()).map(|mut v| {
                    if let Value::Object(m) = &mut v {
                        m.insert("role".into(), role_name(role).into());
                    }
                    v
                }))
            }
            "system.mute" => {
                let Some(muted) = message.get("muted").and_then(Value::as_bool) else {
                    return err(id, "bad_request", Some("no muted"));
                };
                match hub.system_set_mute(muted, role) {
                    Ok(writes) => json(serde_json::to_value(writes).map(|w| json!({"writes": w}))),
                    Err(e) => system_refused(id, &e),
                }
            }
            "system.align" => {
                let apply = message
                    .get("apply")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                json(serde_json::to_value(hub.system_align(apply)))
            }
            "system.scene.apply" => {
                let Some(scene) = message.get("scene").and_then(Value::as_str) else {
                    return err(id, "bad_request", Some("no scene"));
                };
                match hub.system_apply_scene(scene, role) {
                    Ok(report) => json(serde_json::to_value(report)),
                    Err(e) => system_refused(id, &e),
                }
            }
            "system.alarms" => {
                json(serde_json::to_value(hub.system_alarms()).map(|a| json!({"alarms": a})))
            }
            "system.alarm.ack" => match message.get("alarm").and_then(Value::as_u64) {
                Some(alarm) => match hub.system_acknowledge(alarm) {
                    Some(a) => json(serde_json::to_value(a).map(|a| json!({"alarm": a}))),
                    None => err(id, "bad_request", Some("no such alarm on")),
                },
                None => err(id, "bad_request", Some("no alarm")),
            },
            _ => err(id, "unknown_type", None),
        }
    }

    /// Blocks for the image if nothing is cached, on this connection alone.
    fn snapshot(&self, id: Option<u64>, handle: Option<Option<u8>>) -> String {
        let Some(handle) = handle else {
            return err(id, "bad_request", Some("no handle"));
        };
        let Some(handle) = handle else {
            return err(id, "no_device", None);
        };
        match self.shared.hub.snapshot(handle).map(serde_json::to_value) {
            Ok(Ok(body)) => ok(id, body),
            Ok(Err(_)) => err(id, "internal", None),
            Err(refused) => err(id, refused.code(), None),
        }
    }

    fn poll_subscribe(
        &self,
        id: Option<u64>,
        handle: Option<Option<u8>>,
        message: &Map<String, Value>,
        session: &Arc<Session>,
    ) -> String {
        let specs = message
            .get("polls")
            .cloned()
            .and_then(|p| serde_json::from_value::<Vec<PollSpec>>(p).ok());
        let (Some(handle), Some(specs)) = (handle, specs) else {
            return err(id, "bad_request", Some("handle and polls are required"));
        };
        let Some(handle) = handle else {
            return err(id, "no_device", None);
        };
        match self.shared.hub.poll_subscribe(session, handle, &specs) {
            Ok(granted) => match serde_json::to_value(granted) {
                Ok(granted) => ok(id, json!({"granted": granted})),
                Err(_) => err(id, "internal", None),
            },
            Err(refused) => err(id, refused.code(), Some(&refused.to_string())),
        }
    }

    fn poll_unsubscribe(
        &self,
        id: Option<u64>,
        handle: Option<Option<u8>>,
        message: &Map<String, Value>,
        session: &Arc<Session>,
    ) -> String {
        let slots = message
            .get("slots")
            .cloned()
            .and_then(|s| serde_json::from_value::<Vec<u8>>(s).ok());
        let (Some(handle), Some(slots)) = (handle, slots) else {
            return err(id, "bad_request", Some("handle and slots are required"));
        };
        if let Some(handle) = handle {
            self.shared.hub.poll_unsubscribe(session, handle, &slots);
        }
        ok(id, json!({}))
    }

    /// `device.rename` (spec 7.4). `Some` when it is refused here; otherwise the answer
    /// is queued when the hub has one.
    fn rename(
        &self,
        id: Option<u64>,
        handle: Option<Option<u8>>,
        message: &Map<String, Value>,
        session: &Arc<Session>,
    ) -> Option<String> {
        let name = message.get("name").and_then(Value::as_str);
        let (Some(handle), Some(name)) = (handle, name) else {
            return Some(err(id, "bad_request", Some("handle and name are required")));
        };
        let Some(handle) = handle else {
            return Some(err(id, "no_device", None));
        };
        let outbox = Arc::clone(session.outbox());
        self.shared
            .hub
            .rename(session, handle, name, move |answer| {
                let text = match answer {
                    status::OK => ok(id, json!({})),
                    status::LOCKED => err(id, "locked", None),
                    status::NO_DEVICE => err(id, "no_device", None),
                    status::DENIED => err(id, "denied", None),
                    // Past the in-flight limit, or a device that cannot take it now (spec 7.4).
                    status::RATE_LIMITED | status::BUSY | status::BULK_LOCKED => {
                        err(id, "busy", None)
                    }
                    _ => err(id, "internal", None),
                };
                outbox.push_reliable(Outbound::Event(text));
            });
        None
    }

    fn client_list(&self) -> Vec<Value> {
        let online: Vec<u32> = guard(&self.shared.table)
            .conns
            .values()
            .filter(|e| !e.done)
            .filter_map(|e| e.session.as_ref().and_then(|s| s.client_id()))
            .collect();
        self.shared
            .auth
            .clients()
            .into_iter()
            .map(|c| {
                let mut client = json!({
                    "cid": c.cid,
                    "name": c.name,
                    "role": role_name(c.role),
                    "created": iso8601(unix_seconds(c.created)),
                    // -- WP P2RS -- a key with a REST session open counts as online
                    "online": online.contains(&c.cid) || self.shared.rest.is_open(c.cid),
                    "kind": c.kind.wire_name(),
                    "plaintext_ok": c.plaintext_ok,
                });
                if let (Some(seen), Some(map)) = (c.last_seen, client.as_object_mut()) {
                    map.insert("last_seen".into(), iso8601(unix_seconds(seen)).into());
                }
                client
            })
            .collect()
    }

    /// `None` when it has answered already.
    fn auth_revoke(&self, id: Option<u64>, message: &Map<String, Value>) -> Option<String> {
        let refused = || Some(err(id, "bad_request", Some("no such client")));
        let Some(cid) = cid_of(message) else {
            return refused();
        };
        if !self.shared.auth.revoke(cid) {
            return refused();
        }
        // Before the sessions close, since it may be this client revoking itself.
        self.reply(ok(id, json!({})));
        self.shared.close_client(cid);
        None
    }

    fn auth_set_role(&self, id: Option<u64>, message: &Map<String, Value>) -> String {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .and_then(parse_role);
        let (Some(cid), Some(role)) = (cid_of(message), role) else {
            return err(id, "bad_request", Some("cid and role are required"));
        };
        let control = NetworkControl {
            shared: Arc::clone(&self.shared),
        };
        if control.set_role(cid, role) {
            ok(id, json!({}))
        } else {
            err(id, "bad_request", Some("no such client"))
        }
    }
}

fn cid_of(message: &Map<String, Value>) -> Option<u32> {
    message
        .get("cid")
        .and_then(Value::as_u64)
        .and_then(|c| u32::try_from(c).ok())
}

// -- WP P2RS --
/// The handlers a session's control-plane message goes through before the hub's own
/// `device.*`, `poll.*`, `auth.*`, `hub.*` and `system.*` answers: the lock's, the
/// guarded apply's, the limits' and the services'. The WebSocket and the REST mirror
/// both run it, so a REST answer is the WebSocket's. `None`: none of them claims it.
pub(crate) fn control_plane(
    hub: &HubHandle,
    session: &Arc<Session>,
    value: &Value,
) -> Option<String> {
    // -- WP P1HA -- `apply.*` after the lock's messages
    crate::lock::handle_message(hub, session, value)
        .or_else(|| crate::apply_guard::handle_message(hub, session, value))
        // -- WP P1HL -- `limits.*` and `mode.*`
        .or_else(|| crate::limits::handle_message(hub, session, value))
        // -- WP P2SV -- the services (`service.rs`), after the limits
        .or_else(|| hub.service_message(session, value))
}

/// `{"t":"ok","id":…, …body}`.
fn ok(id: Option<u64>, body: Value) -> String {
    let mut reply = match body {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    reply.insert("t".into(), "ok".into());
    if let Some(id) = id {
        reply.insert("id".into(), id.into());
    }
    Value::Object(reply).to_string()
}

// -- fixup sw2x --
/// A refused scene or system mute as an `err`.
fn system_refused(id: Option<u64>, e: &crate::system::SystemRefused) -> String {
    use crate::system::SystemRefused as R;
    use crate::system::scene::SceneRefused as S;
    match e {
        R::Scene(S::NoScene) => err(id, "bad_request", Some("no such scene")),
        R::Scene(S::NoFrame) => err(
            id,
            "no_device",
            Some("no unit of the scene reports an audio link frame"),
        ),
        R::Denied(why) => err(id, "denied", Some(why)),
        R::NoSender => err(id, "no_device", Some("no sender of the model is reachable")),
    }
}

/// `{"t":"err","id":…,"code":…}`. A wrong PIN has no `msg`, as real hubs (spec 7.1).
fn err(id: Option<u64>, code: &str, msg: Option<&str>) -> String {
    let mut reply = json!({"t": "err", "code": code});
    if let Some(map) = reply.as_object_mut() {
        if let Some(id) = id {
            map.insert("id".into(), id.into());
        }
        if let Some(msg) = msg {
            map.insert("msg".into(), msg.into());
        }
    }
    reply.to_string()
}

fn auth_err(id: Option<u64>, e: AuthError) -> String {
    let code = match e {
        AuthError::Unauthenticated => "unauthenticated",
        AuthError::RateLimited => "rate_limited",
        AuthError::TableFull => "busy",
        AuthError::Internal => "internal",
    };
    err(id, code, None)
}

/// The `ok` to `auth.token` and `auth.pair` (spec 7.3).
fn auth_ok(shared: &Shared, id: Option<u64>, session: &Session, token: Option<&str>) -> String {
    let role = session.role();
    let denied = shared.hub.denied(role);
    let mut body = json!({
        "session": session.id(),
        "role": role_name(role),
        "policy": {"denied": denied},
    });
    if let (Some(token), Some(map)) = (token, body.as_object_mut()) {
        map.insert("token".into(), token.into());
    }
    ok(id, body)
}

impl Shared {
    fn hub_info(&self) -> Value {
        json!({
            "id": self.hub_id,
            "name": self.hub.name(),
            "kind": "console",
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    /// The hub's `hello` (spec 7.2). `fw_install` is not in `caps`: HUB.md section 8.
    fn hello(&self) -> String {
        let mut caps = vec!["cmd", "notify", "poll", "snapshot", "lock", "rename"];
        // -- WP P1HA --
        caps.push(crate::apply_guard::CAP);
        // -- WP P1HL --
        caps.push(crate::limits::CAP);
        // -- WP P2SV -- each service's
        for cap in self.hub.service_caps() {
            if !caps.contains(&cap) {
                caps.push(cap);
            }
        }
        if self.web {
            caps.push("web");
        }
        // -- WP P2RS -- `auth.mint` always; `/api/v1/` while it is on
        caps.push(crate::rest::CAP_API_KEYS);
        if self.rest.is_on() {
            caps.push(crate::rest::CAP_REST);
        }
        let mode = self.auth.mode();
        let mut hello = json!({
            "t": "hello",
            "proto": {"major": PROTO_MAJOR, "minor": PROTO_MINOR},
            "hub": self.hub_info(),
            "auth": mode.wire_name(),
            "caps": caps,
            "limits": {
                "max_frame": MAX_FRAME,
                "max_payload": MAX_PAYLOAD,
                "max_inflight": MAX_INFLIGHT,
                "poll_max_hz": POLL_MAX_HZ,
                "poll_budget_bps": POLL_BUDGET_BPS,
            },
        });
        if mode == AccessMode::Pin
            && let Some(map) = hello.as_object_mut()
        {
            // Window PINs only: there is no fixed PIN on a hub here.
            map.insert("pin_source".into(), "window".into());
        }
        hello.to_string()
    }

    /// `GET /dspi/v1/info` (spec 3.2).
    fn info(&self) -> String {
        let devices: Vec<Value> = self
            .hub
            .devices()
            .iter()
            .map(|d| json!({"serial": d.identity.serial, "name": d.name}))
            .collect();
        let mut info = json!({
            "proto": {"major": PROTO_MAJOR, "minor": PROTO_MINOR},
            "hub": self.hub_info(),
            "auth": self.auth.mode().wire_name(),
            // -- WP SECSRV -- served, and the key a client can pin (base64url, as a QR
            // code's `k=`); readable over plaintext in `on`
            "tls": self.policy.serves_tls(),
            "ws": WS_PATH,
            "devices": devices,
        });
        if let (Some(tls), Some(map)) = (&self.tls, info.as_object_mut()) {
            let spki = guard(&tls.identity).pin().to_b64url();
            map.insert("tls_spki".into(), spki.into());
        }
        info.to_string()
    }
}

/// ISO-8601 UTC with a `Z` (spec 7.1), from seconds since the epoch.
fn iso8601(secs: u64) -> String {
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

/// A running server.
pub struct Server {
    control: NetworkControl,
    acceptor: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("port", &self.port())
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Bind and start accepting. Binds LAN interfaces only and refuses peers off the
    /// local network (spec 6.4). A port in use is [`HubError::Io`]. `auth` is the one
    /// owner of the hub's store, which `Hub::start` opened to learn the hub's id.
    pub fn start(
        hub: HubHandle,
        config: &NetworkConfig,
        auth: Arc<AuthStore>,
    ) -> Result<Self, HubError> {
        let hub_id = hub.hub_id().to_owned();
        // -- WP SECSRV -- the key before the port: a hub that cannot serve TLS as asked
        // does not start plaintext instead
        let policy = TlsPolicy {
            mode: config.tls,
            upstream_compat: config.upstream_compat,
        };
        if policy.upstream_compat && policy.mode == TlsMode::Require {
            return Err(HubError::Io(
                "upstream compatibility needs plaintext, which TLS mode require refuses: \
                 use TLS mode on"
                    .into(),
            ));
        }
        let tls = if policy.serves_tls() {
            let names = TlsNames::new(config.mdns_name.as_deref(), &[]);
            let (identity, at_start) =
                TlsIdentity::load(&config.store_dir, &hub_id, &names, SystemTime::now())?;
            log::info!(
                "hub: TLS {}, key {}",
                policy.mode.name(),
                identity.fingerprint()
            );
            let tls_config = tls_config(&identity)?;
            let now = Instant::now();
            Some(TlsState {
                identity: Mutex::new(identity),
                config: Mutex::new(tls_config),
                names: Mutex::new(names),
                next_names: Mutex::new(now),
                next_refresh: Mutex::new(after(now, TLS_REFRESH_EVERY)),
                at_start,
            })
        } else {
            None
        };
        let listener = bind(config.bind, config.port)
            .map_err(|e| HubError::Io(format!("cannot listen on port {}: {e}", config.port)))?;
        let io = |e: io::Error| HubError::Io(format!("cannot set up the listener: {e}"));
        let port = listener.local_addr().map_err(io)?.port();
        listener.set_nonblocking(true).map_err(io)?;
        let shared = Arc::new(Shared {
            hub,
            auth,
            hub_id,
            web: config.web,
            mdns_name: config.mdns_name.clone(),
            port,
            limits: config.limits.clone(),
            stopping: AtomicBool::new(false),
            table: Mutex::new(Table::default()),
            changed: Condvar::new(),
            // -- WP TM26 --
            accepting: AtomicBool::new(!config.hold_accepts),
            // -- WP WB7 --
            console: config.console.clone(),
            // -- WP P2RS --
            rest: crate::rest::Rest::new(config.rest),
            // -- WP SECSRV --
            policy,
            tls,
            host_names: config
                .host_names
                .iter()
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect(),
            own_names: Mutex::new(None),
        });
        let acceptor = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("hub-accept".into())
                .spawn(move || accept_loop(&shared, &listener))
                .map_err(|e| HubError::Io(format!("cannot start a hub thread: {e}")))?
        };
        Ok(Self {
            control: NetworkControl { shared },
            acceptor: Mutex::new(Some(acceptor)),
        })
    }

    /// The way to it for a binding.
    pub fn control(&self) -> NetworkControl {
        self.control.clone()
    }

    /// The port actually bound.
    pub fn port(&self) -> u16 {
        self.control.port()
    }

    /// Stop accepting. Existing connections go on until their outbox closes. The port
    /// is free when this returns.
    pub fn stop_accepting(&self) {
        self.control.shared.stopping.store(true, Ordering::SeqCst);
        let acceptor = guard(&self.acceptor).take();
        if let Some(acceptor) = acceptor {
            let _ = acceptor.join();
        }
    }

    /// Join the connections, whose outboxes the hub has closed with 4003. A peer that
    /// will not take the close frame is cut off after a second.
    pub fn join(self) {
        self.stop_accepting();
        let shared = &self.control.shared;
        // A socket with no session has no outbox for the hub to close.
        {
            let table = guard(&shared.table);
            for e in table
                .conns
                .values()
                .filter(|e| !e.done && e.session.is_none())
            {
                match &e.tx {
                    Some(tx) => tx.close(CLOSE_SHUTTING_DOWN, "hub shutting down"),
                    None => {
                        let _ = e.tcp.shutdown(Shutdown::Both);
                    }
                }
            }
        }
        let cut_off = after(Instant::now(), shared.limits.close_grace);
        let mut table = guard(&shared.table);
        while table.conns.values().any(|e| !e.done) {
            let left = cut_off.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            table = shared
                .changed
                .wait_timeout(table, left)
                .map(|(t, _)| t)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        // Past its grace, whoever is left. Every read and write on them now fails.
        for e in table.conns.values().filter(|e| !e.done) {
            let _ = e.tcp.shutdown(Shutdown::Both);
        }
        while table.conns.values().any(|e| !e.done) {
            table = shared
                .changed
                .wait(table)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(table);
        // A thread stored after it was done is reaped here too, since none is left unstored:
        // the acceptor, which stores them, has been joined.
        shared.reap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.control.shared.stopping.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb1 {
    use super::*;

    fn own() -> OwnNames {
        OwnNames {
            hosts: vec![
                "localhost".into(),
                "127.0.0.1".into(),
                "::1".into(),
                "192.168.1.5".into(),
                "fe80::1234".into(),
                "studio-mac.local".into(),
            ],
            port: 11915,
        }
    }

    #[test]
    fn wp_hb1_a_native_client_with_no_origin_passes() {
        assert!(origin_allowed(None, &own()));
    }

    #[test]
    fn wp_hb1_the_hubs_own_page_passes() {
        for origin in [
            "http://192.168.1.5:11915",
            "http://127.0.0.1:11915",
            "http://localhost:11915",
            "http://LOCALHOST:11915",
            "HTTP://studio-mac.local:11915",
            "http://studio-mac.local:11915",
            "http://[::1]:11915",
            "http://[fe80::1234]:11915",
            "http://[FE80:0::1234]:11915",
        ] {
            assert!(origin_allowed(Some(origin), &own()), "{origin}");
        }
    }

    #[test]
    fn wp_hb1_everything_else_is_refused() {
        for origin in [
            "null",
            "",
            " ",
            "evil.example",
            "http://evil.example:11915",
            "http://evil.example",
            // The scheme: a hub serves no https, and nothing else makes it a page of ours.
            "https://192.168.1.5:11915",
            "ws://192.168.1.5:11915",
            "file://192.168.1.5:11915",
            "chrome-extension://abcdef",
            // The port: another service on the same machine is another origin.
            "http://192.168.1.5",
            "http://192.168.1.5:80",
            "http://192.168.1.5:11916",
            "http://192.168.1.5:",
            "http://192.168.1.5:+11915",
            "http://192.168.1.5:011915",
            "http://192.168.1.5:11915x",
            "http://192.168.1.5:99999",
            "http://[::1]",
            "http://[::1]11915",
            // Our names as a part of someone else's.
            "http://192.168.1.5.evil.example:11915",
            "http://evil.example/192.168.1.5:11915",
            "http://evil-192.168.1.5:11915",
            "http://192.168.1.5@evil.example:11915",
            "http://evil.example:11915@192.168.1.5:11915",
            "http://evil.example#@192.168.1.5:11915",
            // Not the shape a browser sends.
            "http://192.168.1.5:11915/",
            "http://192.168.1.5:11915/path",
            "http://192.168.1.5:11915?x",
            "http://192.168.1.5:11915#x",
            "http://192.168.1.5:11915 http://evil.example",
            "http://192.168.1.5:11915, http://evil.example",
            "http://[::1:11915",
            "http://[not-v6]:11915",
            "http://[fe80::1234%25eth0]:11915",
            "http://:11915",
            "http://",
            "http:",
            // Spelt differently, with a name that is not ours.
            "http://studio-mac.local.:11915",
            "http://127.1:11915",
            "http://2130706433:11915",
            "http://0x7f.0.0.1:11915",
            "http://[::ffff:127.0.0.1]:11915",
            "http://192.168.1.6:11915",
            "http://ÿ:11915",
        ] {
            assert!(!origin_allowed(Some(origin), &own()), "{origin:?}");
        }
    }

    #[test]
    fn wp_hb1_with_no_names_nothing_passes() {
        let nobody = OwnNames {
            hosts: Vec::new(),
            port: 11915,
        };
        assert!(!origin_allowed(Some("http://127.0.0.1:11915"), &nobody));
    }

    #[test]
    fn wp_hb1_only_peers_on_the_local_network_get_in() {
        for local in [
            "127.0.0.1",
            "127.1.2.3",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.200",
            "169.254.3.4",
            "100.64.0.1",
            "100.127.255.255",
            "::1",
            "fe80::1",
            "febf::1",
            "fd00::1",
            "fc00::1",
            "::ffff:192.168.1.9",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_local_peer(local.parse().unwrap()), "{local}");
        }
        // Not on this machine's interfaces in any test environment: documentation
        // ranges, reserved for exactly this.
        for foreign in [
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.200",
            "2001:db8::1",
            "::ffff:203.0.113.200",
        ] {
            assert!(!is_local_peer(foreign.parse().unwrap()), "{foreign}");
        }
    }

    #[test]
    fn wp_hb1_times_are_iso8601_utc() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601(1_792_391_720), "2026-10-19T06:35:20Z");
        // Very far ahead: wrong in no way that panics.
        let _ = iso8601(u64::MAX);
    }

    #[test]
    fn wp_hb1_replies_have_the_envelope_and_no_empty_fields() {
        assert_eq!(
            serde_json::from_str::<Value>(&err(Some(2), "unauthenticated", None)).unwrap(),
            json!({"t": "err", "id": 2, "code": "unauthenticated"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&err(None, "denied", Some("no"))).unwrap(),
            json!({"t": "err", "code": "denied", "msg": "no"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&ok(Some(7), json!({"x": 1}))).unwrap(),
            json!({"t": "ok", "id": 7, "x": 1})
        );
    }
}
