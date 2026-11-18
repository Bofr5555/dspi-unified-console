//! The run kinds (WP P5SES, review RC11): what the generator is told to play
//! for a ramp step and for each run, the one check every configuration passes
//! before it is sent (S7: never `RAW`, never infinite, pink at most 60 s, the
//! level within the cap), and how a run's capture becomes responses and a
//! `measurement` document.

use dspi_measure::cal::apply_correction;
use dspi_measure::doc::{
    CaptureInfo, Curve, DocKind, Measurement, MeasurementDoc, Meta, MicRef, OutputMeasurement,
    RunKind, SCHEMA_VERSION, Stimulus,
};
use dspi_measure::ir::{FIT_WINDOW_CYCLES, OutputIr, WalkedAnalysis, fdw_response};
use dspi_measure::response::{TrustRule, from_grid_values, resample, trust_mask};
use dspi_measure::types::{LogGrid, Response};
use serde_json::Map;

use super::plan::{HARD_CAP_SPEAKERS_DB, PINK_MAX_MS, Plan};
use super::preflight::{SIG_LOG_SWEEP, SIG_PINK};
use super::{Chains, MeasureRunKind};
use crate::consts::{SIGGEN_FLAG_RAW, SIGGEN_FLAG_WALK};
use crate::records::SiggenConfig;
use crate::{DspiError, Result};

/// A ramp step's sweep, ms ("short walked log sweeps, about 1 s", S3).
pub(crate) const RAMP_SWEEP_MS: u32 = 1_000;
/// A ramp step's pink noise, ms.
pub(crate) const RAMP_PINK_MS: u32 = 1_000;

/// The one check before any configuration is sent (S7): a log sweep or pink
/// noise only, never `RAW`, a sweep 1 to 15 s with 1 to 8 repeats, pink noise
/// 1 to 60 s, a level between -120 dBFS and the speakers' hard cap.
///
/// Errors: [`DspiError::InvalidArgument`], nothing sent.
pub(crate) fn check_config(c: &SiggenConfig) -> Result<()> {
    let bad = |m: &str| {
        Err(DspiError::InvalidArgument {
            msg: format!("refused generator configuration: {m}"),
        })
    };
    if c.flags & SIGGEN_FLAG_RAW != 0 {
        return bad("RAW bypasses the protective filters");
    }
    if !c.level_db.is_finite() || !(-120.0..=HARD_CAP_SPEAKERS_DB).contains(&c.level_db) {
        return bad("level outside -120 dBFS to the hard cap");
    }
    if c.channel_mask == 0 {
        return bad("no output");
    }
    match c.signal_type {
        SIG_LOG_SWEEP => {
            if !(1_000..=15_000).contains(&c.duration_ms) || !(1..=8).contains(&c.repeat) {
                return bad("a sweep of 1 to 15 s, 1 to 8 repeats");
            }
        }
        SIG_PINK => {
            if !(1..=PINK_MAX_MS).contains(&c.duration_ms) {
                return bad("pink noise of 1 ms to 60 s");
            }
            if c.flags & SIGGEN_FLAG_WALK != 0 || c.channel_mask.count_ones() != 1 {
                return bad("pink noise on one output");
            }
        }
        _ => return bad("only a log sweep or pink noise"),
    }
    Ok(())
}

fn sweep(
    plan: &Plan,
    mask: u16,
    invert: u16,
    walk: bool,
    level: f32,
    ms: u32,
    repeat: u16,
) -> SiggenConfig {
    SiggenConfig {
        signal_type: SIG_LOG_SWEEP,
        channel_mask: mask,
        invert_mask: invert & mask,
        flags: if walk { SIGGEN_FLAG_WALK } else { 0 },
        level_db: level,
        duration_ms: ms,
        repeat,
        gap_ms: plan.sweep.gap_ms,
        p: vec![plan.sweep.f1_hz, plan.sweep.f2_hz, 0.0, 0.0],
    }
}

fn pink(output: u8, level: f32, ms: u32) -> SiggenConfig {
    SiggenConfig {
        signal_type: SIG_PINK,
        channel_mask: 1u16.checked_shl(u32::from(output)).unwrap_or(0),
        invert_mask: 0,
        flags: 0,
        level_db: level,
        duration_ms: ms,
        repeat: 1,
        gap_ms: 0,
        p: vec![0.0; 4],
    }
}

/// A ramp step of `output` at `level`: the run's own stimulus, short (S3).
pub(crate) fn ramp_config(plan: &Plan, output: u8, level: f32) -> SiggenConfig {
    if plan.kind == MeasureRunKind::Pink {
        return pink(output, level, RAMP_PINK_MS);
    }
    let mask = 1u16.checked_shl(u32::from(output)).unwrap_or(0);
    sweep(plan, mask, 0, false, level, RAMP_SWEEP_MS, 1)
}

/// The run's configuration at `level`.
pub(crate) fn run_config(plan: &Plan, run: &super::plan::RunSpec, level: f32) -> SiggenConfig {
    let mask = run.mask();
    let ms = plan.sweep.duration_ms;
    match plan.kind {
        // `WALK` on one output changes nothing; it keeps the status's walked
        // output for the cycle-identity check.
        MeasureRunKind::Walked => sweep(plan, mask, 0, true, level, ms, plan.passes),
        MeasureRunKind::Summed => sweep(plan, mask, run.invert_mask, false, level, ms, 1),
        MeasureRunKind::LiveSweep => sweep(plan, mask, 0, false, level, ms, 1),
        MeasureRunKind::Pink => pink(
            run.outputs.first().copied().unwrap_or(0),
            level,
            plan.pink_ms,
        ),
    }
}

/// A summed run plays every output at once: its level is the ways' minimum
/// less the sum's possible rise, `20·log10(n)`.
pub(crate) fn run_level(plan: &Plan, levels: &[f32], outputs: usize) -> f32 {
    let min = levels.iter().copied().fold(plan.ceiling_db, f32::min);
    let lvl = if plan.kind == MeasureRunKind::Summed {
        min - 20.0 * (outputs.max(1) as f32).log10()
    } else {
        min
    };
    lvl.clamp(-120.0, plan.ceiling_db)
}

// ── Responses ──────────────────────────────────────────────────────────

/// One output's measured response of a run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputResult {
    pub(crate) output: u8,
    pub(crate) response: Response,
    /// The onset, ms, on the run's timing origin.
    pub(crate) arrival_ms: f64,
}

/// One run's result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunResult {
    pub(crate) position: u8,
    pub(crate) outputs: Vec<OutputResult>,
    pub(crate) level_db: f32,
    pub(crate) timing_reliable: bool,
    pub(crate) max_residual_us: Option<f64>,
    pub(crate) ratio_ppm: f64,
    pub(crate) timing: Option<super::MeasureTiming>,
    /// The ramp's steps before this run, as judged (WP P5RAMPFIX).
    pub(crate) ramp_steps: Vec<super::safety::StepRecord>,
}

fn mic_on(grid: &LogGrid, plan: &Plan) -> Option<Response> {
    let m = plan.mic.as_ref()?;
    if m.correction.grid == *grid {
        Some(m.correction.clone())
    } else {
        resample(&m.correction, grid).ok()
    }
}

/// An output's response from its impulse response (review P5-A, the worked
/// example `dspi-measure/tests/common/mod.rs`): a frequency-dependent window
/// about the onset, onto the standard grid with the arrival as the phase
/// reference, untrusted outside the sweep's band, the trust mask from the
/// noise floor and the repeats' spread, the microphone's correction taken
/// out, then referred to a 0 dBFS stimulus (and to dB SPL with a known
/// offset).
pub(crate) fn output_response(
    o: &OutputIr,
    band_hz: (f64, f64),
    level_db: f32,
    plan: &Plan,
) -> Result<OutputResult> {
    let e =
        |m: dspi_measure::types::MeasureError| DspiError::InvalidArgument { msg: m.to_string() };
    let grid = LogGrid::STANDARD;
    let freqs = grid.freqs().map_err(e)?;
    let centre = o.onset_index;
    let w = fdw_response(&o.ir, centre, &freqs, FIT_WINDOW_CYCLES).map_err(e)?;
    let values: Vec<[f64; 2]> = w.values.iter().map(|c| [c.re, c.im]).collect();
    let rate = f64::from(o.ir.rate_hz.max(1));
    let delay_s = (centre - o.ir.zero_index as f64) / rate;
    let mut resp = from_grid_values(&values, &grid, delay_s).map_err(e)?;
    let noise = o.noise_db.as_ref().map(|n| Response {
        grid,
        mag_db: n
            .iter()
            .zip(&w.sum_w2)
            .map(|(n, s)| n + 10.0 * s.max(1e-30).log10())
            .collect(),
        phase_deg: None,
        valid: None,
        spl: false,
    });
    if let Some(v) = resp.valid.as_mut() {
        for (ok, f) in v.iter_mut().zip(&freqs) {
            *ok = *ok && *f >= band_hz.0 && *f <= band_hz.1;
        }
    }
    let spread = (o.repeats > 1).then_some(o.spread_db.as_slice());
    resp.valid = Some(trust_mask(&resp, noise.as_ref(), spread, &TrustRule::DEFAULT).map_err(e)?);
    let mut resp = match mic_on(&grid, plan) {
        Some(m) => apply_correction(&resp, &m).map_err(e)?,
        None => resp,
    };
    let offset = plan.mic.as_ref().and_then(|m| m.spl_offset_db);
    for v in &mut resp.mag_db {
        *v = *v - f64::from(level_db) + offset.unwrap_or(0.0);
    }
    resp.spl = offset.is_some();
    Ok(OutputResult {
        output: o.output,
        response: resp,
        arrival_ms: (o.onset_index - o.ir.zero_index as f64) * 1000.0 / rate,
    })
}

/// The responses of an analysed walked (or single-sweep) run. A summed run's
/// one response stands for the sum and is filed under its first output.
pub(crate) fn walked_result(
    a: &WalkedAnalysis,
    position: u8,
    level_db: f32,
    plan: &Plan,
) -> Result<RunResult> {
    let outputs = a
        .outputs
        .iter()
        .map(|o| output_response(o, a.band_hz, level_db, plan))
        .collect::<Result<Vec<_>>>()?;
    let reliable = a.quality.reliable && plan.kind == MeasureRunKind::Walked;
    let fin = |v: f64| if v.is_finite() { v } else { 0.0 };
    let timing = super::MeasureTiming {
        reliable,
        ratio_ppm: fin(a.clock.ratio_ppm),
        ratio_fitted: a.clock.fitted,
        max_residual_us: fin(a.quality.max_residual_us),
        residual_bound_us: fin(a.quality.residual_bound_us),
        max_phase_spread_deg: fin(a.quality.max_phase_spread_deg),
        passes: a.quality.passes,
        reasons: a.quality.reasons.clone(),
        cycle_outputs: a.cycles.iter().map(|c| c.output).collect(),
        cycle_residuals_us: a.cycles.iter().map(|c| fin(c.residual_us)).collect(),
    };
    Ok(RunResult {
        timing: Some(timing),
        position,
        outputs,
        level_db,
        timing_reliable: reliable,
        max_residual_us: a
            .quality
            .max_residual_us
            .is_finite()
            .then_some(a.quality.max_residual_us),
        ratio_ppm: if a.clock.ratio_ppm.is_finite() {
            a.clock.ratio_ppm
        } else {
            0.0
        },
        ramp_steps: Vec::new(),
    })
}

/// A pink run's spectrum as a response on the standard grid, referred to a
/// 0 dBFS stimulus.
pub(crate) fn pink_result(spectrum: &Response, output: u8, level_db: f32) -> Result<RunResult> {
    let e =
        |m: dspi_measure::types::MeasureError| DspiError::InvalidArgument { msg: m.to_string() };
    let mut r = resample(spectrum, &LogGrid::STANDARD).map_err(e)?;
    for v in &mut r.mag_db {
        *v -= f64::from(level_db);
    }
    Ok(RunResult {
        timing: None,
        position: 0,
        outputs: vec![OutputResult {
            output,
            response: r,
            arrival_ms: 0.0,
        }],
        level_db,
        timing_reliable: false,
        max_residual_us: None,
        ratio_ppm: 0.0,
        ramp_steps: Vec::new(),
    })
}

// ── The document ───────────────────────────────────────────────────────

/// What the document needs beside the runs.
pub(crate) struct DocContext<'a> {
    pub(crate) serial: &'a str,
    pub(crate) platform: &'a str,
    pub(crate) device_rate_hz: u32,
    pub(crate) chains: &'a Chains,
    pub(crate) saved_ms: i64,
    pub(crate) saved_utc: String,
}

/// The `measurement` document of a finished session: per output the power
/// average across positions (or reseats) with its spread, position 1's phase
/// and arrival; the arrival only from a timing-reliable run.
pub(crate) fn document(plan: &Plan, runs: &[RunResult], ctx: &DocContext<'_>) -> Result<String> {
    let e =
        |m: dspi_measure::types::MeasureError| DspiError::InvalidArgument { msg: m.to_string() };
    let first = runs.first().ok_or_else(|| DspiError::InvalidArgument {
        msg: "no run to file".into(),
    })?;
    let mut outputs = Vec::new();
    for spec in &plan.outputs {
        let mine: Vec<(&RunResult, &OutputResult)> = runs
            .iter()
            .flat_map(|r| r.outputs.iter().map(move |o| (r, o)))
            .filter(|(_, o)| o.output == spec.output)
            .collect();
        let Some((r0, o0)) = mine.first().copied() else {
            continue;
        };
        let (response, spread) = if mine.len() > 1 {
            let responses: Vec<Response> = mine.iter().map(|(_, o)| o.response.clone()).collect();
            let avg =
                dspi_measure::position::average_for(&plan.placement, &responses).map_err(e)?;
            (avg.response, Some(avg.spread_db))
        } else {
            (o0.response.clone(), None)
        };
        let mut curve = Curve::from_response(&response);
        curve.spread_db = spread;
        let chain = ctx
            .chains
            .iter()
            .find(|(o, _)| *o == spec.output)
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| dspi_measure::types::ChannelChain {
                peq: Vec::new(),
                crossover: Vec::new(),
                gain_db: 0.0,
                delay_ms: 0.0,
                feed: dspi_measure::types::ProgrammeFeed {
                    gain_db: 0.0,
                    inverted: false,
                },
            });
        outputs.push(OutputMeasurement {
            output: spec.output,
            role: spec.role.way(),
            speaker: spec.speaker.clone(),
            chain: chain.into(),
            arrival_ms: r0.timing_reliable.then_some(o0.arrival_ms),
            response: curve,
            extra: Map::new(),
        });
    }
    let kind = match plan.kind {
        MeasureRunKind::Walked => RunKind::Walked,
        MeasureRunKind::Summed => RunKind::Summed,
        MeasureRunKind::LiveSweep => RunKind::LiveSweep,
        MeasureRunKind::Pink => RunKind::Pink,
    };
    let level = runs
        .iter()
        .map(|r| r.level_db)
        .fold(f32::INFINITY, f32::min);
    let mic = plan.mic.as_ref().map(|m| MicRef {
        id: m.doc.mic.id.clone(),
        name: m.doc.mic.name.clone(),
        serial: m.doc.mic.serial.clone(),
        sensitivity_db: m.doc.mic.sensitivity_db,
        approximate: m.doc.mic.approximate,
        cal_hash: m.doc.mic.source_hash.clone(),
        extra: Map::new(),
    });
    let name = if plan.name.is_empty() {
        format!("Measurement {}", ctx.saved_utc)
    } else {
        plan.name.clone()
    };
    let ratio = 1.0 + first.ratio_ppm * 1e-6;
    let doc = MeasurementDoc {
        schema_version: SCHEMA_VERSION,
        meta: Meta::new(
            DocKind::Measurement,
            &name,
            &ctx.saved_utc,
            env!("CARGO_PKG_VERSION"),
        ),
        measurement: Measurement {
            id: String::new(),
            serial: ctx.serial.to_owned(),
            platform: ctx.platform.to_owned(),
            saved_ms: ctx.saved_ms,
            placement: plan.placement,
            mic,
            stimulus: Stimulus {
                kind,
                f1_hz: f64::from(plan.sweep.f1_hz),
                f2_hz: f64::from(plan.sweep.f2_hz),
                duration_ms: if plan.kind == MeasureRunKind::Pink {
                    plan.pink_ms
                } else {
                    plan.sweep.duration_ms
                },
                repeat: u8::try_from(plan.passes).unwrap_or(1),
                gap_ms: u32::from(plan.sweep.gap_ms),
                level_db: if level.is_finite() {
                    f64::from(level)
                } else {
                    -120.0
                },
                device_rate_hz: ctx.device_rate_hz,
                extra: Map::new(),
            },
            capture: CaptureInfo {
                rate_hz: plan.capture_rate_hz,
                clock_ratio: if (0.99..=1.01).contains(&ratio) {
                    ratio
                } else {
                    1.0
                },
                timing_reliable: first.timing_reliable,
                max_residual_us: first.max_residual_us,
                // Position 1's timing in detail (P5SPIKE): kept as unknown
                // keys by every reader.
                extra: {
                    let mut m = first
                        .timing
                        .as_ref()
                        .and_then(|t| serde_json::to_value(t).ok())
                        .and_then(|v| v.as_object().cloned())
                        .map(|mut m| {
                            m.retain(|k, _| {
                                [
                                    "ratioPpm",
                                    "residualBoundUs",
                                    "reasons",
                                    "cycleOutputs",
                                    "cycleResidualsUs",
                                ]
                                .contains(&k.as_str())
                            });
                            m
                        })
                        .unwrap_or_default();
                    // -- WP P5RAMPFIX -- position 1's ramp, step by step with
                    // its in-band SNR (P5SPIKE and HIL logs).
                    if !first.ramp_steps.is_empty()
                        && let Ok(v) = serde_json::to_value(&first.ramp_steps)
                    {
                        m.insert("rampSteps".into(), v);
                    }
                    m
                },
            },
            outputs,
            positions: plan.positions.max(1),
            note: String::new(),
            tunes: Vec::new(),
            extra: Map::new(),
        },
        extra: Map::new(),
    };
    doc.to_text().map_err(e)
}

/// `secs` since the Unix epoch as ISO-8601 UTC (`2026-11-16T22:41:35Z`).
// Every value is bounded: `days` < 2^47 from a u64 of seconds, so each
// product and sum below stays far inside i64.
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn utc_text(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let of_day = secs % 86_400;
    // Howard Hinnant's civil_from_days, in i64 (days fit easily).
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}
