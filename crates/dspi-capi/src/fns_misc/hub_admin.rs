//! C functions: the hub's admin API over DSPi Link (WP HUBADMIN), over
//! `dspi_core::hub_admin`. Replies as in the parent module's docs: JSON. An
//! Installer page asks `dspi_installer_target` which copy it edits: the hub's
//! (`copy` `hub`, editable by an Admin of the hub only) or this console's own
//! (`console`, the `dspi_limits_*` and `dspi_installer_*` calls).
//!
//! A call off a hub fails with `{"ok": false, "code": "link", "noticeKey":
//! "dlink.no_limits"}`; below Admin the hub refuses with `code` `denied`.
//! Every `dspi_hub_*` call but the events is a round trip: off the UI thread.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::sync::Arc;

use dspi_core::{HubAdminEvent, HubAdminListener};

use super::{arg, json_reply, optional_text, text};
use crate::with_core;

/// Which copy the Installer page edits: JSON `{ok, copy, editable, serial,
/// role, noticeKey, readOnlyKey}` (`copy` `hub`, `console` or `none`). No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_target(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.installer_target())))
}

/// The hub's limits, PIN state and pinned modes for the open device: JSON
/// `{ok, serial, role, canEdit, profileDocument, pinSet, locked, unlocked,
/// held, wrongTries, retryAfterMs, pinnedModes: [{role, mode}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_limits_get(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_limits_get()))
}

/// Set the hub's limits profile (`document`: a limits document): JSON `{ok,
/// held, state}` (what the hub held at once; the state as
/// `dspi_hub_limits_get`'s), or a failure (`invalid`, `denied`, `link`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_limits_set(
    core: *mut crate::FfiCore,
    document: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(document, "document").and_then(|d| c.hub_limits_set(d.to_owned())))
    })
}

/// Remove the hub's limits profile: the state's JSON, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_limits_clear(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_limits_clear()))
}

/// Set the hub's installer PIN: JSON `{ok, recoveryCode, state}` (the code
/// shown this once; notice `installer_lock.hub_recovery`), or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_pin_set(
    core: *mut crate::FfiCore,
    pin: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(pin, "pin").and_then(|p| c.hub_pin_set(p.to_owned())))
    })
}

/// Remove the hub's installer PIN (and its recovery code): the state's
/// JSON, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_pin_clear(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_pin_clear()))
}

/// Pin `role`'s mode on the hub (`mode` "" or null unpins): the state's
/// JSON, or a failure (`invalid` for an unknown role or mode).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_mode_pin(
    core: *mut crate::FfiCore,
    role: *const c_char,
    mode: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(role, "role")
                .map(str::to_owned)
                .and_then(|r| optional_text(mode, "mode").map(|m| (r, m.unwrap_or_default())))
                .and_then(|(r, m)| c.hub_mode_pin(r, m)),
        )
    })
}

/// The hub's admin events since the last call, oldest first: JSON `{ok,
/// value: [{kind, serial, rows, role, mode, lock}]}`. No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_admin_events(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.hub_admin_events())))
}

/// A hub admin event: `kind` (`limits.changed`, `limits.held`,
/// `mode.changed`) and the event as JSON (`{kind, serial, rows, role, mode,
/// lock}`), both valid during the call only.
///
/// Called on the Link session's supervisor thread with no core lock held,
/// after the core read the lock again: post the work to the app's own thread
/// and return. Do not call `dspi_hub_admin_set_listener` from it.
pub type HubAdminCallback =
    extern "C" fn(kind: *const c_char, event_json: *const c_char, user_data: *mut c_void);

thread_local! {
    /// This thread is inside a [`HubAdminCallback`].
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

struct CHubAdmin {
    callback: HubAdminCallback,
    user_data: *mut c_void,
}

// SAFETY: the callback runs on the core's threads by the ABI's documented
// contract (the caller's `user_data` must allow that); nothing here touches
// the pointer.
unsafe impl Send for CHubAdmin {}
// SAFETY: as for Send: the pointer is only passed back to the caller.
unsafe impl Sync for CHubAdmin {}

impl HubAdminListener for CHubAdmin {
    fn on_hub_admin_event(&self, event: HubAdminEvent) {
        let json = serde_json::to_string(&event).unwrap_or_default();
        // A string with an inner NUL is cut there, never a failure.
        let z = |s: &str| CString::new(s.split('\0').next().unwrap_or_default()).ok();
        let (Some(kind), Some(json)) = (z(&event.kind), z(&json)) else {
            return;
        };
        IN_CALLBACK.with(|c| c.set(true));
        (self.callback)(kind.as_ptr(), json.as_ptr(), self.user_data);
        IN_CALLBACK.with(|c| c.set(false));
    }
}

/// Set (or with a null `callback`, clear) the listener for the hub's admin
/// events. False for a null core, or when called from the callback itself.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_admin_set_listener(
    core: *mut crate::FfiCore,
    callback: Option<
        extern "C" fn(kind: *const c_char, event_json: *const c_char, user_data: *mut c_void),
    >,
    user_data: *mut c_void,
) -> bool {
    if IN_CALLBACK.with(Cell::get) {
        log::warn!("dspi-capi: dspi_hub_admin_set_listener called from its own callback");
        return false;
    }
    with_core(core, false, |h| {
        let listener = callback.map(|callback| {
            Arc::new(CHubAdmin {
                callback,
                user_data,
            }) as Arc<dyn HubAdminListener>
        });
        h.core.set_hub_admin_listener(listener);
        true
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_hubadmin {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    extern "C" fn heard(_kind: *const c_char, _json: *const c_char, _user: *mut c_void) {}

    #[test]
    fn wp_hubadmin_the_target_and_the_hub_calls_through_c() {
        let h = dspi_core_new();
        let t = reply(|b, l| dspi_installer_target(h, b, l));
        assert_eq!(
            (t["ok"].clone(), t["copy"].clone()),
            (true.into(), "none".into()),
            "{t}"
        );
        assert!(dspi_open_demo(h));
        let t = reply(|b, l| dspi_installer_target(h, b, l));
        assert_eq!(t["copy"], "console", "{t}");
        assert_eq!(t["editable"], true);
        assert_eq!(t["noticeKey"], "installer.console_copy");
        // Off a hub: the hub calls say so, by their notice.
        for r in [
            reply(|b, l| dspi_hub_limits_get(h, b, l)),
            reply(|b, l| dspi_hub_pin_clear(h, b, l)),
            reply(|b, l| dspi_hub_limits_clear(h, b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["noticeKey"], "dlink.no_limits", "{r}");
        }
        let short = CString::new("12").unwrap();
        let r = reply(|b, l| dspi_hub_pin_set(h, short.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let role = CString::new("control").unwrap();
        let r = reply(|b, l| dspi_hub_mode_pin(h, role.as_ptr(), std::ptr::null(), b, l));
        assert_eq!(r["noticeKey"], "dlink.no_limits", "{r}");
        let r = reply(|b, l| dspi_hub_admin_events(h, b, l));
        assert_eq!(r["value"], Value::Array(Vec::new()), "{r}");
        assert!(dspi_hub_admin_set_listener(
            h,
            Some(heard),
            std::ptr::null_mut()
        ));
        assert!(dspi_hub_admin_set_listener(h, None, std::ptr::null_mut()));
        assert!(!dspi_hub_admin_set_listener(
            std::ptr::null_mut(),
            None,
            std::ptr::null_mut()
        ));
        dspi_core_free(h);
    }
}
