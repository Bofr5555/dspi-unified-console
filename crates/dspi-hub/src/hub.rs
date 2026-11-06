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

use dspi_link::frames::CmdRequest;
use dspi_link::status;
use dspi_transport::{Transport, TransportError};

use crate::advertise::Advertiser;
use crate::device::{self, Attachment, DeviceShared, SupMsg};
use crate::error::HubError;
use crate::local::LocalTransport;
use crate::policy::Policy;
use crate::registry::{DeviceState, OFFLINE_GRACE, RegisteredDevice, Registry, RegistryEvent};
use crate::relay::{Relay, Scope};
use crate::router::Router;
use crate::server::{NetworkConfig, Server};
use crate::session::{
    CLOSE_SHUTTING_DOWN, Completion, FIRST_REMOTE_SESSION_ID, INTERNAL_SESSION_ID,
    LOCAL_SESSION_ID, NO_ORIGIN, Outbox, Role, Session, SessionId,
};
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

/// What a hub is started with.
pub struct HubConfig {
    /// `hid` (spec 7.2), a lowercase UUID. HB1 takes it from the store.
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
    policy: Arc<Policy>,
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
    /// HB5 sets it after the server has bound. Dropped first in [`Hub::stop`], on the
    /// caller's thread: its goodbye can take a second.
    advertiser: Option<Advertiser>,
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
        let (sup_tx, sup_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let inner = Arc::new(HubInner {
            name: config.name,
            hub_id: config.hub_id,
            policy: Arc::new(Policy::embedded()),
            state: Mutex::new(State {
                registry: Registry::new(config.offline_grace),
                attached: BTreeMap::new(),
                sessions: BTreeMap::new(),
                next_session: FIRST_REMOTE_SESSION_ID,
                next_generation: 1,
            }),
            relay: Arc::new(Relay::default()),
            stopping: AtomicBool::new(false),
            sup: sup_tx,
            events: ev_tx,
            event_thread: OnceLock::new(),
            listener: Mutex::new(None),
            local: Session::new(LOCAL_SESSION_ID, Role::Admin, None, true),
            inaccessible: AtomicU32::new(0),
            #[cfg(feature = "app")]
            app_attached: AtomicBool::new(false),
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
        let hub = Self {
            handle: HubHandle { inner },
            threads: Mutex::new(Some(Threads {
                supervisor,
                events,
                server: None,
                advertiser: None,
            })),
        };
        if let Some(network) = network {
            // `hub` is dropped on failure, which stops what started above.
            let server = Server::start(hub.handle.clone(), &network)?;
            if let Some(t) = guard(&hub.threads).as_mut() {
                t.server = Some(server);
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
        // 1. Off the network: goodbye, no new connections. 2. Everyone gets their close.
        // 3. Connections go.
        {
            let _goodbye = t.advertiser;
        }
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

    /// Take the lock of the device at `handle` for a session (spec 7.7). False if
    /// another session holds it or there is no such device.
    pub fn lock_acquire(&self, session: &Session, handle: u8, timeout: Duration) -> bool {
        let device = guard(&self.inner.state)
            .attached
            .get(&handle)
            .map(|a| Arc::clone(&a.shared));
        device.is_some_and(|d| d.router.lock_acquire(session.id(), timeout, Instant::now()))
    }

    /// Release a session's lock of the device at `handle`.
    pub fn lock_release(&self, session: &Session, handle: u8) {
        let device = guard(&self.inner.state)
            .attached
            .get(&handle)
            .map(|a| Arc::clone(&a.shared));
        if let Some(d) = device {
            d.router.lock_release(session.id());
        }
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
        ))
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
    fn emit(&self, event: HubEvent) {
        let _ = self.events.send(EventMsg::Event(event));
    }

    /// Publish a registry change, with `state` released: the `device.*` message to
    /// network sessions (spec 7.4), and the embedder's event. HB5's advertiser update
    /// goes here too.
    fn publish(&self, event: RegistryEvent) {
        self.relay.broadcast_event(&event.to_json());
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

        let (handle, event, returning) = {
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
            match attachment {
                Ok(a) => {
                    st.attached.insert(online.handle, a);
                }
                Err(e) => {
                    st.registry.offline(&serial, Instant::now());
                    return Err(HubError::Io(format!("cannot start a device thread: {e}")));
                }
            }
            (online.handle, online.event, online.returning)
        };
        self.publish(event);
        if returning {
            self.relay.resync_all(handle, 1);
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
    loop {
        let wait = next_scan.saturating_duration_since(Instant::now());
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
            Ok(SupMsg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(bus) = bus.as_deref_mut().filter(|_| scanning) {
                    inner.scan_bus(bus, &mut retry, false);
                }
                inner.reap();
                next_scan = Instant::now().checked_add(interval).unwrap_or(next_scan);
            }
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
