//! The per-device exclusive lock (spec 7.7). Owned by WP HB2, which adds the wire
//! messages and `device.changed`. Pure: the caller passes the clock.

use std::time::{Duration, Instant};

use crate::session::SessionId;

/// Longest a lock may be held without renewal (spec 7.7).
pub const LOCK_MAX: Duration = Duration::from_secs(60);

/// Who holds a device, until when.
#[derive(Debug, Default)]
pub struct LockTable {
    holder: Option<(SessionId, Instant)>,
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

    /// Take or renew the lock, for at most [`LOCK_MAX`]. False if another session
    /// holds a live one.
    pub fn acquire(&mut self, session: SessionId, timeout: Duration, now: Instant) -> bool {
        if !self.permits(session, now) {
            return false;
        }
        let until = now.checked_add(timeout.min(LOCK_MAX)).unwrap_or(now);
        self.holder = Some((session, until));
        true
    }

    /// Release the lock if `session` holds it.
    pub fn release(&mut self, session: SessionId) {
        if self.holder.is_some_and(|(h, _)| h == session) {
            self.holder = None;
        }
    }
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
        assert!(l.acquire(2, s, t0));
        assert!(l.permits(2, t0));
        assert!(!l.permits(3, t0));
        assert!(!l.acquire(3, s, t0));
        l.release(3);
        assert_eq!(l.holder(t0), Some(2), "only the holder releases");
        assert!(l.permits(3, t0 + 2 * s), "expired");
        assert!(l.acquire(3, s, t0 + 2 * s));
        l.release(3);
        assert_eq!(l.holder(t0 + 2 * s), None);
    }

    #[test]
    fn wp_hb0_a_lock_is_capped() {
        let t0 = Instant::now();
        let mut l = LockTable::default();
        assert!(l.acquire(2, Duration::from_secs(3600), t0));
        assert_eq!(l.holder(t0 + LOCK_MAX), None);
    }
}
