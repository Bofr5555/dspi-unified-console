//! The live response and direct adjustments (WP P5LIVE, PLAN-P5 T9 and T5).
//!
//! **Live mode** plays one output again and again through the measurement
//! session's own run kinds (P5SES), so every safety rule of a measurement
//! holds for each of them: a **single live sweep** (one magnitude result per
//! sweep) or **finite pink noise** (at most 60 s, re-armed every 30 s by
//! default) analysed by the microphone RTA with its averaging presets, hold
//! and compare. Each sweep or pink run is its own session: the preflight, a
//! pre-roll, a ramp, the run. The first ramp starts quiet (S3); a later one
//! starts at the level the previous run settled on, lowered by any boost a
//! direct adjustment added since, so the live view does not climb the whole
//! ramp for every frame. The device lock "Measuring" is held from the start
//! of the live view to its end, between its runs too ([`DspiCore::measure_live_tick`]
//! renews its lease; a client that stops ticking loses it within 30 s).
//!
//! **What is shown** ([`MeasureLiveFrame`]): the measured curve (relative dB,
//! or dB SPL with a calibrated microphone; untrusted points flagged), the
//! target levelled to it, and the **predicted** curve: the measurement with
//! the measured chain taken out and the device's current chain plus a pending
//! change put in, through the core's own cascade (`CoreSectionModel`,
//! OD-P5c). An edit therefore shows at once, and the next sweep measures it.
//!
//! **Direct adjustments** are proposals first, written only by
//! [`DspiCore::measure_live_apply`] with the proposal's id (no write without a
//! preview), as one undo group, unguarded (S10: one band, as a manual edit):
//! "cut this peak" ([`DspiCore::measure_live_cut_peak`], one free PEQ band
//! from P5FIT on a one-octave span, cut only), "fit this range"
//! ([`DspiCore::measure_live_fit_range`], P5FIT on the span with the
//! placement's bounds and the limits profile's boost cap; every band the fit
//! lists is written, and its headroom cut as an output-gain cut), and any
//! edit the client asks to see ([`DspiCore::measure_live_predict`]). Every
//! write is judged by the limits profile before it is offered, and again by
//! the setters when it is written. A fit's range starts at the lowest trusted
//! point of the measurement (review P5-A N11). The run in flight is stopped
//! before the writes (its curve would mix two chains) and the next one
//! starts.
//!
//! **Locks** (in this order): the live control state, then the session's
//! slot, then `inner`; the view and the RTA's copy of the capture are leaves.
//! The [`crate::MeasureListener`] is called while the live control state is
//! held: from a listener, read (`measure_state`, `measure_live_frame`,
//! `measure_live_state`), never drive the live view.

use std::sync::MutexGuard;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dspi_measure::doc::{MeasurementDoc, TargetDoc};
use dspi_measure::fit::{FitOptions, FitResult, fit};
use dspi_measure::response::{resample, smooth};
use dspi_measure::rta::{Averaging, MicRta, RtaConfig};
use dspi_measure::target::{
    DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse, default_target,
};
use dspi_measure::types::{BandSpec, ChannelChain, Coeffs, LogGrid, Response, SectionModel};
use dspi_transport::clock::Instant;
use serde::{Deserialize, Serialize};

use super::plan::{PINK_MAX_MS, Plan};
use super::preflight::{PreflightOptions, chain_of};
use super::{
    MeasureCoupler, MeasureIssue, MeasureOutput, MeasurePhase, MeasurePlacement, MeasureRole,
    MeasureRunKind, MeasureSetup, MeasureSlot, MeasureState, session,
};
use crate::measure_doc::CoreSectionModel;
use crate::records::FilterParams;
use crate::{DspiCore, DspiError, Result};
use dspi_proto::enums::FilterType;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5live;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5live_device;

/// Pink noise a live run plays when the setup names no length, ms: the live
/// view re-arms it this often (S7).
pub(crate) const PINK_REARM_MS: u32 = 30_000;
/// Pink noise skipped at a run's start before the RTA hears it (the input's
/// latency), ms.
const PINK_SKIP_MS: u32 = 300;
/// The most capture the RTA's copy holds between two ticks, samples (about
/// 5 s at 192 kHz); past it the copy starts again.
const TEE_MAX: usize = 1 << 20;
/// How often the live view renews the device lock's lease.
const RENEW_EVERY: Duration = Duration::from_secs(10);
/// The grid the frame's curves are given on: 10 Hz to 22 kHz at 1/48 octave.
const DISPLAY_GRID: LogGrid = LogGrid {
    min_hz: dspi_measure::types::GRID_MIN_HZ,
    max_hz: dspi_measure::types::GRID_MAX_HZ,
    points_per_octave: 48,
};
/// "Cut this peak" fits one octave about the point: half an octave each side.
const CUT_HALF_SPAN: f64 = std::f64::consts::SQRT_2;
/// A span narrower than this ratio is no span (a fit needs points).
const MIN_SPAN_RATIO: f64 = 1.1;
/// A fit keeps a third of an octave from the trusted range's ends.
const EDGE_MARGIN: f64 = 1.259_921_049_894_873_2;
/// The PEQ filter types a live proposal names in words.
const TYPE_FLAT: u8 = 0;
const TYPE_PEAKING: u8 = 1;
const TYPE_LOW_SHELF: u8 = 2;
const TYPE_HIGH_SHELF: u8 = 3;

/// The live view's notice keys (`assets/notices/notices.json`, family
/// `measure`).
pub(crate) const NOTICE_NO_FRAME: &str = "measure.live.no_frame";
pub(crate) const NOTICE_NOTHING: &str = "measure.live.nothing_to_change";
pub(crate) const NOTICE_CHAIN_CHANGED: &str = "measure.live.chain_changed";

// ── Records ─────────────────────────────────────────────────────────────

/// What the live view plays.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureLiveMode {
    /// Short log sweeps, one after the other: one magnitude curve per sweep.
    #[default]
    Sweep,
    /// Finite pink noise with the microphone RTA (re-armed while live).
    PinkRta,
}

/// The microphone RTA's averaging preset.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureRtaAveraging {
    /// Exponential, 125 ms.
    Fast,
    /// Exponential, 1 s.
    #[default]
    Slow,
    /// Every frame since the start (or the last change) alike.
    Infinite,
}

impl MeasureRtaAveraging {
    fn averaging(self) -> Averaging {
        match self {
            MeasureRtaAveraging::Fast => Averaging::FAST,
            MeasureRtaAveraging::Slow => Averaging::SLOW,
            MeasureRtaAveraging::Infinite => Averaging::INFINITE,
        }
    }
}

/// What the live view measures and how (`measure_live_start`). Every number
/// 0 (or absent) is the default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureLiveSetup {
    /// The output, 0-based.
    pub output: u8,
    /// Sweeps or pink noise with the RTA.
    pub mode: MeasureLiveMode,
    /// Its way (the tweeter rule, S6, needs `mid` or `high` for those).
    pub role: MeasureRole,
    /// Where the microphone is: the listening position, near field, or a
    /// headphone coupler (several positions are not live).
    pub placement: MeasurePlacement,
    /// The coupler, for headphones.
    pub coupler: Option<MeasureCoupler>,
    /// The microphone's `mic` document's text; "" for an uncalibrated one.
    pub mic_document: String,
    /// A `target` document's text; "" for the placement's default target.
    pub target_document: String,
    /// The graph's smoothing, 1/N octave (1 to 48); 0 for none.
    pub smoothing: u8,
    /// One sweep, ms (1000 to 15000; 0: 2700).
    pub sweep_ms: u32,
    /// The gap after a sweep, ms (300 to 2000; 0: the placement's).
    pub gap_ms: u32,
    /// Pink noise per run, ms (1000 to 60000; 0: 30000, then re-armed).
    pub pink_ms: u32,
    /// The generator's ceiling, peak dBFS (as a measurement's).
    pub ceiling_db: Option<f32>,
    /// The SPL cap with a calibrated microphone, dB SPL (as a measurement's).
    pub spl_cap_db: Option<f32>,
    /// The capture's rate, Hz (0: 48000).
    pub capture_rate_hz: u32,
    /// The RTA's averaging.
    pub averaging: MeasureRtaAveraging,
    /// The RTA's bands per octave (1 to 48; 0: 3).
    pub bands_per_octave: u8,
}

/// A curve as the live graph draws it (10 Hz to 22 kHz, 1/48 octave).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLiveCurve {
    /// Frequencies, Hz, ascending.
    pub freqs_hz: Vec<f64>,
    /// Magnitude, dB (dB SPL when `spl`).
    pub mag_db: Vec<f64>,
    /// Per point, trusted (untrusted points are drawn faint).
    pub valid: Vec<bool>,
    /// The magnitudes are dB SPL (a calibrated microphone).
    pub spl: bool,
}

/// One PEQ band of the live output, as a proposal writes it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureLiveBand {
    /// The PEQ band, 0-based.
    pub band: u8,
    /// The firmware's filter type (0 flat, 1 peak, 2 low shelf, 3 high
    /// shelf, ...).
    pub filter_type: u8,
    /// Switched off.
    pub bypass: bool,
    /// Centre or corner, Hz.
    pub freq: f32,
    /// Q.
    pub q: f32,
    /// Gain, dB.
    pub gain: f32,
    /// The Linkwitz Transform's target Q; unused otherwise.
    pub qp: f32,
}

impl MeasureLiveBand {
    fn of(band: u8, p: &FilterParams) -> MeasureLiveBand {
        MeasureLiveBand {
            band,
            filter_type: p.filter_type,
            bypass: p.bypass,
            freq: p.freq,
            q: p.q,
            gain: p.gain,
            qp: p.qp,
        }
    }

    fn params(&self) -> FilterParams {
        FilterParams {
            filter_type: self.filter_type,
            bypass: self.bypass,
            freq: self.freq,
            q: self.q,
            gain: self.gain,
            qp: self.qp,
        }
    }
}

/// One band a proposal changes.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLiveChange {
    /// The PEQ band, 0-based.
    pub band: u8,
    /// What it is now.
    pub before: MeasureLiveBand,
    /// What the proposal writes.
    pub after: MeasureLiveBand,
    /// The change in words ("PEQ band 4: flat → peak -5.8 dB at 1.02 kHz,
    /// Q 4.1").
    pub words: String,
}

/// What made a proposal.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureLiveAdjust {
    /// "Cut this peak".
    #[default]
    CutPeak,
    /// "Fit this range".
    FitRange,
    /// An edit the client asked to preview.
    Edit,
}

/// A direct adjustment, previewed: nothing is written until
/// `measure_live_apply` with its id.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLiveProposal {
    /// Its id (`measure_live_apply`).
    pub id: u32,
    /// What made it.
    pub adjust: MeasureLiveAdjust,
    /// The output.
    pub output: u8,
    /// The undo step's label.
    pub label: String,
    /// The bands it writes (empty: nothing to change, see `issue`).
    pub changes: Vec<MeasureLiveChange>,
    /// The output's gain now, dB.
    pub gain_before_db: f32,
    /// The output's gain it writes, dB (a headroom cut; equal: unchanged).
    pub gain_after_db: f32,
    /// The RMS error to the target in the span before, dB (a fit's).
    pub rms_before_db: f64,
    /// The same, predicted after, dB.
    pub rms_after_db: f64,
    /// What was done and why, in words.
    pub notes: Vec<String>,
    /// Why nothing is proposed, when nothing is.
    pub issue: Option<MeasureIssue>,
    /// The curve the change is predicted to give.
    pub predicted: Option<MeasureLiveCurve>,
}

/// The microphone RTA of the live view.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLiveRta {
    /// The averaged spectrum in fractional-octave bands (dBFS, or dB SPL).
    pub current: Option<MeasureLiveCurve>,
    /// The held copy.
    pub held: Option<MeasureLiveCurve>,
    /// The current spectrum minus the held one, dB.
    pub compare: Option<MeasureLiveCurve>,
    /// FFT frames averaged so far.
    pub frames: u64,
    /// The averaging preset.
    pub averaging: MeasureRtaAveraging,
}

/// What the live view shows now (`measure_live_frame`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLiveFrame {
    /// The live view runs.
    pub active: bool,
    /// What it plays.
    pub mode: MeasureLiveMode,
    /// The output.
    pub output: u8,
    /// Counts the measured curves (one per sweep; the RTA's per update).
    pub seq: u32,
    /// The measured curve, if one.
    pub measured: Option<MeasureLiveCurve>,
    /// The target, levelled to the measured curve.
    pub target: Option<MeasureLiveCurve>,
    /// The predicted curve: of the pending change, or of a change already
    /// written that no sweep has measured yet.
    pub predicted: Option<MeasureLiveCurve>,
    /// The microphone RTA (pink mode).
    pub rta: Option<MeasureLiveRta>,
    /// The change previewed, if one.
    pub pending: Option<MeasureLiveProposal>,
    /// Why the live view stopped, if it did.
    pub issue: Option<MeasureIssue>,
    /// The current run's state.
    pub session: MeasureState,
}

/// Where the live view stands (`measure_live_tick` and the other calls).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLiveState {
    /// The live view runs.
    pub active: bool,
    /// What it plays.
    pub mode: MeasureLiveMode,
    /// The output.
    pub output: u8,
    /// Counts the measured curves.
    pub seq: u32,
    /// Runs started (sweeps, or pink runs).
    pub runs: u32,
    /// The level the next run starts its ramp at, peak dBFS.
    pub level_db: Option<f32>,
    /// The pending proposal's id, if one.
    pub pending: Option<u32>,
    /// Why the live view stopped, if it did.
    pub issue: Option<MeasureIssue>,
    /// The current run's state.
    pub session: MeasureState,
}

// ── State ───────────────────────────────────────────────────────────────

/// A measured curve and the chain it was measured through.
#[derive(Debug, Clone)]
struct Measured {
    resp: Response,
    chain: ChannelChain,
}

/// A previewed change.
#[derive(Debug, Clone)]
struct Pending {
    proposal: MeasureLiveProposal,
    /// The output gain to write, when it changes.
    gain: Option<f32>,
}

/// The live view's control state (in [`MeasureSlot`]).
pub(crate) struct Live {
    setup: MeasureLiveSetup,
    session_setup: MeasureSetup,
    target: Response,
    headphones: bool,
    /// Where the next ramp starts.
    level: Option<f32>,
    /// The level of the pink run in flight.
    run_level: Option<f32>,
    measured: Option<Measured>,
    /// The chain the RTA's average is of (pink).
    rta_chain: Option<ChannelChain>,
    rta: Option<MicRta>,
    /// The RTA's held spectrum (kept across a change of averaging).
    held: Option<Response>,
    seq: u32,
    runs: u32,
    pending: Option<Pending>,
    next_id: u32,
    active: bool,
    issue: Option<MeasureIssue>,
    last_renew: Instant,
}

/// What the live view shows: the frame without the session's state (a leaf
/// readers take; built by every live call).
#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    frame: MeasureLiveFrame,
    state: MeasureLiveState,
}

/// The live pink RTA's copy of the capture: filled by the push path during a
/// pink run's playing part, drained by the live tick (no analysis in the push
/// path, RC8). Memory only, for the run (S12).
#[derive(Debug, Default)]
pub(crate) struct Tee {
    on: bool,
    /// Frames of pink noise after the start to keep.
    pink_frames: u64,
    skip_frames: u64,
    buf: Vec<f32>,
}

impl MeasureSlot {
    /// The live view runs (it keeps the device lock).
    pub(crate) fn live_on(&self) -> bool {
        self.live_hold.load(Ordering::SeqCst)
    }

    fn live_lock(&self) -> MutexGuard<'_, Option<Live>> {
        self.live.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn view_lock(&self) -> MutexGuard<'_, Option<View>> {
        self.live_view.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn tee_lock(&self) -> MutexGuard<'_, Tee> {
        self.live_tee.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The push path's hook: keep a copy of a pink run's playing part for the
/// live RTA. O(block), no analysis.
pub(crate) fn tee_push(
    slot: &MeasureSlot,
    state: &MeasureState,
    first_frame: u64,
    samples: &[f32],
) {
    let mut tee = slot.tee_lock();
    if !tee.on
        || state.kind != MeasureRunKind::Pink
        || state.phase != MeasurePhase::Run
        || state.level_db.is_none()
    {
        return;
    }
    let Some(start) = state.generator_start_frame else {
        return;
    };
    let from = start.saturating_add(tee.skip_frames);
    let to = start.saturating_add(tee.pink_frames);
    let len = samples.len() as u64;
    let end = first_frame.saturating_add(len);
    if end <= from || first_frame >= to {
        return;
    }
    let a = usize::try_from(from.saturating_sub(first_frame)).unwrap_or(usize::MAX);
    let b = usize::try_from(to.saturating_sub(first_frame))
        .unwrap_or(usize::MAX)
        .min(samples.len());
    let Some(part) = samples.get(a..b) else {
        return;
    };
    if tee.buf.len().saturating_add(part.len()) > TEE_MAX {
        // Nobody drained it: start again (the RTA averages what it gets).
        tee.buf.clear();
    }
    tee.buf.extend_from_slice(part);
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

fn frames_of(ms: u32, rate: u32) -> u64 {
    u64::from(rate).saturating_mul(u64::from(ms)) / 1000
}

// ── The maths (pure) ────────────────────────────────────────────────────

/// One biquad's magnitude at `f`, dB.
fn biquad_db(c: &Coeffs, f: f64, fs: f64) -> f64 {
    let w = std::f64::consts::TAU * f / fs;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let (nr, ni) = (c.b0 + c.b1 * c1 + c.b2 * c2, -(c.b1 * s1 + c.b2 * s2));
    let (dr, di) = (1.0 + c.a1 * c1 + c.a2 * c2, -(c.a1 * s1 + c.a2 * s2));
    let num = nr * nr + ni * ni;
    let den = dr * dr + di * di;
    let db = 10.0 * (num.max(1e-30) / den.max(1e-30)).log10();
    if db.is_finite() { db } else { 0.0 }
}

/// A chain's magnitude (PEQ, crossover, output gain) on `freqs`, dB, through
/// `model`.
pub(crate) fn chain_db(chain: &ChannelChain, model: &dyn SectionModel, freqs: &[f64]) -> Vec<f64> {
    let fs = model.sample_rate();
    let gain = if chain.gain_db.is_finite() {
        f64::from(chain.gain_db)
    } else {
        0.0
    };
    let mut out = vec![gain; freqs.len()];
    for band in chain.peq.iter().chain(&chain.crossover) {
        for c in model.sections(band) {
            for (o, f) in out.iter_mut().zip(freqs) {
                *o += biquad_db(&c, *f, fs);
            }
        }
    }
    out
}

/// `resp` measured through `measured_with`, re-predicted through `now`: the
/// measured chain taken out, the new one put in (magnitude only).
pub(crate) fn repredict(
    resp: &Response,
    measured_with: &ChannelChain,
    now: &ChannelChain,
    model: &dyn SectionModel,
) -> Result<Response> {
    let freqs = resp.grid.freqs().map_err(|e| invalid(e.to_string()))?;
    if freqs.len() != resp.mag_db.len() {
        return Err(invalid("a response whose length is not its grid's"));
    }
    let old = chain_db(measured_with, model, &freqs);
    let new = chain_db(now, model, &freqs);
    let mag_db = resp
        .mag_db
        .iter()
        .zip(old.iter().zip(&new))
        .map(|(m, (o, n))| {
            let v = m - o + n;
            if v.is_finite() { v } else { *m }
        })
        .collect();
    Ok(Response {
        grid: resp.grid,
        mag_db,
        phase_deg: None,
        valid: resp.valid.clone(),
        spl: resp.spl,
    })
}

/// The largest rise from `before` to `after` anywhere on the standard grid,
/// dB (0 when nothing rises): a ramp after a change starts this much lower.
pub(crate) fn max_rise_db(
    before: &ChannelChain,
    after: &ChannelChain,
    model: &dyn SectionModel,
) -> f64 {
    let Ok(freqs) = LogGrid::STANDARD.freqs() else {
        return 0.0;
    };
    let a = chain_db(before, model, &freqs);
    let b = chain_db(after, model, &freqs);
    a.iter()
        .zip(&b)
        .map(|(x, y)| y - x)
        .filter(|d| d.is_finite())
        .fold(0.0, f64::max)
}

/// The lowest and highest trusted frequency of `resp`.
pub(crate) fn trusted_span(resp: &Response) -> Option<(f64, f64)> {
    let freqs = resp.grid.freqs().ok()?;
    let mut it = freqs
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            resp.valid
                .as_ref()
                .is_none_or(|v| v.get(*i).copied().unwrap_or(false))
        })
        .map(|(_, f)| *f);
    let lo = it.next()?;
    let hi = it.next_back().unwrap_or(lo);
    Some((lo, hi))
}

/// A fit's span held inside the measurement's trusted range (review P5-A
/// N11: never into a roll-off the measurement cannot see): it starts a third
/// of an octave above the lowest trusted point at the earliest and ends as
/// far below the highest (a sweep's edges, where a curve still bends, are
/// trusted but no place for a band). `None` when nothing of it is left.
pub(crate) fn fit_span(resp: &Response, f1: f64, f2: f64) -> Option<(f64, f64)> {
    if !(f1.is_finite() && f2.is_finite()) {
        return None;
    }
    let (lo, hi) = (f1.min(f2), f1.max(f2));
    let (t_lo, t_hi) = trusted_span(resp)?;
    let a = lo.max(t_lo * EDGE_MARGIN);
    let b = hi.min(t_hi / EDGE_MARGIN);
    (a > 0.0 && b >= a * MIN_SPAN_RATIO).then_some((a, b))
}

/// `b` as a band of the chain's kind.
fn band_spec(p: &FilterParams) -> BandSpec {
    BandSpec {
        filter_type: p.filter_type,
        bypass: p.bypass,
        freq: p.freq,
        q: p.q,
        gain: p.gain,
        qp: p.qp,
    }
}

fn params_of(b: &BandSpec) -> FilterParams {
    FilterParams {
        filter_type: b.filter_type,
        bypass: b.bypass,
        freq: b.freq,
        q: b.q,
        gain: b.gain,
        qp: b.qp,
    }
}

fn hz_words(f: f32) -> String {
    if f >= 1_000.0 {
        format!("{:.2} kHz", f / 1_000.0)
    } else {
        format!("{f:.0} Hz")
    }
}

/// A band in words ("peak -5.8 dB at 1.02 kHz, Q 4.1").
pub(crate) fn band_words(p: &FilterParams) -> String {
    let what = match p.filter_type {
        TYPE_FLAT => return "flat".into(),
        TYPE_PEAKING => "peak",
        TYPE_LOW_SHELF => "low shelf",
        TYPE_HIGH_SHELF => "high shelf",
        t => return format!("type {t} at {}", hz_words(p.freq)),
    };
    let off = if p.bypass { " (bypassed)" } else { "" };
    format!(
        "{what} {:+.1} dB at {}, Q {:.2}{off}",
        p.gain,
        hz_words(p.freq),
        p.q
    )
}

/// The writes of a fit: **every** entry of `FitResult.bands`, a flat one
/// included (review P5-A: a released band the fit does not need is written
/// flat), each with the band's value now.
pub(crate) fn fit_writes(r: &FitResult, chain: &ChannelChain) -> Vec<MeasureLiveChange> {
    r.bands
        .iter()
        .map(|b| {
            let before = chain
                .peq
                .get(usize::from(b.index))
                .map(params_of)
                .unwrap_or(FilterParams {
                    filter_type: TYPE_FLAT,
                    bypass: false,
                    freq: 1_000.0,
                    q: 0.707,
                    gain: 0.0,
                    qp: 0.0,
                });
            let mut after = params_of(&b.band);
            // The Linkwitz target Q is the band's own for every other type.
            if !FilterType::from_raw(after.filter_type).is_linkwitz() {
                after.qp = before.qp;
            }
            change(b.index, &before, &after)
        })
        .collect()
}

fn change(band: u8, before: &FilterParams, after: &FilterParams) -> MeasureLiveChange {
    MeasureLiveChange {
        band,
        before: MeasureLiveBand::of(band, before),
        after: MeasureLiveBand::of(band, after),
        words: format!(
            "PEQ band {}: {} → {}",
            u16::from(band).saturating_add(1),
            band_words(before),
            band_words(after)
        ),
    }
}

/// `chain` with `changes` and an output gain put in.
fn chain_with(
    chain: &ChannelChain,
    changes: &[MeasureLiveChange],
    gain: Option<f32>,
) -> ChannelChain {
    let mut c = chain.clone();
    for ch in changes {
        if let Some(b) = c.peq.get_mut(usize::from(ch.band)) {
            *b = band_spec(&ch.after.params());
        }
    }
    if let Some(g) = gain {
        c.gain_db = g;
    }
    c
}

/// A response on the display grid, smoothed by `smoothing` (1/N octave; 0
/// none), as the frame carries it.
pub(crate) fn curve(resp: &Response, smoothing: u8) -> Option<MeasureLiveCurve> {
    let r = if smoothing > 0 {
        smooth(resp, smoothing).ok()?
    } else {
        resp.clone()
    };
    let r = resample(&r, &DISPLAY_GRID).ok()?;
    let freqs = DISPLAY_GRID.freqs().ok()?;
    let valid = r
        .valid
        .clone()
        .unwrap_or_else(|| vec![true; r.mag_db.len()]);
    Some(MeasureLiveCurve {
        freqs_hz: freqs,
        mag_db: r
            .mag_db
            .iter()
            .map(|m| if m.is_finite() { *m } else { 0.0 })
            .collect(),
        valid,
        spl: r.spl,
    })
}

/// `target` moved to `measured`'s level: the mean difference over the
/// reference band's trusted points (else over every trusted point).
pub(crate) fn levelled_target(target: &Response, measured: &Response) -> Response {
    let mut t = target.clone();
    let Ok(on) = resample(target, &measured.grid) else {
        return t;
    };
    let Ok(freqs) = measured.grid.freqs() else {
        return t;
    };
    let trusted = |i: usize| {
        measured
            .valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false))
            && on
                .valid
                .as_ref()
                .is_none_or(|v| v.get(i).copied().unwrap_or(false))
    };
    let mean = |in_band: &dyn Fn(f64) -> bool| {
        let (mut sum, mut n) = (0.0f64, 0u32);
        for (i, f) in freqs.iter().enumerate() {
            if !trusted(i) || !in_band(*f) {
                continue;
            }
            if let (Some(m), Some(x)) = (measured.mag_db.get(i), on.mag_db.get(i)) {
                let d = m - x;
                if d.is_finite() {
                    sum += d;
                    n = n.saturating_add(1);
                }
            }
        }
        (n > 0).then(|| sum / f64::from(n))
    };
    let band = DEFAULT_REFERENCE_BAND;
    let offset = mean(&|f| f >= band.low_hz && f <= band.high_hz).or_else(|| mean(&|_| true));
    if let Some(o) = offset {
        for m in &mut t.mag_db {
            *m += o;
        }
    }
    t.spl = measured.spl;
    t
}

// ── The setup ───────────────────────────────────────────────────────────

/// The checked setup: the session's setup for each run, the target, whether
/// it is headphones.
fn check_setup(setup: &MeasureLiveSetup) -> Result<(MeasureSetup, Response, bool)> {
    let headphones = match setup.placement {
        MeasurePlacement::ListeningPosition | MeasurePlacement::NearField => false,
        MeasurePlacement::Headphones => true,
        MeasurePlacement::MultiPosition => {
            return Err(invalid("the live view measures one position"));
        }
    };
    if setup.smoothing > dspi_measure::response::MAX_SMOOTHING {
        return Err(invalid(format!(
            "smoothing 1/{} octave (0, or 1 to 48)",
            setup.smoothing
        )));
    }
    if setup.bands_per_octave > dspi_measure::rta::MAX_BANDS_PER_OCTAVE {
        return Err(invalid(format!(
            "{} RTA bands per octave (1 to 48)",
            setup.bands_per_octave
        )));
    }
    let pink_ms = match setup.pink_ms {
        0 => PINK_REARM_MS,
        ms if (1_000..=PINK_MAX_MS).contains(&ms) => ms,
        ms => return Err(invalid(format!("pink noise of {ms} ms (1000 to 60000)"))),
    };
    let target_use = if headphones {
        TargetUse::Headphones
    } else {
        TargetUse::Speakers
    };
    let coupler = setup.coupler.map(MeasureCoupler::coupler);
    let target = if setup.target_document.trim().is_empty() {
        Target {
            target_use,
            coupler,
            definition: TargetDefinition::Builtin {
                builtin: default_target(target_use, coupler),
            },
            reference: DEFAULT_REFERENCE_BAND,
        }
    } else {
        TargetDoc::from_text(&setup.target_document)
            .and_then(|d| d.target.to_target())
            .map_err(|e| invalid(format!("the target document: {e}")))?
    };
    let target = target
        .evaluate(&LogGrid::STANDARD)
        .map_err(|e| invalid(format!("the target: {e}")))?;
    let session_setup = MeasureSetup {
        kind: match setup.mode {
            MeasureLiveMode::Sweep => MeasureRunKind::LiveSweep,
            MeasureLiveMode::PinkRta => MeasureRunKind::Pink,
        },
        outputs: vec![MeasureOutput {
            output: setup.output,
            role: setup.role,
            ..MeasureOutput::default()
        }],
        placement: setup.placement,
        count: u8::from(headphones),
        coupler: setup.coupler,
        mic_document: setup.mic_document.clone(),
        sweep_ms: setup.sweep_ms,
        gap_ms: setup.gap_ms,
        passes: 0,
        pink_ms,
        ceiling_db: setup.ceiling_db,
        spl_cap_db: setup.spl_cap_db,
        capture_rate_hz: setup.capture_rate_hz,
        name: "Live".into(),
        start_level_only: false,
    };
    // Every bound a run has, checked now.
    Plan::new(&session_setup)?;
    Ok((session_setup, target, headphones))
}

fn rta_of(setup: &MeasureLiveSetup, plan: &Plan) -> Result<MicRta> {
    let cfg = RtaConfig {
        sample_rate: f64::from(plan.capture_rate_hz),
        fft_len: 16_384,
        bands_per_octave: if setup.bands_per_octave == 0 {
            3
        } else {
            setup.bands_per_octave
        },
        averaging: setup.averaging.averaging(),
    };
    let mic = plan.mic.as_ref().map(|m| m.correction.clone());
    let offset = plan.mic.as_ref().and_then(|m| m.spl_offset_db);
    MicRta::new(cfg, mic.as_ref(), offset).map_err(|e| invalid(e.to_string()))
}

// ── The live view on the core ───────────────────────────────────────────

impl Live {
    fn state(&self, session: MeasureState) -> MeasureLiveState {
        MeasureLiveState {
            active: self.active,
            mode: self.setup.mode,
            output: self.setup.output,
            seq: self.seq,
            runs: self.runs,
            level_db: self.level,
            pending: self.pending.as_ref().map(|p| p.proposal.id),
            issue: self.issue.clone(),
            session,
        }
    }

    fn pink(&self) -> bool {
        self.setup.mode == MeasureLiveMode::PinkRta
    }

    /// The measured curve now: a sweep's, or the RTA's referred to the run's
    /// level on the standard grid.
    fn measured_now(&self) -> Option<Measured> {
        if !self.pink() {
            return self.measured.clone();
        }
        let rta = self.rta.as_ref()?;
        let s = rta.spectrum()?;
        let mut r = resample(&s, &LogGrid::STANDARD).ok()?;
        let level = f64::from(self.run_level.or(self.level)?);
        for m in &mut r.mag_db {
            *m -= level;
        }
        Some(Measured {
            resp: r,
            chain: self.rta_chain.clone()?,
        })
    }
}

impl DspiCore {
    /// The device's chain of `output` now.
    fn live_chain(&self, output: u8) -> ChannelChain {
        chain_of(&self.lock().state, output)
    }

    /// Rebuild what readers see from `live`.
    fn live_publish(&self, live: &Live) {
        let smoothing = live.setup.smoothing;
        let measured = live.measured_now();
        let chain_now = self.live_chain(live.setup.output);
        let predicted = measured.as_ref().and_then(|m| {
            let pending = live.pending.as_ref();
            let next = match pending {
                Some(p) => chain_with(&chain_now, &p.proposal.changes, p.gain),
                None if chain_now != m.chain => chain_now.clone(),
                None => return None,
            };
            repredict(&m.resp, &m.chain, &next, &CoreSectionModel)
                .ok()
                .and_then(|r| curve(&r, smoothing))
        });
        let target = match &measured {
            Some(m) => curve(&levelled_target(&live.target, &m.resp), 0),
            None => curve(&live.target, 0),
        };
        let rta = live.rta.as_ref().map(|r| {
            let current = r.spectrum();
            let compare = current
                .as_ref()
                .zip(live.held.as_ref())
                .and_then(|(c, h)| dspi_measure::rta::compare(c, h).ok());
            MeasureLiveRta {
                current: current.as_ref().and_then(|s| curve(s, 0)),
                held: live.held.as_ref().and_then(|s| curve(s, 0)),
                compare: compare.as_ref().and_then(|s| curve(s, 0)),
                frames: r.frames(),
                averaging: live.setup.averaging,
            }
        });
        let frame = MeasureLiveFrame {
            active: live.active,
            mode: live.setup.mode,
            output: live.setup.output,
            seq: live.seq,
            measured: measured.as_ref().and_then(|m| curve(&m.resp, smoothing)),
            target,
            predicted,
            rta,
            pending: live.pending.as_ref().map(|p| p.proposal.clone()),
            issue: live.issue.clone(),
            session: MeasureState::default(),
        };
        let state = live.state(MeasureState::default());
        *self.measure.view_lock() = Some(View { frame, state });
    }

    /// End the live view: the run stopped (one `STOP_NOW` if it plays), the
    /// lock given back, `issue` kept.
    fn live_end(&self, live: &mut Live, issue: Option<MeasureIssue>) {
        live.active = false;
        if issue.is_some() {
            live.issue = issue;
        }
        self.measure.live_hold.store(false, Ordering::SeqCst);
        {
            let mut tee = self.measure.tee_lock();
            tee.on = false;
            tee.buf = Vec::new();
        }
        if self.measure_state().active {
            self.measure_stop_session();
        }
        // Between runs the live view holds the lock itself.
        self.end_held_lock(Some(crate::presets::Holder::Measure));
        let _ = self.measure_clear();
    }

    /// The next run: its plan (the ramp from the last level), the preflight,
    /// the start. A refusal ends the live view with its reason.
    fn live_rearm(&self, live: &mut Live) {
        let mut plan = match Plan::new(&live.session_setup) {
            Ok(p) => p,
            Err(e) => {
                let i = MeasureIssue::new("measure.no_signal", None, e.to_string());
                self.live_end(live, Some(i));
                return;
            }
        };
        if let Some(l) = live.level {
            // Never above what the last ramp settled on (S3).
            plan.start_db = l.min(plan.ceiling_db).max(-120.0);
        }
        match self.measure_preflight_plan(&plan, &PreflightOptions::default()) {
            Ok(pre) => {
                if let Some(b) = pre.blockers.first() {
                    let b = b.clone();
                    self.live_end(live, Some(b));
                    return;
                }
            }
            Err(e) => {
                let i = MeasureIssue::new("measure.abort.disconnected", None, e.to_string());
                self.live_end(live, Some(i));
                return;
            }
        }
        if live.pink() {
            let rate = plan.capture_rate_hz;
            let mut tee = self.measure.tee_lock();
            tee.on = true;
            tee.buf.clear();
            tee.skip_frames = frames_of(PINK_SKIP_MS, rate);
            tee.pink_frames = frames_of(plan.pink_ms, rate);
            drop(tee);
            live.rta_chain = Some(self.live_chain(live.setup.output));
        }
        live.run_level = None;
        match self.measure_start_plan(plan, &PreflightOptions::default()) {
            Ok(_) => live.runs = live.runs.saturating_add(1),
            Err(e) => {
                let i = MeasureIssue::new("measure.abort.generator", None, e.to_string());
                self.live_end(live, Some(i));
            }
        }
    }

    /// A finished sweep: its curve, its level.
    fn live_take(&self, live: &mut Live) {
        let Ok(text) = self.measure_result() else {
            return;
        };
        let Ok(doc) = MeasurementDoc::from_text(&text) else {
            return;
        };
        let level = doc.measurement.stimulus.level_db;
        if level.is_finite() {
            live.level = Some(level as f32);
        }
        if live.pink() {
            return;
        }
        let output = live.setup.output;
        let Some(o) = doc.measurement.outputs.iter().find(|o| o.output == output) else {
            return;
        };
        let chain = o.chain.chain.clone();
        // A sweep that played through two chains (a write during it) shows
        // neither: the next one measures the new chain.
        if chain != self.live_chain(output) {
            return;
        }
        live.measured = Some(Measured {
            resp: o.response.to_response(),
            chain,
        });
        live.seq = live.seq.saturating_add(1);
    }

    /// Feed the RTA what the push path kept.
    fn live_feed(&self, live: &mut Live) {
        let buf = std::mem::take(&mut self.measure.tee_lock().buf);
        let Some(rta) = live.rta.as_mut() else {
            return;
        };
        if buf.is_empty() {
            return;
        }
        let before = rta.frames();
        for chunk in buf.chunks(dspi_measure::rta::MAX_BLOCK) {
            if let Err(e) = rta.push(chunk) {
                log::debug!("live RTA: {e}");
            }
        }
        if rta.frames() > before {
            live.seq = live.seq.saturating_add(1);
        }
    }

    /// The proposal checked against the limits profile as its writes would be
    /// (S9): a write the core would refuse is not offered.
    fn live_judge(&self, output: u8, changes: &[MeasureLiveChange]) -> Result<()> {
        let mut inner = self.lock();
        let ch = inner.state.num_input_channels.saturating_add(output);
        for c in changes {
            crate::limits::check_band(&mut inner, ch, c.band, &c.after.params())?;
        }
        Ok(())
    }

    /// The live view and its output, for an adjustment of `output`.
    fn live_for(live: &mut Option<Live>, output: u8) -> Result<&mut Live> {
        let live = live
            .as_mut()
            .filter(|l| l.active)
            .ok_or_else(|| invalid("the live view is not running"))?;
        if live.setup.output != output {
            return Err(invalid(format!(
                "the live view measures output {}, not {}",
                u16::from(live.setup.output).saturating_add(1),
                u16::from(output).saturating_add(1)
            )));
        }
        Ok(live)
    }

    /// Keep `proposal` as the pending change (or, empty, say why) and show it.
    fn live_propose(
        &self,
        live: &mut Live,
        mut proposal: MeasureLiveProposal,
        gain: Option<f32>,
        base: Option<&Measured>,
        chain_now: &ChannelChain,
    ) -> MeasureLiveProposal {
        live.next_id = live.next_id.wrapping_add(1).max(1);
        proposal.id = live.next_id;
        if proposal.changes.is_empty() && gain.is_none() {
            proposal.issue.get_or_insert_with(|| {
                MeasureIssue::new(
                    NOTICE_NOTHING,
                    Some(proposal.output),
                    "nothing to change there",
                )
            });
            live.pending = None;
        } else {
            if let Some(m) = base {
                let next = chain_with(chain_now, &proposal.changes, gain);
                proposal.predicted = repredict(&m.resp, &m.chain, &next, &CoreSectionModel)
                    .ok()
                    .and_then(|r| curve(&r, live.setup.smoothing));
            }
            live.pending = Some(Pending {
                proposal: proposal.clone(),
                gain,
            });
        }
        self.live_publish(live);
        proposal
    }

    /// A fit on the live output's measurement in `f1..f2` with `options`
    /// (the range set here), as a proposal.
    fn live_fit(
        &self,
        output: u8,
        f1: f64,
        f2: f64,
        adjust: MeasureLiveAdjust,
        mut options: FitOptions,
    ) -> Result<MeasureLiveProposal> {
        let mut guard = self.measure.live_lock();
        let live = Self::live_for(&mut guard, output)?;
        let chain_now = self.live_chain(output);
        let gain_now = chain_now.gain_db;
        let no_frame = |what: &str| MeasureLiveProposal {
            adjust,
            output,
            label: what.to_owned(),
            gain_before_db: gain_now,
            gain_after_db: gain_now,
            ..MeasureLiveProposal::default()
        };
        let label = match adjust {
            MeasureLiveAdjust::CutPeak => format!("Cut the peak at {}", hz_words(f1 as f32)),
            _ => format!(
                "Fit {} to {}",
                hz_words(f1.min(f2) as f32),
                hz_words(f1.max(f2) as f32)
            ),
        };
        let Some(m) = live.measured_now() else {
            let mut p = no_frame(&label);
            p.issue = Some(MeasureIssue::new(
                NOTICE_NO_FRAME,
                Some(output),
                "no measured curve yet: wait for the first sweep",
            ));
            return Ok(self.live_propose(live, p, None, None, &chain_now));
        };
        // The measurement as the device is now (a change written since the
        // last sweep is in it), fitted with the chain it has now.
        let base = Measured {
            resp: repredict(&m.resp, &m.chain, &chain_now, &CoreSectionModel)?,
            chain: chain_now.clone(),
        };
        let (lo, hi) = match adjust {
            MeasureLiveAdjust::CutPeak => (f1 / CUT_HALF_SPAN, f1 * CUT_HALF_SPAN),
            _ => (f1, f2),
        };
        let Some((lo, hi)) = fit_span(&base.resp, lo, hi) else {
            let mut p = no_frame(&label);
            p.issue = Some(MeasureIssue::new(
                NOTICE_NOTHING,
                Some(output),
                "the measurement is not trusted there",
            ));
            return Ok(self.live_propose(live, p, None, None, &chain_now));
        };
        options.min_hz = lo;
        options.max_hz = hi;
        if let Some(cap) = crate::limits::profile(&mut self.lock()).and_then(|p| p.max_peq_boost_db)
        {
            options.max_boost_db = options.max_boost_db.min(f64::from(cap.max(0.0)));
        }
        let r = fit(
            &base.resp,
            &live.target,
            &chain_now,
            &CoreSectionModel,
            &options,
        );
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                let mut p = no_frame(&label);
                p.issue = Some(MeasureIssue::new(
                    NOTICE_NOTHING,
                    Some(output),
                    e.to_string(),
                ));
                return Ok(self.live_propose(live, p, None, None, &chain_now));
            }
        };
        let mut changes = fit_writes(&r, &chain_now);
        if adjust == MeasureLiveAdjust::CutPeak {
            // Cut only, one band.
            changes.retain(|c| c.after.filter_type == TYPE_PEAKING && c.after.gain < 0.0);
            changes.truncate(1);
        }
        let gain = (r.gain_cut_db < 0.0 && adjust == MeasureLiveAdjust::FitRange)
            .then(|| crate::curves::quantize_gain(gain_now + r.gain_cut_db));
        self.live_judge(output, &changes)?;
        let p = MeasureLiveProposal {
            adjust,
            output,
            label,
            changes,
            gain_before_db: gain_now,
            gain_after_db: gain.unwrap_or(gain_now),
            rms_before_db: r.rms_before_db,
            rms_after_db: r.rms_after_db,
            notes: r.notes.clone(),
            ..MeasureLiveProposal::default()
        };
        Ok(self.live_propose(live, p, gain, Some(&base), &chain_now))
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Start the live view of one output (PLAN-P5 T9, T5): short sweeps one
    /// after the other, or finite pink noise with the microphone RTA, each
    /// run under every rule of a measurement (the preflight, the ramp, the
    /// aborts; `measure_start`'s errors). The device lock is held until
    /// `measure_live_stop`. Then call `measure_live_tick` about every 50 ms
    /// (instead of `measure_tick`) and push capture whenever the session
    /// wants it (`measure_capture_push`); read `measure_live_frame`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a bad setup, a measurement
    /// or a live view already running, or a blocker (named);
    /// [`DspiError::ReadOnly`], [`DspiError::Denied`], [`DspiError::Locked`],
    /// [`DspiError::Disconnected`] as `measure_start`, with nothing sent.
    pub fn measure_live_start(&self, setup: MeasureLiveSetup) -> Result<MeasureLiveState> {
        let (session_setup, target, headphones) = check_setup(&setup)?;
        let plan = Plan::new(&session_setup)?;
        let mut guard = self.measure.live_lock();
        if guard.as_ref().is_some_and(|l| l.active) {
            return Err(invalid("busy: the live view is already running"));
        }
        if self.measure_state().active {
            return Err(invalid("busy: a measurement is running on this device"));
        }
        let rta = if setup.mode == MeasureLiveMode::PinkRta {
            Some(rta_of(&setup, &plan)?)
        } else {
            None
        };
        let output = setup.output;
        let mut live = Live {
            setup,
            session_setup,
            target,
            headphones,
            level: None,
            run_level: None,
            measured: None,
            rta_chain: None,
            rta,
            held: None,
            seq: 0,
            runs: 0,
            pending: None,
            next_id: 0,
            active: true,
            issue: None,
            last_renew: Instant::now(),
        };
        if live.pink() {
            let mut tee = self.measure.tee_lock();
            tee.on = true;
            tee.buf.clear();
            tee.skip_frames = frames_of(PINK_SKIP_MS, plan.capture_rate_hz);
            tee.pink_frames = frames_of(plan.pink_ms, plan.capture_rate_hz);
            drop(tee);
            live.rta_chain = Some(self.live_chain(output));
        }
        self.measure.live_hold.store(true, Ordering::SeqCst);
        let _ = self.measure_clear();
        match self.measure_start_plan(plan, &PreflightOptions::default()) {
            Ok(_) => {}
            Err(e) => {
                self.measure.live_hold.store(false, Ordering::SeqCst);
                self.measure.tee_lock().on = false;
                return Err(e);
            }
        }
        live.runs = 1;
        self.live_publish(&live);
        let state = live.state(self.measure_state());
        *guard = Some(live);
        Ok(state)
    }

    /// Drive the live view: renew the lock's lease, drive the run in flight
    /// (`measure_tick`), take a finished sweep's curve, feed the RTA, start
    /// the next run. Call it about every 50 ms while the live view runs, off
    /// the UI thread. A run's abort ends the live view (its reason in
    /// `issue`). Never fails.
    pub fn measure_live_tick(&self) -> MeasureLiveState {
        let mut guard = self.measure.live_lock();
        let Some(live) = guard.as_mut() else {
            return MeasureLiveState {
                session: self.measure_state(),
                ..MeasureLiveState::default()
            };
        };
        if !live.active {
            return live.state(self.measure_state());
        }
        let now = Instant::now();
        let due = now.saturating_duration_since(live.last_renew) >= RENEW_EVERY;
        if !self.measure_lock_renew(session::LOCK_REASON, session::LEASE, due) {
            let i = MeasureIssue::new("measure.abort.lock", None, "the device lock was lost");
            self.live_end(live, Some(i));
            self.live_publish(live);
            return live.state(self.measure_state());
        }
        if due {
            live.last_renew = now;
        }
        let before = live.seq;
        let session = self.measure_state();
        let st = if session.active {
            self.measure_tick()
        } else {
            session
        };
        self.live_feed(live);
        match st.phase {
            MeasurePhase::Run => {
                if st.level_db.is_some() {
                    live.run_level = st.run_level_db.or(live.run_level);
                }
            }
            MeasurePhase::Done => {
                self.live_take(live);
                let _ = self.measure_clear();
                self.live_rearm(live);
            }
            MeasurePhase::Idle => self.live_rearm(live),
            MeasurePhase::Aborted | MeasurePhase::Failed | MeasurePhase::WaitPosition => {
                let i = st
                    .issue
                    .clone()
                    .unwrap_or_else(|| MeasureIssue::new("measure.stopped", None, "the run ended"));
                self.live_end(live, Some(i));
            }
            MeasurePhase::PreRoll | MeasurePhase::Ramp | MeasurePhase::Analysing => {}
        }
        self.live_publish(live);
        let state = live.state(self.measure_state());
        if live.seq != before || !live.active {
            self.measure.listener.call(|l| l.on_measure());
        }
        state
    }

    /// What the live view shows now: the measured curve, the target levelled
    /// to it, the predicted curve of a pending (or unmeasured) change, the
    /// RTA, the pending proposal, the run's state. No I/O; safe from a
    /// listener.
    pub fn measure_live_frame(&self) -> MeasureLiveFrame {
        let session = self.measure_state();
        let mut frame = self
            .measure
            .view_lock()
            .as_ref()
            .map(|v| v.frame.clone())
            .unwrap_or_default();
        frame.session = session;
        frame
    }

    /// Where the live view stands. No I/O; safe from a listener.
    pub fn measure_live_state(&self) -> MeasureLiveState {
        let session = self.measure_state();
        let mut state = self
            .measure
            .view_lock()
            .as_ref()
            .map(|v| v.state.clone())
            .unwrap_or_default();
        state.session = session;
        state
    }

    /// Stop the live view: the run in flight stopped (one `STOP_NOW`), the
    /// lock given back. `measure_stop` does the same. Idempotent; never
    /// fails.
    pub fn measure_live_stop(&self) -> MeasureLiveState {
        let mut guard = self.measure.live_lock();
        let Some(live) = guard.as_mut() else {
            // Never a walked measurement's run: only the live view's.
            return MeasureLiveState {
                session: self.measure_state(),
                ..MeasureLiveState::default()
            };
        };
        if live.active {
            let stopped = MeasureIssue::new("measure.stopped", None, "stopped");
            self.live_end(live, Some(stopped));
        }
        live.pending = None;
        self.live_publish(live);
        let state = live.state(self.measure_state());
        drop(guard);
        self.measure.listener.call(|l| l.on_measure());
        state
    }

    /// Preview `bands` (PEQ bands of `output`, the live view's) as a pending
    /// change: the predicted curve shows at once, nothing is written until
    /// `measure_live_apply`. An empty list drops the pending change.
    ///
    /// Errors: [`DspiError::InvalidArgument`] when the live view does not
    /// measure `output`, a band index is past the output's PEQ, a value is not
    /// finite, or a type is not one the curves model; the limits profile's
    /// refusal ([`DspiError::LimitExceeded`] and kin) for a write it would
    /// refuse.
    pub fn measure_live_predict(
        &self,
        output: u8,
        bands: Vec<MeasureLiveBand>,
    ) -> Result<MeasureLiveProposal> {
        let mut guard = self.measure.live_lock();
        let live = Self::live_for(&mut guard, output)?;
        let chain_now = self.live_chain(output);
        let mut changes: Vec<MeasureLiveChange> = Vec::new();
        for b in &bands {
            let Some(before) = chain_now.peq.get(usize::from(b.band)) else {
                return Err(invalid(format!(
                    "PEQ band {} (output {} has {})",
                    b.band,
                    u16::from(output).saturating_add(1),
                    chain_now.peq.len()
                )));
            };
            if !(b.freq.is_finite() && b.q.is_finite() && b.gain.is_finite() && b.qp.is_finite()) {
                return Err(invalid("a band with a value that is not a number"));
            }
            let after = b.params();
            if after.filter_type != TYPE_FLAT && !CoreSectionModel.models(&band_spec(&after)) {
                return Err(invalid(format!("filter type {}", after.filter_type)));
            }
            changes.retain(|c| c.band != b.band);
            changes.push(change(b.band, &params_of(before), &after));
        }
        changes.retain(|c| c.before != c.after);
        if bands.is_empty() {
            live.pending = None;
            self.live_publish(live);
            return Ok(MeasureLiveProposal {
                adjust: MeasureLiveAdjust::Edit,
                output,
                gain_before_db: chain_now.gain_db,
                gain_after_db: chain_now.gain_db,
                ..MeasureLiveProposal::default()
            });
        }
        self.live_judge(output, &changes)?;
        let p = MeasureLiveProposal {
            adjust: MeasureLiveAdjust::Edit,
            output,
            label: "Live EQ change".into(),
            changes,
            gain_before_db: chain_now.gain_db,
            gain_after_db: chain_now.gain_db,
            ..MeasureLiveProposal::default()
        };
        let base = live.measured_now();
        Ok(self.live_propose(live, p, None, base.as_ref(), &chain_now))
    }

    /// "Cut this peak": one free PEQ band (flat or bypassed now) from the fit
    /// on one octave about `freq_hz`, cut only, previewed as the pending
    /// change. `issue` says why nothing is proposed (no curve yet, nothing to
    /// cut, not trusted there, no free band).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when the live view does not
    /// measure `output`; the limits profile's refusal of the band.
    pub fn measure_live_cut_peak(&self, output: u8, freq_hz: f64) -> Result<MeasureLiveProposal> {
        let headphones = self.live_headphones();
        let options = FitOptions {
            max_boost_db: 0.0,
            max_bands: Some(1),
            shelves: false,
            headroom: false,
            ..if headphones {
                FitOptions::headphones()
            } else {
                FitOptions::speakers()
            }
        };
        self.live_fit(
            output,
            freq_hz,
            freq_hz,
            MeasureLiveAdjust::CutPeak,
            options,
        )
    }

    /// "Fit this range": the free PEQ bands fitted toward the target in
    /// `f1_hz..f2_hz` (held to the measurement's trusted range), with the
    /// placement's bounds (S9: boost +3 dB speakers, +6 dB headphones, never
    /// above the limits profile's cap; cut -15 dB; Q 0.3 to 10) and a
    /// headroom cut of the output gain for its net boost; previewed as the
    /// pending change. Every band the fit lists is written on apply.
    ///
    /// Errors: as [`DspiCore::measure_live_cut_peak`].
    pub fn measure_live_fit_range(
        &self,
        output: u8,
        f1_hz: f64,
        f2_hz: f64,
    ) -> Result<MeasureLiveProposal> {
        let options = if self.live_headphones() {
            FitOptions::headphones()
        } else {
            FitOptions::speakers()
        };
        self.live_fit(output, f1_hz, f2_hz, MeasureLiveAdjust::FitRange, options)
    }

    /// Write the pending change `proposal_id` as **one undo group**
    /// (unguarded, as a manual edit; S10): the run in flight is stopped, each
    /// band through the typed setter (judged by read-only, the limits and the
    /// lock as any write), then the next run starts. Refused, with nothing
    /// written, unless the device's bands are still the ones the proposal was
    /// computed from. A write that fails puts back the ones before it.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for no such pending proposal or
    /// a chain changed since the preview; the setters' errors
    /// ([`DspiError::ReadOnly`], [`DspiError::Denied`], the limits'
    /// refusals, a transfer failure).
    pub fn measure_live_apply(&self, proposal_id: u32) -> Result<MeasureLiveState> {
        let mut guard = self.measure.live_lock();
        let live = guard
            .as_mut()
            .filter(|l| l.active)
            .ok_or_else(|| invalid("the live view is not running"))?;
        let Some(pending) = live
            .pending
            .clone()
            .filter(|p| p.proposal.id == proposal_id)
        else {
            return Err(invalid(format!("no pending change {proposal_id} to apply")));
        };
        let output = pending.proposal.output;
        let chain_now = self.live_chain(output);
        let unchanged = pending.proposal.changes.iter().all(|c| {
            chain_now
                .peq
                .get(usize::from(c.band))
                .is_some_and(|b| *b == band_spec(&c.before.params()))
        }) && chain_now.gain_db == pending.proposal.gain_before_db;
        if !unchanged {
            live.pending = None;
            self.live_publish(live);
            return Err(invalid(format!(
                "{NOTICE_CHAIN_CHANGED}: output {}'s EQ changed since the preview; preview again",
                u16::from(output).saturating_add(1)
            )));
        }
        // Read-only and the role are refused before anything is stopped or
        // sent.
        self.lock().writable_conn()?;
        // The run in flight would measure two chains.
        if self.measure_state().active {
            self.measure_stop_session();
        }
        let _ = self.measure_clear();
        let ch = self.lock().state.num_input_channels.saturating_add(output);
        self.begin_group(pending.proposal.label.clone());
        let mut written = 0usize;
        let mut result = Ok(());
        for c in &pending.proposal.changes {
            if let Err(e) = self.set_filter(ch, c.band, c.after.params()) {
                result = Err(e);
                break;
            }
            written = written.saturating_add(1);
        }
        if result.is_ok()
            && let Some(g) = pending.gain
        {
            match self.set_output_gain(output, g) {
                Ok(()) => written = written.saturating_add(1),
                Err(e) => result = Err(e),
            }
        }
        self.end_group();
        if let Err(e) = result {
            if written > 0 && self.undo().is_err() {
                log::debug!("live apply: the partial change could not be undone");
            }
            live.pending = None;
            self.live_rearm(live);
            self.live_publish(live);
            return Err(e);
        }
        // The next ramp starts lower by any boost the change added.
        let after = self.live_chain(output);
        let rise = max_rise_db(&chain_now, &after, &CoreSectionModel) as f32;
        if let Some(l) = live.level.as_mut() {
            *l = (*l - rise.max(0.0)).max(-120.0);
        }
        live.pending = None;
        if let Some(r) = live.rta.as_mut() {
            r.reset();
        }
        self.live_rearm(live);
        self.live_publish(live);
        Ok(live.state(self.measure_state()))
    }

    /// Drop the pending change (nothing is written).
    pub fn measure_live_discard(&self) -> MeasureLiveState {
        let mut guard = self.measure.live_lock();
        let Some(live) = guard.as_mut() else {
            return self.measure_live_state();
        };
        live.pending = None;
        self.live_publish(live);
        live.state(self.measure_state())
    }

    /// Hold (`true`) the RTA's spectrum for comparison, or drop the held
    /// copy (`false`).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when no live RTA runs, or there
    /// is no spectrum to hold yet.
    pub fn measure_live_hold(&self, on: bool) -> Result<MeasureLiveState> {
        let mut guard = self.measure.live_lock();
        let live = guard
            .as_mut()
            .ok_or_else(|| invalid("the live view is not running"))?;
        let rta = live
            .rta
            .as_ref()
            .ok_or_else(|| invalid("the live view has no RTA (sweep mode)"))?;
        if on {
            live.held = Some(
                rta.spectrum()
                    .ok_or_else(|| invalid("no spectrum to hold yet"))?,
            );
        } else {
            live.held = None;
        }
        self.live_publish(live);
        Ok(live.state(self.measure_state()))
    }

    /// The RTA's averaging preset; the average starts again (the held copy
    /// is kept).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when no live RTA runs.
    pub fn measure_live_averaging(
        &self,
        averaging: MeasureRtaAveraging,
    ) -> Result<MeasureLiveState> {
        let mut guard = self.measure.live_lock();
        let live = guard
            .as_mut()
            .ok_or_else(|| invalid("the live view is not running"))?;
        if live.rta.is_none() {
            return Err(invalid("the live view has no RTA (sweep mode)"));
        }
        live.setup.averaging = averaging;
        let plan = Plan::new(&live.session_setup)?;
        live.rta = Some(rta_of(&live.setup, &plan)?);
        self.live_publish(live);
        Ok(live.state(self.measure_state()))
    }
}

impl DspiCore {
    fn live_headphones(&self) -> bool {
        self.measure
            .live_lock()
            .as_ref()
            .is_some_and(|l| l.headphones)
    }
}
