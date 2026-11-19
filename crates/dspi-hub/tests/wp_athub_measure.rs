//! WP ATHUB: the hub family `system.measure.*` (PLAN-AT section 1.5) in-process,
//! with ATFAKE's units on one time base (line, USB-direct, S/PDIF), on G1b
//! (`REQ_EXTENSION`) and G2b (the tunnel) fakes:
//!
//! - `open` refuses two domains, an unlocked unit, a unit without the
//!   timeline, a unit another console holds, a Viewer, a Link 1.2 client, and
//!   two units whose domain ids collide but whose `t` disagree; the lease is
//!   clamped;
//! - `run` writes each unit's op 9 at `t0 + offset` in its own numbering (the
//!   fakes' recorded starts; USB as S, time base 1), one pending per unit, the
//!   next once the previous has started; `bad_schedule` for a unit's segments
//!   closer than the lead; `t0` waits for the servos to settle;
//! - `abort` by the owner, another Control console and an Admin, and the
//!   dead-man after the lease: every unit stopped, every queue cancelled, a
//!   later op 9 refused not ready; the broadcast stop only when every unit
//!   has the ops;
//! - a disturbance proved by the counter alone (the unit's events not
//!   announced), a silent whole-sample correction, a clip event, a foreign
//!   preset load and a closed connection each abort (`aborted` sent);
//! - R7b's two-entry unit (a USB-direct follower that is also an S/PDIF
//!   source) on a scripted transport over a fake.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_hub::{Completion, Hub, HubConfig, Outbound, Pop, Role, Session};
use dspi_link::frames::CmdRequest;
use dspi_link::messages::LINK_13_MINOR;
use dspi_link::{Dir, status};
use dspi_proto::ext::alink::{AlinkInfo, AlinkRole, EXT_ALINK_INFO};
use dspi_proto::ext::timebase::{self as tbc, SiggenTimeline, TimelineResult, encode_apply_tb};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::packets::SiggenConfig;
use dspi_transport::fake::{
    AlinkSetup, FakeGeneration, FakeTimeBase, FakeTimeBaseUnit, SpdifSetup, TimeBaseFault,
    UsbDirectSetup,
};
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Result as TResult, Transport,
    TransportCaps,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const RATE: u64 = 48_000;
const G1B: FakeGeneration = FakeGeneration::Bofr116 { page_guard: true };
const G2B: FakeGeneration = FakeGeneration::Bofr117;

// ── The rig ──────────────────────────────────────────────────────────────

fn hub() -> Hub {
    let hub = Hub::start(HubConfig::in_process("athub")).unwrap();
    hub.handle().set_system_measure_settle(Duration::ZERO);
    // The tests decide when the system model is read.
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));
    hub
}

fn session(hub: &Hub, role: Role) -> Arc<Session> {
    let s = hub.open_session(role, None).unwrap();
    s.set_link_minor(LINK_13_MINOR);
    s
}

fn ask(hub: &Hub, s: &Arc<Session>, msg: Value) -> Value {
    let text = dspi_hub::system::measure::handle_message(&hub.handle(), s, &msg)
        .expect("the family answers");
    serde_json::from_str(&text).unwrap()
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum World {
    Line,
    Usb,
    Spdif,
}

/// `n` units of `world` on one time base, as firmware of `g`.
fn world(world: World, n: usize, g: FakeGeneration) -> (FakeTimeBase, Vec<FakeDevice>) {
    let tb = match world {
        World::Line => FakeTimeBase::line(0x21),
        World::Usb => FakeTimeBase::usb_direct(),
        World::Spdif => FakeTimeBase::spdif(0x1234),
    };
    let devs = (0..n)
        .map(|i| {
            let unit = match world {
                World::Line => FakeTimeBaseUnit::Line(AlinkSetup::default()),
                World::Usb => FakeTimeBaseUnit::UsbDirect(UsbDirectSetup {
                    origin_wraps: 1 + 2 * i as u32,
                    ..UsbDirectSetup::default()
                }),
                World::Spdif if i == 0 => FakeTimeBaseUnit::Spdif(SpdifSetup {
                    // Slot 1 (outputs 2, 3) carries the time base.
                    tx_mask: 0b10,
                    ..SpdifSetup::source()
                }),
                World::Spdif => FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)),
            };
            FakeDevice::demo_on_time_base_as(&tb, unit, g).with_serial(&format!("ATHUB-{i}"))
        })
        .collect();
    (tb, devs)
}

struct Units {
    serials: Vec<String>,
    handles: Vec<u8>,
    remotes: Vec<FakeRemote>,
}

fn attach(hub: &Hub, devs: Vec<FakeDevice>) -> Units {
    let mut u = Units {
        serials: Vec::new(),
        handles: Vec::new(),
        remotes: Vec::new(),
    };
    for d in devs {
        u.serials.push(d.descriptor().serial.clone());
        u.remotes.push(d.remote());
        u.handles.push(hub.attach(Box::new(d), 0).unwrap());
    }
    u
}

fn sweep(mask: u16) -> SiggenConfig {
    SiggenConfig {
        signal_type: 4,
        channel_mask: mask,
        level_db: -30.0,
        duration_ms: 1_000,
        repeat_count: 1,
        p1: 100.0,
        p2: 10_000.0,
        ..SiggenConfig::default()
    }
}

fn open(hub: &Hub, s: &Arc<Session>, units: &Units) -> Value {
    ask(
        hub,
        s,
        json!({"t": "system.measure.open", "id": 1, "units": units.serials, "lease_ms": 5000}),
    )
}

fn open_ok(hub: &Hub, s: &Arc<Session>, units: &Units) -> (String, Value) {
    let v = open(hub, s, units);
    assert_eq!(v["t"], "ok", "{v}");
    (v["session"].as_str().unwrap().to_owned(), v)
}

fn stage(hub: &Hub, s: &Arc<Session>, m: &str, serial: &str, cfg: &SiggenConfig) -> Value {
    ask(
        hub,
        s,
        json!({"t": "system.measure.stage", "id": 2, "session": m, "serial": serial,
               "config": cfg.encode().to_vec()}),
    )
}

fn run(hub: &Hub, s: &Arc<Session>, m: &str, segments: &[(u8, u64)]) -> Value {
    let segments: Vec<Value> = segments
        .iter()
        .map(|(u, o)| json!({"unit": u, "offset": o}))
        .collect();
    ask(
        hub,
        s,
        json!({"t": "system.measure.run", "id": 3, "session": m, "segments": segments, "lead_ms": 250}),
    )
}

fn status_of(hub: &Hub, s: &Arc<Session>, m: &str) -> Value {
    ask(
        hub,
        s,
        json!({"t": "system.measure.status", "id": 4, "session": m}),
    )
}

/// Poll `status` (the lease) until `cond`.
fn until(hub: &Hub, s: &Arc<Session>, m: &str, what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        let st = status_of(hub, s, m);
        assert_eq!(st["t"], "ok", "{what}: {st}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The next `system.measure.aborted` sent to `s`.
fn aborted_event(s: &Session) -> Value {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no aborted event");
        match s.outbox().pop(Duration::from_millis(100)) {
            Pop::Item(Outbound::Event(text)) => {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["t"] == "system.measure.aborted" {
                    return v;
                }
            }
            Pop::Closed => panic!("closed"),
            _ => {}
        }
    }
}

/// An extension read on `handle` as session `s`, 1.1.6-shaped (a Link 1.2
/// request, which the hub translates for a G2b unit).
fn ext_get(hub: &Hub, s: &Session, handle: u8, id: u16, len: u16) -> (u8, Vec<u8>) {
    let minor = s.link_minor();
    s.set_link_minor(2);
    let r = ext_get_as(hub, s, handle, id, len);
    s.set_link_minor(minor);
    r
}

fn ext_get_as(hub: &Hub, s: &Session, handle: u8, id: u16, len: u16) -> (u8, Vec<u8>) {
    let (tx, rx) = mpsc::channel();
    let req = CmdRequest {
        tag: 1,
        handle,
        dir: Dir::Get,
        b_request: op::REQ_EXTENSION,
        w_value: id,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: len,
        payload: Vec::new(),
    };
    hub.submit(s, req, Completion::new(1, move |r| tx.send(r).unwrap()));
    let r = rx.recv_timeout(WAIT).unwrap();
    (r.status, r.payload)
}

fn ext_set(hub: &Hub, s: &Session, handle: u8, id: u16, data: Vec<u8>) -> u8 {
    s.set_link_minor(2);
    let (tx, rx) = mpsc::channel();
    let req = CmdRequest {
        tag: 1,
        handle,
        dir: Dir::Set,
        b_request: op::REQ_EXTENSION,
        w_value: id,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: data.len() as u16,
        payload: data,
    };
    hub.submit(s, req, Completion::new(1, move |r| tx.send(r).unwrap()));
    rx.recv_timeout(WAIT).unwrap().status
}

fn timeline(hub: &Hub, s: &Session, handle: u8) -> SiggenTimeline {
    let (st, b) = ext_get(hub, s, handle, tbc::EXT_SIGGEN_TIMELINE, 40);
    assert_eq!(st, status::OK);
    SiggenTimeline::decode(&b).unwrap()
}

// ── open ─────────────────────────────────────────────────────────────────

#[test]
fn wp_athub_open_refuses_what_is_not_one_locked_domain() {
    let hub = hub();
    let admin = session(&hub, Role::Admin);
    // Two domains: a line unit and a USB-direct unit.
    let (_a, line) = world(World::Line, 1, G1B);
    let (_b, usb) = world(World::Usb, 1, G1B);
    let usb = usb
        .into_iter()
        .map(|d| d.with_serial("ATHUB-USB"))
        .collect();
    let mut u = attach(&hub, line);
    let w = attach(&hub, usb);
    u.serials.extend(w.serials.clone());
    let v = open(&hub, &admin, &u);
    assert_eq!(code(&v), "not_one_domain", "{v}");
    // A unit without the timeline (firmware before FWTB1).
    let old = attach(&hub, vec![FakeDevice::demo().with_serial("ATHUB-OLD")]);
    let v = open(&hub, &admin, &old);
    assert_eq!(
        (code(&v), v["serial"].as_str()),
        ("no_timeline", Some("ATHUB-OLD")),
        "{v}"
    );
    // An unlocked unit: a holdover.
    w.remotes[0].time_base_fault(TimeBaseFault::Holdover);
    let v = open(&hub, &admin, &w);
    assert_eq!(
        (code(&v), v["serial"].as_str()),
        ("not_locked", Some("ATHUB-USB")),
        "{v}"
    );
    // A Viewer; a Link 1.2 client (upstream-shaped): no family for either.
    let viewer = session(&hub, Role::Viewer);
    assert_eq!(code(&open(&hub, &viewer, &old)), "denied");
    let old_client = hub.open_session(Role::Admin, None).unwrap();
    assert_eq!(code(&open(&hub, &old_client, &old)), "unsupported");
    assert!(!dspi_hub::system::measure::offered_to(2));
    // Nothing was left locked.
    assert_eq!(hub.handle().system_measure_sessions(), 0);
}

#[test]
fn wp_athub_open_refuses_a_unit_another_console_holds_and_clamps_the_lease() {
    let hub = hub();
    let (_tb, devs) = world(World::Line, 2, G1B);
    let u = attach(&hub, devs);
    let other = session(&hub, Role::Control);
    hub.lock_request(&other, u.handles[1], Duration::from_secs(10))
        .unwrap();
    let control = session(&hub, Role::Control);
    let v = open(&hub, &control, &u);
    assert_eq!(
        (code(&v), v["serial"].as_str()),
        ("locked", Some("ATHUB-1")),
        "{v}"
    );
    hub.lock_release(&other, u.handles[1]);
    for (asked, got) in [(0u32, 500u64), (60_000, 5_000), (1_234, 1_234)] {
        let v = ask(
            &hub,
            &control,
            json!({"t": "system.measure.open", "units": u.serials, "lease_ms": asked}),
        );
        assert_eq!(v["lease_ms"], got, "{v}");
        assert_eq!(v["time_base"], 0);
        assert_eq!(v["domain"], 0x21);
        assert_eq!(v["units"][1]["arm_lead_ms"], 150);
        // A second open of the same units is refused while this one lives.
        assert_eq!(code(&open(&hub, &control, &u)), "locked");
        let m = v["session"].as_str().unwrap();
        let c = ask(
            &hub,
            &control,
            json!({"t": "system.measure.close", "session": m}),
        );
        assert_eq!(c["t"], "ok", "{c}");
    }
    // Another console's session is not this one's to drive.
    let (m, _) = open_ok(&hub, &control, &u);
    assert_eq!(code(&status_of(&hub, &other, &m)), "denied");
}

#[test]
fn wp_athub_an_installer_locked_unit_is_refused_to_control() {
    let hub = hub();
    let (_tb, devs) = world(World::Line, 1, G1B);
    let u = attach(&hub, devs);
    let admin = session(&hub, Role::Admin);
    let limits = |msg: Value| -> Value {
        let text = dspi_hub::limits::handle_message(&hub.handle(), &admin, &msg).unwrap();
        serde_json::from_str(&text).unwrap()
    };
    let profile = json!({"outputs": [{"output": 0, "maxGainDb": 3.0}], "lockedPages": ["eq"]});
    let h = u.handles[0];
    assert_eq!(
        limits(json!({"t": "limits.set", "id": 1, "handle": h, "profile": profile}))["t"],
        "ok"
    );
    assert_eq!(
        limits(json!({"t": "limits.pin", "id": 2, "handle": h, "pin": "2468"}))["t"],
        "ok"
    );
    let control = session(&hub, Role::Control);
    let v = open(&hub, &control, &u);
    assert_eq!(
        (code(&v), v["serial"].as_str()),
        ("installer_lock", Some("ATHUB-0")),
        "{v}"
    );
    // An Admin is never locked out.
    assert_eq!(open(&hub, &admin, &u)["t"], "ok");
}

#[test]
fn wp_athub_colliding_domain_ids_with_disagreeing_t_are_not_one_domain() {
    // Two lines whose senders have the same id, started 300 ms apart.
    let a = FakeTimeBase::line(0x33);
    std::thread::sleep(Duration::from_millis(300));
    let b = FakeTimeBase::line(0x33);
    assert_eq!(a.domain_id(), b.domain_id());
    let hub = hub();
    let devs = vec![
        FakeDevice::demo_on_time_base(&a, FakeTimeBaseUnit::Line(AlinkSetup::default()))
            .with_serial("ATHUB-CA"),
        FakeDevice::demo_on_time_base(&b, FakeTimeBaseUnit::Line(AlinkSetup::default()))
            .with_serial("ATHUB-CB"),
    ];
    let u = attach(&hub, devs);
    let admin = session(&hub, Role::Admin);
    let v = open(&hub, &admin, &u);
    assert_eq!(code(&v), "not_one_domain", "{v}");
    assert!(
        v["msg"].as_str().unwrap().contains("do not share a clock"),
        "{v}"
    );
}

// ── run ──────────────────────────────────────────────────────────────────

/// Two units, one segment each, then unit 0 again: every op 9 at `t0 +
/// offset` in each unit's numbering, on the group's time base.
fn run_lands_on_its_samples(w: World, g: FakeGeneration) {
    let hub = hub();
    let (_tb, devs) = world(w, 2, g);
    let u = attach(&hub, devs);
    // Admin: the test reads the units' timelines itself through the lock.
    let owner = session(&hub, Role::Admin);
    let (m, opened) = open_ok(&hub, &owner, &u);
    let tb = opened["time_base"].as_u64().unwrap() as u8;
    let expect_tb = match w {
        World::Line => tbc::TB_ALINK_LINE,
        World::Usb => tbc::TB_USB_SAMPLE,
        World::Spdif => tbc::TB_SPDIF,
    };
    assert_eq!(tb, expect_tb, "{w:?} {g:?}");
    if w == World::Usb {
        // Their origins are two wraps apart.
        let o = |i: usize| opened["units"][i]["origin_offset"].as_i64().unwrap();
        assert_eq!(o(0), 0);
        assert_eq!(o(1).unsigned_abs() % (2048 * 48), 0);
        assert_ne!(o(1), 0, "{opened}");
    }
    for serial in &u.serials {
        let v = stage(&hub, &owner, &m, serial, &sweep(1));
        assert_eq!(v["t"], "ok", "{v}");
        assert_eq!(v["samples"], 48_000);
    }
    // Unit 0, unit 1, unit 0 again after unit 1.
    let r = run(&hub, &owner, &m, &[(0, 0), (1, 48_000), (0, 96_000)]);
    assert_eq!(r["t"], "ok", "{w:?} {g:?}: {r}");
    let t0 = r["t0"].as_u64().unwrap();
    let segs = r["segments"].as_array().unwrap().clone();
    for (k, off) in [0u64, 48_000, 96_000].into_iter().enumerate() {
        assert_eq!(segs[k]["t"].as_u64().unwrap(), t0 + off);
    }
    // One op 9 pending on unit 0: its first segment's, not its second.
    let tl = timeline(&hub, &owner, u.handles[0]);
    assert_eq!(tl.pending, segs[0]["t_unit"].as_u64().unwrap(), "{tl:?}");
    // One op 9 pending per unit at any time: unit 0's second only once its
    // first has started (R6a).
    let (t_first, t_second) = (
        segs[0]["t_unit"].as_u64().unwrap(),
        segs[2]["t_unit"].as_u64().unwrap(),
    );
    until(&hub, &owner, &m, "three starts", || {
        let tl = timeline(&hub, &owner, u.handles[0]);
        if tl.pending == t_second {
            assert_eq!(
                (tl.start, tl.last_result),
                (t_first, TimelineResult::Started),
                "the second op 9 before the first started: {tl:?}"
            );
        }
        u.remotes[0].measure_timed_starts().len() >= 2
            && !u.remotes[1].measure_timed_starts().is_empty()
    });
    let starts = |i: usize| -> Vec<(u8, u64, bool)> {
        u.remotes[i]
            .measure_timed_starts()
            .iter()
            .map(|s| (s.time_base, s.t, s.timed))
            .collect()
    };
    let t_unit = |k: usize| segs[k]["t_unit"].as_u64().unwrap();
    assert_eq!(
        starts(0),
        vec![(expect_tb, t_unit(0), true), (expect_tb, t_unit(2), true)]
    );
    assert_eq!(starts(1), vec![(expect_tb, t_unit(1), true)]);
    // The run ends; the session stays open and clean.
    let st = status_of(&hub, &owner, &m);
    assert_eq!(st["t"], "ok", "{st}");
    assert!(
        st["status"]["t"].as_u64().unwrap() > t0,
        "the time pin: {st}"
    );
    assert!(st["status"]["hub_us"].is_u64(), "{st}");
    assert_eq!(st["status"]["units"].as_array().unwrap().len(), 2);
    let c = ask(
        &hub,
        &owner,
        json!({"t": "system.measure.close", "session": m}),
    );
    assert_eq!(c["t"], "ok");
}

#[test]
fn wp_athub_run_lands_on_its_samples_on_the_line_g1b() {
    run_lands_on_its_samples(World::Line, G1B);
}

#[test]
fn wp_athub_run_lands_on_its_samples_usb_direct_as_s_g1b() {
    run_lands_on_its_samples(World::Usb, G1B);
}

#[test]
fn wp_athub_run_lands_on_its_samples_spdif_g1b() {
    run_lands_on_its_samples(World::Spdif, G1B);
}

#[test]
fn wp_athub_run_lands_on_its_samples_on_the_line_g2b() {
    run_lands_on_its_samples(World::Line, G2B);
}

#[test]
fn wp_athub_run_lands_on_its_samples_usb_direct_g2b() {
    run_lands_on_its_samples(World::Usb, G2B);
}

#[test]
fn wp_athub_bad_schedules_and_bad_configs_are_refused() {
    let hub = hub();
    let (_tb, devs) = world(World::Spdif, 2, G1B);
    let u = attach(&hub, devs);
    let owner = session(&hub, Role::Control);
    let (m, _) = open_ok(&hub, &owner, &u);
    // Not staged yet.
    assert_eq!(code(&run(&hub, &owner, &m, &[(0, 0)])), "bad_schedule");
    // Pink noise never starts by op 9; a time-base slot of the source; RAW;
    // over the hard cap; an output the unit does not have.
    let pink = SiggenConfig {
        signal_type: 3,
        duration_ms: 1_000,
        ..sweep(1)
    };
    for (cfg, serial) in [
        (pink, &u.serials[1]),
        (sweep(0b100), &u.serials[0]),
        (
            SiggenConfig {
                flags: SiggenConfig::FLAG_RAW,
                ..sweep(1)
            },
            &u.serials[1],
        ),
        (
            SiggenConfig {
                level_db: 0.0,
                ..sweep(1)
            },
            &u.serials[1],
        ),
        (sweep(1 << 15), &u.serials[1]),
    ] {
        let v = stage(&hub, &owner, &m, serial, &cfg);
        assert_eq!(code(&v), "bad_config", "{cfg:?}: {v}");
    }
    // The follower may play outputs 2, 3; the source may not.
    assert_eq!(
        stage(&hub, &owner, &m, &u.serials[1], &sweep(0b100))["t"],
        "ok"
    );
    assert_eq!(stage(&hub, &owner, &m, &u.serials[0], &sweep(1))["t"], "ok");
    // A unit's segments closer than the lead (250 ms); an overlap; too long.
    let close = run(&hub, &owner, &m, &[(0, 0), (0, 48_000 + 11_999)]);
    assert_eq!(code(&close), "bad_schedule", "{close}");
    assert!(close["msg"].as_str().unwrap().contains("lead"), "{close}");
    assert_eq!(
        code(&run(&hub, &owner, &m, &[(0, 0), (1, 47_999)])),
        "bad_schedule"
    );
    assert_eq!(
        code(&run(&hub, &owner, &m, &[(0, 0), (1, 121 * RATE)])),
        "bad_schedule"
    );
    // A summed run is one offset for distinct units: fine.
    let summed = run(&hub, &owner, &m, &[(0, 0), (1, 0)]);
    assert_eq!(summed["t"], "ok", "{summed}");
    let segs = summed["segments"].as_array().unwrap();
    assert_eq!(segs[0]["t"], segs[1]["t"]);
}

#[test]
fn wp_athub_the_run_waits_for_the_servos_to_settle() {
    let hub = hub();
    hub.handle()
        .set_system_measure_settle(Duration::from_millis(1_500));
    let (tb, devs) = world(World::Line, 1, G1B);
    let u = attach(&hub, devs);
    let owner = session(&hub, Role::Admin);
    let (m, _) = open_ok(&hub, &owner, &u);
    stage(&hub, &owner, &m, &u.serials[0], &sweep(1));
    let r = run(&hub, &owner, &m, &[(0, 0)]);
    let ahead = r["t0"].as_u64().unwrap() - tb.t_now();
    assert!(ahead >= RATE * 13 / 10, "t0 only {ahead} samples ahead");
    // A second run, settled now: the lead alone.
    until(&hub, &owner, &m, "the first run", || {
        !u.remotes[0].measure_timed_starts().is_empty()
    });
    until(&hub, &owner, &m, "its end", || {
        timeline(&hub, &owner, u.handles[0]).state == tbc::TimelineState::Idle
    });
    let r = run(&hub, &owner, &m, &[(0, 0)]);
    assert_eq!(r["t"], "ok", "{r}");
    let ahead = r["t0"].as_u64().unwrap() - tb.t_now();
    assert!(
        ahead < RATE * 6 / 10,
        "t0 {ahead} samples ahead once settled"
    );
}

// -- review AT-C -- the system model's poll feeds the settling clock
/// ATHUBM's poll notes every locked `EXT_TIME_BASE` entry it reads
/// (`system_measure_note_locked`): a unit the hub has watched locked for the
/// settling time runs at once, where the family alone would start its clock
/// at `open`.
#[test]
fn review_atc_the_system_poll_starts_the_settling_clock() {
    let hub = hub();
    let settle = Duration::from_millis(2_000);
    hub.handle().set_system_measure_settle(settle);
    let (tb, devs) = world(World::Line, 2, G1B);
    let u = attach(&hub, devs);
    let model = json!({
        "senders": [{"id": "tx", "serial": "ATHUB-TX"}],
        "units": [
            {"id": "A", "serial": u.serials[0], "upstream": "tx"},
            {"id": "B", "serial": u.serials[1], "upstream": "tx"}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let st = hub.handle().system_status();
    let noted = Instant::now();
    assert!(
        u.serials
            .iter()
            .all(|x| st.observed.get(x).is_some_and(|o| o.time_base.is_some())),
        "the poll read EXT_TIME_BASE"
    );
    std::thread::sleep(settle + Duration::from_millis(100));
    let owner = session(&hub, Role::Admin);
    let (m, _) = open_ok(&hub, &owner, &u);
    for serial in &u.serials {
        assert_eq!(stage(&hub, &owner, &m, serial, &sweep(1))["t"], "ok");
    }
    let r = run(&hub, &owner, &m, &[(0, 0), (1, 2 * RATE)]);
    assert_eq!(r["t"], "ok", "{r}");
    let ahead = r["t0"].as_u64().unwrap() - tb.t_now();
    assert!(
        ahead < RATE * 6 / 10,
        "t0 {ahead} samples ahead, {:?} after the poll saw the units locked",
        noted.elapsed()
    );
}

/// A unit with two locked entries (R7b) keeps a settling clock per time
/// base: the poll noting its S/PDIF entry never restarts its USB one.
#[test]
fn review_atc_a_two_entry_unit_settles_per_time_base() {
    let hub = hub();
    let settle = Duration::from_millis(2_000);
    hub.handle().set_system_measure_settle(settle);
    let (tb, devs) = world(World::Usb, 2, G1B);
    let mut devs = devs.into_iter();
    let both = Scripted::new(devs.next().unwrap()).with_spdif_source_entry();
    let mut u = Units {
        serials: vec!["ATHUB-0".into(), "ATHUB-1".into()],
        handles: Vec::new(),
        remotes: vec![both.inner.remote()],
    };
    u.handles.push(hub.attach(Box::new(both), 0).unwrap());
    let second = devs.next().unwrap();
    u.remotes.push(second.remote());
    u.handles.push(hub.attach(Box::new(second), 0).unwrap());
    let model = json!({
        "units": [
            {"id": "A", "serial": "ATHUB-0", "transport": "usb_direct", "usbMode": "phase_lock"},
            {"id": "B", "serial": "ATHUB-1", "transport": "usb_direct", "usbMode": "phase_lock"}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let st = hub.handle().system_status();
    let two = st.observed.get("ATHUB-0").unwrap();
    assert_eq!(two.time_base.as_ref().unwrap().entries.len(), 2);
    std::thread::sleep(Duration::from_millis(1_200));
    let _ = hub.handle().system_status();
    std::thread::sleep(Duration::from_millis(1_000));
    let owner = session(&hub, Role::Admin);
    let (m, opened) = open_ok(&hub, &owner, &u);
    assert_eq!(opened["time_base"], tbc::TB_USB_SAMPLE);
    for serial in &u.serials {
        assert_eq!(stage(&hub, &owner, &m, serial, &sweep(1))["t"], "ok");
    }
    let r = run(&hub, &owner, &m, &[(0, 0), (1, 2 * RATE)]);
    assert_eq!(r["t"], "ok", "{r}");
    let t0 = r["t0"].as_u64().unwrap();
    // The domain's numbering is the first unit's: its own S.
    let now = u.remotes[0].time_base_t().unwrap().1;
    let _ = &tb;
    let ahead = t0.saturating_sub(now);
    assert!(ahead < RATE * 6 / 10, "t0 {ahead} samples ahead");
}

// ── aborts ───────────────────────────────────────────────────────────────

/// After an abort, nothing staged can start: op 9 written directly is
/// refused not ready (the fresh-config flag cleared).
fn later_op9_is_refused(hub: &Hub, u: &Units, tb: &FakeTimeBase, time_base: u8) {
    let probe = session(hub, Role::Admin);
    let t = tb.t_now() + RATE / 2;
    for (i, h) in u.handles.iter().enumerate() {
        let tl = timeline(hub, &probe, *h);
        let t_unit = if time_base == tbc::TB_USB_SAMPLE {
            // Its own S: map by its own reading.
            u.remotes[i].time_base_t().unwrap().1 + RATE / 2
        } else {
            t
        };
        let req = dspi_proto::ext::alink::ApplyRequest {
            frame: t_unit,
            op: dspi_proto::ext::alink::ApplyOp::SiggenStart,
            arg: 0,
            value: 0,
        };
        let st = ext_set(
            hub,
            &probe,
            *h,
            dspi_proto::ext::alink::EXT_APPLY_AT_FRAME,
            encode_apply_tb(&req, time_base).to_vec(),
        );
        // Refused at the write, or at its arming (the fake checks both).
        assert!(st == status::OK || st == status::ERROR, "{st}");
        if st == status::OK {
            wait_until("the op 9 refused", || {
                let now = timeline(hub, &probe, *h);
                now.last_result == TimelineResult::RefusedNotReady && now.run_id == tl.run_id
            });
        }
    }
}

#[test]
fn wp_athub_abort_by_owner_control_admin_and_dead_man_stops_every_unit() {
    for (who, g) in [
        ("owner", G1B),
        ("control", G2B),
        ("admin", G1B),
        ("lease", G2B),
    ] {
        let hub = hub();
        let (tb, devs) = world(World::Usb, 2, g);
        let u = attach(&hub, devs);
        let owner = session(&hub, Role::Control);
        let lease = if who == "lease" { 500 } else { 5_000 };
        let v = ask(
            &hub,
            &owner,
            json!({"t": "system.measure.open", "units": u.serials, "lease_ms": lease}),
        );
        let m = v["session"].as_str().unwrap().to_owned();
        for serial in &u.serials {
            // (The lease runs from here; a status renews it.)
            assert_eq!(status_of(&hub, &owner, &m)["t"], "ok");
            assert_eq!(stage(&hub, &owner, &m, serial, &sweep(1))["t"], "ok");
        }
        assert_eq!(status_of(&hub, &owner, &m)["t"], "ok");
        // Unit 1 later: still pending at the abort.
        let r = run(&hub, &owner, &m, &[(0, 0), (1, 2 * RATE)]);
        assert_eq!(r["t"], "ok", "{r}");
        if who == "lease" {
            let ev = aborted_event(&owner);
            assert_eq!(
                (ev["reason"].as_str(), ev["session"].as_str()),
                (Some("lease"), Some(m.as_str()))
            );
        } else {
            let by = match who {
                "owner" => Arc::clone(&owner),
                "control" => session(&hub, Role::Control),
                _ => session(&hub, Role::Admin),
            };
            let a = ask(
                &hub,
                &by,
                json!({"t": "system.measure.abort", "session": m, "reason": "test"}),
            );
            assert_eq!(a["t"], "ok", "{a}");
            assert!(
                a["units"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|x| x["stopped"] == true),
                "{a}"
            );
            assert_eq!(a["broadcast"], "none", "USB has no broadcast");
            // The owner hears nothing from the hub for its own abort; the session is spent.
            assert_eq!(code(&status_of(&hub, &owner, &m)), "aborted");
        }
        // Nothing pending anywhere, the locks free.
        let probe = session(&hub, Role::Admin);
        for h in &u.handles {
            let tl = timeline(&hub, &probe, *h);
            assert_ne!(tl.state, tbc::TimelineState::Pending, "{who}: {tl:?}");
            assert_ne!(tl.state, tbc::TimelineState::Armed, "{who}: {tl:?}");
        }
        assert!(u.remotes[1].measure_timed_starts().is_empty(), "{who}");
        later_op9_is_refused(&hub, &u, &tb, tbc::TB_USB_SAMPLE);
    }
}

#[test]
fn wp_athub_the_broadcast_stop_goes_only_when_every_unit_has_the_ops() {
    for all_sync in [true, false] {
        let hub = hub();
        let tb = FakeTimeBase::line(0x21);
        let sender = FakeDevice::demo_on_time_base(
            &tb,
            FakeTimeBaseUnit::Line(AlinkSetup {
                role: AlinkRole::Sender,
                ..AlinkSetup::default()
            }),
        )
        .with_serial("ATHUB-TX");
        hub.attach(Box::new(sender), 0).unwrap();
        let a = FakeDevice::demo_on_time_base(&tb, FakeTimeBaseUnit::Line(AlinkSetup::default()))
            .with_serial("ATHUB-0");
        let b = FakeDevice::demo_on_time_base(&tb, FakeTimeBaseUnit::Line(AlinkSetup::default()))
            .with_serial("ATHUB-1");
        let b: Box<dyn Transport> = if all_sync {
            Box::new(b)
        } else {
            Box::new(Scripted::new(b).without_siggen_sync())
        };
        hub.attach(Box::new(a), 0).unwrap();
        hub.attach(b, 0).unwrap();
        let model = json!({
            "rooms": [{"id": "main", "name": "Main"}],
            "senders": [{"id": "tx", "name": "Sender", "serial": "ATHUB-TX"}],
            "units": [
                {"id": "A", "name": "A", "serial": "ATHUB-0", "room": "main", "upstream": "tx"},
                {"id": "B", "name": "B", "serial": "ATHUB-1", "room": "main", "upstream": "tx"}
            ]
        });
        hub.handle()
            .system_set_model(serde_json::from_value(model).unwrap())
            .unwrap();
        let owner = session(&hub, Role::Admin);
        let v = ask(
            &hub,
            &owner,
            json!({"t": "system.measure.open", "units": ["ATHUB-0", "ATHUB-1"]}),
        );
        assert_eq!(v["t"], "ok", "{v}");
        let m = v["session"].as_str().unwrap();
        let a = ask(
            &hub,
            &owner,
            json!({"t": "system.measure.abort", "session": m, "reason": "x"}),
        );
        let want = if all_sync { "sender" } else { "none" };
        assert_eq!(a["broadcast"], want, "{a}");
    }
}

#[test]
fn wp_athub_a_disturbance_is_proved_by_the_counter() {
    // The unit's events are not announced: the abort comes from the
    // `disturbances` read, never from counting 0xE4.
    let remote: Arc<std::sync::Mutex<Option<FakeRemote>>> = Arc::default();
    let r2 = Arc::clone(&remote);
    let hub = hub();
    let (_tb, devs) = world(World::Line, 2, G1B);
    let mut devs = devs.into_iter();
    let first = devs.next().unwrap();
    *r2.lock().unwrap() = Some(first.remote());
    let h0 = hub
        .attach(Box::new(Scripted::new(first).without_events()), 0)
        .unwrap();
    let second = devs.next().unwrap();
    hub.attach(Box::new(second), 0).unwrap();
    let owner = session(&hub, Role::Control);
    let v = ask(
        &hub,
        &owner,
        json!({"t": "system.measure.open", "units": ["ATHUB-0", "ATHUB-1"]}),
    );
    let m = v["session"].as_str().unwrap().to_owned();
    let before = v["units"][0]["disturbances"].as_u64().unwrap();
    let r = remote.lock().unwrap().clone().unwrap();
    assert!(r.time_base_fault(TimeBaseFault::Reanchor));
    let ev = aborted_event(&owner);
    assert_eq!(
        (ev["reason"].as_str(), ev["unit"].as_u64()),
        (Some("disturbance"), Some(0)),
        "{ev}"
    );
    // The counter moved on the unit.
    let probe = session(&hub, Role::Admin);
    let (st, b) = ext_get(&hub, &probe, h0, tbc::EXT_TIME_BASE, 64);
    assert_eq!(st, status::OK);
    let read = tbc::TimeBaseRead::decode(&b).unwrap();
    assert!(u64::from(read.entries[0].disturbances) > before);
    assert_eq!(code(&status_of(&hub, &owner, &m)), "aborted");
}

#[test]
fn wp_athub_clip_foreign_preset_and_closed_connection_abort() {
    // A clip event on unit 0.
    {
        let hub = hub();
        let (_tb, devs) = world(World::Line, 2, G1B);
        let u = attach(&hub, devs);
        let owner = session(&hub, Role::Control);
        let (m, _) = open_ok(&hub, &owner, &u);
        u.remotes[0].push(vec![2, 0xE0, 0, 0, 1, 0, 0, 0]);
        let ev = aborted_event(&owner);
        assert_eq!(
            (ev["reason"].as_str(), ev["unit"].as_u64()),
            (Some("clip"), Some(0)),
            "{ev}"
        );
        assert_eq!(code(&status_of(&hub, &owner, &m)), "aborted");
    }
    // A preset loaded elsewhere on unit 1.
    {
        let hub = hub();
        let (_tb, devs) = world(World::Line, 2, G1B);
        let u = attach(&hub, devs);
        let owner = session(&hub, Role::Control);
        open_ok(&hub, &owner, &u);
        u.remotes[1].measure_preset_load_elsewhere(0);
        let ev = aborted_event(&owner);
        assert_eq!(
            (ev["reason"].as_str(), ev["unit"].as_u64()),
            (Some("foreign"), Some(1)),
            "{ev}"
        );
    }
    // A silent whole-sample correction on unit 1: no event, no counter, only
    // `t - output_frames` shows it.
    {
        let hub = hub();
        let (_tb, devs) = world(World::Line, 2, G1B);
        let u = attach(&hub, devs);
        let owner = session(&hub, Role::Control);
        open_ok(&hub, &owner, &u);
        assert!(u.remotes[1].time_base_fault(TimeBaseFault::Correction {
            samples: 3,
            event: false
        }));
        let ev = aborted_event(&owner);
        assert_eq!(
            (ev["reason"].as_str(), ev["unit"].as_u64()),
            (Some("disturbance"), Some(1)),
            "{ev}"
        );
    }
    // The owner's connection closes mid-run: every unit stopped, the locks free.
    {
        let hub = hub();
        let (_tb, devs) = world(World::Line, 2, G1B);
        let u = attach(&hub, devs);
        let owner = session(&hub, Role::Control);
        let (m, _) = open_ok(&hub, &owner, &u);
        for serial in &u.serials {
            stage(&hub, &owner, &m, serial, &sweep(1));
        }
        assert_eq!(run(&hub, &owner, &m, &[(0, 0), (1, 2 * RATE)])["t"], "ok");
        hub.handle().close_session(owner.id());
        wait_until("the session gone", || {
            hub.handle().system_measure_sessions() == 0
        });
        let probe = session(&hub, Role::Admin);
        for h in &u.handles {
            let tl = timeline(&hub, &probe, *h);
            assert!(
                !matches!(
                    tl.state,
                    tbc::TimelineState::Pending | tbc::TimelineState::Armed
                ),
                "{tl:?}"
            );
        }
        assert!(u.remotes[1].measure_timed_starts().is_empty());
    }
}

// ── R7b on a scripted transport ──────────────────────────────────────────

#[test]
fn wp_athub_a_usb_follower_that_is_also_an_spdif_source_takes_part_by_its_usb_entry() {
    let hub = hub();
    let (_tb, devs) = world(World::Usb, 2, G1B);
    let mut devs = devs.into_iter();
    let both = Scripted::new(devs.next().unwrap()).with_spdif_source_entry();
    let remote = both.inner.remote();
    let mut u = Units {
        serials: vec!["ATHUB-0".into(), "ATHUB-1".into()],
        handles: Vec::new(),
        remotes: vec![remote],
    };
    u.handles.push(hub.attach(Box::new(both), 0).unwrap());
    let second = devs.next().unwrap();
    u.remotes.push(second.remote());
    u.handles.push(hub.attach(Box::new(second), 0).unwrap());
    let owner = session(&hub, Role::Admin);
    // The scripted unit does list two entries.
    let (st, b) = ext_get(&hub, &owner, u.handles[0], tbc::EXT_TIME_BASE, 64);
    assert_eq!(st, status::OK);
    assert_eq!(tbc::TimeBaseRead::decode(&b).unwrap().entries.len(), 2);
    let (m, opened) = open_ok(&hub, &owner, &u);
    assert_eq!(opened["time_base"], tbc::TB_USB_SAMPLE);
    assert_eq!(opened["units"][0]["role"], 0, "the follower entry");
    for serial in &u.serials {
        assert_eq!(stage(&hub, &owner, &m, serial, &sweep(1))["t"], "ok");
    }
    let r = run(&hub, &owner, &m, &[(0, 0), (1, RATE)]);
    assert_eq!(r["t"], "ok", "{r}");
    until(&hub, &owner, &m, "both starts", || {
        u.remotes
            .iter()
            .all(|x| !x.measure_timed_starts().is_empty())
    });
    for (i, x) in u.remotes.iter().enumerate() {
        let s = &x.measure_timed_starts()[0];
        assert_eq!(s.time_base, tbc::TB_USB_SAMPLE, "S, never time base 2");
        assert_eq!(s.t, r["segments"][i]["t_unit"].as_u64().unwrap());
    }
}

/// A fake behind a script: an extra `EXT_TIME_BASE` entry (an S/PDIF source
/// beside its own), no events announced, or no `SIGGEN_SYNC` bit.
struct Scripted {
    inner: FakeDevice,
    spdif_entry: bool,
    no_events: bool,
    no_sync: bool,
}

impl Scripted {
    fn new(inner: FakeDevice) -> Self {
        Self {
            inner,
            spdif_entry: false,
            no_events: false,
            no_sync: false,
        }
    }
    fn with_spdif_source_entry(mut self) -> Self {
        self.spdif_entry = true;
        self
    }
    fn without_events(mut self) -> Self {
        self.no_events = true;
        self
    }
    fn without_siggen_sync(mut self) -> Self {
        self.no_sync = true;
        self
    }

    fn script(&self, opcode: u8, value: u16, mut b: Vec<u8>) -> Vec<u8> {
        if opcode != op::REQ_EXTENSION {
            return b;
        }
        if value == tbc::EXT_TIME_BASE && self.spdif_entry && b.len() >= 60 && b[1] == 1 {
            b[1] = 2;
            let e = 12 + 24;
            b[e] = tbc::TB_SPDIF;
            b[e + 1] = tbc::TB_ROLE_SOURCE;
            b[e + 2] = tbc::TB_STATE_LOCKED;
            b[e + 3] = 0;
            b[e + 4..e + 8].copy_from_slice(&0x0301_4321u32.to_le_bytes());
            b[e + 8..e + 16].copy_from_slice(&777_000u64.to_le_bytes());
            b[e + 16..e + 24].fill(0);
        }
        if value == gext::EXT_NOTIFY_FEATURES && self.no_events {
            b.iter_mut().for_each(|x| *x = 0);
        }
        if value == EXT_ALINK_INFO
            && self.no_sync
            && let Some(mut info) = AlinkInfo::decode(&b)
        {
            info.feature_bits &= !tbc::FEATURE_SIGGEN_SYNC;
            b = info.encode().to_vec();
        }
        b
    }
}

impl Transport for Scripted {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        let b = self.inner.control_in(opcode, value, len)?;
        Ok(self.script(opcode, value, b))
    }
    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        let b = self.inner.control_in_upto(opcode, value, max_len)?;
        Ok(self.script(opcode, value, b))
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
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
    fn generation(&self) -> Option<dspi_transport::Generation> {
        self.inner.generation()
    }
}

// ── The lock budget (coordinator's decision, AT-C) ────────────────────────

/// A measuring session's own locks are exempt from the router's budget (120
/// s, then 30 s cooling) while its lease lives: settling (40 s) and runs for
/// more than 120 s in all, never aborted `lock`. Real time: about 2 minutes.
#[test]
fn wp_athub_a_session_longer_than_the_lock_budget_is_not_aborted() {
    let hub = hub();
    hub.handle()
        .set_system_measure_settle(dspi_hub::system::measure::DEFAULT_SETTLE);
    let (tb, devs) = world(World::Line, 2, G1B);
    let u = attach(&hub, devs);
    let owner = session(&hub, Role::Control);
    let started = Instant::now();
    let (m, _) = open_ok(&hub, &owner, &u);
    for serial in &u.serials {
        assert!(hub.handle().system_measure_lock_exempt(serial), "{serial}");
        assert_eq!(stage(&hub, &owner, &m, serial, &sweep(1))["t"], "ok");
    }
    let mut runs = 0;
    while started.elapsed() < Duration::from_secs(130) {
        let r = run(&hub, &owner, &m, &[(0, 0), (1, 48_000)]);
        if r["t"] != "ok" {
            assert!(
                r["msg"].as_str().unwrap_or("").contains("in progress"),
                "{r}"
            );
            assert_eq!(status_of(&hub, &owner, &m)["t"], "ok");
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        runs += 1;
        let end = r["t0"].as_u64().unwrap() + 2 * RATE;
        let wait = Instant::now();
        while tb.t_now() < end {
            assert!(
                wait.elapsed() < Duration::from_secs(60),
                "run {runs} never ended"
            );
            let st = status_of(&hub, &owner, &m);
            assert_eq!(st["t"], "ok", "after {:?}: {st}", started.elapsed());
            std::thread::sleep(Duration::from_millis(300));
        }
        while u.remotes[1].measure_timed_starts().len() < runs {
            assert_eq!(status_of(&hub, &owner, &m)["t"], "ok");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    assert!(runs >= 3, "{runs} runs");
    assert_eq!(
        status_of(&hub, &owner, &m)["t"],
        "ok",
        "still alive after {:?}",
        started.elapsed()
    );
    for serial in &u.serials {
        assert!(hub.handle().system_measure_lock_exempt(serial));
    }
    // Still the session's: nobody else gets the units.
    let other = session(&hub, Role::Control);
    assert!(
        hub.lock_request(&other, u.handles[0], Duration::from_secs(1))
            .is_err()
    );
    let c = ask(
        &hub,
        &owner,
        json!({"t": "system.measure.close", "session": m}),
    );
    assert_eq!(c["t"], "ok");
    assert!(!hub.handle().system_measure_lock_exempt(&u.serials[0]));
}

/// When the lease lapses the session is aborted, its locks are released and
/// lose the exemption; anyone's next lock (a non-measurement one) is
/// budgeted as before.
#[test]
fn wp_athub_a_lapsed_lease_ends_the_exemption() {
    let hub = hub();
    let (_tb, devs) = world(World::Line, 2, G1B);
    let u = attach(&hub, devs);
    let owner = session(&hub, Role::Control);
    let v = ask(
        &hub,
        &owner,
        json!({"t": "system.measure.open", "units": u.serials, "lease_ms": 500}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    assert!(hub.handle().system_measure_lock_exempt(&u.serials[0]));
    let ev = aborted_event(&owner);
    assert_eq!(ev["reason"], "lease");
    for (serial, h) in u.serials.iter().zip(&u.handles) {
        assert!(!hub.handle().system_measure_lock_exempt(serial), "{serial}");
        // Released: another console takes it, without the exemption.
        let other = session(&hub, Role::Control);
        hub.lock_request(&other, *h, Duration::from_secs(5))
            .unwrap();
        assert!(!hub.handle().system_measure_lock_exempt(serial));
        hub.lock_release(&other, *h);
    }
    // The measuring session's own plain lock is budgeted too.
    let granted = hub
        .lock_request(&owner, u.handles[0], Duration::from_secs(3600))
        .unwrap();
    assert_eq!(granted, dspi_hub::lock::LOCK_MAX);
    assert!(!hub.handle().system_measure_lock_exempt(&u.serials[0]));
}
