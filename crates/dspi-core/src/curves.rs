//! Stateless helpers: filter response curves and version facts.
//!
//! The old core's `dsp_math` did these; `dspi_proto::dsp` has the same maths
//! (biquad coefficients for every filter type). The curve grid is 201 points,
//! log-spaced 10 Hz..20 kHz, as the desktop graph and the Android app expect.
//!
//! Owned by WP C12 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use crate::records::{CoreInfo, FilterParams};

/// Points in a curve from [`compute_magnitude_curve`] / [`compute_phase_curve`].
pub const CURVE_POINTS: usize = 201;

/// Magnitude in dB at one frequency for a chain of bands.
pub fn compute_response(filters: Vec<FilterParams>, freq: f32) -> f32 {
    todo!("WP C12")
}

/// 201-point magnitude curve, dB, on the [`curve_freq`] grid.
pub fn compute_magnitude_curve(filters: Vec<FilterParams>) -> Vec<f64> {
    todo!("WP C12")
}

/// 201-point phase curve in degrees on the same grid; continuous when
/// `unwrap`, else wrapped to ±180°.
pub fn compute_phase_curve(filters: Vec<FilterParams>, unwrap: bool) -> Vec<f64> {
    todo!("WP C12")
}

/// Frequency (Hz) of curve point `i` (0..200).
pub fn curve_freq(i: u32) -> f64 {
    todo!("WP C12")
}

/// Whether `map` (4 bytes, `DspState.crossfeed_pair_map` encoding) is a pair
/// map the firmware accepts for `num_pairs` stereo pairs (4 on RP2350, 2 on
/// RP2040): every pair gets two distinct non-PDM outputs, no output is used
/// twice, bytes beyond `num_pairs` are 0.
pub fn crossfeed_pair_map_valid(map: Vec<u8>, num_pairs: u8) -> bool {
    todo!("WP C12")
}

/// Version facts of the linked core.
pub fn core_info() -> CoreInfo {
    todo!("WP C12")
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C12"]
    fn curves_match_the_old_cores_values() {
        todo!("WP C12: port dsp_math's tests from crates/dspi-core-linux")
    }
}
