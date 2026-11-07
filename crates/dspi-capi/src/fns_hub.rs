//! C functions: being a DSPi Link hub, that is, sharing this machine's devices
//! (feature `hub`, WP HB6), over `dspi_hub`. `docs/core-merge/HUB.md` is the design.
//!
//! The header declares them whatever the build. The library defines them only with
//! the feature: an app built against one without it fails at link time, not at run time.
//!
//! **One handle, many runs.** [`FfiHub`], from `dspi_hub_new`, is the app's way to a hub
//! and outlives each one: a hub exists exactly while sharing is on, between
//! `dspi_hub_start` and `dspi_hub_stop`. Its listener carries over.
//!
//! **Settings** are what `dspi_hub_start` is given. A running hub does not change them:
//! to apply a new name, port or access mode, stop and start, which disconnects the
//! clients (they reconnect by themselves, except after a change of access mode).
//! **Changing the access mode revokes every token** (spec 6.2). Before that start,
//! `dspi_hub_saved_settings` says how many clients it would cost, for a warning; after
//! it, `tokensRevoked` says how many it did.
//!
//! **Failure values.** A text function returns the bytes needed including the NUL, and 0
//! for a null handle, or a panic. `bool` functions return false, `void` ones do nothing.
//! Strings are NUL-terminated UTF-8.
//!
//! **Replies.** `{"ok":true, <fields in camelCase>}` (a list as `"value"`), or
//! `{"ok":false,"code":<code>,"error":<sentence to show>}`. Codes: `invalid` (an
//! argument), `running`, `not_running`, `in_callback`, `open_access`, and the hub's own,
//! `stopped`, `no_device`, `already_attached`, `no_handle`, `identity`, `app_attached`,
//! `io` (a port in use is this).
//!
//! **Short buffers.** A reply that does not fit writes nothing and returns the length
//! needed. `dspi_hub_start` and `dspi_hub_begin_pairing` act, so they check first: a null
//! buffer or one under `HUB_REPLY_MIN` returns that minimum and does nothing, and asking
//! for the length is safe. With a large enough buffer a reply too long means the call
//! **has happened**. It takes a very long path in an error to get there.
//!
//! **Secrets.** A client's token never reaches this ABI: the hub keeps only its hash and
//! hands the token to the client over the socket. The one secret here is the PIN, in
//! `dspi_hub_begin_pairing`'s reply, to show on screen. Nothing here logs either, and
//! the library's logger holds `tungstenite` at debug, as its trace level prints frames.
//!
//! **Threads.** Everything may be called from any thread. `dspi_hub_start`,
//! `dspi_hub_stop`, `dspi_hub_free`, `dspi_hub_attach_app` and `dspi_hub_attach_demo`
//! block, on the bus, the devices and the peers, for seconds: keep them off a UI thread.
//! They run one at a time, so a second `dspi_hub_stop` returns once the first is done.
//! The rest are quick, though `dspi_hub_revoke` and `dspi_hub_set_role` write a file.
//!
//! **Who owns the hub.** Only [`FfiHub`]'s `running` slot holds a `Hub`. Everything else,
//! a read and the listener included, holds a `HubHandle` or a `NetworkControl`, which
//! keep memory alive and not the hub, so the event thread can never hold the last owner
//! (dropping that there would leak the hub: HUB.md section 3).

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dspi_hub::app::AppLink;
use dspi_hub::{
    AccessMode, DeviceState, Hub, HubConfig, HubError, HubEvent, HubHandle, HubListener,
    NetworkConfig, NetworkControl, PairedClient, RegisteredDevice, Role, SavedHub, UsbBus,
};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};

use crate::{FfiCore, guard, with_core};

/// The smallest buffer `dspi_hub_start` and `dspi_hub_begin_pairing` act on.
pub const HUB_REPLY_MIN: u32 = 4096;

/// A new client pairs with a PIN, then presents a token. The default to offer.
pub const HUB_ACCESS_PIN: u8 = 0;
/// Every connection is an admin. Only when the user asks for it.
pub const HUB_ACCESS_OPEN: u8 = 1;

/// `dspi_hub_start` option: find and claim USB devices. Without it the hub serves what
/// `dspi_hub_attach_demo` gives it.
pub const HUB_OPT_USB: u32 = 1;
/// `dspi_hub_start` option: an app holds or will hold devices, so nothing is scanned or
/// claimed until `dspi_hub_attach_app`. Always pass it when an app will attach.
pub const HUB_OPT_APP: u32 = 2;
/// `dspi_hub_start` option: listen on loopback only. For tests.
pub const HUB_OPT_LOOPBACK: u32 = 4;
const HUB_OPT_ALL: u32 = HUB_OPT_USB | HUB_OPT_APP | HUB_OPT_LOOPBACK;

/// A device came online. `code` is its handle, `text` its serial.
pub const HUB_EVENT_DEVICE_ONLINE: u8 = 0;
/// A device went offline, keeping its handle for a while. `code` and `text` as above.
pub const HUB_EVENT_DEVICE_OFFLINE: u8 = 1;
/// A device's handle was given up. `code` and `text` as above.
pub const HUB_EVENT_DEVICE_REMOVED: u8 = 2;
/// A client's session opened, which is also how a finished pairing shows: read the
/// client list and the status again. `code` is the session, `text` "".
pub const HUB_EVENT_SESSION_OPENED: u8 = 3;
/// A client's session closed. `code` is the session, `text` "".
pub const HUB_EVENT_SESSION_CLOSED: u8 = 4;
/// The hub has stopped: the last event of a run. `code` is -1, `text` "".
pub const HUB_EVENT_STOPPED: u8 = 5;

/// What a hub is called when nobody has named it.
const DEFAULT_HUB_NAME: &str = "DSPi Hub";
/// Longest hub name kept, in characters: it goes to every client and into their lists.
const MAX_HUB_NAME: usize = 64;

/// A hub event: `event` is a `HUB_EVENT_*`. `text` is never null, and is valid during
/// the call only.
///
/// Called on the hub's event thread, one at a time, with no hub lock held. It may read:
/// `dspi_hub_status`, `dspi_hub_devices`, `dspi_hub_clients`. Anything else on the hub is
/// refused from here (`false`, -1 or code `in_callback`), since it would block or wait for
/// this very thread: post it to the app's own thread and return. Blocking here holds up
/// every later event, and `dspi_hub_stop`.
pub type HubEventCallback =
    extern "C" fn(event: u8, code: i32, text: *const c_char, user_data: *mut c_void);

thread_local! {
    /// This thread is inside a [`HubEventCallback`].
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// Clears [`IN_CALLBACK`], whatever the callback did.
struct LeaveCallback;

impl Drop for LeaveCallback {
    fn drop(&mut self) {
        IN_CALLBACK.with(|c| c.set(false));
    }
}

/// The C callback and its `user_data`.
struct CListener {
    callback: HubEventCallback,
    user_data: *mut c_void,
}

/// The listener, carried from run to run. Held across a call, so that replacing it
/// waits for the call: that is what lets `user_data` go right after. A callback cannot
/// wait for itself, as [`dspi_hub_set_listener`] refuses it there.
///
/// A hub's listener is this and nothing else, so no reference to a hub ever sits on the
/// event thread.
#[derive(Default)]
struct ListenerSlot(Mutex<Option<CListener>>);

// SAFETY: the callback runs on the hub's event thread by the ABI's documented contract
// (the caller's `user_data` must allow that); nothing here touches the pointer.
unsafe impl Send for ListenerSlot {}
// SAFETY: as for Send: the pointer is only passed back to the caller, under the mutex.
unsafe impl Sync for ListenerSlot {}

impl HubListener for ListenerSlot {
    fn on_event(&self, event: HubEvent) {
        let (kind, code, text) = match event {
            HubEvent::DeviceOnline { handle, serial } => {
                (HUB_EVENT_DEVICE_ONLINE, i32::from(handle), serial)
            }
            HubEvent::DeviceOffline { handle, serial } => {
                (HUB_EVENT_DEVICE_OFFLINE, i32::from(handle), serial)
            }
            HubEvent::DeviceRemoved { handle, serial } => {
                (HUB_EVENT_DEVICE_REMOVED, i32::from(handle), serial)
            }
            HubEvent::SessionOpened { id } => {
                (HUB_EVENT_SESSION_OPENED, i32::from(id), String::new())
            }
            HubEvent::SessionClosed { id } => {
                (HUB_EVENT_SESSION_CLOSED, i32::from(id), String::new())
            }
            HubEvent::Stopped => (HUB_EVENT_STOPPED, -1, String::new()),
        };
        // A string with an inner NUL reads as cut there, never as a failure.
        let mut text = text.into_bytes();
        text.push(0);
        let slot = lock(&self.0);
        if let Some(l) = slot.as_ref() {
            IN_CALLBACK.with(|c| c.set(true));
            let _leave = LeaveCallback;
            (l.callback)(kind, code, text.as_ptr().cast::<c_char>(), l.user_data);
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
}

/// The handle behind the C ABI's `struct FfiHub *`: created by `dspi_hub_new` and freed
/// by `dspi_hub_free`.
pub struct FfiHub {
    /// Held across start, stop and attaches, so they run one at a time. Above `running`.
    /// Never taken on the event thread, as those calls are refused there.
    lifecycle: Mutex<()>,
    /// Taken briefly, and never held across a call into the hub: `Hub::stop` waits for
    /// the event thread, whose listener may be reading this.
    running: Mutex<Option<Running>>,
    listener: Arc<ListenerSlot>,
}

/// A hub keeps its data consistent before anything that could panic.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run `f` on the handle inside [`guard`]; `fail` for a null handle or a panic.
fn with_hub<T>(p: *const FfiHub, fail: T, f: impl FnOnce(&FfiHub) -> T) -> T {
    if p.is_null() {
        return fail;
    }
    // SAFETY: a non-null handle comes from `dspi_hub_new` and stays valid until
    // `dspi_hub_free`; that is the C caller's contract.
    guard(fail, || f(unsafe { &*p }))
}

/// Refused here: said once in the log. True if this thread is inside the callback.
fn in_callback(what: &str) -> bool {
    let inside = IN_CALLBACK.with(Cell::get);
    if inside {
        log::warn!("dspi-capi: {what} called from a hub callback: refused");
    }
    inside
}

/// Borrow a NUL-terminated UTF-8 C string: `None` for null or invalid UTF-8.
fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null string argument is NUL-terminated and outlives the call; that
    // is the C caller's contract.
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// Copy all of `s` into `buf`, NUL-terminated, or nothing if it does not fit. Returns the
/// bytes needed including the NUL.
fn copy_whole(s: &str, buf: *mut c_char, len: u32) -> u32 {
    let Some(needed) = u32::try_from(s.len()).ok().and_then(|n| n.checked_add(1)) else {
        return u32::MAX;
    };
    if !buf.is_null() && needed <= len {
        // SAFETY: `buf` holds `len` bytes (the C caller's contract), `s` and its NUL fit
        // in them, and a Rust string cannot overlap a buffer the caller owns.
        unsafe {
            ptr::copy_nonoverlapping(s.as_ptr(), buf.cast::<u8>(), s.len());
            buf.add(s.len()).write(0);
        }
    }
    needed
}

// ── Replies ───────────────────────────────────────────────────────

/// A failure reply's `code` and sentence.
struct Failure(&'static str, String);

impl Failure {
    fn invalid(what: &str) -> Self {
        Self("invalid", what.to_owned())
    }

    fn not_running() -> Self {
        Self("not_running", "the hub is not running".to_owned())
    }

    fn in_callback() -> Self {
        Self("in_callback", "not possible from a hub callback".to_owned())
    }
}

impl From<HubError> for Failure {
    fn from(e: HubError) -> Self {
        let code = match &e {
            HubError::Stopped => "stopped",
            HubError::NoDevice => "no_device",
            HubError::AlreadyAttached(_) => "already_attached",
            HubError::NoHandle => "no_handle",
            HubError::Identity(_) => "identity",
            HubError::AppAttached => "app_attached",
            HubError::Io(_) => "io",
        };
        Self(code, e.to_string())
    }
}

/// A result as a JSON reply. An object gets `ok`; anything else goes in `value`.
fn reply(r: Result<Value, Failure>) -> String {
    let value = match r {
        Ok(Value::Object(mut m)) => {
            m.insert("ok".to_owned(), Value::Bool(true));
            Value::Object(m)
        }
        Ok(v) => json!({ "ok": true, "value": v }),
        Err(Failure(code, text)) => json!({ "ok": false, "code": code, "error": text }),
    };
    value.to_string()
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Viewer => "viewer",
        Role::Control => "control",
        Role::Admin => "admin",
    }
}

fn parse_role(text: &str) -> Option<Role> {
    match text {
        "viewer" => Some(Role::Viewer),
        "control" => Some(Role::Control),
        "admin" => Some(Role::Admin),
        _ => None,
    }
}

fn unix_seconds(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `online` is the client having a session now; false with no hub running.
fn client_json(c: &PairedClient, online: bool) -> Value {
    json!({
        "cid": c.cid,
        "name": c.name,
        "role": role_name(c.role),
        "created": unix_seconds(c.created),
        "lastSeen": c.last_seen.map(unix_seconds),
        "online": online,
    })
}

/// The shape of `dspi_link_devices`, plus `outputs`: a page can show both the same way.
fn device_json(d: &RegisteredDevice) -> Value {
    json!({
        "handle": d.handle,
        "serial": d.identity.serial,
        "name": d.name,
        "fw": d.identity.firmware,
        "fwBeta": d.identity.firmware_beta,
        "platform": d.identity.platform,
        "outputs": d.identity.outputs,
        "state": match d.state {
            DeviceState::Online => "online",
            DeviceState::Offline => "offline",
        },
        "link": "usb",
        "lockedBy": d.locked_by,
    })
}

// ── Starting ──────────────────────────────────────────────────────

/// What a start was asked for, checked.
#[derive(Debug, PartialEq, Eq)]
struct StartArgs {
    /// Empty: the stored name, or the default.
    name: String,
    store_dir: PathBuf,
    port: u16,
    access: AccessMode,
    options: u32,
}

/// What may be called a hub: no control characters, a bounded length. Empty if nothing
/// is left, which means no name was given.
fn clean_hub_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_HUB_NAME)
        .collect();
    kept.trim().to_owned()
}

fn parse_start(
    name: *const c_char,
    store_dir: *const c_char,
    port: u16,
    access: u8,
    options: u32,
) -> Result<StartArgs, Failure> {
    let name = c_str(name).ok_or_else(|| Failure::invalid("name is null or not UTF-8"))?;
    let store_dir = c_str(store_dir)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| Failure::invalid("store_dir is null, empty or not UTF-8"))?;
    let access = match access {
        HUB_ACCESS_PIN => AccessMode::Pin,
        HUB_ACCESS_OPEN => AccessMode::Open,
        _ => return Err(Failure::invalid("no such access mode")),
    };
    if options & !HUB_OPT_ALL != 0 {
        return Err(Failure::invalid("no such option"));
    }
    Ok(StartArgs {
        name: clean_hub_name(name),
        store_dir: PathBuf::from(store_dir),
        port,
        access,
        options,
    })
}

/// The hub's configuration. It never scans at start, not even a headless one: nothing is
/// claimed until the port is bound and the listener is in place ([`FfiHub::start`]).
fn config_for(args: &StartArgs, name: String) -> HubConfig {
    let mut network = NetworkConfig::new(args.store_dir.clone());
    network.port = args.port;
    network.access = args.access;
    if args.options & HUB_OPT_LOOPBACK != 0 {
        network.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    }
    let mut config = HubConfig::in_process(&name);
    config.network = Some(network);
    config.initial_scan = false;
    if args.options & HUB_OPT_USB != 0 {
        config.bus = Some(Box::new(UsbBus::default()));
    }
    config
}

/// A scan starts the bus's periodic ones. With an app, `AppLink::attach` does it, after
/// the app has let go of what it holds.
fn scans_at_start(options: u32) -> bool {
    options & HUB_OPT_USB != 0 && options & HUB_OPT_APP == 0
}

/// What a read needs of a run: nothing that owns the hub (module docs).
struct View {
    handle: HubHandle,
    control: NetworkControl,
    pairing_expires: Option<Instant>,
}

impl FfiHub {
    fn view(&self) -> Option<View> {
        lock(&self.running).as_ref().map(|r| View {
            handle: r.handle.clone(),
            control: r.control.clone(),
            pairing_expires: r.pairing_expires,
        })
    }

    fn start(&self, args: StartArgs) -> Result<Value, Failure> {
        let _one_at_a_time = lock(&self.lifecycle);
        if lock(&self.running).is_some() {
            return Err(Failure("running", "the hub is already running".to_owned()));
        }
        let name = if args.name.is_empty() {
            SavedHub::peek(&args.store_dir)?
                .and_then(|saved| saved.name)
                .unwrap_or_else(|| DEFAULT_HUB_NAME.to_owned())
        } else {
            args.name.clone()
        };
        let hub = Hub::start(config_for(&args, name.clone()))?;
        let handle = hub.handle();
        // Always there on a networked hub that has just started.
        let control = hub.network().ok_or(HubError::Stopped)?;
        if !args.name.is_empty() {
            control.auth().set_name(&name);
        }
        let tokens_revoked = control.auth().tokens_revoked_at_start();
        let listener: Arc<dyn HubListener> = Arc::clone(&self.listener) as _;
        handle.set_listener(Some(listener));
        let answer = json!({
            "hubId": handle.hub_id(),
            "name": name,
            "port": control.port(),
            "access": args.access.wire_name(),
            "tokensRevoked": tokens_revoked,
        });
        *lock(&self.running) = Some(Running {
            hub,
            handle: handle.clone(),
            control,
            pairing_expires: None,
            app: None,
        });
        if scans_at_start(args.options) {
            handle.scan_now();
        }
        Ok(answer)
    }

    fn stop(&self) {
        let _one_at_a_time = lock(&self.lifecycle);
        // Out of the slot first: a stop waits for the event thread, and the listener on
        // it may be taking `running` for a read.
        let running = lock(&self.running).take();
        let Some(Running { hub, app, .. }) = running else {
            return;
        };
        match app {
            Some(app) => app.stop_hub(&hub),
            None => hub.stop(),
        }
    }

    fn status(&self) -> Value {
        let Some(view) = self.view() else {
            return json!({ "running": false });
        };
        let now = Instant::now();
        let auth = view.control.auth();
        let open = auth.pairing_open(now);
        let left = view
            .pairing_expires
            .filter(|_| open)
            .map_or(0, |e| e.saturating_duration_since(now).as_millis());
        json!({
            "running": true,
            "hubId": view.handle.hub_id(),
            "name": view.handle.name(),
            "port": view.control.port(),
            "access": auth.mode().wire_name(),
            "connections": view.control.connections(),
            "inaccessible": view.handle.inaccessible(),
            "tokensRevoked": auth.tokens_revoked_at_start(),
            "pairingOpen": open,
            "pairingExpiresInMs": u64::try_from(left).unwrap_or(u64::MAX),
        })
    }
}

// ── The handle ────────────────────────────────────────────────────

/// Create a hub's handle, with no hub running. Free it with `dspi_hub_free`. Null if
/// creating it panicked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_new() -> *mut FfiHub {
    guard(ptr::null_mut(), || {
        crate::init_logger();
        Box::into_raw(Box::new(FfiHub {
            lifecycle: Mutex::new(()),
            running: Mutex::new(None),
            listener: Arc::new(ListenerSlot::default()),
        }))
    })
}

/// Stop a running hub and destroy the handle. Null is ignored. Blocks as
/// `dspi_hub_stop` does. The last callback, `HUB_EVENT_STOPPED`, has returned by the time
/// this does, so `user_data` may go then. Refused from inside the callback, with a log
/// line: the handle stays valid and nothing is freed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_free(hub: *mut FfiHub) {
    if hub.is_null() || in_callback("dspi_hub_free") {
        return;
    }
    guard((), || {
        // SAFETY: a non-null handle comes from `dspi_hub_new` and is freed once: the C
        // caller's contract.
        let handle = unsafe { Box::from_raw(hub) };
        handle.stop();
    });
}

/// Register the events callback, which stays for later runs; NULL clears it. Replacing or
/// clearing waits for a call in progress on another thread, so `user_data` may go right
/// after it returns. False for a null handle, and when called from inside the callback,
/// where it would wait for itself.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_set_listener(
    hub: *mut FfiHub,
    // Spelled out: cbindgen renders `Option<HubEventCallback>` as a struct.
    callback: Option<
        extern "C" fn(event: u8, code: i32, text: *const c_char, user_data: *mut c_void),
    >,
    user_data: *mut c_void,
) -> bool {
    if in_callback("dspi_hub_set_listener") {
        return false;
    }
    with_hub(hub, false, |h| {
        *lock(&h.listener.0) = callback.map(|callback| CListener {
            callback,
            user_data,
        });
        true
    })
}

// ── Before a start ────────────────────────────────────────────────

/// What the store in `store_dir` holds, with or without a hub on it: JSON {ok, exists,
/// hubId, name, access, defaultPort, tokensRevokedIfPin, tokensRevokedIfOpen, clients:
/// [{cid, name, role, created, lastSeen, online}]}. Creates nothing and changes nothing.
///
/// `access` is the mode it last ran in (`pin`, `none`) and null before the first start;
/// `name` is null if never named; `hubId` is null when `exists` is false. A store that
/// cannot be parsed reads as not existing: the next start sets it aside.
///
/// **`tokensRevokedIfPin` and `tokensRevokedIfOpen`** are how many paired clients a start
/// in that mode would forget, and must pair again. Warn the user when it is not 0, since
/// it cannot be undone. `created` and `lastSeen` are seconds since the epoch, `lastSeen`
/// may be null, and `online` is false here. Returns the length needed; 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_saved_settings(
    store_dir: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let answer = (|| -> Result<Value, Failure> {
            let dir = c_str(store_dir)
                .filter(|d| !d.is_empty())
                .ok_or_else(|| Failure::invalid("store_dir is null, empty or not UTF-8"))?;
            let saved = SavedHub::peek(Path::new(dir))?;
            let lost = |mode: AccessMode| {
                saved
                    .as_ref()
                    .map_or(0, |s| s.tokens_lost_starting_in(mode))
            };
            let clients: Vec<Value> = saved.as_ref().map_or_else(Vec::new, |s| {
                s.clients.iter().map(|c| client_json(c, false)).collect()
            });
            Ok(json!({
                "exists": saved.is_some(),
                "hubId": saved.as_ref().map(|s| &s.hub_id),
                "name": saved.as_ref().and_then(|s| s.name.as_ref()),
                "access": saved.as_ref().and_then(|s| s.access.as_ref()),
                "defaultPort": dspi_hub::DEFAULT_PORT,
                "tokensRevokedIfPin": lost(AccessMode::Pin),
                "tokensRevokedIfOpen": lost(AccessMode::Open),
                "clients": clients,
            }))
        })();
        copy_whole(&reply(answer), buf, len)
    })
}

// ── Running ───────────────────────────────────────────────────────

/// Start the hub: its threads and its socket, then, with `HUB_OPT_USB` and without
/// `HUB_OPT_APP`, the first bus scan. Replies JSON {ok, hubId, name, port, access,
/// tokensRevoked}. Blocks. If it fails, nothing is left running.
///
/// `name`: "" takes the stored one or a default; another is cleaned and stored.
/// `store_dir` is the hub's own directory, created if need be: its id, clients and device names. Two hubs must not
/// share one. `port` 0 lets the OS choose, and the reply says which. `access` is a
/// `HUB_ACCESS_*`, and `options` are `HUB_OPT_*`.
///
/// **A change of `access` from the store's last run revokes every token.** `tokensRevoked`
/// says how many; `dspi_hub_saved_settings` says so beforehand. Code `running` if a hub
/// already is: stop it first. A port in use is code `io`.
///
/// Returns the length needed; see the module's rule on short buffers.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_start(
    hub: *mut FfiHub,
    name: *const c_char,
    store_dir: *const c_char,
    port: u16,
    access: u8,
    options: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    with_hub(hub, 0, |h| {
        if buf.is_null() || len < HUB_REPLY_MIN {
            return HUB_REPLY_MIN;
        }
        let answer = if in_callback("dspi_hub_start") {
            Err(Failure::in_callback())
        } else {
            parse_start(name, store_dir, port, access, options).and_then(|args| h.start(args))
        };
        copy_whole(&reply(answer), buf, len)
    })
}

/// Stop the hub. Clients are told (close code 4003), queued commands are answered, every
/// thread is joined and the devices are let go when this returns. An app on it gets its
/// direct path back, and its device reopened. Blocks up to a few seconds. True also when
/// no hub was running; a concurrent caller waits for the first. False for a null handle,
/// and from inside the callback, which it would have to wait for.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_stop(hub: *mut FfiHub) -> bool {
    if in_callback("dspi_hub_stop") {
        return false;
    }
    with_hub(hub, false, |h| {
        h.stop();
        true
    })
}

/// Put the app's core on the running hub, so that its USB devices come through it. The
/// device it holds is closed, claimed by the hub and reopened, so the mirror is read
/// afresh, and a flow in progress on it fails, as with an unplug. A demo, MIDI or Link
/// connection stays as it is. Starts the bus's scans. Blocks. False for a null handle, no
/// hub running, a second app, or inside the callback, with a log line.
///
/// `dspi_hub_stop` undoes it. Stop the hub before `dspi_core_free`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_attach_app(hub: *mut FfiHub, core: *mut FfiCore) -> bool {
    if in_callback("dspi_hub_attach_app") {
        return false;
    }
    with_hub(hub, false, |h| {
        with_core(core, false, |c| {
            let _one_at_a_time = lock(&h.lifecycle);
            let Some(view) = h.view() else {
                return false;
            };
            // Not under `running`: this waits for the bus.
            match AppLink::attach(&view.handle, Arc::clone(&c.core)) {
                Ok(link) => {
                    if let Some(r) = lock(&h.running).as_mut() {
                        r.app = Some(link);
                    }
                    true
                }
                Err(e) => {
                    log::warn!("dspi-capi: the app did not attach to the hub: {e}");
                    false
                }
            }
        })
    })
}

/// Give the running hub a demo device to serve, for trying sharing with no hardware.
/// Blocks for its identity reads. Returns its handle, or -1 for a null handle, no hub
/// running, a demo device already attached, or inside the callback, with a log line.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_attach_demo(hub: *mut FfiHub) -> i32 {
    if in_callback("dspi_hub_attach_demo") {
        return -1;
    }
    with_hub(hub, -1, |h| {
        let _one_at_a_time = lock(&h.lifecycle);
        let Some(view) = h.view() else {
            return -1;
        };
        match view.handle.attach(Box::new(FakeDevice::demo()), 0) {
            Ok(handle) => i32::from(handle),
            Err(e) => {
                log::warn!("dspi-capi: the demo device did not attach to the hub: {e}");
                -1
            }
        }
    })
}

/// Where the hub stands: JSON {ok, running}, and while it runs {hubId, name, port, access,
/// connections, inaccessible, tokensRevoked, pairingOpen, pairingExpiresInMs}. `access` is
/// `pin` or `none`, as clients see it. `connections` counts sockets, signed in or not.
/// `inaccessible` is devices on the bus that could not be opened at the last scan: another
/// program, or a permission. `tokensRevoked` is what this run's start cost. Quick, and
/// allowed in the callback. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_status(hub: *mut FfiHub, buf: *mut c_char, len: u32) -> u32 {
    with_hub(hub, 0, |h| copy_whole(&reply(Ok(h.status())), buf, len))
}

/// The hub's devices, online and in their grace: JSON {ok, value: [{handle, serial, name,
/// fw, fwBeta, platform, outputs, state, link, lockedBy}]}, empty with no hub running.
/// Quick, and allowed in the callback. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_devices(hub: *mut FfiHub, buf: *mut c_char, len: u32) -> u32 {
    with_hub(hub, 0, |h| {
        let list = h.view().map_or_else(Vec::new, |v| {
            v.handle.devices().iter().map(device_json).collect()
        });
        copy_whole(&reply(Ok(Value::Array(list))), buf, len)
    })
}

/// The paired clients: JSON {ok, value: [{cid, name, role, created, lastSeen, online}]}.
/// `role` is `viewer`, `control` or `admin`; `created` and `lastSeen` are seconds since the
/// epoch, `lastSeen` may be null; `online` means it has a session now. Code `not_running`
/// with no hub: `dspi_hub_saved_settings` lists them then. An open hub lists none, as its
/// clients do not pair. Quick, and allowed in the callback. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_clients(hub: *mut FfiHub, buf: *mut c_char, len: u32) -> u32 {
    with_hub(hub, 0, |h| {
        let answer = h.view().ok_or_else(Failure::not_running).map(|v| {
            let list = v.control.auth().clients();
            Value::Array(
                list.iter()
                    .map(|c| client_json(c, v.control.is_online(c.cid)))
                    .collect(),
            )
        });
        copy_whole(&reply(answer), buf, len)
    })
}

/// Open the pairing window, replacing an open one, whose PIN then stops working: JSON
/// {ok, pin, expiresInMs}. Show the PIN to the user and to nobody else, with the time it
/// has left. A client's right PIN ends the window (`HUB_EVENT_SESSION_OPENED`), and so do
/// five wrong ones. Code `not_running`, or `open_access` if this hub admits anyone anyway,
/// or `io` if the system has no random numbers. Returns the length needed; see the
/// module's rule on short buffers.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_begin_pairing(hub: *mut FfiHub, buf: *mut c_char, len: u32) -> u32 {
    with_hub(hub, 0, |h| {
        if buf.is_null() || len < HUB_REPLY_MIN {
            return HUB_REPLY_MIN;
        }
        let answer = if in_callback("dspi_hub_begin_pairing") {
            Err(Failure::in_callback())
        } else {
            begin_pairing(h)
        };
        copy_whole(&reply(answer), buf, len)
    })
}

fn begin_pairing(h: &FfiHub) -> Result<Value, Failure> {
    let mut running = lock(&h.running);
    let r = running.as_mut().ok_or_else(Failure::not_running)?;
    let auth = r.control.auth();
    if auth.mode() != AccessMode::Pin {
        return Err(Failure(
            "open_access",
            "this hub lets anyone in, so there is nothing to pair".to_owned(),
        ));
    }
    let now = Instant::now();
    let window = auth.begin_pairing(now)?;
    let left = window.expires.saturating_duration_since(now).as_millis();
    let answer = json!({
        "pin": window.pin,
        "expiresInMs": u64::try_from(left).unwrap_or(u64::MAX),
    });
    r.pairing_expires = Some(window.expires);
    Ok(answer)
}

/// Close the pairing window, if one is open. False for a null handle, no hub running, or
/// inside the callback.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_cancel_pairing(hub: *mut FfiHub) -> bool {
    if in_callback("dspi_hub_cancel_pairing") {
        return false;
    }
    with_hub(hub, false, |h| {
        h.view().is_some_and(|v| {
            v.control.auth().cancel_pairing();
            true
        })
    })
}

/// Forget a paired client: its token stops working and its connections close at once with
/// code 4002, which clients do not reconnect from. It would have to pair again. Writes the
/// store. False for a null handle, no hub running, no such client, or inside the callback.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_revoke(hub: *mut FfiHub, cid: u32) -> bool {
    if in_callback("dspi_hub_revoke") {
        return false;
    }
    with_hub(hub, false, |h| {
        h.view().is_some_and(|v| v.control.revoke(cid))
    })
}

/// Change a paired client's role, `viewer`, `control` or `admin`: its next command is
/// judged by the new one. Writes the store. False for a null handle, no hub running, no
/// such client, another role, or inside the callback.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_set_role(hub: *mut FfiHub, cid: u32, role: *const c_char) -> bool {
    if in_callback("dspi_hub_set_role") {
        return false;
    }
    let Some(role) = c_str(role).and_then(parse_role) else {
        return false;
    };
    with_hub(hub, false, |h| {
        h.view().is_some_and(|v| v.control.set_role(cid, role))
    })
}

#[cfg(test)]
#[path = "fns_hub/tests_wp_hb6.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb6;
