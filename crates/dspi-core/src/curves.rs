//! Stateless helpers: filter response curves and version facts.
//!
//! The old core's `dsp_math` did these; `dspi_proto::dsp` has the same maths
//! (biquad coefficients for every filter type). The curve grid is 201 points,
//! log-spaced 10 Hz..20 kHz, as the desktop graph and the Android app expect.
//!
//! What comes from where:
//! - PEQ types 1..13 except the Linkwitz Transform:
//!   `dspi_proto::dsp::coefficients`.
//! - Crossover types 32..63: decoded by `dspi_proto::xover::meta`, designed
//!   here in `f64`. `dspi_proto::xover::sections` passes each section's
//!   corner and Q through `f32`, which moves a steep slope by several µdB;
//!   the old core (and the firmware) design in full precision, and the graph
//!   must match the old core to 1e-6 dB.
//! - The Linkwitz Transform: here, as in the old core (`dspi_proto::dsp`
//!   draws it flat, having no target Q).
//! - Evaluation: |H|² in the RBJ `φ = sin²(ω/2)` form, as in the old core.
//!   Unlike evaluating at e^jω directly, it keeps the resonance of low,
//!   narrow sections (a 10 Hz, Q 20 bell).
//!
//! Nothing here panics: inputs are plain floats, and a nonsensical band
//! (frequency at or above Nyquist, unknown type, NaN) draws as no
//! contribution or a finite value.
//!
//! Owned by WP C12 (docs/core-merge/PLAN.md).

use std::f64::consts::PI;

use dspi_proto::FilterType;
use dspi_proto::dsp::{self, Band, Coeffs};
use dspi_proto::xover::{self, Family};

use crate::records::{CoreInfo, FilterParams};

/// Points in a curve from [`compute_magnitude_curve`] / [`compute_phase_curve`].
pub const CURVE_POINTS: usize = dsp::POINTS;

// The desktop graph and the apps index the curve by point; the grid is fixed.
const _: () = assert!(CURVE_POINTS == 201);

/// The response is drawn at the device's rate.
const FS: f64 = dsp::SAMPLE_RATE;

/// Magnitude in dB at one frequency (Hz) for a chain of bands.
///
/// Bypassed and off bands are skipped; an empty chain is 0 dB. A total
/// power below -300 dB (a notch's exact zero) reads -300 dB.
pub fn compute_response(filters: Vec<FilterParams>, freq: f32) -> f32 {
    let w = omega(f64::from(freq));
    let power: f64 = filters
        .iter()
        .flat_map(active_sections)
        .map(|c| mag_squared(&c, w))
        .product();
    // The C ABI's and the bindings' type; the precision is for display.
    to_db(power) as f32
}

/// 201-point magnitude curve, dB, on the [`curve_freq`] grid.
///
/// Each band is designed once, then evaluated at every point. Same rules
/// as [`compute_response`].
pub fn compute_magnitude_curve(filters: Vec<FilterParams>) -> Vec<f64> {
    let sections: Vec<Coeffs> = filters.iter().flat_map(active_sections).collect();
    grid()
        .map(|f| {
            let w = omega(f);
            to_db(sections.iter().map(|c| mag_squared(c, w)).product())
        })
        .collect()
}

/// 201-point phase curve in degrees on the same grid; continuous when
/// `unwrap`, else wrapped to ±180°.
///
/// The unwrapped curve starts at the wrapped value of the first point and
/// then takes, between neighbours, the step in (-180°, 180°].
pub fn compute_phase_curve(filters: Vec<FilterParams>, unwrap: bool) -> Vec<f64> {
    let sections: Vec<Coeffs> = filters.iter().flat_map(active_sections).collect();
    let wrapped = grid().map(|f| wrapped_phase(&sections, f));
    if !unwrap {
        return wrapped.collect();
    }
    let mut out: Vec<f64> = Vec::with_capacity(CURVE_POINTS);
    for p in wrapped {
        let next = match out.last() {
            None => p,
            Some(&prev) => {
                // The step to the nearest copy of p, in (-180, 180].
                let mut d = (p - prev).rem_euclid(360.0);
                if d > 180.0 {
                    d -= 360.0;
                }
                prev + d
            }
        };
        out.push(next);
    }
    out
}

/// Frequency (Hz) of curve point `i` (0..200): log-spaced, 10 Hz at 0,
/// 20 kHz at 200. A larger `i` extrapolates on the same spacing.
pub fn curve_freq(i: u32) -> f64 {
    let (lo, hi) = (dsp::MIN_HZ.log10(), dsp::MAX_HZ.log10());
    // CURVE_POINTS - 1 is the constant 200 (asserted above).
    let steps = (CURVE_POINTS - 1) as f64;
    10f64.powf(lo + (f64::from(i) / steps) * (hi - lo))
}

/// Whether `map` (4 bytes, `DspState.crossfeed_pair_map` encoding) is a pair
/// map the firmware accepts for `num_pairs` stereo pairs (4 on RP2350, 2 on
/// RP2040): every pair gets two distinct non-PDM outputs, no output is used
/// twice, bytes beyond `num_pairs` are 0.
///
/// Byte `p` is pair `p`: `0x00` is the default pairing (`2p`, `2p+1`), any
/// other value is `(right << 4) | left`. The non-PDM outputs are
/// `0..2 * num_pairs`. A map that is not exactly 4 bytes, or a `num_pairs`
/// of 0 or above 4, is invalid (false), never a panic.
pub fn crossfeed_pair_map_valid(map: Vec<u8>, num_pairs: u8) -> bool {
    let Ok(map) = <[u8; 4]>::try_from(map.as_slice()) else {
        return false;
    };
    if num_pairs == 0 || num_pairs > 4 {
        return false;
    }
    // num_pairs <= 4, so there are at most 8 outputs, every output below is
    // < 8 once checked, and every shift is in range for a u16.
    let outputs = num_pairs.saturating_mul(2);
    let mut used = 0u16;
    for (p, b) in (0u8..).zip(map) {
        if p >= num_pairs {
            if b != 0 {
                return false;
            }
            continue;
        }
        let (l, r) = if b == 0 {
            let l = p.saturating_mul(2);
            (l, l.saturating_add(1))
        } else {
            (b & 0x0F, b >> 4)
        };
        if l >= outputs || r >= outputs || l == r {
            return false;
        }
        let bits = (1u16 << l) | (1u16 << r);
        if used & bits != 0 {
            return false;
        }
        used |= bits;
    }
    true
}

/// Version facts of the linked core: this crate's version, the wire format
/// versions it reads and writes, the bulk image size and the curve size.
pub fn core_info() -> CoreInfo {
    use dspi_proto::generated::{BULK_SIZE, wire::WIRE_FORMAT_VERSION};
    // The header constants fit the record's fields (checked at compile time),
    // so the casts below are exact.
    const _: () = assert!(WIRE_FORMAT_VERSION <= u8::MAX as _);
    const _: () = assert!(BULK_SIZE <= u32::MAX as usize);
    CoreInfo {
        ffi_version: env!("CARGO_PKG_VERSION").to_owned(),
        wire_format_version: WIRE_FORMAT_VERSION as u8,
        min_wire_format_version: dspi_proto::wire_compat::MIN_WIRE_FORMAT_VERSION,
        bulk_params_size: BULK_SIZE as u32,
        magnitude_points: CURVE_POINTS as u32,
    }
}

// ── Design ──────────────────────────────────────────────────────────

/// The 201 grid frequencies, in order.
fn grid() -> impl Iterator<Item = f64> {
    // CURVE_POINTS is the constant 201.
    (0..CURVE_POINTS as u32).map(curve_freq)
}

/// Angular frequency (rad/sample) of `freq` Hz.
fn omega(freq: f64) -> f64 {
    2.0 * PI * freq / FS
}

/// Phase in degrees of a cascade at `freq` Hz, wrapped to [-180, 180].
fn wrapped_phase(sections: &[Coeffs], freq: f64) -> f64 {
    let w = omega(freq);
    wrap_degrees(
        sections
            .iter()
            .map(|c| phase(c, w))
            .sum::<f64>()
            .to_degrees(),
    )
}

/// Power ratio to dB, floored at -300 dB.
fn to_db(power: f64) -> f64 {
    10.0 * power.max(1e-30).log10()
}

/// Wrap degrees into [-180, 180]; a non-finite phase is drawn as 0.
fn wrap_degrees(deg: f64) -> f64 {
    let mut d = deg % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    if d.is_finite() { d } else { 0.0 }
}

/// The biquad sections a band contributes: none when it is off or bypassed.
fn active_sections(p: &FilterParams) -> Vec<Coeffs> {
    if p.bypass {
        return Vec::new();
    }
    if let Some(m) = xover::meta(p.filter_type) {
        return crossover_sections(m, f64::from(p.freq));
    }
    match FilterType::from_raw(p.filter_type) {
        FilterType::Flat => Vec::new(),
        FilterType::LinkwitzTransform => vec![linkwitz_transform(p)],
        t => vec![dsp::coefficients(&Band {
            filter_type: t,
            freq: p.freq,
            q: p.q,
            gain_db: p.gain,
            bypass: false,
        })],
    }
}

/// Linkwitz Transform: zeros at the driver corner (f0 = `freq`, Q0 = `q`),
/// poles at the target (fp = `gain`, in Hz; Qp = `qp`). Both corners are
/// tan-prewarped, as in the firmware. fp or f0 <= 0 is flat.
fn linkwitz_transform(p: &FilterParams) -> Coeffs {
    let f0 = f64::from(p.freq);
    let fp = f64::from(p.gain);
    if !(fp > 0.0 && f0 > 0.0) {
        return Coeffs::PASSTHROUGH;
    }
    let q0 = f64::from(p.q).max(0.1);
    let qp = f64::from(p.qp).max(0.1);
    let g0 = (PI * f0 / FS).tan();
    let gp = (PI * fp / FS).tan();
    let na0 = 1.0 + gp / qp + gp * gp;
    if !na0.is_normal() {
        return Coeffs::PASSTHROUGH;
    }
    Coeffs {
        b0: (1.0 + g0 / q0 + g0 * g0) / na0,
        b1: 2.0 * (g0 * g0 - 1.0) / na0,
        b2: (1.0 - g0 / q0 + g0 * g0) / na0,
        a1: 2.0 * (gp * gp - 1.0) / na0,
        a2: (1.0 - gp / qp + gp * gp) / na0,
    }
}

/// The biquad cascade of a crossover band (firmware `crossover.c`): an analog
/// prototype through the bilinear transform with prewarping.
fn crossover_sections(m: xover::Meta, fc: f64) -> Vec<Coeffs> {
    let mut s = Vec::with_capacity(4);
    // NaN would pass through clamp; draw such a band flat.
    if fc.is_nan() {
        return s;
    }
    let fc = fc.clamp(10.0, FS * 0.45);
    let omega_a = 2.0 * FS * (PI * fc / FS).tan();
    let hp = m.high_pass;
    match m.family {
        Family::Butterworth => butterworth(&mut s, m.order, hp, omega_a),
        Family::LinkwitzRiley => {
            if m.order == 2 {
                // LR2: one biquad with a double real pole.
                s.push(section_2nd(1.0, 0.0, omega_a, hp));
            } else {
                // LR(2N) = BW(N)²: design BW(N) and repeat every section.
                butterworth(&mut s, m.order / 2, hp, omega_a);
                s.extend_from_within(..);
            }
        }
        Family::Bessel => {
            for &(sigma, w) in bessel_poles(m.order) {
                s.push(section_2nd(sigma, w, omega_a, hp));
            }
        }
    }
    s
}

/// Butterworth of `order` (1..8): an odd order's real pole first, then the
/// pole pairs.
fn butterworth(s: &mut Vec<Coeffs>, order: u8, hp: bool, omega_a: f64) {
    let odd = order % 2 == 1;
    if odd {
        s.push(section_1st(1.0, omega_a, hp));
    }
    let n = f64::from(order);
    for p in 0..order / 2 {
        let k = f64::from(p);
        let theta = if odd {
            PI * (k + 1.0) / n
        } else {
            PI * (2.0 * k + 1.0) / (2.0 * n)
        };
        s.push(section_2nd(theta.cos(), theta.sin(), omega_a, hp));
    }
}

/// Bessel (-3 dB normalised) analog pole pairs (σ, ω), from firmware
/// `crossover.c` (the same table as `dspi_proto::xover`, which keeps it
/// private).
fn bessel_poles(order: u8) -> &'static [(f64, f64)] {
    match order {
        2 => &[(1.10160, 0.63601)],
        4 => &[(1.37007, 0.41025), (0.99521, 1.25711)],
        6 => &[(1.57149, 0.32090), (1.38186, 0.97147), (0.93066, 1.66186)],
        8 => &[
            (1.75741, 0.27287),
            (1.63694, 0.82280),
            (1.37384, 1.38836),
            (0.89287, 1.99833),
        ],
        _ => &[],
    }
}

/// Second-order section from an analog pole pair (σ, ω). A high pass uses the
/// LP→HP reciprocal (σ, ω)/(σ² + ω²): a no-op for BW/LR, required for Bessel.
fn section_2nd(mut sigma_n: f64, mut omega_n: f64, omega_a: f64, hp: bool) -> Coeffs {
    if hp {
        let r2 = sigma_n * sigma_n + omega_n * omega_n;
        if r2 > 0.0 {
            sigma_n /= r2;
            omega_n /= r2;
        }
    }
    let sigma = sigma_n * omega_a;
    let w = omega_n * omega_a;
    let k = 2.0 * FS;
    let a = 2.0 * sigma;
    let b = sigma * sigma + w * w;
    let inv = 1.0 / (k * k + a * k + b);
    let (b0, b1, b2) = if hp {
        let kk = k * k * inv;
        (kk, -2.0 * kk, kk)
    } else {
        let bb = b * inv;
        (bb, 2.0 * bb, bb)
    };
    Coeffs {
        b0,
        b1,
        b2,
        a1: 2.0 * (b - k * k) * inv,
        a2: (k * k - a * k + b) * inv,
    }
}

/// First-order real-pole section (Butterworth odd orders).
fn section_1st(mut sigma_n: f64, omega_a: f64, hp: bool) -> Coeffs {
    if hp && sigma_n > 0.0 {
        sigma_n = 1.0 / sigma_n;
    }
    let sigma = sigma_n * omega_a;
    let k = 2.0 * FS;
    let inv = 1.0 / (k + sigma);
    let (b0, b1) = if hp {
        (k * inv, -k * inv)
    } else {
        (sigma * inv, sigma * inv)
    };
    Coeffs {
        b0,
        b1,
        b2: 0.0,
        a1: (sigma - k) * inv,
        a2: 0.0,
    }
}

// ── Evaluation ──────────────────────────────────────────────────────

/// |H(e^jω)|² of one biquad, in φ = sin²(ω/2) (the RBJ plotting form).
fn mag_squared(c: &Coeffs, w: f64) -> f64 {
    let s = (w / 2.0).sin();
    let phi = s * s;
    let sb = c.b0 + c.b1 + c.b2;
    let sa = 1.0 + c.a1 + c.a2;
    let num = sb * sb - 4.0 * (c.b0 * c.b1 + c.b1 * c.b2 + 4.0 * c.b0 * c.b2) * phi
        + 16.0 * c.b0 * c.b2 * phi * phi;
    let den = sa * sa - 4.0 * (c.a1 + c.a1 * c.a2 + 4.0 * c.a2) * phi + 16.0 * c.a2 * phi * phi;
    if den > 0.0 { num.max(0.0) / den } else { 1.0 }
}

/// Phase (radians) of one biquad at ω.
fn phase(c: &Coeffs, w: f64) -> f64 {
    let (sw, cw) = w.sin_cos();
    let (s2w, c2w) = (2.0 * w).sin_cos();
    let nr = c.b0 + c.b1 * cw + c.b2 * c2w;
    let ni = -(c.b1 * sw + c.b2 * s2w);
    let dr = 1.0 + c.a1 * cw + c.a2 * c2w;
    let di = -(c.a1 * sw + c.a2 * s2w);
    ni.atan2(nr) - di.atan2(dr)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c12 {
    use super::*;
    use proptest::prelude::*;

    /// A reference case from the old core (`curves_ref.rs`).
    struct Case {
        name: &'static str,
        /// (type, freq, q, gain, qp, bypass)
        bands: &'static [(u8, f32, f32, f32, f32, bool)],
        /// `response_at` at [`RESP_HZ`].
        resp: [f32; 6],
        /// (point, magnitude dB, wrapped phase, unwrapped phase)
        pts: &'static [(usize, f64, f32, f32)],
    }

    const RESP_HZ: [f32; 6] = [20.0, 300.0, 1000.0, 4000.0, 10000.0, 20000.0];

    include!("curves_ref.rs");

    fn band(filter_type: u8, freq: f32, q: f32, gain: f32) -> FilterParams {
        FilterParams {
            filter_type,
            bypass: false,
            freq,
            q,
            gain,
            qp: 0.0,
        }
    }

    fn resp(f: &[FilterParams], hz: f32) -> f32 {
        compute_response(f.to_vec(), hz)
    }

    fn phase_at(f: &[FilterParams], hz: f32) -> f64 {
        let s: Vec<Coeffs> = f.iter().flat_map(active_sections).collect();
        wrapped_phase(&s, f64::from(hz))
    }

    /// Distance between two angles in degrees, modulo 360.
    fn angle_diff(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    // ── Numeric parity with the old core ────────────────────────────

    #[test]
    fn wp_c12_curves_match_the_old_cores_values() {
        assert_eq!(REFERENCE.len(), 25);
        for case in REFERENCE {
            let bands: Vec<FilterParams> = case
                .bands
                .iter()
                .map(|&(filter_type, freq, q, gain, qp, bypass)| FilterParams {
                    filter_type,
                    bypass,
                    freq,
                    q,
                    gain,
                    qp,
                })
                .collect();
            let mag = compute_magnitude_curve(bands.clone());
            let wrapped = compute_phase_curve(bands.clone(), false);
            let unwrapped = compute_phase_curve(bands.clone(), true);
            assert_eq!(mag.len(), CURVE_POINTS);
            for &(i, m, pw, pu) in case.pts {
                // Magnitude: the plan's bar, 1e-6 dB.
                assert!(
                    (mag[i] - m).abs() < 1e-6,
                    "{}: magnitude at point {i}: {} dB, old core {m} dB",
                    case.name,
                    mag[i]
                );
                // Phase: the old core rounded through f32 (and its frequency
                // too), this one stays in f64, so allow f32 rounding.
                assert!(
                    angle_diff(wrapped[i], f64::from(pw)) < 1e-3,
                    "{}: wrapped phase at point {i}: {}, old core {pw}",
                    case.name,
                    wrapped[i]
                );
                assert!(
                    (unwrapped[i] - f64::from(pu)).abs() < 1e-3,
                    "{}: unwrapped phase at point {i}: {}, old core {pu}",
                    case.name,
                    unwrapped[i]
                );
            }
            for (&hz, &want) in RESP_HZ.iter().zip(case.resp.iter()) {
                let got = compute_response(bands.clone(), hz);
                // Both are f64 rounded to f32: within 1e-6 dB or one f32 step.
                let tol = 1e-6f32.max(want.abs() * 2.0 * f32::EPSILON);
                assert!(
                    (got - want).abs() <= tol,
                    "{}: response at {hz} Hz: {got} dB, old core {want} dB",
                    case.name
                );
            }
        }
    }

    // ── The old core's dsp_math tests, ported ───────────────────────

    #[test]
    fn wp_c12_flat_is_zero_db() {
        assert!(resp(&[band(0, 1000.0, 0.707, 0.0)], 1000.0).abs() < 0.001);
        assert_eq!(compute_response(Vec::new(), 1000.0), 0.0);
        let c = compute_magnitude_curve(Vec::new());
        assert_eq!(c.len(), CURVE_POINTS);
        assert!(c.iter().all(|v| v.abs() < 1e-9));
        let p = compute_phase_curve(Vec::new(), true);
        assert!(p.iter().all(|v| v.abs() < 1e-9));
    }

    #[test]
    fn wp_c12_peaking_hits_gain_at_center_only() {
        let f = [band(1, 1000.0, 1.0, 6.0)];
        assert!((resp(&f, 1000.0) - 6.0).abs() < 0.01);
        assert!(resp(&f, 20.0).abs() < 0.5);
    }

    #[test]
    fn wp_c12_bypassed_band_is_ignored() {
        let mut f = band(1, 1000.0, 1.0, 6.0);
        f.bypass = true;
        assert!(resp(&[f], 1000.0).abs() < 0.001);
        assert!(
            compute_magnitude_curve(vec![f])
                .iter()
                .all(|v| v.abs() < 1e-9)
        );
        let mut x = band(38, 1000.0, 0.707, 0.0);
        x.bypass = true;
        assert!(resp(&[x], 10000.0).abs() < 0.001);
    }

    #[test]
    fn wp_c12_low_and_high_pass() {
        let lp = [band(4, 1000.0, 0.707, 0.0)];
        assert!(resp(&lp, 20.0).abs() < 0.5);
        assert!(resp(&lp, 10000.0) < -20.0);
        let hp = [band(5, 1000.0, 0.707, 0.0)];
        assert!(resp(&hp, 10000.0).abs() < 0.5);
        assert!(resp(&hp, 100.0) < -20.0);
    }

    #[test]
    fn wp_c12_shelves() {
        let ls = [band(2, 200.0, 0.707, 6.0)];
        assert!((resp(&ls, 20.0) - 6.0).abs() < 1.0);
        assert!(resp(&ls, 10000.0).abs() < 0.5);
        let hs = [band(3, 5000.0, 0.707, 6.0)];
        assert!((resp(&hs, 20000.0) - 6.0).abs() < 1.0);
        assert!(resp(&hs, 100.0).abs() < 0.5);
    }

    #[test]
    fn wp_c12_first_order_types() {
        let lp1 = [band(12, 1000.0, 0.707, 0.0)];
        assert!((resp(&lp1, 1000.0) + 3.01).abs() < 0.1);
        // 6 dB/oct: one decade above the corner is about -20 dB.
        assert!((resp(&lp1, 10000.0) + 20.0).abs() < 1.5);
        let ap1 = [band(8, 1000.0, 0.707, 0.0)];
        assert!(resp(&ap1, 300.0).abs() < 0.001);
        assert!((phase_at(&ap1, 1000.0) + 90.0).abs() < 0.5);
        let ls1 = [band(9, 100.0, 0.707, 6.0)];
        assert!((resp(&ls1, 10.0) - 6.0).abs() < 0.5);
    }

    #[test]
    fn wp_c12_notch_and_allpass() {
        assert!(resp(&[band(6, 1000.0, 2.0, 0.0)], 1000.0) < -40.0);
        let ap = [band(7, 1000.0, 0.707, 0.0)];
        assert!(resp(&ap, 400.0).abs() < 0.001);
        assert!((phase_at(&ap, 1000.0).abs() - 180.0).abs() < 0.5);
    }

    #[test]
    fn wp_c12_linkwitz_transform_shifts_the_corner() {
        // f0 = 50 Hz Q0 = 1.0 -> fp = 25 Hz Qp = 0.707: a big boost around
        // 25-40 Hz, flat well above both corners.
        let mut lt = band(11, 50.0, 1.0, 25.0);
        lt.qp = 0.707;
        assert!(resp(&[lt], 20.0) > 6.0);
        assert!(resp(&[lt], 2000.0).abs() < 0.1);
        // fp <= 0 is flat.
        assert!(resp(&[band(11, 50.0, 1.0, 0.0)], 20.0).abs() < 1e-6);
    }

    #[test]
    fn wp_c12_crossovers_are_minus_3_or_6_db_at_fc() {
        // Butterworth: -3 dB at fc; Linkwitz-Riley: -6 dB at fc.
        for (t, want) in [
            (46u8, -3.01f32),
            (47, -3.01),
            (34, -6.02),
            (35, -6.02),
            (38, -6.02),
            (44, -3.01),
        ] {
            let r = resp(&[band(t, 1000.0, 0.707, 0.0)], 1000.0);
            assert!((r - want).abs() < 0.1, "type {t}: {r} dB at fc");
        }
        // LR4 LP: 24 dB/oct, so about -48 dB two octaves up; at least -40.
        assert!(resp(&[band(34, 1000.0, 0.707, 0.0)], 4000.0) < -40.0);
        // Bessel 4 LP passes DC and rolls off.
        let bes = [band(58, 1000.0, 0.707, 0.0)];
        assert!(resp(&bes, 20.0).abs() < 0.1);
        assert!(resp(&bes, 10000.0) < -30.0);
    }

    #[test]
    fn wp_c12_crossover_section_counts() {
        let n = |t: u8| xover::meta(t).map(|m| crossover_sections(m, 1000.0).len());
        assert_eq!(n(32), Some(1)); // LR2
        assert_eq!(n(38), Some(4)); // LR8
        assert_eq!(n(44), Some(2)); // BW3
        assert_eq!(n(62), Some(4)); // Bessel 8
    }

    #[test]
    fn wp_c12_curve_shapes() {
        let f = vec![band(1, 1000.0, 1.0, 6.0)];
        let m = compute_magnitude_curve(f.clone());
        assert_eq!(m.len(), 201);
        assert!((curve_freq(0) - 10.0).abs() < 1e-9);
        assert!((curve_freq(200) - 20000.0).abs() < 1e-6);
        assert!(curve_freq(201) > 20000.0);
        // The curve and the point evaluation agree.
        assert!((m[120] - f64::from(compute_response(f, curve_freq(120) as f32))).abs() < 0.01);
        let p = compute_phase_curve(vec![band(38, 1000.0, 0.707, 0.0)], true);
        assert!(
            p[200] < -500.0,
            "LR8 unwrapped phase should pass -540°, got {}",
            p[200]
        );
        let w = compute_phase_curve(vec![band(38, 1000.0, 0.707, 0.0)], false);
        assert!(w.iter().all(|v| (-180.0..=180.0).contains(v)));
    }

    // ── Out-of-range bands draw finite, never panic ─────────────────

    #[test]
    fn wp_c12_nonsense_bands_are_finite() {
        for f in [
            band(1, 30000.0, 1.0, 6.0), // above Nyquist
            band(1, 1000.0, 0.0, 6.0),  // Q 0
            band(1, f32::NAN, 1.0, 6.0),
            band(46, f32::NAN, 0.707, 0.0),
            band(46, -5.0, 0.707, 0.0),
            band(11, 50.0, 1.0, f32::INFINITY),
            band(20, 1000.0, 1.0, 6.0),  // reserved PEQ type
            band(200, 1000.0, 1.0, 6.0), // unknown type
        ] {
            let m = compute_magnitude_curve(vec![f]);
            let p = compute_phase_curve(vec![f], true);
            assert!(m.iter().chain(p.iter()).all(|v| v.is_finite()), "{f:?}");
        }
    }

    // ── Crossfeed pair map ──────────────────────────────────────────

    #[test]
    fn wp_c12_crossfeed_pair_map_valid() {
        // K340 (one DAC per ear): woofers L=2/R=0, tweeters L=3/R=1.
        let k340 = vec![0x02, 0x13, 0, 0];
        assert!(crossfeed_pair_map_valid(k340.clone(), 4));
        assert!(crossfeed_pair_map_valid(k340, 2));
        assert!(crossfeed_pair_map_valid(vec![0; 4], 4), "all default");
        assert!(
            crossfeed_pair_map_valid(vec![0x10, 0, 0, 0], 2),
            "explicit default byte"
        );
        assert!(crossfeed_pair_map_valid(vec![0x76, 0x54, 0x32, 0x10], 4));
        assert!(
            !crossfeed_pair_map_valid(vec![0x02, 0, 0, 0], 4),
            "output 2 used twice"
        );
        assert!(
            !crossfeed_pair_map_valid(vec![0x40, 0, 0, 0], 2),
            "PDM sub is not a pair output"
        );
        assert!(
            !crossfeed_pair_map_valid(vec![0x80, 0, 0, 0], 4),
            "output 8 is PDM on RP2350"
        );
        assert!(
            !crossfeed_pair_map_valid(vec![0x11, 0, 0, 0], 4),
            "left == right"
        );
        assert!(
            !crossfeed_pair_map_valid(vec![0, 0, 0x54, 0], 2),
            "byte beyond num_pairs"
        );
        assert!(
            !crossfeed_pair_map_valid(vec![0x02, 0x13, 0], 4),
            "needs 4 bytes"
        );
        assert!(
            !crossfeed_pair_map_valid(vec![0, 0, 0, 0, 0], 4),
            "needs 4 bytes"
        );
        assert!(!crossfeed_pair_map_valid(vec![0; 4], 0));
        assert!(!crossfeed_pair_map_valid(vec![0; 4], 5));
    }

    // ── Version facts ───────────────────────────────────────────────

    #[test]
    fn wp_c12_core_info_matches_protocol() {
        let i = core_info();
        assert_eq!(i.wire_format_version, 32);
        assert_eq!(i.min_wire_format_version, 16);
        assert_eq!(i.bulk_params_size, 6136);
        assert_eq!(i.magnitude_points, 201);
        assert_eq!(i.ffi_version, env!("CARGO_PKG_VERSION"));
    }

    // ── Property tests (§1.9) ───────────────────────────────────────

    fn any_band() -> impl Strategy<Value = FilterParams> {
        (
            any::<u8>(),
            any::<bool>(),
            any::<f32>(),
            any::<f32>(),
            any::<f32>(),
            any::<f32>(),
        )
            .prop_map(|(filter_type, bypass, freq, q, gain, qp)| FilterParams {
                filter_type,
                bypass,
                freq,
                q,
                gain,
                qp,
            })
    }

    proptest! {
        #[test]
        fn proptest_wp_c12_curves_never_panic_or_nan(
            bands in proptest::collection::vec(any_band(), 0..6),
            hz in any::<f32>(),
        ) {
            let _ = compute_response(bands.clone(), hz);
            let m = compute_magnitude_curve(bands.clone());
            let w = compute_phase_curve(bands.clone(), false);
            let u = compute_phase_curve(bands, true);
            prop_assert_eq!(m.len(), CURVE_POINTS);
            prop_assert_eq!(w.len(), CURVE_POINTS);
            prop_assert_eq!(u.len(), CURVE_POINTS);
            prop_assert!(m.iter().chain(&w).chain(&u).all(|v| !v.is_nan()));
            prop_assert!(w.iter().all(|v| (-180.0..=180.0).contains(v)));
        }

        #[test]
        fn proptest_wp_c12_crossfeed_pair_map_never_panics(
            map in proptest::collection::vec(any::<u8>(), 0..8),
            num_pairs in any::<u8>(),
        ) {
            let ok = crossfeed_pair_map_valid(map.clone(), num_pairs);
            if ok {
                prop_assert_eq!(map.len(), 4);
                prop_assert!((1..=4).contains(&num_pairs));
            }
        }
    }
}
