//! C functions: the set of cores, one per open device (WP MD0), over
//! [`dspi_core::CoreSet`].
//!
//! A `struct FfiCoreSet *` from `dspi_core_set_new` holds the open devices.
//! `dspi_core_set_core` hands out a `struct FfiCore *` for one of them, which
//! every `dspi_*` function takes as usual. That handle belongs to the set: it
//! stays valid until `dspi_core_set_free` and must never be given to
//! `dspi_core_free`. After `dspi_core_set_close` of its id it is retired, not
//! freed: a call another thread makes on it late answers as a core with no
//! device (`Disconnected`), never touches freed memory.
//!
//! -- WP MDQT1 -- `dspi_core_set_main` is the app's main core: there from the
//! start, with a device or without, and never retired. What an app does with a
//! core while no device is open (looking for hubs, dialling one) it does on
//! that one; a device opened on it through `dspi_select_device` and the like is
//! in the set like the others, under its id.
//!
//! Replies are JSON, as in `fns_misc`'s docs. A set's own failures have the
//! codes `already_open` (with `id`, the device the unit is open as), `full`
//! and `unknown`; a failure to open the device is the device's reply.

use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::sync::{Arc, Mutex, PoisonError};

use dspi_core::{
    CoreSet, CoreSetConfig, CoreSetError, CoreSetEvent, CoreSetListener, DeviceId, DeviceTarget,
    DspiCore,
};
use serde_json::{Value, json};

use crate::fns_device::{c_str, copy_text, write_devices};
use crate::fns_misc::json_reply;
use crate::types::{DeviceInfo, DspState};
use crate::{FfiCore, guard};

/// The handle behind `struct FfiCoreSet *`.
pub struct FfiCoreSet {
    set: Arc<CoreSet>,
    /// The `FfiCore` handed out per core, boxed so its address stays put
    /// (-- WP MDQT1 -- not per id: an id follows the device).
    #[allow(clippy::vec_box)]
    handles: Mutex<Vec<Box<FfiCore>>>,
    /// Handles of closed devices, kept until the set is freed (review RA F2);
    /// still boxed, since C holds their addresses.
    #[allow(clippy::vec_box)]
    retired: Mutex<Vec<Box<FfiCore>>>,
}

impl FfiCoreSet {
    #[allow(clippy::vec_box)]
    fn handles(&self) -> std::sync::MutexGuard<'_, Vec<Box<FfiCore>>> {
        self.handles.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The handle of `core`, the same one each time.
    fn handle(&self, core: Arc<DspiCore>) -> *mut FfiCore {
        let mut handles = self.handles();
        if let Some(h) = handles.iter_mut().find(|h| Arc::ptr_eq(&h.core, &core)) {
            return ptr::from_mut::<FfiCore>(h.as_mut());
        }
        let state = Box::new(DspState::from(&core.get_state()));
        let mut handle = Box::new(FfiCore {
            core,
            state: Mutex::new(state),
        });
        let p = ptr::from_mut::<FfiCore>(handle.as_mut());
        handles.push(handle);
        p
    }
}

/// Run `f` on the set `p` inside [`guard`]; `fail` for a null handle or a panic.
fn with_set<T>(p: *const FfiCoreSet, fail: T, f: impl FnOnce(&FfiCoreSet) -> T) -> T {
    if p.is_null() {
        return fail;
    }
    guard(fail, || {
        // SAFETY: a non-null handle comes from `dspi_core_set_new` and stays
        // valid until `dspi_core_set_free`; that is the C caller's contract.
        f(unsafe { &*p })
    })
}

// -- WP MCP3 --
/// The set behind `p`, for the MCP server (`fns_mcp`); `None` for null.
#[cfg(feature = "mcp")]
pub(crate) fn set_arc(p: *const FfiCoreSet) -> Option<Arc<CoreSet>> {
    with_set(p, None, |s| Some(Arc::clone(&s.set)))
}

/// A text reply from the set (`fns_misc`'s rules): the bytes needed, 0 for a
/// null handle or a panic.
fn set_text(
    p: *const FfiCoreSet,
    buf: *mut c_char,
    len: u32,
    f: impl FnOnce(&FfiCoreSet) -> String,
) -> u32 {
    with_set(p, 0, |s| copy_text(&f(s), buf, len))
}

/// A set failure as a JSON reply.
fn failure(e: CoreSetError) -> String {
    let (code, extra) = match &e {
        CoreSetError::AlreadyOpen { id, .. } => ("already_open", json!(id.as_str())),
        CoreSetError::Full => ("full", Value::Null),
        CoreSetError::Unknown { .. } => ("unknown", Value::Null),
        CoreSetError::Device(d) => return json_reply::<()>(Err(d.clone())),
    };
    let mut reply = serde_json::Map::new();
    reply.insert("ok".into(), Value::Bool(false));
    reply.insert("code".into(), json!(code));
    reply.insert("error".into(), json!(e.to_string()));
    if !extra.is_null() {
        reply.insert("id".into(), extra);
    }
    Value::Object(reply).to_string()
}

/// A JSON string field of `v`, "" when absent.
fn field(v: &Value, name: &str) -> String {
    v.get(name).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// The target a JSON object names, or why not.
fn target(v: &Value) -> Result<DeviceTarget, String> {
    match v.get("kind").and_then(Value::as_str) {
        Some("demo") => Ok(DeviceTarget::Demo),
        Some("usb") => match field(v, "serial") {
            s if s.is_empty() => Err("a USB target needs its serial".into()),
            serial => Ok(DeviceTarget::Usb { serial }),
        },
        Some("document") => Ok(DeviceTarget::Document {
            text: field(v, "text"),
            file_name: field(v, "fileName"),
        }),
        #[cfg(feature = "link")]
        Some("hub") => {
            let t = dspi_core::HubTarget {
                url: field(v, "url"),
                hub_id: field(v, "hubId"),
                serial: field(v, "serial"),
                client_name: field(v, "clientName"),
                app_version: field(v, "appVersion"),
                credential: field(v, "credential"),
                tls_advertised: v
                    .get("tlsAdvertised")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            if t.url.is_empty() || t.serial.is_empty() {
                return Err("a hub target needs its url and serial".into());
            }
            Ok(DeviceTarget::Hub(t))
        }
        Some(other) => Err(format!("no target kind \"{other}\" in this build")),
        None => Err("the target names no kind".into()),
    }
}

/// An invalid-argument reply.
fn invalid(msg: String) -> String {
    json_reply::<()>(Err(dspi_core::DspiError::InvalidArgument { msg }))
}

/// Create an empty set of cores, and install the logger as `dspi_core_new`
/// does. Free it with `dspi_core_set_free`. Null only if creating it panicked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_new() -> *mut FfiCoreSet {
    guard(ptr::null_mut(), || {
        crate::init_logger();
        Box::into_raw(Box::new(FfiCoreSet {
            set: CoreSet::new(),
            handles: Mutex::new(Vec::new()),
            retired: Mutex::new(Vec::new()),
        }))
    })
}

/// Free a set: its listener is unset, every device is closed as by
/// `dspi_core_set_close`, and every `FfiCore` it handed out dies, those of
/// devices closed earlier too. No other thread may be inside a call on any of
/// them. Null is ignored. Blocks while closing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_free(set: *mut FfiCoreSet) {
    if set.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: a non-null handle comes from dspi_core_set_new and is not
        // used again after this call; the C caller's contract.
        let handle = unsafe { Box::from_raw(set) };
        handle.set.set_listener(None);
        // -- WP MDQT1 -- the main core as `dspi_core_free` would leave it
        let main = handle.set.main_core();
        unset_callbacks(&main);
        #[cfg(feature = "link")]
        main.link_close();
        #[cfg(feature = "mdns")]
        main.link_browse_stop();
        main.disconnect();
        let ids: Vec<DeviceId> = handle.set.list().into_iter().map(|d| d.id).collect();
        for id in ids {
            let _ = close(&handle, &id);
        }
    });
}

/// No callback of `core` runs after this.
fn unset_callbacks(core: &DspiCore) {
    core.set_notify_listener(None);
    core.rta_set_listener(None);
    core.set_hotplug_callback(None);
    #[cfg(feature = "link")]
    core.set_link_listener(None);
}

/// Unset the callbacks of the handle for `id`, close the device, then retire
/// the handle: it lives on, with no device, until the set is freed. The main
/// core keeps both: it goes on with no device.
fn close(s: &FfiCoreSet, id: &DeviceId) -> Result<(), CoreSetError> {
    let core = s.set.core(id);
    let main = core
        .as_ref()
        .is_some_and(|c| Arc::ptr_eq(c, &s.set.main_core()));
    if main {
        return s.set.close(id);
    }
    if let Some(core) = &core {
        unset_callbacks(core);
        // -- WP MDQT1 -- a browse of its own ends with it
        #[cfg(feature = "mdns")]
        core.link_browse_stop();
    }
    s.set.close(id)?;
    let mut handles = s.handles();
    let at = handles
        .iter()
        .position(|h| core.as_ref().is_some_and(|c| Arc::ptr_eq(&h.core, c)));
    if let Some(at) = at {
        s.retired
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(handles.remove(at));
    }
    Ok(())
}

/// Open the device `target` names on a core of its own and add it to the set.
/// `target` is JSON: `{"kind": "demo"}`, `{"kind": "usb", "serial"}`,
/// `{"kind": "document", "text", "fileName"}`, or (with Link) `{"kind": "hub",
/// "url", "hubId", "serial", "clientName", "appVersion", "credential",
/// "tlsAdvertised"}`: a unit behind a hub on its own session, with a credential
/// the app holds. JSON `{ok, id}`, or the failure (module docs). Blocks for the
/// connect sequence: not on the UI thread.
///
/// -- WP INT5 -- `"extra": true` beside the kind: a device the app uses itself
/// without showing it (a unit measured with others). `dspi_core_set_list` says
/// `"extra": true` of it, and AI agents are not offered it.
///
/// -- WP MDQT1 -- It acts once, whatever the buffer: a reply that does not fit
/// (the returned length is over `len`) writes nothing, but the device is open.
/// Do not call it again for the id (it would open a second demo, or be refused
/// as `already_open`): `dspi_core_set_list` has it. 4096 bytes hold any reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_open(
    set: *mut FfiCoreSet,
    target_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    set_text(set, buf, len, |s| {
        let parsed = c_str(target_json)
            .ok_or_else(|| "the target is null or not UTF-8".to_owned())
            .and_then(|t| {
                serde_json::from_str::<Value>(t).map_err(|e| format!("the target is not JSON: {e}"))
            })
            .and_then(|v| Ok((target(&v)?, v.get("extra") == Some(&Value::Bool(true)))));
        match parsed {
            Err(msg) => invalid(msg),
            Ok((t, extra)) => match if extra {
                s.set.open_extra(t)
            } else {
                s.set.open(t)
            } {
                Ok(id) => json!({ "ok": true, "id": id.as_str() }).to_string(),
                Err(e) => failure(e),
            },
        }
    })
}

/// Close the device `id` and drop it from the set. The `FfiCore` handed out
/// for it has its callbacks unset and is retired, not freed: until
/// `dspi_core_set_free` it stays valid and answers as a core with no device
/// (`Disconnected`), so a call another thread is making on it is safe. A later
/// open of the same id hands out a new pointer. JSON `{ok}`, or `unknown`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_close(
    set: *mut FfiCoreSet,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    set_text(set, buf, len, |s| {
        let text = c_str(id).unwrap_or("");
        let found = DeviceId::parse(text).ok_or(CoreSetError::Unknown {
            id: text.to_owned(),
        });
        match found.and_then(|id| close(s, &id)) {
            Ok(()) => json!({ "ok": true }).to_string(),
            Err(e) => failure(e),
        }
    })
}

// -- WP MDQT1 --
/// The set's main core, for every `dspi_*` call: there with a device or
/// without, the same pointer each time, until `dspi_core_set_free`. Never pass
/// it to `dspi_core_free`. Closing the device open on it
/// (`dspi_core_set_close`) leaves it, and its callbacks, in place. Null for a
/// null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_main(set: *mut FfiCoreSet) -> *mut FfiCore {
    with_set(set, ptr::null_mut(), |s| s.handle(s.set.main_core()))
}

// -- WP MDQT1 --
/// The open device `core` (a handle of this set) has, as the last
/// `dspi_core_set_list` or poll saw it: its row as there, in `{ok, value}`;
/// `value` null with no device. Does not wait on a device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_device_of(
    set: *mut FfiCoreSet,
    core: *const FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    set_text(set, buf, len, |s| {
        let device = crate::with_core(core, None, |c| s.set.device_of(&c.core));
        json!({ "ok": true, "value": device }).to_string()
    })
}

/// The open devices, in the order they were opened (the main core's first): JSON `{ok, value: [{id,
/// label, transport, serial, state}]}`. `transport` is `usb`, `hub`, `link`,
/// `demo` or `file`; `state` is `live`, `reconnecting` or `lost`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_list(set: *mut FfiCoreSet, buf: *mut c_char, len: u32) -> u32 {
    set_text(set, buf, len, |s| json_reply(Ok(s.set.list())))
}

/// The core of the open device `id`, for every `dspi_*` call; null for an id
/// that is not open. The set owns it: it stays valid until
/// `dspi_core_set_free` (with no device after `dspi_core_set_close` of `id`),
/// the same pointer each time while the device is open. Never pass it to
/// `dspi_core_free`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_core(set: *mut FfiCoreSet, id: *const c_char) -> *mut FfiCore {
    with_set(set, ptr::null_mut(), |s| {
        let Some(id) = c_str(id).and_then(DeviceId::parse) else {
            return ptr::null_mut();
        };
        let Some(core) = s.set.core(&id) else {
            return ptr::null_mut();
        };
        s.handle(core)
    })
}

/// The open device whose unit has `serial`: its id in `{ok, value}`, `value`
/// null when none.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_find_serial(
    set: *mut FfiCoreSet,
    serial: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    set_text(set, buf, len, |s| {
        let id = s.set.find_serial(c_str(serial).unwrap_or(""));
        json!({ "ok": true, "value": id.as_ref().map(DeviceId::as_str) }).to_string()
    })
}

/// Set the folders every core of the set gets, now and from now on. `config`
/// is JSON `{limitsDir, snapshotDir, measureDir, eventLogDir, linkDir,
/// speakerLayoutDir}`; an absent or empty one is not set. JSON `{ok}`, or the first core's failure.
/// Blocking file I/O. -- WP MDQT1 -- Call it before opening devices (the limits
/// and the speaker layout are then in force from the first); a folder that
/// cannot be used fails this call, and never an open.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_config(
    set: *mut FfiCoreSet,
    config_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    set_text(set, buf, len, |s| {
        let Some(v) = c_str(config_json).and_then(|t| serde_json::from_str::<Value>(t).ok()) else {
            return invalid("the configuration is not JSON".into());
        };
        let config = CoreSetConfig {
            limits_dir: field(&v, "limitsDir"),
            snapshot_dir: field(&v, "snapshotDir"),
            measure_dir: field(&v, "measureDir"),
            event_log_dir: field(&v, "eventLogDir"),
            link_dir: field(&v, "linkDir"),
            speaker_layout_dir: field(&v, "speakerLayoutDir"), // -- WP INT1 --
        };
        json_reply(s.set.set_config(config).map(|()| serde_json::Map::new()))
    })
}

/// Look for DSPi units on USB for the picker, as `dspi_scan_devices`: up to
/// `max_devices` written to `out_devices` (may be null), the units the set
/// holds listed without being opened again. Returns how many were written,
/// or would have been; 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_scan_devices(
    set: *mut FfiCoreSet,
    out_devices: *mut DeviceInfo,
    max_devices: u32,
) -> u32 {
    with_set(set, 0, |s| {
        write_devices(&s.set.scan_devices(), out_devices, max_devices)
    })
}

/// One hotplug poll for every device of the set, with one USB enumeration
/// (about every 500 ms, off the UI thread). The cores' own
/// `dspi_poll_hotplug` does nothing in a set. Events go to the set's listener.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_poll_hotplug(set: *mut FfiCoreSet) {
    with_set(set, (), |s| s.set.poll_hotplug());
}

/// A C listener for the set's events.
struct CEvents {
    callback: extern "C" fn(event_json: *const c_char, user_data: *mut c_void),
    user_data: *mut c_void,
}

// SAFETY: the callback runs on the threads that call the set (open, close,
// poll), which its documentation tells the caller; `user_data` is only
// handed back to it.
unsafe impl Send for CEvents {}
// SAFETY: as for Send.
unsafe impl Sync for CEvents {}

impl CoreSetListener for CEvents {
    fn on_event(&self, event: CoreSetEvent) {
        let text = serde_json::to_string(&event).unwrap_or_default();
        if let Ok(c) = CString::new(text) {
            (self.callback)(c.as_ptr(), self.user_data);
        }
    }
}

/// Set the callback for the set's events: `event_json` is `{kind, id,
/// serial}`, `kind` one of `opened`, `closed`, `lost`, `reconnecting`,
/// `restored`, `arrived`, `departed` (`id` "" for the last two: a unit on the
/// bus, not an open device). It runs on the thread of the call that caused it
/// (`dspi_core_set_open`, `_close`, `_poll_hotplug`) with no lock held, and
/// the string lives for the call. Null unsets it; once that returns, the old
/// one is not running.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_set_set_listener(
    set: *mut FfiCoreSet,
    callback: Option<extern "C" fn(event_json: *const c_char, user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    with_set(set, (), |s| {
        let listener = callback.map(|callback| -> Arc<dyn CoreSetListener> {
            Arc::new(CEvents {
                callback,
                user_data,
            })
        });
        s.set.set_listener(listener);
    });
}

#[cfg(test)]
#[path = "fns_core_set/tests_wp_md0.rs"]
mod tests_wp_md0;

#[cfg(test)]
#[path = "fns_core_set/tests_wp_mdqt1.rs"]
mod tests_wp_mdqt1;
