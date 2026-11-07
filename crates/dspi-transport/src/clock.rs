//! The core's one source of time (WP WB1).
//!
//! `std::time::Instant::now`, `std::time::SystemTime::now` and
//! `std::thread::sleep` compile for `wasm32-unknown-unknown` and panic when they
//! run. Session and core code takes time from here, so a web build needs no change
//! at each call site.
//!
//! **Native:** these are `std`'s own items. [`Instant`] is a re-export of
//! `std::time::Instant` and the two functions forward to `std`, so timing is what
//! it was. The same goes for the other wasm targets (`emscripten`, `wasi`), whose
//! `std` has a clock.
//!
//! **`wasm32-unknown-unknown`:** the page supplies three imports from the wasm
//! import module `dspi`. A module that uses none of them does not import them.
//!
//! | Import | JavaScript |
//! |---|---|
//! | `dspi_now_ms() -> f64` | `performance.now()`: monotonic milliseconds |
//! | `dspi_unix_ms() -> f64` | `Date.now()`: milliseconds since the Unix epoch |
//! | `dspi_sleep_ms(u32)` | a timer. Under JSPI, a `WebAssembly.Suspending` wrapper around one |
//!
//! There, unlike `std`, [`Instant`]'s arithmetic saturates rather than panics: the
//! values reach it from a page, and a page's clock starts near zero.
//!
//! This covers time only. Threads, and what blocks on them, stay as they are:
//! `docs/core-merge/WEB-STUDY.md` section 11 lists what a web build still needs.

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::Instant;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::{Duration, SystemTime};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::{Instant, sleep, system_time};

/// Block the calling thread for `d`: `std::thread::sleep`.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[inline]
pub fn sleep(d: Duration) {
    std::thread::sleep(d);
}

/// The wall clock: `SystemTime::now`. The result is a plain `std` value, so
/// `duration_since(UNIX_EPOCH)` works as usual on every target.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[inline]
pub fn system_time() -> SystemTime {
    SystemTime::now()
}

/// The page's clock: the real imports on the web, a settable fake in native tests.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod host {
    #[link(wasm_import_module = "dspi")]
    unsafe extern "C" {
        safe fn dspi_now_ms() -> f64;
        safe fn dspi_unix_ms() -> f64;
        safe fn dspi_sleep_ms(ms: u32);
    }

    pub(super) fn now_ms() -> f64 {
        dspi_now_ms()
    }

    pub(super) fn unix_ms() -> f64 {
        dspi_unix_ms()
    }

    pub(super) fn sleep_ms(ms: u32) {
        dspi_sleep_ms(ms);
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod host {
    use std::cell::Cell;

    thread_local! {
        pub(super) static NOW_MS: Cell<f64> = const { Cell::new(0.0) };
        pub(super) static UNIX_MS: Cell<f64> = const { Cell::new(0.0) };
        pub(super) static SLEPT_MS: Cell<Option<u32>> = const { Cell::new(None) };
    }

    pub(super) fn now_ms() -> f64 {
        NOW_MS.get()
    }

    pub(super) fn unix_ms() -> f64 {
        UNIX_MS.get()
    }

    pub(super) fn sleep_ms(ms: u32) {
        SLEPT_MS.set(Some(ms));
    }
}

/// The web's clock over [`host`]. Compiled natively for tests, which set the host.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
mod web {
    use std::ops::{Add, AddAssign, Sub, SubAssign};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::host;

    /// Milliseconds from a page as a `Duration`. A value that is negative or not a
    /// number counts as zero, and one too large for a `Duration` as the largest.
    fn from_ms(ms: f64) -> Duration {
        if ms.is_nan() || ms <= 0.0 {
            return Duration::ZERO;
        }
        Duration::try_from_secs_f64(ms / 1000.0).unwrap_or(Duration::MAX)
    }

    /// `std::time::Instant` for a page: time since the page's own origin.
    ///
    /// The API is the part of `std`'s the crates use, without its panics:
    /// `+` and `-` saturate at the ends of the range.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Instant(Duration);

    impl Instant {
        /// The current time.
        pub fn now() -> Self {
            Self(from_ms(host::now_ms()))
        }

        /// Time since this instant; zero if the clock has not reached it.
        pub fn elapsed(&self) -> Duration {
            Self::now().saturating_duration_since(*self)
        }

        /// Time from `earlier` to this instant; zero if `earlier` is later.
        pub fn duration_since(&self, earlier: Self) -> Duration {
            self.saturating_duration_since(earlier)
        }

        /// Time from `earlier` to this instant; zero if `earlier` is later.
        pub fn saturating_duration_since(&self, earlier: Self) -> Duration {
            self.0.saturating_sub(earlier.0)
        }

        /// Time from `earlier` to this instant; `None` if `earlier` is later.
        pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> {
            self.0.checked_sub(earlier.0)
        }

        /// This instant plus `d`, if that can be represented.
        pub fn checked_add(&self, d: Duration) -> Option<Self> {
            self.0.checked_add(d).map(Self)
        }

        /// This instant minus `d`, if that is not before the page's origin.
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

    impl SubAssign<Duration> for Instant {
        fn sub_assign(&mut self, d: Duration) {
            *self = *self - d;
        }
    }

    impl Sub for Instant {
        type Output = Duration;

        fn sub(self, earlier: Self) -> Duration {
            self.saturating_duration_since(earlier)
        }
    }

    /// Wait for `d`, in whole milliseconds and never less than asked for, up to
    /// `u32::MAX` ms.
    pub fn sleep(d: Duration) {
        let ms = d
            .as_millis()
            .saturating_add(u128::from(!d.subsec_nanos().is_multiple_of(1_000_000)));
        host::sleep_ms(u32::try_from(ms).unwrap_or(u32::MAX));
    }

    /// The wall clock. A page's clock set before the epoch reads as the epoch, which
    /// the callers' `duration_since(UNIX_EPOCH)` turn into their usual zero.
    pub fn system_time() -> SystemTime {
        UNIX_EPOCH
            .checked_add(from_ms(host::unix_ms()))
            .unwrap_or(UNIX_EPOCH)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_wb1 {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{host, web};

    fn at(ms: f64) -> web::Instant {
        host::NOW_MS.set(ms);
        web::Instant::now()
    }

    /// Native: the seam is `std` itself, not a lookalike.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn wb1_native_instant_is_std() {
        let seam: super::Instant = super::Instant::now();
        let std_instant: std::time::Instant = seam;
        assert!(std::time::Instant::now() >= std_instant);
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn wb1_native_sleep_waits() {
        let start = std::time::Instant::now();
        super::sleep(Duration::from_millis(20));
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn wb1_native_system_time_is_the_wall_clock() {
        let before = std::time::SystemTime::now();
        let seam = super::system_time();
        let after = std::time::SystemTime::now();
        assert!(before <= seam && seam <= after);
    }

    #[test]
    fn wb1_web_now_follows_the_host() {
        let a = at(1500.0);
        let b = at(1750.5);
        assert_eq!(b - a, Duration::from_micros(250_500));
        assert_eq!(b.duration_since(a), Duration::from_micros(250_500));
        assert_eq!(a.elapsed(), Duration::from_micros(250_500));
        assert!(a < b);
    }

    #[test]
    fn wb1_web_bad_host_values_read_as_zero_or_the_maximum() {
        for bad in [-1.0, f64::NAN, f64::NEG_INFINITY, -0.0] {
            assert_eq!(at(bad), at(0.0), "{bad}");
        }
        assert_eq!(at(f64::INFINITY), at(f64::MAX));
        assert_eq!(at(f64::MAX) - at(0.0), Duration::MAX);
    }

    #[test]
    fn wb1_web_durations_saturate_where_time_runs_backwards() {
        let (a, b) = (at(100.0), at(200.0));
        assert_eq!(a - b, Duration::ZERO);
        assert_eq!(a.duration_since(b), Duration::ZERO);
        assert_eq!(a.saturating_duration_since(b), Duration::ZERO);
        assert_eq!(a.checked_duration_since(b), None);
        assert_eq!(
            b.checked_duration_since(a),
            Some(Duration::from_millis(100))
        );
        // The clock is at 200 ms: `a + 1 s` has not been reached.
        assert_eq!((a + Duration::from_secs(1)).elapsed(), Duration::ZERO);
    }

    #[test]
    fn wb1_web_arithmetic_saturates_at_both_ends() {
        let t = at(100.0);
        assert_eq!(t + Duration::from_millis(50), at(150.0));
        assert_eq!(t + Duration::MAX, at(f64::MAX));
        // A page's clock starts near zero, so this is an ordinary case here.
        assert_eq!(t - Duration::from_secs(5), at(0.0));
        assert_eq!(t - Duration::from_millis(40), at(60.0));

        let mut m = t;
        m += Duration::from_millis(20);
        assert_eq!(m, at(120.0));
        m -= Duration::from_millis(70);
        assert_eq!(m, at(50.0));
    }

    #[test]
    fn wb1_web_checked_arithmetic_reports_the_ends() {
        let t = at(100.0);
        assert_eq!(t.checked_add(Duration::from_millis(1)), Some(at(101.0)));
        assert_eq!(t.checked_add(Duration::MAX), None);
        assert_eq!(t.checked_sub(Duration::from_millis(1)), Some(at(99.0)));
        assert_eq!(t.checked_sub(Duration::from_secs(1)), None);
    }

    #[test]
    fn wb1_web_sleep_rounds_up_and_clamps() {
        for (asked, ms) in [
            (Duration::ZERO, 0),
            (Duration::from_millis(50), 50),
            (Duration::from_micros(1), 1),
            (Duration::from_micros(2500), 3),
            (Duration::from_millis(u64::from(u32::MAX)), u32::MAX),
            (Duration::MAX, u32::MAX),
        ] {
            web::sleep(asked);
            assert_eq!(host::SLEPT_MS.get(), Some(ms), "{asked:?}");
        }
    }

    #[test]
    fn wb1_web_system_time_reads_the_host_wall_clock() {
        host::UNIX_MS.set(1_700_000_000_250.0);
        let since = web::system_time().duration_since(UNIX_EPOCH).ok();
        assert_eq!(since, Some(Duration::from_millis(1_700_000_000_250)));
    }

    #[test]
    fn wb1_web_system_time_never_panics_on_a_bad_clock() {
        for bad in [-5.0, f64::NAN, f64::INFINITY, f64::MAX] {
            host::UNIX_MS.set(bad);
            assert!(web::system_time() >= UNIX_EPOCH, "{bad}");
        }
    }
}
