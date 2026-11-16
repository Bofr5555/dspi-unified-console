//! WP SECDEF (tests written by WP SECDOC): the upgrade path of a hub store written
//! before TLS (PLAN-SEC 5, SECDEF; OD-SECb).
//!
//! A pre-TLS store is a `hub-store.json` with paired clients and no `tls/` folder,
//! its tokens never used over TLS (no `tls_only`). Written here literally, in the
//! shape every hub before SEC-B wrote, so the test does not depend on how today's
//! hub writes it.
//!
//! - In `off` nothing changes: no key is made, the old token works in plaintext.
//! - In `on` the key is made at the first start (0600 in a 0700 folder), kept at
//!   the next, the old token still works over plaintext, and over TLS; once it has
//!   been used over TLS it is `tls_only` (ratchet (b)).
//! - In `require` a plaintext client is told what to do (`426` with `https://` and
//!   the key's fingerprint; a WebSocket closed 4003 `tls_required`), and an updated
//!   client gets in with the same token over TLS, its credential upgraded to a pin:
//!   nobody pairs again.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dspi_core::DspiCore;
use dspi_hub::server::{NetworkControl, ServerLimits};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role, TlsMode};
use dspi_link::tls::{self as link_tls, PinMode, SpkiHash, TlsPipe};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use sha2::Digest;
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const HUB_ID: &str = "6f1c2b8e-3d4a-4f5b-9c6d-7e8f9a0b1c2d";
const OLD_CLIENT: &str = "Old laptop";
const DAY: u64 = 86_400;

fn store_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    // ci/run.sh points TMPDIR at the build cache's home volume, never tmpfs.
    let d = std::env::temp_dir().join("wp-secdef-upgrade").join(format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A token as a hub issues them (32 random bytes, base64url), fixed here.
fn old_token() -> String {
    let bytes: Vec<u8> = (0u8..32)
        .map(|b| b.wrapping_mul(37).wrapping_add(11))
        .collect();
    data_encoding::BASE64URL_NOPAD.encode(&bytes)
}

/// A store as a hub before TLS wrote it: one console paired ten days ago.
fn write_pre_tls_store(dir: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let hash = data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(old_token().as_bytes()));
    let store = json!({
        "hub_id": HUB_ID,
        "name": "Pre-TLS hub",
        "access": "pin",
        "next_cid": 2,
        "clients": [{
            "cid": 1,
            "name": OLD_CLIENT,
            "role": "control",
            "created": now - 10 * DAY,
            "last_seen": now - 10 * DAY,
            "token_sha256": hash,
        }],
        "device_names": {},
    });
    std::fs::write(dir.join("hub-store.json"), store.to_string()).unwrap();
    assert!(!dir.join("tls").exists(), "a pre-TLS store has no key");
}

struct Target {
    hub: Hub,
    net: NetworkControl,
    port: u16,
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

fn start(store: &Path, tls: TlsMode) -> Target {
    let cfg = NetworkConfig {
        port: 0,
        access: AccessMode::Pin,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        limits: ServerLimits {
            head_timeout: Duration::from_millis(1500),
            auth_timeout: Duration::from_millis(3000),
            close_grace: Duration::from_millis(300),
            read_tick: Duration::from_millis(20),
            ..ServerLimits::default()
        },
        tls,
        upstream_compat: false,
        ..NetworkConfig::new(store.to_path_buf())
    };
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        ..HubConfig::in_process("SECDEF hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Target { hub, net, port }
}

// ───────────────────────────── raw clients ─────────────────────────────

fn tcp(port: u16) -> TcpStream {
    let tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp
}

fn tls(t: &Target) -> TlsPipe {
    let deadline = Instant::now() + Duration::from_secs(3);
    let (mut pipe, _) =
        link_tls::connect(tcp(t.port), "localhost", PinMode::Pinned(t.pin()), deadline)
            .expect("a pinned TLS connection");
    pipe.set_deadline(None);
    pipe
}

fn read_head<S: Read>(s: &mut S) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => head.push(byte[0]),
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// A plain `GET`: the status line and the body.
fn get_plain(port: u16, path: &str) -> (String, String) {
    let mut s = tcp(port);
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut all = Vec::new();
    let _ = s.read_to_end(&mut all);
    let text = String::from_utf8_lossy(&all).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (
        head.lines().next().unwrap_or("").to_owned(),
        body.to_owned(),
    )
}

struct Ws<S: Read + Write>(WebSocket<S>);

impl<S: Read + Write> Ws<S> {
    /// Upgrade, without the hello: the caller decides what comes next.
    fn upgrade(mut s: S, port: u16) -> Self {
        let upgrade = format!(
            "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
             Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Protocol: dspi-link-1\r\n\r\n"
        );
        s.write_all(upgrade.as_bytes()).unwrap();
        s.flush().unwrap();
        let head = read_head(&mut s);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        Self(WebSocket::from_raw_socket(s, WsRole::Client, None))
    }

    fn open(s: S, port: u16) -> Self {
        let mut ws = Self::upgrade(s, port);
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "secdef", "kind": "test"}}));
        let hello = ws.recv().expect("hello");
        assert_eq!(hello["t"], "hello", "{hello}");
        assert_eq!(
            hello["hub"]["id"], HUB_ID,
            "the old store's hub id: {hello}"
        );
        ws
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

    /// The close frame the hub ends with, if it does.
    fn close_frame(&mut self) -> Option<(u16, String)> {
        for _ in 0..200 {
            match self.0.read() {
                Ok(Message::Close(Some(f))) => return Some((f.code.into(), f.reason.to_string())),
                Ok(Message::Close(None)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    }

    fn token(&mut self, token: &str) -> Value {
        self.send(json!({"t": "auth.token", "id": 2, "token": token}));
        loop {
            let r = self.recv().expect("a reply");
            if r["id"] == 2 {
                return r;
            }
        }
    }
}

// ───────────────────────────── the core's client ─────────────────────────────

/// An updated client with the credential it kept from before TLS (a bare token,
/// `v1`): it connects, and its credential comes back upgraded to a pin.
fn core_with_v1(t: &Target) -> (std::sync::Arc<DspiCore>, String) {
    let core = DspiCore::new();
    let hello = core
        .link_connect(
            format!("ws://127.0.0.1:{}/dspi/v1", t.port),
            "wp-secdef".into(),
            "0".into(),
            old_token(),
            HUB_ID.into(),
        )
        .expect("the old token gets in");
    assert!(!hello.needs_pairing, "nobody pairs again");
    let security = core.link_security();
    assert!(security.encrypted, "over TLS: {security:?}");
    assert!(security.hub_key_new, "the one first-use moment is shown");
    assert_eq!(security.hub_key, t.pin().display());
    let credential = security.credential.clone();
    assert!(
        credential.starts_with("v2."),
        "upgraded to a pinned credential"
    );
    (core, credential)
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn key_file(store: &Path) -> PathBuf {
    store.join("tls").join("key.pem")
}

// ───────────────────────────── the tests ─────────────────────────────

#[test]
fn wp_secdef_a_pre_tls_store_in_off_is_unchanged() {
    let store = store_dir();
    write_pre_tls_store(&store);
    let t = start(&store, TlsMode::Off);
    assert!(t.net.tls().pin.is_none());
    let mut ws = Ws::open(tcp(t.port), t.port);
    let r = ws.token(&old_token());
    assert_eq!(r["t"], "ok", "{r}");
    assert_eq!(r["role"], "control");
    assert!(r.get("hub_key").is_none(), "{r}");
    drop(ws);
    drop(t);
    assert!(!store.join("tls").exists(), "no key is made in off");
}

#[test]
fn wp_secdef_a_pre_tls_store_in_on_gets_a_key_and_keeps_its_tokens() {
    let store = store_dir();
    write_pre_tls_store(&store);
    let t = start(&store, TlsMode::On);

    // The key, made at this first start, private.
    let key = key_file(&store);
    assert!(key.is_file(), "a key on the first start in on");
    #[cfg(unix)]
    {
        assert_eq!(mode_of(&key), 0o600, "key.pem");
        assert_eq!(mode_of(&store.join("tls")), 0o700, "tls/");
    }
    let pin = t.pin();
    // The paired client is still there, with its role.
    let clients = t.net.auth().clients();
    assert!(
        clients
            .iter()
            .any(|c| c.cid == 1 && c.name == OLD_CLIENT && c.role == Role::Control),
        "{clients:?}"
    );

    // The old token still works over plaintext: a client not yet updated.
    let mut ws = Ws::open(tcp(t.port), t.port);
    let r = ws.token(&old_token());
    assert_eq!(r["t"], "ok", "plaintext, not yet updated: {r}");
    drop(ws);

    // ... and over TLS, where the hub names its key.
    let mut ws = Ws::open(tls(&t), t.port);
    let r = ws.token(&old_token());
    assert_eq!(r["t"], "ok", "{r}");
    assert_eq!(r["hub_key"], pin.to_b64url());
    drop(ws);

    // Once used over TLS it is tls_only: a plaintext replay of it is refused.
    let mut ws = Ws::open(tcp(t.port), t.port);
    let r = ws.token(&old_token());
    assert_eq!(r["code"], "tls_required", "ratchet (b): {r}");
    drop(ws);
    drop(t);

    // The next start keeps the key: no client sees a key change.
    let t = start(&store, TlsMode::On);
    assert_eq!(t.pin(), pin, "the same key at the next start");
}

#[test]
fn wp_secdef_an_updated_client_upgrades_its_v1_credential_in_on() {
    let store = store_dir();
    write_pre_tls_store(&store);
    let t = start(&store, TlsMode::On);
    let (core, credential) = core_with_v1(&t);
    core.link_close();
    // The stored v2 credential gets in again, pinned, with nothing new to show.
    let core = DspiCore::new();
    let hello = core
        .link_connect(
            format!("ws://127.0.0.1:{}/dspi/v1", t.port),
            "wp-secdef".into(),
            "0".into(),
            credential,
            HUB_ID.into(),
        )
        .unwrap();
    assert!(!hello.needs_pairing);
    let security = core.link_security();
    assert!(security.encrypted && !security.hub_key_new, "{security:?}");
    core.link_close();
}

#[test]
fn wp_secdef_a_pre_tls_store_in_require_has_a_clear_path() {
    let store = store_dir();
    write_pre_tls_store(&store);
    let t = start(&store, TlsMode::Require);
    assert!(
        key_file(&store).is_file(),
        "a key on the first start in require"
    );
    let fingerprint = t.pin().display();

    // A browser or script over plaintext is told where to go and which key to expect.
    let (status, body) = get_plain(t.port, "/dspi/v1/info");
    assert!(status.contains("426"), "{status}");
    assert!(body.contains("https://"), "{body}");
    assert!(
        body.contains(&fingerprint),
        "the fingerprint to compare: {body}"
    );

    // A client not yet updated: the upgrade is answered, then closed with the reason.
    let mut ws = Ws::upgrade(tcp(t.port), t.port);
    let (code, reason) = ws.close_frame().expect("a close frame");
    assert_eq!(code, 4003, "{reason}");
    assert!(reason.starts_with("tls_required"), "{reason}");
    drop(ws);

    // The same token over TLS: an updated client needs no new pairing.
    let mut ws = Ws::open(tls(&t), t.port);
    let r = ws.token(&old_token());
    assert_eq!(r["t"], "ok", "{r}");
    assert_eq!(r["hub_key"], t.pin().to_b64url());
    drop(ws);

    // And the core does it by itself, from the v1 credential it kept.
    let (core, _) = core_with_v1(&t);
    core.link_close();
}
