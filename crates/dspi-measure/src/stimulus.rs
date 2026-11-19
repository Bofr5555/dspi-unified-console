//! The generator's log sweep and walk schedule, modelled sample by sample as
//! the firmware's `siggen.c` renders them (WP P5TIME, PLAN-P5 section 1.1
//! step 4).
//!
//! The DSPi's own test signal generator is the only stimulus of phase 5
//! (OD-P5a). The core renders its own copy of the sweep to deconvolve the
//! microphone's capture, so the copy must be the firmware's, not the analytic
//! sweep: `siggen.c` advances a 48-bit phase accumulator by an increment that
//! grows by a Q31 factor every sample, and that factor's quantisation (about one
//! part in 5·10⁴ of the sweep rate) moves the late part of a sweep by many
//! cycles against the analytic formula.
//!
//! The reference is DSPi v1.1.5 `firmware/DSPi/siggen.c` (upstream `ec26bf0`,
//! identical on `bofr-main` `8ba8b72` for everything modelled here):
//! - `compute_derived` (`:926-937`): the cycle length `fs * duration_ms / 1000`
//!   (at least two fades), the start increment `f1 / fs * 2^48` and the Q31
//!   growth `ln(f2 / f1) / dur * 2^31`, after the clamps `f1 >= 1`,
//!   `f2 <= 0.45 fs` (a `float` product) and `f2 > f1`; the fade
//!   `fs * 5 / 1000` (at least 8), which is also the attack and the release.
//! - `synth_sweep` (`:360-372`): per sample `ph += inc`, the sample is
//!   `sin(ph >> 16)` (a Q32 turn), then `inc += ((inc >> 16) * eps) >> 15`.
//! - `synth_reset_cycle`: every cycle restarts at `ph = 0`, `inc = inc0`, so
//!   repeated sweeps are bit-identical.
//! - `window_gain_at`: the linear attack `pos / attack` and release
//!   `(len - pos) / release` inside every cycle; `start_engine`: the 5 ms
//!   start fade overlays the first cycle only.
//! - `segment_finished`: with `SIGGEN_FLAG_WALK` the masked outputs play one per
//!   cycle in ascending order, wrapping; `repeat` counts passes
//!   (`cycles_target = repeat * popcount(mask)`); a gap of `fs * gap_ms / 1000`
//!   samples follows every cycle but the last.
//!
//! Two simplifications, both far below anything the analysis can see: the
//! sample value is `sin` in `f64` (the firmware's 7th-order polynomial has a THD
//! of about -139 dB), and the first cycle's fade times attack is applied per
//! sample (the firmware interpolates that product linearly between block
//! boundaries during the first 5 ms only).
//!
//! The model assumes every walked output is enabled in the matrix: the
//! firmware plays a disabled output's cycle as silence (`siggen_render`'s
//! `eff` mask), which the analysis reports as a cycle not heard.

use crate::types::MeasureError;

/// The rates the generator has precomputed rows for (`siggen_rates`), Hz.
pub const SUPPORTED_RATES: [u32; 3] = [44_100, 48_000, 96_000];
/// The shortest sweep a measurement uses, ms (PLAN-P5 section 1.1 step 1).
pub const MIN_SWEEP_MS: u32 = 1_000;
/// The longest sweep a measurement uses, ms.
pub const MAX_SWEEP_MS: u32 = 15_000;
/// The most passes of a walked run.
pub const MAX_REPEAT: u16 = 8;
/// The most outputs a run plays (the generator's mask is 16 bits).
pub const MAX_OUTPUTS: u32 = 16;
/// The generator's start and stop fade, ms (`SIGGEN_FADE_MS`).
pub const FADE_MS: u32 = 5;
/// The lowest and highest frequency the generator accepts for a sweep
/// parameter, Hz (its caps table).
pub const SWEEP_FREQ_RANGE_HZ: (f32, f32) = (1.0, 40_000.0);

const TWO_48: f64 = 281_474_976_710_656.0;
const TWO_32: f64 = 4_294_967_296.0;
const TWO_31: f64 = 2_147_483_648.0;

/// What the session stages for a log sweep (the `SiggenConfig` fields a sweep
/// uses).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepConfig {
    /// Start frequency (`p1`), Hz.
    pub f1_hz: f32,
    /// End frequency (`p2`), Hz.
    pub f2_hz: f32,
    /// One sweep (`duration_ms`), [`MIN_SWEEP_MS`] to [`MAX_SWEEP_MS`].
    pub duration_ms: u32,
    /// Silence after every cycle but the last (`gap_ms`).
    pub gap_ms: u16,
}

impl SweepConfig {
    /// The default of a speaker measurement: 20 Hz to 20 kHz in 5.5 s, 1 s gaps
    /// (PLAN-P5 section 1.1 step 1).
    pub const SPEAKERS: SweepConfig = SweepConfig {
        f1_hz: 20.0,
        f2_hz: 20_000.0,
        duration_ms: 5_500,
        gap_ms: 1_000,
    };
    /// The default on a headphone coupler: as for speakers with 300 ms gaps
    /// (review S-b).
    pub const COUPLER: SweepConfig = SweepConfig {
        gap_ms: 300,
        ..SweepConfig::SPEAKERS
    };
}

/// A log sweep as the generator derives it for one rate: every number the
/// firmware's render loop uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogSweep {
    /// The device's sample rate, Hz (one of [`SUPPORTED_RATES`]).
    pub rate_hz: u32,
    /// The start frequency after the firmware's clamps, Hz.
    pub f1_hz: f32,
    /// The end frequency after the firmware's clamps (at most `0.45 fs`), Hz.
    pub f2_hz: f32,
    /// Samples in one sweep.
    pub cycle_samples: u32,
    /// Samples of silence after a cycle (none after the last).
    pub gap_samples: u32,
    /// The start fade, which is also the attack and the release, samples.
    pub fade_samples: u32,
    /// The start increment, turns per sample in Q48 (`sweep_inc0`).
    pub inc0_q48: u64,
    /// The per-sample relative growth of the increment, Q31 (`sweep_eps_q31`).
    pub eps_q31: i32,
}

/// The firmware's per-sample sweep state: the Q48 phase and increment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepState {
    /// Phase accumulator, Q48 turns (wraps).
    pub phase_q48: u64,
    /// Increment, Q48 turns per sample.
    pub inc_q48: u64,
}

impl SweepState {
    /// One sample: returns the Q32 phase the sample is rendered at, then
    /// advances the increment, exactly as `synth_sweep` (`siggen.c:363-371`).
    pub fn step(&mut self, eps_q31: i32) -> u32 {
        self.phase_q48 = self.phase_q48.wrapping_add(self.inc_q48);
        // (uint32_t)(ph >> 16): the low 32 bits of the shifted phase.
        let out = (self.phase_q48 >> 16) as u32;
        // inc += (uint64_t)(((int64_t)(inc >> 16) * eps) >> 15)
        let prod = ((self.inc_q48 >> 16) as i64).wrapping_mul(i64::from(eps_q31));
        self.inc_q48 = self.inc_q48.wrapping_add((prod >> 15) as u64);
        out
    }
}

/// Converts a Q32 phase (one turn = 2^32) to the sample value at full scale,
/// as the firmware's `osc_sin` reads it (a signed turn, -0.5 to 0.5).
pub fn q32_phase_to_sample(phase_q32: u32) -> f64 {
    let turns = f64::from(phase_q32 as i32) / TWO_32;
    (std::f64::consts::TAU * turns).sin()
}

/// The generator's peak amplitude for a `level_db` (clamped to -120..0 dBFS,
/// `start_engine`), linear full scale.
pub fn peak_linear(level_db: f32) -> f32 {
    let db = if level_db.is_nan() {
        -120.0
    } else {
        level_db.clamp(-120.0, 0.0)
    };
    10f32.powf(db / 20.0)
}

impl LogSweep {
    /// Derives the sweep for `rate_hz` as `compute_derived` does.
    ///
    /// Errors ([`MeasureError::Invalid`]): a rate the generator has no row for,
    /// a duration outside [`MIN_SWEEP_MS`]..=[`MAX_SWEEP_MS`], a frequency that
    /// is not finite or outside [`SWEEP_FREQ_RANGE_HZ`], or a sweep that does
    /// not rise (`f2 <= f1` once `f2` is clamped to `0.45 fs`): the firmware
    /// would silently turn that into a 1 Hz sweep, which a measurement never
    /// wants.
    pub fn new(rate_hz: u32, config: &SweepConfig) -> Result<LogSweep, MeasureError> {
        if !SUPPORTED_RATES.contains(&rate_hz) {
            return Err(MeasureError::Invalid(format!(
                "the generator runs at 44.1, 48 or 96 kHz, not {rate_hz} Hz"
            )));
        }
        if !(MIN_SWEEP_MS..=MAX_SWEEP_MS).contains(&config.duration_ms) {
            return Err(MeasureError::Invalid(format!(
                "sweep of {} ms (1 to 15 s)",
                config.duration_ms
            )));
        }
        let (lo, hi) = SWEEP_FREQ_RANGE_HZ;
        let in_range = |f: f32| f.is_finite() && (lo..=hi).contains(&f);
        if !in_range(config.f1_hz) || !in_range(config.f2_hz) {
            return Err(MeasureError::Invalid(format!(
                "sweep frequencies {} to {} Hz",
                config.f1_hz, config.f2_hz
            )));
        }
        let fs = u64::from(rate_hz);
        // float nyq = 0.45f * (float)fs;
        let nyq = 0.45f32 * rate_hz as f32;
        let f1 = config.f1_hz.max(1.0);
        let f2 = config.f2_hz.min(nyq);
        if f2 <= f1 {
            return Err(MeasureError::Invalid(format!(
                "the sweep must rise: {f1} Hz to {f2} Hz at {rate_hz} Hz"
            )));
        }
        // fs <= 96000 and the durations are bounded: no product overflows.
        let fade = (fs.saturating_mul(u64::from(FADE_MS)) / 1000).max(8);
        let gap = fs.saturating_mul(u64::from(config.gap_ms)) / 1000;
        let dur = fs.saturating_mul(u64::from(config.duration_ms)) / 1000;
        let dur = dur.max(fade.saturating_mul(2));
        let inc0 = (f64::from(f1) / fs as f64 * TWO_48) as u64;
        let eps = (f64::from(f2) / f64::from(f1)).ln() / dur as f64;
        let eps_q31 = (eps * TWO_31) as i32;
        let to_u32 =
            |v: u64| u32::try_from(v).map_err(|_| MeasureError::Invalid("sweep too long".into()));
        Ok(LogSweep {
            rate_hz,
            f1_hz: f1,
            f2_hz: f2,
            cycle_samples: to_u32(dur)?,
            gap_samples: to_u32(gap)?,
            fade_samples: to_u32(fade)?,
            inc0_q48: inc0,
            eps_q31,
        })
    }

    /// The state at the start of every cycle (`synth_reset_cycle`).
    pub fn initial_state(&self) -> SweepState {
        SweepState {
            phase_q48: 0,
            inc_q48: self.inc0_q48,
        }
    }

    /// The Q32 phase of every sample of one cycle.
    pub fn phases_q32(&self) -> Vec<u32> {
        let mut st = self.initial_state();
        (0..self.cycle_samples)
            .map(|_| st.step(self.eps_q31))
            .collect()
    }

    /// The intra-cycle window (attack and release) at a position, which may be
    /// fractional (`window_gain_at`); zero outside the cycle.
    pub fn window_gain(&self, pos: f64) -> f64 {
        let len = f64::from(self.cycle_samples);
        let ramp = f64::from(self.fade_samples);
        if !(0.0..len).contains(&pos) {
            return 0.0;
        }
        if pos < ramp {
            return pos / ramp;
        }
        if len > ramp && pos >= len - ramp {
            return (len - pos) / ramp;
        }
        1.0
    }

    /// The start fade, which overlays the first cycle of a run only.
    pub fn start_fade_gain(&self, pos: f64) -> f64 {
        let ramp = f64::from(self.fade_samples);
        if pos <= 0.0 {
            0.0
        } else if pos >= ramp {
            1.0
        } else {
            pos / ramp
        }
    }

    /// One cycle at unit amplitude, at the device's rate. `first` adds the start
    /// fade of a run's first cycle.
    pub fn render_cycle(&self, first: bool) -> Vec<f64> {
        let mut st = self.initial_state();
        (0..self.cycle_samples)
            .map(|n| {
                let pos = f64::from(n);
                let mut g = self.window_gain(pos);
                if first {
                    g *= self.start_fade_gain(pos);
                }
                q32_phase_to_sample(st.step(self.eps_q31)) * g
            })
            .collect()
    }

    /// The increment's growth per sample, as the factor minus one (the
    /// quantised `eps`).
    pub fn growth_per_sample(&self) -> f64 {
        f64::from(self.eps_q31) / TWO_31
    }

    /// The sweep's actual start frequency (the quantised `inc0`), Hz.
    pub fn start_freq_hz(&self) -> f64 {
        self.inc0_q48 as f64 / TWO_48 * f64::from(self.rate_hz)
    }

    /// The sweep's actual frequency at the end of the cycle, Hz.
    pub fn end_freq_hz(&self) -> f64 {
        self.start_freq_hz()
            * (f64::from(self.cycle_samples) * self.growth_per_sample().ln_1p()).exp()
    }

    /// When the sweep passes `freq_hz`, seconds after the cycle's start (0
    /// below the start frequency, the cycle's length above the end).
    pub fn time_of_freq(&self, freq_hz: f64) -> f64 {
        let fs = f64::from(self.rate_hz);
        let len_s = f64::from(self.cycle_samples) / fs;
        let g = self.growth_per_sample().ln_1p();
        let f0 = self.start_freq_hz();
        if !(freq_hz.is_finite() && freq_hz > f0 && g > 0.0) {
            return 0.0;
        }
        ((freq_hz / f0).ln() / g / fs).min(len_s)
    }

    /// One cycle at unit amplitude as a microphone at `capture_rate_hz` hears
    /// it with an exact clock: the firmware's phase, interpolated between device
    /// samples (cubic Hermite on the unwrapped phase), with the same windows.
    /// Where the sweep passes `0.42 fs` of the capture it is faded out (a raised
    /// cosine to zero at `0.45 fs`), so the copy holds nothing the capture
    /// cannot.
    ///
    /// At the device's own rate this is [`LogSweep::render_cycle`].
    ///
    /// Errors: [`MeasureError::Invalid`] for a capture rate outside 8 kHz to
    /// 384 kHz.
    pub fn render_at_rate(
        &self,
        capture_rate_hz: u32,
        first: bool,
    ) -> Result<Vec<f64>, MeasureError> {
        if capture_rate_hz == self.rate_hz {
            return Ok(self.render_cycle(first));
        }
        if !(8_000..=384_000).contains(&capture_rate_hz) {
            return Err(MeasureError::Invalid(format!(
                "capture rate {capture_rate_hz} Hz"
            )));
        }
        // Unwrapped phase in turns at device samples 0..=len: sample n is
        // rendered at phase[n]; phase[len] extends the curve by one step.
        let len = self.cycle_samples as usize;
        let mut st = self.initial_state();
        let mut acc: u128 = 0;
        let mut phase = Vec::with_capacity(len.saturating_add(1));
        for _ in 0..=len {
            acc = acc.wrapping_add(u128::from(st.inc_q48));
            st.step(self.eps_q31);
            phase.push(acc as f64 / TWO_48);
        }
        let ratio = f64::from(capture_rate_hz) / f64::from(self.rate_hz);
        let fc = f64::from(capture_rate_hz);
        let (pass, stop) = (0.42 * fc, 0.45 * fc);
        let f0 = self.start_freq_hz();
        let g = self.growth_per_sample().ln_1p();
        let last = phase.len().saturating_sub(1);
        let at = |i: i64| -> f64 {
            let j = i.clamp(0, last as i64) as usize;
            phase.get(j).copied().unwrap_or(0.0)
        };
        let n_out = ((len.saturating_sub(1)) as f64 * ratio).floor() as usize;
        let out = (0..=n_out)
            .map(|m| {
                let t = m as f64 / ratio;
                let n = t.floor();
                let u = t - n;
                let i = n as i64;
                let (p0, p1) = (at(i), at(i.saturating_add(1)));
                // Catmull-Rom tangents; one-sided at the start.
                let pm = if i > 0 {
                    at(i.saturating_sub(1))
                } else {
                    2.0 * p0 - p1
                };
                let p2 = at(i.saturating_add(2));
                let (d0, d1) = ((p1 - pm) / 2.0, (p2 - p0) / 2.0);
                let (u2, u3) = (u * u, u * u * u);
                let ph = (2.0 * u3 - 3.0 * u2 + 1.0) * p0
                    + (u3 - 2.0 * u2 + u) * d0
                    + (-2.0 * u3 + 3.0 * u2) * p1
                    + (u3 - u2) * d1;
                let frac = ph - ph.floor();
                let mut gain = self.window_gain(t);
                if first {
                    gain *= self.start_fade_gain(t);
                }
                // The sweep's instantaneous frequency at device time t.
                let f = f0 * (t * g).exp();
                if f >= stop {
                    gain = 0.0;
                } else if f > pass {
                    let x = (f - pass) / (stop - pass);
                    gain *= 0.5 * (1.0 + (std::f64::consts::PI * x).cos());
                }
                (std::f64::consts::TAU * frac).sin() * gain
            })
            .collect();
        Ok(out)
    }
}

/// One cycle of a walked run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    /// Its index in the run, from 0.
    pub index: u32,
    /// The output that plays it.
    pub output: u8,
    /// The pass it belongs to, from 0.
    pub pass: u16,
    /// Its first sample, device samples after the generator's start.
    pub start: u64,
}

/// The generator's state as `SiggenStatus` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratorStatus {
    /// `active_channel`: the walk's current output, [`NO_CHANNEL`] when not
    /// running.
    pub active_channel: u8,
    /// `cycles_done`: completed cycles.
    pub cycles_done: u16,
}

/// `active_channel` when the generator is not running.
pub const NO_CHANNEL: u8 = 0xFF;

/// A walked run's schedule: which output plays when (`SIGGEN_FLAG_WALK`).
#[derive(Debug, Clone, PartialEq)]
pub struct WalkSchedule {
    /// The sweep every cycle plays.
    pub sweep: LogSweep,
    /// The outputs walked (`channel_mask`).
    pub channel_mask: u16,
    /// Passes (`repeat`).
    pub repeat: u16,
    cycles: Vec<Cycle>,
}

impl WalkSchedule {
    /// The schedule of `repeat` passes over the outputs in `channel_mask`.
    ///
    /// Errors: [`MeasureError::Invalid`] for an empty mask or `repeat` outside
    /// 1..=[`MAX_REPEAT`] (a measurement never stages an infinite run, S7).
    pub fn new(
        sweep: LogSweep,
        channel_mask: u16,
        repeat: u16,
    ) -> Result<WalkSchedule, MeasureError> {
        if channel_mask == 0 {
            return Err(MeasureError::Invalid("no output to walk".into()));
        }
        if !(1..=MAX_REPEAT).contains(&repeat) {
            return Err(MeasureError::Invalid(format!("{repeat} passes (1 to 8)")));
        }
        let outputs = mask_outputs(channel_mask);
        let period = u64::from(sweep.cycle_samples).saturating_add(u64::from(sweep.gap_samples));
        let mut cycles = Vec::with_capacity(outputs.len().saturating_mul(usize::from(repeat)));
        let mut index: u32 = 0;
        for pass in 0..repeat {
            for &output in &outputs {
                cycles.push(Cycle {
                    index,
                    output,
                    pass,
                    start: u64::from(index).saturating_mul(period),
                });
                index = index.saturating_add(1);
            }
        }
        Ok(WalkSchedule {
            sweep,
            channel_mask,
            repeat,
            cycles,
        })
    }

    /// Every cycle in order.
    pub fn cycles(&self) -> &[Cycle] {
        &self.cycles
    }

    /// The outputs walked, ascending (the walk order).
    pub fn outputs(&self) -> Vec<u8> {
        mask_outputs(self.channel_mask)
    }

    /// One cycle plus its gap, device samples.
    pub fn period_samples(&self) -> u64 {
        u64::from(self.sweep.cycle_samples).saturating_add(u64::from(self.sweep.gap_samples))
    }

    /// The run's length from the first sample to the last: no gap after the
    /// last cycle (`segment_finished` stops at once), device samples.
    pub fn total_samples(&self) -> u64 {
        let n = self.cycles.len() as u64;
        n.saturating_sub(1)
            .saturating_mul(self.period_samples())
            .saturating_add(u64::from(self.sweep.cycle_samples))
    }

    /// The capture frames a run needs at `capture_rate_hz`: the pre-roll, the
    /// run, and a post-roll of one gap after the last cycle (PLAN-P5 section
    /// 1.1 steps 2 and 10). The plan step compares this with
    /// [`crate::ir::MAX_CAPTURE_SAMPLES`].
    pub fn planned_capture_frames(&self, capture_rate_hz: u32, pre_roll_ms: u32) -> u64 {
        let dev = self
            .total_samples()
            .saturating_add(u64::from(self.sweep.gap_samples));
        let run = (dev as f64 * f64::from(capture_rate_hz) / f64::from(self.sweep.rate_hz)).ceil();
        let pre = u64::from(capture_rate_hz).saturating_mul(u64::from(pre_roll_ms)) / 1000;
        (run as u64).saturating_add(pre)
    }

    /// What `SIGGEN_GET_STATUS` says at a device sample (0 = the first sample
    /// of the run). Before the first sample this is the state just after the
    /// START (the first output, no cycle done); after the last cycle the
    /// generator is idle.
    pub fn status_at(&self, device_sample: f64) -> GeneratorStatus {
        let first = self.cycles.first().map_or(NO_CHANNEL, |c| c.output);
        let n = self.cycles.len();
        let done_all = u16::try_from(n).unwrap_or(u16::MAX);
        if !device_sample.is_finite() || device_sample < 0.0 {
            return GeneratorStatus {
                active_channel: first,
                cycles_done: 0,
            };
        }
        let period = self.period_samples() as f64;
        let len = f64::from(self.sweep.cycle_samples);
        let k = (device_sample / period).floor();
        let within = device_sample - k * period;
        if k >= n as f64 {
            return GeneratorStatus {
                active_channel: NO_CHANNEL,
                cycles_done: done_all,
            };
        }
        let k = k as usize;
        let out = |i: usize| self.cycles.get(i).map_or(NO_CHANNEL, |c| c.output);
        let done = |i: usize| u16::try_from(i).unwrap_or(u16::MAX);
        if within < len {
            GeneratorStatus {
                active_channel: out(k),
                cycles_done: done(k),
            }
        } else if k.saturating_add(1) >= n {
            GeneratorStatus {
                active_channel: NO_CHANNEL,
                cycles_done: done_all,
            }
        } else {
            // In the gap the walk has already advanced to the next output.
            GeneratorStatus {
                active_channel: out(k.saturating_add(1)),
                cycles_done: done(k.saturating_add(1)),
            }
        }
    }

    /// Whether a polled status agrees with the schedule anywhere between two
    /// device samples (the uncertainty of when the poll was answered). The
    /// session calls this for every poll during a run (PLAN-P5 section 1.1
    /// step 5); a `false` is a cycle-identity mismatch and aborts the run.
    pub fn status_consistent(
        &self,
        device_lo: f64,
        device_hi: f64,
        observed: GeneratorStatus,
    ) -> bool {
        if !(device_lo.is_finite() && device_hi.is_finite()) || device_hi < device_lo {
            return false;
        }
        // The status is piecewise constant; it changes only where a cycle
        // starts or ends. Check the range's ends and every change inside it.
        if self.status_at(device_lo) == observed || self.status_at(device_hi) == observed {
            return true;
        }
        let len = f64::from(self.sweep.cycle_samples);
        self.cycles.iter().any(|c| {
            let s = c.start as f64;
            [s, s + len]
                .iter()
                .any(|&b| b >= device_lo && b <= device_hi && self.status_at(b) == observed)
        })
    }
}

fn mask_outputs(mask: u16) -> Vec<u8> {
    (0u8..16)
        .filter(|&b| mask.checked_shr(u32::from(b)).unwrap_or(0) & 1 != 0)
        .collect()
}

// -- WP ATSYS -- the system analyser's timeline walk, and the static rules on
// a generator configuration for the hub and the core (PLAN-AT section 1.5).
impl WalkSchedule {
    /// A walk over arbitrary cycles (WP ATSYS): the system analyser's
    /// timeline of several segments, each cycle's `output` a key of the run
    /// and its `start` the domain sample after the run's first. Only
    /// [`crate::ir::WalkedAnalyser`] reads it: [`WalkSchedule::status_at`],
    /// [`WalkSchedule::period_samples`] and [`WalkSchedule::total_samples`]
    /// assume a regular walk and mean nothing for it.
    pub(crate) fn from_cycles(
        sweep: LogSweep,
        channel_mask: u16,
        repeat: u16,
        cycles: Vec<Cycle>,
    ) -> WalkSchedule {
        WalkSchedule {
            sweep,
            channel_mask,
            repeat,
            cycles,
        }
    }
}

/// The generator's signal type of a log sweep (`siggen.h`).
pub const SIGNAL_LOG_SWEEP: u8 = 4;
/// The generator's signal type of pink noise (`siggen.h`).
pub const SIGNAL_PINK: u8 = 3;
/// The longest pink noise a measurement plays, ms.
pub const PINK_MAX_MS: u32 = 60_000;
/// The highest level any measurement configuration may carry, peak dBFS: the
/// speakers' hard cap (S3; `dspi-core`'s `HARD_CAP_SPEAKERS_DB`). A session's
/// own ceiling is lower; this bound is what a hub can judge without knowing
/// the session.
pub const LEVEL_HARD_CAP_DB: f32 = -6.0;
/// The lowest level a configuration may carry, peak dBFS.
pub const LEVEL_MIN_DB: f32 = -120.0;

/// The static rules on a generator configuration (P5's S7 and S3, PLAN-AT
/// S-AT6), pure, so that the hub (`system.measure.stage`) and the core judge
/// a `SiggenConfig` alike: version 1; never `RAW`; a log sweep of
/// [`MIN_SWEEP_MS`] to [`MAX_SWEEP_MS`] with 1 to [`MAX_REPEAT`] passes and
/// finite frequencies in [`SWEEP_FREQ_RANGE_HZ`], `p1 < p2`, or pink noise of
/// 1 ms to [`PINK_MAX_MS`] on one output without the walk; never another type
/// (so never an infinite one); a level from [`LEVEL_MIN_DB`] to
/// [`LEVEL_HARD_CAP_DB`]; some output, the inverted ones among them; and no
/// output in `forbidden_mask` (an S/PDIF source's time-base slots, S-AT6).
///
/// Errors: [`MeasureError::Invalid`] naming the first rule broken.
pub fn check_config_bounds(
    config: &dspi_proto::packets::SiggenConfig,
    forbidden_mask: u16,
) -> Result<(), MeasureError> {
    use dspi_proto::packets::SiggenConfig as C;
    let bad = |m: &str| {
        Err(MeasureError::Invalid(format!(
            "refused generator configuration: {m}"
        )))
    };
    let c = config;
    if c.version != 1 {
        return bad("version is not 1");
    }
    if c.flags & C::FLAG_RAW != 0 {
        return bad("RAW bypasses the protective filters");
    }
    if c.flags & !(C::FLAG_RAW | C::FLAG_DECORR | C::FLAG_WALK) != 0 {
        return bad("undefined flags");
    }
    if !c.level_db.is_finite() || !(LEVEL_MIN_DB..=LEVEL_HARD_CAP_DB).contains(&c.level_db) {
        return bad("level outside -120 dBFS to the hard cap");
    }
    if c.channel_mask == 0 {
        return bad("no output");
    }
    if c.invert_mask & !c.channel_mask != 0 {
        return bad("an inverted output that does not play");
    }
    if c.channel_mask & forbidden_mask != 0 {
        return bad("an output that carries the time base to other units");
    }
    match c.signal_type {
        SIGNAL_LOG_SWEEP => {
            if !(MIN_SWEEP_MS..=MAX_SWEEP_MS).contains(&c.duration_ms)
                || !(1..=MAX_REPEAT).contains(&c.repeat_count)
            {
                return bad("a sweep of 1 to 15 s, 1 to 8 repeats");
            }
            let (lo, hi) = SWEEP_FREQ_RANGE_HZ;
            let in_range = |f: f32| f.is_finite() && (lo..=hi).contains(&f);
            if !in_range(c.p1) || !in_range(c.p2) || c.p1 >= c.p2 {
                return bad("sweep frequencies outside 1 Hz to 40 kHz or not rising");
            }
        }
        SIGNAL_PINK => {
            if !(1..=PINK_MAX_MS).contains(&c.duration_ms) {
                return bad("pink noise of 1 ms to 60 s");
            }
            if c.flags & C::FLAG_WALK != 0 || c.channel_mask.count_ones() != 1 {
                return bad("pink noise on one output");
            }
        }
        _ => return bad("only a log sweep or pink noise"),
    }
    Ok(())
}

#[cfg(test)]
mod tests_wp_p5time;
