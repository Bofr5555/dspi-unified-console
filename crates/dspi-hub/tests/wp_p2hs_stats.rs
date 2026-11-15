//! WP P2HS: `hub.stats` and `hub.rename` (spec 7.9, hub-api.md section 15) through the
//! real hub and the real server.
//!
//! The counts rise with commands, the errors with a failing transfer, an average round
//! trip is reported, and a session that never reads shows dropped notifications.
//! `hub.rename` is in use at once (a new `hello`, `/dspi/v1/info`), kept in the store
//! and announced again over mDNS (a browser kept to loopback, as HB5's). Control and
//! Viewer are `denied` both.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_hub::server::{Advertise, NetworkControl};
use dspi_hub::{AccessMode, Completion, Hub, HubConfig, NetworkConfig, Role, SavedHub, Session};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, NotificationSource, Result as TResult, Transport, TransportCaps,
};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
const GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;
const HELLO: &str = r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"p2hs","app":"test","version":"0"}}"#;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2hs").join(format!(
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
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The demo device, each control transfer taking at least 2 ms once `slow` is set: a
/// round trip a mean in ms can show.
struct Slow {
    inner: FakeDevice,
    slow: Arc<AtomicBool>,
}

impl Slow {
    fn pause(&self) {
        if self.slow.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Transport for Slow {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.pause();
        self.inner.control_in(opcode, value, len)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        self.pause();
        self.inner.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        self.pause();
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
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

// ── Over the socket ─────────────────────────────────────────────────────────

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

fn ask(ws: &mut Ws, mut msg: Value) -> Value {
    static ID: AtomicU32 = AtomicU32::new(100);
    let id = ID.fetch_add(1, Ordering::SeqCst);
    msg["id"] = id.into();
    ws.send(Message::text(msg.to_string())).unwrap();
    next(ws, |v| v["id"] == id)
}

/// A socket paired with a window PIN as `role` (a PIN gives at most Control), made
/// Admin on the hub's side when asked, as `uconsole hub role <cid> admin` does.
fn paired(net: &NetworkControl, role: Role) -> Ws {
    let (mut ws, _) = ws(net.port());
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    let wire = if role == Role::Viewer {
        "viewer"
    } else {
        "control"
    };
    let ok = ask(
        &mut ws,
        json!({"t": "auth.pair", "pin": pin, "name": format!("p2hs {wire}"), "role": wire}),
    );
    assert_eq!(ok["t"], "ok", "{ok}");
    assert_eq!(ok["role"], wire, "{ok}");
    if role == Role::Admin {
        let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
        assert!(net.set_role(cid, Role::Admin));
        wait_until("the admin role", || {
            ask(&mut ws, json!({"t": "hub.stats"}))["t"] == "ok"
        });
    }
    ws
}

fn network(dir: PathBuf, advertise: Advertise) -> NetworkConfig {
    NetworkConfig {
        port: 0,
        access: AccessMode::Pin,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        advertise,
        ..NetworkConfig::new(dir)
    }
}

fn hub_on(dir: PathBuf, name: &str, hub_id: &str, advertise: Advertise) -> Hub {
    Hub::start(HubConfig {
        hub_id: hub_id.to_owned(),
        network: Some(network(dir, advertise)),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        ..HubConfig::in_process(name)
    })
    .unwrap()
}

fn submit(hub: &Hub, session: &Session, req: CmdRequest) -> CmdResponse {
    let (tx, rx) = mpsc::channel();
    let tag = req.tag;
    hub.submit(
        session,
        req,
        Completion::new(tag, move |r| {
            let _ = tx.send(r);
        }),
    );
    rx.recv_timeout(WAIT).unwrap()
}

fn set_volume(handle: u8, db: f32) -> CmdRequest {
    CmdRequest {
        tag: 1,
        handle,
        dir: Dir::Set,
        b_request: op::REQ_SET_MASTER_VOLUME,
        w_value: 0,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: 4,
        payload: db.to_le_bytes().to_vec(),
    }
}

fn get_volume(handle: u8) -> CmdRequest {
    CmdRequest {
        tag: 2,
        handle,
        dir: Dir::Get,
        b_request: op::REQ_GET_MASTER_VOLUME,
        w_value: 0,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: 4,
        payload: Vec::new(),
    }
}

/// The device row of `stats` for `serial`.
fn row<'a>(stats: &'a Value, serial: &str) -> &'a Value {
    stats["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["serial"] == serial)
        .unwrap_or_else(|| panic!("no row for {serial}: {stats}"))
}

fn count(row: &Value, field: &str) -> u64 {
    row[field]
        .as_u64()
        .unwrap_or_else(|| panic!("{field}: {row}"))
}

#[test]
fn wp_p2hs_stats_count_commands_errors_round_trips_and_drops() {
    let hub = hub_on(scratch("stats"), "P2HS hub", "", Advertise::Off);
    let slow = Arc::new(AtomicBool::new(false));
    let device = FakeDevice::demo().with_serial("P2HS-A");
    let remote = device.remote();
    let handle = hub
        .attach(
            Box::new(Slow {
                inner: device,
                slow: Arc::clone(&slow),
            }),
            0,
        )
        .unwrap();
    let net = hub.network().unwrap();
    let mut admin = paired(&net, Role::Admin);

    let first = ask(&mut admin, json!({"t": "hub.stats"}));
    assert_eq!(first["t"], "ok", "{first}");
    assert_eq!(first["v"], 1, "{first}");
    assert_eq!(first["sessions"], 1, "the admin's own: {first}");
    assert!(first["uptime_s"].is_u64(), "{first}");
    let before = row(&first, "P2HS-A").clone();
    assert_eq!(before["handle"], handle);
    for field in ["cmds", "errors", "notify_dropped"] {
        count(&before, field);
    }
    assert!(before["avg_rtt_ms"].is_number(), "{before}");

    // Commands: the count rises by each one, and a round trip is reported.
    slow.store(true, Ordering::SeqCst);
    let client = hub.open_session(Role::Control, None).unwrap();
    for i in 0..4 {
        let r = submit(&hub, &client, set_volume(handle, -10.0 - i as f32));
        assert_eq!(r.status, status::OK);
    }
    assert_eq!(submit(&hub, &client, get_volume(handle)).status, status::OK);
    let stats = ask(&mut admin, json!({"t": "hub.stats"}));
    let now = row(&stats, "P2HS-A");
    assert_eq!(count(now, "cmds"), count(&before, "cmds") + 5, "{stats}");
    assert_eq!(count(now, "errors"), count(&before, "errors"), "{stats}");
    let rtt = now["avg_rtt_ms"].as_f64().unwrap();
    assert!(rtt > 0.0, "a 2 ms transfer shows: {stats}");
    assert_eq!(
        stats["sessions"], 2,
        "the admin and the in-process client: {stats}"
    );
    slow.store(false, Ordering::SeqCst);

    // A failing transfer is an error and still a command.
    remote.stall(op::REQ_SET_MASTER_VOLUME);
    assert_eq!(
        submit(&hub, &client, set_volume(handle, -30.0)).status,
        status::ERROR
    );
    let stats = ask(&mut admin, json!({"t": "hub.stats"}));
    let failed = row(&stats, "P2HS-A");
    assert_eq!(count(failed, "cmds"), count(&before, "cmds") + 6, "{stats}");
    assert_eq!(
        count(failed, "errors"),
        count(&before, "errors") + 1,
        "{stats}"
    );
    // A command refused before the device is neither.
    let viewer = hub.open_session(Role::Viewer, None).unwrap();
    assert_eq!(
        submit(&hub, &viewer, set_volume(handle, -1.0)).status,
        status::DENIED
    );
    let stats = ask(&mut admin, json!({"t": "hub.stats"}));
    assert_eq!(count(row(&stats, "P2HS-A"), "cmds"), count(failed, "cmds"));

    // A session that never reads its outbox: past its queue, notifications drop.
    assert_eq!(count(row(&stats, "P2HS-A"), "notify_dropped"), 0, "{stats}");
    let unread = hub.open_session(Role::Viewer, None).unwrap();
    // The master volume's place in the image: the last set that landed.
    let last = (-13.0f32).to_le_bytes();
    let at = remote.image().windows(4).position(|w| w == last).unwrap();
    for i in 0..400 {
        let v = if i % 2 == 0 { -3.0f32 } else { -4.0f32 };
        assert!(remote.poke(at, &v.to_le_bytes(), GPIO));
        // Let the reader take each one, so the device's queue never merges them.
        if i % 50 == 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    wait_until("dropped notifications", || {
        let stats = ask(&mut admin, json!({"t": "hub.stats"}));
        count(row(&stats, "P2HS-A"), "notify_dropped") > 0
    });
    drop(unread);
    drop(admin);
    hub.stop();
}

#[test]
fn wp_p2hs_rename_saves_and_shows_in_hello_and_info() {
    let dir = scratch("rename");
    let hub = hub_on(dir.clone(), "P2HS before", "", Advertise::Off);
    let net = hub.network().unwrap();
    let port = net.port();
    let (_, hello) = ws(port);
    let caps = hello["caps"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "hub_stats"), "{hello}");
    assert_eq!(hello["hub"]["name"], "P2HS before");

    let mut admin = paired(&net, Role::Admin);
    let ok = ask(
        &mut admin,
        json!({"t": "hub.rename", "name": "  Studio\nMac  "}),
    );
    assert_eq!(ok["t"], "ok", "{ok}");
    assert_eq!(ok["v"], 1, "{ok}");
    assert_eq!(
        ok["name"], "StudioMac",
        "control characters dropped, trimmed: {ok}"
    );
    let ok = ask(
        &mut admin,
        json!({"t": "hub.rename", "name": "Studio Mac ✓"}),
    );
    assert_eq!(ok["name"], "Studio Mac ✓", "{ok}");

    // A new `hello`, `/dspi/v1/info` and the handle carry it.
    let (_, hello) = ws(port);
    assert_eq!(hello["hub"]["name"], "Studio Mac ✓", "{hello}");
    assert_eq!(hub.name(), "Studio Mac ✓");
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    write!(
        tcp,
        "GET /dspi/v1/info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    let _ = tcp.read_to_string(&mut answer);
    let body = &answer[answer.find("\r\n\r\n").unwrap() + 4..];
    let info: Value = serde_json::from_str(body).unwrap();
    assert_eq!(info["hub"]["name"], "Studio Mac ✓", "{info}");

    // Saved for the next start.
    let saved = SavedHub::peek(&dir).unwrap().unwrap();
    assert_eq!(saved.name.as_deref(), Some("Studio Mac ✓"));

    // Refused names change nothing.
    for bad in [json!(""), json!(" \t "), json!(5), json!("é".repeat(65))] {
        let refused = ask(&mut admin, json!({"t": "hub.rename", "name": bad}));
        assert_eq!(refused["code"], "bad_request", "{bad}: {refused}");
    }
    let refused = ask(&mut admin, json!({"t": "hub.rename"}));
    assert_eq!(refused["code"], "bad_request", "{refused}");
    assert_eq!(hub.name(), "Studio Mac ✓");
    drop(admin);
    hub.stop();
}

#[test]
fn wp_p2hs_control_and_viewer_are_denied() {
    let dir = scratch("roles");
    let hub = hub_on(dir.clone(), "P2HS roles", "", Advertise::Off);
    hub.attach(Box::new(FakeDevice::demo().with_serial("P2HS-R")), 0)
        .unwrap();
    let net = hub.network().unwrap();
    for role in [Role::Control, Role::Viewer] {
        let mut ws = paired(&net, role);
        let stats = ask(&mut ws, json!({"t": "hub.stats"}));
        assert_eq!(stats["code"], "denied", "{role:?}: {stats}");
        let rename = ask(&mut ws, json!({"t": "hub.rename", "name": "Nope"}));
        assert_eq!(rename["code"], "denied", "{role:?}: {rename}");
    }
    assert_eq!(hub.name(), "P2HS roles");
    assert_ne!(
        SavedHub::peek(&dir)
            .unwrap()
            .and_then(|s| s.name)
            .as_deref(),
        Some("Nope")
    );
    hub.stop();
}

/// With the service off (`ServiceConfig::stats`), both stay `unsupported` and the cap
/// is not announced.
#[test]
fn wp_p2hs_off_answers_unsupported() {
    let hub = Hub::start(HubConfig {
        network: Some(network(scratch("off"), Advertise::Off)),
        initial_scan: false,
        services: dspi_hub::ServiceConfig {
            stats: false,
            ..dspi_hub::ServiceConfig::default()
        },
        ..HubConfig::in_process("P2HS off")
    })
    .unwrap();
    let net = hub.network().unwrap();
    let (_, hello) = ws(net.port());
    assert!(
        !hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "hub_stats"),
        "{hello}"
    );
    let mut ws = paired(&net, Role::Control);
    let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
    assert!(net.set_role(cid, Role::Admin));
    wait_until("the admin role", || {
        ask(&mut ws, json!({"t": "hub.stats"}))["code"] == "unsupported"
    });
    let rename = ask(&mut ws, json!({"t": "hub.rename", "name": "x"}));
    assert_eq!(rename["code"], "unsupported", "{rename}");
    hub.stop();
}

/// A renamed hub is announced again under its new name (the HB5 browser, on loopback).
#[cfg(all(feature = "mdns", not(windows)))]
mod mdns_wp_p2hs {
    use std::collections::BTreeMap;

    use dspi_link::discovery::{SERVICE_TYPE, ServiceRecord, parse_service};
    use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent};

    use super::*;

    const ID: &str = "e2a50001-0000-4000-8000-000000000000";

    /// The names the browser has resolved for the hub `ID`, now.
    fn names(heard: &BTreeMap<String, ServiceRecord>) -> Vec<String> {
        heard
            .values()
            .filter_map(|r| parse_service(r).ok())
            .filter(|h| h.id == ID)
            .map(|h| h.name)
            .collect()
    }

    #[test]
    fn wp_p2hs_rename_is_announced_again_over_mdns() {
        let daemon = ServiceDaemon::new().unwrap();
        daemon.disable_interface(IfKind::All).unwrap();
        daemon.enable_interface(IfKind::LoopbackV4).unwrap();
        let events = daemon.browse(SERVICE_TYPE).unwrap();
        let mut heard: BTreeMap<String, ServiceRecord> = BTreeMap::new();
        let mut until = |what: &str, ok: &dyn Fn(&BTreeMap<String, ServiceRecord>) -> bool| {
            let deadline = Instant::now() + WAIT;
            while !ok(&heard) {
                assert!(Instant::now() < deadline, "never heard {what}: {heard:?}");
                let Ok(event) = events.recv_timeout(Duration::from_millis(100)) else {
                    continue;
                };
                match event {
                    ServiceEvent::ServiceResolved(s) => {
                        let record = ServiceRecord {
                            instance: s.get_fullname().to_owned(),
                            port: s.get_port(),
                            addresses: s
                                .get_addresses()
                                .iter()
                                .map(mdns_sd::ScopedIp::to_ip_addr)
                                .collect(),
                            txt: s
                                .get_properties()
                                .iter()
                                .map(|p| {
                                    let v = String::from_utf8_lossy(p.val().unwrap_or_default());
                                    (p.key().to_owned(), v.into_owned())
                                })
                                .collect(),
                            ..ServiceRecord::default()
                        };
                        heard.insert(record.instance.clone(), record);
                    }
                    ServiceEvent::ServiceRemoved(_, instance) => {
                        heard.remove(&instance);
                    }
                    _ => {}
                }
            }
        };

        let hub = hub_on(
            scratch("mdns"),
            "P2HS mdns before",
            ID,
            Advertise::LoopbackOnly,
        );
        until("the hub", &|h| names(h) == ["P2HS mdns before"]);
        let net = hub.network().unwrap();
        let mut admin = paired(&net, Role::Admin);
        let ok = ask(
            &mut admin,
            json!({"t": "hub.rename", "name": "P2HS mdns after"}),
        );
        assert_eq!(ok["t"], "ok", "{ok}");
        until("the new name", &|h| names(h) == ["P2HS mdns after"]);
        drop(admin);
        hub.stop();
        if let Ok(done) = daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_secs(5));
        }
    }
}
