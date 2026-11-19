//! WP HB1's done criteria, and what a hostile client on the LAN can try, on real
//! sockets. Loopback only. The limits are shortened, nothing else is changed.
#![allow(clippy::arithmetic_side_effects)]

use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_hub::server::{MAX_FRAME, MAX_HEAD, NetworkControl, ServerLimits};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_link::frames::{CmdRequest, Frame, kind};
use dspi_link::messages::{Inbound, PollSpec};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions, Dir, LinkClient, LinkError, status};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps, TransportError,
};
use serde_json::{Value, json};
use socket2::{Domain, SockAddr, Socket, Type};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

// -- review secb -- the TLS mode these tests run against: `off`, or under the
// test-only feature `test-tls-mode` what DSPI_HUB_TEST_TLS asks for. Where TLS is
// served, an `https` own Origin is the hub's page too, `/info` says `tls`, and a
// ClientHello is a TLS handshake, not a bad request.
fn tls_served() -> bool {
    NetworkConfig::new(std::path::PathBuf::new()).tls != dspi_hub::TlsMode::Off
}

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const WAIT: Duration = Duration::from_secs(10);
const HELLO: &str = r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"raw","app":"test","version":"0"}}"#;

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hb1").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// Short enough to wait out, long enough for a loaded build machine to do the work
/// inside them.
fn quick() -> ServerLimits {
    ServerLimits {
        head_timeout: Duration::from_millis(300),
        auth_timeout: Duration::from_millis(500),
        ping_interval: Duration::from_millis(100),
        dead_peer: Duration::from_millis(600),
        write_stall: Duration::from_millis(300),
        close_grace: Duration::from_millis(200),
        read_tick: Duration::from_millis(20),
        ..ServerLimits::default()
    }
}

struct Bench {
    hub: Hub,
    net: NetworkControl,
    port: u16,
    dir: PathBuf,
}

fn network(dir: PathBuf, access: AccessMode, limits: ServerLimits) -> NetworkConfig {
    NetworkConfig {
        port: 0,
        access,
        bind: LOOPBACK,
        limits,
        ..NetworkConfig::new(dir)
    }
}

fn start(config: NetworkConfig) -> Bench {
    let dir = config.store_dir.clone();
    let hub = Hub::start(HubConfig {
        network: Some(config),
        ..HubConfig::in_process("Test hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Bench {
        hub,
        net,
        port,
        dir,
    }
}

fn bench(access: AccessMode, limits: ServerLimits) -> Bench {
    start(network(store_dir(), access, limits))
}

fn pin_hub() -> Bench {
    bench(AccessMode::Pin, ServerLimits::default())
}

impl Bench {
    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/dspi/v1", self.port)
    }

    fn pin(&self) -> String {
        self.net.auth().begin_pairing(Instant::now()).unwrap().pin
    }

    /// -- WP HWLK1F, PAIRADM -- a PIN gives at most control: the newest paired
    /// client made admin as `uconsole hub role <cid> admin` does, live.
    fn promote(&self) {
        let cid = self.net.auth().clients().iter().map(|c| c.cid).max();
        assert!(
            self.net
                .set_role(cid.expect("a paired client"), Role::Admin)
        );
    }

    /// A raw, authenticated `admin` socket, and its token.
    fn admin(&self) -> (Ws, String) {
        let mut ws = ws(self.port);
        hello(&mut ws);
        let pin = self.pin();
        let reply = request(
            &mut ws,
            json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "raw", "role": "admin"}),
        );
        assert_eq!(reply["t"], "ok", "{reply}");
        self.promote();
        (ws, reply["token"].as_str().unwrap().to_owned())
    }

    fn wait_connections(&self, count: usize) {
        wait_until(&format!("{count} connections"), || {
            self.net.connections() == count
        });
    }
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

type Ws = WebSocket<TcpStream>;

fn request_text(port: u16, extra: &[&str]) -> String {
    let mut text = format!(
        "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n"
    );
    for header in extra {
        text.push_str(header);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    text
}

fn connect(port: u16) -> TcpStream {
    let tcp = TcpStream::connect((LOOPBACK, port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    tcp.set_write_timeout(Some(WAIT)).unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp
}

/// The status and head of an HTTP answer.
fn read_status(tcp: &mut TcpStream) -> (u16, String) {
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        match tcp.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, head)
}

/// The status an upgrade with these extra headers gets.
fn upgrade_status(port: u16, extra: &[&str]) -> u16 {
    let mut tcp = connect(port);
    tcp.write_all(request_text(port, extra).as_bytes()).unwrap();
    read_status(&mut tcp).0
}

fn http(port: u16, request: &str) -> (u16, String, String) {
    let mut tcp = connect(port);
    tcp.write_all(request.as_bytes()).unwrap();
    let (status, head) = read_status(&mut tcp);
    let mut body = String::new();
    let _ = tcp.read_to_string(&mut body);
    (status, head, body)
}

fn ws(port: u16) -> Ws {
    let mut tcp = connect(port);
    tcp.write_all(request_text(port, &[]).as_bytes()).unwrap();
    let (status, head) = read_status(&mut tcp);
    assert_eq!(status, 101, "{head}");
    assert!(
        head.contains("Sec-WebSocket-Protocol: dspi-link-1"),
        "{head}"
    );
    // RFC 6455's own example of this key.
    assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
    WebSocket::from_raw_socket(tcp, WsRole::Client, None)
}

fn send(ws: &mut Ws, value: &Value) {
    ws.send(Message::text(value.to_string())).unwrap();
}

fn next_text(ws: &mut Ws, t: &[&str]) -> Value {
    let start = Instant::now();
    loop {
        // Pings keep a read from ever timing out.
        assert!(start.elapsed() < WAIT, "no {t:?} in time");
        match ws.read().unwrap() {
            Message::Text(text) => {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if t.iter().any(|t| v["t"] == *t) {
                    return v;
                }
            }
            Message::Close(frame) => panic!("closed while waiting for {t:?}: {frame:?}"),
            _ => {}
        }
    }
}

fn request(ws: &mut Ws, value: Value) -> Value {
    send(ws, &value);
    next_text(ws, &["ok", "err"])
}

fn hello(ws: &mut Ws) -> Value {
    ws.send(Message::text(HELLO)).unwrap();
    next_text(ws, &["hello"])
}

/// Read until it closes: its close code, if a close frame came.
fn close_code(ws: &mut Ws) -> Option<u16> {
    loop {
        match ws.read() {
            Ok(Message::Close(frame)) => return frame.map(|f| u16::from(f.code)),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

/// The close code on a stream a client has stopped answering: raw, since a client that
/// answered its pings now would fail on the half-closed socket before reaching the close.
fn raw_close_code(tcp: &mut TcpStream) -> Option<u16> {
    let mut bytes = Vec::new();
    let _ = tcp.read_to_end(&mut bytes);
    let mut at = 0;
    while at + 2 <= bytes.len() {
        let opcode = bytes[at] & 0x0F;
        let (len, header) = match bytes[at + 1] & 0x7F {
            126 => (
                u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize,
                4,
            ),
            127 => (usize::MAX / 2, 10),
            short => (short as usize, 2),
        };
        if opcode == 8 {
            return Some(u16::from_be_bytes([
                bytes[at + header],
                bytes[at + header + 1],
            ]));
        }
        at += header + len;
    }
    None
}

fn is_closed_by_peer(tcp: &mut TcpStream) -> bool {
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let mut buf = [0u8; 256];
    loop {
        match tcp.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e) => {
                return !matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                );
            }
        }
    }
}

/// A frame as a client must send it, with whatever header a test wants.
fn masked(first: u8, length: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    out.extend_from_slice(length);
    out[1] |= 0x80; // the mask bit a client must set
    let key = [1u8, 2, 3, 4];
    out.extend_from_slice(&key);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
    out
}

fn options(name: &str, token: Option<String>) -> ConnectOptions {
    ConnectOptions {
        client_name: name.into(),
        app: "wp_hb1".into(),
        app_version: "0".into(),
        token,
    }
}

fn client(b: &Bench, token: Option<String>) -> Result<(LinkClient, AuthState), LinkError> {
    LinkClient::connect(
        &TcpConnector::default(),
        &b.url(),
        &options("test client", token),
        None,
    )
}

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/link/messages")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn frame_fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/link/frames")
            .join(name),
    )
    .unwrap()
}

/// Every key of a fixture's object is in ours, recursively into objects and the first
/// element of arrays: what a client built on the fixtures looks for is there.
fn assert_has_shape(fixture: &Value, ours: &Value, skip: &[&str], at: &str) {
    match (fixture, ours) {
        (Value::Object(f), Value::Object(o)) => {
            for (key, value) in f {
                if skip.contains(&key.as_str()) {
                    continue;
                }
                let inner = o.get(key);
                assert!(inner.is_some(), "{at}.{key} is missing from {ours}");
                assert_has_shape(value, inner.unwrap(), skip, &format!("{at}.{key}"));
            }
        }
        (Value::Array(f), Value::Array(o)) => {
            if let (Some(f), Some(o)) = (f.first(), o.first()) {
                assert_has_shape(f, o, skip, &format!("{at}[0]"));
            }
        }
        (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_))
        | (Value::Bool(_), Value::Bool(_)) => {}
        (f, o) => panic!("{at}: {f} and {o} are different kinds"),
    }
}

fn binary_response(ws: &mut Ws) -> dspi_link::frames::CmdResponse {
    loop {
        match ws.read().unwrap() {
            Message::Binary(bytes) => {
                if let Frame::CmdResponse(r) = Frame::decode(&bytes).unwrap() {
                    return r;
                }
            }
            Message::Close(frame) => panic!("closed: {frame:?}"),
            _ => {}
        }
    }
}

// ---- The done criteria ----

/// The L1 client end to end: pair, reconnect with the token, command, notify with
/// attribution, and a second client seeing the first one's write as foreign.
#[test]
fn wp_hb1_the_client_and_fixtures_pass_against_the_server() {
    let b = pin_hub();

    let (first, state) = client(&b, None).unwrap();
    assert!(
        matches!(state, AuthState::NeedsPairing { ref pin_source } if pin_source == "window"),
        "{state:?}"
    );
    let hello = first.hello().clone();
    assert!(hello.pin_required());
    assert_eq!(hello.pin_source(), "window");
    assert_eq!(hello.hub.id, b.hub.hub_id());
    assert_eq!(hello.hub.name, "Test hub");
    assert_eq!(hello.hub.kind, "console");
    assert!(hello.has_cap("cmd"));
    assert!(!hello.has_cap("fw_install"), "HUB.md section 8");
    assert!(!hello.has_cap("web"), "the web UI is off");
    assert_eq!(hello.limits.max_frame, MAX_FRAME);
    assert_eq!(hello.limits.max_payload, 8192);
    assert_eq!(hello.limits.max_inflight, 8);

    let token = first.pair(&b.pin(), "admin").unwrap();
    // -- WP PAIRADM -- a PIN gives at most control; admin is given on the hub's machine
    assert_eq!(first.role(), "control");
    b.promote();
    let session = first.session_id().unwrap();
    assert!(session >= 2, "0 and 1 are not for the network");
    let devices = first.devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].state, "online");
    assert_eq!(devices[0].handle, 0);

    let platform = first.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(platform.status, status::OK);
    assert_eq!(platform.payload.len(), 7);
    let missing = first.command(9, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(missing.status, status::NO_DEVICE);

    let (second, state) = client(&b, Some(token)).unwrap();
    assert!(matches!(state, AuthState::Ready), "{state:?}");
    assert_ne!(second.session_id(), first.session_id());
    assert_eq!(
        second.role(),
        "admin",
        "the token carries the role given since"
    );
    assert!(second.denied().is_empty());
    let online = b.net.auth().clients();
    assert_eq!(online.len(), 1);
    assert_eq!(online[0].name, "test client");

    let own = first.notifications(0);
    let foreign = second.notifications(0);
    let volume = (-12.0f32).to_le_bytes();
    let set = first.command(0, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, 2, 4, &volume);
    assert_eq!(set.status, status::OK);
    let echo = own.recv_timeout(WAIT).unwrap();
    assert_eq!(Some(echo.origin), first.session_id(), "the echo rule");
    let heard = foreign.recv_timeout(WAIT).unwrap();
    assert_eq!(heard.origin, session, "the other client sees whose it was");

    first.close();
    second.close();
    b.wait_connections(0);
    b.hub.stop();
}

/// What the hub sends has the shape of the upstream fixtures, parses with the client's
/// own types, and the fixtures' requests and frames are understood.
#[test]
fn wp_hb1_replies_match_the_upstream_fixtures() {
    let b = pin_hub();
    let mut raw = ws(b.port);

    let hub_hello = hello(&mut raw);
    assert_has_shape(&fixture("hello_hub.json"), &hub_hello, &[], "hello");
    assert!(matches!(
        Inbound::parse(&hub_hello.to_string()).unwrap(),
        Inbound::Hello(_)
    ));

    // A fixture's own requests, before authentication.
    let unauth = request(&mut raw, fixture("device_list.json"));
    assert_eq!(
        unauth,
        fixture("err_unauthenticated.json")
            .as_object()
            .map(|m| {
                let mut m = m.clone();
                m.insert("id".into(), fixture("device_list.json")["id"].clone());
                Value::Object(m)
            })
            .unwrap()
    );
    let pair = request(&mut raw, fixture("auth_pair.json"));
    assert_eq!(pair["code"], "unauthenticated");
    let token = request(&mut raw, fixture("auth_token.json"));
    assert_eq!(token["code"], "unauthenticated");

    // And after.
    let mut pair = fixture("auth_pair.json");
    pair["pin"] = b.pin().into();
    pair["role"] = "admin".into();
    let ok = request(&mut raw, pair);
    assert_has_shape(&fixture("auth_ok.json"), &ok, &[], "auth ok");
    b.promote(); // -- WP PAIRADM -- for `auth.list`, admin's
    assert!(matches!(
        Inbound::parse(&ok.to_string()).unwrap(),
        Inbound::Ok { .. }
    ));

    let list = request(&mut raw, fixture("device_list.json"));
    assert_has_shape(
        &fixture("device_list_ok.json"),
        &list,
        // Left out when not read, as spec 7.4 allows: HB3's.
        &["inputs", "wire_version"],
        "device.list",
    );
    assert_eq!(list["devices"][0]["locked_by"], Value::Null);

    let clients = request(&mut raw, fixture("auth_list.json"));
    assert_has_shape(&fixture("auth_list_ok.json"), &clients, &[], "auth.list");
    assert_eq!(clients["clients"][0]["online"], true);
    let created = clients["clients"][0]["created"].as_str().unwrap();
    assert!(created.ends_with('Z') && created.len() == 20, "{created}");

    let lock = request(&mut raw, fixture("lock_acquire.json"));
    assert_eq!(lock["t"], "ok", "{lock}");
    assert_eq!(lock["timeout_ms"], 10_000);
    let release = request(&mut raw, fixture("lock_release.json"));
    assert_eq!(release["t"], "ok", "{release}");

    // The frame fixtures: fixed handles, so the one for handle 0 reaches the demo.
    let bytes = frame_fixture("cmd_request_set_master_volume.bin");
    raw.send(Message::binary(bytes.clone())).unwrap();
    let response = binary_response(&mut raw);
    let Frame::CmdRequest(sent) = Frame::decode(&bytes).unwrap() else {
        panic!("not a request")
    };
    assert_eq!((response.tag, response.status), (sent.tag, status::OK));
    let bytes = frame_fixture("cmd_request_get_platform.bin");
    let Frame::CmdRequest(sent) = Frame::decode(&bytes).unwrap() else {
        panic!("not a request")
    };
    raw.send(Message::binary(bytes)).unwrap();
    let response = binary_response(&mut raw);
    assert_eq!(response.tag, sent.tag);
    assert_eq!(response.status, status::NO_DEVICE, "handle {}", sent.handle);
    b.hub.stop();
}

#[test]
fn wp_hb1_a_wrong_pin_is_answered_unauthenticated() {
    let b = pin_hub();
    let right = b.pin();
    let wrong = if right == "000000" {
        "000001"
    } else {
        "000000"
    };

    let mut raw = ws(b.port);
    hello(&mut raw);
    let reply = request(
        &mut raw,
        json!({"t": "auth.pair", "id": 3, "pin": wrong, "name": "x", "role": "control"}),
    );
    assert_eq!(reply["t"], "err");
    assert_eq!(reply["id"], 3);
    assert_eq!(reply["code"], "unauthenticated", "never `denied`: {reply}");
    assert!(reply.get("msg").is_none(), "as real hubs: {reply}");

    let (c, _) = client(&b, None).unwrap();
    assert_eq!(
        c.pair(wrong, "control"),
        Err(LinkError::WrongPin {
            pin_source: dspi_link::PinSource::Window
        })
    );
    assert!(!c.is_closed(), "a wrong PIN does not end the connection");
    b.hub.stop();
}

/// The foreign `Origin`, on the wire: 403 at the upgrade, for every way of forging it.
#[test]
fn wp_hb1_a_foreign_origin_is_refused() {
    // No page on a hub with no web UI, so nothing a browser can send is right, its own
    // address included.
    let closed = pin_hub();
    let own = format!("Origin: http://127.0.0.1:{}", closed.port);
    assert_eq!(upgrade_status(closed.port, &[&own]), 403);
    assert_eq!(upgrade_status(closed.port, &["Origin: null"]), 403);
    assert_eq!(
        upgrade_status(closed.port, &["Origin: http://evil.example"]),
        403
    );
    assert_eq!(upgrade_status(closed.port, &[]), 101, "a native client");
    closed.hub.stop();

    let open = start(NetworkConfig {
        web: true,
        mdns_name: Some("Studio-Mac.local".into()),
        ..network(store_dir(), AccessMode::Pin, ServerLimits::default())
    });
    let port = open.port;
    for good in [
        format!("Origin: http://127.0.0.1:{port}"),
        format!("Origin: http://localhost:{port}"),
        format!("origin: http://LOCALHOST:{port}"),
        format!("Origin: http://[::1]:{port}"),
        format!("Origin: http://studio-mac.local:{port}"),
    ] {
        assert_eq!(upgrade_status(port, &[&good]), 101, "{good}");
    }
    for bad in [
        "Origin: null".to_owned(),
        "Origin:".to_owned(),
        "Origin: http://evil.example".to_owned(),
        format!("Origin: http://evil.example:{port}"),
        format!("Origin: http://127.0.0.1:{}", port.wrapping_add(1)),
        format!("Origin: http://127.0.0.1:{port}/"),
        format!("Origin: http://127.0.0.1:{port}.evil.example"),
        format!("Origin: http://user@127.0.0.1:{port}"),
        "Origin: http://127.0.0.1".to_owned(),
    ] {
        assert_eq!(upgrade_status(port, &[&bad]), 403, "{bad}");
    }
    // -- review secb -- our own https page: ours only where the hub serves TLS
    let own_https = format!("Origin: https://127.0.0.1:{port}");
    let want = if tls_served() { 101 } else { 403 };
    assert_eq!(upgrade_status(port, &[&own_https]), want, "{own_https}");
    // Neither of two headers can make the other not count.
    let two = [
        format!("Origin: http://127.0.0.1:{port}"),
        "Origin: http://evil.example".to_owned(),
    ];
    assert_eq!(upgrade_status(port, &[&two[0], &two[1]]), 403);
    assert_eq!(upgrade_status(port, &[&two[1], &two[0]]), 403);
    // Bytes that are not text are not "no Origin".
    let mut tcp = connect(port);
    let mut bytes = request_text(port, &[]).into_bytes();
    bytes.truncate(bytes.len() - 2);
    bytes.extend_from_slice(b"Origin: http://\xFF\xFE\r\n\r\n");
    tcp.write_all(&bytes).unwrap();
    assert_eq!(read_status(&mut tcp).0, 403);

    // DNS rebinding: the page's name resolves to the hub, so `Host` agrees with `Origin`.
    // Only `OwnNames` counts.
    let mut tcp = connect(port);
    let text = request_text(port, &["Origin: http://rebound.example:1234"]).replace(
        &format!("Host: 127.0.0.1:{port}"),
        "Host: rebound.example:1234",
    );
    tcp.write_all(text.as_bytes()).unwrap();
    assert_eq!(read_status(&mut tcp).0, 403);
    // And a `Host` naming the hub gets a foreign page nothing either.
    let mut tcp = connect(port);
    let text = request_text(port, &["Origin: http://rebound.example:1234"]);
    tcp.write_all(text.as_bytes()).unwrap();
    assert_eq!(read_status(&mut tcp).0, 403);
    // The reverse: a forged `Host` costs a native client nothing, since it is not looked at.
    let mut tcp = connect(port);
    let text = request_text(port, &[]).replace(&format!("127.0.0.1:{port}"), "whatever");
    tcp.write_all(text.as_bytes()).unwrap();
    assert_eq!(read_status(&mut tcp).0, 101);
    open.hub.stop();
}

// ---- The handshake and HTTP ----

#[test]
fn wp_hb1_hello_says_what_the_hub_is_and_leaves_out_fw_install() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    let h = hello(&mut raw);
    assert_eq!(h["auth"], "pin");
    assert_eq!(h["pin_source"], "window", "no fixed PIN here");
    // -- WP PGHDEV -- DSPi Link 1.3 (PLAN-PG R4), with our cap `legacy_devices`
    assert_eq!(h["proto"], json!({"major": 1, "minor": 3}));
    let caps: Vec<&str> = h["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    for cap in ["cmd", "notify", "poll", "snapshot", "lock"] {
        assert!(caps.contains(&cap), "{caps:?}");
    }
    assert!(!caps.contains(&"fw_install"));
    assert!(caps.contains(&"legacy_devices"), "{caps:?}"); // -- WP PGHDEV --
    assert_eq!(h["hub"]["id"], b.hub.hub_id());
    b.hub.stop();

    let open = bench(AccessMode::Open, ServerLimits::default());
    let mut raw = ws(open.port);
    let h = hello(&mut raw);
    assert_eq!(h["auth"], "none");
    assert!(h.get("pin_source").is_none(), "only in `pin` mode: {h}");
    open.hub.stop();

    let web = start(NetworkConfig {
        web: true,
        ..network(store_dir(), AccessMode::Pin, ServerLimits::default())
    });
    let h = hello(&mut ws(web.port));
    assert!(h["caps"].as_array().unwrap().contains(&json!("web")));
    web.hub.stop();
}

/// The hub's id comes from its store, the same across a restart, with paired clients.
#[test]
fn wp_hb1_the_hub_id_and_the_clients_survive_a_restart() {
    let dir = store_dir();
    let first = start(network(
        dir.clone(),
        AccessMode::Pin,
        ServerLimits::default(),
    ));
    let id = first.hub.hub_id().to_owned();
    assert_eq!(id.len(), 36);
    let (_ws, token) = first.admin();
    first.hub.stop();
    let text = fs::read_to_string(first.dir.join("hub-store.json")).unwrap();
    assert!(!text.contains(&token), "only its hash");

    let second = start(network(dir, AccessMode::Pin, ServerLimits::default()));
    assert_eq!(second.hub.hub_id(), id);
    let (c, state) = client(&second, Some(token)).unwrap();
    assert!(matches!(state, AuthState::Ready), "{state:?}");
    assert_eq!(c.hello().hub.id, id);
    c.close();
    second.hub.stop();
}

#[test]
fn wp_hb1_the_info_endpoint_answers_as_spec_3_2() {
    let b = pin_hub();
    let (status, head, body) = http(b.port, "GET /dspi/v1/info HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status, 200);
    assert!(head.contains("Access-Control-Allow-Origin: *"), "{head}");
    assert!(head.contains("application/json"), "{head}");
    let info: dspi_link::messages::InfoDoc = serde_json::from_str(&body).unwrap();
    assert_eq!(info.auth, "pin");
    // -- review secb -- as the mode says
    assert_eq!(info.tls, tls_served());
    assert_eq!(info.ws.as_deref(), Some("/dspi/v1"));
    assert_eq!(info.hub.id, b.hub.hub_id());
    assert_eq!(info.proto.major, 1);
    assert_eq!(info.devices.len(), 1);
    assert_eq!(info.devices[0].serial, b.hub.devices()[0].identity.serial);
    // With a query, as a cache-busting page might.
    let (status, ..) = http(b.port, "GET /dspi/v1/info?x=1 HTTP/1.1\r\n\r\n");
    assert_eq!(status, 200);
    // The info endpoint is what dspi-link's own probe reads.
    b.hub.stop();
}

#[test]
fn wp_hb1_other_http_gets_a_plain_refusal() {
    let b = pin_hub();
    let port = b.port;
    // -- review secb -- a ClientHello: a plain 400 at once with TLS off, a TLS
    // handshake (that these bytes fail, with no HTTP answer) where TLS is served
    let hello_answer = if tls_served() { 0 } else { 400 };
    for (request, expect) in [
        ("GET / HTTP/1.1\r\n\r\n", 404),
        ("GET /index.html HTTP/1.1\r\n\r\n", 404),
        ("GET /../../etc/passwd HTTP/1.1\r\n\r\n", 404),
        ("GET /dspi/v1/other HTTP/1.1\r\n\r\n", 404),
        ("POST /dspi/v1/info HTTP/1.1\r\n\r\n", 405),
        ("DELETE / HTTP/1.1\r\n\r\n", 405),
        ("HEAD /dspi/v1/info HTTP/1.1\r\n\r\n", 405),
        // The WebSocket path without an upgrade.
        ("GET /dspi/v1 HTTP/1.1\r\n\r\n", 400),
        ("POST /dspi/v1 HTTP/1.1\r\n\r\n", 400),
        // Not HTTP at all.
        (
            "\x16\x03\x01\x02\x00\x01\x00\x01\x7c\x03\x03\r\n\r\n",
            hello_answer,
        ),
        ("\r\n\r\n", 400),
        ("GET\r\n\r\n", 400),
        ("GET / HTTP/9.9\r\n\r\n", 400),
    ] {
        let (status, head, _) = http(port, request);
        assert_eq!(status, expect, "{request:?}: {head}");
        assert!(
            !head.contains("Access-Control"),
            "that header is the info endpoint's alone: {head}"
        );
    }
    assert_eq!(b.hub.devices().len(), 1);
    b.wait_connections(0);
    b.hub.stop();
}

#[test]
fn wp_hb1_a_malformed_upgrade_is_refused() {
    let b = pin_hub();
    let port = b.port;
    let plain = request_text(port, &[]);
    for broken in [
        plain.replace("Sec-WebSocket-Protocol: dspi-link-1\r\n", ""),
        plain.replace("dspi-link-1", "other"),
        plain.replace("Sec-WebSocket-Version: 13", "Sec-WebSocket-Version: 8"),
        plain.replace("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n", ""),
        plain.replace("Upgrade: websocket", "Upgrade: h2c"),
        plain.replace("Connection: Upgrade", "Connection: close"),
        plain.replace("HTTP/1.1", "HTTP/1.0"),
        plain.replace(
            "Sec-WebSocket-Key:",
            "Sec-WebSocket-Version: 8\r\nSec-WebSocket-Key:",
        ),
    ] {
        let mut tcp = connect(port);
        tcp.write_all(broken.as_bytes()).unwrap();
        let (status, head) = read_status(&mut tcp);
        assert_eq!(status, 400, "{broken}\n{head}");
    }
    // Tokens in the lists, in any case, as a browser may send them.
    let listed = plain
        .replace("Connection: Upgrade", "Connection: keep-alive, UPGRADE")
        .replace("Upgrade: websocket", "Upgrade: WebSocket")
        .replace("dspi-link-1", "other, dspi-link-1");
    let mut tcp = connect(port);
    tcp.write_all(listed.as_bytes()).unwrap();
    assert_eq!(read_status(&mut tcp).0, 101);
    b.hub.stop();
}

/// Whatever came behind the upgrade request in the same packet is the WebSocket's.
#[test]
fn wp_hb1_frames_right_behind_the_upgrade_are_not_lost() {
    let b = pin_hub();
    let mut bytes = request_text(b.port, &[]).into_bytes();
    bytes.extend(masked(0x81, &[HELLO.len() as u8], HELLO.as_bytes()));
    let mut tcp = connect(b.port);
    tcp.write_all(&bytes).unwrap();
    assert_eq!(read_status(&mut tcp).0, 101);
    let mut raw = WebSocket::from_raw_socket(tcp, WsRole::Client, None);
    assert_eq!(next_text(&mut raw, &["hello"])["auth"], "pin");
    b.hub.stop();
}

/// A head split anywhere, the end marker included, is still one head.
#[test]
fn wp_hb1_a_head_in_pieces_is_put_together() {
    let b = pin_hub();
    let text = request_text(b.port, &[]);
    for at in [1, 10, text.len() - 3, text.len() - 2, text.len() - 1] {
        let mut tcp = connect(b.port);
        tcp.write_all(&text.as_bytes()[..at]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        tcp.write_all(&text.as_bytes()[at..]).unwrap();
        assert_eq!(read_status(&mut tcp).0, 101, "split at {at}");
    }
    b.hub.stop();
}

// ---- Open access ----

#[test]
fn wp_hb1_open_access_makes_every_connection_admin_with_no_auth_message() {
    let b = bench(AccessMode::Open, ServerLimits::default());
    let (c, state) = client(&b, None).unwrap();
    assert!(matches!(state, AuthState::Ready), "{state:?}");
    assert_eq!(c.role(), "admin");
    assert!(
        c.session_id().is_some(),
        "the courtesy `auth.pair` got an id"
    );
    let platform = c.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(platform.status, status::OK);
    c.close();

    let mut raw = ws(b.port);
    hello(&mut raw);
    let list = request(&mut raw, json!({"t": "device.list", "id": 1}));
    assert_eq!(list["t"], "ok", "no auth message is needed: {list}");
    let pair = request(
        &mut raw,
        json!({"t": "auth.pair", "id": 2, "pin": "any", "name": "x"}),
    );
    assert_eq!(pair["role"], "admin");
    assert!(pair.get("token").is_none(), "{pair}");
    assert_eq!(pair["policy"]["denied"], json!([]));
    let again = request(
        &mut raw,
        json!({"t": "auth.token", "id": 3, "token": "any"}),
    );
    assert_eq!(
        again["session"], pair["session"],
        "the same session, not a second one"
    );
    assert!(
        b.net.auth().clients().is_empty(),
        "nothing is stored for it"
    );
    b.hub.stop();
}

/// A socket on an open hub still has to say hello before it is a session.
#[test]
fn wp_hb1_open_access_still_closes_a_silent_socket() {
    let b = bench(AccessMode::Open, quick());
    let mut raw = ws(b.port);
    assert_eq!(close_code(&mut raw), Some(4001));
    b.wait_connections(0);
    b.hub.stop();
}

// ---- Before authentication ----

#[test]
fn wp_hb1_before_auth_every_other_message_is_unauthenticated() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    hello(&mut raw);
    for t in [
        "device.list",
        "device.snapshot",
        "poll.subscribe",
        "lock.acquire",
        "auth.list",
        "auth.revoke",
        "auth.set_role",
        "hub.stats",
        "fw.install",
        "no.such.type",
    ] {
        let reply = request(&mut raw, json!({"t": t, "id": 9, "handle": 0, "cid": 1}));
        assert_eq!(reply["code"], "unauthenticated", "{t}: {reply}");
        assert_eq!(reply["id"], 9);
    }
    // No id: the reply has none either.
    send(&mut raw, &json!({"t": "device.list"}));
    let reply = next_text(&mut raw, &["err"]);
    assert!(reply.get("id").is_none(), "{reply}");
    assert!(b.net.auth().clients().is_empty());
    b.hub.stop();
}

/// Binary frames before authentication are discarded (spec 5): no answer, and no
/// command reaches the device.
#[test]
fn wp_hb1_before_auth_commands_are_discarded_unanswered() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    hello(&mut raw);
    let command = Frame::CmdRequest(CmdRequest {
        tag: 5,
        handle: 0,
        dir: Dir::Set,
        b_request: op::REQ_SET_MASTER_VOLUME,
        w_value: 0,
        w_index: 2,
        w_length: 4,
        payload: 0.0f32.to_le_bytes().to_vec(),
    });
    for _ in 0..20 {
        raw.send(Message::binary(command.encode())).unwrap();
    }
    raw.send(Message::binary(vec![0x7E, 0, 1, 0])).unwrap();
    raw.send(Message::binary(vec![1])).unwrap();
    // The next thing to come back answers this, so nothing was sent for those.
    let reply = request(&mut raw, json!({"t": "device.list", "id": 77}));
    assert_eq!(
        (reply["code"].as_str(), reply["id"].as_u64()),
        (Some("unauthenticated"), Some(77))
    );

    let (c, _) = client(&b, None).unwrap();
    c.pair(&b.pin(), "admin").unwrap();
    b.promote(); // -- WP PAIRADM --
    let rx = c.notifications(0);
    let volume = -33.0f32;
    let r = c.command(
        0,
        Dir::Set,
        op::REQ_SET_MASTER_VOLUME,
        0,
        2,
        4,
        &volume.to_le_bytes(),
    );
    assert_eq!(r.status, status::OK);
    // The first change the device reports is this one, not one of the 20.
    let first = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(Some(first.origin), c.session_id());
    c.close();
    b.hub.stop();
}

#[test]
fn wp_hb1_a_socket_not_authenticated_in_time_closes_with_4001() {
    let b = bench(AccessMode::Pin, quick());
    let start = Instant::now();
    let mut raw = ws(b.port);
    hello(&mut raw);
    // Talking does not buy time, and neither do pings.
    let talker = std::thread::spawn({
        let port = b.port;
        move || {
            let mut raw = ws(port);
            hello(&mut raw);
            loop {
                if raw.send(Message::Ping(Vec::new().into())).is_err()
                    || raw.send(Message::text(HELLO)).is_err()
                {
                    return close_code(&mut raw);
                }
                match raw.read() {
                    Ok(Message::Close(frame)) => return frame.map(|f| u16::from(f.code)),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
        }
    });
    assert_eq!(close_code(&mut raw), Some(4001));
    let took = start.elapsed();
    assert!(took >= Duration::from_millis(450), "{took:?}");
    assert_eq!(talker.join().unwrap(), Some(4001));
    b.wait_connections(0);
    b.hub.stop();
}

/// The head, byte by byte: the limit is on the whole of it.
#[test]
fn wp_hb1_slowloris_on_the_http_head_loses_its_slot() {
    let b = bench(AccessMode::Pin, quick());
    let mut tcp = connect(b.port);
    let started = Instant::now();
    let text = request_text(b.port, &[]);
    let mut closed_after = None;
    for byte in text.as_bytes() {
        if tcp.write_all(&[*byte]).is_err() {
            closed_after = Some(started.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
        if b.net.connections() == 0 {
            closed_after = Some(started.elapsed());
            break;
        }
    }
    let took = closed_after.expect("still open after the whole head was trickled");
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert!(is_closed_by_peer(&mut tcp));
    b.hub.stop();
}

/// An unfinished frame is not an answer. Before auth the auth limit ends it.
#[test]
fn wp_hb1_slowloris_on_a_frame_before_auth_closes_with_4001() {
    let b = bench(AccessMode::Pin, quick());
    let mut raw = ws(b.port);
    let frame = masked(0x81, &[100], &[b'x'; 100]);
    let started = Instant::now();
    for byte in frame.iter().take(60) {
        if raw.get_mut().write_all(&[*byte]).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
        if started.elapsed() > Duration::from_millis(700) {
            break;
        }
    }
    assert_eq!(close_code(&mut raw), Some(4001));
    b.wait_connections(0);
    b.hub.stop();
}

#[test]
fn wp_hb1_a_head_too_large_is_431_and_too_many_headers_400() {
    let b = pin_hub();
    let padded = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
    let mut tcp = connect(b.port);
    let _ = tcp.write_all(padded.as_bytes());
    assert_eq!(read_status(&mut tcp).0, 431);
    // With no end in sight.
    let mut tcp = connect(b.port);
    let _ = tcp.write_all(&vec![b'a'; MAX_HEAD * 2]);
    assert_eq!(read_status(&mut tcp).0, 431);
    let many: Vec<String> = (0..100).map(|n| format!("X-{n}: 1")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(upgrade_status(b.port, &many), 400);
    b.wait_connections(0);
    b.hub.stop();
}

// ---- Frames ----

/// A frame that says it is enormous is refused from its header: nothing is allocated
/// for it, and no more of it is waited for. Before auth and after.
#[test]
fn wp_hb1_an_oversized_frame_is_refused_from_its_header() {
    let b = pin_hub();
    let huge = 1u64 << 40;
    let mut length = vec![0xFF];
    length.extend_from_slice(&huge.to_be_bytes());

    for (name, first) in [("text", 0x81u8), ("binary", 0x82)] {
        let mut raw = ws(b.port);
        hello(&mut raw);
        raw.get_mut()
            .write_all(&masked(first, &length, b"x"))
            .unwrap();
        assert_eq!(close_code(&mut raw), Some(4000), "before auth, {name}");

        let (mut raw, _) = b.admin();
        raw.get_mut()
            .write_all(&masked(first, &length, b"x"))
            .unwrap();
        assert_eq!(close_code(&mut raw), Some(4000), "after auth, {name}");
    }

    // One byte over the limit, which is what `hello` promises, actually sent.
    let mut raw = ws(b.port);
    let over = (MAX_FRAME as u64 + 1).to_be_bytes();
    let mut length = vec![0xFF];
    length.extend_from_slice(&over);
    let _ = raw
        .get_mut()
        .write_all(&masked(0x82, &length, &vec![0; MAX_FRAME as usize + 1]));
    assert_eq!(close_code(&mut raw), Some(4000));

    // At the limit is fine.
    let (mut raw, _) = b.admin();
    let padding = " ".repeat(MAX_FRAME as usize - 32);
    let text = format!(r#"{{"t":"device.list","id":8}}{padding}"#);
    assert!(text.len() <= MAX_FRAME as usize);
    raw.send(Message::text(text)).unwrap();
    assert_eq!(next_text(&mut raw, &["ok"])["id"], 8);
    b.hub.stop();
}

/// Small fragments that add up past the limit: the message limit, not just the frame's.
#[test]
fn wp_hb1_fragments_adding_up_past_the_limit_are_refused() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    let chunk = vec![b' '; 32 * 1024];
    let mut length = vec![126];
    length.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
    let mut stream = masked(0x01, &length, &chunk); // first fragment
    stream.extend(masked(0x00, &length, &chunk)); // continuation
    stream.extend(masked(0x00, &length, &chunk)); // one too many
    let _ = raw.get_mut().write_all(&stream);
    assert_eq!(close_code(&mut raw), Some(4000));
    b.hub.stop();
}

#[test]
fn wp_hb1_broken_framing_closes_with_4000() {
    let b = pin_hub();
    let unmasked = {
        let mut f = vec![0x81, 2];
        f.extend_from_slice(b"{}");
        f
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not masked", unmasked),
        ("reserved bits", masked(0xF1, &[2], b"{}")),
        ("reserved opcode", masked(0x83, &[2], b"{}")),
        ("not UTF-8", masked(0x81, &[3], &[0xFF, 0xFE, 0xFD])),
        ("fragmented ping", masked(0x09, &[1], b"x")),
        ("long ping", {
            let mut l = vec![126];
            l.extend_from_slice(&200u16.to_be_bytes());
            masked(0x89, &l, &[0; 200])
        }),
        (
            "continuation with nothing to continue",
            masked(0x80, &[1], b"x"),
        ),
    ];
    for (name, bytes) in cases {
        let mut raw = ws(b.port);
        raw.get_mut().write_all(&bytes).unwrap();
        assert_eq!(close_code(&mut raw), Some(4000), "{name}");
    }
    b.wait_connections(0);
    b.hub.stop();
}

#[test]
fn wp_hb1_malformed_json_closes_with_4000_and_harms_nobody_else() {
    let b = pin_hub();
    let (mut bystander, _) = b.admin();
    let deep = "[".repeat(50_000);
    let deep_object = format!("{{\"t\":{}", "{\"t\":".repeat(10_000));
    for text in [
        "",
        " ",
        "not json",
        "{",
        "}",
        "null",
        "true",
        "12",
        "\"hello\"",
        "[]",
        "[{\"t\":\"hello\"}]",
        "{}",
        "{\"t\":null}",
        "{\"t\":7}",
        "{\"t\":{\"t\":\"hello\"}}",
        "{\"id\":1}",
        "{\"t\":\"hello\"",
        "{\"t\":\"hello\"} trailing",
        "{\"t\":\"\\ud800\"}",
        deep.as_str(),
        deep_object.as_str(),
    ] {
        let mut raw = ws(b.port);
        raw.send(Message::text(text)).unwrap();
        let shown: String = text.chars().take(40).collect();
        assert_eq!(close_code(&mut raw), Some(4000), "{shown:?}");
    }
    let reply = request(&mut bystander, json!({"t": "device.list", "id": 1}));
    assert_eq!(reply["t"], "ok");
    b.hub.stop();
}

/// Well-formed JSON with nonsense in the fields: a client's mistake, answered, not fatal.
#[test]
fn wp_hb1_bad_fields_are_answered_and_the_session_goes_on() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    hello(&mut raw);
    for message in [
        json!({"t": "auth.pair", "id": 1}),
        json!({"t": "auth.pair", "id": 1, "pin": 123456}),
        json!({"t": "auth.pair", "id": 1, "pin": "1", "role": "root"}),
        json!({"t": "auth.pair", "id": 1, "pin": "1", "role": 7}),
        json!({"t": "auth.token", "id": 1}),
        json!({"t": "auth.token", "id": 1, "token": null}),
    ] {
        assert_eq!(
            request(&mut raw, message.clone())["code"],
            "bad_request",
            "{message}"
        );
    }
    assert!(b.net.auth().clients().is_empty());

    let (mut raw, _) = b.admin();
    for (message, code) in [
        (json!({"t": "lock.acquire", "id": 1}), "bad_request"),
        (
            json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": -1}),
            "bad_request",
        ),
        (
            json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": "x"}),
            "bad_request",
        ),
        (
            json!({"t": "lock.acquire", "id": 1, "handle": 300}),
            "bad_request",
        ),
        (
            json!({"t": "lock.acquire", "id": 1, "handle": 9}),
            "no_device",
        ),
        (json!({"t": "lock.release", "id": 1}), "bad_request"),
        (json!({"t": "device.snapshot", "id": 1}), "bad_request"),
        (
            json!({"t": "device.snapshot", "id": 1, "handle": 99}),
            "no_device",
        ),
        (
            json!({"t": "device.snapshot", "id": 1, "handle": -1}),
            "bad_request",
        ),
        (json!({"t": "poll.subscribe", "id": 1}), "bad_request"),
        (
            json!({"t": "poll.subscribe", "id": 1, "handle": 0, "polls": 7}),
            "bad_request",
        ),
        (
            json!({"t": "poll.subscribe", "id": 1, "handle": 0, "polls": [{"slot": 900}]}),
            "bad_request",
        ),
        (
            json!({"t": "poll.subscribe", "id": 1, "handle": 99, "polls": []}),
            "no_device",
        ),
        (
            json!({"t": "poll.unsubscribe", "id": 1, "handle": 0}),
            "bad_request",
        ),
        (json!({"t": "auth.revoke", "id": 1}), "bad_request"),
        (
            json!({"t": "auth.revoke", "id": 1, "cid": 999}),
            "bad_request",
        ),
        (
            json!({"t": "auth.revoke", "id": 1, "cid": "x"}),
            "bad_request",
        ),
        (
            json!({"t": "auth.revoke", "id": 1, "cid": 99999999999u64}),
            "bad_request",
        ),
        (
            json!({"t": "auth.set_role", "id": 1, "cid": 1, "role": "root"}),
            "bad_request",
        ),
        (
            json!({"t": "auth.set_role", "id": 1, "cid": 999, "role": "viewer"}),
            "bad_request",
        ),
        (json!({"t": "no.such.type", "id": 1}), "unknown_type"),
        (json!({"t": "fw.install", "id": 1}), "unsupported"),
        (
            json!({"t": "device.rename", "id": 1, "handle": 0}),
            "bad_request",
        ),
    ] {
        let reply = request(&mut raw, message.clone());
        assert_eq!(reply["code"], code, "{message}: {reply}");
    }
    // An id that is not an integer is no id.
    send(
        &mut raw,
        &json!({"t": "no.such.type", "id": {"nested": [1, 2]}}),
    );
    let reply = next_text(&mut raw, &["err"]);
    assert!(reply.get("id").is_none(), "{reply}");
    let reply = request(&mut raw, json!({"t": "device.list", "id": 2}));
    assert_eq!(reply["t"], "ok");
    b.hub.stop();
}

/// Random data-plane frames from an authenticated client, including hand-picked bad
/// ones. Whatever they do to the fake device, they do nothing to the server.
#[test]
fn wp_hb1_hostile_binary_frames_panic_nothing() {
    let b = pin_hub();
    let (mut raw, token) = b.admin();

    // Answered with its tag where there is a tag to answer.
    for (bytes, tag, expect) in [
        (vec![kind::CMD_REQUEST, 0, 7, 0], 7, status::BAD_FRAME),
        (
            vec![kind::CMD_REQUEST, 0, 8, 0, 0, 2, 0xD2, 0, 0, 0, 2, 0, 0, 0],
            8,
            status::BAD_FRAME,
        ),
        (
            vec![
                kind::CMD_REQUEST,
                0,
                9,
                0,
                0,
                0,
                0xD2,
                0,
                0,
                0,
                2,
                0,
                0xFF,
                0xFF,
            ],
            9,
            status::BAD_FRAME,
        ),
        (vec![0x7E, 0, 10, 0], 10, status::UNSUPPORTED),
        (
            vec![kind::FWDATA, 0, 11, 0, 0, 0, 0, 0, 1, 2, 3],
            11,
            status::UNSUPPORTED,
        ),
    ] {
        raw.send(Message::binary(bytes.clone())).unwrap();
        let r = binary_response(&mut raw);
        assert_eq!((r.tag, r.status), (tag, expect), "{bytes:?}");
    }
    // Ignored: too short for a tag, and types only a hub sends. The next answer is this.
    for bytes in [
        vec![],
        vec![1],
        vec![1, 0, 1],
        vec![kind::CMD_RESPONSE, 0, 1, 0, 0, 0, 0xFF, 0xFF],
        vec![kind::NOTIFY, 0, 1, 0],
        vec![kind::POLL, 0, 1, 0],
        vec![kind::RESYNC, 0, 1, 0],
        vec![0xEE, 0, 1, 0],
    ] {
        raw.send(Message::binary(bytes)).unwrap();
    }
    let r = request(&mut raw, json!({"t": "device.list", "id": 44}));
    assert_eq!(r["id"], 44);

    // Random bytes, from a fixed seed. Opcodes that only read, and handles with no device
    // beyond that, so nothing here depends on what the fake does with a random write.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..2000 {
        let length = (next() % 40) as usize;
        let mut bytes: Vec<u8> = (0..length).map(|_| next() as u8).collect();
        if let Some(handle) = bytes.get_mut(4) {
            *handle |= 0x80; // no device there: answered, and never reaches one
        }
        raw.send(Message::binary(bytes)).unwrap();
        // Keep the queue short, so this tests the frames and not the write limit.
        if next() % 16 == 0 {
            let _ = request(&mut raw, json!({"t": "device.list", "id": 1}));
        }
    }
    let r = request(&mut raw, json!({"t": "device.list", "id": 45}));
    assert_eq!(r["devices"][0]["state"], "online");
    let (c, state) = client(&b, Some(token)).unwrap();
    assert!(matches!(state, AuthState::Ready), "{state:?}");
    c.close();
    b.hub.stop();
}

// ---- Limits ----

fn connect_from(source: Ipv4Addr, port: u16) -> TcpStream {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    socket
        .bind(&SockAddr::from(SocketAddr::new(IpAddr::V4(source), 0)))
        .unwrap();
    socket
        .connect(&SockAddr::from(SocketAddr::new(LOOPBACK, port)))
        .unwrap();
    let tcp: TcpStream = socket.into();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    tcp.set_write_timeout(Some(WAIT)).unwrap();
    tcp
}

/// The pre-auth flood: one address cannot hold more than its share however many sockets
/// it opens; another address and an authenticated session are untouched; and every slot
/// comes back on its own. -- WP HB9 -- Silent sockets are plain HTTP, in their own budget
/// (the address's oldest goes to make room for its next), and the WebSocket share is
/// counted at the upgrade.
#[test]
fn wp_hb1_a_pre_auth_flood_from_one_address_does_not_fill_the_hub() {
    let limits = ServerLimits {
        max_connections: 8,
        max_preauth_per_peer: 3,
        // Longer than it takes to open all those sockets on a loaded machine.
        head_timeout: Duration::from_secs(3),
        auth_timeout: Duration::from_secs(3),
        dead_peer: Duration::from_secs(60),
        ..quick()
    };
    let http_share = limits.max_http_per_peer;
    let b = bench(AccessMode::Pin, limits);
    let (mut authed, _) = b.admin();
    b.wait_connections(1);

    let flooder = Ipv4Addr::new(127, 0, 0, 9);
    let mut flood: Vec<TcpStream> = (0..200).map(|_| connect_from(flooder, b.port)).collect();
    wait_until("the flood's share", || {
        b.net.connections() == 1 + http_share
    });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        b.net.connections(),
        1 + http_share,
        "only its share, whatever it opens"
    );
    assert_eq!(b.net.websockets(), 1);
    // Upgrades from the same address: its WebSocket share, then 503.
    let mut upgraded = Vec::new();
    for _ in 0..3 {
        let mut tcp = connect_from(flooder, b.port);
        tcp.write_all(request_text(b.port, &[]).as_bytes()).unwrap();
        assert_eq!(read_status(&mut tcp).0, 101);
        upgraded.push(tcp);
    }
    assert_eq!(b.net.websockets(), 1 + 3);
    let mut extra = connect_from(flooder, b.port);
    extra
        .write_all(request_text(b.port, &[]).as_bytes())
        .unwrap();
    assert_eq!(read_status(&mut extra).0, 503);

    // Someone else gets in. And the session already in goes on.
    let mut other = connect_from(Ipv4Addr::new(127, 0, 0, 10), b.port);
    other
        .write_all(request_text(b.port, &[]).as_bytes())
        .unwrap();
    assert_eq!(read_status(&mut other).0, 101);
    assert_eq!(
        request(&mut authed, json!({"t": "device.list", "id": 2}))["t"],
        "ok"
    );

    // The flood's sockets are all closed by the hub, without a byte from them.
    for tcp in flood.iter_mut().chain(&mut upgraded) {
        assert!(is_closed_by_peer(tcp));
    }
    drop(other);
    b.wait_connections(1);
    let (c, _) = client(&b, None).unwrap();
    assert!(!c.is_closed());
    c.close();
    b.hub.stop();
}

/// Many addresses at once: the total holds, and they all time out.
#[test]
fn wp_hb1_a_pre_auth_flood_from_many_addresses_stops_at_the_limit_and_clears() {
    let limits = ServerLimits {
        max_connections: 8,
        max_preauth_per_peer: 3,
        // -- WP HB9 -- silent sockets are plain HTTP
        max_http: 8,
        max_refusing: 2,
        ..quick()
    };
    let b = bench(AccessMode::Pin, limits);
    let mut flood = Vec::new();
    for n in 0..40u8 {
        for _ in 0..5 {
            flood.push(connect_from(Ipv4Addr::new(127, 0, 1, n + 1), b.port));
        }
    }
    let mut peak = 0;
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(200) {
        peak = peak.max(b.net.connections());
        assert!(peak <= 8, "{peak}");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(peak >= 1, "the hub is not refusing everything");
    for tcp in &mut flood {
        assert!(is_closed_by_peer(tcp));
    }
    b.wait_connections(0);
    let (c, _) = client(&b, None).unwrap();
    c.close();
    b.hub.stop();
}

/// Past the limit the upgrade answers 503, which the client knows as `HubBusy`, and a
/// slot that frees up is free.
#[test]
fn wp_hb1_the_session_limit_answers_503_at_the_upgrade() {
    let limits = ServerLimits {
        max_connections: 2,
        ..ServerLimits::default()
    };
    let b = bench(AccessMode::Pin, limits);
    let (a, _) = b.admin();
    let (_second, _) = b.admin();
    let refused = client(&b, None);
    assert_eq!(refused.err(), Some(LinkError::HubBusy));
    assert_eq!(upgrade_status(b.port, &[]), 503);
    drop(a);
    b.wait_connections(1);
    let (c, _) = client(&b, None).unwrap();
    assert!(!c.is_closed());
    c.close();
    b.hub.stop();
}

/// Connect and go, in a loop, on every path. The count returns to zero; no slot leaks.
#[test]
fn wp_hb1_a_reconnect_storm_leaks_no_slot() {
    let b = bench(AccessMode::Pin, quick());
    let (_keep, token) = b.admin();
    for n in 0..60 {
        match n % 4 {
            0 => drop(connect(b.port)),
            1 => drop(ws(b.port)),
            2 => {
                let mut raw = ws(b.port);
                hello(&mut raw);
                let _ = raw.close(None);
            }
            _ => {
                let mut raw = ws(b.port);
                let reply = request(
                    &mut raw,
                    json!({"t": "auth.token", "id": 1, "token": token}),
                );
                assert_eq!(reply["t"], "ok", "attempt {n}: {reply}");
            }
        }
        wait_until("room", || b.net.connections() < 8);
    }
    b.wait_connections(1);
    b.hub.stop();
}

// ---- PINs and tokens ----

/// The burst, over the wire and past the point where the right PIN stops helping.
#[test]
fn wp_hb1_a_wrong_pin_burst_is_rate_limited() {
    let b = pin_hub();
    let right = b.pin();
    let wrong = if right == "000000" {
        "000001"
    } else {
        "000000"
    };
    let pair = |pin: &str| json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "x"});
    let mut raw = ws(b.port);
    hello(&mut raw);
    for _ in 0..5 {
        assert_eq!(request(&mut raw, pair(wrong))["code"], "unauthenticated");
    }
    assert_eq!(
        request(&mut raw, pair(&right))["code"],
        "rate_limited",
        "even the right PIN"
    );
    assert_eq!(
        request(&mut raw, json!({"t": "auth.token", "id": 1, "token": "x"}))["code"],
        "rate_limited",
        "tokens too (spec 6.2)"
    );
    // A new socket from the same address gains nothing.
    let mut again = ws(b.port);
    hello(&mut again);
    assert_eq!(request(&mut again, pair(&right))["code"], "rate_limited");
    assert!(b.net.auth().clients().is_empty());
    let (c, _) = client(&b, None).unwrap();
    assert!(matches!(
        c.pair(&right, "admin"),
        Err(LinkError::RateLimited { .. })
    ));

    // The window died with the fifth failure. From another address nothing is left to try.
    let mut other = connect_from(Ipv4Addr::new(127, 0, 2, 1), b.port);
    other
        .write_all(request_text(b.port, &[]).as_bytes())
        .unwrap();
    assert_eq!(read_status(&mut other).0, 101);
    let mut other = WebSocket::from_raw_socket(other, WsRole::Client, None);
    assert_eq!(request(&mut other, pair(&right))["code"], "unauthenticated");
    b.hub.stop();
}

#[test]
fn wp_hb1_a_pin_is_good_for_one_client() {
    let b = pin_hub();
    let pin = b.pin();
    let pair = json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "x"});
    let mut first = ws(b.port);
    assert_eq!(request(&mut first, pair.clone())["t"], "ok");
    let mut second = ws(b.port);
    assert_eq!(request(&mut second, pair)["code"], "unauthenticated");
    assert_eq!(b.net.auth().clients().len(), 1);
    b.hub.stop();
}

/// A token again and again: fine while it is good, on as many sockets as there are, and
/// nothing once it is revoked.
#[test]
fn wp_hb1_a_revoked_token_cannot_be_replayed_and_live_sessions_close_4002() {
    let b = pin_hub();
    let (mut first, token) = b.admin();
    let cid = b.net.auth().clients()[0].cid;
    let auth = json!({"t": "auth.token", "id": 5, "token": token});
    let mut second = ws(b.port);
    let ok = request(&mut second, auth.clone());
    assert_eq!(ok["t"], "ok", "{ok}");
    assert!(
        ok.get("token").is_none(),
        "a token is only ever handed over once: {ok}"
    );
    assert!(b.net.is_online(cid));

    // On a socket that is already in: refused, and the session stands.
    let again = request(&mut second, auth.clone());
    assert_eq!(again["code"], "bad_request", "{again}");
    assert_eq!(
        request(&mut second, json!({"t": "device.list", "id": 6}))["t"],
        "ok"
    );

    assert!(b.net.revoke(cid));
    assert_eq!(close_code(&mut first), Some(4002));
    assert_eq!(close_code(&mut second), Some(4002));
    b.wait_connections(0);
    assert!(!b.net.is_online(cid));
    assert!(!b.net.revoke(cid), "already gone");

    let mut replay = ws(b.port);
    hello(&mut replay);
    let refused = request(&mut replay, auth);
    assert_eq!(refused["code"], "unauthenticated", "{refused}");
    let (_, state) = client(&b, Some(token)).unwrap();
    assert!(matches!(state, AuthState::TokenRefused { .. }), "{state:?}");
    b.hub.stop();
}

/// `auth.revoke` from an admin, of another client and of itself, which still hears `ok`.
#[test]
fn wp_hb1_revoking_over_the_wire_closes_the_clients_sessions() {
    let b = pin_hub();
    let (mut other, _) = b.admin();
    let (mut me, my_token) = b.admin();
    let ids: Vec<u32> = b.net.auth().clients().iter().map(|c| c.cid).collect();
    let (other_id, my_id) = (ids[0], ids[1]);

    let list = request(&mut me, json!({"t": "auth.list", "id": 1}));
    assert_eq!(list["clients"].as_array().unwrap().len(), 2);
    assert_eq!(list["clients"][0]["online"], true);
    assert!(list["clients"][0].get("token").is_none());
    assert!(!list.to_string().contains(&my_token));

    let reply = request(
        &mut me,
        json!({"t": "auth.revoke", "id": 2, "cid": other_id}),
    );
    assert_eq!(reply["t"], "ok", "{reply}");
    assert_eq!(close_code(&mut other), Some(4002));

    let reply = request(&mut me, json!({"t": "auth.revoke", "id": 3, "cid": my_id}));
    assert_eq!(reply["t"], "ok", "{reply}");
    assert_eq!(close_code(&mut me), Some(4002));
    assert!(b.net.auth().clients().is_empty());
    b.hub.stop();
}

/// A revoked session's command cannot run: after the revocation nothing of its is
/// submitted, even if its socket has not gone yet.
#[test]
fn wp_hb1_a_revoked_sessions_later_frames_do_not_run() {
    let b = bench(AccessMode::Pin, ServerLimits::default());
    let (mut raw, _) = b.admin();
    let cid = b.net.auth().clients()[0].cid;
    let (watcher, _) = b.admin();
    let (c, _) = client(&b, None).unwrap();
    drop(watcher);
    c.pair(&b.pin(), "admin").unwrap();
    b.promote(); // -- WP PAIRADM --
    let heard = c.notifications(0);
    assert!(b.net.revoke(cid));
    let volume = (-41.0f32).to_le_bytes();
    let late = Frame::CmdRequest(CmdRequest {
        tag: 1,
        handle: 0,
        dir: Dir::Set,
        b_request: op::REQ_SET_MASTER_VOLUME,
        w_value: 0,
        w_index: 2,
        w_length: 4,
        payload: volume.to_vec(),
    });
    let _ = raw.send(Message::binary(late.encode()));
    let _ = close_code(&mut raw);
    assert_eq!(
        c.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[])
            .status,
        status::OK
    );
    assert!(
        heard.recv_timeout(Duration::from_millis(300)).is_err(),
        "the device reported a change, so the write ran"
    );
    c.close();
    b.hub.stop();
}

/// A role change reaches a live session at once, without a new connection.
#[test]
fn wp_hb1_a_role_change_applies_to_the_live_session() {
    let b = pin_hub();
    let (mut raw, _) = b.admin();
    let cid = b.net.auth().clients()[0].cid;
    assert_eq!(
        request(&mut raw, json!({"t": "auth.list", "id": 1}))["t"],
        "ok"
    );

    assert!(b.net.set_role(cid, Role::Viewer));
    assert!(!b.net.set_role(cid + 100, Role::Admin));
    let reply = request(&mut raw, json!({"t": "auth.list", "id": 2}));
    assert_eq!(reply["code"], "denied", "{reply}");
    let reply = request(&mut raw, json!({"t": "auth.revoke", "id": 3, "cid": cid}));
    assert_eq!(reply["code"], "denied", "{reply}");
    let reply = request(
        &mut raw,
        json!({"t": "auth.set_role", "id": 3, "cid": cid, "role": "admin"}),
    );
    assert_eq!(
        reply["code"], "denied",
        "a viewer cannot promote itself: {reply}"
    );
    let reply = request(&mut raw, json!({"t": "lock.acquire", "id": 4, "handle": 0}));
    assert_eq!(reply["code"], "denied", "spec 7.7: {reply}");
    for t in ["hub.stats", "hub.rename"] {
        assert_eq!(
            request(&mut raw, json!({"t": t, "id": 5}))["code"],
            "denied"
        );
    }
    assert_eq!(
        request(&mut raw, json!({"t": "device.list", "id": 6}))["t"],
        "ok"
    );
    assert_eq!(
        b.net.auth().clients()[0].role,
        Role::Viewer,
        "and it is kept"
    );

    assert!(b.net.set_role(cid, Role::Control));
    let reply = request(
        &mut raw,
        json!({"t": "lock.acquire", "id": 7, "handle": 0, "timeout_ms": 999999}),
    );
    assert_eq!(reply["timeout_ms"], 60_000, "capped: {reply}");
    b.hub.stop();
}

#[test]
fn wp_hb1_a_role_below_admin_gets_its_denial_list() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    let pair = json!({"t": "auth.pair", "id": 1, "pin": b.pin(), "name": "x", "role": "viewer"});
    let ok = request(&mut raw, pair);
    assert_eq!(ok["role"], "viewer");
    assert!(
        !ok["policy"]["denied"].as_array().unwrap().is_empty(),
        "{ok}"
    );
    assert_eq!(
        request(&mut raw, json!({"t": "auth.list", "id": 2}))["code"],
        "denied"
    );
    let token = ok["token"].as_str().unwrap();
    let mut again = ws(b.port);
    let ok = request(
        &mut again,
        json!({"t": "auth.token", "id": 1, "token": token}),
    );
    assert_eq!(ok["role"], "viewer");
    assert!(!ok["policy"]["denied"].as_array().unwrap().is_empty());
    b.hub.stop();
}

/// Names a client sends are cut down before anything stores them.
#[test]
fn wp_hb1_a_clients_name_is_capped_on_the_wire() {
    let b = pin_hub();
    let mut raw = ws(b.port);
    let name = format!("\u{1b}[31m{}", "n".repeat(40_000));
    let reply = request(
        &mut raw,
        json!({"t": "auth.pair", "id": 1, "pin": b.pin(), "name": name, "role": "admin"}),
    );
    assert_eq!(reply["t"], "ok");
    let stored = &b.net.auth().clients()[0].name;
    assert_eq!(stored.chars().count(), dspi_hub::auth::MAX_CLIENT_NAME);
    assert!(!stored.contains('\u{1b}'));
    b.hub.stop();
}

// ---- Keepalive and a client that stops ----

#[test]
fn wp_hb1_the_hub_pings_and_a_peer_that_answers_stays() {
    let b = bench(AccessMode::Pin, quick());
    let (mut raw, _) = b.admin();
    let started = Instant::now();
    let mut pings = 0;
    while started.elapsed() < Duration::from_millis(1000) {
        // Reading is what answers: tungstenite queues the pong and sends it on the next read.
        match raw.read().unwrap() {
            Message::Ping(_) => pings += 1,
            Message::Close(frame) => panic!("closed: {frame:?}"),
            _ => {}
        }
    }
    assert!(pings >= 5, "{pings} pings in a second at 100 ms");
    assert_eq!(
        request(&mut raw, json!({"t": "device.list", "id": 1}))["t"],
        "ok"
    );
    b.hub.stop();
}

/// A vanished client: close 1001, the slot back, and its lock released (spec 7.7).
#[test]
fn wp_hb1_a_silent_peer_closes_with_1001_and_gives_up_its_lock() {
    let b = bench(AccessMode::Pin, quick());
    let (mut gone, _) = b.admin();
    let reply = request(
        &mut gone,
        json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": 60000}),
    );
    assert_eq!(reply["t"], "ok");
    let (mut other, _) = b.admin();
    // Keep the bystander answering on another thread while this one's silence runs out.
    let locked = request(
        &mut other,
        json!({"t": "lock.acquire", "id": 2, "handle": 0}),
    );
    assert_eq!(locked["code"], "locked");
    let started = Instant::now();
    wait_until("the silent peer to be dropped", || {
        let _ = other.read();
        b.net.connections() == 1
    });
    assert!(started.elapsed() < Duration::from_secs(3));
    let free = request(
        &mut other,
        json!({"t": "lock.acquire", "id": 3, "handle": 0}),
    );
    assert_eq!(free["t"], "ok", "{free}");
    assert_eq!(raw_close_code(gone.get_mut()), Some(1001));
    b.hub.stop();
}

/// A frame trickled and never finished is not an answer.
#[test]
fn wp_hb1_slowloris_on_a_frame_after_auth_closes_with_1001() {
    let b = bench(AccessMode::Pin, quick());
    let (mut raw, _) = b.admin();
    let frame = masked(0x81, &[100], &[b'x'; 100]);
    let started = Instant::now();
    for byte in frame.iter().take(100) {
        if raw.get_mut().write_all(&[*byte]).is_err()
            || started.elapsed() > Duration::from_millis(900)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    assert_eq!(raw_close_code(raw.get_mut()), Some(1001));
    b.wait_connections(0);
    b.hub.stop();
}

/// A client that sends and never reads. Its queue fills and its writes stall. It costs
/// itself the connection, and costs nobody else anything.
#[test]
fn wp_hb1_a_client_that_stops_reading_loses_its_connection_alone() {
    let b = bench(AccessMode::Pin, quick());
    let (mut bystander, _) = b.admin();
    let (mut hog, _) = b.admin();
    b.wait_connections(2);
    let sender = std::thread::spawn(move || {
        let started = Instant::now();
        let request = json!({"t": "device.list", "id": 1}).to_string();
        let frame = masked(0x81, &[request.len() as u8], request.as_bytes());
        let tcp = hog.get_mut();
        tcp.set_write_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        while started.elapsed() < WAIT {
            if tcp.write_all(&frame).is_err() {
                return;
            }
        }
    });
    wait_until("the hog to be dropped", || {
        assert_eq!(
            request(&mut bystander, json!({"t": "device.list", "id": 2}))["t"],
            "ok"
        );
        b.net.connections() == 1
    });
    sender.join().unwrap();
    b.hub.stop();
}

// ---- Events and stop ----

#[test]
fn wp_hb1_device_events_reach_a_session_and_not_a_socket_before_auth() {
    let b = pin_hub();
    let mut waiting = ws(b.port);
    hello(&mut waiting);
    let (mut raw, _) = b.admin();
    let handle = b.hub.devices()[0].handle;
    b.hub.detach(handle).unwrap();
    let changed = next_text(&mut raw, &["device.changed"]);
    assert_eq!(changed["device"]["state"], "offline", "{changed}");
    // Only an answer to this comes back: no event reached it.
    let reply = request(&mut waiting, json!({"t": "device.list", "id": 2}));
    assert_eq!(reply["code"], "unauthenticated");
    let snapshot = request(
        &mut raw,
        json!({"t": "device.snapshot", "id": 3, "handle": handle}),
    );
    assert_eq!(snapshot["code"], "no_device");
    let reply = request(
        &mut raw,
        json!({"t": "lock.acquire", "id": 4, "handle": handle}),
    );
    assert_eq!(reply["code"], "no_device");
    b.hub.stop();
}

#[test]
fn wp_hb1_stop_closes_everyone_with_4003_and_frees_the_port() {
    let b = bench(AccessMode::Pin, quick());
    let port = b.port;
    let (mut session, _) = b.admin();
    let mut waiting = ws(port);
    hello(&mut waiting);
    let mut in_http = connect(port);
    in_http.write_all(b"GET /").unwrap();
    wait_until("three sockets", || b.net.connections() == 3);

    b.hub.stop();

    assert!(b.hub.network().is_none());
    // -- WP P2MQ -- raw: the session has pings waiting before the close, and answering
    // one on the socket the hub has closed fails before the close is read (seen as a
    // flake once the `hello` grew by a cap).
    assert_eq!(raw_close_code(session.get_mut()), Some(4003));
    assert_eq!(close_code(&mut waiting), Some(4003));
    assert!(is_closed_by_peer(&mut in_http));
    assert_eq!(b.net.connections(), 0);
    assert!(
        TcpStream::connect((LOOPBACK, port)).is_err(),
        "the port is free"
    );
    b.hub.stop();
}

/// A peer that takes nothing cannot hold up a stop.
#[test]
fn wp_hb1_a_peer_that_will_not_read_does_not_hold_up_stop() {
    let b = bench(
        AccessMode::Pin,
        ServerLimits {
            close_grace: Duration::from_millis(200),
            ..ServerLimits::default()
        },
    );
    let mut stuck = Vec::new();
    for _ in 0..3 {
        let (raw, _) = b.admin();
        stuck.push(raw);
    }
    let mut waiting = ws(b.port);
    hello(&mut waiting);
    stuck.push(waiting);
    let started = Instant::now();
    b.hub.stop();
    let took = started.elapsed();
    assert!(took < Duration::from_secs(3), "{took:?}");
    assert_eq!(b.net.connections(), 0);
}

#[test]
fn wp_hb1_a_session_opened_during_a_stop_is_told_so() {
    // Sockets arriving as the hub stops: each gets its answer or a closed socket, and
    // the stop finishes.
    let b = bench(AccessMode::Open, quick());
    let port = b.port;
    let storm = std::thread::spawn(move || {
        let mut outcomes = 0;
        for _ in 0..200 {
            let Ok(tcp) = TcpStream::connect((LOOPBACK, port)) else {
                break;
            };
            tcp.set_read_timeout(Some(WAIT)).unwrap();
            let mut tcp = tcp;
            if tcp.write_all(request_text(port, &[]).as_bytes()).is_err() {
                continue;
            }
            if read_status(&mut tcp).0 != 101 {
                continue;
            }
            let mut raw = WebSocket::from_raw_socket(tcp, WsRole::Client, None);
            let _ = raw.send(Message::text(HELLO));
            let _ = close_code_soon(&mut raw);
            outcomes += 1;
        }
        outcomes
    });
    std::thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    b.hub.stop();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(b.net.connections(), 0);
    let _ = storm.join().unwrap();
}

fn close_code_soon(ws: &mut Ws) -> Option<u16> {
    let _ = ws
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)));
    match ws.read() {
        Ok(Message::Close(frame)) => frame.map(|f| u16::from(f.code)),
        _ => None,
    }
}

#[test]
fn wp_hb1_a_port_in_use_is_an_error_and_leaves_nothing_running() {
    let b = pin_hub();
    let result = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: b.port,
            ..network(store_dir(), AccessMode::Pin, ServerLimits::default())
        }),
        ..HubConfig::in_process("second")
    });
    assert!(matches!(result, Err(dspi_hub::HubError::Io(_))));
    b.hub.stop();
}

// ---- Over the wire: what HB2 and HB3 hang on this server ----

/// A device on older firmware: the name read stalls.
struct Older {
    inner: FakeDevice,
    descriptor: DeviceDescriptor,
}

impl Older {
    fn new(serial: &str) -> Self {
        Self {
            inner: FakeDevice::demo(),
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
                ..Default::default()
            },
        }
    }
}

impl Transport for Older {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if o == op::REQ_GET_DEVICE_NAME {
            return Err(TransportError::Stalled { opcode: o });
        }
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if o == op::REQ_GET_DEVICE_NAME {
            return Err(TransportError::Stalled { opcode: o });
        }
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

fn paired_as(b: &Bench, role: &str) -> Ws {
    let mut raw = ws(b.port);
    let ok = request(
        &mut raw,
        json!({"t": "auth.pair", "id": 1, "pin": b.pin(), "name": role, "role": role}),
    );
    assert_eq!(ok["role"], role, "{ok}");
    raw
}

/// Send a rename that will succeed. Its `ok` and its `device.changed` come in either
/// order: the first is the board's answer, the other its notification. The new name.
fn rename_ok(ws: &mut Ws, id: u64, handle: u8, name: &str) -> String {
    send(
        ws,
        &json!({"t": "device.rename", "id": id, "handle": handle, "name": name}),
    );
    let (mut answered, mut changed) = (false, None);
    while !answered || changed.is_none() {
        let v = next_text(ws, &["ok", "err", "device.changed"]);
        match v["t"].as_str().unwrap() {
            "ok" => {
                assert_eq!(v["id"], id, "{v}");
                answered = true;
            }
            "device.changed" => changed = Some(v["device"]["name"].as_str().unwrap().to_owned()),
            _ => panic!("{v}"),
        }
    }
    changed.unwrap()
}

/// `device.rename` from a Control session, on a board that stores names: `ok`, then
/// `device.changed` from its notification. A viewer may not; a lock holds it off.
#[test]
fn wp_hb1_a_control_session_renames_a_board_over_the_wire() {
    let b = pin_hub();
    let mut control = paired_as(&b, "control");
    let hello_caps = hello(&mut control);
    assert!(
        hello_caps["caps"]
            .as_array()
            .unwrap()
            .contains(&json!("rename"))
    );
    assert_eq!(rename_ok(&mut control, 2, 0, "Living room"), "Living room");
    assert_eq!(b.hub.devices()[0].name, "Living room");

    let mut viewer = paired_as(&b, "viewer");
    let refused = request(
        &mut viewer,
        json!({"t": "device.rename", "id": 3, "handle": 0, "name": "x"}),
    );
    assert_eq!(refused["code"], "denied", "{refused}");

    let (mut admin, _) = b.admin();
    let lock = request(
        &mut admin,
        json!({"t": "lock.acquire", "id": 4, "handle": 0}),
    );
    assert_eq!(lock["t"], "ok");
    let refused = request(
        &mut control,
        json!({"t": "device.rename", "id": 5, "handle": 0, "name": "y"}),
    );
    assert_eq!(refused["code"], "locked", "{refused}");
    let missing = request(
        &mut admin,
        json!({"t": "device.rename", "id": 6, "handle": 9, "name": "y"}),
    );
    assert_eq!(missing["code"], "no_device", "{missing}");
    b.hub.stop();
}

/// Older firmware: the hub keeps the name, on disk in its one store. Clients and names
/// share the file, so each must survive the other's saves, in both orders.
#[test]
fn wp_hb1_stored_names_and_clients_share_the_file_and_survive_a_restart() {
    let dir = store_dir();
    let start = || {
        let hub = Hub::start(HubConfig {
            network: Some(network(
                dir.clone(),
                AccessMode::Pin,
                ServerLimits::default(),
            )),
            ..HubConfig::in_process("Test hub")
        })
        .unwrap();
        let net = hub.network().unwrap();
        let port = net.port();
        Bench {
            hub,
            net,
            port,
            dir: dir.clone(),
        }
    };
    let b = start();
    let handle = b.hub.attach(Box::new(Older::new("OLD")), 0).unwrap();
    let (mut first, first_token) = b.admin();
    assert_eq!(rename_ok(&mut first, 2, handle, "Garage"), "Garage");
    // A pairing after the rename, and a rename after that pairing.
    let (mut second, second_token) = b.admin();
    assert_eq!(rename_ok(&mut second, 3, handle, "Garage 2"), "Garage 2");
    b.hub.stop();
    let text = fs::read_to_string(b.dir.join("hub-store.json")).unwrap();
    assert!(text.contains("Garage 2"), "{text}");

    let b = start();
    let handle = b.hub.attach(Box::new(Older::new("OLD")), 0).unwrap();
    let device = b
        .hub
        .devices()
        .into_iter()
        .find(|d| d.handle == handle)
        .unwrap();
    assert_eq!(device.name, "Garage 2", "kept across the restart");
    for token in [first_token, second_token] {
        let (c, state) = client(&b, Some(token)).unwrap();
        assert!(matches!(state, AuthState::Ready), "{state:?}");
        c.close();
    }
    b.hub.stop();
}

/// Polls and the snapshot, from the client's own calls, through the server.
#[test]
fn wp_hb1_polls_and_the_snapshot_work_over_the_wire() {
    let b = pin_hub();
    let (c, _) = client(&b, None).unwrap();
    c.pair(&b.pin(), "viewer").unwrap();
    assert!(!c.denied().is_empty());
    let snapshot = c.snapshot(0).unwrap();
    assert!(!snapshot.bulk().unwrap().is_empty());
    assert!(matches!(
        c.snapshot(9),
        Err(LinkError::Rejected { ref code, .. }) if code == "no_device"
    ));
    let polls = c.polls(0);
    let spec = PollSpec {
        slot: 1,
        req: op::REQ_GET_PLATFORM,
        val: 0,
        idx: 2,
        len: 7,
        hz: 20,
    };
    let granted = c.poll_subscribe(0, &[spec]).unwrap();
    assert_eq!(granted.len(), 1);
    assert_eq!(granted[0].slot, 1);
    let first = polls.recv_timeout(WAIT).unwrap();
    let second = polls.recv_timeout(WAIT).unwrap();
    assert_eq!(first.payload.len(), 7);
    assert_eq!(
        second.seq,
        first.seq.wrapping_add(1),
        "the writer's own sequence"
    );
    assert!(matches!(
        c.poll_subscribe(9, &[spec]),
        Err(LinkError::Rejected { ref code, .. }) if code == "no_device"
    ));
    let write = PollSpec {
        req: op::REQ_SET_MASTER_VOLUME,
        ..spec
    };
    assert!(matches!(
        c.poll_subscribe(0, &[write]),
        Err(LinkError::Rejected { ref code, .. }) if code == "denied"
    ));
    c.poll_unsubscribe(0, &[1]);
    c.close();
    b.hub.stop();
}

/// Role and payload limits, as `hello` advertises them, on real frames.
#[test]
fn wp_hb1_commands_are_judged_over_the_wire() {
    let b = pin_hub();
    let (viewer, _) = client(&b, None).unwrap();
    viewer.pair(&b.pin(), "viewer").unwrap();
    let read = viewer.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(read.status, status::OK);
    let volume = 0.0f32.to_le_bytes();
    let write = viewer.command(0, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, 2, 4, &volume);
    assert_eq!(write.status, status::DENIED);

    let (control, _) = client(&b, None).unwrap();
    control.pair(&b.pin(), "control").unwrap();
    let write = control.command(0, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, 2, 4, &volume);
    assert_eq!(write.status, status::OK);
    let big = vec![0u8; dspi_hub::router::MAX_PAYLOAD + 1];
    let too_big = control.command(
        0,
        Dir::Set,
        op::REQ_SET_ALL_PARAMS,
        0,
        2,
        big.len() as u16,
        &big,
    );
    assert_eq!(too_big.status, status::TOO_LARGE);
    assert_eq!(
        viewer.hello().limits.max_payload as usize,
        dspi_hub::router::MAX_PAYLOAD
    );
    assert_eq!(
        viewer.hello().limits.max_inflight,
        dspi_hub::hub::MAX_INFLIGHT
    );
    assert_eq!(viewer.hello().limits.poll_max_hz, dspi_hub::POLL_MAX_HZ);
    viewer.close();
    control.close();
    b.hub.stop();
}

/// The control messages HB2 and HB3 handle, with what a hostile admin can put in them.
/// Answered, with no panic, and the session and the hub go on.
#[test]
fn wp_hb1_hostile_lock_poll_and_rename_messages_are_answered() {
    let b = pin_hub();
    let (mut raw, _) = b.admin();
    let spec =
        json!({"slot": 1, "req": op::REQ_GET_PLATFORM, "val": 0, "idx": 2, "len": 7, "hz": 20});
    let many: Vec<Value> = std::iter::repeat_n(spec.clone(), 800).collect();
    let huge_len =
        json!({"slot": 2, "req": op::REQ_GET_PLATFORM, "val": 0, "idx": 2, "len": 65535, "hz": 20});
    let huge_hz = json!({"slot": 3, "req": op::REQ_GET_PLATFORM, "val": 0, "idx": 2, "len": 7, "hz": 4294967295u32});
    let zero_hz =
        json!({"slot": 4, "req": op::REQ_GET_PLATFORM, "val": 0, "idx": 2, "len": 7, "hz": 0});
    let long_name = "n\u{1b}\u{0}".repeat(4_000);
    for message in [
        json!({"t": "poll.subscribe", "id": 1, "handle": 0, "polls": many}),
        json!({"t": "poll.subscribe", "id": 1, "handle": 0, "polls": [huge_len]}),
        json!({"t": "poll.subscribe", "id": 1, "handle": 0, "polls": [huge_hz]}),
        json!({"t": "poll.subscribe", "id": 1, "handle": 0, "polls": [zero_hz]}),
        json!({"t": "poll.unsubscribe", "id": 1, "handle": 0, "slots": [1, 1, 255, 0]}),
        json!({"t": "poll.unsubscribe", "id": 1, "handle": 200, "slots": [1]}),
        json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": 1e308}),
        json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": u64::MAX}),
        json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": 0}),
        json!({"t": "lock.release", "id": 1, "handle": 0}),
        json!({"t": "lock.release", "id": 1, "handle": 200}),
        json!({"t": "device.rename", "id": 1, "handle": 0, "name": long_name}),
        json!({"t": "device.rename", "id": 1, "handle": 200, "name": "x"}),
        json!({"t": "device.rename", "id": 1, "handle": 0, "name": 7}),
    ] {
        let reply = request(&mut raw, message.clone());
        assert_eq!(reply["id"], 1, "{reply}");
    }
    assert_eq!(
        request(&mut raw, json!({"t": "device.list", "id": 2}))["t"],
        "ok"
    );
    b.hub.stop();
}

// -- WP PGHDEV --
/// The client's `hello` minor reaches its session: on an open hub with a paged
/// (1.1.7) device, a page-0 CMD from a minor-2 client is translated and answered,
/// the same CMD from a minor-3 client is `0x10`. The device object says `protocol`;
/// a Viewer's `ok` carries `policy.denied_ids` next to `denied`.
#[test]
fn wp_pghdev_the_hello_minor_decides_page_0_on_a_paged_device() {
    use dspi_transport::fake::FakeGeneration;

    let open = bench(AccessMode::Open, ServerLimits::default());
    let paged = FakeDevice::demo_generation(FakeGeneration::STOCK117).with_serial("PGHDEV-WS");
    let h = open.hub.attach(Box::new(paged), 0).unwrap();
    let read = Frame::CmdRequest(CmdRequest {
        tag: 11,
        handle: h,
        dir: Dir::Get,
        b_request: op::REQ_GET_MASTER_VOLUME,
        w_value: 0,
        w_index: 2,
        w_length: 4,
        payload: Vec::new(),
    });
    for (minor, want) in [(2u32, status::OK), (3, 0x10)] {
        let mut raw = ws(open.port);
        raw.send(Message::text(
            json!({"t": "hello", "proto": {"major": 1, "minor": minor}, "client": {"name": "raw"}})
                .to_string(),
        ))
        .unwrap();
        let h = next_text(&mut raw, &["hello"]);
        assert_eq!(h["proto"]["minor"], 3);
        let list = request(&mut raw, json!({"t": "device.list", "id": 1}));
        let device = list["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["serial"] == "PGHDEV-WS")
            .unwrap()
            .clone();
        assert_eq!(
            device["protocol"],
            json!({"major": 1, "minor": 0}),
            "{device}"
        );
        assert_eq!(device["ext"], json!(false));
        raw.send(Message::binary(read.encode())).unwrap();
        assert_eq!(binary_response(&mut raw).status, want, "minor {minor}");
    }
    open.hub.stop();

    let b = pin_hub();
    let mut raw = ws(b.port);
    let pair = json!({"t": "auth.pair", "id": 1, "pin": b.pin(), "name": "x", "role": "viewer"});
    let ok = request(&mut raw, pair);
    let ids = ok["policy"]["denied_ids"].as_array().unwrap();
    // ROUTE.MASTER_VOLUME SET is `control`: denied to a Viewer, by id.
    assert!(ids.contains(&json!([0x0550, Dir::Set as u16])), "{ok}");
    assert!(!ok["policy"]["denied"].as_array().unwrap().is_empty());
    b.hub.stop();
}
