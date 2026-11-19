//! The bounded PEQ fit toward a target (WP P5FIT, PLAN-P5 T4, OD-P5l).
//!
//! [`fit`] places peaking (and, by choice, second-order shelving) bands in a
//! channel's **free** PEQ bands so the measured response follows the target in
//! a frequency range, inside hard bounds:
//!
//! - every band's frequency inside the fit's range (and below 0.45 of the
//!   model's sample rate), its Q inside the Q range, its gain between the max
//!   cut and the max boost (S9: +3 dB speakers, +6 dB headphones; the caller
//!   lowers it to the limits profile's `max_peq_boost_db`);
//! - the **sum** of the fitted bands never boosts more than the max boost at
//!   any point of the grid, so two bands cannot stack past the cap;
//! - only the band indices the caller allows (by default the free ones: flat
//!   or bypassed), never more than the band budget;
//! - no point the `valid` mask distrusts is fitted; a dip that varies across
//!   positions by more than the spread limit is left alone (the cost there is
//!   the EQ change itself, so the fit is pulled towards no change); a dip
//!   deeper than 10 dB is filled by at most the max boost;
//! - the error after is never above the error before: otherwise no change is
//!   returned.
//!
//! **The level** (WP P5TRUST, review P5-C2 follow-up 3): [`FitLevel::Free`]
//! (the default) fits the error's shape in the range and leaves its level
//! free, so a broad excess over the whole range is not seen;
//! [`FitLevel::Anchored`] holds the target at a level the caller gives (the
//! target levelled to the measurement in a reference band outside the range,
//! [`reference_offset_db`]), so the range's level against the rest is fitted
//! too and a broad band or a shelf means what it says. With
//! [`FitOptions::leave_out_dips`] (the auto-tune's and the live view's
//! choice; off in [`FitOptions::speakers`] and [`FitOptions::headphones`]), a
//! point more than the max boost plus [`DIP_LEAVE_OUT_MARGIN_DB`] below the
//! levelled target is a dip no band may fill: it is left out of the cost and
//! of the level (only the EQ change there counts, so no band deepens it), so
//! it does not pull the level down and make every other point read as a peak.
//!
//! The method: the measured response (optionally smoothed) is levelled to the
//! target by the median difference in the range (which dips are deep or vary
//! is judged there); the cost is the squared error with its mean taken out
//! (the level is free, so no band is spent moving it, and a broad boost
//! never pays), plus the squared EQ change at dips left alone (against the
//! EQ's own level over the fitted points while the level is free, WP
//! P5TRUST: cutting everything around a dip boosts it); then greedy
//! placement on the
//! largest error area, a short refinement of the new band, a bounded
//! coordinate descent with box constraints over every fitted band, until the
//! RMS error target, the band budget or the evaluation cap. Every candidate is
//! evaluated through the caller's [`SectionModel`] (the core's own cascade,
//! OD-P5c), so the prediction is the curve the user sees.
//!
//! The result is **quantised** as the core's setters quantise (gain on the
//! device's 0.1 dB grid, halves away from zero, as `dspi-core`'s
//! `quantize_gain` in `curves.rs`), frequency to 0.1 Hz and Q to 0.01 (the
//! wire carries `f32`; these steps make proposals stable and readable), held
//! to the bounds again and predicted again from the quantised bands.
//! Identical input gives identical quantised bands on one target (RC10): no
//! hash-map order, no time, no randomness, a fixed evaluation cap.

use crate::response::{SmoothingDomain, smooth_magnitudes};
use crate::types::{
    BandSpec, ChannelChain, Coeffs, GRID_MAX_HZ, GRID_MIN_HZ, MeasureError, Response, SectionModel,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests_wp_p5fit;

/// The firmware's flat (off) filter type.
const TYPE_FLAT: u8 = 0;
/// Peaking (`FilterType::Peaking`).
pub const TYPE_PEAKING: u8 = 1;
/// Low shelf, second order (`FilterType::LowShelf`).
pub const TYPE_LOW_SHELF: u8 = 2;
/// High shelf, second order (`FilterType::HighShelf`).
pub const TYPE_HIGH_SHELF: u8 = 3;

/// The most PEQ bands a chain may hold for the fit (the device has 10).
pub const MAX_PEQ_BANDS: usize = 32;
/// The largest evaluation cap a caller may ask for.
pub const MAX_EVALUATIONS: u32 = 100_000;
/// The default evaluation cap: 10 bands on the standard grid stay well under
/// 200 ms in a native release build.
pub const DEFAULT_EVALUATIONS: u32 = 6_000;
/// The hard ceiling on any boost, dB, whatever the options say.
pub const HARD_MAX_BOOST_DB: f64 = 12.0;
/// The hard floor on any cut, dB (as a positive number).
pub const HARD_MAX_CUT_DB: f64 = 24.0;

/// Highest band frequency as a fraction of the model's sample rate (the
/// firmware clamps crossover corners at the same fraction).
const MAX_FREQ_FRACTION: f64 = 0.45;
/// Q range of the second-order shelves the fit may use (above about 0.9 a
/// shelf overshoots; below 0.5 it is too shallow to be useful).
const SHELF_Q: (f64, f64) = (0.5, 0.9);
/// Points outside the range are checked for the boost cap at this decimation.
const OUTSIDE_DECIMATION: usize = 8;
/// Fewer usable points than this in the range: nothing to fit.
const MIN_POINTS: usize = 8;
/// Regions tried per greedy step (largest area first).
const REGIONS_PER_STEP: usize = 2;
/// Evaluations spent refining a candidate band on its own.
const PROBE_EVALUATIONS: u32 = 40;
/// Evaluations of the joint refinement after each added band.
const STEP_EVALUATIONS: u32 = 600;
/// Weight of the squared boost in the cost: with the level free, a broad
/// boost and a cut of the rest fit alike; this small preference for cuts keeps
/// the headroom (and fills a dip about 5 % short).
const BOOST_WEIGHT: f64 = 0.05;
/// Slack on the boost cap for rounding in the cascade, dB.
const BOOST_SLACK_DB: f64 = 1e-6;
/// With [`FitOptions::leave_out_dips`], a point this far below the levelled
/// target beyond the max boost is left out, dB (WP P5TRUST).
pub const DIP_LEAVE_OUT_MARGIN_DB: f64 = 1.0;

// -- WP P5TRUST --
/// How the fit sets the target's level.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum FitLevel {
    /// Free in the fit's range: the error's mean over the fitted points is
    /// taken out, so only the error's shape is fitted.
    #[default]
    Free,
    /// The target moved by `offset_db` (the measurement's level against the
    /// target, e.g. [`reference_offset_db`] in a reference band) and held
    /// there: the error is fitted as it is, its level included.
    Anchored {
        /// The target's level shift, dB (finite, at most 200 dB).
        offset_db: f64,
    },
}

/// The measurement's level against the target in `low_hz..=high_hz`: the
/// mean difference over the points trusted in both (as the graph levels its
/// target); `None` when fewer than 8 such points lie there or a grid is bad.
pub fn reference_offset_db(
    measured: &Response,
    target: &Response,
    low_hz: f64,
    high_hz: f64,
) -> Option<f64> {
    let freqs = measured.grid.freqs().ok()?;
    let tfreqs = target.grid.freqs().ok()?;
    if measured.mag_db.len() != freqs.len() || target.mag_db.len() != tfreqs.len() {
        return None;
    }
    let (mut sum, mut n) = (0.0f64, 0usize);
    for (i, (f, m)) in freqs.iter().zip(&measured.mag_db).enumerate() {
        let ok = measured
            .valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false));
        if !ok || *f < low_hz || *f > high_hz || !m.is_finite() {
            continue;
        }
        if let Some(t) = interp(target, &tfreqs, *f) {
            sum += m - t;
            n = n.saturating_add(1);
        }
    }
    let o = sum / n.max(1) as f64;
    (n >= MIN_POINTS && o.is_finite()).then_some(o)
}

/// What the fit may do. [`FitOptions::speakers`] and
/// [`FitOptions::headphones`] give OD-P5l's defaults; the caller lowers the
/// boost to the limits profile's cap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FitOptions {
    /// Lowest frequency fitted, Hz.
    pub min_hz: f64,
    /// Highest frequency fitted, Hz.
    pub max_hz: f64,
    /// Fractional-octave smoothing of the measurement before the fit
    /// (`Some(24)` is 1/24 octave, 1..=96); `None` fits it as given.
    pub smoothing: Option<u16>,
    /// Largest boost, dB (0 to 12): per band and for the sum of the bands.
    pub max_boost_db: f64,
    /// Largest cut per band, dB, as a positive number (0 to 24).
    pub max_cut_db: f64,
    /// Lowest Q.
    pub q_min: f64,
    /// Highest Q.
    pub q_max: f64,
    /// A fitted band with a smaller |gain|, dB, is dropped.
    pub min_gain_db: f64,
    /// The PEQ band indices the fit may write; `None` is every free band
    /// (flat or bypassed). A band given here that is in use is **replaced**.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bands: Option<Vec<u8>>,
    /// At most this many bands; `None` is as many as are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bands: Option<u8>,
    /// Low and high shelves (second order) may be used at the range's ends.
    pub shelves: bool,
    /// The spread across positions or reseats per point of the measurement's
    /// grid, dB (P5CAL's `position`); `None` for a single position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread_db: Option<Vec<f64>>,
    /// A dip whose spread is above this, dB, is left alone (3 dB).
    pub spread_limit_db: f64,
    /// A dip deeper than this below the target, dB, is filled by at most the
    /// max boost (10 dB).
    pub deep_dip_db: f64,
    /// Offset the fit's net boost by an equal output-gain cut.
    pub headroom: bool,
    // -- WP P5TRUST --
    /// The target's level: free in the range, or anchored ([`FitLevel`]).
    #[serde(default)]
    pub level: FitLevel,
    /// Leave out the dips more than the max boost plus
    /// [`DIP_LEAVE_OUT_MARGIN_DB`] below the levelled target (see the module
    /// docs).
    #[serde(default)]
    pub leave_out_dips: bool,
    /// With `shelves`: low shelves only, never a high shelf at the range's
    /// top (it would tilt everything above the range, which the fit does not
    /// see).
    #[serde(default)]
    pub no_high_shelf: bool,
    /// Stop once the RMS error in the range is at or below this, dB.
    pub target_rms_db: f64,
    /// The evaluation cap (at most [`MAX_EVALUATIONS`]).
    pub max_evaluations: u32,
}

impl FitOptions {
    /// Speakers: 20 Hz to 300 Hz, boost +3 dB, cut -15 dB, Q 0.3 to 10,
    /// 1/24-octave smoothing, shelves allowed, headroom kept, the level free
    /// and every dip in the cost (a dip of up to 10 dB is filled by up to the
    /// max boost). The auto-tune and the live view's "fit this range" anchor
    /// the level where they have a reference band and leave out the dips
    /// past the cap (WP P5TRUST).
    pub fn speakers() -> Self {
        FitOptions {
            min_hz: 20.0,
            max_hz: 300.0,
            smoothing: Some(24),
            max_boost_db: 3.0,
            max_cut_db: 15.0,
            q_min: 0.3,
            q_max: 10.0,
            min_gain_db: 0.5,
            bands: None,
            max_bands: None,
            shelves: true,
            spread_db: None,
            spread_limit_db: 3.0,
            deep_dip_db: 10.0,
            headroom: true,
            level: FitLevel::Free,
            leave_out_dips: false,
            no_high_shelf: false,
            target_rms_db: 0.5,
            max_evaluations: DEFAULT_EVALUATIONS,
        }
    }

    /// Headphones: 20 Hz to 10 kHz, boost +6 dB, 1/12-octave smoothing, the
    /// rest as [`FitOptions::speakers`].
    pub fn headphones() -> Self {
        FitOptions {
            max_hz: 10_000.0,
            smoothing: Some(12),
            max_boost_db: 6.0,
            ..Self::speakers()
        }
    }
}

impl Default for FitOptions {
    fn default() -> Self {
        Self::speakers()
    }
}

/// One band the fit proposes: write `band` to PEQ band `index`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FittedBand {
    /// The PEQ band index (0-based).
    pub index: u8,
    /// The band's quantised settings.
    pub band: BandSpec,
}

/// What [`fit`] proposes. Nothing is written by this crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FitResult {
    /// The bands to write, ascending by index; empty when the fit changes
    /// nothing. A released band that was in use and is not needed is in the
    /// list as a flat band (type 0): it must be written too, the prediction
    /// has it taken out.
    pub bands: Vec<FittedBand>,
    /// The measured response with the proposal applied: the replaced bands
    /// taken out, the fitted ones and the headroom cut put in (phase too when
    /// the measurement has it). On the measurement's grid.
    pub predicted: Response,
    /// RMS error to the levelled target over the trusted points in the
    /// range, before, dB. The target's level is free: it is moved to the
    /// mean of the fitted points' error, so a fit never spends a band on
    /// moving the level.
    pub rms_before_db: f64,
    /// The same after (quantised bands, without the headroom cut, which only
    /// moves the level), dB. Never above [`FitResult::rms_before_db`].
    pub rms_after_db: f64,
    /// The largest |error| after, dB, over the points the fit may change
    /// (trusted, in the range, not left alone).
    pub max_error_after_db: f64,
    /// The level the target was moved to after the fit, dB: the mean of
    /// the (smoothed) predicted response minus the target over the points
    /// the fit may change. (Dips and their spread are judged against the
    /// median before the fit.)
    pub target_offset_db: f64,
    /// The output-gain change for headroom, dB (<= 0, on the 0.1 dB grid).
    pub gain_cut_db: f32,
    /// Response evaluations spent (at most the cap).
    pub evaluations: u32,
    /// What was done and why, in words.
    pub notes: Vec<String>,
}

/// Fit the allowed PEQ bands of `chain` so `measured` follows `target` in
/// the options' range (see the module docs for every bound).
///
/// `measured` is what the output did with `chain` in place; `target` may be
/// on another grid (it is interpolated in log frequency; points outside it or
/// next to an untrusted target point are not fitted). Errors:
/// [`MeasureError::Invalid`] for a bad grid, lengths that do not match, a
/// non-finite value, options out of range, a band index outside the chain, or
/// a band in use (to be replaced) that the model cannot represent;
/// [`MeasureError::NoSignal`] when fewer than 8 trusted points lie in the
/// range.
pub fn fit(
    measured: &Response,
    target: &Response,
    chain: &ChannelChain,
    model: &dyn SectionModel,
    options: &FitOptions,
) -> Result<FitResult, MeasureError> {
    let setup = Setup::new(measured, target, chain, model, options)?;
    Ok(setup.run())
}

/// A band's parameters during the fit: log2 frequency, log2 Q, gain dB.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cand {
    ty: u8,
    lf: f64,
    lq: f64,
    g: f64,
}

impl Cand {
    fn spec(&self) -> BandSpec {
        BandSpec {
            filter_type: self.ty,
            bypass: false,
            freq: self.lf.exp2() as f32,
            q: self.lq.exp2() as f32,
            gain: self.g as f32,
            qp: 0.0,
        }
    }
}

/// Box bounds of the fitted parameters (log2 Hz, log2 Q, dB).
#[derive(Debug, Clone, Copy)]
struct Bounds {
    lf: (f64, f64),
    lq_peak: (f64, f64),
    lq_shelf: Option<(f64, f64)>,
    g: (f64, f64),
}

impl Bounds {
    fn lq(&self, ty: u8) -> (f64, f64) {
        if ty == TYPE_PEAKING {
            self.lq_peak
        } else {
            self.lq_shelf.unwrap_or(self.lq_peak)
        }
    }

    fn clamp(&self, c: Cand) -> Cand {
        let lq = self.lq(c.ty);
        Cand {
            ty: c.ty,
            lf: c.lf.clamp(self.lf.0, self.lf.1),
            lq: c.lq.clamp(lq.0, lq.1),
            g: c.g.clamp(self.g.0, self.g.1),
        }
    }
}

/// How a point enters the cost.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// Fitted: the cost is the squared error.
    Normal,
    /// A dip that varies across positions: the cost is the squared EQ change.
    LeaveAlone,
    /// Not trusted, or outside the range: only the boost cap applies.
    Ignored,
}

/// A point's sines and cosines at the model's rate (the biquad response).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Trig {
    c1: f64,
    s1: f64,
    c2: f64,
    s2: f64,
}

impl Trig {
    pub(crate) fn at(freq: f64, fs: f64) -> Self {
        let w = std::f64::consts::TAU * freq / fs;
        Trig {
            c1: w.cos(),
            s1: w.sin(),
            c2: (2.0 * w).cos(),
            s2: (2.0 * w).sin(),
        }
    }

    fn parts(&self, c: &Coeffs) -> (f64, f64, f64, f64) {
        let nr = c.b0 + c.b1 * self.c1 + c.b2 * self.c2;
        let ni = -(c.b1 * self.s1 + c.b2 * self.s2);
        let dr = 1.0 + c.a1 * self.c1 + c.a2 * self.c2;
        let di = -(c.a1 * self.s1 + c.a2 * self.s2);
        (nr, ni, dr, di)
    }

    /// |H|^2 of one section; 1 where the denominator vanishes (as the graph,
    /// `dspi_proto::dsp`).
    fn power(&self, c: &Coeffs) -> f64 {
        let (nr, ni, dr, di) = self.parts(c);
        let den = dr * dr + di * di;
        if den < 1e-20 {
            1.0
        } else {
            (nr * nr + ni * ni) / den
        }
    }

    /// Phase of one section, radians.
    fn phase(&self, c: &Coeffs) -> f64 {
        let (nr, ni, dr, di) = self.parts(c);
        ni.atan2(nr) - di.atan2(dr)
    }
}

/// The dB response of a cascade at the given points into `out`; false if any
/// value is not finite.
pub(crate) fn cascade_db(sections: &[Coeffs], pts: &[Trig], out: &mut Vec<f64>) -> bool {
    out.clear();
    for t in pts {
        let p: f64 = sections.iter().map(|s| t.power(s)).product();
        let db = 10.0 * p.max(1e-20).log10();
        if !db.is_finite() {
            return false;
        }
        out.push(db);
    }
    true
}

/// Everything checked and prepared once.
struct Setup<'a> {
    model: &'a dyn SectionModel,
    opts: &'a FitOptions,
    measured: &'a Response,
    freqs: Vec<f64>,
    trig: Vec<Trig>,
    /// The level is free (the mean error taken out), not anchored.
    free_level: bool,
    /// Per grid point, trusted and in the range: the (smoothed) measurement
    /// minus the levelled target, as the chain stands.
    d0: Vec<Option<f64>>,
    /// Per grid point: `d0` without the replaced bands, deep dips clamped to
    /// the max boost (what the new bands work against).
    e0: Vec<Option<f64>>,
    kind: Vec<Kind>,
    /// The replaced bands' dB per grid point (zero where none).
    old_db: Vec<f64>,
    /// The replaced bands' sections (for the phase).
    old_sections: Vec<Coeffs>,
    offset: f64,
    bounds: Bounds,
    allowed: Vec<u8>,
    /// The allowed bands that are in use now: their response is taken out
    /// of the prediction, so the proposal must overwrite or clear each.
    replaced_idx: Vec<u8>,
    budget: usize,
    notes: Vec<String>,
}

fn invalid(s: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(s.into())
}

fn finite_all(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn band_no(i: u8) -> u16 {
    u16::from(i).saturating_add(1)
}

impl<'a> Setup<'a> {
    fn new(
        measured: &'a Response,
        target: &Response,
        chain: &ChannelChain,
        model: &'a dyn SectionModel,
        opts: &'a FitOptions,
    ) -> Result<Self, MeasureError> {
        let freqs = measured.grid.freqs()?;
        let n = freqs.len();
        if measured.mag_db.len() != n || !finite_all(&measured.mag_db) {
            return Err(invalid(
                "measured magnitudes do not match the grid or are not finite",
            ));
        }
        if let Some(p) = &measured.phase_deg
            && (p.len() != n || !finite_all(p))
        {
            return Err(invalid("measured phase does not match the grid"));
        }
        if let Some(v) = &measured.valid
            && v.len() != n
        {
            return Err(invalid(
                "the measured validity mask does not match the grid",
            ));
        }
        let tfreqs = target.grid.freqs()?;
        if target.mag_db.len() != tfreqs.len() || !finite_all(&target.mag_db) {
            return Err(invalid(
                "the target does not match its grid or is not finite",
            ));
        }
        if let Some(v) = &target.valid
            && v.len() != tfreqs.len()
        {
            return Err(invalid(
                "the target's validity mask does not match its grid",
            ));
        }
        if let Some(s) = &opts.spread_db
            && (s.len() != n || !finite_all(s))
        {
            return Err(invalid("the spread does not match the measurement's grid"));
        }
        let fs = model.sample_rate();
        if !(fs.is_finite() && (8_000.0..=768_000.0).contains(&fs)) {
            return Err(invalid(format!("model sample rate {fs}")));
        }
        let bounds = check_options(opts, fs)?;
        if chain.peq.len() > MAX_PEQ_BANDS {
            return Err(invalid(format!(
                "{} PEQ bands (at most {MAX_PEQ_BANDS})",
                chain.peq.len()
            )));
        }
        for ty in [TYPE_PEAKING, TYPE_LOW_SHELF, TYPE_HIGH_SHELF] {
            let probe = BandSpec {
                filter_type: ty,
                bypass: false,
                freq: 100.0,
                q: 1.0,
                gain: 1.0,
                qp: 0.0,
            };
            if (ty == TYPE_PEAKING || opts.shelves) && !model.models(&probe) {
                return Err(invalid(format!("the model does not know filter type {ty}")));
            }
        }

        // The allowed bands: given, or the free ones.
        let mut allowed: Vec<u8> = match &opts.bands {
            Some(b) => b.clone(),
            None => (0u8..)
                .zip(chain.peq.iter())
                .filter(|(_, b)| b.bypass || b.filter_type == TYPE_FLAT)
                .map(|(i, _)| i)
                .collect(),
        };
        allowed.sort_unstable();
        allowed.dedup();
        let mut notes = Vec::new();
        let mut replaced: Vec<BandSpec> = Vec::new();
        let mut replaced_idx: Vec<u8> = Vec::new();
        for &i in &allowed {
            let Some(b) = chain.peq.get(usize::from(i)) else {
                return Err(invalid(format!(
                    "PEQ band {} (the chain has {})",
                    band_no(i),
                    chain.peq.len()
                )));
            };
            if b.bypass || b.filter_type == TYPE_FLAT {
                continue;
            }
            if !model.models(b) {
                return Err(invalid(format!(
                    "PEQ band {} has a filter type the model does not know",
                    band_no(i)
                )));
            }
            notes.push(format!(
                "PEQ band {} is in use and may be replaced",
                band_no(i)
            ));
            replaced.push(*b);
            replaced_idx.push(i);
        }
        if allowed.is_empty() {
            notes.push("no free PEQ band".to_string());
        }
        let budget = opts
            .max_bands
            .map_or(allowed.len(), |m| allowed.len().min(usize::from(m)));

        let trig: Vec<Trig> = freqs.iter().map(|&f| Trig::at(f, fs)).collect();
        let old_sections: Vec<Coeffs> = replaced.iter().flat_map(|b| model.sections(b)).collect();
        let mut old_db = Vec::with_capacity(n);
        if !cascade_db(&old_sections, &trig, &mut old_db) {
            return Err(invalid("a replaced band's response is not finite"));
        }

        // The measurement smoothed, the target on its grid.
        let valid_at = |i: usize| -> bool {
            measured
                .valid
                .as_ref()
                .is_none_or(|v| v.get(i).copied().unwrap_or(false))
        };
        // The measurement smoothed as energy (review P5-A: the crate's one
        // smoothing, `response::smooth_magnitudes`, in the power domain). A
        // fit must not see a deep narrow null as a dip to fill, and a dB mean
        // would deepen it; the graph's dB smoothing is `response::smooth`.
        let meas = match opts.smoothing {
            Some(n) => smooth_magnitudes(
                &measured.mag_db,
                measured.valid.as_deref(),
                measured.grid.points_per_octave,
                f64::from(n),
                SmoothingDomain::Power,
            ),
            None => measured.mag_db.clone(),
        };
        let in_range = |f: f64| f >= opts.min_hz && f <= opts.max_hz && f < fs * 0.5;
        let mut diffs: Vec<f64> = Vec::new();
        let mut d_raw: Vec<Option<f64>> = Vec::with_capacity(n);
        for (i, (&f, m)) in freqs.iter().zip(meas.iter()).enumerate() {
            let d = if in_range(f) && valid_at(i) {
                interp(target, &tfreqs, f).map(|t| m - t)
            } else {
                None
            };
            if let Some(d) = d {
                diffs.push(d);
            }
            d_raw.push(d);
        }
        if diffs.len() < MIN_POINTS {
            return Err(MeasureError::NoSignal(format!(
                "{} trusted points in {} to {}",
                diffs.len(),
                hz(opts.min_hz),
                hz(opts.max_hz)
            )));
        }
        let free_level = opts.level == FitLevel::Free;
        let offset = match opts.level {
            FitLevel::Free => median(&mut diffs),
            FitLevel::Anchored { offset_db } => offset_db,
        };
        let dip_floor = -(opts.max_boost_db + DIP_LEAVE_OUT_MARGIN_DB);

        let spread_at = |i: usize| -> f64 {
            opts.spread_db
                .as_ref()
                .and_then(|s| s.get(i).copied())
                .unwrap_or(0.0)
        };
        let mut e0 = Vec::with_capacity(n);
        let mut d0 = Vec::with_capacity(n);
        let mut kind = Vec::with_capacity(n);
        let mut alone: Vec<(f64, f64)> = Vec::new();
        let mut deep: Vec<(f64, f64)> = Vec::new();
        let mut out_dips: Vec<(f64, f64)> = Vec::new();
        for (i, (d, (&f, &o))) in d_raw
            .iter()
            .zip(freqs.iter().zip(old_db.iter()))
            .enumerate()
        {
            let Some(d) = d else {
                d0.push(None);
                e0.push(None);
                kind.push(Kind::Ignored);
                continue;
            };
            let d = d - offset;
            let e = d - o;
            d0.push(Some(d));
            if e < 0.0 && spread_at(i) > opts.spread_limit_db {
                kind.push(Kind::LeaveAlone);
                e0.push(Some(e));
                alone.push((f, e));
            } else if opts.leave_out_dips && e < dip_floor {
                // -- WP P5TRUST -- a dip no band may fill: out of the cost
                // and of the level, only the EQ change there counts.
                kind.push(Kind::LeaveAlone);
                e0.push(Some(e));
                out_dips.push((f, e));
            } else if e < -opts.deep_dip_db {
                kind.push(Kind::Normal);
                e0.push(Some(e.max(-opts.max_boost_db)));
                deep.push((f, e));
            } else {
                kind.push(Kind::Normal);
                e0.push(Some(e));
            }
        }
        let ppo = measured.grid.points_per_octave;
        for (f, e) in runs_minimum(&alone, ppo) {
            notes.push(format!(
                "dip of {e:.1} dB at {} varies across positions: left alone",
                hz(f)
            ));
        }
        for (f, e) in runs_minimum(&out_dips, ppo) {
            notes.push(format!(
                "dip of {e:.1} dB at {} is deeper than the boost cap {:+.1} dB and {:.0} dB: \
                 left out of the fit and its level",
                hz(f),
                opts.max_boost_db,
                DIP_LEAVE_OUT_MARGIN_DB
            ));
        }
        for (f, e) in runs_minimum(&deep, ppo) {
            notes.push(format!(
                "dip of {e:.1} dB at {} is deeper than {:.0} dB: boost held to {:+.1} dB",
                hz(f),
                opts.deep_dip_db,
                opts.max_boost_db
            ));
        }
        Ok(Setup {
            model,
            opts,
            measured,
            freqs,
            trig,
            d0,
            e0,
            kind,
            old_db,
            old_sections,
            offset,
            free_level,
            bounds,
            allowed,
            replaced_idx,
            budget,
            notes,
        })
    }

    /// The work points: every grid point in the range, and every
    /// [`OUTSIDE_DECIMATION`]th one outside it for the boost cap.
    fn work(&self) -> Work {
        let mut w = Work::default();
        let fs = self.model.sample_rate();
        for (i, ((&f, &k), t)) in self
            .freqs
            .iter()
            .zip(self.kind.iter())
            .zip(self.trig.iter())
            .enumerate()
        {
            if f >= fs * 0.5 {
                continue;
            }
            let in_range = f >= self.opts.min_hz && f <= self.opts.max_hz;
            if !in_range && !i.is_multiple_of(OUTSIDE_DECIMATION) {
                continue;
            }
            w.trig.push(*t);
            w.kind.push(k);
            w.e0.push(self.e0.get(i).copied().flatten().unwrap_or(0.0));
            w.lf.push(f.log2());
            w.free_level = self.free_level;
            if k != Kind::Ignored {
                w.n_cost = w.n_cost.saturating_add(1);
            }
        }
        w
    }

    fn run(mut self) -> FitResult {
        let work = self.work();
        let cap = self.opts.max_evaluations;
        let shelves = self.opts.shelves && self.bounds.lq_shelf.is_some();
        let high_shelf = shelves && !self.opts.no_high_shelf;
        let ppo = f64::from(self.measured.grid.points_per_octave.max(1));
        let mut st = State::new(&work, self.model, self.bounds, self.opts.max_boost_db);
        while st.bands.len() < self.budget && st.rms() > self.opts.target_rms_db && st.evals < cap {
            let regions = st.regions(self.opts.min_gain_db, self.bounds.lf);
            let mut best: Option<State> = None;
            let mut spent = 0u32;
            'regions: for r in regions.iter().take(REGIONS_PER_STEP) {
                for c in r.candidates(&work, ppo, shelves, high_shelf) {
                    let room = cap.saturating_sub(st.evals.saturating_add(spent));
                    if room == 0 {
                        break 'regions;
                    }
                    let mut trial = st.clone();
                    trial.evals = 0;
                    if trial.push(self.bounds.clamp(c), room) {
                        let idx = trial.bands.len().saturating_sub(1);
                        let room = room.saturating_sub(trial.evals);
                        trial.refine(&[idx], PROBE_EVALUATIONS.min(room));
                    }
                    spent = spent.saturating_add(trial.evals);
                    if trial.bands.len() > st.bands.len()
                        && trial.cost < st.cost - 1e-9
                        && best.as_ref().is_none_or(|b| trial.cost < b.cost)
                    {
                        best = Some(trial);
                    }
                }
                if best.is_some() {
                    break;
                }
            }
            let evals = st.evals.saturating_add(spent);
            let Some(mut b) = best else {
                st.evals = evals;
                break;
            };
            b.evals = evals;
            st = b;
            let room = cap.saturating_sub(st.evals);
            st.refine_all(STEP_EVALUATIONS.min(room));
        }
        st.refine_all(cap.saturating_sub(st.evals));
        if st.drop_small(self.opts.min_gain_db) {
            st.refine_all(cap.saturating_sub(st.evals));
        }
        let evaluations = st.evals;
        let cands = st.bands.clone();
        self.finish(&cands, evaluations)
    }

    /// Quantise, hold to the bounds, predict again, and decide.
    fn finish(&mut self, cands: &[Cand], evaluations: u32) -> FitResult {
        let mut specs: Vec<BandSpec> = cands.iter().map(|c| self.quantise(c)).collect();
        specs.retain(|s| s.gain != 0.0);
        specs.sort_by(|a, b| {
            a.freq
                .total_cmp(&b.freq)
                .then(a.filter_type.cmp(&b.filter_type))
        });
        let mut new_db = self.sum_db(&specs);
        // Hold the sum of the bands to the boost cap after rounding.
        let mut guard = 0u32;
        while max_of(&new_db) > self.opts.max_boost_db + BOOST_SLACK_DB && guard < 200 {
            guard = guard.saturating_add(1);
            let top = specs
                .iter_mut()
                .filter(|s| s.gain > 0.0)
                .max_by(|a, b| a.gain.total_cmp(&b.gain));
            match top {
                Some(s) => s.gain = snap_gain(s.gain - 0.1),
                None => break,
            }
            specs.retain(|s| s.gain != 0.0);
            new_db = self.sum_db(&specs);
        }
        let before_len = specs.len();
        specs.retain(|s| f64::from(s.gain.abs()) >= self.opts.min_gain_db);
        if specs.len() != before_len {
            new_db = self.sum_db(&specs);
        }
        if max_of(&new_db) > self.opts.max_boost_db + BOOST_SLACK_DB {
            specs.clear();
        }

        let range = format!("{} to {}", hz(self.opts.min_hz), hz(self.opts.max_hz));
        let mut notes = std::mem::take(&mut self.notes);
        let rms_before = rms(&self.errors(None));
        if specs.is_empty() {
            notes.push(format!("no band lowers the error in {range}: no change"));
            return self.no_change(rms_before, evaluations, notes);
        }
        let rms_after = rms(&self.errors(Some(&new_db)));
        if rms_after > rms_before {
            notes.push(format!(
                "the fit did not lower the error in {range}: no change"
            ));
            return self.no_change(rms_before, evaluations, notes);
        }

        // The change: replaced bands out, fitted bands in.
        let delta: Vec<f64> = new_db
            .iter()
            .zip(self.old_db.iter())
            .map(|(n, o)| n - o)
            .collect();
        let cut = if self.opts.headroom {
            let nyq = self.model.sample_rate() * 0.5;
            let peak = delta
                .iter()
                .zip(self.freqs.iter())
                .filter(|(_, f)| **f < nyq)
                .map(|(d, _)| *d)
                .fold(0.0f64, f64::max);
            if peak > 1e-9 {
                -((peak * 10.0 - 1e-9).ceil() / 10.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // -- review P5-A (F1) -- The prediction has every replaced band
        // taken out, so the proposal must do the same on the device: the
        // fitted bands go to the bands in use first, and a band in use that
        // gets none is written flat. (Before, a fitted band could land on a
        // free index while the band in use stayed, and the device then held
        // both: not what was predicted.)
        let mut order: Vec<u8> = self.replaced_idx.clone();
        order.extend(
            self.allowed
                .iter()
                .copied()
                .filter(|i| !self.replaced_idx.contains(i)),
        );
        let mut out_bands: Vec<FittedBand> = order
            .iter()
            .zip(specs.iter())
            .map(|(&index, &band)| FittedBand { index, band })
            .collect();
        for &index in self.replaced_idx.iter().skip(specs.len()) {
            out_bands.push(FittedBand {
                index,
                band: BandSpec {
                    filter_type: TYPE_FLAT,
                    bypass: false,
                    freq: 1000.0,
                    q: 1.0,
                    gain: 0.0,
                    qp: 0.0,
                },
            });
        }
        out_bands.sort_by_key(|b| b.index);
        for b in &out_bands {
            notes.push(describe(b));
        }
        notes.push(format!(
            "error in {range}: {rms_before:.1} dB RMS before, {rms_after:.1} dB RMS after"
        ));
        if cut < 0.0 {
            notes.push(format!(
                "output gain {cut:.1} dB to keep the boost's headroom"
            ));
        }
        let predicted = self.predict(&delta, cut, &specs);
        FitResult {
            bands: out_bands,
            predicted,
            rms_before_db: rms_before,
            rms_after_db: rms_after,
            max_error_after_db: self.max_fit_error(Some(&new_db)),
            target_offset_db: self.offset + self.level(Some(&new_db)),
            gain_cut_db: cut as f32,
            evaluations,
            notes,
        }
    }

    fn no_change(&self, rms_before: f64, evaluations: u32, notes: Vec<String>) -> FitResult {
        FitResult {
            bands: Vec::new(),
            predicted: self.measured.clone(),
            rms_before_db: rms_before,
            rms_after_db: rms_before,
            max_error_after_db: self.max_fit_error(None),
            target_offset_db: self.offset + self.level(None),
            gain_cut_db: 0.0,
            evaluations,
            notes,
        }
    }

    /// The error at grid point `i` with the given new bands' dB (`None`: as
    /// the chain stands).
    fn error_at(&self, i: usize, d: f64, new_db: Option<&[f64]>) -> f64 {
        match new_db {
            None => d,
            Some(n) => {
                d - self.old_db.get(i).copied().unwrap_or(0.0) + n.get(i).copied().unwrap_or(0.0)
            }
        }
    }

    /// The errors at the points the fit may change (trusted, in the range,
    /// not left alone).
    fn fit_errors(&self, new_db: Option<&[f64]>) -> Vec<f64> {
        self.d0
            .iter()
            .zip(self.kind.iter())
            .enumerate()
            .filter(|(_, (_, k))| **k == Kind::Normal)
            .filter_map(|(i, (d, _))| d.map(|d| self.error_at(i, d, new_db)))
            .collect()
    }

    /// The level of the error: its mean over the points the fit may change
    /// (the fit's cost leaves the level free); 0 when it is anchored.
    fn level(&self, new_db: Option<&[f64]>) -> f64 {
        if !self.free_level {
            return 0.0;
        }
        let e = self.fit_errors(new_db);
        e.iter().sum::<f64>() / e.len().max(1) as f64
    }

    /// Per trusted point in the range: the error to the levelled target.
    fn errors(&self, new_db: Option<&[f64]>) -> Vec<f64> {
        let mu = self.level(new_db);
        self.d0
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.map(|d| self.error_at(i, d, new_db) - mu))
            .collect()
    }

    /// The largest |error| to the levelled target over the points the fit
    /// may change.
    fn max_fit_error(&self, new_db: Option<&[f64]>) -> f64 {
        let mu = self.level(new_db);
        self.fit_errors(new_db)
            .iter()
            .map(|e| (e - mu).abs())
            .fold(0.0, f64::max)
    }

    /// The new bands' summed dB on the whole grid.
    fn sum_db(&self, specs: &[BandSpec]) -> Vec<f64> {
        let secs: Vec<Coeffs> = specs.iter().flat_map(|s| self.model.sections(s)).collect();
        let mut out = Vec::with_capacity(self.freqs.len());
        if cascade_db(&secs, &self.trig, &mut out) {
            out
        } else {
            // Not finite: fails the boost-cap check, so nothing is proposed.
            vec![f64::INFINITY; self.freqs.len()]
        }
    }

    fn predict(&self, delta: &[f64], cut: f64, specs: &[BandSpec]) -> Response {
        let mag_db: Vec<f64> = self
            .measured
            .mag_db
            .iter()
            .zip(delta.iter())
            .map(|(m, d)| m + d + cut)
            .collect();
        let phase_deg = self.measured.phase_deg.as_ref().map(|p| {
            let new: Vec<Coeffs> = specs.iter().flat_map(|s| self.model.sections(s)).collect();
            let rad: Vec<f64> = self
                .trig
                .iter()
                .map(|t| {
                    let n: f64 = new.iter().map(|c| t.phase(c)).sum();
                    let o: f64 = self.old_sections.iter().map(|c| t.phase(c)).sum();
                    n - o
                })
                .collect();
            p.iter()
                .zip(unwrap(&rad).iter())
                .map(|(p, r)| {
                    let v = p + r.to_degrees();
                    if v.is_finite() { v } else { *p }
                })
                .collect()
        });
        Response {
            grid: self.measured.grid,
            mag_db,
            phase_deg,
            valid: self.measured.valid.clone(),
            spl: self.measured.spl,
        }
    }

    /// The band on the setters' grids, held inside the bounds.
    fn quantise(&self, c: &Cand) -> BandSpec {
        let c = self.bounds.clamp(*c);
        let (flo, fhi) = (self.bounds.lf.0.exp2(), self.bounds.lf.1.exp2());
        let lq = self.bounds.lq(c.ty);
        let (qlo, qhi) = (lq.0.exp2(), lq.1.exp2());
        let freq = inside(round_to(c.lf.exp2(), 10.0), flo, fhi, 10.0);
        let q = inside(round_to(c.lq.exp2(), 100.0), qlo, qhi, 100.0);
        let mut gain = snap_gain(c.g as f32);
        if f64::from(gain) > self.bounds.g.1 {
            gain = ((self.bounds.g.1 * 10.0).floor() / 10.0) as f32;
        }
        if f64::from(gain) < self.bounds.g.0 {
            gain = ((self.bounds.g.0 * 10.0).ceil() / 10.0) as f32;
        }
        BandSpec {
            filter_type: c.ty,
            bypass: false,
            freq: freq as f32,
            q: q as f32,
            gain: if gain == 0.0 { 0.0 } else { gain },
            qp: 0.0,
        }
    }
}

/// The options checked; the bounds they give.
fn check_options(o: &FitOptions, fs: f64) -> Result<Bounds, MeasureError> {
    let nums = [
        o.min_hz,
        o.max_hz,
        o.max_boost_db,
        o.max_cut_db,
        o.q_min,
        o.q_max,
        o.min_gain_db,
        o.spread_limit_db,
        o.deep_dip_db,
        o.target_rms_db,
    ];
    if !finite_all(&nums) {
        return Err(invalid("a fit option is not finite"));
    }
    if let FitLevel::Anchored { offset_db } = o.level
        && !(offset_db.is_finite() && offset_db.abs() <= 200.0)
    {
        return Err(invalid(format!("anchored level {offset_db} dB")));
    }
    let flo = o.min_hz.max(GRID_MIN_HZ);
    let fhi = o.max_hz.min(fs * MAX_FREQ_FRACTION).min(GRID_MAX_HZ);
    if !(flo > 0.0 && fhi > flo) {
        return Err(invalid(format!(
            "fit range {} Hz to {} Hz",
            o.min_hz, o.max_hz
        )));
    }
    if !(0.0..=HARD_MAX_BOOST_DB).contains(&o.max_boost_db) {
        return Err(invalid(format!("max boost {} dB", o.max_boost_db)));
    }
    if !(0.0..=HARD_MAX_CUT_DB).contains(&o.max_cut_db) {
        return Err(invalid(format!("max cut {} dB", o.max_cut_db)));
    }
    if !(o.q_min >= 0.05 && o.q_max >= o.q_min && o.q_max <= 40.0) {
        return Err(invalid(format!("Q range {} to {}", o.q_min, o.q_max)));
    }
    if !(0.0..=6.0).contains(&o.min_gain_db)
        || o.spread_limit_db < 0.0
        || o.deep_dip_db < 0.0
        || o.target_rms_db < 0.0
    {
        return Err(invalid("a fit threshold is out of range"));
    }
    if let Some(s) = o.smoothing
        && !(1..=96).contains(&s)
    {
        return Err(invalid(format!("smoothing 1/{s} octave")));
    }
    if o.max_evaluations > MAX_EVALUATIONS {
        return Err(invalid(format!(
            "{} evaluations (at most {MAX_EVALUATIONS})",
            o.max_evaluations
        )));
    }
    let sq = (o.q_min.max(SHELF_Q.0), o.q_max.min(SHELF_Q.1));
    Ok(Bounds {
        lf: (flo.log2(), fhi.log2()),
        lq_peak: (o.q_min.log2(), o.q_max.log2()),
        lq_shelf: (sq.0 <= sq.1).then(|| (sq.0.log2(), sq.1.log2())),
        g: (-o.max_cut_db, o.max_boost_db),
    })
}

/// The work points of the fit.
#[derive(Debug, Clone, Default)]
struct Work {
    /// The level is free (see [`FitLevel`]).
    free_level: bool,
    trig: Vec<Trig>,
    kind: Vec<Kind>,
    e0: Vec<f64>,
    lf: Vec<f64>,
    n_cost: usize,
}

/// A run of same-signed error: where a band may go.
#[derive(Debug, Clone)]
struct Region {
    start: usize,
    end: usize,
    area: f64,
    peak: usize,
    peak_e: f64,
    sum_e: f64,
    half: usize,
    touches_lo: bool,
    touches_hi: bool,
}

impl Region {
    /// Candidate bands: a peak, and a shelf where the region touches an end
    /// of the range.
    fn candidates(&self, w: &Work, ppo: f64, shelves: bool, high_shelf: bool) -> Vec<Cand> {
        let lf_at = |j: usize| w.lf.get(j).copied().unwrap_or(0.0);
        let n = self.end.saturating_sub(self.start).saturating_add(1) as f64;
        let mean = self.sum_e / n;
        // Q from the half-amplitude width: Q = sqrt(r) / (r - 1), r = 2^BW.
        let bw = self.half.max(1) as f64 / ppo;
        let r = bw.exp2();
        let q = r.sqrt() / (r - 1.0).max(1e-6);
        let shelf_q = std::f64::consts::FRAC_1_SQRT_2.log2();
        let mut out = vec![Cand {
            ty: TYPE_PEAKING,
            lf: lf_at(self.peak),
            lq: q.max(1e-6).log2(),
            g: -self.peak_e,
        }];
        if shelves && self.touches_lo {
            out.push(Cand {
                ty: TYPE_LOW_SHELF,
                lf: lf_at(self.end),
                lq: shelf_q,
                g: -mean,
            });
        }
        if shelves && high_shelf && self.touches_hi {
            out.push(Cand {
                ty: TYPE_HIGH_SHELF,
                lf: lf_at(self.start),
                lq: shelf_q,
                g: -mean,
            });
        }
        out
    }
}

/// The fit's state: bands, their dB at the work points, the cost.
#[derive(Clone)]
struct State<'w> {
    w: &'w Work,
    model: &'w dyn SectionModel,
    bounds: Bounds,
    max_boost: f64,
    bands: Vec<Cand>,
    contrib: Vec<Vec<f64>>,
    total: Vec<f64>,
    cost: f64,
    evals: u32,
    scratch: Vec<f64>,
}

impl<'w> State<'w> {
    fn new(w: &'w Work, model: &'w dyn SectionModel, bounds: Bounds, max_boost: f64) -> Self {
        let mut s = State {
            w,
            model,
            bounds,
            max_boost,
            bands: Vec::new(),
            contrib: Vec::new(),
            total: vec![0.0; w.trig.len()],
            cost: 0.0,
            evals: 0,
            scratch: Vec::new(),
        };
        s.cost = s.cost_of(None, None).unwrap_or(f64::INFINITY);
        s
    }

    fn rms(&self) -> f64 {
        self.cost.max(0.0).sqrt()
    }

    /// The mean error over the fitted points (the free level); 0 when it is
    /// anchored.
    fn level(&self) -> f64 {
        if !self.w.free_level {
            return 0.0;
        }
        let (s, n) = self
            .w
            .kind
            .iter()
            .zip(self.w.e0.iter().zip(self.total.iter()))
            .filter(|(k, _)| **k == Kind::Normal)
            .fold((0.0, 0usize), |(s, n), (_, (e, t))| {
                (s + e + t, n.saturating_add(1))
            });
        s / n.max(1) as f64
    }

    /// The cost with band `replace` (if any) taken out and `add` (if any)
    /// put in; `None` when the boost cap is broken.
    fn cost_of(&self, replace: Option<usize>, add: Option<&[f64]>) -> Option<f64> {
        let old = replace.and_then(|b| self.contrib.get(b));
        // The fitted points' error enters with its mean taken out: the level
        // is free, so no band is spent on moving it (sum of squares minus
        // n times the squared mean).
        let (mut sum, mut lin, mut n_fit) = (0.0, 0.0, 0usize);
        // -- WP P5TRUST -- the EQ change at the points left alone, against
        // the EQ's own level over the fitted points when the level is free:
        // cutting everything around a dip is boosting the dip.
        let (mut eq_fit, mut a1, mut a2, mut n_alone) = (0.0, 0.0, 0.0, 0usize);
        for (j, ((&t, &k), &e0)) in self
            .total
            .iter()
            .zip(self.w.kind.iter())
            .zip(self.w.e0.iter())
            .enumerate()
        {
            let mut eq = t;
            if let Some(o) = old {
                eq -= o.get(j).copied().unwrap_or(0.0);
            }
            if let Some(a) = add {
                eq += a.get(j).copied().unwrap_or(0.0);
            }
            if eq > self.max_boost + BOOST_SLACK_DB {
                return None;
            }
            match k {
                Kind::Normal => {
                    let e = e0 + eq;
                    let boost = eq.max(0.0);
                    sum += e * e + BOOST_WEIGHT * boost * boost;
                    lin += e;
                    eq_fit += eq;
                    n_fit = n_fit.saturating_add(1);
                }
                Kind::LeaveAlone => {
                    a1 += eq;
                    a2 += eq * eq;
                    n_alone = n_alone.saturating_add(1);
                }
                Kind::Ignored => {}
            }
        }
        let (level, mu) = if self.w.free_level {
            let n = n_fit.max(1) as f64;
            (lin * lin / n, eq_fit / n)
        } else {
            (0.0, 0.0)
        };
        // Σ (eq - mu)² over the points left alone.
        sum += (a2 - 2.0 * mu * a1 + n_alone as f64 * mu * mu).max(0.0);
        Some((sum - level).max(0.0) / self.w.n_cost.max(1) as f64)
    }

    /// Cost of band `b` set to `c` (`None`: a new band); one evaluation. The
    /// band's response is left in `scratch`.
    fn try_band(&mut self, b: Option<usize>, c: &Cand) -> Option<f64> {
        self.evals = self.evals.saturating_add(1);
        let mut buf = std::mem::take(&mut self.scratch);
        let secs = self.model.sections(&c.spec());
        let cost = if cascade_db(&secs, &self.w.trig, &mut buf) {
            self.cost_of(b, Some(&buf))
        } else {
            None
        };
        self.scratch = buf;
        cost.filter(|c| c.is_finite())
    }

    fn recompute_total(&mut self) {
        self.total.iter_mut().for_each(|t| *t = 0.0);
        for c in &self.contrib {
            for (t, v) in self.total.iter_mut().zip(c.iter()) {
                *t += v;
            }
        }
    }

    /// Add a band, halving its gain until the boost cap holds, within `room`
    /// evaluations; false if it never does.
    fn push(&mut self, mut c: Cand, room: u32) -> bool {
        for _ in 0..room.min(8) {
            if let Some(cost) = self.try_band(None, &c) {
                let resp = std::mem::take(&mut self.scratch);
                self.bands.push(c);
                self.contrib.push(resp);
                self.recompute_total();
                self.cost = cost;
                return true;
            }
            c.g *= 0.5;
        }
        false
    }

    fn refine_all(&mut self, budget: u32) {
        let all: Vec<usize> = (0..self.bands.len()).collect();
        self.refine(&all, budget);
    }

    /// Bounded coordinate descent over the given bands' frequency, Q and
    /// gain, inside the box bounds; spends at most `budget` evaluations and
    /// only ever lowers the cost.
    fn refine(&mut self, idxs: &[usize], budget: u32) {
        const START: [f64; 3] = [1.0 / 6.0, 0.5, 1.0];
        const MIN: [f64; 3] = [1.0 / 384.0, 1.0 / 128.0, 0.02];
        let stop_at = self.evals.saturating_add(budget);
        let mut steps: Vec<[f64; 3]> = vec![START; idxs.len()];
        loop {
            let mut active = false;
            for (&b, st) in idxs.iter().zip(steps.iter_mut()) {
                for (p, (step, min)) in st.iter_mut().zip(MIN.iter()).enumerate() {
                    if *step < *min {
                        continue;
                    }
                    active = true;
                    let Some(cur) = self.bands.get(b).copied() else {
                        continue;
                    };
                    let mut best: Option<(f64, Cand, Vec<f64>)> = None;
                    for dir in [1.0, -1.0] {
                        if self.evals >= stop_at {
                            return;
                        }
                        let mut c = cur;
                        match p {
                            0 => c.lf += dir * *step,
                            1 => c.lq += dir * *step,
                            _ => c.g += dir * *step,
                        }
                        let c = self.bounds.clamp(c);
                        if c == cur {
                            continue;
                        }
                        if let Some(cost) = self.try_band(Some(b), &c)
                            && cost < self.cost - 1e-12
                            && best.as_ref().is_none_or(|(bc, _, _)| cost < *bc)
                        {
                            best = Some((cost, c, self.scratch.clone()));
                        }
                    }
                    match best {
                        Some((cost, c, resp)) => {
                            if let (Some(slot), Some(cslot)) =
                                (self.bands.get_mut(b), self.contrib.get_mut(b))
                            {
                                *slot = c;
                                *cslot = resp;
                            }
                            self.recompute_total();
                            self.cost = cost;
                        }
                        None => *step *= 0.5,
                    }
                }
            }
            if !active {
                return;
            }
        }
    }

    /// Drop bands whose |gain| is below `min`; true if any was dropped.
    fn drop_small(&mut self, min: f64) -> bool {
        let keep: Vec<bool> = self.bands.iter().map(|c| c.g.abs() >= min).collect();
        if keep.iter().all(|k| *k) {
            return false;
        }
        let mut it = keep.iter();
        self.bands.retain(|_| it.next().copied().unwrap_or(false));
        let mut it = keep.iter();
        self.contrib.retain(|_| it.next().copied().unwrap_or(false));
        self.recompute_total();
        // Dropping a cut under a boost can break the cap: then the cost is
        // infinite, the refinement accepts nothing, and the final check in
        // `finish` drops the proposal.
        self.cost = self.cost_of(None, None).unwrap_or(f64::INFINITY);
        true
    }

    /// The same-signed error regions in the range, largest area first.
    fn regions(&self, min_gain: f64, lf_bounds: (f64, f64)) -> Vec<Region> {
        let thr = (min_gain * 0.5).max(0.25);
        let edge = 1.0 / 6.0;
        let mu = self.level();
        let mut out: Vec<Region> = Vec::new();
        let mut cur: Option<Region> = None;
        for (j, ((&k, &e0), (&t, &lf))) in self
            .w
            .kind
            .iter()
            .zip(self.w.e0.iter())
            .zip(self.total.iter().zip(self.w.lf.iter()))
            .enumerate()
        {
            let e = e0 + t - mu;
            // A dip is usable only while the EQ there has boost left.
            let usable =
                k == Kind::Normal && e.abs() >= thr && (e > 0.0 || t < self.max_boost - 0.1);
            if let Some(r) = cur.as_mut()
                && usable
                && (r.peak_e > 0.0) == (e > 0.0)
                && j == r.end.saturating_add(1)
            {
                r.end = j;
                r.area += e.abs();
                r.sum_e += e;
                if e.abs() > r.peak_e.abs() {
                    r.peak = j;
                    r.peak_e = e;
                }
                r.touches_hi = lf >= lf_bounds.1 - edge;
                continue;
            }
            if let Some(r) = cur.take() {
                out.push(r);
            }
            if usable {
                cur = Some(Region {
                    start: j,
                    end: j,
                    area: e.abs(),
                    peak: j,
                    peak_e: e,
                    sum_e: e,
                    half: 0,
                    touches_lo: lf <= lf_bounds.0 + edge,
                    touches_hi: lf >= lf_bounds.1 - edge,
                });
            }
        }
        if let Some(r) = cur.take() {
            out.push(r);
        }
        for r in &mut out {
            let half = r.peak_e.abs() * 0.5;
            r.half = (r.start..=r.end)
                .filter(|&j| {
                    let e = self.w.e0.get(j).copied().unwrap_or(0.0)
                        + self.total.get(j).copied().unwrap_or(0.0)
                        - mu;
                    e.abs() >= half
                })
                .count();
        }
        out.sort_by(|a, b| b.area.total_cmp(&a.area).then(a.start.cmp(&b.start)));
        out
    }
}

/// The target at `f`, linear in log frequency; `None` outside it or next to
/// an untrusted target point.
fn interp(target: &Response, tf: &[f64], f: f64) -> Option<f64> {
    let ok = |i: usize| {
        target
            .valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false))
    };
    let k = tf.partition_point(|&x| x < f);
    let hi_f = *tf.get(k)?;
    let hi_v = *target.mag_db.get(k)?;
    if (hi_f - f).abs() <= f * 1e-12 {
        return ok(k).then_some(hi_v);
    }
    let lo_i = k.checked_sub(1)?;
    let lo_f = *tf.get(lo_i)?;
    let lo_v = *target.mag_db.get(lo_i)?;
    if !(ok(k) && ok(lo_i)) {
        return None;
    }
    let t = (f / lo_f).ln() / (hi_f / lo_f).ln();
    let v = lo_v + (hi_v - lo_v) * t;
    v.is_finite().then_some(v)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    let upper = v.get(mid).copied().unwrap_or(0.0);
    if v.len().is_multiple_of(2) {
        let lower = mid
            .checked_sub(1)
            .and_then(|i| v.get(i).copied())
            .unwrap_or(upper);
        (lower + upper) * 0.5
    } else {
        upper
    }
}

fn rms(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt()
}

fn max_of(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Phase in radians unwrapped along the grid.
fn unwrap(rad: &[f64]) -> Vec<f64> {
    let tau = std::f64::consts::TAU;
    let mut out = Vec::with_capacity(rad.len());
    let mut off = 0.0;
    let mut prev: Option<f64> = None;
    for &r in rad {
        if let Some(p) = prev {
            let d = r - p;
            if d.is_finite() {
                off -= tau * (d / tau).round();
            }
        }
        prev = Some(r);
        out.push(r + off);
    }
    out
}

/// The deepest point of each run of points closer than two grid steps.
fn runs_minimum(points: &[(f64, f64)], ppo: u16) -> Vec<(f64, f64)> {
    let gap = (2.0 / f64::from(ppo.max(1))).exp2() * 1.000_001;
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut last_f = 0.0;
    for &(f, e) in points {
        match out.last_mut() {
            Some(m) if f <= last_f * gap => {
                if e < m.1 {
                    *m = (f, e);
                }
            }
            _ => out.push((f, e)),
        }
        last_f = f;
    }
    out
}

/// `v` on a grid of `1 / scale`, halves away from zero.
fn round_to(v: f64, scale: f64) -> f64 {
    (v * scale).round() / scale
}

/// `v` held inside `[lo, hi]` on the `1 / scale` grid. The bounds win over
/// the grid (review P5-A, F4): a range narrower than one grid step holds no
/// grid value, and the nearest one lies outside it, so the value is then held
/// to the range off the grid.
fn inside(v: f64, lo: f64, hi: f64, scale: f64) -> f64 {
    let on_grid = if v < lo {
        (lo * scale).ceil() / scale
    } else if v > hi {
        (hi * scale).floor() / scale
    } else {
        return v;
    };
    if on_grid >= lo && on_grid <= hi {
        on_grid
    } else {
        v.clamp(lo, hi)
    }
}

/// A gain on the device's 0.1 dB grid, as `dspi-core`'s `quantize_gain`
/// (`curves.rs`): halves away from zero, -0 becomes +0.
fn snap_gain(db: f32) -> f32 {
    let v = (db * 10.0).round() / 10.0;
    if v == 0.0 { 0.0 } else { v }
}

fn hz(f: f64) -> String {
    if f >= 1000.0 {
        format!("{:.2} kHz", f / 1000.0)
    } else {
        format!("{f:.1} Hz")
    }
}

/// A band in words, as the proposal lists it.
fn describe(b: &FittedBand) -> String {
    if b.band.filter_type == TYPE_FLAT {
        return format!("PEQ band {}: cleared (it was in use)", band_no(b.index));
    }
    let what = match b.band.filter_type {
        TYPE_LOW_SHELF => "low shelf",
        TYPE_HIGH_SHELF => "high shelf",
        _ => "peak",
    };
    format!(
        "PEQ band {}: {what} {:+.1} dB at {}, Q {:.2}",
        band_no(b.index),
        b.band.gain,
        hz(f64::from(b.band.freq)),
        b.band.q
    )
}
