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

/// The most units one schedule spans.
pub const MAX_UNITS: usize = 16;
/// The most segments one schedule holds (passes 1 to 8 over up to 16 units).
pub const MAX_SEGMENTS: usize = 128;

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
        })
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

    /// The last segment's end.
    pub fn end(&self) -> u64 {
        self.segments.last().map_or(0, Segment::end)
    }

    /// The units that play, ascending, each once.
    pub fn units(&self) -> Vec<UnitIndex> {
        let mut u: Vec<UnitIndex> = self.segments.iter().map(|s| s.unit).collect();
        u.sort_unstable();
        u.dedup();
        u
    }
}

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
}
