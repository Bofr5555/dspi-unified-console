//! The hub: lifecycle, the supervisor thread, attach and detach, sessions.
//!
//! A [`Hub`] exists exactly while sharing is on. [`Hub::start`] claims the devices,
//! [`Hub::stop`] gives them back, and the USB interfaces are free when it returns.
//! HUB.md is the design; section 3 has the lock order this file keeps: `state` (L1)
//! is never held across a transfer, a join, or a listener call.

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use dspi_link::discovery::{AdvertiseScope, AdvertisedNames, HubAdvert};
use dspi_link::frames::CmdRequest;
use dspi_link::messages::{PollGrant, PollSpec, Snapshot};
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{Transport, TransportError};

use crate::advertise::Advertiser;
use crate::device::{self, Attachment, DeviceShared, SupMsg};
use crate::error::HubError;
use crate::local::LocalTransport;
use crate::lock::LockRefusal;
use crate::policy::Policy;
use crate::polls::PollRefused;
use crate::registry::{
    DeviceState, NameStore, OFFLINE_GRACE, RegisteredDevice, Registry, RegistryEvent,
    board_name_from, board_name_payload,
};
use crate::relay::{Relay, Scope};
use crate::router::Router;
use crate::server::{NetworkConfig, Server};
use crate::session::{
    CLOSE_SHUTTING_DOWN, Completion, FIRST_REMOTE_SESSION_ID, INTERNAL_SESSION_ID,
    LOCAL_SESSION_ID, NO_ORIGIN, Outbox, Role, Session, SessionCtx, SessionId,
};
use crate::snapshot::SnapshotRefused;
use crate::usb::DeviceBus;

/// `limits.max_inflight` (spec 7.2).
pub const MAX_INFLIGHT: u32 = 8;
/// How often the supervisor scans the bus and reaps.
const SCAN_INTERVAL: Duration = Duration::from_millis(500);
/// The wait after a device on the bus could not be opened or did not answer.
const REOPEN_BACKOFF: Duration = Duration::from_secs(2);
/// Pauses before each identity read, the first immediate. Short, because it blocks
/// the supervisor: a board still booting is picked up by a later scan.
const IDENTITY_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(200),
    Duration::from_millis(800),
];
/// How long [`HubHandle::scan_now`] waits for the supervisor.
const SCAN_NOW_TIMEOUT: Duration = Duration::from_secs(10);
// -- WP HB3 --
/// How long `device.snapshot` waits for the image. Well inside a client's own 15 s.
pub const SNAPSHOT_WAIT: Duration = Duration::from_secs(3);

/// What a hub is started with.
pub struct HubConfig {
    /// `hid` (spec 7.2), a lowercase UUID. Empty on a networked hub: the one in its store.
    pub hub_id: String,
    /// The hub's user-visible name.
    pub name: String,
    /// The network side. `None` is an in-process hub: the app and nothing else.
    pub network: Option<NetworkConfig>,
    /// Where devices come from, if not only from [`HubHandle::attach`].
    pub bus: Option<Box<dyn DeviceBus>>,
    /// How long a device that dropped keeps its handle.
    pub offline_grace: Duration,
    /// How often the bus is scanned.
    pub scan_interval: Duration,
    /// Scan the bus at start. An app that holds a device directly clears this: nothing
    /// is scanned or claimed until [`crate::app::AppLink::attach`] has closed that
    /// connection and called [`HubHandle::scan_now`], so the hub never depends on the OS
    /// refusing a second claim. A headless hub keeps it.
    pub initial_scan: bool,
    // -- WP HB3 --
    /// Where hub-stored device names persist. `None`: for this run only.
    pub names: Option<Arc<dyn NameStore>>,
    /// Read each device's image as it attaches, so the first `device.snapshot` needs no
    /// read. Always on with [`HubConfig::network`]; an in-process hub has nobody to ask.
    pub warm_snapshots: bool,
}

impl HubConfig {
    /// A hub for the app alone, with no socket and no bus: attach what it serves.
    pub fn in_process(name: &str) -> Self {
        Self {
            hub_id: String::new(),
            name: name.to_owned(),
            network: None,
            bus: None,
            offline_grace: OFFLINE_GRACE,
            scan_interval: SCAN_INTERVAL,
            initial_scan: true,
            // -- WP HB3 --
            names: None,
            warm_snapshots: false,
        }
    }
}

/// Something the embedder may want to show. Delivered on the hub's event thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEvent {
    /// A device came online.
    DeviceOnline {
        /// Its handle.
        handle: u8,
        /// Its serial.
        serial: String,
    },
    /// A device went offline; it keeps its handle for the grace period.
    DeviceOffline {
        /// Its handle.
        handle: u8,
        /// Its serial.
        serial: String,
    },
    /// A device's grace ended.
    DeviceRemoved {
        /// Its handle.
        handle: u8,
        /// Its serial.
        serial: String,
    },
    /// A network session opened.
    SessionOpened {
        /// Its id.
        id: SessionId,
    },
    /// A network session closed.
    SessionClosed {
        /// Its id.
        id: SessionId,
    },
    /// The hub has stopped. The last event.
    Stopped,
}

/// The embedder's ear on the hub (C ABI: WP HB6).
pub trait HubListener: Send + Sync {
    /// Called on the hub's event thread, with no hub lock held, one event at a time.
    /// May call the hub's read methods. Must not block, and must not call
    /// [`Hub::stop`] (refused from this thread, since stop joins it).
    fn on_event(&self, event: HubEvent);
}

enum EventMsg {
    Event(HubEvent),
    Stop,
}

struct State {
    registry: Registry,
    /// The live attachments, by handle.
    attached: BTreeMap<u8, Attachment>,
    /// Network sessions and the local one, by id.
    sessions: BTreeMap<SessionId, Arc<Session>>,
    next_session: SessionId,
    next_generation: u64,
}

/// What every thread and handle shares.
pub(crate) struct HubInner {
    name: String,
    hub_id: String,
    // -- fixup sw2x -- read by the system model's per-op role check.
    pub(crate) policy: Arc<Policy>,
    state: Mutex<State>,
    pub(crate) relay: Arc<Relay>,
    /// Set under `state`, so an attach or an open either finished before it or fails.
    stopping: AtomicBool,
    sup: Sender<SupMsg>,
    events: Sender<EventMsg>,
    event_thread: OnceLock<ThreadId>,
    listener: Mutex<Option<Arc<dyn HubListener>>>,
    /// The app's session.
    local: Arc<Session>,
    /// Devices on the bus that could not be opened at the last scan.
    inaccessible: AtomicU32,
    /// Whether an `AppLink` holds the local session.
    #[cfg(feature = "app")]
    pub(crate) app_attached: AtomicBool,
    // -- WP HB3 --
    names: Option<Arc<dyn NameStore>>,
    /// Keeps saves in the order of the changes. Above L1: L1 is taken under it, briefly,
    /// to copy the names, and it is never taken under L1.
    names_saving: Mutex<()>,
    warm_snapshots: bool,
    // -- WP SW2 --
    /// The audio link's system model. Its lock is a leaf.
    pub(crate) system: Arc<crate::system::SystemShared>,
    // -- WP HB5 --
    /// The responder while sharing. Above L1: L1 is taken under it, so that the last
    /// update has the latest devices, and it is never taken under L1. Not held across
    /// the goodbye in [`Hub::stop`].
    advertiser: Mutex<Option<Advertiser>>,
    // -- WP HUBFLAKE4 --
    /// Held across a whole [`HubInner::sync_locks`], so lock changes reach the registry
    /// and network sessions in order, and a sync returns only once every change it found
    /// in the registry has been sent. Above L1, as `names_saving` is: L1, L2 and L3 are
    /// taken under it, and it is never taken under any of them.
    locks_syncing: Mutex<()>,
    // -- WP P1HA --
    /// Guarded applies (`apply_guard.rs`). A leaf lock.
    pub(crate) apply_guards: crate::apply_guard::ApplyGuards,
}

/// A cheap, clonable way to the hub, for the server and the app's broker. It keeps
/// memory alive, not the hub: after [`Hub::stop`] its calls fail with
/// [`HubError::Stopped`] or answer `NO_DEVICE`.
#[derive(Clone)]
pub struct HubHandle {
    pub(crate) inner: Arc<HubInner>,
}

struct Threads {
    supervisor: JoinHandle<()>,
    events: JoinHandle<()>,
    server: Option<Server>,
    // -- WP AUSB2 --
    /// The hub's own poll of the system model ([`system_poll_loop`]): its stop
    /// and its thread. `None` if it could not start.
    system_poll: Option<(Sender<()>, JoinHandle<()>)>,
}

/// A running hub. Stops when dropped.
pub struct Hub {
    handle: HubHandle,
    threads: Mutex<Option<Threads>>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data consistent before it can
    // panic, and a hub must go on serving after one bad call.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hub {
    /// Start a hub: the threads, a first bus scan (so its devices are there when this
    /// returns), then the network side.
    ///
    /// Blocks for the scan: off the UI thread. A device another program or the app
    /// still holds is simply not attached yet. [`HubError::Io`] when a thread or the
    /// port cannot be had; nothing is left running then.
    pub fn start(mut config: HubConfig) -> Result<Self, HubError> {
        let network = config.network.take();
        let bus = config.bus.take();
        // -- WP HB1 -- One owner of the store, since clients and device names share a
        // file. A networked hub given no id has the one in it, and keeps its names there
        // unless given another place.
        let auth = match &network {
            Some(network) => {
                let store = crate::store::HubStore::load(&network.store_dir)?;
                let auth = Arc::new(crate::auth::AuthStore::new(store, network.access)?);
                if config.hub_id.is_empty() {
                    config.hub_id = auth.hub_id();
                }
                if config.names.is_none() {
                    config.names = Some(Arc::new(crate::auth::StoredNames(Arc::clone(&auth))));
                }
                Some(auth)
            }
            None => None,
        };
        // -- WP HB3 --
        let names = config.names.take();
        let stored = names.as_ref().map(|n| n.load()).unwrap_or_default();
        // -- WP SW2 -- the model persists where the clients and names do.
        let system_store = auth.as_ref().map(|a| {
            Arc::new(crate::auth::StoredSystem(Arc::clone(a)))
                as Arc<dyn crate::system::SystemStore>
        });
        let system = Arc::new(crate::system::SystemShared::new(system_store));
        let relay = Arc::new(Relay::default());
        relay.set_tap(Arc::clone(&system));
        let (sup_tx, sup_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let inner = Arc::new(HubInner {
            name: config.name,
            hub_id: config.hub_id,
            policy: Arc::new(Policy::embedded()),
            state: Mutex::new(State {
                registry: Registry::with_names(config.offline_grace, stored),
                attached: BTreeMap::new(),
                sessions: BTreeMap::new(),
                next_session: FIRST_REMOTE_SESSION_ID,
                next_generation: 1,
            }),
            relay,
            stopping: AtomicBool::new(false),
            sup: sup_tx,
            events: ev_tx,
            event_thread: OnceLock::new(),
            listener: Mutex::new(None),
            local: Session::new(LOCAL_SESSION_ID, Role::Admin, None, true),
            inaccessible: AtomicU32::new(0),
            #[cfg(feature = "app")]
            app_attached: AtomicBool::new(false),
            // -- WP HB3 --
            names,
            names_saving: Mutex::new(()),
            warm_snapshots: config.warm_snapshots || network.is_some(),
            // -- WP SW2 --
            system,
            // -- WP HB5 --
            advertiser: Mutex::new(None),
            // -- WP HUBFLAKE4 --
            locks_syncing: Mutex::new(()),
            // -- WP P1HA --
            apply_guards: crate::apply_guard::ApplyGuards::default(),
        });
        let io = |e: std::io::Error| HubError::Io(format!("cannot start a hub thread: {e}"));
        let events = {
            let inner = Arc::clone(&inner);
            thread::Builder::new()
                .name("hub-events".into())
                .spawn(move || event_loop(&inner, &ev_rx))
                .map_err(io)?
        };
        let (scanned_tx, scanned_rx) = mpsc::channel();
        if config.initial_scan {
            let _ = inner.sup.send(SupMsg::ScanNow(scanned_tx));
        }
        let supervisor = {
            let inner = Arc::clone(&inner);
            let (interval, initial_scan) = (config.scan_interval, config.initial_scan);
            thread::Builder::new()
                .name("hub-supervisor".into())
                .spawn(move || supervisor_loop(&inner, &sup_rx, bus, interval, initial_scan))
        };
        let supervisor = match supervisor {
            Ok(s) => s,
            Err(e) => {
                let _ = inner.events.send(EventMsg::Stop);
                let _ = events.join();
                return Err(io(e));
            }
        };
        if config.initial_scan {
            let _ = scanned_rx.recv_timeout(SCAN_NOW_TIMEOUT);
        }
        // -- WP AUSB2 -- the system model's own poll (USB-direct units)
        let system_poll = {
            let handle = HubHandle {
                inner: Arc::clone(&inner),
            };
            let (stop_tx, stop_rx) = mpsc::channel();
            match thread::Builder::new()
                .name("hub-system".into())
                .spawn(move || system_poll_loop(&handle, &stop_rx))
            {
                Ok(t) => {
                    // -- WP P1HA -- the guarded applies' timer runs on this thread
                    inner.apply_guards.set_wake(Some(stop_tx.clone()));
                    Some((stop_tx, t))
                }
                Err(e) => {
                    log::warn!(
                        "hub: the system poll did not start ({e}): USB-direct units are read only when a client asks"
                    );
                    None
                }
            }
        };
        let hub = Self {
            handle: HubHandle { inner },
            threads: Mutex::new(Some(Threads {
                supervisor,
                events,
                server: None,
                system_poll,
            })),
        };
        if let (Some(mut network), Some(auth)) = (network, auth) {
            // -- WP HB5 -- Before the server starts, which copies it for its `Origin` check.
            let scope = network.advertise_scope();
            if scope.is_some() && network.mdns_name.is_none() {
                network.mdns_name = dspi_link::discovery::mdns_host_name(hub.hub_id());
            }
            // `hub` is dropped on failure, which stops what started above.
            let server = Server::start(hub.handle.clone(), &network, auth)?;
            let port = server.port();
            if let Some(t) = guard(&hub.threads).as_mut() {
                t.server = Some(server);
            }
            // -- WP HB5 --
            if let Some(scope) = scope {
                hub.handle.inner.start_advertising(&network, port, scope);
            }
        }
        Ok(hub)
    }

    /// A handle for the server, the app's broker or a binding.
    pub fn handle(&self) -> HubHandle {
        self.handle.clone()
    }

    /// Stop. When this returns every session is closed (network clients with 4003),
    /// every queued command has been answered `NO_DEVICE`, every thread is joined and
    /// every transport dropped, so the USB interfaces are free. Idempotent. Takes as
    /// long as the slowest transfer in flight, bounded by the transports' timeouts,
    /// plus up to a second per peer that will not take its close frame: off the UI
    /// thread. Refused, with a log line, from inside a [`HubListener`].
    pub fn stop(&self) {
        let inner = &self.handle.inner;
        if inner.event_thread.get() == Some(&thread::current().id()) {
            log::error!("Hub::stop called from a hub listener: refused");
            return;
        }
        // Held across the whole stop: a second caller returns only once it is done.
        let mut threads = guard(&self.threads);
        let Some(t) = threads.take() else {
            return;
        };
        {
            let _state = guard(&inner.state);
            inner.stopping.store(true, Ordering::SeqCst);
        }
        // -- WP AUSB2 -- the system poll first: it writes as the hub's own session
        if let Some((stop, thread)) = t.system_poll {
            let _ = stop.send(());
            let _ = thread.join();
        }
        // -- WP P1HA -- guarded applies revert while their devices are still here
        inner.apply_guards.set_wake(None);
        self.handle.apply_stop();
        // 1. Off the network: goodbye, no new connections. 2. Everyone gets their close.
        // 3. Connections go.
        let goodbye = guard(&inner.advertiser).take();
        drop(goodbye);
        if let Some(server) = &t.server {
            server.stop_accepting();
        }
        inner
            .relay
            .close_all(CLOSE_SHUTTING_DOWN, "hub shutting down");
        if let Some(server) = t.server {
            server.join();
        }
        // 4. Nothing attaches or detaches behind this thread's back any more.
        let _ = inner.sup.send(SupMsg::Stop);
        let _ = t.supervisor.join();
        // 5. The devices, with no lock held: reader, worker, transport.
        let attached: Vec<Attachment> = {
            let mut st = guard(&inner.state);
            let now = Instant::now();
            let all = std::mem::take(&mut st.attached);
            for a in all.values() {
                st.registry.offline(&a.shared.serial(), now);
            }
            all.into_values().collect()
        };
        for a in attached {
            a.tear_down();
        }
        // 6. The last event, then its thread.
        inner.emit(HubEvent::Stopped);
        let _ = inner.events.send(EventMsg::Stop);
        let _ = t.events.join();
    }
}

// -- WP HB1 --
impl Hub {
    /// The network side, for a binding's pairing window and client list. `None` for an
    /// in-process hub, and once stopped.
    pub fn network(&self) -> Option<crate::server::NetworkControl> {
        guard(&self.threads)
            .as_ref()
            .and_then(|t| t.server.as_ref())
            .map(Server::control)
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.stop();
    }
}

impl std::ops::Deref for Hub {
    type Target = HubHandle;

    fn deref(&self) -> &HubHandle {
        &self.handle
    }
}

impl HubHandle {
    /// The hub's user-visible name.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// `hid`.
    pub fn hub_id(&self) -> &str {
        &self.inner.hub_id
    }

    /// Whether the hub has stopped or is stopping.
    pub fn is_stopped(&self) -> bool {
        self.inner.stopping.load(Ordering::SeqCst)
    }

    /// Register (`Some`) or clear (`None`) the listener.
    pub fn set_listener(&self, listener: Option<Arc<dyn HubListener>>) {
        *guard(&self.inner.listener) = listener;
    }

    /// Every device the hub knows, online or inside its grace, by handle.
    pub fn devices(&self) -> Vec<RegisteredDevice> {
        guard(&self.inner.state).registry.devices()
    }

    // -- WP HB5 --
    /// The names the hub is advertised under, which a conflict on the network can have
    /// changed from what was asked. `None` when it is not advertised.
    pub fn advertised(&self) -> Option<AdvertisedNames> {
        guard(&self.inner.advertiser)
            .as_mut()
            .map(Advertiser::names)
    }

    /// Devices on the bus the hub could not open at its last scan.
    pub fn inaccessible(&self) -> u32 {
        self.inner.inaccessible.load(Ordering::Relaxed)
    }

    /// Attach a device the embedder opened (the demo device, a MIDI port). Reads its
    /// identity first, so this blocks: off the UI thread. `vid` is 0 for a non-USB
    /// link. Returns its handle.
    pub fn attach(&self, transport: Box<dyn Transport>, vid: u16) -> Result<u8, HubError> {
        self.inner.attach(transport, vid, 0, false)
    }

    /// Detach a device: it goes offline and its transport is dropped when this
    /// returns. Blocks for a transfer in flight. [`HubError::NoDevice`] if not attached.
    pub fn detach(&self, handle: u8) -> Result<(), HubError> {
        if self.inner.detach(handle, None) {
            Ok(())
        } else {
            Err(HubError::NoDevice)
        }
    }

    /// Scan the bus now, past any backoff, and wait for the result: for right after
    /// the app let go of a device. With [`HubConfig::initial_scan`] off, this is also
    /// what starts the periodic scans.
    pub fn scan_now(&self) {
        let (tx, rx) = mpsc::channel();
        if self.inner.sup.send(SupMsg::ScanNow(tx)).is_ok() {
            let _ = rx.recv_timeout(SCAN_NOW_TIMEOUT);
        }
    }

    /// Open a session for a network client that has authenticated. It follows every
    /// device through its [`Session::outbox`].
    pub fn open_session(
        &self,
        role: Role,
        client_id: Option<u32>,
    ) -> Result<Arc<Session>, HubError> {
        let session = {
            let mut st = guard(&self.inner.state);
            if self.is_stopped() {
                return Err(HubError::Stopped);
            }
            let id = next_session_id(&mut st);
            let session = Session::new(id, role, client_id, false);
            st.sessions.insert(id, Arc::clone(&session));
            self.inner
                .relay
                .add(id, Scope::AllDevices, Arc::clone(session.outbox()));
            session
        };
        self.inner
            .emit(HubEvent::SessionOpened { id: session.id() });
        Ok(session)
    }

    /// A session ended: its locks and its polls go, and it hears nothing more.
    pub fn close_session(&self, id: SessionId) {
        let (session, devices) = {
            let mut st = guard(&self.inner.state);
            let session = st.sessions.remove(&id);
            let devices: Vec<Arc<DeviceShared>> = st
                .attached
                .values()
                .map(|a| Arc::clone(&a.shared))
                .collect();
            (session, devices)
        };
        let Some(session) = session else {
            return;
        };
        self.inner.relay.remove_session(id);
        session.outbox().close();
        for d in devices {
            d.router.session_closed(id);
            d.polls.session_closed(id);
        }
        // -- WP P1HA -- its guarded applies revert
        self.apply_session_closed(id);
        self.inner.sync_locks();
        self.inner.emit(HubEvent::SessionClosed { id });
    }

    /// Run one command for a session. `done` is called exactly once: at once if it is
    /// refused or the handle has no device, else in the device's order.
    pub fn submit(&self, session: &Session, req: CmdRequest, done: Completion) {
        let device = guard(&self.inner.state)
            .attached
            .get(&req.handle)
            .map(|a| Arc::clone(&a.shared));
        match device {
            Some(d) => d.submit(session.ctx(), req, done, None),
            None => done.answer(status::NO_DEVICE, Vec::new()),
        }
    }

    // -- WP HB2 --
    /// Take the lock of the device at `handle` for a session (spec 7.7). False if it is
    /// refused: [`HubHandle::lock_request`].
    pub fn lock_acquire(&self, session: &Session, handle: u8, timeout: Duration) -> bool {
        self.lock_request(session, handle, timeout).is_ok()
    }

    /// Take or renew the lock of the device at `handle`, and how long it was granted
    /// for (`crate::lock`). `device.changed` follows. [`crate::lock::handle_message`] is
    /// this as a message.
    pub fn lock_request(
        &self,
        session: &Session,
        handle: u8,
        timeout: Duration,
    ) -> Result<Duration, LockRefusal> {
        if session.role() == Role::Viewer {
            return Err(LockRefusal::Denied);
        }
        let device = guard(&self.inner.state)
            .attached
            .get(&handle)
            .map(|a| Arc::clone(&a.shared))
            .ok_or(LockRefusal::NoDevice)?;
        let granted = device
            .router
            .lock_grant(session.id(), timeout, Instant::now())
            .ok_or(LockRefusal::Locked)?;
        // -- WP P1BFIX -- F2: another session's guarded apply there ends, kept.
        self.apply_lock_taken(&device.serial(), session.id());
        self.inner.sync_locks();
        Ok(granted)
    }

    /// Release a session's lock of the device at `handle`. `device.changed` follows.
    pub fn lock_release(&self, session: &Session, handle: u8) {
        let device = guard(&self.inner.state)
            .attached
            .get(&handle)
            .map(|a| Arc::clone(&a.shared));
        if let Some(d) = device {
            d.router.lock_release(session.id());
            self.inner.sync_locks();
        }
    }

    /// `policy.denied` for a role (spec 7.3).
    pub fn denied(&self, role: Role) -> Vec<[u16; 2]> {
        self.inner.policy.denied(role)
    }

    /// The app's way to the online device with this serial, for
    /// `DspiCore::open_transport`. Follows the device's notifications from here on.
    pub fn open_local(&self, serial: &str) -> Result<LocalTransport, HubError> {
        let inner = &self.inner;
        // Subscribed under the lock: a detach either comes after, and closes this
        // outbox, or came before, and the device is not found.
        let st = guard(&inner.state);
        if self.is_stopped() {
            return Err(HubError::Stopped);
        }
        let device = st
            .registry
            .by_serial(serial)
            .filter(|d| d.state == DeviceState::Online)
            .and_then(|d| st.attached.get(&d.handle))
            .map(|a| Arc::clone(&a.shared))
            .ok_or(HubError::NoDevice)?;
        let outbox = Arc::new(Outbox::new());
        let subscriber = inner.relay.add(
            LOCAL_SESSION_ID,
            Scope::Attachment(device.handle),
            Arc::clone(&outbox),
        );
        drop(st);
        Ok(LocalTransport::new(
            device,
            Arc::clone(&inner.local),
            Arc::clone(&inner.relay),
            subscriber,
            outbox,
        )
        .with_hub(Arc::downgrade(inner)))
    }
}

// -- WP SW2 --
impl HubHandle {
    /// The online device with this serial.
    pub(crate) fn device_by_serial(&self, serial: &str) -> Option<Arc<DeviceShared>> {
        let st = guard(&self.inner.state);
        st.registry
            .by_serial(serial)
            .filter(|d| d.state == DeviceState::Online)
            .and_then(|d| st.attached.get(&d.handle))
            .map(|a| Arc::clone(&a.shared))
    }
}

// -- WP HB3 --
impl HubHandle {
    fn device_at(&self, handle: u8) -> Option<Arc<DeviceShared>> {
        guard(&self.inner.state)
            .attached
            .get(&handle)
            .map(|a| Arc::clone(&a.shared))
    }

    /// `poll.subscribe` (spec 7.6): what each slot was granted. Frames go to the
    /// session's outbox. A subscription ends with the session, with
    /// [`HubHandle::poll_unsubscribe`], or when the device goes offline, when the client
    /// subscribes again if it still wants the feed.
    pub fn poll_subscribe(
        &self,
        session: &Session,
        handle: u8,
        specs: &[PollSpec],
    ) -> Result<Vec<PollGrant>, PollRefused> {
        self.device_at(handle)
            .ok_or(PollRefused::NoDevice)?
            .poll_subscribe(session, &self.inner.policy, specs)
    }

    /// `poll.unsubscribe`. A slot or a device that is not there is nothing to do.
    pub fn poll_unsubscribe(&self, session: &Session, handle: u8, slots: &[u8]) {
        if let Some(d) = self.device_at(handle) {
            d.polls.unsubscribe(session.id(), slots, Instant::now());
        }
    }

    /// `device.snapshot` (spec 7.5). Served from the cache; one with nothing cached yet,
    /// or just after the device invalidated it, waits for the read, up to
    /// [`SNAPSHOT_WAIT`]. So this blocks: on a connection's reader, never a hub thread.
    pub fn snapshot(&self, handle: u8) -> Result<Snapshot, SnapshotRefused> {
        self.device_at(handle)
            .ok_or(SnapshotRefused::NoDevice)?
            .snapshot_wait(SNAPSHOT_WAIT)
            .ok_or(SnapshotRefused::Busy)
    }

    /// `device.rename` (spec 7.4). `done` gets the Link status, once. It runs on the
    /// device's worker or this thread, so it must not block.
    ///
    /// A board that stores names is written as this session, so locks and the in-flight
    /// limit apply as to a `cmd`: `OK` means it accepted the write, and `device.changed`
    /// follows its notification. For older firmware the hub keeps the name, emits
    /// `device.changed` itself, and offline will do. `DENIED` for a viewer, `NO_DEVICE`
    /// for an unknown handle or a board that stores names and is not attached.
    pub fn rename(
        &self,
        session: &Session,
        handle: u8,
        name: &str,
        done: impl FnOnce(u8) + Send + 'static,
    ) {
        if session.role() == Role::Viewer {
            return done(status::DENIED);
        }
        let inner = &self.inner;
        let (known, stores_names, device, event) = {
            let mut st = guard(&inner.state);
            let stores_names = st
                .registry
                .by_handle(handle)
                .map(|d| d.identity.board_name.is_some());
            let device = st.attached.get(&handle).map(|a| Arc::clone(&a.shared));
            let event = match stores_names {
                Some(false) => st.registry.rename(handle, name),
                _ => None,
            };
            (
                stores_names.is_some(),
                stores_names == Some(true),
                device,
                event,
            )
        };
        if !known {
            return done(status::NO_DEVICE);
        }
        if !stores_names {
            inner.save_names();
            if let Some(event) = event {
                inner.publish(event);
            }
            return done(status::OK);
        }
        let Some(device) = device else {
            return done(status::NO_DEVICE);
        };
        let req = name_write(handle, &board_name_from(name));
        let answer = Completion::new(req.tag, move |r| done(r.status));
        device.submit(session.ctx(), req, answer, None);
    }
}

/// The write of a board's name.
fn name_write(handle: u8, name: &str) -> CmdRequest {
    let payload = board_name_payload(name);
    CmdRequest {
        tag: 0,
        handle,
        dir: Dir::Set,
        b_request: op::REQ_SET_DEVICE_NAME,
        w_value: 0,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: u16::try_from(payload.len()).unwrap_or(u16::MAX),
        payload,
    }
}

/// The next free id, skipping the reserved ones and those still open.
fn next_session_id(st: &mut State) -> SessionId {
    loop {
        let id = st.next_session;
        st.next_session = id.wrapping_add(1);
        let reserved = matches!(id, NO_ORIGIN | LOCAL_SESSION_ID | INTERNAL_SESSION_ID);
        if !reserved && !st.sessions.contains_key(&id) {
            return id;
        }
    }
}

impl HubInner {
    // -- WP HB3 --
    /// Save the stored names, with L1 released. A failure is logged: the name holds for
    /// this run.
    fn save_names(&self) {
        let Some(store) = &self.names else {
            return;
        };
        let _in_order = guard(&self.names_saving);
        let names = guard(&self.state).registry.stored_names().clone();
        if let Err(e) = store.save(&names) {
            log::warn!("hub: device names not saved: {e}");
        }
    }

    /// Write a stored name to the board that lacks one, as the hub's own session. Once
    /// accepted the entry goes, on the supervisor (`Completion`s must not block).
    fn adopt_name(&self, device: &DeviceShared, serial: String, name: &str) {
        let ctx = SessionCtx {
            id: INTERNAL_SESSION_ID,
            role: Role::Admin,
            exempt_inflight: true,
        };
        let req = name_write(device.handle, name);
        let sup = self.sup.clone();
        let answer = Completion::new(req.tag, move |r| {
            if r.status == status::OK {
                let _ = sup.send(SupMsg::NameAdopted { serial });
            }
        });
        device.submit(ctx, req, answer, None);
    }

    fn emit(&self, event: HubEvent) {
        let _ = self.events.send(EventMsg::Event(event));
    }

    /// Publish a registry change, with `state` released: the `device.*` message to
    /// network sessions (spec 7.4), the embedder's event, and the advertisement.
    fn publish(&self, event: RegistryEvent) {
        self.relay.broadcast_event(&event.to_json());
        // -- WP HB5 --
        self.readvertise();
        let hub_event = match event {
            RegistryEvent::Added(d) => HubEvent::DeviceOnline {
                handle: d.handle,
                serial: d.identity.serial,
            },
            RegistryEvent::Changed(d) => match d.state {
                DeviceState::Online => HubEvent::DeviceOnline {
                    handle: d.handle,
                    serial: d.identity.serial,
                },
                DeviceState::Offline => HubEvent::DeviceOffline {
                    handle: d.handle,
                    serial: d.identity.serial,
                },
            },
            RegistryEvent::Removed { handle, serial } => HubEvent::DeviceRemoved { handle, serial },
        };
        self.emit(hub_event);
    }

    // -- WP HB5 --
    /// What the record says about the devices shared now: those online.
    fn advertised_devices(&self) -> (u32, Vec<String>) {
        let serials: Vec<String> = guard(&self.state)
            .registry
            .devices()
            .into_iter()
            .filter(|d| d.state == DeviceState::Online)
            .map(|d| d.identity.serial)
            .collect();
        let count = u32::try_from(serials.len()).unwrap_or(u32::MAX);
        (count, serials)
    }

    /// A hub that cannot advertise goes on serving: it can still be reached by a typed
    /// address, and a machine with no multicast is not a reason to refuse to share.
    /// The device list is read under the lock [`Self::readvertise`] takes, so a device
    /// attaching meanwhile is in this record or updates it.
    fn start_advertising(&self, network: &NetworkConfig, port: u16, scope: AdvertiseScope) {
        let mut slot = guard(&self.advertiser);
        let (device_count, serials) = self.advertised_devices();
        let advert = HubAdvert {
            name: self.name.clone(),
            port,
            hub_id: self.hub_id.clone(),
            kind: "console".to_owned(),
            auth: network.access.wire_name().to_owned(),
            device_count,
            serials,
            web: network.web,
            path: None,
        };
        match Advertiser::start(&advert, scope) {
            Ok(advertiser) => *slot = Some(advertiser),
            Err(e) => log::warn!("hub: not advertised: {e}"),
        }
        // Wakes the supervisor from its tick, for the quick look that follows a start.
        drop(slot);
        let _ = self.sup.send(SupMsg::AdvertisingStarted);
    }

    /// With `state` released. Costs a comparison unless the record changed.
    fn readvertise(&self) {
        let mut slot = guard(&self.advertiser);
        let Some(advertiser) = slot.as_mut() else {
            return;
        };
        let (device_count, serials) = self.advertised_devices();
        let advert = HubAdvert {
            device_count,
            serials,
            ..advertiser.current().clone()
        };
        if let Err(e) = advertiser.update(&advert) {
            log::warn!("hub: advertisement not updated: {e}");
        }
    }

    /// On the supervisor's tick: conflicts dealt with and held changes sent, and the
    /// responder's bounded queue read before it fills. Returns how soon to come back,
    /// if sooner than the tick.
    fn keep_up_with_the_responder(&self) -> Option<Duration> {
        let mut slot = guard(&self.advertiser);
        let advertiser = slot.as_mut()?;
        advertiser.names();
        advertiser.poll_wait()
    }

    /// Read the identity, then register and start it. On the caller's thread, with no
    /// lock held for the reads.
    fn attach(
        &self,
        mut transport: Box<dyn Transport>,
        vid: u16,
        location_id: u32,
        from_bus: bool,
    ) -> Result<u8, HubError> {
        let mut last = TransportError::NotFound;
        let mut identity = None;
        for delay in IDENTITY_DELAYS {
            if self.stopping.load(Ordering::SeqCst) {
                return Err(HubError::Stopped);
            }
            thread::sleep(delay);
            match device::read_identity(&mut *transport, vid, location_id) {
                Ok(id) => {
                    identity = Some(id);
                    break;
                }
                Err(e @ TransportError::Disconnected) => {
                    last = e;
                    break;
                }
                Err(e) => last = e,
            }
        }
        let identity = identity.ok_or_else(|| HubError::Identity(last.to_string()))?;
        let serial = identity.serial.clone();
        // Taken before the transport moves to its worker.
        let source = transport.notifications();

        let (handle, event, returning, adopt, names_changed, shared) = {
            let mut st = guard(&self.state);
            if self.stopping.load(Ordering::SeqCst) {
                return Err(HubError::Stopped);
            }
            let online_now = st
                .registry
                .by_serial(&serial)
                .is_some_and(|d| d.state == DeviceState::Online);
            if online_now {
                return Err(HubError::AlreadyAttached(serial));
            }
            let online = st
                .registry
                .online(identity.clone())
                .ok_or(HubError::NoHandle)?;
            let generation = st.next_generation;
            st.next_generation = generation.wrapping_add(1);
            let router = Router::new(Arc::clone(&self.policy), MAX_INFLIGHT);
            let attachment = Attachment::spawn(
                transport,
                source,
                (online.handle, generation, identity, router),
                from_bus,
                Arc::clone(&self.relay),
                self.sup.clone(),
            );
            // -- WP HB3 --
            let shared;
            match attachment {
                Ok(a) => {
                    shared = Arc::clone(&a.shared);
                    st.attached.insert(online.handle, a);
                }
                Err(e) => {
                    st.registry.offline(&serial, Instant::now());
                    return Err(HubError::Io(format!("cannot start a device thread: {e}")));
                }
            }
            (
                online.handle,
                online.event,
                online.returning,
                online.adopt,
                online.names_changed,
                shared,
            )
        };
        self.publish(event);
        if returning {
            self.relay.resync_all(handle, 1);
        }
        // -- WP HB3 --
        if names_changed {
            self.save_names();
        }
        if self.warm_snapshots {
            shared.warm_snapshot();
        }
        // -- fixup sw2x (F5) -- a unit of the system model, new or back on another
        // handle: learn whether it sends 0xE1, without waiting.
        if self.system.watches(&serial) {
            // -- WP AUSB2 -- maybe new firmware: what it refused no longer holds
            self.system.attached(&serial);
            crate::system::learn_events(&self.system, &shared);
        }
        if let Some(name) = adopt {
            self.adopt_name(&shared, serial.clone(), &name);
        }
        log::info!("hub: device {serial} attached at handle {handle}");
        Ok(handle)
    }

    /// Take a device offline and tear it down, if it is still this `generation`
    /// (`None`: whichever). Never on the device's own threads.
    fn detach(&self, handle: u8, generation: Option<u64>) -> bool {
        let (attachment, event) = {
            let mut st = guard(&self.state);
            let current = st
                .attached
                .get(&handle)
                .is_some_and(|a| generation.is_none_or(|g| a.shared.generation == g));
            if !current {
                return false;
            }
            let Some(attachment) = st.attached.remove(&handle) else {
                return false;
            };
            let event = st
                .registry
                .offline(&attachment.shared.serial(), Instant::now());
            (attachment, event)
        };
        // Registry first, then the stream's end: a core that sees the end and polls
        // finds the device already off the list.
        self.relay.device_gone(handle);
        if let Some(event) = event {
            self.publish(event);
        }
        attachment.tear_down();
        true
    }

    fn scan_bus(
        &self,
        bus: &mut dyn DeviceBus,
        retry: &mut BTreeMap<String, Instant>,
        force: bool,
    ) {
        // "Cannot tell" is not "nothing there": everything stays as it is.
        let seen = match bus.scan() {
            Ok(seen) => seen,
            Err(e) => {
                log::debug!("hub: bus scan failed: {e}");
                return;
            }
        };
        // Off the bus: gone, even if no reader has noticed yet.
        let departed: Vec<u8> = {
            let st = guard(&self.state);
            st.attached
                .iter()
                .filter(|(_, a)| a.from_bus)
                .filter(|(_, a)| {
                    let serial = a.shared.serial();
                    !seen.iter().any(|d| d.serial == serial)
                })
                .map(|(h, _)| *h)
                .collect()
        };
        for handle in departed {
            self.detach(handle, None);
        }
        retry.retain(|serial, _| seen.iter().any(|d| d.serial == *serial));
        let now = Instant::now();
        let mut inaccessible = 0u32;
        for d in &seen {
            let online = guard(&self.state)
                .registry
                .by_serial(&d.serial)
                .is_some_and(|r| r.state == DeviceState::Online);
            if online {
                continue;
            }
            let waiting = retry.get(&d.serial).is_some_and(|at| now < *at);
            if waiting && !force {
                inaccessible = inaccessible.saturating_add(1);
                continue;
            }
            let attached = bus
                .open(&d.serial)
                .map_err(|e| HubError::Identity(e.to_string()))
                .and_then(|t| self.attach(t, d.vid, d.location_id, true));
            match attached {
                Ok(_) => {
                    retry.remove(&d.serial);
                }
                Err(e) => {
                    log::debug!("hub: device {} not attached: {e}", d.serial);
                    inaccessible = inaccessible.saturating_add(1);
                    retry.insert(
                        d.serial.clone(),
                        now.checked_add(REOPEN_BACKOFF).unwrap_or(now),
                    );
                }
            }
        }
        self.inaccessible.store(inaccessible, Ordering::Relaxed);
    }

    fn reap(&self) {
        let events = guard(&self.state).registry.reap(Instant::now());
        for event in events {
            self.publish(event);
        }
        self.sync_locks();
    }

    // -- WP HB2 --
    /// Bring the registry's `locked_by` up to what the routers say, and tell network
    /// sessions what changed. Called where a lock changes, and on the supervisor's tick,
    /// which is how a lock that timed out and the app's own locks are noticed. L1 and L2
    /// in turn, never nested. Not an embedder event: the device has not come or gone.
    ///
    /// One at a time (WP HUBFLAKE4). Unserialised, the tick could write the registry,
    /// and a `lock_request` find nothing new and answer before the tick had sent the
    /// `device.changed`; or a sync could write a holder it read before another sync's
    /// newer one, and send it last.
    fn sync_locks(&self) {
        let _in_order = guard(&self.locks_syncing);
        let devices: Vec<Arc<DeviceShared>> = guard(&self.state)
            .attached
            .values()
            .map(|a| Arc::clone(&a.shared))
            .collect();
        let now = Instant::now();
        let holders: Vec<(u8, Option<SessionId>)> = devices
            .iter()
            .map(|d| (d.handle, d.router.lock_holder(now)))
            .collect();
        let events: Vec<RegistryEvent> = {
            let mut st = guard(&self.state);
            holders
                .into_iter()
                .filter_map(|(handle, by)| st.registry.set_locked_by(handle, by))
                .collect()
        };
        for event in events {
            self.relay.broadcast_event(&event.to_json());
        }
    }
}

/// Owns the bus, and is the one thread that detaches a lost device: a reader or a
/// worker cannot join itself, so they post here.
fn supervisor_loop(
    inner: &HubInner,
    rx: &Receiver<SupMsg>,
    mut bus: Option<Box<dyn DeviceBus>>,
    interval: Duration,
    mut scanning: bool,
) {
    let mut retry = BTreeMap::new();
    let mut next_scan = Instant::now();
    // -- WP HB5 --
    let mut responder_wait = None;
    loop {
        let wait = next_scan.saturating_duration_since(Instant::now());
        let wait = responder_wait.map_or(wait, |sooner: Duration| wait.min(sooner));
        match rx.recv_timeout(wait) {
            Ok(SupMsg::Lost { handle, generation }) => {
                if inner.detach(handle, Some(generation)) {
                    log::info!("hub: device at handle {handle} lost");
                }
            }
            Ok(SupMsg::ScanNow(done)) => {
                scanning = true;
                if let Some(bus) = bus.as_deref_mut() {
                    inner.scan_bus(bus, &mut retry, true);
                }
                let _ = done.send(());
            }
            // -- WP HB3 --
            Ok(SupMsg::BoardName { serial, name }) => {
                let named = guard(&inner.state)
                    .registry
                    .board_name_stored(&serial, &name);
                if named.names_changed {
                    inner.save_names();
                }
                if let Some(event) = named.event {
                    inner.publish(event);
                }
            }
            Ok(SupMsg::NameAdopted { serial }) => {
                let retired = guard(&inner.state).registry.adoption_accepted(&serial);
                if retired {
                    inner.save_names();
                }
            }
            // -- WP HB5 --
            Ok(SupMsg::AdvertisingStarted) => {
                responder_wait = inner.keep_up_with_the_responder();
            }
            Ok(SupMsg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                // -- WP HB5 --
                responder_wait = inner.keep_up_with_the_responder();
                if Instant::now() < next_scan {
                    continue;
                }
                if let Some(bus) = bus.as_deref_mut().filter(|_| scanning) {
                    inner.scan_bus(bus, &mut retry, false);
                }
                inner.reap();
                next_scan = Instant::now().checked_add(interval).unwrap_or(next_scan);
            }
        }
    }
}

// -- WP AUSB2 -- (review fixm1 F2)
/// The hub's own poll of the system model: every
/// [`crate::system::SYSTEM_POLL`] (or as set), while the model has USB-direct
/// units, [`HubHandle::system_poll_now`]. Ends when `stop` is sent or dropped.
fn system_poll_loop(hub: &HubHandle, stop: &Receiver<()>) {
    // -- WP P1HA -- also the guarded applies' timer: a message while the hub is not
    // stopping is a wake (a guard begun or ended early), and the wait ends at the
    // first guard's deadline. The system poll keeps its own schedule.
    let mut next_poll = Instant::now()
        .checked_add(hub.system_poll_due().0)
        .unwrap_or_else(Instant::now);
    loop {
        let until = hub
            .apply_next_due()
            .map_or(next_poll, |due| due.min(next_poll));
        match stop.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) if !hub.is_stopped() => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
        if hub.is_stopped() {
            return;
        }
        // -- WP P1HA --
        hub.apply_run_due();
        if Instant::now() >= next_poll {
            let (wait, due) = hub.system_poll_due();
            if due {
                hub.system_poll_now();
            }
            next_poll = Instant::now().checked_add(wait).unwrap_or(next_poll);
        }
    }
}

/// Calls the listener, so no device thread ever waits for the embedder.
fn event_loop(inner: &HubInner, rx: &Receiver<EventMsg>) {
    let _ = inner.event_thread.set(thread::current().id());
    while let Ok(EventMsg::Event(event)) = rx.recv() {
        let listener = guard(&inner.listener).clone();
        if let Some(listener) = listener {
            // A listener's panic loses that call, not the hub (PLAN section 1.7).
            let called = panic::catch_unwind(AssertUnwindSafe(|| listener.on_event(event)));
            if called.is_err() {
                log::error!("a hub listener panicked");
            }
        }
    }
}

// -- WP HUBFLAKE4 --
#[cfg(test)]
mod tests_wp_hubflake4 {
    use super::*;
    use crate::session::{Outbound, Pop};
    use dspi_transport::FakeDevice;

    const WAIT: Duration = Duration::from_secs(10);

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn device_changes(s: &Session) -> Vec<String> {
        let mut all = Vec::new();
        while let Pop::Item(item) = s.outbox().pop(Duration::ZERO) {
            if let Outbound::Event(json) = item {
                all.push(json);
            }
        }
        all
    }

    /// The order that made `wp_hb2_device_changed_follows_the_lock_even_when_it_times_out`
    /// flaky, held still: one sync (the tick's) has written the registry and not yet sent
    /// `device.changed`. A second sync (a `lock_request`'s) finds nothing new to write,
    /// and must still not return before that message is out.
    #[test]
    fn wp_hubflake4_a_lock_sync_returns_only_after_the_change_is_sent() {
        let hub = Hub::start(HubConfig {
            // No tick in the way: the test runs both syncs itself.
            scan_interval: Duration::from_secs(3600),
            ..HubConfig::in_process("hubflake4")
        })
        .unwrap();
        let handle = hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
        let h = hub.handle();
        let watcher = h.open_session(Role::Viewer, None).unwrap();
        let holder = h.open_session(Role::Admin, None).unwrap();
        let device = Arc::clone(&guard(&h.inner.state).attached[&handle].shared);
        let locked_by = || guard(&h.inner.state).registry.devices()[usize::from(handle)].locked_by;
        // The lock changes in the router; nobody has told the registry yet.
        device
            .router
            .lock_grant(holder.id(), Duration::from_secs(60), Instant::now())
            .unwrap();
        assert_eq!(locked_by(), None);

        let held = h.inner.relay.hold();
        let first = {
            let inner = Arc::clone(&h.inner);
            thread::spawn(move || inner.sync_locks())
        };
        wait_until("the first sync to write the registry", || {
            locked_by() == Some(holder.id())
        });
        let (done, second) = mpsc::channel();
        let second_sync = {
            let inner = Arc::clone(&h.inner);
            thread::spawn(move || {
                inner.sync_locks();
                let _ = done.send(());
            })
        };
        assert!(
            second.recv_timeout(Duration::from_millis(200)).is_err(),
            "a sync returned before the device.changed it found was sent"
        );
        drop(held);
        second.recv_timeout(WAIT).unwrap();
        first.join().unwrap();
        second_sync.join().unwrap();

        let sent = device_changes(&watcher);
        assert_eq!(sent.len(), 1, "{sent:?}");
        let event: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(event["t"], "device.changed");
        assert_eq!(event["device"]["locked_by"], holder.id());
    }
}
