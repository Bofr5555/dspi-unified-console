//! WP P2RS's done criteria, through the real server on the demo device: the REST mirror
//! under `/api/v1/` (each endpoint onto its WebSocket message), API keys (`auth.mint`,
//! `kind` in `auth.list`, the shared cap of 64), and the hostile-LAN cases of the body
//! read (a trickled PUT, a wrong or missing `Content-Length`, a body longer than its
//! length), each with a hand-written HTTP client on a raw socket.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_hub::server::ServerLimits;
use dspi_hub::session::{
    FIRST_REMOTE_SESSION_ID, GUARD_SESSION_ID, INTERNAL_SESSION_ID, LOCAL_SESSION_ID,
};
use dspi_hub::{
    AccessMode, BusMessage, ClientKind, HealthConfig, HistoryConfig, Hub, HubConfig, HubEvent,
    HubHandle, HubListener, HubService, NetworkConfig, Role, ServiceConfig, Session,
};
use dspi_session::snapshot_store::SnapshotStore;
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the work volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2rs").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `alarm.*` as P2AL will answer them, which is not merged yet: what the mirror sent,
/// and on which session.
#[derive(Default)]
struct FakeAlarms {
    seen: Mutex<Vec<(u16, Value)>>,
}

impl HubService for FakeAlarms {
    fn name(&self) -> &'static str {
        "wp-p2rs-fake-alarms"
    }

    fn handle_message(
        &self,
        _hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let t = msg["t"].as_str()?;
        if !t.starts_with("alarm.") {
            return None;
        }
        self.seen.lock().unwrap().push((session.id(), msg.clone()));
        let id = msg["id"].clone();
        Some(
            match t {
                "alarm.list" => {
                    json!({"t": "ok", "id": id, "v": 1, "alarms": [{"id": "device:1"}]})
                }
                "alarm.ack" if session.role() == Role::Viewer => {
                    json!({"t": "err", "id": id, "v": 1, "code": "denied"})
                }
                "alarm.ack" => json!({"t": "ok", "id": id, "v": 1, "alarm": {"id": msg["alarm"]}}),
                _ => json!({"t": "err", "id": id, "code": "unknown_type"}),
            }
            .to_string(),
        )
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<HubEvent>>);

impl HubListener for Events {
    fn on_event(&self, event: HubEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct Rig {
    hub: Hub,
    dir: PathBuf,
    port: u16,
    alarms: Arc<FakeAlarms>,
    events: Arc<Events>,
}

fn quick() -> ServerLimits {
    ServerLimits {
        close_grace: Duration::from_millis(200),
        read_tick: Duration::from_millis(20),
        ..ServerLimits::default()
    }
}

fn rig_with(tag: &str, rest: bool, limits: ServerLimits) -> Rig {
    let dir = scratch(tag);
    let alarms = Arc::new(FakeAlarms::default());
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            rest,
            limits,
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            inventory: true,
            health: Some(HealthConfig::default()),
            history: Some(HistoryConfig::default()),
            extra: vec![Arc::clone(&alarms) as Arc<dyn HubService>],
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p2rs")
    })
    .unwrap();
    let events = Arc::new(Events::default());
    hub.set_listener(Some(Arc::clone(&events) as Arc<dyn HubListener>));
    let port = hub.network().unwrap().port();
    Rig {
        hub,
        dir,
        port,
        alarms,
        events,
    }
}

fn rig(tag: &str) -> Rig {
    rig_with(tag, true, quick())
}

impl Rig {
    fn plug(&self, serial: &str) -> u8 {
        self.hub
            .attach(Box::new(FakeDevice::demo().with_serial(serial)), 0)
            .unwrap()
    }

    /// A key minted on the hub's machine.
    fn key(&self, name: &str, role: Role) -> String {
        self.hub
            .network()
            .unwrap()
            .auth()
            .mint_api_key(name, role, false)
            .unwrap()
            .key
    }

    /// A paired console with `role`, logged in over the socket.
    fn client(&self, name: &str, role: Role) -> Ws {
        let net = self.hub.network().unwrap();
        let window = net.auth().begin_pairing(Instant::now()).unwrap();
        let authed = net
            .auth()
            .pair(
                &window.pin,
                name,
                Some(role.min(Role::Control)),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                Instant::now(),
            )
            .unwrap();
        if role == Role::Admin {
            assert!(net.auth().set_role(authed.cid, Role::Admin));
        }
        let mut ws = Ws::connect(net.port());
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": name, "app": "wp_p2rs", "version": "0"}}));
        let hello = ws.next("hello");
        ws.caps = hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        let ok = ws.ask(json!({"t": "auth.token", "token": authed.token.unwrap()}));
        assert_eq!(ok["t"], "ok", "{ok}");
        ws
    }

    fn closed(&self, id: u16) -> bool {
        self.events
            .0
            .lock()
            .unwrap()
            .contains(&HubEvent::SessionClosed { id })
    }

    /// The device's snapshot as a client files it, put into the hub's store: its id.
    fn put_snapshot(&self, ws: &mut Ws, serial: &str, name: &str) -> String {
        let core = DspiCore::new();
        let t = self.hub.handle().open_local(serial).unwrap();
        core.open_transport(Box::new(t), serial.into(), 0).unwrap();
        let _ = core.feature_present("device_name");
        let text = core
            .snapshot_capture_document(name.into(), String::new())
            .unwrap();
        let store = SnapshotStore::new(self.dir.join(format!("client-{name}")));
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        let e = store.save(serial, &mut doc, 1_763_200_000_000).unwrap();
        let (_, filed) = store.read(&e.id).unwrap();
        let compact: Value = serde_json::from_str(&filed).unwrap();
        let compact = compact.to_string();
        if compact.len() < 60_000 {
            let put = ws.ask(json!({"t": "snapshot.put", "serial": serial, "document": compact}));
            assert_eq!(put["t"], "ok", "{put}");
        } else {
            dspi_hub::snapshots::HubSnapshots::open(&self.dir.join("store"))
                .put(serial, &filed)
                .unwrap();
        }
        core.disconnect();
        e.id
    }
}

struct Ws {
    ws: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    caps: Vec<String>,
    seen: Vec<Value>,
}

impl Ws {
    fn connect(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        Self {
            ws: tungstenite::client(req, tcp).unwrap().0,
            next_id: 100,
            caps: Vec::new(),
            seen: Vec::new(),
        }
    }

    fn send(&mut self, v: Value) {
        self.ws.send(Message::text(v.to_string())).unwrap();
    }

    fn read(&mut self) -> Value {
        loop {
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    fn next(&mut self, t: &str) -> Value {
        if let Some(i) = self.seen.iter().position(|v| v["t"] == t) {
            return self.seen.remove(i);
        }
        loop {
            let v = self.read();
            if v["t"] == t {
                return v;
            }
            self.seen.push(v);
        }
    }

    fn ask(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        msg["id"] = id.into();
        self.send(msg);
        loop {
            let v = self.read();
            if v["id"] == id {
                return v;
            }
            self.seen.push(v);
        }
    }
}

/// One whole HTTP answer: status, head, body.
struct Answer {
    status: u16,
    head: String,
    body: Value,
}

impl Answer {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_owned())
        })
    }
}

fn raw(port: u16) -> TcpStream {
    let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    tcp
}

/// Everything up to the close, as an answer.
fn read_all(tcp: &mut TcpStream) -> Answer {
    let mut bytes = Vec::new();
    let _ = tcp.read_to_end(&mut bytes);
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Answer {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Answer {
        status,
        head: head.to_owned(),
        body: serde_json::from_str(body).unwrap_or(Value::String(body.to_owned())),
    }
}

/// A request on a new connection, `Connection: close`, and its answer.
fn call(port: u16, method: &str, path: &str, key: Option<&str>, body: Option<&str>) -> Answer {
    let mut head =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    if let Some(key) = key {
        head.push_str(&format!("Authorization: Bearer {key}\r\n"));
    }
    if let Some(body) = body {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    head.push_str("\r\n");
    head.push_str(body.unwrap_or(""));
    let mut tcp = raw(port);
    tcp.write_all(head.as_bytes()).unwrap();
    read_all(&mut tcp)
}

fn get(port: u16, path: &str, key: &str) -> Answer {
    call(port, "GET", path, Some(key), None)
}

#[test]
fn wp_p2rs_every_endpoint_answers_through_its_message() {
    let r = rig("endpoints");
    let serial = "P2RS-ENDPOINTS";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    assert!(admin.caps.iter().any(|c| c == "rest"), "{:?}", admin.caps);
    assert!(
        admin.caps.iter().any(|c| c == "api_keys"),
        "{:?}",
        admin.caps
    );
    let key = r.key("Venue controller", Role::Control);
    let hub = r.hub.hub_id().to_owned();

    // GET /devices: inventory.list
    let a = get(r.port, "/api/v1/devices", &key);
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(
        (a.body["v"].clone(), a.body["hub"].clone()),
        (json!(1), json!(hub))
    );
    assert!(
        a.body.get("t").is_none() && a.body.get("id").is_none(),
        "{}",
        a.body
    );
    assert!(a.body.to_string().contains(serial), "{}", a.body);
    assert_eq!(
        a.header("Content-Type").as_deref(),
        Some("application/json")
    );

    // GET and PUT /params/{path}: param.get and param.set, with idx.
    let p = format!("/api/v1/devices/{serial}/params/vol.master");
    let a = call(r.port, "PUT", &p, Some(&key), Some(r#"{"value": -21.5}"#));
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.body["value"], -21.5, "{}", a.body);
    let a = get(r.port, &p, &key);
    assert_eq!(
        (a.status, a.body["value"].clone()),
        (200, json!(-21.5)),
        "{}",
        a.body
    );
    let ws = admin.ask(json!({"t": "param.get", "serial": serial, "path": "vol.master"}));
    assert_eq!(ws["value"], -21.5, "the WebSocket reads what REST wrote");
    let g = format!("/api/v1/devices/{serial}/params/out.gain?idx=1");
    let a = call(r.port, "PUT", &g, Some(&key), Some(r#"{"value": -7.0}"#));
    assert_eq!(
        (a.status, a.body["idx"].clone()),
        (200, json!([1])),
        "{}",
        a.body
    );
    let a = get(
        r.port,
        &format!("/api/v1/devices/{serial}/params/out.gain?idx=99"),
        &key,
    );
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (400, json!("bad_request")),
        "{}",
        a.body
    );
    let a = get(r.port, "/api/v1/devices/NOPE/params/vol.master", &key);
    assert_eq!(a.status, 404, "an unknown serial: {}", a.body);

    // POST /presets/{n}/load: preset.load
    let saved = admin.ask(
        json!({"t": "preset.save", "serial": serial, "slot": 2, "name": "Live", "confirm": true}),
    );
    assert_eq!(saved["t"], "ok", "{saved}");
    let a = call(
        r.port,
        "POST",
        &format!("/api/v1/devices/{serial}/presets/2/load"),
        Some(&key),
        None,
    );
    assert_eq!(
        (a.status, a.body["slot"].clone(), a.body["name"].clone()),
        (200, json!(2), json!("Live")),
        "{}",
        a.body
    );
    let a = call(
        r.port,
        "POST",
        &format!("/api/v1/devices/{serial}/presets/x/load"),
        Some(&key),
        None,
    );
    assert_eq!(a.status, 400, "{}", a.body);

    // POST /snapshots/{id}/recall: snapshot.recall
    let id = r.put_snapshot(&mut admin, serial, "Doors");
    let a = call(
        r.port,
        "POST",
        &format!("/api/v1/devices/{serial}/snapshots/{id}/recall"),
        Some(&key),
        Some("{}"),
    );
    assert_eq!(
        (a.status, a.body["snapshot"].clone()),
        (200, json!(id)),
        "{}",
        a.body
    );
    let a = call(
        r.port,
        "POST",
        &format!("/api/v1/devices/{serial}/snapshots/nope/recall"),
        Some(&key),
        None,
    );
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (404, json!("not_found")),
        "{}",
        a.body
    );

    // GET /events: event.list
    let events = format!("/api/v1/devices/{serial}/events?after=0&limit=5");
    let mut a = get(r.port, &events, &key);
    wait_until("the device's history", || {
        a = get(r.port, &events, &key);
        a.status == 200
    });
    assert!(
        a.body["records"]
            .as_array()
            .is_some_and(|r| !r.is_empty() && r.len() <= 5),
        "{}",
        a.body
    );
    assert_eq!(a.body["serial"], serial);

    // GET /alarms and POST /alarms/{id}/ack: alarm.list and alarm.ack, P2AL's (faked)
    let a = get(r.port, "/api/v1/alarms?state=active", &key);
    assert_eq!(
        (a.status, a.body["alarms"][0]["id"].clone()),
        (200, json!("device:1")),
        "{}",
        a.body
    );
    let a = call(
        r.port,
        "POST",
        "/api/v1/alarms/device%3A1/ack",
        Some(&key),
        None,
    );
    assert_eq!(
        (a.status, a.body["alarm"]["id"].clone()),
        (200, json!("device:1")),
        "{}",
        a.body
    );
    {
        let seen = r.alarms.seen.lock().unwrap();
        assert_eq!(
            seen[0].1,
            json!({"t": "alarm.list", "id": 1, "state": "active"})
        );
        assert_eq!(
            seen[1].1,
            json!({"t": "alarm.ack", "id": 1, "alarm": "device:1"})
        );
    }

    // GET /health: health.get
    let a = get(r.port, "/api/v1/health", &key);
    assert_eq!(
        (a.status, a.body["v"].clone()),
        (200, json!(1)),
        "{}",
        a.body
    );

    // Paths and methods it does not have.
    let a = get(r.port, "/api/v1/raw", &key);
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (404, json!("not_found")),
        "{}",
        a.body
    );
    let a = call(r.port, "DELETE", "/api/v1/health", Some(&key), None);
    assert_eq!(a.status, 405, "{}", a.body);
    assert_eq!(a.header("Allow").as_deref(), Some("GET"));
    let a = call(r.port, "POST", &p, Some(&key), Some("{}"));
    assert_eq!(
        (a.status, a.header("Allow")),
        (405, Some("GET, PUT".into()))
    );
    r.hub.stop();
}

#[test]
fn wp_p2rs_a_missing_wrong_or_revoked_key_is_401() {
    let r = rig("keys");
    let serial = "P2RS-KEYS";
    r.plug(serial);
    let a = call(r.port, "GET", "/api/v1/health", None, None);
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (401, json!("unauthenticated")),
        "{}",
        a.body
    );
    assert_eq!(a.header("WWW-Authenticate").as_deref(), Some("Bearer"));
    let a = get(r.port, "/api/v1/health", "not-a-key");
    assert_eq!(a.status, 401);
    let mut admin = r.client("Installer", Role::Admin);
    let minted = admin.ask(json!({"t": "auth.mint", "name": "Script", "role": "control"}));
    let key = minted["key"].as_str().unwrap().to_owned();
    let cid = minted["cid"].clone();
    assert_eq!(get(r.port, "/api/v1/health", &key).status, 200);
    let p = format!("/api/v1/devices/{serial}/params/vol.master");
    assert_eq!(get(r.port, &p, &key).status, 200);
    let list = admin.ask(json!({"t": "auth.list"}));
    let row = list["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["cid"] == cid)
        .unwrap()
        .clone();
    assert_eq!(row["online"], true, "its REST session: {row}");
    let revoked = admin.ask(json!({"t": "auth.revoke", "cid": cid}));
    assert_eq!(revoked["t"], "ok");
    let a = get(r.port, "/api/v1/health", &key);
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (401, json!("unauthenticated")),
        "{}",
        a.body
    );
    let list = admin.ask(json!({"t": "auth.list"}));
    assert!(
        list["clients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["cid"] != cid),
        "{list}"
    );
    r.hub.stop();
}

#[test]
fn wp_p2rs_a_viewer_keys_put_and_a_limit_crossing_are_403() {
    let r = rig("roles");
    let serial = "P2RS-ROLES";
    let handle = r.plug(serial);
    let viewer = r.key("Dashboard", Role::Viewer);
    let control = r.key("Venue controller", Role::Control);
    let p = format!("/api/v1/devices/{serial}/params/vol.master");
    assert_eq!(get(r.port, &p, &viewer).status, 200, "a viewer reads");
    let a = call(r.port, "PUT", &p, Some(&viewer), Some(r#"{"value": -3.0}"#));
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (403, json!("denied")),
        "{}",
        a.body
    );
    let a = call(
        r.port,
        "POST",
        "/api/v1/alarms/device%3A1/ack",
        Some(&viewer),
        None,
    );
    assert_eq!(a.status, 403, "{}", a.body);
    // A limits profile: the master volume at most -10 dB.
    let mut admin = r.client("Installer", Role::Admin);
    let set = admin.ask(json!({"t": "limits.set", "handle": handle,
        "profile": {"outputs": [{"output": 0, "maxGainDb": 3.0}], "masterVolumeMaxDb": -10.0}}));
    assert_eq!(set["t"], "ok", "{set}");
    let before = get(r.port, &p, &control).body["value"].clone();
    let a = call(
        r.port,
        "PUT",
        &p,
        Some(&control),
        Some(r#"{"value": -3.0}"#),
    );
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (403, json!("limit_exceeded")),
        "{}",
        a.body
    );
    assert_eq!(a.body["limit"], "master_volume", "{}", a.body);
    assert_eq!(
        get(r.port, &p, &control).body["value"],
        before,
        "nothing was written"
    );
    let a = call(
        r.port,
        "PUT",
        &p,
        Some(&control),
        Some(r#"{"value": -12.0}"#),
    );
    assert_eq!(a.status, 200, "within the limit: {}", a.body);
    r.hub.stop();
}

#[test]
fn wp_p2rs_any_origin_is_403_before_the_key_is_looked_at() {
    let r = rig("origin");
    let key = r.key("Script", Role::Admin);
    let own = format!("http://127.0.0.1:{}", r.port);
    for origin in [own.as_str(), "http://evil.example", "null", ""] {
        let mut tcp = raw(r.port);
        write!(
            tcp,
            "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: {origin}\r\n\
             Authorization: Bearer {key}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let a = read_all(&mut tcp);
        assert_eq!(
            (a.status, a.body["code"].clone()),
            (403, json!("denied")),
            "{origin}: {}",
            a.body
        );
        assert!(a.header("Access-Control-Allow-Origin").is_none());
    }
    // A preflight too, with no key.
    let mut tcp = raw(r.port);
    write!(
        tcp,
        "OPTIONS /api/v1/devices HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://evil.example\r\n\
         Access-Control-Request-Method: PUT\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    assert_eq!(read_all(&mut tcp).status, 403);
    r.hub.stop();
}

#[test]
fn wp_p2rs_a_keys_requests_past_its_rate_are_429() {
    let r = rig_with(
        "rate",
        true,
        ServerLimits {
            rest_rate: 1,
            rest_burst: 2,
            ..quick()
        },
    );
    let key = r.key("Chatty", Role::Viewer);
    let other = r.key("Quiet", Role::Viewer);
    assert_eq!(get(r.port, "/api/v1/health", &key).status, 200);
    assert_eq!(get(r.port, "/api/v1/health", &key).status, 200);
    let a = get(r.port, "/api/v1/health", &key);
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (429, json!("rate_limited")),
        "{}",
        a.body
    );
    assert_eq!(a.header("Retry-After").as_deref(), Some("1"));
    assert!(
        a.body["retry_in_ms"]
            .as_u64()
            .is_some_and(|ms| ms > 0 && ms <= 1000),
        "{}",
        a.body
    );
    assert_eq!(
        get(r.port, "/api/v1/health", &other).status,
        200,
        "a key of its own"
    );
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(get(r.port, "/api/v1/health", &key).status, 200, "refilled");
    // Wrong keys from one address: the failure ledger's 429 after five.
    for _ in 0..5 {
        assert_eq!(get(r.port, "/api/v1/health", "guess").status, 401);
    }
    let a = get(r.port, "/api/v1/health", "guess");
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (429, json!("rate_limited")),
        "{}",
        a.body
    );
    r.hub.stop();
}

#[test]
fn wp_p2rs_hostile_lan_bodies_are_bounded() {
    let r = rig_with(
        "bodies",
        true,
        ServerLimits {
            rest_body_timeout: Duration::from_millis(500),
            ..quick()
        },
    );
    let serial = "P2RS-BODIES";
    r.plug(serial);
    let key = r.key("Script", Role::Control);
    let p = format!("/api/v1/devices/{serial}/params/vol.master");
    assert_eq!(
        call(r.port, "PUT", &p, Some(&key), Some(r#"{"value": -30.0}"#)).status,
        200
    );
    let unchanged = |what: &str| {
        let v = get(r.port, &p, &key).body["value"].clone();
        assert_eq!(v, -30.0, "{what}: nothing was written");
    };
    let head = |extra: &str| {
        format!("PUT {p} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {key}\r\n{extra}\r\n")
    };

    // Over the cap: 413, unread.
    let mut tcp = raw(r.port);
    tcp.write_all(head("Content-Length: 65537\r\n").as_bytes())
        .unwrap();
    let a = read_all(&mut tcp);
    assert_eq!(
        (a.status, a.body["code"].clone()),
        (413, json!("bad_request")),
        "{}",
        a.body
    );

    // A trickled body: cut at the body timeout, however steadily it comes.
    let body = r#"{"value": -1.0}                                             "#;
    let mut tcp = raw(r.port);
    tcp.write_all(head(&format!("Content-Length: {}\r\n", body.len())).as_bytes())
        .unwrap();
    let start = Instant::now();
    let mut sent = 0;
    // One byte each 50 ms until the hub answers (then nothing is left unread on its side).
    tcp.set_read_timeout(Some(Duration::from_millis(1)))
        .unwrap();
    let mut peek = [0u8; 1];
    for b in body.bytes() {
        if tcp.peek(&mut peek).is_ok_and(|n| n > 0) || tcp.write_all(&[b]).is_err() {
            break;
        }
        sent += 1;
        std::thread::sleep(Duration::from_millis(50));
        if start.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    assert!(
        sent < body.len(),
        "the hub answered before the body was all there"
    );
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let a = read_all(&mut tcp);
    assert_eq!(a.status, 408, "after {sent} bytes: {}", a.body);
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
    unchanged("trickled");

    // A body shorter than its length, and the peer waiting: 408.
    let mut tcp = raw(r.port);
    tcp.write_all(head("Content-Length: 40\r\n").as_bytes())
        .unwrap();
    tcp.write_all(br#"{"value": -1.0}"#).unwrap();
    assert_eq!(read_all(&mut tcp).status, 408);
    unchanged("short");

    // No length on a PUT, a chunked one: 411.
    for extra in ["", "Transfer-Encoding: chunked\r\n"] {
        let mut tcp = raw(r.port);
        tcp.write_all(head(extra).as_bytes()).unwrap();
        if !extra.is_empty() {
            tcp.write_all(b"10\r\n{\"value\": -1.0}\r\n0\r\n\r\n")
                .unwrap();
        }
        let a = read_all(&mut tcp);
        assert_eq!(a.status, 411, "{extra:?}: {}", a.body);
    }
    unchanged("no length");

    // A length that is not one, or two of them: 400.
    for extra in [
        "Content-Length: abc\r\n",
        "Content-Length: -5\r\n",
        "Content-Length: 15\r\nContent-Length: 15\r\n",
        "Content-Length: 15, 15\r\n",
    ] {
        let mut tcp = raw(r.port);
        tcp.write_all(head(extra).as_bytes()).unwrap();
        tcp.write_all(br#"{"value": -1.0}"#).unwrap();
        let a = read_all(&mut tcp);
        assert_eq!(a.status, 400, "{extra:?}: {}", a.body);
    }
    unchanged("bad length");

    // A body longer than its length: only the length is read, which is no JSON here,
    // and the rest is never taken for a request: the connection closes.
    let mut tcp = raw(r.port);
    tcp.write_all(head("Content-Length: 5\r\n").as_bytes())
        .unwrap();
    tcp.write_all(b"{\"value\": -1.0}GET /api/v1/health HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let mut all = Vec::new();
    let _ = tcp.read_to_end(&mut all);
    let text = String::from_utf8_lossy(&all);
    assert!(text.starts_with("HTTP/1.1 400"), "{text}");
    assert_eq!(
        text.matches("HTTP/1.1").count(),
        1,
        "one answer only: {text}"
    );
    unchanged("longer");

    // A body that is JSON but not an object, and one the device cannot take.
    let a = call(r.port, "PUT", &p, Some(&key), Some("[1]"));
    assert_eq!(a.status, 400, "{}", a.body);
    let a = call(r.port, "PUT", &p, Some(&key), Some(r#"{"value": "loud"}"#));
    assert_eq!(a.status, 400, "{}", a.body);
    unchanged("not a value");
    r.hub.stop();
}

#[test]
fn wp_p2rs_keys_are_minted_by_an_admin_shown_once_and_listed_with_their_kind() {
    let r = rig("mint");
    let mut admin = r.client("Installer", Role::Admin);
    let mut control = r.client("Tablet", Role::Control);
    let denied = control.ask(json!({"t": "auth.mint", "name": "Mine", "role": "viewer"}));
    assert_eq!(denied["code"], "denied", "{denied}");
    let bad = admin.ask(json!({"t": "auth.mint", "name": "x", "role": "root"}));
    assert_eq!(bad["code"], "bad_request", "{bad}");
    let bad =
        admin.ask(json!({"t": "auth.mint", "name": "x", "role": "viewer", "plaintext_ok": "yes"}));
    assert_eq!(bad["code"], "bad_request", "{bad}");
    let minted = admin.ask(
        json!({"t": "auth.mint", "name": "Home Assistant", "role": "control",
        "plaintext_ok": true}),
    );
    assert_eq!(minted["t"], "ok", "{minted}");
    assert_eq!(
        (
            minted["v"].clone(),
            minted["kind"].clone(),
            minted["role"].clone(),
            minted["plaintext_ok"].clone()
        ),
        (json!(1), json!("api"), json!("control"), json!(true))
    );
    let key = minted["key"].as_str().unwrap().to_owned();
    assert!(key.len() >= 43, "32 random bytes: {key}");
    let list = admin.ask(json!({"t": "auth.list"}));
    let rows = list["clients"].as_array().unwrap();
    let row = rows.iter().find(|c| c["cid"] == minted["cid"]).unwrap();
    assert_eq!(
        (row["kind"].clone(), row["plaintext_ok"].clone()),
        (json!("api"), json!(true)),
        "{row}"
    );
    assert!(
        row.get("key").is_none() && !list.to_string().contains(&key),
        "shown once"
    );
    assert!(
        rows.iter()
            .filter(|c| c["cid"] != minted["cid"])
            .all(|c| c["kind"] == "console"),
        "{list}"
    );
    let file = std::fs::read_to_string(r.dir.join("store").join("hub-store.json")).unwrap();
    assert!(!file.contains(&key), "only a hash is stored");
    assert!(file.contains("\"kind\": \"api\""), "{file}");
    // A hub that starts again keeps it, with its kind.
    let net = r.hub.network().unwrap();
    let kept = net
        .auth()
        .clients()
        .into_iter()
        .find(|c| c.cid as u64 == minted["cid"].as_u64().unwrap())
        .unwrap();
    assert_eq!((kept.kind, kept.plaintext_ok), (ClientKind::Api, true));
    let saved = dspi_hub::SavedHub::peek(&r.dir.join("store"))
        .unwrap()
        .unwrap();
    assert!(
        saved
            .clients
            .iter()
            .any(|c| c.kind == ClientKind::Api && c.plaintext_ok)
    );
    // An admin may mint an admin key.
    let a = admin.ask(json!({"t": "auth.mint", "name": "Ops", "role": "admin"}));
    assert_eq!(a["role"], "admin", "{a}");
    r.hub.stop();
}

#[test]
fn wp_p2rs_the_65th_paired_client_or_key_is_refused() {
    let r = rig("cap");
    let mut admin = r.client("Installer", Role::Admin);
    let net = r.hub.network().unwrap();
    for i in 0..62 {
        net.auth()
            .mint_api_key(&format!("k{i}"), Role::Viewer, false)
            .unwrap();
    }
    let last = admin.ask(json!({"t": "auth.mint", "name": "64th", "role": "viewer"}));
    assert_eq!(last["t"], "ok", "the 64th: {last}");
    assert_eq!(net.auth().clients().len(), 64);
    let full = admin.ask(json!({"t": "auth.mint", "name": "65th", "role": "viewer"}));
    assert_eq!(full["code"], "conflict", "{full}");
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    let paired = net.auth().pair(
        &window.pin,
        "Phone",
        None,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Instant::now(),
    );
    assert_eq!(
        paired.unwrap_err(),
        dspi_hub::auth::AuthError::TableFull,
        "a console neither"
    );
    // Revoking a key makes room, for either.
    let revoked = admin.ask(json!({"t": "auth.revoke", "cid": last["cid"]}));
    assert_eq!(revoked["t"], "ok");
    let again = admin.ask(json!({"t": "auth.mint", "name": "65th", "role": "viewer"}));
    assert_eq!(again["t"], "ok", "{again}");
    r.hub.stop();
}

#[test]
fn wp_p2rs_a_keys_session_is_a_remote_principal_kept_warm_then_closed() {
    let r = rig_with(
        "principal",
        true,
        ServerLimits {
            rest_idle: Duration::from_millis(600),
            ..quick()
        },
    );
    let serial = "P2RS-PRINCIPAL";
    r.plug(serial);
    let key = r.key("Venue controller", Role::Control);
    let p = format!("/api/v1/devices/{serial}/params/vol.master");
    let sub = r.hub.bus().subscribe("p2rs-attr", 1024);
    let mut sessions = Vec::new();
    // Values the demo device is not at already: an unchanged one sends no notification.
    for v in [-23.5, -26.5] {
        let a = call(
            r.port,
            "PUT",
            &p,
            Some(&key),
            Some(&format!("{{\"value\": {v}}}")),
        );
        assert_eq!(a.status, 200, "{}", a.body);
        let start = Instant::now();
        loop {
            assert!(
                start.elapsed() < WAIT,
                "no notification for {v}: {sessions:?}"
            );
            if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
                && let BusMessage::Notification {
                    serial: s,
                    session,
                    by,
                    ..
                } = &*m
                && s == serial
                && *session != 0
            {
                sessions.push((*session, by.clone()));
                break;
            }
        }
        while sub.recv_timeout(Duration::from_millis(50)).is_some() {}
    }
    let (session, by) = sessions[0].clone();
    assert!(session >= FIRST_REMOTE_SESSION_ID, "{session}");
    assert!(![LOCAL_SESSION_ID, INTERNAL_SESSION_ID, GUARD_SESSION_ID].contains(&session));
    assert_eq!(
        by.as_deref(),
        Some("Venue controller"),
        "attributed to the key's name"
    );
    assert_eq!(
        sessions[1].0, session,
        "one core for the key: the same child session"
    );
    // The alarms' fake saw the key's own session, a remote one too.
    assert_eq!(get(r.port, "/api/v1/alarms", &key).status, 200);
    let principal = r.alarms.seen.lock().unwrap()[0].0;
    assert!(principal >= FIRST_REMOTE_SESSION_ID && principal != INTERNAL_SESSION_ID);
    assert!(!r.closed(principal));
    wait_until("the idle key's session to close", || r.closed(principal));
    // A new request opens a new one.
    assert_eq!(get(r.port, "/api/v1/alarms", &key).status, 200);
    let next = r.alarms.seen.lock().unwrap()[1].0;
    assert_ne!(next, principal);
    r.hub.stop();
}

#[test]
fn wp_p2rs_off_by_default_and_nothing_else_changes() {
    let r = rig_with("off", false, quick());
    let key = r.key("Script", Role::Admin);
    let ws = r.client("Tablet", Role::Control);
    assert!(ws.caps.iter().any(|c| c == "api_keys"), "{:?}", ws.caps);
    assert!(!ws.caps.iter().any(|c| c == "rest"), "{:?}", ws.caps);
    for (method, path) in [
        ("GET", "/api/v1/health"),
        ("PUT", "/api/v1/devices/x/params/vol.master"),
        ("GET", "/api/v1"),
    ] {
        let a = call(r.port, method, path, Some(&key), None);
        assert_eq!(a.status, 404, "{method} {path} while off: {}", a.body);
    }
    // The protocol's paths, with the mirror on.
    let on = rig("on");
    let key = on.key("Script", Role::Admin);
    let info = call(on.port, "GET", "/dspi/v1/info", None, None);
    assert_eq!(info.status, 200);
    assert_eq!(
        info.header("Access-Control-Allow-Origin").as_deref(),
        Some("*")
    );
    assert_eq!(
        call(on.port, "PUT", "/dspi/v1/info", None, None).status,
        405
    );
    assert_eq!(
        call(on.port, "GET", "/dspi/v1/nothing", Some(&key), None).status,
        404
    );
    assert_eq!(
        call(on.port, "GET", "/api/v2/health", Some(&key), None).status,
        404
    );
    assert_eq!(call(on.port, "GET", "/api", Some(&key), None).status, 404);
    // A key is a token: it may also open a WebSocket session.
    let mut ws = Ws::connect(on.port);
    ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2}}));
    let _ = ws.next("hello");
    let authed = ws.ask(json!({"t": "auth.token", "token": key}));
    assert_eq!(
        (authed["t"].clone(), authed["role"].clone()),
        (json!("ok"), json!("admin")),
        "{authed}"
    );
    // Kept alive between two GETs, as a page's requests are.
    let mut tcp = raw(on.port);
    let req = format!(
        "GET /api/v1/health HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {key}\r\nConnection: keep-alive\r\n\r\n"
    );
    for _ in 0..2 {
        tcp.write_all(req.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            assert_eq!(tcp.read(&mut byte).unwrap(), 1);
            buf.push(byte[0]);
        }
        let a = parse(&buf);
        assert_eq!(a.status, 200);
        let n: usize = a.header("Content-Length").unwrap().parse().unwrap();
        let mut body = vec![0u8; n];
        tcp.read_exact(&mut body).unwrap();
    }
    on.hub.stop();
    r.hub.stop();
}
