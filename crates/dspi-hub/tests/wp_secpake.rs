//! WP SECPAKE: `auth.pair2`, SPAKE2 pairing bound to the TLS exporter (PLAN-SEC
//! SECPAKE, OD-SECc (3)), against the real hub and the core's real client, and the
//! SEC-C review's hub nits (`key_next` on the services' clock, `auth.rotate` after a
//! key rotation).
//!
//! The machine in the middle is a real one: a proxy that terminates the client's TLS
//! with its own key (a client pairing for the first time trusts it) and opens its own
//! TLS to the hub, relaying every WebSocket message and keeping what it reads. Against
//! the legacy `auth.pair` it reads the PIN and pairs (the control); against
//! `auth.pair2` it fails at the first guess and has nothing to search offline (the
//! offline search itself is `dspi-link`'s `pake/tests_wp_secpake.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use dspi_hub::auth::PairingState;
use dspi_hub::server::ServerLimits;
use dspi_hub::{
    AccessMode, Hub, HubClock, HubConfig, NetworkConfig, ServiceConfig, TestClock, TlsMode,
};
use dspi_link::LinkError;
use dspi_link::client::{
    AuthState, ConnectOptions, LinkClient, LinkEvents, SessionConfig, TlsOptions,
};
use dspi_link::fakehub::FakeTlsKey;
use dspi_link::pake::{self, Binding, ClientPake};
use dspi_link::socket::TcpConnector;
use dspi_link::tls::{self as link_tls, PinMode, SpkiHash, TlsPipe};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;
const DAY: u64 = 86_400;

// ───────────────────────────── a logger that keeps everything ─────────────────────────────

struct Keep;

fn kept() -> &'static Mutex<Vec<String>> {
    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    LINES.get_or_init(|| Mutex::new(Vec::new()))
}

impl log::Log for Keep {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        !m.target().starts_with("tungstenite")
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            kept()
                .lock()
                .unwrap()
                .push(format!("{} {}: {}", r.level(), r.target(), r.args()));
        }
    }

    fn flush(&self) {}
}

fn keep_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        static KEEP: Keep = Keep;
        let _ = log::set_logger(&KEEP);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// Every PIN and token a test handled, for the last test to look for in the logs.
fn secrets() -> &'static Mutex<Vec<String>> {
    static S: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

fn secret(s: &str) {
    secrets().lock().unwrap().push(s.to_owned());
}

/// Whether `needle` stands in `text` on its own (a PIN inside a longer number is chance).
fn has_word(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

// ───────────────────────────── the hub ─────────────────────────────

fn store_dir() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    // ci/run.sh points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("wp-secpake").join(format!(
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

impl Target {
    fn pin(&self) -> SpkiHash {
        self.net.tls().pin.expect("TLS is served")
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/dspi/v1", self.port)
    }

    fn open_pin(&self) -> String {
        let w = self.net.auth().begin_pairing(Instant::now()).unwrap();
        secret(&w.pin);
        w.pin
    }

    fn state(&self) -> PairingState {
        self.net.auth().pairing_state(Instant::now())
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.hub.stop();
    }
}

fn start_with(tls: TlsMode, compat: bool, clock: Option<Arc<TestClock>>) -> Target {
    keep_logs();
    let cfg = NetworkConfig {
        port: 0,
        access: AccessMode::Pin,
        bind: IpAddr::V4(LOOPBACK),
        limits: ServerLimits {
            head_timeout: Duration::from_millis(1500),
            auth_timeout: Duration::from_millis(5000),
            close_grace: Duration::from_millis(300),
            read_tick: Duration::from_millis(20),
            ..ServerLimits::default()
        },
        tls,
        upstream_compat: compat,
        ..NetworkConfig::new(store_dir())
    };
    let mut services = ServiceConfig::default();
    if let Some(clock) = clock {
        services.clock = clock as Arc<dyn HubClock>;
    }
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        services,
        ..HubConfig::in_process("SECPAKE hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Target { hub, net, port }
}

fn start(tls: TlsMode) -> Target {
    start_with(tls, false, None)
}

// ───────────────────────────── a raw WebSocket client ─────────────────────────────

fn tcp(port: u16) -> TcpStream {
    let tcp = TcpStream::connect((LOOPBACK, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp
}

fn tls_to(port: u16, mode: PinMode) -> TlsPipe {
    let (mut pipe, _) = link_tls::connect(
        tcp(port),
        "localhost",
        mode,
        Instant::now() + Duration::from_secs(3),
    )
    .unwrap();
    pipe.set_deadline(None);
    pipe
}

struct Ws<S: Read + Write>(WebSocket<S>);

impl<S: Read + Write> Ws<S> {
    fn open(mut s: S, port: u16) -> Self {
        let upgrade = format!(
            "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
             Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Protocol: dspi-link-1\r\n\r\n"
        );
        s.write_all(upgrade.as_bytes()).unwrap();
        s.flush().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match s.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => head.push(byte[0]),
            }
        }
        assert!(
            head.starts_with(b"HTTP/1.1 101"),
            "{}",
            String::from_utf8_lossy(&head)
        );
        let mut ws = Self(WebSocket::from_raw_socket(s, WsRole::Client, None));
        let hello = ws.ask_raw(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "secpake", "kind": "test"}}));
        assert_eq!(hello["t"], "hello", "{hello}");
        ws
    }

    fn hello(&mut self) -> Value {
        self.ask_raw(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "secpake", "kind": "test"}}))
    }

    fn ask_raw(&mut self, v: Value) -> Value {
        self.0.send(Message::text(v.to_string())).unwrap();
        self.recv()
    }

    fn recv(&mut self) -> Value {
        for _ in 0..200 {
            match self.0.read() {
                Ok(Message::Text(t)) => return serde_json::from_str(t.as_str()).unwrap(),
                Ok(Message::Close(_)) | Err(_) => return Value::Null,
                Ok(_) => {}
            }
        }
        Value::Null
    }

    /// `v` sent once; the reply with its id.
    fn ask(&mut self, v: Value) -> Value {
        let id = v["id"].clone();
        let mut r = self.ask_raw(v);
        while r["id"] != id && !r.is_null() {
            r = self.recv();
        }
        r
    }
}

/// `auth.pair2` by hand over `pipe`: both steps, with `pin`. The hub's last answer.
fn raw_pair2(pipe: TlsPipe, port: u16, pin: &str, hid: &str) -> Value {
    let binding = Binding::of(&pipe).unwrap();
    let mut ws = Ws::open(pipe, port);
    let (ours, msg_a) = ClientPake::start(pin, hid, "raw", "control", binding).unwrap();
    let r = ws.ask(
        json!({"t": "auth.pair2", "id": 1, "step": 1, "name": "raw", "role": "control",
        "pake": pake::encode(&msg_a)}),
    );
    assert_eq!(r["t"], "ok", "{r}");
    let msg_b = pake::decode(r["pake"].as_str().unwrap(), pake::MESSAGE_LEN).unwrap();
    let confirm = ours.finish(&msg_b).unwrap();
    let r = ws.ask(
        json!({"t": "auth.pair2", "id": 2, "step": 2, "confirm": pake::encode(confirm.confirm())}),
    );
    if r["t"] == "ok" {
        let mac = pake::decode(r["confirm"].as_str().unwrap(), pake::CONFIRM_LEN).unwrap();
        confirm.verify_hub(&mac).unwrap();
        secret(r["token"].as_str().unwrap());
    }
    r
}

// ───────────────────────────── the core's client ─────────────────────────────

fn opts(token: Option<&str>) -> ConnectOptions {
    ConnectOptions {
        client_name: "SECPAKE core".into(),
        app: "test".into(),
        app_version: "0".into(),
        token: token.map(str::to_owned),
    }
}

#[derive(Default)]
struct Tokens(Mutex<Vec<String>>);

impl LinkEvents for Tokens {
    fn token_issued(&self, _hub_id: &str, token: &str) {
        self.0.lock().unwrap().push(token.to_owned());
    }
}

fn connect_url(
    url: &str,
    token: Option<&str>,
    pin: Option<SpkiHash>,
    events: Option<Arc<dyn LinkEvents>>,
) -> Result<(LinkClient, AuthState), LinkError> {
    LinkClient::connect_tls(
        &TcpConnector::default(),
        url,
        &opts(token),
        events,
        &SessionConfig::default(),
        &TlsOptions {
            pin: pin.map(|p| p.0),
            ..TlsOptions::default()
        },
    )
}

// ───────────────────────────── the machine in the middle ─────────────────────────────

/// A TLS-terminating proxy to `hub_port`, with its own key: every WebSocket message
/// relayed both ways, the client's texts kept. One connection at a time, until dropped.
struct Mitm {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Mitm {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect((LOOPBACK, self.port));
    }
}

fn mitm(hub_port: u16) -> Mitm {
    let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let key = FakeTlsKey::generate();
    let (seen2, stop2) = (Arc::clone(&seen), Arc::clone(&stop));
    thread::spawn(move || {
        for conn in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                return;
            }
            let Ok(conn) = conn else { continue };
            let _ = relay(conn, &key, hub_port, &seen2);
        }
    });
    Mitm { port, seen, stop }
}

#[allow(clippy::result_large_err)] // tungstenite's callback signature
fn relay(
    conn: TcpStream,
    key: &FakeTlsKey,
    hub_port: u16,
    seen: &Mutex<Vec<String>>,
) -> io::Result<()> {
    conn.set_write_timeout(Some(Duration::from_secs(3)))?;
    let by = Instant::now() + Duration::from_secs(3);
    let mut client_leg = link_tls::accept(conn, key.server_config(), by)
        .map_err(|e| io::Error::other(e.to_string()))?;
    client_leg.set_deadline(None);
    let hub_leg = tls_to(hub_port, PinMode::FirstUse);
    let echo = |_: &tungstenite::handshake::server::Request,
                mut resp: tungstenite::handshake::server::Response| {
        resp.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        Ok(resp)
    };
    let mut client_ws =
        tungstenite::accept_hdr(client_leg, echo).map_err(|e| io::Error::other(e.to_string()))?;
    let request = format!("ws://127.0.0.1:{hub_port}/dspi/v1");
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(request).unwrap();
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
    let (mut hub_ws, _) =
        tungstenite::client(request, hub_leg).map_err(|e| io::Error::other(e.to_string()))?;
    client_ws
        .get_ref()
        .tcp()
        .set_read_timeout(Some(Duration::from_millis(10)))?;
    hub_ws
        .get_ref()
        .tcp()
        .set_read_timeout(Some(Duration::from_millis(10)))?;
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        match client_ws.read() {
            Ok(Message::Text(t)) => {
                seen.lock().unwrap().push(t.to_string());
                hub_ws
                    .send(Message::text(t.to_string()))
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Ok(()),
        }
        match hub_ws.read() {
            Ok(Message::Text(t)) => {
                client_ws
                    .send(Message::text(t.to_string()))
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
            Ok(Message::Binary(b)) => {
                client_ws
                    .send(Message::Binary(b))
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

// ───────────────────────────── done criteria ─────────────────────────────

#[test]
fn wp_secpake_the_mitm_reads_and_relays_a_legacy_pin_control() {
    // The control: the same proxy against `auth.pair` over its TLS reads the PIN and
    // pairs the attacker. This is the attack `auth.pair2` is for.
    let t = start(TlsMode::Require);
    let m = mitm(t.port);
    let pin = t.open_pin();
    let mut ws = Ws::open(tls_to(m.port, PinMode::FirstUse), m.port);
    let r = ws.ask(json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "browser"}));
    assert_eq!(r["t"], "ok", "{r}");
    secret(r["token"].as_str().unwrap());
    assert!(
        m.seen
            .lock()
            .unwrap()
            .iter()
            .any(|text| text.contains(&pin)),
        "the proxy read the PIN"
    );
}

#[test]
fn wp_secpake_a_mitm_fails_pairing_at_the_first_guess_and_never_sees_the_pin() {
    let t = start(TlsMode::Require);
    let m = mitm(t.port);
    let pin = t.open_pin();
    let url = format!("ws://127.0.0.1:{}/dspi/v1", m.port);
    let (client, state) = connect_url(&url, None, None, None).unwrap();
    assert!(matches!(state, AuthState::NeedsPairing { .. }), "{state:?}");
    assert!(
        client.tls().unwrap().first_use,
        "the client trusted the proxy's key"
    );
    // The right PIN, through the proxy: refused at the first guess.
    let e = client.pair(&pin, "control").unwrap_err();
    assert!(matches!(e, LinkError::WrongPin { .. }), "{e:?}");
    client.close();
    assert!(t.net.auth().clients().is_empty(), "no token was issued");
    assert!(
        matches!(t.state(), PairingState::Open { .. }),
        "one failure: the window stays"
    );
    let seen = m.seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|s| s.contains("\"auth.pair2\"")),
        "{seen:?}"
    );
    assert!(
        !seen.iter().any(|s| s.contains("\"auth.pair\"")),
        "never the legacy pairing over TLS"
    );
    assert!(
        !seen.iter().any(|s| has_word(s, &pin)),
        "the PIN never crossed: {seen:?}"
    );
    // The hub's own ledger counted that one guess, as a wrong PIN.
    drop(m);
    let direct = connect_url(&t.url(), None, None, None).unwrap().0;
    let token = direct.pair(&pin, "control").unwrap();
    secret(&token);
    assert_eq!(
        t.net.auth().clients().len(),
        1,
        "the real client pairs directly"
    );
}

#[test]
fn wp_secpake_five_mitm_guesses_kill_the_window_as_before() {
    let t = start(TlsMode::Require);
    let m = mitm(t.port);
    let pin = t.open_pin();
    let url = format!("ws://127.0.0.1:{}/dspi/v1", m.port);
    for n in 0..5 {
        let (client, _) = connect_url(&url, None, None, None).unwrap();
        let e = client.pair(&pin, "control").unwrap_err();
        assert!(matches!(e, LinkError::WrongPin { .. }), "try {n}: {e:?}");
        client.close();
    }
    assert_eq!(t.state(), PairingState::ClosedAfterWrongPins);
    assert!(t.net.auth().clients().is_empty());
}

#[test]
fn wp_secpake_a_correct_pin_pairs_through_the_core_and_the_token_works() {
    let t = start(TlsMode::Require);
    let (client, _) = connect_url(&t.url(), None, None, None).unwrap();
    let token = client.pair(&t.open_pin(), "control").unwrap();
    secret(&token);
    let key = client.tls().unwrap().key;
    assert_eq!(key, t.pin().0);
    client.close();
    let cid = t.net.auth().clients()[0].cid;
    assert!(t.net.auth().is_tls_only(cid), "issued over TLS");
    let (client, state) = connect_url(&t.url(), Some(&token), Some(SpkiHash(key)), None).unwrap();
    assert_eq!(state, AuthState::Ready);
    client.close();
}

#[test]
fn wp_secpake_a_wrong_pin_is_wrong_pin_and_counted() {
    let t = start(TlsMode::On);
    let pin = t.open_pin();
    let wrong = if pin == "000000" { "000001" } else { "000000" };
    for _ in 0..5 {
        let (client, _) = connect_url(&t.url(), None, None, None).unwrap();
        assert!(matches!(
            client.pair(wrong, "control"),
            Err(LinkError::WrongPin { .. })
        ));
        client.close();
    }
    assert_eq!(t.state(), PairingState::ClosedAfterWrongPins);
}

#[test]
fn wp_secpake_the_hub_side_by_hand_and_its_refusals() {
    let t = start(TlsMode::On);
    let hid = t.net.auth().hub_id();
    let hello = Ws::open(tls_to(t.port, PinMode::Pinned(t.pin())), t.port).hello();
    assert!(
        hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == pake::CAP),
        "{hello}"
    );
    // Over plaintext: refused, nothing counted.
    let pin = t.open_pin();
    let mut plain = Ws::open(tcp(t.port), t.port);
    let r = plain.ask(json!({"t": "auth.pair2", "id": 1, "step": 1, "name": "x", "pake": pake::encode(&[0x41; 33])}));
    assert_eq!(r["code"], "tls_required", "{r}");
    // Step 2 with no step 1, a bad step, a malformed message: bad requests, not guesses.
    let mut ws = Ws::open(tls_to(t.port, PinMode::Pinned(t.pin())), t.port);
    let r =
        ws.ask(json!({"t": "auth.pair2", "id": 1, "step": 2, "confirm": pake::encode(&[0; 32])}));
    assert_eq!(r["code"], "bad_request", "{r}");
    let r = ws.ask(json!({"t": "auth.pair2", "id": 2, "step": 3}));
    assert_eq!(r["code"], "bad_request", "{r}");
    let r =
        ws.ask(json!({"t": "auth.pair2", "id": 3, "step": 1, "name": "x", "pake": "not base64!"}));
    assert_eq!(r["code"], "bad_request", "{r}");
    let r = ws
        .ask(json!({"t": "auth.pair2", "id": 4, "step": 1, "name": "x", "pake": "A".repeat(5000)}));
    assert_eq!(r["code"], "bad_request", "{r}");
    // A missing confirmation after a step 1 is a failed attempt.
    let r = ws.ask(json!({"t": "auth.pair2", "id": 5, "step": 1, "name": "x", "pake": pake::encode(&ClientPake::start("1", &hid, "x", "", Binding::from_bytes([0; 32])).unwrap().1)}));
    assert_eq!(r["t"], "ok", "{r}");
    let r = ws.ask(json!({"t": "auth.pair2", "id": 6, "step": 2}));
    assert_eq!(r["code"], "unauthenticated", "{r}");
    assert!(
        r.get("msg").is_none(),
        "a wrong PIN has no msg, as auth.pair's"
    );
    // The first steps one connection may make are capped.
    for id in 7..20 {
        let r = ws.ask(json!({"t": "auth.pair2", "id": id, "step": 1, "name": "x", "pake": pake::encode(&ClientPake::start("1", &hid, "x", "", Binding::from_bytes([0; 32])).unwrap().1)}));
        if r["code"] == "rate_limited" {
            assert!(id > 7);
            break;
        }
        assert_eq!(r["t"], "ok", "{r}");
        assert!(id < 19, "never capped");
    }
    // And by hand, the right PIN pairs and the hub's MAC verifies.
    let r = raw_pair2(tls_to(t.port, PinMode::Pinned(t.pin())), t.port, &pin, &hid);
    assert_eq!(r["t"], "ok", "{r}");
    assert_eq!(r["hub_key"], t.pin().to_b64url());
    assert_eq!(r["role"], "control");
}

#[test]
fn wp_secpake_legacy_auth_pair_still_works_for_browsers_and_upstream() {
    // A browser: `auth.pair` over TLS (it cannot reach the exporter).
    let t = start(TlsMode::Require);
    let pin = t.open_pin();
    let mut ws = Ws::open(tls_to(t.port, PinMode::Pinned(t.pin())), t.port);
    let r = ws.ask(json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "browser"}));
    assert_eq!(r["t"], "ok", "{r}");
    secret(r["token"].as_str().unwrap());
    // An upstream console: plaintext `auth.pair` on a compatibility-mode hub.
    let t = start_with(TlsMode::On, true, None);
    let pin = t.open_pin();
    let mut ws = Ws::open(tcp(t.port), t.port);
    let r = ws.ask(json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "upstream"}));
    assert_eq!(r["t"], "ok", "{r}");
    secret(r["token"].as_str().unwrap());
    // Our client against a hub without TLS (an upstream WeebLabs hub): `auth.pair`.
    let t = start(TlsMode::Off);
    let (client, _) = connect_url(&t.url(), None, None, None).unwrap();
    assert!(client.tls().is_none());
    let token = client.pair(&t.open_pin(), "control").unwrap();
    secret(&token);
    client.close();
}

// ───────────────────────────── the SEC-C review's nits ─────────────────────────────

#[test]
fn wp_secpake_key_next_follows_the_services_clock() {
    let clock = Arc::new(TestClock::new());
    let t = start_with(TlsMode::Require, false, Some(Arc::clone(&clock)));
    t.net.rotate_tls_key(false).unwrap();
    let hello = Ws::open(tls_to(t.port, PinMode::Pinned(t.pin())), t.port).hello();
    assert!(hello["hub"].get("key_next").is_some(), "{hello}");
    // 31 days on the hub's clock (not the system's): the announcement has ended.
    clock.advance(Duration::from_secs(31 * DAY));
    let hello = Ws::open(tls_to(t.port, PinMode::Pinned(t.pin())), t.port).hello();
    assert!(hello["hub"].get("key_next").is_none(), "{hello}");
}

#[test]
fn wp_secpake_the_core_rotates_its_token_after_accepting_a_key_rotation() {
    let t = start(TlsMode::Require);
    let (client, _) = connect_url(&t.url(), None, None, None).unwrap();
    let old = client.pair(&t.open_pin(), "control").unwrap();
    secret(&old);
    let first = SpkiHash(client.tls().unwrap().key);
    client.close();
    t.net.rotate_tls_key(false).unwrap();
    let tokens = Arc::new(Tokens::default());
    let (client, state) = connect_url(
        &t.url(),
        Some(&old),
        Some(first),
        Some(tokens.clone() as Arc<dyn LinkEvents>),
    )
    .unwrap();
    assert_eq!(state, AuthState::Ready);
    assert!(client.tls().unwrap().rotated);
    let issued = tokens.0.lock().unwrap().clone();
    assert_eq!(
        issued.len(),
        1,
        "auth.rotate after the key rotation: {}",
        issued.len()
    );
    let new = issued[0].clone();
    secret(&new);
    assert_ne!(new, old);
    client.close();
    // The old token is dead, the new one gets in on the new key.
    let (c, state) = connect_url(&t.url(), Some(&old), Some(t.pin()), None).unwrap();
    assert!(matches!(state, AuthState::TokenRefused { .. }), "{state:?}");
    c.close();
    let (c, state) = connect_url(&t.url(), Some(&new), Some(t.pin()), None).unwrap();
    assert_eq!(state, AuthState::Ready);
    c.close();
}

#[test]
fn wp_secpake_a_session_close_refreshes_last_seen() {
    let clock = Arc::new(TestClock::new());
    let t = start_with(TlsMode::Require, false, Some(Arc::clone(&clock)));
    let (client, _) = connect_url(&t.url(), None, None, None).unwrap();
    let token = client.pair(&t.open_pin(), "control").unwrap();
    secret(&token);
    let key = SpkiHash(client.tls().unwrap().key);
    client.close();
    let (client, _) = connect_url(&t.url(), Some(&token), Some(key), None).unwrap();
    // The session lasts 200 days on the hub's clock; meanwhile it is not forgotten.
    clock.advance(Duration::from_secs(200 * DAY));
    assert_eq!(t.net.auth().prune_idle(), 0);
    client.close();
    let deadline = Instant::now() + Duration::from_secs(5);
    let refreshed = loop {
        let seen = t.net.auth().clients()[0].last_seen.unwrap();
        if seen >= clock.wall() - Duration::from_secs(60) || Instant::now() > deadline {
            break seen;
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(
        refreshed >= clock.wall() - Duration::from_secs(60),
        "last_seen is the close"
    );
    let (c, state) = connect_url(&t.url(), Some(&token), Some(key), None).unwrap();
    assert_eq!(
        state,
        AuthState::Ready,
        "still paired after the long session"
    );
    c.close();
}

// ───────────────────────────── nothing secret in the logs ─────────────────────────────

#[test]
fn wp_secpake_zz_no_pin_or_token_in_the_logs() {
    // Runs last by name; the other tests may still be running, so run one more here.
    let t = start(TlsMode::Require);
    let (client, _) = connect_url(&t.url(), None, None, None).unwrap();
    let token = client.pair(&t.open_pin(), "control").unwrap();
    secret(&token);
    client.close();
    drop(t);
    let lines = kept().lock().unwrap().join("\n");
    for s in secrets().lock().unwrap().iter() {
        assert!(!has_word(&lines, s), "a secret is in the logs");
    }
}
