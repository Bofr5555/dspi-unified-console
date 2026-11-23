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

// -- WP P5TUNE --
/// The key of an output's SNR curve in a measurement document (an unknown key
/// to every reader, kept on write).
pub(crate) const SNR_KEY: &str = "snr";

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
    sc: &dspi_measure::stimulus::SweepConfig,
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
        gap_ms: sc.gap_ms,
        p: vec![sc.f1_hz, sc.f2_hz, 0.0, 0.0],
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

/// A ramp step of `output` at `level`: the run's own stimulus, short (S3);
/// -- WP ROLL1 -- `run`'s band, never below `output`'s floor.
pub(crate) fn ramp_config(
    plan: &Plan,
    run: &super::plan::RunSpec,
    output: u8,
    level: f32,
) -> SiggenConfig {
    if plan.kind == MeasureRunKind::Pink {
        return pink(output, level, RAMP_PINK_MS);
    }
    let mask = 1u16.checked_shl(u32::from(output)).unwrap_or(0);
    let sc = ramp_sweep(plan, run, &[output]);
    sweep(&sc, mask, 0, false, level, RAMP_SWEEP_MS, 1)
}

/// The band a ramp step of `outputs` plays: the run's, or (-- WP ADET0 --)
/// the plan's own ramp band; never below their floor.
pub(crate) fn ramp_sweep(
    plan: &Plan,
    run: &super::plan::RunSpec,
    outputs: &[u8],
) -> dspi_measure::stimulus::SweepConfig {
    let mut sc = plan.sweep_of(run);
    if let Some((f1, f2)) = plan.ramp_band {
        sc.f1_hz = f1;
        sc.f2_hz = f2;
    }
    sc.f1_hz = sc.f1_hz.max(plan.floor_of(outputs));
    sc
}

/// The run's configuration at `level`.
pub(crate) fn run_config(plan: &Plan, run: &super::plan::RunSpec, level: f32) -> SiggenConfig {
    let mask = run.mask();
    let ms = plan.sweep.duration_ms;
    // -- WP ROLL1 -- the run's own band
    let sc = plan.sweep_of(run);
    match plan.kind {
        // `WALK` on one output changes nothing; it keeps the status's walked
        // output for the cycle-identity check.
        MeasureRunKind::Walked => sweep(&sc, mask, 0, true, level, ms, plan.passes),
        MeasureRunKind::Summed => sweep(&sc, mask, run.invert_mask, false, level, ms, 1),
        MeasureRunKind::LiveSweep => sweep(&sc, mask, 0, false, level, ms, 1),
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
    // -- WP P5TUNE -- the de-embedding's validity mask needs the SNR (RC12)
    /// The SNR per point of the standard grid, dB (empty without a pre-roll).
    pub(crate) snr_db: Vec<f64>,
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
    // -- WP ROLL1 --
    /// The run's sweep start, Hz (0 for pink noise).
    pub(crate) f1_hz: f32,
    /// The run's arrivals are its alignment group's.
    pub(crate) aligns: bool,
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
    // -- WP P5TUNE -- the SNR before any correction or level shift
    let snr_db: Vec<f64> = noise.as_ref().map_or_else(Vec::new, |n| {
        resp.mag_db
            .iter()
            .zip(&n.mag_db)
            .map(|(s, n)| {
                let v = s - n;
                if v.is_finite() {
                    v.clamp(-100.0, 300.0)
                } else {
                    -100.0
                }
            })
            .collect()
    });
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
        snr_db,
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
        f1_hz: 0.0,
        aligns: true,
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
            snr_db: Vec::new(),
        }],
        level_db,
        timing_reliable: false,
        max_residual_us: None,
        ratio_ppm: 0.0,
        ramp_steps: Vec::new(),
        f1_hz: 0.0,
        aligns: true,
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
        // -- WP ROLL1 -- the response from the widest-band runs, one per
        // position: a non-tweeter's full-band run beside its group's
        // band-limited one. The arrival from position 1's: see `arrival`.
        let lowest = mine
            .iter()
            .map(|(r, _)| r.f1_hz)
            .fold(f32::INFINITY, f32::min);
        let mine: Vec<(&RunResult, &OutputResult)> = mine
            .into_iter()
            .filter(|(r, _)| r.f1_hz <= lowest)
            .collect();
        let Some((_, o0)) = mine.first().copied() else {
            continue;
        };
        // Each on its group's one timing origin (phase and arrival).
        let on_origin: Vec<(Response, Option<f64>)> = mine
            .iter()
            .map(|(r, o)| match origin_shift(runs, r) {
                Some((ms, reliable)) => (
                    shifted(&o.response, ms),
                    reliable.then_some(o.arrival_ms + ms),
                ),
                None => (o.response.clone(), None),
            })
            .collect();
        let arrival_ms = on_origin.first().and_then(|(_, a)| *a);
        let (response, spread) = if mine.len() > 1 {
            let responses: Vec<Response> = on_origin.iter().map(|(r, _)| r.clone()).collect();
            let avg =
                dspi_measure::position::average_for(&plan.placement, &responses).map_err(e)?;
            (avg.response, Some(avg.spread_db))
        } else {
            (
                on_origin
                    .first()
                    .map_or_else(|| o0.response.clone(), |(r, _)| r.clone()),
                None,
            )
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
        // -- WP P5TUNE -- position 1's SNR per point, for the de-embedding's
        // validity mask (RC12): a curve of the response's grid under `snr`.
        let mut extra = Map::new();
        if o0.snr_db.len() == o0.response.mag_db.len() && !o0.snr_db.is_empty() {
            let snr = Curve::from_response(&Response {
                grid: o0.response.grid,
                mag_db: o0.snr_db.clone(),
                phase_deg: None,
                valid: None,
                spl: false,
            });
            if let Ok(v) = serde_json::to_value(&snr) {
                extra.insert(SNR_KEY.into(), v);
            }
        }
        outputs.push(OutputMeasurement {
            output: spec.output,
            role: spec.role.way(),
            speaker: spec.speaker.clone(),
            chain: chain.into(),
            arrival_ms,
            response: curve,
            extra,
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
                // -- WP ROLL1 -- the outputs swept from the tweeter floor
                // (an unknown key to every reader, kept on write)
                extra: floor_extra(plan),
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

// -- WP ROLL1 --
/// How much later its group's timing origin is than run `r`'s, ms, and
/// whether that origin's timing is reliable: 0 and `r`'s own for every run
/// but a band-limited one beside a full-band one. A group of a tweeter and
/// other ways is measured twice at a position: its tweeters from 1 kHz with
/// a reference way (run A), and its other ways full band (run B). The
/// group's origin is B's: the other ways' responses and arrivals are B's;
/// a tweeter's are A's moved onto B's origin by the reference, the way both
/// runs measured (the best heard in A, by its SNR, should there be several),
/// so the tweeter against that way is A's own difference and the ways below
/// against each other B's. `None` when the shift cannot be had (no phase,
/// no shared band).
fn origin_shift(runs: &[RunResult], r: &RunResult) -> Option<(f64, bool)> {
    if !r.aligns {
        return Some((0.0, r.timing_reliable));
    }
    // Run A: its run B is the non-aligning run of the same position that
    // shares a way with it.
    let Some(b) = runs.iter().find(|b| {
        !b.aligns
            && b.position == r.position
            && b.outputs
                .iter()
                .any(|x| r.outputs.iter().any(|y| y.output == x.output))
    }) else {
        return Some((0.0, r.timing_reliable));
    };
    let snr = |x: &OutputResult| -> f64 {
        let v: Vec<f64> = x.snr_db.iter().copied().filter(|s| s.is_finite()).collect();
        if v.is_empty() {
            f64::NEG_INFINITY
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    let shift = b
        .outputs
        .iter()
        .filter_map(|xb| {
            let xa = r.outputs.iter().find(|y| y.output == xb.output)?;
            Some((snr(xa), frame_shift_ms(xa, xb, f64::from(r.f1_hz))?))
        })
        .max_by(|p, q| p.0.total_cmp(&q.0))?;
    Some((shift.1, r.timing_reliable && b.timing_reliable))
}

/// `resp` with its phase moved `shift_ms` later (onto a later origin).
fn shifted(resp: &Response, shift_ms: f64) -> Response {
    let mut out = resp.clone();
    if shift_ms != 0.0
        && let (Some(p), Ok(freqs)) = (out.phase_deg.as_mut(), resp.grid.freqs())
    {
        for (v, f) in p.iter_mut().zip(&freqs) {
            *v -= 360.0 * f * shift_ms / 1000.0;
        }
    }
    out
}

/// How much later run B's timing origin is than run A's, ms, from one way
/// measured in both (`a` in A, played from `f1_hz` up, `b` in B): its
/// arrivals' difference, corrected by the slope of its phases' difference
/// about those arrivals over the trusted band both share (within 20 dB of
/// its loudest there). The onset alone would not do: seen from 1 kHz up, a
/// way low-passed near the tweeter's crossover rings narrowband, and its
/// onset moves by a good part of a millisecond. `None` without phase or a
/// shared band.
fn frame_shift_ms(a: &OutputResult, b: &OutputResult, f1_hz: f64) -> Option<f64> {
    use std::f64::consts::{PI, TAU};
    let (ra, rb) = (&a.response, &b.response);
    if ra.grid != rb.grid {
        return None;
    }
    let freqs = ra.grid.freqs().ok()?;
    let (pa, pb) = (ra.phase_deg.as_ref()?, rb.phase_deg.as_ref()?);
    let ok = |r: &Response, i: usize| {
        r.valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false))
    };
    let lo = f1_hz * 2f64.powf(1.0 / 6.0);
    let shared: Vec<usize> = (0..freqs.len())
        .filter(|i| freqs.get(*i).is_some_and(|f| *f >= lo) && ok(ra, *i) && ok(rb, *i))
        .collect();
    let top = shared
        .iter()
        .filter_map(|i| ra.mag_db.get(*i))
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let (ta, tb) = (a.arrival_ms / 1000.0, b.arrival_ms / 1000.0);
    let mut prev: Option<f64> = None;
    let mut offset = 0.0;
    let (mut num, mut den) = (0.0, 0.0);
    for i in shared {
        let (Some(f), Some(m), Some(x), Some(y)) =
            (freqs.get(i), ra.mag_db.get(i), pa.get(i), pb.get(i))
        else {
            continue;
        };
        if *m < top - 20.0 {
            continue;
        }
        let w = TAU * f;
        // The phases about each arrival, their difference wrapped and then
        // unwrapped along the grid.
        let d = ((x.to_radians() + w * ta) - (y.to_radians() + w * tb) + PI).rem_euclid(TAU) - PI;
        if let Some(q) = prev {
            if d - q > PI {
                offset -= TAU;
            } else if d - q < -PI {
                offset += TAU;
            }
        }
        prev = Some(d);
        num += (d + offset) * w;
        den += w * w;
    }
    if den <= 0.0 {
        return None;
    }
    let shift = b.arrival_ms - a.arrival_ms + num / den * 1000.0;
    shift.is_finite().then_some(shift)
}

/// The stimulus' `tweeterFloor`: the high ways measured from 1 kHz up, and
/// the floor (empty when the plan has none).
fn floor_extra(plan: &Plan) -> Map<String, serde_json::Value> {
    let mut m = Map::new();
    let high: Vec<u8> = plan
        .outputs
        .iter()
        .filter(|o| o.role == super::MeasureRole::High)
        .map(|o| o.output)
        .collect();
    if !high.is_empty() {
        m.insert(
            "tweeterFloor".into(),
            serde_json::json!({
                "hz": super::plan::TWEETER_STIMULUS_FLOOR_HZ,
                "outputs": high,
            }),
        );
    }
    m
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
