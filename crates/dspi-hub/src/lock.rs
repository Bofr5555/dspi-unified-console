//! The per-device exclusive lock (spec 7.7) and its messages. Owned by WP HB2.
//! The table is pure: the caller passes the clock.
//!
//! **A lock cannot be held forever.** Each grant is at most [`LOCK_MAX`], and the
//! holder's session closing or the timeout ends it, as the spec says. That alone would
//! let a client renew for ever, or let a lock pass between sessions, so the device has
//! one budget, whoever holds the lock and however often they reconnect: time held fills
//! it, time free drains it [`LOCK_DRAIN`] times as fast. Once [`LOCK_BUDGET`] is used
//! no session gets the lock until it has drained completely, which takes 30 s with
//! nobody holding it. So no device is locked for longer than the budget at a stretch,
//! and every other session's commands then have those 30 s. A real flow is nowhere near
//! this: the longest takes 60 s (the spec's table), and at 5 s of lock in every 20 s the
//! budget never fills. Refused, a flow says "Another console is applying changes", as
//! for any lock it did not get.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::hub::HubHandle;
use crate::session::{Session, SessionId};

/// Longest a lock may be held without renewal (spec 7.7).
pub const LOCK_MAX: Duration = Duration::from_secs(60);
/// The most a device can be locked at a stretch, by anyone.
pub const LOCK_BUDGET: Duration = Duration::from_secs(120);
/// How much faster time free drains the budget than time held fills it.
pub const LOCK_DRAIN: u32 = 4;
/// `timeout_ms` when a `lock.acquire` has none, as in the Swift hub.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Who holds a device, until when, and how much of its budget is gone.
#[derive(Debug, Default)]
pub struct LockTable {
    holder: Option<(SessionId, Instant)>,
    used: Duration,
    settled_at: Option<Instant>,
    /// The budget ran out and has not drained yet.
    cooling: bool,
    // -- WP ATHUB --
    /// The holder's lock is a live `system.measure` session's: its time held
    /// does not fill the budget (its lease and dead-man bound it instead).
    exempt: bool,
}

impl LockTable {
    /// The live holder. An expired lock counts as released.
    pub fn holder(&self, now: Instant) -> Option<SessionId> {
        self.holder.filter(|(_, until)| now < *until).map(|h| h.0)
    }

    /// Whether `session` may run a command now.
    pub fn permits(&self, session: SessionId, now: Instant) -> bool {
        self.holder(now).is_none_or(|h| h == session)
    }

    /// Bring the budget up to `now`: the time since the last look, held and then free.
    fn settle(&mut self, now: Instant) {
        let Some(at) = self.settled_at else {
            self.settled_at = Some(now);
            return;
        };
        let now = now.max(at);
        self.settled_at = Some(now);
        let held_until = self.holder.map_or(at, |(_, until)| until.max(at).min(now));
        // -- WP ATHUB -- an exempt holder's time fills nothing
        let held = if self.exempt {
            Duration::ZERO
        } else {
            held_until.saturating_duration_since(at)
        };
        let filled = self.used.saturating_add(held);
        if filled >= LOCK_BUDGET {
            self.cooling = true;
        }
        let drained = now
            .saturating_duration_since(held_until)
            .saturating_mul(LOCK_DRAIN);
        self.used = filled.saturating_sub(drained);
        if self.used.is_zero() {
            self.cooling = false;
        }
    }

    /// Take or renew the lock, and how long for: `timeout`, capped by [`LOCK_MAX`] and
    /// what is left of the budget. `None` if another session holds a live one or the
    /// budget has run out.
    pub fn acquire(
        &mut self,
        session: SessionId,
        timeout: Duration,
        now: Instant,
    ) -> Option<Duration> {
        self.settle(now);
        if !self.permits(session, now) || self.cooling {
            return None;
        }
        // -- WP ATHUB -- a renewal keeps the holder's exemption; anyone else's
        // lock, or the same session's after a release or expiry, has none
        self.exempt = self.exempt && self.holder(now) == Some(session);
        let granted = if self.exempt {
            timeout.min(LOCK_MAX)
        } else {
            timeout
                .min(LOCK_MAX)
                .min(LOCK_BUDGET.saturating_sub(self.used))
        };
        let until = now.checked_add(granted).unwrap_or(now);
        self.holder = Some((session, until));
        Some(granted)
    }

    /// Release the lock if `session` holds it. The clock is for the budget.
    pub fn release(&mut self, session: SessionId, now: Instant) {
        if self.holder.is_some_and(|(h, _)| h == session) {
            self.settle(now);
            self.holder = None;
            self.exempt = false; // -- WP ATHUB --
        }
    }

    // -- WP ATHUB --
    /// Exempt `session`'s live lock from the budget from `now` on, while it
    /// holds and renews it: a `system.measure` session's own locks only
    /// (`system/measure.rs`, while its lease is live). False if it does not
    /// hold the lock.
    pub fn exempt_holder(&mut self, session: SessionId, now: Instant) -> bool {
        self.settle(now);
        let holds = self.holder(now) == Some(session);
        if holds {
            self.exempt = true;
        }
        holds
    }

    // -- WP ATHUB --
    /// Whether the live holder's lock is exempt from the budget.
    pub fn holder_exempt(&self, now: Instant) -> bool {
        self.exempt && self.holder(now).is_some()
    }
}

/// Why a session did not get a lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockRefusal {
    /// A viewer: it cannot write, so a lock would only block everyone else (spec 7.7).
    Denied,
    /// No device online at this handle.
    NoDevice,
    /// Another session holds it, or the device's budget has run out.
    Locked,
}

impl LockRefusal {
    /// The `err` message's code (spec 7.1).
    pub fn code(self) -> &'static str {
        match self {
            LockRefusal::Denied => "denied",
            LockRefusal::NoDevice => "no_device",
            LockRefusal::Locked => "locked",
        }
    }
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

/// Answer `lock.acquire` or `lock.release` (spec 7.7). `None` for any other message.
/// For the connection's reader (WP HB1). Never blocks on a device, and tells the
/// registry, so `device.changed` follows.
pub fn handle_message(hub: &HubHandle, session: &Session, msg: &Value) -> Option<String> {
    let t = msg.get("t")?.as_str()?;
    if t != "lock.acquire" && t != "lock.release" {
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
    if t == "lock.release" {
        hub.lock_release(session, handle);
        return Some(json!({"t": "ok", "id": id}).to_string());
    }
    let timeout = match msg.get("timeout_ms") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT,
        Some(ms) => match (ms.as_u64(), ms.as_f64()) {
            (Some(ms), _) => Duration::from_millis(ms),
            (None, Some(ms)) if ms >= 0.0 => Duration::from_secs_f64(ms.min(1e9) / 1000.0),
            _ => {
                return Some(err(
                    Some(id),
                    "bad_request",
                    "timeout_ms must be a number that is not negative",
                ));
            }
        },
    };
    Some(match hub.lock_request(session, handle, timeout) {
        Ok(granted) => {
            json!({"t": "ok", "id": id, "timeout_ms": granted.as_millis() as u64}).to_string()
        }
        Err(refused) => err(Some(id), refused.code(), "the lock was refused"),
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb0 {
    use super::*;

    #[test]
    fn wp_hb0_a_lock_excludes_others_until_released_or_expired() {
        let t0 = Instant::now();
        let s = Duration::from_secs(1);
        let mut l = LockTable::default();
        assert!(l.acquire(2, s, t0).is_some());
        assert!(l.permits(2, t0));
        assert!(!l.permits(3, t0));
        assert!(l.acquire(3, s, t0).is_none());
        l.release(3, t0);
        assert_eq!(l.holder(t0), Some(2), "only the holder releases");
        assert!(l.permits(3, t0 + 2 * s), "expired");
        assert!(l.acquire(3, s, t0 + 2 * s).is_some());
        l.release(3, t0 + 2 * s);
        assert_eq!(l.holder(t0 + 2 * s), None);
    }

    #[test]
    fn wp_hb0_a_lock_is_capped() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        assert_eq!(l.acquire(2, Duration::from_secs(3600), t0), Some(LOCK_MAX));
        assert_eq!(l.holder(t0 + LOCK_MAX), None);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb2 {
    use super::*;
    use proptest::prelude::*;

    const SEC: Duration = Duration::from_secs(1);
    /// How long the budget takes to drain, once it is gone.
    const COOL: Duration = Duration::from_secs(30);

    #[test]
    fn wp_hb2_a_hostile_client_cannot_renew_for_ever() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        let mut now = t0;
        // Renewing every second.
        while l.acquire(2, 60 * SEC, now).is_some() && now - t0 < 10 * LOCK_BUDGET {
            now += SEC;
        }
        assert_eq!(now - t0, LOCK_BUDGET, "refused once the budget is gone");
        assert!(l.permits(3, now), "and the others are through");
        assert!(l.acquire(2, SEC, now + COOL - SEC).is_none(), "not yet");
        assert!(l.acquire(2, SEC, now + COOL).is_some(), "drained");
    }

    /// The renewal that uses up the budget is told so.
    #[test]
    fn wp_hb2_a_grant_says_what_is_left_of_the_budget() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        assert_eq!(l.acquire(2, 60 * SEC, t0), Some(60 * SEC));
        assert_eq!(l.acquire(2, 60 * SEC, t0 + 59 * SEC), Some(60 * SEC));
        assert_eq!(l.acquire(2, 60 * SEC, t0 + 100 * SEC), Some(20 * SEC));
    }

    /// Reconnecting, or a friend on another session, starts no new budget.
    #[test]
    fn wp_hb2_a_new_session_gets_no_new_budget() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        let mut now = t0;
        let mut session = 2;
        while let Some(granted) = l.acquire(session, 60 * SEC, now) {
            now += granted;
            l.release(session, now);
            session += 1;
            assert!(now - t0 <= LOCK_BUDGET, "held past the budget");
        }
        assert_eq!(now - t0, LOCK_BUDGET);
        assert!(l.acquire(99, SEC, now).is_none());
        assert_eq!(l.holder(now), None);
        assert!(l.acquire(99, SEC, now + COOL).is_some());
    }

    /// One tiny grant after another cannot slip past the wait.
    #[test]
    fn wp_hb2_a_grant_right_after_the_budget_ran_out_is_refused() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        l.acquire(2, 60 * SEC, t0).unwrap();
        l.acquire(2, 60 * SEC, t0 + 60 * SEC).unwrap();
        assert_eq!(l.holder(t0 + 119 * SEC), Some(2));
        let just_after = t0 + 120 * SEC + Duration::from_millis(1);
        assert!(l.acquire(3, SEC, just_after).is_none());
    }

    /// The real flows of the spec's table never hit it.
    #[test]
    fn wp_hb2_ordinary_flows_never_run_out() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        let mut now = t0;
        // A full import, then its save.
        assert_eq!(l.acquire(2, 60 * SEC, now), Some(60 * SEC));
        now += 60 * SEC;
        l.release(2, now);
        assert_eq!(l.acquire(2, 5 * SEC, now), Some(5 * SEC));
        l.release(2, now + 5 * SEC);
        now += 5 * SEC;
        // Then a save every 20 s, for an hour.
        for _ in 0..180 {
            now += 15 * SEC;
            assert_eq!(l.acquire(2, 5 * SEC, now), Some(5 * SEC));
            now += 5 * SEC;
            l.release(2, now);
        }
    }

    proptest! {
        /// Whatever anyone does: no unbroken stretch of being locked is longer than the
        /// budget, and after one that uses it all, the lock is free for the wait.
        #[test]
        fn proptest_wp_hb2_no_schedule_locks_a_device_for_ever(
            steps in proptest::collection::vec(
                (0u16..5, 0u8..3, 0u64..70_000, 0u64..70_000),
                1..400,
            )
        ) {
            let t0 = Instant::now();
            let mut l = LockTable::default();
            let mut now = t0;
            let mut stretch_start: Option<Instant> = None;
            let mut last_until = t0;
            for (session, action, timeout_ms, wait_ms) in steps {
                let session = session + 2;
                match action {
                    0 | 1 => {
                        if let Some(g) = l.acquire(session, Duration::from_millis(timeout_ms), now) {
                            prop_assert!(g <= LOCK_MAX);
                            if !g.is_zero() {
                                let start = match stretch_start {
                                    Some(s) if now <= last_until => s,
                                    _ => now,
                                };
                                stretch_start = Some(start);
                                last_until = now + g;
                                prop_assert!(
                                    last_until - start <= LOCK_BUDGET,
                                    "locked for {:?}",
                                    last_until - start
                                );
                            }
                        }
                    }
                    _ => {
                        if l.holder(now) == Some(session) {
                            last_until = now;
                        }
                        l.release(session, now);
                    }
                }
                now += Duration::from_millis(wait_ms);
            }
        }
    }
}

// -- WP ATHUB --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_athub {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);

    /// A measurement session renewing every 2 s for ten minutes is never
    /// refused; once it releases, the same session is budgeted again.
    #[test]
    fn wp_athub_an_exempt_lock_outlives_the_budget_until_released() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        assert!(l.acquire(2, 10 * SEC, t0).is_some());
        assert!(l.exempt_holder(2, t0));
        let mut now = t0;
        while now - t0 < 600 * SEC {
            now += 2 * SEC;
            assert_eq!(
                l.acquire(2, 10 * SEC, now),
                Some(10 * SEC),
                "at {:?}",
                now - t0
            );
            assert!(l.holder_exempt(now));
        }
        l.release(2, now);
        assert!(!l.holder_exempt(now));
        // Budgeted again: refused once 120 s are held.
        let start = now;
        while l.acquire(2, 60 * SEC, now).is_some() && now - start < 10 * LOCK_BUDGET {
            now += SEC;
        }
        assert_eq!(now - start, LOCK_BUDGET);
    }

    /// Only the holder can be exempted, and a lock that lapses loses it.
    #[test]
    fn wp_athub_the_exemption_is_the_holders_and_ends_with_its_lock() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        assert!(!l.exempt_holder(2, t0), "nobody holds it");
        assert!(l.acquire(3, 10 * SEC, t0).is_some());
        assert!(!l.exempt_holder(2, t0), "another session's lock");
        assert!(!l.holder_exempt(t0));
        l.release(3, t0);
        assert!(l.acquire(2, 5 * SEC, t0).is_some());
        assert!(l.exempt_holder(2, t0));
        // Not renewed: it lapses, and the next grant (anyone's) is budgeted.
        let later = t0 + 10 * SEC;
        assert!(!l.holder_exempt(later));
        assert_eq!(l.acquire(4, 60 * SEC, later), Some(60 * SEC));
        assert!(!l.holder_exempt(later));
        assert_eq!(l.acquire(4, 60 * SEC, later + 59 * SEC), Some(60 * SEC));
        assert_eq!(
            l.acquire(4, 60 * SEC, later + 100 * SEC),
            Some(20 * SEC),
            "budgeted"
        );
    }
}
