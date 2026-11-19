//! Several units on one time base (PLAN-AT, `docs/core-merge/PLAN-AT.md`
//! section 1.5): the seam that WP ATSYS (the analyser over segments) and WP
//! ATSIM (the multi-unit simulated world) share. Plain data and its
//! validation only; no analysis here.
//!
//! A *system schedule* is a list of *segments* on one clock domain's time
//! base: each segment is one unit's own walked generator run, started by apply
//! op 9 (`SIGGEN_START`) on the domain's sample `start`. A single unit is the
//! one-segment case. Every position is in the domain's numbering (`T`): the
//! hub maps a USB unit's own S onto it (PLAN-AT OD-ATg); line and S/PDIF
//! units share F already.

use crate::stimulus::WalkSchedule;
use crate::types::MeasureError;

// -- WP PLANAT (review R9e) -- the hub family `system.measure.*`: the request,
// reply and event types ATHUB serves and ATCORE sends.
pub mod messages;

/// The most units one schedule spans.
pub const MAX_UNITS: usize = 16;
/// The most segments one schedule holds (passes 1 to 8 over up to 16 units).
pub const MAX_SEGMENTS: usize = 128;
// -- WP ATSUMOFF --
/// The most a summed run's parts may start apart, ms
/// ([`SystemSchedule::summed_at`]; the hub's `system.measure.run` with
/// `summed` refuses a wider spread). A part starts later by the difference
/// of the predicted arrivals it brings together: the units' play-out and
/// output delays, tens of milliseconds on USB-direct.
pub const MAX_SUM_SPREAD_MS: u32 = 1_000;
// -- WP ATLATE --
/// The largest expected lateness of a unit over a run's earliest unit that a
/// walked run plans for, ms ([`SystemSchedule::with_lateness`]). A unit
/// later than this (a play-out or output delay of a second) is a model or
/// alignment fault, not something to measure around.
pub const MAX_LATENESS_MS: u32 = 1_000;

/// [`MAX_SUM_SPREAD_MS`] in samples at `rate_hz`.
pub fn max_sum_spread_samples(rate_hz: u32) -> u64 {
    u64::from(rate_hz)
        .saturating_mul(u64::from(MAX_SUM_SPREAD_MS))
        .saturating_div(1000)
}

/// A unit of the alignment group: an index into the group's unit list (the
/// session keeps the serials).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitIndex(pub u8);

/// A clock domain: one time base numbering shared by every unit of the group
/// (`EXT_TIME_BASE` entry: `time_base` and `domain`; for USB the hub's bus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DomainId {
    /// The time base (`dspi_proto::ext::timebase::TB_*`).
    pub time_base: u8,
    /// The domain id within it.
    pub id: u32,
}

/// One unit's generator run on the time base.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The unit that plays it.
    pub unit: UnitIndex,
    /// The domain sample `T` of the generator's sample 0.
    pub start: u64,
    /// What it plays: the unit's walk over its outputs in the group.
    pub schedule: WalkSchedule,
}

impl Segment {
    /// The domain sample after the segment's last sample (no gap after the
    /// last cycle, as the firmware stops at once).
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.schedule.total_samples())
    }
}

/// Where one domain sample was heard in the capture: a frame-counter read
/// noted against the capture frame the client had reached when it arrived.
/// Coarse (the read's round trip); it pins cycle identity, not the fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimePin {
    /// The capture's frame index when the reply arrived.
    pub capture_frame: u64,
    /// The domain sample the reply reported.
    pub t: u64,
    /// The capture frames between sending the request and the reply's
    /// arrival: the pin's uncertainty. The analyser uses the shortest.
    pub round_trip_frames: u64,
}

/// An output repeated in every run of a chained measurement, so that runs
/// longer than one capture can be compared (PLAN-AT OD-ATk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorOutput {
    /// Its unit.
    pub unit: UnitIndex,
    /// Its output.
    pub output: u8,
}

/// The segments of one run on one time base.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemSchedule {
    /// The domain every segment is on.
    pub domain: DomainId,
    /// The domain's sample rate, Hz (every segment's sweep is at it).
    pub rate_hz: u32,
    segments: Vec<Segment>,
    summed: bool,
    // -- WP ATLATE --
    /// Each unit's expected lateness over the run's earliest, domain samples
    /// ([`SystemSchedule::with_lateness`]); none: every unit 0.
    lateness: Vec<(UnitIndex, u64)>,
    // -- WP ATOVL --
    /// Overlapped starts ([`SystemSchedule::overlapped`]): the offset
    /// between the units of a group, domain samples.
    overlap: Option<u64>,
}

impl SystemSchedule {
    /// A schedule of `segments`, in time order, none overlapping another (one
    /// generator plays at a time across the group).
    ///
    /// Errors: [`MeasureError::Invalid`] for no segment, more than
    /// [`MAX_SEGMENTS`], a unit index at or past [`MAX_UNITS`], a sweep at
    /// another rate than `rate_hz`, or segments out of order or overlapping.
    pub fn new(
        domain: DomainId,
        rate_hz: u32,
        segments: Vec<Segment>,
    ) -> Result<SystemSchedule, MeasureError> {
        if segments.is_empty() {
            return Err(MeasureError::Invalid("no segment".into()));
        }
        if segments.len() > MAX_SEGMENTS {
            return Err(MeasureError::Invalid(format!(
                "{} segments (at most {MAX_SEGMENTS})",
                segments.len()
            )));
        }
        let mut prev_end: Option<u64> = None;
        for s in &segments {
            if usize::from(s.unit.0) >= MAX_UNITS {
                return Err(MeasureError::Invalid(format!(
                    "unit {} (at most {MAX_UNITS})",
                    s.unit.0
                )));
            }
            if s.schedule.sweep.rate_hz != rate_hz {
                return Err(MeasureError::Invalid(format!(
                    "a segment at {} Hz in a {rate_hz} Hz domain",
                    s.schedule.sweep.rate_hz
                )));
            }
            if let Some(end) = prev_end
                && s.start < end
            {
                return Err(MeasureError::Invalid(
                    "segments out of order or overlapping".into(),
                ));
            }
            prev_end = Some(s.end());
        }
        Ok(SystemSchedule {
            domain,
            rate_hz,
            segments,
            summed: false,
            lateness: Vec::new(), // -- WP ATLATE --
            overlap: None,        // -- WP ATOVL --
        })
    }

    /// A summed run (RC5f across units): every segment starts at the same
    /// `start`, each on a different unit, so their outputs play together
    /// (see [`SystemSchedule::summed_at`] for parts that start apart).
    ///
    /// Errors: [`MeasureError::Invalid`] for fewer than two segments, more than
    /// [`MAX_UNITS`], a unit twice or past [`MAX_UNITS`], or a sweep at another
    /// rate than `rate_hz`.
    pub fn summed(
        domain: DomainId,
        rate_hz: u32,
        start: u64,
        parts: Vec<(UnitIndex, WalkSchedule)>,
    ) -> Result<SystemSchedule, MeasureError> {
        SystemSchedule::summed_at(
            domain,
            rate_hz,
            start,
            parts.into_iter().map(|(u, w)| (u, 0, w)).collect(),
        )
    }

    // -- WP ATSUMOFF --
    /// A summed run whose parts start apart: part `(unit, offset, walk)`
    /// starts at `start + offset`, so that the outputs it brings together
    /// sound together although their units' play-out and output delays
    /// differ (on USB-direct the streams' start offsets put tens of
    /// milliseconds into `out.delay`). The segments are kept in time order
    /// (by offset; equal offsets in the given order); the analysis takes the
    /// earliest start as its reference, so a later part's response is its
    /// offset later in the sum. [`SystemSchedule::summed`] is the case of
    /// every offset 0.
    ///
    /// Errors: as [`SystemSchedule::summed`], plus an offset over
    /// [`MAX_SUM_SPREAD_MS`], no part at offset 0, or a start past the
    /// counter.
    pub fn summed_at(
        domain: DomainId,
        rate_hz: u32,
        start: u64,
        parts: Vec<(UnitIndex, u64, WalkSchedule)>,
    ) -> Result<SystemSchedule, MeasureError> {
        if parts.len() < 2 || parts.len() > MAX_UNITS {
            return Err(MeasureError::Invalid(format!(
                "{} units in a summed run (2 to {MAX_UNITS})",
                parts.len()
            )));
        }
        let spread = max_sum_spread_samples(rate_hz);
        let mut seen: Vec<UnitIndex> = Vec::with_capacity(parts.len());
        let mut segments = Vec::with_capacity(parts.len());
        for (unit, offset, schedule) in parts {
            if usize::from(unit.0) >= MAX_UNITS || seen.contains(&unit) {
                return Err(MeasureError::Invalid(format!(
                    "unit {} twice or out of range",
                    unit.0
                )));
            }
            if schedule.sweep.rate_hz != rate_hz {
                return Err(MeasureError::Invalid(format!(
                    "a segment at {} Hz in a {rate_hz} Hz domain",
                    schedule.sweep.rate_hz
                )));
            }
            if offset > spread {
                return Err(MeasureError::Invalid(format!(
                    "a summed run's part {offset} samples after its first (at most {MAX_SUM_SPREAD_MS} ms)"
                )));
            }
            let start = start
                .checked_add(offset)
                .filter(|s| s.checked_add(schedule.total_samples()).is_some())
                .ok_or_else(|| MeasureError::Invalid("a start past the counter".into()))?;
            seen.push(unit);
            segments.push(Segment {
                unit,
                start,
                schedule,
            });
        }
        if segments.iter().all(|s| s.start != start) {
            return Err(MeasureError::Invalid(
                "no part of the summed run starts at its start".into(),
            ));
        }
        // Stable: equal offsets keep the given order.
        segments.sort_by_key(|s| s.start);
        Ok(SystemSchedule {
            domain,
            rate_hz,
            segments,
            summed: true,
            lateness: Vec::new(), // -- WP ATLATE --
            overlap: None,        // -- WP ATOVL --
        })
    }

    /// A summed run: every segment plays together (from one start, or from
    /// the offsets of [`SystemSchedule::summed_at`]).
    pub fn is_summed(&self) -> bool {
        self.summed
    }

    // -- WP ATOVL --
    /// A walked run with **overlapped starts** (PLAN-AT 1.4): the units'
    /// segments may overlap each other (never a unit's own), each unit's
    /// next segment at least the lead after its previous one (the hub's
    /// rule), in time order of their starts. Built by
    /// [`plan::overlap_late`], which lays the units' sweeps of a group
    /// `offset` apart where they are heard and checks that the analysis can
    /// tell them apart.
    ///
    /// Errors: [`MeasureError::Invalid`] for no segment or fewer than two
    /// units, more than [`MAX_SEGMENTS`], a unit index at or past
    /// [`MAX_UNITS`], a sweep at another rate, starts out of order, a
    /// unit's segments overlapping, or an offset outside
    /// [`plan::OVERLAP_MIN_MS`] to [`plan::OVERLAP_MAX_MS`].
    pub fn overlapped(
        domain: DomainId,
        rate_hz: u32,
        segments: Vec<Segment>,
        offset: u64,
    ) -> Result<SystemSchedule, MeasureError> {
        if segments.is_empty() || segments.len() > MAX_SEGMENTS {
            return Err(MeasureError::Invalid(format!(
                "{} segments (1 to {MAX_SEGMENTS})",
                segments.len()
            )));
        }
        let ms = |m: u32| u64::from(rate_hz).saturating_mul(u64::from(m)) / 1000;
        if offset < ms(plan::OVERLAP_MIN_MS) || offset > ms(plan::OVERLAP_MAX_MS) {
            return Err(MeasureError::Invalid(format!(
                "an overlap of {offset} samples ({} to {} ms)",
                plan::OVERLAP_MIN_MS,
                plan::OVERLAP_MAX_MS
            )));
        }
        let mut prev: Option<u64> = None;
        let mut unit_end: Vec<(UnitIndex, u64)> = Vec::new();
        for s in &segments {
            if usize::from(s.unit.0) >= MAX_UNITS {
                return Err(MeasureError::Invalid(format!(
                    "unit {} (at most {MAX_UNITS})",
                    s.unit.0
                )));
            }
            if s.schedule.sweep.rate_hz != rate_hz {
                return Err(MeasureError::Invalid(format!(
                    "a segment at {} Hz in a {rate_hz} Hz domain",
                    s.schedule.sweep.rate_hz
                )));
            }
            if prev.is_some_and(|p| s.start < p) {
                return Err(MeasureError::Invalid("segments out of order".into()));
            }
            prev = Some(s.start);
            if let Some(&(_, e)) = unit_end.iter().find(|(u, _)| *u == s.unit)
                && s.start < e
            {
                return Err(MeasureError::Invalid(format!(
                    "unit {}'s segments overlap",
                    s.unit.0
                )));
            }
            if s.start.checked_add(s.schedule.total_samples()).is_none() {
                return Err(MeasureError::Invalid("a segment past the counter".into()));
            }
            match unit_end.iter_mut().find(|(u, _)| *u == s.unit) {
                Some(slot) => slot.1 = s.end(),
                None => unit_end.push((s.unit, s.end())),
            }
        }
        if unit_end.len() < 2 {
            return Err(MeasureError::Invalid(
                "overlapped starts need two units or more".into(),
            ));
        }
        Ok(SystemSchedule {
            domain,
            rate_hz,
            segments,
            summed: false,
            lateness: Vec::new(),
            overlap: Some(offset),
        })
    }

    // -- WP ATOVL --
    /// The offset between the units of a group of an overlapped run, domain
    /// samples; `None` for every other run.
    pub fn overlap_samples(&self) -> Option<u64> {
        self.overlap
    }

    /// The one-segment schedule of a single unit (the existing single-unit
    /// model: its START is `start`).
    pub fn single(
        domain: DomainId,
        unit: UnitIndex,
        start: u64,
        schedule: WalkSchedule,
    ) -> Result<SystemSchedule, MeasureError> {
        let rate_hz = schedule.sweep.rate_hz;
        SystemSchedule::new(
            domain,
            rate_hz,
            vec![Segment {
                unit,
                start,
                schedule,
            }],
        )
    }

    /// The segments, in time order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The first segment's start.
    pub fn start(&self) -> u64 {
        self.segments.first().map_or(0, |s| s.start)
    }

    /// The latest segment end.
    pub fn end(&self) -> u64 {
        self.segments.iter().map(Segment::end).max().unwrap_or(0)
    }

    /// The units that play, ascending, each once.
    pub fn units(&self) -> Vec<UnitIndex> {
        let mut u: Vec<UnitIndex> = self.segments.iter().map(|s| s.unit).collect();
        u.sort_unstable();
        u.dedup();
        u
    }
}

// -- WP ATSYS -- the analyser over segments (PLAN-AT section 1.4): the
// timeline walk and its fit, the uncertainty of every cross-unit delay, the
// gate's facts G1 to G4, the interleaved passes, the anchor chaining.
pub mod analysis;
pub mod chain;
pub mod gate;
pub mod plan;

pub use analysis::{
    ARRIVAL_RESOLUTION_US, CrossUnitDelay, MAX_OFFERED_UNCERTAINTY_US, MAX_RUN_OUTPUTS,
    SystemAnalyser, SystemAnalysis, SystemCapture, SystemOutput, SystemStep, UnitOutput,
    UnitStatusObservation, analyse_system, passband, passband_level_db, transition_band,
};
pub use chain::{Chain, ChainOptions, ChainRefusal, ChainRun, ChainedOutput, chain_runs};
pub use gate::{GateFinding, SegmentProof, UnitFacts, check_gate, check_one_domain};
pub use plan::{InterleaveOptions, UnitWalk, check_lead, interleave};
// -- WP ATLATE --
pub use plan::interleave_late;
// -- WP ATOVL --
pub use plan::{overlap_late, overlap_offset};

impl SystemSchedule {
    /// The same run with every start moved `by` domain samples later (a plan
    /// from [`interleave`] placed at the hub's `T0`).
    ///
    /// Errors: [`MeasureError::Invalid`] when a start would pass `u64::MAX`.
    pub fn offset_by(&self, by: u64) -> Result<SystemSchedule, MeasureError> {
        let mut out = self.clone();
        for s in &mut out.segments {
            s.start = s
                .start
                .checked_add(by)
                .ok_or_else(|| MeasureError::Invalid("a start past the counter".into()))?;
            if s.end() == u64::MAX {
                return Err(MeasureError::Invalid("a segment past the counter".into()));
            }
        }
        Ok(out)
    }
}

// -- WP ATLATE --
impl SystemSchedule {
    /// The same walked run told each unit's **expected lateness**: how much
    /// later than the run's earliest unit its sound is expected after its
    /// START, from what the planner knows before the run (the unit's
    /// play-out of a domain sample and its outputs' `out.delay`; on
    /// USB-direct the streams' start offsets are in `out.delay`, tens of
    /// milliseconds a unit). The analysis looks for each unit's cycles that
    /// much later on the timeline, so a unit heard up to
    /// [`MAX_LATENESS_MS`] after the others is still inside the span it keeps
    /// (100 ms either side of the first arrival with a 300 ms gap) and
    /// measured whole; its reported arrival has the lateness added back, so
    /// every arrival stays on the one timeline. A wrong expectation only
    /// moves where the analysis looks: an arrival more than the search span
    /// from where it was expected is not found (the run fails closed), and
    /// one cut at the span's end is not offered. Lateness is relative: the
    /// smallest of the run's units is taken off, and a unit not named is 0.
    /// [`interleave_late`] plans the starts with the same lateness, so that
    /// no cycle is *heard* closer than one sweep and its gap to the next.
    ///
    /// Errors: [`MeasureError::Invalid`] for a summed run (its parts are
    /// started apart instead, [`SystemSchedule::summed_at`]), a unit twice,
    /// or a lateness spread over [`MAX_LATENESS_MS`].
    pub fn with_lateness(
        mut self,
        lateness: &[(UnitIndex, u64)],
    ) -> Result<SystemSchedule, MeasureError> {
        let rel = relative_lateness(&self.units(), lateness, self.rate_hz)?;
        if !rel.is_empty() && self.summed {
            return Err(MeasureError::Invalid(
                "a summed run starts its parts apart: it takes no lateness".into(),
            ));
        }
        self.lateness = rel;
        Ok(self)
    }

    /// Unit `u`'s expected lateness over the run's earliest, domain samples
    /// (0 unless [`SystemSchedule::with_lateness`] gave it one).
    pub fn lateness_of(&self, u: UnitIndex) -> u64 {
        self.lateness
            .iter()
            .find(|(x, _)| *x == u)
            .map_or(0, |(_, l)| *l)
    }

    /// Every unit's non-zero expected lateness, domain samples.
    pub fn lateness(&self) -> &[(UnitIndex, u64)] {
        &self.lateness
    }
}

/// `lateness` of `units` relative to the smallest (a unit not named: 0),
/// the zeros left out ([`SystemSchedule::with_lateness`]).
///
/// Errors: [`MeasureError::Invalid`] for a unit twice or a spread over
/// [`MAX_LATENESS_MS`] at `rate_hz`.
pub(crate) fn relative_lateness(
    units: &[UnitIndex],
    lateness: &[(UnitIndex, u64)],
    rate_hz: u32,
) -> Result<Vec<(UnitIndex, u64)>, MeasureError> {
    for (i, (u, _)) in lateness.iter().enumerate() {
        if lateness
            .iter()
            .skip(i.saturating_add(1))
            .any(|(x, _)| x == u)
        {
            return Err(MeasureError::Invalid(format!(
                "unit {}'s lateness given twice",
                u.0
            )));
        }
    }
    let of = |u: UnitIndex| {
        lateness
            .iter()
            .find(|(x, _)| *x == u)
            .map_or(0, |(_, l)| *l)
    };
    let min = units.iter().map(|&u| of(u)).min().unwrap_or(0);
    let rel: Vec<(UnitIndex, u64)> = units
        .iter()
        .map(|&u| (u, of(u).saturating_sub(min)))
        .filter(|(_, l)| *l > 0)
        .collect();
    let bound = u64::from(rate_hz).saturating_mul(u64::from(MAX_LATENESS_MS)) / 1000;
    if let Some(&(u, l)) = rel.iter().find(|(_, l)| *l > bound) {
        return Err(MeasureError::Invalid(format!(
            "unit {} is expected {:.1} ms after the run's earliest unit, more than a run plans for \
             ({MAX_LATENESS_MS} ms): check its play-out and output delays",
            u.0,
            l as f64 * 1000.0 / f64::from(rate_hz.max(1))
        )));
    }
    Ok(rel)
}

#[cfg(test)]
mod tests_wp_atsys;

#[cfg(test)]
mod tests_wp_planat {
    use super::*;
    use crate::stimulus::{LogSweep, SweepConfig};

    fn walk(rate: u32, mask: u16) -> WalkSchedule {
        let cfg = SweepConfig::COUPLER;
        let sweep = LogSweep::new(rate, &cfg).unwrap_or_else(|e| panic!("sweep: {e}"));
        WalkSchedule::new(sweep, mask, 1).unwrap_or_else(|e| panic!("walk: {e}"))
    }

    const D: DomainId = DomainId {
        time_base: 0,
        id: 0x42,
    };

    #[test]
    fn wp_planat_segments_in_order_are_accepted() {
        let a = walk(48_000, 0b11);
        let a_end = a.total_samples();
        let b = walk(48_000, 0b1);
        let s = SystemSchedule::new(
            D,
            48_000,
            vec![
                Segment {
                    unit: UnitIndex(0),
                    start: 1_000,
                    schedule: a,
                },
                Segment {
                    unit: UnitIndex(1),
                    start: 1_000 + a_end,
                    schedule: b,
                },
            ],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.units(), vec![UnitIndex(0), UnitIndex(1)]);
        assert_eq!(s.start(), 1_000);
    }

    #[test]
    fn wp_planat_overlap_and_rate_mismatch_are_refused() {
        let a = walk(48_000, 0b11);
        let over = SystemSchedule::new(
            D,
            48_000,
            vec![
                Segment {
                    unit: UnitIndex(0),
                    start: 0,
                    schedule: a.clone(),
                },
                Segment {
                    unit: UnitIndex(1),
                    start: 10,
                    schedule: a.clone(),
                },
            ],
        );
        assert!(over.is_err());
        let rate = SystemSchedule::new(
            D,
            96_000,
            vec![Segment {
                unit: UnitIndex(0),
                start: 0,
                schedule: a,
            }],
        );
        assert!(rate.is_err());
        assert!(SystemSchedule::new(D, 48_000, Vec::new()).is_err());
    }

    #[test]
    fn wp_planat_summed_needs_distinct_units() {
        let a = walk(48_000, 0b1);
        let s = SystemSchedule::summed(
            D,
            48_000,
            500,
            vec![(UnitIndex(0), a.clone()), (UnitIndex(1), a.clone())],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(s.is_summed());
        assert!(s.segments().iter().all(|g| g.start == 500));
        assert!(
            SystemSchedule::summed(
                D,
                48_000,
                0,
                vec![(UnitIndex(0), a.clone()), (UnitIndex(0), a.clone())]
            )
            .is_err()
        );
        assert!(SystemSchedule::summed(D, 48_000, 0, vec![(UnitIndex(0), a)]).is_err());
    }

    // -- WP ATSUMOFF --
    #[test]
    fn wp_atsumoff_summed_parts_may_start_apart_within_the_spread() {
        let a = walk(48_000, 0b1);
        let s = SystemSchedule::summed_at(
            D,
            48_000,
            500,
            vec![
                (UnitIndex(0), 1_382, a.clone()),
                (UnitIndex(1), 0, a.clone()),
                (UnitIndex(2), 0, a.clone()),
            ],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(s.is_summed());
        // In time order, equal offsets as given; the first start is the run's.
        let order: Vec<(u8, u64)> = s.segments().iter().map(|g| (g.unit.0, g.start)).collect();
        assert_eq!(order, vec![(1, 500), (2, 500), (0, 1_882)]);
        assert_eq!(s.start(), 500);
        assert_eq!(s.end(), 1_882 + a.total_samples());
        // Every offset 0 is `summed`.
        let same = SystemSchedule::summed_at(
            D,
            48_000,
            7,
            vec![(UnitIndex(0), 0, a.clone()), (UnitIndex(1), 0, a.clone())],
        );
        let plain = SystemSchedule::summed(
            D,
            48_000,
            7,
            vec![(UnitIndex(0), a.clone()), (UnitIndex(1), a.clone())],
        );
        assert_eq!(same.ok(), plain.ok());
        // At most 1 s apart; one part at the start; distinct units.
        let at = |o: u64, p: u64| {
            SystemSchedule::summed_at(
                D,
                48_000,
                0,
                vec![(UnitIndex(0), o, a.clone()), (UnitIndex(1), p, a.clone())],
            )
        };
        assert!(at(0, 48_000).is_ok());
        assert!(at(0, 48_001).is_err());
        assert!(at(10, 20).is_err());
        assert!(
            SystemSchedule::summed_at(
                D,
                48_000,
                0,
                vec![(UnitIndex(0), 0, a.clone()), (UnitIndex(0), 5, a.clone())]
            )
            .is_err()
        );
        assert!(
            SystemSchedule::summed_at(
                D,
                48_000,
                u64::MAX - 10,
                vec![(UnitIndex(0), 0, a.clone()), (UnitIndex(1), 20, a)]
            )
            .is_err()
        );
        assert_eq!(max_sum_spread_samples(48_000), 48_000);
        assert_eq!(max_sum_spread_samples(0), 0);
    }
}
