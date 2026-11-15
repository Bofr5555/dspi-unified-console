//! The hub's service seam (WP P2SV, PLAN-P2 section 2): one module per phase-2
//! family, all plugged in the same way.
//!
//! A service is a [`HubService`]. The hub consults it:
//! - for every control-plane message of an authenticated session, after the lock's,
//!   the guarded apply's and the limits' handlers (`server.rs::on_text`), and before
//!   the hub's own `device.*`, `poll.*`, `auth.*`, `hub.*` and `system.*` answers, so a
//!   service may answer `hub.stats` or `hub.rename` (otherwise they stay `unsupported`);
//! - for `hello.caps`, which lists every service's caps;
//! - on the `hub-system` timer thread, which wakes at the earliest
//!   [`HubService::next_due`] of all services and calls [`HubService::run_due`] of each
//!   one that is due;
//! - when a device comes online, goes offline or is forgotten
//!   ([`HubService::on_device`]).
//!
//! Everything the hub sees also goes onto the [`crate::bus::Bus`], which a service
//! subscribes to when it is made.
//!
//! **Adding a service.** A WP adds its module, its `mod` line in `lib.rs`, a field in
//! [`ServiceConfig`] (off by default) and one line in [`registered`], each under its
//! `// -- WP <ID> --` marker.
//!
//! **The session-id rule (PLAN-P2 M2).** A service that writes on someone's behalf
//! (an API key, an MQTT key, a schedule rule) does so as a principal opened with
//! [`crate::HubHandle::open_principal`]: a session from the remote range, judged by
//! the policy, the limits and the lock like any client. Never as
//! [`crate::session::LOCAL_SESSION_ID`], [`crate::session::INTERNAL_SESSION_ID`] or
//! [`crate::session::GUARD_SESSION_ID`]: the hub's own writes are a closed list
//! (PLAN-P2 M3).

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde_json::{Value, json};

use crate::bus::{Bus, BusMessage};
use crate::clock::{HubClock, SystemClock};
use crate::error::HubError;
use crate::hub::HubHandle;
use crate::relay::{PacketTap, Relay};
use crate::session::{
    GUARD_SESSION_ID, INTERNAL_SESSION_ID, LOCAL_SESSION_ID, NO_ORIGIN, Role, Session, SessionId,
};

/// What `by` says for the embedding app's own session.
pub const LOCAL_LABEL: &str = "Hub console";
/// What `by` says for the hub's own writes.
pub const HUB_LABEL: &str = "Hub";
/// Labels longer than this are cut (a client's name is the client's to choose).
const MAX_LABEL_CHARS: usize = 64;

/// A device coming or going, as [`HubService::on_device`] hears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// Attached: new, or back.
    Online {
        /// Its serial.
        serial: String,
        /// Its handle.
        handle: u8,
    },
    /// Detached; its handle is kept for the grace.
    Offline {
        /// Its serial.
        serial: String,
        /// Its handle.
        handle: u8,
    },
    /// Its grace ended.
    Removed {
        /// Its serial.
        serial: String,
        /// The handle it had.
        handle: u8,
    },
}

impl From<DeviceEvent> for BusMessage {
    fn from(e: DeviceEvent) -> Self {
        match e {
            DeviceEvent::Online { serial, handle } => BusMessage::DeviceOnline { serial, handle },
            DeviceEvent::Offline { serial, handle } => BusMessage::DeviceOffline { serial, handle },
            DeviceEvent::Removed { serial, handle } => BusMessage::DeviceRemoved { serial, handle },
        }
    }
}

/// One phase-2 family of the hub. Every method but [`HubService::name`] has a default
/// that does nothing, so a service implements what it uses.
///
/// Threads: `handle_message` runs on the asking connection's reader and may block it
/// (that connection alone); `run_due` on the `hub-system` thread, which the guarded
/// applies and the limits share, so it must be short; `on_device` on whichever thread
/// attached or detached the device, with no hub lock held, so it must not block
/// (queue the work for `run_due` and call [`crate::HubHandle::wake_services`]). A
/// panic loses that one call, never the hub.
pub trait HubService: Send + Sync {
    /// A short name, for logs.
    fn name(&self) -> &'static str;

    /// What it adds to `hello.caps`.
    fn caps(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Answer a control-plane message, or `None` if it is not this service's. The
    /// answer is the whole reply text (`{"t":"ok"|"err","id":…}`); the service checks
    /// the session's role itself.
    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let _ = (hub, session, msg);
        None
    }

    /// When it wants [`HubService::run_due`] next, on [`crate::HubHandle::clock`].
    fn next_due(&self) -> Option<Instant> {
        None
    }

    /// Its time has come: `now` is the hub clock's, at or past [`HubService::next_due`].
    fn run_due(&self, hub: &HubHandle, now: Instant) {
        let _ = (hub, now);
    }

    /// A device came online, went offline or was forgotten.
    fn on_device(&self, event: &DeviceEvent) {
        let _ = event;
    }

    /// The hub is stopping: end any thread of its own. The devices are still attached.
    fn stop(&self) {}
}

/// The optional services, each off unless asked for. A WP adds its own field here.
#[derive(Clone)]
pub struct ServiceConfig {
    /// The services' time. [`SystemClock`] but in tests.
    pub clock: Arc<dyn HubClock>,
    /// Services the embedder brings, after the hub's own (tests, a binding).
    pub extra: Vec<Arc<dyn HubService>>,
    // -- WP P2HP --
    /// The health poll and device facts (`crate::health`): `None` is off.
    pub health: Option<crate::health::HealthConfig>,
    // -- WP P2IN --
    /// The inventory (`inventory.*`, `<data>/inventory.json`).
    pub inventory: bool,
    // -- WP P2ID --
    /// `device.identify` (`identify.rs`).
    pub identify: bool,
    // -- WP P2HS --
    /// `hub.stats` and `hub.rename` (spec 7.9). On by default: they are DSPi Link's own
    /// messages, not an optional family. Off, both answer `unsupported`.
    pub stats: bool,
    // -- WP P2EV --
    /// The shared event history (`events/`, `event.*`); off when `None`.
    pub history: Option<crate::history::HistoryConfig>,
    // -- WP P2AL --
    /// Device alarms (`alarm.*`, `<data>/alarms.json`, `device_alarms.rs`).
    pub alarms: bool,
    // -- WP P2SC --
    /// Schedules (`schedule.*`, `<data>/schedule.json`); off when `None`. They run
    /// through the parameter API's executor: feature `api`.
    pub schedule: Option<crate::schedule::ScheduleConfig>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            extra: Vec::new(),
            // -- WP P2HP --
            health: None,
            // -- WP P2IN --
            inventory: false,
            // -- WP P2ID --
            identify: false,
            // -- WP P2HS --
            stats: true,
            // -- WP P2EV --
            history: None,
            // -- WP P2AL --
            alarms: false,
            // -- WP P2SC --
            schedule: None,
        }
    }
}

/// What a service is made with.
pub struct ServiceSetup<'a> {
    /// The hub's data folder (`HubConfig::data_dir`); `None` for an in-process hub,
    /// whose services keep nothing on disk.
    pub data_dir: Option<&'a Path>,
    /// The services' time.
    pub clock: &'a Arc<dyn HubClock>,
    /// Where to subscribe and publish.
    pub bus: &'a Arc<Bus>,
    /// How to wake the timer when `next_due` moved earlier.
    pub wake: &'a Waker,
}

/// The list: every service the hub runs, in the order they are consulted.
fn registered(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Vec<Arc<dyn HubService>> {
    #[allow(unused_mut)]
    let mut list: Vec<Arc<dyn HubService>> = Vec::new();
    // One line per service, each under its WP's marker:
    //   list.extend(crate::<module>::service(config, setup));
    // -- WP P2HP --
    list.extend(crate::health::service(config, setup));
    // -- WP P2IN --
    list.extend(crate::inventory::service(config, setup));
    // -- WP P2ID --
    list.extend(crate::identify::service(config, setup));
    // -- WP P2HS --
    list.extend(crate::stats::service(config, setup));
    // -- WP P2EV --
    list.extend(crate::history::service(config, setup));
    // -- WP P2PA -- the `snapshot` family (P2SS's store) and the parameter API beside it
    list.extend(crate::api::services(setup));
    // -- WP P2WH --
    list.extend(crate::outbound::service(config, setup));
    // -- WP P2AL --
    list.extend(crate::device_alarms::service(config, setup));
    // -- WP P2SC --
    list.extend(crate::schedule::service(config, setup));
    // -- WP P2MQ --
    list.extend(crate::mqtt::service::service(config, setup));
    list.extend(config.extra.iter().cloned());
    list
}

/// Wakes the `hub-system` thread, so it reads every `next_due` again.
#[derive(Debug, Clone, Default)]
pub struct Waker(Arc<Mutex<Option<Sender<()>>>>);

impl Waker {
    /// Ask the timer to look again. Never blocks; nothing happens before it runs.
    pub fn wake(&self) {
        if let Some(tx) = guard(&self.0).as_ref() {
            let _ = tx.send(());
        }
    }

    pub(crate) fn set(&self, tx: Option<Sender<()>>) {
        *guard(&self.0) = tx;
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Who a session is, for attribution: the paired client's `cid` and name, or the
/// principal's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLabel {
    /// The session.
    pub session: SessionId,
    /// The paired client it authenticated as.
    pub cid: Option<u32>,
    /// The client's name, or a principal's (`Schedule: Morning`).
    pub name: Option<String>,
}

impl SessionLabel {
    /// What a history record's `by` says: the name, else `Client <cid>`, else
    /// `Session <id>`; `None` for no session.
    pub fn by(&self) -> Option<String> {
        if self.session == NO_ORIGIN {
            return None;
        }
        match (&self.name, self.cid) {
            (Some(name), _) if !name.is_empty() => Some(name.clone()),
            (_, Some(cid)) => Some(format!("Client {cid}")),
            _ => Some(format!("Session {}", self.session)),
        }
    }
}

fn clip(name: &str) -> String {
    name.trim().chars().take(MAX_LABEL_CHARS).collect()
}

/// Labels of the open sessions: written when one opens or closes, read for every
/// notification. A leaf lock.
#[derive(Debug, Default)]
pub(crate) struct Labels(Mutex<BTreeMap<SessionId, SessionLabel>>);

impl Labels {
    fn set(&self, label: SessionLabel) {
        guard(&self.0).insert(label.session, label);
    }

    fn remove(&self, id: SessionId) {
        guard(&self.0).remove(&id);
    }

    fn get(&self, id: SessionId) -> SessionLabel {
        let fixed = |name: &str| SessionLabel {
            session: id,
            cid: None,
            name: Some(name.to_owned()),
        };
        match id {
            LOCAL_SESSION_ID => fixed(LOCAL_LABEL),
            INTERNAL_SESSION_ID | GUARD_SESSION_ID => fixed(HUB_LABEL),
            _ => guard(&self.0).get(&id).cloned().unwrap_or(SessionLabel {
                session: id,
                cid: None,
                name: None,
            }),
        }
    }
}

/// Puts every relayed notification on the bus, with its serial and session.
struct BusTap {
    bus: Arc<Bus>,
    labels: Arc<Labels>,
}

impl PacketTap for BusTap {
    fn wants(&self, _packet: &[u8]) -> bool {
        self.bus.has_subscribers()
    }

    fn on_packet(&self, _relay: &Relay, serial: &str, origin: SessionId, packet: &[u8]) {
        self.bus.publish(BusMessage::Notification {
            serial: serial.to_owned(),
            session: origin,
            by: self.labels.get(origin).by(),
            packet: Arc::from(packet),
        });
    }
}

/// The services of one hub, the bus and what they share.
pub(crate) struct Services {
    list: Vec<Arc<dyn HubService>>,
    clock: Arc<dyn HubClock>,
    bus: Arc<Bus>,
    labels: Arc<Labels>,
    wake: Waker,
    data_dir: Option<PathBuf>,
    /// Client names for labels; `None` for an in-process hub.
    auth: Option<Arc<crate::auth::AuthStore>>,
}

impl Services {
    /// Make every registered service, and tap the relay for the bus.
    pub(crate) fn new(
        config: &ServiceConfig,
        data_dir: Option<PathBuf>,
        auth: Option<Arc<crate::auth::AuthStore>>,
        relay: &Relay,
    ) -> Self {
        let bus = Bus::new();
        let wake = Waker::default();
        let clock = Arc::clone(&config.clock);
        let list = registered(
            config,
            &ServiceSetup {
                data_dir: data_dir.as_deref(),
                clock: &clock,
                bus: &bus,
                wake: &wake,
            },
        );
        let labels = Arc::new(Labels::default());
        relay.add_tap(Arc::new(BusTap {
            bus: Arc::clone(&bus),
            labels: Arc::clone(&labels),
        }));
        Self {
            list,
            clock,
            bus,
            labels,
            wake,
            data_dir,
            auth,
        }
    }

    // -- WP P2HS --
    /// The hub's store, for `hub.rename`. `None` for an in-process hub.
    pub(crate) fn auth(&self) -> Option<&Arc<crate::auth::AuthStore>> {
        self.auth.as_ref()
    }

    pub(crate) fn set_wake(&self, tx: Option<Sender<()>>) {
        self.wake.set(tx);
    }

    /// Label a session that just opened. With no hub lock held (the store's is taken).
    pub(crate) fn session_opened(&self, session: &Session, name: Option<&str>) {
        let name = match (name, session.client_id(), &self.auth) {
            (Some(name), _, _) => Some(clip(name)),
            (None, Some(cid), Some(auth)) => auth
                .clients()
                .into_iter()
                .find(|c| c.cid == cid)
                .map(|c| clip(&c.name)),
            _ => None,
        };
        self.labels.set(SessionLabel {
            session: session.id(),
            cid: session.client_id(),
            name,
        });
    }

    pub(crate) fn session_closed(&self, id: SessionId) {
        self.labels.remove(id);
    }

    /// A device came or went: onto the bus, then to each service.
    pub(crate) fn device(&self, event: DeviceEvent) {
        for s in &self.list {
            let called = panic::catch_unwind(AssertUnwindSafe(|| s.on_device(&event)));
            if called.is_err() {
                log::error!("hub: service {} panicked on a device event", s.name());
            }
        }
        self.bus.publish(event.into());
    }

    fn message(&self, hub: &HubHandle, session: &Arc<Session>, msg: &Value) -> Option<String> {
        self.list.iter().find_map(|s| {
            panic::catch_unwind(AssertUnwindSafe(|| s.handle_message(hub, session, msg)))
                .unwrap_or_else(|_| {
                    log::error!("hub: service {} panicked on a message", s.name());
                    let mut reply = json!({"t": "err", "code": "internal"});
                    if let (Some(id), Some(map)) = (msg.get("id"), reply.as_object_mut()) {
                        map.insert("id".into(), id.clone());
                    }
                    Some(reply.to_string())
                })
        })
    }

    fn caps(&self) -> Vec<&'static str> {
        let mut caps: Vec<&'static str> = Vec::new();
        for c in self.list.iter().flat_map(|s| s.caps()) {
            if !caps.contains(&c) {
                caps.push(c);
            }
        }
        caps
    }

    /// The earliest `next_due`, as a real-time deadline for the timer's wait.
    fn next_due(&self) -> Option<Instant> {
        let due = self.list.iter().filter_map(|s| s.next_due()).min()?;
        let wait = due.saturating_duration_since(self.clock.now());
        let now = Instant::now();
        Some(now.checked_add(wait).unwrap_or(now))
    }

    fn run_due(&self, hub: &HubHandle) {
        let now = self.clock.now();
        for s in &self.list {
            if s.next_due().is_some_and(|due| due <= now) {
                let called = panic::catch_unwind(AssertUnwindSafe(|| s.run_due(hub, now)));
                if called.is_err() {
                    log::error!("hub: service {} panicked on its timer", s.name());
                }
            }
        }
    }

    pub(crate) fn stop(&self) {
        for s in &self.list {
            if panic::catch_unwind(AssertUnwindSafe(|| s.stop())).is_err() {
                log::error!("hub: service {} panicked stopping", s.name());
            }
        }
    }
}

impl HubHandle {
    /// The hub's internal bus: subscribe to see what the hub sees.
    pub fn bus(&self) -> &Arc<Bus> {
        &self.inner.services.bus
    }

    /// The services' time.
    pub fn clock(&self) -> &Arc<dyn HubClock> {
        &self.inner.services.clock
    }

    /// The hub's data folder, where each service keeps its file. `None` in process.
    pub fn data_dir(&self) -> Option<&Path> {
        self.inner.services.data_dir.as_deref()
    }

    /// Who a session is, for a record's `by` (PLAN-P2 M1).
    pub fn session_label(&self, id: SessionId) -> SessionLabel {
        self.inner.services.labels.get(id)
    }

    /// Open a principal's session (PLAN-P2 M2): an API key's, an MQTT key's, a
    /// schedule rule's (`Schedule: <name>`). Its id is from the remote range, so the
    /// policy, the limits, the voltage rule and the lock judge it as any client's.
    /// Close it with [`HubHandle::close_session`] when done.
    pub fn open_principal(
        &self,
        role: Role,
        client_id: Option<u32>,
        name: &str,
    ) -> Result<Arc<Session>, HubError> {
        let session = self.open_session(role, client_id)?;
        self.inner.services.session_opened(&session, Some(name));
        Ok(session)
    }

    /// Make the timer read every service's `next_due` again.
    pub fn wake_services(&self) {
        self.inner.services.wake.wake();
    }

    /// The services' answer to a control-plane message, if one claims it.
    pub(crate) fn service_message(&self, session: &Arc<Session>, msg: &Value) -> Option<String> {
        self.inner.services.message(self, session, msg)
    }

    // -- WP P2TM --
    /// A phase-2 family's answer to `msg` for `session`, as a client on DSPi Link
    /// would get it (the whole reply text), or `None` when no service runs that
    /// family. For the embedder's own console (`uconsole hub`'s prompt): open the
    /// session as a principal ([`HubHandle::open_principal`]) and close it after.
    /// Only the services answer here: no `device.*`, `auth.*` or `limits.*`.
    pub fn service_request(&self, session: &Arc<Session>, msg: &Value) -> Option<String> {
        self.service_message(session, msg)
    }

    /// Every service's caps, for `hello`.
    pub(crate) fn service_caps(&self) -> Vec<&'static str> {
        self.inner.services.caps()
    }

    /// When the timer must wake for a service, in real time.
    pub(crate) fn services_next_due(&self) -> Option<Instant> {
        self.inner.services.next_due()
    }

    /// Run every service that is due.
    pub(crate) fn services_run_due(&self) {
        self.inner.services.run_due(self);
    }
}

#[cfg(test)]
mod tests_wp_p2sv {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn wp_p2sv_labels_say_who_a_session_is() {
        let labels = Labels::default();
        labels.set(SessionLabel {
            session: 7,
            cid: Some(3),
            name: Some("Front of house".into()),
        });
        labels.set(SessionLabel {
            session: 8,
            cid: Some(4),
            name: Some(String::new()),
        });
        assert_eq!(labels.get(7).by().as_deref(), Some("Front of house"));
        assert_eq!(labels.get(7).cid, Some(3));
        assert_eq!(labels.get(8).by().as_deref(), Some("Client 4"));
        assert_eq!(labels.get(9).by().as_deref(), Some("Session 9"));
        assert_eq!(labels.get(NO_ORIGIN).by(), None);
        assert_eq!(
            labels.get(LOCAL_SESSION_ID).by().as_deref(),
            Some(LOCAL_LABEL)
        );
        assert_eq!(
            labels.get(INTERNAL_SESSION_ID).by().as_deref(),
            Some(HUB_LABEL)
        );
        assert_eq!(
            labels.get(GUARD_SESSION_ID).by().as_deref(),
            Some(HUB_LABEL)
        );
        labels.remove(7);
        assert_eq!(labels.get(7).by().as_deref(), Some("Session 7"));
    }

    #[test]
    fn wp_p2sv_a_long_name_is_cut() {
        let long = "é".repeat(200);
        assert_eq!(clip(&long).chars().count(), MAX_LABEL_CHARS);
    }
}
