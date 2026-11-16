//! The REST mirror and API keys (WP P2RS, PLAN-P2 I1 and OD-P2a/e/p/q;
//! `docs/integrator/hub-api.md` section 14).
//!
//! **A thin translator.** Each request under `/api/v1/` becomes the control-plane
//! message of the same meaning (`GET .../params/{path}` is `param.get`, ...), which runs
//! through the very handlers a WebSocket message does ([`crate::server::control_plane`]:
//! the lock's, the guarded apply's, the limits' and the services'), on the session of
//! the key that asked. The answer is the WebSocket answer without `t` and `id`, plus
//! `v` and `hub`; an error keeps its code and gets the HTTP status of section 14.2.
//! Nothing here judges a write itself: the role, the limits, the lock and the voltage
//! rule judge it, as for any client.
//!
//! **Keys are principals (PLAN-P2 M2).** An API key is a paired client of kind
//! [`ClientKind::Api`](crate::store::ClientKind) in the one table ([`AuthStore`]),
//! minted by an Admin with `auth.mint` and shown once. Each key that is in use has one
//! session, opened with [`HubHandle::open_principal`] (a remote-range id, the key's
//! client id and name, so attribution says who wrote), kept for
//! [`ServerLimits::rest_idle`] after its last request so the parameter API's core for it
//! stays warm, and closed when the key is revoked. A key's requests run one at a time,
//! as a WebSocket's do. Its outbox (the events every session is sent) is emptied after
//! each request and by [`Rest::sweep`], so it never fills.
//!
//! **What a peer on the LAN can cost** (on top of the server's HTTP limits):
//!
//! | Cost | Bound |
//! |---|---|
//! | A browser page | Any request with an `Origin` header is 403, before anything else is read |
//! | Guessing keys | [`AuthStore::verify_token`]'s per-address failure limit (429 past it); a key is 32 random bytes, compared as a hash in constant time |
//! | A body | `Content-Length` only (no chunked), at most [`ServerLimits::rest_max_body`] (413 unread past it), all of it within [`ServerLimits::rest_body_timeout`] of the head (408), read only once the key is known |
//! | One key, many requests | [`ServerLimits::rest_rate`] a second on average, [`ServerLimits::rest_burst`] at once (429 with `Retry-After`) |
//! | Memory | One entry per key that asked, at most [`crate::auth::MAX_PAIRED_CLIENTS`] that verified, dropped when idle |
//!
//! **TLS: SECSRV.** The request is read and answered over [`HubStream`], plain or TLS;
//! the server checks `Host` for every route. -- WP SECSRV -- Over plaintext on a hub
//! that speaks TLS, a key not minted `plaintext_ok` is refused with `426`
//! `tls_required` once it is known, before its request is read on (PLAN-SEC 4.7).
//!
//! Lock level: [`Rest`]'s table is a leaf. No hub call is made under it.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::auth::{AuthError, AuthStore};
use crate::hub::HubHandle;
use crate::server::{HubStream, ServerLimits, control_plane};
use crate::session::{Outbound, Pop, Role, Session};
use crate::store::{ClientKind, parse_role, role_name};

/// The `hello` cap of API keys (`auth.mint`, `kind` in `auth.list`). Always there.
pub const CAP_API_KEYS: &str = "api_keys";
/// The `hello` cap of the REST mirror, while the hub runs with it on.
pub const CAP_REST: &str = "rest";
/// The largest body a REST request may have (PLAN-P2 P2RS): 64 KiB.
pub const MAX_BODY: usize = 64 * 1024;
/// Where the mirror lives, outside the protocol's `/dspi/` (OD-P2a).
pub const PREFIX: &str = "/api/v1";
/// `v` in every answer.
const VERSION: u64 = 1;
/// The id every translated request carries; the answer's is dropped.
const REQUEST_ID: u64 = 1;
/// At most this many items are taken out of a key's outbox at once: a device that
/// floods its notifications cannot keep the emptier going.
const MAX_DRAIN: usize = 4096;
/// How often [`Rest::sweep`] looks at the keys, at most.
const SWEEP_EVERY: Duration = Duration::from_secs(1);

const JSON: (&str, &str) = ("Content-Type", "application/json");
const TEXT: (&str, &str) = ("Content-Type", "text/plain; charset=utf-8");

/// Whether a request's path is the mirror's.
pub(crate) fn is_rest_path(path: &str) -> bool {
    path == PREFIX
        || path
            .strip_prefix(PREFIX)
            .is_some_and(|p| p.starts_with('/'))
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn after(now: Instant, wait: Duration) -> Instant {
    now.checked_add(wait).unwrap_or(now)
}

/// What a request is answered with: the server writes it.
#[derive(Debug)]
pub(crate) struct Reply {
    /// `"200 OK"` and so on.
    pub(crate) status: &'static str,
    /// Beside `Content-Length` and the connection's.
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Vec<u8>,
    /// Whether the connection may carry another request: never after a body, read or
    /// not, so nothing of one is ever taken for the next request's head.
    pub(crate) keep: bool,
}

/// What a request is answered from.
pub(crate) struct RestCtx<'a> {
    pub(crate) hub: &'a HubHandle,
    pub(crate) auth: &'a AuthStore,
    pub(crate) hub_id: &'a str,
    pub(crate) limits: &'a ServerLimits,
    // -- WP SECSRV --
    /// The request came over TLS.
    pub(crate) encrypted: bool,
    /// Every API key is taken over plaintext (TLS off, or upstream compatibility),
    /// not only one minted `plaintext_ok` (PLAN-SEC 4.7, OD-SECk).
    pub(crate) any_key_in_plaintext: bool,
}

/// A key that has asked lately.
#[derive(Debug)]
struct Key {
    /// Its principal, once one is open.
    session: Option<Arc<Session>>,
    /// Its last request.
    used: Instant,
    /// A request of it is running.
    busy: bool,
    /// Its rate allowance, in thousandths of a request.
    allowance: u64,
    /// Up to when the allowance is counted.
    counted: Instant,
}

/// The mirror's state, one per server: whether it is on, and the keys in use.
#[derive(Debug)]
pub(crate) struct Rest {
    on: bool,
    keys: Mutex<BTreeMap<u32, Key>>,
    freed: Condvar,
    swept: Mutex<Option<Instant>>,
}

/// Marks a key's request done however it ends.
struct Busy<'a> {
    rest: &'a Rest,
    cid: u32,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        if let Some(key) = guard(&self.rest.keys).get_mut(&self.cid) {
            key.busy = false;
            key.used = Instant::now();
        }
        self.rest.freed.notify_all();
    }
}

/// Throw away what a key's session has been sent: no one reads it. `false` when the
/// session is closed (revoked, the hub stopping, or it fell so far behind that its
/// outbox closed), so it must not be used again.
fn empty_outbox(session: &Session) -> bool {
    for _ in 0..MAX_DRAIN {
        match session.outbox().pop(Duration::ZERO) {
            Pop::Item(Outbound::Close { .. }) | Pop::Closed => return false,
            Pop::Item(_) => {}
            Pop::Idle => break,
        }
    }
    !session.outbox().is_closed()
}

impl Rest {
    pub(crate) fn new(on: bool) -> Self {
        Self {
            on,
            keys: Mutex::new(BTreeMap::new()),
            freed: Condvar::new(),
            swept: Mutex::new(None),
        }
    }

    /// Whether `/api/v1/` is served.
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// The open session of key `cid`, for a revoke or a role change.
    pub(crate) fn session_of(&self, cid: u32) -> Option<Arc<Session>> {
        guard(&self.keys)
            .get(&cid)
            .and_then(|k| k.session.clone())
            .filter(|s| !s.outbox().is_closed())
    }

    /// Whether key `cid` has a session open now (`auth.list`'s `online`).
    pub(crate) fn is_open(&self, cid: u32) -> bool {
        self.session_of(cid).is_some()
    }

    /// Close the sessions of keys idle for [`ServerLimits::rest_idle`], forget those
    /// keys, and empty the outboxes of the others. On the accept thread, at most once a
    /// [`SWEEP_EVERY`]; never blocks on a request.
    pub(crate) fn sweep(&self, hub: &HubHandle, limits: &ServerLimits) {
        if !self.on {
            return;
        }
        let now = Instant::now();
        {
            let mut swept = guard(&self.swept);
            if swept.is_some_and(|at| now.saturating_duration_since(at) < SWEEP_EVERY) {
                return;
            }
            *swept = Some(now);
        }
        let mut close = Vec::new();
        let mut empty = Vec::new();
        {
            let mut keys = guard(&self.keys);
            keys.retain(|_, k| {
                let idle = now.saturating_duration_since(k.used) >= limits.rest_idle;
                if k.busy || !idle {
                    empty.extend(k.session.clone());
                    return true;
                }
                close.extend(k.session.take());
                false
            });
        }
        for session in empty {
            if !empty_outbox(&session) {
                close.push(session);
            }
        }
        for session in close {
            hub.close_session(session.id());
        }
    }

    /// Answer one request under `/api/v1/`. `tail` is what came behind its head; a
    /// body is taken from it first.
    pub(crate) fn answer<S: HubStream + ?Sized>(
        &self,
        ctx: &RestCtx<'_>,
        stream: &mut S,
        request: &httparse::Request<'_, '_>,
        tail: &mut Vec<u8>,
    ) -> Reply {
        if !self.on {
            return Reply {
                status: "404 Not Found",
                headers: vec![(TEXT.0, TEXT.1.to_owned())],
                body: b"Not found.".to_vec(),
                keep: true,
            };
        }
        let fail = |status, code, msg: &str| error_reply(ctx.hub_id, status, code, msg);
        // First, so a page in a browser learns as little as it can (OD-P2e, review S2).
        if request
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("Origin"))
        {
            log::debug!("hub: refused a REST request from a browser page");
            return fail(FORBIDDEN, "denied", "no browser page may use the REST API");
        }
        let Ok(peer) = stream.peer_ip() else {
            return fail(BAD_REQUEST, "bad_request", "no peer");
        };
        let cid_role = match self.authenticate(ctx, request, peer) {
            Ok(cid_role) => cid_role,
            Err(reply) => return reply,
        };
        let (cid, role) = cid_role;
        if let Err(wait_ms) = self.take_allowance(ctx.limits, cid) {
            let mut reply = error_reply(
                ctx.hub_id,
                TOO_MANY,
                "rate_limited",
                "too many requests with this key",
            );
            let secs = wait_ms.div_ceil(1000).max(1);
            reply.headers.push(("Retry-After", secs.to_string()));
            set_field(&mut reply, "retry_in_ms", wait_ms.into());
            return reply;
        }
        let target = request.path.unwrap_or_default();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let method = request.method.unwrap_or_default();
        let endpoint = match route(path) {
            Some((allowed, endpoint)) if allowed.contains(&method) => endpoint,
            Some((allowed, _)) => {
                let mut reply = fail(
                    "405 Method Not Allowed",
                    "bad_request",
                    "this path does not take that method",
                );
                reply.headers.push(("Allow", allowed.join(", ")));
                return reply;
            }
            None => return fail(NOT_FOUND, "not_found", "no such REST path"),
        };
        let query = match parse_query(query) {
            Ok(query) => query,
            Err(why) => return fail(BAD_REQUEST, "bad_request", &why),
        };
        let body = if method == "GET" {
            Map::new()
        } else {
            match read_body(ctx, stream, request, tail, method == "PUT") {
                Ok(body) => body,
                Err(reply) => return reply,
            }
        };
        let mut message = match endpoint.message(&query, &body) {
            Ok(message) => message,
            Err(why) => return fail(BAD_REQUEST, "bad_request", &why),
        };
        // -- WP P2CFIXA -- `apply.*` addresses an attached device by its handle
        if let Endpoint::Apply { serial, .. } = &endpoint {
            let device = ctx
                .hub
                .devices()
                .into_iter()
                .find(|d| d.identity.serial == *serial);
            match device {
                Some(d) if d.state == crate::registry::DeviceState::Online => {
                    if let Some(m) = message.as_object_mut() {
                        m.insert("handle".into(), d.handle.into());
                    }
                }
                Some(_) => return fail("409 Conflict", "offline", "the device is not attached"),
                None => return fail(NOT_FOUND, "not_found", "no such device attached"),
            }
        }
        let answer = match self.run(ctx, cid, role, &message) {
            Ok(answer) => answer,
            Err(reply) => return reply,
        };
        let mut reply = translate(ctx.hub_id, &answer);
        reply.keep = method == "GET";
        reply
    }

    /// The key's `(cid, role)`, from `Authorization: Bearer <key>`.
    fn authenticate(
        &self,
        ctx: &RestCtx<'_>,
        request: &httparse::Request<'_, '_>,
        peer: IpAddr,
    ) -> Result<(u32, Role), Reply> {
        let unauthenticated = |msg: &str| {
            let mut reply = error_reply(ctx.hub_id, UNAUTHORIZED, "unauthenticated", msg);
            reply
                .headers
                .push(("WWW-Authenticate", "Bearer".to_owned()));
            reply
        };
        let mut values = request
            .headers
            .iter()
            .filter(|h| h.name.eq_ignore_ascii_case("Authorization"));
        let (Some(header), None) = (values.next(), values.next()) else {
            return Err(unauthenticated(
                "one Authorization: Bearer header is required",
            ));
        };
        let key = std::str::from_utf8(header.value)
            .ok()
            .and_then(|v| {
                let (scheme, key) = v.trim().split_once(' ')?;
                scheme.eq_ignore_ascii_case("Bearer").then(|| key.trim())
            })
            .filter(|k| !k.is_empty());
        let Some(key) = key else {
            return Err(unauthenticated("Authorization: Bearer <key> is required"));
        };
        match ctx.auth.verify_token(key, peer, Instant::now()) {
            // -- WP P2CFIXA -- a console's token is not a REST bearer: only an API key
            Ok(authed) if authed.kind != ClientKind::Api => Err(unauthenticated(
                "a console's token is not an API key: mint one with auth.mint",
            )),
            // -- WP SECSRV -- the key is good; may it be used in the clear?
            Ok(authed)
                if !ctx.encrypted
                    && !ctx.any_key_in_plaintext
                    && !plaintext_ok(ctx, authed.cid) =>
            {
                log::debug!("hub: refused an API key over plaintext");
                let mut reply = error_reply(
                    ctx.hub_id,
                    "426 Upgrade Required",
                    crate::server::TLS_REQUIRED,
                    "this API key works over https only (it was not minted plaintext_ok)",
                );
                reply.headers.push(("Upgrade", "TLS/1.3".to_owned()));
                Err(reply)
            }
            Ok(authed) => Ok((authed.cid, authed.role)),
            Err(AuthError::RateLimited) => {
                let mut reply = error_reply(
                    ctx.hub_id,
                    TOO_MANY,
                    "rate_limited",
                    "too many wrong keys from this address",
                );
                reply.headers.push(("Retry-After", "60".to_owned()));
                Err(reply)
            }
            Err(_) => Err(unauthenticated("unknown or revoked key")),
        }
    }

    /// Spend one request of key `cid`'s allowance; `Err` is how long until there is
    /// one, in ms. A rate of 0 is no limit.
    fn take_allowance(&self, limits: &ServerLimits, cid: u32) -> Result<(), u64> {
        const ONE: u64 = 1000;
        let now = Instant::now();
        let rate = u64::from(limits.rest_rate);
        let full = u64::from(limits.rest_burst.max(1)).saturating_mul(ONE);
        let mut keys = guard(&self.keys);
        let key = keys.entry(cid).or_insert_with(|| Key {
            session: None,
            used: now,
            busy: false,
            allowance: full,
            counted: now,
        });
        key.used = now;
        if rate == 0 {
            return Ok(());
        }
        let ms = u64::try_from(now.saturating_duration_since(key.counted).as_millis())
            .unwrap_or(u64::MAX);
        // `rate` a second is `rate` thousandths a millisecond. Only whole milliseconds
        // are counted, so none is lost between two requests close together.
        key.allowance = key
            .allowance
            .saturating_add(ms.saturating_mul(rate))
            .min(full);
        key.counted = after(key.counted, Duration::from_millis(ms));
        if key.allowance >= ONE {
            key.allowance = key.allowance.saturating_sub(ONE);
            return Ok(());
        }
        let short = ONE.saturating_sub(key.allowance);
        Err(short.div_ceil(rate).max(1))
    }

    /// Run `message` on key `cid`'s session, one request of a key at a time: the
    /// answer's text, or the reply when the key is busy past the response time or the
    /// session cannot be had.
    fn run(
        &self,
        ctx: &RestCtx<'_>,
        cid: u32,
        role: Role,
        message: &Value,
    ) -> Result<String, Reply> {
        let until = after(Instant::now(), ctx.limits.response_timeout);
        // Held to the end of this call, whichever way it ends.
        let _turn = self.wait_turn(ctx.hub_id, cid, until)?;
        let session = self.session(ctx, cid, role)?;
        let answer = control_plane(ctx.hub, &session, message).unwrap_or_else(|| {
            // Not one of the families: a hub without it (a service off).
            json!({"t": "err", "id": REQUEST_ID, "code": "unknown_type",
                   "msg": "this hub does not run that family"})
            .to_string()
        });
        // What the request made the hub send this session (its own family's events).
        if !empty_outbox(&session) {
            ctx.hub.close_session(session.id());
        }
        Ok(answer)
    }

    /// Wait until no other request of key `cid` runs, then mark one running.
    fn wait_turn(&self, hub_id: &str, cid: u32, until: Instant) -> Result<Busy<'_>, Reply> {
        let mut keys = guard(&self.keys);
        loop {
            let Some(key) = keys.get_mut(&cid) else {
                // Swept between the allowance and here: it starts again.
                keys.insert(
                    cid,
                    Key {
                        session: None,
                        used: Instant::now(),
                        busy: true,
                        allowance: 0,
                        counted: Instant::now(),
                    },
                );
                return Ok(Busy { rest: self, cid });
            };
            if !key.busy {
                key.busy = true;
                return Ok(Busy { rest: self, cid });
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(error_reply(
                    hub_id,
                    UNAVAILABLE,
                    "busy",
                    "another request with this key is still running",
                ));
            }
            keys = self
                .freed
                .wait_timeout(keys, left)
                .map(|(k, _)| k)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Key `cid`'s session, opened if it has none (or a closed one), with `role` now.
    /// Its turn is held, so no other request opens one meanwhile.
    fn session(&self, ctx: &RestCtx<'_>, cid: u32, role: Role) -> Result<Arc<Session>, Reply> {
        let open = guard(&self.keys)
            .get(&cid)
            .and_then(|k| k.session.clone())
            .filter(|s| !s.outbox().is_closed());
        if let Some(session) = open {
            session.set_role(role);
            return Ok(session);
        }
        let name = ctx
            .auth
            .clients()
            .into_iter()
            .find(|c| c.cid == cid)
            .map(|c| c.name)
            .unwrap_or_default();
        let session = ctx
            .hub
            .open_principal(role, Some(cid), &name)
            .map_err(|_| error_reply(ctx.hub_id, UNAVAILABLE, "busy", "the hub is stopping"))?;
        if let Some(key) = guard(&self.keys).get_mut(&cid) {
            let old = key.session.replace(Arc::clone(&session));
            if let Some(old) = old {
                ctx.hub.close_session(old.id());
            }
        }
        // Listed first, then asked, as a WebSocket does: a revoke either finds the
        // session above, or has already happened and is seen here.
        match ctx.auth.role_of(cid) {
            Some(now) => {
                session.set_role(now);
                Ok(session)
            }
            None => {
                ctx.hub.close_session(session.id());
                Err(error_reply(
                    ctx.hub_id,
                    UNAUTHORIZED,
                    "unauthenticated",
                    "unknown or revoked key",
                ))
            }
        }
    }
}

const BAD_REQUEST: &str = "400 Bad Request";
const UNAUTHORIZED: &str = "401 Unauthorized";
const FORBIDDEN: &str = "403 Forbidden";
const NOT_FOUND: &str = "404 Not Found";
const TOO_MANY: &str = "429 Too Many Requests";
const UNAVAILABLE: &str = "503 Service Unavailable";

/// The HTTP status of an answer's code (hub-api.md section 14.2).
fn status_of(code: &str) -> &'static str {
    match code {
        "bad_request" => BAD_REQUEST,
        "unauthenticated" => UNAUTHORIZED,
        "denied"
        | "limit_exceeded"
        | "installer_lock"
        | "image_refused"
        | "confirmation_required" => FORBIDDEN,
        "not_found" | "no_device" => NOT_FOUND,
        "conflict" | "offline" => "409 Conflict",
        "locked" => "423 Locked",
        "rate_limited" => TOO_MANY,
        "unsupported" | "unknown_type" => "501 Not Implemented",
        "busy" | "device" => UNAVAILABLE,
        _ => "500 Internal Server Error",
    }
}

fn json_reply(status: &'static str, body: &Value) -> Reply {
    Reply {
        status,
        headers: vec![
            (JSON.0, JSON.1.to_owned()),
            ("Cache-Control", "no-store".to_owned()),
            ("X-Content-Type-Options", "nosniff".to_owned()),
        ],
        body: body.to_string().into_bytes(),
        keep: false,
    }
}

/// `{"v": 1, "hub": …, "code": …, "msg": …}`.
// -- WP SECSRV --
/// Whether API key `cid` was minted `plaintext_ok`.
fn plaintext_ok(ctx: &RestCtx<'_>, cid: u32) -> bool {
    ctx.auth
        .clients()
        .iter()
        .any(|c| c.cid == cid && c.plaintext_ok)
}

fn error_reply(hub_id: &str, status: &'static str, code: &str, msg: &str) -> Reply {
    json_reply(
        status,
        &json!({"v": VERSION, "hub": hub_id, "code": code, "msg": msg}),
    )
}

/// Add one field to a JSON reply's body.
fn set_field(reply: &mut Reply, name: &str, value: Value) {
    if let Ok(Value::Object(mut map)) = serde_json::from_slice::<Value>(&reply.body) {
        map.insert(name.to_owned(), value);
        reply.body = Value::Object(map).to_string().into_bytes();
    }
}

/// A handler's answer as a REST reply: its body without `t` and `id`, with `v` and
/// `hub`; an `err`'s code decides the status.
fn translate(hub_id: &str, answer: &str) -> Reply {
    let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(answer) else {
        return error_reply(hub_id, "500 Internal Server Error", "internal", "no answer");
    };
    let t = map.remove("t");
    map.remove("id");
    map.entry("v").or_insert(VERSION.into());
    map.insert("hub".into(), hub_id.into());
    let status = match t.as_ref().and_then(Value::as_str) {
        Some("ok") => "200 OK",
        _ => status_of(
            map.get("code")
                .and_then(Value::as_str)
                .unwrap_or("internal"),
        ),
    };
    json_reply(status, &Value::Object(map))
}

/// A request's body as a JSON object (empty for none): `Content-Length` only, at most
/// [`ServerLimits::rest_max_body`], all of it within [`ServerLimits::rest_body_timeout`].
/// `required`: a missing length is 411 rather than no body.
fn read_body<S: HubStream + ?Sized>(
    ctx: &RestCtx<'_>,
    stream: &mut S,
    request: &httparse::Request<'_, '_>,
    tail: &mut Vec<u8>,
    required: bool,
) -> Result<Map<String, Value>, Reply> {
    let fail = |status, msg: &str| error_reply(ctx.hub_id, status, "bad_request", msg);
    const LENGTH_REQUIRED: &str = "411 Length Required";
    if request
        .headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("Transfer-Encoding"))
    {
        return Err(fail(
            LENGTH_REQUIRED,
            "Content-Length only, no Transfer-Encoding",
        ));
    }
    let mut lengths = request
        .headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("Content-Length"));
    let length = match (lengths.next(), lengths.next()) {
        (None, _) if required => return Err(fail(LENGTH_REQUIRED, "Content-Length is required")),
        (None, _) => 0,
        (Some(_), Some(_)) => return Err(fail(BAD_REQUEST, "one Content-Length")),
        (Some(h), None) => match parse_length(h.value) {
            Some(n) => n,
            None => return Err(fail(BAD_REQUEST, "Content-Length is not a length")),
        },
    };
    if length > ctx.limits.rest_max_body {
        return Err(fail(
            "413 Content Too Large",
            &format!("a body is at most {} bytes", ctx.limits.rest_max_body),
        ));
    }
    let mut body: Vec<u8> = Vec::with_capacity(length);
    let from_tail = tail.len().min(length);
    body.extend(tail.drain(..from_tail));
    let deadline = after(Instant::now(), ctx.limits.rest_body_timeout);
    let mut chunk = [0u8; 4096];
    while body.len() < length {
        let left = deadline.saturating_duration_since(Instant::now());
        let timed_out = || fail("408 Request Timeout", "the body did not arrive in time");
        if left.is_zero() {
            return Err(timed_out());
        }
        if stream.set_read_timeout(Some(left)).is_err() {
            return Err(timed_out());
        }
        let want = length.saturating_sub(body.len()).min(chunk.len());
        let Some(buf) = chunk.get_mut(..want) else {
            return Err(timed_out());
        };
        match stream.read(buf) {
            // Gone: nobody to answer, but the server's write fails quietly.
            Ok(0) => return Err(fail(BAD_REQUEST, "the body ended early")),
            Ok(n) => body.extend_from_slice(buf.get(..n).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(timed_out()),
        }
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(fail(BAD_REQUEST, "the body must be a JSON object")),
        Err(e) => Err(fail(BAD_REQUEST, &format!("the body is not JSON: {e}"))),
    }
}

/// A `Content-Length` value: digits only, nothing signed, spaced or listed.
fn parse_length(value: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(value).ok()?.trim();
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// `%XX` decoded, as UTF-8. `+` stays a `+`: this is a path or a query value, not a form.
fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = bytes
                .get(i.saturating_add(1)..i.saturating_add(3))
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            let Some(byte) = hex else {
                return Err(format!("a bad %-escape in {text:?}"));
            };
            out.push(byte);
            i = i.saturating_add(3);
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    String::from_utf8(out).map_err(|_| format!("{text:?} is not UTF-8"))
}

fn parse_query(query: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(name)?, percent_decode(value)?);
    }
    Ok(map)
}

/// One endpoint of section 14.2, with what its path named.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Endpoint {
    Devices,
    Param { serial: String, path: String },
    PresetLoad { serial: String, slot: String },
    Recall { serial: String, snapshot: String },
    Events { serial: String },
    // -- WP P2CFIXA -- a guarded apply's end: `apply.confirm` or `apply.cancel`
    Apply { serial: String, confirm: bool },
    Alarms,
    AlarmAck { alarm: String },
    Health,
}

const GET: &[&str] = &["GET"];
const GET_PUT: &[&str] = &["GET", "PUT"];
const POST: &[&str] = &["POST"];

/// The endpoint a path names, with the methods it takes. `None`: no such path.
fn route(path: &str) -> Option<(&'static [&'static str], Endpoint)> {
    let rest = path.strip_prefix(PREFIX)?.strip_prefix('/')?;
    let segments: Vec<String> = rest
        .split('/')
        .map(percent_decode)
        .collect::<Result<_, _>>()
        .ok()?;
    let segs: Vec<&str> = segments.iter().map(String::as_str).collect();
    let owned = |s: &str| s.to_owned();
    Some(match segs.as_slice() {
        ["devices"] => (GET, Endpoint::Devices),
        ["devices", serial, "params", param] if !serial.is_empty() && !param.is_empty() => (
            GET_PUT,
            Endpoint::Param {
                serial: owned(serial),
                path: owned(param),
            },
        ),
        ["devices", serial, "presets", slot, "load"] if !serial.is_empty() => (
            POST,
            Endpoint::PresetLoad {
                serial: owned(serial),
                slot: owned(slot),
            },
        ),
        ["devices", serial, "snapshots", snapshot, "recall"]
            if !serial.is_empty() && !snapshot.is_empty() =>
        {
            (
                POST,
                Endpoint::Recall {
                    serial: owned(serial),
                    snapshot: owned(snapshot),
                },
            )
        }
        ["devices", serial, "events"] if !serial.is_empty() => (
            GET,
            Endpoint::Events {
                serial: owned(serial),
            },
        ),
        // -- WP P2CFIXA --
        ["devices", serial, "apply", end @ ("confirm" | "cancel")] if !serial.is_empty() => (
            POST,
            Endpoint::Apply {
                serial: owned(serial),
                confirm: *end == "confirm",
            },
        ),
        ["alarms"] => (GET, Endpoint::Alarms),
        ["alarms", alarm, "ack"] if !alarm.is_empty() => (
            POST,
            Endpoint::AlarmAck {
                alarm: owned(alarm),
            },
        ),
        ["health"] => (GET, Endpoint::Health),
        _ => return None,
    })
}

/// A whole number from a path or query.
fn number(what: &str, text: &str) -> Result<u64, String> {
    text.trim()
        .parse::<u64>()
        .map_err(|_| format!("{what} must be a whole number, not {text:?}"))
}

impl Endpoint {
    /// The control-plane message this request is.
    fn message(
        &self,
        query: &BTreeMap<String, String>,
        body: &Map<String, Value>,
    ) -> Result<Value, String> {
        let mut m = Map::new();
        let mut put = |name: &str, value: Value| {
            m.insert(name.to_owned(), value);
        };
        let from_query = |name: &str| query.get(name).map(String::as_str);
        let t = match self {
            // -- WP P2CFIXA -- paged and filtered as `inventory.list` is
            Self::Devices => {
                if let Some(serial) = from_query("serial") {
                    put("serial", serial.into());
                }
                for name in ["offset", "limit"] {
                    if let Some(n) = from_query(name) {
                        put(name, number(name, n)?.into());
                    }
                }
                "inventory.list"
            }
            // -- WP P2CFIXA -- the serial becomes the device's handle in `answer`
            Self::Apply { serial, confirm } => {
                put("serial", serial.as_str().into());
                if *confirm {
                    "apply.confirm"
                } else {
                    "apply.cancel"
                }
            }
            Self::Param { serial, path } => {
                put("serial", serial.as_str().into());
                put("path", path.as_str().into());
                if let Some(idx) = from_query("idx") {
                    let idx: Vec<Value> = idx
                        .split(',')
                        .filter(|i| !i.trim().is_empty())
                        .map(|i| number("idx", i).map(Value::from))
                        .collect::<Result<_, _>>()?;
                    put("idx", Value::Array(idx));
                }
                if body.is_empty() {
                    "param.get"
                } else {
                    // Only what the body may say: never the device, the path or `t`.
                    for name in ["value", "confirm"] {
                        if let Some(v) = body.get(name) {
                            put(name, v.clone());
                        }
                    }
                    "param.set"
                }
            }
            Self::PresetLoad { serial, slot } => {
                put("serial", serial.as_str().into());
                put("slot", number("the preset slot", slot)?.into());
                "preset.load"
            }
            Self::Recall { serial, snapshot } => {
                put("serial", serial.as_str().into());
                put("snapshot", snapshot.as_str().into());
                for name in ["guard", "timeout_ms"] {
                    if let Some(v) = body.get(name) {
                        put(name, v.clone());
                    }
                }
                "snapshot.recall"
            }
            Self::Events { serial } => {
                put("serial", serial.as_str().into());
                if let Some(after) = from_query("after") {
                    put("after_id", number("after", after)?.into());
                }
                if let Some(limit) = from_query("limit") {
                    put("limit", number("limit", limit)?.into());
                }
                "event.list"
            }
            Self::Alarms => {
                for name in ["serial", "state"] {
                    if let Some(v) = from_query(name) {
                        put(name, v.into());
                    }
                }
                "alarm.list"
            }
            Self::AlarmAck { alarm } => {
                put("alarm", alarm.as_str().into());
                "alarm.ack"
            }
            Self::Health => {
                if let Some(serial) = from_query("serial") {
                    put("serial", serial.into());
                }
                "health.get"
            }
        };
        m.insert("t".into(), t.into());
        m.insert("id".into(), REQUEST_ID.into());
        Ok(Value::Object(m))
    }
}

/// `auth.mint {name, role, plaintext_ok?}` from a session with `caller`'s role (the
/// server has checked it is Admin): an API key, in this answer only. Never above the
/// caller's role.
pub(crate) fn mint(
    auth: &AuthStore,
    id: Option<u64>,
    message: &Map<String, Value>,
    caller: Role,
) -> String {
    let answer = |mut body: Map<String, Value>| {
        if let Some(id) = id {
            body.insert("id".into(), id.into());
        }
        body.insert("v".into(), VERSION.into());
        Value::Object(body).to_string()
    };
    let err = |code: &str, msg: &str| {
        let mut m = Map::new();
        m.insert("t".into(), "err".into());
        m.insert("code".into(), code.into());
        m.insert("msg".into(), msg.into());
        answer(m)
    };
    let name = message.get("name").and_then(Value::as_str);
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .and_then(parse_role);
    let (Some(name), Some(role)) = (name, role) else {
        return err(
            "bad_request",
            "name and role (viewer, control or admin) are required",
        );
    };
    let plaintext_ok = match message.get("plaintext_ok") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return err("bad_request", "plaintext_ok is true or false"),
    };
    if role > caller {
        return err(
            "denied",
            "a key's role is never above the minting session's",
        );
    }
    match auth.mint_api_key(name, role, plaintext_ok) {
        Ok(minted) => {
            let mut m = Map::new();
            m.insert("t".into(), "ok".into());
            m.insert("cid".into(), minted.cid.into());
            m.insert("name".into(), minted.name.into());
            m.insert("role".into(), role_name(minted.role).into());
            m.insert("kind".into(), ClientKind::Api.wire_name().into());
            m.insert("plaintext_ok".into(), minted.plaintext_ok.into());
            m.insert("key".into(), minted.key.into());
            answer(m)
        }
        Err(AuthError::TableFull) => err(
            "conflict",
            "64 paired clients and API keys at most: revoke one first",
        ),
        Err(_) => err("internal", "no key could be made or stored"),
    }
}

#[cfg(test)]
mod tests_wp_p2rs {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn wp_p2rs_paths_are_the_mirrors_only_under_api_v1() {
        assert!(is_rest_path("/api/v1"));
        assert!(is_rest_path("/api/v1/devices"));
        assert!(!is_rest_path("/api/v10"));
        assert!(!is_rest_path("/api"));
        assert!(!is_rest_path("/dspi/v1"));
    }

    #[test]
    fn wp_p2rs_every_endpoint_routes_with_its_methods() {
        let (m, e) = route("/api/v1/devices").unwrap();
        assert_eq!((m, e), (GET, Endpoint::Devices));
        let (m, e) = route("/api/v1/devices/E466/params/vol.master").unwrap();
        assert_eq!(m, GET_PUT);
        assert_eq!(
            e,
            Endpoint::Param {
                serial: "E466".into(),
                path: "vol.master".into()
            }
        );
        assert_eq!(
            route("/api/v1/devices/E466/presets/3/load").unwrap().1,
            Endpoint::PresetLoad {
                serial: "E466".into(),
                slot: "3".into()
            }
        );
        assert_eq!(
            route("/api/v1/devices/E466/snapshots/a%20b/recall")
                .unwrap()
                .1,
            Endpoint::Recall {
                serial: "E466".into(),
                snapshot: "a b".into()
            }
        );
        assert_eq!(
            route("/api/v1/devices/E466/events").unwrap().1,
            Endpoint::Events {
                serial: "E466".into()
            }
        );
        assert_eq!(route("/api/v1/alarms").unwrap().1, Endpoint::Alarms);
        assert_eq!(
            route("/api/v1/alarms/device%3A3/ack").unwrap().1,
            Endpoint::AlarmAck {
                alarm: "device:3".into()
            }
        );
        assert_eq!(route("/api/v1/health").unwrap().1, Endpoint::Health);
        for missing in [
            "/api/v1",
            "/api/v1/",
            "/api/v1/devices/",
            "/api/v1/devices//params/x",
            "/api/v1/devices/E466/params",
            "/api/v1/devices/E466/params/a/b",
            "/api/v1/raw",
            "/api/v1/devices/E466/params/%zz",
        ] {
            assert!(route(missing).is_none(), "{missing}");
        }
    }

    #[test]
    fn wp_p2rs_messages_take_only_what_the_body_may_say() {
        let q: BTreeMap<String, String> = [("idx".to_owned(), "0,4".to_owned())].into();
        let body: Map<String, Value> = serde_json::from_value(
            json!({"value": -12.0, "serial": "OTHER", "t": "auth.mint", "path": "x"}),
        )
        .unwrap();
        let e = Endpoint::Param {
            serial: "S".into(),
            path: "vol.master".into(),
        };
        let m = e.message(&q, &body).unwrap();
        assert_eq!(
            m,
            json!({"t": "param.set", "id": 1, "serial": "S", "path": "vol.master",
                   "idx": [0, 4], "value": -12.0})
        );
        assert_eq!(e.message(&q, &Map::new()).unwrap()["t"], "param.get");
        let bad: BTreeMap<String, String> = [("idx".to_owned(), "x".to_owned())].into();
        assert!(e.message(&bad, &Map::new()).is_err());
        let events = Endpoint::Events { serial: "S".into() };
        let q: BTreeMap<String, String> = [
            ("after".to_owned(), "7".to_owned()),
            ("limit".to_owned(), "2".to_owned()),
        ]
        .into();
        assert_eq!(
            events.message(&q, &Map::new()).unwrap(),
            json!({"t": "event.list", "id": 1, "serial": "S", "after_id": 7, "limit": 2})
        );
        let preset = Endpoint::PresetLoad {
            serial: "S".into(),
            slot: "two".into(),
        };
        assert!(preset.message(&BTreeMap::new(), &Map::new()).is_err());
    }

    #[test]
    fn wp_p2rs_lengths_are_digits_only() {
        assert_eq!(parse_length(b"12"), Some(12));
        assert_eq!(parse_length(b" 12 "), Some(12));
        for bad in [
            &b""[..],
            b"-1",
            b"+1",
            b"1,1",
            b"0x10",
            b"1 2",
            b"99999999999999999999",
        ] {
            assert_eq!(parse_length(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn wp_p2rs_codes_have_the_tables_statuses() {
        assert_eq!(status_of("bad_request"), BAD_REQUEST);
        assert_eq!(status_of("unauthenticated"), UNAUTHORIZED);
        for c in [
            "denied",
            "limit_exceeded",
            "installer_lock",
            "image_refused",
            "confirmation_required",
        ] {
            assert_eq!(status_of(c), FORBIDDEN);
        }
        assert_eq!(status_of("no_device"), NOT_FOUND);
        assert_eq!(status_of("offline"), "409 Conflict");
        assert_eq!(status_of("locked"), "423 Locked");
        assert_eq!(status_of("rate_limited"), TOO_MANY);
        assert_eq!(status_of("unsupported"), "501 Not Implemented");
        assert_eq!(status_of("device"), UNAVAILABLE);
        assert_eq!(status_of("whatever"), "500 Internal Server Error");
    }

    #[test]
    fn wp_p2rs_an_answer_loses_t_and_id_and_gains_v_and_hub() {
        let r = translate("H", r#"{"t":"ok","id":1,"value":3}"#);
        assert_eq!(r.status, "200 OK");
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v, json!({"value": 3, "v": 1, "hub": "H"}));
        let r = translate(
            "H",
            r#"{"t":"err","id":1,"v":1,"code":"limit_exceeded","limit":"x"}"#,
        );
        assert_eq!(r.status, FORBIDDEN);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["limit"], "x");
        assert_eq!(v["code"], "limit_exceeded");
        assert!(v.get("t").is_none() && v.get("id").is_none());
    }

    #[test]
    fn wp_p2rs_percent_decoding_is_strict() {
        assert_eq!(percent_decode("a%2Fb+c").unwrap(), "a/b+c");
        assert_eq!(percent_decode("%C3%A9").unwrap(), "é");
        assert!(percent_decode("%").is_err());
        assert!(percent_decode("%4").is_err());
        assert!(percent_decode("%FF").is_err());
    }

    #[test]
    fn wp_p2rs_a_key_is_never_minted_above_the_callers_role() {
        // `ci/run.sh` points TMPDIR at the work volume, not tmpfs.
        let dir = std::env::temp_dir().join(format!("wp-p2rs-unit-mint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = crate::store::HubStore::load(&dir).unwrap();
        let auth = AuthStore::new(store, crate::auth::AccessMode::Pin).unwrap();
        let msg = |v: Value| v.as_object().unwrap().clone();
        let answer = |m: &Map<String, Value>, caller| -> Value {
            serde_json::from_str(&mint(&auth, Some(3), m, caller)).unwrap()
        };
        let above = answer(&msg(json!({"name": "k", "role": "admin"})), Role::Control);
        assert_eq!(
            (above["code"].clone(), above["id"].clone()),
            (json!("denied"), json!(3))
        );
        let same = answer(&msg(json!({"name": "k", "role": "control"})), Role::Control);
        assert_eq!(
            (same["t"].clone(), same["kind"].clone()),
            (json!("ok"), json!("api"))
        );
        assert!(auth.clients().iter().all(|c| c.role <= Role::Control));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wp_p2rs_the_allowance_refills_at_the_rate() {
        let rest = Rest::new(true);
        let limits = ServerLimits {
            rest_rate: 1000,
            rest_burst: 3,
            ..ServerLimits::default()
        };
        for _ in 0..3 {
            assert!(rest.take_allowance(&limits, 9).is_ok());
        }
        let wait = rest.take_allowance(&limits, 9).unwrap_err();
        assert!(wait >= 1);
        std::thread::sleep(Duration::from_millis(5));
        assert!(rest.take_allowance(&limits, 9).is_ok());
        // Another key has its own.
        assert!(rest.take_allowance(&limits, 10).is_ok());
        let unlimited = ServerLimits {
            rest_rate: 0,
            ..limits
        };
        for _ in 0..100 {
            assert!(rest.take_allowance(&unlimited, 11).is_ok());
        }
    }
}
