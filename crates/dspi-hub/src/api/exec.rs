//! The action executor (WP P2PA): one [`DspiCore`] per (principal, device), so every
//! later family (the parameter API, presets, hub-side recall, schedules, MQTT) writes
//! through the same core code as a console.
//!
//! - **Opened lazily, over a child session** of the principal ([`ChildSession`]): its
//!   role, its client, its own id from the remote range (the session-id rule). Each open
//!   is a full attach (probe and image read), so a schedule on eight devices opens
//!   eight cores; the idle close bounds them (review N8).
//! - **Closed** after [`IDLE_CLOSE`] unused, when the principal goes (its session closes),
//!   or when the device goes (a core is bound to one attachment); the sweep runs on the
//!   hub's timer every [`SWEEP`] while any core is open. A device back after a replug gets
//!   a new core on the next action.
//! - **A reader** beside the core, on a second transport of the same child session and
//!   without notifications: `dspi_session::Session::read` reads one parameter back from
//!   the device for `param.get` (the core has no read by path), and its capabilities say
//!   what the device offers for `param.list`.
//! - **Bounded:** at most [`MAX_CORES`] open at once; past it an action answers `busy`.
//!   One action at a time per (principal, device), which a principal's connection already
//!   is (its messages are answered in order on its reader).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_session::Session as Reader;

use super::ApiError;
use crate::clock::HubClock;
use crate::device::DeviceShared;
use crate::hub::HubHandle;
use crate::registry::DeviceState;
use crate::service::Waker;
use crate::session::{ChildSession, Session, SessionId};

/// A core unused this long is closed.
pub const IDLE_CLOSE: Duration = Duration::from_secs(60);
/// How often the open cores are looked at.
pub const SWEEP: Duration = Duration::from_secs(1);
/// The most cores open at once, over every principal and device.
pub const MAX_CORES: usize = 32;

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What an entry opens lazily: the core, the reader.
#[derive(Default)]
struct Opened {
    core: Option<Arc<DspiCore>>,
    reader: Option<Reader>,
}

/// One (principal, device).
struct Entry {
    child: ChildSession,
    serial: String,
    vid: u16,
    device: Arc<DeviceShared>,
    /// The lock an action holds while it runs; the sweep only takes it with `try_lock`.
    opened: Mutex<Opened>,
    used: Mutex<Instant>,
    closed: AtomicBool,
}

impl Entry {
    /// Whether this entry can still be used for the device online now as `device`.
    fn current(&self, device: &Arc<DeviceShared>) -> bool {
        !self.closed.load(Ordering::SeqCst) && Arc::ptr_eq(&self.device, device)
    }
}

/// An action's view of its (principal, device): the core and the reader, opened on
/// first use.
pub(crate) struct Cx<'a> {
    hub: &'a HubHandle,
    entry: &'a Entry,
    opened: &'a mut Opened,
}

impl Cx<'_> {
    /// The device's serial.
    pub(crate) fn serial(&self) -> &str {
        &self.entry.serial
    }

    /// The child session's id (what attributes the writes).
    pub(crate) fn child_id(&self) -> SessionId {
        self.entry.child.id()
    }

    /// The child session's label, for a record's `by`.
    pub(crate) fn by(&self) -> Option<String> {
        self.hub.session_label(self.child_id()).by()
    }

    /// The core, opened (attached) on first use.
    pub(crate) fn core(&mut self) -> Result<Arc<DspiCore>, ApiError> {
        if let Some(core) = &self.opened.core {
            // Fold in what others changed since, so a setter that sends a whole record
            // (an EQ band, a limiter) sends their values, not stale ones.
            let _ = core.process_notifications();
            return Ok(Arc::clone(core));
        }
        let transport = self
            .hub
            .open_as(&self.entry.serial, &self.entry.child, true)
            .map_err(|_| ApiError::offline())?;
        let core = DspiCore::new();
        core.open_transport(
            Box::new(transport),
            self.entry.serial.clone(),
            self.entry.vid,
        )
        .map_err(ApiError::from)?;
        self.opened.core = Some(Arc::clone(&core));
        Ok(core)
    }

    /// The reader, opened (probed) on first use.
    pub(crate) fn reader(&mut self) -> Result<&mut Reader, ApiError> {
        if self.opened.reader.is_none() {
            let mut transport = self
                .hub
                .open_as(&self.entry.serial, &self.entry.child, false)
                .map_err(|_| ApiError::offline())?;
            let caps = dspi_session::probe::probe(&mut transport).map_err(ApiError::transport)?;
            let reader = Reader::new(Box::new(transport), caps).ok_or_else(|| {
                ApiError::new(
                    "unsupported",
                    "the device's channel layout is not one this hub reads",
                )
            })?;
            self.opened.reader = Some(reader);
        }
        self.opened
            .reader
            .as_mut()
            .ok_or_else(|| ApiError::new("internal", "the reader did not open"))
    }
}

/// The executor: every open (principal, device).
pub(crate) struct Executor {
    clock: Arc<dyn HubClock>,
    wake: Waker,
    entries: Mutex<BTreeMap<(SessionId, String), Arc<Entry>>>,
    next_sweep: Mutex<Option<Instant>>,
}

impl Executor {
    pub(crate) fn new(clock: Arc<dyn HubClock>, wake: Waker) -> Self {
        Self {
            clock,
            wake,
            entries: Mutex::new(BTreeMap::new()),
            next_sweep: Mutex::new(None),
        }
    }

    /// Run `f` on `parent`'s core and reader for the online device `serial`, opening
    /// them if needed. `offline` when the device is not online (nothing is sent).
    pub(crate) fn run<T>(
        &self,
        hub: &HubHandle,
        parent: &Arc<Session>,
        serial: &str,
        f: impl FnOnce(&mut Cx<'_>) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let device = hub.device_by_serial(serial).ok_or_else(ApiError::offline)?;
        let entry = self.entry(hub, parent, serial, &device)?;
        let mut opened = guard(&entry.opened);
        if !entry.current(&device) {
            // Swept between the look-up and here: nothing was opened on it.
            drop(opened);
            return Err(ApiError::new(
                "busy",
                "the device's connection was just closed; try again",
            ));
        }
        if !entry.child.follow_parent() {
            return Err(ApiError::new("unauthenticated", "the session has ended"));
        }
        let mut cx = Cx {
            hub,
            entry: &entry,
            opened: &mut opened,
        };
        let out = f(&mut cx);
        *guard(&entry.used) = self.clock.now();
        out
    }

    /// The entry for (`parent`, `serial`), a new one if there is none for this
    /// attachment.
    fn entry(
        &self,
        hub: &HubHandle,
        parent: &Arc<Session>,
        serial: &str,
        device: &Arc<DeviceShared>,
    ) -> Result<Arc<Entry>, ApiError> {
        let key = (parent.id(), serial.to_owned());
        let stale = {
            let mut entries = guard(&self.entries);
            match entries.get(&key) {
                Some(e) if e.current(device) => return Ok(Arc::clone(e)),
                Some(_) => entries.remove(&key),
                None => None,
            }
        };
        if let Some(stale) = stale {
            close(hub, &stale);
        }
        if guard(&self.entries).len() >= MAX_CORES {
            return Err(ApiError::new(
                "busy",
                format!(
                    "the hub has {MAX_CORES} device connections open for the API; try again later"
                ),
            ));
        }
        let child = hub
            .open_child(parent)
            .map_err(|_| ApiError::internal("the hub is stopping"))?;
        let vid = hub
            .devices()
            .iter()
            .find(|d| d.identity.serial == serial && d.state == DeviceState::Online)
            .map_or(0, |d| d.identity.vid);
        let entry = Arc::new(Entry {
            child,
            serial: serial.to_owned(),
            vid,
            device: Arc::clone(device),
            opened: Mutex::new(Opened::default()),
            used: Mutex::new(self.clock.now()),
            closed: AtomicBool::new(false),
        });
        let raced = {
            let mut entries = guard(&self.entries);
            match entries.get(&key) {
                Some(e) if e.current(device) => Some(Arc::clone(e)),
                _ => {
                    entries.insert(key, Arc::clone(&entry));
                    None
                }
            }
        };
        if let Some(winner) = raced {
            close(hub, &entry);
            return Ok(winner);
        }
        self.schedule_sweep();
        Ok(entry)
    }

    fn schedule_sweep(&self) {
        let mut next = guard(&self.next_sweep);
        if next.is_none() {
            let now = self.clock.now();
            *next = Some(now.checked_add(SWEEP).unwrap_or(now));
            drop(next);
            self.wake.wake();
        }
    }

    /// When the sweep is due, while any entry is open.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        *guard(&self.next_sweep)
    }

    /// Close what is idle, orphaned or bound to a device that went. Runs on the hub's
    /// timer thread: an entry in use is left for the next sweep.
    pub(crate) fn sweep(&self, hub: &HubHandle, now: Instant) {
        let due: Vec<Arc<Entry>> = {
            let mut entries = guard(&self.entries);
            let mut due = Vec::new();
            entries.retain(|_, e| {
                let idle = now.saturating_duration_since(*guard(&e.used)) >= IDLE_CLOSE;
                let gone = e.child.parent_gone()
                    || hub
                        .device_by_serial(&e.serial)
                        .is_none_or(|d| !Arc::ptr_eq(&d, &e.device));
                let unused = guard_try(&e.opened).is_some();
                if (idle || gone) && unused {
                    due.push(Arc::clone(e));
                    false
                } else {
                    true
                }
            });
            let mut next = guard(&self.next_sweep);
            *next = (!entries.is_empty()).then(|| now.checked_add(SWEEP).unwrap_or(now));
            due
        };
        for e in &due {
            close(hub, e);
        }
    }

    /// Close every entry (the hub is stopping; its sessions go with it).
    pub(crate) fn close_all(&self) {
        let all: Vec<Arc<Entry>> = std::mem::take(&mut *guard(&self.entries))
            .into_values()
            .collect();
        *guard(&self.next_sweep) = None;
        for e in &all {
            e.closed.store(true, Ordering::SeqCst);
            disconnect(e);
        }
    }
}

/// `try_lock`, a poisoned lock taken as it is.
fn guard_try<T>(m: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match m.try_lock() {
        Ok(g) => Some(g),
        Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

fn disconnect(e: &Entry) {
    let opened = std::mem::take(&mut *guard(&e.opened));
    if let Some(core) = opened.core {
        core.disconnect();
    }
    drop(opened.reader);
}

/// Close an entry: its core, its reader, then its child session.
fn close(hub: &HubHandle, e: &Entry) {
    e.closed.store(true, Ordering::SeqCst);
    disconnect(e);
    hub.close_session(e.child.id());
}
