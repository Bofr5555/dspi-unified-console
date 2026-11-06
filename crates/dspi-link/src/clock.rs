//! The time source of the session's liveness timers, so tests need no long
//! real sleeps: a test owns a [`ManualClock`] and moves it past a limit.

use std::fmt::Debug;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync + Debug {
    fn now(&self) -> Instant;
}

/// Real time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Time that moves only when told to.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<Instant>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self { now: Mutex::new(Instant::now()) }
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(|p| p.into_inner());
        *now += by;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(|p| p.into_inner())
    }
}
