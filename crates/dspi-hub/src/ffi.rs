//! Being a DSPi Link hub, for Swift (WP SW-M3): the UniFFI wrapper over this crate,
//! feature `uniffi`, namespace `dspi_hub`. The Mac app hosts its hub with it, as the Qt
//! app does through the C ABI (`dspi-capi/src/fns_hub.rs`, HB6), whose rules this
//! follows one for one. `docs/core-merge/HUB.md` is the design.
//!
//! The library that carries it is `crates/dspi-ffi` with its feature `hub`, which only
//! the macOS slices build (`apps/apple/build-xcframework.sh`): no hub on iOS or in
//! Android's Play build (PLAN-CLIENTS section 8).
//!
//! **One object, many runs.** [`HubHost`] is the app's way to a hub and outlives each
//! one: a hub exists exactly while sharing is on, between [`HubHost::start`] and
//! [`HubHost::stop`]. Its listener carries over.
//!
//! **Settings** are what a start is given. A running hub keeps them: to apply a new
//! name, port or access mode, stop and start, which disconnects the clients (they
//! reconnect by themselves, except after a change of access mode). **Changing the access
//! mode revokes every token** (spec 6.2): [`hub_saved_settings`] says how many clients
//! it would cost, for a warning; [`HubStarted::tokens_revoked`] says how many it did.
//!
//! **The hub's own rules** (pairing, roles, limits, the installer lock, the guard and its
//! backstop, the core-voltage rule) are the hub's, unchanged: nothing here decides any.
//!
//! **Secrets.** A client's token never reaches this API: the hub keeps only its hash.
//! The one secret here is the PIN in [`HubPairing`], to show on screen. Nothing here
//! logs either.
//!
//! **Threads.** Everything may be called from any thread. `start`, `stop`, `attach_app`
//! and `attach_demo` block, on the bus, the devices and the peers, for seconds: keep them
//! off the UI thread. They run one at a time. The rest are quick, though `revoke` and
//! `set_role` write a file. The listener is called on the hub's event thread: it may
//! read (`status`, `devices`, `clients`); anything else is refused there
//! ([`HubHostError::InCallback`], or false), since it would wait for that very thread.
//!
//! **Who owns the hub.** Only [`HubHost`]'s `running` slot holds a [`Hub`]. Everything
//! else, a read and the listener included, holds a [`HubHandle`] or a
//! [`NetworkControl`], so the event thread can never hold the last owner (HUB.md
//! section 3).

use std::cell::Cell;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dspi_core::DspiCore;
use dspi_transport::{FakeDevice, FakeRemote};

use crate::app::AppLink;
use crate::{
    AccessMode, DeviceState, Hub, HubConfig, HubError, HubEvent, HubHandle, HubListener,
    NetworkConfig, NetworkControl, PairedClient, RegisteredDevice, Role, SavedHub,
};

/// What a hub is called when nobody has named it.
const DEFAULT_HUB_NAME: &str = "DSPi Hub";
/// Longest hub name kept, in characters: it goes to every client and into their lists.
const MAX_HUB_NAME: usize = 64;
/// `PARAM_SRC_GPIO`: a change made at the device itself (a knob).
#[allow(clippy::cast_possible_truncation)]
const SOURCE_AT_DEVICE: u8 = dspi_proto::generated::notify::PARAM_SRC_GPIO as u8;

/// Why a hub call did not happen. Each says what to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum HubHostError {
    /// An argument was not usable.
    #[error("{msg}")]
    Invalid {
        /// What was wrong.
        msg: String,
    },
    /// A hub is already running: stop it first.
    #[error("the hub is already running")]
    Running,
    /// No hub is running.
    #[error("the hub is not running")]
    NotRunning,
    /// Called from the listener, where it would wait for itself.
    #[error("not possible from a hub callback")]
    InCallback,
    /// This hub lets anyone in, so there is nothing to pair.
    #[error("this hub lets anyone in, so there is nothing to pair")]
    OpenAccess,
    /// The hub has stopped.
    #[error("{msg}")]
    Stopped {
        /// The hub's words.
        msg: String,
    },
    /// No such device.
    #[error("{msg}")]
    NoDevice {
        /// The hub's words.
        msg: String,
    },
    /// That device is attached already.
    #[error("{msg}")]
    AlreadyAttached {
        /// The hub's words.
        msg: String,
    },
    /// No handle is free.
    #[error("{msg}")]
    NoHandle {
        /// The hub's words.
        msg: String,
    },
    /// The device's identity could not be read.
    #[error("{msg}")]
    Identity {
        /// The hub's words.
        msg: String,
    },
    /// An app is already on this hub.
    #[error("{msg}")]
    AppAttached {
        /// The hub's words.
        msg: String,
    },
    /// The system said no: a port in use, a store that cannot be written, no random
    /// numbers.
    #[error("{msg}")]
    Io {
        /// The hub's words.
        msg: String,
    },
}

impl From<HubError> for HubHostError {
    fn from(e: HubError) -> Self {
        let msg = e.to_string();
        match e {
            HubError::Stopped => Self::Stopped { msg },
            HubError::NoDevice => Self::NoDevice { msg },
            HubError::AlreadyAttached(_) => Self::AlreadyAttached { msg },
            HubError::NoHandle => Self::NoHandle { msg },
            HubError::Identity(_) => Self::Identity { msg },
            HubError::AppAttached => Self::AppAttached { msg },
            HubError::Io(_) => Self::Io { msg },
        }
    }
}

/// How a new client gets in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HubAccess {
    /// A new client pairs with a PIN, then presents a token. The default to offer.
    Pin,
    /// Every connection is an admin. Only when the user asks for it.
    Open,
}

impl From<HubAccess> for AccessMode {
    fn from(a: HubAccess) -> Self {
        match a {
            HubAccess::Pin => AccessMode::Pin,
            HubAccess::Open => AccessMode::Open,
        }
    }
}

impl From<AccessMode> for HubAccess {
    fn from(a: AccessMode) -> Self {
        match a {
            AccessMode::Pin => HubAccess::Pin,
            AccessMode::Open => HubAccess::Open,
        }
    }
}

/// A paired client's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HubClientRole {
    /// Watches only.
    Viewer,
    /// Adjusts devices.
    Control,
    /// Also changes the hub's settings.
    Admin,
}

impl From<Role> for HubClientRole {
    fn from(r: Role) -> Self {
        match r {
            Role::Viewer => HubClientRole::Viewer,
            Role::Control => HubClientRole::Control,
            Role::Admin => HubClientRole::Admin,
        }
    }
}

impl From<HubClientRole> for Role {
    fn from(r: HubClientRole) -> Self {
        match r {
            HubClientRole::Viewer => Role::Viewer,
            HubClientRole::Control => Role::Control,
            HubClientRole::Admin => Role::Admin,
        }
    }
}

/// What a start is given.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubStartOptions {
    /// "" takes the stored name, or a default; another is cleaned and stored.
    pub name: String,
    /// The hub's own directory, created if need be: its id, clients and device names.
    /// Two hubs must not share one.
    pub store_dir: String,
    /// 0 lets the system choose; [`HubStarted::port`] says which.
    pub port: u16,
    /// How a new client gets in.
    pub access: HubAccess,
    /// Find and claim USB devices. Without it the hub serves what `attach_demo` gives it.
    pub usb: bool,
    /// An app holds or will hold devices, so nothing is scanned or claimed until
    /// `attach_app`. Always set it when an app will attach.
    pub app: bool,
    /// Listen on loopback only. For tests.
    pub loopback: bool,
    /// Serve the web pages (the hub's page, the landing page) on the same port.
    pub web: bool,
    /// This machine's mDNS `.local` name, which then counts as the hub itself for
    /// the web pages' `Origin` check. `None`: addresses only.
    pub mdns_name: Option<String>,
}

/// What a start did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubStarted {
    /// The hub's id, as clients see it (`hid`).
    pub hub_id: String,
    /// The name it runs under.
    pub name: String,
    /// The port it listens on.
    pub port: u16,
    /// How a new client gets in.
    pub access: HubAccess,
    /// Paired clients this start forgot (a change of access mode).
    pub tokens_revoked: u32,
}

/// Where the hub stands.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubHostStatus {
    /// A hub is running. With false the rest is empty.
    pub running: bool,
    /// Its id.
    pub hub_id: String,
    /// Its name.
    pub name: String,
    /// Its port.
    pub port: u16,
    /// How a new client gets in.
    pub access: HubAccess,
    /// Sockets, signed in or not.
    pub connections: u32,
    /// Devices on the bus that could not be opened at the last scan: another program,
    /// or a permission.
    pub inaccessible: u32,
    /// What this run's start cost.
    pub tokens_revoked: u32,
    /// A pairing window is open.
    pub pairing_open: bool,
    /// How long it has left.
    pub pairing_expires_in_ms: u64,
}

impl HubHostStatus {
    fn stopped() -> Self {
        Self {
            running: false,
            hub_id: String::new(),
            name: String::new(),
            port: 0,
            access: HubAccess::Pin,
            connections: 0,
            inaccessible: 0,
            tokens_revoked: 0,
            pairing_open: false,
            pairing_expires_in_ms: 0,
        }
    }
}

/// A device the hub serves, online or in its grace.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubSharedDevice {
    /// Its handle on the hub.
    pub handle: u8,
    /// Its serial.
    pub serial: String,
    /// The name clients show.
    pub name: String,
    /// "1.1.7", without the beta.
    pub firmware: String,
    /// The beta number, if the device says.
    pub firmware_beta: Option<u8>,
    /// 0 RP2040, 1 RP2350, 2 STM32.
    pub platform: u8,
    /// Output channels.
    pub outputs: u8,
    /// Online; false while it is in its grace.
    pub online: bool,
    /// The session holding its lock.
    pub locked_by: Option<u16>,
}

/// A paired client.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubClient {
    /// Its id.
    pub cid: u32,
    /// The name it gave.
    pub name: String,
    /// Its role.
    pub role: HubClientRole,
    /// When it paired, seconds since the epoch.
    pub created: u64,
    /// When it last signed in, seconds since the epoch.
    pub last_seen: Option<u64>,
    /// It has a session now (false with no hub running).
    pub online: bool,
}

/// What a hub's store holds, with or without a hub on it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubSavedSettings {
    /// A usable store is there. A store that cannot be parsed reads as not there: the
    /// next start sets it aside.
    pub exists: bool,
    /// Its hub id; `None` when it does not exist.
    pub hub_id: Option<String>,
    /// Its name; `None` if never named.
    pub name: Option<String>,
    /// The access mode it last ran in; `None` before the first start.
    pub access: Option<HubAccess>,
    /// The port a hub takes by default.
    pub default_port: u16,
    /// How many paired clients a start with PIN access would forget. Warn when not 0:
    /// it cannot be undone.
    pub tokens_revoked_if_pin: u32,
    /// The same for open access.
    pub tokens_revoked_if_open: u32,
    /// The paired clients (`online` false).
    pub clients: Vec<HubClient>,
}

/// An open pairing window.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubPairing {
    /// The PIN: show it to the user and to nobody else.
    pub pin: String,
    /// How long it works.
    pub expires_in_ms: u64,
}

/// By hand: only the screen it is meant for may show a PIN.
impl std::fmt::Debug for HubPairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubPairing")
            .field("expires_in_ms", &self.expires_in_ms)
            .finish_non_exhaustive()
    }
}

/// Something that happened on the hub.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HubHostEvent {
    /// A device came online.
    DeviceOnline {
        /// Its handle.
        handle: u8,
        /// Its serial.
        serial: String,
    },
    /// A device went offline, keeping its handle for a while.
    DeviceOffline {
        /// Its handle.
        handle: u8,
        /// Its serial.
        serial: String,
    },
    /// A device's handle was given up.
    DeviceRemoved {
        /// Its handle.
        handle: u8,
        /// Its serial.
        serial: String,
    },
    /// A client's session opened, which is also how a finished pairing shows: read the
    /// clients and the status again.
    SessionOpened {
        /// The session.
        id: u16,
    },
    /// A client's session closed.
    SessionClosed {
        /// The session.
        id: u16,
    },
    /// The hub has stopped: the last event of a run.
    Stopped,
    // -- review batch6 (SECSRV glue) --
    /// The hub's TLS key or certificate changed while it ran (`HubEvent::Tls`): public
    /// data only, as the C ABI's `HUB_EVENT_TLS_KEY`.
    TlsKey {
        /// What happened.
        change: HubTlsKeyChange,
        /// The key's pin, as clients show it.
        pin: String,
    },
}

// -- review batch6 (SECSRV glue) --
/// What happened to the hub's TLS key ([`HubHostEvent::TlsKey`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HubTlsKeyChange {
    /// A first key, on a hub that had none.
    Created,
    /// The key could not be read and a new one was made: clients see "key changed".
    Replaced,
    /// The certificate was issued again for the same key: no client notices.
    CertificateIssued,
    /// A rotation made a new key.
    Rotated,
}

impl From<HubEvent> for HubHostEvent {
    fn from(e: HubEvent) -> Self {
        match e {
            HubEvent::DeviceOnline { handle, serial } => Self::DeviceOnline { handle, serial },
            HubEvent::DeviceOffline { handle, serial } => Self::DeviceOffline { handle, serial },
            HubEvent::DeviceRemoved { handle, serial } => Self::DeviceRemoved { handle, serial },
            HubEvent::SessionOpened { id } => Self::SessionOpened { id },
            HubEvent::SessionClosed { id } => Self::SessionClosed { id },
            HubEvent::Stopped => Self::Stopped,
            // -- review batch6 (SECSRV glue) --
            HubEvent::Tls(event) => {
                use crate::tls_identity::TlsIdentityEvent as T;
                let (change, pin) = match event {
                    T::Created { pin } => (HubTlsKeyChange::Created, pin),
                    T::KeyReplaced { pin, .. } => (HubTlsKeyChange::Replaced, pin),
                    T::CertificateIssued { pin, .. } => (HubTlsKeyChange::CertificateIssued, pin),
                    T::Rotated { pin, .. } => (HubTlsKeyChange::Rotated, pin),
                };
                Self::TlsKey {
                    change,
                    pin: pin.display(),
                }
            }
        }
    }
}

/// The app's ear on the hub. Called on the hub's event thread, one event at a time,
/// with no hub lock held: read if need be (`status`, `devices`, `clients`), post the
/// rest to the app's own thread, and return. Blocking here holds up every later event,
/// and `stop`.
#[uniffi::export(with_foreign)]
pub trait HubHostListener: Send + Sync {
    /// One event.
    fn on_hub_event(&self, event: HubHostEvent);
}

thread_local! {
    /// This thread is inside a listener call.
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// Clears [`IN_CALLBACK`], whatever the listener did.
struct LeaveCallback;

impl Drop for LeaveCallback {
    fn drop(&mut self) {
        IN_CALLBACK.with(|c| c.set(false));
    }
}

fn in_callback() -> bool {
    IN_CALLBACK.with(Cell::get)
}

/// The listener, carried from run to run, and held across a call so that replacing it
/// waits for the call. A hub's listener is this and nothing else, so no reference to a
/// hub ever sits on the event thread.
#[derive(Default)]
struct ListenerSlot(Mutex<Option<Arc<dyn HubHostListener>>>);

impl HubListener for ListenerSlot {
    fn on_event(&self, event: HubEvent) {
        let slot = lock(&self.0);
        if let Some(l) = slot.as_ref() {
            IN_CALLBACK.with(|c| c.set(true));
            let _leave = LeaveCallback;
            l.on_hub_event(event.into());
        }
    }
}

/// One run of a hub.
struct Running {
    /// The only owner of the hub (module docs).
    hub: Hub,
    handle: HubHandle,
    control: NetworkControl,
    /// When the pairing window opened last closes, for the status.
    pairing_expires: Option<Instant>,
    /// The app, when it is on this hub.
    app: Option<AppLink>,
    /// The demo device's other side, when one is attached.
    demo: Option<FakeRemote>,
}

/// What a read needs of a run: nothing that owns the hub.
struct View {
    handle: HubHandle,
    control: NetworkControl,
    pairing_expires: Option<Instant>,
}

/// The app's hub: one object, many runs (module docs).
#[derive(uniffi::Object)]
pub struct HubHost {
    /// Held across start, stop and attaches, so they run one at a time. Above
    /// `running`. Never taken on the event thread, as those calls are refused there.
    lifecycle: Mutex<()>,
    /// Taken briefly, and never held across a call into the hub: `Hub::stop` waits
    /// for the event thread, whose listener may be reading this.
    running: Mutex<Option<Running>>,
    listener: Arc<ListenerSlot>,
}

/// A hub keeps its data consistent before anything that could panic.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What may be called a hub: no control characters, a bounded length. Empty if
/// nothing is left, which means no name was given.
fn clean_hub_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_HUB_NAME)
        .collect();
    kept.trim().to_owned()
}

fn unix_seconds(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn millis_until(at: Instant, now: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(now).as_millis()).unwrap_or(u64::MAX)
}

fn client_record(c: &PairedClient, online: bool) -> HubClient {
    HubClient {
        cid: c.cid,
        name: c.name.clone(),
        role: c.role.into(),
        created: unix_seconds(c.created),
        last_seen: c.last_seen.map(unix_seconds),
        online,
    }
}

fn device_record(d: &RegisteredDevice) -> HubSharedDevice {
    HubSharedDevice {
        handle: d.handle,
        serial: d.identity.serial.clone(),
        name: d.name.clone(),
        firmware: d.identity.firmware.clone(),
        firmware_beta: d.identity.firmware_beta,
        platform: d.identity.platform,
        outputs: d.identity.outputs,
        online: d.state == DeviceState::Online,
        locked_by: d.locked_by,
    }
}

fn parse_access(text: &str) -> Option<HubAccess> {
    match text {
        "pin" => Some(HubAccess::Pin),
        "none" => Some(HubAccess::Open),
        _ => None,
    }
}

/// The options, checked: the store directory and the cleaned name.
fn checked(options: &HubStartOptions) -> Result<(PathBuf, String), HubHostError> {
    if options.store_dir.is_empty() {
        return Err(HubHostError::Invalid {
            msg: "the hub's store directory is empty".to_owned(),
        });
    }
    if options.usb && !cfg!(all(feature = "usb", not(target_os = "android"))) {
        return Err(HubHostError::Invalid {
            msg: "this build of the hub has no USB".to_owned(),
        });
    }
    Ok((
        PathBuf::from(&options.store_dir),
        clean_hub_name(&options.name),
    ))
}

/// The hub's configuration. It never scans at start: nothing is claimed until the port
/// is bound and the listener is in place ([`HubHost::start`]).
fn config_for(options: &HubStartOptions, store_dir: PathBuf, name: &str) -> HubConfig {
    let mut network = NetworkConfig::new(store_dir);
    network.port = options.port;
    network.access = options.access.into();
    network.web = options.web;
    network.mdns_name = options.mdns_name.clone().filter(|n| !n.is_empty());
    if options.loopback {
        network.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    }
    let mut config = HubConfig::in_process(name);
    config.network = Some(network);
    config.initial_scan = false;
    #[cfg(all(feature = "usb", not(target_os = "android")))]
    if options.usb {
        config.bus = Some(Box::new(crate::UsbBus::default()));
    }
    config
}

/// What a store holds: [`hub_saved_settings`].
fn saved_settings(store_dir: &str) -> Result<HubSavedSettings, HubHostError> {
    if store_dir.is_empty() {
        return Err(HubHostError::Invalid {
            msg: "the hub's store directory is empty".to_owned(),
        });
    }
    let saved = SavedHub::peek(Path::new(store_dir))?;
    let lost = |mode: AccessMode| {
        count(
            saved
                .as_ref()
                .map_or(0, |s| s.tokens_lost_starting_in(mode)),
        )
    };
    Ok(HubSavedSettings {
        exists: saved.is_some(),
        hub_id: saved.as_ref().map(|s| s.hub_id.clone()),
        name: saved.as_ref().and_then(|s| s.name.clone()),
        access: saved
            .as_ref()
            .and_then(|s| s.access.as_deref())
            .and_then(parse_access),
        default_port: crate::DEFAULT_PORT,
        tokens_revoked_if_pin: lost(AccessMode::Pin),
        tokens_revoked_if_open: lost(AccessMode::Open),
        clients: saved.as_ref().map_or_else(Vec::new, |s| {
            s.clients.iter().map(|c| client_record(c, false)).collect()
        }),
    })
}

/// What the store in `store_dir` holds, with or without a hub on it. Creates nothing
/// and changes nothing. Quick.
#[uniffi::export]
pub fn hub_saved_settings(store_dir: String) -> Result<HubSavedSettings, HubHostError> {
    saved_settings(&store_dir)
}

impl HubHost {
    fn view(&self) -> Option<View> {
        lock(&self.running).as_ref().map(|r| View {
            handle: r.handle.clone(),
            control: r.control.clone(),
            pairing_expires: r.pairing_expires,
        })
    }

    fn refuse_in_callback() -> Result<(), HubHostError> {
        if in_callback() {
            log::warn!("dspi-hub: a hub call from inside the hub's listener: refused");
            return Err(HubHostError::InCallback);
        }
        Ok(())
    }
}

#[uniffi::export]
impl HubHost {
    /// A hub's object, with no hub running.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            lifecycle: Mutex::new(()),
            running: Mutex::new(None),
            listener: Arc::new(ListenerSlot::default()),
        })
    }

    /// Set the listener, which stays for later runs; `None` clears it. Replacing or
    /// clearing waits for a call in progress on another thread. False from inside the
    /// listener, where it would wait for itself.
    pub fn set_listener(&self, listener: Option<Arc<dyn HubHostListener>>) -> bool {
        if Self::refuse_in_callback().is_err() {
            return false;
        }
        *lock(&self.listener.0) = listener;
        true
    }

    /// Start the hub: its threads and its socket, then, with `usb` and without `app`,
    /// the first bus scan. Blocks. If it fails, nothing is left running.
    ///
    /// **A change of access from the store's last run revokes every token.**
    /// [`HubStarted::tokens_revoked`] says how many; [`hub_saved_settings`] says so
    /// beforehand. [`HubHostError::Running`] if a hub already is. A port in use is
    /// [`HubHostError::Io`].
    pub fn start(&self, options: HubStartOptions) -> Result<HubStarted, HubHostError> {
        Self::refuse_in_callback()?;
        let (store_dir, cleaned) = checked(&options)?;
        let _one_at_a_time = lock(&self.lifecycle);
        if lock(&self.running).is_some() {
            return Err(HubHostError::Running);
        }
        let name = if cleaned.is_empty() {
            SavedHub::peek(&store_dir)?
                .and_then(|saved| saved.name)
                .unwrap_or_else(|| DEFAULT_HUB_NAME.to_owned())
        } else {
            cleaned.clone()
        };
        let hub = Hub::start(config_for(&options, store_dir, &name))?;
        let handle = hub.handle();
        // Always there on a networked hub that has just started.
        let control = hub.network().ok_or(HubError::Stopped)?;
        if !cleaned.is_empty() {
            control.auth().set_name(&name);
        }
        let tokens_revoked = count(control.auth().tokens_revoked_at_start());
        let listener: Arc<dyn HubListener> = Arc::clone(&self.listener) as _;
        handle.set_listener(Some(listener));
        let started = HubStarted {
            hub_id: handle.hub_id().to_owned(),
            name,
            port: control.port(),
            access: options.access,
            tokens_revoked,
        };
        *lock(&self.running) = Some(Running {
            hub,
            handle: handle.clone(),
            control,
            pairing_expires: None,
            app: None,
            demo: None,
        });
        if options.usb && !options.app {
            handle.scan_now();
        }
        Ok(started)
    }

    /// Stop the hub. Clients are told (close code 4003), queued commands are answered,
    /// every thread is joined and the devices are let go when this returns. An app on
    /// it gets its direct USB path back, and its device reopened. Blocks up to a few
    /// seconds. Nothing happens with no hub running; a concurrent caller waits for the
    /// first. False only from inside the listener.
    pub fn stop(&self) -> bool {
        if Self::refuse_in_callback().is_err() {
            return false;
        }
        let _one_at_a_time = lock(&self.lifecycle);
        // Out of the slot first: a stop waits for the event thread, and the listener
        // on it may be taking `running` for a read.
        let running = lock(&self.running).take();
        if let Some(Running { hub, app, .. }) = running {
            match app {
                Some(app) => app.stop_hub(&hub),
                None => hub.stop(),
            }
        }
        true
    }

    /// Put the app's core on the running hub, so that its USB devices come through it.
    /// The device it holds is closed, claimed by the hub and reopened, so the mirror is
    /// read afresh, and a flow in progress on it fails, as with an unplug; a running
    /// analyser must be started again. A demo, MIDI or Link connection stays as it is.
    /// Starts the bus's scans. Blocks. `stop` undoes it.
    pub fn attach_app(&self, core: Arc<DspiCore>) -> Result<(), HubHostError> {
        Self::refuse_in_callback()?;
        let _one_at_a_time = lock(&self.lifecycle);
        let view = self.view().ok_or(HubHostError::NotRunning)?;
        // Not under `running`: this waits for the bus.
        let link = AppLink::attach(&view.handle, core)?;
        if let Some(r) = lock(&self.running).as_mut() {
            r.app = Some(link);
        }
        Ok(())
    }

    /// Give the running hub a demo device to serve, for trying sharing with no
    /// hardware. Blocks for its identity reads. Returns its handle.
    pub fn attach_demo(&self) -> Result<u8, HubHostError> {
        Self::refuse_in_callback()?;
        let _one_at_a_time = lock(&self.lifecycle);
        let view = self.view().ok_or(HubHostError::NotRunning)?;
        let device = FakeDevice::demo();
        let remote = device.remote();
        let handle = view.handle.attach(Box::new(device), 0)?;
        if let Some(r) = lock(&self.running).as_mut() {
            r.demo = Some(remote);
        }
        Ok(handle)
    }

    /// Turn a knob on the demo device: a change made at the device itself, which every
    /// session hears with origin 0. For demos and conformance runs. False with no demo
    /// device.
    pub fn demo_turn_knob(&self) -> bool {
        let remote = lock(&self.running).as_ref().and_then(|r| r.demo.clone());
        let Some(remote) = remote else {
            return false;
        };
        let image = remote.image();
        let Some(at) = image.len().checked_sub(8) else {
            return false;
        };
        let Some(&byte) = image.get(at) else {
            return false;
        };
        remote.poke(at, &[byte ^ 0x5A], SOURCE_AT_DEVICE)
    }

    /// Where the hub stands. Quick, and allowed in the listener.
    pub fn status(&self) -> HubHostStatus {
        let Some(view) = self.view() else {
            return HubHostStatus::stopped();
        };
        let now = Instant::now();
        let auth = view.control.auth();
        let open = auth.pairing_open(now);
        HubHostStatus {
            running: true,
            hub_id: view.handle.hub_id().to_owned(),
            name: view.handle.name().to_owned(),
            port: view.control.port(),
            access: auth.mode().into(),
            connections: count(view.control.connections()),
            inaccessible: view.handle.inaccessible(),
            tokens_revoked: count(auth.tokens_revoked_at_start()),
            pairing_open: open,
            pairing_expires_in_ms: view
                .pairing_expires
                .filter(|_| open)
                .map_or(0, |e| millis_until(e, now)),
        }
    }

    /// The hub's devices, online and in their grace; empty with no hub running. Quick,
    /// and allowed in the listener.
    pub fn devices(&self) -> Vec<HubSharedDevice> {
        self.view().map_or_else(Vec::new, |v| {
            v.handle.devices().iter().map(device_record).collect()
        })
    }

    /// The paired clients. [`HubHostError::NotRunning`] with no hub:
    /// [`hub_saved_settings`] lists them then. An open hub lists none, as its clients
    /// do not pair. Quick, and allowed in the listener.
    pub fn clients(&self) -> Result<Vec<HubClient>, HubHostError> {
        let view = self.view().ok_or(HubHostError::NotRunning)?;
        Ok(view
            .control
            .auth()
            .clients()
            .iter()
            .map(|c| client_record(c, view.control.is_online(c.cid)))
            .collect())
    }

    /// Open the pairing window, replacing an open one, whose PIN then stops working.
    /// A client's right PIN ends the window ([`HubHostEvent::SessionOpened`]), and so
    /// do five wrong ones.
    pub fn begin_pairing(&self) -> Result<HubPairing, HubHostError> {
        Self::refuse_in_callback()?;
        let mut running = lock(&self.running);
        let r = running.as_mut().ok_or(HubHostError::NotRunning)?;
        let auth = r.control.auth();
        if auth.mode() != AccessMode::Pin {
            return Err(HubHostError::OpenAccess);
        }
        let now = Instant::now();
        let window = auth.begin_pairing(now)?;
        let pairing = HubPairing {
            pin: window.pin.clone(),
            expires_in_ms: millis_until(window.expires, now),
        };
        r.pairing_expires = Some(window.expires);
        Ok(pairing)
    }

    /// Close the pairing window, if one is open. False with no hub running, or inside
    /// the listener.
    pub fn cancel_pairing(&self) -> bool {
        if Self::refuse_in_callback().is_err() {
            return false;
        }
        self.view().is_some_and(|v| {
            v.control.auth().cancel_pairing();
            true
        })
    }

    /// Forget a paired client: its token stops working and its connections close at
    /// once with code 4002, which clients do not reconnect from. Writes the store.
    /// False with no hub running, no such client, or inside the listener.
    pub fn revoke(&self, cid: u32) -> bool {
        if Self::refuse_in_callback().is_err() {
            return false;
        }
        self.view().is_some_and(|v| v.control.revoke(cid))
    }

    /// Change a paired client's role: its next command is judged by the new one. Writes
    /// the store. False with no hub running, no such client, or inside the listener.
    pub fn set_role(&self, cid: u32, role: HubClientRole) -> bool {
        if Self::refuse_in_callback().is_err() {
            return false;
        }
        self.view()
            .is_some_and(|v| v.control.set_role(cid, role.into()))
    }
}

impl Drop for HubHost {
    fn drop(&mut self) {
        // The last reference: no listener call can be in progress on this thread,
        // since the listener holds no reference to this object.
        let running = lock(&self.running).take();
        if let Some(Running { hub, app, .. }) = running {
            match app {
                Some(app) => app.stop_hub(&hub),
                None => hub.stop(),
            }
        }
    }
}

#[cfg(test)]
#[path = "ffi/tests_wp_swm3.rs"]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_swm3;
