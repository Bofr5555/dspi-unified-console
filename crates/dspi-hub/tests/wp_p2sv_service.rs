//! WP P2SV: the hub's service seam, with a test service, through the real hub and
//! (for the messages and `hello`) the real server.
//!
//! A service answers its message type, is consulted after the limits, shows in
//! `hello.caps`, gets its timer calls at its due time on a test clock, and sees
//! devices come and go and their notifications with serial and session on the bus.
//! A slow bus subscriber delays no command and reads its drop count; a principal
//! opened by the helper has a remote-range id and is judged like any client.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::session::{
    FIRST_REMOTE_SESSION_ID, GUARD_SESSION_ID, INTERNAL_SESSION_ID, LOCAL_SESSION_ID, NO_ORIGIN,
};
use dspi_hub::{
    AccessMode, BusMessage, Completion, DeviceEvent, Hub, HubClock, HubConfig, HubHandle,
    HubService, NetworkConfig, Role, ServiceConfig, Session, TestClock,
};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2sv").join(format!(
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
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The test service: answers `p2sv.echo` and claims `hub.stats`; counts what it is
/// asked; runs when due; records device events.
struct Probe {
    clock: Arc<dyn HubClock>,
    asked: Mutex<Vec<String>>,
    due: Mutex<Option<Instant>>,
    ran: Mutex<Vec<Instant>>,
    devices: Mutex<Vec<DeviceEvent>>,
}

impl Probe {
    fn new(clock: Arc<dyn HubClock>) -> Arc<Self> {
        Arc::new(Self {
            clock,
            asked: Mutex::new(Vec::new()),
            due: Mutex::new(None),
            ran: Mutex::new(Vec::new()),
            devices: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self, t: &str) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| *a == t)
            .count()
    }
}

impl HubService for Probe {
    fn name(&self) -> &'static str {
        "p2sv-probe"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec!["p2sv_probe", "limits"]
    }

    fn handle_message(
        &self,
        _hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        self.asked.lock().unwrap().push(t.to_owned());
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        match t {
            "p2sv.echo" => Some(
                json!({"t": "ok", "id": id, "v": 1, "session": session.id(), "now_s": self.clock.unix_seconds()})
                    .to_string(),
            ),
            "hub.stats" => Some(json!({"t": "ok", "id": id, "v": 1, "by": "probe"}).to_string()),
            // Would answer the limits' messages, if it were ever asked one.
            "limits.get" => Some(json!({"t": "err", "id": id, "code": "probe"}).to_string()),
            _ => None,
        }
    }

    fn next_due(&self) -> Option<Instant> {
        *self.due.lock().unwrap()
    }

    fn run_due(&self, _hub: &HubHandle, now: Instant) {
        self.ran.lock().unwrap().push(now);
        *self.due.lock().unwrap() = None;
    }

    fn on_device(&self, event: &DeviceEvent) {
        self.devices.lock().unwrap().push(event.clone());
    }
}

fn config_with(probe: &Arc<Probe>, clock: Arc<dyn HubClock>) -> HubConfig {
    HubConfig {
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            clock,
            extra: vec![Arc::clone(probe) as Arc<dyn HubService>],
            // -- WP P2HP -- the struct gained a field
            // -- WP P2IN -- the services' new fields stay at their defaults
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2SV hub")
    }
}

fn in_process(probe: &Arc<Probe>, clock: Arc<dyn HubClock>) -> Hub {
    Hub::start(config_with(probe, clock)).unwrap()
}

fn submit(hub: &HubHandle, session: &Session, req: CmdRequest) -> Receiver<CmdResponse> {
    let (tx, rx) = mpsc::channel();
    let tag = req.tag;
    hub.submit(
        session,
        req,
        Completion::new(tag, move |r| {
            let _ = tx.send(r);
        }),
    );
    rx
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

fn volume(hub: &HubHandle, session: &Session, handle: u8, db: f32) -> u8 {
    submit(hub, session, set_volume(handle, db))
        .recv_timeout(WAIT)
        .unwrap()
        .status
}

/// The next notification on `sub` from `serial`, as (session, by).
fn next_notification(sub: &dspi_hub::Subscription, serial: &str) -> (u16, Option<String>) {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no notification from {serial}");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
            && let BusMessage::Notification {
                serial: s,
                session,
                by,
                packet,
            } = &*m
            && s == serial
        {
            assert!(!packet.is_empty());
            return (*session, by.clone());
        }
    }
}

// ── Over the socket ─────────────────────────────────────────────────────────

struct Ws(tungstenite::WebSocket<TcpStream>);

impl Ws {
    fn connect(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        Self(tungstenite::client(req, tcp).unwrap().0)
    }

    fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).unwrap();
    }

    fn next(&mut self, t: &str) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            if let Message::Text(text) = self.0.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if v["t"] == t {
                    return v;
                }
            }
        }
    }

    /// The answer to the message with this `id`.
    fn ask(&mut self, mut msg: Value, id: u64) -> Value {
        msg["id"] = id.into();
        self.send(msg);
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {id}");
            if let Message::Text(text) = self.0.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if v["id"] == id {
                    return v;
                }
            }
        }
    }
}

#[test]
fn wp_p2sv_a_service_answers_after_the_limits_and_is_in_hello() {
    let dir = scratch("socket");
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let probe = Probe::new(Arc::clone(&clock));
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.clone())
        }),
        initial_scan: false,
        ..config_with(&probe, Arc::clone(&clock))
    })
    .unwrap();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2SV-W")), 0)
        .unwrap();
    // The data folder defaults to the network store's.
    assert_eq!(hub.data_dir(), Some(dir.as_path()));

    let mut ws = Ws::connect(hub.network().unwrap().port());
    ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
        "client": {"name": "p2sv", "app": "test", "version": "0"}}));
    let hello = ws.next("hello");
    let caps: Vec<&str> = hello["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(caps.contains(&"p2sv_probe"), "{hello}");
    assert_eq!(
        caps.iter().filter(|c| **c == "limits").count(),
        1,
        "a cap the hub has already is listed once: {hello}"
    );

    // Its own message.
    let echo = ws.ask(json!({"t": "p2sv.echo"}), 1);
    assert_eq!(echo["t"], "ok", "{echo}");
    assert_eq!(echo["v"], 1);
    assert!(echo["session"].as_u64().unwrap() >= u64::from(FIRST_REMOTE_SESSION_ID));
    assert_eq!(probe.asked("p2sv.echo"), 1);

    // After the limits: `limits.get` is theirs, and the service is never asked.
    let limits = ws.ask(json!({"t": "limits.get", "handle": handle}), 2);
    assert_eq!(limits["t"], "ok", "{limits}");
    assert_eq!(probe.asked("limits.get"), 0, "the limits answered first");
    // And the lock's messages before that.
    let lock = ws.ask(json!({"t": "lock.release", "handle": handle}), 3);
    assert_eq!(lock["t"], "ok", "{lock}");
    assert_eq!(probe.asked("lock.release"), 0);

    // Before the hub's own: a service may answer `hub.stats`; `hub.rename`, which
    // none answers, stays `unsupported` for an Admin, and so does `fw.install`.
    let stats = ws.ask(json!({"t": "hub.stats"}), 4);
    assert_eq!(stats["by"], "probe", "{stats}");
    let rename = ws.ask(json!({"t": "hub.rename", "name": "x"}), 5);
    assert_eq!(rename["code"], "unsupported", "{rename}");
    let fw = ws.ask(json!({"t": "fw.install"}), 6);
    assert_eq!(fw["code"], "unsupported", "{fw}");
    // Unknown types are still `unknown_type`, after every service passed.
    let unknown = ws.ask(json!({"t": "p2sv.nothing"}), 7);
    assert_eq!(unknown["code"], "unknown_type", "{unknown}");
    assert_eq!(probe.asked("p2sv.nothing"), 1);
    // The hub's own messages still answer as before.
    let list = ws.ask(json!({"t": "device.list"}), 8);
    assert_eq!(list["devices"][0]["serial"], "P2SV-W", "{list}");
    drop(ws);
    hub.stop();
}

#[test]
fn wp_p2sv_the_timer_calls_a_service_at_its_due_time_on_the_test_clock() {
    let test_clock = Arc::new(TestClock::new());
    let clock: Arc<dyn HubClock> = Arc::clone(&test_clock) as Arc<dyn HubClock>;
    let probe = Probe::new(Arc::clone(&clock));
    let hub = in_process(&probe, Arc::clone(&clock));
    assert!(Arc::ptr_eq(hub.clock(), &clock));
    assert_eq!(hub.data_dir(), None, "an in-process hub keeps nothing");

    let due = test_clock.now() + Duration::from_secs(60);
    *probe.due.lock().unwrap() = Some(due);
    hub.wake_services();
    // Real time passes; the hub's clock does not: not due yet.
    std::thread::sleep(Duration::from_millis(300));
    hub.wake_services();
    std::thread::sleep(Duration::from_millis(100));
    assert!(probe.ran.lock().unwrap().is_empty(), "ran before its time");

    test_clock.advance(Duration::from_secs(59));
    hub.wake_services();
    std::thread::sleep(Duration::from_millis(200));
    assert!(probe.ran.lock().unwrap().is_empty(), "ran a second early");

    test_clock.advance(Duration::from_secs(1));
    hub.wake_services();
    wait_until("the service's timer", || {
        !probe.ran.lock().unwrap().is_empty()
    });
    let ran = probe.ran.lock().unwrap().clone();
    assert_eq!(ran, vec![due], "once, with the hub clock's now");
    hub.stop();
}

#[test]
fn wp_p2sv_a_service_sees_devices_and_notifications_with_serial_and_session() {
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let probe = Probe::new(Arc::clone(&clock));
    let hub = Hub::start(HubConfig {
        offline_grace: Duration::from_millis(50),
        ..config_with(&probe, Arc::clone(&clock))
    })
    .unwrap();
    let h = hub.handle();
    let sub = h.bus().subscribe("test", 1024);

    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2SV-N")), 0)
        .unwrap();
    let online = DeviceEvent::Online {
        serial: "P2SV-N".into(),
        handle,
    };
    assert_eq!(probe.devices.lock().unwrap().clone(), vec![online]);
    let first = sub.recv_timeout(WAIT).unwrap();
    assert_eq!(
        *first,
        BusMessage::DeviceOnline {
            serial: "P2SV-N".into(),
            handle
        }
    );

    // A client's write comes back with its serial and its session.
    let client = h.open_session(Role::Control, None).unwrap();
    assert_eq!(volume(&h, &client, handle, -12.0), status::OK);
    let (session, by) = next_notification(&sub, "P2SV-N");
    assert_eq!(session, client.id());
    assert_eq!(by, Some(format!("Session {}", client.id())));

    // A principal's, with its name.
    let rule = h
        .open_principal(Role::Control, None, "Schedule: Morning")
        .unwrap();
    assert_eq!(volume(&h, &rule, handle, -20.0), status::OK);
    let (session, by) = next_notification(&sub, "P2SV-N");
    assert_eq!(session, rule.id());
    assert_eq!(by.as_deref(), Some("Schedule: Morning"));
    h.close_session(rule.id());

    // Offline, then forgotten after the grace.
    hub.detach(handle).unwrap();
    wait_until("removed", || probe.devices.lock().unwrap().len() == 3);
    assert_eq!(
        probe.devices.lock().unwrap()[1..].to_vec(),
        vec![
            DeviceEvent::Offline {
                serial: "P2SV-N".into(),
                handle
            },
            DeviceEvent::Removed {
                serial: "P2SV-N".into(),
                handle
            },
        ]
    );
    let rest: Vec<BusMessage> = sub
        .drain()
        .iter()
        .map(|m| (**m).clone())
        .filter(|m| !matches!(m, BusMessage::Notification { .. }))
        .collect();
    assert_eq!(
        rest,
        vec![
            BusMessage::DeviceOffline {
                serial: "P2SV-N".into(),
                handle
            },
            BusMessage::DeviceRemoved {
                serial: "P2SV-N".into(),
                handle
            },
        ]
    );
    assert_eq!(sub.dropped(), 0);
    hub.stop();
}

#[test]
fn wp_p2sv_a_paired_clients_name_labels_its_session() {
    let dir = scratch("label");
    let probe = Probe::new(Arc::new(TestClock::new()));
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        initial_scan: false,
        ..config_with(&probe, Arc::new(TestClock::new()))
    })
    .unwrap();
    let net = hub.network().unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    let authed = net
        .auth()
        .pair(
            &window.pin,
            "Front of house",
            None,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Instant::now(),
        )
        .unwrap();
    let h = hub.handle();
    let session = h.open_session(authed.role, Some(authed.cid)).unwrap();
    let label = h.session_label(session.id());
    assert_eq!(label.cid, Some(authed.cid));
    assert_eq!(label.by().as_deref(), Some("Front of house"));
    h.close_session(session.id());
    assert_eq!(
        h.session_label(session.id()).by(),
        Some(format!("Session {}", session.id())),
        "a closed session's label goes"
    );
    hub.stop();
}

#[test]
fn wp_p2sv_a_slow_subscriber_delays_no_command_and_sees_its_drops() {
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let probe = Probe::new(Arc::clone(&clock));
    let hub = in_process(&probe, clock);
    let h = hub.handle();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2SV-S")), 0)
        .unwrap();
    // Never read while the commands run.
    let slow = h.bus().subscribe("slow", 4);
    // Read on its own thread, but slowly: it must not hold anyone up either.
    let crawler = h.bus().subscribe("crawler", 8);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let crawling = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = crawler.recv_timeout(Duration::from_millis(10));
                std::thread::sleep(Duration::from_millis(50));
            }
            crawler.dropped()
        })
    };
    let fast = h.bus().subscribe("fast", 4096);
    let client = h.open_session(Role::Control, None).unwrap();
    let started = Instant::now();
    for i in 0..100 {
        let at = Instant::now();
        let db = -30.0 + (i % 20) as f32;
        assert_eq!(volume(&h, &client, handle, db), status::OK);
        assert!(
            at.elapsed() < Duration::from_secs(2),
            "command {i} was held up"
        );
    }
    let took = started.elapsed();
    // The subscriber that keeps up has the writes' notifications, and lost none.
    let mut heard = 0;
    wait_until("the fast subscriber's notifications", || {
        heard += fast
            .drain()
            .iter()
            .filter(|m| matches!(&***m, BusMessage::Notification { .. }))
            .count();
        heard > 0
    });
    assert_eq!(fast.dropped(), 0);
    assert!(slow.dropped() > 0, "the slow subscriber lost the oldest");
    assert_eq!(slow.len(), 4, "and kept the newest it has room for");
    stop.store(true, Ordering::SeqCst);
    let crawler_dropped = crawling.join().unwrap();
    assert!(crawler_dropped > 0, "a slow reader sees its drops too");
    assert!(took < WAIT, "100 commands took {took:?}");
    hub.stop();
}

#[test]
fn wp_p2sv_a_principal_is_a_remote_session_judged_like_any_client() {
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let probe = Probe::new(Arc::clone(&clock));
    let hub = in_process(&probe, clock);
    let h = hub.handle();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2SV-P")), 0)
        .unwrap();
    let mut ids = Vec::new();
    for n in 0..20 {
        let p = h
            .open_principal(Role::Viewer, None, &format!("Schedule: {n}"))
            .unwrap();
        ids.push(p.id());
    }
    for id in &ids {
        assert!(*id >= FIRST_REMOTE_SESSION_ID, "{id}");
        assert!(
            ![
                NO_ORIGIN,
                LOCAL_SESSION_ID,
                INTERNAL_SESSION_ID,
                GUARD_SESSION_ID
            ]
            .contains(id),
            "{id}"
        );
    }
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "distinct");

    // A Viewer principal is refused a write, as a Viewer client is.
    let viewer = h.open_principal(Role::Viewer, None, "Viewer key").unwrap();
    assert_eq!(volume(&h, &viewer, handle, -6.0), status::DENIED);
    // The lock judges it: another session's lock refuses its write.
    let control = h
        .open_principal(Role::Control, None, "Control key")
        .unwrap();
    let holder = h.open_session(Role::Control, None).unwrap();
    assert!(h.lock_acquire(&holder, handle, Duration::from_secs(60)));
    assert_eq!(volume(&h, &control, handle, -6.0), status::LOCKED);
    h.lock_release(&holder, handle);
    assert_eq!(volume(&h, &control, handle, -6.0), status::OK);
    hub.stop();
}
