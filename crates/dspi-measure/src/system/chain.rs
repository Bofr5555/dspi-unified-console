//! Runs longer than one capture, chained through an anchor output (WP ATSYS,
//! PLAN-AT OD-ATk): every run of a group repeats one anchor output, each
//! run's arrivals are kept relative to the anchor's in that run, and the
//! chain holds only when the anchor's responses agree across the runs (the
//! microphone and the room did not change): a magnitude spread of at most
//! [`MAX_ANCHOR_SPREAD_DB`] over [`ANCHOR_BAND_HZ`] (or over the anchor's
//! own passband when it is not a full-range output, suggestion 5), and, when
//! the runs' captures count frames on one clock, the anchor's arrivals on
//! the domain's timeline within [`MAX_ANCHOR_ARRIVAL_US`] of one drift fit.

use crate::types::{MeasureError, Response};

use super::AnchorOutput;
use super::analysis::{MAX_OFFERED_UNCERTAINTY_US, SystemAnalysis, UnitOutput, passband};

/// The band the anchor's magnitudes are compared over, Hz.
pub const ANCHOR_BAND_HZ: (f64, f64) = (200.0, 5_000.0);
/// The largest spread of the anchor's magnitude across runs, dB.
pub const MAX_ANCHOR_SPREAD_DB: f64 = 1.0;
/// The largest distance of an anchor arrival from the runs' drift fit, µs.
pub const MAX_ANCHOR_ARRIVAL_US: f64 = 20.0;
/// The most runs one chain joins.
pub const MAX_CHAIN_RUNS: usize = 16;

/// One run of a chain.
#[derive(Debug, Clone, Copy)]
pub struct ChainRun<'a> {
    /// Its analysis.
    pub analysis: &'a SystemAnalysis,
    /// The anchor's response in it (calibrated, on the run's grid), as the
    /// session derives every output's response.
    pub anchor_response: &'a Response,
}

/// How the chain is judged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChainOptions {
    /// The comparison band, Hz; `None`: [`ANCHOR_BAND_HZ`] when the anchor's
    /// passband (in the first run) covers it, else that passband.
    pub band_hz: Option<(f64, f64)>,
    /// [`MAX_ANCHOR_SPREAD_DB`].
    pub max_spread_db: f64,
    /// [`MAX_ANCHOR_ARRIVAL_US`].
    pub max_arrival_us: f64,
    /// The runs' captures count frames from one origin on one clock (the
    /// microphone kept open across the runs): the anchor's arrivals are
    /// then compared on the timeline too. (Review AT-A/AT-B: a weak check.
    /// The line through three runs takes most of a move itself: the anchor
    /// moved by `d` in the first or last run shows as `d / 3`, in the middle
    /// one as `2 d / 3`; and a capture clock whose rate drifts over minutes
    /// leaves the line by itself. The magnitude check is the one that
    /// holds; this one adds little with few runs.)
    pub one_capture_clock: bool,
}

impl Default for ChainOptions {
    fn default() -> ChainOptions {
        ChainOptions {
            band_hz: None,
            max_spread_db: MAX_ANCHOR_SPREAD_DB,
            max_arrival_us: MAX_ANCHOR_ARRIVAL_US,
            one_capture_clock: false,
        }
    }
}

/// Why runs do not chain.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChainRefusal {
    /// Fewer than two runs, or more than [`MAX_CHAIN_RUNS`].
    #[error("{0} runs (2 to 16 are chained)")]
    Runs(usize),
    /// A run without the anchor output.
    #[error("run {run} does not repeat the anchor output")]
    AnchorMissing {
        /// The run.
        run: usize,
    },
    /// The anchor's magnitudes disagree.
    #[error(
        "the microphone or the room changed between runs: the anchor output's response differs by {spread_db:.1} dB at {freq_hz:.0} Hz"
    )]
    Moved {
        /// The largest spread, dB.
        spread_db: f64,
        /// Where, Hz.
        freq_hz: f64,
    },
    /// The anchor's arrivals disagree on the timeline.
    #[error(
        "the microphone or the room changed between runs: the anchor output arrived {us:.1} µs off the runs' drift fit"
    )]
    ArrivalMoved {
        /// The largest distance, µs.
        us: f64,
    },
    /// No comparison band.
    #[error("no band to compare the anchor output in: {0}")]
    Band(String),
    /// Bad input.
    #[error(transparent)]
    Measure(#[from] MeasureError),
}

/// One output of a chained run.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainedOutput {
    /// The run it was measured in.
    pub run: usize,
    /// The output.
    pub output: UnitOutput,
    /// Its arrival after the anchor's in that run, ms.
    pub relative_ms: f64,
    /// The uncertainty of that, µs (`None` without two clear passes).
    pub uncertainty_us: Option<f64>,
    /// A proposal may use it: its run's timing is reliable and, for an output
    /// on another unit than the anchor, the uncertainty is at most
    /// [`MAX_OFFERED_UNCERTAINTY_US`].
    pub offered: bool,
}

/// Chained runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    /// The anchor.
    pub anchor: AnchorOutput,
    /// The band its magnitudes were compared over, Hz.
    pub band_hz: (f64, f64),
    /// Their largest spread, dB.
    pub anchor_spread_db: f64,
    /// The anchor's largest distance from the drift fit across runs, µs
    /// (`None` when not compared: separate capture clocks or two runs).
    pub anchor_arrival_us: Option<f64>,
    /// Every output of every run but the anchor (the anchor once, from run
    /// 0, at 0 ms).
    pub outputs: Vec<ChainedOutput>,
}

/// Joins runs that each repeat `anchor`.
///
/// Errors: [`ChainRefusal`]; nothing is proposed from refused runs.
pub fn chain_runs(
    anchor: AnchorOutput,
    runs: &[ChainRun<'_>],
    options: &ChainOptions,
) -> Result<Chain, ChainRefusal> {
    if runs.len() < 2 || runs.len() > MAX_CHAIN_RUNS {
        return Err(ChainRefusal::Runs(runs.len()));
    }
    let mut anchors = Vec::with_capacity(runs.len());
    for (i, r) in runs.iter().enumerate() {
        match r.analysis.output(anchor.unit, anchor.output) {
            Some(a) => anchors.push(a),
            None => return Err(ChainRefusal::AnchorMissing { run: i }),
        }
    }
    let first = runs.first().map(|r| r.anchor_response);
    let Some(first) = first else {
        return Err(ChainRefusal::Runs(0));
    };
    let band = match options.band_hz {
        Some(b) => b,
        None => match passband(first) {
            Some((lo, hi)) if lo <= ANCHOR_BAND_HZ.0 && hi >= ANCHOR_BAND_HZ.1 => ANCHOR_BAND_HZ,
            Some(p) => p,
            None => {
                return Err(ChainRefusal::Band(
                    "the anchor output has no passband".into(),
                ));
            }
        },
    };
    if !(band.0.is_finite() && band.1.is_finite() && band.0 > 0.0 && band.1 > band.0) {
        return Err(ChainRefusal::Band(format!("{band:?}")));
    }
    // The magnitudes, point by point where every run trusts it.
    let grid = first.grid;
    if runs.iter().any(|r| r.anchor_response.grid != grid) {
        return Err(MeasureError::Invalid("anchor responses on different grids".into()).into());
    }
    let freqs = grid.freqs()?;
    let mut worst = (0.0f64, 0.0f64);
    let mut compared = 0usize;
    for (i, &f) in freqs.iter().enumerate() {
        if f < band.0 || f > band.1 {
            continue;
        }
        let vals: Option<Vec<f64>> = runs
            .iter()
            .map(|r| {
                let resp = r.anchor_response;
                let trusted = resp
                    .valid
                    .as_ref()
                    .is_none_or(|v| v.get(i).copied().unwrap_or(false));
                resp.mag_db
                    .get(i)
                    .copied()
                    .filter(|m| trusted && m.is_finite())
            })
            .collect();
        let Some(vals) = vals else { continue };
        let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        compared = compared.saturating_add(1);
        if hi - lo > worst.0 {
            worst = (hi - lo, f);
        }
    }
    if compared == 0 {
        return Err(ChainRefusal::Band(
            "no point of the band is trusted in every run".into(),
        ));
    }
    if worst.0 > options.max_spread_db {
        return Err(ChainRefusal::Moved {
            spread_db: worst.0,
            freq_hz: worst.1,
        });
    }
    // The arrivals on the timeline: one line through (t_ref, intercept).
    let anchor_arrival_us = if options.one_capture_clock && runs.len() >= 3 {
        let pts: Vec<(f64, f64, f64)> = runs
            .iter()
            .zip(&anchors)
            .map(|(r, a)| {
                (
                    r.analysis.t_ref as f64,
                    a.intercept_frame,
                    f64::from(r.analysis.walked.capture_rate_hz),
                )
            })
            .collect();
        let n = pts.len() as f64;
        let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
        let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
        let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
        let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
        let us = pts
            .iter()
            .map(|p| ((p.1 - my - b * (p.0 - mx)) * 1e6 / p.2).abs())
            .fold(0.0, f64::max);
        if !us.is_finite() {
            return Err(MeasureError::Invalid("non-finite anchor arrival".into()).into());
        }
        if us > options.max_arrival_us {
            return Err(ChainRefusal::ArrivalMoved { us });
        }
        Some(us)
    } else {
        None
    };
    let mut outputs = Vec::new();
    for (i, (r, a)) in runs.iter().zip(&anchors).enumerate() {
        let an = r.analysis;
        for o in &an.outputs {
            let Some(&m) = o.members.first() else {
                continue;
            };
            if o.members.len() != 1 {
                continue;
            }
            let is_anchor = m.unit == anchor.unit && m.output == anchor.output;
            if is_anchor && i > 0 {
                continue;
            }
            let u = if is_anchor {
                Some(0.0)
            } else {
                an.delays
                    .iter()
                    .find(|d| {
                        (d.from == m && d.to.unit == anchor.unit && d.to.output == anchor.output)
                            || (d.to == m
                                && d.from.unit == anchor.unit
                                && d.from.output == anchor.output)
                    })
                    .map_or_else(
                        || match (o.uncertainty_us, a.uncertainty_us) {
                            (Some(x), Some(y)) => Some((x * x + y * y).sqrt()),
                            _ => None,
                        },
                        |d| d.uncertainty_us,
                    )
            };
            let other_unit = m.unit != anchor.unit;
            let offered = an.timing_reliable
                && (!other_unit || u.is_some_and(|u| u <= MAX_OFFERED_UNCERTAINTY_US));
            outputs.push(ChainedOutput {
                run: i,
                output: m,
                relative_ms: if is_anchor {
                    0.0
                } else {
                    o.arrival_ms - a.arrival_ms
                },
                uncertainty_us: u,
                offered,
            });
        }
    }
    Ok(Chain {
        anchor,
        band_hz: band,
        anchor_spread_db: worst.0,
        anchor_arrival_us,
        outputs,
    })
}
