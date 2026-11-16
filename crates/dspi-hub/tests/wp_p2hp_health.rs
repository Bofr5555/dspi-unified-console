//! WP P2HP: the health poll and device facts, on the demo device through the real hub
//! and, for `health.get`, the real server.
//!
//! The facts of a device with and without the extension IDs; a console's meter poll and
//! the hub's are one read (counted at the transport); the hub never sends
//! `REQ_CLEAR_CLIPS` (a transfer log over a whole run, and its source never names it
//! outside the router's classification); the poll stops with the device; a device whose
//! reads time out is reported unresponsive once and recovers; a reset between two reads
//! gives new facts.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::{
    AccessMode, BusMessage, HealthConfig, Hub, HubConfig, NetworkConfig, Role, ServiceConfig,
    Subscription,
};
use dspi_link::messages::PollSpec;
use dspi_proto::ext::ResetReason;
use dspi_proto::generated::ext as gext;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::SystemStatus;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource, Transport,
    TransportCaps, TransportError,
};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/<branch>, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2hp").join(format!(
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

/// One transfer: `(bRequest, wValue, wLength or the payload's length, OUT)`.
type Transfer = (u8, u16, u16, bool);

/// Every transfer the hub made, and a switch that makes them time out.
#[derive(Clone, Default)]
struct Probe {
    /// `(bRequest, wValue, wLength or the payload's length, OUT)`.
    log: Arc<Mutex<Vec<Transfer>>>,
    hang: Arc<AtomicBool>,
}

impl Probe {
    fn count(&self, f: impl Fn(&Transfer) -> bool) -> usize {
        self.log.lock().unwrap().iter().filter(|t| f(t)).count()
    }

    fn meter_reads(&self, len: u16) -> usize {
        self.count(|t| *t == (op::REQ_GET_STATUS, 9, len, false))
    }

    fn check(&self, opcode: u8, value: u16, len: usize, out: bool) -> dspi_transport::Result<()> {
        let len = u16::try_from(len).unwrap_or(u16::MAX);
        self.log.lock().unwrap().push((opcode, value, len, out));
        if self.hang.load(Ordering::SeqCst) {
            // As a transport that already ran its own retries: returned at once.
            return Err(TransportError::TimedOut { opcode, retries: 1 });
        }
        Ok(())
    }
}

struct Dev {
    inner: FakeDevice,
    probe: Probe,
}

impl Transport for Dev {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.probe.check(o, v, usize::from(l), false)?;
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.probe.check(o, v, usize::from(l), false)?;
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.probe.check(o, v, d.len(), true)?;
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

/// The fork's firmware with identify and reset info (wp/FWX, `fake/diag.rs`).
fn with_ids(serial: &str) -> FakeDevice {
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

/// Stock Weeb Labs firmware: no extension channel, `EXT_LIST` stalls.
fn stock(serial: &str) -> FakeDevice {
    FakeDevice::new(Capture::demo(), FakeOptions::default()).with_serial(serial)
}

fn fast() -> HealthConfig {
    HealthConfig {
        status_every: Duration::from_millis(200),
        temperature_every: Duration::from_millis(300),
        reset_info_every: Duration::from_millis(300),
        unresponsive_after: Duration::from_millis(1500),
        retry_after: Duration::from_millis(300),
        // -- WP P3HB --
        counters_every: Duration::from_millis(300),
        clip_window: Duration::from_secs(60),
    }
}

/// A hub with the health service, on a loopback port with open access.
struct Rig {
    hub: Hub,
    bus: Subscription,
    ws: Ws,
    next_id: u64,
    // -- WP FLAKE4 -- bus messages from another device than the one waited for, kept
    // for its own wait: Facts come once per device, and dropping one lost it for good.
    others: Mutex<Vec<Arc<BusMessage>>>,
}

impl Rig {
    fn start(name: &str, health: HealthConfig) -> Self {
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
                ..ServiceConfig::default()
            },
            ..HubConfig::in_process("P2HP hub")
        })
        .unwrap();
        let bus = hub.bus().subscribe("p2hp-test", 4096);
        let mut ws = Ws::connect(hub.network().unwrap().port());
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "p2hp", "app": "test", "version": "0"}}));
        let hello = ws.next("hello");
        assert!(
            hello["caps"].as_array().unwrap().contains(&json!("health")),
            "{hello}"
        );
        Self {
            hub,
            bus,
            ws,
            next_id: 1,
            others: Mutex::new(Vec::new()),
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

    fn ask(&mut self, msg: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.ws.ask(msg, id)
    }

    /// `health.get {serial}`'s one record.
    fn health(&mut self, serial: &str) -> Value {
        let a = self.ask(json!({"t": "health.get", "serial": serial}));
        assert_eq!(a["t"], "ok", "{a}");
        assert_eq!(a["v"], 1);
        a["devices"][0].clone()
    }

    /// The next bus message from `serial` that `pick` takes. Messages from other
    /// devices are kept for their own wait; `serial`'s others are passed over.
    fn next<T>(&self, serial: &str, mut pick: impl FnMut(&BusMessage) -> Option<T>) -> T {
        // -- WP FLAKE4 -- first what an earlier wait for another device kept.
        {
            let mut kept = self.others.lock().unwrap();
            let mut found = None;
            kept.retain(|m| {
                if found.is_some() || from(m) != Some(serial) {
                    return true;
                }
                found = pick(m);
                false
            });
            if let Some(t) = found {
                return t;
            }
        }
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "nothing in time from {serial}");
            let Some(m) = self.bus.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            match from(&m) {
                Some(s) if s == serial => {
                    if let Some(t) = pick(&m) {
                        return t;
                    }
                }
                Some(_) => self.others.lock().unwrap().push(m),
                None => {}
            }
        }
    }

    fn facts(&self, serial: &str) -> BusMessage {
        self.next(serial, |m| {
            matches!(m, BusMessage::Facts { .. }).then(|| m.clone())
        })
    }

    fn health_msg(&self, serial: &str) -> BusMessage {
        self.next(serial, |m| {
            matches!(m, BusMessage::Health { .. }).then(|| m.clone())
        })
    }
}

fn from(m: &BusMessage) -> Option<&str> {
    match m {
        BusMessage::Facts { serial, .. }
        | BusMessage::Health { serial, .. }
        | BusMessage::Unresponsive { serial, .. }
        | BusMessage::DeviceOnline { serial, .. }
        | BusMessage::DeviceOffline { serial, .. }
        | BusMessage::DeviceRemoved { serial, .. }
        | BusMessage::Notification { serial, .. } => Some(serial),
        _ => None,
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

/// The demo's meter block (17 channels) with these loads and this latch.
fn meters(cpu0: u8, cpu1: u8, clip: u32) -> Vec<u8> {
    SystemStatus {
        peaks: vec![100; 17],
        cpu0,
        cpu1,
        clip_flags: clip,
        active_inputs: 2,
    }
    .encode()
}

// ── Facts ───────────────────────────────────────────────────────────────────

#[test]
fn wp_p2hp_the_facts_of_a_device_with_the_ext_ids() {
    let mut rig = Rig::start("ids", fast());
    let (_, probe, remote) = rig.attach(with_ids("P2HP-IDS"));
    remote.set_status(meters(31, 47, 0));
    let BusMessage::Facts {
        identify,
        reset_info,
        fw,
        ..
    } = rig.facts("P2HP-IDS")
    else {
        unreachable!()
    };
    let id = identify.expect("EXT_IDENTIFY is listed");
    assert_eq!(id.board, "pico2");
    assert_eq!(id.hw_rev, dspi_transport::fake::FAKE_HW_REV);
    let reset = reset_info.expect("EXT_RESET_INFO is listed");
    assert_eq!(reset.reason, ResetReason::PowerOn);
    assert!(!fw.is_empty());
    // Reads, nothing else: no OUT transfer at all from the health service's run.
    wait_until("a full record", || {
        let r = rig.health("P2HP-IDS");
        r.get("temp_c").is_some() && r.get("cpu_pct") == Some(&json!(47))
    });
    let r = rig.health("P2HP-IDS");
    assert_eq!(r["temp_c"], 35.0, "{r}");
    assert_eq!(r["clip_mask"], 0);
    assert_eq!(r["reset"]["reason"], "power_on");
    assert_eq!(r["unresponsive"], false);
    assert!(r["uptime_s"].is_u64() && r["hang_count"].is_u64(), "{r}");
    assert!(r["at"].as_str().unwrap().ends_with('Z'), "{r}");
    assert_eq!(probe.count(|t| t.3), 0, "the hub wrote nothing");
    rig.hub.stop();
}

#[test]
fn wp_p2hp_the_facts_of_a_device_without_the_ext_ids() {
    let mut rig = Rig::start("noids", fast());
    // The extension channel without identify and reset info (bofr-main 04122d3), and
    // stock firmware, where EXT_LIST stalls.
    let (_, demo, _) = rig.attach(FakeDevice::demo().with_serial("P2HP-DEMO"));
    let (_, plain, _) = rig.attach(stock("P2HP-STOCK"));
    for serial in ["P2HP-DEMO", "P2HP-STOCK"] {
        let BusMessage::Facts {
            identify,
            reset_info,
            ..
        } = rig.facts(serial)
        else {
            unreachable!()
        };
        assert_eq!((identify, reset_info), (None, None), "{serial}");
        wait_until("a status", || rig.health(serial).get("cpu_pct").is_some());
        let r = rig.health(serial);
        assert!(
            r.get("reset").is_none() && r.get("uptime_s").is_none(),
            "{r}"
        );
    }
    std::thread::sleep(Duration::from_millis(700));
    for p in [&demo, &plain] {
        let asked = |id: u16| p.count(|t| t.0 == op::REQ_EXTENSION && t.1 == id);
        assert_eq!(asked(gext::EXT_LIST), 1, "the list once");
        assert_eq!(asked(gext::EXT_IDENTIFY), 0, "nothing not listed");
        assert_eq!(asked(gext::EXT_RESET_INFO), 0, "nothing not listed");
    }
    // health.get without a serial: both.
    let all = rig.ask(json!({"t": "health.get"}));
    assert_eq!(all["devices"].as_array().unwrap().len(), 2, "{all}");
    rig.hub.stop();
}

// ── One group ───────────────────────────────────────────────────────────────

#[test]
fn wp_p2hp_a_consoles_meter_poll_and_the_health_poll_are_one_group() {
    // The real periods: the hub's status poll is 1 Hz.
    let mut rig = Rig::start("group", HealthConfig::default());
    let (handle, probe, remote) = rig.attach(with_ids("P2HP-GRP"));
    remote.set_status(meters(20, 25, 0));
    rig.health_msg("P2HP-GRP");
    // A console's meter poll, as dspi-core asks for it (len = num_channels * 2 + 7).
    let console = rig.hub.open_session(Role::Viewer, None).unwrap();
    let spec = PollSpec {
        slot: 1,
        req: op::REQ_GET_STATUS,
        val: 9,
        idx: dspi_proto::VENDOR_INTERFACE,
        len: 41,
        hz: 2,
    };
    let granted = rig.hub.poll_subscribe(&console, handle, &[spec]).unwrap();
    assert_eq!(granted[0].hz, 2);
    std::thread::sleep(Duration::from_millis(600));
    let before = probe.meter_reads(41);
    let window = Duration::from_secs(4);
    std::thread::sleep(window);
    let reads = probe.meter_reads(41) - before;
    // Two groups would be 2 Hz + 1 Hz = 12 reads; one group at 2 Hz is 8.
    assert!((6..=10).contains(&reads), "{reads} reads in 4 s");
    // The hub still had its readings in that time, and the console its frames.
    rig.health_msg("P2HP-GRP");
    let mut frames = 0;
    while let dspi_hub::session::Pop::Item(item) = console.outbox().pop(Duration::ZERO) {
        frames += usize::from(matches!(item, dspi_hub::Outbound::Poll { .. }));
    }
    assert!(frames >= 1);
    let r = rig.health("P2HP-GRP");
    assert_eq!(r["cpu_pct"], 25, "{r}");
    rig.hub.close_session(console.id());
    rig.hub.stop();
}

// ── The clip latch ──────────────────────────────────────────────────────────

#[test]
fn wp_p2hp_the_hub_never_sends_clear_clips_over_a_whole_run() {
    let mut rig = Rig::start("clips", fast());
    let (handle, probe, remote) = rig.attach(with_ids("P2HP-CLIP"));
    remote.set_status(meters(10, 10, 0b1001));
    // A console polls the meters too, as a hub with a console open does.
    let console = rig.hub.open_session(Role::Viewer, None).unwrap();
    let spec = PollSpec {
        slot: 0,
        req: op::REQ_GET_STATUS,
        val: 9,
        idx: dspi_proto::VENDOR_INTERFACE,
        len: 41,
        hz: 10,
    };
    rig.hub.poll_subscribe(&console, handle, &[spec]).unwrap();
    let clip = rig.next("P2HP-CLIP", |m| match m {
        BusMessage::Health { clip_mask, .. } if *clip_mask != 0 => Some(*clip_mask),
        _ => None,
    });
    assert_eq!(clip, 0b1001);
    // Long enough for every period to come round several times.
    std::thread::sleep(Duration::from_secs(2));
    rig.hub.close_session(console.id());
    let r = rig.health("P2HP-CLIP");
    assert_eq!(r["clip_mask"], 0b1001, "the latch is still set: {r}");
    rig.hub.detach(handle).unwrap();
    rig.hub.stop();
    assert!(probe.meter_reads(41) > 5, "it did poll");
    assert_eq!(
        probe.count(|t| t.0 == op::REQ_CLEAR_CLIPS),
        0,
        "the clip latch is every client's: the hub only reads it"
    );
    assert_eq!(probe.count(|t| t.3), 0, "and wrote nothing at all");
}

/// Every `.rs` file under `dir`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn wp_p2hp_the_hubs_source_names_clear_clips_only_in_the_routers_classification() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    assert!(files.len() > 20, "found the sources: {files:?}");
    let name = ["CLEAR", "_CLIPS"].concat();
    let mut named = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.contains(&name) {
                named.push(format!(
                    "{}:{}: {}",
                    f.strip_prefix(&src).unwrap().display(),
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
    // router.rs lists it among the reads that change device state (judged as control).
    assert!(
        named.iter().all(|l| l.starts_with("router.rs:")),
        "named outside the router: {named:#?}"
    );
    assert_eq!(named.len(), 1, "{named:#?}");
}

// ── The device goes ─────────────────────────────────────────────────────────

#[test]
fn wp_p2hp_the_poll_stops_with_the_device_and_starts_again_with_it() {
    let mut rig = Rig::start("stops", fast());
    let (handle, probe, _) = rig.attach(with_ids("P2HP-GONE"));
    rig.health_msg("P2HP-GONE");
    rig.hub.detach(handle).unwrap();
    rig.next("P2HP-GONE", |m| {
        matches!(m, BusMessage::DeviceOffline { .. }).then_some(())
    });
    let reads = probe.count(|_| true);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(probe.count(|_| true), reads, "nothing more was read");
    // No Health after it went.
    let late: Vec<_> = rig
        .bus
        .drain()
        .into_iter()
        .filter(|m| from(m) == Some("P2HP-GONE") && matches!(**m, BusMessage::Health { .. }))
        .collect();
    assert!(late.len() <= 1, "at most one already on its way: {late:?}");
    // Known but not attached: `offline`; the list leaves it out.
    let a = rig.ask(json!({"t": "health.get", "serial": "P2HP-GONE"}));
    assert_eq!(a["code"], "offline", "{a}");
    let all = rig.ask(json!({"t": "health.get"}));
    assert_eq!(all["devices"], json!([]), "{all}");
    // Back: facts and the poll again, on the new attachment.
    let (_, again, _) = rig.attach(with_ids("P2HP-GONE"));
    rig.facts("P2HP-GONE");
    rig.health_msg("P2HP-GONE");
    assert!(again.meter_reads(41) >= 1);
    rig.hub.stop();
}

// ── Unresponsive ────────────────────────────────────────────────────────────

#[test]
fn wp_p2hp_a_device_that_times_out_is_unresponsive_once_and_recovers() {
    let mut rig = Rig::start("hung", fast());
    let (_, probe, _) = rig.attach(with_ids("P2HP-HUNG"));
    rig.health_msg("P2HP-HUNG");
    probe.hang.store(true, Ordering::SeqCst);
    let hung_at = Instant::now();
    let since = rig.next("P2HP-HUNG", |m| match m {
        BusMessage::Unresponsive { since, .. } => Some(*since),
        _ => None,
    });
    assert!(
        hung_at.elapsed() >= Duration::from_millis(1200),
        "not before its time"
    );
    assert!(since > 1_700_000_000);
    let r = rig.health("P2HP-HUNG");
    assert_eq!(r["unresponsive"], true, "{r}");
    assert!(r["unresponsive_since"].is_string(), "{r}");
    // Still silent for a while: no second report.
    std::thread::sleep(Duration::from_secs(2));
    let again = rig
        .bus
        .drain()
        .into_iter()
        .filter(|m| matches!(**m, BusMessage::Unresponsive { .. }))
        .count();
    assert_eq!(again, 0, "reported once");
    // It answers again.
    probe.hang.store(false, Ordering::SeqCst);
    rig.health_msg("P2HP-HUNG");
    wait_until("recovered", || {
        rig.health("P2HP-HUNG")["unresponsive"] == false
    });
    std::thread::sleep(Duration::from_millis(500));
    let after = rig
        .bus
        .drain()
        .into_iter()
        .filter(|m| matches!(**m, BusMessage::Unresponsive { .. }))
        .count();
    assert_eq!(after, 0);
    rig.hub.stop();
}

// ── A reset between reads ───────────────────────────────────────────────────

#[test]
fn wp_p2hp_a_reset_between_two_reads_gives_new_facts() {
    let mut rig = Rig::start("reset", fast());
    let (_, _, remote) = rig.attach(with_ids("P2HP-RST"));
    rig.facts("P2HP-RST");
    remote.vreg_sim(dspi_transport::fake::VregSim::Advance(500));
    wait_until("the uptime read", || {
        rig.health("P2HP-RST")["uptime_s"]
            .as_u64()
            .is_some_and(|u| u >= 500)
    });
    // The watchdog fires; USB stays up in the fake, so the hub sees no detach.
    remote.vreg_sim(dspi_transport::fake::VregSim::Hang);
    let BusMessage::Facts { reset_info, .. } = rig.facts("P2HP-RST") else {
        unreachable!()
    };
    let r = reset_info.unwrap();
    assert_eq!(r.reason, ResetReason::Watchdog);
    assert!(r.uptime_s < 500);
    wait_until("the record", || {
        rig.health("P2HP-RST")["reset"]["reason"] == "watchdog"
    });
    rig.hub.stop();
}

// ── health.get ──────────────────────────────────────────────────────────────

#[test]
fn wp_p2hp_health_get_addresses_by_serial_or_handle() {
    let mut rig = Rig::start("get", fast());
    let (handle, _, _) = rig.attach(with_ids("P2HP-GET"));
    rig.health_msg("P2HP-GET");
    let by_handle = rig.ask(json!({"t": "health.get", "handle": handle}));
    assert_eq!(by_handle["devices"][0]["serial"], "P2HP-GET", "{by_handle}");
    assert_eq!(by_handle["devices"][0]["handle"], handle);
    let both = rig.ask(json!({"t": "health.get", "handle": handle, "serial": "P2HP-GET"}));
    assert_eq!(both["t"], "ok", "{both}");
    let cases = [
        (json!({"serial": "NEVER-SEEN"}), "not_found"),
        (json!({"handle": 200}), "no_device"),
        (json!({"handle": handle, "serial": "OTHER"}), "bad_request"),
        (json!({"serial": 5}), "bad_request"),
        (json!({"handle": "x"}), "bad_request"),
    ];
    for (fields, code) in cases {
        let mut msg = json!({"t": "health.get"});
        msg.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let a = rig.ask(msg.clone());
        assert_eq!(a["code"], code, "{msg} -> {a}");
        assert_eq!(a["v"], 1);
    }
    rig.hub.stop();
}

// ── Under a console's lock ──────────────────────────────────────────────────

#[test]
fn wp_p2hp_a_consoles_device_lock_does_not_stall_the_hubs_reads() {
    let mut rig = Rig::start("locked", fast());
    let (handle, probe, remote) = rig.attach(with_ids("P2HP-LOCK"));
    // Locked at once, before the attach reads run on the hub's timer.
    let console = rig.hub.open_session(Role::Control, None).unwrap();
    assert!(
        rig.hub
            .lock_acquire(&console, handle, Duration::from_secs(60))
    );
    let BusMessage::Facts {
        identify,
        reset_info,
        ..
    } = rig.facts("P2HP-LOCK")
    else {
        unreachable!()
    };
    assert!(
        identify.is_some() && reset_info.is_some(),
        "the facts, under the lock"
    );
    // The periodic single reads too: a new uptime and a temperature arrive.
    remote.vreg_sim(dspi_transport::fake::VregSim::Advance(500));
    wait_until("reads under the lock", || {
        let r = rig.health("P2HP-LOCK");
        r["uptime_s"].as_u64().is_some_and(|u| u >= 500) && r["temp_c"] == 35.0
    });
    std::thread::sleep(Duration::from_millis(1800));
    assert_eq!(rig.health("P2HP-LOCK")["unresponsive"], false);
    assert!(
        rig.bus
            .drain()
            .iter()
            .all(|m| !matches!(**m, BusMessage::Unresponsive { .. }))
    );
    // Still the console's lock: another session's write is refused.
    let other = rig.hub.open_session(Role::Control, None).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    rig.hub.submit(
        &other,
        dspi_link::frames::CmdRequest {
            tag: 9,
            handle,
            dir: dspi_link::Dir::Set,
            b_request: op::REQ_SET_MASTER_VOLUME,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: 4,
            payload: (-30.0f32).to_le_bytes().to_vec(),
        },
        dspi_hub::Completion::new(9, move |r| {
            let _ = tx.send(r.status);
        }),
    );
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), dspi_link::status::LOCKED);
    assert_eq!(probe.count(|t| t.3), 0, "the hub wrote nothing");
    rig.hub.stop();
}
