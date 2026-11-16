//! C functions: finding DSPi Link hubs on the network (WP CG2).
//!
//! Always in the header. In a build without feature `mdns` they exist and refuse:
//! `dspi_link_browse_start` returns false and the text functions reply
//! `{"ok":false,"code":"unsupported",...}`, so a client can tell.
//!
//! Text functions follow `fns_misc`: NUL-terminated JSON into `buf`, returning the
//! bytes needed including the NUL. When that is more than `len`, nothing is
//! written: call again with a larger buffer. 0 for a null handle.

use std::ffi::{c_char, c_void};
use std::ptr;

use crate::{FfiCore, with_core};

/// Borrow a C string: `None` for null or invalid UTF-8.
#[cfg(feature = "mdns")]
fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null string argument is NUL-terminated and outlives the
    // call; that is the C caller's contract.
    unsafe { std::ffi::CStr::from_ptr(p) }.to_str().ok()
}

/// Copy `s` into `buf` if it fits whole, NUL-terminated; the bytes needed.
fn copy_text(s: &str, buf: *mut c_char, len: u32) -> u32 {
    let needed = u32::try_from(s.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .unwrap_or(u32::MAX);
    if needed <= len && !buf.is_null() {
        // SAFETY: `buf` holds `len` bytes (the C caller's contract), s.len() + 1
        // <= len, and a Rust string cannot overlap a buffer the caller owns.
        unsafe {
            ptr::copy_nonoverlapping(s.as_ptr(), buf.cast::<u8>(), s.len());
            buf.add(s.len()).write(0);
        }
    }
    needed
}

#[cfg(feature = "mdns")]
mod on {
    use std::ffi::c_void;
    use std::sync::Arc;

    use dspi_core::DspiCore;
    use dspi_core::link_browse::{BrowseListener, BrowseSnapshot};
    use serde_json::{Value, json};

    /// A C wake-up callback and its `user_data`, as a browse listener.
    pub(super) struct CWake {
        pub(super) callback: extern "C" fn(user_data: *mut c_void),
        pub(super) user_data: *mut c_void,
    }

    // SAFETY: the callback runs on the browser's thread by the documented
    // contract (the caller's `user_data` must allow that); nothing here touches
    // the pointer.
    unsafe impl Send for CWake {}
    // SAFETY: as for Send: the pointer is only passed back to the caller.
    unsafe impl Sync for CWake {}

    impl BrowseListener for CWake {
        fn on_hubs_changed(&self, _: BrowseSnapshot) {
            (self.callback)(self.user_data);
        }
    }

    /// The listener for a C callback; `None` means no listener.
    pub(super) fn listener(
        callback: Option<extern "C" fn(user_data: *mut c_void)>,
        user_data: *mut c_void,
    ) -> Option<Arc<dyn BrowseListener>> {
        callback.map(|callback| {
            Arc::new(CWake {
                callback,
                user_data,
            }) as Arc<dyn BrowseListener>
        })
    }

    pub(super) fn hubs(core: &DspiCore) -> String {
        let snapshot = core.link_browse_snapshot();
        json!({
            "ok": true,
            "browsing": snapshot.browsing,
            "error": snapshot.error,
            "hubs": snapshot.hubs,
        })
        .to_string()
    }

    pub(super) fn resolve(core: &DspiCore, hub_id: &str) -> String {
        let hub = core.link_browse_resolve(hub_id.to_owned());
        let hub = hub
            .and_then(|h| serde_json::to_value(h).ok())
            .unwrap_or(Value::Null);
        json!({ "ok": true, "hub": hub }).to_string()
    }
}

/// The reply of a text function in a build that cannot browse.
#[cfg(not(feature = "mdns"))]
const UNSUPPORTED: &str =
    r#"{"ok":false,"code":"unsupported","error":"this build does not look for hubs"}"#;

/// Start looking for hubs on the local network, replacing any browse running.
/// Returns at once; false only with a null handle, in a build without the
/// feature, or when a thread cannot start. A network failure shows in
/// `dspi_link_browse_hubs`'s `error`, and browsing is tried again by itself.
///
/// `own_hub_id` is this app's own hub, which is not listed; null or "" for none.
/// `callback` (may be null) is a wake-up when the list or state changes. It runs
/// on the browser's thread with `user_data`, which must allow that: schedule work
/// and return. It may call any `dspi_link_browse_*` function.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_browse_start(
    core: *mut FfiCore,
    // Spelled out: cbindgen renders `Option<NotifyCallback>` as a struct.
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
    own_hub_id: *const c_char,
) -> bool {
    with_core(core, false, |_h| {
        #[cfg(feature = "mdns")]
        {
            let own = c_str(own_hub_id).unwrap_or("").to_owned();
            let listener = on::listener(callback, user_data);
            _h.core.link_browse_start(listener, own).is_ok()
        }
        #[cfg(not(feature = "mdns"))]
        {
            let _ = (callback, user_data, own_hub_id);
            false
        }
    })
}

/// Stop looking and forget the hubs. Waits for a callback in progress, so none is
/// in progress or to come afterwards and `user_data` can go, unless called from
/// the callback itself. Also happens in `dspi_core_free`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_browse_stop(core: *mut FfiCore) {
    with_core(core, (), |_h| {
        #[cfg(feature = "mdns")]
        _h.core.link_browse_stop();
    });
}

/// The browse as JSON: `{"ok":true,"browsing":bool,"error":text or null,"hubs":[{"id",
/// "name","kind","auth","deviceCount","serials","host","port","path","url",
/// "addresses","tlsAdvertised"}]}`, sorted by name. Not browsing and no hubs when none
/// runs. `url` goes to the connect function. `tlsAdvertised` (WP SECDISC): the record
/// carried `tls=1`, a hint for the connect's no-fallback rule. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_browse_hubs(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    with_core(core, 0, |_h| {
        #[cfg(feature = "mdns")]
        let reply = on::hubs(&_h.core);
        #[cfg(not(feature = "mdns"))]
        let reply = UNSUPPORTED.to_owned();
        copy_text(&reply, buf, len)
    })
}

/// The hub with this id (any case) as last heard: `{"ok":true,"hub":{...}}`, fields
/// as in `dspi_link_browse_hubs`, or `"hub":null` when it is not found. A null or
/// invalid id is not found. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_browse_resolve(
    core: *mut FfiCore,
    hub_id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    with_core(core, 0, |_h| {
        #[cfg(feature = "mdns")]
        let reply = on::resolve(&_h.core, c_str(hub_id).unwrap_or(""));
        #[cfg(not(feature = "mdns"))]
        let reply = {
            let _ = hub_id;
            UNSUPPORTED.to_owned()
        };
        copy_text(&reply, buf, len)
    })
}

/// Point the current Link session at the address its hub is announced at now; a
/// session waiting to reconnect dials at once. Call it when the browse changes
/// while the session is reconnecting, with the id from the session's hello. False,
/// with nothing changed, when that hub is not found.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_browse_reresolve(core: *mut FfiCore, hub_id: *const c_char) -> bool {
    with_core(core, false, |_h| {
        #[cfg(feature = "mdns")]
        {
            c_str(hub_id).is_some_and(|id| _h.core.link_browse_reresolve(id.to_owned()))
        }
        #[cfg(not(feature = "mdns"))]
        {
            let _ = hub_id;
            false
        }
    })
}

/// Ask the network whether this hub is still there, for when dialling it fails. One
/// that does not answer leaves the list within seconds, and the callback runs.
/// Returns at once.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_browse_verify(core: *mut FfiCore, hub_id: *const c_char) {
    with_core(core, (), |_h| {
        #[cfg(feature = "mdns")]
        if let Some(id) = c_str(hub_id) {
            _h.core.link_browse_verify(id.to_owned());
        }
        #[cfg(not(feature = "mdns"))]
        let _ = hub_id;
    });
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "fns_browse/tests_wp_cg2.rs"]
mod tests_wp_cg2;
