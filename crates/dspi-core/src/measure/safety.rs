//! The safety engine of a session (WP P5SES, PLAN-P5 section 1.2): the level
//! ramp (S3), the per-block checks in the push path (S4, S5), the SPL meter,
//! and the judgement of what the notification path saw (S5). Pure: the
//! session feeds it numbers and acts on what it answers.

use dspi_measure::hearing::{BAND_MAX_HZ, BAND_MIN_HZ, Listener, NO_SNR_DB, Played, hearing_band};
use dspi_measure::stimulus::{LogSweep, SweepConfig};
use serde::Serialize;

use super::plan::Plan;
use super::runs::RAMP_SWEEP_MS;
use super::{MeasureIssue, MeasureRunKind, TapEvent};

/// A capture sample at or above this aborts, dBFS (S4).
pub(crate) const CAPTURE_CLIP_DBFS: f32 = -1.0;
/// A measured output's meter peak at or above this aborts, dBFS (S4).
pub(crate) const OUTPUT_PEAK_DBFS: f32 = -0.5;
/// The ramp's step, dB (S3).
pub(crate) const RAMP_STEP_DB: f32 = 6.0;
/// The in-band SNR at which a ramp stops, dB, in 100 Hz to 10 kHz (S3,
/// [`step_hearing`]).
pub(crate) const RAMP_SNR_DB: f64 = 35.0;
/// The signal is "above the noise" from this in-band SNR on, dB; a ramp never
/// ends on a step heard less (S3, WP P5RAMPFIX).
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

/// A mean square in dB (full scale sine 0 dB, the crate's convention).
pub(crate) fn db(power: f64) -> f64 {
    10.0 * power.max(1e-30).log10() + dspi_measure::types::FULL_SCALE_SINE_DB
}

// ── Hearing (WP P5RAMPFIX) ─────────────────────────────────────────────

/// The listener of a ramp's steps: from the pre-roll (the generator idle)
/// and what the steps play, judged in the stimulus' band (100 Hz to 10 kHz
/// within the sweep's) and never broadband, so low-frequency rumble is never
/// taken for signal ([`dspi_measure::hearing`]). A sweep's steps are judged
/// by its matched filter (the 1 s ramp sweep, rendered at the capture rate
/// from the device rate), pink noise by its band levels.
///
/// Errors: the reason, as text; the session then hears nothing (fail
/// closed).
pub(crate) fn ramp_listener(
    plan: &Plan,
    device_rate_hz: u32,
    pre_roll: &[f32],
) -> std::result::Result<Listener, String> {
    let rate = plan.capture_rate_hz;
    if plan.kind == MeasureRunKind::Pink {
        let band = hearing_band(BAND_MIN_HZ, BAND_MAX_HZ, rate);
        return Listener::new(pre_roll, rate, band, Played::Pink).map_err(|e| e.to_string());
    }
    let cfg = SweepConfig {
        duration_ms: RAMP_SWEEP_MS,
        ..plan.sweep
    };
    let sweep = LogSweep::new(device_rate_hz, &cfg).map_err(|e| e.to_string())?;
    let template = sweep
        .render_at_rate(rate, true)
        .map_err(|e| e.to_string())?;
    let band = hearing_band(sweep.start_freq_hz(), sweep.end_freq_hz(), rate);
    Listener::new(pre_roll, rate, band, Played::Sweep(&template)).map_err(|e| e.to_string())
}

/// A step's in-band SNR and in-band level, dB; [`NO_SNR_DB`] (and no level)
/// without a listener or when the step cannot be judged (fail closed: not
/// heard).
pub(crate) fn step_hearing(listener: Option<&Listener>, step: &[f32]) -> (f64, Option<f64>) {
    match listener.map(|l| l.hear(step)) {
        Some(Ok(h)) => (h.snr_db, Some(h.level_db)),
        Some(Err(e)) => {
            log::debug!("ramp step not judged: {e}");
            (NO_SNR_DB, None)
        }
        None => (NO_SNR_DB, None),
    }
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
    /// The microphone's in-band SNR, dB ([`step_hearing`]).
    pub(crate) snr_db: f64,
    /// The microphone's in-band level, dB on the listener's own scale (only
    /// differences between steps mean anything); `None` if not judged.
    pub(crate) in_band_db: Option<f64>,
    /// The capture clipped.
    pub(crate) clipped: bool,
    /// The loudest 125 ms, dB SPL, with a calibrated microphone.
    pub(crate) spl_db: Option<f64>,
}

/// One judged ramp step, as the document and the notices report it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StepRecord {
    pub(crate) output: u8,
    pub(crate) level_db: f32,
    pub(crate) snr_db: f64,
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
    /// The setup's start level only: the one step must be heard as well.
    pub(crate) start_level_only: bool,
    /// `(level, snr_db, in_band_db)` of each step so far.
    pub(crate) history: Vec<(f32, f64, Option<f64>)>,
}

impl Ramp {
    pub(crate) fn new(
        output: u8,
        start_db: f32,
        ceiling_db: f32,
        spl_cap_db: Option<f32>,
        start_level_only: bool,
    ) -> Ramp {
        Ramp {
            output,
            start_db: start_db.min(ceiling_db),
            ceiling_db,
            spl_cap_db,
            start_level_only,
            history: Vec::new(),
        }
    }

    /// The first step's level.
    pub(crate) fn first(&self) -> f32 {
        self.start_db
    }

    /// The steps so far, for a notice: "-60 dBFS 1.2 dB, -54 dBFS 3.0 dB".
    fn steps_text(&self) -> String {
        self.history
            .iter()
            .map(|(l, s, _)| format!("{l} dBFS {s:.1} dB"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Not heard where it must be (fail closed): the ramp asks.
    fn not_heard(&self) -> RampNext {
        let n = self.output.saturating_add(1);
        let steps = self.steps_text();
        RampNext::Ask(if self.start_level_only {
            MeasureIssue::new(
                "measure.start_level_quiet",
                Some(self.output),
                format!(
                    "the microphone does not hear output {n} at the start level of {} dBFS \
                     (in-band SNR {steps}; at least {HEARD_SNR_DB} dB needed): measure with \
                     the ramp, or check the microphone and the wiring",
                    self.start_db
                ),
            )
        } else {
            MeasureIssue::new(
                "measure.mic_not_hearing",
                Some(self.output),
                format!(
                    "the microphone does not hear output {n} (in-band SNR by step: {steps}; at \
                     least {HEARD_SNR_DB} dB needed): check the microphone and the wiring"
                ),
            )
        })
    }

    /// Judge a step and say what comes next. Every ending but a clip's
    /// needs the step heard (in-band SNR at least [`HEARD_SNR_DB`]): a run
    /// is never started at a level the microphone cannot hear (S3, WP
    /// P5RAMPFIX).
    pub(crate) fn judge(&mut self, e: StepEval) -> RampNext {
        let n = self.output.saturating_add(1);
        let level = e.level_db;
        let snr = if e.snr_db.is_finite() {
            e.snr_db
        } else {
            NO_SNR_DB
        };
        let quieter = |db: f32| (level - db).max(-120.0);
        if e.clipped {
            return RampNext::Done(quieter(CLIP_BACKOFF_DB));
        }
        let prev = self.history.last().copied();
        self.history.push((level, snr, e.in_band_db));
        let heard = snr >= HEARD_SNR_DB;
        if let (Some(cap), Some(spl)) = (self.spl_cap_db, e.spl_db) {
            // Past the cap the push path stopped the step: one step down.
            if spl >= f64::from(cap) {
                return RampNext::Done(quieter(RAMP_STEP_DB));
            }
            if spl + f64::from(RAMP_STEP_DB) > f64::from(cap) {
                return if heard {
                    RampNext::Done(level)
                } else {
                    self.not_heard()
                };
            }
        }
        if snr >= RAMP_SNR_DB {
            return RampNext::Done(level);
        }
        // Once heard, a step must raise the in-band level by half of it (the
        // level, not the SNR: a sweep's SNR may stop rising at its own
        // sidelobes while its level still follows).
        if let Some((prev_level, prev_snr, prev_in_band)) = prev
            && prev_snr >= HEARD_SNR_DB
        {
            let need = FOLLOW_SHARE * f64::from(level - prev_level);
            let rise = match (e.in_band_db, prev_in_band) {
                (Some(now), Some(before)) => now - before,
                _ => snr - prev_snr,
            };
            if rise < need {
                return RampNext::Ask(MeasureIssue::new(
                    "measure.mic_not_following",
                    Some(self.output),
                    format!(
                        "the microphone does not follow output {n} (in-band SNR by step: {}): \
                         check the microphone and the wiring",
                        self.steps_text()
                    ),
                ));
            }
        }
        if level >= self.start_db + HEAR_BY_DB && !heard {
            return self.not_heard();
        }
        if level >= self.ceiling_db {
            return if heard {
                RampNext::Done(self.ceiling_db)
            } else {
                self.not_heard()
            };
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
