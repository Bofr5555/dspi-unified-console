//! WP P1BFIX (review p1b F2): another session taking the device's lock ends a
//! running guarded apply at once, without a revert: the new lock holder is in
//! charge. `apply.ended` says `taken_over`, `kept`, and names the session; the
//! guard's owner hears it. Review p1b's probe (a Control client's guard undoing
//! an Admin's locked work), turned round, plus the timeout form and the hub app's
//! own lock.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::session::Pop;
use dspi_hub::{
    Completion, Hub, HubConfig, HubHandle, LOCAL_SESSION_ID, Outbound, Role, Session, apply_guard,
    lock,
};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(15);

#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<(Dir, u8, u16)>>>);

impl Wire {
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
        ..HubConfig::in_process("p1bfix")
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
            ..Default::default()
        },
    };
    (hub.attach(Box::new(probe), 0).unwrap(), wire)
}

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
        serde_json::from_str(&apply_guard::handle_message(&self.hub, &self.session, &msg).unwrap())
            .unwrap()
    }

    fn lock(&self, timeout_ms: u64) -> String {
        lock::handle_message(
            &self.hub,
            &self.session,
            &json!({"t": "lock.acquire", "id": 1, "handle": self.handle, "timeout_ms": timeout_ms}),
        )
        .unwrap()
    }

    /// The next `apply.ended` this session hears.
    fn ended(&self) -> Value {
        let start = Instant::now();
        loop {
            if let Pop::Item(Outbound::Event(json)) =
                self.session.outbox().pop(Duration::from_millis(20))
            {
                let v: Value = serde_json::from_str(&json).unwrap();
                if v["t"] == "apply.ended" {
                    return v;
                }
            }
            assert!(start.elapsed() < WAIT, "no apply.ended");
        }
    }
}

/// The reviewer's probe, turned round: a Control client begins a guard on an
/// idle device; an Admin takes the lock: the guard ends there, kept, naming the
/// Admin's session, and the Control client hears it. The Admin's work then
/// stays: the Control client has no guard left to cancel, and nothing is ever
/// written back.
#[test]
fn wp_p1bfix_a_lock_taken_during_a_guard_ends_it_without_a_revert() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1BFIX-A");
    let c = Client::open(&hub, handle, Role::Control);
    let admin = Client::open(&hub, handle, Role::Admin);
    let before = c.volume();
    assert_eq!(
        c.msg(json!({"t": "apply.begin", "id": 1, "handle": handle, "timeout_ms": 600_000}))["t"],
        "ok"
    );
    let locked = admin.lock(60_000);
    assert!(locked.contains("\"ok\""), "{locked}");

    let ended = c.ended();
    assert_eq!(ended["reason"], "taken_over", "{ended}");
    assert_eq!(ended["result"], "kept", "{ended}");
    assert_eq!(ended["taken_by"], json!(admin.session.id()), "{ended}");
    assert_eq!(ended["serial"], "P1BFIX-A");
    assert!(ended.get("status").is_none(), "{ended}");

    assert_eq!(admin.set_volume(before - 10.0), status::OK);
    let out = c.msg(json!({"t": "apply.cancel", "id": 2, "handle": handle}));
    assert_eq!(out["code"], "conflict", "nothing left to cancel: {out}");
    let status = c.msg(json!({"t": "apply.status", "id": 3, "handle": handle}));
    assert!(status["active"].is_null(), "{status}");
    assert_eq!(status["last"]["reason"], "taken_over", "{status}");
    // Dropping the Control client reverts nothing either.
    c.hub.close_session(c.session.id());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(wire.bulk_writes(), 0, "never written back");
    assert_eq!(admin.volume(), before - 10.0, "the Admin's work stays");
}

/// The timeout form: the guard's own change stays when another session takes
/// the lock before the deadline; the deadline then passes with nothing written.
#[test]
fn wp_p1bfix_a_taken_over_guard_does_not_revert_at_its_deadline() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1BFIX-B");
    let c = Client::open(&hub, handle, Role::Control);
    let holder = Client::open(&hub, handle, Role::Admin);
    let before = c.volume();
    assert_eq!(
        c.msg(json!({"t": "apply.begin", "id": 1, "handle": handle, "timeout_ms": 1_000}))["t"],
        "ok"
    );
    assert_eq!(c.set_volume(before - 6.0), status::OK);
    assert!(holder.lock(10_000).contains("\"ok\""));
    let ended = holder.ended();
    assert_eq!(
        (ended["reason"].as_str(), ended["result"].as_str()),
        (Some("taken_over"), Some("kept")),
        "{ended}"
    );
    std::thread::sleep(Duration::from_millis(1_500));
    assert_eq!(wire.bulk_writes(), 0);
    assert_eq!(holder.volume(), before - 6.0);
}

/// The guard's own session renewing or taking its lock does not end it, and the
/// hub app's own lock (its `LocalTransport`) ends it as a client's does.
#[test]
fn wp_p1bfix_own_lock_keeps_the_guard_and_the_apps_lock_ends_it() {
    let hub = hub();
    let (handle, wire) = plug(&hub, "P1BFIX-C");
    let c = Client::open(&hub, handle, Role::Control);
    let watcher = Client::open(&hub, handle, Role::Viewer);
    assert_eq!(
        c.msg(json!({"t": "apply.begin", "id": 1, "handle": handle, "timeout_ms": 600_000}))["t"],
        "ok"
    );
    assert!(c.lock(1_000).contains("\"ok\""));
    let status = c.msg(json!({"t": "apply.status", "id": 2, "handle": handle}));
    assert!(!status["active"].is_null(), "still running: {status}");
    lock::handle_message(
        &c.hub,
        &c.session,
        &json!({"t": "lock.release", "id": 3, "handle": handle}),
    )
    .unwrap();

    let mut app = hub.handle().open_local("P1BFIX-C").unwrap();
    app.lock("test", Duration::from_secs(5)).unwrap();
    let ended = watcher.ended();
    assert_eq!(ended["reason"], "taken_over", "{ended}");
    assert_eq!(ended["taken_by"], json!(LOCAL_SESSION_ID), "{ended}");
    app.unlock();
    assert_eq!(wire.bulk_writes(), 0);
}
