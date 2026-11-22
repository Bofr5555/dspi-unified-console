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
//!   measurement, event-log and speaker-layout folders and the hub memory's
//!   folder, applied to
//!   every core the set opens. Cores naming the same hub-memory folder share
//!   one memory (`link/hub_memory.rs` keeps one cache per file for the
//!   process), so a hub learnt by one core is known to all.
//! - **The main core** (WP MDQT1, [`CoreSet::main_core`]): the one core that
//!   lives as long as the set, with a device or without. An app opens its
//!   first device on it and, where choosing a device replaces the open one
//!   (the Classic layout), every next one too, through the core's own opens.
//!   It is a member like the others; with no device open it is not listed.
//! - **An id follows the device.** A member core opened on another device
//!   (the user replaced it) is listed under that device's id from then on, and
//!   its old serial is given back; a core claims a serial before it opens it
//!   ([`DspiCore::select_device`], [`DspiCore::link_select_device`]). So an id
//!   never means another device than it says: [`CoreSet::core`] checks.
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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use dspi_transport::TransportKind;

use crate::hotplug::{Membership, Reserved};
use crate::records::DeviceInfo;
use crate::{DeviceBroker, DspiCore, DspiError, Slot};

/// The most devices a set holds at once.
pub const CORE_SET_CAP: usize = 8;

/// How long a hub open waits for the hub to list the unit after the session
/// is up (the hub lists its devices once the session is authenticated).
#[cfg(feature = "link")]
const HUB_LIST_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long [`CoreSet::list`] waits for a core that is busy.
const LIST_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

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
    /// is no id (an unknown kind, no serial where one belongs, or a `/` in a
    /// hub's id: where the serial starts would be a guess).
    pub fn parse(text: &str) -> Option<Self> {
        let id = Self(text.to_owned());
        let (kind, rest) = text.split_once(':')?;
        let ok = match kind {
            "usb" => !rest.is_empty(),
            "hub" | "link" => rest
                .split_once('/')
                .is_some_and(|(a, s)| !a.is_empty() && !s.is_empty() && !s.contains('/')),
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
    // -- WP INT5 --
    /// Opened with [`CoreSet::open_extra`]: the app's own business, not a
    /// device the user has open.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub extra: bool,
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

/// The id of the unit `serial` on the hub `hub_id` at `url`. A hub that names
/// no id, or (-- WP MDQT1 --, the id is the operator's text) one with a `/`,
/// goes by its address.
#[cfg(feature = "link")]
fn hub_unit_id(hub_id: &str, url: &str, serial: &str) -> DeviceId {
    if hub_id.is_empty() || hub_id.contains('/') {
        DeviceId::link(host_port(url), serial)
    } else {
        DeviceId::hub(hub_id, serial)
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
    /// [`DspiCore::speaker_layout_set_dir`]. -- WP INT1 -- (ROLE0 x MD0)
    pub speaker_layout_dir: String,
}

impl CoreSetConfig {
    /// Apply it to `core`: every folder is tried, the first failure returned.
    /// -- WP MDQT1 -- [`CoreSet::set_config`] reports that failure; an open
    /// logs it and goes on, as a core on its own does: a data folder that
    /// cannot be used must not keep a device from opening.
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
        // -- WP INT1 -- (ROLE0 x MD0)
        if !self.speaker_layout_dir.is_empty() {
            keep(core.speaker_layout_set_dir(self.speaker_layout_dir.clone()));
        }
        first
    }
}

// -- WP MDQT1 --
/// What a core has open, as far as its id goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    Nothing,
    /// A unit on USB (open, or gone and to be reopened).
    Usb(String),
    /// A unit bound through a hub.
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    Hub {
        hub_id: String,
        url: String,
        serial: String,
    },
    Demo(String),
    /// A document, by its connection: another one is another device.
    File {
        generation: u64,
        name: String,
        serial: String,
    },
}

impl Held {
    /// Read what `core` has open. Takes its lock; no I/O.
    fn of(core: &DspiCore) -> Self {
        Self::read(&core.lock())
    }

    fn read(inner: &crate::Inner) -> Self {
        if let Some(c) = &inner.conn {
            let serial = c.serial.clone();
            if c.vid != 0 || matches!(c.caps.kind, TransportKind::Usb | TransportKind::Midi) {
                return Self::Usb(serial);
            }
            if let Some(d) = &c.document {
                return Self::File {
                    generation: c.generation,
                    name: d.file_name.clone(),
                    serial,
                };
            }
            if c.caps.kind != TransportKind::Link {
                return Self::Demo(serial);
            }
        }
        #[cfg(feature = "link")]
        if let Some((hub_id, url, serial)) = inner.link.as_ref().and_then(|s| s.bound_unit()) {
            return Self::Hub {
                hub_id: hub_id.to_owned(),
                url: url.to_owned(),
                serial: serial.to_owned(),
            };
        }
        match &inner.selected {
            Some(s) if inner.conn.is_none() => Self::Usb(s.clone()),
            _ => Self::Nothing,
        }
    }

    /// The same device, as far as its id goes.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            // A hub that is being dialled again has not named itself yet
            (
                Self::Hub { hub_id, serial, .. },
                Self::Hub {
                    hub_id: h,
                    serial: s,
                    ..
                },
            ) => serial == s && (hub_id == h || hub_id.is_empty() || h.is_empty()),
            (Self::Demo(_), Self::Demo(_)) => true,
            (a, b) => a == b,
        }
    }

    fn serial(&self) -> &str {
        match self {
            Self::Nothing => "",
            Self::Usb(s)
            | Self::Demo(s)
            | Self::Hub { serial: s, .. }
            | Self::File { serial: s, .. } => s,
        }
    }

    /// The serial it reserves: a real unit's.
    fn reserves(&self) -> Option<&str> {
        match self {
            Self::Usb(s) | Self::Hub { serial: s, .. } => Some(s),
            _ => None,
        }
    }
}

/// One open device, or the main core with none.
struct Entry {
    /// `None` only for the main core with no device.
    id: Option<DeviceId>,
    core: Arc<DspiCore>,
    label: String,
    serial: String,
    state: DeviceLiveness,
    /// Its token in the reservations.
    token: u64,
    // -- WP MDQT1 --
    /// What it held when `id` was given.
    held: Held,
    main: bool,
    // -- WP INT5 --
    extra: bool,
}

impl Entry {
    fn is(&self, id: &DeviceId) -> bool {
        self.id.as_ref() == Some(id)
    }

    fn view(&self) -> Option<OpenDevice> {
        let id = self.id.clone()?;
        Some(OpenDevice {
            transport: id.transport(),
            id,
            label: self.label.clone(),
            serial: self.serial.clone(),
            state: self.state,
            extra: self.extra,
        })
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

impl SetState {
    // -- WP MDQT1 --
    fn next_number(&mut self, file: bool) -> u32 {
        let n = if file {
            &mut self.next_file
        } else {
            &mut self.next_demo
        };
        *n = n.saturating_add(1);
        *n
    }

    /// The id for what a core was found to hold, and the label to go with it;
    /// `None` for nothing.
    fn name(&mut self, held: &Held, name: &str) -> Option<(DeviceId, String)> {
        let named = |serial: &str| {
            let text = if name.trim().is_empty() { serial } else { name };
            text.to_owned()
        };
        match held {
            Held::Nothing => None,
            Held::Usb(s) => Some((DeviceId::usb(s), named(s))),
            #[cfg(feature = "link")]
            Held::Hub {
                hub_id,
                url,
                serial,
            } => Some((hub_unit_id(hub_id, url, serial), named(serial))),
            #[cfg(not(feature = "link"))]
            Held::Hub { .. } => None,
            Held::Demo(_) => {
                let n = self.next_number(false);
                Some((DeviceId::demo(n), format!("Demo {n}")))
            }
            Held::File { name, .. } => Some((DeviceId::file(self.next_number(true)), name.clone())),
        }
    }
}

/// The open devices of an app, one core each. Create one per app with
/// [`CoreSet::new`].
pub struct CoreSet {
    state: Mutex<SetState>,
    /// The serials held, shared with the cores (`hotplug::Membership`).
    reserved: Reserved,
    listener: Slot<dyn CoreSetListener>,
    // -- WP MDQT1 --
    /// Held over one [`CoreSet::sync`], so what one read is not undone by an
    /// older reading. Never taken with a core's lock or the set's held.
    syncing: Mutex<()>,
    me: Weak<CoreSet>,
}

// -- WP MDQT1 --
/// An open in progress: its slot and its serial are given back, and its core
/// shut, when it goes without having become an entry (a failure, or a panic in
/// the embedder's closure).
struct Opening<'a> {
    set: &'a CoreSet,
    token: u64,
    core: Option<Arc<DspiCore>>,
    /// It is an entry now: nothing to give back.
    entered: bool,
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        if self.entered {
            return;
        }
        // Shut first, then give the serial back: until the device is closed
        // another open of it would fail on the busy interface.
        if let Some(core) = &self.core {
            shut_core(core);
            core.lock().hotplug.member = None;
        }
        self.set.release(self.token);
        let mut s = self.set.lock();
        s.pending = s.pending.saturating_sub(1);
    }
}

impl CoreSet {
    /// An empty set. No I/O.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            state: Mutex::new(SetState::default()),
            reserved: Arc::default(),
            listener: Slot::default(),
            syncing: Mutex::default(),
            me: Weak::clone(me),
        })
    }

    // -- WP MDQT1 --
    /// The core that lives as long as the set, with a device or without (the
    /// module docs): made on the first call, configured as every other, never
    /// removed. [`CoreSet::close`] of its device leaves it with none.
    pub fn main_core(&self) -> Arc<DspiCore> {
        let (core, config, broker) = {
            let mut s = self.lock();
            if let Some(e) = s.entries.iter().find(|e| e.main) {
                return Arc::clone(&e.core);
            }
            let token = s.next_token;
            s.next_token = token.wrapping_add(1);
            let core = DspiCore::new();
            s.entries.insert(
                0,
                Entry {
                    id: None,
                    core: Arc::clone(&core),
                    label: String::new(),
                    serial: String::new(),
                    state: DeviceLiveness::Lost,
                    token,
                    held: Held::Nothing,
                    main: true,
                    extra: false,
                },
            );
            core.lock().hotplug.member = Some(self.membership(token));
            (core, s.config.clone(), s.broker.clone())
        };
        if broker.is_some() {
            core.take_device_broker(broker);
        }
        if let Err(e) = config.apply(&core) {
            log::warn!("the main core was not fully configured: {e}");
        }
        core
    }

    /// The place of the core with `token` in this set.
    fn membership(&self, token: u64) -> Membership {
        Membership {
            token,
            reserved: Arc::clone(&self.reserved),
            set: Weak::clone(&self.me),
        }
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
    /// Call it before opening devices: a speaker layout or a limits profile
    /// is the device's own only once its folder is named.
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
    /// lost, and come back on the next polls through the new owner.
    pub fn set_device_broker(&self, broker: Option<Arc<dyn DeviceBroker>>) -> Vec<DeviceId> {
        let before: Vec<DeviceId> = self.list().into_iter().map(|d| d.id).collect();
        self.route_usb(broker, None);
        let lost = self.list();
        before
            .into_iter()
            .filter(|id| {
                lost.iter()
                    .any(|d| d.id == *id && d.state == DeviceLiveness::Lost)
            })
            .collect()
    }

    // -- WP MDQT1 --
    /// Every core's USB through `broker`. The connections that could not stay
    /// are closed; those of the cores other than `caller` are armed to reopen
    /// on the next polls. `caller`'s serial is returned: it reopens its own.
    pub(crate) fn route_usb(
        &self,
        broker: Option<Arc<dyn DeviceBroker>>,
        caller: Option<&DspiCore>,
    ) -> Option<String> {
        let cores = {
            let mut s = self.lock();
            s.broker = broker.clone();
            s.entries
                .iter()
                .map(|e| Arc::clone(&e.core))
                .collect::<Vec<_>>()
        };
        let mut own = None;
        for core in cores {
            let closed = core.take_device_broker(broker.clone());
            if caller.is_some_and(|c| std::ptr::eq(c, &raw const *core)) {
                own = closed;
            } else if let Some(serial) = closed {
                core.arm_reopen(serial);
            }
        }
        own
    }

    /// Open `target` on a core of its own and add it: its id, or why not.
    /// Blocks for the connect sequence (and, for a hub, the dial): not on a
    /// UI thread. On a failure nothing is added and the new core is closed.
    pub fn open(&self, target: DeviceTarget) -> std::result::Result<DeviceId, CoreSetError> {
        self.open_target(target, false)
    }

    // -- WP INT5 --
    /// [`CoreSet::open`] for a device the app uses itself without showing it
    /// (a unit measured with others): listed as `extra`, and not offered to
    /// AI agents.
    pub fn open_extra(&self, target: DeviceTarget) -> std::result::Result<DeviceId, CoreSetError> {
        self.open_target(target, true)
    }

    fn open_target(
        &self,
        target: DeviceTarget,
        extra: bool,
    ) -> std::result::Result<DeviceId, CoreSetError> {
        match target {
            DeviceTarget::Demo => {
                let id = DeviceId::demo(self.take_number(false));
                self.open_entry(id, String::new(), extra, |c| c.open_demo())
            }
            DeviceTarget::Usb { serial } => {
                self.open_entry(DeviceId::usb(&serial), String::new(), extra, |c| {
                    c.select_device(serial.clone())
                })
            }
            DeviceTarget::Document { text, file_name } => {
                let id = DeviceId::file(self.take_number(true));
                let label = file_name.clone();
                self.open_entry(id, label, extra, |c| {
                    c.open_document(text, file_name).map(drop)
                })
            }
            #[cfg(feature = "link")]
            DeviceTarget::Hub(hub) => {
                if hub.hub_id.contains('/') {
                    return Err(CoreSetError::Device(DspiError::InvalidArgument {
                        msg: "a hub id with a / names no device".into(),
                    }));
                }
                let id = hub_unit_id(&hub.hub_id, &hub.url, &hub.serial);
                self.open_entry(id, String::new(), extra, |c| open_hub(c, &hub))
            }
        }
    }

    /// The next demo or document number, from 1.
    fn take_number(&self, file: bool) -> u32 {
        self.lock().next_number(file)
    }

    /// Open a device the embedder dials itself (an interactive pairing, a
    /// transport of its own) as `id`: `open` runs on a fresh core of this set,
    /// configured and with the serial of `id` reserved for it, and the device
    /// is added when it returns `Ok` with a device connected. `label` ""
    /// takes the unit's name. On a failure nothing is added and the core is
    /// closed; so it is when `open` connected another unit than `id` names
    /// (-- WP MDQT1 --, review RA: the reservation would be another's).
    pub fn open_with(
        &self,
        id: DeviceId,
        label: String,
        open: impl FnOnce(&DspiCore) -> crate::Result<()>,
    ) -> std::result::Result<DeviceId, CoreSetError> {
        self.open_entry(id, label, false, open)
    }

    fn open_entry(
        &self,
        id: DeviceId,
        label: String,
        extra: bool,
        open: impl FnOnce(&DspiCore) -> crate::Result<()>,
    ) -> std::result::Result<DeviceId, CoreSetError> {
        // -- WP MDQT1 -- `hub:a/b/S` from a hub id with a `/`: whose serial?
        if DeviceId::parse(id.as_str()).is_none() {
            return Err(CoreSetError::Device(DspiError::InvalidArgument {
                msg: format!("{id} is no device id"),
            }));
        }
        // What the cores hold now, so a serial given up is free
        self.refresh();
        let serial = id.serial().map(str::to_owned);
        let (token, config, broker, bus) = {
            let mut s = self.lock();
            if s.entries.iter().any(|e| e.is(&id)) {
                return Err(CoreSetError::AlreadyOpen {
                    serial: serial.unwrap_or_default(),
                    id,
                });
            }
            let open = s.entries.iter().filter(|e| e.id.is_some()).count();
            if open.saturating_add(s.pending) >= CORE_SET_CAP {
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
            (
                token,
                s.config.clone(),
                s.broker.clone(),
                s.last_bus.clone(),
            )
        };

        let mut opening = Opening {
            set: self,
            token,
            core: None,
            entered: false,
        };
        let core = DspiCore::new();
        opening.core = Some(Arc::clone(&core));
        core.lock().hotplug.member = Some(self.membership(token));
        core.seed_bus(bus);
        if broker.is_some() {
            core.take_device_broker(broker);
        }
        if let Err(e) = config.apply(&core) {
            log::warn!("{id}: not fully configured: {e}");
        }
        open(&core)?;
        let serial_now = match &core.lock().conn {
            Some(c) => c.serial.clone(),
            None => return Err(DspiError::Disconnected.into()),
        };
        if serial.as_ref().is_some_and(|s| *s != serial_now) {
            return Err(CoreSetError::Device(DspiError::InvalidArgument {
                msg: format!("{id} was asked for, {serial_now} was opened"),
            }));
        }
        let label = if label.is_empty() {
            label_of(&core, &id, &serial_now)
        } else {
            label
        };
        let held = Held::of(&core);
        let entry = Entry {
            id: Some(id.clone()),
            core,
            label,
            serial: serial_now.clone(),
            state: DeviceLiveness::Live,
            token,
            held,
            main: false,
            extra,
        };
        {
            let mut s = self.lock();
            s.pending = s.pending.saturating_sub(1);
            s.entries.push(entry);
            opening.entered = true;
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
    /// left with nothing open. The main core stays in the set. Blocks while
    /// closing.
    pub fn close(&self, id: &DeviceId) -> std::result::Result<(), CoreSetError> {
        self.refresh(); // -- WP MDQT1 -- the id may be another device's by now
        let (entry, main) =
            {
                let mut s = self.lock();
                let at = s.entries.iter().position(|e| e.is(id)).ok_or_else(|| {
                    CoreSetError::Unknown {
                        id: id.as_str().to_owned(),
                    }
                })?;
                match s.entries.get(at) {
                    Some(e) if e.main => (None, Some(Arc::clone(&e.core))),
                    _ => (Some(s.entries.remove(at)), None),
                }
            };
        if let Some(core) = main {
            shut_core(&core);
            self.refresh(); // the closed event, and the serial given back
            return Ok(());
        }
        let Some(entry) = entry else { return Ok(()) };
        // Shut first, then give the serial back (review RA F3).
        shut_core(&entry.core);
        entry.core.lock().hotplug.member = None;
        self.release(entry.token);
        self.emit(vec![CoreSetEvent {
            kind: CoreSetEventKind::Closed,
            id: id.as_str().to_owned(),
            serial: entry.serial,
        }]);
        Ok(())
    }

    /// Close every device, as [`CoreSet::close`] does for one.
    pub fn close_all(&self) {
        let ids: Vec<DeviceId> = self
            .lock()
            .entries
            .iter()
            .filter_map(|e| e.id.clone())
            .collect();
        for id in ids {
            let _ = self.close(&id);
        }
    }

    /// The open devices, the main core's first, then in the order they were
    /// opened. A core that stays inside a device call is listed as it was last
    /// seen, so this waits a tenth of a second on each at most.
    pub fn list(&self) -> Vec<OpenDevice> {
        self.emit(self.sync(false));
        self.lock().entries.iter().filter_map(Entry::view).collect()
    }

    /// The core of the open device `id`, which every `DspiCore` call works
    /// on. It stays usable after [`CoreSet::close`], with nothing open.
    /// -- WP MDQT1 -- `None` too when the core has another device open by now
    /// (the user replaced it), whose own id gives it: waits for a device call
    /// in progress on it. Whoever keeps the core across such a moment compares
    /// `connection_generation` to know it is still that device.
    pub fn core(&self, id: &DeviceId) -> Option<Arc<DspiCore>> {
        let (core, held) = self
            .lock()
            .entries
            .iter()
            .find(|e| e.is(id))
            .map(|e| (Arc::clone(&e.core), e.held.clone()))?;
        let now = Held::of(&core);
        if now == Held::Nothing || now.same(&held) {
            return Some(core);
        }
        self.refresh();
        self.lock()
            .entries
            .iter()
            .find(|e| e.is(id))
            .map(|e| Arc::clone(&e.core))
    }

    // -- WP MDQT1 --
    /// The open device `core` (one of the set's) has, as last seen.
    pub fn device_of(&self, core: &Arc<DspiCore>) -> Option<OpenDevice> {
        self.lock()
            .entries
            .iter()
            .find(|e| Arc::ptr_eq(&e.core, core))
            .and_then(Entry::view)
    }

    /// Take in what the cores hold now (their own opens and closes) and tell
    /// the listener. Waits for device calls in progress.
    pub fn refresh(&self) {
        self.emit(self.sync(true));
    }

    /// [`CoreSet::refresh`]'s events, to be emitted with no lock held.
    /// `wait` false passes over a core that stays inside a device call.
    fn sync(&self, wait: bool) -> Vec<CoreSetEvent> {
        let _one = self.syncing.lock().unwrap_or_else(PoisonError::into_inner);
        let cores: Vec<(u64, Arc<DspiCore>)> = self
            .lock()
            .entries
            .iter()
            .map(|e| (e.token, Arc::clone(&e.core)))
            .collect();
        let mut seen = Vec::with_capacity(cores.len());
        for (token, core) in cores {
            let Some(inner) = (if wait {
                Some(core.lock())
            } else {
                lock_briefly(&core)
            }) else {
                continue;
            };
            let held = Held::read(&inner);
            let name = inner
                .conn
                .as_ref()
                .map(|c| c.device_name.clone())
                .unwrap_or_default();
            let state = liveness(&inner);
            drop(inner);
            seen.push((token, held, name, state));
        }

        let mut events = Vec::new();
        let mut s = self.lock();
        for (token, held, name, state) in seen {
            let Some(at) = s.entries.iter().position(|e| e.token == token) else {
                continue; // closed meanwhile
            };
            let Some(e) = s.entries.get(at) else { continue };
            let nothing = held == Held::Nothing;
            let moved = !held.same(&e.held);
            let named = if moved && (!nothing || e.main) {
                Some(s.name(&held, &name))
            } else {
                None
            };
            let Some(e) = s.entries.get_mut(at) else {
                continue;
            };
            if let Some(named) = named {
                if let Some(old) = e.id.take() {
                    events.push(CoreSetEvent {
                        kind: CoreSetEventKind::Closed,
                        id: old.as_str().to_owned(),
                        serial: e.serial.clone(),
                    });
                }
                let before = std::mem::replace(&mut e.held, held);
                e.serial = e.held.serial().to_owned();
                e.state = state;
                if let Some((id, label)) = named {
                    events.push(CoreSetEvent {
                        kind: CoreSetEventKind::Opened,
                        id: id.as_str().to_owned(),
                        serial: e.serial.clone(),
                    });
                    e.id = Some(id);
                    e.label = label;
                } else {
                    e.label.clear();
                }
                // Only what it held: a serial it has claimed since is kept.
                let id = e.id.as_ref().map(|i| i.as_str().to_owned());
                let now = e.held.reserves().map(str::to_owned);
                let mut reserved = self.reserved();
                if let Some(old) = before.reserves().filter(|o| Some(*o) != now.as_deref()) {
                    reserved.retain(|x, (t, _)| *t != token || x != old);
                }
                if let (Some(serial), Some(id)) = (now, id) {
                    match reserved.get(&serial) {
                        Some((t, _)) if *t != token => {
                            log::warn!("{serial} is open twice: {id} is not its first holder");
                        }
                        _ => {
                            reserved.insert(serial, (token, id));
                        }
                    }
                }
                continue;
            }
            let Some(id) = e.id.clone().filter(|_| e.state != state) else {
                continue;
            };
            e.state = state;
            events.push(CoreSetEvent {
                kind: match state {
                    DeviceLiveness::Live => CoreSetEventKind::Restored,
                    DeviceLiveness::Reconnecting => CoreSetEventKind::Reconnecting,
                    DeviceLiveness::Lost => CoreSetEventKind::Lost,
                },
                id: id.as_str().to_owned(),
                serial: e.serial.clone(),
            });
        }
        events
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
            let cores: Vec<Arc<DspiCore>> = s.entries.iter().map(|e| Arc::clone(&e.core)).collect();
            (cores, events)
        };

        // Every core diffs the one list against its own last; only the
        // core that selected a serial reopens it.
        for core in cores {
            core.poll_with(|| list.clone(), |serial| reopen(&core, serial));
        }
        events.extend(self.sync(true));
        self.emit(events);
    }
}

impl Drop for CoreSet {
    fn drop(&mut self) {
        self.close_all();
        // -- WP MDQT1 -- the main core, with what it has not been seen to open
        for e in std::mem::take(&mut self.lock().entries) {
            shut_core(&e.core);
            e.core.lock().hotplug.member = None;
        }
    }
}

// -- WP MDQT1 --
/// `core`'s lock, unless something holds it for longer than a moment (a device
/// call; a Link session's own looks are far shorter than this).
fn lock_briefly(core: &DspiCore) -> Option<MutexGuard<'_, crate::Inner>> {
    let start = dspi_transport::clock::Instant::now();
    loop {
        if let Some(inner) = core.try_lock() {
            return Some(inner);
        }
        if start.elapsed() >= LIST_WAIT {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Whether the device behind `inner` can be used now.
fn liveness(inner: &crate::Inner) -> DeviceLiveness {
    #[cfg(feature = "link")]
    if let Some(s) = &inner.link
        && s.reconnecting(inner.conn.as_ref().map(|c| c.generation))
    {
        return DeviceLiveness::Reconnecting;
    }
    if inner.conn.is_some() {
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

/// End `core`'s Link session and close its device; it keeps no selection,
/// so no later poll reopens the unit on it.
fn shut_core(core: &DspiCore) {
    core.forget_selected();
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

#[cfg(test)]
#[path = "core_set/tests_wp_mdqt1.rs"]
mod tests_wp_mdqt1;
