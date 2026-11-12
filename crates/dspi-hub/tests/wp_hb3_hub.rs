//! WP HB3 through the hub's own interfaces: polls shared, bounded and stopped, with
//! transfers per second measured on the device; unplug and replug; the snapshot; rename.
//! `tests/wp_hb3_meter_feed.rs` has C15's real client on top.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::session::Pop;
use dspi_hub::{
    Completion, Hub, HubConfig, HubError, MAX_TRANSFERS_PER_SEC, NameStore, Outbound, PollRefused,
    Role, Session, SnapshotRefused,
};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::messages::PollSpec;
use dspi_link::{Dir, status};
use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);
const GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// What a test can see and change about a device.
#[derive(Clone, Default)]
struct Probe {
    /// Every request that reached the device, in order.
    log: Arc<Mutex<Vec<u8>>>,
    stall: Arc<Mutex<BTreeSet<u8>>>,
    /// -- WP HUBFLAKE -- Reads of the image wait while this is set.
    held: Arc<(Mutex<bool>, Condvar)>,
    /// `BULK_INVALIDATED` packets handed to the hub's reader, and of those, the ones it
    /// had dealt with when it next asked for a packet.
    invalidations: Arc<AtomicUsize>,
    invalidations_taken: Arc<AtomicUsize>,
}

impl Probe {
    fn count(&self, opcode: u8) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|o| **o == opcode)
            .count()
    }
    fn total(&self) -> usize {
        self.log.lock().unwrap().len()
    }
    fn stall(&self, opcode: u8, on: bool) {
        let mut s = self.stall.lock().unwrap();
        if on {
            s.insert(opcode);
        } else {
            s.remove(&opcode);
        }
    }
    /// Reads of the bulk image, however they were carried.
    fn image_reads(&self) -> usize {
        self.count(op::REQ_GET_ALL_PARAMS) + self.count(op::REQ_GET_ALL_PARAMS_CHUNK)
    }
    // -- WP HUBFLAKE --
    /// Hold every read of the image at the device until released.
    fn hold_reads(&self, on: bool) {
        *self.held.0.lock().unwrap() = on;
        self.held.1.notify_all();
    }
    fn wait_while_held(&self, opcode: u8) {
        if opcode != op::REQ_GET_ALL_PARAMS && opcode != op::REQ_GET_ALL_PARAMS_CHUNK {
            return;
        }
        let mut held = self.held.0.lock().unwrap();
        while *held {
            held = self.held.1.wait(held).unwrap();
        }
    }
    /// Invalidations the hub has seen through: its reader is back for the next packet.
    fn invalidations_taken(&self) -> usize {
        self.invalidations_taken.load(Ordering::SeqCst)
    }
}

// -- WP HUBFLAKE --
/// The device's notification stream, counted. The hub's reader takes one packet per
/// `read` and deals with it before the next, so what was handed out before a `read` has
/// been dealt with.
struct Counted {
    inner: Box<dyn NotificationSource>,
    probe: Probe,
}

impl NotificationSource for Counted {
    fn read(&mut self, timeout: Duration) -> dspi_transport::Result<Vec<u8>> {
        let handed = self.probe.invalidations.load(Ordering::SeqCst);
        self.probe
            .invalidations_taken
            .store(handed, Ordering::SeqCst);
        let packet = self.inner.read(timeout)?;
        if packet.get(1) == Some(&(nfy::NOTIFY_EVT_BULK_INVALIDATED as u8)) {
            self.probe.invalidations.fetch_add(1, Ordering::SeqCst);
        }
        Ok(packet)
    }
}

struct Dev {
    inner: FakeDevice,
    descriptor: DeviceDescriptor,
    probe: Probe,
    stream: bool,
}

impl Dev {
    fn new(serial: &str) -> (Self, FakeRemote, Probe) {
        let inner = FakeDevice::demo();
        let remote = inner.remote();
        let probe = Probe::default();
        let dev = Self {
            inner,
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
            },
            probe: probe.clone(),
            stream: true,
        };
        (dev, remote, probe)
    }

    /// A board that stores names and has none, with the clearing's notification gone.
    fn unnamed(serial: &str) -> (Self, FakeRemote, Probe) {
        let (mut dev, remote, probe) = Self::new(serial);
        dev.inner
            .control_out(op::REQ_SET_DEVICE_NAME, 0, &[0])
            .unwrap();
        let mut source = dev.inner.notifications().unwrap();
        while !source.read(Duration::ZERO).unwrap().is_empty() {}
        // The fake's sequence goes on from the cleared notification: a reader that starts
        // here simply has no earlier seq to compare with.
        (dev, remote, probe)
    }

    /// Firmware without board names.
    fn older(serial: &str) -> (Self, FakeRemote, Probe) {
        let (dev, remote, probe) = Self::new(serial);
        probe.stall(op::REQ_GET_DEVICE_NAME, true);
        (dev, remote, probe)
    }

    fn check(&self, opcode: u8) -> dspi_transport::Result<()> {
        self.probe.log.lock().unwrap().push(opcode);
        if self.probe.stall.lock().unwrap().contains(&opcode) {
            return Err(TransportError::Stalled { opcode });
        }
        self.probe.wait_while_held(opcode); // -- WP HUBFLAKE --
        Ok(())
    }
}

impl Transport for Dev {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.check(o)?;
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.check(o)?;
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.check(o)?;
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        if self.stream {
            // -- WP HUBFLAKE --
            let inner = self.inner.notifications()?;
            Some(Box::new(Counted {
                inner,
                probe: self.probe.clone(),
            }))
        } else {
            None
        }
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

fn hub() -> Hub {
    Hub::start(HubConfig::in_process("test")).unwrap()
}

fn meter(slot: u8, hz: u32) -> PollSpec {
    PollSpec {
        slot,
        req: op::REQ_GET_STATUS,
        val: 9,
        idx: dspi_proto::VENDOR_INTERFACE,
        len: 41,
        hz,
    }
}

fn session(hub: &Hub, role: Role) -> Arc<Session> {
    hub.open_session(role, None).unwrap()
}

/// What came out of an outbox within `for_`.
#[derive(Default)]
struct Seen {
    polls: Vec<(u8, u8, Vec<u8>)>,
    events: Vec<Value>,
    resyncs: Vec<(u8, u8)>,
}

impl Seen {
    fn event(&self, t: &str) -> Option<&Value> {
        self.events.iter().find(|e| e["t"] == t)
    }
}

/// Drains for `for_`, as a connection's writer would, so no frame is replaced.
fn watch(s: &Session, for_: Duration) -> Seen {
    let mut seen = Seen::default();
    let start = Instant::now();
    loop {
        let left = for_.saturating_sub(start.elapsed());
        if left.is_zero() {
            return seen;
        }
        match s.outbox().pop(left) {
            Pop::Item(Outbound::Poll {
                handle,
                slot,
                payload,
            }) => seen.polls.push((handle, slot, payload)),
            Pop::Item(Outbound::Event(e)) => seen.events.push(serde_json::from_str(&e).unwrap()),
            Pop::Item(Outbound::Resync { handle, reason }) => seen.resyncs.push((handle, reason)),
            Pop::Item(_) => {}
            Pop::Idle => {}
            Pop::Closed => return seen,
        }
    }
}

fn wait_for_event(s: &Session, what: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no such event");
        if let Pop::Item(Outbound::Event(e)) = s.outbox().pop(Duration::from_millis(20)) {
            let v: Value = serde_json::from_str(&e).unwrap();
            if what(&v) {
                return v;
            }
        }
    }
}

fn command(hub: &Hub, s: &Session, req: CmdRequest) -> CmdResponse {
    let (tx, rx) = std::sync::mpsc::channel();
    hub.submit(s, req, Completion::new(1, move |r| tx.send(r).unwrap()));
    rx.recv_timeout(WAIT).unwrap()
}

fn set(code: u8, payload: Vec<u8>) -> CmdRequest {
    CmdRequest {
        tag: 1,
        handle: 0,
        dir: Dir::Set,
        b_request: code,
        w_value: 0,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: payload.len() as u16,
        payload,
    }
}

fn rename(hub: &Hub, s: &Session, handle: u8, name: &str) -> u8 {
    let (tx, rx) = std::sync::mpsc::channel();
    hub.rename(s, handle, name, move |r| tx.send(r).unwrap());
    rx.recv_timeout(WAIT).unwrap()
}

// -- Polls -------------------------------------------------------------------

/// The load that prompted `poll.subscribe`: each client reading the meters for itself.
/// Two here are one read, and it ends with the last of them.
#[test]
fn wp_hb3_two_subscribers_share_one_read_and_it_stops_with_the_last() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let (a, b) = (session(&hub, Role::Viewer), session(&hub, Role::Admin));

    // The worker was asleep with nothing to do: a subscribe wakes it.
    let grant = hub.poll_subscribe(&a, handle, &[meter(0, 15)]).unwrap();
    assert_eq!(grant[0].hz, 15);
    hub.poll_subscribe(&b, handle, &[meter(4, 15)]).unwrap();

    let window = Duration::from_secs(2);
    let before = probe.count(op::REQ_GET_STATUS);
    let (seen_a, seen_b) = std::thread::scope(|t| {
        let (a, b) = (&a, &b);
        let wa = t.spawn(move || watch(a, window));
        let wb = t.spawn(move || watch(b, window));
        (wa.join().unwrap(), wb.join().unwrap())
    });
    let reads = probe.count(op::REQ_GET_STATUS) - before;
    let per_second = reads as f64 / window.as_secs_f64();
    println!(
        "HB3 measured: 2 subscribers at 15 Hz: {per_second:.1} transfers/s on the device, \
         {} and {} frames in {window:?}",
        seen_a.polls.len(),
        seen_b.polls.len()
    );
    assert!(
        (10.0..=17.0).contains(&per_second),
        "one read at 15 Hz, not two: {per_second}"
    );
    assert!(seen_a.polls.len() >= 20, "{}", seen_a.polls.len());
    assert!(seen_b.polls.len() >= 20, "{}", seen_b.polls.len());
    assert!(seen_a.polls.iter().all(|p| p.0 == handle && p.1 == 0));
    assert!(seen_b.polls.iter().all(|p| p.1 == 4), "its own slot");
    assert!(seen_a.polls[0].2.len() == 41, "verbatim");

    // One leaves: the read goes on for the other. Then none: it stops.
    hub.close_session(a.id());
    let n = probe.count(op::REQ_GET_STATUS);
    wait_until("the read to go on", || {
        probe.count(op::REQ_GET_STATUS) > n + 2
    });
    hub.poll_unsubscribe(&b, handle, &[4]);
    std::thread::sleep(Duration::from_millis(100));
    let stopped = probe.total();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(probe.total(), stopped, "nobody subscribes, nothing is read");
    println!("HB3 measured: no subscribers: 0 transfers/s");

    // And it wakes from that sleep again.
    hub.poll_subscribe(&b, handle, &[meter(0, 20)]).unwrap();
    wait_until("a new feed", || probe.total() > stopped);
}

#[test]
fn wp_hb3_a_subscribed_rate_faster_than_the_one_being_slept_on_starts_at_once() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Viewer);
    hub.poll_subscribe(&s, handle, &[meter(0, 1)]).unwrap();
    wait_until("the first read", || probe.count(op::REQ_GET_STATUS) == 1);
    let t = Instant::now();
    hub.poll_subscribe(&s, handle, &[meter(1, 20)]).unwrap();
    wait_until("a faster read", || probe.count(op::REQ_GET_STATUS) >= 3);
    assert!(
        t.elapsed() < Duration::from_millis(900),
        "{:?}",
        t.elapsed()
    );
}

/// However many ask for however much: bounded on the device, commands still served.
#[test]
fn wp_hb3_a_full_load_stays_bounded_and_commands_go_between_reads() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Admin);
    let specs = |base: u16| -> Vec<PollSpec> {
        (0..16)
            .map(|n| PollSpec {
                val: base + u16::from(n),
                len: 1,
                ..meter(n, 1000)
            })
            .collect()
    };
    let grant = hub.poll_subscribe(&s, handle, &specs(100)).unwrap();
    assert!(grant.iter().all(|g| g.hz == 20), "{grant:?}");
    let other = session(&hub, Role::Viewer);
    assert_eq!(
        hub.poll_subscribe(&other, handle, &specs(200)),
        Err(PollRefused::Busy),
        "already at the limit"
    );

    let window = Duration::from_secs(2);
    let start = Instant::now();
    let before = probe.total();
    let mut worst = Duration::ZERO;
    let mut commands = 0usize;
    while start.elapsed() < window {
        let t = Instant::now();
        let r = command(&hub, &s, set(op::REQ_SET_MASTER_VOLUME, vec![0; 4]));
        assert_eq!(r.status, status::OK);
        worst = worst.max(t.elapsed());
        commands += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    let elapsed = start.elapsed().as_secs_f64();
    let reads = (probe.total() - before - commands) as f64 / elapsed;
    println!(
        "HB3 measured: 16 polls at 20 Hz, with {commands} commands: {reads:.0} poll transfers/s \
         (limit {MAX_TRANSFERS_PER_SEC}), slowest command {worst:?}"
    );
    assert!(reads <= f64::from(MAX_TRANSFERS_PER_SEC) * 1.05, "{reads}");
    assert!(reads >= 100.0, "the polls ran: {reads}");
    assert!(worst < Duration::from_millis(500), "{worst:?}");
}

#[test]
fn wp_hb3_a_failing_poll_reports_slows_and_recovers() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Viewer);
    probe.stall(op::REQ_GET_STATUS, true);
    hub.poll_subscribe(&s, handle, &[meter(3, 20)]).unwrap();
    let e = wait_for_event(&s, |e| e["t"] == "poll.error");
    assert_eq!(e["handle"], u64::from(handle));
    assert_eq!(e["slot"], 3);
    assert_eq!(e["code"], "busy");
    let n = probe.count(op::REQ_GET_STATUS);
    let seen = watch(&s, Duration::from_millis(500));
    assert!(seen.polls.is_empty() && seen.events.is_empty(), "once");
    let slowed = probe.count(op::REQ_GET_STATUS) - n;
    assert!(slowed <= 2, "{slowed} reads in 500 ms");
    probe.stall(op::REQ_GET_STATUS, false);
    wait_until("recovery", || {
        matches!(
            s.outbox().pop(Duration::ZERO),
            Pop::Item(Outbound::Poll { .. })
        )
    });
}

/// `bulk_params_chunking.md`: any other vendor request tears a chunk session down, so a
/// client's own `0xA2` must not be interleaved with meter reads.
#[test]
fn wp_hb3_polls_stay_out_of_a_clients_chunk_session() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Admin);
    hub.poll_subscribe(&s, handle, &[meter(0, 20)]).unwrap();
    wait_until("the feed", || probe.count(op::REQ_GET_STATUS) > 1);
    probe.log.lock().unwrap().clear();
    for offset in 0..6u16 {
        let req = CmdRequest {
            dir: Dir::Get,
            w_value: offset * 64,
            w_length: 64,
            ..set(op::REQ_GET_ALL_PARAMS_CHUNK, Vec::new())
        };
        assert_eq!(command(&hub, &s, req).status, status::OK);
        // A slow client: longer than the poll's period, between chunks.
        std::thread::sleep(Duration::from_millis(120));
    }
    let log = probe.log.lock().unwrap().clone();
    let first = log
        .iter()
        .position(|o| *o == op::REQ_GET_ALL_PARAMS_CHUNK)
        .unwrap();
    let last = log
        .iter()
        .rposition(|o| *o == op::REQ_GET_ALL_PARAMS_CHUNK)
        .unwrap();
    assert!(
        log[first..=last]
            .iter()
            .all(|o| *o == op::REQ_GET_ALL_PARAMS_CHUNK),
        "{log:?}"
    );
    drop(log);
    let n = probe.count(op::REQ_GET_STATUS);
    wait_until("the feed to resume", || probe.count(op::REQ_GET_STATUS) > n);
}

#[test]
fn wp_hb3_a_poll_refused_by_the_hub_names_why() {
    let hub = hub();
    let (dev, _remote, _) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Admin);
    assert_eq!(
        hub.poll_subscribe(&s, handle + 1, &[meter(0, 1)]),
        Err(PollRefused::NoDevice)
    );
    let save = PollSpec {
        req: op::REQ_SAVE_PARAMS,
        ..meter(0, 1)
    };
    assert_eq!(
        hub.poll_subscribe(&s, handle, &[save]),
        Err(PollRefused::Denied)
    );
    hub.poll_unsubscribe(&s, handle + 1, &[0]);
}

// -- Unplug and replug ---------------------------------------------------------

#[test]
fn wp_hb3_unplug_ends_the_feed_and_replug_needs_a_new_subscribe() {
    let hub = hub();
    let (dev, remote, probe) = Dev::new("SERIAL");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Viewer);
    hub.poll_subscribe(&s, handle, &[meter(0, 20)]).unwrap();
    wait_until("the feed", || probe.count(op::REQ_GET_STATUS) > 0);

    remote.disconnect();
    let off = wait_for_event(&s, |e| e["device"]["state"] == "offline");
    assert_eq!(off["t"], "device.changed");
    assert_eq!(off["device"]["handle"], u64::from(handle));
    assert_eq!(
        hub.poll_subscribe(&s, handle, &[meter(0, 20)]),
        Err(PollRefused::NoDevice)
    );
    assert_eq!(hub.snapshot(handle), Err(SnapshotRefused::NoDevice));
    let gone = probe.total();

    // Back, at the same handle, with a RESYNC. The old subscription stays gone (spec
    // 7.6), so nothing reads the new board for it, and nothing touches the old one.
    let (dev, _remote2, probe2) = Dev::new("SERIAL");
    assert_eq!(hub.attach(Box::new(dev), 0).unwrap(), handle);
    let seen = watch(&s, Duration::from_millis(300));
    assert!(seen.resyncs.contains(&(handle, 1)), "{:?}", seen.resyncs);
    assert!(seen.polls.is_empty());
    let on = seen.event("device.changed").expect("device.changed");
    assert_eq!(on["device"]["state"], "online");
    assert_eq!(probe.total(), gone);
    assert_eq!(probe2.count(op::REQ_GET_STATUS), 0);

    // A client that still wants the feed asks again.
    hub.poll_subscribe(&s, handle, &[meter(0, 20)]).unwrap();
    let seen = watch(&s, Duration::from_millis(300));
    assert!(!seen.polls.is_empty());
    assert!(probe2.count(op::REQ_GET_STATUS) > 0);
}

/// A device with no notification stream has only its reads to say it has gone.
#[test]
fn wp_hb3_a_poll_notices_a_lost_device() {
    let hub = hub();
    let (mut dev, remote, _) = Dev::new("A");
    dev.stream = false;
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Viewer);
    hub.poll_subscribe(&s, handle, &[meter(0, 20)]).unwrap();
    remote.disconnect();
    wait_until("offline", || {
        hub.devices()[0].state == dspi_hub::DeviceState::Offline
    });
}

/// A hub stops with polls running, and the transport goes.
#[test]
fn wp_hb3_stop_with_polls_running_ends_them() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Viewer);
    hub.poll_subscribe(&s, handle, &[meter(0, 20)]).unwrap();
    wait_until("the feed", || probe.count(op::REQ_GET_STATUS) > 0);
    hub.stop();
    let stopped = probe.total();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(probe.total(), stopped);
    assert_eq!(
        hub.poll_subscribe(&s, handle, &[meter(0, 20)]),
        Err(PollRefused::NoDevice)
    );
}

// -- Snapshot ------------------------------------------------------------------

#[test]
fn wp_hb3_a_snapshot_is_one_read_and_stays_true() {
    let hub = hub();
    let (dev, remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(probe.image_reads(), 0, "cold: no bulk read for nobody");
    let s = session(&hub, Role::Admin);

    let first = hub.snapshot(handle).unwrap();
    assert_eq!(first.bulk(), Some(remote.image()));
    assert_eq!(first.handle, u32::from(handle));
    assert_eq!(first.wire_version, Some(u32::from(remote.image()[0])));
    assert!(first.age_ms.is_some());
    let reads = probe.image_reads();
    assert!(reads > 0);

    // Served from the cache.
    assert_eq!(hub.snapshot(handle).unwrap().bulk(), Some(remote.image()));
    assert_eq!(probe.image_reads(), reads);

    // A write by a client, and a knob: patched, no re-read.
    let db = (-21.5f32).to_le_bytes().to_vec();
    let r = command(&hub, &s, set(op::REQ_SET_MASTER_VOLUME, db.clone()));
    assert_eq!(r.status, status::OK);
    let at = remote.image().windows(4).position(|w| w == db).unwrap();
    assert!(remote.poke(at, &(-3.0f32).to_le_bytes(), GPIO));
    wait_until("the patch", || {
        hub.snapshot(handle).unwrap().bulk() == Some(remote.image())
    });
    assert_eq!(probe.image_reads(), reads, "patched, not re-read");

    // A whole-image change: read again, and not served stale meanwhile.
    let mut other = remote.image();
    other[at] ^= 0x55;
    remote.load_image(other.clone(), GPIO);
    wait_until("the re-read", || probe.image_reads() > reads);
    wait_until("the new image", || {
        hub.snapshot(handle).unwrap().bulk() == Some(other.clone())
    });
}

/// A hub with a network keeps the image from the start, so a first screen costs no read.
#[test]
fn wp_hb3_a_warm_hub_has_the_image_before_anyone_asks() {
    let mut config = HubConfig::in_process("test");
    config.warm_snapshots = true;
    let hub = Hub::start(config).unwrap();
    let (dev, remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    wait_until("the warm read", || probe.image_reads() > 0);
    wait_until("the image", || hub.snapshot(handle).is_ok());
    let reads = probe.image_reads();
    assert_eq!(hub.snapshot(handle).unwrap().bulk(), Some(remote.image()));
    assert_eq!(probe.image_reads(), reads);
}

/// -- WP HUBFLAKE -- The flood lands while the device is still busy with the read the
/// first invalidation asked for, so nothing but the hub's coalescing stands between 200
/// invalidations and 200 reads, however fast or slow the host is: that read, which the
/// flood makes stale, and one more. Counted in reads of the size this image takes.
#[test]
fn wp_hb3_a_flood_of_invalidations_costs_a_bounded_number_of_reads() {
    const FLOOD: usize = 200;
    let hub = hub();
    let (dev, remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Admin);
    let before = probe.image_reads();
    hub.snapshot(handle).unwrap();
    let per_read = probe.image_reads() - before;
    assert!(per_read > 0);

    let before = probe.image_reads();
    let taken = probe.invalidations_taken();
    let image = remote.image();
    probe.hold_reads(true);
    for _ in 0..FLOOD {
        remote.load_image(image.clone(), GPIO);
    }
    wait_until("the hub to take every invalidation", || {
        probe.invalidations_taken() >= taken + FLOOD
    });
    probe.hold_reads(false);
    wait_until("the cache to return", || hub.snapshot(handle).is_ok());
    // The device's jobs run in order: a read still queued has run before this answers.
    let platform = CmdRequest {
        dir: Dir::Get,
        w_length: 64,
        ..set(op::REQ_GET_PLATFORM, Vec::new())
    };
    assert_eq!(command(&hub, &s, platform).status, status::OK);
    let reads = probe.image_reads() - before;
    assert!(
        reads <= 2 * per_read,
        "{reads} requests, {} reads of {per_read}, for {FLOOD} invalidations",
        reads.div_ceil(per_read)
    );
    assert_eq!(hub.snapshot(handle).unwrap().bulk(), Some(image));
}

#[test]
fn wp_hb3_a_snapshot_that_cannot_be_read_says_busy_and_recovers() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    probe.stall(op::REQ_GET_ALL_PARAMS_CHUNK, true);
    probe.stall(op::REQ_GET_ALL_PARAMS, true);
    assert_eq!(hub.snapshot(handle), Err(SnapshotRefused::Busy));
    assert_eq!(SnapshotRefused::Busy.code(), "busy");
    probe.stall(op::REQ_GET_ALL_PARAMS_CHUNK, false);
    probe.stall(op::REQ_GET_ALL_PARAMS, false);
    assert!(hub.snapshot(handle).is_ok());
}

/// With no stream nothing says the device changed, so a cached image would lie.
#[test]
fn wp_hb3_a_device_with_no_stream_reads_for_each_snapshot() {
    let hub = hub();
    let (mut dev, remote, probe) = Dev::new("A");
    dev.stream = false;
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    hub.snapshot(handle).unwrap();
    let reads = probe.image_reads();
    let at = remote.image().len() - 8;
    assert!(remote.poke(at, &[0x5A], GPIO));
    assert_eq!(hub.snapshot(handle).unwrap().bulk(), Some(remote.image()));
    assert!(probe.image_reads() > reads);
}

#[test]
fn wp_hb3_a_replugged_board_gets_a_new_image() {
    let hub = hub();
    let (dev, remote, _) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    hub.snapshot(handle).unwrap();
    remote.disconnect();
    wait_until("offline", || {
        hub.devices()[0].state == dspi_hub::DeviceState::Offline
    });
    let (dev, remote, probe) = Dev::new("A");
    assert_eq!(hub.attach(Box::new(dev), 0).unwrap(), handle);
    let at = remote.image().len() - 8;
    assert!(remote.poke(at, &[0x5A], GPIO));
    assert_eq!(hub.snapshot(handle).unwrap().bulk(), Some(remote.image()));
    assert!(probe.image_reads() > 0);
}

// -- Names ---------------------------------------------------------------------

#[derive(Default)]
struct Names {
    kept: Mutex<BTreeMap<String, String>>,
    saves: Mutex<usize>,
}

impl NameStore for Names {
    fn load(&self) -> BTreeMap<String, String> {
        self.kept.lock().unwrap().clone()
    }
    fn save(&self, names: &BTreeMap<String, String>) -> Result<(), HubError> {
        *self.kept.lock().unwrap() = names.clone();
        *self.saves.lock().unwrap() += 1;
        Ok(())
    }
}

impl Names {
    fn with(serial: &str, name: &str) -> Arc<Self> {
        let n = Arc::new(Self::default());
        n.kept.lock().unwrap().insert(serial.into(), name.into());
        n
    }
    fn kept(&self) -> BTreeMap<String, String> {
        self.kept.lock().unwrap().clone()
    }
}

fn hub_with(names: &Arc<Names>) -> Hub {
    let mut config = HubConfig::in_process("test");
    config.names = Some(Arc::clone(names) as Arc<dyn NameStore>);
    Hub::start(config).unwrap()
}

fn name_of(hub: &Hub, handle: u8) -> String {
    hub.devices()
        .iter()
        .find(|d| d.handle == handle)
        .unwrap()
        .name
        .clone()
}

/// A rename on a board that stores names: written as the session, `ok` when accepted,
/// and the name changes only when the board says so.
#[test]
fn wp_hb3_a_rename_writes_the_board_and_its_notification_sets_the_name() {
    let names = Arc::new(Names::default());
    let hub = hub_with(&names);
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(name_of(&hub, handle), "DSPi Demo", "the board's own");
    let s = session(&hub, Role::Admin);

    assert_eq!(rename(&hub, &s, handle, "  Köök  rack "), status::OK);
    assert_eq!(probe.count(op::REQ_SET_DEVICE_NAME), 1);
    let changed = wait_for_event(&s, |e| e["t"] == "device.changed");
    assert_eq!(
        changed["device"]["name"], "Kk rack",
        "as the board holds it"
    );
    assert_eq!(name_of(&hub, handle), "Kk rack");
    assert!(
        names.kept().is_empty(),
        "a board's name is not the hub's to keep"
    );

    // A clear goes through too: the default.
    assert_eq!(rename(&hub, &s, handle, ""), status::OK);
    wait_until("the default", || name_of(&hub, handle) == "DSPi A");
}

/// "a rename from any source is seen, including a raw 0x56 through `cmd`".
#[test]
fn wp_hb3_a_raw_name_write_through_cmd_is_seen() {
    let hub = hub();
    let (dev, _remote, _) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Admin);
    let mut name = b"Raw".to_vec();
    name.resize(32, 0);
    let r = command(&hub, &s, set(op::REQ_SET_DEVICE_NAME, name));
    assert_eq!(r.status, status::OK);
    let changed = wait_for_event(&s, |e| e["t"] == "device.changed");
    assert_eq!(changed["device"]["name"], "Raw");
    assert_eq!(hub.devices()[0].name, "Raw");
    assert_eq!(handle, 0);
}

#[test]
fn wp_hb3_a_rename_honours_locks_and_roles() {
    let hub = hub();
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let (a, b) = (session(&hub, Role::Admin), session(&hub, Role::Admin));
    assert_eq!(
        rename(&hub, &session(&hub, Role::Viewer), handle, "x"),
        status::DENIED
    );
    assert!(hub.lock_acquire(&a, handle, Duration::from_secs(5)));
    assert_eq!(rename(&hub, &b, handle, "x"), status::LOCKED);
    assert_eq!(probe.count(op::REQ_SET_DEVICE_NAME), 0);
    assert_eq!(rename(&hub, &a, handle, "x"), status::OK, "its own lock");
    assert_eq!(rename(&hub, &a, handle + 1, "x"), status::NO_DEVICE);
}

/// A name-capable board that is not attached answers `no_device`: its own name would
/// override a stored one on return.
#[test]
fn wp_hb3_a_rename_of_an_absent_board_that_stores_names_is_refused() {
    let names = Arc::new(Names::default());
    let hub = hub_with(&names);
    let (dev, remote, _) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    remote.disconnect();
    wait_until("offline", || {
        hub.devices()[0].state == dspi_hub::DeviceState::Offline
    });
    let s = session(&hub, Role::Admin);
    assert_eq!(rename(&hub, &s, handle, "x"), status::NO_DEVICE);
    assert!(names.kept().is_empty());
}

/// Older firmware: the hub keeps the name, emits the change itself, also offline, and
/// nothing is ever sent to `0x56`, which it may take for a legacy gain.
#[test]
fn wp_hb3_older_firmware_gets_a_stored_name_and_never_a_name_write() {
    let names = Arc::new(Names::default());
    let hub = hub_with(&names);
    let (dev, remote, probe) = Dev::older("OLD");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(name_of(&hub, handle), "DSPi OLD");
    assert_eq!(hub.devices()[0].identity.board_name, None);
    let s = session(&hub, Role::Admin);

    assert_eq!(rename(&hub, &s, handle, "Köket"), status::OK);
    let changed = wait_for_event(&s, |e| e["t"] == "device.changed");
    assert_eq!(
        changed["device"]["name"], "Köket",
        "the hub can hold any text"
    );
    assert_eq!(names.kept()["OLD"], "Köket");

    remote.disconnect();
    wait_until("offline", || {
        hub.devices()[0].state == dspi_hub::DeviceState::Offline
    });
    assert_eq!(rename(&hub, &s, handle, "Hall"), status::OK);
    assert_eq!(names.kept()["OLD"], "Hall");

    let (dev, _remote, probe2) = Dev::older("OLD");
    hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(name_of(&hub, handle), "Hall", "kept across the unplug");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        probe.count(op::REQ_SET_DEVICE_NAME) + probe2.count(op::REQ_SET_DEVICE_NAME),
        0
    );
}

#[test]
fn wp_hb3_a_short_name_reply_is_older_firmware() {
    let hub = hub();

    struct Short(Dev);
    impl Transport for Short {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.0.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            if o == op::REQ_GET_DEVICE_NAME {
                return Ok(vec![0; 4]); // A legacy channel mute.
            }
            self.0.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
            self.0.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.0.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            self.0.notifications()
        }
        fn caps(&self) -> TransportCaps {
            self.0.caps()
        }
    }
    let (dev, _remote, _) = Dev::new("OLD");
    hub.attach(Box::new(Short(dev)), 0).unwrap();
    assert_eq!(hub.devices()[0].identity.board_name, None);
}

/// Spec 7.4: a stored name goes onto an unnamed board once, and is then forgotten, so a
/// later clear is never undone.
#[test]
fn wp_hb3_a_stored_name_is_adopted_once_and_a_later_clear_stays() {
    let names = Names::with("A", "Köket vänster");
    let hub = hub_with(&names);
    let watcher = session(&hub, Role::Viewer);
    let (dev, _remote, probe) = Dev::unnamed("A");
    probe.log.lock().unwrap().clear();
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    wait_until("the entry to retire", || names.kept().is_empty());
    assert_eq!(probe.count(op::REQ_SET_DEVICE_NAME), 1);
    let changed = wait_for_event(&watcher, |e| {
        e["t"] == "device.changed" && e["device"]["name"] == "Kket vnster"
    });
    assert_eq!(changed["device"]["handle"], u64::from(handle));

    // Cleared from elsewhere; it comes back as cleared, with nothing to write.
    let s = session(&hub, Role::Admin);
    assert_eq!(rename(&hub, &s, handle, ""), status::OK);
    wait_until("the clear", || name_of(&hub, handle) == "DSPi A");
    hub.detach(handle).unwrap();
    let (dev, _remote, probe) = Dev::unnamed("A");
    probe.log.lock().unwrap().clear();
    hub.attach(Box::new(dev), 0).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(probe.count(op::REQ_SET_DEVICE_NAME), 0);
    assert_eq!(name_of(&hub, handle), "DSPi A");
}

/// A failed adoption keeps the entry for the next attach.
#[test]
fn wp_hb3_a_refused_adoption_is_tried_again_at_the_next_attach() {
    let names = Names::with("A", "Den");
    let hub = hub_with(&names);
    let (dev, _remote, probe) = Dev::unnamed("A");
    probe.log.lock().unwrap().clear();
    probe.stall(op::REQ_SET_DEVICE_NAME, true);
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    wait_until("the write", || probe.count(op::REQ_SET_DEVICE_NAME) == 1);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(names.kept()["A"], "Den", "not accepted, so kept");
    assert_eq!(name_of(&hub, handle), "Den", "shown meanwhile");
    hub.detach(handle).unwrap();
    let (dev, _remote, _) = Dev::unnamed("A");
    hub.attach(Box::new(dev), 0).unwrap();
    wait_until("adopted", || names.kept().is_empty());
    wait_until("named", || name_of(&hub, handle) == "Den");
}

#[test]
fn wp_hb3_a_board_with_its_own_name_retires_the_stored_one_at_attach() {
    let names = Names::with("A", "Old name");
    let hub = hub_with(&names);
    let (dev, _remote, probe) = Dev::new("A");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(name_of(&hub, handle), "DSPi Demo");
    assert!(names.kept().is_empty());
    assert_eq!(*names.saves.lock().unwrap(), 1);
    assert_eq!(probe.count(op::REQ_SET_DEVICE_NAME), 0);
}
