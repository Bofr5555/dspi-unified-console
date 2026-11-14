//! Guarded apply on the hub (WP P1HA, W6's hub half; PLAN-CLIENTS section 5.9).
//!
//! A client about to change a device over Link (an import, a snapshot recall) asks
//! the hub to guard it: `apply.begin` captures the device's whole image (`0xA0`, read
//! in the device's queue order as that client, so every write it queued before is in
//! it), then the client applies as usual. `apply.confirm` keeps the change. Without a
//! confirm inside the timeout, on `apply.cancel`, or when the client's session ends,
//! the hub writes the captured image back (`0xA1`) itself. So a client that vanishes
//! mid-apply cannot leave a half-applied device behind.
//!
//! - **One guard per device** (by serial). A second `apply.begin` is refused `busy`.
//! - **The revert is the hub's own write**: the hub's session ([`INTERNAL_SESSION_ID`]),
//!   so its notification carries the hub's origin, through BULKFIX's guard like any
//!   bulk write. It is **never refused by a lock** (owner, OD-P1g; PLAN-CLIENTS 5.9's
//!   "reverts are never refused"): [`crate::router::Router::pass_revert`] lets exactly
//!   that one write past a client's lock. P1HL's limits must leave it alone too.
//! - **Never on another board.** The revert goes to the online device with the
//!   captured serial. If none is online (unplugged, or another board now at the
//!   handle) nothing is written and the outcome is `not_written` (`no_device`).
//! - **Never replayed.** One write; a failed one is reported (`revert_failed`), not
//!   repeated. A revert queued past [`REVERT_WAIT`] does not run late.
//! - **Timer.** Due guards are reverted on the hub's `hub-system` thread (AUSB2's poll
//!   loop, `hub.rs`), woken when a guard is begun or ends early, so no thread is added.
//!   Without that thread (it could not start) they are reverted on the next `apply.*`
//!   message or session close, on the caller's thread.
//! - **Outcome.** Every end is broadcast as `apply.ended` to every network session and
//!   kept per serial for `apply.status`, so a client that lost its Link can say on
//!   reconnect what happened (P1SA).
//!
//! Additive to Link 1.2: the hub's `hello` lists the cap [`CAP`], answers carry `v`
//! ([`VERSION`]). Other hubs and the upstream page are unaffected. Roles: Admin and
//! Control for `apply.begin/confirm/cancel`, every role for `apply.status`
//! (`policy::apply_guard_allows`). The messages are in `docs/integrator/hub.md`.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use serde::Serialize;
use serde_json::{Value, json};

use crate::hub::HubHandle;
use crate::registry::DeviceState;
use crate::router::MAX_PAYLOAD;
use crate::session::{Completion, INTERNAL_SESSION_ID, Role, Session, SessionCtx, SessionId};

/// The `hello` cap that says this hub answers `apply.*`.
pub const CAP: &str = "apply_guard";
/// The messages' version, `v` in every answer.
pub const VERSION: u32 = 1;
/// The timeout when `apply.begin` gives none (owner, OD-P1g: 30 s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// The shortest timeout granted.
pub const MIN_TIMEOUT: Duration = Duration::from_secs(1);
/// The longest timeout granted.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the capture's read may take, queue included.
const CAPTURE_WAIT: Duration = Duration::from_secs(5);
/// How long the revert's write may take, queue included. Queued past it, it does not run.
pub const REVERT_WAIT: Duration = Duration::from_secs(10);
/// Outcomes kept, one per serial.
const MAX_OUTCOMES: usize = 64;

/// Why a guard ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// `apply.confirm` in time.
    Confirmed,
    /// No confirm inside the timeout.
    Timeout,
    /// The session that began it ended.
    SessionClosed,
    /// `apply.cancel`.
    Cancelled,
    /// The hub stopped (sharing turned off).
    HubStopping,
}

/// What became of the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndResult {
    /// The applied state stays.
    Kept,
    /// The captured image was written back.
    Reverted,
    /// The write back failed (see `status`). It is not repeated.
    RevertFailed,
    /// No device with the captured serial was online: nothing was written.
    NotWritten,
}

/// How a guard ended: `apply.ended` and `apply.status`'s `last`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// The guard's id, from `apply.begin`.
    pub guard: u64,
    /// The device's serial.
    pub serial: String,
    /// The handle it was begun on.
    pub handle: u8,
    /// Why it ended.
    pub reason: EndReason,
    /// What became of the device.
    pub result: EndResult,
    /// The revert's Link status by name when it was not written or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Unix seconds.
    pub at: u64,
}

/// A live guard.
#[derive(Debug)]
struct Guard {
    id: u64,
    serial: String,
    handle: u8,
    session: SessionId,
    image: Vec<u8>,
    deadline: Instant,
    /// Set when it must end before its deadline (cancel, session closed, stop).
    ending: Option<EndReason>,
    /// A thread is writing it back; nothing else may end it.
    reverting: bool,
}

impl Guard {
    /// Why it is due now, if it is.
    fn due(&self, now: Instant) -> Option<EndReason> {
        if self.reverting {
            return None;
        }
        self.ending
            .or_else(|| (now >= self.deadline).then_some(EndReason::Timeout))
    }
}

#[derive(Debug, Default)]
struct State {
    /// By serial: one per device.
    guards: BTreeMap<String, Guard>,
    /// The last outcome per serial.
    outcomes: BTreeMap<String, Outcome>,
    next_id: u64,
}

/// The hub's guards. A leaf lock: never held across a transfer or another hub lock.
#[derive(Debug, Default)]
pub(crate) struct ApplyGuards {
    state: Mutex<State>,
    /// Wakes the `hub-system` thread to look at the guards again.
    wake: Mutex<Option<Sender<()>>>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update under these locks is whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ApplyGuards {
    /// The `hub-system` thread's wake (its stop channel: a message while the hub is
    /// not stopping is a wake).
    pub(crate) fn set_wake(&self, wake: Option<Sender<()>>) {
        *guard(&self.wake) = wake;
    }

    /// Wake the thread. False when there is none to wake.
    fn wake(&self) -> bool {
        guard(&self.wake)
            .as_ref()
            .is_some_and(|w| w.send(()).is_ok())
    }

    /// The earliest deadline of a guard not yet being reverted: when the
    /// `hub-system` thread must look again.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        guard(&self.state)
            .guards
            .values()
            .filter(|g| !g.reverting)
            .map(|g| {
                if g.ending.is_some() {
                    Instant::now()
                } else {
                    g.deadline
                }
            })
            .min()
    }

    /// Take every due guard for reverting: marked, still in the table.
    fn take_due(&self, now: Instant) -> Vec<(String, u64, u8, Vec<u8>, EndReason)> {
        let mut st = guard(&self.state);
        st.guards
            .values_mut()
            .filter_map(|g| {
                let why = g.due(now)?;
                g.reverting = true;
                Some((g.serial.clone(), g.id, g.handle, g.image.clone(), why))
            })
            .collect()
    }

    /// End guard `id` with this outcome, and keep it.
    fn finish(&self, outcome: &Outcome) {
        let mut st = guard(&self.state);
        if st
            .guards
            .get(&outcome.serial)
            .is_some_and(|g| g.id == outcome.guard)
        {
            st.guards.remove(&outcome.serial);
        }
        if !st.outcomes.contains_key(&outcome.serial) && st.outcomes.len() >= MAX_OUTCOMES {
            let oldest = st
                .outcomes
                .iter()
                .min_by_key(|(_, o)| o.guard)
                .map(|(s, _)| s.clone());
            if let Some(oldest) = oldest {
                st.outcomes.remove(&oldest);
            }
        }
        st.outcomes.insert(outcome.serial.clone(), outcome.clone());
    }
}

/// Why an `apply.*` was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefused {
    /// The role may not (`denied`).
    Denied(&'static str),
    /// No device online at this handle (`no_device`).
    NoDevice,
    /// A guard is already running on the device, or its image could not be read
    /// (`busy`).
    Busy(&'static str),
    /// Another session holds the device's lock (`locked`).
    Locked,
    /// No guard to confirm or cancel: none, it timed out, or it is being reverted
    /// (`conflict`).
    NoGuard(&'static str),
}

impl ApplyRefused {
    /// The `err` message's code (spec 7.1).
    pub fn code(&self) -> &'static str {
        match self {
            ApplyRefused::Denied(_) => "denied",
            ApplyRefused::NoDevice => "no_device",
            ApplyRefused::Busy(_) => "busy",
            ApplyRefused::Locked => "locked",
            ApplyRefused::NoGuard(_) => "conflict",
        }
    }

    fn msg(&self) -> &'static str {
        match self {
            ApplyRefused::Denied(m) | ApplyRefused::Busy(m) | ApplyRefused::NoGuard(m) => m,
            ApplyRefused::NoDevice => "no such device online",
            ApplyRefused::Locked => "another session holds the device's lock",
        }
    }
}

/// What `apply.begin` granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Begun {
    /// The guard's id.
    pub guard: u64,
    /// The device's serial: the only board the revert will ever be written to.
    pub serial: String,
    /// The timeout granted.
    pub timeout: Duration,
}

fn unix_now() -> u64 {
    crate::store::unix_seconds(SystemTime::now())
}

fn status_name(s: u8) -> String {
    match s {
        status::OK => "ok".into(),
        status::NO_DEVICE => "no_device".into(),
        status::ERROR => "error".into(),
        status::TIMEOUT => "timeout".into(),
        status::DENIED => "denied".into(),
        status::LOCKED => "locked".into(),
        other => format!("status_{other}"),
    }
}

fn internal() -> SessionCtx {
    SessionCtx {
        id: INTERNAL_SESSION_ID,
        role: Role::Admin,
        exempt_inflight: true,
    }
}

/// One command on the device's worker, waited for at most `wait`; past it, a queued
/// command does not run.
fn exec(
    dev: &crate::device::DeviceShared,
    ctx: SessionCtx,
    req: CmdRequest,
    wait: Duration,
) -> Result<Vec<u8>, u8> {
    let (tx, rx) = mpsc::channel();
    let deadline = Instant::now().checked_add(wait);
    dev.submit(
        ctx,
        req,
        Completion::new(0, move |r| {
            let _ = tx.send(r);
        }),
        deadline,
    );
    // A transfer already running when the deadline passes still ends: wait for it.
    match rx.recv_timeout(wait.saturating_add(Duration::from_secs(5))) {
        Ok(r) if r.status == status::OK => Ok(r.payload),
        Ok(r) => Err(r.status),
        Err(_) => Err(status::TIMEOUT),
    }
}

impl HubHandle {
    fn apply_guards(&self) -> &ApplyGuards {
        &self.inner.apply_guards
    }

    /// The serial of the online device at `handle`.
    fn online_serial(&self, handle: u8) -> Option<String> {
        self.devices()
            .into_iter()
            .find(|d| d.handle == handle && d.state == DeviceState::Online)
            .map(|d| d.identity.serial)
    }

    /// `apply.begin`: capture the device's image and start its guard. Blocks for the
    /// read: on a connection's reader, never a hub thread.
    pub fn apply_begin(
        &self,
        session: &Session,
        handle: u8,
        timeout: Duration,
    ) -> Result<Begun, ApplyRefused> {
        if !crate::policy::apply_guard_allows(session.role(), "apply.begin") {
            return Err(ApplyRefused::Denied("control role required"));
        }
        self.apply_run_due_inline();
        let serial = self.online_serial(handle).ok_or(ApplyRefused::NoDevice)?;
        let dev = self
            .device_by_serial(&serial)
            .ok_or(ApplyRefused::NoDevice)?;
        let busy = "a guarded apply is already running on this device";
        if guard(&self.apply_guards().state)
            .guards
            .contains_key(&serial)
        {
            return Err(ApplyRefused::Busy(busy));
        }
        // As this session: in its order behind what it queued, and through its lock.
        let read = CmdRequest {
            tag: 0,
            handle: dev.handle,
            dir: Dir::Get,
            b_request: op::REQ_GET_ALL_PARAMS,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: u16::try_from(MAX_PAYLOAD).unwrap_or(u16::MAX),
            payload: Vec::new(),
        };
        let image = match exec(&dev, session.ctx(), read, CAPTURE_WAIT) {
            Ok(image) if !image.is_empty() => image,
            Ok(_) => return Err(ApplyRefused::Busy("the device's image was empty")),
            Err(status::LOCKED) => return Err(ApplyRefused::Locked),
            Err(status::NO_DEVICE) => return Err(ApplyRefused::NoDevice),
            Err(_) => {
                return Err(ApplyRefused::Busy(
                    "the device's image could not be read just now",
                ));
            }
        };
        let timeout = timeout.clamp(MIN_TIMEOUT, MAX_TIMEOUT);
        let now = Instant::now();
        let begun = {
            let mut st = guard(&self.apply_guards().state);
            if st.guards.contains_key(&serial) {
                return Err(ApplyRefused::Busy(busy));
            }
            st.next_id = st.next_id.wrapping_add(1);
            let id = st.next_id;
            st.guards.insert(
                serial.clone(),
                Guard {
                    id,
                    serial: serial.clone(),
                    handle,
                    session: session.id(),
                    image,
                    deadline: now.checked_add(timeout).unwrap_or(now),
                    ending: None,
                    reverting: false,
                },
            );
            Begun {
                guard: id,
                serial,
                timeout,
            }
        };
        self.apply_guards().wake();
        Ok(begun)
    }

    /// The guard on the device at `handle` that this session may end, or why not.
    /// Refuses one past its deadline: the confirm did not arrive in time.
    fn own_guard(
        st: &mut State,
        session: &Session,
        serial: Option<&str>,
        now: Instant,
    ) -> Result<String, ApplyRefused> {
        let none = ApplyRefused::NoGuard("no guarded apply is running on this device");
        let g = serial.and_then(|s| st.guards.get(s)).ok_or(none)?;
        if g.session != session.id() {
            return Err(ApplyRefused::Denied("another session's guarded apply"));
        }
        if g.reverting || g.due(now).is_some() {
            return Err(ApplyRefused::NoGuard(
                "the guarded apply has ended: it is being reverted",
            ));
        }
        Ok(g.serial.clone())
    }

    /// `apply.confirm`: keep the change. Only the session that began it, and only
    /// before its deadline.
    pub fn apply_confirm(&self, session: &Session, handle: u8) -> Result<Outcome, ApplyRefused> {
        if !crate::policy::apply_guard_allows(session.role(), "apply.confirm") {
            return Err(ApplyRefused::Denied("control role required"));
        }
        let serial = self.guarded_serial(handle);
        let outcome = {
            let mut st = guard(&self.apply_guards().state);
            let serial = Self::own_guard(&mut st, session, serial.as_deref(), Instant::now());
            let serial = match serial {
                Ok(s) => s,
                Err(e) => {
                    drop(st);
                    // A deadline passed and the thread has not looked yet.
                    self.apply_kick();
                    return Err(e);
                }
            };
            let Some(g) = st.guards.get(&serial) else {
                return Err(ApplyRefused::NoGuard(
                    "no guarded apply is running on this device",
                ));
            };
            Outcome {
                guard: g.id,
                serial: g.serial.clone(),
                handle: g.handle,
                reason: EndReason::Confirmed,
                result: EndResult::Kept,
                status: None,
                at: unix_now(),
            }
        };
        self.apply_end(&outcome);
        Ok(outcome)
    }

    /// `apply.cancel`: revert now, on this thread, and say how it went.
    pub fn apply_cancel(&self, session: &Session, handle: u8) -> Result<Outcome, ApplyRefused> {
        if !crate::policy::apply_guard_allows(session.role(), "apply.cancel") {
            return Err(ApplyRefused::Denied("control role required"));
        }
        let serial = self.guarded_serial(handle);
        let taken = {
            let mut st = guard(&self.apply_guards().state);
            let serial = Self::own_guard(&mut st, session, serial.as_deref(), Instant::now());
            match serial.map(|s| st.guards.get_mut(&s)) {
                Ok(Some(g)) => {
                    g.reverting = true;
                    Ok((g.serial.clone(), g.id, g.handle, g.image.clone()))
                }
                Ok(None) => Err(ApplyRefused::NoGuard(
                    "no guarded apply is running on this device",
                )),
                Err(e) => Err(e),
            }
        };
        let (serial, id, handle, image) = match taken {
            Ok(t) => t,
            Err(e) => {
                self.apply_kick();
                return Err(e);
            }
        };
        Ok(self.apply_revert(serial, id, handle, &image, EndReason::Cancelled))
    }

    /// `apply.status`: the running guard on the device at `handle`, and the last
    /// outcome on it, for a client back after a lost Link.
    pub fn apply_status(&self, handle: u8) -> Value {
        let serial = self.guarded_serial(handle);
        let st = guard(&self.apply_guards().state);
        let now = Instant::now();
        let active = serial.as_deref().and_then(|s| st.guards.get(s)).map(|g| {
            json!({
                "guard": g.id,
                "serial": g.serial,
                "session": g.session,
                "remaining_ms": u64::try_from(g.deadline.saturating_duration_since(now).as_millis())
                    .unwrap_or(u64::MAX),
                "reverting": g.reverting || g.due(now).is_some(),
            })
        });
        let last = serial
            .as_deref()
            .and_then(|s| st.outcomes.get(s))
            .and_then(|o| serde_json::to_value(o).ok());
        json!({"active": active, "last": last})
    }

    /// The serial a guard on `handle` is kept under: the online device's, else that
    /// of a guard begun on the handle (its device unplugged since).
    fn guarded_serial(&self, handle: u8) -> Option<String> {
        self.online_serial(handle).or_else(|| {
            guard(&self.apply_guards().state)
                .guards
                .values()
                .find(|g| g.handle == handle)
                .map(|g| g.serial.clone())
        })
    }

    /// A session ended: its guards revert, on the `hub-system` thread.
    pub(crate) fn apply_session_closed(&self, session: SessionId) {
        let any = {
            let mut st = guard(&self.apply_guards().state);
            let mut any = false;
            for g in st.guards.values_mut().filter(|g| g.session == session) {
                g.ending.get_or_insert(EndReason::SessionClosed);
                any = true;
            }
            any
        };
        if any {
            self.apply_kick();
        }
    }

    /// The hub is stopping: every guard reverts now, on this thread.
    pub(crate) fn apply_stop(&self) {
        {
            let mut st = guard(&self.apply_guards().state);
            for g in st.guards.values_mut() {
                g.ending.get_or_insert(EndReason::HubStopping);
            }
        }
        self.apply_run_due();
    }

    /// Wake the thread to the guards, or run them here if there is none.
    fn apply_kick(&self) {
        if !self.apply_guards().wake() {
            self.apply_run_due();
        }
    }

    /// Run the due guards here when no thread will.
    fn apply_run_due_inline(&self) {
        if guard(&self.apply_guards().wake).is_none() {
            self.apply_run_due();
        }
    }

    /// When the `hub-system` thread must look at the guards again.
    pub(crate) fn apply_next_due(&self) -> Option<Instant> {
        self.apply_guards().next_due()
    }

    /// Revert every due guard. Blocks for the writes: the `hub-system` thread.
    pub(crate) fn apply_run_due(&self) {
        for (serial, id, handle, image, why) in self.apply_guards().take_due(Instant::now()) {
            self.apply_revert(serial, id, handle, &image, why);
        }
    }

    /// Write `image` back to the online device with `serial`, as the hub, past any
    /// lock; end the guard and tell everyone.
    fn apply_revert(
        &self,
        serial: String,
        id: u64,
        handle: u8,
        image: &[u8],
        why: EndReason,
    ) -> Outcome {
        let (result, status) = match self.device_by_serial(&serial) {
            None => (EndResult::NotWritten, Some(status_name(status::NO_DEVICE))),
            Some(dev) => {
                let req = CmdRequest {
                    tag: 0,
                    handle: dev.handle,
                    dir: Dir::Set,
                    b_request: op::REQ_SET_ALL_PARAMS,
                    w_value: 0,
                    w_index: dspi_proto::VENDOR_INTERFACE,
                    w_length: u16::try_from(image.len()).unwrap_or(u16::MAX),
                    payload: image.to_vec(),
                };
                dev.router.pass_revert();
                match exec(&dev, internal(), req, REVERT_WAIT) {
                    Ok(_) => (EndResult::Reverted, None),
                    Err(s) if s == status::NO_DEVICE => {
                        (EndResult::NotWritten, Some(status_name(s)))
                    }
                    Err(s) => (EndResult::RevertFailed, Some(status_name(s))),
                }
            }
        };
        let outcome = Outcome {
            guard: id,
            serial,
            handle,
            reason: why,
            result,
            status,
            at: unix_now(),
        };
        if result != EndResult::Reverted {
            log::warn!(
                "hub: guarded apply {id} on {} ended {why:?}: {result:?} {:?}",
                outcome.serial,
                outcome.status
            );
        }
        self.apply_end(&outcome);
        outcome
    }

    /// Keep the outcome and broadcast `apply.ended`.
    fn apply_end(&self, outcome: &Outcome) {
        self.apply_guards().finish(outcome);
        let mut event = json!({"t": "apply.ended", "v": VERSION});
        if let (Some(map), Ok(Value::Object(body))) =
            (event.as_object_mut(), serde_json::to_value(outcome))
        {
            map.extend(body);
        }
        self.inner.relay.broadcast_event(&event.to_string());
    }
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

fn outcome_ok(id: &Value, o: &Outcome) -> String {
    let mut body = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Ok(Value::Object(o))) = (body.as_object_mut(), serde_json::to_value(o)) {
        map.extend(o);
    }
    body.to_string()
}

/// Answer `apply.begin`, `apply.confirm`, `apply.cancel` or `apply.status`. `None`
/// for any other message. For the connection's reader: `apply.begin` blocks for the
/// image's read and `apply.cancel` for the write back, on this connection alone.
pub fn handle_message(hub: &HubHandle, session: &Session, msg: &Value) -> Option<String> {
    let t = msg.get("t")?.as_str()?;
    if !matches!(
        t,
        "apply.begin" | "apply.confirm" | "apply.cancel" | "apply.status"
    ) {
        return None;
    }
    let id = msg.get("id").filter(|id| id.is_i64() || id.is_u64());
    let handle = msg
        .get("handle")
        .and_then(Value::as_u64)
        .and_then(|h| u8::try_from(h).ok());
    let (Some(id), Some(handle)) = (id, handle) else {
        return Some(err(id, "bad_request", "id and handle are required"));
    };
    if !crate::policy::apply_guard_allows(session.role(), t) {
        return Some(err(Some(id), "denied", "control role required"));
    }
    let refused = |e: ApplyRefused| err(Some(id), e.code(), e.msg());
    Some(match t {
        "apply.begin" => {
            let timeout = match msg.get("timeout_ms") {
                None | Some(Value::Null) => DEFAULT_TIMEOUT,
                Some(ms) => match ms.as_u64() {
                    Some(ms) => Duration::from_millis(ms),
                    None => {
                        return Some(err(
                            Some(id),
                            "bad_request",
                            "timeout_ms must be a whole number of milliseconds",
                        ));
                    }
                },
            };
            match hub.apply_begin(session, handle, timeout) {
                Ok(b) => json!({
                    "t": "ok",
                    "id": id,
                    "v": VERSION,
                    "guard": b.guard,
                    "serial": b.serial,
                    "timeout_ms": u64::try_from(b.timeout.as_millis()).unwrap_or(u64::MAX),
                })
                .to_string(),
                Err(e) => refused(e),
            }
        }
        "apply.confirm" => match hub.apply_confirm(session, handle) {
            Ok(o) => outcome_ok(id, &o),
            Err(e) => refused(e),
        },
        "apply.cancel" => match hub.apply_cancel(session, handle) {
            Ok(o) => outcome_ok(id, &o),
            Err(e) => refused(e),
        },
        _ => {
            let mut body = hub.apply_status(handle);
            if let Some(map) = body.as_object_mut() {
                map.insert("t".into(), "ok".into());
                map.insert("id".into(), id.clone());
                map.insert("v".into(), VERSION.into());
            }
            body.to_string()
        }
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1ha {
    use super::*;

    fn g(id: u64, deadline: Instant) -> Guard {
        Guard {
            id,
            serial: format!("S{id}"),
            handle: 1,
            session: 2,
            image: vec![1, 2, 3],
            deadline,
            ending: None,
            reverting: false,
        }
    }

    #[test]
    fn wp_p1ha_a_guard_is_due_at_its_deadline_or_when_ended() {
        let t0 = Instant::now();
        let mut a = g(1, t0 + Duration::from_secs(1));
        assert_eq!(a.due(t0), None);
        assert_eq!(a.due(t0 + Duration::from_secs(1)), Some(EndReason::Timeout));
        a.ending = Some(EndReason::SessionClosed);
        assert_eq!(a.due(t0), Some(EndReason::SessionClosed));
        a.reverting = true;
        assert_eq!(a.due(t0 + Duration::from_secs(5)), None, "taken once");
    }

    #[test]
    fn wp_p1ha_due_guards_are_taken_once() {
        let t0 = Instant::now();
        let gs = ApplyGuards::default();
        {
            let mut st = guard(&gs.state);
            st.guards.insert("S1".into(), g(1, t0));
            st.guards
                .insert("S2".into(), g(2, t0 + Duration::from_secs(60)));
        }
        assert_eq!(gs.next_due(), Some(t0));
        let due = gs.take_due(t0);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, "S1");
        assert!(gs.take_due(t0).is_empty());
        assert_eq!(gs.next_due(), Some(t0 + Duration::from_secs(60)));
    }

    #[test]
    fn wp_p1ha_outcomes_are_bounded_and_end_only_their_guard() {
        let gs = ApplyGuards::default();
        let t0 = Instant::now();
        guard(&gs.state).guards.insert("S1".into(), g(7, t0));
        let o = |guard: u64, serial: String| Outcome {
            guard,
            serial,
            handle: 1,
            reason: EndReason::Timeout,
            result: EndResult::Reverted,
            status: None,
            at: 0,
        };
        gs.finish(&o(6, "S1".into()));
        assert!(
            guard(&gs.state).guards.contains_key("S1"),
            "another guard's end"
        );
        gs.finish(&o(7, "S1".into()));
        assert!(guard(&gs.state).guards.is_empty());
        for i in 0..200 {
            gs.finish(&o(100 + i, format!("X{i}")));
        }
        assert_eq!(guard(&gs.state).outcomes.len(), MAX_OUTCOMES);
    }

    #[test]
    fn wp_p1ha_refusals_have_link_codes() {
        assert_eq!(ApplyRefused::Denied("x").code(), "denied");
        assert_eq!(ApplyRefused::NoDevice.code(), "no_device");
        assert_eq!(ApplyRefused::Busy("x").code(), "busy");
        assert_eq!(ApplyRefused::Locked.code(), "locked");
        assert_eq!(ApplyRefused::NoGuard("x").code(), "conflict");
    }
}
