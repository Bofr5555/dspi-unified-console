//! WP MQWIRE: the hub's MQTT service with SECOUT's TLS, through the real hub and server.
//!
//! TLS settings round-trip through `mqtt.set` and `mqtt.get` (the client key shown as `set`,
//! kept when sent back); a connection to a TLS fake broker (the TM12 test certificates)
//! reports `encrypted: true`; a `mqtt.json` written before TLS shows the migration notice
//! on the first start only.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use data_encoding::BASE64;
use dspi_hub::mqtt::QoS;
use dspi_hub::mqtt::client::MQTT_MIGRATED_NOTICE;
use dspi_hub::mqtt::codec::{self, Packet};
use dspi_hub::server::NetworkControl;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde_json::{Value, json};
use tungstenite::Message;

const CA: &[u8] = include_bytes!("../../dspi-tui/testdata/tm12/ca.der");
const LEAF: &[u8] = include_bytes!("../../dspi-tui/testdata/tm12/leaf.der");
const LEAF_KEY: &[u8] = include_bytes!("../../dspi-tui/testdata/tm12/leaf-key.der");
const WAIT: Duration = Duration::from_secs(20);
const HELLO: &str = r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"mqwire","app":"test","version":"0"}}"#;

fn pem(label: &str, der: &[u8]) -> String {
    let b64 = BASE64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-mqwire").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

// ── A fake broker in TLS: CONNACK, PUBACK, PINGRESP ─────────────────────────

struct TlsBroker {
    addr: SocketAddr,
    connects: Arc<AtomicUsize>,
}

impl TlsBroker {
    fn start() -> TlsBroker {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = Arc::new(
            ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(LEAF.to_vec())],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(LEAF_KEY.to_vec())),
                )
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let broker = TlsBroker {
            addr: listener.local_addr().unwrap(),
            connects: Arc::new(AtomicUsize::new(0)),
        };
        let connects = Arc::clone(&broker.connects);
        thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { return };
                let config = Arc::clone(&config);
                let connects = Arc::clone(&connects);
                thread::spawn(move || {
                    s.set_read_timeout(Some(WAIT)).unwrap();
                    let mut conn = ServerConnection::new(config).unwrap();
                    while conn.is_handshaking() {
                        if conn.complete_io(&mut s).is_err() {
                            return;
                        }
                    }
                    let mut tls = StreamOwned::new(conn, s);
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        while let Ok(Some((p, used))) = codec::decode(&buf, 1 << 20) {
                            buf.drain(..used);
                            let reply = match p {
                                Packet::Connect(_) => {
                                    connects.fetch_add(1, Ordering::SeqCst);
                                    Some(Packet::Connack {
                                        session_present: false,
                                        return_code: 0,
                                    })
                                }
                                Packet::Publish(p) if p.qos == QoS::AtLeastOnce => {
                                    Some(Packet::Puback {
                                        packet_id: p.packet_id,
                                    })
                                }
                                Packet::Pingreq => Some(Packet::Pingresp),
                                Packet::Disconnect => return,
                                _ => None,
                            };
                            if let Some(r) = reply {
                                let _ = tls.write_all(&codec::encode(&r).unwrap());
                                let _ = tls.flush();
                            }
                        }
                        match tls.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                });
            }
        });
        broker
    }
}

// ── The hub and the socket ──────────────────────────────────────────────────

type Ws = tungstenite::WebSocket<TcpStream>;

fn ask_text(ws: &mut Ws, mut msg: Value) -> (Value, String) {
    static ID: AtomicU32 = AtomicU32::new(100);
    let id = ID.fetch_add(1, Ordering::SeqCst);
    msg["id"] = id.into();
    ws.send(Message::text(msg.to_string())).unwrap();
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no answer to {id}");
        if let Message::Text(text) = ws.read().unwrap() {
            let v: Value = serde_json::from_str(text.as_str()).unwrap();
            if v["id"] == id {
                return (v, text.as_str().to_owned());
            }
        }
    }
}

fn ask(ws: &mut Ws, msg: Value) -> Value {
    ask_text(ws, msg).0
}

fn admin(net: &NetworkControl) -> Ws {
    let url = format!("ws://127.0.0.1:{}/dspi/v1", net.port());
    let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
    let tcp = TcpStream::connect(("127.0.0.1", net.port())).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let mut ws = tungstenite::client(req, tcp).unwrap().0;
    ws.send(Message::text(HELLO)).unwrap();
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    let ok = ask(
        &mut ws,
        json!({"t": "auth.pair", "pin": pin, "name": "mqwire", "role": "control"}),
    );
    assert_eq!(ok["t"], "ok", "{ok}");
    let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
    assert!(net.set_role(cid, Role::Admin));
    wait_until("the admin role", || {
        ask(&mut ws, json!({"t": "mqtt.status"}))["t"] == "ok"
    });
    ws
}

fn hub_on(dir: &Path) -> Hub {
    Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.to_path_buf())
        }),
        initial_scan: false,
        ..HubConfig::in_process("MQWIRE hub")
    })
    .unwrap()
}

fn file(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("mqtt.json")).unwrap()).unwrap()
}

// ── The tests ───────────────────────────────────────────────────────────────

#[test]
fn wp_mqwire_tls_settings_round_trip_and_the_key_is_never_answered() {
    let dir = scratch("roundtrip");
    let hub = hub_on(&dir);
    let mut ws = admin(&hub.network().unwrap());
    let key = pem("PRIVATE KEY", LEAF_KEY);
    let key_line = key.lines().nth(1).unwrap().to_owned();
    let (set, text) = ask_text(
        &mut ws,
        json!({"t": "mqtt.set", "settings": {"broker": "localhost", "tls": {
            "mode": "on", "ca_pem": pem("CERTIFICATE", CA),
            "client_cert_pem": pem("CERTIFICATE", LEAF), "client_key_pem": key}}}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    assert!(!text.contains(&key_line), "the key is never answered");
    let tls = &set["settings"]["tls"];
    assert_eq!(tls["mode"], "on", "{set}");
    assert_eq!(tls["client_key_pem"], "set");
    assert!(
        tls["ca_pem"]
            .as_str()
            .unwrap()
            .contains("BEGIN CERTIFICATE")
    );
    assert_eq!(set["settings"]["port"], 8883, "no port: 8883 with TLS");
    assert!(
        file(&dir)["tls"]["client_key_pem"]
            .as_str()
            .unwrap()
            .contains(&key_line)
    );

    // `mqtt.get`'s settings sent back as they are keep the key.
    let (get, text) = ask_text(&mut ws, json!({"t": "mqtt.get"}));
    assert!(!text.contains(&key_line));
    let back = ask(
        &mut ws,
        json!({"t": "mqtt.set", "settings": get["settings"]}),
    );
    assert_eq!(back["t"], "ok", "{back}");
    assert_eq!(back["settings"], get["settings"]);
    assert!(
        file(&dir)["tls"]["client_key_pem"]
            .as_str()
            .unwrap()
            .contains(&key_line)
    );

    // A bad `tls` is refused whole; off again keeps nothing of TLS.
    let bad = ask(
        &mut ws,
        json!({"t": "mqtt.set", "settings": {"tls": {"mode": "on", "pin_sha256": "nope"}}}),
    );
    assert_eq!(bad["code"], "bad_request", "{bad}");
    assert!(bad["msg"].as_str().unwrap().starts_with("tls:"), "{bad}");
    let off = ask(
        &mut ws,
        // `mqtt.get`'s port is the one in use, so sending it back fixed it: `null` is the default.
        json!({"t": "mqtt.set", "settings": {"port": null, "tls": {"mode": "off"}}}),
    );
    assert_eq!(off["t"], "ok", "{off}");
    assert_eq!(off["settings"]["port"], 1883);
    assert_eq!(off["settings"]["tls"]["client_key_pem"], "not set");
    drop(ws);
    hub.stop();
}

#[test]
fn wp_mqwire_a_tls_broker_is_encrypted() {
    let dir = scratch("encrypted");
    let broker = TlsBroker::start();
    let hub = hub_on(&dir);
    let mut ws = admin(&hub.network().unwrap());
    let set = ask(
        &mut ws,
        json!({"t": "mqtt.set", "settings": {"enabled": true, "broker": "localhost",
            "port": broker.addr.port(), "username": "hub", "password": "pw-mqwire",
            "tls": {"mode": "on", "ca_pem": pem("CERTIFICATE", CA)}}}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    let start = Instant::now();
    let status = loop {
        let s = ask(&mut ws, json!({"t": "mqtt.status"}));
        if s["state"] == "connected" && s["encrypted"] == true {
            break s;
        }
        assert!(start.elapsed() < WAIT, "never connected over TLS: {s}");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.get("notice").is_none(), "{status}");
    assert!(broker.connects.load(Ordering::SeqCst) >= 1);
    drop(ws);
    hub.stop();
}

#[test]
fn wp_mqwire_a_file_from_before_tls_shows_the_notice_once() {
    let dir = scratch("migrated");
    std::fs::create_dir_all(&dir).unwrap();
    // As P2MQ wrote it, MQTT off so nothing connects to the made-up broker.
    std::fs::write(
        dir.join("mqtt.json"),
        r#"{"v":1,"enabled":false,"broker":"broker.example","port":1883,"client_id":null,
            "username":"hub","password":"pw-old","base_topic":null,
            "ha":{"enabled":false,"key":null},"tls":null}"#,
    )
    .unwrap();
    {
        let hub = hub_on(&dir);
        let mut ws = admin(&hub.network().unwrap());
        let get = ask(&mut ws, json!({"t": "mqtt.get"}));
        assert_eq!(get["notice"], MQTT_MIGRATED_NOTICE, "{get}");
        assert_eq!(get["settings"]["tls"]["allow_plaintext_password"], true);
        let status = ask(&mut ws, json!({"t": "mqtt.status"}));
        assert_eq!(status["notice"], MQTT_MIGRATED_NOTICE, "{status}");
        assert_eq!(
            file(&dir)["tls"]["allow_plaintext_password"],
            true,
            "written back"
        );
        drop(ws);
        hub.stop();
    }
    let hub = hub_on(&dir);
    let mut ws = admin(&hub.network().unwrap());
    let get = ask(&mut ws, json!({"t": "mqtt.get"}));
    assert!(get.get("notice").is_none(), "once: {get}");
    assert_eq!(get["settings"]["tls"]["allow_plaintext_password"], true);
    assert!(
        ask(&mut ws, json!({"t": "mqtt.status"}))
            .get("notice")
            .is_none()
    );
    drop(ws);
    hub.stop();
}
