//! WP P1HA's done criteria, through a running hub with real worker threads and the
//! hub's own `hub-system` thread as the timer: confirm in time keeps the change; a
//! timeout reverts; a dropped client reverts; a replugged different serial is not
//! written; a second begin is refused; a Viewer is refused. Also: the revert passes
//! a lock (the guard's own session's; another session's lock ends the guard, WP
//! P1BFIX), is attributed to the hub, and `apply.cancel` and a late
//! confirm behave.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::session::{INTERNAL_SESSION_ID, Pop};
use dspi_hub::{Completion, Hub, HubConfig, HubHandle, Outbound, Role, Session, apply_guard, lock};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(15);

/// What reached a device: `(dir, opcode)`.
#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<(Dir, u8, u16)>>>);

impl Wire {
    /// Bulk images written: a `0xA1`, or a chunked one's first chunk.
    fn bulk_writes(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, o, v)| {
                *d == Dir::Set
                    && (*o == op::REQ_SET_ALL_PARAMS
                        || (*o == op::REQ_SET_ALL_PARAMS_CHUNK && *v == 0))
            })
            .count()
    }
}

struct Probe {
    inner: FakeDevice,
    wire: Wire,
    descriptor: DeviceDescriptor,
}

impl Transport for Probe {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.wire.0.lock().unwrap().push((Dir::Get, o, v));
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.wire.0.lock().unwrap().push((Dir::Get, o, v));
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.wire.0.lock().unwrap().push((Dir::Set, o, v));
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

fn hub() -> Hub {
    Hub::start(HubConfig {
        scan_interval: Duration::from_millis(10),
        ..HubConfig::in_process("p1ha")
    })
    .unwrap()
}

fn plug(hub: &Hub, serial: &str) -> (u8, Wire) {
    let wire = Wire::default();
    let probe = Probe {
        inner: FakeDevice::demo(),
        wire: wire.clone(),
        descriptor: DeviceDescriptor {
            serial: serial.into(),
            bus_id: "test".into(),
            address: 1,
        },
    };
    let handle = hub.attach(Box::new(probe), 0).unwrap();
    (handle, wire)
}

/// A network client minus the socket.
struct Client {
    hub: HubHandle,
    session: Arc<Session>,
    handle: u8,
}

impl Client {
    fn open(hub: &Hub, handle: u8, role: Role) -> Self {
        let hub = hub.handle();
        let session = hub.open_session(role, None).unwrap();
        Self {
            hub,
            session,
            handle,
        }
    }

    fn command(&self, dir: Dir, code: u8, len: u16, payload: Vec<u8>) -> CmdResponse {
        let (tx, rx) = mpsc::channel();
        let req = CmdRequest {
            tag: 1,
            handle: self.handle,
            dir,
            b_request: code,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: len,
            payload,
        };
        self.hub.submit(
            &self.session,
            req,
            Completion::new(1, move |r| {
                let _ = tx.send(r);
            }),
        );
        rx.recv_timeout(WAIT).unwrap()
    }

    fn set_volume(&self, db: f32) -> u8 {
        self.command(
            Dir::Set,
            op::REQ_SET_MASTER_VOLUME,
            4,
            db.to_le_bytes().to_vec(),
        )
        .status
    }

    fn volume(&self) -> f32 {
        let r = self.command(Dir::Get, op::REQ_GET_MASTER_VOLUME, 4, Vec::new());
        assert_eq!(r.status, status::OK);
        f32::from_le_bytes(r.payload[..4].try_into().unwrap())
    }

    fn msg(&self, msg: Value) -> Value {
        let text = apply_guard::handle_message(&self.hub, &self.session, &msg)
            .expect("an apply.* message is answered");
        serde_json::from_str(&text).unwrap()
    }

    fn begin(&self, timeout_ms: u64) -> Value {
        self.msg(
            json!({"t": "apply.begin", "id": 1, "handle": self.handle, "timeout_ms": timeout_ms}),
        )
    }

    fn confirm(&self) -> Value {
        self.msg(json!({"t": "apply.confirm", "id": 2, "handle": self.handle}))
    }

    fn cancel(&self) -> Value {
        self.msg(json!({"t": "apply.cancel", "id": 3, "handle": self.handle}))
    }

    fn status(&self) -> Value {
        self.msg(json!({"t": "apply.status", "id": 4, "handle": self.handle}))
    }

    /// Wait for the next `apply.ended` this session hears, and the notifications'
    /// origins seen until then.
    fn ended(&self) -> (Value, Vec<u16>) {
        let start = Instant::now();
        let mut origins = Vec::new();
        loop {
            match self.session.outbox().pop(Duration::from_millis(20)) {
                Pop::Item(Outbound::Event(json)) => {
                    let v: Value = serde_json::from_str(&json).unwrap();
                    if v["t"] == "apply.ended" {
                        return (v, origins);
                    }
                }
                Pop::Item(Outbound::Notify { origin, .. }) => origins.push(origin),
                Pop::Item(_) | Pop::Idle => {}
                other => panic!("session ended: {other:?}"),
            }
            assert!(start.elapsed() < WAIT, "no apply.ended");
        }
    }

    /// The origins of the notifications that arrive within `d`.
    fn origins_for(&self, d: Duration) -> Vec<u16> {
        let start = Instant::now();
        let mut origins = Vec::new();
        while start.elapsed() < d {
            if let Pop::Item(Outbound::Notify { origin, .. }) =
                self.session.outbox().pop(Duration::from_millis(20))
            {
                origins.push(origin);
            }
        }
        origins
    }
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

#[test]
fn wp_p1ha_confirm_in_time_keeps_the_change() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-A");
    let c = Client::open(&hub, handle, Role::Control);
    let before = c.volume();
    let begun = c.begin(2_000);
    assert!(ok(&begun), "{begun}");
    assert_eq!(begun["v"], 1);
    assert_eq!(begun["serial"], "P1HA-A");
    assert_eq!(begun["timeout_ms"], 2_000);
    assert_eq!(c.set_volume(before - 12.0), status::OK);
    let kept = c.confirm();
    assert!(ok(&kept), "{kept}");
    assert_eq!(kept["result"], "kept");
    assert_eq!(kept["reason"], "confirmed");
    let (ended, _) = c.ended();
    assert_eq!(ended["result"], "kept");
    // Past the old deadline: nothing is written back.
    std::thread::sleep(Duration::from_millis(2_500));
    assert_eq!(wire.bulk_writes(), 0);
    assert_eq!(c.volume(), before - 12.0);
    let st = c.status();
    assert!(st["active"].is_null(), "{st}");
    assert_eq!(st["last"]["result"], "kept");
}

#[test]
fn wp_p1ha_a_timeout_reverts_as_the_hub() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-B");
    let c = Client::open(&hub, handle, Role::Admin);
    let watcher = Client::open(&hub, handle, Role::Viewer);
    let before = c.volume();
    assert!(ok(&c.begin(1_000)));
    assert_eq!(c.set_volume(before - 20.0), status::OK);
    let _ = watcher.origins_for(Duration::from_millis(200));
    let started = Instant::now();
    let (ended, origins) = watcher.ended();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the timer runs on time"
    );
    assert_eq!(ended["result"], "reverted", "{ended}");
    assert_eq!(ended["reason"], "timeout");
    assert_eq!(ended["serial"], "P1HA-B");
    assert_eq!(wire.bulk_writes(), 1, "one write, never replayed");
    assert_eq!(c.volume(), before);
    // The revert's notification goes to the hub's session, not the client's.
    let mut origins = origins;
    origins.extend(watcher.origins_for(Duration::from_millis(500)));
    assert!(origins.contains(&INTERNAL_SESSION_ID), "{origins:?}");
    // A confirm after the end is refused.
    let late = c.confirm();
    assert_eq!(late["code"], "conflict", "{late}");
    assert_eq!(c.status()["last"]["result"], "reverted");
}

#[test]
fn wp_p1ha_a_dropped_client_reverts() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-C");
    let c = Client::open(&hub, handle, Role::Control);
    let watcher = Client::open(&hub, handle, Role::Viewer);
    let before = c.volume();
    assert!(ok(&c.begin(60_000)));
    assert_eq!(c.set_volume(before - 30.0), status::OK);
    let started = Instant::now();
    c.hub.close_session(c.session.id());
    let (ended, _) = watcher.ended();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "not at the timeout"
    );
    assert_eq!(ended["result"], "reverted", "{ended}");
    assert_eq!(ended["reason"], "session_closed");
    assert_eq!(wire.bulk_writes(), 1);
    assert_eq!(watcher.volume(), before);
}

#[test]
fn wp_p1ha_a_replugged_different_serial_is_not_written() {
    let hub = hub();
    let (handle, wire_a) = plug(&hub, "P1HA-D1");
    let c = Client::open(&hub, handle, Role::Admin);
    let watcher = Client::open(&hub, handle, Role::Viewer);
    assert!(ok(&c.begin(1_500)));
    hub.detach(handle).unwrap();
    let (handle_b, wire_b) = plug(&hub, "P1HA-D2");
    let (ended, _) = watcher.ended();
    assert_eq!(ended["result"], "not_written", "{ended}");
    assert_eq!(ended["status"], "no_device");
    assert_eq!(ended["serial"], "P1HA-D1");
    assert_eq!(wire_b.bulk_writes(), 0, "the other board is never written");
    assert_eq!(wire_a.bulk_writes(), 0);
    // On the new board's handle a new guard may begin.
    let other = Client::open(&hub, handle_b, Role::Control);
    assert!(ok(&other.begin(1_000)));
    assert!(ok(&other.confirm()));
}

#[test]
fn wp_p1ha_a_second_begin_is_refused() {
    let hub = hub();
    let (handle, _) = plug(&hub, "P1HA-E");
    let a = Client::open(&hub, handle, Role::Control);
    let b = Client::open(&hub, handle, Role::Admin);
    assert!(ok(&a.begin(10_000)));
    let second = b.begin(10_000);
    assert_eq!(second["code"], "busy", "{second}");
    let again = a.begin(10_000);
    assert_eq!(again["code"], "busy", "the same session too");
    // Another session cannot end it.
    assert_eq!(b.confirm()["code"], "denied");
    assert_eq!(b.cancel()["code"], "denied");
    assert_eq!(b.status()["active"]["session"], a.session.id());
    assert!(ok(&a.confirm()));
    assert!(ok(&b.begin(10_000)), "free once ended");
    assert!(ok(&b.confirm()));
}

#[test]
fn wp_p1ha_a_viewer_is_refused() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-F");
    let v = Client::open(&hub, handle, Role::Viewer);
    for t in ["apply.begin", "apply.confirm", "apply.cancel"] {
        let answer = v.msg(json!({"t": t, "id": 9, "handle": handle}));
        assert_eq!(answer["code"], "denied", "{t}: {answer}");
    }
    let st = v.status();
    assert!(ok(&st), "a viewer may read the status: {st}");
    assert!(st["active"].is_null());
    assert_eq!(wire.bulk_writes(), 0);
}

/// The revert passes a lock: since WP P1BFIX (review p1b F2) another session's
/// lock ends the guard without a revert (`wp_p1bfix_takeover.rs`), so the lock a
/// revert meets is the guard's own session's.
#[test]
fn wp_p1ha_the_revert_passes_a_lock() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-G");
    let c = Client::open(&hub, handle, Role::Control);
    let other = Client::open(&hub, handle, Role::Admin);
    let before = c.volume();
    assert!(ok(&c.begin(1_000)));
    let locked = lock::handle_message(
        &c.hub,
        &c.session,
        &json!({"t": "lock.acquire", "id": 1, "handle": handle, "timeout_ms": 10_000}),
    )
    .unwrap();
    assert!(locked.contains("\"ok\""), "{locked}");
    assert_eq!(c.set_volume(before - 6.0), status::OK);
    let (ended, _) = other.ended();
    assert_eq!(ended["result"], "reverted", "{ended}");
    assert_eq!(ended["reason"], "timeout", "its own lock does not end it");
    assert_eq!(wire.bulk_writes(), 1);
    assert_eq!(c.volume(), before);
    // The pass was for that one write: the others are still locked out.
    assert_eq!(other.set_volume(before - 3.0), status::LOCKED);
}

#[test]
fn wp_p1ha_cancel_reverts_at_once() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-H");
    let c = Client::open(&hub, handle, Role::Control);
    let before = c.volume();
    assert!(ok(&c.begin(60_000)));
    assert_eq!(c.set_volume(before - 9.0), status::OK);
    let undone = c.cancel();
    assert!(ok(&undone), "{undone}");
    assert_eq!(undone["result"], "reverted");
    assert_eq!(undone["reason"], "cancelled");
    assert_eq!(wire.bulk_writes(), 1);
    assert_eq!(c.volume(), before);
    assert_eq!(c.cancel()["code"], "conflict", "nothing left to cancel");
}

#[test]
fn wp_p1ha_messages_are_checked() {
    let hub = hub();
    let (handle, _) = plug(&hub, "P1HA-I");
    let c = Client::open(&hub, handle, Role::Admin);
    assert_eq!(
        c.msg(json!({"t": "apply.begin", "id": 1}))["code"],
        "bad_request"
    );
    assert_eq!(
        c.msg(json!({"t": "apply.begin", "id": 1, "handle": handle, "timeout_ms": -1}))["code"],
        "bad_request"
    );
    assert_eq!(
        c.msg(json!({"t": "apply.begin", "id": 1, "handle": 200}))["code"],
        "no_device"
    );
    // Clamped to the shortest timeout, and the default is OD-P1g's 30 s.
    let short = c.begin(1);
    assert_eq!(short["timeout_ms"], 1_000, "{short}");
    assert!(ok(&c.confirm()));
    let default = c.msg(json!({"t": "apply.begin", "id": 1, "handle": handle}));
    assert_eq!(default["timeout_ms"], 30_000, "{default}");
    assert!(ok(&c.confirm()));
    assert!(
        apply_guard::handle_message(&c.hub, &c.session, &json!({"t": "device.list"})).is_none()
    );
}

#[test]
fn wp_p1ha_stopping_the_hub_reverts() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1HA-J");
    let c = Client::open(&hub, handle, Role::Control);
    let before = c.volume();
    assert!(ok(&c.begin(60_000)));
    assert_eq!(c.set_volume(before - 15.0), status::OK);
    hub.stop();
    assert_eq!(wire.bulk_writes(), 1, "reverted before the devices went");
}
