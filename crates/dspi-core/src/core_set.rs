//! A set of cores, one per open device (WP MD0, docs: MPLAN section 1.2).
//!
//! A [`DspiCore`] holds one device. An app that keeps several devices open at
//! once (the Modern layout's device tabs, MCP addressing any open device)
//! holds one core per device in a [`CoreSet`], each keyed by a stable
//! [`DeviceId`]:
//!
//! - `usb:<serial>`, `hub:<hub-id>/<serial>`, `link:<host:port>/<serial>` (a
//!   hub that names no id), `demo:<n>` and `file:<n>`.
//! - **One live session per serial.** A unit open over USB is refused over a
//!   hub, and the other way round ([`CoreSetError::AlreadyOpen`]): two writers
//!   on one unit are never allowed. The set's cores refuse it too: a member's
//!   [`DspiCore::select_device`] of a serial another member holds fails.
//! - **One scanner.** Cores in a set do not scan: their own
//!   [`DspiCore::poll_hotplug`] does nothing, and [`CoreSet::poll_hotplug`]
//!   enumerates the bus once and hands that list to every core, so each
//!   closes its own departed unit and reopens its own returning one. A
//!   member's [`DspiCore::scan_devices`] lists the units its set holds without
//!   opening them.
//! - **Shared configuration** ([`CoreSetConfig`]): the limits, snapshot,
//!   measurement and event-log folders and the hub memory's folder, applied to
//!   every core the set opens. Cores naming the same hub-memory folder share
//!   one memory (`link/hub_memory.rs` keeps one cache per file for the
//!   process), so a hub learnt by one core is known to all.
//! - [`CoreSet::open`], [`CoreSet::close`], [`CoreSet::list`] and events
//!   ([`CoreSetListener`]): opened, closed, lost, reconnecting, restored, and
//!   the bus's arrivals and departures.
//!
//! Each core keeps its own mirror, undo journal, Link session and jobs, so
//! switching the active device loses nothing. There is no idle close; the cap
//! is [`CORE_SET_CAP`] devices.
//!
//! Thread rules (PLAN §1.2): the set's lock is never held while a core is
//! called or a listener runs; a core's `Inner` lock is never held while the
//! set's reservations are taken. Opening blocks for the connect sequence: not
//! on a UI thread. Native only: the web keeps one Worker per session instead.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::hotplug::{Membership, Reserved};
use crate::records::DeviceInfo;
use crate::{DeviceBroker, DspiCore, DspiError, Slot};

/// The most devices a set holds at once.
pub const CORE_SET_CAP: usize = 8;

/// How long a hub open waits for the hub to list the unit after the session
/// is up (the hub lists its devices once the session is authenticated).
#[cfg(feature = "link")]
const HUB_LIST_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// A stable name for an open device: `<kind>:<rest>`, see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    /// A unit on USB.
    pub fn usb(serial: &str) -> Self {
        Self(format!("usb:{serial}"))
    }

    /// A unit behind the hub `hub_id`.
    pub fn hub(hub_id: &str, serial: &str) -> Self {
        Self(format!("hub:{hub_id}/{serial}"))
    }

    /// A unit behind a hub that names no id, at `addr` (`host:port`).
    pub fn link(addr: &str, serial: &str) -> Self {
        Self(format!("link:{addr}/{serial}"))
    }

    /// The `n`th demo device opened.
    pub fn demo(n: u32) -> Self {
        Self(format!("demo:{n}"))
    }

    /// The `n`th document opened.
    pub fn file(n: u32) -> Self {
        Self(format!("file:{n}"))
    }

    /// An id as text, as [`DeviceId::as_str`] gives it. `None` for text that
    /// is no id (an unknown kind, or no serial where one belongs).
    pub fn parse(text: &str) -> Option<Self> {
        let id = Self(text.to_owned());
        let (kind, rest) = text.split_once(':')?;
        let ok = match kind {
            "usb" => !rest.is_empty(),
            "hub" | "link" => rest
                .split_once('/')
                .is_some_and(|(a, s)| !a.is_empty() && !s.is_empty()),
            "demo" | "file" => rest.parse::<u32>().is_ok(),
            _ => false,
        };
        ok.then_some(id)
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The way the device is reached.
    pub fn transport(&self) -> DeviceTransport {
        match self.0.split_once(':').map_or("", |(k, _)| k) {
            "usb" => DeviceTransport::Usb,
            "hub" => DeviceTransport::Hub,
            "link" => DeviceTransport::Link,
            "file" => DeviceTransport::File,
            _ => DeviceTransport::Demo,
        }
    }

    /// The unit's serial for the kinds that name a real unit (USB, hub,
    /// link); `None` for a demo or a document, which any number may open.
    pub fn serial(&self) -> Option<&str> {
        let (kind, rest) = self.0.split_once(':')?;
        match kind {
            "usb" => Some(rest),
            "hub" | "link" => rest.split_once('/').map(|(_, s)| s),
            _ => None,
        }
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How an open device is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceTransport {
    /// USB, directly.
    Usb,
    /// A DSPi Link hub.
    Hub,
    /// A DSPi Link hub that names no id.
    Link,
    /// The built-in demo device.
    Demo,
    /// A document, with no device.
    File,
}

/// Whether an open device can be used now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceLiveness {
    /// Connected.
    Live,
    /// Its Link session is reconnecting by itself.
    Reconnecting,
    /// Gone (unplugged, or its session ended). A USB unit comes back by
    /// itself when it is plugged in again; it stays in the set until closed.
    Lost,
}

/// One open device, as [`CoreSet::list`] gives it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDevice {
    /// Its id.
    #[serde(serialize_with = "id_text")]
    pub id: DeviceId,
    /// What to call it: the unit's own name, else its serial; "Demo n"; a
    /// document's file name.
    pub label: String,
    /// How it is reached.
    pub transport: DeviceTransport,
    /// The unit's serial; the fake's for a demo or a document.
    pub serial: String,
    /// Whether it can be used now.
    pub state: DeviceLiveness,
}

fn id_text<S: serde::Serializer>(id: &DeviceId, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(id.as_str())
}

/// What to open.
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceTarget {
    /// A new demo device.
    Demo,
    /// The USB unit with this serial.
    Usb {
        /// Its serial.
        serial: String,
    },
    /// A `.dspipreset` document, with no device ([`DspiCore::open_document`]).
    Document {
        /// The document's text.
        text: String,
        /// What the client calls it.
        file_name: String,
    },
    /// A unit behind a hub, on its own Link session.
    #[cfg(feature = "link")]
    Hub(HubTarget),
}

/// A unit behind a hub, dialled with a credential the app already holds, as
/// the several-units measuring does: one session per unit.
#[cfg(feature = "link")]
#[derive(Debug, Clone, PartialEq)]
pub struct HubTarget {
    /// The hub's address, as for [`DspiCore::link_connect_advertised`].
    pub url: String,
    /// The hub's id ("" for a hub that names none: the id is then `link:`).
    pub hub_id: String,
    /// The unit's serial.
    pub serial: String,
    /// This console's name, for the hub's session list.
    pub client_name: String,
    /// This app's version.
    pub app_version: String,
    /// The stored credential for the hub ("" for none).
    pub credential: String,
    /// The hub's advert said TLS.
    pub tls_advertised: bool,
}

#[cfg(feature = "link")]
impl HubTarget {
    fn id(&self) -> DeviceId {
        if self.hub_id.is_empty() {
            DeviceId::link(host_port(&self.url), &self.serial)
        } else {
            DeviceId::hub(&self.hub_id, &self.serial)
        }
    }
}

/// `host:port` of a `ws://host:port/path` address.
#[cfg(feature = "link")]
fn host_port(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// Why a [`CoreSet`] call failed.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreSetError {
    /// This unit is open already, as `id`.
    AlreadyOpen {
        /// The unit's serial.
        serial: String,
        /// The id it is open as.
        id: DeviceId,
    },
    /// The set holds [`CORE_SET_CAP`] devices.
    Full,
    /// No open device has this id.
    Unknown {
        /// The id asked for.
        id: String,
    },
    /// Opening the device failed; nothing was added.
    Device(DspiError),
}

impl fmt::Display for CoreSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOpen { serial, id } => write!(f, "{serial} is already open as {id}"),
            Self::Full => write!(f, "{CORE_SET_CAP} devices are open; close one first"),
            Self::Unknown { id } => write!(f, "no open device is {id}"),
            Self::Device(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CoreSetError {}

impl From<DspiError> for CoreSetError {
    fn from(e: DspiError) -> Self {
        Self::Device(e)
    }
}

/// What happened, for [`CoreSetListener::on_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CoreSetEventKind {
    /// A device was added to the set.
    Opened,
    /// A device was closed and left the set.
    Closed,
    /// An open device went away; it stays in the set.
    Lost,
    /// An open device's Link session is reconnecting.
    Reconnecting,
    /// A lost or reconnecting device is connected again.
    Restored,
    /// A DSPi appeared on the bus (no id: it is not open).
    Arrived,
    /// A DSPi left the bus (no id).
    Departed,
}

/// One event of a [`CoreSet`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreSetEvent {
    /// What happened.
    pub kind: CoreSetEventKind,
    /// The open device it concerns; "" for the bus's arrivals and departures.
    pub id: String,
    /// The unit's serial.
    pub serial: String,
}

/// Hears a [`CoreSet`]'s events. Called with no lock of the set held, on
/// the thread of the call that caused them (`open`, `close`,
/// `poll_hotplug`); it may call back into the set and its cores, but must
/// not set the listener from inside the callback.
pub trait CoreSetListener: Send + Sync {
    /// Something happened.
    fn on_event(&self, event: CoreSetEvent);
}

/// The configuration every core of a set gets. An empty folder is not set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreSetConfig {
    /// [`DspiCore::limits_set_dir`].
    pub limits_dir: String,
    /// [`DspiCore::snapshot_set_dir`].
    pub snapshot_dir: String,
    /// [`DspiCore::measure_set_dir`].
    pub measure_dir: String,
    /// [`DspiCore::event_log_set_dir`].
    pub event_log_dir: String,
    /// [`DspiCore::link_set_dir`]: the hub memory.
    pub link_dir: String,
}

impl CoreSetConfig {
    /// Apply it to `core`: every folder is tried, the first failure returned.
    fn apply(&self, core: &DspiCore) -> crate::Result<()> {
        let mut first = Ok(());
        let mut keep = |r: crate::Result<()>| {
            if first.is_ok() {
                first = r;
            }
        };
        if !self.limits_dir.is_empty() {
            keep(core.limits_set_dir(self.limits_dir.clone()));
        }
        if !self.snapshot_dir.is_empty() {
            keep(core.snapshot_set_dir(self.snapshot_dir.clone()));
        }
        if !self.measure_dir.is_empty() {
            keep(core.measure_set_dir(self.measure_dir.clone()));
        }
        if !self.event_log_dir.is_empty() {
            keep(core.event_log_set_dir(self.event_log_dir.clone()));
        }
        #[cfg(feature = "link")]
        if !self.link_dir.is_empty() {
            keep(core.link_set_dir(self.link_dir.clone()));
        }
        first
    }
}

/// One open device.
struct Entry {
    id: DeviceId,
    core: Arc<DspiCore>,
    label: String,
    serial: String,
    state: DeviceLiveness,
    /// Its token in the reservations.
    token: u64,
}

impl Entry {
    fn view(&self) -> OpenDevice {
        OpenDevice {
            id: self.id.clone(),
            label: self.label.clone(),
            transport: self.id.transport(),
            serial: self.serial.clone(),
            state: self.state,
        }
    }
}

/// Behind the set's lock.
#[derive(Default)]
struct SetState {
    entries: Vec<Entry>,
    /// Opens running now, which count against the cap.
    pending: usize,
    next_token: u64,
    next_demo: u32,
    next_file: u32,
    config: CoreSetConfig,
    broker: Option<Arc<dyn DeviceBroker>>,
    /// The bus at the last poll, sorted; `None` before the first, which then
    /// diffs against the picker's last scan.
    last_bus: Option<Vec<String>>,
    /// The picker's last scan.
    devices: Vec<DeviceInfo>,
    inaccessible: u32,
}

/// The open devices of an app, one core each. Create one per app with
/// [`CoreSet::new`].
pub struct CoreSet {
    state: Mutex<SetState>,
    /// The serials held, shared with the cores (`hotplug::Membership`).
    reserved: Reserved,
    listener: Slot<dyn CoreSetListener>,
}

impl Default for CoreSet {
    fn default() -> Self {
        Self {
            state: Mutex::new(SetState::default()),
            reserved: Arc::default(),
            listener: Slot::default(),
        }
    }
}

impl CoreSet {
    /// An empty set. No I/O.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, SetState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reserved(&self) -> MutexGuard<'_, BTreeMap<String, (u64, String)>> {
        self.reserved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register (`Some`) or clear (`None`) the listener. Clearing waits for a
    /// call in progress on another thread.
    pub fn set_listener(&self, listener: Option<Arc<dyn CoreSetListener>>) {
        self.listener.set(listener);
    }

    fn emit(&self, events: Vec<CoreSetEvent>) {
        for event in events {
            self.listener.call(|l| l.on_event(event));
        }
    }

    /// Set the folders every core gets, now and from now on. Every open core
    /// is tried; the first failure is returned (a core whose installer PIN is
    /// set refuses a new limits folder while locked, as
    /// [`DspiCore::limits_set_dir`] does). Blocking file I/O.
    pub fn set_config(&self, config: CoreSetConfig) -> crate::Result<()> {
        let cores = {
            let mut s = self.lock();
            s.config = config.clone();
            s.entries
                .iter()
                .map(|e| Arc::clone(&e.core))
                .collect::<Vec<_>>()
        };
        let mut first = Ok(());
        for core in cores {
            let r = config.apply(&core);
            if first.is_ok() {
                first = r;
            }
        }
        first
    }

    /// Hand USB to `broker` (`Some`, a Link hub this process runs) or take
    /// it back (`None`) for every core, as [`DspiCore::set_device_broker`]
    /// does for one, and for the set's own scans. The ids of the devices
    /// whose connection could not stay are returned: they stay in the set,
    /// lost, for the caller to select again through their core once the new
    /// owner has the unit.
    pub fn set_device_broker(&self, broker: Option<Arc<dyn DeviceBroker>>) -> Vec<DeviceId> {
        let cores = {
            let mut s = self.lock();
            s.broker = broker.clone();
            s.entries
                .iter()
                .map(|e| (e.id.clone(), Arc::clone(&e.core)))
                .collect::<Vec<_>>()
        };
        cores
            .into_iter()
            .filter_map(|(id, core)| core.set_device_broker(broker.clone()).map(|_| id))
            .collect()
    }

    /// Open `target` on a core of its own and add it: its id, or why not.
    /// Blocks for the connect sequence (and, for a hub, the dial): not on a
    /// UI thread. On a failure nothing is added and the new core is closed.
    pub fn open(&self, target: DeviceTarget) -> std::result::Result<DeviceId, CoreSetError> {
        match target {
            DeviceTarget::Demo => {
                let id = DeviceId::demo(self.take_number(false));
                self.open_with(id, String::new(), |c| c.open_demo())
            }
            DeviceTarget::Usb { serial } => {
                self.open_with(DeviceId::usb(&serial), String::new(), |c| {
                    c.select_device(serial.clone())
                })
            }
            DeviceTarget::Document { text, file_name } => {
                let id = DeviceId::file(self.take_number(true));
                let label = file_name.clone();
                self.open_with(id, label, |c| c.open_document(text, file_name).map(drop))
            }
            #[cfg(feature = "link")]
            DeviceTarget::Hub(hub) => {
                self.open_with(hub.id(), String::new(), |c| open_hub(c, &hub))
            }
        }
    }

    /// The next demo or document number, from 1.
    fn take_number(&self, file: bool) -> u32 {
        let mut s = self.lock();
        let n = if file {
            &mut s.next_file
        } else {
            &mut s.next_demo
        };
        *n = n.saturating_add(1);
        *n
    }

    /// Open a device the embedder dials itself (an interactive pairing, a
    /// transport of its own) as `id`: `open` runs on a fresh core of this set,
    /// configured and with the serial of `id` reserved for it, and the device
    /// is added when it returns `Ok` with a device connected. `label` ""
    /// takes the unit's name. On a failure nothing is added and the core is
    /// closed.
    pub fn open_with(
        &self,
        id: DeviceId,
        label: String,
        open: impl FnOnce(&DspiCore) -> crate::Result<()>,
    ) -> std::result::Result<DeviceId, CoreSetError> {
        let serial = id.serial().map(str::to_owned);
        let (token, config, broker) = {
            let mut s = self.lock();
            if s.entries.iter().any(|e| e.id == id) {
                return Err(CoreSetError::AlreadyOpen {
                    serial: serial.unwrap_or_default(),
                    id,
                });
            }
            if s.entries.len().saturating_add(s.pending) >= CORE_SET_CAP {
                return Err(CoreSetError::Full);
            }
            let token = s.next_token;
            s.next_token = token.wrapping_add(1);
            if let Some(serial) = &serial {
                let mut reserved = self.reserved();
                if let Some((_, held)) = reserved.get(serial) {
                    return Err(CoreSetError::AlreadyOpen {
                        serial: serial.clone(),
                        id: DeviceId::parse(held).unwrap_or_else(|| id.clone()),
                    });
                }
                reserved.insert(serial.clone(), (token, id.as_str().to_owned()));
            }
            s.pending = s.pending.saturating_add(1);
            (token, s.config.clone(), s.broker.clone())
        };

        let core = DspiCore::new();
        core.lock().hotplug.member = Some(Membership {
            token,
            reserved: Arc::clone(&self.reserved),
        });
        if broker.is_some() {
            core.set_device_broker(broker);
        }
        let opened = config
            .apply(&core)
            .and_then(|()| open(&core))
            .and_then(|()| {
                if core.is_connected() {
                    Ok(())
                } else {
                    Err(DspiError::Disconnected)
                }
            });
        if let Err(e) = opened {
            self.release(token);
            {
                let mut s = self.lock();
                s.pending = s.pending.saturating_sub(1);
            }
            shut_core(&core);
            return Err(e.into());
        }

        let serial_now = core
            .lock()
            .conn
            .as_ref()
            .map(|c| c.serial.clone())
            .unwrap_or_default();
        let label = if label.is_empty() {
            label_of(&core, &id, &serial_now)
        } else {
            label
        };
        let entry = Entry {
            id: id.clone(),
            core,
            label,
            serial: serial_now.clone(),
            state: DeviceLiveness::Live,
            token,
        };
        {
            let mut s = self.lock();
            s.pending = s.pending.saturating_sub(1);
            s.entries.push(entry);
        }
        self.emit(vec![CoreSetEvent {
            kind: CoreSetEventKind::Opened,
            id: id.as_str().to_owned(),
            serial: serial_now,
        }]);
        Ok(id)
    }

    /// Give back every serial `token` holds.
    fn release(&self, token: u64) {
        self.reserved().retain(|_, (t, _)| *t != token);
    }

    /// Close the device `id` and drop it from the set: its Link session ends
    /// and its connection closes; the core handed out by [`CoreSet::core`] is
    /// left with nothing open. Blocks while closing.
    pub fn close(&self, id: &DeviceId) -> std::result::Result<(), CoreSetError> {
        let entry = {
            let mut s = self.lock();
            let at = s.entries.iter().position(|e| e.id == *id).ok_or_else(|| {
                CoreSetError::Unknown {
                    id: id.as_str().to_owned(),
                }
            })?;
            s.entries.remove(at)
        };
        self.release(entry.token);
        entry.core.lock().hotplug.member = None;
        shut_core(&entry.core);
        self.emit(vec![CoreSetEvent {
            kind: CoreSetEventKind::Closed,
            id: entry.id.as_str().to_owned(),
            serial: entry.serial,
        }]);
        Ok(())
    }

    /// Close every device, as [`CoreSet::close`] does for one.
    pub fn close_all(&self) {
        let ids: Vec<DeviceId> = self.lock().entries.iter().map(|e| e.id.clone()).collect();
        for id in ids {
            let _ = self.close(&id);
        }
    }

    /// The open devices, in the order they were opened.
    pub fn list(&self) -> Vec<OpenDevice> {
        self.lock().entries.iter().map(Entry::view).collect()
    }

    /// The core of the open device `id`, which every `DspiCore` call works
    /// on. It stays usable after [`CoreSet::close`], with nothing open.
    pub fn core(&self, id: &DeviceId) -> Option<Arc<DspiCore>> {
        self.lock()
            .entries
            .iter()
            .find(|e| e.id == *id)
            .map(|e| Arc::clone(&e.core))
    }

    /// The open device whose unit has this serial.
    pub fn find_serial(&self, serial: &str) -> Option<DeviceId> {
        let held = self.reserved().get(serial).map(|(_, id)| id.clone())?;
        DeviceId::parse(&held)
    }

    /// The picker's scan: the DSPis on USB (or the broker's), in serial
    /// order, the ones the set holds listed without being opened again.
    /// Updates [`CoreSet::inaccessible_devices`].
    pub fn scan_devices(&self) -> Vec<DeviceInfo> {
        let broker = self.lock().broker.clone();
        let (devices, inaccessible) = match broker {
            Some(b) => (b.devices(), b.inaccessible()),
            None => {
                let ours: Vec<String> = self.reserved().keys().cloned().collect();
                crate::device::usb_scan(&ours)
            }
        };
        let mut s = self.lock();
        s.devices = devices.clone();
        s.inaccessible = inaccessible;
        devices
    }

    /// DSPis the last [`CoreSet::scan_devices`] saw but could not open.
    pub fn inaccessible_devices(&self) -> u32 {
        self.lock().inaccessible
    }

    /// One hotplug poll for every core, with one enumeration of the bus:
    /// each core closes its own departed unit and reopens its own returning
    /// one ([`DspiCore::poll_hotplug`]'s steps), then the listener hears the
    /// bus's arrivals and departures and each device that was lost,
    /// started reconnecting or came back. About every 500 ms, off the UI
    /// thread; a reopen blocks it for the connect sequence.
    pub fn poll_hotplug(&self) {
        let broker = self.lock().broker.clone();
        self.poll_with(
            || match broker {
                Some(b) => b.devices(),
                None => crate::device::usb_scan_quiet(),
            },
            |core, serial| core.select_device(serial.to_owned()),
        );
    }

    /// [`CoreSet::poll_hotplug`] with the enumeration and the reopen
    /// supplied (tests use a fake bus).
    pub(crate) fn poll_with(
        &self,
        scan: impl FnOnce() -> Vec<DeviceInfo>,
        reopen: impl Fn(&DspiCore, &str) -> crate::Result<()>,
    ) {
        let list = scan();
        let mut now: Vec<String> = list.iter().map(|d| d.serial.clone()).collect();
        now.sort();
        now.dedup();

        let (cores, mut events) = {
            let mut s = self.lock();
            let before = s
                .last_bus
                .take()
                .unwrap_or_else(|| s.devices.iter().map(|d| d.serial.clone()).collect());
            let bus = |kind, serial: &String| CoreSetEvent {
                kind,
                id: String::new(),
                serial: serial.clone(),
            };
            let mut events: Vec<CoreSetEvent> = before
                .iter()
                .filter(|x| !now.contains(x))
                .map(|x| bus(CoreSetEventKind::Departed, x))
                .collect();
            events.extend(
                now.iter()
                    .filter(|x| !before.contains(x))
                    .map(|x| bus(CoreSetEventKind::Arrived, x)),
            );
            s.last_bus = Some(now);
            let cores: Vec<(DeviceId, Arc<DspiCore>)> = s
                .entries
                .iter()
                .map(|e| (e.id.clone(), Arc::clone(&e.core)))
                .collect();
            (cores, events)
        };

        // Every core diffs the one list against its own last; only the
        // core that selected a serial reopens it.
        let mut states = Vec::with_capacity(cores.len());
        for (id, core) in cores {
            core.poll_with(|| list.clone(), |serial| reopen(&core, serial));
            states.push((id, liveness(&core)));
        }

        {
            let mut s = self.lock();
            for (id, state) in states {
                let Some(e) = s.entries.iter_mut().find(|e| e.id == id) else {
                    continue; // closed meanwhile
                };
                if e.state == state {
                    continue;
                }
                let kind = match state {
                    DeviceLiveness::Live => CoreSetEventKind::Restored,
                    DeviceLiveness::Reconnecting => CoreSetEventKind::Reconnecting,
                    DeviceLiveness::Lost => CoreSetEventKind::Lost,
                };
                e.state = state;
                events.push(CoreSetEvent {
                    kind,
                    id: id.as_str().to_owned(),
                    serial: e.serial.clone(),
                });
            }
        }
        self.emit(events);
    }
}

impl Drop for CoreSet {
    fn drop(&mut self) {
        self.close_all();
    }
}

/// Whether `core`'s device can be used now.
fn liveness(core: &DspiCore) -> DeviceLiveness {
    #[cfg(feature = "link")]
    if core.lock().link.is_some() && core.link_status().phase == crate::LinkPhase::Reconnecting {
        return DeviceLiveness::Reconnecting;
    }
    if core.is_connected() {
        DeviceLiveness::Live
    } else {
        DeviceLiveness::Lost
    }
}

/// The name for a device opened without one.
fn label_of(core: &DspiCore, id: &DeviceId, serial: &str) -> String {
    if id.transport() == DeviceTransport::Demo {
        let n = id.as_str().split_once(':').map_or("", |(_, n)| n);
        return format!("Demo {n}");
    }
    match core.get_device_name() {
        Ok(name) if !name.trim().is_empty() => name,
        _ => serial.to_owned(),
    }
}

/// End `core`'s Link session and close its device.
fn shut_core(core: &DspiCore) {
    #[cfg(feature = "link")]
    core.link_close();
    core.disconnect();
}

/// Dial the hub, wait for it to list the unit, bind it and read it.
#[cfg(feature = "link")]
fn open_hub(core: &DspiCore, hub: &HubTarget) -> crate::Result<()> {
    let hello = core.link_connect_advertised(
        hub.url.clone(),
        hub.client_name.clone(),
        hub.app_version.clone(),
        hub.credential.clone(),
        hub.hub_id.clone(),
        hub.tls_advertised,
    )?;
    if hello.needs_pairing {
        return Err(DspiError::Link {
            code: "needs_pairing".into(),
            msg: "the hub asks this console to pair again".into(),
        });
    }
    let start = dspi_transport::clock::Instant::now();
    let handle = loop {
        if let Some(d) = core
            .link_devices()
            .into_iter()
            .find(|d| d.serial == hub.serial)
        {
            break d.handle;
        }
        if start.elapsed() >= HUB_LIST_WAIT {
            return Err(DspiError::Disconnected);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    core.link_select_device(handle, hub.serial.clone())?;
    core.fetch_all()
}

#[cfg(test)]
#[path = "core_set/tests_wp_md0.rs"]
mod tests_wp_md0;
