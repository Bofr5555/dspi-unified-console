//! WP SECSRV: TLS on the DSPi Link port (PLAN-SEC 4.1, 4.6, 4.7), the upstream
//! compatibility mode, and the `Host` rule (DNS rebinding).
//!
//! Clients here are dspi-link's own TLS (`dspi_link::tls`, pinned to the hub's key) and
//! hand-written HTTP and WebSocket bytes, so every answer is seen as the wire carries it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::server::{ServerLimits, TLS_REQUIRED_REASON, UPSTREAM_COMPAT_EFFECTS};
use dspi_hub::tls_identity::TlsIdentityEvent;
use dspi_hub::{
    AccessMode, HealthConfig, Hub, HubConfig, HubEvent, HubListener, NetworkConfig, Role,
    ServiceConfig, TlsMode, TlsPolicy, downgrade_warning,
};
use dspi_link::tls::{self as link_tls, PinMode, SpkiHash, TlsError, TlsPipe};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use socket2::{Domain, Socket, Type};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

fn store_dir() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    // ci/run.sh points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("wp-secsrv").join(format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[derive(Default)]
struct Events(Mutex<Vec<HubEvent>>);

impl HubListener for Events {
    fn on_event(&self, event: HubEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct Target {
    hub: Hub,
    net: dspi_hub::server::NetworkControl,
    port: u16,
    store: std::path::PathBuf,
    events: Arc<Events>,
}

impl Target {
    fn pin(&self) -> SpkiHash {
        self.net.tls().pin.expect("TLS is served")
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.hub.stop();
    }
}

struct Opts {
    tls: TlsMode,
    compat: bool,
    web: bool,
    rest: bool,
    limits: ServerLimits,
    host_names: Vec<String>,
}

fn opts(tls: TlsMode) -> Opts {
    Opts {
        tls,
        compat: false,
        web: false,
        rest: false,
        limits: fast(),
        host_names: Vec::new(),
    }
}

fn start(o: Opts) -> Target {
    let store = store_dir();
    let cfg = NetworkConfig {
        port: 0,
        access: AccessMode::Pin,
        web: o.web,
        rest: o.rest,
        bind: IpAddr::V4(LOOPBACK),
        limits: o.limits,
        tls: o.tls,
        upstream_compat: o.compat,
        host_names: o.host_names,
        ..NetworkConfig::new(store.clone())
    };
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        // health.get behind /api/v1/health
        services: ServiceConfig {
            health: Some(HealthConfig::default()),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("SECSRV hub")
    })
    .unwrap();
    let events = Arc::new(Events::default());
    hub.handle().set_listener(Some(events.clone()));
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Target {
        hub,
        net,
        port,
        store,
        events,
    }
}

fn fast() -> ServerLimits {
    ServerLimits {
        head_timeout: Duration::from_millis(600),
        auth_timeout: Duration::from_millis(2000),
        close_grace: Duration::from_millis(300),
        read_tick: Duration::from_millis(20),
        ..ServerLimits::default()
    }
}

/// A TCP socket from a chosen loopback source (127.0.0.x).
fn raw_from(src: Option<Ipv4Addr>, port: u16) -> TcpStream {
    let s = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    if let Some(ip) = src {
        s.bind(&SocketAddr::new(IpAddr::V4(ip), 0).into()).unwrap();
    }
    s.connect(&SocketAddr::new(IpAddr::V4(LOOPBACK), port).into())
        .unwrap();
    let tcp: TcpStream = s.into();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp
}

fn tls_connect(port: u16, mode: PinMode) -> Result<TlsPipe, TlsError> {
    let tcp = raw_from(None, port);
    let deadline = Instant::now() + Duration::from_secs(3);
    let (mut pipe, _) = link_tls::connect(tcp, "localhost", mode, deadline)?;
    pipe.set_deadline(None);
    Ok(pipe)
}

fn tls(t: &Target) -> TlsPipe {
    tls_connect(t.port, PinMode::Pinned(t.pin())).expect("a pinned TLS connection")
}

/// One request with `Connection: close`: (status, head, body).
fn http<S: Read + Write>(mut s: S, request: &str) -> (u16, String, String) {
    s.write_all(request.as_bytes()).unwrap();
    s.flush().unwrap();
    let mut all = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => all.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&all).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, head.to_owned(), body.to_owned())
}

fn get(path: &str, host: &str, extra: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{extra}\r\n")
}

fn upgrade_bytes(host: &str, origin: Option<&str>) -> String {
    let mut r = format!(
        "GET /dspi/v1 HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n"
    );
    if let Some(o) = origin {
        r.push_str(&format!("Origin: {o}\r\n"));
    }
    r.push_str("\r\n");
    r
}

/// Read up to the end of an HTTP head, a byte at a time (nothing behind it is taken).
fn read_head<S: Read>(s: &mut S) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => buf.push(byte[0]),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn status_of(head: &str) -> u16 {
    head.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

/// A WebSocket client over any stream.
struct Ws<S: Read + Write>(WebSocket<S>);

impl<S: Read + Write> Ws<S> {
    fn open(mut s: S, host: &str, origin: Option<&str>) -> Result<Self, u16> {
        s.write_all(upgrade_bytes(host, origin).as_bytes()).unwrap();
        s.flush().unwrap();
        let code = status_of(&read_head(&mut s));
        if code != 101 {
            return Err(code);
        }
        Ok(Self(WebSocket::from_raw_socket(s, WsRole::Client, None)))
    }

    fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).unwrap();
    }

    fn recv(&mut self) -> Option<Value> {
        for _ in 0..200 {
            match self.0.read() {
                Ok(Message::Text(t)) => return serde_json::from_str(t.as_str()).ok(),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    }

    fn ask(&mut self, v: Value) -> Value {
        let id = v["id"].clone();
        self.send(v);
        loop {
            let r = self.recv().expect("a reply");
            if r["id"] == id {
                return r;
            }
        }
    }

    fn close_frame(&mut self) -> Option<(u16, String)> {
        for _ in 0..200 {
            match self.0.read() {
                Ok(Message::Close(Some(f))) => {
                    return Some((u16::from(f.code), f.reason.to_string()));
                }
                Ok(Message::Close(None)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    }
}

fn hello<S: Read + Write>(ws: &mut Ws<S>) -> Value {
    ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2}, "client": {"name": "secsrv", "kind": "test"}}));
    ws.recv().expect("hello")
}

fn pair<S: Read + Write>(t: &Target, ws: &mut Ws<S>) -> Value {
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    ws.ask(json!({"t": "auth.pair", "id": 1, "pin": w.pin, "name": "secsrv"}))
}

fn host(t: &Target) -> String {
    format!("127.0.0.1:{}", t.port)
}

// ───────────────────────────── conformance over TLS ─────────────────────────────

#[test]
fn wp_secsrv_pair_and_reconnect_over_tls() {
    let t = start(opts(TlsMode::Require));
    let mut ws = Ws::open(tls(&t), &host(&t), None).unwrap();
    let h = hello(&mut ws);
    assert_eq!(h["t"], "hello", "{h}");
    let ok = pair(&t, &mut ws);
    assert_eq!(ok["t"], "ok", "{ok}");
    let token = ok["token"].as_str().unwrap().to_owned();
    drop(ws);

    let mut ws = Ws::open(tls(&t), &host(&t), None).unwrap();
    hello(&mut ws);
    let ok = ws.ask(json!({"t": "auth.token", "id": 2, "token": token}));
    assert_eq!(ok["t"], "ok", "{ok}");
    // A command, through the session over TLS
    let r = ws.ask(json!({"t": "device.list", "id": 3}));
    assert_eq!(r["t"], "ok", "{r}");
}

#[test]
fn wp_secsrv_info_page_and_rest_over_tls() {
    let mut o = opts(TlsMode::Require);
    o.web = true;
    o.rest = true;
    let t = start(o);
    let pin = t.pin();

    let (status, _, body) = http(tls(&t), &get("/dspi/v1/info", &host(&t), ""));
    assert_eq!(status, 200);
    let info: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(info["tls"], true);
    assert_eq!(info["tls_spki"], pin.to_b64url());

    // A static page, its policy naming wss://
    let (status, head, _) = http(tls(&t), &get("/hub/", &host(&t), ""));
    assert_eq!(status, 200, "{head}");
    assert!(head.contains(&format!("wss://{}", host(&t))), "{head}");
    let (status, _, _) = http(tls(&t), &get("/", &host(&t), ""));
    assert_eq!(status, 200);

    // REST with a key that was never allowed plaintext
    let key = t
        .net
        .auth()
        .mint_api_key("ha", Role::Viewer, false)
        .unwrap();
    let auth = format!("Authorization: Bearer {}\r\n", key.key);
    let (status, _, body) = http(tls(&t), &get("/api/v1/health", &host(&t), &auth));
    assert_eq!(status, 200, "{body}");
}

#[test]
fn wp_secsrv_a_wrong_pin_gets_no_byte() {
    let t = start(opts(TlsMode::On));
    let e = tls_connect(t.port, PinMode::Pinned(SpkiHash([7; 32]))).err();
    assert_eq!(e, Some(TlsError::KeyMismatch));
}

// ───────────────────────────── on: both ─────────────────────────────

#[test]
fn wp_secsrv_on_serves_both_and_refuses_plaintext_pairing() {
    let t = start(opts(TlsMode::On));
    for (status, ..) in [
        http(tls(&t), &get("/dspi/v1/info", &host(&t), "")),
        http(raw_from(None, t.port), &get("/dspi/v1/info", &host(&t), "")),
    ] {
        assert_eq!(status, 200);
    }
    // Pairing over plaintext: refused before the PIN is looked at, and the window
    // stays good for a TLS client
    let mut plain = Ws::open(raw_from(None, t.port), &host(&t), None).unwrap();
    hello(&mut plain);
    let refused = pair(&t, &mut plain);
    assert_eq!(refused["code"], "tls_required", "{refused}");
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    let mut ws = Ws::open(tls(&t), &host(&t), None).unwrap();
    hello(&mut ws);
    let ok = ws.ask(json!({"t": "auth.pair", "id": 1, "pin": w.pin, "name": "x"}));
    assert_eq!(ok["t"], "ok", "{ok}");
    // A token works over plaintext in `on` (clients not yet updated)
    let token = ok["token"].as_str().unwrap().to_owned();
    let mut plain = Ws::open(raw_from(None, t.port), &host(&t), None).unwrap();
    hello(&mut plain);
    let ok = plain.ask(json!({"t": "auth.token", "id": 2, "token": token}));
    assert_eq!(ok["t"], "ok", "{ok}");
}

#[test]
fn wp_secsrv_plaintext_rest_needs_plaintext_ok_in_on() {
    let mut o = opts(TlsMode::On);
    o.rest = true;
    let t = start(o);
    let strict = t
        .net
        .auth()
        .mint_api_key("strict", Role::Viewer, false)
        .unwrap();
    let legacy = t
        .net
        .auth()
        .mint_api_key("legacy", Role::Viewer, true)
        .unwrap();
    let ask = |key: &str| {
        let auth = format!("Authorization: Bearer {key}\r\n");
        http(
            raw_from(None, t.port),
            &get("/api/v1/health", &host(&t), &auth),
        )
    };
    let (status, head, body) = ask(&strict.key);
    assert_eq!(status, 426, "{head}");
    assert!(body.contains("tls_required"), "{body}");
    assert_eq!(ask(&legacy.key).0, 200);
    // and over TLS the strict key works
    let auth = format!("Authorization: Bearer {}\r\n", strict.key);
    assert_eq!(
        http(tls(&t), &get("/api/v1/health", &host(&t), &auth)).0,
        200
    );
}

#[test]
fn wp_secsrv_upstream_compat_lets_plaintext_pair_and_use_any_key() {
    let mut o = opts(TlsMode::On);
    o.rest = true;
    o.compat = true;
    let t = start(o);
    let mut plain = Ws::open(raw_from(None, t.port), &host(&t), None).unwrap();
    hello(&mut plain);
    let ok = pair(&t, &mut plain);
    assert_eq!(ok["t"], "ok", "{ok}");
    let key = t.net.auth().mint_api_key("k", Role::Viewer, false).unwrap();
    let auth = format!("Authorization: Bearer {}\r\n", key.key);
    let (status, ..) = http(
        raw_from(None, t.port),
        &get("/api/v1/health", &host(&t), &auth),
    );
    assert_eq!(status, 200);
    // TLS is still there for updated clients
    assert_eq!(http(tls(&t), &get("/dspi/v1/info", &host(&t), "")).0, 200);
}

#[test]
fn wp_secsrv_upstream_compat_with_require_is_refused() {
    let mut o = opts(TlsMode::Require);
    o.compat = true;
    let r = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            bind: IpAddr::V4(LOOPBACK),
            tls: o.tls,
            upstream_compat: o.compat,
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process("x")
    });
    assert!(r.is_err());
}

#[test]
fn wp_secsrv_the_downgrade_warning_lists_what_is_off() {
    let compat = TlsPolicy {
        mode: TlsMode::On,
        upstream_compat: true,
    };
    let w = downgrade_warning(compat, false).unwrap();
    assert!(w.contains("SECURITY DOWNGRADED"));
    for effect in UPSTREAM_COMPAT_EFFECTS {
        assert!(w.contains(effect), "{effect}");
    }
    let off = TlsPolicy {
        mode: TlsMode::Off,
        upstream_compat: false,
    };
    assert!(
        downgrade_warning(off, false)
            .unwrap()
            .contains("TLS IS OFF")
    );
    assert_eq!(
        downgrade_warning(off, true),
        None,
        "loopback crosses no network"
    );
    let require = TlsPolicy {
        mode: TlsMode::Require,
        upstream_compat: false,
    };
    assert_eq!(downgrade_warning(require, false), None);
    let on = TlsPolicy {
        mode: TlsMode::On,
        upstream_compat: false,
    };
    assert_eq!(downgrade_warning(on, false), None);
}

// ───────────────────────────── require ─────────────────────────────

#[test]
fn wp_secsrv_require_answers_426_and_closes_a_plaintext_websocket_4003() {
    let mut o = opts(TlsMode::Require);
    o.web = true;
    o.rest = true;
    let t = start(o);
    for path in ["/dspi/v1/info", "/", "/api/v1/health"] {
        let (status, head, body) = http(raw_from(None, t.port), &get(path, &host(&t), ""));
        assert_eq!(status, 426, "{path}: {head}");
        assert!(head.contains("Upgrade: TLS/1.3"), "{head}");
        assert!(body.contains(&t.pin().display()), "the fingerprint: {body}");
    }
    let mut ws = Ws::open(raw_from(None, t.port), &host(&t), None).expect("upgraded");
    let (code, reason) = ws.close_frame().expect("a close frame");
    assert_eq!(code, 4003);
    assert_eq!(reason, TLS_REQUIRED_REASON);
    assert!(reason.starts_with("tls_required"));
}

// ───────────────────────────── Host and Origin ─────────────────────────────

#[test]
fn wp_secsrv_a_rebound_host_is_refused_on_info_and_assets() {
    let mut o = opts(TlsMode::Off);
    o.web = true;
    o.host_names = vec!["studio.lan".into()];
    let t = start(o);
    let p = t.port;
    for path in ["/dspi/v1/info", "/", "/hub/"] {
        for evil in [
            format!("evil.example:{p}"),
            format!("attacker.rebind.example:{p}"),
        ] {
            let (status, ..) = http(raw_from(None, p), &get(path, &evil, ""));
            assert_eq!(status, 421, "{path} {evil}");
        }
        for fine in [
            format!("127.0.0.1:{p}"),
            format!("localhost:{p}"),
            format!("[::1]:{p}"),
            format!("10.1.2.3:{p}"),
            format!("studio.lan:{p}"),
            format!("STUDIO.lan:{p}"),
            "x".to_owned(),
        ] {
            let (status, ..) = http(raw_from(None, p), &get(path, &fine, ""));
            assert_eq!(status, 200, "{path} {fine}");
        }
    }
    // The WebSocket too
    let refused = Ws::open(raw_from(None, p), &format!("evil.example:{p}"), None).err();
    assert_eq!(refused, Some(421));
    // Over TLS a name is fine: a rebound one fails the browser's certificate check
    let t = start(opts(TlsMode::On));
    let (status, ..) = http(
        tls(&t),
        &get("/dspi/v1/info", &format!("evil.example:{}", t.port), ""),
    );
    assert_eq!(status, 200);
}

#[test]
fn wp_secsrv_https_origins_own_accepted_foreign_refused() {
    let mut o = opts(TlsMode::On);
    o.web = true;
    let t = start(o);
    let p = t.port;
    let own = format!("https://127.0.0.1:{p}");
    assert!(Ws::open(tls(&t), &host(&t), Some(&own)).is_ok());
    assert!(Ws::open(raw_from(None, p), &host(&t), Some(&own)).is_ok());
    for foreign in [
        format!("https://evil.example:{p}"),
        format!("https://127.0.0.1:{}", p + 1),
    ] {
        assert_eq!(
            Ws::open(tls(&t), &host(&t), Some(&foreign)).err(),
            Some(403),
            "{foreign}"
        );
    }
    // Over TLS, the page of the very name it was asked at
    let named = format!("hub.example:{p}");
    assert!(Ws::open(tls(&t), &named, Some(&format!("https://{named}"))).is_ok());
    // A plaintext hub still refuses https (it serves none)
    let mut o = opts(TlsMode::Off);
    o.web = true;
    let t = start(o);
    let own = format!("https://127.0.0.1:{}", t.port);
    assert_eq!(
        Ws::open(raw_from(None, t.port), &host(&t), Some(&own)).err(),
        Some(403)
    );
}

// ───────────────────────────── budgets ─────────────────────────────

#[test]
fn wp_secsrv_a_slow_handshake_is_cut_by_head_timeout() {
    let t = start(opts(TlsMode::On));
    let mut tcp = raw_from(None, t.port);
    // A handshake record's header and nothing after it
    tcp.write_all(&[0x16, 0x03, 0x01, 0x00, 0x40]).unwrap();
    let from = Instant::now();
    let mut buf = [0u8; 64];
    let gone = matches!(tcp.read(&mut buf), Ok(0) | Err(_));
    let took = from.elapsed();
    assert!(gone, "the hub closed it");
    assert!(
        took < Duration::from_millis(1500),
        "within head_timeout: {took:?}"
    );
    assert!(took >= Duration::from_millis(300), "not at once: {took:?}");
}

#[test]
fn wp_secsrv_half_open_handshakes_do_not_starve_others() {
    let mut o = opts(TlsMode::Require);
    o.limits = ServerLimits {
        head_timeout: Duration::from_secs(10),
        max_http: 8,
        max_http_per_peer: 4,
        ..fast()
    };
    let t = start(o);
    // One machine holds many half-open handshakes
    let hostile: Vec<TcpStream> = (0..12)
        .map(|_| {
            let mut s = raw_from(Some(Ipv4Addr::new(127, 0, 0, 2)), t.port);
            s.write_all(&[0x16, 0x03, 0x01]).unwrap();
            s
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    assert!(t.net.connections() <= 8, "bounded: {}", t.net.connections());
    // Another machine still gets through, over TLS, at once
    let from = Instant::now();
    let (status, ..) = http(tls(&t), &get("/dspi/v1/info", &host(&t), ""));
    assert_eq!(status, 200);
    assert!(from.elapsed() < Duration::from_secs(3));
    // And still pairs
    let mut ws = Ws::open(tls(&t), &host(&t), None).unwrap();
    hello(&mut ws);
    assert_eq!(pair(&t, &mut ws)["t"], "ok");
    drop(hostile);
}

// ───────────────────────────── the identity ─────────────────────────────

#[test]
fn wp_secsrv_rotation_serves_the_new_key_and_tells_the_embedder() {
    let t = start(opts(TlsMode::Require));
    let old = t.pin();
    let event = t.net.rotate_tls_key(false).unwrap();
    let new = t.pin();
    assert_ne!(old, new);
    assert!(matches!(
        event,
        TlsIdentityEvent::Rotated { signed: true, .. }
    ));
    assert_eq!(
        tls_connect(t.port, PinMode::Pinned(old)).err(),
        Some(TlsError::KeyMismatch)
    );
    assert!(tls_connect(t.port, PinMode::Pinned(new)).is_ok());
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let seen = t
            .events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, HubEvent::Tls(TlsIdentityEvent::Rotated { .. })));
        if seen {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no HubEvent::Tls(Rotated)");
}

#[test]
fn wp_secsrv_a_first_start_reports_its_key_and_shows_no_secret() {
    let t = start(opts(TlsMode::On));
    let info = t.net.tls();
    assert!(matches!(
        info.at_start.as_slice(),
        [TlsIdentityEvent::Created { .. }]
    ));
    assert!(info.certificate_pem.unwrap().contains("BEGIN CERTIFICATE"));
    let key = std::fs::read_to_string(t.store.join("tls").join("key.pem")).unwrap();
    let secret: String = key
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let debug = format!(
        "{:?} {:?} {:?}",
        t.net,
        t.net.tls(),
        t.events.0.lock().unwrap()
    );
    assert!(!debug.contains(&secret[..32]), "key material in Debug");
    // An `off` hub makes no key
    let t = start(opts(TlsMode::Off));
    assert_eq!(t.net.tls().pin, None);
    assert!(!t.store.join("tls").exists());
    let (status, _, body) = http(raw_from(None, t.port), &get("/dspi/v1/info", &host(&t), ""));
    assert_eq!(status, 200);
    let info: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(info["tls"], false);
    assert!(info.get("tls_spki").is_none());
}

#[test]
fn wp_secsrv_an_off_hub_answers_a_client_hello_at_once() {
    let mut o = opts(TlsMode::Off);
    o.limits = ServerLimits {
        head_timeout: Duration::from_secs(10),
        ..fast()
    };
    let t = start(o);
    let from = Instant::now();
    let e = tls_connect(t.port, PinMode::FirstUse).err();
    assert!(e.is_some(), "no TLS on an off hub");
    assert!(
        from.elapsed() < Duration::from_millis(1500),
        "not after the head timeout: {:?}",
        from.elapsed()
    );
}
