//! DSPi Link (WP WB4b): a DSPi behind a hub, through the browser's WebSocket.
//!
//! The client is the core's own: `dspi-link`'s [`LinkClient`] under the core's
//! session and supervisor (`dspi_core::link`), with every rule they have (the
//! token looked up after the hello, reconnect with backoff, re-bind, a write
//! made while the link is down never sent later, 30 s of silence ending a
//! session). Only the socket is the browser's: [`JsConnector`] is a
//! `dspi_link::socket::WsConnector` over the Worker's `ws` imports
//! (`apps/web/src/core/ws.ts`). No `@dspi/link` JavaScript.
//!
//! **No threads.** On this target the client and the supervisor are
//! caller-driven (`dspi_link::client::caller_driven`): whoever waits for an
//! answer reads the socket meanwhile, and the Worker turns the session with
//! `linkTick` on its timer and whenever the socket has something
//! ([`tick`]). `poll` and `meters` read what arrived first
//! (`DspiCore::link_service`), so notifications and meter frames are current.
//!
//! **The calls**, in the order a page makes them (Link's choose_device step):
//! `linkConnect {url, token, hubId}` (the hello: whether a PIN is needed),
//! `linkPair {pin}` (the token to keep for this hub), `linkDevices`, then
//! `open {"kind": "hub", "url", "deviceId"}` with a device's serial, which
//! binds it and reads it. `linkStatus` is the phase and its reason;
//! `close` leaves the hub (`link_close`).
//!
//! **The socket imports** (module `dspi`), all on the Worker's socket number:
//! `dspi_ws_open(url, len, proto, len) -> i32` (Suspending: the open, or a
//! negative failure; `dspi_ws_error` has the words), `dspi_ws_send(sock,
//! binary, ptr, len) -> i32`, `dspi_ws_close(sock, code)`, `dspi_ws_poll(sock)
//! -> i32` (what is next: [`next`]) and `dspi_ws_wait(sock, ms) -> i32` (the
//! same, Suspending until something comes or `ms` pass), `dspi_ws_take(sock,
//! ptr, cap) -> i32` (the next message's bytes)
//! after `dspi_ws_len(sock) -> u32`, and `dspi_ws_closed(sock) -> i32` (the
//! close code, -1 for none).
//!
//! **Browser limits**, which the shim absorbs: a page cannot send a WebSocket
//! ping, so a ping is a cheap request the hub answers `unknown_type` (as the
//! upstream JavaScript client does); it is a round trip all the same. A page
//! may close only with 1000 or 3000-4999. A refused upgrade (the hub's
//! `Origin` rule, a full hub) looks like any failed connection to a page.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use dspi_core::{DspiCore, LinkDevice, LinkHello, LinkStatus, link_phase_key};
use dspi_link::socket::WsConnector;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

thread_local! {
    /// The serial of the hub device each session has bound.
    static BOUND: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

/// The name the hub lists this client under.
const CLIENT_NAME: &str = "uConsole for DSPi (web)";

/// What `dspi_ws_wait` says comes next.
pub mod next {
    /// Nothing within the wait.
    pub const IDLE: i32 = 0;
    /// A text message: `dspi_ws_take` it.
    pub const TEXT: i32 = 1;
    /// A binary message: `dspi_ws_take` it.
    pub const BINARY: i32 = 2;
    /// The socket closed: `dspi_ws_closed` says how.
    pub const CLOSED: i32 = 3;
}

/// The ping a page can send: a request every hub answers (`unknown_type`).
/// Its id is 0, which the client never uses, so the answer goes nowhere.
pub const PING: &str = r#"{"t":"ping","id":0}"#;

/// The close code a page may send for `code`: browsers refuse all but 1000
/// and 3000-4999.
pub fn page_close_code(code: u16) -> u16 {
    if code == 1000 || (3000..=4999).contains(&code) {
        code
    } else {
        1000
    }
}

/// The platform's WebSocket: the browser's through the imports on the web,
/// `dspi-link`'s TCP one in native tests.
fn connector() -> Arc<dyn WsConnector> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        Arc::new(js::JsConnector)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Arc::new(dspi_link::socket::TcpConnector::default())
    }
}

/// `linkConnect`'s parameters.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectParams {
    /// `ws://host:port` or `ws://host:port/dspi/v1`.
    pub url: String,
    /// The token kept for this hub; "" or absent: none.
    #[serde(default)]
    pub token: String,
    /// The hub's id that token belongs to: it goes out only to that hub.
    #[serde(default)]
    pub hub_id: String,
    /// The name the hub lists this client under.
    #[serde(default)]
    pub client_name: String,
}

/// `ws://host:port` (or `wss://`) with the protocol's path when it has none.
///
/// -- WP WB7 -- `wss://` too (review c6m N1): the browser does the TLS, so a fleet
/// server behind TLS, whose `https` page may not open `ws://` at all, carries Link to
/// its hubs as `wss://` on its own origin. Natively `TcpConnector` has no TLS; this is
/// the binding's only.
pub fn ws_url(url: &str) -> Result<String, CallError> {
    let (scheme, rest) = url
        .strip_prefix("ws://")
        .map(|r| ("ws", r))
        .or_else(|| url.strip_prefix("wss://").map(|r| ("wss", r)))
        .ok_or_else(|| CallError::new("invalid", format!("not a hub address: {url}")))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return Err(CallError::new(
            "invalid",
            format!("not a hub address: {url}"),
        ));
    }
    let path = if path.is_empty() {
        dspi_link::DEFAULT_PATH.to_owned()
    } else {
        format!("/{path}")
    };
    Ok(format!("{scheme}://{authority}{path}"))
}

/// `linkConnect`: a session with the hub at `url`, replacing any other.
pub fn connect(core: &DspiCore, p: ConnectParams) -> Result<Value, CallError> {
    let url = ws_url(&p.url)?;
    let name = if p.client_name.trim().is_empty() {
        CLIENT_NAME.to_owned()
    } else {
        p.client_name
    };
    let hello = core.link_connect_over(
        connector(),
        url,
        name,
        env!("CARGO_PKG_VERSION").to_owned(),
        p.token,
        p.hub_id,
    )?;
    Ok(hello_json(&hello))
}

/// `linkPair`'s parameters.
#[derive(Deserialize)]
pub struct PairParams {
    pub pin: String,
}

/// `linkPair`: the token to keep for this hub ("" when it issued none).
pub fn pair(core: &DspiCore, p: PairParams) -> Result<Value, CallError> {
    let token = core.link_pair(p.pin)?;
    Ok(json!({ "token": token }))
}

/// `linkDevices`: the DSPis the hub shares.
pub fn devices(core: &DspiCore) -> Value {
    Value::Array(core.link_devices().iter().map(device_json).collect())
}

/// `linkStatus`.
pub fn status(core: &DspiCore) -> Value {
    status_json(&core.link_status())
}

/// `linkTick`: one turn of the session; when to come back (ms, null: no
/// session to turn) and where it stands.
pub fn tick(core: &DspiCore) -> Value {
    let wait = core
        .link_tick()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    json!({ "waitMs": wait, "status": status_json(&core.link_status()) })
}

/// `open {"kind": "hub", "url", "deviceId"}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubTarget {
    /// The hub the session was connected to; checked, not dialled.
    #[serde(default)]
    pub url: String,
    /// The device's serial, from `linkDevices` or a fleet's own list.
    #[serde(default)]
    pub device_id: String,
}

/// Bind the hub's device with this serial and read it.
pub fn open(h: u32, core: &DspiCore, t: &HubTarget) -> Result<(), CallError> {
    forget(h);
    if t.device_id.is_empty() {
        return Err(CallError::new("invalid", "Choose a DSPi on the hub first"));
    }
    if core.link_status().phase == dspi_core::LinkPhase::Idle {
        return Err(CallError::new("not_open", "Not connected to a hub"));
    }
    let device = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == t.device_id)
        .ok_or_else(|| CallError::new("not_found", "The hub does not offer this DSPi now"))?;
    core.link_select_device(device.handle, device.serial)?;
    if let Err(e) = core.fetch_all() {
        core.disconnect();
        return Err(e.into());
    }
    BOUND.with_borrow_mut(|b| b.insert(h, t.device_id.clone()));
    Ok(())
}

/// Session `h` no longer has a hub device.
pub fn forget(h: u32) {
    BOUND.with_borrow_mut(|b| b.remove(&h));
}

/// A snapshot with the bound hub device's serial, which the core's mirror
/// does not carry, when session `h` has one.
pub fn with_serial(h: u32, mut snapshot: Value) -> Value {
    let serial = BOUND.with_borrow(|b| b.get(&h).cloned());
    if let (Some(serial), Some(d)) = (serial, snapshot.get_mut("device")) {
        d["serial"] = json!(serial);
    }
    snapshot
}

fn hello_json(h: &LinkHello) -> Value {
    json!({
        "hubId": h.hub_id,
        "hubName": h.hub_name,
        "hubKind": h.hub_kind,
        "auth": h.auth,
        "pinSource": h.pin_source,
        "needsPairing": h.needs_pairing,
        "tokenRefused": h.token_refused,
        "caps": h.caps,
        "maxInflight": h.max_inflight,
    })
}

fn device_json(d: &LinkDevice) -> Value {
    json!({
        "handle": d.handle,
        "serial": d.serial,
        "name": d.name,
        "firmware": d.fw,
        "platform": match d.platform { 0 => "RP2040", 1 => "RP2350", _ => "" },
        "state": d.state,
    })
}

fn status_json(s: &LinkStatus) -> Value {
    json!({
        "phase": link_phase_key(s.phase),
        "pinSource": s.pin_source,
        "role": s.role,
        "closeCode": s.close_code,
        "closeReason": s.close_reason,
        "reconnectAttempt": s.reconnect_attempt,
        "reconnectInMs": s.reconnect_in_ms,
        "tokenRefused": s.token_refused,
    })
}

/// The browser's WebSocket, through the Worker's imports (module `dspi`).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)]
mod js {
    use std::io;
    use std::time::Duration;

    use dspi_link::LinkError;
    use dspi_link::socket::{WsConnector, WsMessage, WsPair, WsReceiver, WsRecv, WsSender};

    use super::{PING, next, page_close_code};

    #[link(wasm_import_module = "dspi")]
    unsafe extern "C" {
        /// Open a socket; its number, or negative (`dspi_ws_error`). Suspends.
        fn dspi_ws_open(url: *const u8, url_len: u32, proto: *const u8, proto_len: u32) -> i32;
        /// The last failed open's words, up to `cap` bytes; their length.
        fn dspi_ws_error(out: *mut u8, cap: u32) -> u32;
        /// Send a message (`binary` 0: text); 0, or -1 when the socket is gone.
        fn dspi_ws_send(sock: u32, binary: u32, data: *const u8, len: u32) -> i32;
        /// Close it with `code` (1000 or 3000-4999), and forget it.
        fn dspi_ws_close(sock: u32, code: u32);
        /// What is next ([`next`]) without waiting.
        fn dspi_ws_poll(sock: u32) -> i32;
        /// What is next, waiting up to `ms` for it. Suspends.
        fn dspi_ws_wait(sock: u32, ms: u32) -> i32;
        /// The next message's length, before it is taken.
        fn dspi_ws_len(sock: u32) -> u32;
        /// Take the next message into `out`; its length, or -1 when none fits.
        fn dspi_ws_take(sock: u32, out: *mut u8, cap: u32) -> i32;
        /// The close code of a closed socket; -1 when it had none (a drop).
        fn dspi_ws_closed(sock: u32) -> i32;
    }

    /// Longest close reason or error kept.
    const WORDS: usize = 256;

    fn words(f: impl FnOnce(*mut u8, u32) -> u32) -> String {
        let mut buf = vec![0u8; WORDS];
        let n = f(buf.as_mut_ptr(), u32::try_from(WORDS).unwrap_or(0));
        buf.truncate(usize::try_from(n).unwrap_or(0).min(WORDS));
        String::from_utf8_lossy(&buf).into_owned()
    }

    pub struct JsConnector;

    impl WsConnector for JsConnector {
        fn connect(&self, url: &str, protocol: &str) -> Result<WsPair, LinkError> {
            let (u, p) = (url.as_bytes(), protocol.as_bytes());
            let ul =
                u32::try_from(u.len()).map_err(|_| LinkError::Io("address too long".into()))?;
            let pl = u32::try_from(p.len()).unwrap_or(0);
            // SAFETY: both buffers are valid for their lengths; the import only reads them.
            let sock = unsafe { dspi_ws_open(u.as_ptr(), ul, p.as_ptr(), pl) };
            let Ok(sock) = u32::try_from(sock) else {
                // SAFETY: `out` is valid for `cap` bytes; the import writes at most that.
                return Err(LinkError::Io(words(|out, cap| unsafe {
                    dspi_ws_error(out, cap)
                })));
            };
            Ok((Box::new(JsSender { sock }), Box::new(JsReceiver { sock })))
        }
    }

    struct JsSender {
        sock: u32,
    }

    impl JsSender {
        fn put(&self, binary: bool, data: &[u8]) -> io::Result<()> {
            let len =
                u32::try_from(data.len()).map_err(|_| io::Error::other("message too long"))?;
            // SAFETY: `data` is valid for `len` bytes; the import copies them.
            match unsafe { dspi_ws_send(self.sock, u32::from(binary), data.as_ptr(), len) } {
                0 => Ok(()),
                _ => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }
    }

    impl WsSender for JsSender {
        fn send(&self, msg: WsMessage) -> io::Result<()> {
            match &msg {
                WsMessage::Text(t) => self.put(false, t.as_bytes()),
                WsMessage::Binary(b) => self.put(true, b),
            }
        }

        fn ping(&self) -> io::Result<()> {
            self.put(false, PING.as_bytes())
        }

        fn close(&self, code: u16) {
            // SAFETY: no memory is passed.
            unsafe { dspi_ws_close(self.sock, u32::from(page_close_code(code))) };
        }
    }

    struct JsReceiver {
        sock: u32,
    }

    impl WsReceiver for JsReceiver {
        fn recv(&mut self) -> WsRecv {
            self.recv_within(Duration::from_secs(1))
        }

        fn recv_within(&mut self, max: Duration) -> WsRecv {
            let ms = u32::try_from(max.as_millis()).unwrap_or(u32::MAX);
            // SAFETY: no memory is passed.
            let mut kind = unsafe { dspi_ws_poll(self.sock) };
            if kind == next::IDLE && ms > 0 {
                // SAFETY: no memory is passed.
                kind = unsafe { dspi_ws_wait(self.sock, ms) };
            }
            match kind {
                next::TEXT | next::BINARY => {
                    // SAFETY: no memory is passed.
                    let len = unsafe { dspi_ws_len(self.sock) } as usize;
                    let mut buf = vec![0u8; len];
                    let cap = u32::try_from(len).unwrap_or(0);
                    // SAFETY: `buf` is valid for `cap` bytes; the import writes at most that.
                    let n = unsafe { dspi_ws_take(self.sock, buf.as_mut_ptr(), cap) };
                    let Ok(n) = usize::try_from(n) else {
                        return WsRecv::Closed {
                            code: None,
                            reason: "a message was lost".into(),
                        };
                    };
                    buf.truncate(n);
                    if kind == next::BINARY {
                        return WsRecv::Message(WsMessage::Binary(buf));
                    }
                    match String::from_utf8(buf) {
                        Ok(t) => WsRecv::Message(WsMessage::Text(t)),
                        // The browser decodes text frames; this cannot come from it
                        Err(_) => WsRecv::Idle,
                    }
                }
                next::CLOSED => {
                    // SAFETY: no memory is passed.
                    let code = unsafe { dspi_ws_closed(self.sock) };
                    WsRecv::Closed {
                        code: u16::try_from(code).ok(),
                        reason: "closed".into(),
                    }
                }
                _ => WsRecv::Idle,
            }
        }
    }
}

#[cfg(test)]
mod tests_wp_wb4b {
    use std::time::{Duration, Instant};

    use dspi_link::client::set_caller_driven_on_this_thread;
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};

    use super::*;
    use crate::calls::{call, session_new};
    use crate::guard;

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    /// `linkTick` as the Worker's timer calls it, until the phase is `phase`.
    fn tick_until(h: u32, phase: &str) -> Vec<String> {
        let start = Instant::now();
        let mut seen: Vec<String> = Vec::new();
        loop {
            let t = ok(h, "linkTick", Value::Null);
            let p = t["status"]["phase"].as_str().unwrap().to_owned();
            if seen.last() != Some(&p) {
                seen.push(p.clone());
            }
            if p == phase {
                return seen;
            }
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "no {phase}: {seen:?}"
            );
            assert!(t["waitMs"].as_u64().is_some_and(|w| w <= 250), "{t}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn wp_wb4b_the_calls_pair_list_open_write_and_follow_a_drop() {
        guard::reset_for_test();
        set_caller_driven_on_this_thread(true);
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Fixed("246810".into()),
            ..HubConfig::default()
        })
        .unwrap();
        let addr = hub.addr();
        let h = session_new().unwrap();

        let hello = ok(h, "linkConnect", json!({ "url": format!("ws://{addr}") }));
        assert_eq!(hello["needsPairing"], true);
        assert_eq!(hello["pinSource"], "fixed");
        assert_eq!(ok(h, "linkStatus", Value::Null)["phase"], "needs_pairing");
        let wrong = run(h, "linkPair", json!({ "pin": "111111" }));
        assert_eq!(wrong["code"], "wrong_pin", "{wrong}");
        let token = ok(h, "linkPair", json!({ "pin": "246810" }))["token"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!token.is_empty());
        tick_until(h, "choose_device");

        let devices = ok(h, "linkDevices", Value::Null);
        let serial = devices[0]["serial"].as_str().unwrap().to_owned();
        assert_eq!(devices[0]["state"], "online");
        let snap = ok(
            h,
            "open",
            json!({ "kind": "hub", "url": format!("ws://{addr}"), "deviceId": serial }),
        );
        assert_eq!(snap["device"]["serial"], serial.as_str());
        tick_until(h, "bound");
        let v = ok(
            h,
            "write",
            json!({ "path": "vol.master", "value": { "float": -9.5 } }),
        );
        assert_eq!(v, json!({ "float": -9.5 }), "read back");

        hub.drop_connections();
        let seen = tick_until(h, "reconnecting");
        assert!(!seen.iter().any(|p| p == "closed"), "{seen:?}");
        tick_until(h, "bound");
        assert_eq!(ok(h, "state", Value::Null)["state"]["masterVolumeDb"], -9.5);
        ok(h, "close", Value::Null);
        assert_eq!(ok(h, "linkStatus", Value::Null)["phase"], "idle");
        assert_eq!(ok(h, "linkTick", Value::Null)["waitMs"], Value::Null);

        // A second session with the token: no PIN.
        let h2 = session_new().unwrap();
        let hello = ok(
            h2,
            "linkConnect",
            json!({ "url": format!("ws://{addr}/dspi/v1"), "token": token, "hubId": hello["hubId"] }),
        );
        assert_eq!(hello["needsPairing"], false);
        tick_until(h2, "choose_device");
        let missing = run(h2, "open", json!({ "kind": "hub", "deviceId": "NOPE" }));
        assert_eq!(missing["code"], "not_found");
        ok(h2, "close", Value::Null);
        set_caller_driven_on_this_thread(false);
    }

    #[test]
    fn wp_wb4b_addresses_and_close_codes() {
        assert_eq!(
            ws_url("ws://hub.local:11915").unwrap(),
            "ws://hub.local:11915/dspi/v1"
        );
        assert_eq!(
            ws_url("ws://10.0.0.2:80/dspi/v1").unwrap(),
            "ws://10.0.0.2:80/dspi/v1"
        );
        assert!(ws_url("http://hub.local").is_err());
        assert!(ws_url("ws://").is_err());
        // -- WP WB7 -- (c6m N1) a TLS-fronted fleet server
        assert_eq!(
            ws_url("wss://fleet.example/hubs/7/dspi/v1").unwrap(),
            "wss://fleet.example/hubs/7/dspi/v1"
        );
        assert_eq!(
            ws_url("wss://fleet.example:8443").unwrap(),
            "wss://fleet.example:8443/dspi/v1"
        );
        assert!(ws_url("wss://").is_err());
        assert!(ws_url("https://fleet.example").is_err());
        assert_eq!(page_close_code(1001), 1000);
        assert_eq!(page_close_code(4000), 4000);
        assert_eq!(page_close_code(1000), 1000);
    }
}
