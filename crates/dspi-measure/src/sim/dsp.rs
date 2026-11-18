//! The simulator's signal processing (WP P5SIM): biquad designs at any rate,
//! a biquad runner, and one windowed-sinc kernel used for fractional delays,
//! reflections and the microphone clock.
//!
//! Everything here is `pub(crate)`: the public surface is `sim`'s.

use serde::{Deserialize, Serialize};

use crate::types::Coeffs;

/// One analogue-style filter of the simulated world (a driver, a room mode, a
/// coupler, a microphone), designed by the RBJ cookbook at the rate it runs at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SimFilter {
    /// A peak or dip: centre Hz, Q, gain dB.
    Peak {
        /// Centre, Hz.
        freq: f64,
        /// Quality factor.
        q: f64,
        /// Gain, dB.
        gain_db: f64,
    },
    /// A second-order low shelf.
    LowShelf {
        /// Corner, Hz.
        freq: f64,
        /// Quality factor.
        q: f64,
        /// Gain, dB.
        gain_db: f64,
    },
    /// A second-order high shelf.
    HighShelf {
        /// Corner, Hz.
        freq: f64,
        /// Quality factor.
        q: f64,
        /// Gain, dB.
        gain_db: f64,
    },
    /// A second-order high-pass.
    HighPass {
        /// Corner, Hz.
        freq: f64,
        /// Quality factor.
        q: f64,
    },
    /// A second-order low-pass.
    LowPass {
        /// Corner, Hz.
        freq: f64,
        /// Quality factor.
        q: f64,
    },
}

impl SimFilter {
    /// Whether every parameter is finite (a world with a non-finite filter is
    /// refused before anything is rendered).
    pub(crate) fn is_finite(&self) -> bool {
        match *self {
            SimFilter::Peak { freq, q, gain_db }
            | SimFilter::LowShelf { freq, q, gain_db }
            | SimFilter::HighShelf { freq, q, gain_db } => {
                freq.is_finite() && q.is_finite() && gain_db.is_finite()
            }
            SimFilter::HighPass { freq, q } | SimFilter::LowPass { freq, q } => {
                freq.is_finite() && q.is_finite()
            }
        }
    }

    /// The biquad at `fs`. A corner at or below 0 Hz or at or above 0.49 `fs`
    /// does nothing (a pass-through): such a filter is outside the band the
    /// rate can represent. Q is held to 0.05..100 and gain to ±60 dB, so the
    /// result is always a stable, finite section.
    pub(crate) fn design(&self, fs: f64) -> Coeffs {
        let (freq, q, gain_db) = match *self {
            SimFilter::Peak { freq, q, gain_db }
            | SimFilter::LowShelf { freq, q, gain_db }
            | SimFilter::HighShelf { freq, q, gain_db } => (freq, q, gain_db),
            SimFilter::HighPass { freq, q } | SimFilter::LowPass { freq, q } => (freq, q, 0.0),
        };
        if !(fs.is_finite() && fs > 0.0 && freq.is_finite() && q.is_finite() && gain_db.is_finite())
        {
            return Coeffs::PASSTHROUGH;
        }
        if freq <= 0.0 || freq >= 0.49 * fs {
            return Coeffs::PASSTHROUGH;
        }
        let q = q.clamp(0.05, 100.0);
        let gain_db = gain_db.clamp(-60.0, 60.0);
        let w = 2.0 * std::f64::consts::PI * freq / fs;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match self {
            SimFilter::Peak { .. } => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            SimFilter::LowShelf { .. } => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cs + sq),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - sq),
                    (a + 1.0) + (a - 1.0) * cs + sq,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - sq,
                )
            }
            SimFilter::HighShelf { .. } => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - sq),
                    (a + 1.0) - (a - 1.0) * cs + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - sq,
                )
            }
            SimFilter::HighPass { .. } => (
                (1.0 + cs) / 2.0,
                -(1.0 + cs),
                (1.0 + cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            SimFilter::LowPass { .. } => (
                (1.0 - cs) / 2.0,
                1.0 - cs,
                (1.0 - cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
        };
        let c = Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        };
        if coeffs_finite(&c) {
            c
        } else {
            Coeffs::PASSTHROUGH
        }
    }

    /// The filter's magnitude at `freq` when run at `fs`, dB.
    pub(crate) fn magnitude_db(&self, freq: f64, fs: f64) -> f64 {
        biquad_magnitude_db(&self.design(fs), freq, fs)
    }
}

/// Whether a section's coefficients are all finite.
pub(crate) fn coeffs_finite(c: &Coeffs) -> bool {
    c.b0.is_finite() && c.b1.is_finite() && c.b2.is_finite() && c.a1.is_finite() && c.a2.is_finite()
}

/// A section's magnitude at `freq` for rate `fs`, dB.
pub(crate) fn biquad_magnitude_db(c: &Coeffs, freq: f64, fs: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / fs;
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let nr = c.b0 + c.b1 * c1 + c.b2 * c2;
    let ni = -(c.b1 * s1 + c.b2 * s2);
    let dr = 1.0 + c.a1 * c1 + c.a2 * c2;
    let di = -(c.a1 * s1 + c.a2 * s2);
    let num = nr * nr + ni * ni;
    let den = (dr * dr + di * di).max(1e-300);
    let db = 10.0 * (num / den).max(1e-30).log10();
    // A non-finite frequency, rate or section answers 0 dB, never NaN.
    if db.is_finite() { db } else { 0.0 }
}

/// Below this a filter's output and state are silence (-600 dB).
const FLUSH: f64 = 1e-30;

/// Run one biquad over `buf` in place (direct form II transposed, `f64` state).
pub(crate) fn run_biquad(c: &Coeffs, buf: &mut [f32]) {
    let (mut z1, mut z2) = (0.0f64, 0.0f64);
    for v in buf.iter_mut() {
        let x = f64::from(*v);
        let y = c.b0 * x + z1;
        z1 = c.b1 * x - c.a1 * y + z2;
        z2 = c.b2 * x - c.a2 * y;
        // A decayed tail is silence (no subnormals, and the delay stage
        // skips the zeros).
        if y.abs() < FLUSH && z1.abs() < FLUSH && z2.abs() < FLUSH {
            z1 = 0.0;
            z2 = 0.0;
            *v = 0.0;
        } else {
            *v = y as f32;
        }
    }
}

/// Fractional phases per sample in the kernel's table.
const PHASES: usize = 1024;
/// Half the kernel's length in samples at full bandwidth (64 taps in all).
const HALF_TAPS: usize = 32;
/// The Kaiser window's beta: about -90 dB side lobes.
const KAISER_BETA: f64 = 10.0;

/// A windowed-sinc interpolation kernel, tabulated at [`PHASES`] phases per
/// sample and read with linear interpolation between phases (error about
/// -110 dB). `cut` (0..1] scales the pass band for an anti-alias low-pass when
/// the reading side's rate is lower than the buffer's.
pub(crate) struct Kernel {
    /// Half-width, samples of the buffer read.
    half: usize,
    /// `h(x)` for `x = -half + i / PHASES`, `i = 0 ..= 2 * half * PHASES`, plus one guard zero.
    table: Vec<f64>,
    /// Full band: an integer position reads the sample exactly.
    full_band: bool,
}

impl Kernel {
    /// A kernel with pass band `cut` of the buffer's Nyquist frequency
    /// (clamped to 0.05..1).
    pub(crate) fn new(cut: f64) -> Kernel {
        let cut = if cut.is_finite() {
            cut.clamp(0.05, 1.0)
        } else {
            1.0
        };
        // At most HALF_TAPS / 0.05 = 640 samples: bounded.
        let half = ((HALF_TAPS as f64) / cut).ceil().max(1.0) as usize;
        let n = half.saturating_mul(2).saturating_mul(PHASES);
        let i0b = bessel_i0(KAISER_BETA);
        let mut table: Vec<f64> = (0..=n)
            .map(|i| {
                let x = i as f64 / PHASES as f64 - half as f64;
                let r = x / half as f64;
                let w = if r.abs() >= 1.0 {
                    0.0
                } else {
                    bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / i0b
                };
                let px = std::f64::consts::PI * cut * x;
                let sinc = if px.abs() < 1e-12 { 1.0 } else { px.sin() / px };
                cut * sinc * w
            })
            .collect();
        table.push(0.0);
        Kernel {
            half,
            table,
            full_band: cut >= 1.0,
        }
    }

    /// The band-limited value of `buf` at the fractional position `pos`
    /// (samples); zero outside the buffer.
    pub(crate) fn interp(&self, buf: &[f32], pos: f64) -> f64 {
        if !pos.is_finite() {
            return 0.0;
        }
        let n0 = pos.floor();
        let frac = pos - n0;
        if self.full_band && frac == 0.0 {
            if n0 >= 0.0 && n0 < buf.len() as f64 {
                return buf.get(n0 as usize).map_or(0.0, |v| f64::from(*v));
            }
            return 0.0;
        }
        let Some((window, skip)) = self.window(buf, n0) else {
            return 0.0;
        };
        let fp = frac * PHASES as f64;
        let fi = fp.floor();
        let t = fp - fi;
        let fi = fi as usize;
        let taps = self.half.saturating_mul(2);
        let mut acc = 0.0;
        for (i, v) in window.iter().enumerate() {
            // Tap i of the full window reads x = frac + half - 1 - i, at
            // table position (x + half) * PHASES.
            let tap = i.saturating_add(skip);
            let idx = fi.saturating_add(
                taps.saturating_sub(1)
                    .saturating_sub(tap)
                    .saturating_mul(PHASES),
            );
            let a = self.table.get(idx).copied().unwrap_or(0.0);
            let b = self
                .table
                .get(idx.saturating_add(1))
                .copied()
                .unwrap_or(0.0);
            acc += f64::from(*v) * (a + (b - a) * t);
        }
        acc
    }

    /// The weights of [`Kernel::interp`] for a fixed fractional part `frac`
    /// (0..1), tap `i` reading sample `floor(pos) - half + 1 + i`.
    pub(crate) fn weights(&self, frac: f64) -> Vec<f64> {
        let fp = frac.clamp(0.0, 1.0) * PHASES as f64;
        let fi = fp.floor();
        let t = fp - fi;
        let fi = fi as usize;
        let taps = self.half.saturating_mul(2);
        (0..taps)
            .map(|i| {
                let idx = fi.saturating_add(
                    taps.saturating_sub(1)
                        .saturating_sub(i)
                        .saturating_mul(PHASES),
                );
                let a = self.table.get(idx).copied().unwrap_or(0.0);
                let b = self
                    .table
                    .get(idx.saturating_add(1))
                    .copied()
                    .unwrap_or(0.0);
                a + (b - a) * t
            })
            .collect()
    }

    /// `sum(w[i] * buf[n0 - half + 1 + i])` over the samples inside `buf`.
    pub(crate) fn apply(&self, buf: &[f32], n0: f64, w: &[f64]) -> f64 {
        let Some((window, skip)) = self.window(buf, n0) else {
            return 0.0;
        };
        window
            .iter()
            .zip(w.iter().skip(skip))
            .map(|(v, w)| f64::from(*v) * w)
            .sum()
    }

    /// The part of `buf` the window around `n0` covers, and how many of the
    /// window's first taps fall before the buffer.
    fn window<'b>(&self, buf: &'b [f32], n0: f64) -> Option<(&'b [f32], usize)> {
        let half = self.half as f64;
        let taps = half * 2.0;
        let base = n0 - half + 1.0;
        let len = buf.len() as f64;
        if !(base + taps > 0.0 && base < len) {
            return None;
        }
        let skip = (-base).max(0.0);
        let first = base.max(0.0);
        let end = (base + taps).min(len);
        let w = buf.get(first as usize..end as usize)?;
        Some((w, skip as usize))
    }

    /// Half-width in samples (how far the kernel reads around a position).
    pub(crate) fn half(&self) -> usize {
        self.half
    }
}

/// The modified Bessel function of the first kind, order 0 (series).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..64u32 {
        let kf = f64::from(k);
        term *= q / (kf * kf);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// A counter-based normal deviate: the same `(seed, index)` always gives the
/// same value, so noise follows the microphone's own frame and a slip moves
/// it with the signal.
pub(crate) fn normal(seed: u64, index: u64) -> f64 {
    let a = splitmix64(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let b = splitmix64(a ^ 0xD1B5_4A32_D192_ED03);
    // 53-bit uniforms in (0, 1].
    let u1 = ((a >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
    let u2 = (b >> 11) as f64 / (1u64 << 53) as f64;
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

// -- WP P5RAMPFIX --
/// A seeded uniform in [0, 1) for `index`.
pub(crate) fn uniform(seed: u64, index: u64) -> f64 {
    let a = splitmix64(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    (a >> 11) as f64 / (1u64 << 53) as f64
}

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
