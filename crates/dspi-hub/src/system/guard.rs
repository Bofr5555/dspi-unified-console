//! The guard of a system apply (WP ATGUARD; `docs/integrator/hub-api.md`
//! section 23.4): what [`crate::apply_guard`] is to one device, for the
//! hub's system model and the alignment it writes across units.
//!
//! A console about to apply a measured alignment (WP ATCORE,
//! `measure_system_apply`) asks the hub to guard it: `system.guard.begin`
//! files the model and its revision, then the console writes `system.set`
//! and `system.align` as before. `system.guard.confirm` keeps it (Keep).
//! Without a confirm before the deadline, on `system.guard.cancel`
//! (Revert), when the console's session ends (a crash, a lost connection)
//! or when the hub stops, the hub reverts by itself: `system.set` back to
//! the filed model **at the revision its owner's last `system.set` left**
//! (a model someone else changed meanwhile is not overwritten: the outcome
//! is `not_written`), then `system.align` writes the units.
//!
//! - **One guard at a time** (the model is one): a second `begin` is
//!   refused `busy`.
//! - **While it is pending, a `system.set` from another session is refused
//!   `conflict`**, as a guarded device refuses a foreign apply; the owner's
//!   own `system.set` goes through here, so the guard knows the revision
//!   it left. The hub's own API (`HubHandle::system_set_model`) is not
//!   refused; a change made there makes the revert `not_written`.
//! - **Before the revert** every measuring session of the owner
//!   (`system.measure.*`, a verify still playing) is aborted and closed:
//!   nothing plays and its locks do not refuse the alignment's writes.
//! - **Timer**: the `hub-system` thread, as [`crate::apply_guard`]'s (woken
//!   when a guard begins or ends early); without that thread, on the next
//!   `system.guard.*` message or session close, on the caller's thread.
//! - **Never replayed**: one set and one alignment; a failure is reported.
//! - Every end is broadcast as `system.guard.ended` to every network
//!   session and kept for `system.guard.status`.
//!
//! The bodies are the seam's ([`dspi_measure::system::messages::guard`]).
//! Roles: Admin for `begin`, `confirm`, `cancel` (the owner) and the
//! owner's `system.set`; every role for `status`. The cap is
//! [`wire::CAP`].

use std::sync::mpsc::Sender;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_measure::system::messages::guard::{self as wire, Active, BeginReply, Outcome};
use serde_json::{Map, Value, json};

use crate::hub::HubHandle;
use crate::session::{Role, Session, SessionId};

/// The `hello` cap.
pub const CAP: &str = wire::CAP;

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update under these locks is whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a guard ended (the outcome's `reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndReason {
    Confirmed,
    Timeout,
    SessionClosed,
    Cancelled,
    HubStopping,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            EndReason::Confirmed => "confirmed",
            EndReason::Timeout => "timeout",
            EndReason::SessionClosed => "session_closed",
            EndReason::Cancelled => "cancelled",
            EndReason::HubStopping => "hub_stopping",
        }
    }
}

/// The live guard.
#[derive(Debug, Clone)]
struct Pending {
    id: u64,
    session: SessionId,
    /// The model filed at `begin`, as JSON (the model's own serde form).
    model: Value,
    revision_filed: u64,
    /// The revision the owner's last `system.set` left.
    revision_left: u64,
    deadline: Instant,
    /// Set when it must end before its deadline.
    ending: Option<EndReason>,
    /// A revert is running: nothing else may end it.
    reverting: bool,
}

impl Pending {
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
    pending: Option<Pending>,
    last: Option<Outcome>,
    next_id: u64,
}

/// The hub's system guard. A leaf lock: never held across a transfer, a
/// save or another hub lock.
#[derive(Debug, Default)]
pub(crate) struct SystemGuards {
    state: Mutex<State>,
    /// Wakes the `hub-system` thread (its stop channel, as `apply_guard`'s).
    wake: Mutex<Option<Sender<()>>>,
}

impl SystemGuards {
    /// The `hub-system` thread's wake.
    pub(crate) fn set_wake(&self, wake: Option<Sender<()>>) {
        *guard(&self.wake) = wake;
    }

    fn wake(&self) -> bool {
        guard(&self.wake)
            .as_ref()
            .is_some_and(|w| w.send(()).is_ok())
    }

    fn threaded(&self) -> bool {
        guard(&self.wake).is_some()
    }

    /// When the `hub-system` thread must look again.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        guard(&self.state)
            .pending
            .as_ref()
            .filter(|p| !p.reverting)
            .map(|p| {
                if p.ending.is_some() {
                    Instant::now()
                } else {
                    p.deadline
                }
            })
    }

    /// Take the guard for reverting if it is due: marked, still pending.
    fn take_due(&self, now: Instant) -> Option<(Pending, EndReason)> {
        let mut st = guard(&self.state);
        let p = st.pending.as_mut()?;
        let why = p.due(now)?;
        p.reverting = true;
        Some((p.clone(), why))
    }

    /// End guard `id` with `outcome`.
    fn finish(&self, outcome: &Outcome) {
        let mut st = guard(&self.state);
        if st.pending.as_ref().is_some_and(|p| p.id == outcome.guard) {
            st.pending = None;
        }
        st.last = Some(outcome.clone());
    }
}

/// Why a `system.guard.*` (or a guarded `system.set`) was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuardRefused {
    /// The role may not, or another session's guard (`denied`).
    Denied(&'static str),
    /// A guard is already pending (`busy`).
    Busy,
    /// No such guard pending, or it is being reverted; or a `system.set`
    /// from another session while one is pending (`conflict`).
    Conflict(String),
    /// A malformed body (`bad_request`).
    BadRequest(String),
}

impl GuardRefused {
    fn code(&self) -> &'static str {
        match self {
            GuardRefused::Denied(_) => "denied",
            GuardRefused::Busy => "busy",
            GuardRefused::Conflict(_) => "conflict",
            GuardRefused::BadRequest(_) => "bad_request",
        }
    }

    fn msg(&self) -> String {
        match self {
            GuardRefused::Denied(m) => (*m).to_owned(),
            GuardRefused::Busy => {
                "an applied system alignment is already waiting for Keep or Revert".into()
            }
            GuardRefused::Conflict(m) | GuardRefused::BadRequest(m) => m.clone(),
        }
    }
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl HubHandle {
    fn system_guards(&self) -> &SystemGuards {
        &self.inner.system_guard
    }

    /// `system.guard.begin`: file the model and its revision and start the
    /// countdown. Admin only; one at a time.
    pub(crate) fn system_guard_begin(
        &self,
        session: &Session,
        timeout: Duration,
    ) -> Result<BeginReply, GuardRefused> {
        if session.role() != Role::Admin {
            return Err(GuardRefused::Denied("admin role required"));
        }
        self.system_guard_run_due_inline();
        let (model, revision) = self.system_model();
        let model = serde_json::to_value(&model)
            .map_err(|e| GuardRefused::BadRequest(format!("the model could not be filed: {e}")))?;
        let timeout_ms =
            wire::clamp_timeout_ms(Some(u32::try_from(ms(timeout)).unwrap_or(u32::MAX)));
        let now = Instant::now();
        let reply = {
            let mut st = guard(&self.system_guards().state);
            if st.pending.is_some() {
                return Err(GuardRefused::Busy);
            }
            st.next_id = st.next_id.wrapping_add(1);
            let id = st.next_id;
            st.pending = Some(Pending {
                id,
                session: session.id(),
                model,
                revision_filed: revision,
                revision_left: revision,
                deadline: now
                    .checked_add(Duration::from_millis(u64::from(timeout_ms)))
                    .unwrap_or(now),
                ending: None,
                reverting: false,
            });
            BeginReply {
                guard: id,
                revision,
                timeout_ms,
            }
        };
        log::info!(
            "hub: system apply guard {} begun by session {} at revision {revision}, {timeout_ms} ms",
            reply.guard,
            session.id()
        );
        self.system_guards().wake();
        Ok(reply)
    }

    /// The pending guard `id` if `session` owns it and it is not ending.
    fn system_guard_own(&self, session: &Session, id: u64) -> Result<(), GuardRefused> {
        if session.role() != Role::Admin {
            return Err(GuardRefused::Denied("admin role required"));
        }
        let st = guard(&self.system_guards().state);
        let gone = || {
            GuardRefused::Conflict(format!(
                "system apply guard {id} is not pending: it has ended (system.guard.status says how)"
            ))
        };
        let p = st
            .pending
            .as_ref()
            .filter(|p| p.id == id)
            .ok_or_else(gone)?;
        if p.session != session.id() {
            return Err(GuardRefused::Denied("another session's system apply guard"));
        }
        if p.due(Instant::now()).is_some() || p.reverting {
            return Err(GuardRefused::Conflict(format!(
                "system apply guard {id} has ended: it is being reverted"
            )));
        }
        Ok(())
    }

    /// `system.guard.confirm`: keep it.
    pub(crate) fn system_guard_confirm(
        &self,
        session: &Session,
        id: u64,
    ) -> Result<Outcome, GuardRefused> {
        if let Err(e) = self.system_guard_own(session, id) {
            self.system_guard_kick();
            return Err(e);
        }
        let (_, revision) = self.system_model();
        let outcome = {
            let mut st = guard(&self.system_guards().state);
            // -- review AT-D2 -- judged and taken under one lock: a guard
            // that came due (its deadline, its session closed) since the
            // check above is the revert's, never reported as kept while the
            // timer thread reverts it.
            let now = Instant::now();
            let Some(p) = st
                .pending
                .take_if(|p| p.id == id && !p.reverting && p.due(now).is_none())
            else {
                return Err(GuardRefused::Conflict(format!(
                    "system apply guard {id} is not pending: it has ended or is being reverted \
                     (system.guard.status says how)"
                )));
            };
            Outcome {
                guard: id,
                reason: EndReason::Confirmed.as_str().into(),
                result: "kept".into(),
                revision_filed: p.revision_filed,
                revision,
                message: "kept: the applied alignment stays".into(),
                at: crate::system::now(),
                ..Outcome::default()
            }
        };
        self.system_guard_end(&outcome);
        Ok(outcome)
    }

    /// `system.guard.cancel`: revert now, on this thread.
    pub(crate) fn system_guard_cancel(
        &self,
        session: &Session,
        id: u64,
    ) -> Result<Outcome, GuardRefused> {
        if let Err(e) = self.system_guard_own(session, id) {
            self.system_guard_kick();
            return Err(e);
        }
        let taken = {
            let mut st = guard(&self.system_guards().state);
            match st.pending.as_mut().filter(|p| p.id == id && !p.reverting) {
                Some(p) => {
                    p.reverting = true;
                    Some(p.clone())
                }
                None => None,
            }
        };
        match taken {
            Some(p) => Ok(self.system_guard_revert(p, EndReason::Cancelled)),
            None => Err(GuardRefused::Conflict(format!(
                "system apply guard {id} is not pending"
            ))),
        }
    }

    /// `system.guard.status`.
    pub(crate) fn system_guard_status(&self) -> wire::StatusReply {
        let st = guard(&self.system_guards().state);
        let now = Instant::now();
        wire::StatusReply {
            active: st.pending.as_ref().map(|p| Active {
                guard: p.id,
                session: u64::from(p.session),
                revision_filed: p.revision_filed,
                revision_left: p.revision_left,
                remaining_ms: ms(p.deadline.saturating_duration_since(now)),
                reverting: p.reverting || p.due(now).is_some(),
            }),
            last: st.last.clone(),
        }
    }

    /// Whether a `system.set` from `session` may go on, and if it is the
    /// guard's owner. `Err(conflict)` for another session while a guard is
    /// pending.
    fn system_guard_set_allowed(&self, session: &Session) -> Result<bool, GuardRefused> {
        let st = guard(&self.system_guards().state);
        match st.pending.as_ref() {
            None => Ok(false),
            Some(p) if p.session == session.id() && !p.reverting => Ok(true),
            Some(p) if p.session == session.id() => Err(GuardRefused::Conflict(
                "the applied system alignment is being reverted".into(),
            )),
            Some(p) => Err(GuardRefused::Conflict(format!(
                "an applied system alignment is waiting for Keep or Revert (guard {}, another \
                 session's): the installation cannot be changed until it ends",
                p.id
            ))),
        }
    }

    /// The owner's `system.set` while its guard is pending: as the server's
    /// `system.set` (`server.rs`), and the revision it leaves is noted.
    fn system_guard_owner_set(&self, id: Option<&Value>, msg: &Map<String, Value>) -> String {
        let Some(model) = msg.get("model").cloned() else {
            return err(id, "bad_request", "no model");
        };
        let model = match serde_json::from_value(model) {
            Ok(m) => m,
            Err(e) => return err(id, "bad_request", &e.to_string()),
        };
        let expected = msg.get("revision").and_then(Value::as_u64);
        match self.system_set_model_at(model, expected) {
            Ok(revision) => {
                if let Some(p) = guard(&self.system_guards().state).pending.as_mut() {
                    p.revision_left = revision;
                }
                let warnings = self.system_alink_warnings();
                ok(id, json!({"revision": revision, "warnings": warnings}))
            }
            Err(crate::system::ModelRefused::Invalid(e)) => err(id, "bad_request", &e.to_string()),
            Err(e @ crate::system::ModelRefused::Stale { .. }) => {
                err(id, "conflict", &e.to_string())
            }
            Err(e @ crate::system::ModelRefused::NotSaved(_)) => {
                err(id, "internal", &e.to_string())
            }
        }
    }

    /// A session ended: its guard reverts, on the `hub-system` thread.
    pub(crate) fn system_guard_session_closed(&self, session: SessionId) {
        let any = {
            let mut st = guard(&self.system_guards().state);
            match st.pending.as_mut().filter(|p| p.session == session) {
                Some(p) => {
                    p.ending.get_or_insert(EndReason::SessionClosed);
                    true
                }
                None => false,
            }
        };
        if any {
            self.system_guard_kick();
        }
    }

    /// The hub is stopping: the guard reverts now, on this thread.
    pub(crate) fn system_guard_stop(&self) {
        if let Some(p) = guard(&self.system_guards().state).pending.as_mut() {
            p.ending.get_or_insert(EndReason::HubStopping);
        }
        self.system_guard_run_due();
    }

    fn system_guard_kick(&self) {
        if !self.system_guards().wake() {
            self.system_guard_run_due();
        }
    }

    fn system_guard_run_due_inline(&self) {
        if !self.system_guards().threaded() {
            self.system_guard_run_due();
        }
    }

    /// When the `hub-system` thread must look at the guard again.
    pub(crate) fn system_guard_next_due(&self) -> Option<Instant> {
        self.system_guards().next_due()
    }

    /// Revert the guard if it is due. Blocks for the writes.
    pub(crate) fn system_guard_run_due(&self) {
        if let Some((p, why)) = self.system_guards().take_due(Instant::now()) {
            self.system_guard_revert(p, why);
        }
    }

    /// Stop the owner's measuring, set the filed model back at the
    /// revision the owner left, align, end the guard and tell everyone.
    fn system_guard_revert(&self, p: Pending, why: EndReason) -> Outcome {
        let stopped = self.system_measure_end_sessions_of(p.session, "guard");
        let mut outcome = Outcome {
            guard: p.id,
            reason: why.as_str().into(),
            revision_filed: p.revision_filed,
            at: crate::system::now(),
            ..Outcome::default()
        };
        let model = serde_json::from_value::<crate::system::model::SystemModel>(p.model.clone());
        let set = match model {
            Ok(m) => self
                .system_set_model_at(m, Some(p.revision_left))
                .map_err(|e| e.to_string()),
            Err(e) => Err(format!("the filed model could not be read back: {e}")),
        };
        match set {
            Ok(revision) => {
                outcome.revision = revision;
                let report = self.system_align(true);
                for w in &report.writes {
                    if w.result == "ok" || w.result == "unchanged" {
                        outcome.writes_ok = outcome.writes_ok.saturating_add(1);
                    } else {
                        outcome
                            .write_failures
                            .push(format!("{} {}: {}", w.serial, w.what, w.result));
                    }
                }
                if outcome.write_failures.is_empty() {
                    outcome.result = "reverted".into();
                    outcome.message =
                        "reverted: the filed model is set again and the units aligned".into();
                } else {
                    outcome.result = "revert_failed".into();
                    outcome.message = format!(
                        "the filed model is set again, but not every unit was written: {}",
                        outcome.write_failures.join("; ")
                    );
                }
            }
            Err(e) => {
                outcome.revision = self.system_model().1;
                outcome.result = "not_written".into();
                outcome.message = format!("nothing was set back: {e}");
            }
        }
        if stopped > 0 {
            outcome.message = format!(
                "{} (its measuring session was stopped first)",
                outcome.message
            );
        }
        if outcome.result != "reverted" {
            log::warn!(
                "hub: system apply guard {} ended {}: {}",
                p.id,
                outcome.reason,
                outcome.message
            );
        } else {
            log::info!(
                "hub: system apply guard {} ended {}: reverted",
                p.id,
                outcome.reason
            );
        }
        self.system_guard_end(&outcome);
        outcome
    }

    fn system_guard_end(&self, outcome: &Outcome) {
        self.system_guards().finish(outcome);
        let mut event = json!({"t": wire::ENDED, "v": 1});
        if let (Some(map), Ok(Value::Object(body))) =
            (event.as_object_mut(), serde_json::to_value(outcome))
        {
            map.extend(body);
        }
        self.inner.relay.broadcast_event(&event.to_string());
        // Its deadline is gone: the thread looks again.
        self.system_guards().wake();
    }
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

fn ok(id: Option<&Value>, body: Value) -> String {
    let mut m = match body {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("t".into(), "ok".into());
    m.insert("v".into(), 1.into());
    if let Some(id) = id {
        m.insert("id".into(), id.clone());
    }
    Value::Object(m).to_string()
}

fn body<T: serde::Serialize>(id: Option<&Value>, v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(v) => ok(id, v),
        Err(_) => err(id, "internal", "the answer could not be written"),
    }
}

/// Answer `system.guard.*`, and a `system.set` while a guard is pending
/// (another session's: `conflict`; the owner's: done here). `None` for
/// anything else, and for `system.set` with no guard pending.
pub(crate) fn handle_message(hub: &HubHandle, session: &Session, msg: &Value) -> Option<String> {
    let t = msg.get("t")?.as_str()?;
    let map = msg.as_object()?;
    let id = msg.get("id").filter(|id| id.is_i64() || id.is_u64());
    let refused = |e: GuardRefused| err(id, e.code(), &e.msg());
    if t == "system.set" {
        // Not ours to judge the role: an Admin-only message (server.rs).
        if session.role() != Role::Admin {
            return None;
        }
        return match hub.system_guard_set_allowed(session) {
            Ok(false) => None,
            Ok(true) => Some(hub.system_guard_owner_set(id, map)),
            Err(e) => Some(refused(e)),
        };
    }
    if !matches!(t, wire::BEGIN | wire::CONFIRM | wire::CANCEL | wire::STATUS) {
        return None;
    }
    if t == wire::STATUS {
        hub.system_guard_run_due_inline();
        return Some(body(id, &hub.system_guard_status()));
    }
    if session.role() != Role::Admin {
        return Some(err(id, "denied", "admin role required"));
    }
    let guard_id = || -> Result<u64, GuardRefused> {
        map.get("guard")
            .and_then(Value::as_u64)
            .ok_or_else(|| GuardRefused::BadRequest("guard is required".into()))
    };
    Some(match t {
        wire::BEGIN => {
            let timeout = match map.get("timeout_ms") {
                None | Some(Value::Null) => Some(wire::TIMEOUT_MS_DEFAULT),
                Some(v) => v.as_u64().map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
            };
            let Some(timeout) = timeout else {
                return Some(err(
                    id,
                    "bad_request",
                    "timeout_ms must be a whole number of milliseconds",
                ));
            };
            match hub.system_guard_begin(session, Duration::from_millis(u64::from(timeout))) {
                Ok(r) => body(id, &r),
                Err(e) => refused(e),
            }
        }
        wire::CONFIRM => match guard_id().and_then(|g| hub.system_guard_confirm(session, g)) {
            Ok(o) => body(id, &o),
            Err(e) => refused(e),
        },
        _ => match guard_id().and_then(|g| hub.system_guard_cancel(session, g)) {
            Ok(o) => body(id, &o),
            Err(e) => refused(e),
        },
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_atguard {
    use super::*;

    fn p(deadline: Instant) -> Pending {
        Pending {
            id: 1,
            session: 2,
            model: Value::Null,
            revision_filed: 3,
            revision_left: 4,
            deadline,
            ending: None,
            reverting: false,
        }
    }

    #[test]
    fn wp_atguard_due_at_the_deadline_or_when_ended_and_taken_once() {
        let t0 = Instant::now();
        let mut a = p(t0 + Duration::from_secs(1));
        assert_eq!(a.due(t0), None);
        assert_eq!(a.due(t0 + Duration::from_secs(1)), Some(EndReason::Timeout));
        a.ending = Some(EndReason::SessionClosed);
        assert_eq!(a.due(t0), Some(EndReason::SessionClosed));
        let g = SystemGuards::default();
        guard(&g.state).pending = Some(p(t0));
        assert_eq!(g.next_due(), Some(t0));
        assert!(g.take_due(t0).is_some());
        assert!(g.take_due(t0).is_none(), "taken once");
        assert_eq!(g.next_due(), None, "being reverted");
        g.finish(&Outcome {
            guard: 9,
            ..Outcome::default()
        });
        assert!(guard(&g.state).pending.is_some(), "another guard's end");
        g.finish(&Outcome {
            guard: 1,
            ..Outcome::default()
        });
        assert!(guard(&g.state).pending.is_none());
        assert_eq!(guard(&g.state).last.as_ref().map(|o| o.guard), Some(1));
    }

    #[test]
    fn wp_atguard_refusals_have_link_codes() {
        assert_eq!(GuardRefused::Denied("x").code(), "denied");
        assert_eq!(GuardRefused::Busy.code(), "busy");
        assert_eq!(GuardRefused::Conflict("x".into()).code(), "conflict");
        assert_eq!(GuardRefused::BadRequest("x".into()).code(), "bad_request");
        assert_eq!(CAP, "system_apply_guard");
    }
}
