//! Filters a scenario adds to one output of a world (WP FUNC1): an output's
//! own irregularities and the room modes its position excites, where a
//! [`super::World`] has one driver model and one room for all. They are run
//! over the generator's samples before [`super::render`], and their complex
//! response is the ground truth a harness judges a tuning with.

use super::SimFilter;
use super::dsp::run_biquad;
use crate::types::Coeffs;

/// Run `filters` over `buf` in place at the rate `fs`.
pub fn apply_filters(filters: &[SimFilter], fs: f64, buf: &mut [f32]) {
    for f in filters {
        run_biquad(&f.design(fs), buf);
    }
}

/// One section's complex response at `freq` for the rate `fs`.
pub fn section_response(c: &Coeffs, freq: f64, fs: f64) -> (f64, f64) {
    let w = 2.0 * std::f64::consts::PI * freq / fs;
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let (nr, ni) = (c.b0 + c.b1 * c1 + c.b2 * c2, -(c.b1 * s1 + c.b2 * s2));
    let (dr, di) = (1.0 + c.a1 * c1 + c.a2 * c2, -(c.a1 * s1 + c.a2 * s2));
    let d = (dr * dr + di * di).max(1e-300);
    ((nr * dr + ni * di) / d, (ni * dr - nr * di) / d)
}

/// The complex response of `filters` in a row at `freq` for the rate `fs`.
pub fn filters_response(filters: &[SimFilter], freq: f64, fs: f64) -> (f64, f64) {
    filters.iter().fold((1.0, 0.0), |(a, b), f| {
        let (c, d) = section_response(&f.design(fs), freq, fs);
        (a * c - b * d, a * d + b * c)
    })
}
