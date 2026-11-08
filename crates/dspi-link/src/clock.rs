//! The time source of the session's liveness timers, so tests need no long
//! real sleeps: a test owns a [`ManualClock`] and moves it past a limit.
//!
//! [`Instant`] and [`sleep`] are the crate's own time (WP WB2). Natively they
//! are `std`'s: a re-export and a forwarding function, so timing is unchanged,
//! and the type is the one `dspi_transport::clock` re-exports. On
//! `wasm32-unknown-unknown`, where `std`'s clock and sleep panic, they call the
//! page's imports `dspi.dspi_now_ms` and `dspi.dspi_sleep_ms`, the same two as
//! `dspi_transport::clock`. That seam lives in `dspi-transport`, which depends
//! on this crate, so it cannot be used from here; this copy keeps the time base
//! the same, with a type of its own on the web only.

use std::fmt::Debug;
use std::sync::Mutex;
use std::time::Duration;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::{sleep, Instant};

/// Block the calling thread for `d`: `std::thread::sleep`.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[inline]
pub fn sleep(d: Duration) {
    std::thread::sleep(d);
}

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

/// The page's clock (WP WB2): what this crate uses of `std::time::Instant`,
/// saturating instead of panicking, as `dspi_transport::clock`'s does.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web {
    use std::ops::{Add, AddAssign, Sub};
    use std::time::Duration;

    #[link(wasm_import_module = "dspi")]
    unsafe extern "C" {
        safe fn dspi_now_ms() -> f64;
        safe fn dspi_sleep_ms(ms: u32);
    }

    /// Time since the page's own origin.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Instant(Duration);

    impl Instant {
        pub fn now() -> Self {
            let ms = dspi_now_ms();
            if ms.is_nan() || ms <= 0.0 {
                return Self(Duration::ZERO);
            }
            Self(Duration::try_from_secs_f64(ms / 1000.0).unwrap_or(Duration::MAX))
        }

        pub fn elapsed(&self) -> Duration {
            Self::now().saturating_duration_since(*self)
        }

        pub fn duration_since(&self, earlier: Self) -> Duration {
            self.saturating_duration_since(earlier)
        }

        pub fn saturating_duration_since(&self, earlier: Self) -> Duration {
            self.0.saturating_sub(earlier.0)
        }

        pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> {
            self.0.checked_sub(earlier.0)
        }

        pub fn checked_add(&self, d: Duration) -> Option<Self> {
            self.0.checked_add(d).map(Self)
        }

        pub fn checked_sub(&self, d: Duration) -> Option<Self> {
            self.0.checked_sub(d).map(Self)
        }
    }

    impl Add<Duration> for Instant {
        type Output = Self;

        fn add(self, d: Duration) -> Self {
            Self(self.0.saturating_add(d))
        }
    }

    impl AddAssign<Duration> for Instant {
        fn add_assign(&mut self, d: Duration) {
            *self = *self + d;
        }
    }

    impl Sub<Duration> for Instant {
        type Output = Self;

        fn sub(self, d: Duration) -> Self {
            Self(self.0.saturating_sub(d))
        }
    }

    impl Sub for Instant {
        type Output = Duration;

        fn sub(self, earlier: Self) -> Duration {
            self.saturating_duration_since(earlier)
        }
    }

    /// Wait for `d`, in whole milliseconds and never less than asked for.
    pub fn sleep(d: Duration) {
        // -- WP WB4b -- (the binding's clippy now reaches this crate)
        let ms = d.as_millis().saturating_add(u128::from(!d.subsec_nanos().is_multiple_of(1_000_000)));
        dspi_sleep_ms(u32::try_from(ms).unwrap_or(u32::MAX));
    }
}

/// `getrandom`'s custom backend on the web (WP WB2), which `tungstenite`
/// reaches through `rand` for the WebSocket key and masks. The build sets
/// `--cfg getrandom_backend="custom"` for this target (`.cargo/config.toml`);
/// the page supplies `dspi.dspi_random_fill(ptr, len) -> i32`, filling `len`
/// bytes at `ptr` in the module's memory with `crypto.getRandomValues` and
/// answering 0. It is defined here, beside the dependency that needs it,
/// rather than in each binding, so a module built with `link` links once.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod random {
    #[link(wasm_import_module = "dspi")]
    unsafe extern "C" {
        fn dspi_random_fill(ptr: *mut u8, len: usize) -> i32;
    }

    /// # Safety
    /// `dest` is valid for `len` bytes, as `getrandom` promises.
    #[no_mangle]
    unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom::Error> {
        // SAFETY: the page writes `len` bytes at `dest`, inside the module's
        // memory, which `getrandom` handed us for exactly that.
        match unsafe { dspi_random_fill(dest, len) } {
            0 => Ok(()),
            _ => Err(getrandom::Error::UNSUPPORTED),
        }
    }
}
