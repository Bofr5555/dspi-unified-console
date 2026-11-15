//! The hub services' time (WP P2SV): the monotonic clock their timers run on and the
//! wall clock their records and schedules read, behind one seam, so a test moves time
//! instead of sleeping.
//!
//! [`SystemClock`] is the real one. [`TestClock`] moves only when told to: `advance`
//! moves both clocks, `set_wall` jumps the wall clock alone (a hub without a battery
//! clock that the network sets after boot). A service reads the time it is given
//! (`run_due(now)`) or [`HubClock::now`]; never `Instant::now()` itself.

use std::fmt::Debug;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The time now, on both clocks.
pub trait HubClock: Send + Sync + Debug {
    /// The monotonic clock: what `next_due` and `run_due` are measured on.
    fn now(&self) -> Instant;
    /// The wall clock.
    fn wall(&self) -> SystemTime;

    /// The wall clock in ms since the Unix epoch (an event record's `t`); 0 before it.
    fn unix_ms(&self) -> i64 {
        self.wall()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    /// The wall clock in seconds since the Unix epoch (an alarm's `at`); 0 before it.
    fn unix_seconds(&self) -> u64 {
        self.wall()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// Real time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl HubClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Time that moves only when a test says so.
#[derive(Debug)]
pub struct TestClock {
    now: Mutex<(Instant, SystemTime)>,
}

impl TestClock {
    /// Both clocks at the real time now.
    pub fn new() -> Self {
        Self::at(SystemTime::now())
    }

    /// The monotonic clock at the real time now, the wall clock at `wall`.
    pub fn at(wall: SystemTime) -> Self {
        Self {
            now: Mutex::new((Instant::now(), wall)),
        }
    }

    /// Move both clocks on by `by`.
    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        now.0 = now.0.checked_add(by).unwrap_or(now.0);
        now.1 = now.1.checked_add(by).unwrap_or(now.1);
    }

    /// Set the wall clock alone, forward or back; the monotonic clock does not move.
    pub fn set_wall(&self, wall: SystemTime) {
        self.now.lock().unwrap_or_else(PoisonError::into_inner).1 = wall;
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HubClock for TestClock {
    fn now(&self) -> Instant {
        self.now.lock().unwrap_or_else(PoisonError::into_inner).0
    }

    fn wall(&self) -> SystemTime {
        self.now.lock().unwrap_or_else(PoisonError::into_inner).1
    }
}

#[cfg(test)]
mod tests_wp_p2sv {
    #![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
    use super::*;

    #[test]
    fn wp_p2sv_the_test_clock_moves_only_when_told() {
        let start = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let c = TestClock::at(start);
        let t0 = c.now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(c.now(), t0, "the monotonic clock stood still");
        assert_eq!(c.wall(), start);
        c.advance(Duration::from_secs(2));
        assert_eq!(c.now() - t0, Duration::from_secs(2));
        assert_eq!(c.unix_seconds(), 1_800_000_002);
        assert_eq!(c.unix_ms(), 1_800_000_002_000);
        // A wall-clock jump back leaves the monotonic clock where it was.
        c.set_wall(start - Duration::from_secs(3600));
        assert_eq!(c.now() - t0, Duration::from_secs(2));
        assert_eq!(c.unix_seconds(), 1_799_996_400);
        c.set_wall(UNIX_EPOCH - Duration::from_secs(1));
        assert_eq!(c.unix_ms(), 0, "before the epoch reads as 0, no panic");
    }

    #[test]
    fn wp_p2sv_the_system_clock_is_real_time() {
        let before = Instant::now();
        let c = SystemClock;
        assert!(c.now() >= before);
        assert!(c.unix_seconds() > 1_700_000_000);
    }
}
