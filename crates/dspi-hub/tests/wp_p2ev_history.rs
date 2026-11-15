//! WP P2EV: the hub's shared event history through the real hub and server, on the
//! demo device.
//!
//! A write by client A shows A's session and name in the record client B reads; a slider
//! sweep is one coalesced record; detaching and attaching record `disconnected` and
//! `connected` and reach a watching session as `event.added`; a watchdog reset from
//! injected `Facts` records `reset` with `watchdog`; a flooded history writes
//! `events_dropped`; `event.list` pages by id; a viewer reads and may not export; the
//! files read with `EventLog::export_in`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_hub::{
    AccessMode, BusMessage, Completion, HistoryConfig, Hub, HubClock, HubConfig, HubHandle,
    NetworkConfig, ServiceConfig, Session, TestClock,
};
use dspi_link::Dir;
use dspi_link::frames::CmdRequest;
use dspi_link::status;
use dspi_proto::ext::{Chip, ResetInfo, ResetReason};
use dspi_proto::generated::opcodes as op;
use dspi_session::events::{EventLog, parse_line};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "P2EV-DEMO";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/wp-P2EV, not tmpfs.
    let dir = std::env::temp_dir().join("wp-P2EV").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn start(dir: &Path, access: AccessMode, history: HistoryConfig) -> Hub {
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    Hub::start(HubConfig {
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        network: Some(NetworkConfig {
            port: 0,
            access,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.to_path_buf())
        }),
        services: ServiceConfig {
            clock,
            history: Some(history),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2EV hub")
    })
    .unwrap()
}

fn volume(hub: &HubHandle, session: &Session, handle: u8, db: f32) -> u8 {
    let (tx, rx) = mpsc::channel();
    hub.submit(
        session,
        CmdRequest {
            tag: 1,
            handle,
            dir: Dir::Set,
            b_request: op::REQ_SET_MASTER_VOLUME,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: 4,
            payload: db.to_le_bytes().to_vec(),
        },
        Completion::new(1, move |r| {
            let _ = tx.send(r);
        }),
    );
    rx.recv_timeout(WAIT).unwrap().status
}

struct Ws {
    ws: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    /// Events read while waiting for an answer.
    events: Vec<Value>,
}

impl Ws {
    fn connect(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        let mut me = Self {
            ws: tungstenite::client(req, tcp).unwrap().0,
            next_id: 100,
            events: Vec::new(),
        };
        me.ws
            .send(Message::text(
                json!({"t": "hello", "proto": {"major": 1, "minor": 2},
                    "client": {"name": "p2ev", "app": "test", "version": "0"}})
                .to_string(),
            ))
            .unwrap();
        let hello = me.next_text();
        assert_eq!(hello["t"], "hello", "{hello}");
        let caps = hello["caps"].as_array().unwrap();
        assert!(caps.iter().any(|c| c == "events"), "{hello}");
        me
    }

    fn next_text(&mut self) -> Value {
        loop {
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    fn ask(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        msg["id"] = id.into();
        self.ws.send(Message::text(msg.to_string())).unwrap();
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {msg}");
            let v = self.next_text();
            if v["id"] == id {
                return v;
            }
            if v.get("id").is_none() {
                self.events.push(v);
            }
        }
    }

    /// The next `event.added` of `kind`.
    fn added(&mut self, kind: &str) -> Value {
        if let Some(i) = self
            .events
            .iter()
            .position(|e| e["t"] == "event.added" && e["record"]["kind"] == kind)
        {
            return self.events.remove(i);
        }
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no event.added {kind}");
            let v = self.next_text();
            if v["t"] == "event.added" && v["record"]["kind"] == kind {
                return v;
            }
        }
    }

    fn records(&mut self, serial: &str) -> Vec<Value> {
        let list = self.ask(json!({"t": "event.list", "serial": serial, "limit": 500}));
        assert_eq!(list["t"], "ok", "{list}");
        assert_eq!(list["v"], 1);
        list["records"].as_array().unwrap().clone()
    }

    fn wait_record(&mut self, serial: &str, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no record: {what}");
            if let Some(r) = self.records(serial).into_iter().find(|r| f(r)) {
                return r;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn kinds(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .map(|r| r["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn wp_p2ev_a_write_by_a_is_attributed_in_what_b_reads_and_a_sweep_is_one_record() {
    let dir = scratch("attribution");
    let hub = start(&dir, AccessMode::Pin, HistoryConfig::default());
    let h = hub.handle();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial(SERIAL)), 0)
        .unwrap();
    let net = hub.network().unwrap();

    // Client A, paired as "Front of house", sweeps the volume.
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    let a = net
        .auth()
        .pair(
            &window.pin,
            "Front of house",
            None,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Instant::now(),
        )
        .unwrap();
    let a = h.open_session(a.role, Some(a.cid)).unwrap();
    let seen = h.bus().subscribe("test", 4096);
    for i in 0..20 {
        assert_eq!(volume(&h, &a, handle, -40.0 + i as f32), status::OK);
    }
    // The sweep's changes are on the bus (the history's queue has them too): wait until
    // they stop, then give its thread a moment before B's read writes the record out.
    let mut heard = 0u32;
    let mut quiet_since = Instant::now();
    let start = Instant::now();
    while heard == 0 || quiet_since.elapsed() < Duration::from_millis(500) {
        assert!(start.elapsed() < WAIT, "no notifications of the sweep");
        if let Some(m) = seen.recv_timeout(Duration::from_millis(50))
            && let BusMessage::Notification {
                session, packet, ..
            } = &*m
            && *session == a.id()
            && packet.get(1) == Some(&2)
        {
            heard += 1;
            quiet_since = Instant::now();
        }
    }
    std::thread::sleep(Duration::from_millis(300));

    // Client B, a viewer on the socket, reads the history.
    let mut b = Ws::connect(net.port());
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    let paired =
        b.ask(json!({"t": "auth.pair", "pin": pin, "name": "Back office", "role": "viewer"}));
    assert_eq!(paired["t"], "ok", "{paired}");
    let records = b.records(SERIAL);
    assert_eq!(records[0]["kind"], "connected", "{records:?}");
    let by_a: Vec<&Value> = records
        .iter()
        .filter(|r| r["session"] == u64::from(a.id()))
        .collect();
    assert_eq!(by_a.len(), 1, "one coalesced record: {by_a:?}");
    let sweep = by_a[0];
    assert_eq!(sweep["kind"], "notification");
    assert_eq!(sweep["by"], "Front of house");
    assert_eq!(sweep["source"], "HOST");
    assert_eq!(
        sweep["count"].as_u64().unwrap_or(1),
        u64::from(heard),
        "every change of the sweep: {sweep}"
    );
    assert!(heard >= 2, "a sweep: {heard}");
    assert_eq!(sweep["serial"], SERIAL);

    // A viewer may not export.
    let export = b.ask(json!({"t": "event.export", "serial": SERIAL}));
    assert_eq!(export["code"], "denied", "{export}");
    // An unknown device, a bad request.
    let unknown = b.ask(json!({"t": "event.list", "serial": "NOT-SEEN"}));
    assert_eq!(unknown["code"], "not_found", "{unknown}");
    let bad = b.ask(json!({"t": "event.list"}));
    assert_eq!(bad["code"], "bad_request", "{bad}");
    let no_device = b.ask(json!({"t": "event.list", "handle": 200}));
    assert_eq!(no_device["code"], "no_device", "{no_device}");
    // By handle, too.
    let by_handle = b.ask(json!({"t": "event.list", "handle": handle}));
    assert_eq!(by_handle["serial"], SERIAL, "{by_handle}");

    // B watches: a detach and an attach reach it as they are written.
    let watch = b.ask(json!({"t": "event.watch", "on": true, "serial": SERIAL}));
    assert_eq!(watch["on"], true, "{watch}");
    hub.detach(handle).unwrap();
    let gone = b.added("disconnected");
    assert_eq!(gone["v"], 1);
    assert_eq!(gone["hub"], hub.hub_id());
    assert_eq!(gone["serial"], SERIAL);
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial(SERIAL)), 0)
        .unwrap();
    let back = b.added("connected");
    assert!(
        back["record"]["id"].as_u64() > gone["record"]["id"].as_u64(),
        "{back} after {gone}"
    );
    let off = b.ask(json!({"t": "event.watch", "on": false}));
    assert_eq!(off["on"], false);
    let tail: Vec<String> = kinds(&b.records(SERIAL))
        .into_iter()
        .rev()
        .take(2)
        .collect();
    assert_eq!(tail, ["connected", "disconnected"]);

    // The files are the event log's own: a client's reader reads them unchanged.
    let text = EventLog::export_in(&dir.join("events"), SERIAL);
    let lines: Vec<_> = text.lines().map(|l| parse_line(l).unwrap()).collect();
    assert!(
        lines
            .iter()
            .any(|r| r.by.as_deref() == Some("Front of house"))
    );
    assert_eq!(lines.len(), b.records(SERIAL).len());
    let _ = handle;
    drop(b);
    hub.stop();
}

fn reset_facts(reason: ResetReason, uptime_s: u32) -> BusMessage {
    BusMessage::Facts {
        serial: SERIAL.into(),
        identify: None,
        reset_info: Some(ResetInfo {
            reason,
            hang_count: 1,
            chip: Chip::Rp2350,
            uptime_s,
            prev_uptime_s: Some(86_400),
            chip_reset: 0,
            watchdog_reason: 0x100,
        }),
        fw: "1.1.7".into(),
    }
}

#[test]
fn wp_p2ev_a_watchdog_reset_is_recorded_and_list_and_export_page_by_id() {
    let dir = scratch("reset");
    let hub = start(&dir, AccessMode::Open, HistoryConfig::default());
    let h = hub.handle();
    hub.attach(Box::new(FakeDevice::demo().with_serial(SERIAL)), 0)
        .unwrap();
    let mut admin = Ws::connect(hub.network().unwrap().port());
    admin.wait_record(SERIAL, "connected", |r| r["kind"] == "connected");

    // P2HP's facts, injected: a watchdog reset.
    h.bus().publish(reset_facts(ResetReason::Watchdog, 9));
    let reset = admin.wait_record(SERIAL, "the reset", |r| r["kind"] == "reset");
    assert_eq!(reset["source"], "watchdog", "{reset}");
    assert_eq!(reset["uptime_s"], 9);
    // The same facts again (the device did not restart): no second record.
    h.bus().publish(reset_facts(ResetReason::Watchdog, 9));
    for since in 1..=40u64 {
        h.bus().publish(BusMessage::Unresponsive {
            serial: SERIAL.into(),
            since,
        });
    }
    let records = loop {
        let records = admin.records(SERIAL);
        if records
            .iter()
            .filter(|r| r["kind"] == "unresponsive")
            .count()
            == 40
        {
            break records;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        records.iter().filter(|r| r["kind"] == "reset").count(),
        1,
        "{records:?}"
    );

    // Paging by id.
    let newest = records.last().unwrap()["id"].as_u64().unwrap();
    // A first read is the newest page.
    let first = admin.ask(json!({"t": "event.list", "serial": SERIAL, "limit": 7}));
    assert_eq!(first["records"].as_array().unwrap().len(), 7);
    assert_eq!(first["records"][6]["id"], newest);
    assert_eq!(first["more"], false);
    // Paging from the oldest id.
    // (`after_id` 0 is a first read, so the oldest record starts the pages.)
    let mut after = records[0]["id"].as_u64().unwrap();
    let mut paged = vec![records[0].clone()];
    loop {
        let page =
            admin.ask(json!({"t": "event.list", "serial": SERIAL, "after_id": after, "limit": 7}));
        assert_eq!(page["newest_id"], newest, "{page}");
        let rs = page["records"].as_array().unwrap();
        assert!(rs.len() <= 7);
        paged.extend(rs.iter().cloned());
        if page["more"] == false {
            break;
        }
        after = rs.last().unwrap()["id"].as_u64().unwrap();
    }
    assert_eq!(paged, records);
    let past = admin.ask(json!({"t": "event.list", "serial": SERIAL, "after_id": newest + 50}));
    assert!(past["records"].as_array().unwrap().is_empty());
    assert_eq!(
        past["newest_id"], newest,
        "a client past the newest id sees it"
    );

    // An admin exports, in pages of whole records, the file's own text.
    let export = admin.ask(json!({"t": "event.export", "serial": SERIAL}));
    assert_eq!(export["t"], "ok", "{export}");
    assert_eq!(export["next_after_id"], Value::Null);
    assert_eq!(
        export["text"],
        EventLog::export_in(&dir.join("events"), SERIAL)
    );
    let tail = admin.ask(json!({"t": "event.export", "serial": SERIAL, "after_id": newest - 2}));
    assert_eq!(tail["text"].as_str().unwrap().lines().count(), 2);
    drop(admin);
    hub.stop();
}

#[test]
fn wp_p2ev_a_flooded_history_writes_events_dropped() {
    let dir = scratch("flood");
    let hold = Arc::new(AtomicBool::new(false));
    let hub = start(
        &dir,
        AccessMode::Open,
        HistoryConfig {
            bus_capacity: 4,
            test_hold: Some(Arc::clone(&hold)),
            ..HistoryConfig::default()
        },
    );
    let h = hub.handle();
    hub.attach(Box::new(FakeDevice::demo().with_serial(SERIAL)), 0)
        .unwrap();
    let mut ws = Ws::connect(hub.network().unwrap().port());
    ws.wait_record(SERIAL, "connected", |r| r["kind"] == "connected");

    hold.store(true, Ordering::SeqCst);
    // The thread may be inside one wait on the bus; after it, it holds.
    std::thread::sleep(Duration::from_millis(400));
    for since in 0..100 {
        h.bus().publish(BusMessage::Unresponsive {
            serial: SERIAL.into(),
            since,
        });
    }
    hold.store(false, Ordering::SeqCst);
    ws.wait_record(SERIAL, "events_dropped", |r| r["kind"] == "events_dropped");
    // Every message is either written or counted lost. (Counted lost can also be the
    // history's own `Event` echoes, which share its queue: at most once each.)
    let start = Instant::now();
    let (records, lost) = loop {
        assert!(start.elapsed() < WAIT, "not every message is accounted for");
        let records = ws.records(SERIAL);
        let kept = records
            .iter()
            .filter(|r| r["kind"] == "unresponsive")
            .count() as u64;
        let lost: u64 = records
            .iter()
            .filter(|r| r["kind"] == "events_dropped")
            .map(|r| r["count"].as_u64().unwrap_or(1))
            .sum();
        if kept + lost >= 100 {
            break (records, lost);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(lost >= 90, "{records:?}");
    // The loss is recorded before what was kept after it.
    let at = records
        .iter()
        .position(|r| r["kind"] == "events_dropped")
        .unwrap();
    assert!(
        records[at + 1..]
            .iter()
            .all(|r| r["kind"] == "unresponsive" || r["kind"] == "events_dropped")
    );
    drop(ws);
    hub.stop();
}
