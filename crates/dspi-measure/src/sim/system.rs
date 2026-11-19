//! The multi-unit simulated world (WP ATSIM, PLAN-AT section 2): several DSPi
//! units on one time base, each playing its own generator from a domain
//! sample, heard by one microphone.
//!
//! A [`SystemWorld`] is P5SIM's world split into units: every [`SimUnit`]
//! carries its outputs (P5SIM's [`OutputPath`]: driver and path) and their
//! chains, its play-out offset (the follower's `D` plus the post-alink
//! latency), a lock jitter (white plus a slow wander), a start error in whole
//! samples, its link (line with a fan-out hop error, USB-direct with its own S
//! origin, S/PDIF source or follower), its domain ([`DomainClock`]: two
//! domains are possible, each with its own clock and numbering) and its
//! faults ([`UnitFault`]). The room, the microphone, its noise and the
//! capture clock are P5SIM's ([`MicClock`], [`CaptureFault`]); on top the
//! microphone's clock may wander in bursts as P5SPIKE's did
//! ([`WanderBursts::p5spike`]: 55 to 100 ppm for 0.25 to 1.5 s, the
//! excursion within ±50 us).
//!
//! [`render_system`] plays a [`SystemSchedule`] in it and returns the capture
//! with its ground truth ([`SystemTruth`]): each output's true arrival, every
//! cycle's sample 0 in device and capture frames, each segment's fate (the
//! `EXT_SIGGEN_TIMELINE` proof a unit would give), each fault's time and what
//! it counts, and the wander bursts. It never calls the analyser (ATSYS owns
//! that; ATE2E joins the two).
//!
//! **Timelines.** The *reference* device clock is the world's device clock
//! (the one [`MicClock::ppm`] is relative to): device frame `tau` at the
//! schedule's rate. Domain `d` maps its sample `T` to
//! `tau = (T - origin_t) / (1 + ppm_d 1e-6)` ([`DomainClock`]). Unit `u`
//! plays the generator sample carried as `T` at
//! `tau_d(T) + playout + hop_error + shift_u(T) + jitter_u`, where `shift_u`
//! sums the whole-sample corrections and re-anchor shifts that acted at or
//! before `T`. Every unit reads the schedule's `T` in **its own** domain's
//! numbering, so a unit of another domain lands elsewhere (what G1 refuses).
//! Fault times are domain samples in the numbering the run started with.
//!
//! **How it renders.** Each cycle's samples are warped onto the reference
//! timeline (the unit's mapping, its jitter, the microphone's wander bursts
//! evaluated where that output's sound reaches the microphone) and handed to
//! P5SIM's [`render`] as one burst; render then applies the chain, the driver,
//! the path, the room, the microphone and the capture clock. The bursts' sum
//! is bounded by P5SIM's [`MAX_DEVICE_FRAMES`], so memory stays within
//! P5SIM's constants.

use super::dsp::{Kernel, normal, uniform};
use super::{
    Acoustics, Burst, Capture, CaptureFault, MAX_DEVICE_FRAMES, MAX_SIM_OUTPUTS, MAX_SLIP_FRAMES,
    MicClock, Microphone, OutputPath, Rumble, Stimulus, World, render,
};
use crate::stimulus::{LogSweep, WalkSchedule, peak_linear};
use crate::system::{DomainId, MAX_UNITS, SystemSchedule, UnitIndex};
use crate::types::{ChannelChain, MeasureError, SectionModel};

/// The most unit faults one world has.
pub const MAX_UNIT_FAULTS: usize = 16;
/// The most wander bursts one render generates (later bursts are not
/// generated: the wander stays at its last value).
pub const MAX_WANDER_BURSTS: usize = 1024;
/// The largest play-out offset a unit has, frames.
pub const MAX_PLAYOUT_FRAMES: f64 = 100_000.0;
/// The largest fan-out hop error, frames.
pub const MAX_HOP_ERROR_FRAMES: i32 = 1024;
/// The largest lock jitter term (white RMS or wander amplitude), ns.
pub const MAX_JITTER_NS: f64 = 1_000_000.0;
/// The largest microphone wander bound, us.
pub const MAX_WANDER_BOUND_US: f64 = 10_000.0;

fn invalid(what: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(what.into())
}

fn in_range(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && v >= lo && v <= hi
}

/// One clock domain of the world: a time-base numbering and its clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DomainClock {
    /// Its identity (what `EXT_TIME_BASE` reports: time base and domain).
    pub domain: DomainId,
    /// Its sample clock against the reference device clock, ppm (±1000;
    /// above zero runs fast).
    pub ppm: f64,
    /// The domain sample `T` at reference device frame 0.
    pub origin_t: u64,
}

/// How a unit is on its time base.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnitLink {
    /// A line follower (or the sender's own outputs with `hops` 0).
    Line {
        /// Hops from the sender (reported only).
        hops: u8,
        /// A fan-out's hop error: the unit plays this many whole frames
        /// later than its configured `hops_offset` assumes (±
        /// [`MAX_HOP_ERROR_FRAMES`]).
        hop_error_frames: i32,
    },
    /// USB-direct: the unit's own S is the domain's `T` plus `origin`
    /// (multiples of `2048 * spf` on a real bus; any value here).
    UsbDirect {
        /// The unit's S at domain sample 0.
        origin: i64,
    },
    /// An S/PDIF source: it emits the domain's F (its play-out `D` is 0 by
    /// definition; `playout_frames` is its post-alink latency only).
    SpdifSource,
    /// An S/PDIF follower of a source unit of the same domain. The source's
    /// epoch changes re-anchor it (reason `EPOCH`).
    SpdifFollower {
        /// The source unit.
        source: UnitIndex,
    },
}

/// A unit's lock jitter: how far its play-out instant moves about the
/// locked position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LockJitter {
    /// White jitter per output frame, ns RMS (0 to [`MAX_JITTER_NS`]).
    pub white_rms_ns: f64,
    /// A slow sinusoidal wander's amplitude, ns (0 to [`MAX_JITTER_NS`]).
    pub wander_ns: f64,
    /// Its period, s (0.1 to 100 000).
    pub wander_period_s: f64,
}

/// A fault on one unit's time base, at a domain sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnitFault {
    /// The unit refuses every START (`REFUSED_NOT_READY`): it stays silent.
    NeverStarts,
    /// The time base holds over from `at_t` for `frames`: outputs silent, a
    /// generator op 9 started stops (`STOPPED_LOSS`), a START due in it is
    /// cancelled (`CANCELLED_LOSS`); the timeline is kept.
    Holdover {
        /// The domain sample it starts at.
        at_t: u64,
        /// Its length, frames (at most [`MAX_DEVICE_FRAMES`]).
        frames: u64,
    },
    /// A re-anchor at `at_t` taking `frames` (as a holdover for the
    /// generator), after which the play-out sits `shift_frames` later.
    Reanchor {
        /// The domain sample it starts at.
        at_t: u64,
        /// Its length, frames (at most [`MAX_DEVICE_FRAMES`]).
        frames: u64,
        /// The play-out's whole-frame shift after it (± [`MAX_SLIP_FRAMES`]).
        shift_frames: i32,
    },
    /// A whole-sample correction while locked: from `at_t` on the play-out
    /// sits `frames` later; a running generator keeps playing (with the
    /// step). `event` false: no 0xE4 is sent (what G2's counter read
    /// catches).
    Correction {
        /// The domain sample it acts on.
        at_t: u64,
        /// Frames (± [`MAX_SLIP_FRAMES`]).
        frames: i32,
        /// Whether an event is sent.
        event: bool,
    },
    /// The unit's counter starts a new epoch at `at_t` (an S/PDIF source's
    /// output restart): its numbering jumps by `t_jump` (a START written in
    /// the old numbering lands `-t_jump` samples away), it re-anchors for
    /// `frames` (a running generator stops), and every S/PDIF follower of it
    /// does the same.
    EpochChange {
        /// The domain sample it happens at.
        at_t: u64,
        /// The numbering's jump, samples.
        t_jump: i64,
        /// The re-anchor's length, frames (at most [`MAX_DEVICE_FRAMES`]).
        frames: u64,
    },
}

impl UnitFault {
    fn at(&self) -> Option<u64> {
        match *self {
            UnitFault::NeverStarts => None,
            UnitFault::Holdover { at_t, .. }
            | UnitFault::Reanchor { at_t, .. }
            | UnitFault::Correction { at_t, .. }
            | UnitFault::EpochChange { at_t, .. } => Some(at_t),
        }
    }

    /// The window in which the time base is not `LOCKED`, `[at, at + frames]`.
    fn loss_window(&self) -> Option<(u64, u64)> {
        match *self {
            UnitFault::Holdover { at_t, frames }
            | UnitFault::Reanchor { at_t, frames, .. }
            | UnitFault::EpochChange { at_t, frames, .. } => {
                Some((at_t, at_t.saturating_add(frames)))
            }
            _ => None,
        }
    }

    /// The play-out's whole-frame step from `at_t` on.
    fn shift(&self) -> i64 {
        match *self {
            UnitFault::Reanchor { shift_frames, .. } => i64::from(shift_frames),
            UnitFault::Correction { frames, .. } => i64::from(frames),
            _ => 0,
        }
    }
}

/// One unit of the world.
#[derive(Debug, Clone, PartialEq)]
pub struct SimUnit {
    /// A name for reports.
    pub name: String,
    /// Its domain: an index into [`SystemWorld::domains`].
    pub domain: usize,
    /// How it is on its time base.
    pub link: UnitLink,
    /// Its outputs, output 0 first (the generator's channel mask bits).
    pub outputs: Vec<OutputPath>,
    /// Their chains, one per output.
    pub chains: Vec<ChannelChain>,
    /// The play-out offset: `D` plus the post-alink latency, frames (0 to
    /// [`MAX_PLAYOUT_FRAMES`], fractional allowed).
    pub playout_frames: f64,
    /// The generator's sample 0 lands this many samples after the commanded
    /// `T` (a firmware fault; 0 for an exact START; ± [`MAX_SLIP_FRAMES`]).
    pub start_error_samples: i32,
    /// Its lock jitter; `None` for none.
    pub jitter: Option<LockJitter>,
    /// The generator's level, dBFS peak (-120 to 0).
    pub level_db: f32,
    /// Outputs played inverted (the generator's `invert_mask`).
    pub invert_mask: u16,
    /// Its faults.
    pub faults: Vec<UnitFault>,
}

impl SimUnit {
    /// A unit with `outputs` flat outputs and flat chains on domain 0, a line
    /// follower with no hop error, no play-out offset, no jitter, at -20 dBFS.
    pub fn flat(name: &str, outputs: usize) -> SimUnit {
        let n = outputs.clamp(1, MAX_SIM_OUTPUTS);
        let w = World::flat(n, 48_000);
        SimUnit {
            name: name.to_string(),
            domain: 0,
            link: UnitLink::Line {
                hops: 1,
                hop_error_frames: 0,
            },
            outputs: w.outputs,
            chains: super::flat_chains(n),
            playout_frames: 0.0,
            start_error_samples: 0,
            jitter: None,
            level_db: -20.0,
            invert_mask: 0,
            faults: Vec::new(),
        }
    }
}

/// The microphone clock's wander in bursts (P5SPIKE): between quiet gaps the
/// rate departs by `min_ppm..max_ppm` (either sign) for `min_s..max_s`, with
/// smooth edges, and the time error stays within `±bound_us`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WanderBursts {
    /// The bursts' seed.
    pub seed: u64,
    /// The smallest burst rate, ppm (above 0, at most `max_ppm`).
    pub min_ppm: f64,
    /// The largest burst rate, ppm (at most 1000).
    pub max_ppm: f64,
    /// The shortest burst, s (0.01 to `max_s`).
    pub min_s: f64,
    /// The longest burst, s (at most 100).
    pub max_s: f64,
    /// The time error's bound, us (above 0, at most [`MAX_WANDER_BOUND_US`]);
    /// a burst that would leave it is turned or shortened.
    pub bound_us: f64,
    /// The shortest quiet gap before a burst, s (0 to `max_gap_s`).
    pub min_gap_s: f64,
    /// The longest, s (at most 1000).
    pub max_gap_s: f64,
}

impl WanderBursts {
    /// P5SPIKE's microphone: 55 to 100 ppm for 0.25 to 1.5 s, within ±50 us,
    /// quiet for 0.2 to 2 s between bursts.
    pub fn p5spike(seed: u64) -> WanderBursts {
        WanderBursts {
            seed,
            min_ppm: 55.0,
            max_ppm: 100.0,
            min_s: 0.25,
            max_s: 1.5,
            bound_us: 50.0,
            min_gap_s: 0.2,
            max_gap_s: 2.0,
        }
    }

    fn valid(&self) -> bool {
        in_range(self.min_ppm, 1e-6, 1000.0)
            && in_range(self.max_ppm, self.min_ppm, 1000.0)
            && in_range(self.min_s, 0.01, 100.0)
            && in_range(self.max_s, self.min_s, 100.0)
            && in_range(self.bound_us, 1e-6, MAX_WANDER_BOUND_US)
            && in_range(self.min_gap_s, 0.0, 1000.0)
            && in_range(self.max_gap_s, self.min_gap_s, 1000.0)
    }

    /// The bursts over the first `horizon_s` seconds (at most
    /// [`MAX_WANDER_BURSTS`]); empty for invalid parameters.
    pub fn bursts(&self, horizon_s: f64) -> Vec<WanderBurst> {
        let mut out = Vec::new();
        if !self.valid() || !horizon_s.is_finite() {
            return out;
        }
        let lerp = |lo: f64, hi: f64, u: f64| lo + (hi - lo) * u;
        let mut t = 0.0;
        let mut err = 0.0;
        let mut i: u64 = 0;
        while t < horizon_s && out.len() < MAX_WANDER_BURSTS {
            let base = i.saturating_mul(8);
            let u = |k: u64| uniform(self.seed ^ 0x5741_4E44, base.saturating_add(k));
            t += lerp(self.min_gap_s, self.max_gap_s, u(0));
            let rate = lerp(self.min_ppm, self.max_ppm, u(1));
            let mut len = lerp(self.min_s, self.max_s, u(2));
            let mut sign = if u(3) < 0.5 { -1.0 } else { 1.0 };
            // Room left in a direction, us.
            let room = |s: f64| (self.bound_us - s * err).max(0.0);
            if room(sign) < rate * burst_shift_s(self.min_s) {
                sign = -sign;
            }
            // Shorten the burst until its displacement fits the room.
            let fit = room(sign);
            while len > self.min_s && rate * burst_shift_s(len) > fit {
                len = (len * 0.95).max(self.min_s);
            }
            let ppm = sign * rate;
            let shift_us = ppm * burst_shift_s(len);
            if (err + shift_us).abs() > self.bound_us {
                // Even the shortest burst does not fit (a tiny bound): stay.
                t += len;
                i = i.saturating_add(1);
                continue;
            }
            out.push(WanderBurst {
                start_s: t,
                duration_s: len,
                ppm,
                error_before_us: err,
            });
            err += shift_us;
            t += len;
            i = i.saturating_add(1);
        }
        out
    }
}

/// The edge of a burst's rate profile, s.
fn burst_edge_s(len: f64) -> f64 {
    (len / 4.0).min(0.05)
}

/// The time a burst of unit rate moves the clock by, s per (rate 1e-6), i.e.
/// the integral of its rate profile: a plateau with raised-cosine edges.
fn burst_shift_s(len: f64) -> f64 {
    len - burst_edge_s(len)
}

/// The integral of the rate profile from the burst's start to `x` s.
fn burst_profile_integral(len: f64, x: f64) -> f64 {
    let e = burst_edge_s(len);
    let x = x.clamp(0.0, len);
    let pi = std::f64::consts::PI;
    if e <= 0.0 {
        return x;
    }
    if x < e {
        x / 2.0 - e / (2.0 * pi) * (pi * x / e).sin()
    } else if x <= len - e {
        e / 2.0 + (x - e)
    } else {
        let y = len - x;
        burst_shift_s(len) - (y / 2.0 - e / (2.0 * pi) * (pi * y / e).sin())
    }
}

/// One wander burst of the microphone's clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WanderBurst {
    /// Its start, s of reference device time at the microphone.
    pub start_s: f64,
    /// Its length, s.
    pub duration_s: f64,
    /// Its plateau rate, ppm (signed: above zero the microphone runs ahead).
    pub ppm: f64,
    /// The microphone's time error when it starts, us.
    pub error_before_us: f64,
}

impl WanderBurst {
    /// The time error this burst adds by `t` s (device time), us.
    fn error_by(&self, t: f64) -> f64 {
        self.ppm * burst_profile_integral(self.duration_s, t - self.start_s)
    }
}

/// The microphone's time error at reference device time `t_s` from `bursts`
/// (as [`WanderBursts::bursts`] made them), us: above zero the microphone
/// hears device time `t + error` at what its clock calls `t`.
pub fn wander_error_us(bursts: &[WanderBurst], t_s: f64) -> f64 {
    if !t_s.is_finite() {
        return 0.0;
    }
    let i = bursts.partition_point(|b| b.start_s <= t_s);
    match i.checked_sub(1).and_then(|j| bursts.get(j)) {
        Some(b) => b.error_before_us + b.error_by(t_s),
        None => 0.0,
    }
}

/// The multi-unit world: deterministic for a given value, seeds included.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemWorld {
    /// A name for reports.
    pub name: String,
    /// The units, unit 0 first ([`UnitIndex`] indexes it; 1 to
    /// [`MAX_UNITS`]; all their outputs together at most
    /// [`MAX_SIM_OUTPUTS`]).
    pub units: Vec<SimUnit>,
    /// The clock domains (1 to [`MAX_UNITS`]).
    pub domains: Vec<DomainClock>,
    /// The room, the free field, or the coupler (P5SIM's).
    pub acoustics: Acoustics,
    /// The microphone.
    pub mic: Microphone,
    /// The noise floor at the capture, dBFS RMS; `None` for none.
    pub noise_dbfs: Option<f64>,
    /// Low-frequency rumble; `None` for none.
    pub rumble: Option<Rumble>,
    /// The capture's clock and blocks (P5SIM's; its ppm is against the
    /// reference device clock).
    pub clock: MicClock,
    /// The capture faults (P5SIM's; a device slip moves every unit).
    pub capture_faults: Vec<CaptureFault>,
    /// The microphone clock's wander bursts; `None` for none.
    pub wander: Option<WanderBursts>,
    /// Silence rendered after the last sample, ms (0 to 10 000).
    pub post_roll_ms: u32,
    /// The noise's and the jitter's seed.
    pub seed: u64,
}

impl SystemWorld {
    /// `units` (cloned) on one domain `domain` whose sample `origin_t` is
    /// reference frame 0, in a free field with a flat microphone, no noise,
    /// an exact capture clock at `rate_hz` with blocks of 128, no faults, no
    /// wander, 500 ms of post-roll.
    pub fn free_field(units: Vec<SimUnit>, domain: DomainId, origin_t: u64, rate_hz: u32) -> Self {
        SystemWorld {
            name: "system free field".to_string(),
            units,
            domains: vec![DomainClock {
                domain,
                ppm: 0.0,
                origin_t,
            }],
            acoustics: Acoustics::Free,
            mic: Microphone::flat(),
            noise_dbfs: None,
            rumble: None,
            clock: MicClock {
                rate_hz,
                ppm: 0.0,
                latency_ms: 0.0,
                block_frames: 128,
                first_frame: 0,
            },
            capture_faults: Vec::new(),
            wander: None,
            post_roll_ms: 500,
            seed: 0,
        }
    }

    /// The first output of every unit in the flattened world, and the total.
    fn output_bases(&self) -> (Vec<usize>, usize) {
        let mut bases = Vec::with_capacity(self.units.len());
        let mut n = 0usize;
        for u in &self.units {
            bases.push(n);
            n = n.saturating_add(u.outputs.len());
        }
        (bases, n)
    }

    /// P5SIM's world of every unit's outputs in order (unit 0's first).
    pub fn flat_world(&self) -> World {
        World {
            name: self.name.clone(),
            outputs: self
                .units
                .iter()
                .flat_map(|u| u.outputs.iter().cloned())
                .collect(),
            acoustics: self.acoustics.clone(),
            mic: self.mic.clone(),
            noise_dbfs: self.noise_dbfs,
            clock: self.clock.clone(),
            faults: self.capture_faults.clone(),
            seed: self.seed,
            rumble: self.rumble,
        }
    }

    /// The world's output index of `unit`'s `output`; `None` for one the
    /// world lacks.
    pub fn global_output(&self, unit: UnitIndex, output: u8) -> Option<usize> {
        let (bases, _) = self.output_bases();
        let u = self.units.get(usize::from(unit.0))?;
        if usize::from(output) >= u.outputs.len() {
            return None;
        }
        bases
            .get(usize::from(unit.0))
            .map(|b| b.saturating_add(usize::from(output)))
    }

    fn validate(&self) -> Result<(), MeasureError> {
        if self.units.is_empty() || self.units.len() > MAX_UNITS {
            return Err(invalid(format!("a system has 1 to {MAX_UNITS} units")));
        }
        if self.domains.is_empty() || self.domains.len() > MAX_UNITS {
            return Err(invalid(format!("a system has 1 to {MAX_UNITS} domains")));
        }
        if !self
            .domains
            .iter()
            .all(|d| in_range(d.ppm, -1000.0, 1000.0))
        {
            return Err(invalid("a domain clock is out of range"));
        }
        let (_, total) = self.output_bases();
        if total == 0 || total > MAX_SIM_OUTPUTS {
            return Err(invalid(format!(
                "{total} outputs in all (1 to {MAX_SIM_OUTPUTS})"
            )));
        }
        let mut faults = 0usize;
        for (i, u) in self.units.iter().enumerate() {
            let name = &u.name;
            if u.outputs.is_empty() || u.chains.len() != u.outputs.len() {
                return Err(invalid(format!("unit {name:?}: one chain per output")));
            }
            if u.domain >= self.domains.len() {
                return Err(invalid(format!("unit {name:?}: no domain {}", u.domain)));
            }
            let ok = in_range(u.playout_frames, 0.0, MAX_PLAYOUT_FRAMES)
                && (-MAX_SLIP_FRAMES..=MAX_SLIP_FRAMES).contains(&u.start_error_samples)
                && in_range(f64::from(u.level_db), -120.0, 0.0)
                && u.jitter.is_none_or(|j| {
                    in_range(j.white_rms_ns, 0.0, MAX_JITTER_NS)
                        && in_range(j.wander_ns, 0.0, MAX_JITTER_NS)
                        && in_range(j.wander_period_s, 0.1, 100_000.0)
                });
            if !ok {
                return Err(invalid(format!("unit {name:?} is out of range")));
            }
            match u.link {
                UnitLink::Line {
                    hop_error_frames, ..
                } => {
                    if !(-MAX_HOP_ERROR_FRAMES..=MAX_HOP_ERROR_FRAMES).contains(&hop_error_frames) {
                        return Err(invalid(format!("unit {name:?}: hop error out of range")));
                    }
                }
                UnitLink::SpdifFollower { source } => {
                    let s = usize::from(source.0);
                    let ok = s != i
                        && self.units.get(s).is_some_and(|src| {
                            src.link == UnitLink::SpdifSource && src.domain == u.domain
                        });
                    if !ok {
                        return Err(invalid(format!(
                            "unit {name:?}: its S/PDIF source is not a source of its domain"
                        )));
                    }
                }
                UnitLink::UsbDirect { .. } | UnitLink::SpdifSource => {}
            }
            faults = faults.saturating_add(u.faults.len());
            for f in &u.faults {
                let ok = match *f {
                    UnitFault::NeverStarts => true,
                    UnitFault::Holdover { frames, .. } => frames <= MAX_DEVICE_FRAMES,
                    UnitFault::Reanchor {
                        frames,
                        shift_frames,
                        ..
                    } => {
                        frames <= MAX_DEVICE_FRAMES
                            && (-MAX_SLIP_FRAMES..=MAX_SLIP_FRAMES).contains(&shift_frames)
                    }
                    UnitFault::Correction { frames, .. } => {
                        (-MAX_SLIP_FRAMES..=MAX_SLIP_FRAMES).contains(&frames)
                    }
                    UnitFault::EpochChange { frames, t_jump, .. } => {
                        frames <= MAX_DEVICE_FRAMES
                            && t_jump.unsigned_abs() <= MAX_DEVICE_FRAMES.saturating_mul(1024)
                    }
                };
                if !ok {
                    return Err(invalid(format!("unit {name:?}: fault {f:?} out of range")));
                }
            }
        }
        if faults > MAX_UNIT_FAULTS {
            return Err(invalid(format!(
                "{faults} unit faults (at most {MAX_UNIT_FAULTS})"
            )));
        }
        if self.wander.is_some_and(|w| !w.valid()) {
            return Err(invalid("the wander bursts are out of range"));
        }
        if self.post_roll_ms > 10_000 {
            return Err(invalid("the post-roll is out of range"));
        }
        super::render::validate_world(&self.flat_world())
    }

    /// The faults acting on `unit`: its own, plus (for an S/PDIF follower)
    /// its source's epoch changes, with whether each is inherited.
    fn unit_faults(&self, unit: usize) -> Vec<(UnitFault, bool)> {
        let Some(u) = self.units.get(unit) else {
            return Vec::new();
        };
        let mut out: Vec<(UnitFault, bool)> = u.faults.iter().map(|f| (*f, false)).collect();
        if let UnitLink::SpdifFollower { source } = u.link
            && let Some(src) = self.units.get(usize::from(source.0))
        {
            out.extend(
                src.faults
                    .iter()
                    .filter(|f| matches!(f, UnitFault::EpochChange { .. }))
                    .map(|f| (*f, true)),
            );
        }
        out
    }
}

/// What became of one segment (what the unit's `EXT_SIGGEN_TIMELINE` shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentFate {
    /// It started and played to its end.
    Played,
    /// The unit refused the START (`REFUSED_NOT_READY`): silent.
    RefusedNotReady,
    /// Its `T` had passed when it came due (an epoch jump moved it into the
    /// past or into the re-anchor; `REFUSED_LATE`): silent.
    RefusedLate,
    /// A loss of the time base cancelled the pending START
    /// (`CANCELLED_LOSS`): silent.
    CancelledLoss,
    /// It started and a loss of the time base stopped it (`STOPPED_LOSS`).
    StoppedLoss,
}

/// The truth of one cycle of a segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycleTruth {
    /// Its index in the segment's walk.
    pub index: u32,
    /// The unit's output.
    pub output: u8,
    /// The world's output ([`SystemWorld::global_output`]).
    pub global_output: usize,
    /// Its pass in the segment's walk.
    pub pass: u16,
    /// The domain sample (the run's numbering) its sample 0 played on.
    pub t: u64,
    /// Whether any of it played.
    pub played: bool,
    /// Whether a stop cut it short.
    pub truncated: bool,
    /// The reference device frame its sample 0 left the unit at (jitter and
    /// corrections included).
    pub device_frame: f64,
    /// The capture frame (the index a client reports) at which its sample 0
    /// reaches the microphone through the chain's delay and the output's
    /// acoustic delay, on the microphone's clock and wander (P5SIM's step
    /// slips not included, as [`World::capture_position`]).
    pub capture_frame: f64,
}

/// The truth of one segment.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentTruth {
    /// Its index in the schedule.
    pub segment: usize,
    /// Its unit.
    pub unit: UnitIndex,
    /// The scheduled `T` (the schedule's numbering).
    pub scheduled_t: u64,
    /// The scheduled `T` in the unit's own numbering when it was written
    /// (USB: plus the unit's S origin).
    pub scheduled_t_unit: u64,
    /// What became of it.
    pub fate: SegmentFate,
    /// The domain sample (the run's numbering) its sample 0 played on, the
    /// start error included; `None` when it did not start.
    pub started_t: Option<u64>,
    /// The `start` the unit's timeline reports, in its own numbering at the
    /// time; `None` when it did not start.
    pub reported_start: Option<u64>,
    /// Generator samples rendered (a stop's fade included).
    pub samples: u64,
    /// Discontinuities while it ran (corrections, the stopping loss).
    pub discontinuities: u16,
    /// Its cycles, in order.
    pub cycles: Vec<CycleTruth>,
}

/// The truth of one output of a unit: its arrival after the domain's play
/// time of `T`, at the start of the run (no fault, no jitter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputTruth {
    /// The unit.
    pub unit: UnitIndex,
    /// Its output.
    pub output: u8,
    /// The world's output.
    pub global_output: usize,
    /// The unit's play-out offset, frames.
    pub playout_frames: f64,
    /// The fan-out hop error, frames.
    pub hop_error_frames: i32,
    /// The generator's start error, samples.
    pub start_error_samples: i32,
    /// The chain's output delay as the device runs it (whole samples).
    pub chain_delay_frames: u64,
    /// The driver's offset plus the path's time of flight, frames.
    pub acoustic_frames: f64,
    /// All of them, frames.
    pub arrival_frames: f64,
    /// All of them, us.
    pub arrival_us: f64,
}

/// The truth of one fault on one unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaultTruth {
    /// The unit it acts on.
    pub unit: UnitIndex,
    /// The fault.
    pub fault: UnitFault,
    /// Inherited from the unit's S/PDIF source (an epoch change).
    pub inherited: bool,
    /// The domain sample it acts at; `None` for [`UnitFault::NeverStarts`].
    pub t: Option<u64>,
    /// The reference device frame of that sample (the domain's mapping).
    pub device_frame: Option<f64>,
    /// The capture frame at which that device frame is heard without any
    /// output's delay ([`SystemRender::capture_frame_at_mic`]).
    pub capture_frame: Option<f64>,
    /// Whether it falls inside the run (from the schedule's start to its
    /// end).
    pub in_run: bool,
    /// What it adds to `EXT_TIME_BASE.disturbances`.
    pub disturbances: u16,
    /// Whether a 0xE4 event reports it.
    pub event: bool,
    /// Whether it moves `t - output_frames`.
    pub moves_t_minus_output: bool,
}

/// The truth of one unit over the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitTruth {
    /// The unit.
    pub unit: UnitIndex,
    /// Its domain.
    pub domain: DomainId,
    /// Whether that is the schedule's domain.
    pub in_schedule_domain: bool,
    /// `disturbances` counted inside the run.
    pub disturbances: u16,
    /// 0xE4 events inside the run.
    pub events: u16,
    /// Whether `t - output_frames` moved inside the run.
    pub t_minus_output_moved: bool,
}

/// The ground truth of a rendered run.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemTruth {
    /// The device rate (the schedule's), Hz.
    pub device_rate_hz: u32,
    /// The rendered timeline, reference device frames.
    pub timeline_frames: u64,
    /// Every unit, in order.
    pub units: Vec<UnitTruth>,
    /// Every output of every unit that a segment walks, unit then output.
    pub outputs: Vec<OutputTruth>,
    /// Every segment, in the schedule's order.
    pub segments: Vec<SegmentTruth>,
    /// Every fault, unit by unit.
    pub faults: Vec<FaultTruth>,
    /// The microphone's wander bursts.
    pub wander: Vec<WanderBurst>,
}

impl SystemTruth {
    /// The truth of `unit`'s `output`.
    pub fn output(&self, unit: UnitIndex, output: u8) -> Option<&OutputTruth> {
        self.outputs
            .iter()
            .find(|o| o.unit == unit && o.output == output)
    }
}

/// A rendered run: the capture and its truth.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemRender {
    /// What the microphone captured (P5SIM's blocks).
    pub capture: Capture,
    /// The ground truth.
    pub truth: SystemTruth,
    world: World,
}

impl SystemRender {
    /// The capture frame (the index a client reports) at which the sound at
    /// the microphone at reference device frame `p` is heard: the
    /// microphone's clock, latency, ratio wander and wander bursts (P5SIM's
    /// step slips not included). `None` for a position out of range.
    pub fn capture_frame_at_mic(&self, p: f64) -> Option<f64> {
        capture_at_mic(
            &self.world,
            &self.truth.wander,
            self.truth.device_rate_hz,
            p,
        )
    }
}

fn capture_at_mic(world: &World, bursts: &[WanderBurst], rate: u32, p: f64) -> Option<f64> {
    let fd = f64::from(rate);
    // pos(k) + W(pos(k)) = p: W moves slowly (|W'| <= 1e-3), so this
    // contracts at once.
    let mut q = p;
    for _ in 0..6 {
        q = p - wander_error_us(bursts, q / fd) * 1e-6 * fd;
    }
    world.capture_position(q, rate).ok()
}

/// One unit's mapping from domain samples to reference device frames.
struct UnitMap {
    eps: f64,
    origin_t: u64,
    fixed: f64,
    /// Whole-frame steps `(at_t, frames)`, by `at_t`.
    steps: Vec<(u64, i64)>,
    jitter: Option<LockJitter>,
    seed: u64,
    phase: f64,
    fd: f64,
}

impl UnitMap {
    /// The domain's nominal reference frame of `t`.
    fn base(&self, t: f64) -> f64 {
        (t - self.origin_t as f64) / (1.0 + self.eps)
    }

    fn shift(&self, t: f64) -> f64 {
        self.steps
            .iter()
            .filter(|(at, _)| (*at as f64) <= t)
            .map(|(_, n)| *n as f64)
            .sum()
    }

    /// The jitter at reference frame `q`, frames.
    fn jitter(&self, q: f64) -> f64 {
        let Some(j) = self.jitter else {
            return 0.0;
        };
        let white = if j.white_rms_ns > 0.0 {
            j.white_rms_ns * normal(self.seed, q.round().max(0.0) as u64)
        } else {
            0.0
        };
        let w = std::f64::consts::TAU * q / (j.wander_period_s * self.fd);
        (white + j.wander_ns * (w + self.phase).sin()) * 1e-9 * self.fd
    }

    /// Where domain sample `t` leaves the unit, reference frames.
    fn play(&self, t: f64) -> f64 {
        let q = self.base(t) + self.fixed + self.shift(t);
        q + self.jitter(q)
    }
}

/// One cycle to render.
struct CyclePlan {
    sweep: LogSweep,
    output: usize,
    /// Domain sample of sample 0.
    t0: f64,
    /// Samples it plays (a stop's fade included).
    samples: u64,
    /// The sample a stop's fade starts at, if any.
    fade_at: Option<u64>,
    first: bool,
    gain: f32,
    unit: usize,
    /// The output's delay after the unit (chain + acoustic), frames.
    delay: f64,
    /// Reference frames it covers.
    lo: i64,
    hi: i64,
}

/// Render the capture of `schedule` in `world`, the chains' filters from
/// `model` at the schedule's rate, with its ground truth.
///
/// Errors: [`MeasureError::Invalid`] for a world parameter outside its
/// documented range, a segment on a unit the world lacks or walking an output
/// its unit lacks, a cycle that would play before reference frame 0 (choose
/// the domains' `origin_t` so the run starts after it), a timeline past
/// [`MAX_DEVICE_FRAMES`] or bursts whose samples together pass it, and every
/// error of [`render`].
pub fn render_system(
    world: &SystemWorld,
    schedule: &SystemSchedule,
    model: &dyn SectionModel,
) -> Result<SystemRender, MeasureError> {
    world.validate()?;
    let rate = schedule.rate_hz;
    let fd = f64::from(rate);
    let (bases, _) = world.output_bases();
    for s in schedule.segments() {
        let u = world
            .units
            .get(usize::from(s.unit.0))
            .ok_or_else(|| invalid(format!("no unit {}", s.unit.0)))?;
        if s.schedule
            .outputs()
            .iter()
            .any(|o| usize::from(*o) >= u.outputs.len())
        {
            return Err(invalid(format!(
                "unit {:?} lacks an output its walk plays",
                u.name
            )));
        }
    }

    let maps: Vec<UnitMap> = world
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| unit_map(world, i, u, fd))
        .collect();

    // The cycles and their reference-frame spans, before any allocation.
    let mut plans: Vec<CyclePlan> = Vec::new();
    let mut seg_truth: Vec<SegmentTruth> = Vec::new();
    for (si, seg) in schedule.segments().iter().enumerate() {
        let ui = usize::from(seg.unit.0);
        let (Some(unit), Some(map), Some(base)) =
            (world.units.get(ui), maps.get(ui), bases.get(ui))
        else {
            continue;
        };
        let faults = world.unit_faults(ui);
        let st = plan_segment(
            world,
            si,
            seg.unit,
            seg.start,
            &seg.schedule,
            unit,
            map,
            *base,
            &faults,
            rate,
            &mut plans,
        )?;
        seg_truth.push(st);
    }

    let mut total: u64 = 0;
    let mut end: i64 = 0;
    for p in &plans {
        total = total.saturating_add(p.hi.saturating_sub(p.lo).unsigned_abs());
        end = end.max(p.hi);
    }
    if total > MAX_DEVICE_FRAMES {
        return Err(invalid(format!(
            "bursts of {total} frames in all (at most {MAX_DEVICE_FRAMES})"
        )));
    }
    let post = u64::from(rate).saturating_mul(u64::from(world.post_roll_ms)) / 1000;
    let frames = end.unsigned_abs().saturating_add(post).max(1);
    if frames > MAX_DEVICE_FRAMES {
        return Err(invalid(format!(
            "a timeline of {frames} frames (at most {MAX_DEVICE_FRAMES})"
        )));
    }

    let bursts_w = world
        .wander
        .map(|w| w.bursts(frames as f64 / fd + 1.0))
        .unwrap_or_default();

    // The generator's cycles, at unit amplitude, rendered once per segment
    // kind (first or not).
    let mut signals: Vec<Vec<f32>> = Vec::with_capacity(plans.len());
    let kernel = Kernel::new(1.0);
    for p in &plans {
        let sweep = &p.sweep;
        let amp = world
            .units
            .get(p.unit)
            .map_or(0.0, |u| peak_linear(u.level_db));
        let mut cyc: Vec<f32> = sweep
            .render_cycle(p.first)
            .iter()
            .map(|v| *v as f32 * amp)
            .collect();
        if let Some(at) = p.fade_at {
            fade_out(&mut cyc, at, u64::from(sweep.fade_samples));
        }
        cyc.truncate(usize::try_from(p.samples).unwrap_or(usize::MAX));
        let Some(map) = maps.get(p.unit) else {
            signals.push(Vec::new());
            continue;
        };
        signals.push(warp(&cyc, p, map, &bursts_w, fd, &kernel));
    }
    let bursts: Vec<Burst<'_>> = plans
        .iter()
        .zip(&signals)
        .map(|(p, s)| Burst {
            output: p.output,
            start_frame: p.lo.unsigned_abs(),
            samples: s,
            gain: p.gain,
        })
        .collect();
    let stimulus = Stimulus { bursts, frames };
    let flat = world.flat_world();
    let chains: Vec<ChannelChain> = world
        .units
        .iter()
        .flat_map(|u| u.chains.iter().cloned())
        .collect();
    let capture = render(&flat, &stimulus, &chains, model, rate)?;

    // The truth.
    for st in &mut seg_truth {
        for c in &mut st.cycles {
            c.capture_frame =
                capture_at_mic(&flat, &bursts_w, rate, c.capture_frame).unwrap_or(f64::NAN);
        }
    }
    let outputs = output_truths(world, schedule, &bases, fd);
    let (units, faults) = fault_truths(world, schedule, &maps, &flat, &bursts_w, rate);
    Ok(SystemRender {
        capture,
        truth: SystemTruth {
            device_rate_hz: rate,
            timeline_frames: frames,
            units,
            outputs,
            segments: seg_truth,
            faults,
            wander: bursts_w,
        },
        world: flat,
    })
}

fn unit_map(world: &SystemWorld, i: usize, u: &SimUnit, fd: f64) -> UnitMap {
    let d = world.domains.get(u.domain).copied().unwrap_or(DomainClock {
        domain: DomainId {
            time_base: 0xFF,
            id: 0,
        },
        ppm: 0.0,
        origin_t: 0,
    });
    let hop = match u.link {
        UnitLink::Line {
            hop_error_frames, ..
        } => f64::from(hop_error_frames),
        _ => 0.0,
    };
    let mut steps: Vec<(u64, i64)> = world
        .unit_faults(i)
        .iter()
        .filter_map(|(f, _)| f.at().map(|at| (at, f.shift())))
        .filter(|(_, n)| *n != 0)
        .collect();
    steps.sort_by_key(|s| s.0);
    let seed = world.seed ^ (i as u64).wrapping_mul(0x4A49_5454_4552_0001);
    UnitMap {
        eps: d.ppm * 1e-6,
        origin_t: d.origin_t,
        fixed: u.playout_frames + hop,
        steps,
        jitter: u.jitter,
        seed,
        phase: std::f64::consts::TAU * uniform(seed, 0x5048),
        fd,
    }
}

/// The chain's delay as the device runs it, whole samples.
fn chain_delay_frames(chain: Option<&ChannelChain>, fd: f64) -> u64 {
    chain.map_or(0, |c| {
        (f64::from(c.delay_ms) * fd / 1000.0).round().max(0.0) as u64
    })
}

#[allow(clippy::too_many_arguments)]
fn plan_segment(
    world: &SystemWorld,
    si: usize,
    unit_ix: UnitIndex,
    start: u64,
    walk: &WalkSchedule,
    unit: &SimUnit,
    map: &UnitMap,
    base: usize,
    faults: &[(UnitFault, bool)],
    rate: u32,
    plans: &mut Vec<CyclePlan>,
) -> Result<SegmentTruth, MeasureError> {
    let fd = f64::from(rate);
    let usb_origin = match unit.link {
        UnitLink::UsbDirect { origin } => origin,
        _ => 0,
    };
    let mut truth = SegmentTruth {
        segment: si,
        unit: unit_ix,
        scheduled_t: start,
        scheduled_t_unit: start.wrapping_add_signed(usb_origin),
        fate: SegmentFate::Played,
        started_t: None,
        reported_start: None,
        samples: 0,
        discontinuities: 0,
        cycles: Vec::new(),
    };
    if faults
        .iter()
        .any(|(f, _)| matches!(f, UnitFault::NeverStarts))
    {
        truth.fate = SegmentFate::RefusedNotReady;
        return Ok(truth);
    }
    // Epoch jumps before the START: the unit reads `start` in its new
    // numbering, i.e. at physical `start - jump`.
    let mut phys = i128::from(start);
    let mut jump_total: i64 = 0;
    for (f, _) in faults {
        if let UnitFault::EpochChange {
            at_t,
            t_jump,
            frames,
        } = *f
            && at_t < start
        {
            jump_total = jump_total.saturating_add(t_jump);
            phys = phys.saturating_sub(i128::from(t_jump));
            if phys <= i128::from(at_t.saturating_add(frames)) {
                truth.fate = SegmentFate::RefusedLate;
                return Ok(truth);
            }
        }
    }
    let phys = u64::try_from(phys).unwrap_or(0);
    if faults
        .iter()
        .any(|(f, _)| f.loss_window().is_some_and(|(a, b)| phys >= a && phys <= b))
    {
        truth.fate = SegmentFate::CancelledLoss;
        return Ok(truth);
    }
    let actual = phys.saturating_add_signed(i64::from(unit.start_error_samples));
    let total = walk.total_samples();
    let fade = u64::from(walk.sweep.fade_samples);
    // The first loss inside the run stops it (with the fade).
    let stop = faults
        .iter()
        .filter_map(|(f, _)| f.loss_window().map(|(a, _)| a))
        .filter(|a| *a > actual && *a < actual.saturating_add(total))
        .min();
    let corrections = faults
        .iter()
        .filter(|(f, _)| matches!(f, UnitFault::Correction { .. }))
        .filter_map(|(f, _)| f.at())
        .filter(|a| *a > actual && *a < stop.unwrap_or(actual.saturating_add(total)))
        .count();
    truth.discontinuities = u16::try_from(corrections)
        .unwrap_or(u16::MAX)
        .saturating_add(u16::from(stop.is_some()));
    truth.fate = if stop.is_some() {
        SegmentFate::StoppedLoss
    } else {
        SegmentFate::Played
    };
    truth.started_t = Some(actual);
    truth.reported_start = Some(
        actual
            .wrapping_add_signed(usb_origin)
            .wrapping_add_signed(jump_total),
    );
    let played_until = stop.map_or(actual.saturating_add(total), |s| s.saturating_add(fade));
    truth.samples = played_until.saturating_sub(actual).min(total);

    let cycle_len = u64::from(walk.sweep.cycle_samples);
    for c in walk.cycles() {
        let t0 = actual.saturating_add(c.start);
        let global = base.saturating_add(usize::from(c.output));
        let chain = unit.chains.get(usize::from(c.output));
        let acoustic = unit
            .outputs
            .get(usize::from(c.output))
            .map_or(0.0, |p| (p.driver.offset_ms + p.delay_ms) * fd / 1000.0);
        let delay = chain_delay_frames(chain, fd) as f64 + acoustic;
        let device_frame = map.play(t0 as f64);
        let played = t0 < played_until;
        let samples = played_until.saturating_sub(t0).min(cycle_len);
        let truncated = played && samples < cycle_len;
        truth.cycles.push(CycleTruth {
            index: c.index,
            output: c.output,
            global_output: global,
            pass: c.pass,
            t: t0,
            played,
            truncated,
            device_frame,
            // The position at the microphone for now; the capture frame is
            // filled in once the wander is known.
            capture_frame: device_frame + delay,
        });
        if !played {
            continue;
        }
        let fade_at = stop
            .filter(|s| *s >= t0 && *s < t0.saturating_add(cycle_len))
            .map(|s| s.saturating_sub(t0));
        let gain = if unit.invert_mask & 1u16.checked_shl(u32::from(c.output)).unwrap_or(0) != 0 {
            -1.0
        } else {
            1.0
        };
        // The span: the mapping's ends plus every bounded excursion (the
        // wander bound, the jitter, the kernel).
        let margin = 64.0
            + world.wander.map_or(0.0, |w| w.bound_us * 1e-6 * fd)
            + unit
                .jitter
                .map_or(0.0, |j| (6.0 * j.white_rms_ns + j.wander_ns) * 1e-9 * fd);
        let a = map.play(t0 as f64);
        let b = map.play(t0.saturating_add(samples) as f64);
        if a.min(b) < 0.0 {
            return Err(invalid(
                "a cycle plays before reference frame 0 (move the domain's origin_t)",
            ));
        }
        let lo = (a.min(b) - margin).floor().max(0.0);
        let hi = (a.max(b) + margin).ceil();
        if !(lo.is_finite() && hi.is_finite() && hi - lo <= MAX_DEVICE_FRAMES as f64 * 2.0) {
            return Err(invalid("a cycle maps outside the timeline"));
        }
        plans.push(CyclePlan {
            sweep: walk.sweep,
            output: global,
            t0: t0 as f64,
            samples,
            fade_at,
            first: c.index == 0,
            gain,
            unit: usize::from(unit_ix.0),
            delay,
            lo: lo as i64,
            hi: hi as i64,
        });
    }
    Ok(truth)
}

/// A raised-cosine fade to zero over `len` samples from `at`; zeros after.
fn fade_out(sig: &mut [f32], at: u64, len: u64) {
    let len = len.max(1);
    for (i, v) in sig
        .iter_mut()
        .enumerate()
        .skip(usize::try_from(at).unwrap_or(usize::MAX))
    {
        let k = (i as u64).saturating_sub(at);
        if k >= len {
            *v = 0.0;
        } else {
            let g = 0.5 + 0.5 * (std::f64::consts::PI * k as f64 / len as f64).cos();
            *v = (f64::from(*v) * g) as f32;
        }
    }
}

/// The cycle's samples on the reference timeline from `p.lo` to `p.hi`.
fn warp(
    sig: &[f32],
    p: &CyclePlan,
    map: &UnitMap,
    wander: &[WanderBurst],
    fd: f64,
    kernel: &Kernel,
) -> Vec<f32> {
    let n = p.hi.saturating_sub(p.lo).max(0).unsigned_abs();
    let n = usize::try_from(n).unwrap_or(0);
    let mut out = Vec::with_capacity(n);
    let base0 = map.base(p.t0);
    for i in 0..n {
        let q = p.lo as f64 + i as f64;
        // The wander where this sound reaches the microphone.
        let w = wander_error_us(wander, (q + p.delay) / fd) * 1e-6 * fd;
        let target = q + w - map.jitter(q) - map.fixed;
        // The sample index without the steps, then with the steps at it.
        let n0 = (target - base0) * (1.0 + map.eps);
        let s = map.shift(p.t0 + n0.floor());
        let idx = (target - s - base0) * (1.0 + map.eps);
        out.push(kernel.interp(sig, idx) as f32);
    }
    out
}

fn output_truths(
    world: &SystemWorld,
    schedule: &SystemSchedule,
    bases: &[usize],
    fd: f64,
) -> Vec<OutputTruth> {
    let mut walked: Vec<(UnitIndex, u8)> = schedule
        .segments()
        .iter()
        .flat_map(|s| s.schedule.outputs().into_iter().map(move |o| (s.unit, o)))
        .collect();
    walked.sort_unstable();
    walked.dedup();
    walked
        .into_iter()
        .filter_map(|(unit, output)| {
            let u = world.units.get(usize::from(unit.0))?;
            let global = bases
                .get(usize::from(unit.0))?
                .saturating_add(usize::from(output));
            let path = u.outputs.get(usize::from(output))?;
            let hop = match u.link {
                UnitLink::Line {
                    hop_error_frames, ..
                } => hop_error_frames,
                _ => 0,
            };
            let chain_delay = chain_delay_frames(u.chains.get(usize::from(output)), fd);
            let acoustic = (path.driver.offset_ms + path.delay_ms) * fd / 1000.0;
            let total = u.playout_frames
                + f64::from(hop)
                + f64::from(u.start_error_samples)
                + chain_delay as f64
                + acoustic;
            Some(OutputTruth {
                unit,
                output,
                global_output: global,
                playout_frames: u.playout_frames,
                hop_error_frames: hop,
                start_error_samples: u.start_error_samples,
                chain_delay_frames: chain_delay,
                acoustic_frames: acoustic,
                arrival_frames: total,
                arrival_us: total / fd * 1e6,
            })
        })
        .collect()
}

fn fault_truths(
    world: &SystemWorld,
    schedule: &SystemSchedule,
    maps: &[UnitMap],
    flat: &World,
    wander: &[WanderBurst],
    rate: u32,
) -> (Vec<UnitTruth>, Vec<FaultTruth>) {
    let (run_a, run_b) = (schedule.start(), schedule.end());
    let mut units = Vec::with_capacity(world.units.len());
    let mut faults = Vec::new();
    for (i, u) in world.units.iter().enumerate() {
        let unit = UnitIndex(u8::try_from(i).unwrap_or(u8::MAX));
        let mut ut = UnitTruth {
            unit,
            domain: world.domains.get(u.domain).map_or(
                DomainId {
                    time_base: 0xFF,
                    id: 0,
                },
                |d| d.domain,
            ),
            in_schedule_domain: world
                .domains
                .get(u.domain)
                .is_some_and(|d| d.domain == schedule.domain),
            disturbances: 0,
            events: 0,
            t_minus_output_moved: false,
        };
        for (f, inherited) in world.unit_faults(i) {
            let t = f.at();
            let (dist, event, moves) = match f {
                UnitFault::NeverStarts => (0u16, false, false),
                // Holdover entered, resumed.
                UnitFault::Holdover { .. } => (2, true, false),
                UnitFault::Reanchor { shift_frames, .. } => (1, true, shift_frames != 0),
                UnitFault::Correction { frames, event, .. } => (1, event, frames != 0),
                UnitFault::EpochChange { .. } => (1, true, true),
            };
            let in_run = t.is_some_and(|t| t >= run_a && t <= run_b);
            let device_frame = t.and_then(|t| maps.get(i).map(|m| m.base(t as f64)));
            let capture_frame = device_frame.and_then(|p| capture_at_mic(flat, wander, rate, p));
            if in_run {
                ut.disturbances = ut.disturbances.saturating_add(dist);
                ut.events = ut.events.saturating_add(u16::from(event));
                ut.t_minus_output_moved |= moves;
            }
            faults.push(FaultTruth {
                unit,
                fault: f,
                inherited,
                t,
                device_frame,
                capture_frame,
                in_run,
                disturbances: dist,
                event,
                moves_t_minus_output: moves,
            });
        }
        units.push(ut);
    }
    (units, faults)
}

#[cfg(test)]
mod tests_wp_atsim {
    #![allow(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::unwrap_used
    )]

    use super::*;
    use crate::sim::ProtoModel;
    use crate::stimulus::SweepConfig;
    use crate::system::Segment;
    use proptest::prelude::*;
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex;

    const FS: u32 = 48_000;
    const D: DomainId = DomainId {
        time_base: 0,
        id: 0x42,
    };
    const ORIGIN: u64 = 1_000_000_000;
    /// 0.1 us in frames at 48 kHz.
    const US01: f64 = 0.1e-6 * 48_000.0;

    fn cfg() -> SweepConfig {
        SweepConfig {
            f1_hz: 20.0,
            f2_hz: 20_000.0,
            duration_ms: 1_000,
            gap_ms: 300,
        }
    }

    fn walk(mask: u16, repeat: u16) -> WalkSchedule {
        let sweep = crate::stimulus::LogSweep::new(FS, &cfg()).unwrap();
        WalkSchedule::new(sweep, mask, repeat).unwrap()
    }

    /// Segments back to back, `gap` frames apart, from `start`.
    fn schedule(parts: &[(u8, u16)], start: u64, gap: u64) -> SystemSchedule {
        let mut at = start;
        let segs = parts
            .iter()
            .map(|&(u, m)| {
                let s = Segment {
                    unit: UnitIndex(u),
                    start: at,
                    schedule: walk(m, 1),
                };
                at = s.end() + gap;
                s
            })
            .collect();
        SystemSchedule::new(D, FS, segs).unwrap()
    }

    fn unit(name: &str, outputs: usize, playout: f64) -> SimUnit {
        let mut u = SimUnit::flat(name, outputs);
        u.playout_frames = playout;
        u
    }

    fn render_ok(w: &SystemWorld, s: &SystemSchedule) -> SystemRender {
        render_system(w, s, &ProtoModel).unwrap_or_else(|e| panic!("render: {e}"))
    }

    /// `x(j) = y(j - d)`: `d` by the cross-spectrum's phase slope over
    /// `lo..hi` Hz (|d| below about one sample).
    fn phase_delay(x: &[f64], y: &[f64], lo: f64, hi: f64) -> f64 {
        let n = x.len().max(y.len()).next_power_of_two();
        let mut planner = FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        let spec = |v: &[f64]| {
            let mut b: Vec<Complex<f64>> = v.iter().map(|r| Complex::new(*r, 0.0)).collect();
            b.resize(n, Complex::new(0.0, 0.0));
            fft.process(&mut b);
            b
        };
        let (xs, ys) = (spec(x), spec(y));
        let (mut num, mut den) = (0.0, 0.0);
        for b in 1..n / 2 {
            let f = b as f64 * f64::from(FS) / n as f64;
            if f < lo || f > hi {
                continue;
            }
            let c = xs[b] * ys[b].conj();
            let w = c.norm();
            let om = std::f64::consts::TAU * b as f64 / n as f64;
            num += w * om * (-c.arg());
            den += w * om * om;
        }
        num / den
    }

    /// The capture frame at which the cycle's sample 0 is heard, measured
    /// from the capture against the generator's own cycle.
    fn measured_arrival(r: &SystemRender, c: &CycleTruth, sweep_first: bool, gain: f64) -> f64 {
        measured_arrival_band(r, c, sweep_first, gain, 15_000.0)
    }

    fn measured_arrival_band(
        r: &SystemRender,
        c: &CycleTruth,
        sweep_first: bool,
        gain: f64,
        hi: f64,
    ) -> f64 {
        let sweep = crate::stimulus::LogSweep::new(FS, &cfg()).unwrap();
        let g: Vec<f64> = sweep
            .render_cycle(sweep_first)
            .iter()
            .map(|v| v * gain)
            .collect();
        let m = 512usize;
        let k_int = c.capture_frame.floor() as usize;
        let len = (g.len() + 2 * m).next_power_of_two();
        let x: Vec<f64> = r.capture.samples[k_int - m..k_int - m + len]
            .iter()
            .map(|v| f64::from(*v))
            .collect();
        let mut y = vec![0.0; len];
        y[m..m + g.len()].copy_from_slice(&g);
        k_int as f64 + phase_delay(&x, &y, 100.0, hi)
    }

    #[test]
    fn wp_atsim_rendered_arrivals_match_closed_form_without_jitter() {
        let mut a = unit("A", 2, 12.37);
        a.outputs[0].delay_ms = 3.21;
        a.outputs[1].delay_ms = 2.5;
        a.outputs[1].driver.offset_ms = 0.25;
        a.chains[1].delay_ms = 0.5;
        let mut b = unit("B", 2, 40.5);
        b.link = UnitLink::UsbDirect { origin: 4096 * 48 };
        b.outputs[0].delay_ms = 4.04;
        b.outputs[1].delay_ms = 1.7;
        let mut w = SystemWorld::free_field(vec![a, b], D, ORIGIN, FS);
        w.clock.latency_ms = 7.3;
        let s = schedule(&[(0, 0b11), (1, 0b11)], ORIGIN + 4_800, 2_400);
        let r = render_ok(&w, &s);
        let lat = 7.3e-3 * f64::from(FS);
        let mut n = 0;
        for seg in &r.truth.segments {
            assert_eq!(seg.fate, SegmentFate::Played);
            let u = &w.units[usize::from(seg.unit.0)];
            for c in &seg.cycles {
                let p = &u.outputs[usize::from(c.output)];
                let chain = (f64::from(u.chains[usize::from(c.output)].delay_ms) * 48.0).round();
                let closed = (c.t - ORIGIN) as f64
                    + u.playout_frames
                    + chain
                    + (p.delay_ms + p.driver.offset_ms) * 48.0
                    + lat;
                assert!((c.capture_frame - closed).abs() < 1e-6, "{c:?} vs {closed}");
                let m = measured_arrival(&r, c, c.index == 0, 1.0);
                assert!(
                    (m - closed).abs() < US01,
                    "cycle {c:?}: measured {m}, closed form {closed}"
                );
                n += 1;
            }
        }
        assert_eq!(n, 4);
        // The USB unit's own numbering.
        let sb = &r.truth.segments[1];
        assert_eq!(sb.scheduled_t_unit, sb.scheduled_t + 4096 * 48);
        assert_eq!(sb.reported_start, Some(sb.scheduled_t + 4096 * 48));
        // Output truth: arrival relative to the domain's play time of T.
        let o = r.truth.output(UnitIndex(0), 1).unwrap();
        assert!((o.arrival_frames - (12.37 + 24.0 + 2.75 * 48.0)).abs() < 1e-9);
        assert!((o.arrival_us - o.arrival_frames / 48_000.0 * 1e6).abs() < 1e-9);
    }

    #[test]
    fn wp_atsim_fan_out_hop_error_is_one_frame_late() {
        let a = unit("A", 1, 30.0);
        let mut b = unit("B", 1, 30.0);
        b.link = UnitLink::Line {
            hops: 2,
            hop_error_frames: 1,
        };
        let w = SystemWorld::free_field(vec![a, b], D, ORIGIN, FS);
        let s = schedule(&[(0, 1), (1, 1)], ORIGIN + 1_000, 0);
        let r = render_ok(&w, &s);
        let ta = r.truth.output(UnitIndex(0), 0).unwrap().arrival_us;
        let tb = r.truth.output(UnitIndex(1), 0).unwrap().arrival_us;
        assert!((tb - ta - 1e6 / 48_000.0).abs() < 1e-9);
        let ca = &r.truth.segments[0].cycles[0];
        let cb = &r.truth.segments[1].cycles[0];
        let ma = measured_arrival(&r, ca, true, 1.0) - (ca.t - ORIGIN) as f64;
        let mb = measured_arrival(&r, cb, true, 1.0) - (cb.t - ORIGIN) as f64;
        assert!((mb - ma - 1.0).abs() < 2.0 * US01, "{ma} {mb}");
    }

    #[test]
    fn wp_atsim_mic_clock_and_two_domains_closed_form() {
        let a = unit("A", 1, 5.0);
        let mut b = unit("B", 1, 5.0);
        b.domain = 1;
        let mut w = SystemWorld::free_field(vec![a, b], D, ORIGIN, FS);
        w.domains.push(DomainClock {
            domain: DomainId {
                time_base: 0,
                id: 0x77,
            },
            ppm: -20.0,
            origin_t: ORIGIN - 2_000,
        });
        w.clock.ppm = 37.5;
        w.clock.latency_ms = 50.0;
        let s = schedule(&[(0, 1), (1, 1)], ORIGIN + 1_000, 0);
        let r = render_ok(&w, &s);
        assert!(r.truth.units[0].in_schedule_domain);
        assert!(!r.truth.units[1].in_schedule_domain);
        for (seg, (ppm, origin)) in r
            .truth
            .segments
            .iter()
            .zip([(0.0, ORIGIN), (-20.0, ORIGIN - 2_000)])
        {
            let c = &seg.cycles[0];
            let tau = (c.t - origin) as f64 / (1.0 + ppm * 1e-6) + 5.0;
            // P5SIM's convention: capture frame k samples k / (1 + ppm) - lat.
            let closed = (tau + 50e-3 * 48_000.0) * (1.0 + 37.5e-6);
            assert!((c.capture_frame - closed).abs() < 1e-6);
            // The capture holds it there (the clock stretches the sweep by
            // about 2 frames, so only coarsely).
            let m = measured_arrival_band(&r, c, true, 1.0, 2_000.0);
            assert!((m - closed).abs() < 2.0, "{m} vs {closed}");
        }
    }

    #[test]
    fn wp_atsim_wander_bursts_follow_p5spike() {
        for seed in 0..100u64 {
            let wb = WanderBursts::p5spike(seed).bursts(120.0);
            assert!(wb.len() > 20, "seed {seed}: {} bursts", wb.len());
            for b in &wb {
                assert!((55.0..=100.0).contains(&b.ppm.abs()), "{b:?}");
                assert!((0.25..=1.5).contains(&b.duration_s), "{b:?}");
            }
            let mut t = 0.0;
            let mut peak: f64 = 0.0;
            while t < 120.0 {
                let e = wander_error_us(&wb, t);
                assert!(e.abs() <= 50.0 + 1e-9, "seed {seed} t {t}: {e}");
                peak = peak.max(e.abs());
                t += 0.01;
            }
            assert!(peak > 20.0, "seed {seed}: peak {peak}");
        }
    }

    #[test]
    fn wp_atsim_rendered_wander_matches_truth() {
        let a = unit("A", 1, 0.0);
        let mut w = SystemWorld::free_field(vec![a], D, ORIGIN, FS);
        w.wander = Some(WanderBursts::p5spike(7));
        let s = SystemSchedule::new(
            D,
            FS,
            vec![Segment {
                unit: UnitIndex(0),
                start: ORIGIN + 1_000,
                schedule: walk(1, 8),
            }],
        )
        .unwrap();
        let r = render_ok(&w, &s);
        assert!(!r.truth.wander.is_empty());
        let sweep = crate::stimulus::LogSweep::new(FS, &cfg()).unwrap();
        let g = sweep.render_cycle(false);
        let len = 4096usize;
        let mut moved: f64 = 0.0;
        for c in r.truth.segments[0].cycles.iter().skip(1) {
            for n_s in [32_000usize, 36_000, 40_000] {
                // Where sample n is heard, from the truth's mapping.
                let at = |n: usize| r.capture_frame_at_mic(c.device_frame + n as f64).unwrap();
                let k0 = at(n_s).floor() as usize;
                let hann =
                    |j: usize| 0.5 - 0.5 * (std::f64::consts::TAU * j as f64 / len as f64).cos();
                let x: Vec<f64> = (0..len)
                    .map(|j| f64::from(r.capture.samples[k0 + j]) * hann(j))
                    .collect();
                let y: Vec<f64> = (0..len).map(|j| g[n_s + j] * hann(j)).collect();
                let d = phase_delay(&x, &y, 500.0, 12_000.0);
                let expect = at(n_s + len / 2) - (len / 2) as f64 - k0 as f64;
                assert!(
                    (d - expect).abs() < 5.0 * US01,
                    "cycle {} n {n_s}: {d} vs {expect}",
                    c.index
                );
                moved = moved.max((at(n_s) - (c.device_frame + n_s as f64)).abs());
            }
        }
        assert!(moved > 0.2, "the wander moved {moved} frames");
    }

    #[test]
    fn wp_atsim_faults_appear_in_the_ground_truth() {
        // A: a holdover in its second pass. B: a silent one-frame correction.
        // C: never starts. S: an S/PDIF source whose epoch changes; F: its
        // follower. Then A again, after a re-anchor that left it 2 frames late.
        let a = unit("A", 1, 10.0);
        let b = unit("B", 1, 10.0);
        let mut c = unit("C", 1, 10.0);
        c.faults.push(UnitFault::NeverStarts);
        let mut src = unit("S", 1, 3.0);
        src.link = UnitLink::SpdifSource;
        let mut fol = unit("F", 1, 64.0);
        fol.link = UnitLink::SpdifFollower {
            source: UnitIndex(3),
        };
        let w0 = SystemWorld::free_field(vec![a, b, c, src, fol], D, ORIGIN, FS);
        let segs = || {
            let mut at = ORIGIN + 1_000;
            let mut out = Vec::new();
            for (u, rep) in [(0u8, 2u16), (1, 2), (2, 1), (3, 1), (4, 1), (0, 1)] {
                let s = Segment {
                    unit: UnitIndex(u),
                    start: at,
                    schedule: walk(1, rep),
                };
                at = s.end() + 4_800;
                out.push(s);
            }
            out
        };
        let sch = SystemSchedule::new(D, FS, segs()).unwrap();
        let sg = sch.segments().to_vec();
        let period = sg[0].schedule.period_samples();
        let mut w = w0.clone();
        // A's holdover in the middle of its second cycle.
        let hold_at = sg[0].start + period + 20_000;
        w.units[0].faults.push(UnitFault::Holdover {
            at_t: hold_at,
            frames: 4_800,
        });
        // A re-anchors before its last segment, 2 frames late afterwards.
        w.units[0].faults.push(UnitFault::Reanchor {
            at_t: sg[4].end() + 100,
            frames: 480,
            shift_frames: 2,
        });
        // B: a correction without an event between its cycles.
        let corr_at = sg[1].start + period - 1_000;
        w.units[1].faults.push(UnitFault::Correction {
            at_t: corr_at,
            frames: 1,
            event: false,
        });
        // The source's epoch changes before its segment: the START it was
        // given in the old numbering lands 9600 samples earlier, before the
        // re-anchor ends: refused late, on the source and the follower.
        w.units[3].faults.push(UnitFault::EpochChange {
            at_t: sg[3].start - 2_000,
            t_jump: 9_600,
            frames: 480,
        });
        let r = render_ok(&w, &sch);
        let t = &r.truth;
        let fates: Vec<SegmentFate> = t.segments.iter().map(|s| s.fate).collect();
        assert_eq!(
            fates,
            vec![
                SegmentFate::StoppedLoss,
                SegmentFate::Played,
                SegmentFate::RefusedNotReady,
                SegmentFate::RefusedLate,
                SegmentFate::Played,
                SegmentFate::Played,
            ]
        );
        // The follower: the source's epoch change is inherited, yet its own
        // START (after the re-anchor) plays.
        assert!(
            t.faults
                .iter()
                .any(|f| f.unit == UnitIndex(4) && f.inherited && f.t == Some(sg[3].start - 2_000))
        );
        assert_eq!(
            t.segments[4].reported_start,
            Some(sg[4].start),
            "the follower reports its start in the new numbering"
        );
        // Every fault at its T, with its counts.
        let find = |u: u8, k: fn(&UnitFault) -> bool| {
            t.faults
                .iter()
                .find(|f| f.unit == UnitIndex(u) && k(&f.fault))
                .copied()
                .unwrap()
        };
        let h = find(0, |f| matches!(f, UnitFault::Holdover { .. }));
        assert_eq!(h.t, Some(hold_at));
        assert!(h.in_run && h.event && h.disturbances == 2);
        let dev = (hold_at - ORIGIN) as f64;
        assert!((h.device_frame.unwrap() - dev).abs() < 1e-9);
        assert!((h.capture_frame.unwrap() - dev).abs() < 1e-6);
        let cr = find(1, |f| matches!(f, UnitFault::Correction { .. }));
        assert_eq!(cr.t, Some(corr_at));
        assert!(!cr.event && cr.disturbances == 1 && cr.moves_t_minus_output);
        assert_eq!(t.units[1].events, 0);
        assert_eq!(t.units[1].disturbances, 1);
        assert!(t.units[1].t_minus_output_moved);
        assert_eq!(t.units[2].disturbances, 0);
        assert!(find(3, |f| matches!(f, UnitFault::EpochChange { .. })).in_run);
        assert_eq!(t.units[0].disturbances, 3);
        // A's first segment: cycle 1 cut short, silence after the fade.
        let s0 = &t.segments[0];
        assert!(s0.cycles[0].played && !s0.cycles[0].truncated);
        assert!(s0.cycles[1].played && s0.cycles[1].truncated);
        assert_eq!(s0.discontinuities, 1);
        let fade = u64::from(sg[0].schedule.sweep.fade_samples);
        assert_eq!(s0.samples, hold_at + fade - sg[0].start);
        let quiet_from = (hold_at + fade - ORIGIN) as usize + 10 + 200;
        let quiet_to = (sg[1].start - ORIGIN) as usize;
        let e: f64 = r.capture.samples[quiet_from..quiet_to]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum();
        assert!(e < 1e-12, "energy {e} after the stop");
        // B: one discontinuity, its second cycle one frame later than the
        // closed form without the correction, in the truth and the capture.
        let s1 = &t.segments[1];
        assert_eq!(s1.discontinuities, 1);
        for (i, cyc) in s1.cycles.iter().enumerate() {
            let closed = (cyc.t - ORIGIN) as f64 + 10.0 + i as f64;
            assert!((cyc.capture_frame - closed).abs() < 1e-9, "{cyc:?}");
            let m = measured_arrival(&r, cyc, i == 0, 1.0);
            assert!((m - closed).abs() < US01, "{m} vs {closed}");
        }
        // C and the source are silent; A's last segment is 2 frames late.
        assert!(t.segments[2].cycles.iter().all(|c| !c.played));
        assert_eq!(t.segments[3].started_t, None);
        let last = &t.segments[5].cycles[0];
        let closed = (last.t - ORIGIN) as f64 + 10.0 + 2.0;
        assert!((last.capture_frame - closed).abs() < 1e-9);
        let m = measured_arrival(&r, last, true, 1.0);
        assert!((m - closed).abs() < US01, "{m} vs {closed}");
    }

    #[test]
    fn wp_atsim_a_start_in_a_loss_is_cancelled_and_start_error_shifts() {
        let mut a = unit("A", 1, 0.0);
        a.start_error_samples = 1;
        let mut b = unit("B", 1, 0.0);
        let s = schedule(&[(0, 1), (1, 1)], ORIGIN + 1_000, 0);
        b.faults.push(UnitFault::Holdover {
            at_t: s.segments()[1].start - 10,
            frames: 100,
        });
        let w = SystemWorld::free_field(vec![a, b], D, ORIGIN, FS);
        let r = render_ok(&w, &s);
        assert_eq!(r.truth.segments[1].fate, SegmentFate::CancelledLoss);
        let s0 = &r.truth.segments[0];
        assert_eq!(s0.started_t, Some(ORIGIN + 1_001));
        assert_eq!(s0.reported_start, Some(ORIGIN + 1_001));
        let o = r.truth.output(UnitIndex(0), 0).unwrap();
        assert!((o.arrival_us - 1e6 / 48_000.0).abs() < 1e-9);
        let m = measured_arrival(&r, &s0.cycles[0], true, 1.0);
        assert!((m - 1_001.0).abs() < US01, "{m}");
    }

    #[test]
    fn wp_atsim_summed_inverted_twin_cancels() {
        let a = unit("A", 1, 17.25);
        let mut b = unit("B", 1, 17.25);
        b.invert_mask = 1;
        b.link = UnitLink::UsbDirect { origin: -5 };
        let w = SystemWorld::free_field(vec![a, b], D, ORIGIN, FS);
        let s = SystemSchedule::summed(
            D,
            FS,
            ORIGIN + 2_000,
            vec![(UnitIndex(0), walk(1, 1)), (UnitIndex(1), walk(1, 1))],
        )
        .unwrap();
        let r = render_ok(&w, &s);
        let t = &r.truth;
        assert_eq!(t.segments[0].cycles[0].t, t.segments[1].cycles[0].t);
        assert_eq!(t.segments[1].scheduled_t_unit, ORIGIN + 2_000 - 5);
        let peak = r.capture.samples.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak < 1e-5, "peak {peak}");
        // Not inverted: the sum doubles.
        let mut w2 = w.clone();
        w2.units[1].invert_mask = 0;
        let r2 = render_ok(&w2, &s);
        let peak2 = r2.capture.samples.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((f64::from(peak2) - 0.2).abs() < 0.01, "peak {peak2}");
    }

    #[test]
    fn wp_atsim_lock_jitter_moves_arrivals_within_its_bounds() {
        let mut a = unit("A", 1, 8.0);
        a.jitter = Some(LockJitter {
            white_rms_ns: 20.0,
            wander_ns: 2_000.0,
            wander_period_s: 100.0,
        });
        let mut w = SystemWorld::free_field(vec![a], D, ORIGIN, FS);
        w.seed = 3;
        let s = SystemSchedule::single(D, UnitIndex(0), ORIGIN + 1_000, walk(1, 3)).unwrap();
        let r = render_ok(&w, &s);
        let mut moved: f64 = 0.0;
        for c in &r.truth.segments[0].cycles {
            let nominal = (c.t - ORIGIN) as f64 + 8.0;
            let dev = (c.capture_frame - nominal) / 48_000.0 * 1e9;
            assert!(dev.abs() <= 6.0 * 20.0 + 2_000.0, "{dev} ns");
            moved = moved.max(dev.abs());
            let m = measured_arrival(&r, c, c.index == 0, 1.0);
            assert!((m - c.capture_frame).abs() < 3.0 * US01, "{m} vs {c:?}");
        }
        assert!(moved > 50.0, "the jitter moved nothing ({moved} ns)");
        // Off: exactly the nominal position.
        let mut w0 = w.clone();
        w0.units[0].jitter = None;
        let r0 = render_ok(&w0, &s);
        for c in &r0.truth.segments[0].cycles {
            assert!((c.capture_frame - ((c.t - ORIGIN) as f64 + 8.0)).abs() < 1e-9);
        }
    }

    #[test]
    fn wp_atsim_memory_and_parameters_are_bounded() {
        let w = SystemWorld::free_field(vec![unit("A", 1, 0.0)], D, ORIGIN, FS);
        // A timeline past MAX_DEVICE_FRAMES.
        let far = SystemSchedule::single(D, UnitIndex(0), ORIGIN + MAX_DEVICE_FRAMES, walk(1, 1))
            .unwrap();
        assert!(render_system(&w, &far, &ProtoModel).is_err());
        // Before reference frame 0.
        let early = SystemSchedule::single(D, UnitIndex(0), ORIGIN - 10, walk(1, 1)).unwrap();
        assert!(render_system(&w, &early, &ProtoModel).is_err());
        // A unit or an output the world lacks.
        let none = SystemSchedule::single(D, UnitIndex(3), ORIGIN, walk(1, 1)).unwrap();
        assert!(render_system(&w, &none, &ProtoModel).is_err());
        let wide = SystemSchedule::single(D, UnitIndex(0), ORIGIN, walk(0b10, 1)).unwrap();
        assert!(render_system(&w, &wide, &ProtoModel).is_err());
        // Bursts beyond the bound in all: 16 units summed, 15 s at 96 kHz.
        let long = SweepConfig {
            duration_ms: 15_000,
            ..cfg()
        };
        let sweep = crate::stimulus::LogSweep::new(96_000, &long).unwrap();
        let units: Vec<SimUnit> = (0..16).map(|i| unit(&format!("u{i}"), 1, 0.0)).collect();
        let w16 = SystemWorld::free_field(units, D, ORIGIN, 96_000);
        let parts = (0..16)
            .map(|i| (UnitIndex(i), WalkSchedule::new(sweep, 1, 1).unwrap()))
            .collect();
        let big = SystemSchedule::summed(D, 96_000, ORIGIN, parts).unwrap();
        assert!(render_system(&w16, &big, &ProtoModel).is_err());
        // Invalid worlds.
        let one = SystemSchedule::single(D, UnitIndex(0), ORIGIN + 10, walk(1, 1)).unwrap();
        let bad: Vec<Mutate> = vec![
            Box::new(|w| w.units[0].playout_frames = f64::NAN),
            Box::new(|w| w.units[0].playout_frames = -1.0),
            Box::new(|w| w.units[0].domain = 5),
            Box::new(|w| w.units[0].chains.clear()),
            Box::new(|w| w.units.clear()),
            Box::new(|w| w.domains.clear()),
            Box::new(|w| w.domains[0].ppm = 2000.0),
            Box::new(|w| {
                w.units[0].link = UnitLink::SpdifFollower {
                    source: UnitIndex(0),
                }
            }),
            Box::new(|w| {
                w.units[0].link = UnitLink::Line {
                    hops: 0,
                    hop_error_frames: 5000,
                }
            }),
            Box::new(|w| w.units[0].level_db = 3.0),
            Box::new(|w| {
                w.units[0].jitter = Some(LockJitter {
                    white_rms_ns: 1.0,
                    wander_ns: 1.0,
                    wander_period_s: 0.0,
                })
            }),
            Box::new(|w| w.units[0].faults = vec![UnitFault::NeverStarts; 17]),
            Box::new(|w| {
                w.units[0].faults = vec![UnitFault::Holdover {
                    at_t: 0,
                    frames: u64::MAX,
                }]
            }),
            Box::new(|w| {
                w.wander = Some(WanderBursts {
                    max_ppm: 1e9,
                    ..WanderBursts::p5spike(0)
                })
            }),
            Box::new(|w| w.post_roll_ms = 20_000),
            Box::new(|w| w.units[0].outputs[0].delay_ms = -1.0),
            Box::new(|w| {
                let u = unit("x", 16, 0.0);
                w.units.push(u);
            }),
        ];
        for (i, f) in bad.iter().enumerate() {
            let mut w2 = w.clone();
            f(&mut w2);
            assert!(render_system(&w2, &one, &ProtoModel).is_err(), "case {i}");
        }
        assert!(render_system(&w, &one, &ProtoModel).is_ok());
    }

    type Mutate = Box<dyn Fn(&mut SystemWorld)>;

    fn any_fault() -> impl Strategy<Value = UnitFault> {
        let t = ORIGIN..ORIGIN + 200_000;
        prop_oneof![
            Just(UnitFault::NeverStarts),
            (t.clone(), 0u64..100_000)
                .prop_map(|(at_t, frames)| UnitFault::Holdover { at_t, frames }),
            (t.clone(), 0u64..10_000, -50i32..50).prop_map(|(at_t, frames, shift_frames)| {
                UnitFault::Reanchor {
                    at_t,
                    frames,
                    shift_frames,
                }
            }),
            (t.clone(), -50i32..50, any::<bool>()).prop_map(|(at_t, frames, event)| {
                UnitFault::Correction {
                    at_t,
                    frames,
                    event,
                }
            }),
            (t, -100_000i64..100_000, 0u64..10_000).prop_map(|(at_t, t_jump, frames)| {
                UnitFault::EpochChange {
                    at_t,
                    t_jump,
                    frames,
                }
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 10, ..ProptestConfig::default() })]

        #[test]
        fn proptest_wp_atsim_render_system_never_panics(
            playout in prop_oneof![0.0f64..200.0, proptest::num::f64::ANY],
            start_err in -5i32..5,
            hop in -3i32..3,
            ppm in -100.0f64..100.0,
            mic_ppm in -100.0f64..100.0,
            jitter in proptest::option::of((0.0f64..100.0, 0.0f64..5_000.0, 0.1f64..100.0)),
            faults in proptest::collection::vec(any_fault(), 0..4),
            wander in proptest::option::of(any::<u64>()),
            follower in any::<bool>(),
            start in ORIGIN..ORIGIN + 20_000,
        ) {
            let mut a = unit("A", 1, playout);
            a.start_error_samples = start_err;
            a.link = UnitLink::Line { hops: 1, hop_error_frames: hop };
            a.jitter = jitter.map(|(w, s, p)| LockJitter { white_rms_ns: w, wander_ns: s, wander_period_s: p });
            a.faults = faults;
            let mut b = unit("B", 1, 3.0);
            if follower {
                b.link = UnitLink::SpdifSource;
                a.link = UnitLink::SpdifFollower { source: UnitIndex(1) };
            }
            let mut w = SystemWorld::free_field(vec![a, b], D, ORIGIN, FS);
            w.domains[0].ppm = ppm;
            w.clock.ppm = mic_ppm;
            w.wander = wander.map(WanderBursts::p5spike);
            let s = schedule(&[(0, 1), (1, 1)], start, 0);
            if let Ok(r) = render_system(&w, &s, &ProtoModel) {
                prop_assert!(r.capture.samples.iter().all(|v| v.is_finite()));
                prop_assert_eq!(r.truth.segments.len(), 2);
                for seg in &r.truth.segments {
                    prop_assert!(seg.samples <= seg.cycles.len() as u64 * 48_000 + 1_000_000);
                    for c in &seg.cycles {
                        prop_assert!(c.capture_frame.is_finite());
                    }
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn proptest_wp_atsim_wander_stays_within_its_bound(
            seed in any::<u64>(),
            lo in 1.0f64..200.0,
            span in 0.0f64..200.0,
            min_s in 0.01f64..2.0,
            span_s in 0.0f64..3.0,
            bound in 1.0f64..200.0,
            gap in 0.0f64..1.0,
            t in prop_oneof![0.0f64..300.0, proptest::num::f64::ANY],
        ) {
            let w = WanderBursts {
                seed,
                min_ppm: lo,
                max_ppm: lo + span,
                min_s,
                max_s: min_s + span_s,
                bound_us: bound,
                min_gap_s: gap,
                max_gap_s: gap * 2.0,
            };
            let b = w.bursts(300.0);
            prop_assert!(b.len() <= MAX_WANDER_BURSTS);
            let e = wander_error_us(&b, t);
            prop_assert!(e.is_finite());
            prop_assert!(e.abs() <= bound + 1e-6, "{} > {}", e, bound);
            for x in &b {
                prop_assert!(x.ppm.abs() >= lo - 1e-9 && x.ppm.abs() <= lo + span + 1e-9);
            }
        }
    }
}
