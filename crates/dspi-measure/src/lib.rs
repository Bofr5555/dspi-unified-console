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
//! | [`stimulus`] | the generator's signals, modelled sample by sample as `siggen.c` renders them | P5DSP |
//! | [`ir`] | deconvolution, cycle segmentation, clock-drift estimate, coherent averaging, windowing | P5DSP |
//! | [`response`] | impulse response to magnitude and phase on a [`types::LogGrid`]; fractional-octave smoothing | P5DSP |
//! | [`rta`] | the microphone's real-time spectrum (Welch, pink-referenced) | P5DSP |
//! | [`export`] | REW-compatible text export of responses and impulse responses | P5DSP |
//! | [`cal`] | microphone calibration files: parse, sensitivity, apply | P5CAL |
//! | [`target`] | target curves: built-ins, import, coupler compensation | P5CAL |
//! | [`position`] | positions: averaging, spread, near-field splice | P5CAL |
//! | [`fit`] | the PEQ fit toward a target, bounded | P5FIT |
//! | [`align`] | delay and polarity between ways | P5XO |
//! | [`xover`] | crossover choice (frequency, family and order, levels) and the predicted sum | P5XO |
//! | `sim` (feature `sim`) | the simulated acoustic world | P5SIM |
//! | [`wav`] | a bounded WAV reader and writer (captures, replay golden files) | P5SIM |
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
// -- WP P5DSP --
pub mod export;
pub mod ir;
pub mod response;
pub mod rta;
pub mod stimulus;
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

pub use types::*;
