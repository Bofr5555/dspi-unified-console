//! Interleaved passes (WP ATSYS, PLAN-AT OD-ATj, reviews R6a and R8b):
//! pass 1 is every unit's walk over its group outputs in turn, then pass 2,
//! ...; each a segment of `repeat` 1 on the unit's one staged config, back
//! to back on the timeline with the gap between them, and no unit's next
//! segment nearer than the lead after its previous one ends (the hub keeps
//! one START pending per unit and writes the next once the previous segment
//! has started).

use crate::ir::MAX_CAPTURE_SAMPLES;
use crate::stimulus::WalkSchedule;
use crate::types::MeasureError;

use super::analysis::MAX_RUN_OUTPUTS;
use super::{DomainId, MAX_SEGMENTS, Segment, SystemSchedule, UnitIndex};

/// The longest run, s (P5's S7).
pub const MAX_RUN_SECONDS: f64 = 120.0;
/// The fewest passes of a system run.
pub const MIN_PASSES: u16 = 2;
/// The most passes of a system run.
pub const MAX_PASSES: u16 = 8;
/// Passes by default when the run fits one capture (review R8b).
pub const DEFAULT_PASSES: u16 = 5;
/// Passes by default when five do not fit.
pub const FALLBACK_PASSES: u16 = 3;

/// One unit's walk in a system run.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitWalk {
    /// The unit.
    pub unit: UnitIndex,
    /// The domain its time-base entry is on (all must be one).
    pub domain: DomainId,
    /// Its walk over its group outputs, one pass (`repeat` 1: its one staged
    /// config).
    pub walk: WalkSchedule,
}

/// How [`interleave`] lays out a run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterleaveOptions {
    /// Passes, [`MIN_PASSES`] to [`MAX_PASSES`]; `None`: [`DEFAULT_PASSES`]
    /// where the run fits one capture, else [`FALLBACK_PASSES`].
    pub passes: Option<u16>,
    /// Silence between one segment's end and the next one's start, domain
    /// samples (at least the sweep's gap).
    pub gap_samples: u64,
    /// The least time between a unit's segment end and its next start,
    /// domain samples (`max(arm_lead) + 100 ms` and the write time).
    pub lead_samples: u64,
    /// The capture's rate, Hz (for the capture bound).
    pub capture_rate_hz: u32,
    /// The capture's pre-roll, ms.
    pub pre_roll_ms: u32,
}

/// The segments of `units`' interleaved passes, from domain sample 0 (the
/// hub's `system.measure.run` adds its `T0`; [`SystemSchedule::offset_by`]).
///
/// Errors: [`MeasureError::Invalid`] for no unit, a unit twice, two domains
/// ("these units do not share a clock"), a walk of more than one pass, walks
/// at different rates or with different sweeps, more than
/// [`MAX_RUN_OUTPUTS`] outputs in all, passes outside 2..=8, a gap
/// shorter than the sweep's, or a run that does not fit one capture
/// ([`MAX_CAPTURE_SAMPLES`], [`MAX_RUN_SECONDS`]) with the passes asked for
/// (or with three).
pub fn interleave(
    units: &[UnitWalk],
    options: &InterleaveOptions,
) -> Result<SystemSchedule, MeasureError> {
    let bad = |m: String| Err(MeasureError::Invalid(m));
    let Some(first) = units.first() else {
        return bad("no unit".into());
    };
    for (i, u) in units.iter().enumerate() {
        if units
            .iter()
            .skip(i.saturating_add(1))
            .any(|v| v.unit == u.unit)
        {
            return bad(format!("unit {} twice", u.unit.0));
        }
        if u.domain != first.domain {
            return bad(format!(
                "units {} and {} are on different time bases: these units do not share a clock",
                first.unit.0, u.unit.0
            ));
        }
        if u.walk.repeat != 1 {
            return bad(format!("unit {}: a walk of one pass is staged", u.unit.0));
        }
        if u.walk.sweep != first.walk.sweep {
            return bad("every unit of a run plays the same sweep".into());
        }
    }
    // -- review AT-A/AT-B -- a run the analyser cannot lay out is refused
    // here, before anything plays.
    let keys: usize = units.iter().map(|u| u.walk.outputs().len()).sum();
    if keys > MAX_RUN_OUTPUTS {
        return bad(format!(
            "{keys} outputs in one run (at most {MAX_RUN_OUTPUTS}): split the group into runs through an anchor output"
        ));
    }
    if options.gap_samples < u64::from(first.walk.sweep.gap_samples) {
        return bad("a gap between segments shorter than the sweep's".into());
    }
    if options.capture_rate_hz == 0 {
        return bad("capture rate 0".into());
    }
    let build = |passes: u16| -> Result<SystemSchedule, MeasureError> {
        let mut segments = Vec::new();
        let mut prev_end: Option<u64> = None;
        let mut unit_end: Vec<(UnitIndex, u64)> = Vec::new();
        for _ in 0..passes {
            for u in units {
                let mut start = prev_end.map_or(0, |e| e.saturating_add(options.gap_samples));
                if let Some(&(_, e)) = unit_end.iter().find(|(x, _)| *x == u.unit) {
                    start = start.max(e.saturating_add(options.lead_samples));
                }
                let seg = Segment {
                    unit: u.unit,
                    start,
                    schedule: u.walk.clone(),
                };
                let end = seg.end();
                prev_end = Some(end);
                match unit_end.iter_mut().find(|(x, _)| *x == u.unit) {
                    Some(slot) => slot.1 = end,
                    None => unit_end.push((u.unit, end)),
                }
                segments.push(seg);
                if segments.len() > MAX_SEGMENTS {
                    return Err(MeasureError::Invalid(format!(
                        "more than {MAX_SEGMENTS} segments"
                    )));
                }
            }
        }
        SystemSchedule::new(first.domain, first.walk.sweep.rate_hz, segments)
    };
    let fits = |s: &SystemSchedule| {
        let rate = f64::from(first.walk.sweep.rate_hz);
        let dev = s
            .end()
            .saturating_add(u64::from(first.walk.sweep.gap_samples)) as f64;
        let secs = dev / rate;
        let frames = (dev * f64::from(options.capture_rate_hz) / rate).ceil()
            + f64::from(options.capture_rate_hz) * f64::from(options.pre_roll_ms) / 1000.0;
        secs <= MAX_RUN_SECONDS && frames <= MAX_CAPTURE_SAMPLES as f64
    };
    let too_long = || {
        Err(MeasureError::Invalid(
            "the run does not fit one capture: split the group into runs through an anchor output"
                .into(),
        ))
    };
    match options.passes {
        Some(p) => {
            if !(MIN_PASSES..=MAX_PASSES).contains(&p) {
                return bad(format!("{p} passes (2 to 8)"));
            }
            let s = build(p)?;
            if fits(&s) { Ok(s) } else { too_long() }
        }
        None => {
            let s = build(DEFAULT_PASSES)?;
            if fits(&s) {
                return Ok(s);
            }
            let s = build(FALLBACK_PASSES)?;
            if fits(&s) { Ok(s) } else { too_long() }
        }
    }
}

/// The hub's and the planner's rule (R6a, `bad_schedule`): no unit's segment
/// starts less than `lead_samples` after its previous segment ends.
///
/// Errors: [`MeasureError::Invalid`] naming the unit and segment.
pub fn check_lead(schedule: &SystemSchedule, lead_samples: u64) -> Result<(), MeasureError> {
    let mut unit_end: Vec<(UnitIndex, u64)> = Vec::new();
    for (i, s) in schedule.segments().iter().enumerate() {
        if let Some(&(_, e)) = unit_end.iter().find(|(x, _)| *x == s.unit)
            && s.start < e.saturating_add(lead_samples)
        {
            return Err(MeasureError::Invalid(format!(
                "segment {i}: unit {} starts {} samples after its previous segment, less than the lead of {lead_samples}",
                s.unit.0,
                s.start.saturating_sub(e)
            )));
        }
        match unit_end.iter_mut().find(|(x, _)| *x == s.unit) {
            Some(slot) => slot.1 = s.end(),
            None => unit_end.push((s.unit, s.end())),
        }
    }
    Ok(())
}
