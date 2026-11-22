//! C functions: the MCP server for AI agents (feature `mcp`, WP MCP3), over
//! `dspi_mcp`. MPLAN section 3 and MCPDES are the design.
//!
//! **One handle, many runs.** [`FfiMcp`], from `dspi_mcp_new`, belongs to the
//! app's set of cores and outlives each run of the server: the server listens
//! exactly between `dspi_mcp_start` and `dspi_mcp_stop` (Settings, AI agents
//! on). Its listener and the pause carry over.
//!
//! **Where it listens.** `127.0.0.1` and `::1` only, on the port the start
//! names (47860 unless the user changed it). Requests from web pages, other
//! host names and unknown tokens are refused (`dspi_mcp::http`).
//!
//! **Clients.** Each agent has its own token. `dspi_mcp_add_client` makes one
//! for an agent that takes it in its configuration (shown once: the app keeps
//! only its hash). The stdio relay pairs instead: the listener gets a
//! `pairingRequested` event with a name and a 4-digit code, and
//! `dspi_mcp_pairing_answer` allows or denies it. The clients live in
//! `mcp-clients.json` in the start's `storeDir`; listing, adding and revoking
//! work while the server is stopped too, on that folder.
//!
//! **Replies.** `{"ok":true, <fields in camelCase>}` (a list as `"value"`), or
//! `{"ok":false,"code":<code>,"error":<sentence to show>}`. Codes: `invalid`,
//! `running`, `not_running`, `io` (the port), `store` (the clients' file),
//! `full`, `in_callback`.
//!
//! **Short buffers.** A text function returns the bytes needed including the
//! NUL, 0 for a null handle or a panic. `dspi_mcp_start` and
//! `dspi_mcp_add_client` act, so they check first: a null buffer or one under
//! `MCP_REPLY_MIN` returns that minimum and does nothing.
//!
//! **Secrets.** A token crosses this ABI once, in `dspi_mcp_add_client`'s
//! reply, and in the relay's token callback; nothing here logs one.
//!
//! **Threads.** Any thread. `dspi_mcp_start` binds and `dspi_mcp_stop` waits
//! for the listening threads, briefly. The listener runs on the server's
//! threads, or inside the call that caused the event (`dspi_mcp_add_client`,
//! `dspi_mcp_revoke`, `dspi_mcp_pairing_answer`: `clientsChanged`), with none
//! of this handle's state locked: it may call every function here but
//! `dspi_mcp_set_listener` and `dspi_mcp_free`, which are refused. It must not
//! wait for another thread that sets the listener.
//!
//! **The stdio relay** (`dspi_mcp_relay_stdio`) is the app's `--mcp` mode: an
//! agent host starts the app's binary with pipes, and it relays between them
//! and the app that runs the server. It needs no handle.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dspi_core::{CoreSet, DeviceId};
use dspi_mcp::auth::{AuthError, Clients, PairedClient};
use dspi_mcp::http::{DEFAULT_PORT, Event, HttpConfig, HttpServer, Listener, StartError};
use dspi_mcp::relay::{self, RelayConfig, TokenKeeper};
use dspi_mcp::{Server, ServerConfig};
use serde_json::{Value, json};

use crate::fns_core_set::{FfiCoreSet, set_arc};
use crate::fns_device::{c_str, copy_text};
use crate::guard;

/// The smallest buffer `dspi_mcp_start` and `dspi_mcp_add_client` act on.
pub const MCP_REPLY_MIN: u32 = 1024;

/// The C listener: `event_json` is one of `{"event":"pairingRequested","id",
/// "name","code"}`, `{"event":"pairingEnded","id","outcome"}` (`paired`,
/// `denied`, `expired`) and `{"event":"clientsChanged"}`.
pub type DspiMcpListener = extern "C" fn(event_json: *const c_char, user_data: *mut c_void);

/// The relay's token callback: a new token to keep, or null to forget the
/// kept one (the app refused it).
pub type DspiMcpTokenSink = extern "C" fn(token: *const c_char, user_data: *mut c_void);

struct CListener {
    callback: DspiMcpListener,
    user_data: *mut c_void,
}

// SAFETY: the callback runs on the server's threads, which its documentation
// tells the caller; `user_data` is only handed back to it.
unsafe impl Send for CListener {}
// SAFETY: as for Send.
unsafe impl Sync for CListener {}

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

fn in_callback() -> bool {
    IN_CALLBACK.with(Cell::get)
}

/// The handle behind `struct FfiMcp *`.
pub struct FfiMcp {
    set: Arc<CoreSet>,
    running: Mutex<Option<Arc<HttpServer>>>,
    listener: Arc<RwLock<Option<CListener>>>,
    /// The last start's folder, for the clients while stopped.
    store_dir: Mutex<Option<PathBuf>>,
    paused: Mutex<bool>,
    active: Mutex<Option<DeviceId>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn with_mcp<T>(p: *const FfiMcp, fail: T, f: impl FnOnce(&FfiMcp) -> T) -> T {
    if p.is_null() {
        return fail;
    }
    guard(fail, || {
        // SAFETY: a non-null handle comes from `dspi_mcp_new` and stays valid
        // until `dspi_mcp_free`; that is the C caller's contract.
        f(unsafe { &*p })
    })
}

fn failure(code: &str, error: impl std::fmt::Display) -> Value {
    json!({"ok": false, "code": code, "error": error.to_string()})
}

fn auth_failure(e: &AuthError) -> Value {
    let code = match e {
        AuthError::Full => "full",
        AuthError::Store(_) => "store",
        _ => "io",
    };
    failure(code, e)
}

fn ok(mut fields: Value) -> Value {
    if let Some(m) = fields.as_object_mut() {
        m.insert("ok".into(), json!(true));
    }
    fields
}

fn unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl FfiMcp {
    // -- WP RBFIX -- (review RB R4)
    /// The running server, `running` released: what is called on it may call
    /// the listener, which may call back in here.
    fn server(&self) -> Option<Arc<HttpServer>> {
        lock(&self.running).clone()
    }

    /// Run `f` on the clients: the running server's, or the stored ones.
    fn on_clients<T>(
        &self,
        on_server: impl FnOnce(&HttpServer) -> T,
        on_store: impl FnOnce(&mut Clients) -> T,
    ) -> Result<T, Value> {
        if let Some(s) = self.server() {
            return Ok(on_server(&s));
        }
        let Some(dir) = lock(&self.store_dir).clone() else {
            return Err(failure(
                "not_running",
                "AI agents have not been started in this run",
            ));
        };
        let mut clients = Clients::load(&dir).map_err(|e| auth_failure(&e))?;
        Ok(on_store(&mut clients))
    }

    fn start(&self, config: &str) -> Value {
        let Ok(cfg) = serde_json::from_str::<Value>(config) else {
            return failure("invalid", "the configuration is not JSON");
        };
        let port = match cfg.get("port").map(Value::as_u64) {
            None => DEFAULT_PORT,
            Some(Some(p)) if (1..=65535).contains(&p) => u16::try_from(p).unwrap_or(DEFAULT_PORT),
            Some(_) => return failure("invalid", "port must be 1 to 65535"),
        };
        let store_dir = cfg
            .get("storeDir")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from);
        let mut running = lock(&self.running);
        if running.is_some() {
            return failure("running", "AI agents are on already");
        }
        let mut server_config = ServerConfig::default();
        if let Some(v) = cfg.get("appVersion").and_then(Value::as_str) {
            server_config.version = v.to_owned();
        }
        let mut server = Server::new(Arc::clone(&self.set), server_config);
        server.set_active_device(lock(&self.active).clone());
        let listener = Arc::clone(&self.listener);
        let on_event: Listener = Arc::new(move |e: &Event| {
            let guard = listener.read().unwrap_or_else(PoisonError::into_inner);
            if let Some(l) = guard.as_ref()
                && let Ok(text) = CString::new(e.to_json().to_string())
            {
                // -- WP RBFIX -- an event of a call made inside an event's callback
                let outer = IN_CALLBACK.with(|c| c.replace(true));
                (l.callback)(text.as_ptr(), l.user_data);
                IN_CALLBACK.with(|c| c.set(outer));
            }
        });
        let http = HttpConfig {
            port,
            store_dir: store_dir.clone(),
            ..HttpConfig::default()
        };
        match HttpServer::start(server, http, Some(on_event)) {
            Ok(s) => {
                s.set_paused(*lock(&self.paused));
                let addresses: Vec<String> = s.addrs().iter().map(ToString::to_string).collect();
                let reply = ok(json!({"port": s.port(), "addresses": addresses}));
                *running = Some(Arc::new(s));
                *lock(&self.store_dir) = store_dir;
                reply
            }
            Err(StartError::Bind(e)) => failure("io", StartError::Bind(e)),
            Err(StartError::Auth(e)) => auth_failure(&e),
        }
    }
}

/// A handle for the MCP server over `set`'s devices. Null for a null set.
/// Free it with `dspi_mcp_free`, before the set.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_new(set: *const FfiCoreSet) -> *mut FfiMcp {
    guard(ptr::null_mut(), || {
        let Some(set) = set_arc(set) else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(FfiMcp {
            set,
            running: Mutex::new(None),
            listener: Arc::new(RwLock::new(None)),
            store_dir: Mutex::new(None),
            paused: Mutex::new(false),
            active: Mutex::new(None),
        }))
    })
}

/// Stop the server if it runs and free the handle. Null is ignored, and so is
/// a call from inside the listener.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_free(mcp: *mut FfiMcp) {
    if mcp.is_null() || in_callback() {
        return;
    }
    guard((), || {
        // SAFETY: a non-null handle comes from dspi_mcp_new and is not used
        // again after this call; the C caller's contract.
        let handle = unsafe { Box::from_raw(mcp) };
        if let Some(s) = lock(&handle.running).take() {
            s.stop();
        }
    });
}

/// Start listening for AI agents. `config_json`: `{"port": 47860, "storeDir":
/// "<the app's folder>", "appVersion": "0.15.0"}`, each optional (without a
/// folder the clients last only for this run). Reply `{ok, port,
/// addresses}`; code `running` when it runs already, `io` when the port is in
/// use, `store` when the clients' file is damaged (it is left as it is).
///
/// Returns the length needed; see the module's rule on short buffers.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_start(
    mcp: *mut FfiMcp,
    config_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    with_mcp(mcp, 0, |m| {
        if buf.is_null() || len < MCP_REPLY_MIN {
            return MCP_REPLY_MIN;
        }
        let reply = match c_str(config_json) {
            Some(c) => m.start(c),
            None if config_json.is_null() => m.start("{}"),
            None => failure("invalid", "the configuration is not UTF-8"),
        };
        copy_text(&reply.to_string(), buf, len)
    })
}

/// Stop listening: the agents' next requests fail, a pairing request waiting
/// ends. True if it was running.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_stop(mcp: *mut FfiMcp) -> bool {
    with_mcp(mcp, false, |m| {
        let running = lock(&m.running).take();
        running.map(|s| s.stop()).is_some()
    })
}

/// `{ok, running, port, addresses, paused, clients: [{id, name, created,
/// lastUsed}], pairing: {id, name, code} | null}`; while stopped `{ok,
/// running: false, paused}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_status(mcp: *mut FfiMcp, buf: *mut c_char, len: u32) -> u32 {
    with_mcp(mcp, 0, |m| {
        let reply = match m.server() {
            Some(s) => ok(s.status()),
            None => ok(json!({"running": false, "paused": *lock(&m.paused)})),
        };
        copy_text(&reply.to_string(), buf, len)
    })
}

/// Set the listener for the server's events (the module's list); null unsets
/// it. Once this returns, the old one is not running. Refused from inside the
/// listener.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_set_listener(
    mcp: *mut FfiMcp,
    callback: Option<extern "C" fn(event_json: *const c_char, user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    if in_callback() {
        return;
    }
    with_mcp(mcp, (), |m| {
        let mut l = m.listener.write().unwrap_or_else(PoisonError::into_inner);
        *l = callback.map(|callback| CListener {
            callback,
            user_data,
        });
    });
}

/// A client the user adds by hand under `name`: `{ok, id, token}`. Show the
/// token once; it is not kept anywhere but in the agent's configuration.
/// While stopped it goes into the last start's folder.
///
/// Returns the length needed; see the module's rule on short buffers.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_add_client(
    mcp: *mut FfiMcp,
    name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    with_mcp(mcp, 0, |m| {
        if buf.is_null() || len < MCP_REPLY_MIN {
            return MCP_REPLY_MIN;
        }
        let name = c_str(name).unwrap_or("");
        let minted = m.on_clients(|s| s.add_client(name), |c| c.mint(name, unix_s()));
        let reply = match minted {
            Ok(Ok(t)) => ok(json!({"id": t.id, "token": t.token})),
            Ok(Err(e)) => auth_failure(&e),
            Err(f) => f,
        };
        copy_text(&reply.to_string(), buf, len)
    })
}

/// The clients, `{ok, value: [{id, name, created, lastUsed}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_clients(mcp: *mut FfiMcp, buf: *mut c_char, len: u32) -> u32 {
    with_mcp(mcp, 0, |m| {
        let rows =
            |list: &[PairedClient]| -> Value { list.iter().map(PairedClient::to_json).collect() };
        let reply = match m.on_clients(|s| rows(&s.clients()), |c| rows(c.list())) {
            Ok(v) => json!({"ok": true, "value": v}),
            Err(f) => f,
        };
        copy_text(&reply.to_string(), buf, len)
    })
}

/// Remove client `id`: its token stops working at once. False if there was
/// none.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_revoke(mcp: *mut FfiMcp, id: u64) -> bool {
    with_mcp(mcp, false, |m| {
        m.on_clients(|s| s.revoke(id), |c| c.revoke(id))
            .unwrap_or(false)
    })
}

/// The user's answer to pairing request `request` (from `pairingRequested`).
/// False when it no longer waits, or the token could not be stored.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_pairing_answer(mcp: *mut FfiMcp, request: u64, allow: bool) -> bool {
    with_mcp(mcp, false, |m| {
        m.server()
            .is_some_and(|s| s.pairing_answer(request, allow).unwrap_or(false))
    })
}

/// Pause the agents (their tool calls fail with "paused by the user" and
/// change nothing) or resume them. Kept across stop and start.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_pause(mcp: *mut FfiMcp, paused: bool) {
    with_mcp(mcp, (), |m| {
        *lock(&m.paused) = paused;
        if let Some(s) = m.server() {
            s.set_paused(paused);
        }
    });
}

/// The newest `n` entries of the agents' activity, oldest first: `{ok, value:
/// [entry]}` (`dspi_mcp::activity::ActivityEntry::to_json`). Empty while
/// stopped.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_activity(mcp: *mut FfiMcp, n: u32, buf: *mut c_char, len: u32) -> u32 {
    with_mcp(mcp, 0, |m| {
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        let rows: Vec<Value> = m.server().map_or_else(Vec::new, |s| {
            s.with_server(|srv| srv.activity(n).iter().map(|e| e.to_json()).collect())
        });
        copy_text(&json!({"ok": true, "value": rows}).to_string(), buf, len)
    })
}

/// The device the app shows (a set id), which tools use when an agent names
/// none; null for none. False for an id that is no id.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_set_active_device(mcp: *mut FfiMcp, id: *const c_char) -> bool {
    with_mcp(mcp, false, |m| {
        let active = if id.is_null() {
            None
        } else {
            match c_str(id).and_then(DeviceId::parse) {
                Some(d) => Some(d),
                None => return false,
            }
        };
        *lock(&m.active) = active.clone();
        if let Some(s) = m.server() {
            s.with_server(|srv| srv.set_active_device(active));
        }
        true
    })
}

struct CKeeper {
    token: Option<String>,
    sink: Option<DspiMcpTokenSink>,
    user_data: *mut c_void,
}

impl TokenKeeper for CKeeper {
    fn load(&mut self) -> Option<String> {
        self.token.clone()
    }
    fn save(&mut self, token: &str) {
        self.token = Some(token.to_owned());
        if let (Some(sink), Ok(t)) = (self.sink, CString::new(token)) {
            sink(t.as_ptr(), self.user_data);
        }
    }
    fn forget(&mut self) {
        self.token = None;
        if let Some(sink) = self.sink {
            sink(ptr::null(), self.user_data);
        }
    }
}

/// The stdio relay, on this process's standard input and output, until
/// standard input ends: the app's `--mcp` mode. `config_json`: `{"port":
/// 47860, "token": "<the kept one>", "name": "<to pair under when the agent
/// names nothing>"}`, each optional. `token_sink` (may be null) is called on
/// this thread with a new token to keep, or null to forget the kept one.
/// Returns 0 when standard input ended, 1 when standard output failed (the
/// agent went), 2 for a bad configuration.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mcp_relay_stdio(
    config_json: *const c_char,
    token_sink: Option<extern "C" fn(token: *const c_char, user_data: *mut c_void)>,
    user_data: *mut c_void,
) -> i32 {
    guard(2, || {
        let cfg: Value = match c_str(config_json) {
            Some(c) => match serde_json::from_str(c) {
                Ok(v) => v,
                Err(_) => return 2,
            },
            None if config_json.is_null() => json!({}),
            None => return 2,
        };
        let mut relay_cfg = RelayConfig::default();
        match cfg.get("port").map(Value::as_u64) {
            None => {}
            Some(Some(p)) if (1..=65535).contains(&p) => {
                relay_cfg.port = u16::try_from(p).unwrap_or(DEFAULT_PORT)
            }
            Some(_) => return 2,
        }
        if let Some(n) = cfg
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
        {
            relay_cfg.fallback_name = n.to_owned();
        }
        let mut keeper = CKeeper {
            token: cfg
                .get("token")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
            sink: token_sink,
            user_data,
        };
        let stdin = std::io::stdin();
        let mut input: Box<dyn BufRead> = Box::new(stdin.lock());
        let mut output = std::io::stdout().lock();
        let mut log = std::io::stderr().lock();
        match relay::run(relay_cfg, &mut keeper, &mut input, &mut output, &mut log) {
            Ok(()) => 0,
            Err(e) => {
                let _ = writeln!(log, "dspi-mcp relay: stopped: {e}");
                1
            }
        }
    })
}

#[cfg(test)]
#[path = "fns_mcp/tests_wp_mcp3.rs"]
mod tests_wp_mcp3;
