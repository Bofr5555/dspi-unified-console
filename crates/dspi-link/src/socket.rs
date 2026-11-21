//! The WebSocket byte pipe under the Link client.
//!
//! [`WsConnector`] / [`WsSender`] / [`WsReceiver`] are the seam: the client
//! logic in `client.rs` only sees whole text and binary messages, so another
//! platform can supply its own socket (iOS `URLSessionWebSocketTask`, Android
//! OkHttp) as a foreign implementation later (CONSOLE-UNIFICATION-STUDY.md
//! section 6.2, "byte-pipe transports"). [`TcpConnector`] is the built-in
//! one: `tungstenite` over a blocking `std::net::TcpStream`, plaintext
//! (`ws://`) or, with feature `tls` off the web, TLS 1.3 to a pinned or
//! first-use key ([`WsConnector::connect_tls`], `wss://`; WP SECCORE,
//! PLAN-SEC 4.3) over [`crate::tls::TlsPipe`].
//!
//! Concurrency: one reader thread blocks in [`WsReceiver::recv`] while any
//! thread may send. tungstenite has no split API, so the stream is cloned
//! into a reading and a writing `WebSocket`; both buffer their output and
//! write each frame whole under one shared lock on flush, so the reader's
//! automatic pong/close replies can never interleave with a frame the writer
//! is sending.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;
use std::time::Duration;

use crate::clock::Instant;

// -- WP WB2 -- socket2 has no wasm32-unknown-unknown backend.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use socket2::{SockRef, TcpKeepalive};

use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

use crate::LinkError;
// -- WP SECCORE --
use crate::credential::Pin;
#[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
use crate::tls::TlsPipe;

/// One whole WebSocket message.
#[derive(Clone, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
}

/// By hand: a text message may be an `auth.pair` or an `auth.token`, so the
/// content never reaches a log.
impl fmt::Debug for WsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WsMessage::Text(t) => write!(f, "Text({} bytes)", t.len()),
            WsMessage::Binary(b) => write!(f, "Binary({} bytes)", b.len()),
        }
    }
}

/// What [`WsReceiver::recv`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsRecv {
    Message(WsMessage),
    /// A control frame arrived (ping or pong): nothing for the client logic, but
    /// proof the peer is alive. A foreign socket must report these, or a healthy
    /// idle session looks dead.
    Alive,
    /// Nothing arrived within the receiver's tick (about a second): lets the
    /// reader loop check for shutdown and keepalive.
    Idle,
    /// The peer closed (`code` from its close frame, if any) or the socket failed.
    Closed {
        code: Option<u16>,
        reason: String,
    },
}

pub trait WsSender: Send + Sync {
    fn send(&self, msg: WsMessage) -> io::Result<()>;
    fn ping(&self) -> io::Result<()>;
    /// Send a close frame (best effort) and shut the socket down, which also
    /// ends a blocked [`WsReceiver::recv`].
    fn close(&self, code: u16);

    // -- WP SECPAKE --
    /// The TLS exporter `auth.pair2` binds to (`crate::pake::EXPORTER_LABEL`, 32
    /// bytes) when this socket is our own TLS; `None` otherwise (plaintext, the
    /// browser's socket, test fakes). Secret to the connection: never logged.
    fn tls_binding(&self) -> Option<[u8; 32]> {
        None
    }
}

pub trait WsReceiver: Send {
    fn recv(&mut self) -> WsRecv;

    // -- WP WB4b --
    /// [`Self::recv`] waiting at most about `max` ([`WsRecv::Idle`] after it;
    /// zero: only what has arrived). A caller-driven session
    /// ([`crate::client::caller_driven`]) reads with this. The default is
    /// `recv`, whose wait is its own tick.
    fn recv_within(&mut self, _max: Duration) -> WsRecv {
        self.recv()
    }
}

/// The two halves of an open WebSocket.
pub type WsPair = (Box<dyn WsSender>, Box<dyn WsReceiver>);

/// Opens a WebSocket to `url` requesting sub-protocol `protocol`.
pub trait WsConnector: Send + Sync {
    fn connect(&self, url: &str, protocol: &str) -> Result<WsPair, LinkError>;

    // -- WP SECCORE --
    /// [`Self::connect`] over TLS 1.3 (PLAN-SEC 4.3): `url` may say `ws://` or
    /// `wss://`, the hub is the same. Returns the pin of the key the hub
    /// presented, whose handshake signature has been verified; with
    /// [`TlsTrust::Pinned`] another key is reported rather than refused
    /// (rotation mode: the caller withholds every secret until it has judged
    /// it). The TLS handshake must finish within `handshake_within` when given.
    ///
    /// A TLS handshake that fails (refused, reset, not TLS, too slow) is
    /// [`LinkError::TlsRefused`]; a failure before it (name, TCP) or after it
    /// (the upgrade) is the error [`Self::connect`] would give. The default: a
    /// connector without TLS (the browser's, test fakes) refuses.
    fn connect_tls(
        &self,
        url: &str,
        protocol: &str,
        trust: TlsTrust,
        handshake_within: Option<Duration>,
    ) -> Result<(WsPair, Pin), LinkError> {
        let _ = (url, protocol, trust, handshake_within);
        Err(LinkError::TlsRefused { advertised: false })
    }

    // -- WP LKFAST --
    /// [`Self::connect_tls`] where a refused TLS falls back to plaintext (an
    /// unpinned hub that did not advertise TLS; PLAN-SEC 4.3 rule 2): the pin of
    /// the key presented, or `None` for the plaintext connection. The default
    /// tries TLS, then [`Self::connect`]. A connector may get the plaintext
    /// connection ready while TLS is tried, as long as nothing goes out on it
    /// before TLS was refused.
    fn connect_tls_or_plain(
        &self,
        url: &str,
        protocol: &str,
        trust: TlsTrust,
        handshake_within: Option<Duration>,
    ) -> Result<(WsPair, Option<Pin>), LinkError> {
        match self.connect_tls(url, protocol, trust, handshake_within) {
            Ok((pair, pin)) => Ok((pair, Some(pin))),
            Err(LinkError::TlsRefused { .. }) => {
                log::info!("dspi-link: the hub has no TLS; connecting unencrypted");
                self.connect(url, protocol).map(|pair| (pair, None))
            }
            Err(e) => Err(e),
        }
    }

    // -- WP SECWEB --
    /// The socket under this connector does TLS itself, by the URL's scheme (the
    /// browser's WebSocket: `wss://` is TLS checked by the browser's own trust, `ws://`
    /// is not, and no key can be pinned). [`crate::client::LinkClient::connect_tls`]
    /// then connects with [`Self::connect`] as the URL says, never trying TLS of its own.
    fn tls_by_scheme(&self) -> bool {
        false
    }
}

// -- WP SECCORE --
/// What a TLS dial trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsTrust {
    /// Any key (first contact, or a `v1` credential): the caller decides.
    FirstUse,
    /// This key; any other is reported, not trusted (rotation mode).
    Pinned(Pin),
}

/// Largest message accepted from a hub (its `max_frame` is at most 64 KiB in
/// every known hub; a hostile peer cannot make us buffer more than this).
const MAX_MESSAGE: usize = 1 << 20;
const READ_TICK: Duration = Duration::from_millis(1000);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// The close frame is a courtesy on a socket that may be stuck.
const CLOSE_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// SO_KEEPALIVE, as the bridge sets on its side (`http_server.c`): a backstop
/// for when nothing is being sent; the client's own pings notice sooner.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
const KEEPALIVE_TIME: Duration = Duration::from_secs(30);
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios", target_os = "windows"))]
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);

/// `tungstenite` over `std::net::TcpStream`.
#[derive(Debug, Clone)]
pub struct TcpConnector {
    /// The whole of [`WsConnector::connect`]: name lookup, TCP connect (shared
    /// between a host's addresses) and the WebSocket handshake.
    pub connect_timeout: Duration,
}

impl Default for TcpConnector {
    fn default() -> Self {
        Self { connect_timeout: Duration::from_secs(5) }
    }
}

/// A `TcpStream` that, once `buffered`, collects writes and puts them on the
/// wire whole on `flush`, under the lock shared by both halves.
pub struct Pipe {
    tcp: TcpStream,
    buffered: bool,
    out: Vec<u8>,
    wlock: Arc<Mutex<()>>,
    /// A write failed part way, so a frame may be cut in two on the wire: the
    /// stream can never be trusted again. Shared by both halves.
    broken: Arc<AtomicBool>,
    /// During the handshake: reads end here however slowly bytes trickle in.
    deadline: Option<Instant>,
    // -- WP SECCORE --
    /// The TLS connection this half reads and writes through; `tcp` is then a
    /// clone of its socket, for timeouts and the shutdown.
    #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
    tls: Option<TlsPipe>,
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // -- WP SECCORE --
        #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
        if let Some(tls) = self.tls.as_mut() {
            tls.set_deadline(self.deadline);
            return tls.read(buf);
        }
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            self.tcp.set_read_timeout(Some(left))?;
        }
        self.tcp.read(buf)
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.buffered {
            self.out.extend_from_slice(buf);
            Ok(buf.len())
        } else {
            // -- WP SECCORE --
            #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
            if let Some(tls) = self.tls.as_mut() {
                return tls.write(buf);
            }
            self.tcp.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.broken.load(Ordering::SeqCst) {
            self.out.clear();
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if !self.out.is_empty() {
            let _g = self.wlock.lock().unwrap_or_else(|p| p.into_inner());
            // -- WP SECCORE -- a frame is encrypted whole, in order (TlsPipe)
            #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
            let r = match self.tls.as_mut() {
                Some(tls) => tls.write_all(&self.out),
                None => self.tcp.write_all(&self.out),
            };
            #[cfg(not(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown")))))]
            let r = self.tcp.write_all(&self.out);
            self.out.clear();
            if let Err(e) = r {
                self.broken.store(true, Ordering::SeqCst);
                return Err(e);
            }
        }
        self.tcp.flush()
    }
}

fn config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE))
}

fn io_err(e: tungstenite::Error) -> io::Error {
    match e {
        tungstenite::Error::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

/// Host and port out of `ws://host:port/...` or `wss://...` (brackets kept off
/// IPv6). -- WP SECCORE -- `wss://`; its port defaults to 443.
fn authority(url: &str) -> Option<(String, u16)> {
    let (rest, default_port) = match url.strip_prefix("ws://") {
        Some(rest) => (rest, 80),
        None => (url.strip_prefix("wss://")?, 443),
    };
    let auth = rest.split('/').next()?;
    if let Some(v6) = auth.strip_prefix('[') {
        let (host, tail) = v6.split_once(']')?;
        let port = tail.strip_prefix(':').map(str::parse).transpose().ok()?.unwrap_or(default_port);
        return Some((host.to_owned(), port));
    }
    match auth.rsplit_once(':') {
        Some((h, p)) => Some((h.to_owned(), p.parse().ok()?)),
        None => Some((auth.to_owned(), default_port)),
    }
}

/// Keeps a hostile hub's refusal text down to a line for a UI.
const MAX_REFUSAL_TEXT: usize = 200;

/// What a failed upgrade means: a status a hub chose to send, a timeout, or a
/// broken exchange.
fn handshake_error<S>(e: HandshakeError<S>) -> LinkError
where
    S: tungstenite::handshake::HandshakeRole,
{
    match e {
        HandshakeError::Interrupted(_) => LinkError::Timeout,
        HandshakeError::Failure(tungstenite::Error::Http(response)) => {
            let text = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default();
            let text: String = text.trim().chars().take(MAX_REFUSAL_TEXT).collect();
            LinkError::from_handshake_status(response.status().as_u16(), text)
        }
        HandshakeError::Failure(tungstenite::Error::Io(e)) => match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => LinkError::Timeout,
            _ => LinkError::Io(e.to_string()),
        },
        HandshakeError::Failure(e) => LinkError::Handshake(e.to_string()),
    }
}

/// The addresses of `host`. A lookup can block far longer than a caller may wait
/// (mDNS names on a dead network), so it runs on its own thread, which ends by
/// itself when the resolver gives up.
fn resolve(host: &str, port: u16, deadline: Instant) -> Result<Vec<SocketAddr>, LinkError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let name = host.to_owned();
    thread::Builder::new()
        .name("dspi-link-dns".into())
        .spawn(move || {
            let _ = tx.send((name.as_str(), port).to_socket_addrs().map(Iterator::collect::<Vec<_>>));
        })
        .map_err(|e| LinkError::Io(e.to_string()))?;
    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(r) => r.map_err(|e| LinkError::Io(e.to_string())),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(LinkError::Timeout),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(LinkError::Io("name lookup failed".into())),
    }
}

/// The first of `addrs` to connect, with the time left shared out among them.
fn connect_any(addrs: &[SocketAddr], deadline: Instant) -> Result<TcpStream, LinkError> {
    let mut last = None;
    for (a, left_to_try) in addrs.iter().zip((1..=addrs.len()).rev()) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(LinkError::Timeout);
        }
        let share = u32::try_from(left_to_try).map_or(left, |n| left / n);
        match TcpStream::connect_timeout(a, share) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => LinkError::Timeout,
        Some(e) => LinkError::Io(e.to_string()),
        None => LinkError::Io("no address".into()),
    })
}

/// NODELAY and SO_KEEPALIVE. Neither is worth failing a connection for.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn tune(tcp: &TcpStream) {
    let _ = tcp.set_nodelay(true);
    let keepalive = TcpKeepalive::new().with_time(KEEPALIVE_TIME);
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    ))]
    let keepalive = keepalive.with_interval(KEEPALIVE_INTERVAL);
    if let Err(e) = SockRef::from(tcp).set_tcp_keepalive(&keepalive) {
        log::warn!("dspi-link: no TCP keepalive: {e}");
    }
}

/// No sockets on the web (WP WB2): `std`'s connect fails there before this.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn tune(tcp: &TcpStream) {
    let _ = tcp.set_nodelay(true);
}

impl WsConnector for TcpConnector {
    fn connect(&self, url: &str, protocol: &str) -> Result<WsPair, LinkError> {
        let deadline = Instant::now() + self.connect_timeout;
        if url.starts_with("wss://") {
            return Err(LinkError::Io(format!("a wss:// URL needs TLS: {url}")));
        }
        let tcp = dial_tcp(url, deadline)?;
        plain_over(url, protocol, tcp, deadline)
    }

    // -- WP LKFAST --
    /// The plaintext connection's TCP is dialled on a thread of its own while
    /// TLS is tried on this one, and used (the WebSocket upgrade written) only
    /// once TLS was refused: a hub without TLS (the ESP32 bridge) is reached in
    /// the time of the longer dial, not of both. A hub with TLS sees a TCP
    /// connect that closes unused; nothing is sent in plaintext.
    #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
    fn connect_tls_or_plain(
        &self,
        url: &str,
        protocol: &str,
        trust: TlsTrust,
        handshake_within: Option<Duration>,
    ) -> Result<(WsPair, Option<Pin>), LinkError> {
        let spare_by = Instant::now() + self.connect_timeout;
        let spare_url = url.to_owned();
        // Not joined when TLS answers: a spare still dialling ends by itself and drops its socket
        let spare = std::thread::Builder::new()
            .name("dspi-link-dial".into())
            .spawn(move || dial_tcp(&spare_url, spare_by))
            .ok();
        match self.connect_tls(url, protocol, trust, handshake_within) {
            Ok((pair, pin)) => Ok((pair, Some(pin))),
            Err(LinkError::TlsRefused { .. }) => {
                log::info!("dspi-link: the hub has no TLS; connecting unencrypted");
                let deadline = Instant::now() + self.connect_timeout;
                match spare.map(std::thread::JoinHandle::join) {
                    // -- WP LKFAST3 -- (review LKREV 9) a spare the hub closed while TLS was
                    // tried (an idle timeout): one fresh dial, as without a spare
                    Some(Ok(Ok(tcp))) => {
                        plain_over(url, protocol, tcp, deadline).or_else(|_| self.connect(url, protocol))
                    }
                    _ => self.connect(url, protocol),
                }
                .map(|pair| (pair, None))
            }
            Err(e) => Err(e),
        }
    }

    // -- WP SECCORE --
    #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
    fn connect_tls(
        &self,
        url: &str,
        protocol: &str,
        trust: TlsTrust,
        handshake_within: Option<Duration>,
    ) -> Result<(WsPair, Pin), LinkError> {
        use crate::tls::{PinMode, SpkiHash};
        let deadline = Instant::now() + self.connect_timeout;
        let (host, _) = authority(url).ok_or_else(|| LinkError::Io(format!("not a ws:// URL: {url}")))?;
        let tcp = dial_tcp(url, deadline)?;
        let handshake_by = handshake_within.map_or(deadline, |d| (Instant::now() + d).min(deadline));
        let mode = match trust {
            TlsTrust::FirstUse => PinMode::FirstUse,
            TlsTrust::Pinned(pin) => PinMode::Rotating(SpkiHash(pin)),
        };
        let (tls, outcome) = crate::tls::connect(tcp, &host, mode, handshake_by).map_err(|e| {
            log::info!("dspi-link: no TLS at {host}: {e}");
            LinkError::TlsRefused { advertised: false }
        })?;
        let io = |e: io::Error| LinkError::Io(e.to_string());
        // -- WP SECPAKE --
        let binding = crate::pake::Binding::of(&tls).map(|b| b.into_bytes());
        let wlock = Arc::new(Mutex::new(()));
        let broken = Arc::new(AtomicBool::new(false));
        let write_tls = tls.try_clone().map_err(io)?;
        let shutdown = tls.tcp().try_clone().map_err(io)?;
        let reader = Pipe::over_tls(tls, Arc::clone(&wlock), Arc::clone(&broken), Some(deadline)).map_err(io)?;
        let mut writer = Pipe::over_tls(write_tls, wlock, Arc::clone(&broken), None).map_err(io)?;
        writer.buffered = true;
        let wss = url.strip_prefix("ws://").map_or_else(|| url.to_owned(), |rest| format!("wss://{rest}"));
        let pair = upgrade(&wss, protocol, reader, writer, shutdown, broken, binding)?;
        Ok((pair, outcome.presented().0))
    }
}

/// The plaintext WebSocket over a dialled `tcp`, its upgrade answered by `deadline`.
fn plain_over(url: &str, protocol: &str, tcp: TcpStream, deadline: Instant) -> Result<WsPair, LinkError> {
    let wlock = Arc::new(Mutex::new(()));
    let write_half = tcp.try_clone().map_err(|e| LinkError::Io(e.to_string()))?;
    let shutdown = write_half.try_clone().map_err(|e| LinkError::Io(e.to_string()))?;
    let broken = Arc::new(AtomicBool::new(false));
    let reader = Pipe::plain(tcp, Arc::clone(&wlock), Arc::clone(&broken), Some(deadline));
    let mut writer = Pipe::plain(write_half, wlock, Arc::clone(&broken), None);
    writer.buffered = true;
    upgrade(url, protocol, reader, writer, shutdown, broken, None)
}

/// Name lookup and TCP connect by `deadline`, tuned, with the write-stall timeout.
fn dial_tcp(url: &str, deadline: Instant) -> Result<TcpStream, LinkError> {
    let (host, port) = authority(url).ok_or_else(|| LinkError::Io(format!("not a ws:// URL: {url}")))?;
    let addrs = resolve(&host, port, deadline)?;
    let tcp = connect_any(&addrs, deadline)?;
    tune(&tcp);
    tcp.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(|e| LinkError::Io(e.to_string()))?;
    Ok(tcp)
}

/// The WebSocket upgrade over `reader` (which carries the deadline), then the
/// split into the sender (over `writer`, buffered) and the receiver.
fn upgrade(
    url: &str,
    protocol: &str,
    reader: Pipe,
    writer: Pipe,
    shutdown: TcpStream,
    broken: Arc<AtomicBool>,
    binding: Option<[u8; 32]>, // -- WP SECPAKE --
) -> Result<WsPair, LinkError> {
    let mut request = url.into_client_request().map_err(|e| LinkError::Handshake(e.to_string()))?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        protocol.parse().map_err(|_| LinkError::Handshake("bad sub-protocol".into()))?,
    );
    let (mut reader, response) =
        tungstenite::client::client_with_config(request, reader, Some(config())).map_err(handshake_error)?;
    match response.headers().get("Sec-WebSocket-Protocol").and_then(|v| v.to_str().ok()) {
        Some(p) if p.eq_ignore_ascii_case(protocol) => {}
        other => log::warn!("dspi-link: hub did not echo sub-protocol {protocol} (got {other:?})"),
    }
    reader.get_mut().buffered = true;
    reader.get_mut().deadline = None;
    reader.get_ref().tcp.set_read_timeout(Some(READ_TICK)).map_err(|e| LinkError::Io(e.to_string()))?;
    let writer = WebSocket::from_raw_socket(writer, Role::Client, Some(config()));
    Ok((
        Box::new(TcpSender { ws: Mutex::new(writer), shutdown, binding }),
        Box::new(TcpReceiver { ws: reader, close_code: None, close_text: String::new(), broken }),
    ))
}

struct TcpSender {
    ws: Mutex<WebSocket<Pipe>>,
    shutdown: TcpStream,
    // -- WP SECPAKE --
    binding: Option<[u8; 32]>,
}

impl WsSender for TcpSender {
    // -- WP SECPAKE --
    fn tls_binding(&self) -> Option<[u8; 32]> {
        self.binding
    }

    fn send(&self, msg: WsMessage) -> io::Result<()> {
        let m = match msg {
            WsMessage::Text(t) => Message::text(t),
            WsMessage::Binary(b) => Message::binary(b),
        };
        self.ws.lock().unwrap_or_else(|p| p.into_inner()).send(m).map_err(io_err)
    }

    fn ping(&self) -> io::Result<()> {
        self.ws.lock().unwrap_or_else(|p| p.into_inner()).send(Message::Ping(Vec::new().into())).map_err(io_err)
    }

    fn close(&self, code: u16) {
        // A writer stuck on a dead link holds the lock for up to WRITE_TIMEOUT.
        // Then there is no use for a close frame, and the shutdown frees it.
        let _ = self.shutdown.set_write_timeout(Some(CLOSE_WRITE_TIMEOUT));
        let ws = match self.ws.try_lock() {
            Ok(ws) => Some(ws),
            Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        };
        if let Some(mut ws) = ws {
            let _ = ws.close(Some(CloseFrame { code: CloseCode::from(code), reason: "".into() }));
            let _ = ws.flush();
        }
        let _ = self.shutdown.shutdown(Shutdown::Both);
    }
}

struct TcpReceiver {
    ws: WebSocket<Pipe>,
    close_code: Option<u16>,
    // -- WP SECBFIXA -- the close frame's reason, bounded (a 4003 `tls_required`)
    close_text: String,
    broken: Arc<AtomicBool>,
}

// -- WP SECBFIXA --
impl TcpReceiver {
    /// The hub's close reason when it gave one, else `fallback`.
    fn closed_text(&self, fallback: &str) -> String {
        if self.close_code.is_some() && !self.close_text.is_empty() {
            self.close_text.clone()
        } else {
            fallback.to_owned()
        }
    }
}

impl WsReceiver for TcpReceiver {
    fn recv(&mut self) -> WsRecv {
        loop {
            // Read errors include our own failed pong: see Pipe::broken. -- WP LKCLOSE --
            // A failed write does not stop the reading: the peer's close frame may be
            // waiting, and its code says whether to reconnect. Once nothing more comes,
            // a broken socket is closed (below), not idle.
            match self.ws.read() {
                Ok(Message::Text(t)) => return WsRecv::Message(WsMessage::Text(t.as_str().to_owned())),
                Ok(Message::Binary(b)) => return WsRecv::Message(WsMessage::Binary(b.to_vec())),
                // tungstenite has already queued the reply to a ping
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => return WsRecv::Alive,
                Ok(Message::Frame(_)) => continue,
                Ok(Message::Close(frame)) => {
                    // -- WP SECBFIXA -- the reason too (a control frame: 123 bytes at most)
                    self.close_text = frame.as_ref().map(|f| f.reason.as_str().to_owned()).unwrap_or_default();
                    self.close_code = frame.map(|f| u16::from(f.code));
                    continue; // the next read finishes the close handshake
                }
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
                        && !self.broken.load(Ordering::SeqCst) =>
                {
                    return WsRecv::Idle
                }
                Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                    return WsRecv::Closed { code: self.close_code, reason: self.closed_text("closed") }
                }
                Err(e) => return WsRecv::Closed { code: self.close_code, reason: self.closed_text(&e.to_string()) },
            }
        }
    }

    // -- WP WB4b -- only a caller-driven session (natively: tests) waits less than the tick
    fn recv_within(&mut self, max: Duration) -> WsRecv {
        let wait = max.clamp(Duration::from_millis(1), READ_TICK);
        if self.ws.get_ref().tcp.set_read_timeout(Some(wait)).is_err() {
            return self.recv();
        }
        let received = self.recv();
        let _ = self.ws.get_ref().tcp.set_read_timeout(Some(READ_TICK));
        received
    }
}

// -- WP HB1 --
/// The two halves of a WebSocket a hub accepted, for its reader and its writer
/// threads: the same split as [`TcpConnector`]'s, with role server. The hub has read
/// and answered the upgrade itself, so this starts on the WebSocket's first byte.
/// `tail` is what arrived behind the upgrade request. `max_frame` limits a frame and
/// a message from the peer, checked before anything is buffered for it.
pub fn split_server(tcp: TcpStream, tail: Vec<u8>, max_frame: usize) -> io::Result<(WebSocket<Pipe>, WebSocket<Pipe>)> {
    let (reader, writer) = server_pipes(tcp)?;
    let config = server_ws_config(max_frame);
    let reader = WebSocket::from_partially_read(reader, tail, Role::Server, Some(config));
    let writer = WebSocket::from_raw_socket(writer, Role::Server, Some(config));
    Ok((reader, writer))
}

/// Per socket and half, so an unauthenticated socket costs little.
const SERVER_READ_BUFFER: usize = 16 * 1024;

// -- WP SECSRV --
/// The two halves of an accepted plain socket as [`Pipe`]s (buffered, one write
/// lock, one broken flag), as [`split_server`] makes them, for a hub that builds
/// its WebSocket over a stream type of its own: plain or TLS on one port. Use
/// [`server_ws_config`] for the WebSocket.
pub fn server_pipes(tcp: TcpStream) -> io::Result<(Pipe, Pipe)> {
    let write_half = tcp.try_clone()?;
    let wlock = Arc::new(Mutex::new(()));
    let broken = Arc::new(AtomicBool::new(false));
    let pipe = |tcp, wlock, broken| {
        let mut p = Pipe::plain(tcp, wlock, broken, None);
        p.buffered = true;
        p
    };
    Ok((pipe(tcp, Arc::clone(&wlock), Arc::clone(&broken)), pipe(write_half, wlock, broken)))
}

// -- WP SECSRV --
/// The WebSocket config of [`split_server`]'s halves: `max_frame` for a frame
/// and a message, and a small read buffer per half.
pub fn server_ws_config(max_frame: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(max_frame))
        .max_frame_size(Some(max_frame))
        .read_buffer_size(SERVER_READ_BUFFER)
}

impl Pipe {
    /// Unbuffered, over a plain socket.
    fn plain(tcp: TcpStream, wlock: Arc<Mutex<()>>, broken: Arc<AtomicBool>, deadline: Option<Instant>) -> Pipe {
        Pipe {
            tcp,
            buffered: false,
            out: Vec::new(),
            wlock,
            broken,
            deadline,
            #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
            tls: None,
        }
    }

    // -- WP SECCORE --
    /// Unbuffered, over one half of a TLS connection.
    #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
    fn over_tls(
        tls: TlsPipe,
        wlock: Arc<Mutex<()>>,
        broken: Arc<AtomicBool>,
        deadline: Option<Instant>,
    ) -> io::Result<Pipe> {
        let mut pipe = Pipe::plain(tls.tcp().try_clone()?, wlock, broken, deadline);
        pipe.tls = Some(tls);
        Ok(pipe)
    }

    /// A write failed part way, so nothing more can be sent on this stream. A read
    /// timeout means nothing while this is set: it may be that write's timeout.
    pub fn is_broken(&self) -> bool {
        #[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
        if self.tls.as_ref().is_some_and(TlsPipe::is_broken) {
            return true;
        }
        self.broken.load(Ordering::SeqCst)
    }

    /// The socket under it, to read what is left after the WebSocket has closed.
    pub fn tcp_mut(&mut self) -> &mut TcpStream {
        &mut self.tcp
    }
}

#[cfg(test)]
mod tests {
    use super::{authority, connect_any, tune, TcpConnector, WsConnector};
    use crate::LinkError;
    use socket2::SockRef;
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    #[test]
    fn wp_l1_tuned_sockets_keep_alive_and_no_delay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        assert!(!SockRef::from(&tcp).keepalive().unwrap());
        tune(&tcp);
        assert!(SockRef::from(&tcp).keepalive().unwrap());
        assert!(tcp.nodelay().unwrap());
    }

    /// F10's hypothesis: a timed-out write may have cut a frame in two, and
    /// carrying on would desynchronise the stream for good.
    #[test]
    fn wp_l1_a_failed_write_breaks_the_pipe_for_good() {
        use super::Pipe;
        use std::io::Write;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _peer_never_reads = listener.accept().unwrap();
        tcp.set_write_timeout(Some(Duration::from_millis(20))).unwrap();
        let broken = Arc::new(AtomicBool::new(false));
        let mut pipe = Pipe::plain(tcp, Arc::new(Mutex::new(())), Arc::clone(&broken), None);
        pipe.buffered = true;
        pipe.write_all(&vec![0u8; 64 << 20]).unwrap();
        assert!(pipe.flush().is_err(), "more than the socket buffers can hold");
        assert!(broken.load(Ordering::SeqCst));
        pipe.write_all(b"x").unwrap();
        assert_eq!(pipe.flush().unwrap_err().kind(), std::io::ErrorKind::BrokenPipe, "nothing more goes out");
    }

    #[test]
    fn wp_l1_a_spent_budget_connects_to_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let spent = Instant::now();
        let e = connect_any(&[listener.local_addr().unwrap()], spent).err().unwrap();
        assert_eq!(e, LinkError::Timeout);
    }

    #[test]
    fn wp_l1_a_refused_port_is_an_io_error_not_a_timeout() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let c = TcpConnector { connect_timeout: Duration::from_secs(2) };
        assert!(matches!(c.connect(&url, "dspi-link-1").err().unwrap(), LinkError::Io(_)));
    }

    #[test]
    fn wp_l1_a_trickling_handshake_ends_at_the_deadline() {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/dspi/v1", listener.local_addr().unwrap());
        let feeder = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // Bytes keep coming, so no single read ever times out
            let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nX-Slow: ");
            while s.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let started = Instant::now();
        let c = TcpConnector { connect_timeout: Duration::from_millis(100) };
        assert_eq!(c.connect(&url, "dspi-link-1").err().unwrap(), LinkError::Timeout);
        assert!(started.elapsed() < Duration::from_secs(3));
        drop(feeder); // Ends by itself on the broken pipe
    }

    #[test]
    fn authority_parses_hosts() {
        assert_eq!(authority("ws://192.168.1.20:11915/dspi/v1"), Some(("192.168.1.20".into(), 11915)));
        assert_eq!(authority("ws://[fe80::1]:11915/dspi/v1"), Some(("fe80::1".into(), 11915)));
        assert_eq!(authority("ws://studio.local/dspi/v1"), Some(("studio.local".into(), 80)));
        assert_eq!(authority("http://x"), None);
    }
}
