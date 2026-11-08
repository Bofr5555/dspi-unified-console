//! Independent hostile-LAN suite, written by review w2a rather than the hub's WPs.
//! Its own hostile client: raw TCP + WebSocket + hand-built JSON/binary frames,
//! not the WP's test helpers. Each test prints what it sent and what came back.
//! Run with: ci/run.sh 'cargo test -p dspi-hub --test hostile_lan -- --nocapture --test-threads=1'

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_hub::server::ServerLimits;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_link::Dir;
use dspi_link::frames::{CmdRequest, CmdResponse, Frame, kind, status};
use dspi_proto::generated::ext as extg;
use dspi_proto::generated::opcodes as op;
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use socket2::{Domain, Socket, Type};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

fn store_dir() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    // ci/run.sh points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("w2a-attack").join(format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Target {
    hub: Hub,
    net: dspi_hub::server::NetworkControl,
    port: u16,
}

fn start(access: AccessMode, limits: ServerLimits, web: bool) -> Target {
    let cfg = NetworkConfig {
        port: 0,
        access,
        web,
        bind: IpAddr::V4(LOOPBACK),
        limits,
        ..NetworkConfig::new(store_dir())
    };
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        ..HubConfig::in_process("Attack hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Target { hub, net, port }
}

fn fast() -> ServerLimits {
    ServerLimits {
        head_timeout: Duration::from_millis(400),
        auth_timeout: Duration::from_millis(600),
        ping_interval: Duration::from_millis(200),
        dead_peer: Duration::from_millis(800),
        write_stall: Duration::from_millis(400),
        close_grace: Duration::from_millis(200),
        read_tick: Duration::from_millis(20),
        ..ServerLimits::default()
    }
}

/// A raw TCP socket, optionally from a chosen loopback source (127.0.0.x), so the
/// per-address limits can be probed from several "machines".
fn raw_from(src: Option<Ipv4Addr>, port: u16) -> TcpStream {
    let s = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    if let Some(ip) = src {
        s.bind(&SocketAddr::new(IpAddr::V4(ip), 0).into()).unwrap();
    }
    s.connect(&SocketAddr::new(IpAddr::V4(LOOPBACK), port).into())
        .unwrap();
    let tcp: TcpStream = s.into();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp
}

fn read_http_head(tcp: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match tcp.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// The upgrade request this server wants. `origin` is attached verbatim when given.
fn upgrade_bytes(port: u16, origin: Option<&str>, path: &str) -> String {
    let mut r = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n"
    );
    if let Some(o) = origin {
        r.push_str(&format!("Origin: {o}\r\n"));
    }
    r.push_str("\r\n");
    r
}

/// Returns the HTTP status code of an upgrade attempt (no WebSocket wrap).
fn upgrade_status(port: u16, origin: Option<&str>) -> u16 {
    let mut tcp = raw_from(None, port);
    tcp.write_all(upgrade_bytes(port, origin, "/dspi/v1").as_bytes())
        .unwrap();
    let head = read_http_head(&mut tcp);
    head.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// A connected, hostile WebSocket client I drive by hand.
struct Cx {
    ws: WebSocket<TcpStream>,
}

impl Cx {
    fn open(port: u16, src: Option<Ipv4Addr>, origin: Option<&str>) -> Result<Self, u16> {
        let mut tcp = raw_from(src, port);
        tcp.write_all(upgrade_bytes(port, origin, "/dspi/v1").as_bytes())
            .unwrap();
        let head = read_http_head(&mut tcp);
        let code = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if code != 101 {
            return Err(code);
        }
        Ok(Self {
            ws: WebSocket::from_raw_socket(tcp, WsRole::Client, None),
        })
    }

    fn send_json(&mut self, v: &Value) {
        self.ws.send(Message::text(v.to_string())).unwrap();
    }

    fn send_text_raw(&mut self, s: &str) {
        self.ws.send(Message::text(s.to_owned())).unwrap();
    }

    fn send_bin(&mut self, bytes: Vec<u8>) {
        self.ws.send(Message::binary(bytes)).unwrap();
    }

    fn send_cmd(&mut self, req: &CmdRequest) {
        self.send_bin(Frame::CmdRequest(req.clone()).encode());
    }

    /// Next text or binary message, skipping pings/pongs. `None` on close/timeout.
    fn recv(&mut self) -> Option<Message> {
        for _ in 0..200 {
            match self.ws.read() {
                Ok(Message::Ping(p)) => {
                    let _ = self.ws.send(Message::Pong(p));
                }
                Ok(Message::Pong(_)) => {}
                Ok(m @ Message::Text(_)) | Ok(m @ Message::Binary(_)) => return Some(m),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    }

    /// Next JSON text, skipping interleaved binary POLL/NOTIFY frames the hub
    /// legitimately streams once a session polls or a device changes.
    fn recv_json(&mut self) -> Option<Value> {
        for _ in 0..500 {
            match self.recv()? {
                Message::Text(t) => return serde_json::from_str(t.as_str()).ok(),
                Message::Binary(b) => {
                    // Evidence for the reviewer: what the hub streamed between replies.
                    let k = b.first().copied().unwrap_or(0);
                    let name = match k {
                        0x02 => "NOTIFY",
                        0x03 => "POLL",
                        0x05 => "RESYNC",
                        0x81 => "CMD_RESPONSE",
                        _ => "?",
                    };
                    eprintln!("  (skipped binary frame kind={k:#04x} {name})");
                    continue;
                }
                _ => return None,
            }
        }
        None
    }

    /// The JSON reply carrying `id`, skipping broadcast events (`device.changed`
    /// and friends) the hub legitimately interleaves after a write.
    fn reply_for(&mut self, id: u64) -> Value {
        for _ in 0..50 {
            let v = self.recv_json().expect("a JSON message");
            if v["id"] == json!(id) {
                return v;
            }
            eprintln!("  (skipped interleaved event: t={})", v["t"]);
        }
        panic!("no reply with id {id}");
    }

    /// Pre-auth only: read the peer's Close via tungstenite. Safe pre-auth because
    /// the hub closes on the auth deadline regardless of pings.
    fn recv_close_code(&mut self) -> Option<(u16, Duration)> {
        let from = Instant::now();
        for _ in 0..1000 {
            match self.ws.read() {
                Ok(Message::Close(Some(f))) => return Some((u16::from(f.code), from.elapsed())),
                Ok(Message::Close(None)) => return Some((1005, from.elapsed())),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// Truly-silent peer: read the raw TCP bytes so tungstenite does NOT auto-send a
    /// pong (which `ws.read()` does and which would keep the connection alive). Returns
    /// (close code if the server sent one, else None=EOF-only, elapsed).
    fn recv_raw_close(&mut self) -> (Option<u16>, Duration) {
        let from = Instant::now();
        let s = self.ws.get_mut();
        let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            match s.read(&mut chunk) {
                Ok(0) | Err(_) => return (close_code_in(&buf), from.elapsed()),
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(c) = close_code_in(&buf) {
                        return (Some(c), from.elapsed());
                    }
                }
            }
        }
    }

    /// Make this peer vanish abruptly: shut the socket down both ways (FIN/RST).
    fn kill(&mut self) {
        let _ = self.ws.get_ref().shutdown(std::net::Shutdown::Both);
    }

    /// Read until a CmdResponse with this tag; returns it.
    fn await_resp(&mut self, tag: u16) -> Option<CmdResponse> {
        for _ in 0..200 {
            match self.recv()? {
                Message::Binary(b) => {
                    if let Ok(Frame::CmdResponse(r)) = Frame::decode(&b[..])
                        && r.tag == tag
                    {
                        return Some(r);
                    }
                }
                Message::Text(_) => {}
                _ => return None,
            }
        }
        None
    }

    fn hello(&mut self) -> Value {
        self.send_json(&json!({"t":"hello","proto":{"major":1,"minor":2},
            "client":{"name":"atk","app":"atk","version":"0"}}));
        self.recv_json().expect("hello reply")
    }

    /// Pair with a PIN; returns the ok/err reply.
    fn pair(&mut self, pin: &str, role: Option<&str>, name: &str) -> Value {
        let mut m = json!({"t":"auth.pair","id":1,"pin":pin,"name":name});
        if let Some(r) = role {
            m.as_object_mut().unwrap().insert("role".into(), r.into());
        }
        self.send_json(&m);
        self.recv_json().expect("pair reply")
    }
}

/// Scan server→client (unmasked) WebSocket bytes for a Close frame (opcode 0x8) and
/// return its 2-byte big-endian status code.
fn close_code_in(b: &[u8]) -> Option<u16> {
    let mut i = 0usize;
    while i + 1 < b.len() {
        let len = (b[i + 1] & 0x7f) as usize; // server frames are not masked, len < 126 here
        let payload = i + 2;
        if b[i] == 0x88 && len >= 2 && payload + 2 <= b.len() {
            return Some((u16::from(b[payload]) << 8) | u16::from(b[payload + 1]));
        }
        i = if payload + len <= b.len() {
            payload + len
        } else {
            i + 1
        };
    }
    None
}

fn set(code: u8, value: u16, payload: Vec<u8>) -> CmdRequest {
    CmdRequest {
        tag: 7,
        handle: 0,
        dir: Dir::Set,
        b_request: code,
        w_value: value,
        w_index: 2,
        w_length: payload.len() as u16,
        payload,
    }
}
fn get(code: u8, value: u16, len: u16) -> CmdRequest {
    CmdRequest {
        tag: 7,
        handle: 0,
        dir: Dir::Get,
        b_request: code,
        w_value: value,
        w_index: 2,
        w_length: len,
        payload: Vec::new(),
    }
}

/// Open a window and pair one client at `role`; returns (token, session reply).
fn paired(t: &Target, role: &str) -> (String, Value) {
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let mut c = Cx::open(t.port, None, None).unwrap();
    c.hello();
    let reply = c.pair(&w.pin, Some(role), "x");
    let token = reply["token"].as_str().unwrap_or("").to_owned();
    (token, reply)
}

// ─────────────────────────── A. PIN brute force ───────────────────────────

#[test]
fn atk_pin_bruteforce_across_reconnects_one_address() {
    let t = start(AccessMode::Pin, fast(), false);
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let wrong = if w.pin == "000000" {
        "000001"
    } else {
        "000000"
    };
    let mut outcomes = Vec::new();
    for i in 0..7 {
        // A *fresh* connection each attempt: reconnecting must not reset the ledger.
        let mut c = Cx::open(t.port, None, None).unwrap();
        c.hello();
        let r = c.pair(wrong, None, "x");
        outcomes.push(r["code"].as_str().unwrap_or("?").to_owned());
        eprintln!("attempt {i}: sent wrong PIN on a new socket -> {}", r);
    }
    // 5 answered unauthenticated, the 6th+ rate_limited.
    assert_eq!(&outcomes[..5], &["unauthenticated"; 5], "{outcomes:?}");
    assert!(
        outcomes[5..].iter().all(|c| c == "rate_limited"),
        "{outcomes:?}"
    );
    // And the right PIN no longer helps that address.
    let mut c = Cx::open(t.port, None, None).unwrap();
    c.hello();
    let r = c.pair(&w.pin, None, "x");
    eprintln!("right PIN from the blocked address -> {r}");
    assert_eq!(r["code"], "rate_limited");
    t.hub.stop();
}

#[test]
fn atk_pin_bruteforce_from_many_addresses_kills_the_window() {
    let t = start(AccessMode::Pin, fast(), false);
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let wrong = if w.pin == "000000" {
        "000001"
    } else {
        "000000"
    };
    for n in 0..5u8 {
        let src = Ipv4Addr::new(127, 0, 0, 2 + n);
        let mut c = Cx::open(t.port, Some(src), None).unwrap();
        c.hello();
        let r = c.pair(wrong, None, "x");
        eprintln!("wrong PIN from {src} -> {}", r["code"]);
        assert_eq!(r["code"], "unauthenticated", "from {src}");
    }
    // The window itself is spent after 5 wrong from anywhere: the right PIN,
    // from a brand-new address with no failures of its own, is refused.
    let fresh = Ipv4Addr::new(127, 0, 0, 50);
    let mut c = Cx::open(t.port, Some(fresh), None).unwrap();
    c.hello();
    let r = c.pair(&w.pin, None, "x");
    eprintln!("right PIN from a clean address {fresh} after 5 wrong elsewhere -> {r}");
    assert_eq!(r["code"], "unauthenticated", "the window must be dead");
    t.hub.stop();
}

// ─────────────────── D. handshake / auth state machine games ───────────────────

#[test]
fn atk_protocol_state_games() {
    let t = start(AccessMode::Pin, fast(), false);

    // A text request before hello/auth: unauthenticated, no crash.
    let mut c = Cx::open(t.port, None, None).unwrap();
    c.send_json(&json!({"t":"device.list","id":1}));
    let r = c.recv_json().expect("reply");
    eprintln!("device.list before hello -> {r}");
    assert_eq!(r["code"], "unauthenticated");

    // A binary CMD before auth: silently discarded (no answer). Prove the socket
    // still lives by then getting the unauthenticated answer to a text request.
    c.send_cmd(&get(op::REQ_GET_PLATFORM, 0, 8));
    c.send_json(&json!({"t":"device.snapshot","id":2,"handle":0}));
    let r = c.recv_json().expect("reply after discarded binary");
    eprintln!("snapshot before auth (after a discarded binary) -> {r}");
    assert_eq!(r["code"], "unauthenticated");
    drop(c);

    // Double hello then a second auth on an open session.
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let mut c = Cx::open(t.port, None, None).unwrap();
    let h1 = c.hello();
    let h2 = c.hello();
    eprintln!("hello #1 t={}, hello #2 t={}", h1["t"], h2["t"]);
    assert_eq!(h1["t"], "hello");
    assert_eq!(h2["t"], "hello", "a second hello is answered, not fatal");
    let ok = c.pair(&w.pin, Some("control"), "x");
    assert_eq!(ok["role"], "control");
    let session = ok["session"].clone();
    // A second auth.pair on the same session: bad_request, session stands.
    let w2 = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let again = c.pair(&w2.pin, Some("admin"), "x");
    eprintln!("second auth.pair on an authed session -> {again}");
    assert_eq!(again["code"], "bad_request");
    // The session is still the same one and still control (no escalation).
    c.send_json(&json!({"t":"device.snapshot","id":9,"handle":0}));
    let snap = c.recv_json().expect("snapshot");
    assert_eq!(snap["t"], "ok", "session still usable: {snap}");
    assert_eq!(session, ok["session"], "session id unchanged");
    t.hub.stop();
}

// ─────────────────── E/F. each role against every route ───────────────────

#[test]
fn atk_roles_against_every_route() {
    let t = start(AccessMode::Pin, fast(), false);
    for role in ["viewer", "control", "admin"] {
        let (_tok, reply) = paired(&t, role);
        // Reopen a driveable client for this client's token.
        let token = reply["token"].as_str().unwrap().to_owned();
        let mut c = Cx::open(t.port, None, None).unwrap();
        c.hello();
        c.send_json(&json!({"t":"auth.token","id":1,"token":token}));
        let authed = c.recv_json().unwrap();
        assert_eq!(
            authed["role"], role,
            "token re-auth kept the role: {authed}"
        );

        // -- binary CMD routes --
        // read (GET_PLATFORM): allowed for all.
        c.send_cmd(&get(op::REQ_GET_PLATFORM, 0, 8));
        let read = c.await_resp(7).unwrap();
        eprintln!("[{role}] GET_PLATFORM status={:#x}", read.status);
        assert_eq!(read.status, status::OK, "[{role}] read");
        // control write (SET_MASTER_VOLUME): viewer denied, others ok.
        c.send_cmd(&set(op::REQ_SET_MASTER_VOLUME, 0, vec![0, 0, 0, 0]));
        let vol = c.await_resp(7).unwrap();
        eprintln!("[{role}] SET_MASTER_VOLUME status={:#x}", vol.status);
        if role == "viewer" {
            assert_eq!(vol.status, status::DENIED, "[{role}] control write");
        } else {
            assert_ne!(vol.status, status::DENIED, "[{role}] control write");
        }
        // bulk SET_ALL_PARAMS (0xA1): the upstream table classes it CONTROL (as the
        // Swift reference does), so viewer denied, control and admin allowed.
        c.send_cmd(&set(op::REQ_SET_ALL_PARAMS, 0, vec![0u8; 64]));
        let bulk = c.await_resp(7).unwrap();
        eprintln!("[{role}] SET_ALL_PARAMS status={:#x}", bulk.status);
        if role == "viewer" {
            assert_eq!(bulk.status, status::DENIED, "viewer bulk");
        } else {
            assert_ne!(
                bulk.status,
                status::DENIED,
                "[{role}] bulk is control-class"
            );
        }
        // 0x00 both directions: admin only.
        for dir in [Dir::Get, Dir::Set] {
            let mut r = get(op::REQ_EXTENSION, extg::EXT_CROSSFEED_PAIR_MAP, 4);
            if dir == Dir::Set {
                r = set(
                    op::REQ_EXTENSION,
                    extg::EXT_CROSSFEED_PAIR_MAP,
                    vec![0, 1, 2, 3],
                );
            }
            c.send_cmd(&r);
            let resp = c.await_resp(7).unwrap();
            eprintln!("[{role}] 0x00 {dir:?} status={:#x}", resp.status);
            if role == "admin" {
                assert_ne!(resp.status, status::DENIED, "[{role}] 0x00 {dir:?}");
            } else {
                assert_eq!(resp.status, status::DENIED, "[{role}] 0x00 {dir:?}");
            }
        }

        // -- JSON routes --
        // snapshot: read, all roles.
        c.send_json(&json!({"t":"device.snapshot","id":10,"handle":0}));
        let snap = c.reply_for(10);
        assert_eq!(snap["t"], "ok", "[{role}] snapshot: {snap}");
        // poll of a READ opcode: granted for all.
        c.send_json(&json!({"t":"poll.subscribe","id":11,"handle":0,
            "polls":[{"slot":0,"req":op::REQ_GET_STATUS,"val":9,"idx":2,"len":41,"hz":5}]}));
        let poll = c.reply_for(11);
        assert_eq!(poll["t"], "ok", "[{role}] poll read: {poll}");
        // poll of a MUTATING opcode (SET_MASTER_VOLUME): denied for everyone.
        c.send_json(&json!({"t":"poll.subscribe","id":12,"handle":0,
            "polls":[{"slot":1,"req":op::REQ_SET_MASTER_VOLUME,"val":0,"idx":2,"len":4,"hz":5}]}));
        let badpoll = c.reply_for(12);
        eprintln!("[{role}] poll of SET_MASTER_VOLUME -> {badpoll}");
        assert_eq!(
            badpoll["code"], "denied",
            "[{role}] poll of a mutating opcode"
        );
        // rename: viewer denied, others ok.
        c.send_json(&json!({"t":"device.rename","id":13,"handle":0,"name":"Studio"}));
        let rn = c.reply_for(13);
        eprintln!("[{role}] device.rename -> {rn}");
        if role == "viewer" {
            assert_eq!(rn["code"], "denied", "viewer rename");
        } else {
            assert_eq!(rn["t"], "ok", "[{role}] rename: {rn}");
        }
        // lock: viewer may not lock (would block everyone), others ok.
        c.send_json(&json!({"t":"lock.acquire","id":14,"handle":0,"timeout_ms":1000}));
        let lk = c.reply_for(14);
        eprintln!("[{role}] lock.acquire -> {lk}");
        if role == "viewer" {
            assert_eq!(lk["code"], "denied", "viewer lock");
        } else {
            assert_eq!(lk["t"], "ok", "[{role}] lock: {lk}");
        }
        c.send_json(&json!({"t":"lock.release","id":15,"handle":0}));
        let _ = c.reply_for(15);
        // hub.* admin-only.
        c.send_json(&json!({"t":"hub.stats","id":16}));
        let hs = c.reply_for(16);
        eprintln!("[{role}] hub.stats -> {hs}");
        if role == "admin" {
            assert_eq!(
                hs["code"], "unsupported",
                "admin hub.stats is unsupported, not denied"
            );
        } else {
            assert_eq!(hs["code"], "denied", "[{role}] hub.stats");
        }
        // auth.list / auth.revoke admin-only.
        c.send_json(&json!({"t":"auth.list","id":17}));
        let al = c.reply_for(17);
        if role == "admin" {
            assert_eq!(al["t"], "ok", "admin auth.list");
        } else {
            assert_eq!(al["code"], "denied", "[{role}] auth.list");
        }
    }
    t.hub.stop();
}

// ─────────────────── G. the voltage rule from the network ───────────────────

#[test]
fn atk_voltage_rule_every_route() {
    let t = start(AccessMode::Pin, fast(), false);
    let v = extg::EXT_CORE_VREG;
    // demo build default is 1.20 V (0x0D). A raise to 1.25 (0x0E) must be refused.
    for role in ["admin", "control", "viewer"] {
        let (_tok, reply) = paired(&t, role);
        let mut c = Cx::open(t.port, None, None).unwrap();
        c.hello();
        c.send_json(&json!({"t":"auth.token","id":1,"token":reply["token"]}));
        let _ = c.recv_json();
        // Admin is the only role even allowed to reach 0x00; viewer/control are
        // DENIED at the policy gate. For admin, the voltage rule refuses the raise.
        let probes: &[(&str, Vec<u8>)] = &[
            ("Set 1.25", vec![0, 0x0E]),
            ("Trial 1.25", vec![1, 0x0E]),
            ("Set build-default on 1.20", vec![0, 0x00]), // SET 0: current known 1.20 -> allowed
            ("malformed len 1", vec![0x00]),
            ("malformed len 3", vec![0, 0x0E, 0x00]),
            ("unknown subop", vec![9, 0x0E]),
            ("non-voltage code", vec![1, 0x40]),
        ];
        for (name, payload) in probes {
            let req = set(op::REQ_EXTENSION, v, payload.clone());
            c.send_cmd(&req);
            let r = c.await_resp(7).unwrap();
            eprintln!(
                "[{role}] vreg {name} payload={payload:?} -> status={:#x}",
                r.status
            );
            if role != "admin" {
                assert_eq!(
                    r.status,
                    status::DENIED,
                    "[{role}] {name} must be denied pre-0x00"
                );
                continue;
            }
            match *name {
                "Set build-default on 1.20" => {
                    assert_ne!(
                        r.status,
                        status::DENIED,
                        "SET 0 on a readable 1.20 unit is a level/down move"
                    );
                }
                _ => assert_eq!(
                    r.status,
                    status::DENIED,
                    "admin {name} raise/garbage must be DENIED"
                ),
            }
        }
    }
    // A bulk write cannot carry a voltage raise: SET_ALL_PARAMS is not REQ_EXTENSION,
    // so the vreg path never sees it, and there is no vreg field in the blob. Admin
    // bulk (even a split one) returns a device status, never a voltage change path.
    let (_tok, reply) = paired(&t, "admin");
    let mut c = Cx::open(t.port, None, None).unwrap();
    c.hello();
    c.send_json(&json!({"t":"auth.token","id":1,"token":reply["token"]}));
    let _ = c.recv_json();
    for (i, chunk) in [vec![0u8; 2048], vec![0u8; 2048]].into_iter().enumerate() {
        let mut r = set(op::REQ_SET_ALL_PARAMS_CHUNK, (i * 2048) as u16, chunk);
        r.tag = 20 + i as u16;
        c.send_cmd(&r);
        let resp = c.await_resp(20 + i as u16).unwrap();
        eprintln!("admin split-bulk chunk {i} -> status={:#x}", resp.status);
        assert_ne!(resp.status, status::DENIED, "admin bulk chunk denied?");
    }
    t.hub.stop();
}

// ─────────────────── I. forged attribution (client-sent control frames) ───────────────────

#[test]
fn atk_forged_control_frames_are_ignored() {
    let t = start(AccessMode::Pin, fast(), false);
    let (_tok, reply) = paired(&t, "admin");
    let mut c = Cx::open(t.port, None, None).unwrap();
    c.hello();
    c.send_json(&json!({"t":"auth.token","id":1,"token":reply["token"]}));
    let _ = c.recv_json();
    // Inject hub->client frame kinds from the client: NOTIFY, POLL, RESYNC, response.
    for kindb in [kind::NOTIFY, kind::POLL, kind::RESYNC, kind::CMD_RESPONSE] {
        c.send_bin(vec![kindb, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    // The session must still be alive and correct: a real read still answers.
    c.send_cmd(&get(op::REQ_GET_PLATFORM, 0, 8));
    let r = c
        .await_resp(7)
        .expect("session survived injected control frames");
    eprintln!(
        "after injecting NOTIFY/POLL/RESYNC/RESPONSE, GET_PLATFORM -> {:#x}",
        r.status
    );
    assert_eq!(r.status, status::OK);
    t.hub.stop();
}

// ─────────────────── J. Origin / DNS rebinding ───────────────────

#[test]
fn atk_origin_and_rebinding() {
    // web OFF: any Origin header is refused; a native client (no Origin) gets in.
    let t = start(AccessMode::Pin, fast(), false);
    assert_eq!(upgrade_status(t.port, None), 101, "native client");
    for o in [
        "http://127.0.0.1",
        "http://evil.example",
        "null",
        &format!("http://127.0.0.1:{}", t.port), // even our own name: web is off
        &format!("http://127.0.0.1:{}.evil.example", t.port),
    ] {
        let code = upgrade_status(t.port, Some(o));
        eprintln!("web-off Origin {o:?} -> {code}");
        assert_eq!(code, 403, "web off must refuse any Origin: {o}");
    }
    t.hub.stop();

    // web ON: our own page passes, a foreign one and a rebind (Host lies, Origin foreign) 403.
    let t = start(AccessMode::Pin, fast(), true);
    assert_eq!(
        upgrade_status(t.port, Some(&format!("http://127.0.0.1:{}", t.port))),
        101
    );
    for o in [
        "http://evil.example".to_string(),
        format!("http://127.0.0.1:{}", t.port.wrapping_add(1)),
        format!("https://127.0.0.1:{}", t.port),
        format!("http://attacker.127.0.0.1:{}", t.port),
    ] {
        let code = upgrade_status(t.port, Some(&o));
        eprintln!("web-on Origin {o:?} -> {code}");
        assert_eq!(code, 403, "foreign/rebind origin: {o}");
    }
    t.hub.stop();
}

// ─────────────────── K. hostile JSON ───────────────────

#[test]
fn atk_hostile_json() {
    let t = start(AccessMode::Pin, fast(), false);
    // Each on its own socket: a protocol-level close must not harm anyone else.
    let depth = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let huge = r#"{"t":"auth.pair","id":99999999999999999999999999,"pin":"000000"}"#;
    let dup = r#"{"t":"hello","t":"auth.pair","pin":"000000"}"#;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("depth bomb", depth.into_bytes()),
        ("huge number id", huge.as_bytes().to_vec()),
        ("duplicate keys", dup.as_bytes().to_vec()),
        (
            "invalid utf8",
            vec![0x7b, 0x22, 0x74, 0x22, 0x3a, 0xff, 0xfe, 0x22, 0x7d],
        ),
        ("not json", b"garbage~~~".to_vec()),
        ("bare array", b"[1,2,3]".to_vec()),
    ];
    for (name, bytes) in cases {
        let mut c = Cx::open(t.port, None, None).unwrap();
        // Send as a text frame if valid utf8, else binary (binary pre-auth is dropped).
        match std::str::from_utf8(&bytes) {
            Ok(s) => c.send_text_raw(s),
            Err(_) => c.send_bin(bytes.clone()),
        }
        let reply = c.recv();
        eprintln!(
            "hostile JSON {name:?}: reply = {:?}",
            reply.as_ref().map(|m| matches!(m, Message::Text(_)))
        );
        // No hang, no panic. A malformed text closes (None) or is answered; fine either way.
        drop(c);
    }
    // The hub is still healthy: a fresh client can pair and read.
    let (_tok, reply) = paired(&t, "admin");
    assert_eq!(
        reply["t"], "ok",
        "hub survived every hostile message: {reply}"
    );
    t.hub.stop();
}

// ─────────────────── L. connection exhaustion; good session stays usable ───────────────────

#[test]
fn atk_connection_exhaustion_bounded_and_good_session_survives() {
    // Default keepalive (30 s dead-peer): the point is the flood, not whether a
    // client that stops pumping its socket for 800 ms is declared dead (it is,
    // correctly; the first run proved that).
    let limits = ServerLimits {
        max_connections: 8,
        max_preauth_per_peer: 8,
        close_grace: Duration::from_millis(200),
        ..ServerLimits::default()
    };
    let t = start(AccessMode::Pin, limits, false);
    // One legitimate, authenticated session first.
    let (_tok, reply) = paired(&t, "admin");
    let mut good = Cx::open(t.port, None, None).unwrap();
    good.hello();
    good.send_json(&json!({"t":"auth.token","id":1,"token":reply["token"]}));
    assert_eq!(good.recv_json().unwrap()["t"], "ok");

    // Flood the rest of the slots and then some. Hold the sockets open.
    let mut held = Vec::new();
    let mut refusals = 0;
    for _ in 0..40 {
        match Cx::open(t.port, None, None) {
            Ok(c) => held.push(c),
            Err(503) => refusals += 1,
            Err(code) => panic!("unexpected upgrade status {code}"),
        }
    }
    // -- WP HB9 -- WebSockets: a 503's plain HTTP socket may still be closing.
    let live = t.net.websockets();
    let threads = std::fs::read_dir("/proc/self/task")
        .map(|d| d.count())
        .unwrap_or(0);
    eprintln!(
        "exhaustion: held={} refused(503)={} net.websockets()={} process_threads={}",
        held.len(),
        refusals,
        live,
        threads
    );
    assert!(refusals > 0, "the limit must refuse someone");
    assert!(live <= 8, "connections bounded by max_connections: {live}");

    // The good session is still usable during the flood, and fast: its command
    // takes the per-device worker queue, which the pre-auth flood sockets (each on
    // its own reader thread, no session, no device access) never touch. Measure the
    // round-trip to show there is no shared-queue/lock starvation.
    let t0 = Instant::now();
    good.send_cmd(&get(op::REQ_GET_PLATFORM, 0, 8));
    let r = good
        .await_resp(7)
        .expect("good session answers during a flood");
    let rtt = t0.elapsed();
    eprintln!(
        "good session during flood: GET_PLATFORM -> {:#x} in {rtt:?}",
        r.status
    );
    assert_eq!(r.status, status::OK);
    assert!(rtt < Duration::from_secs(2), "no starvation: {rtt:?}");
    t.hub.stop();
}

// ─────────────────── M. hostile device text into JSON/logs ───────────────────

#[test]
fn atk_hostile_device_name_into_json() {
    let t = start(AccessMode::Pin, fast(), false);
    let (_tok, reply) = paired(&t, "admin");
    let mut c = Cx::open(t.port, None, None).unwrap();
    c.hello();
    c.send_json(&json!({"t":"auth.token","id":1,"token":reply["token"]}));
    let _ = c.recv_json();
    // A name with quotes, control bytes, a RTL override and an overlong run.
    let nasty = "ev\"il\u{0007}\u{202E}name\u{0000}\n".to_string() + &"z".repeat(5000);
    c.send_json(&json!({"t":"device.rename","id":1,"handle":0,"name":nasty}));
    // The rename also broadcasts `device.changed` to every session; read until
    // the reply that carries OUR id, so an interleaved event is not mistaken for it.
    let reply_with = |c: &mut Cx, id: u64| -> Value {
        for _ in 0..50 {
            let v = c.recv_json().expect("a JSON message");
            if v["id"] == json!(id) {
                return v;
            }
            eprintln!("  (skipped interleaved event: t={})", v["t"]);
        }
        panic!("no reply with id {id}");
    };
    let rn = reply_with(&mut c, 1);
    eprintln!("rename with hostile text -> {}", rn["t"]);
    // device.list must still be well-formed JSON with a cleaned name.
    c.send_json(&json!({"t":"device.list","id":2}));
    let list = reply_with(&mut c, 2);
    eprintln!("device.list after hostile rename -> {list}");
    assert_eq!(list["t"], "ok");
    let name = list["devices"][0]["name"].as_str().unwrap_or("");
    assert!(
        !name.contains('\u{0000}') && !name.contains('\u{0007}') && !name.contains('\u{202E}'),
        "control/override chars must be stripped: {name:?}"
    );
    assert!(
        name.chars().count() <= 64,
        "name length capped: {}",
        name.chars().count()
    );
    t.hub.stop();
}

// ─────────────── N. rate-limit evasion by rotating addresses ───────────────

/// The two limiters are independent: a per-address block AND a per-window cap of 5
/// failures from anywhere. An attacker who spreads guesses so no single address is
/// blocked still only gets 5 against one window — the window dies, and the right
/// PIN from an *unblocked* address is then refused `unauthenticated` (not
/// `rate_limited`), proving the window cap, not just the address block, stops it.
/// (The per-address block's 60 s lapse cannot be waited out from a real clock in a
/// unit test; the window cap makes waiting it out pointless anyway, which is the
/// evasion the attacker would attempt.)
#[test]
fn atk_ratelimit_evasion_rotating_addresses() {
    let t = start(AccessMode::Pin, fast(), false);
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let wrong = if w.pin == "000000" {
        "000001"
    } else {
        "000000"
    };
    // 4 wrong from A (A not yet blocked: block is at 5), 1 wrong from B.
    let a = Ipv4Addr::new(127, 0, 0, 2);
    let b = Ipv4Addr::new(127, 0, 0, 3);
    for i in 0..4 {
        let mut c = Cx::open(t.port, Some(a), None).unwrap();
        c.hello();
        let r = c.pair(wrong, None, "x");
        eprintln!("A wrong #{i} -> {}", r["code"]);
        assert_eq!(r["code"], "unauthenticated");
    }
    let mut c = Cx::open(t.port, Some(b), None).unwrap();
    c.hello();
    let r = c.pair(wrong, None, "x");
    eprintln!("B wrong #0 (5th against the window) -> {}", r["code"]);
    assert_eq!(r["code"], "unauthenticated");
    // A has only 4 failures (not blocked), yet the window is spent: the right PIN
    // from A is refused as a wrong PIN, not as rate-limited.
    let mut c = Cx::open(t.port, Some(a), None).unwrap();
    c.hello();
    let r = c.pair(&w.pin, None, "x");
    eprintln!("right PIN from the unblocked A after the window hit 5 -> {r}");
    assert_eq!(
        r["code"], "unauthenticated",
        "the window cap must stop it, and A is not blocked"
    );
    t.hub.stop();
}

// ─────────────── O. slowloris (my own, head and pre-auth silence) ───────────────

#[test]
fn atk_slowloris_own() {
    let t = start(AccessMode::Pin, fast(), false); // head_timeout 400ms, auth_timeout 600ms

    // (1) Dribble the HTTP head one byte at a time and stall past head_timeout.
    let mut tcp = raw_from(None, t.port);
    let req = upgrade_bytes(t.port, None, "/dspi/v1");
    let partial = &req.as_bytes()[..20]; // "GET /dspi/v1 HTTP/1.1"
    for byte in partial {
        let _ = tcp.write_all(&[*byte]);
        let _ = tcp.flush();
        std::thread::sleep(Duration::from_millis(40));
    }
    // Now stall. The hub must give up on the incomplete head and close.
    let t0 = Instant::now();
    let mut sink = [0u8; 64];
    let head = read_http_head(&mut tcp);
    let closed = {
        tcp.set_read_timeout(Some(Duration::from_secs(2))).ok();
        matches!(tcp.read(&mut sink), Ok(0) | Err(_))
    };
    eprintln!(
        "slowloris head: after {:?} server head={:?} socket_closed={closed}",
        t0.elapsed(),
        head.lines().next()
    );
    assert!(
        head.is_empty() || head.starts_with("HTTP/1.1 4") || closed,
        "an unfinished head must not hold the slot open: {head:?}"
    );

    // (2) Finish the upgrade but then send nothing (a silent authenticated-less peer):
    // the hub closes with 4001 within auth_timeout.
    let mut c = Cx::open(t.port, None, None).unwrap();
    let t0 = Instant::now();
    let close = c.recv_close_code();
    eprintln!("silent pre-auth peer: close={close:?}");
    match close {
        Some((code, took)) => {
            assert_eq!(code, 4001, "silent peer must close 4001");
            assert!(
                took < Duration::from_secs(3),
                "within auth_timeout: {took:?}"
            );
            let _ = t0;
        }
        None => panic!("the hub must close a silent pre-auth peer, not hold it"),
    }
    t.hub.stop();
}

// ─────────────── P. a lock held through a reconnect is released ───────────────

/// The holder vanishing (socket dropped) must free the device lock, so a lock cannot
/// be "held through" a disconnect/reconnect. A acquires; B is LOCKED; A's socket is
/// dropped; B can then take the lock. Proves `session_closed` releases it.
#[test]
fn atk_lock_not_held_through_reconnect() {
    let t = start(AccessMode::Pin, fast(), false);
    let (_ta, ra) = paired(&t, "control");
    let (_tb, rb) = paired(&t, "control");

    let mut a = Cx::open(t.port, None, None).unwrap();
    a.hello();
    a.send_json(&json!({"t":"auth.token","id":1,"token":ra["token"]}));
    let _ = a.reply_for(1);
    let mut b = Cx::open(t.port, None, None).unwrap();
    b.hello();
    b.send_json(&json!({"t":"auth.token","id":1,"token":rb["token"]}));
    let _ = b.reply_for(1);

    a.send_json(&json!({"t":"lock.acquire","id":2,"handle":0,"timeout_ms":60000}));
    let la = a.reply_for(2);
    eprintln!("A lock -> {la}");
    assert_eq!(la["t"], "ok");
    // B is refused the lock and a write while A holds it.
    b.send_cmd(&set(op::REQ_SET_MASTER_VOLUME, 0, vec![0, 0, 0, 0]));
    let bw = b.await_resp(7).unwrap();
    eprintln!("B write while A holds the lock -> {:#x}", bw.status);
    assert_eq!(bw.status, status::LOCKED, "A's lock must exclude B");

    // A vanishes abruptly: shut its socket down (FIN/RST), no graceful close. Its
    // session must close and free the lock. Diagnostics show whether A's connection
    // is actually gone (net.connections drops) to tell a hub bug from a harness one.
    let before = t.net.connections();
    a.kill();
    drop(a);
    let mut ok = false;
    for attempt in 0..40 {
        std::thread::sleep(Duration::from_millis(200));
        b.send_cmd(&set(op::REQ_SET_MASTER_VOLUME, 0, vec![0, 0, 0, 0]));
        let r = b.await_resp(7).unwrap();
        if r.status != status::LOCKED {
            let conns_when_freed = t.net.connections();
            eprintln!(
                "B write after A vanished (attempt {attempt}, ~{}ms) -> {:#x}; net.connections {before}->{conns_when_freed}",
                (attempt + 1) * 200,
                r.status
            );
            ok = r.status == status::OK;
            break;
        }
    }
    assert!(
        ok,
        "A's lock must be released when its connection dies, not held through it (conns {before}->{})",
        t.net.connections()
    );
    t.hub.stop();
}

// ─────────────── Probe: WHY run 1's exhaustion test lost its good session ───────────────
// Run 1 used `fast()` (dead_peer 800 ms, ping 200 ms) and did not read the good
// socket during the flood. This reproduces exactly that and reads the hub's answer:
// the connection's own reader thread (`hub-conn-rx-N`, server.rs Connection::run)
// saw `last_heard` exceed dead_peer and sent close 1001 (keepalive). The session
// waited on no hub queue; it was dropped as a dead peer because MY client went silent.
#[test]
fn probe_run1_exhaustion_loss_was_a_keepalive_drop_not_a_stall() {
    let t = start(AccessMode::Pin, fast(), false); // dead_peer 800 ms, ping 200 ms
    let (_tok, reply) = paired(&t, "admin");
    let mut good = Cx::open(t.port, None, None).unwrap();
    good.hello();
    good.send_json(&json!({"t":"auth.token","id":1,"token":reply["token"]}));
    let _ = good.reply_for(1);
    // Go silent (raw read, so tungstenite does NOT auto-pong for us). The hub closes
    // us with 1001 after ~dead_peer (800 ms). This is the exact mechanism that made
    // run 1's good session disappear: a keepalive drop of MY silent client, not a
    // queue/lock stall on the hub side.
    let (code, took) = good.recv_raw_close();
    eprintln!("silent authed peer under fast() -> code={code:?} after {took:?}");
    assert!(
        took < Duration::from_secs(2),
        "dropped within dead_peer: {took:?}"
    );
    assert_eq!(code, Some(1001), "the drop is keepalive 1001");
    t.hub.stop();
}

// ─────────── Q. WP HB9: page loads apart from the WebSocket pre-auth budget ───────────

const BIG: usize = 32 << 20;

/// A console dist with a file far bigger than any socket buffer, for readers that stall.
fn console_dist() -> std::path::PathBuf {
    let dir = store_dir().with_extension("dist");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        r#"<head><meta name="dspi-hub" content="" /><meta name="dspi-hub-page" content="" /></head>"#,
    )
    .unwrap();
    std::fs::write(dir.join("app.js"), b"console.log(1);").unwrap();
    std::fs::write(dir.join("big.js"), vec![b'x'; BIG]).unwrap();
    dir
}

fn start_console(access: AccessMode, limits: ServerLimits) -> Target {
    let console = dspi_hub::web::Console::load(&console_dist()).unwrap();
    let cfg = NetworkConfig {
        port: 0,
        access,
        web: true,
        bind: IpAddr::V4(LOOPBACK),
        limits,
        console: Some(std::sync::Arc::new(console)),
        ..NetworkConfig::new(store_dir())
    };
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        ..HubConfig::in_process("Attack hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Target { hub, net, port }
}

fn get_text(port: u16, path: &str, connection: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: {connection}\r\n\r\n")
}

/// One response on a socket that may stay open: status, head, its `Content-Length` body.
fn read_response(tcp: &mut TcpStream) -> (u16, String, Vec<u8>) {
    let head = read_http_head(tcp);
    let code = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let length = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().to_owned())
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    tcp.read_exact(&mut body).unwrap();
    (code, head, body)
}

/// Whether the hub has closed this socket, without waiting for more than a moment.
fn closed_by_hub(tcp: &mut TcpStream) -> bool {
    tcp.set_read_timeout(Some(Duration::from_millis(5)))
        .unwrap();
    let mut byte = [0u8; 1];
    match tcp.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => !matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
    }
}

/// HB9 (a), review c6r WB7-1: one dual-stack machine is two peer keys (its IPv4 and its
/// IPv6 /64; two loopback addresses here). Each holds every unauthenticated WebSocket it
/// may, half its plain HTTP share in responses it never reads, and opens three times its
/// share of silent sockets on top. A third address still reads `/dspi/v1/info`, pairs
/// with the PIN, and loads a page. With one budget for both (WB7), the same two
/// addresses held all 16 slots and the third was answered 503.
#[test]
fn atk_hb9_a_dual_stack_host_cannot_keep_a_third_peer_from_pairing() {
    let limits = ServerLimits {
        close_grace: Duration::from_millis(200),
        ..ServerLimits::default()
    };
    let (http_share, ws_share) = (limits.max_http_per_peer, limits.max_preauth_per_peer);
    let t = start_console(AccessMode::Pin, limits);
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let mut sockets = Vec::new();
    let mut websockets = Vec::new();
    for ip in [Ipv4Addr::new(127, 0, 0, 2), Ipv4Addr::new(127, 0, 0, 3)] {
        for _ in 0..ws_share {
            websockets.push(Cx::open(t.port, Some(ip), None).unwrap());
        }
        assert_eq!(
            Cx::open(t.port, Some(ip), None).err(),
            Some(503),
            "{ip}: past its WebSocket share"
        );
        for _ in 0..http_share / 2 {
            let mut tcp = raw_from(Some(ip), t.port);
            tcp.write_all(get_text(t.port, "/console/big.js", "keep-alive").as_bytes())
                .unwrap();
            sockets.push(tcp);
        }
        for _ in 0..http_share * 3 {
            sockets.push(raw_from(Some(ip), t.port));
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    let (live, ws) = (t.net.connections(), t.net.websockets());
    eprintln!(
        "two addresses opened {} sockets and {} WebSockets: connections()={live} websockets()={ws}",
        sockets.len(),
        websockets.len()
    );
    assert_eq!(ws, 2 * ws_share);
    assert!(live <= 2 * (http_share + ws_share), "{live}");

    let third = Ipv4Addr::new(127, 0, 0, 4);
    let mut info = raw_from(Some(third), t.port);
    info.write_all(get_text(t.port, "/dspi/v1/info", "close").as_bytes())
        .unwrap();
    let (code, _, _) = read_response(&mut info);
    eprintln!("the third address's GET /dspi/v1/info -> {code}");
    assert_eq!(code, 200);
    let mut cx = Cx::open(t.port, Some(third), None).expect("the third address's upgrade");
    cx.hello();
    let reply = cx.pair(&w.pin, Some("admin"), "third");
    eprintln!("the third address pairs -> {}", reply["t"]);
    assert_eq!(reply["t"], "ok", "{reply}");
    let mut page = raw_from(Some(third), t.port);
    page.write_all(get_text(t.port, "/console/", "keep-alive").as_bytes())
        .unwrap();
    assert_eq!(read_response(&mut page).0, 200);
    drop((sockets, websockets));
    t.hub.stop();
}

/// HB9 (b): slowloris on the static GETs. A head trickled a byte at a time is cut
/// `request_timeout` after its first byte, long before `head_timeout`; a kept-alive
/// socket answers twice, then closes `keepalive_idle` after its last answer.
#[test]
fn atk_hb9_slowloris_heads_and_idle_keepalive_are_cut() {
    let limits = ServerLimits {
        head_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_millis(400),
        keepalive_idle: Duration::from_millis(400),
        close_grace: Duration::from_millis(200),
        ..ServerLimits::default()
    };
    let t = start_console(AccessMode::Pin, limits);

    let mut tcp = raw_from(None, t.port);
    let request = get_text(t.port, "/console/app.js", "keep-alive");
    let t0 = Instant::now();
    let mut cut = None;
    for byte in request.bytes().take(60) {
        if tcp.write_all(&[byte]).is_err() || closed_by_hub(&mut tcp) {
            cut = Some(t0.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    eprintln!("a head trickled at 25 bytes/s: cut after {cut:?}");
    let cut = cut.expect("the hub must cut a trickled head");
    assert!(cut < Duration::from_millis(1500), "{cut:?}");

    let mut tcp = raw_from(None, t.port);
    for path in ["/landing.css", "/console/app.js"] {
        tcp.write_all(get_text(t.port, path, "keep-alive").as_bytes())
            .unwrap();
        let (code, head, _) = read_response(&mut tcp);
        assert_eq!(code, 200, "{path}");
        assert!(head.contains("Keep-Alive: timeout="), "{head}");
    }
    let t0 = Instant::now();
    while !closed_by_hub(&mut tcp) {
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "an idle kept-alive socket stays"
        );
    }
    eprintln!("an idle kept-alive socket: closed after {:?}", t0.elapsed());
    // Asked to close, or not asked to stay: closed behind the answer, as before.
    for connection in ["close", "Upgrade"] {
        let mut tcp = raw_from(None, t.port);
        tcp.write_all(get_text(t.port, "/landing.css", connection).as_bytes())
            .unwrap();
        let (code, head, _) = read_response(&mut tcp);
        assert_eq!(code, 200);
        assert!(head.contains("Connection: close"), "{head}");
    }
    t.hub.stop();
}

/// HB9 (b): slow readers of a big file. One that stops reading frees its slot a few
/// `write_stall`s after its last progress (a write that took some bytes may wait out its
/// timeout before it returns), well before `response_timeout`; one that keeps taking
/// some is cut at `response_timeout`. Neither holds it for `auth_timeout` (30 s from
/// open, under WB7).
#[test]
fn atk_hb9_slow_readers_of_a_big_file_are_cut() {
    let limits = ServerLimits {
        write_stall: Duration::from_millis(400),
        response_timeout: Duration::from_secs(3),
        close_grace: Duration::from_millis(200),
        ..ServerLimits::default()
    };
    let t = start_console(AccessMode::Pin, limits);

    let mut stopped = raw_from(None, t.port);
    stopped
        .write_all(get_text(t.port, "/console/big.js", "keep-alive").as_bytes())
        .unwrap();
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(t.net.connections(), 1);
    while t.net.connections() > 0 {
        assert!(
            t0.elapsed() < Duration::from_millis(2400),
            "not by the stall"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!(
        "a reader that stops: its slot free after {:?}",
        t0.elapsed()
    );
    let mut got = Vec::new();
    stopped
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _ = stopped.read_to_end(&mut got);
    assert!(got.len() < BIG, "{}", got.len());

    let mut trickle = raw_from(None, t.port);
    trickle
        .write_all(get_text(t.port, "/console/big.js", "keep-alive").as_bytes())
        .unwrap();
    let t0 = Instant::now();
    let mut got = 0usize;
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        match trickle.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let took = t0.elapsed();
    eprintln!("a reader at up to 6 MB/s: cut after {took:?} and {got} of {BIG} bytes");
    assert!(got < BIG, "{got}");
    assert!(
        took >= Duration::from_millis(2800) && took < Duration::from_secs(6),
        "{took:?}"
    );
    t.hub.stop();
}

/// HB9: a kept-alive socket is plain HTTP until its request upgrades, and only then counts
/// in the WebSocket budget: a page's socket can carry its files and then the page's
/// WebSocket.
#[test]
fn atk_hb9_a_kept_alive_socket_upgrades_into_the_websocket_budget() {
    let t = start_console(AccessMode::Open, fast());
    let mut tcp = raw_from(None, t.port);
    tcp.write_all(get_text(t.port, "/console/", "keep-alive").as_bytes())
        .unwrap();
    assert_eq!(read_response(&mut tcp).0, 200);
    assert_eq!((t.net.connections(), t.net.websockets()), (1, 0));
    tcp.write_all(upgrade_bytes(t.port, None, "/dspi/v1").as_bytes())
        .unwrap();
    let head = read_http_head(&mut tcp);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert_eq!((t.net.connections(), t.net.websockets()), (1, 1));
    let mut cx = Cx {
        ws: WebSocket::from_raw_socket(tcp, WsRole::Client, None),
    };
    assert_eq!(cx.hello()["t"], "hello");
    t.hub.stop();
}
