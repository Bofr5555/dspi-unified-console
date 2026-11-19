//! Measurement maths for the DSPi console (PRO phase 5, `docs/core-merge/PLAN-P5.md`).
//!
//! Pure functions and plain data: no I/O, no threads, no clock, no device and no
//! audio API, so the same code runs in every client and in wasm32. The measurement
//! *workflow* (the generator, the device lock, the safety ramp, the documents) is
//! `dspi-core`'s `measure` module; this crate only computes.
//!
//! Rules for every module (PLAN.md section 1.7 and 1.9): no panic on any input
//! (captured audio, a calibration or target file, a document from another client);
//! every buffer bounded by a named constant; non-finite input is an error or is
//! skipped, never propagated into a proposal; every public function has a
//! `proptest_*` test that it neither panics nor returns non-finite values.
//!
//! | Module | What | WP |
//! |---|---|---|
//! | [`types`] | the shared seam: grids, responses, bands, chains, the section model | PLANP5 |
//! | [`stimulus`] | the generator's log sweep and walk schedule, modelled sample by sample as `siggen.c` renders them | P5TIME |
//! | [`ir`] | deconvolution, cycle identity, clock-drift estimate and correction, the timing quality gate, coherent averaging, windows | P5TIME |
//! | [`response`] | impulse response to magnitude and phase on a [`types::LogGrid`]; fractional-octave smoothing | P5RESP |
//! | [`rta`] | the microphone's real-time spectrum (Welch, pink-referenced) | P5RESP |
//! | [`export`] | REW-compatible text export of responses | P5RESP |
//! | [`cal`] | microphone calibration files: parse, sensitivity, apply | P5CAL |
//! | [`target`] | target curves: built-ins, import, coupler compensation | P5CAL |
//! | [`position`] | positions and reseats: averaging, spread | P5CAL |
//! | [`fit`] | the PEQ fit toward a target, bounded | P5FIT |
//! | [`align`] | delay and polarity between ways | P5XO |
//! | [`xover`] | crossover choice (corner and levels) and the predicted sum | P5XO |
//! | `sim` (feature `sim`) | the simulated acoustic world | P5SIM |
//! | [`wav`] | a bounded WAV reader and writer (captures, replay golden files) | P5SIM |
//! | [`doc`] | the `measurement`, `mic` and `target` documents | P5MDOC |
//! | [`hearing`] | does the microphone hear a ramp step: the sweep's whitened matched filter, pink noise's band levels | P5RAMPFIX |
//! | [`system`] | several units on one time base: segments, domains, time pins (PLAN-AT) | PLANAT, ATSYS |
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod types;

// Each WP below fills its own file; the `mod` lines are here so that wave P5-A
// needs no edit to this file.
// -- WP P5TIME --
pub mod ir;
pub mod stimulus;
// -- WP P5RESP --
pub mod export;
pub mod response;
pub mod rta;
// -- WP P5CAL --
pub mod cal;
pub mod position;
pub mod target;
// -- WP P5FIT --
pub mod fit;
// -- WP P5XO --
pub mod align;
pub mod xover;
// -- WP P5SIM --
#[cfg(feature = "sim")]
pub mod sim;
pub mod wav;
// -- WP P5MDOC --
pub mod doc;
// -- WP P5RAMPFIX --
pub mod hearing;
// -- WP PLANAT -- several units on one time base (PLAN-AT; ATSYS, ATSIM fill it)
pub mod system;

pub use types::*;

// -- review P5-A -- what the wave's parts must agree on (one level
// convention, one smoothing, one filter response, cal and target types that
// line up with the documents).
#[cfg(test)]
mod tests_review_p5a;

// -- WP ATE2E -- ATSIM's world through ATSYS's analyser over many seeds: the
// offer rule of R8c, the gate's pass rate as the microphone's wander falls,
// and three runs chained through an anchor in one capture.
#[cfg(all(test, feature = "sim"))]
mod tests_wp_ate2e;
