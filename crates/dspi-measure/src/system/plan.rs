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
    interleave_late(units, options, &[])
}

// -- WP ATLATE --
/// [`interleave`] for units whose sound is expected `lateness` domain
/// samples after their STARTs, more for some units than for others (see
/// [`SystemSchedule::with_lateness`]): a segment that follows one of a unit
/// heard later starts that much later again, so every cycle is *heard* at
/// least one sweep and its gap after the one before (a unit heard 100 ms
/// late would otherwise be heard over a third of the next unit's gap). The
/// schedule carries the lateness for the analysis. With no lateness (or
/// every unit's the same) the schedule is [`interleave`]'s.
///
/// Errors: [`interleave`]'s, and [`SystemSchedule::with_lateness`]'s.
pub fn interleave_late(
    units: &[UnitWalk],
    options: &InterleaveOptions,
    lateness: &[(UnitIndex, u64)],
) -> Result<SystemSchedule, MeasureError> {
    let bad = |m: String| Err(MeasureError::Invalid(m));
    // Checked (and made relative) before anything is laid out.
    let run_units: Vec<UnitIndex> = units.iter().map(|u| u.unit).collect();
    let rate = units.first().map_or(0, |u| u.walk.sweep.rate_hz);
    let lateness = super::relative_lateness(&run_units, lateness, rate)?;
    let late_of = |u: UnitIndex| {
        lateness
            .iter()
            .find(|(x, _)| *x == u)
            .map_or(0, |(_, l)| *l)
    };
    // -- WP ATOVL -- (the checks moved to `check_walks`, unchanged)
    let first = check_walks(units, options)?;
    let build = |passes: u16| -> Result<SystemSchedule, MeasureError> {
        let mut segments = Vec::new();
        let mut prev_end: Option<u64> = None;
        // -- WP ATLATE -- the lateness of the segment before.
        let mut prev_late = 0u64;
        let mut unit_end: Vec<(UnitIndex, u64)> = Vec::new();
        for _ in 0..passes {
            for u in units {
                let late = late_of(u.unit);
                let mut start = prev_end.map_or(0, |e| {
                    e.saturating_add(options.gap_samples)
                        .saturating_add(prev_late.saturating_sub(late))
                });
                prev_late = late;
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
        SystemSchedule::new(first.domain, first.walk.sweep.rate_hz, segments)?
            .with_lateness(&lateness)
    };
    let fits = |s: &SystemSchedule| run_fits(s, options, 0);
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

// -- WP ATOVL --
/// The walks' checks of [`interleave_late`] and [`overlap_late`]: some unit,
/// each once, one domain, one pass each, one sweep, at most
/// [`MAX_RUN_OUTPUTS`] outputs, a gap at least the sweep's, a capture rate.
/// The first walk.
fn check_walks<'a>(
    units: &'a [UnitWalk],
    options: &InterleaveOptions,
) -> Result<&'a UnitWalk, MeasureError> {
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
    Ok(first)
}

// -- WP ATOVL --
/// Whether `s` fits one capture with `slack` domain samples more after its
/// end ([`MAX_RUN_SECONDS`], [`MAX_CAPTURE_SAMPLES`] with the pre-roll), as
/// [`interleave`] judges it (`slack` 0).
pub fn run_fits(s: &SystemSchedule, options: &InterleaveOptions, slack: u64) -> bool {
    let Some(first) = s.segments().first() else {
        return false;
    };
    let rate = f64::from(first.schedule.sweep.rate_hz.max(1));
    let dev = s
        .end()
        .saturating_add(u64::from(first.schedule.sweep.gap_samples))
        .saturating_add(slack) as f64;
    let secs = dev / rate;
    let frames = (dev * f64::from(options.capture_rate_hz) / rate).ceil()
        + f64::from(options.capture_rate_hz) * f64::from(options.pre_roll_ms) / 1000.0;
    secs <= MAX_RUN_SECONDS && frames <= MAX_CAPTURE_SAMPLES as f64
}

// -- WP ATOVL -- overlapped starts (owner 2026-11-19, PLAN-AT 1.4).

/// The least offset between the units of an overlapped group, ms.
pub const OVERLAP_MIN_MS: u32 = 50;
/// The largest, ms.
pub const OVERLAP_MAX_MS: u32 = 200;
/// What an overlapped run's analysis keeps before each arrival, and how far
/// either side of its place it looks for each cycle, ms (P5 keeps
/// `search_ms`, 100 ms).
pub const OVERLAP_PRE_MS: f64 = 20.0;
/// How far apart the units of a group may be heard beyond their plan (the
/// acoustic paths and play-out not planned for), ms.
pub const OVERLAP_SPREAD_MS: f64 = 15.0;
/// Clear of another unit's impulse and of every harmonic image around each
/// window, ms.
pub const OVERLAP_GUARD_MS: f64 = 5.0;
/// The harmonic images kept clear: orders 2 to this.
pub const OVERLAP_HARMONICS: u32 = 9;
/// The shortest window an overlapped run keeps after the timing origin,
/// ms.
pub const OVERLAP_MIN_TAIL_MS: f64 = 25.0;

/// The windows an overlapped run's analysis keeps with units `offset`
/// domain samples apart: `(before, after)` the timing origin, ms; after is
/// the offset less twice [`OVERLAP_GUARD_MS`] (so the next unit's impulse
/// stays a guard clear of it), never more than P5 keeps (the gap's rest).
pub fn overlap_window_ms(sweep: &crate::stimulus::LogSweep, offset: u64) -> (f64, f64) {
    let rate = f64::from(sweep.rate_hz.max(1));
    let gap_ms = f64::from(sweep.gap_samples) * 1000.0 / rate;
    let p5_pre = 100.0f64.min(gap_ms / 3.0);
    let p5_tail = (gap_ms - 2.0 * p5_pre).max(0.0);
    let tail = (offset as f64 * 1000.0 / rate - 2.0 * OVERLAP_GUARD_MS).min(p5_tail);
    (OVERLAP_PRE_MS, tail)
}

/// Why `units` units of `sweep` cannot start `offset` domain samples apart
/// with a gap of `gap_samples` between a unit's sweeps; `None` when they
/// can: every unit's impulse and harmonic images (orders 2 to
/// [`OVERLAP_HARMONICS`], allowing [`OVERLAP_SPREAD_MS`] of spread) stay
/// [`OVERLAP_GUARD_MS`] clear of every other unit's analysed window, the
/// window is at least [`OVERLAP_MIN_TAIL_MS`], and a group's cuts stay
/// inside the gap.
pub fn overlap_conflict(
    sweep: &crate::stimulus::LogSweep,
    units: usize,
    offset: u64,
    gap_samples: u64,
) -> Option<String> {
    if units < 2 {
        return Some("overlapped starts need two units or more".into());
    }
    let rate = f64::from(sweep.rate_hz.max(1));
    let ms = |samples: f64| samples * 1000.0 / rate;
    let d_ms = ms(offset as f64);
    let (pre, tail) = overlap_window_ms(sweep, offset);
    let (spread, guard) = (OVERLAP_SPREAD_MS, OVERLAP_GUARD_MS);
    if !(f64::from(OVERLAP_MIN_MS) - 1e-9..=f64::from(OVERLAP_MAX_MS) + 1e-9).contains(&d_ms) {
        return Some(format!(
            "an offset of {d_ms:.1} ms ({OVERLAP_MIN_MS} to {OVERLAP_MAX_MS} ms)"
        ));
    }
    if tail < OVERLAP_MIN_TAIL_MS {
        return Some(format!(
            "the window after each arrival would be {tail:.1} ms (at least {OVERLAP_MIN_TAIL_MS} ms)"
        ));
    }
    if d_ms <= pre + spread + guard {
        return Some(format!(
            "{d_ms:.1} ms apart, a unit's window would hold the unit before it"
        ));
    }
    let gap_ms = ms(gap_samples as f64);
    let span = (units.saturating_sub(1)) as f64 * d_ms;
    let need = span + spread + pre.max(tail) + 2.0 * guard;
    if gap_ms < need {
        return Some(format!(
            "{units} units {d_ms:.0} ms apart need gaps of at least {need:.0} ms between sweeps \
             (the gap is {gap_ms:.0} ms)"
        ));
    }
    let ratio = sweep.end_freq_hz() / sweep.start_freq_hz();
    let t = f64::from(sweep.cycle_samples);
    for m in 1..units {
        let d = m as f64 * d_ms;
        if d < tail + guard {
            return Some(format!(
                "a unit {d:.0} ms later would be heard inside the window before it"
            ));
        }
        for k in 2..=OVERLAP_HARMONICS {
            let h = ms(t * f64::from(k).ln() / ratio.ln());
            let (lo, hi) = (d - h, d - h + spread);
            if hi > -pre - guard && lo < tail + guard {
                return Some(format!(
                    "the harmonic image {k} of a unit {d:.0} ms later would fall {lo:.0} ms from \
                     an arrival, inside its window"
                ));
            }
        }
    }
    None
}

/// The smallest offset from [`OVERLAP_MIN_MS`] to [`OVERLAP_MAX_MS`] (1 ms
/// steps) at which `units` units of `sweep` can start apart
/// ([`overlap_conflict`]), domain samples.
///
/// Errors: [`MeasureError::Invalid`] with why none can (the reason at the
/// smallest offset).
pub fn overlap_offset(
    sweep: &crate::stimulus::LogSweep,
    units: usize,
    gap_samples: u64,
) -> Result<u64, MeasureError> {
    let rate = u64::from(sweep.rate_hz);
    let mut first_reason: Option<String> = None;
    for d_ms in OVERLAP_MIN_MS..=OVERLAP_MAX_MS {
        let offset = rate.saturating_mul(u64::from(d_ms)) / 1000;
        match overlap_conflict(sweep, units, offset, gap_samples) {
            None => return Ok(offset),
            Some(why) => {
                first_reason.get_or_insert(why);
            }
        }
    }
    Err(MeasureError::Invalid(format!(
        "overlapped starts do not fit: {}",
        first_reason.unwrap_or_default()
    )))
}

/// A walked run with **overlapped starts** (owner 2026-11-19, PLAN-AT 1.4):
/// pass 1 is interleaved (each unit's walk alone, so the first cycle and
/// every unit's own are told apart as in [`interleave`]); then every pass
/// plays every unit's walk together, unit `i` heard `i` times the offset
/// ([`overlap_offset`]) after the first, so the capture clock's wander is
/// common to what plays together and one deconvolution of each group
/// measures their differences. The starts leave room for each unit's
/// expected lateness ([`SystemSchedule::with_lateness`]) so that the units
/// are *heard* the offset apart, and keep the lead between a unit's own
/// segments. Passes: `options.passes` overlapped passes after the first
/// (default [`DEFAULT_PASSES`] where the run fits one capture, else
/// [`FALLBACK_PASSES`]).
///
/// Errors: [`interleave`]'s, an offset that cannot keep the units apart
/// (`overlap_offset`), more than [`MAX_SEGMENTS`] segments.
pub fn overlap_late(
    units: &[UnitWalk],
    options: &InterleaveOptions,
    lateness: &[(UnitIndex, u64)],
) -> Result<SystemSchedule, MeasureError> {
    let bad = |m: String| Err(MeasureError::Invalid(m));
    let run_units: Vec<UnitIndex> = units.iter().map(|u| u.unit).collect();
    let rate = units.first().map_or(0, |u| u.walk.sweep.rate_hz);
    let lateness = super::relative_lateness(&run_units, lateness, rate)?;
    let late_of = |u: UnitIndex| {
        lateness
            .iter()
            .find(|(x, _)| *x == u)
            .map_or(0, |(_, l)| *l)
    };
    let first = check_walks(units, options)?;
    let sweep = first.walk.sweep;
    let offset = overlap_offset(&sweep, units.len(), options.gap_samples)?;
    let most_late = lateness.iter().map(|(_, l)| *l).max().unwrap_or(0);
    let build = |passes: u16| -> Result<SystemSchedule, MeasureError> {
        // Where each segment is heard (its start plus its unit's lateness).
        let mut heard: Vec<(UnitIndex, u64, &WalkSchedule)> = Vec::new();
        let mut unit_end: Vec<(UnitIndex, u64)> = Vec::new();
        let mut h = 0u64;
        for u in units {
            heard.push((u.unit, h, &u.walk));
            let end = h.saturating_add(u.walk.total_samples());
            unit_end.push((u.unit, end));
            h = end.saturating_add(options.gap_samples);
        }
        for _ in 0..passes {
            let mut base = h;
            for (rank, u) in units.iter().enumerate() {
                if let Some(&(_, e)) = unit_end.iter().find(|(x, _)| *x == u.unit) {
                    let at = offset.saturating_mul(rank as u64);
                    base = base.max(e.saturating_add(options.lead_samples).saturating_sub(at));
                }
            }
            let mut last = base;
            for (rank, u) in units.iter().enumerate() {
                let at = base.saturating_add(offset.saturating_mul(rank as u64));
                heard.push((u.unit, at, &u.walk));
                let end = at.saturating_add(u.walk.total_samples());
                if let Some(slot) = unit_end.iter_mut().find(|(x, _)| *x == u.unit) {
                    slot.1 = end;
                }
                last = last.max(end);
            }
            h = last.saturating_add(options.gap_samples);
        }
        let mut segments: Vec<Segment> = heard
            .into_iter()
            .map(|(unit, at, walk)| Segment {
                unit,
                start: at.saturating_add(most_late).saturating_sub(late_of(unit)),
                schedule: walk.clone(),
            })
            .collect();
        if segments.len() > MAX_SEGMENTS {
            return Err(MeasureError::Invalid(format!(
                "more than {MAX_SEGMENTS} segments"
            )));
        }
        segments.sort_by_key(|s| s.start);
        let s = SystemSchedule::overlapped(first.domain, sweep.rate_hz, segments, offset)?
            .with_lateness(&lateness)?;
        // The analysis must be able to lay it out (refused here, before
        // anything plays).
        super::analysis::overlap_layout_check(&s)?;
        Ok(s)
    };
    let fits = |s: &SystemSchedule| run_fits(s, options, 0);
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
