//! C functions: DSPi Link sessions (feature `link`) and desktop MIDI ports
//! (feature `midir`) (WP CG1), over [`dspi_core::DspiCore`]'s `link_*` methods
//! and `dspi_transport::midi::midir_port`.
//!
//! The header declares them whatever the build (it is the same file for every
//! feature set). The library defines them only with the features: an app built
//! against one without them fails at link time, not at run time.
//!
//! **Failure values.** A text function returns the bytes needed including the
//! NUL, and 0 for a null handle, a null required string or a panic. `bool`
//! functions return false, `void` ones do nothing. Strings are NUL-terminated
//! UTF-8.
//!
//! **Replies.** Functions with a `buf` reply in JSON, as `fns_misc`'s do:
//! `{"ok":true, <fields in camelCase>}` (a list as `"value"`), or
//! `{"ok":false,"code":<code>,"error":<sentence to show>}`. Here `code` is
//! the Link error's own (`wrong_hub`, `wrong_pin`, `rate_limited`, `hub_busy`,
//! `handshake_refused`, `io`, `handshake`, `protocol`, `timeout`, `closed`, or
//! the hub's own), else the kinds `fns_misc` lists: `disconnected`, `invalid`, ...
//!
//! **Short buffers.** A reply that does not fit writes nothing and returns the
//! length needed. `dspi_link_connect`, `dspi_link_pair` and
//! `dspi_link_select_device` act on the hub, so they check first: a null buffer
//! or one under `LINK_REPLY_MIN` returns that minimum and does nothing,
//! and asking for the length is safe. With a large enough buffer a reply too
//! long means the call **has happened**: never repeat a `dspi_link_pair` for
//! that, a PIN is good once. The listener delivers the token anyway.
//!
//! **Threads.** Everything here may be called from any thread. Connecting,
//! pairing, selecting, `dspi_link_fetch_info` and `dspi_open_midi` block on the
//! network or the device: keep them off a UI thread. `dspi_link_close` does not.

use std::ffi::{CStr, c_char};
use std::ptr;

/// Borrow a NUL-terminated UTF-8 C string: `None` for null or invalid UTF-8.
fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null string argument is NUL-terminated and outlives the
    // call; that is the C caller's contract.
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// Copy all of `s` into `buf`, NUL-terminated, or nothing if it does not fit.
/// Returns the bytes needed including the NUL.
fn copy_whole(s: &str, buf: *mut c_char, len: u32) -> u32 {
    let Some(needed) = u32::try_from(s.len()).ok().and_then(|n| n.checked_add(1)) else {
        return u32::MAX;
    };
    if !buf.is_null() && needed <= len {
        // SAFETY: `buf` holds `len` bytes (the C caller's contract), `s` and
        // its NUL fit in them, and a Rust string cannot overlap a buffer the
        // caller owns.
        unsafe {
            ptr::copy_nonoverlapping(s.as_ptr(), buf.cast::<u8>(), s.len());
            buf.add(s.len()).write(0);
        }
    }
    needed
}

#[cfg(feature = "link")]
pub(crate) mod link {
    use std::cell::Cell;
    use std::ffi::{c_char, c_void};
    use std::sync::Arc;

    use dspi_core::link::{link_fetch_info, link_parse_address, link_ws_url};
    use dspi_core::{
        DspiCore, DspiError, LinkDevice, LinkHello, LinkInfo, LinkListener, LinkPhase, LinkStatus,
        Result,
    };
    use serde_json::{Value, json};

    use super::{c_str, copy_whole};
    use crate::{FfiCore, guard, with_core};

    /// The smallest buffer `dspi_link_connect`, `dspi_link_pair` and
    /// `dspi_link_select_device` act on.
    pub const LINK_REPLY_MIN: u32 = 4096;

    /// `link_devices` changed. `code` is -1, the strings are "".
    pub const LINK_EVENT_DEVICES_CHANGED: u8 = 0;
    /// The session ended for good. `code` is the hub's WebSocket close code,
    /// -1 = none; `text` the reason. A lost connection is not an end.
    pub const LINK_EVENT_CLOSED: u8 = 1;
    /// `dspi_link_status`'s phase changed, or a reconnect try failed. `code` is
    /// -1, the strings are "".
    pub const LINK_EVENT_STATUS_CHANGED: u8 = 2;
    /// The hub issued a token: store it now. `text` is the hub id, `token` the
    /// token; `code` is -1. It comes even if `dspi_link_pair` has timed out.
    pub const LINK_EVENT_TOKEN_ISSUED: u8 = 3;

    /// A Link session event: `event` is a `LINK_EVENT_*`. `text` and `token` are
    /// never null, and are valid during the call only.
    ///
    /// Called on a core thread, with no core lock held. It may call
    /// `dspi_link_close` and `dspi_link_status`; anything that blocks belongs on
    /// the app's own thread: post it there and return.
    pub type LinkEventCallback = extern "C" fn(
        event: u8,
        code: i32,
        text: *const c_char,
        token: *const c_char,
        user_data: *mut c_void,
    );

    thread_local! {
        /// This thread is inside a [`LinkEventCallback`].
        static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
    }

    /// The C callback and its `user_data`, as the core's listener.
    struct CLink {
        callback: LinkEventCallback,
        user_data: *mut c_void,
    }

    // SAFETY: the callback runs on core threads by the ABI's documented
    // contract (the caller's `user_data` must allow that); nothing here touches
    // the pointer.
    unsafe impl Send for CLink {}
    // SAFETY: as for Send: the pointer is only passed back to the caller.
    unsafe impl Sync for CLink {}

    impl CLink {
        fn fire(&self, event: u8, code: i32, text: &str, token: &str) {
            // A string with an inner NUL reads as cut there, never as a failure.
            let z = |s: &str| {
                let mut v = Vec::with_capacity(s.len().saturating_add(1));
                v.extend_from_slice(s.as_bytes());
                v.push(0);
                v
            };
            let (text, token) = (z(text), z(token));
            IN_CALLBACK.with(|c| c.set(true));
            (self.callback)(
                event,
                code,
                text.as_ptr().cast::<c_char>(),
                token.as_ptr().cast::<c_char>(),
                self.user_data,
            );
            IN_CALLBACK.with(|c| c.set(false));
        }
    }

    impl LinkListener for CLink {
        fn on_link_devices_changed(&self) {
            self.fire(LINK_EVENT_DEVICES_CHANGED, -1, "", "");
        }

        fn on_link_closed(&self, code: i32, reason: String) {
            self.fire(LINK_EVENT_CLOSED, code, &reason, "");
        }

        fn on_link_status_changed(&self) {
            self.fire(LINK_EVENT_STATUS_CHANGED, -1, "", "");
        }

        fn on_link_token_issued(&self, hub_id: String, token: String) {
            self.fire(LINK_EVENT_TOKEN_ISSUED, -1, &hub_id, &token);
        }
    }

    // ── Replies ───────────────────────────────────────────────────────

    /// The `code` of a failure reply (module docs).
    fn error_code(e: &DspiError) -> &str {
        match e {
            DspiError::Link { code, .. } => code,
            DspiError::Disconnected => "disconnected",
            DspiError::Transfer { .. } => "transfer",
            DspiError::Refused { .. } => "refused",
            DspiError::InvalidArgument { .. } => "invalid",
            DspiError::Unsupported => "unsupported",
            DspiError::ReadOnly => "read_only",
            DspiError::Locked => "locked",
            DspiError::Denied => "denied",
            DspiError::Io { .. } => "io",
        }
    }

    /// A result as a JSON reply. `v` is an object without a field named `ok`.
    fn reply(r: Result<Value>) -> String {
        let value = match r {
            Ok(Value::Object(mut m)) => {
                m.insert("ok".to_owned(), Value::Bool(true));
                Value::Object(m)
            }
            Ok(v) => json!({ "ok": true, "value": v }),
            Err(e) => {
                let text = match &e {
                    // Written for the user, without the prefix.
                    DspiError::Link { msg, .. } => msg.clone(),
                    other => other.to_string(),
                };
                json!({ "ok": false, "code": error_code(&e), "error": text })
            }
        };
        value.to_string()
    }

    /// A required string argument.
    fn arg<'a>(p: *const c_char, name: &str) -> Result<&'a str> {
        c_str(p).ok_or_else(|| DspiError::InvalidArgument {
            msg: format!("{name} is null or not UTF-8"),
        })
    }

    /// The JSON name of a phase.
    fn phase_name(p: &LinkPhase) -> &'static str {
        match p {
            LinkPhase::Idle => "idle",
            LinkPhase::NeedsPairing => "needs_pairing",
            LinkPhase::NoDevice => "no_device",
            LinkPhase::ChooseDevice => "choose_device",
            LinkPhase::Bound => "bound",
            LinkPhase::Reconnecting => "reconnecting",
            LinkPhase::Closed => "closed",
            LinkPhase::Refused => "refused",
        }
    }

    fn hello_json(h: LinkHello) -> Value {
        json!({
            "hubId": h.hub_id,
            "hubName": h.hub_name,
            "hubKind": h.hub_kind,
            "hubVersion": h.hub_version,
            "protoMinor": h.proto_minor,
            "auth": h.auth,
            "pinSource": h.pin_source,
            "needsPairing": h.needs_pairing,
            "caps": h.caps,
            "maxPayload": h.max_payload,
            "maxInflight": h.max_inflight,
            "tokenRefused": h.token_refused,
        })
    }

    fn device_json(d: LinkDevice) -> Value {
        json!({
            "handle": d.handle,
            "serial": d.serial,
            "name": d.name,
            "fw": d.fw,
            "fwBeta": d.fw_beta,
            "platform": d.platform,
            "state": d.state,
            "link": d.link,
            "lockedBy": d.locked_by,
        })
    }

    fn status_json(s: LinkStatus) -> Value {
        let denied: Vec<Value> = s
            .denied
            .iter()
            .map(|d| json!({ "request": d.request, "dir": d.dir }))
            .collect();
        json!({
            "connected": s.connected,
            "ready": s.ready,
            "session": s.session,
            "role": s.role,
            "denied": denied,
            "closed": s.closed,
            "closeCode": s.close_code,
            "closeReason": s.close_reason,
            "phase": phase_name(&s.phase),
            "reconnectAttempt": s.reconnect_attempt,
            "reconnectInMs": s.reconnect_in_ms,
            "pinSource": s.pin_source,
            "tokenRefused": s.token_refused,
        })
    }

    fn info_json(i: LinkInfo) -> Value {
        json!({
            "hubId": i.hub_id,
            "hubName": i.hub_name,
            "hubKind": i.hub_kind,
            "auth": i.auth,
            "protoMinor": i.proto_minor,
            "wsPath": i.ws_path,
            "serials": i.serials,
        })
    }

    /// A read: reply into `buf`, 0 for a null handle or a panic.
    fn read(
        core: *mut FfiCore,
        buf: *mut c_char,
        len: u32,
        f: impl FnOnce(&DspiCore) -> String,
    ) -> u32 {
        with_core(core, 0, |h| copy_whole(&f(&h.core), buf, len))
    }

    /// A call that acts on the hub: nothing happens unless `buf` can take a
    /// normal reply (module docs).
    fn act(
        core: *mut FfiCore,
        buf: *mut c_char,
        len: u32,
        f: impl FnOnce(&DspiCore) -> Result<Value>,
    ) -> u32 {
        with_core(core, 0, |h| {
            if buf.is_null() || len < LINK_REPLY_MIN {
                return LINK_REPLY_MIN;
            }
            copy_whole(&reply(f(&h.core)), buf, len)
        })
    }

    // ── Sessions ──────────────────────────────────────────────────────

    /// Open a Link session to `url` (`dspi_link_ws_url`), replacing any previous
    /// one, and reply with the hub's hello: JSON {ok, hubId, hubName, hubKind,
    /// hubVersion, protoMinor, auth, pinSource, needsPairing, caps, maxPayload,
    /// maxInflight, tokenRefused}. Blocks up to about 15 s. All strings are
    /// required.
    ///
    /// `token`: the token stored for this hub, "" = none. `expected_hub_id`: the
    /// id it was stored under. With it, a token goes out only after the hello
    /// has named that hub, and another hub at the address gets code `wrong_hub`
    /// having seen no token. "" = not known: the token goes to whichever hub
    /// answers. Forget a stored token only when `tokenRefused` says so.
    ///
    /// When `needsPairing`, ask for the PIN and call `dspi_link_pair`. From here
    /// on the core keeps the session up: do not redial, do not queue writes.
    /// Returns the length needed; see the module's rule on short buffers.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_connect(
        core: *mut FfiCore,
        url: *const c_char,
        client_name: *const c_char,
        app_version: *const c_char,
        token: *const c_char,
        expected_hub_id: *const c_char,
        buf: *mut c_char,
        len: u32,
    ) -> u32 {
        act(core, buf, len, |c| {
            let url = arg(url, "url")?.to_owned();
            let client_name = arg(client_name, "client_name")?.to_owned();
            let app_version = arg(app_version, "app_version")?.to_owned();
            let token = arg(token, "token")?.to_owned();
            let expected_hub_id = arg(expected_hub_id, "expected_hub_id")?.to_owned();
            c.link_connect(url, client_name, app_version, token, expected_hub_id)
                .map(hello_json)
        })
    }

    /// Pair with the PIN the user typed: JSON {ok, token}; store the token
    /// against the hub id ("" when the hub issued none). Code `wrong_pin`: ask
    /// again. `rate_limited`: the hub refuses for a minute. `timeout`: the hub
    /// may still have accepted it, and `LINK_EVENT_TOKEN_ISSUED` then delivers
    /// the token. Blocks. Returns the length needed; see the module's rule on
    /// short buffers.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_pair(
        core: *mut FfiCore,
        pin: *const c_char,
        buf: *mut c_char,
        len: u32,
    ) -> u32 {
        act(core, buf, len, |c| {
            let pin = arg(pin, "pin")?.to_owned();
            c.link_pair(pin).map(|token| json!({ "token": token }))
        })
    }

    /// The hub's devices (empty before authentication): JSON {ok, value:
    /// [{handle, serial, name, fw, fwBeta, platform, state, link, lockedBy}]}.
    /// Returns the length needed.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_devices(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
        read(core, buf, len, |c| {
            let list = c.link_devices().into_iter().map(device_json).collect();
            reply(Ok(Value::Array(list)))
        })
    }

    /// Where the session stands: JSON {ok, connected, ready, session, role,
    /// denied: [{request, dir}], closed, closeCode, closeReason, phase,
    /// reconnectAttempt, reconnectInMs, pinSource, tokenRefused}. `phase` is
    /// what to tell the user: `idle`, `needs_pairing`, `no_device`,
    /// `choose_device`, `bound`, `reconnecting`, `closed` or `refused`. No
    /// blocking beyond the core lock. Returns the length needed.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_status(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
        read(core, buf, len, |c| reply(Ok(status_json(c.link_status()))))
    }

    /// Bind the core to the hub's device `handle` (from `dspi_link_devices`), and
    /// reply JSON {ok}. Then `dspi_fetch_all`, as for USB. The core binds it again
    /// by itself after a reconnect. Blocks. Returns the length needed; see the
    /// module's rule on short buffers.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_select_device(
        core: *mut FfiCore,
        handle: u32,
        serial: *const c_char,
        buf: *mut c_char,
        len: u32,
    ) -> u32 {
        act(core, buf, len, |c| {
            let serial = arg(serial, "serial")?.to_owned();
            c.link_select_device(handle, serial)?;
            Ok(json!({}))
        })
    }

    /// Close the session, drop a device bound to it and stop any reconnecting:
    /// how an app leaves a hub, and the lost-connection banner's abort button.
    /// Safe on a UI thread and inside a [`LinkEventCallback`]: it returns within
    /// about 100 ms whatever the link does. Quiet: no `LINK_EVENT_CLOSED`.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_close(core: *mut FfiCore) {
        with_core(core, (), |h| h.core.link_close());
    }

    /// Try to reconnect now instead of waiting out the backoff (the network is
    /// back). A no-op unless reconnecting.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_retry_now(core: *mut FfiCore) {
        with_core(core, (), |h| h.core.link_retry_now());
    }

    /// The hub answers at a new address: reconnect there from now on. False for
    /// a null handle, or a null or non-UTF-8 `url`. An address that cannot be
    /// dialled is logged and ignored, and this still returns true.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_set_url(core: *mut FfiCore, url: *const c_char) -> bool {
        let Some(url) = c_str(url) else {
            return false;
        };
        with_core(core, false, |h| {
            h.core.link_set_url(url.to_owned());
            true
        })
    }

    /// Register the session events callback; NULL clears it. Replacing or
    /// clearing waits for a call in progress on another thread, so `user_data`
    /// may go right after it returns. False for a null handle, and when called
    /// from inside the callback, where it would wait for itself. `dspi_core_free`
    /// clears it too, and must not be called from inside it.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_set_listener(
        core: *mut FfiCore,
        // Spelled out: cbindgen renders `Option<LinkEventCallback>` as a struct.
        callback: Option<
            extern "C" fn(
                event: u8,
                code: i32,
                text: *const c_char,
                token: *const c_char,
                user_data: *mut c_void,
            ),
        >,
        user_data: *mut c_void,
    ) -> bool {
        if IN_CALLBACK.with(Cell::get) {
            log::warn!("dspi-capi: dspi_link_set_listener called from its own callback");
            return false;
        }
        with_core(core, false, |h| {
            let listener = callback.map(|callback| {
                Arc::new(CLink {
                    callback,
                    user_data,
                }) as Arc<dyn LinkListener>
            });
            h.core.set_link_listener(listener);
            true
        })
    }

    // ── No session needed ─────────────────────────────────────────────

    /// Parse a hub address the user typed (`host`, `host:port`, `[v6]:port`):
    /// JSON {ok, host, port}. Returns the length needed; 0 for a panic.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_parse_address(
        text: *const c_char,
        buf: *mut c_char,
        len: u32,
    ) -> u32 {
        guard(0, || {
            let parsed = arg(text, "text")
                .and_then(|t| link_parse_address(t.to_owned()))
                .map(|a| json!({ "host": a.host, "port": a.port }));
            copy_whole(&reply(parsed), buf, len)
        })
    }

    /// The WebSocket URL for a hub, as plain text. `path` null or "" is the
    /// default (`/dspi/v1`). Returns the length needed; 0 for a null or non-UTF-8
    /// `host` or a non-UTF-8 `path`.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_ws_url(
        host: *const c_char,
        port: u16,
        path: *const c_char,
        buf: *mut c_char,
        len: u32,
    ) -> u32 {
        guard(0, || {
            let Some(host) = c_str(host) else {
                return 0;
            };
            let path = if path.is_null() {
                ""
            } else if let Some(p) = c_str(path) {
                p
            } else {
                return 0;
            };
            let url = link_ws_url(host.to_owned(), port, path.to_owned());
            copy_whole(&url, buf, len)
        })
    }

    /// `GET /dspi/v1/info`, for a hub added by address: JSON {ok, hubId, hubName,
    /// hubKind, auth, protoMinor, wsPath, serials}. Blocks up to a few seconds.
    /// Returns the length needed; 0 for a panic. A short buffer means asking the
    /// hub again.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_link_fetch_info(
        host: *const c_char,
        port: u16,
        buf: *mut c_char,
        len: u32,
    ) -> u32 {
        guard(0, || {
            let info = arg(host, "host")
                .and_then(|h| link_fetch_info(h.to_owned(), port))
                .map(info_json);
            copy_whole(&reply(info), buf, len)
        })
    }
}

#[cfg(feature = "midir")]
pub(crate) mod midi {
    use std::ffi::c_char;

    use dspi_transport::midi::midir_port::{self, PortInfo};

    use super::{c_str, copy_whole};
    use crate::{FfiCore, guard, with_core};

    /// The port `name` asks for; no name takes the only one. `None` when it is
    /// not there, or when no name leaves a choice.
    pub(crate) fn pick<'a>(ports: &'a [PortInfo], name: Option<&str>) -> Option<&'a PortInfo> {
        match name {
            Some(n) => ports.iter().find(|p| p.name == n),
            None => match ports {
                [one] => Some(one),
                _ => None,
            },
        }
    }

    /// The DSPi control ports this host can see: JSON {ok, value: [name]}. Lists
    /// MIDI ports, so keep it off a UI thread and call it at most once a second.
    /// Returns the length needed; 0 for a panic.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_midi_ports(buf: *mut c_char, len: u32) -> u32 {
        guard(0, || {
            let names: Vec<String> = midir_port::list_ports()
                .into_iter()
                .map(|p| p.name)
                .collect();
            copy_whole(
                &serde_json::json!({ "ok": true, "value": names }).to_string(),
                buf,
                len,
            )
        })
    }

    /// Open a DSPi through its USB MIDI control ports. `name` is one of
    /// `dspi_midi_ports`' names; null or "" takes the only port. Replaces any
    /// open device; follow it with `dspi_fetch_all`. Blocks for the connect
    /// sequence. False for a null handle, a non-UTF-8 `name`, no such port, a
    /// choice of several, or when the device does not answer.
    #[unsafe(no_mangle)]
    pub extern "C" fn dspi_open_midi(core: *mut FfiCore, name: *const c_char) -> bool {
        let name = if name.is_null() {
            None
        } else if let Some(n) = c_str(name) {
            Some(n).filter(|n| !n.is_empty())
        } else {
            return false;
        };
        with_core(core, false, |h| {
            let ports = midir_port::list_ports();
            let Some(port) = pick(&ports, name) else {
                log::warn!("dspi-capi: no single DSPi MIDI port among {}", ports.len());
                return false;
            };
            let opened = midir_port::open(port)
                .map_err(dspi_core::DspiError::from)
                .and_then(|t| h.core.open_transport(Box::new(t), port.name.clone(), 0));
            opened
                .inspect_err(|e| log::warn!("dspi-capi: MIDI open failed: {e}"))
                .is_ok()
        })
    }
}

#[cfg(test)]
#[path = "fns_link/tests_wp_cg1.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg1;
