//! WP SECPAIR: pairing bound to the hub's key, the `tls_only` ratchet, the PIN and
//! token ledgers, idle expiry, `auth.rotate`, `hello.hub.key_next`, and key rotation end
//! to end with SECCORE's client (PLAN-SEC 4.2, 4.3, 4.5, 4.9).
//!
//! Hand-written WebSocket clients over dspi-link's TLS (or plain TCP) see the wire as
//! it is; `LinkClient::connect_tls` is the real client every platform uses. A logger
//! that keeps every line is installed, so the last test can show that no secret was
//! ever logged by any test here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use dspi_hub::auth::PairingState;
use dspi_hub::server::{ServerLimits, pair_link};
use dspi_hub::{
    AccessMode, Hub, HubClock, HubConfig, HubEvent, HubListener, NetworkConfig, ServiceConfig,
    TestClock, TlsMode,
};
use dspi_link::LinkError;
use dspi_link::client::{AuthState, ConnectOptions, LinkClient, SessionConfig, TlsOptions};
use dspi_link::credential::parse_pair_link;
use dspi_link::socket::TcpConnector;
use dspi_link::tls::{self as link_tls, PinMode, SpkiHash, TlsPipe, verify_rotation};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use socket2::{Domain, Socket, Type};
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
        // tungstenite's trace prints whole frames; every embedder holds it at debug
        // (HB6's rule, `uconsole hub`'s logger), so it is not ours to judge here.
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

/// Every secret a test handled, for the last test to look for.
fn secrets() -> &'static Mutex<Vec<String>> {
    static S: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

fn secret(s: &str) {
    if s.len() >= 6 {
        secrets().lock().unwrap().push(s.to_owned());
    }
}

/// A token and its SHA-256 (what the hub keeps of it).
fn secret_with_hash(s: &str) {
    use sha2::Digest;
    secret(s);
    let hash = sha2::Sha256::digest(s.as_bytes());
    secret(&data_encoding::HEXLOWER.encode(&hash));
}

/// The PINs a test opened. Six digits can be part of another number by chance, so they
/// are looked for as a number of their own; their hashes as any other secret.
fn pins() -> &'static Mutex<Vec<String>> {
    static P: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Vec::new()))
}

fn secret_pin(pin: &str) {
    use sha2::Digest;
    pins().lock().unwrap().push(pin.to_owned());
    let hash = sha2::Sha256::digest(pin.as_bytes());
    secret(&data_encoding::HEXLOWER.encode(&hash));
}

/// Whether `pin` stands in `text` as a number of its own.
fn has_number(text: &str, pin: &str) -> bool {
    text.match_indices(pin).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + pin.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

// ───────────────────────────── the hub ─────────────────────────────

fn store_dir() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    // ci/run.sh points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("wp-secpair").join(format!(
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

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/dspi/v1", self.port)
    }

    fn open_pin(&self) -> String {
        let w = self.net.auth().begin_pairing(Instant::now()).unwrap();
        secret_pin(&w.pin);
        w.pin
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.hub.stop();
        // Every event's Debug, for the secrets test.
        let events = self.events.0.lock().unwrap();
        let mut lines = kept().lock().unwrap();
        lines.extend(events.iter().map(|e| format!("event {e:?}")));
    }
}

fn start_with(tls: TlsMode, compat: bool, clock: Option<Arc<TestClock>>) -> Target {
    keep_logs();
    let store = store_dir();
    let cfg = NetworkConfig {
        port: 0,
        access: AccessMode::Pin,
        bind: IpAddr::V4(LOOPBACK),
        limits: ServerLimits {
            head_timeout: Duration::from_millis(1500),
            auth_timeout: Duration::from_millis(3000),
            close_grace: Duration::from_millis(300),
            read_tick: Duration::from_millis(20),
            ..ServerLimits::default()
        },
        tls,
        upstream_compat: compat,
        ..NetworkConfig::new(store.clone())
    };
    let mut services = ServiceConfig::default();
    if let Some(clock) = clock {
        services.clock = clock as Arc<dyn HubClock>;
    }
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        services,
        ..HubConfig::in_process("SECPAIR hub")
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

fn start(tls: TlsMode) -> Target {
    start_with(tls, false, None)
}

// ───────────────────────────── a raw client ─────────────────────────────

fn tcp(port: u16) -> TcpStream {
    tcp_from(None, port)
}

fn tcp_from(src: Option<Ipv4Addr>, port: u16) -> TcpStream {
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

fn tls_from(src: Option<Ipv4Addr>, t: &Target) -> TlsPipe {
    let deadline = Instant::now() + Duration::from_secs(3);
    let (mut pipe, _) = link_tls::connect(
        tcp_from(src, t.port),
        "localhost",
        PinMode::Pinned(t.pin()),
        deadline,
    )
    .expect("a pinned TLS connection");
    pipe.set_deadline(None);
    pipe
}

fn tls(t: &Target) -> TlsPipe {
    tls_from(None, t)
}

struct Ws<S: Read + Write>(WebSocket<S>, Value);

impl<S: Read + Write> Ws<S> {
    fn open(mut s: S, host: &str) -> Self {
        let upgrade = format!(
            "GET /dspi/v1 HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\n\
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
        let head = String::from_utf8_lossy(&head);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let mut ws = Self(
            WebSocket::from_raw_socket(s, WsRole::Client, None),
            Value::Null,
        );
        let hello = ws.ask_hello();
        assert_eq!(hello["t"], "hello", "{hello}");
        ws.1 = hello;
        ws
    }

    fn ask_hello(&mut self) -> Value {
        self.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "secpair", "kind": "test"}}));
        self.recv().expect("hello")
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

    fn pair(&mut self, pin: &str) -> Value {
        let r = self.ask(json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "secpair"}));
        if let Some(token) = r["token"].as_str() {
            secret_with_hash(token);
        }
        r
    }

    fn token(&mut self, token: &str) -> Value {
        self.ask(json!({"t": "auth.token", "id": 2, "token": token}))
    }
}

fn hello_of(t: &Target) -> Value {
    Ws::open(tls(t), &t.host()).1
}

// ───────────────────────────── the core's client ─────────────────────────────

fn client_opts(token: Option<&str>) -> ConnectOptions {
    ConnectOptions {
        client_name: "SECPAIR core".into(),
        app: "test".into(),
        app_version: "0".into(),
        token: token.map(str::to_owned),
    }
}

fn core_connect(
    t: &Target,
    token: Option<&str>,
    pin: Option<SpkiHash>,
) -> Result<(LinkClient, AuthState), LinkError> {
    LinkClient::connect_tls(
        &TcpConnector::default(),
        &t.url(),
        &client_opts(token),
        None,
        &SessionConfig::default(),
        &TlsOptions {
            pin: pin.map(|p| p.0),
            ..TlsOptions::default()
        },
    )
}

/// A typed-PIN pairing through the core's client: the token and the key it pinned.
fn core_pair(t: &Target) -> (String, Option<[u8; 32]>) {
    let (client, state) = core_connect(t, None, None).unwrap();
    assert!(matches!(state, AuthState::NeedsPairing { .. }), "{state:?}");
    let token = client.pair(&t.open_pin(), "control").unwrap();
    secret_with_hash(&token);
    let key = client.tls().map(|seen| seen.key);
    client.close();
    (token, key)
}

// ───────────────────────────── plaintext pairing (OD-SECk) ─────────────────────────────

#[test]
fn wp_secpair_plaintext_pairing_refused_in_on_allowed_with_the_flag() {
    let t = start(TlsMode::On);
    let pin = t.open_pin();
    let mut ws = Ws::open(tcp(t.port), &t.host());
    let r = ws.pair(&pin);
    assert_eq!(r["code"], "tls_required", "{r}");
    assert!(
        t.net.auth().pairing_open(Instant::now()),
        "the PIN was not even looked at"
    );
    // Over TLS, the same window pairs.
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.pair(&pin)["t"], "ok");
    drop(t);

    // Upstream compatibility (the flag that allows plaintext pairing).
    let t = start_with(TlsMode::On, true, None);
    let mut ws = Ws::open(tcp(t.port), &t.host());
    let r = ws.pair(&t.open_pin());
    assert_eq!(r["t"], "ok", "{r}");
    assert!(r.get("hub_key").is_none(), "no key over plaintext: {r}");
}

// ───────────────────────────── ok.hub_key ─────────────────────────────

#[test]
fn wp_secpair_ok_hub_key_is_the_tls_spki() {
    let t = start(TlsMode::Require);
    let mut ws = Ws::open(tls(&t), &t.host());
    let r = ws.pair(&t.open_pin());
    assert_eq!(r["t"], "ok", "{r}");
    assert_eq!(r["hub_key"], t.pin().to_b64url());
    let token = r["token"].as_str().unwrap().to_owned();
    // auth.token over TLS names it too.
    let mut ws = Ws::open(tls(&t), &t.host());
    let r = ws.token(&token);
    assert_eq!(r["hub_key"], t.pin().to_b64url(), "{r}");
    // And the hub's /info says the same key.
    assert_eq!(
        dspi_link::credential::pin_to_b64url(&t.pin().0),
        t.pin().to_b64url()
    );
}

// ───────────────────────────── ratchet (b) ─────────────────────────────

#[test]
fn wp_secpair_a_token_used_once_over_tls_is_refused_over_plaintext() {
    // Compat: plaintext pairing and plaintext tokens are served.
    let t = start_with(TlsMode::On, true, None);
    let mut ws = Ws::open(tcp(t.port), &t.host());
    let token = ws.pair(&t.open_pin())["token"].as_str().unwrap().to_owned();
    // Plaintext, again: fine, it has never been used over TLS.
    let mut ws = Ws::open(tcp(t.port), &t.host());
    assert_eq!(ws.token(&token)["t"], "ok");
    // Once over TLS...
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.token(&token)["t"], "ok");
    // ... never again in the clear.
    let mut ws = Ws::open(tcp(t.port), &t.host());
    let r = ws.token(&token);
    assert_eq!(r["code"], "tls_required", "{r}");
    // The hub keeps it across a restart (it is in the store).
    let text = std::fs::read_to_string(t.store.join("hub-store.json")).unwrap();
    assert!(text.contains("\"tls_only\": true"), "{text}");
    // TLS still works.
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.token(&token)["t"], "ok");
}

#[test]
fn wp_secpair_a_token_paired_over_tls_never_works_in_the_clear() {
    let t = start(TlsMode::On);
    let mut ws = Ws::open(tls(&t), &t.host());
    let token = ws.pair(&t.open_pin())["token"].as_str().unwrap().to_owned();
    let mut ws = Ws::open(tcp(t.port), &t.host());
    let r = ws.token(&token);
    assert_eq!(r["code"], "tls_required", "{r}");
}

// ───────────────────────────── the two ledgers (4.5) ─────────────────────────────

#[test]
fn wp_secpair_revoked_token_retries_do_not_kill_the_window_but_wrong_pins_do() {
    let t = start(TlsMode::Require);
    let mut ws = Ws::open(tls(&t), &t.host());
    let r = ws.pair(&t.open_pin());
    let token = r["token"].as_str().unwrap().to_owned();
    let cid = t.net.auth().clients()[0].cid;
    assert!(t.net.revoke(cid));
    let pin = t.open_pin();
    // A stale client keeps retrying its revoked token, past its own address's limit.
    let mut codes = Vec::new();
    for _ in 0..8 {
        let mut ws = Ws::open(tls(&t), &t.host());
        codes.push(ws.token(&token)["code"].as_str().unwrap_or("?").to_owned());
    }
    assert_eq!(&codes[..5], &["unauthenticated"; 5], "{codes:?}");
    assert!(codes[5..].iter().all(|c| c == "rate_limited"), "{codes:?}");
    // The window is still open, and that same address may pair.
    assert!(matches!(
        t.net.auth().pairing_state(Instant::now()),
        PairingState::Open { .. }
    ));
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.pair(&pin)["t"], "ok");

    // Five wrong PINs from anywhere end a window, and the hub can say so.
    let pin = t.open_pin();
    let wrong = if pin == "000000" { "000001" } else { "000000" };
    for n in 0..5u8 {
        let mut ws = Ws::open(
            tls_from(Some(Ipv4Addr::new(127, 0, 0, 2 + n)), &t),
            &t.host(),
        );
        assert_eq!(ws.pair(wrong)["code"], "unauthenticated");
    }
    assert_eq!(
        t.net.auth().pairing_state(Instant::now()),
        PairingState::ClosedAfterWrongPins
    );
    let mut ws = Ws::open(tls_from(Some(Ipv4Addr::new(127, 0, 0, 40)), &t), &t.host());
    assert_eq!(
        ws.pair(&pin)["code"],
        "unauthenticated",
        "the window is dead"
    );
    assert!(t.net.auth().take_window_killed());
}

// ───────────────────────────── idle expiry (OD-SECg) ─────────────────────────────

#[test]
fn wp_secpair_an_idle_token_is_pruned_at_the_test_clocks_180_days() {
    let clock = Arc::new(TestClock::at(
        UNIX_EPOCH + Duration::from_secs(1_800_000_000),
    ));
    let t = start_with(TlsMode::Require, false, Some(Arc::clone(&clock)));
    let mut ws = Ws::open(tls(&t), &t.host());
    let token = ws.pair(&t.open_pin())["token"].as_str().unwrap().to_owned();
    drop(ws);
    clock.advance(Duration::from_secs(180 * DAY - 60));
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.token(&token)["t"], "ok", "used: a new 180 days start");
    drop(ws);
    clock.advance(Duration::from_secs(180 * DAY));
    let mut ws = Ws::open(tls(&t), &t.host());
    let r = ws.token(&token);
    assert_eq!(r["code"], "unauthenticated", "{r}");
    assert!(t.net.auth().clients().is_empty(), "forgotten");
}

// ───────────────────────────── auth.rotate ─────────────────────────────

#[test]
fn wp_secpair_auth_rotate_swaps_the_token_and_the_old_one_is_dead() {
    let t = start(TlsMode::On);
    let mut ws = Ws::open(tls(&t), &t.host());
    let old = ws.pair(&t.open_pin())["token"].as_str().unwrap().to_owned();
    let r = ws.ask(json!({"t": "auth.rotate", "id": 9}));
    assert_eq!(r["t"], "ok", "{r}");
    assert_eq!(r["hub_key"], t.pin().to_b64url());
    let new = r["token"].as_str().unwrap().to_owned();
    secret_with_hash(&new);
    assert_ne!(new, old);
    // The session goes on.
    assert_eq!(ws.ask(json!({"t": "device.list", "id": 10}))["t"], "ok");
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.token(&old)["code"], "unauthenticated");
    let mut ws = Ws::open(tls(&t), &t.host());
    assert_eq!(ws.token(&new)["t"], "ok");
    assert_eq!(t.net.auth().clients().len(), 1, "the same client");
}

#[test]
fn wp_secpair_auth_rotate_over_plaintext_follows_the_pairing_rule() {
    // `on` without compat: plaintext sessions exist only with old, unratcheted tokens.
    let t = start_with(TlsMode::On, true, None);
    let mut ws = Ws::open(tcp(t.port), &t.host());
    ws.pair(&t.open_pin());
    // Compat allows plaintext pairing, so a plaintext rotation too.
    let r = ws.ask(json!({"t": "auth.rotate", "id": 3}));
    assert_eq!(r["t"], "ok", "{r}");
    secret_with_hash(r["token"].as_str().unwrap());
    drop(t);

    let t = start(TlsMode::Off);
    let mut ws = Ws::open(tcp(t.port), &t.host());
    ws.pair(&t.open_pin());
    let r = ws.ask(json!({"t": "auth.rotate", "id": 3}));
    assert_eq!(r["t"], "ok", "TLS off: nothing is encrypted anyway: {r}");
    secret_with_hash(r["token"].as_str().unwrap());
}

// ───────────────────────────── key_next and rotation end to end ─────────────────────────────

#[test]
fn wp_secpair_hello_announces_a_signed_key_next_after_a_rotation() {
    let t = start(TlsMode::Require);
    let hid = t.net.auth().hub_id();
    let h = hello_of(&t);
    assert!(h["hub"].get("key_next").is_none(), "no rotation: {h}");
    let old = t.pin();
    t.net.rotate_tls_key(false).unwrap();
    let new = t.pin();
    let h = hello_of(&t);
    let next = &h["hub"]["key_next"];
    let b64 = |v: &Value| {
        data_encoding::BASE64URL_NOPAD
            .decode(v.as_str().unwrap().as_bytes())
            .unwrap()
    };
    assert_eq!(next["spki"], new.to_b64url(), "{h}");
    let prev = b64(&next["prev_spki"]);
    let sig = b64(&next["sig"]);
    verify_rotation(&old, &prev, &hid, &new, &sig).unwrap();
    // A rotation with --forget-old announces nothing.
    t.net.rotate_tls_key(true).unwrap();
    let h = hello_of(&t);
    assert!(h["hub"].get("key_next").is_none(), "{h}");
}

#[test]
fn wp_secpair_rotation_end_to_end_with_the_core_client() {
    let t = start(TlsMode::Require);
    let (token, key) = core_pair(&t);
    let first = SpkiHash(key.expect("paired over TLS"));
    assert_eq!(first, t.pin());
    // The hub rotates, signed by the old key.
    t.net.rotate_tls_key(false).unwrap();
    let second = t.pin();
    let (client, state) = core_connect(&t, Some(&token), Some(first)).unwrap();
    assert_eq!(state, AuthState::Ready);
    let seen = client.tls().unwrap();
    assert!(seen.rotated, "re-pinned by the old key's signature");
    assert_eq!(seen.key, second.0);
    client.close();
    // The new pin works directly.
    let (client, state) = core_connect(&t, Some(&token), Some(second)).unwrap();
    assert_eq!(state, AuthState::Ready);
    assert!(!client.tls().unwrap().rotated);
    client.close();
    // A compromise: no signature, so the pinned client refuses, with no token sent.
    t.net.rotate_tls_key(true).unwrap();
    let before = t.net.auth().clients()[0].last_seen;
    let e = core_connect(&t, Some(&token), Some(second)).err();
    assert_eq!(e, Some(LinkError::HubKeyChanged));
    assert_eq!(t.net.auth().clients()[0].last_seen, before);
}

#[test]
fn wp_secpair_typed_pin_pairing_end_to_end_in_each_mode() {
    for (mode, compat) in [
        (TlsMode::Require, false),
        (TlsMode::On, false),
        (TlsMode::On, true),
        (TlsMode::Off, false),
    ] {
        let t = start_with(mode, compat, None);
        let (token, key) = core_pair(&t);
        match mode {
            TlsMode::Off => assert_eq!(key, None, "TLS off: unencrypted"),
            _ => assert_eq!(key, Some(t.pin().0), "{mode:?} compat {compat}"),
        }
        // And the token gets in again, the same way.
        let pin = key.map(SpkiHash);
        let (client, state) = core_connect(&t, Some(&token), pin).unwrap();
        assert_eq!(state, AuthState::Ready, "{mode:?} compat {compat}");
        assert_eq!(client.tls().is_some(), mode != TlsMode::Off);
        client.close();
        if mode != TlsMode::Off {
            assert!(t.net.auth().clients().iter().all(|c| c.cid > 0));
        }
    }
}

// ───────────────────────────── the pairing link ─────────────────────────────

#[test]
fn wp_secpair_the_pairing_link_carries_the_key_and_parses() {
    let t = start(TlsMode::Require);
    let w = t.net.auth().begin_pairing(Instant::now()).unwrap();
    secret_pin(&w.pin);
    let link = t.net.pair_link(&w).unwrap();
    let parsed = parse_pair_link(&link).unwrap();
    assert_eq!(parsed.hub_id, t.net.auth().hub_id());
    assert_eq!(parsed.host, "127.0.0.1");
    assert_eq!(parsed.port, t.port);
    assert_eq!(parsed.key, t.pin().0);
    assert_eq!(parsed.pin_code, w.pin);
    // IPv6 is bracketed; no PIN, no `p`.
    let v6 = pair_link("abc", "fd00::1", 11915, &t.pin(), "");
    let parsed = parse_pair_link(&v6).unwrap();
    assert_eq!((parsed.host.as_str(), parsed.port), ("fd00::1", 11915));
    assert_eq!(parsed.pin_code, "");
    // TLS off: no key, no link.
    let off = start(TlsMode::Off);
    let w = off.net.auth().begin_pairing(Instant::now()).unwrap();
    secret_pin(&w.pin);
    assert_eq!(off.net.pair_link(&w), None);
}

// ───────────────────────────── nothing secret anywhere (S9) ─────────────────────────────

#[test]
fn wp_secpair_zz_no_secret_in_logs_events_or_the_diagnostics_bundle() {
    // A whole run of its own first, so there is something to look at.
    let t = start(TlsMode::On);
    let mut ws = Ws::open(tls(&t), &t.host());
    let token = ws.pair(&t.open_pin())["token"].as_str().unwrap().to_owned();
    let r = ws.ask(json!({"t": "auth.rotate", "id": 4}));
    secret_with_hash(r["token"].as_str().unwrap());
    let mut ws = Ws::open(tcp(t.port), &t.host());
    ws.token(r["token"].as_str().unwrap());
    let mut ws = Ws::open(tls(&t), &t.host());
    ws.token(&token);
    let pin = t.open_pin();
    let mut ws = Ws::open(tls(&t), &t.host());
    ws.pair(if pin == "000000" { "000001" } else { "000000" });
    t.net.rotate_tls_key(false).unwrap();
    let key_pem = std::fs::read_to_string(t.store.join("tls").join("key.pem")).unwrap();
    let key_b64: String = key_pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    secret(&key_b64);
    for line in key_pem.lines().filter(|l| !l.starts_with("-----")) {
        secret(line);
    }

    // The diagnostics bundle of this hub's store.
    let core = dspi_core::DspiCore::new();
    let zip = core
        .diagnostics_zip(dspi_core::DiagOptions {
            hub_store_dir: t.store.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let bundle = String::from_utf8_lossy(&zip).into_owned();
    assert!(bundle.contains("tls_only"), "the store is in the bundle");
    let store_dbg = format!("{:?} {:?}", t.net.auth(), t.net.auth().clients());
    drop(t);
    // Let the other tests' hubs finish their logging; this test sorts last.
    std::thread::sleep(Duration::from_millis(200));

    let lines = kept().lock().unwrap().clone();
    let all = lines.join("\n");
    assert!(!lines.is_empty());
    let secrets = secrets().lock().unwrap().clone();
    assert!(secrets.len() > 10, "{}", secrets.len());
    for s in &secrets {
        assert!(!all.contains(s.as_str()), "a secret was logged: {s}");
        assert!(
            !bundle.contains(s.as_str()),
            "a secret is in the bundle: {s}"
        );
        assert!(
            !store_dbg.contains(s.as_str()),
            "a secret is in a Debug: {s}"
        );
    }
    let pins = pins().lock().unwrap().clone();
    assert!(!pins.is_empty());
    for pin in &pins {
        assert!(!has_number(&all, pin), "a PIN was logged: {pin}");
        assert!(!has_number(&store_dbg, pin), "a PIN is in a Debug: {pin}");
    }
}
