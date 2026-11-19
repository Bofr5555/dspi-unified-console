//! A measurement document's curves for a graph (WP P5CURV): each output's
//! stored response smoothed, and optionally the target levelled to it, with
//! the same smoothing and levelling the live view uses (P5LIVE: P5RESP's
//! `smooth`, the live view's `levelled_target`). The curves stay on
//! [`LogGrid::STANDARD`] (1/96 octave, the documents' grid) and carry the
//! trusted mask, so a client draws untrusted points faint and computes
//! nothing itself (R1). No device, no files.

use dspi_measure::doc::{MeasurementDoc, TargetDoc};
use dspi_measure::response::{MAX_SMOOTHING, resample, smooth};
use dspi_measure::target::{
    DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse, default_target,
};
use dspi_measure::types::{LogGrid, Placement, Response, WayRole};
use serde::{Deserialize, Serialize};

use super::live::levelled_target;
use super::{MeasureLiveCurve, MeasureRole};
use crate::{DspiError, Result};

/// How to give a measurement's curves (`measure_doc_curves`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureDocCurvesOptions {
    /// The smoothing, 1/N octave (1 to 48); 0 for none.
    pub smoothing: u8,
    /// Give the target too, levelled to each output's curve.
    pub target: bool,
    /// A `target` document's text; "" for the placement's default target.
    pub target_document: String,
}

/// One output's curves.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDocCurve {
    /// The output, 0-based.
    pub output: u8,
    /// Its way.
    pub role: MeasureRole,
    /// The speaker it belongs to ("" when the layout named none).
    pub speaker: String,
    /// The measured response, smoothed (untrusted points flagged).
    pub measured: MeasureLiveCurve,
    /// The target levelled to the measured response, when asked for.
    pub target: Option<MeasureLiveCurve>,
    /// Per point, the spread across positions or reseats, dB; empty when
    /// the document has none.
    pub spread_db: Vec<f64>,
}

/// A measurement document's curves (`measure_doc_curves`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDocCurves {
    /// The smoothing applied, 1/N octave; 0 none.
    pub smoothing: u8,
    /// The run's timing was reliable.
    pub timing_reliable: bool,
    /// One per measured output, in the document's order.
    pub outputs: Vec<MeasureDocCurve>,
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

fn role_of(r: WayRole) -> MeasureRole {
    match r {
        WayRole::Full => MeasureRole::Full,
        WayRole::Sub => MeasureRole::Sub,
        WayRole::Low => MeasureRole::Low,
        WayRole::Mid => MeasureRole::Mid,
        WayRole::High => MeasureRole::High,
    }
}

/// `resp` on the standard grid: as it is when it is on it already (the
/// documents' grid), else resampled.
fn on_standard(resp: Response) -> Result<Response> {
    if resp.grid == LogGrid::STANDARD {
        return Ok(resp);
    }
    resample(&resp, &LogGrid::STANDARD).map_err(|e| invalid(e.to_string()))
}

/// `resp` as a curve on the standard grid (the live view's `curve`, without
/// its resampling to the display grid).
fn standard_curve(resp: Response) -> Result<MeasureLiveCurve> {
    let r = on_standard(resp)?;
    let freqs = LogGrid::STANDARD
        .freqs()
        .map_err(|e| invalid(e.to_string()))?;
    let valid = r
        .valid
        .clone()
        .unwrap_or_else(|| vec![true; r.mag_db.len()]);
    Ok(MeasureLiveCurve {
        freqs_hz: freqs,
        mag_db: r
            .mag_db
            .iter()
            .map(|m| if m.is_finite() { *m } else { 0.0 })
            .collect(),
        valid,
        spl: r.spl,
    })
}

/// The target `options` names, on the standard grid.
fn target_of(doc: &MeasurementDoc, options: &MeasureDocCurvesOptions) -> Result<Response> {
    let target = if options.target_document.trim().is_empty() {
        let (target_use, coupler) = match doc.measurement.placement {
            Placement::Headphones { coupler, .. } => (TargetUse::Headphones, Some(coupler)),
            _ => (TargetUse::Speakers, None),
        };
        Target {
            target_use,
            coupler,
            definition: TargetDefinition::Builtin {
                builtin: default_target(target_use, coupler),
            },
            reference: DEFAULT_REFERENCE_BAND,
        }
    } else {
        TargetDoc::from_text(&options.target_document)
            .and_then(|d| d.target.to_target())
            .map_err(|e| invalid(format!("the target document: {e}")))?
    };
    target
        .evaluate(&LogGrid::STANDARD)
        .map_err(|e| invalid(format!("the target: {e}")))
}

/// A measurement document's curves for a graph: each output's response
/// smoothed by `options.smoothing` (1/N octave, 0 none) and, with
/// `options.target`, the target (the placement's default, or
/// `options.target_document`) levelled to it as the live view levels it. On
/// the standard grid (1/96 octave), with the trusted mask and the spread
/// carried through. No device, no files.
///
/// Errors: [`DspiError::InvalidArgument`] for a text that is not a valid
/// measurement document, a smoothing past 1/48 octave, or a target document
/// that does not read.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn measure_doc_curves(
    text: String,
    options: MeasureDocCurvesOptions,
) -> Result<MeasureDocCurves> {
    if options.smoothing > MAX_SMOOTHING {
        return Err(invalid(format!(
            "smoothing 1/{} octave (0, or 1 to 48)",
            options.smoothing
        )));
    }
    let doc = MeasurementDoc::from_text(&text)
        .map_err(|e| invalid(format!("the measurement document: {e}")))?;
    let target = if options.target {
        Some(target_of(&doc, &options)?)
    } else {
        None
    };
    let mut outputs = Vec::with_capacity(doc.measurement.outputs.len());
    for o in &doc.measurement.outputs {
        let raw = on_standard(o.response.to_response())?;
        let shown = if options.smoothing > 0 {
            smooth(&raw, options.smoothing).map_err(|e| invalid(e.to_string()))?
        } else {
            raw.clone()
        };
        let target = match &target {
            Some(t) => Some(standard_curve(levelled_target(t, &raw))?),
            None => None,
        };
        let spread_db = match &o.response.spread_db {
            Some(s) => {
                on_standard(Response {
                    grid: o.response.grid,
                    mag_db: s.clone(),
                    phase_deg: None,
                    valid: None,
                    spl: false,
                })?
                .mag_db
            }
            None => Vec::new(),
        };
        outputs.push(MeasureDocCurve {
            output: o.output,
            role: role_of(o.role),
            speaker: o.speaker.clone(),
            measured: standard_curve(shown)?,
            target,
            spread_db,
        });
    }
    Ok(MeasureDocCurves {
        smoothing: options.smoothing,
        timing_reliable: doc.measurement.capture.timing_reliable,
        outputs,
    })
}
