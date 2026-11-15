//! Response curves for the graph, and the quantizers the setters use.
//!
//! A channel's filters become a list of biquad sections, which are then
//! evaluated on the unit circle:
//!
//! - the PEQ shapes come from `dspi_proto::dsp::coefficients` (RBJ cookbook
//!   and the firmware's first-order forms);
//! - the Linkwitz Transform is the firmware's bilinear form with two
//!   prewarped corners (`f0`/`Q0` in `freq`/`q`, `fp`/`Qp` in `gain`/`qp`);
//! - a crossover type is expanded the way `crossover.c` builds it on the
//!   device: its analog prototype poles (Butterworth, Linkwitz-Riley as a
//!   squared Butterworth, Bessel from the firmware's pole table), scaled to
//!   the prewarped corner and mapped with the bilinear transform, in `f64`.
//!
//! Magnitudes use the `sin^2(w/2)` form of `|H|^2`, which stays accurate deep
//! in a stopband where the `cos`/`sin` form cancels; the floor is -300 dB.
//! Everything is stateless and never panics, whatever the band values.

use dspi_proto::dsp::{self, Band, Coeffs, SAMPLE_RATE};
use dspi_proto::enums::FilterType;
use dspi_proto::ext::PairMap;
use dspi_proto::xover::{self, Family};

use crate::records::{BiquadSection, CoreInfo, FilterParams};

/// Points on a magnitude or phase curve, log-spaced from 10 Hz to 20 kHz.
pub const CURVE_POINTS: usize = dsp::POINTS;

/// The filter type byte of the Linkwitz Transform.
const LINKWITZ_TRANSFORM: u8 = 11;

/// Smallest power ratio a curve shows: -300 dB.
const POWER_FLOOR: f64 = 1e-30;

/// Below this the Linkwitz Transform's Q values are not trusted.
const LT_MIN_Q: f64 = 0.1;

/// The crossover corner is kept inside what the firmware accepts.
const XOVER_MIN_HZ: f64 = 10.0;
const XOVER_MAX_FRACTION: f64 = 0.45;

/// Bessel pole pairs (sigma, omega), normalised to the -3 dB corner, per
/// order: the table in the firmware's `crossover.c`.
const BESSEL_2: [(f64, f64); 1] = [(1.10160, 0.63601)];
const BESSEL_4: [(f64, f64); 2] = [(1.37007, 0.41025), (0.99521, 1.25711)];
const BESSEL_6: [(f64, f64); 3] = [(1.57149, 0.32090), (1.38186, 0.97147), (0.93066, 1.66186)];
const BESSEL_8: [(f64, f64); 4] = [
    (1.75741, 0.27287),
    (1.63694, 0.82280),
    (1.37384, 1.38836),
    (0.89287, 1.99833),
];

// ------------------------------------------------------------- sections

/// A section that passes everything unchanged.
const UNITY: Coeffs = Coeffs::PASSTHROUGH;

/// The biquads one band adds to the cascade. A bypassed or flat band adds
/// none; a type this build does not know adds a unity section.
fn band_sections(f: &FilterParams) -> Vec<Coeffs> {
    if f.bypass || f.filter_type == 0 {
        return Vec::new();
    }
    if let Some(meta) = xover::meta(f.filter_type) {
        return crossover_sections(meta, f.freq);
    }
    if f.filter_type == LINKWITZ_TRANSFORM {
        return vec![linkwitz_transform(f)];
    }
    vec![dsp::coefficients(&Band {
        filter_type: FilterType::from_raw(f.filter_type),
        freq: f.freq,
        q: f.q,
        gain_db: f.gain,
        bypass: false,
    })]
}

/// All sections of a channel, in order. A section that came out with a
/// non-finite coefficient (a NaN or infinite band value the type does not
/// screen out) is drawn as unity rather than poisoning the whole curve.
fn cascade(filters: &[FilterParams]) -> Vec<Coeffs> {
    filters
        .iter()
        .flat_map(band_sections)
        .map(|c| if is_finite(&c) { c } else { UNITY })
        .collect()
}

fn is_finite(c: &Coeffs) -> bool {
    [c.b0, c.b1, c.b2, c.a1, c.a2].iter().all(|x| x.is_finite())
}

/// The Linkwitz Transform: zeros at the speaker's `(f0, Q0)`, poles at the
/// target `(fp, Qp)`, each prewarped with `g = tan(pi f / fs)`.
fn linkwitz_transform(f: &FilterParams) -> Coeffs {
    let f0 = f64::from(f.freq);
    let fp = f64::from(f.gain);
    // Written so that NaN fails too.
    if !(f0 > 0.0 && fp > 0.0) {
        return UNITY;
    }
    let q0 = f64::from(f.q).max(LT_MIN_Q);
    let qp = f64::from(f.qp).max(LT_MIN_Q);
    let (zero, pole) = (lt_quadratic(f0, q0), lt_quadratic(fp, qp));
    let norm = pole[0];
    Coeffs {
        b0: zero[0] / norm,
        b1: zero[1] / norm,
        b2: zero[2] / norm,
        a1: pole[1] / norm,
        a2: pole[2] / norm,
    }
}

/// `1 + g/Q + g^2`, `2(g^2 - 1)`, `1 - g/Q + g^2` for one corner.
fn lt_quadratic(freq: f64, q: f64) -> [f64; 3] {
    let g = (std::f64::consts::PI * freq / SAMPLE_RATE).tan();
    let gg = g * g;
    [1.0 + g / q + gg, 2.0 * (gg - 1.0), 1.0 - g / q + gg]
}

/// The sections of a crossover band, built like the firmware does.
fn crossover_sections(meta: xover::Meta, freq: f32) -> Vec<Coeffs> {
    let fc = f64::from(freq).clamp(XOVER_MIN_HZ, SAMPLE_RATE * XOVER_MAX_FRACTION);
    // The analog corner that lands on `fc` after the bilinear transform.
    let wa = 2.0 * SAMPLE_RATE * (std::f64::consts::PI * fc / SAMPLE_RATE).tan();
    let hp = meta.high_pass;
    let mut out = Vec::new();
    match meta.family {
        Family::Butterworth => butterworth_into(&mut out, meta.order, wa, hp),
        Family::LinkwitzRiley if meta.order == 2 => {
            // A double real pole: one section with Q = 0.5.
            out.push(pole_pair_section(1.0, 0.0, wa, hp));
        }
        Family::LinkwitzRiley => {
            // LR(2n) is BW(n) twice.
            butterworth_into(&mut out, meta.order / 2, wa, hp);
            out.extend_from_within(..);
        }
        Family::Bessel => {
            let poles: &[(f64, f64)] = match meta.order {
                2 => &BESSEL_2,
                4 => &BESSEL_4,
                6 => &BESSEL_6,
                8 => &BESSEL_8,
                _ => &[],
            };
            out.extend(poles.iter().map(|&(s, w)| pole_pair_section(s, w, wa, hp)));
        }
    }
    out
}

/// Butterworth of `order`: the real pole first for an odd order, then the
/// pole pairs from the lowest Q up.
fn butterworth_into(out: &mut Vec<Coeffs>, order: u8, wa: f64, hp: bool) {
    let n = f64::from(order);
    let odd = order % 2 == 1;
    if odd {
        out.push(real_pole_section(1.0, wa, hp));
    }
    for k in 0..order / 2 {
        let k = f64::from(k);
        let theta = if odd {
            std::f64::consts::PI * (k + 1.0) / n
        } else {
            std::f64::consts::PI * (2.0 * k + 1.0) / (2.0 * n)
        };
        out.push(pole_pair_section(theta.cos(), theta.sin(), wa, hp));
    }
}

/// One second-order section from a normalised pole pair `-sigma +- j omega`.
/// For a high pass the pole is mirrored through the unit circle first.
fn pole_pair_section(sigma: f64, omega: f64, wa: f64, hp: bool) -> Coeffs {
    let (mut sn, mut wn) = (sigma, omega);
    if hp {
        let r2 = sn * sn + wn * wn;
        if r2 > 0.0 {
            sn /= r2;
            wn /= r2;
        }
    }
    let (s, w) = (sn * wa, wn * wa);
    // Bilinear transform of B / (p^2 + A p + B) (or p^2 / ... for a high
    // pass) with p = K (1 - z^-1) / (1 + z^-1).
    let k = 2.0 * SAMPLE_RATE;
    let lin = 2.0 * s;
    let con = s * s + w * w;
    let inv = 1.0 / (k * k + lin * k + con);
    let a1 = 2.0 * (con - k * k) * inv;
    let a2 = (k * k - lin * k + con) * inv;
    let g = if hp { k * k * inv } else { con * inv };
    let b1 = if hp { -2.0 * g } else { 2.0 * g };
    Coeffs {
        b0: g,
        b1,
        b2: g,
        a1,
        a2,
    }
}

/// One first-order section from the real pole `-sigma`.
fn real_pole_section(sigma: f64, wa: f64, hp: bool) -> Coeffs {
    let sn = if hp && sigma > 0.0 {
        1.0 / sigma
    } else {
        sigma
    };
    let s = sn * wa;
    let k = 2.0 * SAMPLE_RATE;
    let inv = 1.0 / (k + s);
    let (b0, b1) = if hp {
        (k * inv, -k * inv)
    } else {
        (s * inv, s * inv)
    };
    Coeffs {
        b0,
        b1,
        b2: 0.0,
        a1: (s - k) * inv,
        a2: 0.0,
    }
}

// ------------------------------------------------------------ evaluation

/// Angular frequency in radians per sample.
fn omega(freq: f64) -> f64 {
    2.0 * std::f64::consts::PI * freq / SAMPLE_RATE
}

/// `|H|^2` of one section at `w`, written in `phi = sin^2(w/2)`.
fn power_gain(c: &Coeffs, w: f64) -> f64 {
    let half = (w / 2.0).sin();
    let phi = half * half;
    let num_dc = c.b0 + c.b1 + c.b2;
    let den_dc = 1.0 + c.a1 + c.a2;
    let num = num_dc * num_dc - 4.0 * (c.b0 * c.b1 + c.b1 * c.b2 + 4.0 * c.b0 * c.b2) * phi
        + 16.0 * c.b0 * c.b2 * phi * phi;
    let den =
        den_dc * den_dc - 4.0 * (c.a1 + c.a1 * c.a2 + 4.0 * c.a2) * phi + 16.0 * c.a2 * phi * phi;
    if den <= 0.0 { 1.0 } else { num.max(0.0) / den }
}

/// Phase of one section at `w`, in radians.
fn phase_rad(c: &Coeffs, w: f64) -> f64 {
    let (c1, s1) = (w.cos(), w.sin());
    let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
    let num_re = c.b0 + c.b1 * c1 + c.b2 * c2;
    let num_im = -(c.b1 * s1 + c.b2 * s2);
    let den_re = 1.0 + c.a1 * c1 + c.a2 * c2;
    let den_im = -(c.a1 * s1 + c.a2 * s2);
    num_im.atan2(num_re) - den_im.atan2(den_re)
}

/// The cascade's level at `freq`, in dB.
fn level_db(sections: &[Coeffs], freq: f64) -> f64 {
    let w = omega(freq);
    let power = sections.iter().fold(1.0, |p, c| p * power_gain(c, w));
    10.0 * power.max(POWER_FLOOR).log10()
}

/// The cascade's phase at `freq`, in degrees within +-180 (0 where it is
/// not a number).
fn phase_deg(sections: &[Coeffs], freq: f64) -> f64 {
    let w = omega(freq);
    // -0.0 is the neutral element: an empty cascade reads -0.0.
    let total = sections.iter().fold(-0.0, |acc, c| acc + phase_rad(c, w));
    let deg = total.to_degrees() % 360.0;
    if deg > 180.0 {
        deg - 360.0
    } else if deg < -180.0 {
        deg + 360.0
    } else if deg.is_nan() {
        0.0
    } else {
        deg
    }
}

/// Undo the +-180 wraps: each step of more than half a turn is taken as a
/// wrap and compensated from there on. The first point is kept as it is.
fn unwrapped(wrapped: Vec<f64>) -> Vec<f64> {
    let mut out = Vec::with_capacity(wrapped.len());
    let mut turns = 0.0;
    let mut prev: Option<f64> = None;
    for deg in wrapped {
        match prev {
            None => out.push(deg),
            Some(p) => {
                let step = deg - p;
                if step > 180.0 {
                    turns -= 360.0;
                } else if step < -180.0 {
                    turns += 360.0;
                }
                out.push(deg + turns);
            }
        }
        prev = Some(deg);
    }
    out
}

// ---------------------------------------------------------------- exports

/// The combined level of `filters` at `freq` Hz, in dB.
///
/// Bypassed and flat bands do not count; a crossover type counts with all
/// its sections; a type this build does not know counts as flat.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn compute_response(filters: Vec<FilterParams>, freq: f32) -> f32 {
    level_db(&cascade(&filters), f64::from(freq)) as f32
}

/// The combined level of `filters` in dB at each of the [`CURVE_POINTS`]
/// frequencies of [`curve_freq`]. Never below -300 dB.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn compute_magnitude_curve(filters: Vec<FilterParams>) -> Vec<f64> {
    let sections = cascade(&filters);
    (0..CURVE_POINTS as u32)
        .map(|i| level_db(&sections, curve_freq(i)))
        .collect()
}

/// The combined phase of `filters` in degrees at each of the
/// [`CURVE_POINTS`] frequencies of [`curve_freq`]: within +-180, or, with
/// `unwrap`, continuous from the first point on.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn compute_phase_curve(filters: Vec<FilterParams>, unwrap: bool) -> Vec<f64> {
    let sections = cascade(&filters);
    let wrapped: Vec<f64> = (0..CURVE_POINTS as u32)
        .map(|i| phase_deg(&sections, curve_freq(i)))
        .collect();
    if unwrap { unwrapped(wrapped) } else { wrapped }
}

/// The frequency of curve point `i` in Hz: 10 Hz at 0, 20 kHz at
/// `CURVE_POINTS - 1`, evenly spaced in log frequency. Larger `i` continue
/// the same spacing past 20 kHz.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn curve_freq(i: u32) -> f64 {
    let lo = dsp::MIN_HZ.log10();
    let hi = dsp::MAX_HZ.log10();
    let steps = (CURVE_POINTS - 1) as f64;
    10f64.powf(lo + f64::from(i) / steps * (hi - lo))
}

/// Whether the firmware accepts `map` as the crossfeed pair map on a device
/// with `num_pairs` stereo pairs: exactly [`crate::consts::CROSSFEED_MAP_PAIRS`]
/// bytes, and the rule of `dspi_proto::ext::PairMap::is_valid`.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn crossfeed_pair_map_valid(map: Vec<u8>, num_pairs: u8) -> bool {
    <[u8; PairMap::SIZE]>::try_from(map.as_slice())
        .is_ok_and(|bytes| PairMap(bytes).is_valid(usize::from(num_pairs)))
}

/// The version facts of this build: crate version, the wire format versions
/// it writes and reads, the bulk image size and the curve size.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn core_info() -> CoreInfo {
    CoreInfo {
        ffi_version: env!("CARGO_PKG_VERSION").to_owned(),
        wire_format_version: dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8,
        min_wire_format_version: dspi_proto::wire_compat::MIN_WIRE_FORMAT_VERSION,
        bulk_params_size: dspi_proto::generated::BULK_SIZE as u32,
        magnitude_points: CURVE_POINTS as u32,
    }
}

/// `value` on a grid of `1 / scale`, halves rounded away from zero, and a
/// zero result always positive.
fn snap(value: f32, scale: f32) -> f32 {
    let v = (value * scale).round() / scale;
    if v == 0.0 { 0.0 } else { v }
}

/// A gain in dB on the device's 0.1 dB grid, as the gain setters send it.
///
/// Halves round away from zero and -0 becomes +0. No clamping: NaN and the
/// infinities come back unchanged, and so does a value too large to scale.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn quantize_gain(db: f32) -> f32 {
    snap(db, 10.0)
}

/// A delay in ms on a 0.0001 ms grid, as the delay setters send it.
///
/// Halves round away from zero and -0 becomes +0. No clamping (callers bound
/// the delay to its range afterwards); NaN and the infinities pass through.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn quantize_delay(ms: f32) -> f32 {
    snap(ms, 10_000.0)
}

// -- WP CG-SWS-a --
/// The master volume's mute value: at or below it the volume mutes.
const MASTER_VOLUME_MUTE_DB: f32 = -128.0;
/// The quietest master volume that is not mute.
const MASTER_VOLUME_MIN_DB: f32 = -127.0;

/// A master volume in dB as [`crate::DspiCore::set_master_volume`] keeps
/// it: -128 (mute) at or below -128, else on a 0.5 dB grid (halves away
/// from zero) within -127..=0. -0 becomes +0. NaN comes back as NaN (the
/// setter refuses it).
///
/// A slider shows this value while the user drags, so it shows what the
/// device gets.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn quantize_master_volume(db: f32) -> f32 {
    if db <= MASTER_VOLUME_MUTE_DB {
        return MASTER_VOLUME_MUTE_DB;
    }
    let v = snap(db, 2.0).clamp(MASTER_VOLUME_MIN_DB, 0.0);
    if v == 0.0 { 0.0 } else { v }
}

// -- WP CG-SWS-a --
/// The biquad sections the curves of `filters` are computed from, in
/// cascade order: what a GPU renderer evaluates per pixel to draw the same
/// curve as [`compute_magnitude_curve`].
///
/// A bypassed or flat band adds none; a crossover type adds each of its
/// sections as the firmware builds them; a type this build does not know,
/// or a band whose values give a non-finite coefficient, adds a unity
/// section. Call it with one band for that band's own sections.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn curve_sections(filters: Vec<FilterParams>) -> Vec<BiquadSection> {
    cascade(&filters)
        .into_iter()
        .map(|c| BiquadSection {
            b0: c.b0,
            b1: c.b1,
            b2: c.b2,
            a1: c.a1,
            a2: c.a2,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_clean1 {
    use super::*;

    /// One case of the reference table: bands as `(type, freq, q, gain, qp,
    /// bypass)`, the response at [`RESP_HZ`], and `(point, magnitude dB,
    /// phase, unwrapped phase)` at every 8th curve point.
    struct Case {
        name: &'static str,
        bands: &'static [(u8, f32, f32, f32, f32, bool)],
        resp: [f32; 6],
        pts: &'static [(usize, f64, f32, f32)],
    }

    /// Where the reference table's `resp` values were taken.
    const RESP_HZ: [f32; 6] = [20.0, 300.0, 1000.0, 4000.0, 10_000.0, 20_000.0];

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

    fn at(filters: &[FilterParams], hz: f32) -> f64 {
        f64::from(compute_response(filters.to_vec(), hz))
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// `a` and `b` agree to 1e-9 of their size (of 1 near zero).
    fn same(a: f64, b: f64) -> bool {
        a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    // ------------------------------------------------- recorded outputs

    /// Bands written with `{:?}`, as in the fixture.
    fn parse_filters(text: &str) -> Vec<FilterParams> {
        let field = |part: &str, name: &str| -> String {
            part.trim()
                .trim_end_matches(&[']', ',', ' ', '}'][..])
                .split(", ")
                .find_map(|kv| kv.strip_prefix(name)?.strip_prefix(": "))
                .unwrap()
                .to_owned()
        };
        text.split("FilterParams {")
            .skip(1)
            .map(|p| FilterParams {
                filter_type: field(p, "filter_type").parse().unwrap(),
                bypass: field(p, "bypass").parse().unwrap(),
                freq: field(p, "freq").parse().unwrap(),
                q: field(p, "q").parse().unwrap(),
                gain: field(p, "gain").parse().unwrap(),
                qp: field(p, "qp").parse().unwrap(),
            })
            .collect()
    }

    /// The number array under `key`, read from the line's text: serde_json's
    /// default float parsing may be one unit in the last place off, Rust's
    /// `str::parse` is exact.
    fn numbers(line: &str, key: &str) -> Vec<f64> {
        let start = line.find(&format!("\"{key}\":[")).unwrap() + key.len() + 4;
        let body = &line[start..];
        let body = &body[..body.find(']').unwrap()];
        body.split(',')
            .filter(|t| !t.is_empty())
            .map(|t| t.trim().parse().unwrap())
            .collect()
    }

    fn pairs(v: &serde_json::Value) -> Vec<(String, String)> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p[0].as_str().unwrap().to_owned(),
                    p[1].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn fixture() -> Vec<(String, serde_json::Value)> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/curves/responses.jsonl"
        );
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| (l.to_owned(), serde_json::from_str(l).unwrap()))
            .collect()
    }

    #[test]
    fn clean1_curves_match_the_recorded_outputs() {
        let mut cases = 0;
        for (line, rec) in fixture() {
            let Some(text) = rec["filters"].as_str() else {
                continue;
            };
            let filters = parse_filters(text);
            cases += 1;
            let checks = [
                ("mag", compute_magnitude_curve(filters.clone())),
                ("phase", compute_phase_curve(filters.clone(), false)),
                (
                    "phase_unwrapped",
                    compute_phase_curve(filters.clone(), true),
                ),
            ];
            for (key, got) in checks {
                let want = numbers(&line, key);
                assert_eq!(got.len(), want.len(), "{key} {text}");
                for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                    assert!(same(*g, *w), "{key}[{i}] {g} vs {w} for {text}");
                }
            }
            let want = numbers(&line, "resp");
            for (hz, w) in [10.0f32, 100.0, 1000.0, 10_000.0, 20_000.0]
                .iter()
                .zip(want)
            {
                let g = compute_response(filters.clone(), *hz);
                assert!(same(f64::from(g), w), "resp at {hz}: {g} vs {w} for {text}");
            }
        }
        assert!(cases > 200, "only {cases} recorded filter sets");
    }

    #[test]
    fn clean1_scalars_match_the_recorded_outputs() {
        let mut seen = 0;
        for (line, rec) in fixture() {
            if rec.get("curve_freq").is_some() {
                assert_eq!(rec["curve_points"].as_u64(), Some(CURVE_POINTS as u64));
                for (i, want) in numbers(&line, "curve_freq").into_iter().enumerate() {
                    assert_eq!(curve_freq(i as u32), want, "curve_freq({i})");
                }
                seen += 1;
            }
            if let Some(list) = rec.get("quantize_gain") {
                for (input, want) in pairs(list) {
                    let got = quantize_gain(input.parse().unwrap());
                    assert_eq!(format!("{got:?}"), want, "quantize_gain({input})");
                }
                seen += 1;
            }
            if let Some(list) = rec.get("quantize_delay") {
                for (input, want) in pairs(list) {
                    let got = quantize_delay(input.parse().unwrap());
                    assert_eq!(format!("{got:?}"), want, "quantize_delay({input})");
                }
                seen += 1;
            }
            if let Some(list) = rec.get("pair_map_valid") {
                for case in list.as_array().unwrap() {
                    let map: Vec<u8> = case[0]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|b| b.as_u64().unwrap() as u8)
                        .collect();
                    let pairs = case[1].as_u64().unwrap() as u8;
                    let want = case[2].as_bool().unwrap();
                    assert_eq!(
                        crossfeed_pair_map_valid(map.clone(), pairs),
                        want,
                        "{map:?} {pairs}"
                    );
                }
                seen += 1;
            }
            if let Some(info) = rec.get("core_info") {
                assert_eq!(format!("{:?}", core_info()), info.as_str().unwrap());
                seen += 1;
            }
        }
        assert_eq!(seen, 5);
    }

    /// The reference cases in `curves_ref.rs`: response values were stored
    /// as `f32` and phases rounded through `f32`, so they are compared at
    /// that precision.
    #[test]
    fn clean1_curves_match_the_reference_table() {
        for case in REFERENCE {
            let filters: Vec<FilterParams> = case
                .bands
                .iter()
                .map(|&(t, freq, q, gain, qp, bypass)| FilterParams {
                    filter_type: t,
                    bypass,
                    freq,
                    q,
                    gain,
                    qp,
                })
                .collect();
            for (hz, want) in RESP_HZ.iter().zip(case.resp) {
                let got = compute_response(filters.clone(), *hz);
                assert!(
                    close(
                        f64::from(got),
                        f64::from(want),
                        1e-4 * want.abs().max(1.0) as f64
                    ),
                    "{} at {hz}: {got} vs {want}",
                    case.name
                );
            }
            let mag = compute_magnitude_curve(filters.clone());
            let wrapped = compute_phase_curve(filters.clone(), false);
            let unwrapped = compute_phase_curve(filters, true);
            for &(i, m, p, u) in case.pts {
                assert!(same(mag[i], m), "{} mag[{i}]: {} vs {m}", case.name, mag[i]);
                let tol = |x: f32| 1e-5 * f64::from(x.abs()).max(1.0);
                assert!(
                    close(wrapped[i], f64::from(p), tol(p)),
                    "{} phase[{i}]",
                    case.name
                );
                assert!(
                    close(unwrapped[i], f64::from(u), tol(u)),
                    "{} unwrapped[{i}]",
                    case.name
                );
            }
        }
    }

    // ---------------------------------------------------------- behaviour

    #[test]
    fn clean1_no_bands_or_flat_bands_are_zero_db() {
        assert!(compute_magnitude_curve(vec![]).iter().all(|d| *d == 0.0));
        let flat = vec![band(0, 1000.0, 0.707, 12.0)];
        assert!(
            compute_magnitude_curve(flat.clone())
                .iter()
                .all(|d| *d == 0.0)
        );
        assert!(compute_phase_curve(flat, true).iter().all(|d| *d == 0.0));
    }

    #[test]
    fn clean1_peaking_reaches_its_gain_at_the_centre_only() {
        for gain in [-12.0, -3.5, 4.0, 15.0] {
            let b = [band(1, 1000.0, 2.0, gain)];
            assert!(close(at(&b, 1000.0), f64::from(gain), 1e-3), "{gain}");
            assert!(at(&b, 20.0).abs() < 0.05);
            assert!(at(&b, 15_000.0).abs() < 0.2);
            assert!(at(&b, 700.0).abs() < f64::from(gain).abs());
        }
    }

    #[test]
    fn clean1_a_bypassed_band_does_not_count() {
        let mut b = band(1, 1000.0, 1.0, 9.0);
        b.bypass = true;
        assert_eq!(compute_response(vec![b], 1000.0), 0.0);
        let mut x = band(36, 500.0, 0.707, 0.0);
        x.bypass = true;
        assert!(compute_magnitude_curve(vec![x]).iter().all(|d| *d == 0.0));
    }

    #[test]
    fn clean1_second_order_passes() {
        let lp = [band(4, 1000.0, 0.707, 0.0)];
        let hp = [band(5, 1000.0, 0.707, 0.0)];
        assert!(close(at(&lp, 1000.0), -3.0, 0.05));
        assert!(close(at(&hp, 1000.0), -3.0, 0.05));
        assert!(at(&lp, 50.0).abs() < 0.01 && at(&hp, 15_000.0).abs() < 0.1);
        // 12 dB per octave in the stopband.
        let slope = at(&lp, 4000.0) - at(&lp, 8000.0);
        assert!(close(slope, 12.0, 2.0), "{slope}");
        let slope = at(&hp, 200.0) - at(&hp, 100.0);
        assert!(close(slope, 12.0, 0.5), "{slope}");
    }

    #[test]
    fn clean1_shelves_settle_on_the_right_side() {
        let low = [band(2, 200.0, 0.707, 6.0)];
        assert!(close(at(&low, 10.0), 6.0, 0.05));
        assert!(at(&low, 10_000.0).abs() < 0.05);
        let high = [band(3, 5000.0, 0.707, -6.0)];
        assert!(at(&high, 50.0).abs() < 0.05);
        assert!(at(&high, 20_000.0) < -5.0);
    }

    #[test]
    fn clean1_first_order_types() {
        let lp1 = [band(12, 1000.0, 0.707, 0.0)];
        let hp1 = [band(13, 1000.0, 0.707, 0.0)];
        assert!(close(at(&lp1, 1000.0), -3.01, 0.05));
        assert!(close(at(&hp1, 1000.0), -3.01, 0.05));
        // 6 dB per octave.
        let slope = at(&hp1, 100.0) - at(&hp1, 50.0);
        assert!(close(slope, 6.0, 0.2), "{slope}");
        // An all-pass is flat.
        let ap1 = vec![band(8, 1000.0, 0.707, 0.0)];
        assert!(
            compute_magnitude_curve(ap1.clone())
                .iter()
                .all(|d| d.abs() < 1e-9)
        );
        let ls1 = [band(9, 100.0, 0.707, 6.0)];
        assert!(close(at(&ls1, 10.0), 6.0, 0.3) && at(&ls1, 10_000.0).abs() < 0.05);
        let hs1 = [band(10, 3000.0, 0.707, -4.0)];
        assert!(close(at(&hs1, 20_000.0), -4.0, 0.6) && at(&hs1, 20.0).abs() < 0.05);
    }

    #[test]
    fn clean1_notch_and_all_pass() {
        let notch = [band(6, 1000.0, 2.0, 0.0)];
        assert!(at(&notch, 1000.0) < -60.0);
        assert!(at(&notch, 100.0).abs() < 0.05);
        let ap = vec![band(7, 1000.0, 0.707, 0.0)];
        assert!(
            compute_magnitude_curve(ap.clone())
                .iter()
                .all(|d| d.abs() < 1e-9)
        );
        let phase = compute_phase_curve(ap.clone(), true);
        assert!(phase[0].abs() < 5.0);
        assert!(
            close(phase[CURVE_POINTS - 1], -360.0, 30.0),
            "{}",
            phase[CURVE_POINTS - 1]
        );
        // The wrapped curve stays on its axis.
        assert!(
            compute_phase_curve(ap, false)
                .iter()
                .all(|d| d.abs() <= 180.0)
        );
    }

    #[test]
    fn clean1_linkwitz_transform_moves_the_corner() {
        // f0 50 Hz Q 1 to fp 25 Hz Qp 0.707: lifts the bass, leaves the top.
        let lt = [FilterParams {
            filter_type: LINKWITZ_TRANSFORM,
            bypass: false,
            freq: 50.0,
            q: 1.0,
            gain: 25.0,
            qp: 0.707,
        }];
        assert!(at(&lt, 20.0) > 5.0);
        assert!(at(&lt, 5000.0).abs() < 0.01);
        // No target corner: no effect.
        let mut off = lt[0];
        off.gain = 0.0;
        assert_eq!(compute_response(vec![off], 20.0), 0.0);
        off.gain = f32::NAN;
        assert_eq!(compute_response(vec![off], 20.0), 0.0);
    }

    #[test]
    fn clean1_crossovers_at_their_corner() {
        // Butterworth: -3 dB at fc; Linkwitz-Riley: -6 dB.
        for t in [40u8, 41, 44, 45, 46, 47, 54, 55] {
            let b = [band(t, 1000.0, 0.707, 0.0)];
            assert!(close(at(&b, 1000.0), -3.01, 0.05), "type {t}");
        }
        for t in [32u8, 33, 34, 35, 36, 37, 38, 39] {
            let b = [band(t, 1000.0, 0.707, 0.0)];
            assert!(close(at(&b, 1000.0), -6.02, 0.05), "type {t}");
        }
        // A Linkwitz-Riley pair sums flat in phase.
        let lp = band_sections(&band(34, 500.0, 0.707, 0.0));
        let hp = band_sections(&band(35, 500.0, 0.707, 0.0));
        for f in [100.0, 500.0, 2000.0] {
            let w = omega(f);
            let p = |s: &[Coeffs]| s.iter().map(|c| phase_rad(c, w)).sum::<f64>();
            let a = 10f64.powf(level_db(&lp, f) / 20.0);
            let b = 10f64.powf(level_db(&hp, f) / 20.0);
            let (re, im) = (
                a * p(&lp).cos() + b * p(&hp).cos(),
                a * p(&lp).sin() + b * p(&hp).sin(),
            );
            assert!(close(re.hypot(im), 1.0, 1e-6), "{f}");
        }
    }

    #[test]
    fn clean1_crossover_section_counts() {
        let count = |t: u8| band_sections(&band(t, 1000.0, 0.707, 0.0)).len();
        // LR2..LR8
        assert_eq!([32, 34, 36, 38].map(count), [1, 2, 4, 4]);
        // BW1..BW8
        assert_eq!(
            [40, 42, 44, 46, 48, 50, 52, 54].map(count),
            [1, 1, 2, 2, 3, 3, 4, 4]
        );
        // Bessel 2..8
        assert_eq!([56, 58, 60, 62].map(count), [1, 2, 3, 4]);
        // High passes match their low passes.
        for t in (32u8..64).step_by(2) {
            assert_eq!(count(t), count(t + 1), "{t}");
        }
        // Not crossovers.
        assert_eq!(count(64), 1);
        assert_eq!(count(14), 1);
    }

    #[test]
    fn clean1_curve_shapes() {
        assert_eq!(CURVE_POINTS, 201);
        assert_eq!(curve_freq(0), 10.0);
        assert!(close(curve_freq(200), 20_000.0, 1e-9));
        let ratio = curve_freq(1) / curve_freq(0);
        for i in 1..CURVE_POINTS as u32 {
            assert!(close(curve_freq(i) / curve_freq(i - 1), ratio, 1e-12));
        }
        let some = vec![band(1, 300.0, 1.0, 3.0), band(47, 80.0, 0.707, 0.0)];
        assert_eq!(compute_magnitude_curve(some.clone()).len(), CURVE_POINTS);
        assert_eq!(compute_phase_curve(some.clone(), false).len(), CURVE_POINTS);
        assert_eq!(compute_phase_curve(some, true).len(), CURVE_POINTS);
    }

    #[test]
    fn clean1_nonsense_bands_stay_finite() {
        let values = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -1.0,
            0.0,
            1e-30,
            1e30,
        ];
        for t in [
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 32, 47, 63, 64, 255,
        ] {
            for v in values {
                let mut b = band(t, v, v, v);
                b.qp = v;
                check_finite(vec![b]);
                check_finite(vec![band(t, 1000.0, v, 6.0)]);
                check_finite(vec![band(t, v, 0.707, 6.0)]);
            }
        }
    }

    fn check_finite(filters: Vec<FilterParams>) {
        for d in compute_magnitude_curve(filters.clone()) {
            assert!(!d.is_nan() && d >= -300.0, "{filters:?}");
        }
        for d in compute_phase_curve(filters.clone(), true) {
            assert!(!d.is_nan(), "{filters:?}");
        }
        assert!(!compute_response(filters, 1000.0).is_nan());
    }

    #[test]
    fn clean1_crossfeed_pair_map_valid() {
        assert!(crossfeed_pair_map_valid(vec![0; 4], 4));
        assert!(crossfeed_pair_map_valid(vec![0; 4], 2));
        assert!(crossfeed_pair_map_valid(PairMap::K340.0.to_vec(), 2));
        assert!(crossfeed_pair_map_valid(vec![0x76, 0x54, 0x32, 0x10], 4));
        assert!(!crossfeed_pair_map_valid(vec![0x02, 0, 0, 0], 4));
        assert!(!crossfeed_pair_map_valid(vec![0, 0, 0x54, 0], 2));
        assert!(!crossfeed_pair_map_valid(vec![0; 4], 0));
        assert!(!crossfeed_pair_map_valid(vec![0; 4], 5));
        assert!(!crossfeed_pair_map_valid(vec![0; 3], 2));
        assert!(!crossfeed_pair_map_valid(vec![0; 5], 2));
        assert!(!crossfeed_pair_map_valid(vec![], 1));
    }

    #[test]
    fn clean1_quantize_gain() {
        assert_eq!(quantize_gain(3.12), 3.1);
        assert_eq!(quantize_gain(3.15), 3.2);
        assert_eq!(quantize_gain(-3.15), -3.2);
        assert_eq!(quantize_gain(-0.04).to_bits(), 0.0f32.to_bits());
        assert_eq!(quantize_gain(-0.0).to_bits(), 0.0f32.to_bits());
        assert!(quantize_gain(f32::NAN).is_nan());
        assert_eq!(quantize_gain(f32::INFINITY), f32::INFINITY);
        assert_eq!(quantize_gain(f32::MAX), f32::INFINITY);
        assert_eq!(quantize_gain(99.0), 99.0);
    }

    #[test]
    fn clean1_quantize_delay() {
        assert_eq!(quantize_delay(0.25), 0.25);
        assert_eq!(quantize_delay(1.23456), 1.2346);
        assert_eq!(quantize_delay(-0.00002).to_bits(), 0.0f32.to_bits());
        assert_eq!(quantize_delay(-1.31), -1.31);
        assert!(quantize_delay(f32::NAN).is_nan());
        assert_eq!(quantize_delay(f32::NEG_INFINITY), f32::NEG_INFINITY);
    }

    #[test]
    fn clean1_core_info_is_the_protocols() {
        let info = core_info();
        assert_eq!(info.ffi_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            u16::from(info.wire_format_version),
            dspi_proto::generated::wire::WIRE_FORMAT_VERSION
        );
        assert_eq!(
            info.min_wire_format_version,
            dspi_proto::wire_compat::MIN_WIRE_FORMAT_VERSION
        );
        assert_eq!(
            info.bulk_params_size as usize,
            dspi_proto::generated::BULK_SIZE
        );
        assert_eq!(info.magnitude_points as usize, CURVE_POINTS);
    }

    mod props {
        use proptest::prelude::*;

        use super::*;

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
            fn proptest_clean1_curves_never_panic_or_nan(
                filters in prop::collection::vec(any_band(), 0..6),
            ) {
                check_finite(filters);
            }

            #[test]
            fn proptest_clean1_quantizers_round_half_away(x in -1.0e4f32..1.0e4) {
                let g = quantize_gain(x);
                prop_assert_eq!(g, {
                    let v = (x * 10.0).round() / 10.0;
                    if v == 0.0 { 0.0 } else { v }
                });
                prop_assert!(g.is_sign_positive() || g != 0.0);
                prop_assert!((f64::from(g) - f64::from(x)).abs() <= 0.05 + 1e-3);
                let d = quantize_delay(x / 1000.0);
                prop_assert!(d.is_sign_positive() || d != 0.0);
                prop_assert_eq!(quantize_delay(d), d);
            }

            #[test]
            fn proptest_clean1_crossfeed_pair_map_never_panics(
                map in prop::collection::vec(any::<u8>(), 0..9),
                pairs in any::<u8>(),
            ) {
                let ok = crossfeed_pair_map_valid(map.clone(), pairs);
                prop_assert!(!ok || (map.len() == 4 && (1..=4).contains(&pairs)));
            }
        }
    }
}

// -- WP CG-SWS-a --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "curves/tests_wp_cgswsa.rs"]
mod tests_wp_cgswsa;
