//! The compact curve encoding of the measurement documents (WP P5MDOC,
//! PLAN-P5 section 1.6, review RC7).
//!
//! A curve is held on a [`LogGrid`] given by its three numbers. Each array is a
//! base64 (standard alphabet, padded) string of little-endian integers:
//!
//! | Key | Values | Integer | Step |
//! |---|---|---|---|
//! | `mag` | magnitude, dB (dB SPL when `spl`) | `i16` | 0.01 dB |
//! | `phase` | unwrapped phase, degrees (optional) | `i32` | 0.1° |
//! | `spread` | spread across positions or reseats, dB (optional) | `i16` | 0.01 dB |
//! | `valid` | the trusted mask (optional) | bitset | point `i` is bit `i % 8` of byte `i / 8` |
//!
//! Every array holds exactly as many points as the grid; a bitset holds
//! `ceil(points / 8)` bytes and its padding bits are ignored. A standard-grid
//! curve (1066 points) with phase and mask is about 9 KB of text.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{LogGrid, MeasureError, Response};

/// The step a magnitude or spread is stored in, dB.
pub const MAG_STEP_DB: f64 = 0.01;
/// The step a phase is stored in, degrees.
pub const PHASE_STEP_DEG: f64 = 0.1;
/// The largest magnitude or spread a curve holds, dB (`i16::MAX` steps).
pub const MAX_MAG_DB: f64 = i16::MAX as f64 * MAG_STEP_DB;
/// The largest unwrapped phase a curve holds, degrees (`i32::MAX` steps).
pub const MAX_PHASE_DEG: f64 = i32::MAX as f64 * PHASE_STEP_DEG;

/// A response as a document holds it: the grid, the magnitude and optionally
/// the phase, the spread and the trusted mask, rounded to the steps above when
/// written. Unknown keys of the curve's object are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CurveWire", into = "CurveWire")]
pub struct Curve {
    /// The grid every array is given on.
    pub grid: LogGrid,
    /// Magnitude, dB, one per grid point.
    pub mag_db: Vec<f64>,
    /// Unwrapped phase, degrees, one per grid point.
    pub phase_deg: Option<Vec<f64>>,
    /// Per point, whether it is trusted.
    pub valid: Option<Vec<bool>>,
    /// Per point, the spread across positions or reseats, dB (at least 0).
    pub spread_db: Option<Vec<f64>>,
    /// True when `mag_db` is dB SPL through a calibrated microphone.
    pub spl: bool,
    /// Keys this build does not know, written back unchanged.
    pub extra: Map<String, Value>,
}

/// The curve's JSON object.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurveWire {
    grid: LogGrid,
    mag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    valid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spread: Option<String>,
    #[serde(default)]
    spl: bool,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl TryFrom<CurveWire> for Curve {
    type Error = MeasureError;

    fn try_from(w: CurveWire) -> Result<Self, Self::Error> {
        let n = w.grid.freqs()?.len();
        let mag_db = decode_i16(&w.mag, n, "mag")?;
        let phase_deg = w
            .phase
            .as_deref()
            .map(|s| decode_i32(s, n, "phase"))
            .transpose()?;
        let valid = w.valid.as_deref().map(|s| decode_bits(s, n)).transpose()?;
        let spread_db = w
            .spread
            .as_deref()
            .map(|s| decode_i16(s, n, "spread"))
            .transpose()?;
        Ok(Curve {
            grid: w.grid,
            mag_db,
            phase_deg,
            valid,
            spread_db,
            spl: w.spl,
            extra: w.extra,
        })
    }
}

impl From<Curve> for CurveWire {
    /// Encodes, saturating a value past the range (a non-finite value is 0):
    /// the documents' writers validate first, so this never loses a value
    /// they accept.
    fn from(c: Curve) -> Self {
        CurveWire {
            grid: c.grid,
            mag: encode_i16(&c.mag_db),
            phase: c.phase_deg.as_deref().map(encode_i32),
            valid: c.valid.as_deref().map(encode_bits),
            spread: c.spread_db.as_deref().map(encode_i16),
            spl: c.spl,
            extra: c.extra,
        }
    }
}

impl Curve {
    /// A curve holding `r` (no spread, no unknown keys).
    pub fn from_response(r: &Response) -> Curve {
        Curve {
            grid: r.grid,
            mag_db: r.mag_db.clone(),
            phase_deg: r.phase_deg.clone(),
            valid: r.valid.clone(),
            spread_db: None,
            spl: r.spl,
            extra: Map::new(),
        }
    }

    /// The curve as a [`Response`] (the spread is left out).
    pub fn to_response(&self) -> Response {
        Response {
            grid: self.grid,
            mag_db: self.mag_db.clone(),
            phase_deg: self.phase_deg.clone(),
            valid: self.valid.clone(),
            spl: self.spl,
        }
    }

    /// The curve with every value rounded to its stored step: what a write
    /// and a read give back.
    pub fn rounded(&self) -> Curve {
        let mut c = self.clone();
        c.mag_db
            .iter_mut()
            .for_each(|x| *x = round_to(*x, MAG_STEP_DB));
        for v in [c.phase_deg.as_mut()].into_iter().flatten() {
            v.iter_mut().for_each(|x| *x = round_to(*x, PHASE_STEP_DEG));
        }
        for v in [c.spread_db.as_mut()].into_iter().flatten() {
            v.iter_mut().for_each(|x| *x = round_to(*x, MAG_STEP_DB));
        }
        c
    }

    /// Checks the curve: a valid grid of at most
    /// [`MAX_GRID_POINTS`](crate::types::MAX_GRID_POINTS) points, every array
    /// as long as the grid, every value finite and within its range
    /// (magnitude and spread ±[`MAX_MAG_DB`], spread at least 0, phase
    /// ±[`MAX_PHASE_DEG`]).
    pub fn validate(&self, field: &str) -> Result<(), MeasureError> {
        let n = self
            .grid
            .freqs()
            .map_err(|e| MeasureError::Invalid(format!("{field}: {e}")))?
            .len();
        let len = |name: &str, got: usize| {
            if got == n {
                Ok(())
            } else {
                Err(MeasureError::Invalid(format!(
                    "{field}.{name}: {got} points on a grid of {n}"
                )))
            }
        };
        len("mag", self.mag_db.len())?;
        in_range(field, "mag", &self.mag_db, -MAX_MAG_DB, MAX_MAG_DB)?;
        if let Some(p) = &self.phase_deg {
            len("phase", p.len())?;
            in_range(field, "phase", p, -MAX_PHASE_DEG, MAX_PHASE_DEG)?;
        }
        if let Some(v) = &self.valid {
            len("valid", v.len())?;
        }
        if let Some(s) = &self.spread_db {
            len("spread", s.len())?;
            in_range(field, "spread", s, 0.0, MAX_MAG_DB)?;
        }
        Ok(())
    }
}

fn in_range(field: &str, name: &str, v: &[f64], lo: f64, hi: f64) -> Result<(), MeasureError> {
    match v
        .iter()
        .position(|x| !(x.is_finite() && (lo..=hi).contains(x)))
    {
        None => Ok(()),
        Some(i) => Err(MeasureError::Invalid(format!(
            "{field}.{name}: point {i} is not finite or outside {lo} to {hi}"
        ))),
    }
}

fn round_to(x: f64, step: f64) -> f64 {
    (x / step).round() * step
}

// `as` from f64 saturates and maps NaN to 0: the documented saturation.
fn encode_i16(v: &[f64]) -> String {
    let bytes: Vec<u8> = v
        .iter()
        .flat_map(|x| ((x / MAG_STEP_DB).round() as i16).to_le_bytes())
        .collect();
    B64.encode(bytes)
}

fn encode_i32(v: &[f64]) -> String {
    let bytes: Vec<u8> = v
        .iter()
        .flat_map(|x| ((x / PHASE_STEP_DEG).round() as i32).to_le_bytes())
        .collect();
    B64.encode(bytes)
}

fn encode_bits(v: &[bool]) -> String {
    let bytes: Vec<u8> = v
        .chunks(8)
        .map(|c| {
            c.iter().zip(0u32..).fold(0u8, |acc, (&on, k)| {
                if on {
                    acc | 1u8.checked_shl(k).unwrap_or(0)
                } else {
                    acc
                }
            })
        })
        .collect();
    B64.encode(bytes)
}

fn decode_bytes(s: &str, want: usize, name: &str) -> Result<Vec<u8>, MeasureError> {
    // A string longer than the grid's padded base64 is refused before decoding.
    let max_text = want.div_ceil(3).saturating_mul(4);
    if s.len() > max_text {
        return Err(MeasureError::Invalid(format!(
            "{name}: {} characters for {want} bytes",
            s.len()
        )));
    }
    let bytes = B64
        .decode(s)
        .map_err(|e| MeasureError::Parse(format!("{name}: not base64 ({e})")))?;
    if bytes.len() != want {
        return Err(MeasureError::Invalid(format!(
            "{name}: {} bytes, the grid needs {want}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn decode_i16(s: &str, n: usize, name: &str) -> Result<Vec<f64>, MeasureError> {
    let bytes = decode_bytes(s, n.saturating_mul(2), name)?;
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f64::from(i16::from_le_bytes(*b)) * MAG_STEP_DB)
        .collect())
}

fn decode_i32(s: &str, n: usize, name: &str) -> Result<Vec<f64>, MeasureError> {
    let bytes = decode_bytes(s, n.saturating_mul(4), name)?;
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f64::from(i32::from_le_bytes(*b)) * PHASE_STEP_DEG)
        .collect())
}

fn decode_bits(s: &str, n: usize) -> Result<Vec<bool>, MeasureError> {
    let bytes = decode_bytes(s, n.div_ceil(8), "valid")?;
    Ok(bytes
        .iter()
        .flat_map(|b| (0u32..8).map(move |k| b.checked_shr(k).unwrap_or(0) & 1 == 1))
        .take(n)
        .collect())
}
