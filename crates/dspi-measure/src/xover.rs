//! Crossover choice and the predicted acoustic sum (WP P5XO; PLAN-P5 section
//! 1.4 steps 3, 5 and 6, T10).
//!
//! - [`de_embed`]: a driver's own response is the measurement divided by the
//!   chain it was measured with (crossover, PEQ, output gain, delay), computed
//!   through the core's [`SectionModel`] so the maths and the graph share one
//!   filter model (OD-P5c). Its validity mask: trusted only where the
//!   measurement's SNR minus the chain's attenuation is at least 20 dB
//!   (review RC12).
//! - [`crossover_corner`]: per transition, candidate corners on a 1/12-octave
//!   grid within a window around the current corner (half an octave by
//!   default), **only where both drivers are trusted one octave each side**
//!   (RC12) and never below the protective high-pass rule (RC4, S6); the
//!   current family and order kept; levels cut only and re-normalised so the
//!   way that needs the least cut keeps its gain (S9, S-f; no gain is ever
//!   raised and repeated tunes do not ratchet down); delay and polarity
//!   re-searched per candidate; each candidate scored by the predicted
//!   **programme** sum (programme feed included, RC1) against the target
//!   through the transition; at most [`MAX_EVALUATIONS_PER_TRANSITION`] cost
//!   evaluations a transition.
//!
//!   The per-candidate delay and polarity are chosen by the two ways' **phase
//!   agreement through their overlap** (weighted where both contribute), not
//!   by the sum against the target: a magnitude-only cost bends the timing to
//!   cancel a driver's resonance with a half-cancelled crossover (seen on the
//!   synthetic three-way), which no summed check would then confirm. The
//!   crossover filters' own phases are included, so an LR2 or BW2 pair is
//!   judged with the inversion it needs. [`crate::align::align`] (no crossover
//!   change) keeps the flattest-sum rule of T2.
//! - [`predict_sum`] and [`summed_check`]: the mandatory summed check (RC5f).
//!   A summed generator run bypasses the matrix, so its prediction uses the
//!   generator's `invert_mask` ([`SumMode::Generator`]); programme audio uses
//!   the feed ([`SumMode::Programme`]). A measured sum that differs from the
//!   prediction by more than [`SUMMED_CHECK_MAX_RMS_DB`] RMS through the
//!   transition (one octave each side) fails, and the caller withdraws every
//!   timing write.
//!
//! Pure and deterministic: identical input gives identical quantised
//! proposals (RC10); no panics on any input.

use crate::align::{
    AlignOptions, CostKind, Cx, Edge, MIN_WINDOW_POINTS, PairCtx, Point, REFINE_ITERS, Transition,
    Way, check_chain, check_count, check_options, common_grid, compose, db_to_lin, feed_factor,
    half_window_ms, response_to_complex, transition_corner, transition_ways,
};
use crate::types::{BandSpec, ChannelChain, Coeffs, MeasureError, Response, SectionModel, WayRole};
use serde::{Deserialize, Serialize};

/// The most cost evaluations the corner search spends on one transition.
pub const MAX_EVALUATIONS_PER_TRANSITION: u32 = 5000;
/// The corner search's step, octaves.
pub const CORNER_STEP_OCTAVES: f64 = 1.0 / 12.0;
/// The default search window each side of the current corner, octaves.
pub const DEFAULT_WINDOW_OCTAVES: f64 = 0.5;
/// The widest window the user may choose, octaves each side.
pub const MAX_WINDOW_OCTAVES: f64 = 2.0;
/// A de-embedded point is trusted when the measurement's SNR minus the
/// chain's attenuation there is at least this, dB (RC12).
pub const DEEMBED_MIN_SNR_DB: f64 = 20.0;
/// The summed check fails above this RMS difference through a transition, dB
/// (RC5f).
pub const SUMMED_CHECK_MAX_RMS_DB: f64 = 1.5;
/// The default largest level cut of a way, dB.
pub const DEFAULT_MAX_LEVEL_CUT_DB: f64 = 12.0;
/// Coarse delay steps each side per candidate, at most.
const XO_COARSE_STEPS: usize = 48;
/// A way's passband for its level: where its crossover is within this of 0 dB.
const PASSBAND_DB: f64 = -0.5;
/// Below this the chain is treated as silent and the driver unknown there.
const SILENT_DB: f64 = -120.0;
/// The most biquad sections one bank may expand to.
const MAX_SECTIONS: usize = 1024;
/// A candidate must beat the best so far by this much to replace it, dB, so
/// ties keep the candidate nearest the current corner.
const TIE_DB: f64 = 1e-9;

/// The protective high-pass a way must keep (S6, review RC4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HighpassRule {
    /// The lowest corner, Hz.
    pub min_hz: f32,
    /// The least slope, dB/octave.
    pub min_slope_db: u8,
}

impl HighpassRule {
    /// The default for a tweeter without a limits profile: 1 kHz, 12 dB/oct.
    pub const HIGH: HighpassRule = HighpassRule {
        min_hz: 1000.0,
        min_slope_db: 12,
    };
    /// The default for a mid without a limits profile: 80 Hz, 12 dB/oct.
    pub const MID: HighpassRule = HighpassRule {
        min_hz: 80.0,
        min_slope_db: 12,
    };

    /// The rule of a role when no limits profile gives one.
    pub fn default_for(role: WayRole) -> Option<HighpassRule> {
        match role {
            WayRole::High => Some(Self::HIGH),
            WayRole::Mid => Some(Self::MID),
            WayRole::Full | WayRole::Sub | WayRole::Low => None,
        }
    }

    /// Whether `chain` keeps this high-pass: an active band (PEQ or
    /// crossover) of a high-pass type at least `min_slope_db` steep with its
    /// corner at or above `min_hz` (as `dspi-session`'s `highpass_kept`).
    pub fn kept_by(&self, chain: &ChannelChain) -> bool {
        chain.peq.iter().chain(&chain.crossover).any(|b| {
            !b.bypass
                && b.freq.is_finite()
                && b.freq >= self.min_hz - 1e-3
                && highpass_slope(b.filter_type).is_some_and(|s| s >= self.min_slope_db)
        })
    }
}

/// The slope of a high-pass filter type, dB/octave; `None` for any other type.
/// The same table as `dspi-session`'s `limits::highpass_slope`.
fn highpass_slope(filter_type: u8) -> Option<u8> {
    match filter_type {
        5 => Some(12),
        13 => Some(6),
        _ => dspi_proto::xover::meta(filter_type)
            .filter(|m| m.high_pass)
            .map(|m| m.order.saturating_mul(6)),
    }
}

/// A protective high-pass from the limits profile for one output; it replaces
/// the role's default.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HighpassOverride {
    /// The output.
    pub output: u8,
    /// Its rule.
    pub rule: HighpassRule,
}

/// Bounds and choices of the crossover search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XoConstraints {
    /// The window each side of the current corner, octaves (0 to
    /// [`MAX_WINDOW_OCTAVES`]); half an octave by default.
    pub window_octaves: f64,
    /// "Keep the crossover, align only": every corner and every level stays;
    /// only delay and polarity are proposed.
    pub keep_corners: bool,
    /// Keep every output gain.
    pub keep_levels: bool,
    /// The largest cut one way's level may get, dB (0 to 40).
    pub max_level_cut_db: f64,
    /// The limits profile's high-pass rules; outputs not listed use their
    /// role's default ([`HighpassRule::default_for`]).
    pub highpass: Vec<HighpassOverride>,
    /// The delay range and the "keep the delays" choice.
    pub align: AlignOptions,
}

impl Default for XoConstraints {
    fn default() -> Self {
        XoConstraints {
            window_octaves: DEFAULT_WINDOW_OCTAVES,
            keep_corners: false,
            keep_levels: false,
            max_level_cut_db: DEFAULT_MAX_LEVEL_CUT_DB,
            highpass: Vec::new(),
            align: AlignOptions::default(),
        }
    }
}

impl XoConstraints {
    fn rule_for(&self, output: u8, role: WayRole) -> Option<HighpassRule> {
        self.highpass
            .iter()
            .find(|h| h.output == output)
            .map(|h| h.rule)
            .or_else(|| HighpassRule::default_for(role))
    }

    fn check(&self) -> Result<(), MeasureError> {
        let ok = self.window_octaves.is_finite()
            && (0.0..=MAX_WINDOW_OCTAVES).contains(&self.window_octaves)
            && self.max_level_cut_db.is_finite()
            && (0.0..=40.0).contains(&self.max_level_cut_db)
            && self.highpass.len() <= 64
            && self
                .highpass
                .iter()
                .all(|h| h.rule.min_hz.is_finite() && h.rule.min_hz >= 0.0);
        if !ok {
            return Err(MeasureError::Invalid(
                "crossover constraints out of range".into(),
            ));
        }
        check_options(&self.align)
    }
}

/// A driver's own response: a way's measurement divided by its measured
/// chain ([`de_embed`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
    /// The output it was measured on.
    pub output: u8,
    /// Its role.
    pub role: WayRole,
    /// The chain it was measured with (the starting point of a proposal).
    pub chain: ChannelChain,
    /// The driver's response with phase (the measured phase minus the chain's)
    /// on the run's timing reference; `valid` is the de-embedding's mask.
    pub response: Response,
    /// The coarse arrival without the chain's delay, ms.
    pub arrival_ms: f64,
}

/// De-embed a way: its measurement divided by its measured chain, with the
/// validity mask "SNR minus the chain's attenuation at least 20 dB" (RC12).
///
/// Errors: [`MeasureError::Invalid`] for a response without phase, a size
/// mismatch, a non-finite chain, a band `model` cannot model or a bad sample
/// rate.
pub fn de_embed(way: &Way, model: &dyn SectionModel) -> Result<Driver, MeasureError> {
    check_chain(&way.chain)?;
    let fs = check_model(model)?;
    check_modelled(&way.chain, model, way.output)?;
    let freqs = way.response.grid.freqs()?;
    let n = freqs.len();
    if way.snr_db.len() != n {
        return Err(MeasureError::Invalid(format!(
            "an SNR of {} points on a grid of {n}",
            way.snr_db.len()
        )));
    }
    let (h, ok) = response_to_complex(&way.response, n)?;
    let peq = sections(&way.chain.peq, model)?;
    let xo = sections(&way.chain.crossover, model)?;
    let gain = db_to_lin(f64::from(way.chain.gain_db));
    let d = f64::from(way.chain.delay_ms);
    let mut mag = Vec::with_capacity(n);
    let mut phase = Vec::with_capacity(n);
    let mut valid = Vec::with_capacity(n);
    for (i, f) in freqs.iter().enumerate() {
        let shape =
            cascade_at(&peq, *f, fs).and_then(|p| cascade_at(&xo, *f, fs).map(|x| p.mul(x)));
        let meas = h.get(i).copied().unwrap_or(Cx::ZERO);
        let trusted_meas = ok.get(i).copied().unwrap_or(false);
        let snr = way.snr_db.get(i).copied().unwrap_or(f64::NAN);
        let full = shape.map(|s| s.scale(gain).mul(Cx::delay(*f, d)));
        let drv = match (shape, full) {
            (Some(s), Some(c)) if s.db() > SILENT_DB => meas.div(c).map(|x| (x, s)),
            _ => None,
        };
        match drv {
            Some((x, s)) => {
                let att = (-s.db()).max(0.0);
                let trusted = trusted_meas && snr.is_finite() && snr - att >= DEEMBED_MIN_SNR_DB;
                let m = x.db();
                // The measured phase minus the chain's, so the driver's phase
                // stays as continuous as the measurement's.
                let p_meas = way
                    .response
                    .phase_deg
                    .as_ref()
                    .and_then(|p| p.get(i).copied())
                    .filter(|p| p.is_finite() && p.abs() < 1e9)
                    .unwrap_or(0.0);
                let p = p_meas - s.arg().to_degrees() + 360.0 * f * d / 1000.0;
                let p_ok = p.is_finite() && p.abs() < 1e9;
                mag.push(if m.is_finite() {
                    m.clamp(-200.0, 300.0)
                } else {
                    -200.0
                });
                phase.push(if p_ok { p } else { 0.0 });
                valid.push(trusted && p_ok && (-200.0..300.0).contains(&m));
            }
            None => {
                mag.push(-200.0);
                phase.push(0.0);
                valid.push(false);
            }
        }
    }
    let arrival = way.arrival_ms - d;
    Ok(Driver {
        output: way.output,
        role: way.role,
        chain: way.chain.clone(),
        response: Response {
            grid: way.response.grid,
            mag_db: mag,
            phase_deg: Some(phase),
            valid: Some(valid),
            spl: way.response.spl,
        },
        arrival_ms: if arrival.is_finite() { arrival } else { 0.0 },
    })
}

/// The proposal for one output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XoOutput {
    /// The output.
    pub output: u8,
    /// Its chain now.
    pub chain_before: ChannelChain,
    /// The proposed chain: crossover corners, output gain (0.1 dB grid, never
    /// raised), delay (0.0001 ms grid) and the feed's net polarity changed;
    /// everything else as before.
    pub chain: ChannelChain,
    /// The level change of this way, dB (0 or negative).
    pub level_change_db: f64,
}

/// What the search did for one transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XoTransition {
    /// The lower way's output.
    pub low_output: u8,
    /// The upper way's output.
    pub high_output: u8,
    /// The corner now (geometric mean of the low-pass and the high-pass), Hz.
    pub corner_before_hz: f64,
    /// The proposed corner, Hz.
    pub corner_hz: f64,
    /// Every candidate corner evaluated, Hz, in the order evaluated.
    pub candidates_hz: Vec<f64>,
    /// Cost evaluations spent (at most [`MAX_EVALUATIONS_PER_TRANSITION`]).
    pub evaluations: u32,
    /// The current chain's cost through the window, dB RMS against the
    /// target; `None` when the current corner is not a valid candidate.
    pub cost_before_db: Option<f64>,
    /// The proposal's cost, dB; `None` when nothing was evaluated.
    pub cost_db: Option<f64>,
    /// The upper way's new delay minus the lower way's, ms.
    pub relative_delay_ms: f64,
    /// The upper way's polarity is flipped relative to the lower way's.
    pub flipped: bool,
    /// Why the transition was left as it is, if it was.
    pub kept_reason: Option<String>,
}

/// The crossover search's proposal ([`crossover_corner`]). Nothing is written:
/// the core judges and quantises every write again (OD-P5c, S9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XoProposal {
    /// One entry per driver, in the order given.
    pub outputs: Vec<XoOutput>,
    /// One entry per transition, in the order given.
    pub transitions: Vec<XoTransition>,
    /// Remarks in words.
    pub notes: Vec<String>,
}

/// One driver's precomputed data.
struct Prep {
    h: Vec<Cx>,
    trusted: Vec<bool>,
    peq: Vec<Cx>,
}

/// The crossover corner search (T10 step 5; see the module comment).
///
/// `drivers` from [`de_embed`], all on one grid; `transitions` index them;
/// `target` is any response (interpolated onto the drivers' grid; only its
/// shape counts). Transitions are searched in the order given, each one with
/// the corners already chosen for the ones before.
///
/// Errors: [`MeasureError::Invalid`] for malformed input (sizes, grids, a
/// transition out of range or without a crossover pair and corner, a band the
/// model cannot model, constraints out of range, a delay the alignment needs
/// outside the range).
pub fn crossover_corner(
    drivers: &[Driver],
    transitions: &[Transition],
    target: &Response,
    constraints: &XoConstraints,
    model: &dyn SectionModel,
) -> Result<XoProposal, MeasureError> {
    check_count(drivers.len())?;
    constraints.check()?;
    if transitions.len() > drivers.len().saturating_mul(2) {
        return Err(MeasureError::Invalid(format!(
            "{} transitions for {} drivers",
            transitions.len(),
            drivers.len()
        )));
    }
    let fs = check_model(model)?;
    let grid = common_grid(drivers.iter().map(|d| &d.response))?;
    let freqs = grid.freqs()?;
    let n = freqs.len();
    let tgt = target_on(target, &freqs)?;
    let mut prep = Vec::with_capacity(drivers.len());
    for d in drivers {
        check_chain(&d.chain)?;
        check_modelled(&d.chain, model, d.output)?;
        let (h, trusted) = response_to_complex(&d.response, n)?;
        let secs = sections(&d.chain.peq, model)?;
        let peq = freqs
            .iter()
            .map(|f| cascade_at(&secs, *f, fs).unwrap_or(Cx::ZERO))
            .collect();
        prep.push(Prep { h, trusted, peq });
    }
    let mut work: Vec<ChannelChain> = drivers.iter().map(|d| d.chain.clone()).collect();
    let mut notes = Vec::new();

    // Levels: cut only, re-normalised per joined set so its loudest way keeps
    // its gain (S9, S-f).
    let mut level_change = vec![0f64; drivers.len()];
    if !constraints.keep_levels && !constraints.keep_corners {
        let mut level: Vec<Option<f64>> = Vec::with_capacity(drivers.len());
        for ((d, p), c) in drivers.iter().zip(&prep).zip(&work) {
            let xo = sections(&c.crossover, model)?;
            let k = feed_factor(c).scale(db_to_lin(f64::from(c.gain_db)));
            let mut sum = 0.0;
            let mut cnt = 0usize;
            for (i, f) in freqs.iter().enumerate() {
                let (Some(true), Some(h), Some(pq), Some(Some(t))) = (
                    p.trusted.get(i).copied(),
                    p.h.get(i),
                    p.peq.get(i),
                    tgt.get(i).copied(),
                ) else {
                    continue;
                };
                let Some(x) = cascade_at(&xo, *f, fs) else {
                    continue;
                };
                if x.db() < PASSBAND_DB {
                    continue;
                }
                // The way's level without its crossover's shape: the driver,
                // its PEQ, its gain and its feed.
                let v = h.mul(*pq).mul(k).db() - t;
                if v.is_finite() {
                    sum += v;
                    cnt = cnt.saturating_add(1);
                }
            }
            if cnt == 0 {
                notes.push(format!(
                    "output {}: no trusted passband to judge its level; its gain is kept",
                    d.output
                ));
                level.push(None);
            } else {
                level.push(Some(sum / cnt as f64));
            }
        }
        let comp = components(drivers.len(), transitions);
        for k in comp
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            let members: Vec<usize> = (0..drivers.len())
                .filter(|i| comp.get(*i) == Some(&k))
                .collect();
            if members.len() < 2 {
                continue;
            }
            // The quietest way (relative to the target) keeps its gain.
            let min_level = members
                .iter()
                .filter_map(|i| level.get(*i).copied().flatten())
                .fold(f64::INFINITY, f64::min);
            if !min_level.is_finite() {
                continue;
            }
            for i in members {
                let (Some(Some(l)), Some(c), Some(lc), Some(d)) = (
                    level.get(i).copied(),
                    work.get_mut(i),
                    level_change.get_mut(i),
                    drivers.get(i),
                ) else {
                    continue;
                };
                let mut delta = min_level - l;
                if delta < -constraints.max_level_cut_db {
                    notes.push(format!(
                        "output {}: its level would need a cut of {:.1} dB; held to {:.1} dB",
                        d.output, -delta, constraints.max_level_cut_db
                    ));
                    delta = -constraints.max_level_cut_db;
                }
                if delta <= -0.05 {
                    let g = f64::from(c.gain_db);
                    let mut q = quantize_gain(g + delta).min(g);
                    if q < g - constraints.max_level_cut_db {
                        // Rounding past the cap: the grid value inside it.
                        q = ((g - constraints.max_level_cut_db) * 10.0).ceil() / 10.0;
                    }
                    *lc = q - g;
                    // In the chain's checked range (|gain| <= 200 dB) minus at
                    // most 40 dB, so the narrowing is exact to f32.
                    c.gain_db = q as f32;
                }
            }
        }
    }

    let mut edges = Vec::with_capacity(transitions.len());
    let mut out_t = Vec::with_capacity(transitions.len());
    for t in transitions {
        let (da, db) = transition_ways(drivers, t)?;
        let (Some(pa), Some(pb)) = (prep.get(t.low), prep.get(t.high)) else {
            return Err(MeasureError::Invalid("transition out of range".into()));
        };
        let (Some(ca), Some(cb)) = (work.get(t.low).cloned(), work.get(t.high).cloned()) else {
            return Err(MeasureError::Invalid("transition out of range".into()));
        };
        let info = transition_corner(&ca, &cb, t.corner_hz)?;
        let delta_now = f64::from(cb.delay_ms) - f64::from(ca.delay_ms);
        let rule_a = constraints.rule_for(da.output, da.role);
        let rule_b = constraints.rule_for(db.output, db.role);
        // Candidates: (corner, low way's chain, high way's chain).
        let steps_each_side = if constraints.keep_corners || info.slots.is_none() {
            0
        } else {
            // Bounded by MAX_WINDOW_OCTAVES * 12 = 24 by the constraints' check.
            (constraints.window_octaves / CORNER_STEP_OCTAVES + 1e-9).floor() as i32
        };
        let mut order = vec![0i32];
        for k in 1..=steps_each_side {
            order.push(k.saturating_neg());
            order.push(k);
        }
        let mut allowed: Vec<(f64, ChannelChain, ChannelChain, f64, f64, i32)> = Vec::new();
        for k in order {
            let factor = (f64::from(k) * CORNER_STEP_OCTAVES).exp2();
            let (mut a2, mut b2) = (ca.clone(), cb.clone());
            let (lo, hi, corner) = match info.slots {
                Some((ls, hs)) => {
                    let (Some(lp), Some(hp)) = (a2.crossover.get_mut(ls), b2.crossover.get_mut(hs))
                    else {
                        continue;
                    };
                    // The current corner stays exactly as it is.
                    if k != 0 {
                        lp.freq = round_corner(f64::from(lp.freq) * factor);
                        hp.freq = round_corner(f64::from(hp.freq) * factor);
                    }
                    let (l, h) = (f64::from(lp.freq), f64::from(hp.freq));
                    if l >= 0.45 * fs || h >= 0.45 * fs || !(l > 0.0 && h > 0.0) {
                        continue;
                    }
                    (l.min(h), l.max(h), (l * h).sqrt())
                }
                None => (info.corner_hz, info.corner_hz, info.corner_hz),
            };
            let (span_lo, span_hi) = (lo / 2.0, hi * 2.0);
            if !span_covered(&freqs, span_lo, span_hi, &pa.trusted, &pb.trusted) {
                continue;
            }
            if rule_a.is_some_and(|r| !r.kept_by(&a2)) || rule_b.is_some_and(|r| !r.kept_by(&b2)) {
                continue;
            }
            allowed.push((corner, a2, b2, span_lo, span_hi, k));
        }
        if allowed.is_empty() {
            let reason = format!(
                "outputs {} and {}: no corner inside both drivers' trusted range one octave each side{}; the crossover and this pair's timing are kept",
                da.output,
                db.output,
                if rule_a.is_some() || rule_b.is_some() {
                    " that keeps the protective high-pass"
                } else {
                    ""
                }
            );
            notes.push(reason.clone());
            edges.push(Edge {
                low: t.low,
                high: t.high,
                delta_ms: delta_now,
                flip: false,
            });
            out_t.push(XoTransition {
                low_output: da.output,
                high_output: db.output,
                corner_before_hz: info.corner_hz,
                corner_hz: info.corner_hz,
                candidates_hz: Vec::new(),
                evaluations: 0,
                cost_before_db: None,
                cost_db: None,
                relative_delay_ms: delta_now,
                flipped: false,
                kept_reason: Some(reason),
            });
            continue;
        }
        let win_lo = allowed.iter().map(|c| c.3).fold(f64::INFINITY, f64::min);
        let win_hi = allowed.iter().map(|c| c.4).fold(0.0, f64::max);
        let idx: Vec<usize> = freqs
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                **f >= win_lo
                    && **f <= win_hi
                    && pa.trusted.get(*i).copied().unwrap_or(false)
                    && pb.trusted.get(*i).copied().unwrap_or(false)
                    && tgt.get(*i).copied().flatten().is_some()
            })
            .map(|(i, _)| i)
            .collect();
        let start = {
            let s = da.arrival_ms - db.arrival_ms;
            if s.is_finite() && s.abs() <= crate::align::MAX_DELAY_BOUND_MS {
                s
            } else {
                delta_now
            }
        };
        let budget = MAX_EVALUATIONS_PER_TRANSITION.saturating_sub(1) as usize;
        let per = budget.checked_div(allowed.len()).unwrap_or(0);
        let refine = REFINE_ITERS.min(per / 4);
        let refine = if refine < 2 { 0 } else { refine };
        // Per candidate: 2·(2·steps + 1) + refine for the timing, one to
        // judge it; one more for the current chain.
        let steps = XO_COARSE_STEPS.min(per.saturating_sub(refine).saturating_sub(3) / 4);
        let mut evaluations = 0u32;
        let mut cost_before = None;
        let mut best: Option<(usize, crate::align::PairFit)> = None;
        let mut cands = Vec::with_capacity(allowed.len());
        for (ci, (corner, a2, b2, _, _, k)) in allowed.iter().enumerate() {
            let ctx = pair_ctx(&idx, &freqs, &tgt, pa, pb, a2, b2, model, fs)?;
            if *k == 0 {
                let c0 = PairCtx {
                    points: pair_points(
                        &idx,
                        &freqs,
                        &tgt,
                        pa,
                        pb,
                        drivers.get(t.low).map(|d| &d.chain),
                        drivers.get(t.high).map(|d| &d.chain),
                        model,
                        fs,
                    )?,
                    kind: CostKind::Target,
                };
                cost_before = Some(c0.cost(delta_now, false));
                evaluations = evaluations.saturating_add(1);
            }
            // Timing by the ways' phase agreement through the overlap; the
            // candidate is then judged by its sum against the target.
            let mut fit = if constraints.align.keep_delays {
                ctx.search(delta_now, 0.0, 0, 0)
            } else {
                ctx.search(start, half_window_ms(*corner), steps, refine)
            };
            let judged = PairCtx {
                points: ctx.points,
                kind: CostKind::Target,
            };
            fit.cost = judged.cost(fit.delta_ms, fit.flip);
            evaluations = evaluations.saturating_add(fit.evals).saturating_add(1);
            cands.push(*corner);
            if best.is_none_or(|(_, b)| fit.cost < b.cost - TIE_DB) {
                best = Some((ci, fit));
            }
        }
        let Some((bi, fit)) = best else {
            return Err(MeasureError::Invalid("no candidate".into()));
        };
        let Some((corner, a2, b2, _, _, _)) = allowed.get(bi) else {
            return Err(MeasureError::Invalid("no candidate".into()));
        };
        if let Some(c) = work.get_mut(t.low) {
            c.crossover.clone_from(&a2.crossover);
        }
        if let Some(c) = work.get_mut(t.high) {
            c.crossover.clone_from(&b2.crossover);
        }
        edges.push(Edge {
            low: t.low,
            high: t.high,
            delta_ms: fit.delta_ms,
            flip: fit.flip,
        });
        out_t.push(XoTransition {
            low_output: da.output,
            high_output: db.output,
            corner_before_hz: info.corner_hz,
            corner_hz: *corner,
            candidates_hz: cands,
            evaluations,
            cost_before_db: cost_before,
            cost_db: Some(fit.cost),
            relative_delay_ms: fit.delta_ms,
            flipped: fit.flip,
            kept_reason: None,
        });
    }

    let current: Vec<f64> = drivers
        .iter()
        .map(|d| f64::from(d.chain.delay_ms))
        .collect();
    let (delays, flips) = compose(&current, &edges, &constraints.align, &mut notes)?;
    let mut outputs = Vec::with_capacity(drivers.len());
    for (i, d) in drivers.iter().enumerate() {
        let mut c = work.get(i).cloned().unwrap_or_else(|| d.chain.clone());
        if !constraints.align.keep_delays {
            c.delay_ms = delays.get(i).copied().unwrap_or(c.delay_ms);
        }
        c.feed.inverted ^= flips.get(i).copied().unwrap_or(false);
        outputs.push(XoOutput {
            output: d.output,
            chain_before: d.chain.clone(),
            chain: c,
            level_change_db: level_change.get(i).copied().unwrap_or(0.0),
        });
    }
    Ok(XoProposal {
        outputs,
        transitions: out_t,
        notes,
    })
}

#[allow(clippy::too_many_arguments)]
fn pair_ctx(
    idx: &[usize],
    freqs: &[f64],
    tgt: &[Option<f64>],
    pa: &Prep,
    pb: &Prep,
    a: &ChannelChain,
    b: &ChannelChain,
    model: &dyn SectionModel,
    fs: f64,
) -> Result<PairCtx, MeasureError> {
    Ok(PairCtx {
        points: pair_points(idx, freqs, tgt, pa, pb, Some(a), Some(b), model, fs)?,
        kind: CostKind::Phase,
    })
}

/// The window's points for chains `a` and `b` (their delays left out: the
/// search adds the relative delay).
#[allow(clippy::too_many_arguments)]
fn pair_points(
    idx: &[usize],
    freqs: &[f64],
    tgt: &[Option<f64>],
    pa: &Prep,
    pb: &Prep,
    a: Option<&ChannelChain>,
    b: Option<&ChannelChain>,
    model: &dyn SectionModel,
    fs: f64,
) -> Result<Vec<Point>, MeasureError> {
    let (Some(a), Some(b)) = (a, b) else {
        return Ok(Vec::new());
    };
    let xa = sections(&a.crossover, model)?;
    let xb = sections(&b.crossover, model)?;
    let ka = feed_factor(a).scale(db_to_lin(f64::from(a.gain_db)));
    let kb = feed_factor(b).scale(db_to_lin(f64::from(b.gain_db)));
    let mut pts = Vec::with_capacity(idx.len());
    for i in idx {
        let (Some(f), Some(Some(t)), Some(ha), Some(hb), Some(qa), Some(qb)) = (
            freqs.get(*i),
            tgt.get(*i).copied(),
            pa.h.get(*i),
            pb.h.get(*i),
            pa.peq.get(*i),
            pb.peq.get(*i),
        ) else {
            continue;
        };
        let (Some(xa), Some(xb)) = (cascade_at(&xa, *f, fs), cascade_at(&xb, *f, fs)) else {
            continue;
        };
        let ca = ha.mul(*qa).mul(xa).mul(ka);
        let cb = hb.mul(*qb).mul(xb).mul(kb);
        if ca.is_finite() && cb.is_finite() {
            pts.push(Point::new(*f, ca, cb, t));
        }
    }
    Ok(pts)
}

/// Whether every grid point in `[lo, hi]` is trusted in both drivers, the
/// span lies on the grid, and it holds enough points.
fn span_covered(freqs: &[f64], lo: f64, hi: f64, ta: &[bool], tb: &[bool]) -> bool {
    let (Some(first), Some(last)) = (freqs.first(), freqs.last()) else {
        return false;
    };
    // Half a 1/96-octave step of slack at the grid's ends.
    if lo < first * 0.996 || hi > last * 1.004 {
        return false;
    }
    let mut count = 0usize;
    for (i, f) in freqs.iter().enumerate() {
        if *f < lo || *f > hi {
            continue;
        }
        if !(ta.get(i).copied().unwrap_or(false) && tb.get(i).copied().unwrap_or(false)) {
            return false;
        }
        count = count.saturating_add(1);
    }
    count >= MIN_WINDOW_POINTS
}

/// Joined sets of drivers: a label per driver.
fn components(n: usize, transitions: &[Transition]) -> Vec<usize> {
    let mut label: Vec<usize> = (0..n).collect();
    // Relabel until stable; n and the transitions are small and bounded.
    for _ in 0..n {
        let mut changed = false;
        for t in transitions {
            let (Some(a), Some(b)) = (label.get(t.low).copied(), label.get(t.high).copied()) else {
                continue;
            };
            let m = a.min(b);
            for l in label.iter_mut() {
                if *l == a.max(b) && a != b {
                    *l = m;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    label
}

/// What a summed prediction reproduces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum SumMode {
    /// A summed generator run: the generator bypasses the matrix, each output
    /// is inverted when its bit in `invert_mask` is set (RC1).
    Generator {
        /// Bit `n` inverts output `n`.
        invert_mask: u32,
    },
    /// Programme audio: each output's feed gain and net polarity.
    Programme,
}

/// The acoustic sum of `drivers` played through `chains` (one per driver,
/// current or proposed), as a summed generator run or programme audio hears
/// it, on the drivers' grid with phase.
///
/// A point is valid when every driver is trusted there, or the untrusted
/// drivers' contributions are at least 20 dB below the sum.
///
/// Errors: [`MeasureError::Invalid`] for sizes that differ, different grids,
/// a band the model cannot model, an output past bit 31 in generator mode.
pub fn predict_sum(
    drivers: &[Driver],
    chains: &[ChannelChain],
    mode: SumMode,
    model: &dyn SectionModel,
) -> Result<Response, MeasureError> {
    check_count(drivers.len())?;
    if chains.len() != drivers.len() {
        return Err(MeasureError::Invalid(format!(
            "{} chains for {} drivers",
            chains.len(),
            drivers.len()
        )));
    }
    let fs = check_model(model)?;
    let grid = common_grid(drivers.iter().map(|d| &d.response))?;
    let freqs = grid.freqs()?;
    let n = freqs.len();
    let mut contrib: Vec<(Vec<Cx>, Vec<bool>)> = Vec::with_capacity(drivers.len());
    for (d, c) in drivers.iter().zip(chains) {
        check_chain(c)?;
        check_modelled(c, model, d.output)?;
        let sign = match mode {
            SumMode::Programme => feed_factor(c),
            SumMode::Generator { invert_mask } => {
                let bit = 1u32
                    .checked_shl(u32::from(d.output))
                    .filter(|_| d.output < 32)
                    .ok_or_else(|| {
                        MeasureError::Invalid(format!("output {} has no invert bit", d.output))
                    })?;
                if invert_mask & bit != 0 {
                    Cx { re: -1.0, im: 0.0 }
                } else {
                    Cx::ONE
                }
            }
        };
        let (h, trusted) = response_to_complex(&d.response, n)?;
        let peq = sections(&c.peq, model)?;
        let xo = sections(&c.crossover, model)?;
        let k = sign.scale(db_to_lin(f64::from(c.gain_db)));
        let dl = f64::from(c.delay_ms);
        let v: Vec<Cx> = freqs
            .iter()
            .zip(&h)
            .map(
                |(f, x)| match (cascade_at(&peq, *f, fs), cascade_at(&xo, *f, fs)) {
                    (Some(p), Some(q)) => x.mul(p).mul(q).mul(k).mul(Cx::delay(*f, dl)),
                    _ => Cx::ZERO,
                },
            )
            .collect();
        contrib.push((v, trusted));
    }
    let mut mag = Vec::with_capacity(n);
    let mut wrapped = Vec::with_capacity(n);
    let mut valid = Vec::with_capacity(n);
    for i in 0..n {
        let mut s = Cx::ZERO;
        for (v, _) in &contrib {
            s = s.add(v.get(i).copied().unwrap_or(Cx::ZERO));
        }
        let floor = s.norm_sqr() * 0.01;
        let ok = s.is_finite()
            && contrib.iter().all(|(v, t)| {
                t.get(i).copied().unwrap_or(false)
                    || v.get(i).is_some_and(|x| x.norm_sqr() <= floor)
            });
        let m = s.db();
        mag.push(if m.is_finite() {
            m.clamp(-200.0, 400.0)
        } else {
            -200.0
        });
        wrapped.push(if s.is_finite() {
            s.arg().to_degrees()
        } else {
            0.0
        });
        valid.push(ok && m.is_finite());
    }
    Ok(Response {
        grid,
        mag_db: mag,
        phase_deg: Some(unwrap_deg(&wrapped)),
        valid: Some(valid),
        spl: drivers.iter().all(|d| d.response.spl),
    })
}

/// The result of [`summed_check`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummedCheck {
    /// RMS difference between measured and predicted, dB, through the window.
    pub rms_db: f64,
    /// Points compared.
    pub points: usize,
    /// `rms_db` is at most [`SUMMED_CHECK_MAX_RMS_DB`]: the walked run's
    /// timing is confirmed.
    pub passed: bool,
}

/// The mandatory summed check (RC5f): the measured summed run against the
/// prediction from the walked run, through one octave each side of
/// `corner_hz`, on the points both trust. `level_offset_db` is added to the
/// prediction (the summed run's generator level minus the walked run's).
///
/// Errors: [`MeasureError::Invalid`] for different grids, sizes or a bad
/// corner or offset; [`MeasureError::NoSignal`] when fewer than 12 points are
/// trusted in both.
pub fn summed_check(
    measured: &Response,
    predicted: &Response,
    corner_hz: f64,
    level_offset_db: f64,
) -> Result<SummedCheck, MeasureError> {
    if measured.grid != predicted.grid {
        return Err(MeasureError::Invalid(
            "measured and predicted sums on different grids".into(),
        ));
    }
    if !(corner_hz.is_finite() && corner_hz > 0.0 && level_offset_db.is_finite()) {
        return Err(MeasureError::Invalid(format!(
            "corner {corner_hz} Hz, offset {level_offset_db} dB"
        )));
    }
    let freqs = measured.grid.freqs()?;
    let n = freqs.len();
    for r in [measured, predicted] {
        if r.mag_db.len() != n || r.valid.as_ref().is_some_and(|v| v.len() != n) {
            return Err(MeasureError::Invalid(format!(
                "a sum of {} points on a grid of {n}",
                r.mag_db.len()
            )));
        }
    }
    let ok = |r: &Response, i: usize| {
        r.valid
            .as_ref()
            .and_then(|v| v.get(i).copied())
            .unwrap_or(true)
            && r.mag_db.get(i).is_some_and(|m| m.is_finite())
    };
    let mut ss = 0.0;
    let mut cnt = 0usize;
    for (i, f) in freqs.iter().enumerate() {
        if *f < corner_hz / 2.0 || *f > corner_hz * 2.0 || !ok(measured, i) || !ok(predicted, i) {
            continue;
        }
        let (Some(m), Some(p)) = (measured.mag_db.get(i), predicted.mag_db.get(i)) else {
            continue;
        };
        let e = m - (p + level_offset_db);
        ss += e * e;
        cnt = cnt.saturating_add(1);
    }
    if cnt < MIN_WINDOW_POINTS {
        return Err(MeasureError::NoSignal(format!(
            "{cnt} trusted points through the transition at {corner_hz:.0} Hz (at least {MIN_WINDOW_POINTS})"
        )));
    }
    let rms = (ss / cnt as f64).sqrt();
    let rms = if rms.is_finite() { rms } else { f64::MAX };
    Ok(SummedCheck {
        rms_db: rms,
        points: cnt,
        passed: rms <= SUMMED_CHECK_MAX_RMS_DB,
    })
}

// ---------------------------------------------------------------------------
// Filter evaluation through the section model.
// ---------------------------------------------------------------------------

fn check_model(model: &dyn SectionModel) -> Result<f64, MeasureError> {
    let fs = model.sample_rate();
    if fs.is_finite() && (8_000.0..=768_000.0).contains(&fs) {
        Ok(fs)
    } else {
        Err(MeasureError::Invalid(format!("sample rate {fs} Hz")))
    }
}

fn check_modelled(
    chain: &ChannelChain,
    model: &dyn SectionModel,
    output: u8,
) -> Result<(), MeasureError> {
    if let Some(b) = chain
        .peq
        .iter()
        .chain(&chain.crossover)
        .find(|b| !b.bypass && b.filter_type != 0 && !model.models(b))
    {
        return Err(MeasureError::Invalid(format!(
            "output {output}: a band of type {} cannot be modelled",
            b.filter_type
        )));
    }
    Ok(())
}

/// Every section of a bank, bounded.
pub(crate) fn sections(
    bands: &[BandSpec],
    model: &dyn SectionModel,
) -> Result<Vec<Coeffs>, MeasureError> {
    let mut out = Vec::new();
    for b in bands {
        if b.bypass || b.filter_type == 0 {
            continue;
        }
        out.extend(model.sections(b));
        if out.len() > MAX_SECTIONS {
            return Err(MeasureError::Invalid(format!(
                "a bank of more than {MAX_SECTIONS} sections"
            )));
        }
    }
    Ok(out)
}

/// The cascade's complex response at `f`; `None` at or above the Nyquist
/// frequency or for a non-finite result.
pub(crate) fn cascade_at(secs: &[Coeffs], f: f64, fs: f64) -> Option<Cx> {
    if !(f.is_finite() && f > 0.0 && f < fs / 2.0) {
        return None;
    }
    let w = std::f64::consts::TAU * f / fs;
    let z1 = Cx::polar(1.0, -w);
    let z2 = Cx::polar(1.0, -2.0 * w);
    let mut h = Cx::ONE;
    for c in secs {
        let num = Cx { re: c.b0, im: 0.0 }
            .add(z1.scale(c.b1))
            .add(z2.scale(c.b2));
        let den = Cx::ONE.add(z1.scale(c.a1)).add(z2.scale(c.a2));
        h = h.mul(num.div(den)?);
    }
    h.is_finite().then_some(h)
}

/// `target` on `freqs`: directly when on the same grid, else interpolated
/// linearly in log frequency and held at its ends; `None` where the target
/// has no finite value.
fn target_on(target: &Response, freqs: &[f64]) -> Result<Vec<Option<f64>>, MeasureError> {
    let tf = target.grid.freqs()?;
    if target.mag_db.len() != tf.len() {
        return Err(MeasureError::Invalid(format!(
            "a target of {} points on a grid of {}",
            target.mag_db.len(),
            tf.len()
        )));
    }
    if tf.len() == freqs.len() && tf.iter().zip(freqs).all(|(a, b)| a == b) {
        return Ok(target
            .mag_db
            .iter()
            .map(|v| v.is_finite().then_some(*v))
            .collect());
    }
    let pts: Vec<(f64, f64)> = tf
        .iter()
        .zip(&target.mag_db)
        .filter(|(_, v)| v.is_finite())
        .map(|(f, v)| (f.log2(), *v))
        .collect();
    let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) else {
        return Err(MeasureError::Invalid(
            "a target without a finite value".into(),
        ));
    };
    Ok(freqs
        .iter()
        .map(|f| {
            let x = f.log2();
            let k = pts.partition_point(|p| p.0 < x);
            let v = match (k.checked_sub(1).and_then(|j| pts.get(j)), pts.get(k)) {
                (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0),
                (Some(a), Some(_)) => a.1,
                (None, _) => first.1,
                (Some(_), None) => last.1,
            };
            Some(v)
        })
        .collect())
}

/// Remove the 360° jumps of a wrapped phase, degrees.
fn unwrap_deg(wrapped: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(wrapped.len());
    let mut offset = 0.0;
    let mut prev: Option<f64> = None;
    for w in wrapped {
        if let Some(p) = prev {
            let d = w - p;
            if d > 180.0 {
                offset -= 360.0;
            } else if d < -180.0 {
                offset += 360.0;
            }
        }
        prev = Some(*w);
        out.push(w + offset);
    }
    out
}

/// A corner on a 0.1 Hz grid, as the f32 a band carries.
fn round_corner(hz: f64) -> f32 {
    let v = (hz * 10.0).round() / 10.0;
    // Corners are bounded by the chain's checked finite f32 times at most
    // 2^2, so the narrowing stays finite or becomes infinity, which the
    // caller's range check refuses.
    v as f32
}

/// A gain on the device's 0.1 dB grid (as `dspi-core`'s `quantize_gain`).
fn quantize_gain(db: f64) -> f64 {
    let v = (db * 10.0).round() / 10.0;
    let v = f64::from(v as f32);
    if v == 0.0 { 0.0 } else { v }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5xo;
