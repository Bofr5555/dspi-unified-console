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
//! | Sockets and threads | [`ServerLimits::max_connections`] over every kind of socket, of which [`ServerLimits::max_preauth_per_peer`] unauthenticated from one address. Past that an answer of HTTP 503 on a short-lived thread, at most [`ServerLimits::max_refusing`] at a time; past that, closed unread |
//! | A socket that says nothing | The request head within [`ServerLimits::head_timeout`], however slowly it trickles; authenticated within [`ServerLimits::auth_timeout`] of opening, or close 4001 |
//! | Memory per socket | A head of [`MAX_HEAD`] bytes; a frame or message of [`MAX_FRAME`], refused from its header before it is buffered |
//! | A peer that stops reading | One write's [`ServerLimits::write_stall`]. Behind that, the outbox's limits (HUB.md section 6) |
//! | A peer that stops answering | [`ServerLimits::dead_peer`], close 1001. A frame trickled and never finished does not count as an answer |
//! | Guessing | [`crate::auth`]'s limits, and a token is not guessable |
//!
//! Lock level: the connection table is a leaf. Nothing else is taken under it, and
//! nothing blocks under it.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dspi_link::frames::{CmdResponse, Frame, NotifyFrame, PollFrame, kind};
use dspi_link::messages::{PROTO_MAJOR, PROTO_MINOR, PollSpec};
use dspi_link::socket::{Pipe, split_server};
use dspi_link::status;
use serde_json::{Map, Value, json};
use socket2::{Domain, Protocol, Socket, Type};
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Message, WebSocket};

use crate::auth::{AccessMode, AuthError, AuthStore, peer_key};
use crate::error::HubError;
use crate::hub::{HubHandle, MAX_INFLIGHT};
use crate::polls::{POLL_BUDGET_BPS, POLL_MAX_HZ};
use crate::router::MAX_PAYLOAD;
use crate::session::{CLOSE_REVOKED, CLOSE_SHUTTING_DOWN, Outbound, Outbox, Pop, Role, Session};
use crate::store::{parse_role, role_name, unix_seconds};

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

/// What one socket may cost. The defaults are the spec's where it names one. Tests
/// shorten the times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLimits {
    /// Every socket at once, authenticated or not. Each costs up to two threads, and
    /// up to `max_inflight` queued commands. Past it: HTTP 503.
    pub max_connections: usize,
    /// Unauthenticated sockets from one address (an IPv6 /64 counts as one), so one
    /// machine cannot hold every slot. Past it: HTTP 503.
    pub max_preauth_per_peer: usize,
    /// Threads answering 503 at once. Past it a socket is closed unanswered.
    pub max_refusing: usize,
    /// From accept to the whole HTTP request head.
    pub head_timeout: Duration,
    /// From accept to a successful authentication (spec 5).
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
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            max_connections: 16,
            max_preauth_per_peer: 4,
            max_refusing: 4,
            head_timeout: Duration::from_secs(10),
            auth_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(10),
            dead_peer: Duration::from_secs(30),
            write_stall: Duration::from_secs(10),
            close_grace: Duration::from_secs(1),
            read_tick: Duration::from_secs(1),
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
}

impl NetworkConfig {
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
    let rest = origin
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("http://"))
        .and_then(|_| origin.get(7..))?;
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
        None => 80,
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
    let Some(origin) = origin else {
        return true;
    };
    parse_http_origin(origin).is_some_and(|(host, port)| {
        port == own.port && own.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host))
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
    /// Anything without a session yet.
    PreAuth,
    Session,
}

struct Entry {
    /// The address, as limits count it.
    peer: IpAddr,
    stage: Stage,
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

    /// Clients and pairing: [`AuthStore::begin_pairing`], [`AuthStore::clients`].
    /// To revoke or re-role, use [`Self::revoke`] and [`Self::set_role`], which also
    /// reach the client's live sessions.
    pub fn auth(&self) -> &AuthStore {
        &self.shared.auth
    }

    /// Sockets the limits count now, authenticated or not.
    pub fn connections(&self) -> usize {
        guard(&self.shared.table)
            .conns
            .values()
            .filter(|e| !e.done && e.stage != Stage::Refusing)
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

impl Shared {
    fn sessions_of(&self, cid: u32) -> Vec<Arc<Session>> {
        guard(&self.table)
            .conns
            .values()
            .filter(|e| !e.done)
            .filter_map(|e| e.session.as_ref())
            .filter(|s| s.client_id() == Some(cid))
            .map(Arc::clone)
            .collect()
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
            let live = || table.conns.values().filter(|e| !e.done);
            let connections = live().filter(|e| e.stage != Stage::Refusing).count();
            let from_peer = live()
                .filter(|e| e.stage == Stage::PreAuth && e.peer == key)
                .count();
            let refusing = live().filter(|e| e.stage == Stage::Refusing).count();
            let stage = if connections < self.limits.max_connections
                && from_peer < self.limits.max_preauth_per_peer
            {
                Stage::PreAuth
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
        match listener.accept() {
            Ok((tcp, peer)) => {
                shared.reap();
                shared.admit(tcp, peer);
            }
            Err(e) => {
                // Out of descriptors looks the same to this loop: wait and go on.
                if e.kind() != io::ErrorKind::WouldBlock {
                    log::debug!("hub: accept: {e}");
                }
                shared.reap();
                thread::sleep(ACCEPT_TICK);
            }
        }
    }
}

/// Read up to the end of the HTTP head: `(head, what came behind it)`. Bounded in bytes
/// and, however slowly they trickle, in time.
fn read_head(tcp: &mut TcpStream, deadline: Instant) -> Result<(Vec<u8>, Vec<u8>), HeadError> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
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
        // Back over the seam: the end of the head can straddle two reads.
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
        let end = buf
            .get(from..)
            .and_then(|tail| tail.windows(4).position(|w| w == b"\r\n\r\n"))
            .map(|at| from.saturating_add(at).saturating_add(4));
        match end {
            Some(end) if end <= MAX_HEAD => {
                let tail = buf.split_off(end);
                return Ok((buf, tail));
            }
            Some(_) => return Err(HeadError::TooLarge),
            None if buf.len() > MAX_HEAD => return Err(HeadError::TooLarge),
            None => {}
        }
    }
}

enum HeadError {
    /// Timed out or closed: nobody to answer.
    Gone,
    TooLarge,
}

/// Write all of it by the deadline, however slowly the peer takes it.
fn write_by(tcp: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
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
fn drain(tcp: &mut TcpStream, deadline: Instant) {
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

/// One whole HTTP answer on a connection that closes behind it.
fn respond(
    shared: &Shared,
    tcp: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    deadline: Instant,
) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    if write_by(tcp, head.as_bytes(), deadline).is_err() || write_by(tcp, body, deadline).is_err() {
        return;
    }
    let _ = tcp.shutdown(Shutdown::Write);
    drain(tcp, after(Instant::now(), shared.limits.close_grace));
}

const TEXT: (&str, &str) = ("Content-Type", "text/plain; charset=utf-8");

/// The answer for a socket past the limits. On its own short-lived thread, so that a
/// legitimate client hears 503 (`LinkError::HubBusy`, spec 4) and not a reset: closing
/// with its request unread would be one.
fn refuse(shared: &Shared, mut tcp: TcpStream, opened: Instant) {
    let deadline = after(opened, shared.limits.close_grace);
    let _ = read_head(&mut tcp, deadline);
    respond(
        shared,
        &mut tcp,
        "503 Service Unavailable",
        &[TEXT, ("Retry-After", "5")],
        b"The hub has no room for another session.",
        deadline,
    );
}

/// The whole life of a socket, on its reader thread.
fn serve(shared: &Arc<Shared>, id: u64, mut tcp: TcpStream, peer: IpAddr, opened: Instant) {
    let deadline = after(opened, shared.limits.head_timeout);
    let (head, tail) = match read_head(&mut tcp, deadline) {
        Ok(read) => read,
        Err(HeadError::Gone) => return,
        Err(HeadError::TooLarge) => {
            let status = "431 Request Header Fields Too Large";
            respond(shared, &mut tcp, status, &[TEXT], b"Too large.", deadline);
            return;
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    if !matches!(request.parse(&head), Ok(httparse::Status::Complete(_))) {
        respond(
            shared,
            &mut tcp,
            "400 Bad Request",
            &[TEXT],
            b"Not HTTP.",
            deadline,
        );
        return;
    }
    let path = request
        .path
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    if path == WS_PATH {
        match check_upgrade(shared, &request) {
            Ok(accept) => upgrade(shared, id, tcp, tail, &accept, peer, opened),
            Err((status, why)) => {
                respond(shared, &mut tcp, status, &[TEXT], why.as_bytes(), deadline)
            }
        }
        return;
    }
    if request.method != Some("GET") {
        let status = "405 Method Not Allowed";
        respond(
            shared,
            &mut tcp,
            status,
            &[TEXT, ("Allow", "GET")],
            b"GET only.",
            deadline,
        );
        return;
    }
    if path == INFO_PATH {
        let body = shared.info();
        let headers = [
            ("Content-Type", "application/json"),
            ("Access-Control-Allow-Origin", "*"),
        ];
        respond(
            shared,
            &mut tcp,
            "200 OK",
            &headers,
            body.as_bytes(),
            deadline,
        );
        return;
    }
    // `/dspi/` is the protocol's (spec 4.2), and the web UI is only served when on.
    let asset = (shared.web && !path.starts_with("/dspi/"))
        .then(|| crate::web::asset(path))
        .flatten();
    match asset {
        Some(asset) => {
            let headers = [("Content-Type", asset.content_type)];
            // The peer's slot is held until its time runs out, however big the file.
            let until = after(opened, shared.limits.auth_timeout);
            respond(shared, &mut tcp, "200 OK", &headers, asset.body, until);
        }
        None => respond(
            shared,
            &mut tcp,
            "404 Not Found",
            &[TEXT],
            b"Not found.",
            deadline,
        ),
    }
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
) -> Result<String, (&'static str, &'static str)> {
    const FORBIDDEN: &str = "403 Forbidden";
    const BAD: &str = "400 Bad Request";
    // First, so a web page learns as little as it can from the answer.
    let origin = single_header(request, "Origin").map_err(|()| (FORBIDDEN, "Origin refused."))?;
    let allowed = match origin {
        None => true,
        // No page to be the origin of, so nothing a browser sends can be right.
        Some(_) if !shared.web => false,
        Some(_) => origin_allowed(
            origin,
            &OwnNames::current(shared.port, shared.mdns_name.as_deref()),
        ),
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

fn upgrade(
    shared: &Arc<Shared>,
    id: u64,
    mut tcp: TcpStream,
    tail: Vec<u8>,
    accept: &str,
    peer: IpAddr,
    opened: Instant,
) {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: {SUBPROTOCOL}\r\n\r\n"
    );
    let until = after(opened, shared.limits.head_timeout);
    if write_by(&mut tcp, response.as_bytes(), until).is_err() {
        return;
    }
    let limits = &shared.limits;
    let set = tcp
        .set_write_timeout(Some(limits.write_stall))
        .and(tcp.set_read_timeout(Some(limits.read_tick)));
    let halves = tcp
        .try_clone()
        .and_then(|cut_off| Ok((cut_off, split_server(tcp, tail, MAX_FRAME as usize)?)));
    let (Ok(()), Ok((cut_off, (reader, writer)))) = (set, halves) else {
        return;
    };
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
    };
    connection.run(reader, opened);
}

/// The sending half of a WebSocket. The writer thread sends through it once there is
/// one; before that the reader does, since a socket with no session has no thread to
/// spare for a writer.
struct WsTx {
    ws: Mutex<WebSocket<Pipe>>,
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
}

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
    fn run(&mut self, mut reader: WebSocket<Pipe>, opened: Instant) {
        let limits = self.shared.limits.clone();
        let auth_deadline = after(opened, limits.auth_timeout);
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
            drain(reader.get_mut().tcp_mut(), deadline);
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
                    match crate::lock::handle_message(&self.shared.hub, &session, &value) {
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
            // `fw.install` is later work (PLAN-CLIENTS), and the hub messages are not in
            // HB1's entry.
            "hub.stats" | "hub.rename" if !admin => denied(),
            "fw.install" | "hub.stats" | "hub.rename" => err(id, "unsupported", None),
            _ => err(id, "unknown_type", None),
        };
        self.reply(answer);
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
                    "online": online.contains(&c.cid),
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
        if self.web {
            caps.push("web");
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
        json!({
            "proto": {"major": PROTO_MAJOR, "minor": PROTO_MINOR},
            "hub": self.hub_info(),
            "auth": self.auth.mode().wire_name(),
            "tls": false,
            "ws": WS_PATH,
            "devices": devices,
        })
        .to_string()
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
