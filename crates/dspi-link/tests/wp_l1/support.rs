//! Shared helpers.

#![allow(dead_code, unused_macros)]

use std::sync::Arc;
use std::time::Duration;

use dspi_link::clock::{Clock, ManualClock};
use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::{TcpConnector, WsConnector, WsMessage, WsPair, WsRecv};
use dspi_link::ConnectOptions;

pub fn opts(token: Option<String>) -> ConnectOptions {
    ConnectOptions {
        client_name: "Pixel 9".into(),
        app: "DSPi Console for Android".into(),
        app_version: "test".into(),
        token,
    }
}

/// Polls `f` (up to 5 s of real time, normally a few ms).
pub fn wait(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..1000 {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

pub fn manual_clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new())
}

pub fn dyn_clock(c: &Arc<ManualClock>) -> Arc<dyn Clock> {
    Arc::clone(c) as Arc<dyn Clock>
}

/// A hub whose timers run on `clock`.
pub fn hub_on(clock: &Arc<ManualClock>, cfg: HubConfig) -> FakeHub {
    FakeHub::start(HubConfig { clock: dyn_clock(clock), ..cfg }).unwrap()
}

/// A bare WebSocket with no client logic: it answers nothing by itself once
/// its reader stops being called, like a client that vanished.
pub fn raw_socket(hub: &FakeHub) -> WsPair {
    TcpConnector::default().connect(&hub.url(), "dspi-link-1").unwrap()
}

const HELLO: &str = r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"raw","app":"test","version":"0"}}"#;

/// Exchange hellos on a bare socket.
pub fn raw_hello(pair: &mut WsPair) {
    pair.0.send(WsMessage::Text(HELLO.into())).unwrap();
    raw_reply(pair, "hello");
}

/// The next text message with `t` = `t`, as JSON.
pub fn raw_reply(pair: &mut WsPair, t: &str) -> serde_json::Value {
    for _ in 0..50 {
        if let WsRecv::Message(WsMessage::Text(m)) = pair.1.recv() {
            let v: serde_json::Value = serde_json::from_str(&m).unwrap();
            if v["t"] == t {
                return v;
            }
        }
    }
    panic!("no `{t}` from the hub");
}

/// Send `msg` on a bare socket and read the `ok` or `err` it gets.
pub fn raw_request(pair: &mut WsPair, msg: serde_json::Value) -> serde_json::Value {
    pair.0.send(WsMessage::Text(msg.to_string())).unwrap();
    for _ in 0..50 {
        if let WsRecv::Message(WsMessage::Text(m)) = pair.1.recv() {
            let v: serde_json::Value = serde_json::from_str(&m).unwrap();
            if v["t"] == "ok" || v["t"] == "err" {
                return v;
            }
        }
    }
    panic!("no reply from the hub");
}

// ── A scripted socket: no hub thread and no network between the test and the
// client, so liveness tests decide exactly what arrives and when ────────────

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

use dspi_link::socket::{WsReceiver, WsSender};
use dspi_link::{LinkClient, LinkError, LinkEvents, SessionConfig, Timing};

/// The scripted socket's hub side.
pub struct MockHub {
    inbound: Sender<WsRecv>,
    pub pings: AtomicU32,
    pub answer_pings: AtomicBool,
    pub fail_sends: AtomicBool,
    pub closed: AtomicBool,
    pub sent: Mutex<VecDeque<WsMessage>>,
    /// Text requests -> reply; `None` means silence.
    respond: Box<dyn Fn(&serde_json::Value) -> Option<serde_json::Value> + Send + Sync>,
}

impl MockHub {
    pub fn push(&self, r: WsRecv) {
        let _ = self.inbound.send(r);
    }
}

struct MockSender(Arc<MockHub>);
struct MockReceiver(Receiver<WsRecv>);

impl WsSender for MockSender {
    fn send(&self, msg: WsMessage) -> std::io::Result<()> {
        if self.0.fail_sends.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("send failed"));
        }
        if let WsMessage::Text(t) = &msg {
            let v: serde_json::Value = serde_json::from_str(t).unwrap();
            if let Some(r) = (self.0.respond)(&v) {
                self.0.push(WsRecv::Message(WsMessage::Text(r.to_string())));
            }
        }
        self.0.sent.lock().unwrap().push_back(msg);
        Ok(())
    }
    fn ping(&self) -> std::io::Result<()> {
        self.0.pings.fetch_add(1, Ordering::SeqCst);
        if self.0.answer_pings.load(Ordering::SeqCst) {
            self.0.push(WsRecv::Alive);
        }
        Ok(())
    }
    fn close(&self, code: u16) {
        self.0.closed.store(true, Ordering::SeqCst);
        self.0.push(WsRecv::Closed { code: Some(code), reason: "closed".into() });
    }
}

impl WsReceiver for MockReceiver {
    fn recv(&mut self) -> WsRecv {
        // A short tick, so the client looks at its clock often
        self.0.recv_timeout(Duration::from_millis(2)).unwrap_or(WsRecv::Idle)
    }
}

pub struct MockConnector {
    hub: Arc<MockHub>,
    rx: Mutex<Option<Receiver<WsRecv>>>,
}

impl WsConnector for MockConnector {
    fn connect(&self, _url: &str, _protocol: &str) -> Result<WsPair, LinkError> {
        let rx = self.rx.lock().unwrap().take().ok_or(LinkError::Io("one connect only".into()))?;
        Ok((Box::new(MockSender(Arc::clone(&self.hub))), Box::new(MockReceiver(rx))))
    }
}

/// A scripted hub: `hello` is answered with `auth`, then `respond` decides.
pub fn mock_hub(
    auth: &'static str,
    respond: impl Fn(&serde_json::Value) -> Option<serde_json::Value> + Send + Sync + 'static,
) -> (Arc<MockHub>, MockConnector) {
    let (tx, rx) = mpsc::channel();
    let hub = Arc::new(MockHub {
        inbound: tx,
        pings: AtomicU32::new(0),
        answer_pings: AtomicBool::new(false),
        fail_sends: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        sent: Mutex::new(VecDeque::new()),
        respond: Box::new(move |v| {
            if v["t"] == "hello" {
                Some(serde_json::json!({
                    "t": "hello", "proto": {"major": 1, "minor": 2},
                    "hub": {"id": "mock-hub", "name": "Mock", "kind": "console"},
                    "auth": auth, "caps": ["cmd"], "limits": {"max_inflight": 4},
                }))
            } else {
                respond(v)
            }
        }),
    });
    (Arc::clone(&hub), MockConnector { hub, rx: Mutex::new(Some(rx)) })
}

/// Answers the requests of an open hub's connect; the rest is silence.
pub fn open_hub_replies(v: &serde_json::Value) -> Option<serde_json::Value> {
    match v["t"].as_str() {
        Some("auth.pair") => Some(serde_json::json!({"t": "ok", "id": v["id"], "session": 3, "role": "admin"})),
        Some("device.list") => Some(serde_json::json!({"t": "ok", "id": v["id"], "devices": []})),
        _ => None,
    }
}

/// Production's limits, on `clock`, with a fast keepalive tick.
pub fn session_on(clock: &Arc<ManualClock>) -> SessionConfig {
    SessionConfig {
        clock: dyn_clock(clock),
        timing: Timing { keepalive_tick: Duration::from_millis(2), ..Timing::default() },
        ..SessionConfig::default()
    }
}

/// An open session on a scripted socket.
pub fn mock_session(clock: &Arc<ManualClock>, cfg: SessionConfig) -> (Arc<MockHub>, LinkClient) {
    let _ = clock;
    let (hub, conn) = mock_hub("none", open_hub_replies);
    let (c, _) = LinkClient::connect_with(&conn, "ws://mock", &opts(None), None, &cfg).unwrap();
    (hub, c)
}

/// Records every event.
#[derive(Default)]
pub struct Recorder {
    pub closed: Mutex<Vec<(u64, Option<u16>, String)>>,
    pub devices: Mutex<Vec<u64>>,
    pub tokens: Mutex<Vec<(String, String)>>,
}

impl LinkEvents for Recorder {
    fn session_devices_changed(&self, epoch: u64) {
        self.devices.lock().unwrap().push(epoch);
    }
    fn session_closed(&self, epoch: u64, code: Option<u16>, reason: &str) {
        self.closed.lock().unwrap().push((epoch, code, reason.to_owned()));
    }
    fn token_issued(&self, hub_id: &str, token: &str) {
        self.tokens.lock().unwrap().push((hub_id.to_owned(), token.to_owned()));
    }
}

/// A scripted PIN hub whose hello has no `hub.id`.
pub fn mock_hub_without_id() -> (Arc<MockHub>, MockConnector) {
    let (tx, rx) = mpsc::channel();
    let hub = Arc::new(MockHub {
        inbound: tx,
        pings: AtomicU32::new(0),
        answer_pings: AtomicBool::new(false),
        fail_sends: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        sent: Mutex::new(VecDeque::new()),
        respond: Box::new(|v| {
            (v["t"] == "hello").then(|| {
                serde_json::json!({"t": "hello", "proto": {"major": 1, "minor": 2}, "hub": {"name": "Odd"}, "auth": "pin"})
            })
        }),
    });
    (Arc::clone(&hub), MockConnector { hub, rx: Mutex::new(Some(rx)) })
}
