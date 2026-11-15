//! WP P2MQ: MQTT on the hub, through the real hub and server, to a fake broker in this
//! test (loopback only).
//!
//! Off by default; `mqtt.set` (admin) turns it on, the hub connects and says `online`
//! (retained) with the will `offline`; an `Alarm` put on the bus (as P2AL will) and a
//! history record are published to `<base>/<serial>/event`; a lost connection makes the
//! broker publish the will and the hub says `online` again when back; a clean stop says
//! `offline`. A wrong password is `failed` in `mqtt.status`, without the password. The
//! setting is kept 0600 and connects again on the next start. Control and viewer are
//! `denied` every `mqtt.*`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dspi_hub::mqtt::QoS;
use dspi_hub::mqtt::codec::{self, Packet, Will};
use dspi_hub::server::NetworkControl;
use dspi_hub::system::alarms::AlarmState;
use dspi_hub::{AccessMode, BusMessage, Hub, HubConfig, NetworkConfig, Role};
use dspi_session::events::EventRecord;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
const HELLO: &str = r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"p2mq","app":"test","version":"0"}}"#;
const PASSWORD: &str = "p2mq-sekrit-pw";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2mq").join(format!(
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

// ── The fake broker ─────────────────────────────────────────────────────────

#[derive(Default)]
struct State {
    /// Every packet from a client, with its connection's number.
    received: Vec<(usize, Packet)>,
    /// Each retained topic's payloads in the order they were set (a will included).
    retained_log: Vec<(String, Vec<u8>)>,
    conns: Vec<Option<TcpStream>>,
    wills: Vec<Option<Will>>,
    connack_code: u8,
}

#[derive(Clone)]
struct Broker {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
}

impl Broker {
    fn start() -> Broker {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let broker = Broker {
            addr: listener.local_addr().unwrap(),
            state: Arc::new(Mutex::new(State::default())),
        };
        let b = broker.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let n = {
                    let mut s = b.state.lock().unwrap();
                    s.conns.push(Some(stream.try_clone().unwrap()));
                    s.wills.push(None);
                    s.conns.len() - 1
                };
                let b = b.clone();
                thread::spawn(move || b.serve(n, stream));
            }
        });
        broker
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    fn send(&self, conn: usize, p: &Packet) {
        let bytes = codec::encode(p).unwrap();
        let mut s = self.state.lock().unwrap();
        if let Some(Some(w)) = s.conns.get_mut(conn) {
            let _ = w.write_all(&bytes);
        }
    }

    fn serve(&self, conn: usize, mut stream: TcpStream) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let clean = 'conn: loop {
            while let Ok(Some((p, used))) = codec::decode(&buf, 1 << 20) {
                buf.drain(..used);
                self.state.lock().unwrap().received.push((conn, p.clone()));
                match p {
                    Packet::Connect(c) => {
                        let code = {
                            let mut s = self.state.lock().unwrap();
                            s.wills[conn] = c.will.clone();
                            s.connack_code
                        };
                        self.send(
                            conn,
                            &Packet::Connack {
                                session_present: false,
                                return_code: code,
                            },
                        );
                        if code != 0 {
                            break 'conn true;
                        }
                    }
                    Packet::Publish(p) => {
                        if p.qos == QoS::AtLeastOnce {
                            self.send(
                                conn,
                                &Packet::Puback {
                                    packet_id: p.packet_id,
                                },
                            );
                        }
                        if p.retain {
                            self.state
                                .lock()
                                .unwrap()
                                .retained_log
                                .push((p.topic.clone(), p.payload.clone()));
                        }
                    }
                    Packet::Pingreq => self.send(conn, &Packet::Pingresp),
                    Packet::Disconnect => break 'conn true,
                    _ => {}
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break false,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let mut s = self.state.lock().unwrap();
        if let Some(Some(w)) = s.conns.get_mut(conn) {
            let _ = w.shutdown(Shutdown::Both);
        }
        s.conns[conn] = None;
        // An unclean end: the broker publishes the will.
        let will = s.wills[conn].take();
        if !clean
            && let Some(w) = will
            && w.retain
        {
            s.retained_log.push((w.topic, w.payload));
        }
    }

    /// Cut every connection, as a lost network does.
    fn cut_all(&self) {
        let s = self.state.lock().unwrap();
        for w in s.conns.iter().flatten() {
            let _ = w.shutdown(Shutdown::Both);
        }
    }

    fn retained(&self, topic: &str) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .retained_log
            .iter()
            .filter(|(t, _)| t == topic)
            .map(|(_, p)| String::from_utf8_lossy(p).into_owned())
            .collect()
    }

    fn connects(&self) -> Vec<codec::Connect> {
        self.state
            .lock()
            .unwrap()
            .received
            .iter()
            .filter_map(|(_, p)| match p {
                Packet::Connect(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    fn published(&self, topic: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .received
            .iter()
            .filter_map(|(_, p)| match p {
                Packet::Publish(p) if p.topic == topic => serde_json::from_slice(&p.payload).ok(),
                _ => None,
            })
            .collect()
    }

    fn disconnects(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .received
            .iter()
            .filter(|(_, p)| matches!(p, Packet::Disconnect))
            .count()
    }
}

// ── The hub and the socket ──────────────────────────────────────────────────

type Ws = tungstenite::WebSocket<TcpStream>;

fn ws(port: u16) -> (Ws, Value) {
    let url = format!("ws://127.0.0.1:{port}/dspi/v1");
    let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
    let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let mut ws = tungstenite::client(req, tcp).unwrap().0;
    ws.send(Message::text(HELLO)).unwrap();
    let hello = next(&mut ws, |v| v["t"] == "hello");
    (ws, hello)
}

fn next(ws: &mut Ws, want: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "nothing wanted in time");
        if let Message::Text(text) = ws.read().unwrap() {
            let v: Value = serde_json::from_str(text.as_str()).unwrap();
            if want(&v) {
                return v;
            }
        }
    }
}

/// The answer to `msg`, and its raw text.
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

/// A socket paired with a window PIN as `role`, made admin on the hub's side when asked
/// (as `uconsole hub role <cid> admin` does).
fn paired(net: &NetworkControl, role: Role) -> Ws {
    let (mut ws, hello) = ws(net.port());
    assert!(
        hello["caps"].as_array().unwrap().contains(&json!("mqtt")),
        "{hello}"
    );
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    let wire = if role == Role::Viewer {
        "viewer"
    } else {
        "control"
    };
    let ok = ask(
        &mut ws,
        json!({"t": "auth.pair", "pin": pin, "name": format!("p2mq {wire}"), "role": wire}),
    );
    assert_eq!(ok["t"], "ok", "{ok}");
    if role == Role::Admin {
        let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
        assert!(net.set_role(cid, Role::Admin));
        wait_until("the admin role", || {
            ask(&mut ws, json!({"t": "mqtt.status"}))["t"] == "ok"
        });
    }
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
        ..HubConfig::in_process("P2MQ hub")
    })
    .unwrap()
}

fn status(ws: &mut Ws) -> Value {
    let s = ask(ws, json!({"t": "mqtt.status"}));
    assert_eq!(s["t"], "ok", "{s}");
    assert_eq!(s["v"], 1);
    assert_eq!(s["encrypted"], false, "a plain broker: {s}");
    s
}

fn wait_state(ws: &mut Ws, state: &str) -> Value {
    let start = Instant::now();
    loop {
        let s = status(ws);
        if s["state"] == state {
            return s;
        }
        assert!(start.elapsed() < WAIT, "never {state}: {s}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn raised(serial: &str, at: u64) -> BusMessage {
    BusMessage::Alarm {
        id: "device:1".into(),
        source: "device".into(),
        code: "temperature".into(),
        subject: serial.into(),
        serial: Some(serial.into()),
        severity: "critical".into(),
        state: AlarmState::Raised,
        message: "72 °C".into(),
        at,
        raised_at: at,
        cleared_at: None,
        acknowledged_by: None,
        count: 1,
    }
}

// ── The tests ───────────────────────────────────────────────────────────────

#[test]
fn wp_p2mq_publishes_an_alarm_says_online_and_leaves_a_will() {
    let dir = scratch("alarm");
    let broker = Broker::start();
    let hub = hub_on(&dir);
    let net = hub.network().unwrap();
    let mut admin = paired(&net, Role::Admin);
    let hub_id = hub.handle().hub_id().to_owned();

    // Off by default, with the documented defaults.
    let s = status(&mut admin);
    assert_eq!(s["state"], "off", "{s}");
    let get = ask(&mut admin, json!({"t": "mqtt.get"}));
    assert_eq!(get["settings"]["enabled"], false, "{get}");
    assert_eq!(get["settings"]["port"], 1883);
    assert_eq!(get["settings"]["password"], "not set");
    assert_eq!(get["settings"]["base_topic"], format!("dspi/{hub_id}"));
    assert!(!dir.join("mqtt.json").exists());
    assert!(broker.connects().is_empty());

    let (set, set_text) = ask_text(
        &mut admin,
        json!({"t": "mqtt.set", "settings": {"enabled": true, "broker": "127.0.0.1",
            "port": broker.port(), "username": "hub", "password": PASSWORD,
            "base_topic": "p2mq/a", "client_id": "p2mq-a"}}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    assert_eq!(set["settings"]["password"], "set");
    assert!(
        !set_text.contains(PASSWORD),
        "the password is never answered"
    );
    let file = std::fs::read_to_string(dir.join("mqtt.json")).unwrap();
    assert!(file.contains(PASSWORD), "kept in the private file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("mqtt.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    wait_state(&mut admin, "connected");
    let c = broker.connects().pop().unwrap();
    assert_eq!(c.client_id, "p2mq-a");
    assert_eq!(c.username.as_deref(), Some("hub"));
    assert_eq!(c.password.as_deref(), Some(PASSWORD.as_bytes()));
    let will = c.will.unwrap();
    assert_eq!(will.topic, "p2mq/a/status");
    assert_eq!(will.payload, b"offline");
    assert!(will.retain);
    wait_until("online", || {
        broker.retained("p2mq/a/status") == vec!["online".to_owned()]
    });

    // An alarm, as P2AL puts it on the bus, and a history record, as P2EV does.
    let h = hub.handle();
    h.bus().publish(raised("P2MQ-1", 1_763_200_000));
    let record: EventRecord = serde_json::from_value(json!({
        "kind": "preset_loaded", "text": "Preset 2 loaded", "serial": "P2MQ-1", "by": "FOH"
    }))
    .unwrap_or_else(|_| {
        serde_json::from_value(json!({"text": "Preset 2 loaded", "serial": "P2MQ-1"})).unwrap()
    });
    h.bus().publish(BusMessage::Event {
        serial: "P2MQ-1".into(),
        record,
    });
    wait_until("the alarm and the record", || {
        broker.published("p2mq/a/P2MQ-1/event").len() == 2
    });
    let got = broker.published("p2mq/a/P2MQ-1/event");
    let alarm = got.iter().find(|v| v["kind"] == "alarm").unwrap();
    assert_eq!(alarm["code"], "temperature");
    assert_eq!(alarm["state"], "raised");
    assert_eq!(alarm["serial"], "P2MQ-1");
    assert_eq!(alarm["hub"], hub_id.as_str());
    assert_eq!(alarm["raised_at"], "2025-11-15T09:46:40Z");
    let event = got.iter().find(|v| v["kind"] == "event").unwrap();
    assert_eq!(event["record"]["text"], "Preset 2 loaded");
    // Something not for MQTT is not published.
    h.bus().publish(BusMessage::DeviceOnline {
        serial: "P2MQ-1".into(),
        handle: 1,
    });

    // A lost connection: the broker publishes the will; the hub comes back online.
    broker.cut_all();
    wait_until("offline, then online again", || {
        broker.retained("p2mq/a/status") == ["online", "offline", "online"]
    });
    wait_state(&mut admin, "connected");
    assert!(broker.connects().len() >= 2);

    // Changing only `ha` does not reconnect.
    let before = broker.connects().len();
    let ha = ask(
        &mut admin,
        json!({"t": "mqtt.set", "settings": {"ha": {"enabled": true}}}),
    );
    assert_eq!(ha["settings"]["ha"]["enabled"], true, "{ha}");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(broker.connects().len(), before);

    // A clean stop: `offline`, retained, and a DISCONNECT.
    drop(admin);
    hub.stop();
    wait_until("offline on stop", || {
        broker.retained("p2mq/a/status").last().map(String::as_str) == Some("offline")
    });
    assert_eq!(broker.disconnects(), 1);
    assert!(
        !std::fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".bad-")),
        "nothing set aside"
    );
}

#[test]
fn wp_p2mq_a_wrong_password_is_reported_and_off_stops_trying() {
    let dir = scratch("password");
    let broker = Broker::start();
    broker.state.lock().unwrap().connack_code = 4;
    let hub = hub_on(&dir);
    let net = hub.network().unwrap();
    let mut admin = paired(&net, Role::Admin);
    let set = ask(
        &mut admin,
        json!({"t": "mqtt.set", "settings": {"enabled": true, "broker": "127.0.0.1",
            "port": broker.port(), "username": "hub", "password": PASSWORD}}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    let (s, text) = {
        let s = wait_state(&mut admin, "failed");
        let (_, text) = ask_text(&mut admin, json!({"t": "mqtt.status"}));
        (s, text)
    };
    assert!(
        s["msg"]
            .as_str()
            .unwrap()
            .contains("bad user name or password"),
        "{s}"
    );
    assert!(s["since"].as_str().unwrap().ends_with('Z'), "{s}");
    assert!(!text.contains(PASSWORD), "{text}");
    let get = ask(&mut admin, json!({"t": "mqtt.get"}));
    assert!(!get.to_string().contains(PASSWORD));

    // Turned off: `off`, and no more attempts.
    let off = ask(
        &mut admin,
        json!({"t": "mqtt.set", "settings": {"enabled": false}}),
    );
    assert_eq!(off["t"], "ok", "{off}");
    assert_eq!(status(&mut admin)["state"], "off");
    let tries = broker.connects().len();
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(broker.connects().len(), tries, "no attempt while off");

    // A bad setting is refused whole, naming the field.
    let bad = ask(
        &mut admin,
        json!({"t": "mqtt.set", "settings": {"port": 0, "broker": "other"}}),
    );
    assert_eq!(bad["code"], "bad_request", "{bad}");
    assert!(bad["msg"].as_str().unwrap().contains("port"), "{bad}");
    let get = ask(&mut admin, json!({"t": "mqtt.get"}));
    assert_eq!(get["settings"]["broker"], "127.0.0.1", "unchanged: {get}");
    drop(admin);
    hub.stop();
}

#[test]
fn wp_p2mq_a_saved_setting_connects_on_the_next_start() {
    let dir = scratch("restart");
    let broker = Broker::start();
    {
        let hub = hub_on(&dir);
        let mut admin = paired(&hub.network().unwrap(), Role::Admin);
        let set = ask(
            &mut admin,
            json!({"t": "mqtt.set", "settings": {"enabled": true, "broker": "127.0.0.1",
                "port": broker.port(), "base_topic": "p2mq/r"}}),
        );
        assert_eq!(set["t"], "ok", "{set}");
        wait_state(&mut admin, "connected");
        drop(admin);
        hub.stop();
    }
    let first = broker.connects().len();
    // SECOUT's `tls`, as set by hand: kept on the next rewrite (WP MQWIRE).
    let mut file: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("mqtt.json")).unwrap()).unwrap();
    file["tls"] = json!({"mode": "off", "allow_plaintext_password": true});
    std::fs::write(dir.join("mqtt.json"), file.to_string()).unwrap();

    let hub = hub_on(&dir);
    wait_until("a connection with nobody asking", || {
        broker.connects().len() > first
    });
    wait_until("online", || {
        broker.retained("p2mq/r/status").last().map(String::as_str) == Some("online")
    });
    let mut admin = paired(&hub.network().unwrap(), Role::Admin);
    let get = ask(&mut admin, json!({"t": "mqtt.get"}));
    assert_eq!(
        get["settings"]["tls"]["allow_plaintext_password"], true,
        "{get}"
    );
    assert!(get.get("notice").is_none(), "{get}");
    let set = ask(
        &mut admin,
        json!({"t": "mqtt.set", "settings": {"ha": {"enabled": false}}}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("mqtt.json")).unwrap()).unwrap();
    assert_eq!(file["tls"]["mode"], "off", "tls kept on rewrite");
    assert_eq!(
        file["tls"]["allow_plaintext_password"], true,
        "tls kept on rewrite"
    );
    drop(admin);
    hub.stop();
}

#[test]
fn wp_p2mq_control_and_viewer_are_denied() {
    let dir = scratch("roles");
    let hub = hub_on(&dir);
    let net = hub.network().unwrap();
    for role in [Role::Control, Role::Viewer] {
        let mut ws = paired(&net, role);
        for msg in [
            json!({"t": "mqtt.get"}),
            json!({"t": "mqtt.status"}),
            json!({"t": "mqtt.set", "settings": {"enabled": true, "broker": "10.0.0.9"}}),
        ] {
            let a = ask(&mut ws, msg.clone());
            assert_eq!(a["code"], "denied", "{role:?} {msg}: {a}");
        }
    }
    assert!(!dir.join("mqtt.json").exists(), "nothing kept");
    hub.stop();
}
