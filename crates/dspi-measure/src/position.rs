//! Measurement positions and reseats: averaging and spread (WP P5CAL,
//! PLAN-P5 T8 and section 1.3).
//!
//! - **Several positions** (3 to 9, speakers): the power average of the
//!   magnitudes, the spread per point (max minus min, dB) and **position 1's**
//!   phase (position 1 is the listening position and the only timing reference;
//!   nothing compares arrivals across runs).
//! - **Reseats** (1 to 9, headphones, left and right each averaged separately):
//!   the power average with the spread; no phase (a reseat moves the driver).
//! - **Near-field** and the **listening position** are one run each: no maths
//!   here beyond the placement checks; a near-field result is labelled as such
//!   by its document.
//!
//! The fit leaves alone any dip whose spread exceeds [`VARIABLE_DIP_SPREAD_DB`].

use crate::types::{LogGrid, MeasureError, Placement, Response};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// The most positions or reseats averaged.
pub const MAX_POSITIONS: usize = 9;
/// The fewest positions of a multi-position placement.
pub const MIN_MULTI_POSITIONS: usize = 3;
/// A spread above this (dB) marks a point as position-dependent (PLAN-P5
/// section 1.3: such dips are not filled).
pub const VARIABLE_DIP_SPREAD_DB: f64 = 3.0;

/// How the runs are averaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AverageKind {
    /// Speaker positions: position 1's phase is kept.
    Positions,
    /// Headphone reseats: no phase.
    Reseats,
}

/// An average of several runs of one output (or one headphone side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Averaged {
    /// The power-averaged magnitude (position 1's phase for positions), valid
    /// where every run is valid; `spl` when every run is.
    pub response: Response,
    /// Per point, the largest minus the smallest magnitude, dB (0 for one run).
    pub spread_db: Vec<f64>,
    /// How many runs were averaged.
    pub count: usize,
}

impl Averaged {
    /// Per point, whether the spread marks it as position-dependent.
    pub fn variable_mask(&self) -> Vec<bool> {
        self.spread_db
            .iter()
            .map(|s| *s > VARIABLE_DIP_SPREAD_DB)
            .collect()
    }
}

/// Checks a placement's counts: 3 to 9 positions, 1 to 9 reseats.
///
/// Errors: [`MeasureError::Invalid`].
pub fn validate_placement(p: &Placement) -> Result<(), MeasureError> {
    match *p {
        Placement::ListeningPosition | Placement::NearField => Ok(()),
        Placement::MultiPosition { count } => {
            if (MIN_MULTI_POSITIONS..=MAX_POSITIONS).contains(&usize::from(count)) {
                Ok(())
            } else {
                Err(MeasureError::Invalid(format!(
                    "{count} positions (3 to {MAX_POSITIONS})"
                )))
            }
        }
        Placement::Headphones { reseats, .. } => {
            if (1..=MAX_POSITIONS).contains(&usize::from(reseats)) {
                Ok(())
            } else {
                Err(MeasureError::Invalid(format!(
                    "{reseats} reseats (1 to {MAX_POSITIONS})"
                )))
            }
        }
    }
}

/// The runs a placement takes per output (per side for headphones).
pub fn runs_for(p: &Placement) -> usize {
    match *p {
        Placement::ListeningPosition | Placement::NearField => 1,
        Placement::MultiPosition { count } => usize::from(count),
        Placement::Headphones { reseats, .. } => usize::from(reseats),
    }
}

/// The power average of `runs` (1 to [`MAX_POSITIONS`], all on one grid):
/// `10 log10(mean(10^(dB/10)))` per point, with the spread.
///
/// Errors: [`MeasureError::Invalid`] for no runs or too many, different grids
/// or lengths, or a non-finite value.
pub fn average(runs: &[Response], kind: AverageKind) -> Result<Averaged, MeasureError> {
    let Some(first) = runs.first() else {
        return Err(MeasureError::Invalid("no runs to average".to_string()));
    };
    if runs.len() > MAX_POSITIONS {
        return Err(MeasureError::Invalid(format!(
            "{} runs (at most {MAX_POSITIONS})",
            runs.len()
        )));
    }
    let n = first.mag_db.len();
    let grid_len = grid_len(&first.grid)?;
    if n != grid_len {
        return Err(MeasureError::Invalid(
            "response length does not match its grid".to_string(),
        ));
    }
    for r in runs {
        let shape_ok = r.grid == first.grid
            && r.mag_db.len() == n
            && r.valid.as_ref().is_none_or(|v| v.len() == n)
            && r.phase_deg.as_ref().is_none_or(|v| v.len() == n);
        if !shape_ok {
            return Err(MeasureError::Invalid("runs on different grids".to_string()));
        }
        if r.mag_db.iter().any(|m| !m.is_finite())
            || r.phase_deg
                .as_ref()
                .is_some_and(|p| p.iter().any(|x| !x.is_finite()))
        {
            return Err(MeasureError::Invalid("non-finite run value".to_string()));
        }
    }
    let count_f = runs.len() as f64;
    let mut mag = Vec::with_capacity(n);
    let mut spread = Vec::with_capacity(n);
    let mut valid = Vec::with_capacity(n);
    for i in 0..n {
        let vals = runs.iter().filter_map(|r| r.mag_db.get(i).copied());
        // Power average relative to the largest value, so that very loud or
        // very quiet levels neither overflow nor underflow.
        let max = vals.clone().fold(f64::NEG_INFINITY, f64::max);
        let min = vals.clone().fold(f64::INFINITY, f64::min);
        let sum: f64 = vals.map(|m| 10f64.powf((m - max) / 10.0)).sum();
        let avg = max + 10.0 * (sum / count_f).log10();
        if !avg.is_finite() || !(max - min).is_finite() {
            return Err(MeasureError::Invalid("values out of range".to_string()));
        }
        mag.push(avg);
        spread.push((max - min).max(0.0));
        valid.push(runs.iter().all(|r| {
            r.valid
                .as_ref()
                .is_none_or(|v| v.get(i).copied().unwrap_or(false))
        }));
    }
    let phase = match kind {
        AverageKind::Positions => first.phase_deg.clone(),
        AverageKind::Reseats => None,
    };
    Ok(Averaged {
        response: Response {
            grid: first.grid,
            mag_db: mag,
            phase_deg: phase,
            valid: Some(valid),
            spl: runs.iter().all(|r| r.spl),
        },
        spread_db: spread,
        count: runs.len(),
    })
}

/// [`average`] for a placement: checks the count against the placement
/// (all its positions or reseats must be present) and picks the kind.
///
/// Errors: [`MeasureError::Invalid`] for a wrong count, a placement that takes
/// one run with more than one given, or [`average`]'s errors.
pub fn average_for(placement: &Placement, runs: &[Response]) -> Result<Averaged, MeasureError> {
    validate_placement(placement)?;
    let want = runs_for(placement);
    if runs.len() != want {
        return Err(MeasureError::Invalid(format!(
            "{} runs for a placement of {want}",
            runs.len()
        )));
    }
    let kind = match placement {
        Placement::Headphones { .. } => AverageKind::Reseats,
        Placement::ListeningPosition | Placement::NearField | Placement::MultiPosition { .. } => {
            AverageKind::Positions
        }
    };
    average(runs, kind)
}

fn grid_len(g: &LogGrid) -> Result<usize, MeasureError> {
    Ok(g.freqs()?.len())
}
