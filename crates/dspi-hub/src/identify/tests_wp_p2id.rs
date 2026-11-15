//! WP P2ID's done criteria, on the demo device (`fake/diag.rs` answers 0x0004) through
//! an in-process hub with the identify service on: blink, stop, a tone output out of
//! range, the tone `locked` under another session's lock while the blink passes, the
//! rate limit, a Viewer refused, `unsupported` with no identify transfer, the history
//! record on the bus.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_link::Dir;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, Transport,
    TransportCaps,
};
use serde_json::{Value, json};

use super::*;
use crate::clock::{HubClock, TestClock};
use crate::hub::{Hub, HubConfig};

/// One transfer that reached a device: `(dir, opcode, wValue, payload)`.
type Transfer = (Dir, u8, u16, Vec<u8>);

/// What reached a device.
#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<Transfer>>>);

impl Wire {
    fn ext(&self, dir: Dir, value: u16) -> Vec<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, o, v, _)| *d == dir && *o == op::REQ_EXTENSION && *v == value)
            .map(|(_, _, _, p)| p.clone())
            .collect()
    }

    fn identify_writes(&self) -> Vec<Vec<u8>> {
        self.ext(Dir::Set, gext::EXT_IDENTIFY)
    }

    fn identify_any(&self) -> usize {
        self.identify_writes().len() + self.ext(Dir::Get, gext::EXT_IDENTIFY).len()
    }
}

struct Probe {
    inner: FakeDevice,
    wire: Wire,
    descriptor: DeviceDescriptor,
}

impl Transport for Probe {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.wire
            .0
            .lock()
            .unwrap()
            .push((Dir::Get, o, v, Vec::new()));
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.wire
            .0
            .lock()
            .unwrap()
            .push((Dir::Get, o, v, Vec::new()));
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.wire
            .0
            .lock()
            .unwrap()
            .push((Dir::Set, o, v, d.to_vec()));
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
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

/// The demo device as firmware from wp/FWX (identify, reset info).
fn with_identify() -> FakeDevice {
    FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            diag_ext: true,
            ..FakeOptions::default()
        },
    )
}

struct Rig {
    hub: Hub,
    clock: Arc<TestClock>,
}

impl Rig {
    fn new() -> Self {
        let clock = Arc::new(TestClock::new());
        let hub = Hub::start(HubConfig {
            scan_interval: Duration::from_millis(10),
            services: ServiceConfig {
                clock: Arc::clone(&clock) as Arc<dyn HubClock>,
                identify: true,
                ..ServiceConfig::default()
            },
            ..HubConfig::in_process("P2ID hub")
        })
        .unwrap();
        Self { hub, clock }
    }

    fn plug(&self, dev: FakeDevice, serial: &str) -> (u8, Wire) {
        let wire = Wire::default();
        let probe = Probe {
            inner: dev,
            wire: wire.clone(),
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
            },
        };
        (self.hub.attach(Box::new(probe), 0).unwrap(), wire)
    }

    fn session(&self, role: Role, name: &str) -> Arc<Session> {
        self.hub.handle().open_principal(role, None, name).unwrap()
    }

    fn ask(&self, session: &Arc<Session>, fields: Value) -> Value {
        let mut msg = json!({"t": MESSAGE, "id": 7});
        for (k, v) in fields.as_object().unwrap() {
            msg[k] = v.clone();
        }
        let text = self
            .hub
            .handle()
            .service_message(session, &msg)
            .expect("the identify service answers");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["id"], 7, "{v}");
        assert_eq!(v["v"], 1, "{v}");
        v
    }

    /// Past the rate window.
    fn later(&self) {
        self.clock.advance(RATE_WINDOW);
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.hub.stop();
    }
}

#[test]
fn wp_p2id_the_request_parses_and_is_bounded() {
    let a = IdentifyAsk::parse(&json!({"serial": "S", "seconds": 500})).unwrap();
    assert_eq!(a.seconds, IDENTIFY_MAX_S, "longer is shortened");
    assert_eq!(a.tone_output, None);
    let a = IdentifyAsk::parse(&json!({"handle": 3, "seconds": 0, "tone_output": 2})).unwrap();
    assert_eq!((a.handle, a.seconds, a.tone_output), (Some(3), 0, Some(2)));
    for bad in [
        json!({"seconds": 5}),
        json!({"serial": "S"}),
        json!({"serial": "S", "seconds": -1}),
        json!({"serial": "S", "seconds": 1.5}),
        json!({"serial": "S", "seconds": "10"}),
        json!({"serial": "", "seconds": 1}),
        json!({"serial": "x".repeat(65), "seconds": 1}),
        json!({"serial": 12, "seconds": 1}),
        json!({"handle": 256, "seconds": 1}),
        json!({"handle": -1, "seconds": 1}),
        json!({"serial": "S", "seconds": 1, "tone_output": 255}),
        json!({"serial": "S", "seconds": 1, "tone_output": -2}),
        json!({"serial": "S", "seconds": 1, "tone_output": "1"}),
    ] {
        assert_eq!(
            IdentifyAsk::parse(&bad).unwrap_err().code,
            "bad_request",
            "{bad}"
        );
    }
}

#[test]
fn wp_p2id_the_rate_gate_lets_one_through_per_window() {
    let clock = TestClock::new();
    let mut gate = RateGate::default();
    let t0 = clock.now();
    assert!(gate.take("A", t0).is_ok());
    let wait = gate.take("A", t0 + Duration::from_millis(500)).unwrap_err();
    assert_eq!(wait, Duration::from_millis(1500));
    assert!(gate.take("B", t0).is_ok(), "per device");
    assert!(gate.take("A", t0 + RATE_WINDOW).is_ok());
    // Bounded: old entries go.
    for k in 0..5000 {
        let _ = gate.take(&format!("D{k}"), t0 + RATE_WINDOW * 3);
    }
    assert!(gate.last.len() <= MAX_REMEMBERED);
}

#[test]
fn wp_p2id_blink_then_stop_on_the_demo_device() {
    let rig = Rig::new();
    let (handle, wire) = rig.plug(with_identify(), "P2ID-BLINK");
    let control = rig.session(Role::Control, "Front of house");

    let ok = rig.ask(&control, json!({"serial": "P2ID-BLINK", "seconds": 10}));
    assert_eq!(ok["t"], "ok", "{ok}");
    let lists = wire.ext(Dir::Get, gext::EXT_LIST).len();
    assert!(lists >= 1);
    assert_eq!(ok["serial"], "P2ID-BLINK");
    let left = ok["status"]["remaining_s"].as_u64().unwrap();
    assert!((9..=10).contains(&left), "{ok}");
    assert_eq!(
        ok["status"]["last_tone"],
        Value::Null,
        "no tone unless asked"
    );
    assert_eq!(ok["status"]["chip"], "RP2350");
    assert_eq!(ok["status"]["hw_rev"], dspi_transport::fake::FAKE_HW_REV);
    assert!(ok["status"]["board"].is_string());
    assert_eq!(wire.identify_writes(), vec![vec![10, 0xFF, 0, 0]]);

    rig.later();
    let stop = rig.ask(&control, json!({"handle": handle, "seconds": 0}));
    assert_eq!(stop["t"], "ok", "{stop}");
    assert_eq!(stop["status"]["remaining_s"], 0, "{stop}");
    assert_eq!(wire.identify_writes().last().unwrap(), &vec![0, 0xFF, 0, 0]);

    // An Admin may ask too; past 120 s is shortened; the tone when asked.
    rig.later();
    let admin = rig.session(Role::Admin, "Installer");
    let long = rig.ask(
        &admin,
        json!({"serial": "P2ID-BLINK", "handle": handle, "seconds": 900, "tone_output": 8}),
    );
    assert_eq!(long["t"], "ok", "{long}");
    assert_eq!(long["status"]["last_tone"], 8, "{long}");
    assert!(long["status"]["remaining_s"].as_u64().unwrap() > 100);
    assert_eq!(wire.identify_writes().last().unwrap(), &vec![120, 8, 0, 0]);
    // EXT_LIST was read once, for the first.
    assert_eq!(wire.ext(Dir::Get, gext::EXT_LIST).len(), lists);
}

#[test]
fn wp_p2id_a_tone_output_out_of_range_is_refused_with_nothing_sent() {
    let rig = Rig::new();
    let (_, wire) = rig.plug(with_identify(), "P2ID-RANGE");
    let control = rig.session(Role::Control, "c");
    // The demo has 9 outputs: 0..=8.
    let err = rig.ask(
        &control,
        json!({"serial": "P2ID-RANGE", "seconds": 5, "tone_output": 9}),
    );
    assert_eq!(err["t"], "err");
    assert_eq!(err["code"], "bad_request", "{err}");
    assert_eq!(wire.identify_any(), 0);
    // Not counted against the rate: a good one goes at once.
    let ok = rig.ask(
        &control,
        json!({"serial": "P2ID-RANGE", "seconds": 5, "tone_output": 0}),
    );
    assert_eq!(ok["t"], "ok", "{ok}");
}

#[test]
fn wp_p2id_the_tone_is_locked_under_another_sessions_lock_and_the_blink_passes() {
    let rig = Rig::new();
    let (handle, wire) = rig.plug(with_identify(), "P2ID-LOCK");
    let hub = rig.hub.handle();
    let a = rig.session(Role::Control, "A");
    let b = rig.session(Role::Control, "B");
    assert!(hub.lock_acquire(&b, handle, Duration::from_secs(30)));

    let tone = rig.ask(
        &a,
        json!({"serial": "P2ID-LOCK", "seconds": 5, "tone_output": 1}),
    );
    assert_eq!(tone["code"], "locked", "{tone}");
    assert_eq!(wire.identify_any(), 0, "nothing sent");

    let blink = rig.ask(&a, json!({"serial": "P2ID-LOCK", "seconds": 5}));
    assert_eq!(blink["t"], "ok", "the blink passes the lock: {blink}");
    assert_eq!(wire.identify_writes(), vec![vec![5, 0xFF, 0, 0]]);
    // The lock still holds for everything else of A's, and for the hub's other
    // internal writes (the pass was for that one write).
    assert_eq!(dev_lock_holder(&rig, "P2ID-LOCK"), Some(b.id()));
    let other = request(handle, Dir::Set, gext::EXT_IDENTIFY, vec![1, 0xFF, 0, 0], 0);
    let dev = hub.device_by_serial("P2ID-LOCK").unwrap();
    assert_eq!(
        exec(&dev, other, false).unwrap_err().status,
        status::LOCKED,
        "an unpassed internal write stays locked out"
    );

    // The lock's holder may play the tone.
    rig.later();
    let own = rig.ask(
        &b,
        json!({"serial": "P2ID-LOCK", "seconds": 5, "tone_output": 1}),
    );
    assert_eq!(own["t"], "ok", "{own}");
    assert_eq!(own["status"]["last_tone"], 1);
    // Released, A's tone goes.
    hub.lock_release(&b, handle);
    rig.later();
    let after = rig.ask(
        &a,
        json!({"serial": "P2ID-LOCK", "seconds": 5, "tone_output": 2}),
    );
    assert_eq!(after["t"], "ok", "{after}");
}

fn dev_lock_holder(rig: &Rig, serial: &str) -> Option<u16> {
    rig.hub
        .handle()
        .device_by_serial(serial)
        .unwrap()
        .router
        .lock_holder(Instant::now())
}

#[test]
fn wp_p2id_one_request_per_device_per_two_seconds() {
    let rig = Rig::new();
    let (_, wire) = rig.plug(with_identify(), "P2ID-RATE");
    let (_, other_wire) = rig.plug(with_identify(), "P2ID-RATE2");
    let a = rig.session(Role::Control, "A");
    let b = rig.session(Role::Control, "B");
    assert_eq!(
        rig.ask(&a, json!({"serial": "P2ID-RATE", "seconds": 3}))["t"],
        "ok"
    );
    rig.clock.advance(Duration::from_millis(500));
    // Whoever asks: the limit is the device's.
    let again = rig.ask(&b, json!({"serial": "P2ID-RATE", "seconds": 3}));
    assert_eq!(again["code"], "rate_limited", "{again}");
    assert_eq!(again["retry_in_ms"], 1500, "{again}");
    assert_eq!(wire.identify_writes().len(), 1);
    // Another device is not limited by it.
    assert_eq!(
        rig.ask(&b, json!({"serial": "P2ID-RATE2", "seconds": 3}))["t"],
        "ok"
    );
    assert_eq!(other_wire.identify_writes().len(), 1);
    rig.clock.advance(Duration::from_millis(1500));
    assert_eq!(
        rig.ask(&b, json!({"serial": "P2ID-RATE", "seconds": 3}))["t"],
        "ok"
    );
    assert_eq!(wire.identify_writes().len(), 2);
}

#[test]
fn wp_p2id_a_viewer_is_refused_with_nothing_sent() {
    let rig = Rig::new();
    let (_, wire) = rig.plug(with_identify(), "P2ID-VIEW");
    let viewer = rig.session(Role::Viewer, "v");
    let err = rig.ask(&viewer, json!({"serial": "P2ID-VIEW", "seconds": 5}));
    assert_eq!(err["code"], "denied", "{err}");
    assert_eq!(wire.identify_any(), 0);
    assert!(wire.ext(Dir::Get, gext::EXT_LIST).is_empty());
}

#[test]
fn wp_p2id_unsupported_without_ext_identify_and_no_transfer_sent() {
    let rig = Rig::new();
    // The demo device as bofr-main before FWX: an `EXT_LIST` without 0x0004.
    let (_, wire) = rig.plug(FakeDevice::demo(), "P2ID-OLD");
    // Stock firmware: `EXT_LIST` stalls.
    let stock = FakeDevice::new(Capture::demo(), FakeOptions::default());
    let (_, stock_wire) = rig.plug(stock, "P2ID-STOCK");
    let control = rig.session(Role::Control, "c");
    let mut lists = (0, 0);
    for round in 0..2 {
        for serial in ["P2ID-OLD", "P2ID-STOCK"] {
            let err = rig.ask(
                &control,
                json!({"serial": serial, "seconds": 5, "tone_output": 0}),
            );
            assert_eq!(err["code"], "unsupported", "{serial}: {err}");
        }
        let now = (
            wire.ext(Dir::Get, gext::EXT_LIST).len(),
            stock_wire.ext(Dir::Get, gext::EXT_LIST).len(),
        );
        if round == 1 {
            assert_eq!(now, lists, "EXT_LIST is read once per attach");
        }
        lists = now;
    }
    assert_eq!(wire.identify_any(), 0);
    assert_eq!(stock_wire.identify_any(), 0);
    // No tone fallback (OD-P2i): no generator, no output write.
    for w in [&wire, &stock_wire] {
        let sets: Vec<u8> =
            w.0.lock()
                .unwrap()
                .iter()
                .filter(|(d, ..)| *d == Dir::Set)
                .map(|(_, o, ..)| *o)
                .collect();
        assert!(sets.is_empty(), "nothing written: {sets:?}");
    }
}

#[test]
fn wp_p2id_addressing_unknown_offline_and_mismatched() {
    let rig = Rig::new();
    let (h1, _) = rig.plug(with_identify(), "P2ID-ONE");
    let (h2, _) = rig.plug(with_identify(), "P2ID-TWO");
    let c = rig.session(Role::Control, "c");
    let code = |fields: Value| rig.ask(&c, fields)["code"].as_str().unwrap().to_owned();
    assert_eq!(code(json!({"serial": "NOPE", "seconds": 1})), "not_found");
    assert_eq!(code(json!({"handle": 200, "seconds": 1})), "no_device");
    assert_eq!(
        code(json!({"serial": "P2ID-ONE", "handle": h2, "seconds": 1})),
        "bad_request"
    );
    rig.hub.detach(h1).unwrap();
    assert_eq!(code(json!({"serial": "P2ID-ONE", "seconds": 1})), "offline");
    assert_eq!(code(json!({"handle": h1, "seconds": 1})), "offline");
    // Other messages are not this service's.
    assert!(
        rig.hub
            .handle()
            .service_message(&c, &json!({"t": "device.list", "id": 1}))
            .is_none()
    );
}

#[test]
fn wp_p2id_one_history_record_with_the_caller_in_by() {
    let rig = Rig::new();
    let (_, _wire) = rig.plug(with_identify(), "P2ID-HIST");
    let sub = rig.hub.handle().bus().subscribe("p2id-test", 256);
    let caller = rig.session(Role::Control, "Front of house");
    let ok = rig.ask(
        &caller,
        json!({"serial": "P2ID-HIST", "seconds": 10, "tone_output": 2}),
    );
    assert_eq!(ok["t"], "ok", "{ok}");
    // A refused one writes nothing.
    let refused = rig.ask(&caller, json!({"serial": "P2ID-HIST", "seconds": 10}));
    assert_eq!(refused["code"], "rate_limited");

    let records: Vec<EventRecord> = sub
        .drain()
        .iter()
        .filter_map(|m| match &**m {
            BusMessage::Event { serial, record } if serial == "P2ID-HIST" => Some(record.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1, "{records:?}");
    let r = &records[0];
    assert_eq!(r.source.as_deref(), Some(RECORD_SOURCE));
    assert_eq!(r.by.as_deref(), Some("Front of house"));
    assert_eq!(r.session, Some(u32::from(caller.id())));
    assert_ne!(r.session, Some(u32::from(INTERNAL_SESSION_ID)));
    assert_eq!(r.channel, Some(2));
    assert_eq!(r.value, Some(10.0));
    assert_eq!(r.id, 0, "not yet stored: the history stores it");
    assert_eq!(r.t, rig.clock.unix_ms());
    assert!(r.text.contains("tone on output 3"), "{}", r.text);
    assert!(r.text.contains("Front of house"), "{}", r.text);
}

#[test]
fn wp_p2id_off_by_default_and_in_caps_when_on() {
    let hub = Hub::start(HubConfig::in_process("P2ID off")).unwrap();
    assert!(!hub.handle().service_caps().contains(&CAP));
    let s = hub.handle().open_session(Role::Admin, None).unwrap();
    assert!(
        hub.handle()
            .service_message(
                &s,
                &json!({"t": MESSAGE, "id": 1, "serial": "x", "seconds": 1})
            )
            .is_none()
    );
    hub.stop();
    let rig = Rig::new();
    assert!(rig.hub.handle().service_caps().contains(&CAP));
}
