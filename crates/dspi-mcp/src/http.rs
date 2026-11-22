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
//! it uses. While the app has paused the agents, `tools/call` answers with a
//! tool error and changes nothing.
//!
//! **Threads.** One accepting thread per address and one per connection.
//! Calls are serialised on the server's mutex; a tool call runs on its
//! connection's thread. Events ([`Event`]) reach the app's listener on these
//! threads, with no lock held.

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

/// The tool error a paused server answers `tools/call` with.
pub const PAUSED_TEXT: &str = "DSPi Console: the user has paused AI agents. Nothing was changed.";

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
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_connections: 8,
            head_bytes: 16 * 1024,
            body_bytes: 4 * 1024 * 1024,
            io_timeout: Duration::from_secs(10),
            pair_wait: Duration::from_secs(25),
            sessions_per_client: 8,
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
    stopping: AtomicBool,
    paused: AtomicBool,
    connections: AtomicUsize,
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
}

/// The running listener. Dropping it stops it.
pub struct HttpServer {
    shared: Arc<Shared>,
    addrs: Vec<SocketAddr>,
    accepting: Mutex<Vec<JoinHandle<()>>>,
}

impl std::fmt::Debug for HttpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpServer")
            .field("addrs", &self.addrs)
            .finish_non_exhaustive()
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
        let v4 = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), config.port).map_err(StartError::Bind)?;
        let port = v4.local_addr().map_err(StartError::Bind)?.port();
        let mut listeners = vec![v4];
        match bind(IpAddr::V6(Ipv6Addr::LOCALHOST), port) {
            Ok(v6) => listeners.push(v6),
            Err(e) => log::info!("mcp: no IPv6 loopback ({e}), listening on 127.0.0.1 only"),
        }
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
            stopping: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
        });
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

    /// Run `f` on the MCP server (the active device, the activity log).
    pub fn with_server<R>(&self, f: impl FnOnce(&mut Server) -> R) -> R {
        f(&mut lock(&self.shared.mcp).server)
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
        {
            let mut mcp = lock(&self.shared.mcp);
            mcp.revoked.insert(id); // -- WP MCP2c --
            if let Some(cid) = mcp.principals.remove(&id) {
                mcp.server.disconnect(cid);
            }
            mcp.sessions.retain(|_, s| s.owner != id);
        }
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
        self.shared.paused.store(paused, Ordering::SeqCst);
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
    /// call does nothing.
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
                let mut stream = stream;
                serve(&counted.0, &mut stream);
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

fn serve(shared: &Arc<Shared>, stream: &mut TcpStream) {
    let response = match read_request(stream, &shared.limits) {
        Ok(request) => answer(shared, &request),
        Err(refusal) => refusal,
    };
    if let Err(e) = write_response(stream, &response) {
        log::debug!("mcp: a reply was not delivered: {e}");
    }
    drain(stream);
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

fn answer(shared: &Arc<Shared>, request: &Request) -> Response {
    // Before anything else, so a page learns as little as it can.
    if request.has("Origin") {
        return Response::error(403, "requests from web pages are refused");
    }
    if !host_ok(request) {
        return Response::error(
            403,
            "the Host header is not this computer's loopback address",
        );
    }
    if shared.stopping.load(Ordering::SeqCst) {
        return Response::error(503, "AI agents have been turned off in DSPi Console");
    }
    let path = request.path.split('?').next().unwrap_or("");
    match (request.method.as_str(), path) {
        ("POST", MCP_PATH | PAIR_PATH | PAIR_WAIT_PATH) if !is_json(request) => {
            Response::error(415, "the body must be application/json")
        }
        ("POST", MCP_PATH) => post_mcp(shared, request),
        ("DELETE", MCP_PATH) => delete_session(shared, request),
        (_, MCP_PATH) => {
            Response::error(405, "POST a JSON-RPC message").with("Allow", "POST, DELETE")
        }
        ("POST", PAIR_PATH) => pair(shared, request),
        ("POST", PAIR_WAIT_PATH) => pair_wait(shared, request),
        (_, PAIR_PATH | PAIR_WAIT_PATH) => {
            Response::error(405, "POST a JSON object").with("Allow", "POST")
        }
        _ => Response::error(404, "no such path: the MCP endpoint is /mcp"),
    }
}

/// The paired client the request's token belongs to.
fn authorise(shared: &Shared, request: &Request) -> Option<u64> {
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
    lock(&shared.auth).clients.verify(presented, unix_s())
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
fn client_for(mcp: &mut Mcp, owner: u64) -> Option<ClientId> {
    if mcp.revoked.contains(&owner) {
        return None;
    }
    if let Some(c) = mcp.principals.get(&owner) {
        return Some(*c);
    }
    let c = mcp.server.connect();
    mcp.principals.insert(owner, c);
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
    let again = client_for(&mut mcp, 5).is_some() || mcp.principals.contains_key(&5);
    // Another client still is one.
    again || client_for(&mut mcp, 6).is_none()
}

fn new_session_id() -> Option<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).ok()?;
    Some(HEXLOWER.encode(&b))
}

fn post_mcp(shared: &Shared, request: &Request) -> Response {
    let Some(owner) = authorise(shared, request) else {
        return unauthorised();
    };
    let msg: Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(_) => {
            return Response::json(
                400,
                protocol::error_reply(&Value::Null, &RpcError::new(PARSE_ERROR, "not JSON")),
            );
        }
    };
    let Some(obj) = msg.as_object() else {
        // The server says why (a batch, not an object).
        return relay_reply(shared, owner, msg, 400);
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
        return Response::empty(202);
    };
    if method == "initialize" {
        return initialize(shared, owner, msg);
    }
    if let Some(version) = version {
        // Modern: the headers must say what the body says.
        if MODERN_VERSIONS.contains(&version) {
            let header = |n: &str| request.header(n).ok().flatten();
            if header("MCP-Protocol-Version") != Some(version) {
                return mismatch(
                    &id,
                    "MCP-Protocol-Version must equal the _meta protocol version",
                );
            }
            if header("Mcp-Method") != Some(method) {
                return mismatch(&id, "Mcp-Method must equal the method");
            }
            let named = match method {
                "tools/call" | "prompts/get" => params.and_then(|p| p.get("name")),
                "resources/read" => params.and_then(|p| p.get("uri")),
                _ => None,
            };
            if let Some(named) = named
                && header("Mcp-Name") != named.as_str()
            {
                return mismatch(&id, "Mcp-Name must equal the name or uri");
            }
        }
        let caps_missing = !meta
            .and_then(|m| m.get(META_CAPABILITIES))
            .is_some_and(Value::is_object);
        let is_call = method == "tools/call";
        return forward(shared, owner, msg, is_call, |code| match code {
            UNSUPPORTED_VERSION => 400,
            INVALID_PARAMS if caps_missing => 400,
            METHOD_NOT_FOUND => 404,
            _ => 200,
        });
    }
    // Legacy: a session from `initialize`.
    if let Ok(Some(v)) = request.header("MCP-Protocol-Version")
        && !LEGACY_VERSIONS.contains(&v)
    {
        return Response::json(
            400,
            protocol::error_reply(&id, &RpcError::unsupported_version(v)),
        );
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
            );
        }
    };
    if !session.is_empty() {
        let mut mcp = lock(&shared.mcp);
        mcp.uses = mcp.uses.saturating_add(1);
        let used = mcp.uses;
        match mcp.sessions.get_mut(session) {
            Some(s) if s.owner == owner => s.used = used,
            _ => return Response::error(404, "no such session: send initialize again"),
        }
    }
    let is_call = method == "tools/call";
    forward(shared, owner, msg, is_call, |_| 200)
}

/// Hand `msg` to the server as `owner`'s client; the HTTP status of an error
/// reply comes from `status_of(code)`. `is_call`: it is a `tools/call`.
fn forward(
    shared: &Shared,
    owner: u64,
    msg: Value,
    is_call: bool,
    status_of: impl Fn(i64) -> u16,
) -> Response {
    if is_call && shared.paused.load(Ordering::SeqCst) {
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
        result.insert("isError".into(), json!(true));
        if modern {
            result.insert("resultType".into(), json!("complete"));
        }
        return Response::json(200, protocol::reply(&id, Value::Object(result)));
    }
    let now_ms = shared.now_ms();
    let reply = {
        let mut mcp = lock(&shared.mcp);
        let Some(client) = client_for(&mut mcp, owner) else {
            return unauthorised(); // -- WP MCP2c --
        };
        mcp.server.handle_value(client, now_ms, msg)
    };
    match reply {
        None => Response::empty(202),
        Some(r) => {
            let status = r
                .pointer("/error/code")
                .and_then(Value::as_i64)
                .map_or(200, status_of);
            Response::json(status, r)
        }
    }
}

fn relay_reply(shared: &Shared, owner: u64, msg: Value, status: u16) -> Response {
    let now_ms = shared.now_ms();
    let mut mcp = lock(&shared.mcp);
    let Some(client) = client_for(&mut mcp, owner) else {
        return unauthorised(); // -- WP MCP2c --
    };
    match mcp.server.handle_value(client, now_ms, msg) {
        Some(r) => Response::json(status, r),
        None => Response::empty(202),
    }
}

fn initialize(shared: &Shared, owner: u64, msg: Value) -> Response {
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
            .filter(|(_, s)| s.owner == owner)
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
    mcp.sessions
        .insert(session.clone(), Session { owner, used });
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
        Some(s) if s.owner == owner => {
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
