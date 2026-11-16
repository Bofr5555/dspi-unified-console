//! The shared seam of phase 5 (WP PLANP5): the types every measurement module
//! and the core's `measure` workflow exchange.
//!
//! Kept small on purpose. A WP that needs another shared type adds it here under
//! its own `// -- WP <ID> --` marker and lists the edit in its report; it never
//! changes the meaning of a field below.

use serde::{Deserialize, Serialize};

pub use dspi_proto::dsp::Coeffs;

/// Lowest frequency any response, target or calibration is held at, Hz.
pub const GRID_MIN_HZ: f64 = 10.0;
/// Highest frequency any response is held at, Hz (below the Nyquist frequency
/// of the lowest rate the generator runs at, 44.1 kHz).
pub const GRID_MAX_HZ: f64 = 22_000.0;
/// Points per octave of the standard grid ([`LogGrid::STANDARD`]).
pub const GRID_POINTS_PER_OCTAVE: u16 = 96;
/// The most points any grid may hold: bounds every response, target,
/// calibration and document (OD-P5g).
pub const MAX_GRID_POINTS: usize = 2048;

/// Errors of the measurement maths. Typed, never a panic (PLAN.md section 1.7).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MeasureError {
    /// An input outside what the function accepts (a grid too large, a rate of
    /// zero, a length past a bound); nothing was computed.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// Not enough signal to answer: too short, too quiet, or too noisy.
    #[error("not enough signal: {0}")]
    NoSignal(String),
    /// A file (calibration, target, WAV) that could not be read; the text says
    /// where and why, never the file's contents.
    #[error("unreadable file: {0}")]
    Parse(String),
}

/// A logarithmic frequency grid: `points_per_octave` points per octave from
/// `min_hz` to `max_hz` inclusive of the first point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogGrid {
    /// The first point, Hz (> 0).
    pub min_hz: f64,
    /// No point lies above this, Hz.
    pub max_hz: f64,
    /// Points per octave (1 to 384).
    pub points_per_octave: u16,
}

impl LogGrid {
    /// The grid every stored response uses: 10 Hz to 22 kHz at 1/96 octave
    /// (1066 points).
    pub const STANDARD: LogGrid = LogGrid {
        min_hz: GRID_MIN_HZ,
        max_hz: GRID_MAX_HZ,
        points_per_octave: GRID_POINTS_PER_OCTAVE,
    };

    /// The grid's frequencies, Hz, ascending.
    ///
    /// Errors: [`MeasureError::Invalid`] for a non-finite or non-positive
    /// bound, `max_hz <= min_hz`, a density outside 1..=384, or more than
    /// [`MAX_GRID_POINTS`] points.
    pub fn freqs(&self) -> Result<Vec<f64>, MeasureError> {
        let ok = self.min_hz.is_finite()
            && self.max_hz.is_finite()
            && self.min_hz > 0.0
            && self.max_hz > self.min_hz
            && (1..=384).contains(&self.points_per_octave);
        if !ok {
            return Err(MeasureError::Invalid(format!("bad grid {self:?}")));
        }
        let octaves = (self.max_hz / self.min_hz).log2();
        let n_f = (octaves * f64::from(self.points_per_octave)).floor() + 1.0;
        if !(1.0..=MAX_GRID_POINTS as f64).contains(&n_f) {
            return Err(MeasureError::Invalid(format!(
                "grid of {n_f} points (at most {MAX_GRID_POINTS})"
            )));
        }
        // In range by the check above.
        let n = n_f as usize;
        let step = 1.0 / f64::from(self.points_per_octave);
        Ok((0..n)
            .map(|i| self.min_hz * (i as f64 * step).exp2())
            .collect())
    }
}

/// A frequency response on a [`LogGrid`]: what a measurement, a calibration,
/// a target, a prediction and a fit all exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// The grid `mag_db` (and `phase_deg`) are given on.
    pub grid: LogGrid,
    /// Magnitude, dB (relative, or dB SPL when [`Response::spl`] is true).
    pub mag_db: Vec<f64>,
    /// Unwrapped phase, degrees, relative to the measurement's timing reference
    /// (OD-P5b); `None` where phase is not known (an RTA, a magnitude-only
    /// target or calibration, a multi-position average).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_deg: Option<Vec<f64>>,
    /// Per point, whether the value is trusted (signal above the noise floor,
    /// inside the window's resolution, inside the calibration's range). A fit
    /// or an alignment never uses an untrusted point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<Vec<bool>>,
    /// True when `mag_db` is absolute (dB SPL through a calibrated microphone).
    #[serde(default)]
    pub spl: bool,
}

/// One band of a channel, as the device holds it: the core's `FilterParams`
/// without the binding attributes, so this crate needs no `dspi-core`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandSpec {
    /// The firmware's filter type byte: 0 flat, 1..13 the PEQ shapes, 32..63
    /// the crossover family, order and pass codes (`dspi_proto::xover`).
    pub filter_type: u8,
    /// Switched off without losing its settings.
    pub bypass: bool,
    /// Centre or corner, Hz (`f0` for a Linkwitz Transform).
    pub freq: f32,
    /// Quality factor (`Q0` for a Linkwitz Transform).
    pub q: f32,
    /// Gain, dB (the target corner `fp`, Hz, for a Linkwitz Transform).
    pub gain: f32,
    /// The Linkwitz Transform's target Q; unused otherwise.
    pub qp: f32,
}

/// One output channel's processing **after the generator's injection point**:
/// the PEQ bank, the crossover bank, output gain and delay. This is the
/// *measured chain* a measurement is de-embedded with (PLAN-P5 section 1.1).
///
/// The generator **replaces** the matrix mix on the outputs it plays
/// (`audio_pipeline.c:503-536`), so crosspoint gain and crosspoint polarity are
/// not in its path; they are in [`ChannelChain::feed`], used only to predict
/// programme audio (review RC1). Master and user volume are a common gain and
/// left out; processing the maths does not model (crossfeed, bass, tube,
/// subharmonic, loudness, limiter) is refused by the preflight (RC2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelChain {
    /// The PEQ bank, band 0 first.
    pub peq: Vec<BandSpec>,
    /// The crossover bank, slot 0 first.
    pub crossover: Vec<BandSpec>,
    /// Output gain, dB.
    pub gain_db: f32,
    /// Output delay, ms.
    pub delay_ms: f32,
    /// How programme audio reaches this output through the matrix: **not**
    /// in the generator's path.
    pub feed: ProgrammeFeed,
}

/// How programme audio reaches an output through the matrix (the crosspoints
/// that feed it). The generator bypasses this; a prediction of what programme
/// audio will do adds it, and a summed generator run reproduces its polarity
/// with the generator's `invert_mask` (review RC1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgrammeFeed {
    /// The net crosspoint gain of the output's main source, dB.
    pub gain_db: f32,
    /// The net polarity of the feeding crosspoints is inverted.
    pub inverted: bool,
}

/// The biquad sections the device builds for a band. The core implements it
/// with its own cascade (`dspi-core` `curves.rs`), so the measurement maths
/// and the graph can never disagree about a filter; tests use a plain model
/// over `dspi_proto::dsp` and `dspi_proto::xover`.
pub trait SectionModel {
    /// The sections for one band, in cascade order; empty for a flat or
    /// bypassed band. Never panics; a band it cannot model gives no sections
    /// and [`SectionModel::models`] says false for it.
    fn sections(&self, band: &BandSpec) -> Vec<Coeffs>;
    /// Whether this model knows the band's type (a fit or a crossover search
    /// refuses a chain with a band it cannot model).
    fn models(&self, band: &BandSpec) -> bool;
    /// The sample rate the sections are for, Hz.
    fn sample_rate(&self) -> f64;
}

/// Where the microphone is, as the user chose it (owner requirement 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum Placement {
    /// Speakers, one listening position.
    ListeningPosition,
    /// Speakers, several positions around the listening position, averaged.
    MultiPosition {
        /// Positions to take, 3 to 9.
        count: u8,
    },
    /// Speakers, the microphone close to one driver (driver data, crossover
    /// design; spliced to a far-field measurement below the splice frequency).
    NearField,
    /// Headphones on a coupler.
    Headphones {
        /// The coupler type.
        coupler: Coupler,
        /// Reseats to take and average, 1 to 9.
        reseats: u8,
    },
}

/// A headphone coupler type: each has its own compensation and default target
/// (P5CAL's `target`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coupler {
    /// An IEC 60318-4 (711-type) occluded-ear simulator, with or without pinna.
    Iec60318_4,
    /// A flat-plate coupler.
    FlatPlate,
    /// A head and torso simulator or another ear coupler with pinna
    /// (IEC 60318-7 style).
    Hats,
}

/// A way of a loudspeaker, as the tune's layout names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WayRole {
    /// A full-range output (no crossover to tune).
    Full,
    /// A subwoofer.
    Sub,
    /// The lowest way of a multi-way speaker.
    Low,
    /// A middle way.
    Mid,
    /// The highest way: never measured without its protective high-pass (OD-P5i).
    High,
}

#[cfg(test)]
mod tests_wp_planp5 {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn wp_planp5_standard_grid_spans_10_hz_to_22_khz_at_1_96_octave() {
        let f = LogGrid::STANDARD.freqs().unwrap();
        assert_eq!(f.len(), 1066);
        assert!((f[0] - 10.0).abs() < 1e-9);
        assert!(*f.last().unwrap() <= GRID_MAX_HZ);
        assert!(f.windows(2).all(|w| w[1] > w[0]));
        let ratio = f[96] / f[0];
        assert!(
            (ratio - 2.0).abs() < 1e-9,
            "one octave is 96 points: {ratio}"
        );
    }

    #[test]
    fn wp_planp5_bad_grids_are_refused() {
        let bad = [
            LogGrid {
                min_hz: 0.0,
                ..LogGrid::STANDARD
            },
            LogGrid {
                min_hz: f64::NAN,
                ..LogGrid::STANDARD
            },
            LogGrid {
                max_hz: 5.0,
                ..LogGrid::STANDARD
            },
            LogGrid {
                points_per_octave: 0,
                ..LogGrid::STANDARD
            },
            LogGrid {
                points_per_octave: 384,
                min_hz: 1.0,
                max_hz: 1e9,
            },
        ];
        for g in bad {
            assert!(matches!(g.freqs(), Err(MeasureError::Invalid(_))), "{g:?}");
        }
    }

    #[test]
    fn wp_planp5_placement_round_trips_as_json() {
        let p = Placement::Headphones {
            coupler: Coupler::Iec60318_4,
            reseats: 5,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Placement>(&s).unwrap(), p);
    }

    proptest! {
        #[test]
        fn proptest_wp_planp5_grid_never_panics(min in proptest::num::f64::ANY, max in proptest::num::f64::ANY, ppo in 0u16..1000) {
            let g = LogGrid { min_hz: min, max_hz: max, points_per_octave: ppo };
            if let Ok(f) = g.freqs() {
                prop_assert!(!f.is_empty() && f.len() <= MAX_GRID_POINTS);
                prop_assert!(f.iter().all(|x| x.is_finite() && *x > 0.0));
            }
        }
    }
}
