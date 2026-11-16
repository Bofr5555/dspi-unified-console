//! The safety engine of a session (WP P5SES, PLAN-P5 section 1.2): the level
//! ramp (S3), the per-block checks in the push path (S4, S5), the SPL meter,
//! and the judgement of what the notification path saw (S5). Pure: the
//! session feeds it numbers and acts on what it answers.

use super::{MeasureIssue, TapEvent};

/// A capture sample at or above this aborts, dBFS (S4).
pub(crate) const CAPTURE_CLIP_DBFS: f32 = -1.0;
/// A measured output's meter peak at or above this aborts, dBFS (S4).
pub(crate) const OUTPUT_PEAK_DBFS: f32 = -0.5;
/// The ramp's step, dB (S3).
pub(crate) const RAMP_STEP_DB: f32 = 6.0;
/// The SNR at which a ramp stops, dB, in 100 Hz to 10 kHz (S3).
pub(crate) const RAMP_SNR_DB: f64 = 35.0;
/// The signal is "above the noise" from this SNR on, dB.
pub(crate) const HEARD_SNR_DB: f64 = 6.0;
/// Once heard, a step must raise the microphone's level by at least this
/// share of the step (3 dB of 6, S3).
pub(crate) const FOLLOW_SHARE: f64 = 0.5;
/// By the start level plus this the signal must be above the noise, dB (S3).
pub(crate) const HEAR_BY_DB: f32 = 18.0;
/// A ramp step that clips ends the ramp this far below it, dB (S4).
pub(crate) const CLIP_BACKOFF_DB: f32 = 12.0;
/// The SPL meter's window, ms (Leq, S3).
pub(crate) const SPL_WINDOW_MS: u32 = 125;
/// A limiter whose gain reduction passes this acts, dB (S13).
pub(crate) const LIMITER_ACTS_DB: f32 = 0.1;

/// Linear full scale of a dBFS peak.
pub(crate) fn linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The capture's clip threshold, linear.
pub(crate) fn clip_linear() -> f32 {
    linear(CAPTURE_CLIP_DBFS)
}

// ── Band level (100 Hz to 10 kHz) ──────────────────────────────────────

/// A biquad, direct form I, `f64`.
#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    /// RBJ's second-order high-pass (`high`) or low-pass at `f0`, Q 0.707.
    fn pass(f0: f64, fs: f64, high: bool) -> Biquad {
        let w = std::f64::consts::TAU * (f0 / fs).clamp(1e-6, 0.49);
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high {
            ((1.0 + cos) / 2.0, -(1.0 + cos))
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos)
        };
        Biquad {
            b: [b0 / a0, b1 / a0, b0 / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            ..Biquad::default()
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        let y = b0 * x + b1 * self.x[0] + b2 * self.x[1] - a1 * self.y[0] - a2 * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        if y.is_finite() { y } else { 0.0 }
    }
}

/// The capture's power in 100 Hz to 10 kHz per window of `window_ms`, each as
/// a mean square (full scale ±1). Non-finite samples count as 0.
pub(crate) fn band_powers(samples: &[f32], rate_hz: u32, window_ms: u32) -> Vec<f64> {
    let fs = f64::from(rate_hz.max(1));
    let mut hp = Biquad::pass(100.0, fs, true);
    let mut lp = Biquad::pass(10_000.0, fs, false);
    let n = usize::try_from(u64::from(rate_hz).saturating_mul(u64::from(window_ms)) / 1000)
        .unwrap_or(usize::MAX)
        .max(1);
    let mut out = Vec::new();
    let (mut sum, mut count) = (0.0f64, 0usize);
    for s in samples {
        let x = if s.is_finite() { f64::from(*s) } else { 0.0 };
        let y = lp.run(hp.run(x));
        sum += y * y;
        count = count.saturating_add(1);
        if count >= n {
            out.push(sum / n as f64);
            sum = 0.0;
            count = 0;
        }
    }
    if count > n / 2 {
        out.push(sum / count as f64);
    }
    out
}

/// A mean square in dB (full scale sine 0 dB, the crate's convention).
pub(crate) fn db(power: f64) -> f64 {
    10.0 * power.max(1e-30).log10() + dspi_measure::types::FULL_SCALE_SINE_DB
}

/// The noise floor of a pre-roll, dB (its windows' median band power).
pub(crate) fn noise_db(samples: &[f32], rate_hz: u32) -> f64 {
    let mut p = band_powers(samples, rate_hz, SPL_WINDOW_MS);
    // The filters settle in the first window.
    if p.len() > 2 {
        p.remove(0);
    }
    if p.is_empty() {
        return db(0.0);
    }
    p.sort_by(f64::total_cmp);
    db(p.get(p.len() / 2).copied().unwrap_or(0.0))
}

/// A ramp step's signal level, dB: the mean of its louder half of windows (a
/// sweep spends a third of its time below 100 Hz).
pub(crate) fn signal_db(samples: &[f32], rate_hz: u32) -> f64 {
    let mut p = band_powers(samples, rate_hz, SPL_WINDOW_MS);
    if p.is_empty() {
        return db(0.0);
    }
    p.sort_by(|a, b| b.total_cmp(a));
    let half = p.len().div_ceil(2);
    let top: f64 = p.iter().take(half).sum();
    db(top / half as f64)
}

// ── SPL (Leq over 125 ms, broadband) ───────────────────────────────────

/// The SPL meter of the push path: O(block), a running mean square over
/// consecutive windows of [`SPL_WINDOW_MS`].
#[derive(Debug, Clone, Default)]
pub(crate) struct SplMeter {
    window: usize,
    sum: f64,
    count: usize,
    /// The loudest window so far, dB (full scale sine 0 dB).
    pub(crate) max_db: Option<f64>,
}

impl SplMeter {
    pub(crate) fn new(rate_hz: u32) -> SplMeter {
        let window =
            usize::try_from(u64::from(rate_hz).saturating_mul(u64::from(SPL_WINDOW_MS)) / 1000)
                .unwrap_or(usize::MAX)
                .max(1);
        SplMeter {
            window,
            ..SplMeter::default()
        }
    }

    /// Takes a block; the loudest window it completed, dBFS.
    pub(crate) fn push(&mut self, samples: &[f32]) -> Option<f64> {
        let mut loudest: Option<f64> = None;
        for s in samples {
            let x = if s.is_finite() { f64::from(*s) } else { 0.0 };
            self.sum += x * x;
            self.count = self.count.saturating_add(1);
            if self.count >= self.window {
                let level = db(self.sum / self.window as f64);
                loudest = Some(loudest.map_or(level, |l: f64| l.max(level)));
                self.sum = 0.0;
                self.count = 0;
            }
        }
        if let Some(l) = loudest {
            self.max_db = Some(self.max_db.map_or(l, |m| m.max(l)));
        }
        loudest
    }

    /// Forget the loudest window (a new ramp step).
    pub(crate) fn reset_max(&mut self) {
        self.max_db = None;
    }
}

// ── The ramp (S3) ──────────────────────────────────────────────────────

/// What one ramp step measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StepEval {
    /// The generator's level of the step, peak dBFS.
    pub(crate) level_db: f32,
    /// The microphone's level in 100 Hz to 10 kHz, dB.
    pub(crate) signal_db: f64,
    /// The pre-roll's, dB.
    pub(crate) noise_db: f64,
    /// The capture clipped.
    pub(crate) clipped: bool,
    /// The loudest 125 ms, dB SPL, with a calibrated microphone.
    pub(crate) spl_db: Option<f64>,
}

/// What the ramp does next.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RampNext {
    /// Play the next step at this level.
    Step(f32),
    /// The output's level is this, peak dBFS.
    Done(f32),
    /// Stop and ask the user (fail closed).
    Ask(MeasureIssue),
}

/// One output's ramp: from `start`, +6 dB a step, up to `ceiling` (S3).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ramp {
    pub(crate) output: u8,
    pub(crate) start_db: f32,
    pub(crate) ceiling_db: f32,
    pub(crate) spl_cap_db: Option<f32>,
    /// `(level, signal_db)` of each step so far.
    pub(crate) history: Vec<(f32, f64)>,
}

impl Ramp {
    pub(crate) fn new(output: u8, start_db: f32, ceiling_db: f32, spl_cap_db: Option<f32>) -> Ramp {
        Ramp {
            output,
            start_db: start_db.min(ceiling_db),
            ceiling_db,
            spl_cap_db,
            history: Vec::new(),
        }
    }

    /// The first step's level.
    pub(crate) fn first(&self) -> f32 {
        self.start_db
    }

    /// Judge a step and say what comes next.
    pub(crate) fn judge(&mut self, e: StepEval) -> RampNext {
        let n = self.output.saturating_add(1);
        let level = e.level_db;
        let quieter = |db: f32| (level - db).max(-120.0);
        if e.clipped {
            return RampNext::Done(quieter(CLIP_BACKOFF_DB));
        }
        if let (Some(cap), Some(spl)) = (self.spl_cap_db, e.spl_db) {
            if spl >= f64::from(cap) {
                return RampNext::Done(quieter(RAMP_STEP_DB));
            }
            if spl + f64::from(RAMP_STEP_DB) > f64::from(cap) {
                self.history.push((level, e.signal_db));
                return RampNext::Done(level);
            }
        }
        let snr = e.signal_db - e.noise_db;
        let prev = self.history.last().copied();
        self.history.push((level, e.signal_db));
        if snr >= RAMP_SNR_DB {
            return RampNext::Done(level);
        }
        if let Some((prev_level, prev_db)) = prev
            && prev_db - e.noise_db >= HEARD_SNR_DB
        {
            let need = FOLLOW_SHARE * f64::from(level - prev_level);
            if e.signal_db - prev_db < need {
                return RampNext::Ask(MeasureIssue::new(
                    "measure.mic_not_following",
                    Some(self.output),
                    format!(
                        "the microphone does not follow output {n}: check the microphone and \
                         the wiring"
                    ),
                ));
            }
        }
        if level >= self.start_db + HEAR_BY_DB && snr < HEARD_SNR_DB {
            return RampNext::Ask(MeasureIssue::new(
                "measure.mic_not_hearing",
                Some(self.output),
                format!(
                    "the microphone does not hear output {n}: check the microphone and the wiring"
                ),
            ));
        }
        if level >= self.ceiling_db {
            return RampNext::Done(self.ceiling_db);
        }
        RampNext::Step((level + RAMP_STEP_DB).min(self.ceiling_db))
    }
}

// ── What the notification path saw (S5) ────────────────────────────────

/// `SIGGEN_STATE_IDLE` and the stop reasons (`siggen.h`).
pub(crate) const SG_IDLE: u8 = 0;
pub(crate) const STOP_HOST: u8 = 1;
pub(crate) const STOP_COMPLETED: u8 = 2;

/// What the session knows when it judges a tapped event.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GenView {
    /// The generator plays for us now (a START acknowledged, no end seen).
    pub(crate) playing: bool,
    /// We asked for STOP_NOW and await its echo.
    pub(crate) stopping: bool,
    /// Our STARTs whose start event has not come yet.
    pub(crate) starts_pending: u8,
}

/// The judgement of one tapped event: `Ok(true)` the generator ended as we
/// expected, `Ok(false)` nothing to do, `Err` abort with this issue.
pub(crate) fn judge_event(e: TapEvent, g: &mut GenView) -> Result<bool, MeasureIssue> {
    let abort = |key: &str, msg: &str| Err(MeasureIssue::new(key, None, msg));
    match e {
        TapEvent::Volume => abort(
            "measure.abort.volume",
            "the volume changed during the measurement",
        ),
        TapEvent::ForeignWrite => abort(
            "measure.abort.foreign",
            "another console or the DSPi itself changed a setting",
        ),
        TapEvent::PresetLoad => abort("measure.abort.preset", "a preset was loaded"),
        TapEvent::InputFormat => abort("measure.abort.input_format", "the input's format changed"),
        TapEvent::InputStream => abort(
            "measure.abort.input_stream",
            "an input stream started or stopped",
        ),
        TapEvent::Clip => abort("measure.abort.clip", "the DSPi reported a clip"),
        TapEvent::Lost => abort(
            "measure.abort.foreign",
            "notifications were lost: a change may have gone unseen",
        ),
        TapEvent::Siggen { state, reason } => {
            if state != SG_IDLE {
                // A start: ours is announced once per START; any other is
                // another console's.
                if g.starts_pending > 0 {
                    g.starts_pending = g.starts_pending.saturating_sub(1);
                    return Ok(false);
                }
                return if g.playing {
                    Ok(false)
                } else {
                    abort(
                        "measure.abort.generator",
                        "another console started the generator",
                    )
                };
            }
            match reason {
                STOP_COMPLETED if g.playing => {
                    g.playing = false;
                    Ok(true)
                }
                // It ended by itself just before our STOP_NOW reached it
                // (a ramp step the push path stopped at its very end): an
                // idle generator sends no second event (review P5-B).
                STOP_COMPLETED if g.stopping => {
                    g.stopping = false;
                    Ok(true)
                }
                STOP_HOST if g.stopping => {
                    g.stopping = false;
                    Ok(true)
                }
                // Idle and stopped already: an echo of something past.
                _ if !g.playing && !g.stopping => Ok(false),
                _ => abort(
                    "measure.abort.generator",
                    "the generator was stopped by someone else (another console or a preset load)",
                ),
            }
        }
    }
}
