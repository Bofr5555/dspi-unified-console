//! The WebSocket byte pipe under the Link client.
//!
//! [`WsConnector`] / [`WsSender`] / [`WsReceiver`] are the seam: the client
//! logic in `client.rs` only sees whole text and binary messages, so another
//! platform can supply its own socket (iOS `URLSessionWebSocketTask`, Android
//! OkHttp) as a foreign implementation later (CONSOLE-UNIFICATION-STUDY.md
//! section 6.2, "byte-pipe transports"). [`TcpConnector`] is the built-in
//! one: `tungstenite` over a blocking `std::net::TcpStream`, plaintext
//! (`ws://`; no hub implements TLS, spec 6.4).
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
use std::time::{Duration, Instant};

use socket2::{SockRef, TcpKeepalive};

use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

use crate::LinkError;

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
}

pub trait WsReceiver: Send {
    fn recv(&mut self) -> WsRecv;
}

/// The two halves of an open WebSocket.
pub type WsPair = (Box<dyn WsSender>, Box<dyn WsReceiver>);

/// Opens a WebSocket to `url` requesting sub-protocol `protocol`.
pub trait WsConnector: Send + Sync {
    fn connect(&self, url: &str, protocol: &str) -> Result<WsPair, LinkError>;
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
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
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

/// Host and port out of `ws://host:port/...` (brackets kept off IPv6).
fn authority(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("ws://")?;
    let auth = rest.split('/').next()?;
    if let Some(v6) = auth.strip_prefix('[') {
        let (host, tail) = v6.split_once(']')?;
        let port = tail.strip_prefix(':').map(str::parse).transpose().ok()?.unwrap_or(80);
        return Some((host.to_owned(), port));
    }
    match auth.rsplit_once(':') {
        Some((h, p)) => Some((h.to_owned(), p.parse().ok()?)),
        None => Some((auth.to_owned(), 80)),
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

impl WsConnector for TcpConnector {
    fn connect(&self, url: &str, protocol: &str) -> Result<WsPair, LinkError> {
        let deadline = Instant::now() + self.connect_timeout;
        let (host, port) = authority(url).ok_or_else(|| LinkError::Io(format!("not a ws:// URL: {url}")))?;
        let addrs = resolve(&host, port, deadline)?;
        let tcp = connect_any(&addrs, deadline)?;
        tune(&tcp);
        tcp.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(|e| LinkError::Io(e.to_string()))?;

        let mut request = url.into_client_request().map_err(|e| LinkError::Handshake(e.to_string()))?;
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocol.parse().map_err(|_| LinkError::Handshake("bad sub-protocol".into()))?,
        );
        let wlock = Arc::new(Mutex::new(()));
        let write_half = tcp.try_clone().map_err(|e| LinkError::Io(e.to_string()))?;
        let broken = Arc::new(AtomicBool::new(false));
        let pipe = Pipe {
            tcp,
            buffered: false,
            out: Vec::new(),
            wlock: Arc::clone(&wlock),
            broken: Arc::clone(&broken),
            deadline: Some(deadline),
        };
        let (mut reader, response) =
            tungstenite::client::client_with_config(request, pipe, Some(config())).map_err(handshake_error)?;
        match response.headers().get("Sec-WebSocket-Protocol").and_then(|v| v.to_str().ok()) {
            Some(p) if p.eq_ignore_ascii_case(protocol) => {}
            other => log::warn!("dspi-link: hub did not echo sub-protocol {protocol} (got {other:?})"),
        }
        reader.get_mut().buffered = true;
        reader.get_mut().deadline = None;
        reader.get_ref().tcp.set_read_timeout(Some(READ_TICK)).map_err(|e| LinkError::Io(e.to_string()))?;
        let shutdown = write_half.try_clone().map_err(|e| LinkError::Io(e.to_string()))?;
        let writer = WebSocket::from_raw_socket(
            Pipe {
                tcp: write_half,
                buffered: true,
                out: Vec::new(),
                wlock,
                broken: Arc::clone(&broken),
                deadline: None,
            },
            Role::Client,
            Some(config()),
        );
        Ok((
            Box::new(TcpSender { ws: Mutex::new(writer), shutdown }),
            Box::new(TcpReceiver { ws: reader, close_code: None, broken }),
        ))
    }
}

struct TcpSender {
    ws: Mutex<WebSocket<Pipe>>,
    shutdown: TcpStream,
}

impl WsSender for TcpSender {
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
    broken: Arc<AtomicBool>,
}

impl WsReceiver for TcpReceiver {
    fn recv(&mut self) -> WsRecv {
        loop {
            // Read errors include our own failed pong: see Pipe::broken
            if self.broken.load(Ordering::SeqCst) {
                return WsRecv::Closed { code: self.close_code, reason: "write failed".into() };
            }
            match self.ws.read() {
                Ok(Message::Text(t)) => return WsRecv::Message(WsMessage::Text(t.as_str().to_owned())),
                Ok(Message::Binary(b)) => return WsRecv::Message(WsMessage::Binary(b.to_vec())),
                // tungstenite has already queued the reply to a ping
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => return WsRecv::Alive,
                Ok(Message::Frame(_)) => continue,
                Ok(Message::Close(frame)) => {
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
                    return WsRecv::Closed { code: self.close_code, reason: "closed".into() }
                }
                Err(e) => return WsRecv::Closed { code: self.close_code, reason: e.to_string() },
            }
        }
    }
}

// -- WP HB1 --
/// The two halves of a WebSocket a hub accepted, for its reader and its writer
/// threads: the same split as [`TcpConnector`]'s, with role server. The hub has read
/// and answered the upgrade itself, so this starts on the WebSocket's first byte.
/// `tail` is what arrived behind the upgrade request. `max_frame` limits a frame and
/// a message from the peer, checked before anything is buffered for it.
pub fn split_server(tcp: TcpStream, tail: Vec<u8>, max_frame: usize) -> io::Result<(WebSocket<Pipe>, WebSocket<Pipe>)> {
    let write_half = tcp.try_clone()?;
    let wlock = Arc::new(Mutex::new(()));
    let broken = Arc::new(AtomicBool::new(false));
    let pipe = |tcp, wlock, broken| Pipe { tcp, buffered: true, out: Vec::new(), wlock, broken, deadline: None };
    let config = WebSocketConfig::default()
        .max_message_size(Some(max_frame))
        .max_frame_size(Some(max_frame))
        .read_buffer_size(SERVER_READ_BUFFER);
    let reader = WebSocket::from_partially_read(
        pipe(tcp, Arc::clone(&wlock), Arc::clone(&broken)),
        tail,
        Role::Server,
        Some(config),
    );
    let writer = WebSocket::from_raw_socket(pipe(write_half, wlock, broken), Role::Server, Some(config));
    Ok((reader, writer))
}

/// Per socket and half, so an unauthenticated socket costs little.
const SERVER_READ_BUFFER: usize = 16 * 1024;

impl Pipe {
    /// A write failed part way, so nothing more can be sent on this stream. A read
    /// timeout means nothing while this is set: it may be that write's timeout.
    pub fn is_broken(&self) -> bool {
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
        let mut pipe = Pipe {
            tcp,
            buffered: true,
            out: Vec::new(),
            wlock: Arc::new(Mutex::new(())),
            broken: Arc::clone(&broken),
            deadline: None,
        };
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
