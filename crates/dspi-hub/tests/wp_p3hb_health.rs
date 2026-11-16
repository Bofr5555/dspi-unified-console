//! WP P3HB: the health board's facts on the demo device, through the real hub and, for
//! `health.get`, the real server.
//!
//! The stream counters (underruns added up over PDM ring, PDM DMA and S/PDIF, and the
//! starvation total) step when the device steps them and cost exactly their one read a
//! period; a device that sends extension clip events (0xE0) counts ten in a minute and
//! forgets them after, one with only the latch has no `clip_per_min`; `last_change`
//! names the client that wrote; the hub writes nothing.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use dspi_hub::{
    AccessMode, BusMessage, Completion, HealthConfig, HistoryConfig, Hub, HubConfig, NetworkConfig,
    ServiceConfig,
};
use dspi_link::Dir;
use dspi_link::frames::CmdRequest;
use dspi_link::status;
use dspi_proto::ext::NOTIFY_EVT_EXT_CLIP;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource, Transport,
    TransportCaps,
};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/<branch>, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3hb").join(format!(
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
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `(bRequest, wValue, OUT, when)`.
type Transfer = (u8, u16, bool, Instant);

/// The device's two stream counters, as a test sets them, and every transfer made.
#[derive(Clone, Default)]
struct Probe {
    log: Arc<Mutex<Vec<Transfer>>>,
    underruns: Arc<AtomicU32>,
    starvation: Arc<AtomicU32>,
}

impl Probe {
    /// The transfers in `[from, to)` that `f` takes.
    fn count(&self, from: Instant, to: Instant, f: impl Fn(u8, u16, bool) -> bool) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.3 >= from && t.3 < to && f(t.0, t.1, t.2))
            .count()
    }
}

struct Dev {
    inner: FakeDevice,
    probe: Probe,
}

impl Dev {
    fn log(&self, o: u8, v: u16, out: bool) {
        self.probe
            .log
            .lock()
            .unwrap()
            .push((o, v, out, Instant::now()));
    }

    /// The counters the test sets, in place of the fake's quiet zeros.
    fn counter(&self, o: u8, v: u16) -> Option<Vec<u8>> {
        let c = match (o, v) {
            (op::REQ_GET_STATUS, 8) => &self.probe.underruns,
            (op::REQ_GET_STATUS, 17) => &self.probe.starvation,
            _ => return None,
        };
        Some(c.load(Ordering::SeqCst).to_le_bytes().to_vec())
    }
}

impl Transport for Dev {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.log(o, v, false);
        match self.counter(o, v) {
            Some(b) => Ok(b),
            None => self.inner.control_in(o, v, l),
        }
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.log(o, v, false);
        match self.counter(o, v) {
            Some(b) => Ok(b),
            None => self.inner.control_in_upto(o, v, l),
        }
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.log(o, v, true);
        self.inner.control_out(o, v, d)
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

/// The fork's firmware with identify, reset info and the clip event (wp/FWX).
fn with_clip_events(serial: &str) -> FakeDevice {
    FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            diag_ext: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(serial)
}

fn fast() -> HealthConfig {
    HealthConfig {
        status_every: Duration::from_millis(200),
        temperature_every: Duration::from_millis(300),
        reset_info_every: Duration::from_millis(300),
        unresponsive_after: Duration::from_millis(1500),
        retry_after: Duration::from_millis(300),
        counters_every: Duration::from_millis(300),
        clip_window: Duration::from_secs(60),
    }
}

/// One extension clip event of channel `ch`, as the firmware sends it.
fn clip_packet(ch: u8) -> Vec<u8> {
    let mut p = vec![2, NOTIFY_EVT_EXT_CLIP, 0, 0, ch, 1, 0, 0];
    p.extend((1u32 << ch).to_le_bytes());
    p
}

struct Rig {
    hub: Hub,
    ws: Ws,
    next_id: u64,
}

impl Rig {
    fn start(name: &str, health: HealthConfig, history: bool) -> Self {
        let dir = scratch(name);
        let hub = Hub::start(HubConfig {
            network: Some(NetworkConfig {
                port: 0,
                access: AccessMode::Open,
                bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
                ..NetworkConfig::new(dir)
            }),
            initial_scan: false,
            offline_grace: Duration::from_secs(60),
            services: ServiceConfig {
                health: Some(health),
                history: history.then(HistoryConfig::default),
                ..ServiceConfig::default()
            },
            ..HubConfig::in_process("P3HB hub")
        })
        .unwrap();
        let mut ws = Ws::connect(hub.network().unwrap().port());
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "p3hb", "app": "test", "version": "0"}}));
        ws.next("hello");
        Self {
            hub,
            ws,
            next_id: 1,
        }
    }

    fn attach(&self, dev: FakeDevice) -> (u8, Probe, FakeRemote) {
        let probe = Probe::default();
        let remote = dev.remote();
        let handle = self
            .hub
            .attach(
                Box::new(Dev {
                    inner: dev,
                    probe: probe.clone(),
                }),
                0,
            )
            .unwrap();
        (handle, probe, remote)
    }

    /// `health.get {serial}`'s one record; null while the device is not online in it.
    fn health(&mut self, serial: &str) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let a = self
            .ws
            .ask(json!({"t": "health.get", "serial": serial}), id);
        // Attached but not yet in the health service: nothing to read yet.
        if a["code"] == "offline" {
            return Value::Null;
        }
        assert_eq!(a["t"], "ok", "{a}");
        a["devices"][0].clone()
    }
}

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

// ── Counters ────────────────────────────────────────────────────────────────

#[test]
fn wp_p3hb_counters_step_when_the_fake_steps_them() {
    let mut rig = Rig::start("counters", fast(), false);
    let bus = rig.hub.bus().subscribe("p3hb-test", 4096);
    // The fake's PDM counters (4, 6) read 0, so the sum is the S/PDIF counter here.
    let (_, probe, _) = rig.attach(with_clip_events("P3HB-CNT"));
    probe.underruns.store(3, Ordering::SeqCst);
    probe.starvation.store(4, Ordering::SeqCst);
    wait_until("the first counters", || {
        rig.health("P3HB-CNT")["counters"] == json!({"underruns": 3, "starvation": 4})
    });
    probe.underruns.store(10, Ordering::SeqCst);
    probe.starvation.store(12, Ordering::SeqCst);
    wait_until("the counters stepped", || {
        rig.health("P3HB-CNT")["counters"] == json!({"underruns": 10, "starvation": 12})
    });
    // And in the bus's Health, for the alarms.
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no Health with the new counters");
        let Some(m) = bus.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        if let BusMessage::Health {
            serial,
            counters: Some(c),
            ..
        } = &*m
            && serial == "P3HB-CNT"
            && (c.underruns, c.starvation) == (Some(10), Some(12))
        {
            break;
        }
    }
    rig.hub.stop();
}

#[test]
fn wp_p3hb_the_counters_cost_exactly_their_one_read_a_period() {
    let mut rig = Rig::start("budget", fast(), false);
    let (_, probe, _) = rig.attach(with_clip_events("P3HB-BUD"));
    // Past the attach reads.
    wait_until("running", || {
        rig.health("P3HB-BUD").get("counters").is_some()
    });
    std::thread::sleep(Duration::from_millis(500));
    let from = Instant::now();
    std::thread::sleep(Duration::from_secs(3));
    let to = Instant::now();
    let counters = probe.count(from, to, |o, v, _| {
        o == op::REQ_GET_STATUS && matches!(v, 4 | 6 | 8 | 17)
    });
    // One every 300 ms: ten in 3 s, give or take the edges.
    assert!(
        (8..=12).contains(&counters),
        "{counters} counter reads in 3 s"
    );
    // Every other transfer is one P2HP already made: the meter poll, the temperature
    // and the reset record. Nothing else was added, and nothing was written.
    let others = probe.count(from, to, |o, v, out| {
        out || !matches!(
            (o, v),
            (op::REQ_GET_STATUS, 4 | 6 | 8 | 9 | 16 | 17)
                | (
                    op::REQ_EXTENSION,
                    dspi_proto::generated::ext::EXT_RESET_INFO
                )
        )
    });
    assert_eq!(others, 0, "only P2HP's reads and the counter read");
    rig.hub.stop();
}

// ── Clip per minute ─────────────────────────────────────────────────────────

#[test]
fn wp_p3hb_ten_clip_events_are_counted_and_forgotten_after_the_window() {
    let health = HealthConfig {
        clip_window: Duration::from_millis(2500),
        ..fast()
    };
    let mut rig = Rig::start("clips", health, false);
    let (_, probe, remote) = rig.attach(with_clip_events("P3HB-CLIP"));
    // A latch-only device beside it: the extension channel without the clip event.
    let (_, _, latch) = rig.attach(FakeDevice::demo().with_serial("P3HB-LATCH"));
    wait_until("the features read", || {
        rig.health("P3HB-CLIP")["clip_per_min"] == 0
            && rig.health("P3HB-LATCH").get("cpu_pct").is_some()
    });
    for i in 0..10 {
        remote.push(clip_packet(i % 4));
        latch.push(clip_packet(i % 4));
    }
    wait_until("ten clip events", || {
        rig.health("P3HB-CLIP")["clip_per_min"] == 10
    });
    let r = rig.health("P3HB-LATCH");
    assert!(r.get("clip_per_min").is_none(), "latch only: {r}");
    // Forgotten after the window.
    wait_until("the window passed", || {
        rig.health("P3HB-CLIP")["clip_per_min"] == 0
    });
    assert!(rig.health("P3HB-LATCH").get("clip_per_min").is_none());
    let any = Instant::now() - Duration::from_secs(3600);
    assert_eq!(
        probe.count(any, Instant::now() + WAIT, |_, _, out| out),
        0,
        "the hub wrote nothing"
    );
    rig.hub.stop();
}

// ── Last change ─────────────────────────────────────────────────────────────

#[test]
fn wp_p3hb_last_change_names_the_writing_client() {
    let mut rig = Rig::start("change", fast(), true);
    let (handle, _, _) = rig.attach(FakeDevice::demo().with_serial("P3HB-CHG"));
    wait_until("a status", || {
        rig.health("P3HB-CHG").get("cpu_pct").is_some()
    });
    let net = rig.hub.network().unwrap();
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
    let h = rig.hub.handle();
    let a = h.open_session(a.role, Some(a.cid)).unwrap();
    let (tx, rx) = mpsc::channel();
    h.submit(
        &a,
        CmdRequest {
            tag: 1,
            handle,
            dir: Dir::Set,
            b_request: op::REQ_SET_MASTER_VOLUME,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: 4,
            payload: (-23.0f32).to_le_bytes().to_vec(),
        },
        Completion::new(1, move |r| {
            let _ = tx.send(r.status);
        }),
    );
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), status::OK);
    // The history writes the record once its coalescing window has passed.
    wait_until("the last change", || {
        rig.health("P3HB-CHG")["last_change"]["by"] == "Front of house"
    });
    let lc = rig.health("P3HB-CHG")["last_change"].clone();
    assert!(lc["at"].as_str().unwrap().ends_with('Z'), "{lc}");
    assert!(!lc["text"].as_str().unwrap().is_empty(), "{lc}");
    rig.hub.stop();
}
