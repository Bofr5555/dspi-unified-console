//! The loopback HTTP transport (MCP Streamable HTTP, both eras; WP MCP3,
//! MCPDES 1.4 and 3.1). It *is* the server: the app starts it over its
//! [`Server`], and the agents call it, directly or through the stdio
//! [`relay`](crate::relay).
//!
//! **Where it listens.** `127.0.0.1` and `::1` on one port ([`DEFAULT_PORT`]
//! unless the app says otherwise), never any other address, and there is no
//! setting for that. A peer that is not loopback is dropped unanswered.
//!
//! **What it refuses, in this order** (one JSON object per request and
//! reply, `Connection: close` after each):
//! 1. a request with an `Origin` header, HTTP 403: no browser page has a
//!    reason to call it (MCP's DNS-rebinding rule);
//! 2. a `Host` that is not `127.0.0.1`, `localhost` or `[::1]` (any port: an
//!    SSH tunnel's own), or none, or two: 403;
//! 3. a head over [`HttpLimits::head_bytes`] (431), a body over
//!    [`HttpLimits::body_bytes`] (413), no `Content-Length` (411), a type
//!    other than `application/json` (415), a slow sender (408), more than
//!    [`HttpLimits::max_connections`] at once (503);
//! 4. on `/mcp`, no token or a wrong one: 401, the same reply for both, after
//!    the same work ([`Clients::verify`]).
//!
//! **Paths.** `POST /mcp` is MCP. `POST /pair` and `POST /pair/wait` are the
//! relay's pairing ([`crate::auth`]): `{"name"}` gives `{"request", "code",
//! "ticket", "expiresInMs"}`; `{"ticket"}` waits up to
//! [`HttpLimits::pair_wait`] and gives `{"status": "pending"}`, `"allowed"`
//! with `token` (200), `"denied"` (403) or `"gone"` (410). `GET /mcp` is
//! 405: the server sends no requests or notifications of its own.
//!
//! **The eras over HTTP.**
//! - 2026-07-28: `MCP-Protocol-Version` must equal the `_meta` version, and
//!   `Mcp-Method` the method, and `Mcp-Name` the `name` or `uri` of
//!   `tools/call`, `resources/read` and `prompts/get`: otherwise 400 with
//!   error -32020 (HeaderMismatch). An unsupported version or a missing
//!   capabilities `_meta` is 400, an unknown method 404. No session ids.
//! - 2025-11-25 and older: `initialize` gets an `Mcp-Session-Id`, which
//!   every later request must carry (400 without, 404 for one this token
//!   does not own). `DELETE /mcp` ends a session.
//! - A notification or a response is 202 with no body.
//!
//! **Clients.** One paired token is one client of the [`Server`] (one actor
//! in the History, one rate limit), whichever era and however many sessions
//! it uses, under the name the user paired it as. While the app has paused
//! the agents, `tools/call` answers `status: "paused"` and changes nothing
//! (-- WP AIVIS -- not an error: the agent waits).
//!
//! **Held calls** (-- WP MCP4 --). A call the policy holds for the user's
//! word ([`Server::handle_held_value`]) leaves its connection's thread at
//! once: the connection is parked, and counts for none of
//! [`HttpLimits::max_connections`]. The ticking thread answers it when the
//! confirmation has ended; after [`HttpLimits::hold`], shorter than the time
//! agents wait for a reply, it answers `awaiting_confirmation` instead, and
//! the agent asks `dspi_get_confirmation`. A parked connection the agent has
//! closed ends its confirmation with nothing changed. The app answers with
//! [`HttpServer::answer_confirmation`], which runs the call on its thread.
//!
//! **Threads.** One accepting thread per address, one per connection, and one
//! that ticks: once a second, and when a confirmation is answered, it drives
//! [`Server::tick`] and the parked connections. Calls are serialised on the
//! server's mutex; a tool call runs on its connection's thread, and holds the
//! mutex while it writes to the device: the app's calls that need the server
//! ([`HttpServer::with_server`] and the ones that say so) may wait that long,
//! so they never belong on a UI thread. Events ([`Event`]) reach the app's
//! listener on these threads, with no lock held.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use data_encoding::HEXLOWER;
use serde_json::{Map, Value, json};

use crate::auth::{
    AuthError, Clients, Minted, PAIRING_TTL_MS, PairEnd, PairPoll, PairedClient, Pairing,
};
use crate::confirm::AnswerError; // -- WP MCP4 --
use crate::protocol::{
    self, INVALID_PARAMS, INVALID_REQUEST, LEGACY_VERSIONS, META_CAPABILITIES, META_VERSION,
    METHOD_NOT_FOUND, MODERN_VERSIONS, PARSE_ERROR, RpcError, UNSUPPORTED_VERSION,
};
use crate::{ClientId, Server};

/// The port agents are told about, unless the user changes it.
pub const DEFAULT_PORT: u16 = 47860;

/// The MCP endpoint.
pub const MCP_PATH: &str = "/mcp";
/// The relay asks to pair here.
pub const PAIR_PATH: &str = "/pair";
/// The relay waits for the user's answer here.
pub const PAIR_WAIT_PATH: &str = "/pair/wait";

/// MCP 2026-07-28: a header disagrees with the body.
pub const HEADER_MISMATCH: i64 = -32020;

/// What a paused server answers `tools/call` with (-- WP AIVIS -- as `status:
/// "paused"`, not as an error).
pub const PAUSED_TEXT: &str = "DSPi Console: the user has paused AI agents. This call was not run \
and nothing was changed. Wait, or ask the user to press Resume; then call again.";

// -- WP MCP4 --
/// The 503 reply's words while the server stops.
const STOPPED_TEXT: &str = "AI agents have been turned off in DSPi Console";

/// The 401 reply's words.
pub const UNAUTHORISED_TEXT: &str = "DSPi Console does not know this token. Pair this agent again, or add it \
in DSPi Console under Settings, AI agents.";

/// The bounds of the listener.
#[derive(Debug, Clone)]
pub struct HttpLimits {
    /// Connections served at once; more get 503.
    pub max_connections: usize,
    /// The request line and headers.
    pub head_bytes: usize,
    /// The body.
    pub body_bytes: usize,
    /// How long a request may take to arrive, and a reply to leave.
    pub io_timeout: Duration,
    /// How long one `/pair/wait` waits for the user.
    pub pair_wait: Duration,
    /// Legacy sessions kept per client; the oldest goes first.
    pub sessions_per_client: usize,
    // -- WP MCP4 --
    /// How long a held call's connection waits for the user before it is
    /// answered `awaiting_confirmation`. Agent hosts give up on a reply after
    /// 60 s or more.
    pub hold: Duration,
}

/// How often the ticking thread runs (-- WP MCP4 --).
const TICK: Duration = Duration::from_secs(1);

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_connections: 8,
            head_bytes: 16 * 1024,
            body_bytes: 4 * 1024 * 1024,
            io_timeout: Duration::from_secs(10),
            pair_wait: Duration::from_secs(25),
            sessions_per_client: 8,
            hold: Duration::from_secs(45), // -- WP MCP4 --
        }
    }
}

/// How the listener is set up.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// The port; 0 picks a free one (tests).
    pub port: u16,
    /// Where the clients' file is kept; `None` keeps them in memory only.
    pub store_dir: Option<PathBuf>,
    /// The bounds.
    pub limits: HttpLimits,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            store_dir: None,
            limits: HttpLimits::default(),
        }
    }
}

/// Something the app shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A relay asks to pair: show the name and code, Allow or Deny.
    PairingRequested {
        /// For [`HttpServer::pairing_answer`].
        id: u64,
        /// Cleaned.
        name: String,
        /// Four digits.
        code: String,
    },
    /// A pairing request is over.
    PairingEnded {
        /// The request.
        id: u64,
        /// How.
        outcome: PairEnd,
    },
    /// A client was added or removed.
    ClientsChanged,
    // -- WP MCP4 --
    /// A confirmation was asked or has ended: read
    /// [`HttpServer::confirmations`] again.
    ConfirmationsChanged,
    /// An agent called a tool: read [`HttpServer::activity`] again.
    Activity,
    // -- WP MCP5 --
    /// The agents' measurement requests changed: read
    /// [`HttpServer::measure_requests`] again and take what waits.
    MeasureRequests,
    // -- WP FEAT2 --
    /// The agents' requests to open or close a device changed: read
    /// [`HttpServer::device_requests`] again and take what waits.
    DeviceRequests,
    // -- WP AIVIS --
    /// What the agents have running changed ([`HttpServer::work`]): a call
    /// began or ended, a measurement request or a timed preview did.
    Work,
}

impl Event {
    /// `{"event": "pairingRequested" | "pairingEnded" | "clientsChanged", ...}`.
    pub fn to_json(&self) -> Value {
        match self {
            Self::PairingRequested { id, name, code } => {
                json!({"event": "pairingRequested", "id": id, "name": name, "code": code})
            }
            Self::PairingEnded { id, outcome } => {
                json!({"event": "pairingEnded", "id": id, "outcome": outcome.as_str()})
            }
            Self::ClientsChanged => json!({"event": "clientsChanged"}),
            Self::ConfirmationsChanged => json!({"event": "confirmationsChanged"}),
            Self::Activity => json!({"event": "activity"}),
            Self::MeasureRequests => json!({"event": "measureRequests"}), // -- WP MCP5 --
            Self::DeviceRequests => json!({"event": "deviceRequests"}),   // -- WP FEAT2 --
            Self::Work => json!({"event": "work"}),                       // -- WP AIVIS --
        }
    }
}

/// The app's listener for [`Event`]s. It runs on the server's threads.
pub type Listener = Arc<dyn Fn(&Event) + Send + Sync>;

/// Why the listener did not start.
#[derive(Debug)]
pub enum StartError {
    /// The port could not be had (in use, most likely).
    Bind(io::Error),
    /// The clients' file.
    Auth(AuthError),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bind(e) => write!(f, "the AI agents' port cannot be opened: {e}"),
            Self::Auth(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StartError {}

struct Session {
    owner: u64,
    /// When it was last used, in [`Mcp::uses`] order.
    used: u64,
}

/// The MCP side: the server and who is who in it.
struct Mcp {
    server: Server,
    /// Paired client id to the server's client.
    principals: BTreeMap<u64, ClientId>,
    /// Legacy sessions by id.
    sessions: BTreeMap<String, Session>,
    /// Counts session uses, to find the least recently used.
    uses: u64,
    // -- WP MCP2c -- review RB: a request authorised just before a revoke
    // must not make the revoked client a principal again
    /// Paired clients revoked while serving (ids are never reused).
    revoked: std::collections::BTreeSet<u64>,
}

struct Auth {
    clients: Clients,
    pairing: Pairing,
}

struct Shared {
    mcp: Mutex<Mcp>,
    auth: Mutex<Auth>,
    /// Signalled when a pairing request changes.
    paired: Condvar,
    listener: Option<Listener>,
    limits: HttpLimits,
    port: u16,
    started: Instant,
    /// The Unix time of `started`, in ms (-- WP MCP4 --).
    started_unix_ms: u64,
    stopping: AtomicBool,
    paused: AtomicBool,
    connections: AtomicUsize,
    // -- WP MCP4 --
    /// The held calls' connections. Locked after `mcp` when both are.
    parked: Mutex<Vec<Parked>>,
    /// Set to make the ticking thread run now.
    kicked: Mutex<bool>,
    kick: Condvar,
    // -- WP AIVIS --
    /// What the agents have running. A leaf lock: the app reads it on its UI
    /// thread while a call holds `mcp`.
    work: Mutex<Work>,
    /// The tools' titles and whether they write.
    titles: BTreeMap<&'static str, (&'static str, bool)>,
    /// The user's Pause and Resume, for the agents, until `mcp` is next held.
    pending_user: Mutex<Vec<(&'static str, &'static str)>>,
}

// -- WP AIVIS --
/// What the agents have running.
#[derive(Default)]
struct Work {
    next: u64,
    /// The calls being served, by number.
    calls: BTreeMap<u64, Value>,
    /// The newest call, served or done.
    last: Value,
    /// `Server::work`: the requests to the Measure view and the previews.
    jobs: Value,
}

/// A held call's connection, waiting for the user's word (-- WP MCP4 --).
struct Parked {
    /// The confirmation.
    id: u64,
    stream: TcpStream,
    since: Instant,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Shared {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn emit(&self, events: &[Event]) {
        if let Some(l) = &self.listener {
            for e in events {
                l(e);
            }
        }
    }

    // -- WP MCP4 --
    /// Run `f` on the MCP side, then tell the app, unlocked, of the
    /// confirmations it asked or ended, and wake the ticking thread for them.
    fn with_mcp<R>(&self, f: impl FnOnce(&mut Mcp) -> R) -> R {
        let (r, changed, measure, devices, work) = {
            let mut mcp = lock(&self.mcp);
            // -- WP AIVIS -- the user's Pause, told to the agents
            for (kind, what) in std::mem::take(&mut *lock(&self.pending_user)) {
                mcp.server.note_user(kind, what);
            }
            let r = f(&mut mcp);
            let measure = mcp.server.take_measure_events(); // -- WP MCP5 --
            let devices = mcp.server.take_device_events(); // -- WP FEAT2 --
            // -- WP AIVIS --
            let jobs = mcp.server.work();
            let work = {
                let mut w = lock(&self.work);
                let changed = w.jobs != jobs;
                w.jobs = jobs;
                changed
            };
            (
                r,
                !mcp.server.take_confirm_events().is_empty(),
                measure,
                devices,
                work,
            )
        };
        if changed {
            self.wake();
            self.emit(&[Event::ConfirmationsChanged]);
        }
        if measure {
            self.emit(&[Event::MeasureRequests]); // -- WP MCP5 --
        }
        if devices {
            self.emit(&[Event::DeviceRequests]); // -- WP FEAT2 --
        }
        // -- WP AIVIS --
        if work {
            self.emit(&[Event::Work]);
        }
        r
    }

    // -- WP AIVIS --
    /// `client` calls a tool: it runs until [`Shared::call_ended`]. The app
    /// hears of it before the call waits for the server.
    fn call_began(&self, client: &str, msg: &Value) -> u64 {
        let tool = msg
            .pointer("/params/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some((name, (title, write))) = self.titles.get_key_value(tool) else {
            return 0;
        };
        let args = msg.pointer("/params/arguments");
        let mut row = Map::new();
        row.insert("tool".into(), json!(name));
        row.insert("title".into(), json!(title));
        row.insert("write".into(), json!(write));
        row.insert(
            "client_name".into(),
            json!(crate::untrusted::actor_label(client)),
        );
        // Whose, and the numbers it names; the text an agent gives is not shown
        let device = args
            .and_then(|a| a.get("device"))
            .and_then(Value::as_str)
            .and_then(dspi_core::DeviceId::parse);
        row.insert(
            "device".into(),
            json!(device.as_ref().map(dspi_core::DeviceId::as_str)),
        );
        for key in ["output", "input", "channel", "band", "slot"] {
            if let Some(n) = args.and_then(|a| a.get(key)).and_then(Value::as_u64) {
                row.insert(key.into(), json!(n));
            }
        }
        row.insert(
            "at_unix_ms".into(),
            json!(self.started_unix_ms.saturating_add(self.now_ms())),
        );
        let no = {
            let mut w = lock(&self.work);
            w.next = w.next.saturating_add(1);
            let no = w.next;
            row.insert("id".into(), json!(no));
            w.last = Value::Object(row.clone());
            w.calls.insert(no, Value::Object(row));
            no
        };
        self.emit(&[Event::Work]);
        no
    }

    fn call_ended(&self, no: u64) {
        if lock(&self.work).calls.remove(&no).is_some() {
            self.emit(&[Event::Work]);
        }
    }

    fn wake(&self) {
        *lock(&self.kicked) = true;
        self.kick.notify_all();
    }
}

/// The running listener. Dropping it stops it.
pub struct HttpServer {
    shared: Arc<Shared>,
    addrs: Vec<SocketAddr>,
    accepting: Mutex<Vec<JoinHandle<()>>>,
    /// The ticking thread (-- WP MCP4 --).
    ticking: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for HttpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpServer")
            .field("addrs", &self.addrs)
            .finish_non_exhaustive()
    }
}

// -- WP MCP4 --
/// Listen on `127.0.0.1` and `::1`, both or not at all where both exist: with
/// `::1` left to another program, an agent told `localhost` would hand that
/// program its token (review RB). A computer without IPv6 gets `127.0.0.1`
/// alone. Port 0 picks one that is free on both.
fn bind_loopback(port: u16) -> io::Result<Vec<TcpListener>> {
    let mut tries: u8 = if port == 0 { 16 } else { 1 };
    loop {
        tries = tries.saturating_sub(1);
        let v4 = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), port)?;
        let got = v4.local_addr()?.port();
        match bind(IpAddr::V6(Ipv6Addr::LOCALHOST), got) {
            Ok(v6) => return Ok(vec![v4, v6]),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                if tries > 0 {
                    continue;
                }
                return Err(io::Error::new(
                    e.kind(),
                    format!("another program has port {got} on ::1 ({e})"),
                ));
            }
            Err(e) => {
                log::info!("mcp: no IPv6 loopback ({e}), listening on 127.0.0.1 only");
                return Ok(vec![v4]);
            }
        }
    }
}

/// Listen on `ip:port`, without the option that lets another program take a
/// port in use on Windows (std sets `SO_REUSEADDR` only on Unix).
fn bind(ip: IpAddr, port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::new(ip, port))
}

impl HttpServer {
    /// Serve `server` on the loopback addresses. Blocks only to bind.
    pub fn start(
        server: Server,
        config: HttpConfig,
        listener: Option<Listener>,
    ) -> Result<Self, StartError> {
        let clients = match &config.store_dir {
            Some(dir) => Clients::load(dir).map_err(StartError::Auth)?,
            None => Clients::in_memory(),
        };
        let listeners = bind_loopback(config.port).map_err(StartError::Bind)?;
        let port = listeners
            .first()
            .map_or(Ok(config.port), |l| l.local_addr().map(|a| a.port()))
            .map_err(StartError::Bind)?;
        // -- WP AIVIS --
        let titles = server
            .tool_titles()
            .into_iter()
            .map(|(name, title, write)| (name, (title, write)))
            .collect();
        let shared = Arc::new(Shared {
            mcp: Mutex::new(Mcp {
                server,
                principals: BTreeMap::new(),
                sessions: BTreeMap::new(),
                uses: 0,
                revoked: std::collections::BTreeSet::new(), // -- WP MCP2c --
            }),
            auth: Mutex::new(Auth {
                clients,
                pairing: Pairing::default(),
            }),
            paired: Condvar::new(),
            listener,
            limits: config.limits,
            port,
            started: Instant::now(),
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| u64::try_from(d.as_millis()).ok())
                .unwrap_or(0),
            stopping: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            parked: Mutex::new(Vec::new()),
            kicked: Mutex::new(false),
            kick: Condvar::new(),
            work: Mutex::default(), // -- WP AIVIS --
            titles,
            pending_user: Mutex::default(),
        });
        // -- WP MCP4 -- before anything is served: without it confirmations
        // would never lapse and timed previews never be put back
        let ticking = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("mcp-tick".into())
                .spawn(move || tick_loop(&shared))
                .map_err(StartError::Bind)?
        };
        let mut addrs = Vec::new();
        let mut accepting = Vec::new();
        for l in listeners {
            if let Ok(a) = l.local_addr() {
                addrs.push(a);
            }
            let shared = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name("mcp-accept".into())
                .spawn(move || accept_loop(&shared, &l));
            match spawned {
                Ok(h) => accepting.push(h),
                Err(e) => log::error!("mcp: no thread to accept on: {e}"),
            }
        }
        Ok(Self {
            shared,
            addrs,
            accepting: Mutex::new(accepting),
            ticking: Mutex::new(Some(ticking)),
        })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.shared.port
    }

    /// The addresses it listens on: loopback only.
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    /// Run `f` on the MCP server (the active device, the activity log). It
    /// waits for a tool call in progress (the module's Threads).
    pub fn with_server<R>(&self, f: impl FnOnce(&mut Server) -> R) -> R {
        self.shared.with_mcp(|m| f(&mut m.server))
    }

    // -- WP MCP4 --
    /// The confirmations waiting for the user, oldest first:
    /// `Confirmation::to_json` with `title` (the tool's) and `remaining_ms`.
    /// Waits as [`HttpServer::with_server`].
    pub fn confirmations(&self) -> Vec<Value> {
        let now_ms = self.shared.now_ms();
        self.shared.with_mcp(|m| {
            let mut rows = m.server.waiting_confirmations();
            for r in &mut rows {
                let left = r
                    .get("expires_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_sub(now_ms);
                let tool = r.get("tool").and_then(Value::as_str).unwrap_or("");
                let title = m.server.tool_title(tool);
                if let Some(o) = r.as_object_mut() {
                    o.insert("remaining_ms".into(), json!(left));
                    o.insert("title".into(), json!(title));
                }
            }
            rows
        })
    }

    /// The user's answer to confirmation `id`. Allow runs the call on this
    /// thread, then its parked connection is answered. Waits as
    /// [`HttpServer::with_server`].
    pub fn answer_confirmation(&self, id: u64, allow: bool) -> Result<(), AnswerError> {
        let now_ms = self.shared.now_ms();
        let answered = self
            .shared
            .with_mcp(|m| m.server.answer_confirmation(id, allow, now_ms));
        if answered.is_ok() {
            self.shared.emit(&[Event::Activity]);
        }
        answered
    }

    // -- WP MCP5 --
    /// The agents' measurement requests the app has to take, oldest first
    /// (`Server::measure_requests`). Waits as [`HttpServer::with_server`].
    pub fn measure_requests(&self) -> Vec<Value> {
        self.shared.with_mcp(|m| m.server.measure_requests())
    }

    /// The app takes request `id`: its ticket (`tools::measure::begin`
    /// starts it), `None` when it no longer waits. Waits as
    /// [`HttpServer::with_server`].
    pub fn measure_take(&self, id: u64) -> Option<Value> {
        self.shared.with_mcp(|m| m.server.measure_take(id))
    }

    /// How request `id` went (`Server::measure_report`). Waits as
    /// [`HttpServer::with_server`].
    pub fn measure_report(&self, id: u64, ok: bool, note: &str) -> bool {
        let done = self
            .shared
            .with_mcp(|m| m.server.measure_report(id, ok, note));
        self.shared.emit(&[Event::Activity]);
        done
    }

    // -- WP FEAT2 --
    /// The devices the app remembers the user opened
    /// (`Server::set_known_devices`). Waits as [`HttpServer::with_server`].
    pub fn set_known_devices(&self, rows: &Value) {
        self.shared.with_mcp(|m| m.server.set_known_devices(rows));
    }

    /// The agents' requests to open or close a device, oldest first
    /// (`Server::device_requests`). Waits as [`HttpServer::with_server`].
    pub fn device_requests(&self) -> Vec<Value> {
        self.shared.with_mcp(|m| m.server.device_requests())
    }

    /// The app takes request `id`: its ticket, `None` when it no longer
    /// waits. Waits as [`HttpServer::with_server`].
    pub fn device_take(&self, id: u64) -> Option<Value> {
        self.shared.with_mcp(|m| m.server.device_take(id))
    }

    /// How request `id` went (`Server::device_report`). Waits as
    /// [`HttpServer::with_server`].
    pub fn device_report(&self, id: u64, ok: bool, note: &str) -> bool {
        let done = self
            .shared
            .with_mcp(|m| m.server.device_report(id, ok, note));
        self.shared.emit(&[Event::Activity]);
        done
    }

    /// The newest `n` activity entries, oldest first:
    /// `ActivityEntry::to_json` with `title` (the tool's) and `at_unix_ms`.
    /// Waits as [`HttpServer::with_server`].
    pub fn activity(&self, n: usize) -> Vec<Value> {
        let started = self.shared.started_unix_ms;
        self.shared.with_mcp(|m| {
            m.server
                .activity(n)
                .iter()
                .map(|e| {
                    let mut v = e.to_json();
                    let title = m.server.tool_title(&e.tool);
                    if let Some(o) = v.as_object_mut() {
                        o.insert("title".into(), json!(title));
                        o.insert("at_unix_ms".into(), json!(started.saturating_add(e.at_ms)));
                    }
                    v
                })
                .collect()
        })
    }

    /// A client the user adds by hand: its token, shown once.
    pub fn add_client(&self, name: &str) -> Result<Minted, AuthError> {
        let minted = lock(&self.shared.auth).clients.mint(name, unix_s())?;
        self.shared.emit(&[Event::ClientsChanged]);
        Ok(minted)
    }

    /// The paired clients.
    pub fn clients(&self) -> Vec<PairedClient> {
        lock(&self.shared.auth).clients.list().to_vec()
    }

    /// Remove client `id`: its token stops working at once, its sessions end.
    pub fn revoke(&self, id: u64) -> bool {
        let removed = lock(&self.shared.auth).clients.revoke(id);
        self.shared.with_mcp(|mcp| {
            mcp.revoked.insert(id); // -- WP MCP2c --
            if let Some(cid) = mcp.principals.remove(&id) {
                mcp.server.disconnect(cid);
            }
            mcp.sessions.retain(|_, s| s.owner != id);
        });
        if removed {
            self.shared.emit(&[Event::ClientsChanged]);
        }
        removed
    }

    /// The user's answer to pairing request `id`. False when it is not
    /// waiting (answered, expired, never was).
    pub fn pairing_answer(&self, id: u64, allow: bool) -> Result<bool, AuthError> {
        let now_ms = self.shared.now_ms();
        let answered = {
            let mut guard = lock(&self.shared.auth);
            let auth = &mut *guard;
            auth.pairing
                .answer(id, allow, now_ms, unix_s(), &mut auth.clients)?
        };
        self.shared.paired.notify_all();
        if answered && allow {
            self.shared.emit(&[Event::ClientsChanged]);
        }
        Ok(answered)
    }

    /// The pairing request waiting for the user: `(id, name, code)`.
    pub fn pairing(&self) -> Option<(u64, String, String)> {
        let now_ms = self.shared.now_ms();
        let mut guard = lock(&self.shared.auth);
        let auth = &mut *guard;
        auth.pairing.waiting(now_ms, &mut auth.clients)
    }

    /// Pause or resume the agents' tool calls.
    pub fn set_paused(&self, paused: bool) {
        // -- WP AIVIS -- the agents are told; never waits for a call
        if self.shared.paused.swap(paused, Ordering::SeqCst) != paused {
            lock(&self.shared.pending_user).push(if paused {
                ("paused", "paused AI agents")
            } else {
                ("resumed", "let AI agents go on (Resume)")
            });
            self.shared.emit(&[Event::Work]);
        }
    }

    // -- WP AIVIS --
    /// What the agents have running, for the app's working strip: `{calls,
    /// last, jobs, guards}` ([`Server::work`]). Never waits for a call.
    pub fn work(&self) -> Value {
        let w = lock(&self.shared.work);
        let of = |key: &str| w.jobs.get(key).cloned().unwrap_or_else(|| json!([]));
        json!({
            "calls": w.calls.values().collect::<Vec<_>>(),
            "last": w.last,
            "jobs": of("jobs"),
            "guards": of("guards"),
        })
    }

    /// Whether the agents are paused.
    pub fn paused(&self) -> bool {
        self.shared.paused.load(Ordering::SeqCst)
    }

    /// `{running, port, addresses, paused, clients, pairing}` for the app.
    pub fn status(&self) -> Value {
        let clients: Vec<Value> = self.clients().iter().map(PairedClient::to_json).collect();
        let pairing = self.pairing().map_or(
            Value::Null,
            |(id, name, code)| json!({"id": id, "name": name, "code": code}),
        );
        let addresses: Vec<String> = self.addrs.iter().map(ToString::to_string).collect();
        json!({
            "running": true,
            "port": self.port(),
            "addresses": addresses,
            "paused": self.paused(),
            "clients": clients,
            "pairing": pairing,
        })
    }

    /// Stop listening. A request in progress finishes; later ones on open
    /// connections get 503. The clients' last use is written down. A second
    /// call does nothing. -- WP MCP4 -- The confirmations waiting end with
    /// nothing changed, the held calls get that answer, and every agent's
    /// timed preview is put back (which may write to a device).
    pub fn stop(&self) {
        if self.shared.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        self.shared.paired.notify_all();
        for a in &self.addrs {
            // Wake the accepting thread; it sees `stopping` and ends.
            let _ = TcpStream::connect_timeout(a, Duration::from_millis(500));
        }
        let accepting = std::mem::take(&mut *lock(&self.accepting));
        for h in accepting {
            let _ = h.join();
        }
        // -- WP MCP4 --
        self.shared.wake();
        if let Some(h) = lock(&self.ticking).take()
            // (an event's listener that stops the server, on that thread)
            && h.thread().id() != std::thread::current().id()
        {
            let _ = h.join();
        }
        let now_ms = self.shared.now_ms();
        self.shared.with_mcp(|m| m.server.stop_agents(now_ms));
        for (mut stream, response) in sweep(&self.shared) {
            let _ = stream.set_write_timeout(Some(TICK));
            let _ = write_response(&mut stream, &response);
        }
        if let Err(e) = lock(&self.shared.auth).clients.save() {
            log::warn!("mcp: the clients' last use was not written: {e}");
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Counts a connection while it is served.
struct Counted(Arc<Shared>);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(shared: &Arc<Shared>, listener: &TcpListener) {
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        // Bound to loopback, so this holds already; a peer from elsewhere
        // would mean the bind is not what it says, and gets nothing.
        if !stream
            .peer_addr()
            .is_ok_and(|p| p.ip().to_canonical().is_loopback())
        {
            continue;
        }
        let _ = stream.set_read_timeout(Some(shared.limits.io_timeout));
        let _ = stream.set_write_timeout(Some(shared.limits.io_timeout));
        if shared.connections.fetch_add(1, Ordering::SeqCst) >= shared.limits.max_connections {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
            let mut s = stream;
            let _ = write_response(
                &mut s,
                &Response::error(503, "too many connections at once"),
            );
            continue;
        }
        let counted = Counted(Arc::clone(shared));
        let spawned = std::thread::Builder::new()
            .name("mcp-conn".into())
            .spawn(move || {
                serve(&counted.0, stream);
                drop(counted);
            });
        if let Err(e) = spawned {
            log::error!("mcp: no thread for a connection: {e}");
        }
    }
}

// ── Requests and replies ────────────────────────────────────────────────

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// The one value of header `name`: `Ok(None)` when absent, `Err` when
    /// repeated.
    fn header(&self, name: &str) -> Result<Option<&str>, ()> {
        let mut found = None;
        for (n, v) in &self.headers {
            if n.eq_ignore_ascii_case(name) {
                if found.is_some() {
                    return Err(());
                }
                found = Some(v.as_str());
            }
        }
        Ok(found)
    }

    fn has(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
    }
}

struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Option<Value>,
}

impl Response {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Some(body),
        }
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: None,
        }
    }

    /// A refusal: a JSON-RPC error with no id (the transport's words).
    fn error(status: u16, message: &str) -> Self {
        let code = if status == 400 {
            INVALID_REQUEST
        } else {
            -32000
        };
        Self::json(
            status,
            protocol::error_reply(&Value::Null, &RpcError::new(code, message)),
        )
    }

    fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn write_response(stream: &mut TcpStream, r: &Response) -> io::Result<()> {
    let body = r.body.as_ref().map(Value::to_string).unwrap_or_default();
    let mut head = format!("HTTP/1.1 {} {}\r\n", r.status, reason(r.status));
    if r.body.is_some() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str("Cache-Control: no-store\r\nConnection: close\r\n");
    for (n, v) in &r.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

/// Read one request, or the reply that refuses it.
fn read_request(stream: &mut TcpStream, limits: &HttpLimits) -> Result<Request, Response> {
    let deadline = Instant::now().checked_add(limits.io_timeout);
    let late = || deadline.is_some_and(|d| Instant::now() > d);
    let timeout = || Response::error(408, "the request took too long to arrive");
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i.saturating_add(4);
        }
        if buf.len() > limits.head_bytes {
            return Err(Response::error(431, "the request's headers are too large"));
        }
        if late() {
            return Err(timeout());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(Response::error(400, "the request ended early")),
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(timeout());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(Response::error(400, "the request could not be read")),
        }
    };
    if head_end > limits.head_bytes {
        return Err(Response::error(431, "the request's headers are too large"));
    }
    let mut slots = [httparse::EMPTY_HEADER; 48];
    let mut parsed = httparse::Request::new(&mut slots);
    match parsed.parse(buf.get(..head_end).unwrap_or_default()) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) | Err(_) => {
            return Err(Response::error(400, "not an HTTP request"));
        }
    }
    let mut headers = Vec::with_capacity(parsed.headers.len());
    for h in parsed.headers.iter() {
        let Ok(v) = std::str::from_utf8(h.value) else {
            return Err(Response::error(400, "a header is not text"));
        };
        headers.push((h.name.to_owned(), v.trim().to_owned()));
    }
    let mut request = Request {
        method: parsed.method.unwrap_or("").to_owned(),
        path: parsed.path.unwrap_or("").to_owned(),
        headers,
        body: Vec::new(),
    };
    if request.has("Transfer-Encoding") {
        return Err(Response::error(411, "send the body with a Content-Length"));
    }
    let length = match request.header("Content-Length") {
        Err(()) => return Err(Response::error(400, "two Content-Length headers")),
        Ok(None) => 0,
        Ok(Some(v)) => match v.parse::<usize>() {
            Ok(n) => n,
            Err(_) => return Err(Response::error(400, "a bad Content-Length")),
        },
    };
    if request.method == "POST" && request.header("Content-Length").ok().flatten().is_none() {
        return Err(Response::error(411, "send the body with a Content-Length"));
    }
    if length > limits.body_bytes {
        return Err(Response::error(413, "the request's body is too large"));
    }
    let mut body = buf.split_off(head_end);
    while body.len() < length {
        if late() {
            return Err(timeout());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(Response::error(400, "the request ended early")),
            Ok(n) => body.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(timeout());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(Response::error(400, "the request could not be read")),
        }
    }
    body.truncate(length);
    request.body = body;
    Ok(request)
}

/// The host of a `Host` header and its port, if it names one.
fn split_host(authority: &str) -> Option<(String, Option<u16>)> {
    let authority = authority.trim();
    let (host, rest) = if let Some(inner) = authority.strip_prefix('[') {
        let (h, r) = inner.split_once(']')?;
        (h, r)
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h, authority.get(h.len()..).unwrap_or(p)),
            None => (authority, ""),
        }
    };
    let port = match rest {
        "" => None,
        r => Some(r.strip_prefix(':')?.parse::<u16>().ok()?),
    };
    Some((host.to_ascii_lowercase(), port))
}

/// Whether `Host` names this computer's loopback: `127.0.0.1`, `localhost`
/// or `[::1]`, with any port. The name is what stops DNS rebinding (a page's
/// request carries the page's host name); the port is free because an SSH
/// tunnel (`ssh -L 50000:127.0.0.1:47860`, docs/mcp-ssh-forwarding.md)
/// brings the request with the tunnel's own.
fn host_ok(request: &Request) -> bool {
    let Ok(Some(host)) = request.header("Host") else {
        return false;
    };
    let Some((name, _port)) = split_host(host) else {
        return false;
    };
    matches!(name.as_str(), "127.0.0.1" | "localhost" | "::1")
}

/// Whether the body is declared JSON.
fn is_json(request: &Request) -> bool {
    request
        .header("Content-Type")
        .ok()
        .flatten()
        .is_some_and(|t| {
            t.split(';')
                .next()
                .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
        })
}

/// What a request got (-- WP MCP4 --).
enum Answer {
    Now(Response),
    /// Held for the confirmation with this id.
    Held(u64),
}

impl From<Response> for Answer {
    fn from(r: Response) -> Self {
        Self::Now(r)
    }
}

fn serve(shared: &Arc<Shared>, mut stream: TcpStream) {
    let answer = match read_request(&mut stream, &shared.limits) {
        Ok(request) => answer(shared, &request),
        Err(refusal) => refusal.into(),
    };
    match answer {
        Answer::Now(response) => deliver(stream, &response),
        // -- WP MCP4 -- never waited for here
        Answer::Held(id) => {
            lock(&shared.parked).push(Parked {
                id,
                stream,
                since: Instant::now(),
            });
            // Answered meanwhile, or the server is stopping
            shared.wake();
            if shared.stopping.load(Ordering::SeqCst) {
                for (stream, response) in sweep(shared) {
                    deliver(stream, &response);
                }
            }
        }
    }
}

fn deliver(mut stream: TcpStream, response: &Response) {
    if let Err(e) = write_response(&mut stream, response) {
        log::debug!("mcp: a reply was not delivered: {e}");
    }
    drain(&mut stream);
}

// -- WP MCP4 --
/// The ticking thread: the confirmations' and the timed previews' clocks, and
/// the parked connections.
fn tick_loop(shared: &Arc<Shared>) {
    while !shared.stopping.load(Ordering::SeqCst) {
        {
            let mut kicked = lock(&shared.kicked);
            if !*kicked {
                kicked = shared
                    .kick
                    .wait_timeout(kicked, TICK)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            *kicked = false;
        }
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let now_ms = shared.now_ms();
        shared.with_mcp(|m| m.server.tick(now_ms));
        for (stream, response) in sweep(shared) {
            // A slow reader must not hold the clocks up.
            let spawned = std::thread::Builder::new()
                .name("mcp-conn".into())
                .spawn(move || deliver(stream, &response));
            if let Err(e) = spawned {
                log::error!("mcp: no thread for a held reply: {e}");
            }
        }
    }
}

/// Take the parked connections that are due, with their replies: the
/// confirmation has ended, or the hold is over. One the agent closed is
/// dropped, and its confirmation ends.
fn sweep(shared: &Shared) -> Vec<(TcpStream, Response)> {
    shared.with_mcp(|m| {
        let mut parked = lock(&shared.parked);
        let mut due = Vec::new();
        for p in std::mem::take(&mut *parked) {
            if gone(&p.stream) {
                m.server.abandon_held(p.id);
                continue;
            }
            let reply = m.server.resume_value(p.id).or_else(|| {
                (p.since.elapsed() >= shared.limits.hold)
                    .then(|| m.server.release_held(p.id))
                    .flatten()
            });
            match reply {
                Some(r) => due.push((p.stream, Response::json(200, r))),
                None => parked.push(p),
            }
        }
        due
    })
}

/// Whether the peer has closed `stream`.
fn gone(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let mut b = [0u8; 1];
    let gone = match stream.peek(&mut b) {
        Ok(n) => n == 0,
        Err(e) => !matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        ),
    };
    gone || stream.set_nonblocking(false).is_err()
}

/// Read what the peer still sends, for at most a second, before closing: closing with
/// unread bytes resets the connection, which can lose the reply on its way
/// (a refused body that was not read, say).
fn drain(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let until = Instant::now().checked_add(Duration::from_secs(1));
    let mut sink = [0u8; 4096];
    let mut left: usize = 1024 * 1024;
    while left > 0 && until.is_some_and(|u| Instant::now() < u) {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => left = left.saturating_sub(n),
        }
    }
}

fn answer(shared: &Arc<Shared>, request: &Request) -> Answer {
    // Before anything else, so a page learns as little as it can.
    if request.has("Origin") {
        return Response::error(403, "requests from web pages are refused").into();
    }
    if !host_ok(request) {
        return Response::error(
            403,
            "the Host header is not this computer's loopback address",
        )
        .into();
    }
    if shared.stopping.load(Ordering::SeqCst) {
        return Response::error(503, STOPPED_TEXT).into();
    }
    let path = request.path.split('?').next().unwrap_or("");
    match (request.method.as_str(), path) {
        ("POST", MCP_PATH | PAIR_PATH | PAIR_WAIT_PATH) if !is_json(request) => {
            Response::error(415, "the body must be application/json").into()
        }
        ("POST", MCP_PATH) => post_mcp(shared, request),
        ("DELETE", MCP_PATH) => delete_session(shared, request).into(),
        (_, MCP_PATH) => Response::error(405, "POST a JSON-RPC message")
            .with("Allow", "POST, DELETE")
            .into(),
        ("POST", PAIR_PATH) => pair(shared, request).into(),
        ("POST", PAIR_WAIT_PATH) => pair_wait(shared, request).into(),
        (_, PAIR_PATH | PAIR_WAIT_PATH) => Response::error(405, "POST a JSON object")
            .with("Allow", "POST")
            .into(),
        _ => Response::error(404, "no such path: the MCP endpoint is /mcp").into(),
    }
}

/// The paired client the request's token belongs to.
fn authorise(shared: &Shared, request: &Request) -> Option<Owner> {
    let presented = request
        .header("Authorization")
        .ok()
        .flatten()
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim)
        .unwrap_or("");
    // Verified even when empty, so no token costs what a wrong one does.
    let mut auth = lock(&shared.auth);
    let id = auth.clients.verify(presented, unix_s())?;
    // -- WP MCP4 -- the name the user paired it under
    let name = auth
        .clients
        .list()
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    Some(Owner { id, name })
}

/// A paired client (-- WP MCP4 --).
struct Owner {
    id: u64,
    name: String,
}

fn unauthorised() -> Response {
    Response::error(401, UNAUTHORISED_TEXT)
        .with("WWW-Authenticate", "Bearer realm=\"DSPi Console\"")
}

fn mismatch(id: &Value, what: &str) -> Response {
    Response::json(
        400,
        protocol::error_reply(
            id,
            &RpcError::new(HEADER_MISMATCH, format!("header mismatch: {what}")),
        ),
    )
}

/// The server's client for paired client `owner`, made on first use;
/// `None` once `owner` was revoked (-- WP MCP2c --: a request authorised
/// before the revoke and served after it).
fn client_for(mcp: &mut Mcp, owner: &Owner) -> Option<ClientId> {
    if mcp.revoked.contains(&owner.id) {
        return None;
    }
    if let Some(c) = mcp.principals.get(&owner.id) {
        return Some(*c);
    }
    let c = mcp.server.connect_as(&owner.name);
    mcp.principals.insert(owner.id, c);
    Some(c)
}

// -- WP MCP2c --
/// Whether a revoked paired client `owner` would become a principal again
/// (tests: review RB's revoke race). False is right.
#[cfg(test)]
pub(crate) fn tests_wp_mcp2c_revoked_becomes_a_principal() -> bool {
    let mut mcp = Mcp {
        server: Server::new(dspi_core::CoreSet::new(), crate::ServerConfig::default()),
        principals: BTreeMap::new(),
        sessions: BTreeMap::new(),
        uses: 0,
        revoked: std::collections::BTreeSet::from([5]),
    };
    let owner = |id| Owner {
        id,
        name: String::new(),
    };
    let again = client_for(&mut mcp, &owner(5)).is_some() || mcp.principals.contains_key(&5);
    // Another client still is one.
    again || client_for(&mut mcp, &owner(6)).is_none()
}

fn new_session_id() -> Option<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).ok()?;
    Some(HEXLOWER.encode(&b))
}

fn post_mcp(shared: &Shared, request: &Request) -> Answer {
    let Some(owner) = authorise(shared, request) else {
        return unauthorised().into();
    };
    let msg: Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(_) => {
            return Response::json(
                400,
                protocol::error_reply(&Value::Null, &RpcError::new(PARSE_ERROR, "not JSON")),
            )
            .into();
        }
    };
    let Some(obj) = msg.as_object() else {
        // The server says why (a batch, not an object).
        return relay_reply(shared, &owner, msg, 400).into();
    };
    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    let method = obj.get("method").and_then(Value::as_str);
    let params = obj.get("params").and_then(Value::as_object);
    let meta = params
        .and_then(|p| p.get("_meta"))
        .and_then(Value::as_object);
    let version = meta
        .and_then(|m| m.get(META_VERSION))
        .map(|v| v.as_str().unwrap_or(""));
    let Some(method) = method else {
        // A response to the server, which sends no requests.
        return Response::empty(202).into();
    };
    // -- WP MCP6 -- only a legacy `initialize`: with the modern `_meta` it is a
    // method 2026-07-28 removed, which the server answers (404 below)
    if method == "initialize" && version.is_none() {
        return initialize(shared, &owner, msg).into();
    }
    if let Some(version) = version {
        let header = |n: &str| request.header(n).ok().flatten();
        // -- WP MCP6 -- a header that names another version than the body is
        // a mismatch, before the version is judged (the conformance suite's
        // sep-2575-http-server-header-mismatch-400)
        let said = header("MCP-Protocol-Version");
        if said.is_some_and(|h| h != version) {
            return mismatch(
                &id,
                "MCP-Protocol-Version must equal the _meta protocol version",
            )
            .into();
        }
        // Modern: the headers must say what the body says.
        if MODERN_VERSIONS.contains(&version) {
            if said != Some(version) {
                return mismatch(
                    &id,
                    "MCP-Protocol-Version must equal the _meta protocol version",
                )
                .into();
            }
            if header("Mcp-Method") != Some(method) {
                return mismatch(&id, "Mcp-Method must equal the method").into();
            }
            let named = match method {
                "tools/call" | "prompts/get" => params.and_then(|p| p.get("name")),
                "resources/read" => params.and_then(|p| p.get("uri")),
                _ => None,
            };
            if let Some(named) = named
                && header("Mcp-Name") != named.as_str()
            {
                return mismatch(&id, "Mcp-Name must equal the name or uri").into();
            }
        }
        let caps_missing = !meta
            .and_then(|m| m.get(META_CAPABILITIES))
            .is_some_and(Value::is_object);
        let is_call = method == "tools/call";
        return forward(shared, &owner, msg, is_call, |code| match code {
            UNSUPPORTED_VERSION => 400,
            INVALID_PARAMS if caps_missing => 400,
            METHOD_NOT_FOUND => 404,
            _ => 200,
        });
    }
    // -- WP MCP6 -- the header says 2026-07-28 but the body has no `_meta`
    // version: the request is modern and lacks it, -32602 (the conformance
    // suite's sep-2575-request-meta-invalid-*), not an unsupported version
    if let Ok(Some(v)) = request.header("MCP-Protocol-Version")
        && MODERN_VERSIONS.contains(&v)
    {
        let why = format!("a {v} request carries `_meta` with `{META_VERSION}`");
        return Response::json(
            400,
            protocol::error_reply(&id, &RpcError::new(INVALID_PARAMS, why)),
        )
        .into();
    }
    // Legacy: a session from `initialize`.
    if let Ok(Some(v)) = request.header("MCP-Protocol-Version")
        && !LEGACY_VERSIONS.contains(&v)
    {
        return Response::json(
            400,
            protocol::error_reply(&id, &RpcError::unsupported_version(v)),
        )
        .into();
    }
    let session = match request.header("Mcp-Session-Id") {
        Ok(Some(s)) => s,
        Ok(None) if method == "ping" => "",
        _ => {
            return Response::json(
                400,
                protocol::error_reply(
                    &id,
                    &RpcError::new(
                        INVALID_REQUEST,
                        "send initialize first, then its Mcp-Session-Id with every request",
                    ),
                ),
            )
            .into();
        }
    };
    if !session.is_empty() {
        let mut mcp = lock(&shared.mcp);
        mcp.uses = mcp.uses.saturating_add(1);
        let used = mcp.uses;
        match mcp.sessions.get_mut(session) {
            Some(s) if s.owner == owner.id => s.used = used,
            _ => return Response::error(404, "no such session: send initialize again").into(),
        }
    }
    let is_call = method == "tools/call";
    forward(shared, &owner, msg, is_call, |_| 200)
}

// -- WP AIVIS --
/// The reply to a `tools/call` while the user has paused the agents: not an
/// error, the call was not served and nothing was changed.
fn paused_call(msg: &Value) -> Response {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let modern = msg
        .pointer("/params/_meta")
        .and_then(|m| m.get(META_VERSION))
        .is_some();
    let mut result = Map::new();
    result.insert(
        "content".into(),
        json!([{"type": "text", "text": PAUSED_TEXT}]),
    );
    result.insert(
        "structuredContent".into(),
        json!({"status": "paused", "note": PAUSED_TEXT}),
    );
    result.insert("isError".into(), json!(false));
    if modern {
        result.insert("resultType".into(), json!("complete"));
    }
    Response::json(200, protocol::reply(&id, Value::Object(result)))
}

/// Hand `msg` to the server as `owner`'s client; the HTTP status of an error
/// reply comes from `status_of(code)`. `is_call`: it is a `tools/call`, which
/// the policy may hold (-- WP MCP4 --).
fn forward(
    shared: &Shared,
    owner: &Owner,
    msg: Value,
    is_call: bool,
    status_of: impl Fn(i64) -> u16,
) -> Answer {
    let now_ms = shared.now_ms();
    // -- WP AIVIS -- the app shows the call before it waits for the server
    let call = if is_call && !shared.paused.load(Ordering::SeqCst) {
        shared.call_began(&owner.name, &msg)
    } else {
        0
    };
    // Judged with the server locked: a call that waited for another's end
    // while the user pressed Pause or Stop does not run.
    let outcome = shared.with_mcp(|mcp| {
        if shared.stopping.load(Ordering::SeqCst) {
            return Err(Response::error(503, STOPPED_TEXT));
        }
        if is_call && shared.paused.load(Ordering::SeqCst) {
            return Err(paused_call(&msg)); // -- WP AIVIS --
        }
        let Some(client) = client_for(mcp, owner) else {
            return Err(unauthorised()); // -- WP MCP2c --
        };
        Ok(mcp.server.handle_held_value(client, now_ms, msg))
    });
    shared.call_ended(call); // -- WP AIVIS --
    let (reply, held) = match outcome {
        Ok(o) => o,
        Err(refusal) => return refusal.into(),
    };
    if is_call {
        shared.emit(&[Event::Activity]);
    }
    if let Some(id) = held {
        return Answer::Held(id);
    }
    match reply {
        None => Response::empty(202).into(),
        Some(r) => {
            let status = r
                .pointer("/error/code")
                .and_then(Value::as_i64)
                .map_or(200, status_of);
            Response::json(status, r).into()
        }
    }
}

fn relay_reply(shared: &Shared, owner: &Owner, msg: Value, status: u16) -> Response {
    let now_ms = shared.now_ms();
    let reply = shared.with_mcp(|mcp| {
        let client = client_for(mcp, owner)?; // -- WP MCP2c --
        Some(mcp.server.handle_value(client, now_ms, msg))
    });
    let Some(reply) = reply else {
        return unauthorised(); // -- WP MCP2c --
    };
    match reply {
        Some(r) => Response::json(status, r),
        None => Response::empty(202),
    }
}

fn initialize(shared: &Shared, owner: &Owner, msg: Value) -> Response {
    let now_ms = shared.now_ms();
    let mut mcp = lock(&shared.mcp);
    let Some(client) = client_for(&mut mcp, owner) else {
        return unauthorised(); // -- WP MCP2c --
    };
    let Some(reply) = mcp.server.handle_value(client, now_ms, msg) else {
        return Response::empty(202);
    };
    if reply.get("result").is_none() {
        return Response::json(200, reply);
    }
    let Some(session) = new_session_id() else {
        return Response::error(503, "the system gave no random numbers for a session");
    };
    // The newest few per client; the oldest goes.
    let cap = shared.limits.sessions_per_client.max(1);
    loop {
        let mine: Vec<(&String, u64)> = mcp
            .sessions
            .iter()
            .filter(|(_, s)| s.owner == owner.id)
            .map(|(k, s)| (k, s.used))
            .collect();
        if mine.len() < cap {
            break;
        }
        let Some(oldest) = mine
            .iter()
            .min_by_key(|(_, t)| *t)
            .map(|(k, _)| (*k).clone())
        else {
            break;
        };
        mcp.sessions.remove(&oldest);
    }
    mcp.uses = mcp.uses.saturating_add(1);
    let used = mcp.uses;
    mcp.sessions.insert(
        session.clone(),
        Session {
            owner: owner.id,
            used,
        },
    );
    Response::json(200, reply).with("Mcp-Session-Id", session)
}

fn delete_session(shared: &Shared, request: &Request) -> Response {
    let Some(owner) = authorise(shared, request) else {
        return unauthorised();
    };
    let Ok(Some(session)) = request.header("Mcp-Session-Id") else {
        return Response::error(400, "DELETE ends a session: send its Mcp-Session-Id");
    };
    let mut mcp = lock(&shared.mcp);
    match mcp.sessions.get(session) {
        Some(s) if s.owner == owner.id => {
            mcp.sessions.remove(session);
            Response::empty(204)
        }
        _ => Response::error(404, "no such session"),
    }
}

fn body_object(request: &Request) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

fn pair(shared: &Shared, request: &Request) -> Response {
    let Some(body) = body_object(request) else {
        return Response::error(400, "send {\"name\": ...}");
    };
    let name = body.get("name").and_then(Value::as_str).unwrap_or("");
    let now_ms = shared.now_ms();
    let asked = {
        let mut guard = lock(&shared.auth);
        let auth = &mut *guard;
        auth.pairing.request(name, now_ms, &mut auth.clients)
    };
    match asked {
        Ok(r) => {
            shared.emit(&[Event::PairingRequested {
                id: r.id,
                name: r.name.clone(),
                code: r.code.clone(),
            }]);
            Response::json(
                200,
                json!({"request": r.id, "code": r.code, "ticket": r.ticket, "expiresInMs": PAIRING_TTL_MS}),
            )
        }
        Err(e @ AuthError::Busy) => Response::error(409, &e.to_string()),
        Err(e @ (AuthError::TooMany | AuthError::Full)) => Response::error(429, &e.to_string()),
        Err(e) => Response::error(503, &e.to_string()),
    }
}

fn pair_wait(shared: &Shared, request: &Request) -> Response {
    let Some(body) = body_object(request) else {
        return Response::error(400, "send {\"ticket\": ...}");
    };
    let ticket = body
        .get("ticket")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let until = Instant::now().checked_add(shared.limits.pair_wait);
    let mut guard = lock(&shared.auth);
    loop {
        let now_ms = shared.now_ms();
        let auth = &mut *guard;
        let (poll, ended) = auth.pairing.poll(&ticket, now_ms, &mut auth.clients);
        let reply = match poll {
            PairPoll::Waiting => {
                let left = until.and_then(|u| u.checked_duration_since(Instant::now()));
                match left {
                    Some(left) if !left.is_zero() && !shared.stopping.load(Ordering::SeqCst) => {
                        let wait = left.min(Duration::from_secs(1));
                        guard = shared
                            .paired
                            .wait_timeout(guard, wait)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                        continue;
                    }
                    _ => Response::json(200, json!({"status": "pending"})),
                }
            }
            PairPoll::Allowed(m) => Response::json(
                200,
                json!({"status": "allowed", "token": m.token, "client": m.id}),
            ),
            PairPoll::Denied => Response::json(403, json!({"status": "denied"})),
            PairPoll::Gone => Response::json(410, json!({"status": "gone"})),
        };
        drop(guard);
        if let Some((id, outcome)) = ended {
            shared.emit(&[Event::PairingEnded { id, outcome }]);
        }
        return reply;
    }
}
