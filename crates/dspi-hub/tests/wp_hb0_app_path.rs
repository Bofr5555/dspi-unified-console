//! WP HB0's done criterion: the app reaches a demo device through the hub, as a
//! session like any other, with the core's echo rule unchanged. Plus what HUB.md
//! promises for a second client, a stop, an unplug and a slow client.
#![allow(clippy::arithmetic_side_effects)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::consts::{NOTIFY_REFRESHED, NOTIFY_STATE};
use dspi_core::{DeviceEvents, DspiCore, NotifyListener};
use dspi_hub::app::AppLink;
use dspi_hub::session::{CLOSE_SHUTTING_DOWN, Pop};
use dspi_hub::{
    AccessMode, BusDevice, Completion, DeviceBus, DeviceState, Hub, HubConfig, HubError, HubEvent,
    HubListener, LOCAL_SESSION_ID, NetworkConfig, Outbound, Role, Session,
};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError,
};

const VID: u16 = 0x2E8B;
const WAIT: Duration = Duration::from_secs(10);
const GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A demo device with its own serial, which frees its "interface" when dropped,
/// as a USB transport does.
struct Dev {
    inner: FakeDevice,
    descriptor: DeviceDescriptor,
    usb: bool,
    claim: Arc<AtomicBool>,
}

impl Dev {
    fn new(serial: &str) -> (Self, FakeRemote, Arc<AtomicBool>) {
        let inner = FakeDevice::demo();
        let remote = inner.remote();
        let claim = Arc::new(AtomicBool::new(true));
        let dev = Self {
            inner,
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
            },
            usb: false,
            claim: Arc::clone(&claim),
        };
        (dev, remote, claim)
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        self.claim.store(false, Ordering::SeqCst);
    }
}

impl Transport for Dev {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
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
        if self.usb {
            TransportCaps::USB
        } else {
            self.inner.caps()
        }
    }
}

/// A bus a test plugs into, with USB's rule: an interface has one owner.
#[derive(Clone, Default)]
struct TestBus(Arc<Mutex<BusState>>);

#[derive(Default)]
struct BusState {
    present: Vec<String>,
    remotes: Vec<FakeRemote>,
    claim: Arc<AtomicBool>,
    opens: usize,
}

impl TestBus {
    fn plug(&self, serial: &str) {
        self.0.lock().unwrap().present.push(serial.into());
    }
    fn unplug(&self, serial: &str) {
        let mut s = self.0.lock().unwrap();
        s.present.retain(|p| p != serial);
        for r in s.remotes.drain(..) {
            r.disconnect();
        }
    }
    fn claim(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0.lock().unwrap().claim)
    }
    fn opens(&self) -> usize {
        self.0.lock().unwrap().opens
    }
    fn remote(&self) -> FakeRemote {
        self.0.lock().unwrap().remotes.last().unwrap().clone()
    }
}

impl DeviceBus for TestBus {
    fn scan(&mut self) -> Vec<BusDevice> {
        self.0
            .lock()
            .unwrap()
            .present
            .iter()
            .map(|serial| BusDevice {
                serial: serial.clone(),
                vid: VID,
                location_id: 9,
            })
            .collect()
    }
    fn open(&mut self, serial: &str) -> Result<Box<dyn Transport>, TransportError> {
        let mut s = self.0.lock().unwrap();
        if s.claim.swap(true, Ordering::SeqCst) {
            return Err(TransportError::Usb("busy".into()));
        }
        s.opens += 1;
        let inner = FakeDevice::demo();
        s.remotes.push(inner.remote());
        Ok(Box::new(Dev {
            inner,
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
            },
            usb: true,
            claim: Arc::clone(&s.claim),
        }))
    }
}

fn hub_with_bus(bus: &TestBus) -> Hub {
    let mut config = HubConfig::in_process("test");
    config.bus = Some(Box::new(bus.clone()));
    config.scan_interval = Duration::from_millis(10);
    Hub::start(config).unwrap()
}

/// A hub with one attached device, and a core on it through the hub.
fn app_on_demo() -> (Hub, Arc<DspiCore>, AppLink, FakeRemote, String) {
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (dev, remote, _) = Dev::new("AAAA");
    hub.attach(Box::new(dev), VID).unwrap();
    let core = DspiCore::new();
    let link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();
    core.select_device("AAAA".into()).unwrap();
    settle(&core);
    (hub, core, link, remote, "AAAA".into())
}

/// Process notifications until things have been quiet for a moment; the union of
/// the flags seen.
fn settle(core: &DspiCore) -> u32 {
    let mut flags = 0;
    let mut quiet = Instant::now();
    while quiet.elapsed() < Duration::from_millis(150) {
        let r = core.process_notifications();
        if r.flags != 0 {
            flags |= r.flags;
            quiet = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    flags
}

/// A client on the network side, minus the socket: commands in, frames out.
struct Client {
    hub: dspi_hub::HubHandle,
    session: Arc<Session>,
}

impl Client {
    fn open(hub: &Hub, role: Role) -> Self {
        let hub = hub.handle();
        let session = hub.open_session(role, None).unwrap();
        Self { hub, session }
    }

    fn command(&self, handle: u8, dir: Dir, code: u8, len: u16, payload: Vec<u8>) -> CmdResponse {
        let (tx, rx) = mpsc::channel();
        let req = CmdRequest {
            tag: 1,
            handle,
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
            Completion::new(1, move |r| tx.send(r).unwrap()),
        );
        rx.recv_timeout(WAIT).unwrap()
    }

    fn set_volume(&self, handle: u8, db: f32) -> u8 {
        self.command(
            handle,
            Dir::Set,
            op::REQ_SET_MASTER_VOLUME,
            4,
            db.to_le_bytes().to_vec(),
        )
        .status
    }

    /// The next notification's origin, skipping nothing.
    fn next_origin(&self) -> u16 {
        let start = Instant::now();
        loop {
            match self.session.outbox().pop(Duration::from_millis(50)) {
                Pop::Item(Outbound::Notify { origin, .. }) => return origin,
                Pop::Idle if start.elapsed() < WAIT => {}
                other => panic!("no notification: {other:?}"),
            }
        }
    }

    fn drain(&self) -> Vec<Outbound> {
        let mut all = Vec::new();
        while let Pop::Item(item) = self.session.outbox().pop(Duration::from_millis(100)) {
            all.push(item);
        }
        all
    }
}

// -- The done criterion ------------------------------------------------------

#[test]
fn wp_hb0_the_app_drives_a_demo_device_through_the_hub() {
    let (hub, core, link, remote, serial) = app_on_demo();
    let found = core.scan_devices();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].serial, serial);
    assert!(core.is_connected());

    // The mirror came through the hub, whole.
    let state = core.get_state();
    assert!(state.bulk_valid);
    assert_eq!(state.platform_id, 1, "the demo is an RP2350");

    // A setter reaches the device, and a full fetch reads it back.
    core.set_master_volume(-23.5).unwrap();
    let client = Client::open(&hub, Role::Admin);
    let read = client.command(0, Dir::Get, op::REQ_GET_MASTER_VOLUME, 4, Vec::new());
    assert_eq!(read.status, status::OK);
    assert_eq!(read.payload, (-23.5f32).to_le_bytes());
    core.fetch_all().unwrap();
    assert_eq!(core.get_state().master_volume_db, -23.5);

    // A bulk write goes through as well, and the device has what the core sent.
    let before = remote.image();
    core.apply_all_params().unwrap();
    assert_eq!(remote.image().len(), before.len());
    link.stop_hub(&hub);
}

// -- The echo rule -----------------------------------------------------------

#[test]
fn wp_hb0_each_side_sees_the_right_origin() {
    let (hub, core, _link, remote, _) = app_on_demo();
    let client = Client::open(&hub, Role::Admin);

    core.set_master_volume(-23.5).unwrap();
    assert_eq!(client.next_origin(), LOCAL_SESSION_ID, "the app's write");

    assert_eq!(client.set_volume(0, -30.0), status::OK);
    assert_eq!(
        client.next_origin(),
        client.session.id(),
        "the client's own"
    );

    let at = find_volume(&remote, -30.0);
    assert!(remote.poke(at, &(-40.0f32).to_le_bytes(), GPIO));
    assert_eq!(client.next_origin(), 0, "a knob at the device is nobody's");
}

/// The offset of the master volume in the image, by value.
fn find_volume(remote: &FakeRemote, db: f32) -> usize {
    let bytes = db.to_le_bytes();
    remote
        .image()
        .windows(4)
        .rposition(|w| w == bytes)
        .expect("the volume is in the image")
}

#[test]
fn wp_hb0_the_apps_own_write_needs_no_re_read_and_a_clients_shows_up() {
    let (hub, core, _link, _remote, _) = app_on_demo();
    let client = Client::open(&hub, Role::Admin);

    // Our own scalar write: already in the mirror, no re-read.
    core.set_master_volume(-23.5).unwrap();
    let flags = settle(&core);
    assert_eq!(flags & NOTIFY_REFRESHED, 0, "flags {flags:#x}");
    assert_eq!(core.get_state().master_volume_db, -23.5);

    // Another client's: patched into the mirror, and reported.
    assert_eq!(client.set_volume(0, -33.0), status::OK);
    wait_until("the client's write in the mirror", || {
        let flags = core.process_notifications().flags;
        assert_eq!(flags & NOTIFY_REFRESHED, 0, "a scalar needs no re-read");
        core.get_state().master_volume_db == -33.0
    });
}

#[test]
fn wp_hb0_the_apps_own_bulk_write_needs_no_re_read_and_a_clients_forces_one() {
    let (hub, core, _link, remote, _) = app_on_demo();
    let client = Client::open(&hub, Role::Admin);

    core.apply_all_params().unwrap();
    let flags = settle(&core);
    assert_eq!(flags & NOTIFY_REFRESHED, 0, "own bulk echo: {flags:#x}");

    // Same bytes but for a volume the core has not seen; the core must re-read.
    assert_eq!(client.set_volume(0, -21.0), status::OK);
    settle(&core);
    let at = find_volume(&remote, -21.0);
    let mut image = remote.image();
    image[at..at + 4].copy_from_slice(&(-44.0f32).to_le_bytes());
    let len = image.len() as u16;
    let r = client.command(0, Dir::Set, op::REQ_SET_ALL_PARAMS, len, image);
    assert_eq!(r.status, status::OK);
    let flags = settle(&core);
    assert_ne!(flags & NOTIFY_REFRESHED, 0, "foreign bulk: {flags:#x}");
    assert_eq!(core.get_state().master_volume_db, -44.0);
}

#[test]
fn wp_hb0_a_change_at_the_device_reaches_the_app() {
    let (_hub, core, _link, remote, _) = app_on_demo();
    core.set_master_volume(-23.5).unwrap();
    settle(&core);
    let at = find_volume(&remote, -23.5);
    assert!(remote.poke(at, &(-50.0f32).to_le_bytes(), GPIO));
    wait_until("the knob in the mirror", || {
        core.process_notifications();
        core.get_state().master_volume_db == -50.0
    });
}

/// The lock rule: core calls run the hub's workers with the core's lock held, so a
/// listener calling back into the core must not stall anything.
#[test]
fn wp_hb0_a_listener_calling_the_core_during_traffic_does_not_deadlock() {
    struct Calls(Mutex<Option<Arc<DspiCore>>>, AtomicUsize);
    impl NotifyListener for Calls {
        fn on_notifications(&self) {
            self.1.fetch_add(1, Ordering::SeqCst);
            let core = self.0.lock().unwrap().clone();
            if let Some(core) = core {
                core.process_notifications();
                let _ = core.get_state();
            }
        }
    }
    let (hub, core, _link, _remote, _) = app_on_demo();
    let listener = Arc::new(Calls(
        Mutex::new(Some(Arc::clone(&core))),
        AtomicUsize::new(0),
    ));
    core.set_notify_listener(Some(listener.clone()));
    let client = Client::open(&hub, Role::Admin);
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut n = 0u32;
            while !stop.load(Ordering::SeqCst) {
                n += 1;
                let db = -((n % 60) as f32);
                assert_eq!(client.set_volume(0, db), status::OK);
            }
        })
    };
    for n in 0..100 {
        core.set_master_volume(-((n % 60) as f32)).unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    assert!(listener.1.load(Ordering::SeqCst) > 0);
    *listener.0.lock().unwrap() = None;
    core.set_notify_listener(None);
}

// -- Ordering, roles, locks --------------------------------------------------

#[test]
fn wp_hb0_a_lock_holds_across_sessions_the_app_included() {
    let (hub, _core, _link, _remote, serial) = app_on_demo();
    let client = Client::open(&hub, Role::Admin);
    let mut local = hub.open_local(&serial).unwrap();
    local.lock("test", Duration::from_secs(5)).unwrap();
    assert_eq!(client.set_volume(0, -9.0), status::LOCKED);
    assert!(!hub.lock_acquire(&client.session, 0, Duration::from_secs(1)));
    drop(local);
    assert_eq!(client.set_volume(0, -9.0), status::OK, "released with it");

    assert!(hub.lock_acquire(&client.session, 0, Duration::from_secs(5)));
    let mut local = hub.open_local(&serial).unwrap();
    assert!(matches!(
        local.lock("test", Duration::from_secs(1)),
        Err(TransportError::Locked)
    ));
    assert!(matches!(
        local.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4]),
        Err(TransportError::Locked)
    ));
    hub.close_session(client.session.id());
    assert!(
        local
            .control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4])
            .is_ok()
    );
}

#[test]
fn wp_hb0_commands_run_in_arrival_order_across_sessions() {
    let (hub, _core, _link, remote, _) = app_on_demo();
    let (a, b) = (
        Client::open(&hub, Role::Admin),
        Client::open(&hub, Role::Admin),
    );
    let (tx, rx) = mpsc::channel();
    for n in 0..40u32 {
        let client = if n % 2 == 0 { &a } else { &b };
        let tx: Sender<u32> = tx.clone();
        // Paced under max_inflight: at most one of each session is ever queued.
        let (one_tx, one_rx) = mpsc::channel();
        hub.submit(
            &client.session,
            CmdRequest {
                tag: n as u16,
                handle: 0,
                dir: Dir::Set,
                b_request: op::REQ_SET_MASTER_VOLUME,
                w_value: 0,
                w_index: dspi_proto::VENDOR_INTERFACE,
                w_length: 4,
                payload: (-(n as f32)).to_le_bytes().to_vec(),
            },
            Completion::new(n as u16, move |r| {
                assert_eq!(r.status, status::OK);
                tx.send(u32::from(r.tag)).unwrap();
                one_tx.send(()).unwrap();
            }),
        );
        one_rx.recv_timeout(WAIT).unwrap();
    }
    let order: Vec<u32> = rx.try_iter().collect();
    assert_eq!(order, (0..40).collect::<Vec<_>>());
    let _ = find_volume(&remote, -39.0);
}

#[test]
fn wp_hb0_max_inflight_answers_rate_limited_without_the_device() {
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (gate, dev) = gated("AAAA");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let client = Client::open(&hub, Role::Admin);
    let (tx, rx) = mpsc::channel();
    for tag in 0..9u16 {
        let tx = tx.clone();
        hub.submit(
            &client.session,
            volume_request(handle, tag),
            Completion::new(tag, move |r| tx.send(r).unwrap()),
        );
    }
    let refused = rx.recv_timeout(WAIT).unwrap();
    assert_eq!((refused.tag, refused.status), (8, status::RATE_LIMITED));
    gate.open();
    let mut oks = 0;
    for _ in 0..8 {
        assert_eq!(rx.recv_timeout(WAIT).unwrap().status, status::OK);
        oks += 1;
    }
    assert_eq!(oks, 8);
}

#[test]
fn wp_hb0_an_unknown_handle_answers_no_device() {
    let (hub, _core, _link, _remote, _) = app_on_demo();
    let client = Client::open(&hub, Role::Admin);
    assert_eq!(client.set_volume(77, 0.0), status::NO_DEVICE);
}

#[test]
fn wp_hb0_two_devices_are_served_independently() {
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (a, ra, _) = Dev::new("AAAA");
    let (b, rb, _) = Dev::new("BBBB");
    let ha = hub.attach(Box::new(a), VID).unwrap();
    let hb = hub.attach(Box::new(b), VID).unwrap();
    assert_ne!(ha, hb);
    assert!(matches!(
        hub.attach(Box::new(Dev::new("BBBB").0), VID),
        Err(HubError::AlreadyAttached(_))
    ));

    let client = Client::open(&hub, Role::Admin);
    assert_eq!(client.set_volume(hb, -11.0), status::OK);
    assert_eq!(client.set_volume(ha, -22.0), status::OK);
    let _ = (find_volume(&ra, -22.0), find_volume(&rb, -11.0));

    let handles: Vec<u8> = client
        .drain()
        .iter()
        .filter_map(|o| match o {
            Outbound::Notify { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect();
    assert!(
        handles.contains(&ha) && handles.contains(&hb),
        "{handles:?}"
    );

    // The app's transport follows its own device only.
    let core = DspiCore::new();
    let _link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();
    core.select_device("BBBB".into()).unwrap();
    settle(&core);
    let at = find_volume(&rb, -11.0);
    assert_eq!(core.get_state().master_volume_db, -11.0);
    assert!(ra.poke(find_volume(&ra, -22.0), &(-1.0f32).to_le_bytes(), GPIO));
    assert_eq!(settle(&core) & NOTIFY_STATE, 0, "not this app's device");
    assert!(rb.poke(at, &(-2.0f32).to_le_bytes(), GPIO));
    wait_until("this device's change", || {
        core.process_notifications();
        core.get_state().master_volume_db == -2.0
    });
}

// -- Unplug ------------------------------------------------------------------

#[derive(Default)]
struct Departures(Mutex<Vec<String>>);

impl DeviceEvents for Departures {
    fn on_arrived(&self, serial: String) {
        self.0.lock().unwrap().push(format!("+{serial}"));
    }
    fn on_departed(&self, serial: String) {
        self.0.lock().unwrap().push(format!("-{serial}"));
    }
}

#[test]
fn wp_hb0_unplug_and_replug_keep_the_handle_and_reach_the_app() {
    let bus = TestBus::default();
    bus.plug("AAAA");
    let hub = hub_with_bus(&bus);
    let handle = hub.devices()[0].handle;
    assert_eq!(hub.devices()[0].identity.vid, VID);
    let client = Client::open(&hub, Role::Admin);

    let core = DspiCore::new();
    let events = Arc::new(Departures::default());
    core.set_hotplug_callback(Some(events.clone()));
    let _link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();
    core.scan_devices();
    core.select_device("AAAA".into()).unwrap();
    settle(&core);

    // Unplug: the hub keeps the handle, the client gets NO_DEVICE, the app departs.
    bus.unplug("AAAA");
    wait_until("the device offline", || {
        hub.devices()[0].state == DeviceState::Offline
    });
    assert_eq!(hub.devices()[0].handle, handle);
    assert_eq!(client.set_volume(handle, -5.0), status::NO_DEVICE);
    wait_until("the app to notice", || {
        core.poll_hotplug();
        !core.is_connected()
    });
    assert!(events.0.lock().unwrap().contains(&"-AAAA".to_string()));
    assert!(core.scan_devices().is_empty());
    assert!(!client.session.outbox().is_closed(), "the client stays");

    // Replug: same handle, a RESYNC for the client, and the core reconnects itself.
    client.drain();
    bus.plug("AAAA");
    wait_until("the device online", || {
        hub.devices()[0].state == DeviceState::Online
    });
    assert_eq!(hub.devices()[0].handle, handle);
    assert!(
        client
            .drain()
            .contains(&Outbound::Resync { handle, reason: 1 }),
        "a RESYNC for the reattached device"
    );
    wait_until("the app to reconnect", || {
        core.poll_hotplug();
        core.is_connected()
    });
    core.set_master_volume(-7.0).unwrap();
    let _ = find_volume(&bus.remote(), -7.0);
    assert_eq!(client.set_volume(handle, -8.0), status::OK);
    core.set_hotplug_callback(None);
}

#[test]
fn wp_hb0_a_transport_from_before_the_unplug_never_reaches_the_new_board() {
    let bus = TestBus::default();
    bus.plug("AAAA");
    let hub = hub_with_bus(&bus);
    let mut old = hub.open_local("AAAA").unwrap();
    bus.unplug("AAAA");
    wait_until("the device offline", || {
        hub.devices()[0].state == DeviceState::Offline
    });
    bus.plug("AAAA");
    wait_until("the device online", || {
        hub.devices()[0].state == DeviceState::Online
    });
    let write = old.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-3.0f32).to_le_bytes());
    assert!(matches!(write, Err(TransportError::Disconnected)));
    let mut source = old.notifications().unwrap();
    assert!(matches!(
        source.read_attributed(WAIT),
        Err(TransportError::Disconnected)
    ));
    let mut new = hub.open_local("AAAA").unwrap();
    assert!(
        new.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-3.0f32).to_le_bytes())
            .is_ok()
    );
}

#[test]
fn wp_hb0_an_offline_device_is_forgotten_after_its_grace() {
    let bus = TestBus::default();
    bus.plug("AAAA");
    let mut config = HubConfig::in_process("test");
    config.bus = Some(Box::new(bus.clone()));
    config.scan_interval = Duration::from_millis(10);
    config.offline_grace = Duration::from_millis(50);
    let hub = Hub::start(config).unwrap();
    bus.unplug("AAAA");
    wait_until("the device forgotten", || hub.devices().is_empty());
}

/// A transport with no notification stream: a lost device shows only at a command.
#[test]
fn wp_hb0_a_lost_device_is_noticed_at_a_command_too() {
    struct Silent(Dev);
    impl Transport for Silent {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.0.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.0.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
            self.0.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
    }
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (dev, remote, claim) = Dev::new("AAAA");
    let handle = hub.attach(Box::new(Silent(dev)), 0).unwrap();
    let client = Client::open(&hub, Role::Admin);
    remote.disconnect();
    assert_eq!(client.set_volume(handle, -3.0), status::NO_DEVICE);
    wait_until("the transport dropped", || !claim.load(Ordering::SeqCst));
    assert_eq!(hub.devices()[0].state, DeviceState::Offline);
}

// -- A slow client -----------------------------------------------------------

#[test]
fn wp_hb0_a_slow_client_is_slow_alone() {
    let (hub, core, _link, remote, _) = app_on_demo();
    let slow = Client::open(&hub, Role::Viewer);
    let quick = Client::open(&hub, Role::Viewer);
    core.set_master_volume(-23.5).unwrap();
    settle(&core);
    let at = find_volume(&remote, -23.5);

    // Far more than an outbox holds, with one client not reading at all.
    let total = 3 * dspi_hub::session::NOTIFY_CAPACITY;
    let reader = std::thread::spawn(move || {
        let mut got = 0;
        while got < total {
            quick.next_origin();
            got += 1;
        }
    });
    for n in 0..total {
        let db = -1.0 - (n % 100) as f32;
        while !remote.poke(at, &db.to_le_bytes(), GPIO) {
            std::thread::yield_now();
        }
        // The fake's own queue holds 1024; the hub's reader keeps up with this.
        if n % 256 == 255 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    reader.join().unwrap();

    // The app was not held up either: it has the last value.
    let last = -1.0 - ((total - 1) % 100) as f32;
    wait_until("the app's mirror", || {
        core.process_notifications();
        core.get_state().master_volume_db == last
    });

    let frames = slow.drain();
    let notifies = frames
        .iter()
        .filter(|f| matches!(f, Outbound::Notify { .. }))
        .count();
    assert_eq!(notifies, dspi_hub::session::NOTIFY_CAPACITY, "the newest");
    assert_eq!(
        frames.last(),
        Some(&Outbound::Resync {
            handle: 0,
            reason: 0
        }),
        "told to re-read, after what is left"
    );
    let resyncs = frames.len() - notifies;
    assert_eq!(resyncs, 1);
}

// -- Stop --------------------------------------------------------------------

/// A device whose writes wait for the test.
struct Gate(Arc<(Mutex<bool>, std::sync::Condvar)>, Arc<AtomicBool>);

impl Gate {
    fn open(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}

struct Gated(Dev, Arc<(Mutex<bool>, std::sync::Condvar)>, Arc<AtomicBool>);

impl Transport for Gated {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.0.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.0.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.2.store(true, Ordering::SeqCst);
        let (open, wake) = &*self.1;
        let mut g = open.lock().unwrap();
        while !*g {
            g = wake.wait(g).unwrap();
        }
        drop(g);
        self.0.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.0.descriptor()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.0.notifications()
    }
}

fn gated(serial: &str) -> (Gate, Gated) {
    let shared = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let (dev, _, _) = Dev::new(serial);
    let entered = Arc::new(AtomicBool::new(false));
    (
        Gate(Arc::clone(&shared), Arc::clone(&entered)),
        Gated(dev, shared, entered),
    )
}

fn volume_request(handle: u8, tag: u16) -> CmdRequest {
    CmdRequest {
        tag,
        handle,
        dir: Dir::Set,
        b_request: op::REQ_SET_MASTER_VOLUME,
        w_value: 0,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: 4,
        payload: vec![0; 4],
    }
}

#[test]
fn wp_hb0_stop_answers_every_queued_command_and_frees_the_transport() {
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (gate, dev) = gated("AAAA");
    let claim = Arc::clone(&dev.0.claim);
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let client = Client::open(&hub, Role::Admin);
    let (tx, rx): (Sender<CmdResponse>, Receiver<CmdResponse>) = mpsc::channel();
    for tag in 0..4 {
        let tx = tx.clone();
        hub.submit(
            &client.session,
            volume_request(handle, tag),
            Completion::new(tag, move |r| tx.send(r).unwrap()),
        );
    }
    // One command is inside the device: stop waits for it, and only it.
    wait_until("a command in the device", || gate.1.load(Ordering::SeqCst));
    let stopper = std::thread::spawn(move || {
        hub.stop();
        hub
    });
    std::thread::sleep(Duration::from_millis(50));
    assert!(claim.load(Ordering::SeqCst), "not dropped under a transfer");
    gate.open();
    let hub = stopper.join().unwrap();
    assert!(!claim.load(Ordering::SeqCst), "free when stop returns");

    let answers: Vec<u8> = rx.try_iter().map(|r| r.status).collect();
    assert_eq!(answers.len(), 4, "each answered once: {answers:?}");
    assert_eq!(answers[0], status::OK, "the one in the device");
    assert!(answers[1..].iter().all(|s| *s == status::NO_DEVICE));

    // And afterwards:
    assert_eq!(client.set_volume(handle, 0.0), status::NO_DEVICE);
    assert!(hub.is_stopped());
    assert!(matches!(
        hub.open_session(Role::Admin, None),
        Err(HubError::Stopped)
    ));
    assert!(matches!(hub.open_local("AAAA"), Err(HubError::Stopped)));
    hub.stop();
}

#[test]
fn wp_hb0_stop_closes_clients_with_4003_and_ends_the_apps_stream() {
    let (hub, core, link, _remote, _) = app_on_demo();
    let client = Client::open(&hub, Role::Viewer);
    hub.stop();
    assert!(client.drain().iter().any(|f| matches!(
        f,
        Outbound::Close {
            code: CLOSE_SHUTTING_DOWN,
            ..
        }
    )));
    assert_eq!(client.session.outbox().pop(Duration::ZERO), Pop::Closed);

    // Stopped behind the app's back: the app sees a lost device, nothing hangs.
    wait_until("the app to notice", || {
        core.poll_hotplug();
        !core.is_connected()
    });
    assert!(core.scan_devices().is_empty());
    assert!(matches!(
        core.select_device("AAAA".into()),
        Err(dspi_core::DspiError::Disconnected)
    ));
    drop(link);
}

#[test]
fn wp_hb0_a_core_call_during_stop_fails_and_does_not_hang() {
    let (hub, core, _link, _remote, _) = app_on_demo();
    let hammer = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || {
            let mut failed = false;
            for n in 0..2000 {
                match core.set_master_volume(-((n % 60) as f32)) {
                    Ok(()) => {}
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            failed
        })
    };
    std::thread::sleep(Duration::from_millis(5));
    hub.stop();
    assert!(
        hammer.join().unwrap(),
        "the writes fail once the hub is gone"
    );
}

/// The last line of defence: a transport that never answers cannot hold the app
/// (and the core's lock) past the deadline, and its timeout is final for the core.
#[test]
fn wp_hb0_a_wedged_device_times_out_for_the_app() {
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (gate, dev) = gated("AAAA");
    hub.attach(Box::new(dev), 0).unwrap();
    let mut local = hub
        .open_local("AAAA")
        .unwrap()
        .with_answer_timeout(Duration::from_millis(50));
    let write = local.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4]);
    assert!(matches!(
        write,
        Err(TransportError::TimedOut { retries, .. }) if retries > 0
    ));
    gate.open();
    hub.stop();
}

/// Handover both ways, with USB's rule that an interface has one owner: the hub
/// cannot claim what the core holds, nor the core what the hub holds.
#[test]
fn wp_hb0_the_app_hands_usb_to_the_hub_and_gets_it_back() {
    let bus = TestBus::default();
    bus.plug("AAAA");
    let claim = bus.claim();

    // The core holds the device directly.
    let direct = {
        let (mut dev, _, _) = Dev::new("AAAA");
        dev.usb = true;
        claim.store(true, Ordering::SeqCst);
        dev.claim = Arc::clone(&claim);
        dev
    };
    let core = DspiCore::new();
    core.open_transport(Box::new(direct), "AAAA".into(), VID)
        .unwrap();

    // The hub starts and finds it taken.
    let hub = hub_with_bus(&bus);
    assert!(hub.devices().is_empty());
    assert_eq!(hub.inaccessible(), 1);
    assert_eq!(bus.opens(), 0);

    // The app steps aside, the hub claims, the core is back on it, through the hub.
    let link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();
    assert_eq!(bus.opens(), 1);
    assert_eq!(hub.inaccessible(), 0);
    assert!(core.is_connected(), "reopened through the hub");
    let client = Client::open(&hub, Role::Viewer);
    core.set_master_volume(-23.5).unwrap();
    assert_eq!(client.next_origin(), LOCAL_SESSION_ID, "as session 1");

    // One app per hub.
    assert!(matches!(
        AppLink::attach(&hub.handle(), DspiCore::new()),
        Err(HubError::AppAttached)
    ));

    // Back: the interface is free before the core tries to claim it.
    link.stop_hub(&hub);
    assert!(!claim.load(Ordering::SeqCst));
    assert!(hub.is_stopped());
    assert!(!core.is_connected(), "no real USB here to reopen");
    // The core is on its own again: no broker, so a demo opens as ever.
    core.open_demo().unwrap();
    assert!(core.is_connected());
}

#[test]
fn wp_hb0_a_dropped_link_leaves_the_hub_running_for_a_new_app() {
    let (hub, core, link, _remote, serial) = app_on_demo();
    drop(link);
    assert!(!core.is_connected());
    assert!(!hub.is_stopped());
    let core2 = DspiCore::new();
    let _link = AppLink::attach(&hub.handle(), Arc::clone(&core2)).unwrap();
    core2.select_device(serial).unwrap();
    assert!(core2.is_connected());
}

// -- Events, defaults --------------------------------------------------------

#[test]
fn wp_hb0_events_reach_the_listener_on_its_own_thread() {
    struct Ear(Mutex<Vec<HubEvent>>, Mutex<Option<dspi_hub::HubHandle>>);
    impl HubListener for Ear {
        fn on_event(&self, event: HubEvent) {
            assert_eq!(std::thread::current().name(), Some("hub-events"));
            // May read the hub from here.
            if let Some(hub) = self.1.lock().unwrap().as_ref() {
                let _ = hub.devices();
            }
            self.0.lock().unwrap().push(event);
        }
    }
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let ear = Arc::new(Ear(Mutex::new(Vec::new()), Mutex::new(Some(hub.handle()))));
    hub.set_listener(Some(ear.clone()));
    let (dev, _, _) = Dev::new("AAAA");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let session = hub.open_session(Role::Viewer, None).unwrap();
    hub.close_session(session.id());
    hub.detach(handle).unwrap();
    assert_eq!(hub.detach(handle), Err(HubError::NoDevice));
    hub.stop();
    let serial = "AAAA".to_string();
    assert_eq!(
        *ear.0.lock().unwrap(),
        [
            HubEvent::DeviceOnline {
                handle,
                serial: serial.clone()
            },
            HubEvent::SessionOpened { id: session.id() },
            HubEvent::SessionClosed { id: session.id() },
            HubEvent::DeviceOffline { handle, serial },
            HubEvent::Stopped,
        ]
    );
}

/// A listener that panics or calls `stop` costs that call, not the hub.
#[test]
fn wp_hb0_a_bad_listener_does_not_take_the_hub_down() {
    struct Bad(Arc<Hub>, Sender<()>);
    impl HubListener for Bad {
        fn on_event(&self, _: HubEvent) {
            self.0.stop();
            self.1.send(()).unwrap();
            panic!("a listener's own panic");
        }
    }
    let hub = Arc::new(Hub::start(HubConfig::in_process("test")).unwrap());
    let (tx, rx) = mpsc::channel();
    hub.set_listener(Some(Arc::new(Bad(Arc::clone(&hub), tx))));
    let (dev, _, _) = Dev::new("AAAA");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    rx.recv_timeout(WAIT).unwrap();
    assert!(!hub.is_stopped(), "stop from the listener was refused");
    let client = Client::open(&hub, Role::Admin);
    let platform = client.command(handle, Dir::Get, op::REQ_GET_PLATFORM, 7, Vec::new());
    assert_eq!(platform.status, status::OK);
    hub.set_listener(None);
    hub.stop();
}

#[test]
fn wp_hb0_pin_pairing_is_the_default_and_open_access_is_opt_in() {
    assert_eq!(AccessMode::default(), AccessMode::Pin);
    let config = NetworkConfig::new("unused".into());
    assert_eq!(config.access, AccessMode::Pin);
    assert_eq!(config.port, dspi_link::DEFAULT_PORT);
    assert!(!config.web, "the web UI is on only if asked for");
}
