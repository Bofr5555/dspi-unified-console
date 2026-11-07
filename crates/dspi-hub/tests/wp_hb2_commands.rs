//! WP HB2's done criteria, through a running hub with real worker threads: two clients
//! and their echoes, roles, locks and their timeouts, the voltage rule on every path,
//! writes never replayed, and what a hostile client cannot do to the others.
#![allow(clippy::arithmetic_side_effects)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::session::Pop;
use dspi_hub::{Completion, Hub, HubConfig, HubHandle, Outbound, Role, Session, lock};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::ext::{CoreVregStatus, VregOp};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::rta;
use dspi_session::ext::{self as sx, ExtError};
use dspi_session::overvolt::WORD;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError, TransportKind, is_own_write,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(10);
const V115: u8 = 0x0C;
const V120: u8 = 0x0D;
const V125: u8 = 0x0E;
const V130: u8 = 0x0F;

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// One call that reached the device.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    dir: Dir,
    code: u8,
    value: u16,
    payload: Vec<u8>,
}

/// What a test holds of a [`Probe`] once the hub has it.
#[derive(Clone, Default)]
struct Wire {
    calls: Arc<Mutex<Vec<Call>>>,
    /// The next OUT calls fail like this, whatever they are.
    fail_out: Arc<Mutex<Option<TransportError>>>,
    /// IN calls of this opcode fail with a stall.
    stall_in: Arc<Mutex<Option<u8>>>,
    /// Calls wait here while it is true.
    held: Arc<(Mutex<bool>, Condvar)>,
    /// How many calls are waiting on `held`.
    waiting: Arc<AtomicUsize>,
}

impl Wire {
    fn hold(&self) {
        *self.held.0.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.held.0.lock().unwrap() = false;
        self.held.1.notify_all();
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    fn outs(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.dir == Dir::Set)
            .collect()
    }

    fn pass(&self) {
        let (lock, cv) = &*self.held;
        let mut held = lock.lock().unwrap();
        if *held {
            self.waiting.fetch_add(1, Ordering::SeqCst);
            while *held {
                held = cv.wait(held).unwrap();
            }
            self.waiting.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// A demo device with a log of what reached it, a gate, and failures to order.
struct Probe {
    inner: FakeDevice,
    wire: Wire,
    descriptor: DeviceDescriptor,
}

impl Probe {
    fn note(&self, dir: Dir, code: u8, value: u16, payload: &[u8]) {
        self.wire.pass();
        self.wire.calls.lock().unwrap().push(Call {
            dir,
            code,
            value,
            payload: payload.to_vec(),
        });
    }

    fn check_in(&self, code: u8) -> dspi_transport::Result<()> {
        if *self.wire.stall_in.lock().unwrap() == Some(code) {
            return Err(TransportError::Stalled { opcode: code });
        }
        Ok(())
    }
}

impl Transport for Probe {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(Dir::Get, o, v, &[]);
        self.check_in(o)?;
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(Dir::Get, o, v, &[]);
        self.check_in(o)?;
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.note(Dir::Set, o, v, d);
        if let Some(e) = self.wire.fail_out.lock().unwrap().clone() {
            return Err(e);
        }
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

/// A running hub with one device on it.
struct Rig {
    hub: Hub,
    handle: u8,
    wire: Wire,
    remote: FakeRemote,
    serial: String,
}

fn hub() -> Hub {
    Hub::start(HubConfig {
        // The tick that notices an expired lock.
        scan_interval: Duration::from_millis(10),
        ..HubConfig::in_process("hb2")
    })
    .unwrap()
}

fn plug(hub: &Hub, serial: &str, inner: FakeDevice) -> (u8, Wire, FakeRemote) {
    let remote = inner.remote();
    let wire = Wire::default();
    let probe = Probe {
        inner,
        wire: wire.clone(),
        descriptor: DeviceDescriptor {
            serial: serial.into(),
            bus_id: "test".into(),
            address: 1,
        },
    };
    let handle = hub.attach(Box::new(probe), 0).unwrap();
    wire.clear();
    (handle, wire, remote)
}

fn rig_with(inner: FakeDevice) -> Rig {
    let hub = hub();
    let serial = "HB2".to_owned();
    let (handle, wire, remote) = plug(&hub, &serial, inner);
    Rig {
        hub,
        handle,
        wire,
        remote,
        serial,
    }
}

fn rig() -> Rig {
    rig_with(FakeDevice::demo())
}

/// A unit that boots at this voltage, nothing stored.
fn rig_at(code: u8) -> Rig {
    let r = rig();
    r.remote.vreg_set_build_default(code);
    r
}

/// A network client, minus the socket: commands in, frames out.
struct Client {
    hub: HubHandle,
    session: Arc<Session>,
    handle: u8,
}

fn request(handle: u8, dir: Dir, code: u8, value: u16, len: u16, payload: Vec<u8>) -> CmdRequest {
    CmdRequest {
        tag: 1,
        handle,
        dir,
        b_request: code,
        w_value: value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: len,
        payload,
    }
}

impl Client {
    fn open(r: &Rig, role: Role) -> Self {
        let hub = r.hub.handle();
        let session = hub.open_session(role, None).unwrap();
        Self {
            hub,
            session,
            handle: r.handle,
        }
    }

    fn id(&self) -> u16 {
        self.session.id()
    }

    fn submit(&self, req: CmdRequest) -> Receiver<CmdResponse> {
        let (tx, rx) = mpsc::channel();
        self.hub.submit(
            &self.session,
            req,
            Completion::new(1, move |r| {
                let _ = tx.send(r);
            }),
        );
        rx
    }

    fn command(&self, dir: Dir, code: u8, value: u16, len: u16, payload: Vec<u8>) -> CmdResponse {
        self.submit(request(self.handle, dir, code, value, len, payload))
            .recv_timeout(WAIT)
            .unwrap()
    }

    fn set(&self, code: u8, value: u16, payload: &[u8]) -> u8 {
        let len = u16::try_from(payload.len()).unwrap();
        self.command(Dir::Set, code, value, len, payload.to_vec())
            .status
    }

    fn get(&self, code: u8, value: u16, len: u16) -> CmdResponse {
        self.command(Dir::Get, code, value, len, Vec::new())
    }

    fn set_volume(&self, db: f32) -> u8 {
        self.set(op::REQ_SET_MASTER_VOLUME, 0, &db.to_le_bytes())
    }

    fn vreg(&self, request: VregOp) -> u8 {
        self.set(op::REQ_EXTENSION, g::EXT_CORE_VREG, &request.encode())
    }

    fn vreg_status(&self) -> CoreVregStatus {
        let reply = self.get(
            op::REQ_EXTENSION,
            g::EXT_CORE_VREG,
            CoreVregStatus::SIZE as u16,
        );
        assert_eq!(reply.status, status::OK);
        CoreVregStatus::decode(&reply.payload).unwrap()
    }

    /// The next notification, with its `(origin, source byte)`.
    fn next_notify(&self) -> (u16, u8) {
        let start = Instant::now();
        loop {
            match self.session.outbox().pop(Duration::from_millis(20)) {
                Pop::Item(Outbound::Notify { origin, packet, .. }) => {
                    return (origin, dspi_hub::relay::packet_source(&packet).unwrap().0);
                }
                Pop::Item(_) => {}
                Pop::Idle if start.elapsed() < WAIT => {}
                other => panic!("no notification: {other:?}"),
            }
        }
    }

    /// The control-plane messages queued, without waiting.
    fn events(&self) -> Vec<Value> {
        let mut all = Vec::new();
        while let Pop::Item(item) = self.session.outbox().pop(Duration::ZERO) {
            if let Outbound::Event(json) = item {
                all.push(serde_json::from_str(&json).unwrap());
            }
        }
        all
    }
}

// -- Two clients, and echoes ------------------------------------------------------

/// The done criterion: each client sees the other's write, and drops its own by the
/// rule every client runs (`is_own_write`).
#[test]
fn wp_hb2_two_clients_see_each_others_writes_and_drop_their_own_echo() {
    let r = rig();
    let (a, b) = (
        Client::open(&r, Role::Control),
        Client::open(&r, Role::Control),
    );
    assert_ne!(a.id(), b.id());
    let own = |c: &Client, origin: u16, source: u8| {
        is_own_write(
            TransportKind::Link,
            source,
            Some(origin),
            Some(c.id()),
            false,
        )
    };

    assert_eq!(a.set_volume(-11.0), status::OK);
    let (origin, source) = a.next_notify();
    assert_eq!(origin, a.id());
    assert!(own(&a, origin, source), "its own echo: dropped");
    let (origin, source) = b.next_notify();
    assert_eq!(origin, a.id());
    assert!(!own(&b, origin, source), "the other's write: applied");

    assert_eq!(b.set_volume(-12.0), status::OK);
    let (origin, source) = a.next_notify();
    assert_eq!(origin, b.id());
    assert!(!own(&a, origin, source));
    let (origin, source) = b.next_notify();
    assert_eq!(origin, b.id());
    assert!(own(&b, origin, source));
}

/// Writes of two clients in a row go to their own writers, in order.
#[test]
fn wp_hb2_origins_follow_the_order_of_the_writes() {
    let r = rig();
    let (a, b) = (
        Client::open(&r, Role::Control),
        Client::open(&r, Role::Control),
    );
    let watcher = Client::open(&r, Role::Viewer);
    assert_eq!(a.set_volume(-11.0), status::OK);
    assert_eq!(watcher.next_notify().0, a.id());
    assert_eq!(b.set_volume(-12.0), status::OK);
    assert_eq!(watcher.next_notify().0, b.id());
}

// -- Roles ------------------------------------------------------------------------

/// By the table, at a running hub: what is refused never touches the device.
#[test]
fn wp_hb2_roles_follow_the_command_table() {
    let r = rig();
    let volume = (-10.0f32).to_le_bytes();
    for (role, read, control, config) in [
        (Role::Viewer, status::OK, status::DENIED, status::DENIED),
        (Role::Control, status::OK, status::OK, status::DENIED),
        (Role::Admin, status::OK, status::OK, status::OK),
    ] {
        let c = Client::open(&r, role);
        r.wire.clear();
        assert_eq!(c.get(op::REQ_GET_PLATFORM, 0, 8).status, read, "{role:?}");
        assert_eq!(
            c.set(op::REQ_SET_MASTER_VOLUME, 0, &volume),
            control,
            "{role:?}"
        );
        // Config: an extension write the voltage rule has nothing to say about.
        assert_eq!(
            c.set(
                op::REQ_EXTENSION,
                g::EXT_CROSSFEED_PAIR_MAP,
                &[0x10, 0, 0, 0]
            ),
            config,
            "{role:?}"
        );
        let touched = r.wire.calls().len();
        let want = 1 + usize::from(control == status::OK) + usize::from(config == status::OK);
        assert_eq!(touched, want, "{role:?}: {:?}", r.wire.calls());
    }
}

#[test]
fn wp_hb2_a_role_changed_mid_session_decides_the_next_command() {
    let r = rig();
    let c = Client::open(&r, Role::Control);
    assert_eq!(c.set_volume(-10.0), status::OK);
    c.session.set_role(Role::Viewer);
    assert_eq!(c.set_volume(-11.0), status::DENIED);
}

/// `0x00` is `config` both ways (HUB.md section 13), and the status read is no exception.
#[test]
fn wp_hb2_only_an_admin_reaches_the_extension_channel() {
    let r = rig();
    for role in [Role::Viewer, Role::Control] {
        let c = Client::open(&r, role);
        r.wire.clear();
        assert_eq!(
            c.get(op::REQ_EXTENSION, g::EXT_LIST, 64).status,
            status::DENIED
        );
        assert_eq!(
            c.get(op::REQ_EXTENSION, g::EXT_CORE_VREG, 16).status,
            status::DENIED
        );
        assert!(r.wire.calls().is_empty());
    }
    let admin = Client::open(&r, Role::Admin);
    assert_eq!(
        admin.get(op::REQ_EXTENSION, g::EXT_LIST, 64).status,
        status::OK
    );
}

#[test]
fn wp_hb2_the_hub_hands_out_each_roles_denial_list() {
    let r = rig();
    assert!(r.hub.denied(Role::Admin).is_empty());
    assert!(r.hub.denied(Role::Viewer).len() > r.hub.denied(Role::Control).len());
}

// -- The voltage rule -------------------------------------------------------------

/// The owner's example, raw through the hub: any role, nothing reaches the unit.
#[test]
fn wp_hb2_the_owners_example_is_refused_for_every_role() {
    let r = rig_at(V125);
    for role in [Role::Viewer, Role::Control, Role::Admin] {
        let c = Client::open(&r, role);
        r.wire.clear();
        assert_eq!(c.vreg(VregOp::Set(V130)), status::DENIED, "{role:?}");
        assert_eq!(c.vreg(VregOp::Trial(V130)), status::DENIED, "{role:?}");
        assert!(r.wire.outs().is_empty(), "{role:?}: {:?}", r.wire.calls());
    }
    let admin = Client::open(&r, Role::Admin);
    let s = admin.vreg_status();
    assert_eq!((s.live_vreg, s.stored_sel), (V125, 0));
    assert!(!s.trial_active);
}

/// All the rule's raises, and what it does not ask about, for an admin on a real worker.
#[test]
fn wp_hb2_the_voltage_rule_decides_for_an_admin() {
    for (unit, request, answer) in [
        (V120, VregOp::Set(V125), status::DENIED),
        (V120, VregOp::Trial(V125), status::DENIED),
        (V120, VregOp::Set(V130), status::DENIED),
        (V115, VregOp::Trial(V125), status::DENIED),
        (V125, VregOp::Set(V130), status::DENIED),
        (V115, VregOp::Set(V120), status::OK),
        (V115, VregOp::Trial(V120), status::OK),
        (V120, VregOp::Set(V120), status::OK),
        (V125, VregOp::Trial(V125), status::OK),
        (V125, VregOp::Trial(V120), status::OK),
        (V125, VregOp::Set(0), status::OK),
        (V120, VregOp::TrialEnd, status::OK),
        (V120, VregOp::Auto(false), status::OK),
        (V120, VregOp::Clear, status::OK),
    ] {
        let r = rig_at(unit);
        let c = Client::open(&r, Role::Admin);
        assert_eq!(c.vreg(request), answer, "{unit:#04x} {request:?}");
        let wrote = r.wire.outs().len();
        assert_eq!(
            wrote,
            usize::from(answer == status::OK),
            "{unit:#04x} {request:?}"
        );
    }
}

/// Persisting a trial is a raise, even though the unit runs at that voltage now.
#[test]
fn wp_hb2_persisting_a_running_trial_is_a_raise() {
    let mut inner = FakeDevice::demo();
    inner
        .control_out(
            op::REQ_EXTENSION,
            g::EXT_CORE_VREG,
            &VregOp::Trial(V125).encode(),
        )
        .unwrap();
    let r = rig_with(inner);
    let c = Client::open(&r, Role::Admin);
    assert_eq!(c.vreg(VregOp::Set(V125)), status::DENIED);
    assert_eq!(c.vreg(VregOp::Trial(V125)), status::OK, "no higher");
    assert_eq!(c.vreg(VregOp::TrialEnd), status::OK);
    assert!(!c.vreg_status().trial_active);
}

#[test]
fn wp_hb2_an_unreadable_unit_counts_as_needing_the_word() {
    let r = rig_at(V120);
    let c = Client::open(&r, Role::Admin);
    *r.wire.stall_in.lock().unwrap() = Some(op::REQ_EXTENSION);
    assert_eq!(c.vreg(VregOp::Set(V125)), status::DENIED);
    assert_eq!(c.vreg(VregOp::Trial(V130)), status::DENIED);
    assert_eq!(c.vreg(VregOp::Set(0)), status::DENIED, "no known target");
    assert!(r.wire.outs().is_empty());
    assert_eq!(
        c.vreg(VregOp::Set(V120)),
        status::OK,
        "1.20 V needs no word"
    );
}

/// Payloads the rule cannot read, however the firmware would take them.
#[test]
fn wp_hb2_a_malformed_voltage_write_never_reaches_the_unit() {
    let r = rig_at(V130);
    let c = Client::open(&r, Role::Admin);
    for bad in [&[][..], &[0], &[0, V125, 0], &[9, V125], &[3, 2]] {
        assert_eq!(
            c.set(op::REQ_EXTENSION, g::EXT_CORE_VREG, bad),
            status::DENIED,
            "{bad:?}"
        );
    }
    assert!(r.wire.outs().is_empty());
}

/// The read and the write are one turn of the queue: each write has its own read
/// right before it, and another session's command cannot land between.
#[test]
fn wp_hb2_the_voltage_is_read_at_that_moment_in_the_queue() {
    let r = rig_at(V125);
    let (a, b) = (Client::open(&r, Role::Admin), Client::open(&r, Role::Admin));
    r.wire.hold();
    let volume = request(
        r.handle,
        Dir::Set,
        op::REQ_SET_MASTER_VOLUME,
        0,
        4,
        vec![0; 4],
    );
    let set = |value: u8| {
        request(
            r.handle,
            Dir::Set,
            op::REQ_EXTENSION,
            g::EXT_CORE_VREG,
            2,
            VregOp::Set(value).encode().to_vec(),
        )
    };
    let first = a.submit(volume.clone());
    wait_until("the worker to be inside the device", || {
        r.wire.waiting.load(Ordering::SeqCst) == 1
    });
    let answers = [a.submit(set(V125)), b.submit(volume), a.submit(set(V125))];
    r.wire.release();
    assert_eq!(first.recv_timeout(WAIT).unwrap().status, status::OK);
    for answer in answers {
        assert_eq!(answer.recv_timeout(WAIT).unwrap().status, status::OK);
    }
    let codes: Vec<(Dir, u8)> = r.wire.calls().iter().map(|c| (c.dir, c.code)).collect();
    let (ext, vol) = (op::REQ_EXTENSION, op::REQ_SET_MASTER_VOLUME);
    assert_eq!(
        codes,
        [
            (Dir::Set, vol),
            (Dir::Get, ext),
            (Dir::Set, ext),
            (Dir::Set, vol),
            (Dir::Get, ext),
            (Dir::Set, ext),
        ]
    );
}

/// The app's word is judged on its side, by the same code its core calls, and the hub
/// lets the result through. The same raise from a network admin stays refused.
/// `tests/wp_hb2_app_vreg.rs` has this through a real `DspiCore`.
#[test]
fn wp_hb2_the_app_is_judged_by_its_own_word_and_a_network_admin_stays_refused() {
    let r = rig_at(V120);
    let admin = Client::open(&r, Role::Admin);
    let mut app = r.hub.open_local(&r.serial).unwrap();

    let refused = sx::core_vreg_op(&mut app, VregOp::Set(V125), None);
    assert!(
        matches!(refused, Err(ExtError::WordRequired { .. })),
        "{refused:?}"
    );
    assert!(r.wire.outs().is_empty());

    assert_eq!(admin.vreg(VregOp::Set(V125)), status::DENIED);
    assert!(r.wire.outs().is_empty());

    sx::core_vreg_op(&mut app, VregOp::Set(V125), Some(WORD)).unwrap();
    assert_eq!(r.wire.outs().len(), 1);
    assert_eq!(admin.vreg_status().stored_sel, V125);

    assert_eq!(admin.vreg(VregOp::Set(V130)), status::DENIED);
    assert_eq!(r.wire.outs().len(), 1);
}

/// The rule does not reach another sub-command.
#[test]
fn wp_hb2_another_extension_write_is_left_alone() {
    let r = rig_at(V120);
    let c = Client::open(&r, Role::Admin);
    assert_eq!(
        c.set(
            op::REQ_EXTENSION,
            g::EXT_CROSSFEED_PAIR_MAP,
            &[0x10, 0, 0, 0]
        ),
        status::OK
    );
    assert_eq!(
        r.wire
            .calls()
            .iter()
            .filter(|c| c.dir == Dir::Get && c.code == op::REQ_EXTENSION)
            .count(),
        0,
        "no status read for it"
    );
}

// -- A write is never replayed ----------------------------------------------------

/// Whatever a write comes back with, the device saw it once.
#[test]
fn wp_hb2_a_failed_write_is_never_replayed() {
    for (failure, answer) in [
        (
            TransportError::Stalled {
                opcode: op::REQ_SET_MASTER_VOLUME,
            },
            status::ERROR,
        ),
        (
            TransportError::TimedOut {
                opcode: op::REQ_SET_MASTER_VOLUME,
                retries: 0,
            },
            status::TIMEOUT,
        ),
        (TransportError::Usb("a bus error".into()), status::ERROR),
        (TransportError::Disconnected, status::NO_DEVICE),
    ] {
        let r = rig();
        let c = Client::open(&r, Role::Control);
        // A write has just run, so the busy window is open too.
        assert_eq!(c.set_volume(-10.0), status::OK);
        r.wire.clear();
        *r.wire.fail_out.lock().unwrap() = Some(failure.clone());
        assert_eq!(c.set_volume(-11.0), answer, "{failure:?}");
        // Time for anything that might come back to it.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(r.wire.outs().len(), 1, "{failure:?}");
    }
}

/// A command that writes through a GET is a write. Beside it, the read that may repeat.
#[test]
fn wp_hb2_a_write_as_read_is_not_retried_and_a_parameter_read_is() {
    let r = rig();
    let c = Client::open(&r, Role::Admin);
    assert_eq!(c.set_volume(-10.0), status::OK);

    r.wire.clear();
    *r.wire.stall_in.lock().unwrap() = Some(op::REQ_PRESET_LOAD);
    assert_eq!(c.get(op::REQ_PRESET_LOAD, 0, 1).status, status::ERROR);
    assert_eq!(r.wire.calls().len(), 1, "a preset load runs once");

    assert_eq!(c.set_volume(-10.0), status::OK);
    r.wire.clear();
    *r.wire.stall_in.lock().unwrap() = Some(op::REQ_GET_MASTER_VOLUME);
    assert_eq!(c.get(op::REQ_GET_MASTER_VOLUME, 0, 4).status, status::ERROR);
    assert!(r.wire.calls().len() > 1, "a read inside the window does");
}

/// The app's side of the same promise.
#[test]
fn wp_hb2_the_apps_failed_write_is_not_replayed() {
    let r = rig();
    let mut app = r.hub.open_local(&r.serial).unwrap();
    *r.wire.fail_out.lock().unwrap() = Some(TransportError::TimedOut {
        opcode: op::REQ_SET_MASTER_VOLUME,
        retries: 0,
    });
    assert!(
        app.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4])
            .is_err()
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(r.wire.outs().len(), 1);
}

/// A write that had not run when the device went does not land on the board that comes
/// back at that handle.
#[test]
fn wp_hb2_a_write_queued_when_the_device_left_does_not_land_on_its_return() {
    let r = rig();
    let c = Client::open(&r, Role::Control);
    r.wire.hold();
    let inside = c.submit(request(
        r.handle,
        Dir::Get,
        op::REQ_GET_MASTER_VOLUME,
        0,
        4,
        Vec::new(),
    ));
    wait_until("the worker to be inside the device", || {
        r.wire.waiting.load(Ordering::SeqCst) == 1
    });
    let queued = c.submit(request(
        r.handle,
        Dir::Set,
        op::REQ_SET_MASTER_VOLUME,
        0,
        4,
        vec![0; 4],
    ));
    let hub = r.hub.handle();
    let handle = r.handle;
    let detach = std::thread::spawn(move || hub.detach(handle));
    std::thread::sleep(Duration::from_millis(50));
    r.wire.release();
    detach.join().unwrap().unwrap();
    let _ = inside.recv_timeout(WAIT).unwrap();
    assert_eq!(queued.recv_timeout(WAIT).unwrap().status, status::NO_DEVICE);
    let (again, wire, _remote) = plug(&r.hub, &r.serial, FakeDevice::demo());
    assert_eq!(again, handle);
    std::thread::sleep(Duration::from_millis(50));
    assert!(wire.outs().is_empty(), "{:?}", wire.calls());
    assert!(r.wire.outs().is_empty(), "{:?}", r.wire.calls());
}

// -- Locks ------------------------------------------------------------------------

fn device_changes(c: &Client) -> Vec<Value> {
    c.events()
        .into_iter()
        .filter(|e| e["t"] == "device.changed")
        .map(|e| e["device"]["locked_by"].clone())
        .collect()
}

/// The done criterion. A client takes the lock and is never heard from again: its
/// session stays open, and everybody else gets the device back at the lock's timeout.
#[test]
fn wp_hb2_a_vanished_clients_lock_is_released_by_timeout() {
    let r = rig();
    let (gone, other) = (
        Client::open(&r, Role::Admin),
        Client::open(&r, Role::Control),
    );
    let timeout = Duration::from_millis(300);
    let taken = Instant::now();
    assert!(r.hub.lock_acquire(&gone.session, r.handle, timeout));
    if taken.elapsed() < timeout {
        assert_eq!(other.set_volume(-10.0), status::LOCKED);
    }
    assert!(!r.hub.lock_acquire(&other.session, r.handle, timeout));
    wait_until("the lock to time out", || {
        other.set_volume(-10.0) == status::OK
    });
    assert!(taken.elapsed() >= timeout, "not a moment early");
    assert!(r.hub.lock_acquire(&other.session, r.handle, timeout));
    drop(gone);
}

/// `device.changed` says who holds it, and says so again when it times out, which
/// nobody asks for: the supervisor's tick notices.
#[test]
fn wp_hb2_device_changed_follows_the_lock_even_when_it_times_out() {
    let r = rig();
    let (holder, watcher) = (
        Client::open(&r, Role::Admin),
        Client::open(&r, Role::Viewer),
    );
    assert!(
        r.hub
            .lock_acquire(&holder.session, r.handle, Duration::from_millis(100))
    );
    assert_eq!(device_changes(&watcher), [json!(holder.id())]);
    assert_eq!(
        r.hub.devices()[usize::from(r.handle)].locked_by,
        Some(holder.id())
    );
    wait_until("the registry to hear of the timeout", || {
        r.hub.devices()[usize::from(r.handle)].locked_by.is_none()
    });
    assert_eq!(device_changes(&watcher), [Value::Null]);

    assert!(r.hub.lock_acquire(&holder.session, r.handle, WAIT));
    r.hub.lock_release(&holder.session, r.handle);
    assert_eq!(
        device_changes(&watcher),
        [json!(holder.id()), Value::Null],
        "acquired and released"
    );

    assert!(r.hub.lock_acquire(&holder.session, r.handle, WAIT));
    r.hub.close_session(holder.id());
    assert_eq!(
        device_changes(&watcher),
        [json!(holder.id()), Value::Null],
        "gone with its session"
    );
}

/// The app's lock goes through no message, and still shows.
#[test]
fn wp_hb2_the_apps_lock_shows_too() {
    let r = rig();
    let watcher = Client::open(&r, Role::Viewer);
    let mut app = r.hub.open_local(&r.serial).unwrap();
    app.lock("Saving", Duration::from_secs(5)).unwrap();
    assert_eq!(
        watcher.get(op::REQ_GET_PLATFORM, 0, 8).status,
        status::LOCKED
    );
    wait_until("the tick", || {
        r.hub.devices()[usize::from(r.handle)].locked_by == Some(dspi_hub::LOCAL_SESSION_ID)
    });
    app.unlock();
    wait_until("the tick", || {
        r.hub.devices()[usize::from(r.handle)].locked_by.is_none()
    });
}

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../../tests/fixtures/link/messages/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn answer(c: &Client, msg: &Value) -> Value {
    serde_json::from_str(&lock::handle_message(&c.hub, &c.session, msg).unwrap()).unwrap()
}

/// Spec 7.7's messages, from the spec's own fixtures.
#[test]
fn wp_hb2_lock_messages_answer_as_the_spec_says() {
    let r = rig();
    let (admin, other) = (
        Client::open(&r, Role::Admin),
        Client::open(&r, Role::Control),
    );
    let acquire = fixture("lock_acquire");
    let release = fixture("lock_release");

    assert_eq!(
        answer(&admin, &acquire),
        json!({"t": "ok", "id": 10, "timeout_ms": 10000})
    );
    let refused = answer(&other, &acquire);
    assert_eq!(
        (&refused["t"], &refused["code"]),
        (&json!("err"), &json!("locked"))
    );
    assert_eq!(refused["id"], 10);
    assert_eq!(other.set_volume(-1.0), status::LOCKED);
    assert_eq!(answer(&other, &release), json!({"t": "ok", "id": 11}));
    assert_eq!(other.set_volume(-1.0), status::LOCKED, "not its lock");
    assert_eq!(answer(&admin, &release), json!({"t": "ok", "id": 11}));
    assert_eq!(other.set_volume(-1.0), status::OK);

    let with = |ms: Value| json!({"t": "lock.acquire", "id": 1, "handle": 0, "timeout_ms": ms});
    assert_eq!(
        answer(&admin, &with(json!(3_600_000)))["timeout_ms"],
        60000,
        "capped, and says so"
    );
    assert_eq!(answer(&admin, &release)["t"], "ok");
    for bad in [json!(-1), json!("soon"), json!(-0.5)] {
        assert_eq!(
            answer(&admin, &with(bad.clone()))["code"],
            "bad_request",
            "{bad}"
        );
    }
    assert_eq!(
        answer(&admin, &json!({"t": "lock.acquire", "id": 1}))["code"],
        "bad_request"
    );
    assert_eq!(
        answer(&admin, &json!({"t": "lock.acquire", "handle": 0}))["code"],
        "bad_request"
    );
    assert_eq!(
        answer(&admin, &json!({"t": "lock.acquire", "id": 1, "handle": 99}))["code"],
        "no_device"
    );
    let viewer = Client::open(&r, Role::Viewer);
    assert_eq!(answer(&viewer, &acquire)["code"], "denied");
    assert!(
        lock::handle_message(&admin.hub, &admin.session, &json!({"t": "device.list"})).is_none()
    );
    assert!(lock::handle_message(&admin.hub, &admin.session, &json!({})).is_none());
}

// -- A hostile client -------------------------------------------------------------

/// Every session gets its turn however hard one pushes: past `max_inflight` the flood
/// is turned away without the device, and the other's command runs after what was
/// already queued, not after the flood.
#[test]
fn wp_hb2_a_flood_cannot_starve_another_session() {
    let r = rig();
    let (hostile, victim) = (
        Client::open(&r, Role::Control),
        Client::open(&r, Role::Control),
    );
    r.wire.hold();
    let volume = |h: u8| request(h, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, 4, vec![0; 4]);
    let mut admitted = Vec::new();
    let mut limited = 0;
    for _ in 0..500 {
        let answer = hostile.submit(volume(r.handle));
        match answer.try_recv() {
            Ok(a) => {
                assert_eq!(a.status, status::RATE_LIMITED);
                limited += 1;
            }
            Err(_) => admitted.push(answer),
        }
    }
    assert_eq!(admitted.len(), dspi_hub::hub::MAX_INFLIGHT as usize);
    assert_eq!(limited, 500 - admitted.len());

    let turn = victim.submit(request(
        r.handle,
        Dir::Get,
        op::REQ_GET_PLATFORM,
        0,
        8,
        Vec::new(),
    ));
    // Not even the flood's refusals passed the gate.
    r.wire.release();
    assert_eq!(turn.recv_timeout(WAIT).unwrap().status, status::OK);
    let calls = r.wire.calls();
    let at = calls
        .iter()
        .position(|c| c.code == op::REQ_GET_PLATFORM)
        .unwrap();
    assert_eq!(at, admitted.len(), "behind what was queued, and only that");
    assert_eq!(calls.len(), admitted.len() + 1, "the flood did not run");
}

/// Reconnecting does not buy more of the queue: what a closed session left behind goes
/// without the device, so the queue stays what `max_inflight` says it is.
#[test]
fn wp_hb2_reconnecting_buys_no_more_of_the_queue() {
    let r = rig();
    let victim = Client::open(&r, Role::Control);
    r.wire.hold();
    let volume = request(
        r.handle,
        Dir::Set,
        op::REQ_SET_MASTER_VOLUME,
        0,
        4,
        vec![0; 4],
    );
    let mut answers = Vec::new();
    for _ in 0..50 {
        let hostile = Client::open(&r, Role::Control);
        for _ in 0..dspi_hub::hub::MAX_INFLIGHT {
            answers.push(hostile.submit(volume.clone()));
        }
        r.hub.close_session(hostile.id());
    }
    let turn = victim.submit(volume.clone());
    r.wire.release();
    assert_eq!(turn.recv_timeout(WAIT).unwrap().status, status::OK);
    // One was already inside the device when the first session closed, at most.
    assert!(r.wire.outs().len() <= 2, "{:?}", r.wire.outs());
    for answer in answers {
        answer.recv_timeout(WAIT).unwrap();
    }
    assert_eq!(victim.set_volume(-1.0), status::OK, "the queue is free");
}

/// A session that closes gets its place back at once: its id is not left behind in
/// the count, and a later command of another is not affected.
#[test]
fn wp_hb2_a_closed_sessions_answers_all_come() {
    let r = rig();
    let c = Client::open(&r, Role::Control);
    r.wire.hold();
    let volume = request(
        r.handle,
        Dir::Set,
        op::REQ_SET_MASTER_VOLUME,
        0,
        4,
        vec![0; 4],
    );
    let answers: Vec<_> = (0..4).map(|_| c.submit(volume.clone())).collect();
    r.hub.close_session(c.id());
    r.wire.release();
    for answer in answers {
        answer.recv_timeout(WAIT).unwrap();
    }
}

/// A lock's refusals and the budget, on a running device: a holder that renews without
/// a break cannot keep the others out for longer than the budget. Time comes from the
/// router's clock, so this does not take two minutes.
#[test]
fn wp_hb2_a_lock_cannot_be_held_for_ever() {
    use dspi_hub::lock::{LOCK_BUDGET, LOCK_MAX};
    use dspi_hub::policy::Policy;
    use dspi_hub::router::{ExecTicket, Router};
    use dspi_hub::session::SessionCtx;

    let router = Router::new(Arc::new(Policy::embedded()), 8);
    let victim = SessionCtx {
        id: 3,
        role: Role::Control,
        exempt_inflight: false,
    };
    let volume = request(0, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, 4, vec![0; 4]);
    let t0 = Instant::now();
    let tick = Duration::from_millis(500);
    let total = Duration::from_secs(1800);
    let (mut now, mut free, mut shut_out, mut longest) =
        (t0, Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let mut session = 4u16;
    while now - t0 < total {
        // Renews at every tick. Refused, it comes back as a new session.
        if !router.lock_acquire(session, LOCK_MAX, now) {
            session = session.wrapping_add(1).max(4);
        }
        if router.admit(&victim, &volume, now).is_ok() {
            router.after_exec(&victim, ExecTicket::none(), true);
            free += tick;
            shut_out = Duration::ZERO;
        } else {
            shut_out += tick;
            longest = longest.max(shut_out);
        }
        now += tick;
    }
    assert!(longest <= LOCK_BUDGET, "shut out for {longest:?}");
    let share = free.as_secs_f64() / total.as_secs_f64();
    assert!(share > 0.15, "the others got {:.0} %", share * 100.0);
}

/// Commands inside a lock are the holder's; others' reads wait too (spec 7.7).
#[test]
fn wp_hb2_a_lock_refuses_others_at_once_and_touches_nothing() {
    let r = rig();
    let (holder, other) = (
        Client::open(&r, Role::Admin),
        Client::open(&r, Role::Control),
    );
    assert!(r.hub.lock_acquire(&holder.session, r.handle, WAIT));
    r.wire.clear();
    assert_eq!(other.get(op::REQ_GET_PLATFORM, 0, 8).status, status::LOCKED);
    assert_eq!(other.set_volume(-1.0), status::LOCKED);
    assert!(r.wire.calls().is_empty());
    assert_eq!(holder.set_volume(-1.0), status::OK);
}

#[test]
fn wp_hb2_a_viewer_cannot_lock_anyone_out() {
    let r = rig();
    let (viewer, other) = (
        Client::open(&r, Role::Viewer),
        Client::open(&r, Role::Control),
    );
    assert!(!r.hub.lock_acquire(&viewer.session, r.handle, WAIT));
    assert_eq!(other.set_volume(-1.0), status::OK);
}

/// Too large never reaches the device, and the size is checked before the role.
#[test]
fn wp_hb2_too_large_is_refused_without_the_device() {
    let r = rig();
    let c = Client::open(&r, Role::Admin);
    let at = dspi_hub::router::MAX_PAYLOAD;
    r.wire.clear();
    let over = vec![0; at + 1];
    assert_eq!(
        c.command(Dir::Set, op::REQ_SET_ALL_PARAMS, 0, (at + 1) as u16, over)
            .status,
        status::TOO_LARGE
    );
    assert_eq!(
        c.get(op::REQ_GET_ALL_PARAMS, 0, (at + 1) as u16).status,
        status::TOO_LARGE
    );
    assert!(r.wire.calls().is_empty());
    assert_eq!(
        c.get(op::REQ_GET_ALL_PARAMS, 0, at as u16).status,
        status::OK
    );
}

// -- The analyser, on the wire ----------------------------------------------------

#[test]
fn wp_hb2_an_analyser_stop_reaches_the_device_only_when_nobody_else_watches() {
    let r = rig();
    let (a, b) = (
        Client::open(&r, Role::Control),
        Client::open(&r, Role::Control),
    );
    let stops = || {
        r.wire
            .calls()
            .iter()
            .filter(|c| c.code == op::REQ_RTA_CONTROL && c.value == rta::RTA_CTL_STOP)
            .count()
    };
    let _ = b.get(op::REQ_RTA_GET_BANDS, 0, 82);
    let stop = a.get(op::REQ_RTA_CONTROL, rta::RTA_CTL_STOP, 1);
    assert_eq!((stop.status, stop.payload), (status::OK, vec![1]));
    assert_eq!(stops(), 0, "b is watching");

    r.hub.close_session(b.id());
    assert_eq!(stops(), 0, "nothing is sent on a session's behalf");
    let _ = a.get(op::REQ_RTA_CONTROL, rta::RTA_CTL_STOP, 1);
    assert_eq!(stops(), 1, "with b gone, a's STOP goes through");
}

// -- Notifications carry what a client needs ---------------------------------------

/// A change at the device itself has nobody behind it.
#[test]
fn wp_hb2_a_change_nobody_here_made_has_no_origin() {
    let r = rig();
    let c = Client::open(&r, Role::Viewer);
    let master_volume = 4700;
    assert!(r.remote.poke(
        master_volume,
        &(-3.0f32).to_le_bytes(),
        nfy::PARAM_SRC_GPIO as u8
    ));
    assert_eq!(c.next_notify(), (0, nfy::PARAM_SRC_GPIO as u8));
}
