//! Meter ballistics (WP P0M, PRO1 M2): peak hold with decay, the clip hold,
//! and each channel's peak maximum since a reset, computed once from the raw
//! block peaks the device sends.
//!
//! The firmware does no ballistics: `REQ_GET_STATUS` 9 is the peak of the last
//! block per channel and a sticky clip bit per channel, set until
//! `REQ_CLEAR_CLIPS` (`config.h`, `global_status.clip_flags`). Every client
//! used to hold, decay and latch on its own; this is the one copy. It lives in
//! the session because the Terminal does not link `dspi-core`, which applies it
//! inside `fetch_status` for C, UniFFI and wasm.
//!
//! Pure: time comes in as an [`Instant`] from the caller (the clock seam,
//! WP WB1), and the one piece of device I/O the clip hold needs, clearing the
//! device's sticky flags when the hold ends, is returned as
//! [`MeterUpdate::clear_due`] for the caller to send.
//!
//! # Defaults ([`BallisticsConfig::DEFAULT`])
//!
//! - **Peak hold 1 s, then a linear fall of 0.5 full scale per second.** The
//!   macOS console draws no hold marker (its bar glides 60 ms between polls,
//!   `Components.swift` `MeterBarNSView`); these are the Terminal's values,
//!   which drew the only hold. A client that wants the Mac's look ignores
//!   `peak_hold`.
//! - **Clip hold 10 s after the last new clip, then the device is cleared**:
//!   the macOS console's latch (`Commands.swift` `fetchStatus`: OR the flags
//!   into a latch, stamp it when a new bit appears, clear both the latch and
//!   the device 10 s after the stamp). A channel still clipping lights again
//!   on the first read after the clear; where the clear is refused (a viewer
//!   on a hub) the hold follows the device's sticky bits ([`Ballistics`]).
//! - **Peak maximum** never falls until [`Ballistics::reset_max`] or a new
//!   device.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
#![warn(missing_docs)]

use std::time::Duration;

use dspi_transport::clock::Instant;

/// Channels the meter block carries at most (`MAX_CHANNELS` on the wire).
pub const METER_CHANNELS: usize = 17;

/// The ballistics' timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallisticsConfig {
    /// How long a new peak holds before it starts to fall, ms.
    pub hold_ms: u32,
    /// How fast a held peak falls once its hold ends, in linear full scale
    /// per second. 0 keeps it until a higher peak or a reset.
    pub decay_per_s: f32,
    /// How long the clip hold lasts after the last new clip, ms. 0 holds
    /// until [`Ballistics::clear_clips`].
    pub clip_hold_ms: u32,
}

impl BallisticsConfig {
    /// The defaults described in the module documentation.
    pub const DEFAULT: Self = Self {
        hold_ms: 1000,
        decay_per_s: 0.5,
        clip_hold_ms: 10_000,
    };
}

impl Default for BallisticsConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One channel's held peak.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Held {
    peak: f32,
    /// Hold time left, seconds.
    hold_s: f32,
    max: f32,
}

/// How long a clear's echo is ignored at most: bits a clear dropped stay out
/// of the hold until a read taken after the clear succeeded arrives, or this
/// long, whichever is first. Covers DSPi Link's 2 Hz meter feed with room.
pub const ECHO_GUARD_MS: u64 = 2000;

/// What one [`Ballistics::update`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeterUpdate {
    /// Channels whose clip hold lit on this update (bit per wire channel):
    /// one event per clip episode, which is what a log records.
    pub new_clips: u32,
    /// `new_clips` came with the first read after a reset: the device's
    /// sticky flags may be from before this connection, not fresh clips.
    pub from_before: bool,
    /// The clip hold just ended: clear the device's sticky flags
    /// (`REQ_CLEAR_CLIPS`, `meters.clear`) and report how it went with
    /// [`Ballistics::clear_result`].
    pub clear_due: bool,
}

/// Peak hold, clip hold and peak maximum for one device's meters.
///
/// # The clip hold
///
/// A clip lights the hold; it goes dark `clip_hold_ms` after the last new clip
/// and asks for the device's sticky flags to be cleared ([`MeterUpdate::clear_due`]).
/// The caller sends the clear and reports it ([`Ballistics::clear_result`]):
///
/// - **Cleared:** a read taken after the clear is the truth. A bit set there
///   is a new clip, so a channel that keeps clipping lights again on the next
///   read (the macOS console re-latches the same way). Reads taken before the
///   clear (an older DSPi Link frame, by its age) are its echo and ignored, for
///   at most [`ECHO_GUARD_MS`].
/// - **Refused or failed** (a viewer on a hub may not clear; a lost
///   transfer), or no answer within [`ECHO_GUARD_MS`]: the device keeps its
///   sticky bits, so the hold follows them: lit while the device reports a bit,
///   dark `clip_hold_ms` after the last read that showed one. No further clear
///   is asked for on this connection, so nothing is retried.
#[derive(Debug, Clone)]
pub struct Ballistics {
    config: BallisticsConfig,
    channels: Vec<Held>,
    /// The clip hold, bit per wire channel.
    clip_hold: u32,
    /// When the hold's timer last started: a new clip, or (when the device
    /// cannot be cleared) a read that still showed a held bit.
    last_new_clip: Option<Instant>,
    /// Bits a clear dropped whose echo may still arrive.
    echo: u32,
    /// The latest the echo guard lasts.
    echo_until: Option<Instant>,
    /// When the last clear succeeded: reads taken after it are trusted.
    cleared_at: Option<Instant>,
    /// A clear was asked for and not answered yet.
    clear_pending: bool,
    /// False once a clear failed on this connection.
    can_clear: bool,
    /// A read has been fed since the last reset.
    seen: bool,
    last: Option<Instant>,
}

impl Default for Ballistics {
    fn default() -> Self {
        Self::new(BallisticsConfig::DEFAULT)
    }
}

impl Ballistics {
    /// Empty ballistics with `config`.
    pub fn new(config: BallisticsConfig) -> Self {
        Self {
            config,
            channels: vec![Held::default(); METER_CHANNELS],
            clip_hold: 0,
            last_new_clip: None,
            echo: 0,
            echo_until: None,
            cleared_at: None,
            clear_pending: false,
            can_clear: true,
            seen: false,
            last: None,
        }
    }

    /// The timing in use.
    pub fn config(&self) -> BallisticsConfig {
        self.config
    }

    /// Change the timing; what is held stays. A negative or non-finite decay
    /// reads as 0.
    pub fn set_config(&mut self, config: BallisticsConfig) {
        let decay = if config.decay_per_s.is_finite() {
            config.decay_per_s.max(0.0)
        } else {
            0.0
        };
        self.config = BallisticsConfig {
            decay_per_s: decay,
            ..config
        };
    }

    /// Forget everything: a new device or a reconnect.
    pub fn reset(&mut self) {
        *self = Self::new(self.config);
    }

    /// Start every channel's peak maximum again from now.
    pub fn reset_max(&mut self) {
        for c in &mut self.channels {
            c.max = 0.0;
        }
    }

    /// Drop the clip hold now (a user's "clear clips"). The caller clears the
    /// device and reports it with [`Ballistics::clear_result`]; if the clear
    /// fails, the bits the device still reports come back.
    pub fn clear_clips(&mut self, now: Instant) {
        self.start_clear(now);
    }

    /// Whether the clear asked for (a hold that ended, or
    /// [`Ballistics::clear_clips`]) reached the device: `ok` when
    /// `REQ_CLEAR_CLIPS` was answered, false when it was refused or failed.
    pub fn clear_result(&mut self, now: Instant, ok: bool) {
        if !self.clear_pending {
            return;
        }
        self.clear_pending = false;
        if ok {
            self.cleared_at = Some(now);
        } else {
            self.clear_failed(now);
        }
    }

    /// Whether a clear failed on this connection, so the hold follows the
    /// device's sticky bits.
    pub fn can_clear(&self) -> bool {
        self.can_clear
    }

    fn start_clear(&mut self, now: Instant) {
        self.echo |= self.clip_hold;
        self.clip_hold = 0;
        self.last_new_clip = None;
        self.echo_until = now.checked_add(Duration::from_millis(ECHO_GUARD_MS));
        self.cleared_at = None;
        self.clear_pending = true;
    }

    /// The device keeps its sticky bits: the dropped ones come back, as the
    /// same episode, and the hold follows the device from now on.
    fn clear_failed(&mut self, now: Instant) {
        self.can_clear = false;
        self.clear_pending = false;
        self.clip_hold |= self.echo;
        self.echo = 0;
        self.echo_until = None;
        if self.clip_hold != 0 {
            self.last_new_clip = Some(now);
        }
    }

    /// Feed one meter read taken just now: `peaks` linear 0..1 per wire
    /// channel (more than [`METER_CHANNELS`] are ignored, fewer leave the rest
    /// at 0) and the device's sticky `clip_flags`.
    pub fn update(&mut self, now: Instant, peaks: &[f32], clip_flags: u32) -> MeterUpdate {
        self.update_aged(now, Duration::ZERO, peaks, clip_flags)
    }

    /// [`Ballistics::update`] for a read that is `age` old at `now` (a DSPi
    /// Link poll frame, `SystemStatus.age_ms`): the age decides whether it
    /// was taken after a clear.
    pub fn update_aged(
        &mut self,
        now: Instant,
        age: Duration,
        peaks: &[f32],
        clip_flags: u32,
    ) -> MeterUpdate {
        let dt = self
            .last
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.last = Some(now);
        let hold_s = ms(self.config.hold_ms).as_secs_f32();
        let decay = self.config.decay_per_s;
        for (i, c) in self.channels.iter_mut().enumerate() {
            let level = sane(peaks.get(i).copied().unwrap_or(0.0));
            c.max = c.max.max(level);
            if level >= c.peak {
                c.peak = level;
                c.hold_s = hold_s;
            } else if c.hold_s >= dt {
                c.hold_s -= dt;
            } else {
                // The hold ran out inside this interval (or before it): fall
                // for the rest of it, so the hold lasts `hold_ms` whatever
                // the poll rate.
                let fall = dt - c.hold_s;
                c.hold_s = 0.0;
                c.peak = (c.peak - decay * fall).max(level);
            }
        }
        let first = !self.seen;
        self.seen = true;
        self.release_echo(now, now.checked_sub(age).unwrap_or(now));
        let mut u = self.note(now, clip_flags & !self.echo, true);
        u.from_before = first && u.new_clips != 0;
        u
    }

    /// End the echo guard when a read from after a successful clear arrived,
    /// or when it has lasted [`ECHO_GUARD_MS`] (an unanswered clear counts as
    /// failed then).
    fn release_echo(&mut self, now: Instant, taken: Instant) {
        if self.echo == 0 && !self.clear_pending {
            return;
        }
        let expired = self.echo_until.is_some_and(|u| now >= u);
        if self.clear_pending {
            if expired {
                self.clear_failed(now);
            }
            return;
        }
        if expired || self.cleared_at.is_some_and(|c| taken >= c) {
            self.echo = 0;
            self.echo_until = None;
        }
    }

    /// Add clips that arrived another way than the meter block (a firmware
    /// clip notification, WP FWX) to the hold: `clips` a bit per wire channel.
    /// Returns what [`Ballistics::update`] would.
    pub fn note_clips(&mut self, now: Instant, clips: u32) -> MeterUpdate {
        self.note(now, clips, false)
    }

    /// `clips` (sticky flags when `sticky`) into the hold; ends it when due.
    fn note(&mut self, now: Instant, clips: u32, sticky: bool) -> MeterUpdate {
        let new_clips = clips & !self.clip_hold;
        if new_clips != 0 {
            self.clip_hold |= new_clips;
            self.last_new_clip = Some(now);
        }
        // A device that cannot be cleared: a held bit it still reports keeps
        // the hold lit.
        if sticky && !self.can_clear && clips & self.clip_hold != 0 {
            self.last_new_clip = Some(now);
        }
        let ended = self.config.clip_hold_ms > 0
            && self.clip_hold != 0
            && self
                .last_new_clip
                .is_some_and(|t| now.saturating_duration_since(t) >= ms(self.config.clip_hold_ms));
        let mut clear_due = false;
        if ended {
            if self.can_clear && !self.clear_pending {
                self.start_clear(now);
                clear_due = true;
            } else if self.clear_pending {
                // One clear at a time: this one's answer covers these too.
                self.echo |= self.clip_hold;
                self.clip_hold = 0;
                self.last_new_clip = None;
            } else {
                self.clip_hold = 0;
                self.last_new_clip = None;
            }
        }
        MeterUpdate {
            new_clips,
            from_before: false,
            clear_due,
        }
    }

    /// The held peak per wire channel, [`METER_CHANNELS`] entries.
    pub fn peak_hold(&self) -> Vec<f32> {
        self.channels.iter().map(|c| c.peak).collect()
    }

    /// One channel's held peak; 0 past the end.
    pub fn hold(&self, channel: usize) -> f32 {
        self.channels.get(channel).map_or(0.0, |c| c.peak)
    }

    /// The peak maximum since the last reset per wire channel,
    /// [`METER_CHANNELS`] entries.
    pub fn peak_max(&self) -> Vec<f32> {
        self.channels.iter().map(|c| c.max).collect()
    }

    /// The clip hold, bit per wire channel.
    pub fn clip_hold(&self) -> u32 {
        self.clip_hold
    }
}

fn ms(v: u32) -> Duration {
    Duration::from_millis(u64::from(v))
}

/// A level from the wire is 0..1; anything else (NaN from a bad decode)
/// must not stick in a hold or a maximum.
fn sane(level: f32) -> f32 {
    if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0m {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn wp_p0m_peak_holds_then_falls_linearly() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.update(t0, &[0.8], 0);
        // Held for the full second, whatever the level does.
        for step in 1..=10 {
            b.update(at(t0, step * 100), &[0.1], 0);
        }
        assert!(close(b.hold(0), 0.8), "held: {}", b.hold(0));
        // Then 0.5 per second from the end of the hold: 0.3 s later 0.15 lower.
        b.update(at(t0, 1100), &[0.1], 0);
        b.update(at(t0, 1300), &[0.1], 0);
        assert!(close(b.hold(0), 0.65), "falling: {}", b.hold(0));
        // Never below the live level.
        b.update(at(t0, 5000), &[0.1], 0);
        assert!(close(b.hold(0), 0.1));
        // A higher peak takes over at once and holds again.
        b.update(at(t0, 5100), &[0.9], 0);
        b.update(at(t0, 5900), &[0.0], 0);
        assert!(close(b.hold(0), 0.9));
    }

    #[test]
    fn wp_p0m_the_terminal_sequence_is_unchanged() {
        // The Terminal's own PeakHold test, on the shared code: 1 s hold,
        // then 0.5/s down to the live level.
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.update(t0, &[0.8], 0);
        b.update(at(t0, 500), &[0.2], 0);
        assert!(close(b.hold(0), 0.8));
        b.update(at(t0, 1000), &[0.2], 0);
        b.update(at(t0, 1400), &[0.2], 0);
        assert!(close(b.hold(0), 0.6), "{}", b.hold(0));
        b.update(at(t0, 10_000), &[0.2], 0);
        assert!(close(b.hold(0), 0.2));
    }

    #[test]
    fn wp_p0m_the_maximum_stays_until_reset() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.update(t0, &[0.3, 0.6], 0);
        b.update(at(t0, 100), &[0.5, 0.1], 0);
        b.update(at(t0, 60_000), &[0.0, 0.0], 0);
        assert_eq!(&b.peak_max()[..2], &[0.5, 0.6]);
        assert_eq!(b.peak_max().len(), METER_CHANNELS);
        b.reset_max();
        b.update(at(t0, 60_100), &[0.2, 0.0], 0);
        assert_eq!(&b.peak_max()[..2], &[0.2, 0.0]);
    }

    #[test]
    fn wp_p0m_clip_hold_is_ten_seconds_after_the_last_new_clip() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        let u = b.update(t0, &[], 0b0100);
        assert_eq!(u.new_clips, 0b0100);
        // The sticky bit repeats: no new episode.
        let u = b.update(at(t0, 2000), &[], 0b0100);
        assert_eq!(u, MeterUpdate::default());
        // A second channel restarts the ten seconds.
        let u = b.update(at(t0, 6000), &[], 0b0110);
        assert_eq!(u.new_clips, 0b0010);
        let u = b.update(at(t0, 15_000), &[], 0b0110);
        assert!(!u.clear_due);
        assert_eq!(b.clip_hold(), 0b0110);
        let u = b.update(at(t0, 16_000), &[], 0b0110);
        assert!(u.clear_due);
        assert_eq!(b.clip_hold(), 0);
        // Before the clear is answered the bits are its echo.
        let u = b.update(at(t0, 16_050), &[], 0b0110);
        assert_eq!(u.new_clips, 0);
        b.clear_result(at(t0, 16_060), true);
        // A read after the clear that shows them clear: dark.
        b.update(at(t0, 16_100), &[], 0);
        assert_eq!(b.clip_hold(), 0);
        // A clip after that is a new episode.
        let u = b.update(at(t0, 16_300), &[], 0b0010);
        assert_eq!(u.new_clips, 0b0010);
    }

    /// Review p0m F1 (the reviewer's 120 s probe, adopted): a channel that
    /// clips without pause stays shown as clipping. The device sets the
    /// sticky bit again within a block after each clear, so every read shows
    /// it; after each clear the next read lights the hold again, as the macOS
    /// console re-latches.
    #[test]
    fn wp_p0m_review_f1_a_sustained_clip_stays_lit_over_usb() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        let (mut lit, mut reads, mut clears, mut episodes) = (0u32, 0u32, 0u32, 0u32);
        let mut t = 0;
        while t <= 120_000 {
            let now = at(t0, t);
            let u = b.update(now, &[1.0], 1);
            episodes += u32::from(u.new_clips != 0);
            if u.clear_due {
                clears += 1;
                // USB: the clear goes out at once and is answered.
                b.clear_result(now, true);
            }
            reads += 1;
            lit += u32::from(b.clip_hold() & 1 != 0);
            t += 60;
        }
        assert!(clears >= 10, "a clear every ten seconds: {clears}");
        assert_eq!(episodes, clears + 1, "lit again after each clear");
        // Dark for one read per clear at most.
        assert!(
            reads - lit <= clears,
            "dark reads {} of {reads}",
            reads - lit
        );
        // And in the last seconds it is lit.
        assert_eq!(b.clip_hold() & 1, 1);
    }

    /// The same over DSPi Link's 2 Hz feed, whose frames are up to 500 ms
    /// old: a frame taken before the clear is its echo, a later one the truth.
    #[test]
    fn wp_p0m_review_f1_a_sustained_clip_stays_lit_over_link() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        let (mut lit, mut reads, mut clears) = (0u32, 0u32, 0u32);
        let mut t = 0;
        while t <= 120_000 {
            let now = at(t0, t);
            let u = b.update_aged(now, Duration::from_millis(400), &[1.0], 1);
            if u.clear_due {
                clears += 1;
                // Through the hub: answered a round trip later.
                b.clear_result(at(t0, t + 80), true);
            }
            reads += 1;
            lit += u32::from(b.clip_hold() & 1 != 0);
            t += 500;
        }
        assert!(clears >= 10);
        assert!(
            reads - lit <= 2 * clears,
            "dark reads {} of {reads}",
            reads - lit
        );
        assert_eq!(b.clip_hold() & 1, 1);
    }

    /// Review p0m D1: a viewer on a hub may not clear. The hold then follows
    /// the device's sticky bits instead of going dark after ten seconds.
    #[test]
    fn wp_p0m_review_d1_a_refused_clear_keeps_the_sticky_clips() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.update(t0, &[], 0b11);
        let u = b.update(at(t0, 10_000), &[], 0b11);
        assert!(u.clear_due);
        b.clear_result(at(t0, 10_050), false);
        assert!(!b.can_clear());
        // Back at once, as the same episode.
        assert_eq!(b.clip_hold(), 0b11);
        for t in (10_500..=120_000).step_by(500) {
            let u = b.update(at(t0, t), &[], 0b11);
            assert!(!u.clear_due, "never asked again (t = {t})");
            assert_eq!(u.new_clips, 0, "no new episode (t = {t})");
            assert_eq!(
                b.clip_hold(),
                0b11,
                "lit while the device reports it (t = {t})"
            );
        }
        // A controller clears the device: dark ten seconds after the last
        // read that showed the bits.
        b.update(at(t0, 120_500), &[], 0);
        assert_eq!(b.clip_hold(), 0b11);
        b.update(at(t0, 130_000), &[], 0);
        assert_eq!(b.clip_hold(), 0);
        // A new clip after that is a new episode, and still not cleared by us.
        assert_eq!(b.update(at(t0, 131_000), &[], 0b100).new_clips, 0b100);
        assert!(!b.update(at(t0, 145_000), &[], 0b100).clear_due);
    }

    /// A clear that is never answered (the lock stayed busy) counts as failed
    /// after the echo guard.
    #[test]
    fn wp_p0m_an_unanswered_clear_counts_as_failed() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.update(t0, &[], 1);
        assert!(b.update(at(t0, 10_000), &[], 1).clear_due);
        b.update(at(t0, 11_000), &[], 1);
        assert_eq!(b.clip_hold(), 0, "within the guard");
        b.update(at(t0, 12_100), &[], 1);
        assert_eq!(b.clip_hold(), 1, "the device still has it");
        assert!(!b.can_clear());
        // A late answer changes nothing.
        b.clear_result(at(t0, 12_200), true);
        assert!(!b.can_clear());
    }

    #[test]
    fn wp_p0m_flags_at_the_first_read_are_from_before() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        let u = b.update(t0, &[], 0b10);
        assert!(u.from_before && u.new_clips == 0b10);
        let u = b.update(at(t0, 60), &[], 0b110);
        assert!(!u.from_before && u.new_clips == 0b100);
        b.reset();
        assert!(b.update(at(t0, 120), &[], 1).from_before);
        // No clips at the first read: nothing to mark.
        b.reset();
        assert!(!b.update(at(t0, 180), &[], 0).from_before);
    }

    /// Review p0m F2: the hold lasts `hold_ms` whatever the poll rate; the
    /// overshoot of the last interval goes into the fall.
    #[test]
    fn wp_p0m_review_f2_the_hold_does_not_depend_on_the_poll_rate() {
        // The continuous reference: 0.8 held 1 s, then 0.5 per second.
        let reference = |t_ms: u64| {
            let fall = (t_ms as f32 / 1000.0 - 1.0).max(0.0) * 0.5;
            (0.8 - fall).max(0.0)
        };
        for step in [60u64, 500, 600] {
            let t0 = Instant::now();
            let mut b = Ballistics::default();
            b.update(t0, &[0.8], 0);
            let mut t = step;
            while t <= 2400 {
                b.update(at(t0, t), &[0.0], 0);
                assert!(
                    close(b.hold(0), reference(t)),
                    "{step} ms polls at {t} ms: {} against {}",
                    b.hold(0),
                    reference(t)
                );
                t += step;
            }
        }
    }

    #[test]
    fn wp_p0m_a_user_clear_and_a_zero_hold() {
        let t0 = Instant::now();
        let mut b = Ballistics::new(BallisticsConfig {
            clip_hold_ms: 0,
            ..BallisticsConfig::DEFAULT
        });
        b.update(t0, &[], 1);
        let u = b.update(at(t0, 3_600_000), &[], 1);
        assert!(!u.clear_due, "0 holds until cleared");
        assert_eq!(b.clip_hold(), 1);
        b.clear_clips(at(t0, 3_600_000));
        assert_eq!(b.clip_hold(), 0);
        assert_eq!(b.update(at(t0, 3_600_050), &[], 1).new_clips, 0, "its echo");
        b.clear_result(at(t0, 3_600_060), true);
        // Cleared: the device says so.
        b.update(at(t0, 3_600_100), &[], 0);
        assert_eq!(b.clip_hold(), 0);
        // A refused user clear brings the bits back.
        b.update(at(t0, 3_600_200), &[], 1);
        b.clear_clips(at(t0, 3_600_300));
        b.clear_result(at(t0, 3_600_310), false);
        assert_eq!(b.clip_hold(), 1);
    }

    #[test]
    fn wp_p0m_notified_clips_join_the_same_hold() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        assert_eq!(b.note_clips(t0, 0b1000).new_clips, 0b1000);
        // The poll then sees the same sticky bit: no second event.
        assert_eq!(b.update(at(t0, 50), &[], 0b1000).new_clips, 0);
    }

    #[test]
    fn wp_p0m_bad_levels_do_not_stick() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.update(t0, &[f32::NAN, 7.0, -1.0], 0);
        assert_eq!(&b.peak_max()[..3], &[0.0, 1.0, 0.0]);
        assert_eq!(b.hold(0), 0.0);
        assert_eq!(b.hold(99), 0.0);
        // More channels than the wire has are ignored.
        b.update(at(t0, 1), &[0.5; 40], 0);
        assert_eq!(b.peak_hold().len(), METER_CHANNELS);
    }

    #[test]
    fn wp_p0m_zero_decay_and_bad_config() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.set_config(BallisticsConfig {
            decay_per_s: f32::NAN,
            ..BallisticsConfig::DEFAULT
        });
        assert_eq!(b.config().decay_per_s, 0.0);
        b.update(t0, &[0.5], 0);
        b.update(at(t0, 100_000), &[0.0], 0);
        assert!(close(b.hold(0), 0.5));
        // A clock that goes backwards counts as no time.
        b.update(t0, &[0.0], 0);
        assert!(close(b.hold(0), 0.5));
    }
}
