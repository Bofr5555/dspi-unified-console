//! Impulse response or spectrum to magnitude and phase on a log grid;
//! fractional-octave smoothing; the trusted-point mask (WP P5RESP).
//!
//! PLAN-P5 section 1.1 steps 7 and 9, T1: a measured impulse response (from
//! P5TIME's `ir`) or a linear-frequency spectrum becomes a [`Response`] on a
//! [`LogGrid`] (normally [`LogGrid::STANDARD`]) with its **unwrapped** phase;
//! [`smooth`] applies 1/1 to 1/48 octave smoothing; [`trust_mask`] marks the
//! points a fit or an alignment may use (SNR above the noise floor, repeat
//! spread, PLAN-P5 section 1.1 step 9).
//!
//! How the grid is sampled: the impulse response is zero-padded to at least
//! four times its length (at least 2^16, at most 2^20 points) and transformed
//! once (`rustfft`, scalar). The phase is unwrapped **on the FFT bins**, where
//! the padding keeps the step between bins below a quarter turn, and only then
//! interpolated to the grid; the magnitude is interpolated in dB between the
//! two neighbouring bins. No averaging happens here: a grid point is the
//! spectrum at that frequency, smoothing is [`smooth`]'s job.

use crate::types::{LogGrid, MeasureError, Response};
use rustfft::FftPlannerScalar;
use rustfft::num_complex::Complex;

/// The longest impulse response [`from_impulse`] accepts, samples (5.4 s at
/// 48 kHz: well past the 15-cycle fit window at 20 Hz).
pub const MAX_IR_SAMPLES: usize = 1 << 18;
/// The shortest FFT [`from_impulse`] uses (fine bins at the grid's low end).
pub const MIN_FFT_LEN: usize = 1 << 16;
/// The longest FFT [`from_impulse`] uses (16 MiB of complex `f64`).
pub const MAX_FFT_LEN: usize = 1 << 20;
/// The most bins [`from_spectrum`] and [`from_power_spectrum`] accept.
pub const MAX_SPECTRUM_BINS: usize = MAX_FFT_LEN / 2 + 1;
/// The lowest level a magnitude is given as, dB (a power of 1e-30): zero
/// and silence become this instead of minus infinity.
pub const FLOOR_DB: f64 = -300.0;
/// The coarsest smoothing, as an octave fraction (1/1 octave).
pub const MIN_SMOOTHING: u8 = 1;
/// The finest smoothing, as an octave fraction (1/48 octave).
pub const MAX_SMOOTHING: u8 = 48;
/// The most responses [`repeat_spread_db`] compares (2 × the 8 repeats).
pub const MAX_REPEATS: usize = 16;
/// Sample rates an impulse response may have, Hz.
pub const SAMPLE_RATE_RANGE: std::ops::RangeInclusive<f64> = 8_000.0..=384_000.0;

/// What [`trust_mask`] demands of a point (PLAN-P5 section 1.1 step 9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrustRule {
    /// The least signal-to-noise ratio, dB.
    pub min_snr_db: f64,
    /// The largest repeat-to-repeat spread, dB.
    pub max_spread_db: f64,
}

impl TrustRule {
    /// The plan's rule: SNR at least 20 dB and a spread at most 1 dB.
    pub const DEFAULT: TrustRule = TrustRule {
        min_snr_db: 20.0,
        max_spread_db: 1.0,
    };
}

impl Default for TrustRule {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn invalid(msg: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(msg.into())
}

/// `10·log10(p)`, never below [`FLOOR_DB`] and always finite.
pub(crate) fn power_db(p: f64) -> f64 {
    if p.is_finite() && p > 1e-30 {
        10.0 * p.log10()
    } else if p.is_finite() || p.is_nan() {
        FLOOR_DB
    } else {
        // +inf power: as loud as f64 can state finitely.
        10.0 * f64::MAX.log10()
    }
}

/// Checks a response's shape: the grid is valid, `mag_db` (and `phase_deg`,
/// `valid` when present) have one value per grid point, every value finite.
/// Returns the grid's frequencies.
///
/// Errors: [`MeasureError::Invalid`] naming what is wrong.
pub fn check(resp: &Response) -> Result<Vec<f64>, MeasureError> {
    let freqs = resp.grid.freqs()?;
    let n = freqs.len();
    if resp.mag_db.len() != n {
        return Err(invalid(format!(
            "{} magnitudes for a grid of {n} points",
            resp.mag_db.len()
        )));
    }
    if !resp.mag_db.iter().all(|v| v.is_finite()) {
        return Err(invalid("non-finite magnitude"));
    }
    if let Some(p) = &resp.phase_deg {
        if p.len() != n {
            return Err(invalid(format!(
                "{} phases for a grid of {n} points",
                p.len()
            )));
        }
        if !p.iter().all(|v| v.is_finite()) {
            return Err(invalid("non-finite phase"));
        }
    }
    if let Some(v) = &resp.valid
        && v.len() != n
    {
        return Err(invalid(format!(
            "{} validity flags for a grid of {n} points",
            v.len()
        )));
    }
    Ok(freqs)
}

/// The response of an impulse response `ir` sampled at `sample_rate`, on
/// `grid`, with unwrapped phase.
///
/// `time_zero_s` is the instant (seconds after `ir[0]`) the phase is
/// referred to: a pure delay of that length reads as zero phase (the caller
/// passes the run's timing reference, OD-P5b; 0 keeps `ir[0]`).
/// Grid points above the Nyquist frequency are held at the last bin's value
/// and marked untrusted in `valid`; every other point is marked trusted (the
/// SNR and the spread are [`trust_mask`]'s).
///
/// Errors: [`MeasureError::Invalid`] for an empty or too long `ir`
/// ([`MAX_IR_SAMPLES`]), a non-finite sample, a rate outside
/// [`SAMPLE_RATE_RANGE`], a non-finite `time_zero_s` or one beyond 10 s, or
/// a bad grid.
pub fn from_impulse(
    ir: &[f64],
    sample_rate: f64,
    grid: &LogGrid,
    time_zero_s: f64,
) -> Result<Response, MeasureError> {
    if ir.is_empty() || ir.len() > MAX_IR_SAMPLES {
        return Err(invalid(format!(
            "impulse response of {} samples (1 to {MAX_IR_SAMPLES})",
            ir.len()
        )));
    }
    if !ir.iter().all(|v| v.is_finite()) {
        return Err(invalid("non-finite sample in the impulse response"));
    }
    if !SAMPLE_RATE_RANGE.contains(&sample_rate) {
        return Err(invalid(format!("sample rate {sample_rate} Hz")));
    }
    if !time_zero_s.is_finite() || time_zero_s.abs() > 10.0 {
        return Err(invalid(format!("time zero {time_zero_s} s")));
    }
    // Validate the grid before spending an FFT on it.
    grid.freqs()?;
    let n = ir
        .len()
        .saturating_mul(4)
        .checked_next_power_of_two()
        .unwrap_or(MAX_FFT_LEN)
        .clamp(MIN_FFT_LEN, MAX_FFT_LEN);
    let mut buf: Vec<Complex<f64>> = Vec::with_capacity(n);
    buf.extend(ir.iter().map(|v| Complex::new(*v, 0.0)));
    buf.resize(n, Complex::new(0.0, 0.0));
    let mut planner = FftPlannerScalar::<f64>::new();
    planner.plan_fft_forward(n).process(&mut buf);
    buf.truncate((n / 2).saturating_add(1));
    let bin_hz = sample_rate / n as f64;
    let bins: Vec<[f64; 2]> = buf.iter().map(|c| [c.re, c.im]).collect();
    spectrum_to_grid(&bins, bin_hz, grid, time_zero_s)
}

/// A complex linear-frequency spectrum (`bins[k]` is `[re, im]` at
/// `k * bin_hz`, from DC) on `grid`, with unwrapped phase.
///
/// The phase is unwrapped along the bins, so they must be dense enough that
/// the phase moves less than half a turn from one to the next (true for the
/// FFT of a signal zero-padded to twice its length). Grid points above the
/// last bin are held at its value and marked untrusted.
///
/// Errors: [`MeasureError::Invalid`] for fewer than 2 or more than
/// [`MAX_SPECTRUM_BINS`] bins, a non-finite value, a non-positive or
/// non-finite `bin_hz`, or a bad grid.
pub fn from_spectrum(
    bins: &[[f64; 2]],
    bin_hz: f64,
    grid: &LogGrid,
) -> Result<Response, MeasureError> {
    check_bins(bins.len(), bin_hz)?;
    if !bins.iter().flatten().all(|v| v.is_finite()) {
        return Err(invalid("non-finite spectrum value"));
    }
    spectrum_to_grid(bins, bin_hz, grid, 0.0)
}

/// A power spectrum (`power[k]` a power, not dB, at `k * bin_hz`, from DC)
/// on `grid`, magnitude only, in dB (`10·log10`, interpolated in dB). Grid
/// points above the last bin are held and marked untrusted.
///
/// Errors: as [`from_spectrum`], and for a negative power.
pub fn from_power_spectrum(
    power: &[f64],
    bin_hz: f64,
    grid: &LogGrid,
) -> Result<Response, MeasureError> {
    check_bins(power.len(), bin_hz)?;
    if !power.iter().all(|v| v.is_finite() && *v >= 0.0) {
        return Err(invalid("non-finite or negative power"));
    }
    let freqs = grid.freqs()?;
    let db: Vec<f64> = power.iter().map(|p| power_db(*p)).collect();
    let mut mag = Vec::with_capacity(freqs.len());
    let mut valid = Vec::with_capacity(freqs.len());
    for f in &freqs {
        let (v, inside) = interp_at(&db, *f / bin_hz);
        mag.push(v);
        valid.push(inside);
    }
    Ok(Response {
        grid: *grid,
        mag_db: mag,
        phase_deg: None,
        valid: Some(valid),
        spl: false,
    })
}

/// The longest reference delay [`from_grid_values`] accepts, seconds.
pub const MAX_REFERENCE_DELAY_S: f64 = 10.0;

// -- review P5-A -- the seam between P5TIME's windowed responses (complex
// values at the grid's own frequencies) and a `Response`.
/// Complex values given **at the grid's own points** (`values[i]` is
/// `[re, im]` at `grid.freqs()[i]`: P5TIME's fixed-gate and
/// frequency-dependent-window responses) as a [`Response`] with unwrapped
/// phase.
///
/// A log grid is coarse at high frequencies (145 Hz between points at
/// 20 kHz on the standard grid), so a delay of a few milliseconds turns the
/// phase by more than half a turn from one point to the next and cannot be
/// unwrapped from the points alone. `reference_delay_s` is the delay the
/// values are known to carry (the output's arrival after the run's timing
/// origin): it is taken out before unwrapping and put back as a straight
/// line, so the phase is right for any arrival. What is left to unwrap is the
/// response's own phase about its arrival.
///
/// A zero value reads [`FLOOR_DB`] and is marked untrusted; every other
/// point is trusted (the SNR and the spread are [`trust_mask`]'s).
///
/// Errors: [`MeasureError::Invalid`] for a bad grid, a number of values other
/// than the grid's points, a non-finite value, or a reference delay that is
/// not finite or beyond [`MAX_REFERENCE_DELAY_S`].
pub fn from_grid_values(
    values: &[[f64; 2]],
    grid: &LogGrid,
    reference_delay_s: f64,
) -> Result<Response, MeasureError> {
    use std::f64::consts::{PI, TAU};
    let freqs = grid.freqs()?;
    if values.len() != freqs.len() {
        return Err(invalid(format!(
            "{} values for a grid of {} points",
            values.len(),
            freqs.len()
        )));
    }
    if !values.iter().flatten().all(|v| v.is_finite()) {
        return Err(invalid("non-finite response value"));
    }
    if !reference_delay_s.is_finite() || reference_delay_s.abs() > MAX_REFERENCE_DELAY_S {
        return Err(invalid(format!("reference delay {reference_delay_s} s")));
    }
    let mut mag = Vec::with_capacity(freqs.len());
    let mut phase = Vec::with_capacity(freqs.len());
    let mut valid = Vec::with_capacity(freqs.len());
    let mut prev: Option<f64> = None;
    let mut offset = 0.0f64;
    for ([re, im], f) in values.iter().zip(&freqs) {
        let p = re * re + im * im;
        mag.push(power_db(p));
        let present = p > 0.0;
        valid.push(present);
        // The phase about the arrival, wrapped, then unwrapped along the grid;
        // a zero value keeps the previous point's phase.
        let line = TAU * f * reference_delay_s;
        let residual = if present {
            let w = (im.atan2(*re) + line + PI).rem_euclid(TAU) - PI;
            if let Some(q) = prev {
                let d = w - q;
                if d > PI {
                    offset -= TAU;
                } else if d < -PI {
                    offset += TAU;
                }
            }
            prev = Some(w);
            w + offset
        } else {
            prev.map_or(0.0, |q| q + offset)
        };
        let deg = (residual - line).to_degrees();
        phase.push(if deg.is_finite() { deg } else { 0.0 });
    }
    Ok(Response {
        grid: *grid,
        mag_db: mag,
        phase_deg: Some(phase),
        valid: Some(valid),
        spl: false,
    })
}

fn check_bins(len: usize, bin_hz: f64) -> Result<(), MeasureError> {
    if !(2..=MAX_SPECTRUM_BINS).contains(&len) {
        return Err(invalid(format!(
            "spectrum of {len} bins (2 to {MAX_SPECTRUM_BINS})"
        )));
    }
    if !bin_hz.is_finite() || bin_hz <= 0.0 {
        return Err(invalid(format!("bin spacing {bin_hz} Hz")));
    }
    Ok(())
}

/// Linear interpolation in `values` at the fractional index `x`. Returns the
/// value and whether `x` lay inside the values (else the nearest end is
/// held).
fn interp_at(values: &[f64], x: f64) -> (f64, bool) {
    let last = values.len().saturating_sub(1);
    let first_v = values.first().copied().unwrap_or(0.0);
    let last_v = values.last().copied().unwrap_or(0.0);
    if !x.is_finite() || x < 0.0 {
        // A point a hair below the first (rounding) still counts as inside.
        return (first_v, x.is_finite() && x > -1e-9);
    }
    if x >= last as f64 {
        // A point a hair above the last (rounding) still counts as inside.
        return (last_v, x <= last as f64 + 1e-9);
    }
    // 0 <= x < last, so k < last and k + 1 <= last: both in range.
    let k = x.floor() as usize;
    let t = x - k as f64;
    let a = values.get(k).copied().unwrap_or(last_v);
    let b = values.get(k.saturating_add(1)).copied().unwrap_or(last_v);
    (a + (b - a) * t, true)
}

fn spectrum_to_grid(
    bins: &[[f64; 2]],
    bin_hz: f64,
    grid: &LogGrid,
    time_zero_s: f64,
) -> Result<Response, MeasureError> {
    use std::f64::consts::{PI, TAU};
    let freqs = grid.freqs()?;
    let db: Vec<f64> = bins
        .iter()
        .map(|[re, im]| power_db(re * re + im * im))
        .collect();
    // Unwrap along the bins after removing the reference delay.
    let mut phase = Vec::with_capacity(bins.len());
    let mut prev = 0.0f64;
    let mut offset = 0.0f64;
    for (k, [re, im]) in bins.iter().enumerate() {
        let f = k as f64 * bin_hz;
        let raw = im.atan2(*re) + TAU * f * time_zero_s;
        let wrapped = (raw + PI).rem_euclid(TAU) - PI;
        let wrapped = if wrapped.is_finite() { wrapped } else { 0.0 };
        if k > 0 {
            let d = wrapped - prev;
            if d > PI {
                offset -= TAU;
            } else if d < -PI {
                offset += TAU;
            }
        }
        prev = wrapped;
        phase.push(wrapped + offset);
    }
    let mut mag = Vec::with_capacity(freqs.len());
    let mut ph = Vec::with_capacity(freqs.len());
    let mut valid = Vec::with_capacity(freqs.len());
    for f in &freqs {
        let x = *f / bin_hz;
        let (m, inside) = interp_at(&db, x);
        let (p, _) = interp_at(&phase, x);
        mag.push(m);
        let deg = p.to_degrees();
        ph.push(if deg.is_finite() { deg } else { 0.0 });
        valid.push(inside);
    }
    Ok(Response {
        grid: *grid,
        mag_db: mag,
        phase_deg: Some(ph),
        valid: Some(valid),
        spl: false,
    })
}

/// `resp` on another grid: magnitude and phase interpolated linearly in
/// log-frequency; outside `resp`'s range the end value is held and the point
/// marked untrusted; inside, a point is trusted when both neighbours are.
/// The `spl` flag is kept.
///
/// Errors: [`MeasureError::Invalid`] for a malformed `resp` ([`check`]) or a
/// bad grid.
pub fn resample(resp: &Response, grid: &LogGrid) -> Result<Response, MeasureError> {
    let src = check(resp)?;
    let dst = grid.freqs()?;
    let lo = src.first().copied().unwrap_or(1.0).log2();
    let ppo = f64::from(resp.grid.points_per_octave);
    let last = src.len().saturating_sub(1);
    let mut mag = Vec::with_capacity(dst.len());
    let mut ph = resp
        .phase_deg
        .as_ref()
        .map(|_| Vec::with_capacity(dst.len()));
    let mut valid = Vec::with_capacity(dst.len());
    for f in &dst {
        let x = (f.log2() - lo) * ppo;
        let (m, inside) = interp_at(&resp.mag_db, x);
        mag.push(m);
        if let (Some(out), Some(p)) = (ph.as_mut(), resp.phase_deg.as_ref()) {
            out.push(interp_at(p, x).0);
        }
        let neighbours_ok = match &resp.valid {
            None => true,
            Some(v) => {
                let k = if x > 0.0 {
                    (x.floor() as usize).min(last)
                } else {
                    0
                };
                let k1 = if x > k as f64 {
                    k.saturating_add(1).min(last)
                } else {
                    k
                };
                v.get(k).copied().unwrap_or(false) && v.get(k1).copied().unwrap_or(false)
            }
        };
        valid.push(inside && neighbours_ok);
    }
    Ok(Response {
        grid: *grid,
        mag_db: mag,
        phase_deg: ph,
        valid: Some(valid),
        spl: resp.spl,
    })
}

/// The weights of a rectangular window of half-width `h` grid points,
/// applied to the linear interpolant of the points (exact for
/// piecewise-linear data), for offsets `-K..=K`; they sum to 1.
fn window_weights(h: f64) -> Vec<f64> {
    let m_f = h.floor();
    let r = h - m_f;
    // 0 < h <= 384 / 2 by the callers' bounds.
    let m = m_f as usize;
    let k = if r > 0.0 { m.saturating_add(1) } else { m };
    let len = k.saturating_mul(2).saturating_add(1);
    let mut w = vec![0.0f64; len];
    for (idx, slot) in w.iter_mut().enumerate() {
        let j = idx.abs_diff(k);
        *slot = if m == 0 {
            match j {
                0 => 2.0 * (r - r * r / 2.0),
                _ => r * r / 2.0,
            }
        } else if j < m {
            1.0
        } else if j == m {
            0.5 + r - r * r / 2.0
        } else {
            r * r / 2.0
        };
    }
    let sum: f64 = w.iter().sum();
    if sum > 0.0 {
        for x in &mut w {
            *x /= sum;
        }
    }
    w
}

/// What a smoothing window averages (review P5-A: one smoothing for the whole
/// crate, two domains).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothingDomain {
    /// The mean of the dB values: **what a graph draws**. A peak one window
    /// wide is halved in dB, a dip is drawn as deep as it is on average.
    Db,
    /// The mean of the powers (`10^(dB/10)`), given back in dB: **what a fit
    /// and a level estimate use**, and how the ear and an analyser's bands
    /// sum. A deep narrow null hardly lowers the smoothed curve (there is
    /// almost no energy in it to average), so a fit sees no dip to fill; a
    /// peak keeps its energy and is still cut.
    Power,
}

/// Fractional-octave smoothing for a graph: each point becomes the mean, **in
/// dB**, of the curve within `1/(2·fraction)` octave either side
/// ([`smooth_as`] with [`SmoothingDomain::Db`]).
///
/// `fraction` is the octave fraction: 1 (1/1 octave) to 48 (1/48 octave).
///
/// Errors: [`MeasureError::Invalid`] for a fraction outside
/// [`MIN_SMOOTHING`]..=[`MAX_SMOOTHING`] or a malformed `resp`.
pub fn smooth(resp: &Response, fraction: u8) -> Result<Response, MeasureError> {
    smooth_as(resp, fraction, SmoothingDomain::Db)
}

/// Fractional-octave smoothing in a chosen domain: each point becomes the
/// mean (of the dB values or of the powers, [`SmoothingDomain`]) of the curve
/// within `1/(2·fraction)` octave either side: a rectangular window on the
/// log-frequency axis, applied exactly to the curve's linear interpolation;
/// the end values are held beyond the grid's ends. The phase, when present,
/// is smoothed as plain degrees in either domain (it is unwrapped).
///
/// Untrusted points (`valid` false) do not contribute to their neighbours; a
/// point keeps its own trust flag, and one with no trusted point in its
/// window keeps its value.
///
/// `fraction` is the octave fraction: 1 (1/1 octave) to 48 (1/48 octave).
///
/// Errors: [`MeasureError::Invalid`] for a fraction outside
/// [`MIN_SMOOTHING`]..=[`MAX_SMOOTHING`] or a malformed `resp`.
pub fn smooth_as(
    resp: &Response,
    fraction: u8,
    domain: SmoothingDomain,
) -> Result<Response, MeasureError> {
    if !(MIN_SMOOTHING..=MAX_SMOOTHING).contains(&fraction) {
        return Err(invalid(format!(
            "smoothing 1/{fraction} octave (1/{MIN_SMOOTHING} to 1/{MAX_SMOOTHING})"
        )));
    }
    check(resp)?;
    let valid = resp.valid.as_deref();
    let ppo = resp.grid.points_per_octave;
    let mag = smooth_magnitudes(&resp.mag_db, valid, ppo, f64::from(fraction), domain);
    let phase = resp
        .phase_deg
        .as_ref()
        .map(|p| smooth_magnitudes(p, valid, ppo, f64::from(fraction), SmoothingDomain::Db));
    Ok(Response {
        grid: resp.grid,
        mag_db: mag,
        phase_deg: phase,
        valid: resp.valid.clone(),
        spl: resp.spl,
    })
}

/// The one smoothing kernel of the crate ([`smooth_as`], and the fit's own
/// smoothing of the measurement): `values` in dB on a grid of
/// `points_per_octave`, smoothed over `1/fraction` octave in `domain`. A
/// `fraction` that is not finite or not positive, or a grid density of zero,
/// gives the values back unchanged; a result that is not finite keeps the
/// point's own value.
pub(crate) fn smooth_magnitudes(
    values: &[f64],
    valid: Option<&[bool]>,
    points_per_octave: u16,
    fraction: f64,
    domain: SmoothingDomain,
) -> Vec<f64> {
    let h = f64::from(points_per_octave) / (2.0 * fraction);
    if !(h.is_finite() && h > 0.0 && h <= 1024.0) {
        return values.to_vec();
    }
    let w = window_weights(h);
    let k = w.len() / 2;
    match domain {
        SmoothingDomain::Db => smooth_values(values, valid, &w, k),
        SmoothingDomain::Power => {
            let power: Vec<f64> = values.iter().map(|m| 10f64.powf(m / 10.0)).collect();
            smooth_values(&power, valid, &w, k)
                .iter()
                .zip(values)
                .map(|(p, own)| {
                    let db = 10.0 * p.log10();
                    if db.is_finite() { db } else { *own }
                })
                .collect()
        }
    }
}

fn smooth_values(y: &[f64], valid: Option<&[bool]>, w: &[f64], k: usize) -> Vec<f64> {
    let last = y.len().saturating_sub(1);
    let ok = |i: usize| valid.is_none_or(|v| v.get(i).copied().unwrap_or(false));
    y.iter()
        .enumerate()
        .map(|(i, own)| {
            let mut acc = 0.0;
            let mut wsum = 0.0;
            for (idx, wj) in w.iter().enumerate() {
                // The point at offset idx - k from i, clamped to the grid
                // (end values held).
                let j = i.saturating_add(idx).saturating_sub(k).min(last);
                if ok(j)
                    && let Some(v) = y.get(j)
                {
                    acc += wj * v;
                    wsum += wj;
                }
            }
            if wsum > 1e-12 { acc / wsum } else { *own }
        })
        .collect()
}

/// The repeat-to-repeat spread per point: the largest minus the smallest
/// magnitude among `repeats` (all on one grid), dB. One response gives zeros.
///
/// Errors: [`MeasureError::Invalid`] for no response, more than
/// [`MAX_REPEATS`], a malformed one, or different grids.
pub fn repeat_spread_db(repeats: &[Response]) -> Result<Vec<f64>, MeasureError> {
    let first = repeats
        .first()
        .ok_or_else(|| invalid("no response to compare"))?;
    if repeats.len() > MAX_REPEATS {
        return Err(invalid(format!(
            "{} repeats (at most {MAX_REPEATS})",
            repeats.len()
        )));
    }
    for r in repeats {
        check(r)?;
        if r.grid != first.grid {
            return Err(invalid("repeats on different grids"));
        }
    }
    Ok((0..first.mag_db.len())
        .map(|i| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for r in repeats {
                if let Some(v) = r.mag_db.get(i) {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
            }
            let s = hi - lo;
            if s.is_finite() { s } else { 0.0 }
        })
        .collect())
}

/// The trusted-point mask (PLAN-P5 section 1.1 step 9): a point is trusted
/// when `signal` marks it trusted (or has no mask), its level is at least
/// `rule.min_snr_db` above `noise` (the pre-roll's noise floor on the same
/// grid, when given), and its repeat spread (when given) is at most
/// `rule.max_spread_db`.
///
/// Errors: [`MeasureError::Invalid`] for a malformed response, a noise
/// floor on another grid, a spread of the wrong length, or a non-finite rule.
pub fn trust_mask(
    signal: &Response,
    noise: Option<&Response>,
    spread_db: Option<&[f64]>,
    rule: &TrustRule,
) -> Result<Vec<bool>, MeasureError> {
    if !rule.min_snr_db.is_finite() || !rule.max_spread_db.is_finite() {
        return Err(invalid("non-finite trust rule"));
    }
    check(signal)?;
    let n = signal.mag_db.len();
    if let Some(nf) = noise {
        check(nf)?;
        if nf.grid != signal.grid {
            return Err(invalid("noise floor on another grid"));
        }
    }
    if let Some(s) = spread_db
        && s.len() != n
    {
        return Err(invalid(format!("{} spread values for {n} points", s.len())));
    }
    Ok((0..n)
        .map(|i| {
            let own = signal
                .valid
                .as_ref()
                .is_none_or(|v| v.get(i).copied().unwrap_or(false));
            let level = signal.mag_db.get(i);
            let snr_ok = match (noise, level) {
                (Some(nf), Some(s)) => nf.mag_db.get(i).is_some_and(|nv| s - nv >= rule.min_snr_db),
                (None, Some(_)) => true,
                (_, None) => false,
            };
            let spread_ok = spread_db.is_none_or(|s| {
                s.get(i)
                    .is_some_and(|v| v.is_finite() && *v <= rule.max_spread_db)
            });
            own && snr_ok && spread_ok
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::needless_range_loop)]
mod tests_wp_p5resp {
    use super::*;
    use dspi_proto::FilterType;
    use dspi_proto::dsp::{self, Band, Coeffs};
    use proptest::prelude::*;

    /// Runs `x` through one biquad (direct form I).
    fn biquad(c: &Coeffs, x: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&xn| {
                let y = c.b0 * xn + c.b1 * x1 + c.b2 * x2 - c.a1 * y1 - c.a2 * y2;
                x2 = x1;
                x1 = xn;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }

    fn wrap180(d: f64) -> f64 {
        (d + 180.0).rem_euclid(360.0) - 180.0
    }

    #[test]
    fn wp_p5resp_known_response_on_the_grid_within_0_05_db() {
        let bands = [
            Band {
                filter_type: FilterType::Peaking,
                freq: 1000.0,
                q: 2.0,
                gain_db: 6.0,
                bypass: false,
            },
            Band {
                filter_type: FilterType::HighPass,
                freq: 60.0,
                q: 0.707,
                gain_db: 0.0,
                bypass: false,
            },
            Band {
                filter_type: FilterType::LowShelf,
                freq: 200.0,
                q: 0.707,
                gain_db: -4.0,
                bypass: false,
            },
        ];
        // The cascade's impulse response, 1 s long (it decays well inside).
        let len = 48_000;
        let mut sig: Vec<f64> = (0..len).map(|n| if n == 0 { 1.0 } else { 0.0 }).collect();
        for b in &bands {
            sig = biquad(&dsp::coefficients(b), &sig);
        }
        let r = from_impulse(&sig, dsp::SAMPLE_RATE, &LogGrid::STANDARD, 0.0).unwrap();
        let freqs = LogGrid::STANDARD.freqs().unwrap();
        let phase = r.phase_deg.as_ref().unwrap();
        let mut worst = 0.0f64;
        let mut worst_ph = 0.0f64;
        for (i, f) in freqs.iter().enumerate() {
            if *f < 20.0 || *f > 20_000.0 {
                continue;
            }
            let want = dsp::response_at(*f, &bands);
            worst = worst.max((r.mag_db[i] - want).abs());
            let want_ph = dsp::phase_at(*f, &bands);
            worst_ph = worst_ph.max(wrap180(phase[i] - want_ph).abs());
        }
        assert!(worst < 0.05, "worst magnitude error {worst} dB");
        assert!(worst_ph < 0.5, "worst phase error {worst_ph} deg");
        assert!(r.valid.as_ref().unwrap().iter().all(|v| *v));
    }

    #[test]
    fn wp_p5resp_delay_is_unwrapped_and_time_zero_removes_it() {
        let fs = 48_000.0;
        let d = 240; // 5 ms
        let mut ir = vec![0.0; 4096];
        ir[d] = 1.0;
        let r = from_impulse(&ir, fs, &LogGrid::STANDARD, 0.0).unwrap();
        let freqs = LogGrid::STANDARD.freqs().unwrap();
        let ph = r.phase_deg.unwrap();
        for (i, f) in freqs.iter().enumerate() {
            let want = -360.0 * f * d as f64 / fs;
            assert!((ph[i] - want).abs() < 1e-6, "{f} Hz: {} vs {want}", ph[i]);
            assert!(r.mag_db[i].abs() < 1e-9);
        }
        let r0 = from_impulse(&ir, fs, &LogGrid::STANDARD, d as f64 / fs).unwrap();
        assert!(r0.phase_deg.unwrap().iter().all(|p| p.abs() < 1e-6));
    }

    #[test]
    fn wp_p5resp_points_above_nyquist_are_untrusted() {
        let mut ir = vec![0.0; 64];
        ir[0] = 1.0;
        let r = from_impulse(&ir, 16_000.0, &LogGrid::STANDARD, 0.0).unwrap();
        let freqs = LogGrid::STANDARD.freqs().unwrap();
        let v = r.valid.unwrap();
        for (i, f) in freqs.iter().enumerate() {
            assert_eq!(v[i], *f <= 8_000.0, "{f}");
        }
    }

    #[test]
    fn wp_p5resp_power_spectrum_on_the_grid() {
        // A power of 1/f (pink) reads -10 dB a decade on the grid.
        let p: Vec<f64> = (0..30_000)
            .map(|k| if k == 0 { 0.0 } else { 1.0 / k as f64 })
            .collect();
        let r = from_power_spectrum(&p, 1.0, &LogGrid::STANDARD).unwrap();
        let freqs = LogGrid::STANDARD.freqs().unwrap();
        for (i, f) in freqs.iter().enumerate().filter(|(_, f)| **f >= 100.0) {
            assert!((r.mag_db[i] + 10.0 * f.log10()).abs() < 0.05, "{f}");
        }
        // The complex path agrees with the power path.
        let c: Vec<[f64; 2]> = p.iter().map(|v| [v.sqrt(), 0.0]).collect();
        let rc = from_spectrum(&c, 1.0, &LogGrid::STANDARD).unwrap();
        for i in 0..freqs.len() {
            assert!((rc.mag_db[i] - r.mag_db[i]).abs() < 1e-9);
        }
    }

    fn triangle_peak(height: f64, centre: usize, half_base: usize) -> Response {
        let n = LogGrid::STANDARD.freqs().unwrap().len();
        let mag = (0..n)
            .map(|i| {
                let d = i.abs_diff(centre) as f64;
                (height * (1.0 - d / half_base as f64)).max(0.0)
            })
            .collect();
        Response {
            grid: LogGrid::STANDARD,
            mag_db: mag,
            phase_deg: None,
            valid: None,
            spl: false,
        }
    }

    #[test]
    fn wp_p5resp_third_octave_smoothing_halves_a_third_octave_peak() {
        // A peak 1/3 octave wide at its base (±16 points at 1/96 octave),
        // centred at 1 kHz.
        let centre = (96.0 * (1000.0f64 / 10.0).log2()).round() as usize;
        for h in [3.0, 6.0, 12.0] {
            let r = triangle_peak(h, centre, 16);
            let s = smooth(&r, 3).unwrap();
            let peak = s.mag_db.iter().cloned().fold(f64::MIN, f64::max);
            assert!((peak - h / 2.0).abs() < 0.5, "{h} dB peak -> {peak}");
            // Flat regions stay put.
            assert!(s.mag_db[100].abs() < 1e-12);
        }
    }

    #[test]
    fn wp_p5resp_smoothing_keeps_a_tilt_and_skips_untrusted_points() {
        let n = LogGrid::STANDARD.freqs().unwrap().len();
        let tilt: Vec<f64> = (0..n).map(|i| -3.0 * i as f64 / 96.0).collect();
        let line = Response {
            grid: LogGrid::STANDARD,
            mag_db: tilt.clone(),
            phase_deg: Some(tilt.clone()),
            valid: None,
            spl: false,
        };
        // An untrusted spike on a flat curve.
        let mut valid = vec![true; n];
        let mut spiky = vec![0.0; n];
        for i in 500..510 {
            spiky[i] = 40.0;
            valid[i] = false;
        }
        let spike = Response {
            grid: LogGrid::STANDARD,
            mag_db: spiky.clone(),
            phase_deg: None,
            valid: Some(valid.clone()),
            spl: false,
        };
        for frac in [1, 3, 6, 12, 24, 48] {
            // Away from the ends a straight line is unchanged.
            let s = smooth(&line, frac).unwrap();
            for i in 60..n - 60 {
                assert!((s.mag_db[i] - tilt[i]).abs() < 1e-9, "1/{frac} at {i}");
                assert!((s.phase_deg.as_ref().unwrap()[i] - tilt[i]).abs() < 1e-9);
            }
            // The spike leaks into no trusted neighbour and keeps its flags.
            let s = smooth(&spike, frac).unwrap();
            for i in 0..n {
                if valid[i] {
                    assert!(s.mag_db[i].abs() < 1e-12, "1/{frac} at {i}");
                }
            }
            assert_eq!(s.valid.as_ref().unwrap(), &valid);
        }
        assert!(smooth(&line, 0).is_err());
        assert!(smooth(&line, 49).is_err());
    }

    #[test]
    fn wp_p5resp_trust_mask_from_snr_and_spread() {
        let g = LogGrid {
            min_hz: 100.0,
            max_hz: 800.0,
            points_per_octave: 1,
        };
        let sig = Response {
            grid: g,
            mag_db: vec![60.0, 60.0, 60.0, 60.0],
            phase_deg: None,
            valid: Some(vec![true, true, true, false]),
            spl: false,
        };
        let noise = Response {
            mag_db: vec![30.0, 45.0, 30.0, 30.0],
            valid: None,
            ..sig.clone()
        };
        let spread = [0.5, 0.5, 1.5, 0.0];
        let m = trust_mask(&sig, Some(&noise), Some(&spread), &TrustRule::DEFAULT).unwrap();
        assert_eq!(m, vec![true, false, false, false]);
        let other = Response {
            grid: LogGrid::STANDARD,
            ..noise.clone()
        };
        assert!(trust_mask(&sig, Some(&other), None, &TrustRule::DEFAULT).is_err());
        let r2 = Response {
            mag_db: vec![61.0, 59.5, 60.0, 60.0],
            ..sig.clone()
        };
        assert_eq!(
            repeat_spread_db(&[sig.clone(), r2]).unwrap(),
            vec![1.0, 0.5, 0.0, 0.0]
        );
        assert!(repeat_spread_db(&[]).is_err());
    }

    #[test]
    fn wp_p5resp_resample_round_trip() {
        let n = LogGrid::STANDARD.freqs().unwrap().len();
        let mag: Vec<f64> = (0..n).map(|i| (i as f64 / 50.0).sin()).collect();
        let r = Response {
            grid: LogGrid::STANDARD,
            mag_db: mag.clone(),
            phase_deg: None,
            valid: None,
            spl: true,
        };
        // Twice as fine, from below the standard grid to 12 kHz.
        let fine = LogGrid {
            min_hz: 10.0 / 2f64.powf(4.0 / 192.0),
            max_hz: 12_000.0,
            points_per_octave: 192,
        };
        let up = resample(&r, &fine).unwrap();
        assert!(up.spl);
        let v = up.valid.as_ref().unwrap();
        assert!(!v[0] && v[4] && v[v.len() - 1]);
        let back = resample(&up, &LogGrid::STANDARD).unwrap();
        let f = LogGrid::STANDARD.freqs().unwrap();
        let bv = back.valid.as_ref().unwrap();
        for (i, (b, m)) in back.mag_db.iter().zip(&mag).enumerate() {
            assert_eq!(bv[i], f[i] <= 12_000.0, "{i}");
            if bv[i] {
                assert!((b - m).abs() < 1e-6, "{i}");
            }
        }
    }

    fn arb_response() -> impl Strategy<Value = Response> {
        (
            1.0f64..1000.0,
            0.5f64..12.0,
            1u16..=96,
            proptest::collection::vec(-200.0f64..200.0, 1..64),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(min, oct, ppo, vals, ph, va)| {
                let grid = LogGrid {
                    min_hz: min,
                    max_hz: min * oct.exp2(),
                    points_per_octave: ppo,
                };
                let n = grid.freqs().map(|f| f.len()).unwrap_or(0);
                let mag: Vec<f64> = (0..n).map(|i| vals[i % vals.len()]).collect();
                Response {
                    grid,
                    phase_deg: ph.then(|| mag.iter().map(|m| m * 3.0).collect()),
                    valid: va.then(|| mag.iter().map(|m| *m > 0.0).collect()),
                    mag_db: mag,
                    spl: false,
                }
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn proptest_wp_p5resp_from_impulse_never_panics(
            ir in proptest::collection::vec(proptest::num::f64::ANY, 0..300),
            fs in proptest::num::f64::ANY,
            t0 in -0.02f64..0.02,
        ) {
            if let Ok(r) = from_impulse(&ir, fs, &LogGrid::STANDARD, t0) {
                prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
                prop_assert!(r.phase_deg.unwrap().iter().all(|v| v.is_finite()));
            }
        }

        #[test]
        fn proptest_wp_p5resp_finite_impulse_gives_finite_response(
            ir in proptest::collection::vec(-1e6f64..1e6, 1..300),
            fs in 8_000.0f64..192_000.0,
            t0 in -0.02f64..0.02,
        ) {
            let r = from_impulse(&ir, fs, &LogGrid::STANDARD, t0).unwrap();
            prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
            prop_assert!(r.phase_deg.unwrap().iter().all(|v| v.is_finite()));
        }

    }

    // Cheap: the default case count (the proptest step's 20000).
    proptest! {
        #[test]
        fn proptest_wp_p5resp_spectra_never_panic(
            bins in proptest::collection::vec((proptest::num::f64::ANY, proptest::num::f64::ANY), 0..400),
            bin_hz in proptest::num::f64::ANY,
            min in proptest::num::f64::ANY,
            ppo in 0u16..400,
        ) {
            let grid = LogGrid { min_hz: min, max_hz: 20_000.0, points_per_octave: ppo };
            let c: Vec<[f64; 2]> = bins.iter().map(|(a, b)| [*a, *b]).collect();
            if let Ok(r) = from_spectrum(&c, bin_hz, &grid) {
                prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
                prop_assert!(r.phase_deg.unwrap().iter().all(|v| v.is_finite()));
            }
            let p: Vec<f64> = bins.iter().map(|(a, _)| *a).collect();
            if let Ok(r) = from_power_spectrum(&p, bin_hz, &grid) {
                prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
            }
        }

        #[test]
        fn proptest_wp_p5resp_smooth_resample_spread_trust_never_panic(
            r in arb_response(),
            frac in 0u8..60,
            other in arb_response(),
        ) {
            if let Ok(s) = smooth(&r, frac) {
                prop_assert_eq!(s.mag_db.len(), r.mag_db.len());
                // Smoothing never leaves the input's range.
                let lo = r.mag_db.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = r.mag_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                prop_assert!(s.mag_db.iter().all(|v| v.is_finite() && *v >= lo - 1e-9 && *v <= hi + 1e-9));
            }
            if let Ok(x) = resample(&r, &other.grid) {
                prop_assert!(x.mag_db.iter().all(|v| v.is_finite()));
            }
            if let Ok(s) = repeat_spread_db(&[r.clone(), other.clone()]) {
                prop_assert!(s.iter().all(|v| v.is_finite() && *v >= 0.0));
            }
            let _ = trust_mask(&r, Some(&other), None, &TrustRule::DEFAULT);
            let _ = trust_mask(&r, None, Some(&other.mag_db), &TrustRule::DEFAULT);
        }
    }
}
