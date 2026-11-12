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
//!   the device 10 s after the stamp).
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

/// What one [`Ballistics::update`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeterUpdate {
    /// Channels whose clip hold lit on this update (bit per wire channel):
    /// one event per clip episode, which is what a log records.
    pub new_clips: u32,
    /// The clip hold just ended: clear the device's sticky flags
    /// (`REQ_CLEAR_CLIPS`, `meters.clear`) so they do not light it again.
    pub clear_due: bool,
}

/// Peak hold, clip hold and peak maximum for one device's meters.
#[derive(Debug, Clone)]
pub struct Ballistics {
    config: BallisticsConfig,
    channels: Vec<Held>,
    /// The clip hold, bit per wire channel.
    clip_hold: u32,
    /// When a bit last joined the clip hold.
    last_new_clip: Option<Instant>,
    /// Bits a cleared hold dropped while the device may still report them:
    /// ignored until a poll shows the bit clear (the clear reached the device)
    /// so a late or refused clear cannot light the hold again by itself.
    awaiting_clear: u32,
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
            awaiting_clear: 0,
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
    /// device too; until a poll shows the bits clear they are not held again.
    pub fn clear_clips(&mut self) {
        self.awaiting_clear |= self.clip_hold;
        self.clip_hold = 0;
        self.last_new_clip = None;
    }

    /// Feed one meter read: `peaks` linear 0..1 per wire channel (more than
    /// [`METER_CHANNELS`] are ignored, fewer leave the rest at 0) and the
    /// device's sticky `clip_flags`, read at `now`.
    pub fn update(&mut self, now: Instant, peaks: &[f32], clip_flags: u32) -> MeterUpdate {
        let dt = self
            .last
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.last = Some(now);
        let hold_s = ms(self.config.hold_ms).as_secs_f32();
        for (i, c) in self.channels.iter_mut().enumerate() {
            let level = sane(peaks.get(i).copied().unwrap_or(0.0));
            c.max = c.max.max(level);
            if level >= c.peak {
                c.peak = level;
                c.hold_s = hold_s;
            } else if c.hold_s > 0.0 {
                c.hold_s = (c.hold_s - dt).max(0.0);
            } else {
                c.peak = (c.peak - self.config.decay_per_s * dt).max(level);
            }
        }
        // A bit the device no longer reports has been cleared there.
        self.awaiting_clear &= clip_flags;
        self.note_clips(now, clip_flags & !self.awaiting_clear)
    }

    /// Add clips that arrived another way than the meter block (a firmware
    /// clip notification, WP FWX) to the hold. Returns what [`update`]
    /// would: the bits new to the hold and whether the hold just ended.
    ///
    /// [`update`]: Ballistics::update
    pub fn note_clips(&mut self, now: Instant, clips: u32) -> MeterUpdate {
        let new_clips = clips & !self.clip_hold;
        if new_clips != 0 {
            self.clip_hold |= new_clips;
            self.last_new_clip = Some(now);
        }
        let ended = self.config.clip_hold_ms > 0
            && self.clip_hold != 0
            && self
                .last_new_clip
                .is_some_and(|t| now.saturating_duration_since(t) >= ms(self.config.clip_hold_ms));
        if ended {
            self.clear_clips();
        }
        MeterUpdate {
            new_clips,
            clear_due: ended,
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
        // The device still reports the bits until the clear lands: not a
        // new clip.
        let u = b.update(at(t0, 16_100), &[], 0b0110);
        assert_eq!(u.new_clips, 0);
        assert_eq!(b.clip_hold(), 0);
        // Cleared there; a clip after that is a new episode.
        b.update(at(t0, 16_200), &[], 0);
        let u = b.update(at(t0, 16_300), &[], 0b0010);
        assert_eq!(u.new_clips, 0b0010);
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
        b.clear_clips();
        assert_eq!(b.clip_hold(), 0);
        assert_eq!(b.update(at(t0, 3_600_100), &[], 1).new_clips, 0);
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
