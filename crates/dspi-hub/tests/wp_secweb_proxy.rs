//! WP SECWEB: the `uconsole web` pinning proxy (PLAN-SEC 4.6, way 2). A hub bound to
//! loopback with its web pages (as `uconsole web` runs it) relays a page's
//! `ws://localhost:PORT/hub/<target>/dspi/v1` to a remote hub over TLS pinned to its key.
//!
//! The page is played by hand-written HTTP and WebSocket bytes, so every answer is seen
//! as the wire carries it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_hub::server::ServerLimits;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, TlsMode};
use dspi_link::tls::SpkiHash;
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

fn store_dir() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    // ci/run.sh points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("wp-secweb").join(format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Node {
    hub: Hub,
    net: dspi_hub::server::NetworkControl,
    port: u16,
    store: std::path::PathBuf,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.hub.stop();
        let _ = std::fs::remove_dir_all(&self.store);
    }
}

fn fast() -> ServerLimits {
    ServerLimits {
        head_timeout: Duration::from_millis(1500),
        auth_timeout: Duration::from_millis(4000),
        close_grace: Duration::from_millis(300),
        read_tick: Duration::from_millis(20),
        ..ServerLimits::default()
    }
}

fn start(name: &str, tls: TlsMode, web: bool, bind: IpAddr) -> Node {
    let store = store_dir();
    let cfg = NetworkConfig {
        port: 0,
        access: AccessMode::Pin,
        web,
        bind,
        limits: fast(),
        tls,
        upstream_compat: false,
        ..NetworkConfig::new(store.clone())
    };
    let hub = Hub::start(HubConfig {
        network: Some(cfg),
        ..HubConfig::in_process(name)
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let net = hub.network().unwrap();
    let port = net.port();
    Node {
        hub,
        net,
        port,
        store,
    }
}

/// A remote hub with TLS, and the `uconsole web` hub in front of it.
fn pair_of_hubs(tls: TlsMode) -> (Node, Node) {
    let remote = start("SECWEB remote", tls, false, IpAddr::V4(LOOPBACK));
    let proxy = start("SECWEB web", TlsMode::Off, true, IpAddr::V4(LOOPBACK));
    (remote, proxy)
}

fn connect(port: u16) -> TcpStream {
    let tcp = TcpStream::connect((LOOPBACK, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp
}

fn read_head(s: &mut TcpStream) -> String {
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

fn upgrade_bytes(path: &str, host: &str, origin: Option<&str>) -> String {
    let mut r = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n"
    );
    if let Some(o) = origin {
        r.push_str(&format!("Origin: {o}\r\n"));
    }
    r.push_str("\r\n");
    r
}

/// One request with `Connection: close`: (status, body).
fn http(port: u16, request: &str) -> (u16, String) {
    let mut s = connect(port);
    s.write_all(request.as_bytes()).unwrap();
    let mut all = Vec::new();
    let _ = s.read_to_end(&mut all);
    let text = String::from_utf8_lossy(&all).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (status_of(head), body.to_owned())
}

struct Ws(WebSocket<TcpStream>);

impl Ws {
    /// The page's upgrade through the proxy at `port`: the status when not 101.
    fn open(port: u16, path: &str, host: &str, origin: Option<&str>) -> Result<Ws, (u16, String)> {
        let mut s = connect(port);
        s.write_all(upgrade_bytes(path, host, origin).as_bytes())
            .unwrap();
        let head = read_head(&mut s);
        let code = status_of(&head);
        if code != 101 {
            let mut body = String::new();
            let _ = s.read_to_string(&mut body);
            return Err((code, body));
        }
        Ok(Ws(WebSocket::from_raw_socket(s, WsRole::Client, None)))
    }

    fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).unwrap();
    }

    /// The next text message, or the close frame's `(code, reason)`.
    fn next(&mut self) -> Result<Value, (u16, String)> {
        for _ in 0..500 {
            match self.0.read() {
                Ok(Message::Text(t)) => return Ok(serde_json::from_str(t.as_str()).unwrap()),
                Ok(Message::Close(Some(f))) => {
                    return Err((u16::from(f.code), f.reason.to_string()));
                }
                Ok(Message::Close(None)) | Err(_) => return Err((0, String::new())),
                Ok(_) => {}
            }
        }
        Err((0, "nothing".into()))
    }

    fn hello(&mut self) -> Result<Value, (u16, String)> {
        self.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2}, "client": {"name": "secweb", "kind": "test"}}));
        self.next()
    }

    fn ask(&mut self, v: Value) -> Value {
        let id = v["id"].clone();
        self.send(v);
        loop {
            let r = self.next().expect("a reply");
            if r["id"] == id {
                return r;
            }
        }
    }
}

fn page_origin(p: &Node) -> String {
    format!("http://127.0.0.1:{}", p.port)
}

fn page_host(p: &Node) -> String {
    format!("127.0.0.1:{}", p.port)
}

fn link_path(remote: &Node) -> String {
    format!("/hub/127.0.0.1%3A{}/dspi/v1", remote.port)
}

fn security(p: &Node, target: &str) -> Value {
    let (code, body) = http(
        p.port,
        &format!(
            "GET /hub/{target}/dspi/v1/security HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            page_host(p)
        ),
    );
    assert_eq!(code, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn remote_pin(r: &Node) -> SpkiHash {
    r.net.tls().pin.expect("TLS is served")
}

fn open_page(p: &Node, r: &Node) -> Ws {
    Ws::open(p.port, &link_path(r), &page_host(p), Some(&page_origin(p))).expect("an upgrade")
}

#[test]
fn wp_secweb_the_proxy_pins_relays_and_pairs_over_tls() {
    let (remote, p) = pair_of_hubs(TlsMode::Require);
    let mut ws = open_page(&p, &remote);
    let hello = ws.hello().unwrap();
    assert_eq!(hello["t"], "hello", "{hello}");
    let hid = hello["hub"]["id"].as_str().unwrap().to_owned();
    // Pairing through the relay: the PIN and the token cross only the pinned TLS.
    let w = remote.net.auth().begin_pairing(Instant::now()).unwrap();
    let ok = ws.ask(json!({"t": "auth.pair", "id": 1, "pin": w.pin, "name": "secweb"}));
    assert!(ok["token"].as_str().is_some_and(|t| !t.is_empty()), "{ok}");
    let r = ws.ask(json!({"t": "device.list", "id": 2}));
    assert_eq!(r["t"], "ok", "{r}");
    drop(ws);

    let target = format!("127.0.0.1%3A{}", remote.port);
    let s = security(&p, &target);
    assert_eq!(s["encrypted"], true, "{s}");
    assert_eq!(s["hubId"], hid.as_str(), "{s}");
    assert_eq!(s["hubKey"], remote_pin(&remote).display(), "{s}");
    assert_eq!(s["hubKeyNew"], true, "{s}");
    assert_eq!(s["tls"], true, "{s}");
    assert_eq!(s["upstreamCompat"], false, "{s}");
    assert_eq!(s["refused"], "", "{s}");
    // Nothing secret in what the page may read.
    assert!(!s.to_string().contains(&w.pin), "{s}");
    assert!(
        !s.to_string().contains(ok["token"].as_str().unwrap()),
        "{s}"
    );

    // The pin is kept, 0600, and the hub is reachable by its id too; not new any more.
    let file = p.store.join(dspi_hub::proxy::PINS_FILE);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains(&hid) && text.contains(&remote_pin(&remote).to_b64url()),
        "{text}"
    );
    assert!(!text.contains(&w.pin));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    let by_id = format!("/hub/{hid}/dspi/v1");
    let mut ws = Ws::open(p.port, &by_id, &page_host(&p), Some(&page_origin(&p))).unwrap();
    assert_eq!(ws.hello().unwrap()["hub"]["id"], hid.as_str());
    drop(ws);
    assert_eq!(security(&p, &hid)["hubKeyNew"], false);
}

#[test]
fn wp_secweb_a_changed_key_is_refused_before_any_token() {
    let (remote, p) = pair_of_hubs(TlsMode::Require);
    let mut ws = open_page(&p, &remote);
    ws.hello().unwrap();
    drop(ws);
    // A new key with no signature by the old one (a compromise, or another machine).
    remote.net.rotate_tls_key(true).unwrap();
    let mut ws = open_page(&p, &remote);
    // The hub's hello never reaches the page, so neither could a token.
    assert_eq!(
        ws.hello().unwrap_err(),
        (4000, "hub_key_changed".to_owned())
    );
    let s = security(&p, &format!("127.0.0.1%3A{}", remote.port));
    assert_eq!(s["refused"], "hub_key_changed", "{s}");
    assert_eq!(s["encrypted"], false, "{s}");
    // ... and stays refused, until the user forgets the pin on purpose.
    let mut ws = open_page(&p, &remote);
    assert_eq!(ws.hello().unwrap_err().1, "hub_key_changed");
    let forget = format!(
        "POST /hub/127.0.0.1%3A{}/dspi/v1/forget HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        remote.port,
        page_host(&p),
        page_origin(&p)
    );
    let (code, body) = http(p.port, &forget);
    assert_eq!((code, body.contains("true")), (200, true), "{body}");
    let mut ws = open_page(&p, &remote);
    assert_eq!(ws.hello().unwrap()["t"], "hello");
}

#[test]
fn wp_secweb_a_pinned_hub_that_drops_tls_is_never_reached_in_plaintext() {
    let (remote, p) = pair_of_hubs(TlsMode::On);
    let mut ws = open_page(&p, &remote);
    ws.hello().unwrap();
    drop(ws);
    let port = remote.port;
    drop(remote);
    // The same address now serves plaintext only (a downgrade).
    let store = store_dir();
    let plain = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port,
            access: AccessMode::Pin,
            bind: IpAddr::V4(LOOPBACK),
            limits: fast(),
            tls: TlsMode::Off,
            ..NetworkConfig::new(store.clone())
        }),
        ..HubConfig::in_process("SECWEB plain")
    })
    .expect("the same port again");
    let target = format!("/hub/127.0.0.1%3A{port}/dspi/v1");
    let refused = Ws::open(p.port, &target, &page_host(&p), Some(&page_origin(&p))).err();
    assert_eq!(refused.map(|r| r.0), Some(502));
    let s = security(&p, &format!("127.0.0.1%3A{port}"));
    assert_eq!(s["refused"], "tls_refused", "{s}");
    plain.stop();
    let _ = std::fs::remove_dir_all(&store);
}

#[test]
fn wp_secweb_an_upstream_plaintext_hub_still_connects_unencrypted() {
    // Compatibility policy (owner, 2026-11-15): our clients must still reach an upstream
    // hub, which has no TLS. The page is told it is unencrypted (its red bar).
    let (remote, p) = pair_of_hubs(TlsMode::Off);
    let mut ws = open_page(&p, &remote);
    assert_eq!(ws.hello().unwrap()["t"], "hello");
    let s = security(&p, &format!("127.0.0.1%3A{}", remote.port));
    assert_eq!(s["encrypted"], false, "{s}");
    assert_eq!(s["hubKey"], "", "{s}");
    assert_eq!(s["tls"], false, "{s}");
    assert_eq!(s["refused"], "", "{s}");
    // Not pinned: nothing to pin without TLS.
    assert!(!p.store.join(dspi_hub::proxy::PINS_FILE).exists());
}

#[test]
fn wp_secweb_foreign_origin_host_or_missing_origin_is_refused() {
    let (remote, p) = pair_of_hubs(TlsMode::Require);
    let path = link_path(&remote);
    let host = page_host(&p);
    for origin in [
        None,
        Some("http://evil.example".to_owned()),
        Some("null".to_owned()),
        Some(format!("http://127.0.0.1:{}", p.port.wrapping_add(1))),
        Some(format!("https://127.0.0.1:{}", p.port)),
    ] {
        let r = Ws::open(p.port, &path, &host, origin.as_deref()).err();
        assert_eq!(r.map(|r| r.0), Some(403), "{origin:?}");
    }
    // A rebound name, or a LAN address in Host, though the peer is loopback.
    for bad_host in [
        "evil.example",
        &format!("evil.example:{}", p.port),
        &format!("10.1.2.3:{}", p.port),
    ] {
        let r = Ws::open(p.port, &path, bad_host, Some(&page_origin(&p))).err();
        assert!(matches!(r.map(|r| r.0), Some(403 | 421)), "{bad_host}");
    }
    // `localhost` is the page's name too.
    let lh = format!("localhost:{}", p.port);
    let mut ws = Ws::open(p.port, &path, &lh, Some(&format!("http://{lh}"))).unwrap();
    assert_eq!(ws.hello().unwrap()["t"], "hello");
    // Forget needs the page's Origin and POST.
    let get_forget = format!(
        "GET /hub/x/dspi/v1/forget HTTP/1.1\r\nHost: {host}\r\nOrigin: {}\r\nConnection: close\r\n\r\n",
        page_origin(&p)
    );
    assert_eq!(http(p.port, &get_forget).0, 405);
    let no_origin = format!(
        "POST /hub/x/dspi/v1/forget HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(http(p.port, &no_origin).0, 403);
    // Not a target: refused before any dial.
    let r = Ws::open(p.port, "/hub/a%2Fb/dspi/v1", &host, Some(&page_origin(&p))).err();
    assert_eq!(r.map(|r| r.0), Some(404));
    // /info says the proxy is there.
    let (_, info) = http(
        p.port,
        &format!("GET /dspi/v1/info HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
    );
    assert!(info.contains("\"proxy\":true"), "{info}");
}

#[test]
fn wp_secweb_no_proxy_on_a_lan_bound_hub_nor_without_web() {
    // Bound to every interface: the proxy routes do not exist (nor on a LAN address).
    let lan = start(
        "SECWEB lan",
        TlsMode::Off,
        true,
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    );
    let remote = start(
        "SECWEB remote",
        TlsMode::Require,
        false,
        IpAddr::V4(LOOPBACK),
    );
    let path = link_path(&remote);
    let host = format!("127.0.0.1:{}", lan.port);
    let r = Ws::open(lan.port, &path, &host, Some(&format!("http://{host}"))).err();
    assert!(
        matches!(r.as_ref().map(|r| r.0), Some(404 | 400 | 403)),
        "{r:?}"
    );
    let (_, info) = http(
        lan.port,
        &format!("GET /dspi/v1/info HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
    );
    assert!(info.contains("\"proxy\":false"), "{info}");
    // Loopback without the web pages: none either.
    let quiet = start("SECWEB quiet", TlsMode::Off, false, IpAddr::V4(LOOPBACK));
    let host = format!("127.0.0.1:{}", quiet.port);
    let r = Ws::open(quiet.port, &path, &host, Some(&format!("http://{host}"))).err();
    assert!(r.is_some());
}
