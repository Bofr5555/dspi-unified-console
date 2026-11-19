//! The gate's added conditions G1 to G4 (WP ATSYS, PLAN-AT section 1.4), as
//! plain facts the session fills from the hub's replies and judges here.
//! G1 to G4 discard a run; G5 (the timing) is
//! [`super::analysis::SystemAnalysis::timing_reliable`] and the offered
//! delays.
//!
//! The gate trusts counters and read-backs, not events (OD-ATi): a
//! `disturbances` count and `t - output_frames` read at open and at the end
//! cannot miss a transition the rate-limited events collapse; the timeline
//! read-back proves every start; the `t` agreement of one hub tick catches a
//! collision of short domain ids.

use dspi_proto::ext::timebase::{TB_STATE_LOCKED, timeline_result};

use super::{DomainId, SystemSchedule, UnitIndex};
use crate::types::MeasureError;

/// What the session knows of one unit for the gate (from
/// `system.measure.open`, `status` and the end of the run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitFacts {
    /// The unit.
    pub unit: UnitIndex,
    /// The `EXT_TIME_BASE` entry the run uses: its time base and domain.
    pub domain: DomainId,
    /// How many of the unit's entries carry the group's `(time_base,
    /// domain)` (G1: exactly one, the one used for T, G2 and op 9).
    pub matching_entries: u8,
    /// G1's agreement: the unit's `t` read in one hub tick, mapped to the
    /// domain numbering and corrected by the hub's own timestamps of its
    /// read, minus the group's reference `t` of that tick, samples.
    pub t_offset_samples: i64,
    /// That tick's read round trip, samples (the agreement's tolerance).
    pub tick_round_trip_samples: u64,
    /// The entry's contract state at open (`TB_STATE_*`).
    pub state_open: u8,
    /// Its state at the end.
    pub state_end: u8,
    /// Its `disturbances` at open.
    pub disturbances_open: u16,
    /// Its `disturbances` at the end.
    pub disturbances_end: u16,
    /// `t - output_frames` at open, modulo 2^32.
    pub t_minus_output_open: u32,
    /// `t - output_frames` at the end, modulo 2^32.
    pub t_minus_output_end: u32,
    /// The unit's own numbering against the domain's: unit `T` = domain `T`
    /// minus this (a USB unit's S origin resolved at open; 0 on the line and
    /// S/PDIF).
    pub origin: i64,
    /// The 0xE1/0xE3/0xE4 transitions, aborts and foreign generator stops
    /// the hub saw on the unit during the run (G4).
    pub events: u32,
}

/// `EXT_SIGGEN_TIMELINE` read after a segment (G3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentProof {
    /// The segment's index in the schedule.
    pub segment: usize,
    /// The `run_id` the session expects for it (one more than the unit's
    /// before its START).
    pub expected_run_id: u32,
    /// `run_id` read.
    pub run_id: u32,
    /// `start`: T of the generator's sample 0, in the unit's own numbering.
    pub start: u64,
    /// `last_result` (`timeline_result::*`).
    pub last_result: u8,
    /// `discontinuities` of the run.
    pub discontinuities: u16,
    /// `samples` rendered in the run.
    pub samples: u32,
}

/// Why a run fails G1 to G4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFinding {
    /// A unit of the schedule without facts.
    NoFacts {
        /// The unit.
        unit: UnitIndex,
    },
    /// G1: a unit on another time base or domain than the schedule's.
    NotOneDomain {
        /// The unit.
        unit: UnitIndex,
        /// Its domain.
        domain: DomainId,
    },
    /// G1: a unit with no or several entries of the group's domain.
    EntryNotUnique {
        /// The unit.
        unit: UnitIndex,
        /// How many.
        entries: u8,
    },
    /// G1: the unit's `t` disagrees with the group's beyond the read's
    /// round trip.
    TimeDisagrees {
        /// The unit.
        unit: UnitIndex,
        /// The difference, samples.
        offset_samples: i64,
        /// The tolerance, samples.
        round_trip_samples: u64,
    },
    /// G2: the entry was not `LOCKED` at open or at the end.
    NotLocked {
        /// The unit.
        unit: UnitIndex,
        /// At the end (else at open).
        at_end: bool,
        /// The state read.
        state: u8,
    },
    /// G2: the disturbance counter moved.
    Disturbed {
        /// The unit.
        unit: UnitIndex,
        /// At open.
        before: u16,
        /// At the end.
        after: u16,
    },
    /// G2: `t - output_frames` moved by more than one sample.
    TimelineMoved {
        /// The unit.
        unit: UnitIndex,
        /// The change, samples (modulo 2^32, signed).
        by: i64,
    },
    /// G3: no read-back of a segment.
    NoProof {
        /// The segment.
        segment: usize,
    },
    /// G3: the read-back does not prove the segment.
    ProofMismatch {
        /// The segment.
        segment: usize,
        /// What differs, in words.
        what: String,
    },
    /// G4: the hub saw an event on the unit during the run.
    Events {
        /// The unit.
        unit: UnitIndex,
        /// How many.
        count: u32,
    },
}

impl std::fmt::Display for GateFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GateFinding::NoFacts { unit } => write!(f, "unit {}: nothing known of it", unit.0),
            GateFinding::NotOneDomain { unit, domain } => write!(
                f,
                "unit {} is on time base {} domain {:#x}: these units do not share a clock",
                unit.0, domain.time_base, domain.id
            ),
            GateFinding::EntryNotUnique { unit, entries } => write!(
                f,
                "unit {} lists {entries} time-base entries of the group's domain (one is needed)",
                unit.0
            ),
            GateFinding::TimeDisagrees {
                unit,
                offset_samples,
                round_trip_samples,
            } => write!(
                f,
                "unit {}: its time base is {offset_samples} samples off the group's (more than the read's {round_trip_samples}): these units do not share a clock",
                unit.0
            ),
            GateFinding::NotLocked {
                unit,
                at_end,
                state,
            } => write!(
                f,
                "unit {}: its time base was not locked {} (state {state})",
                unit.0,
                if *at_end {
                    "at the end"
                } else {
                    "at the start"
                }
            ),
            GateFinding::Disturbed {
                unit,
                before,
                after,
            } => write!(
                f,
                "unit {}: its time base was disturbed during the run ({before} -> {after})",
                unit.0
            ),
            GateFinding::TimelineMoved { unit, by } => write!(
                f,
                "unit {}: its timeline moved by {by} samples during the run",
                unit.0
            ),
            GateFinding::NoProof { segment } => {
                write!(f, "segment {segment}: its start was not read back")
            }
            GateFinding::ProofMismatch { segment, what } => {
                write!(f, "segment {segment}: {what}")
            }
            GateFinding::Events { unit, count } => write!(
                f,
                "unit {}: {count} time-base or generator events during the run",
                unit.0
            ),
        }
    }
}

/// G1 alone, as the plan step checks it at open: every unit of the schedule
/// on the schedule's one domain, with one entry of it, and its `t` agreeing.
///
/// Errors: [`MeasureError::Invalid`] "these units do not share a clock",
/// naming the first finding.
pub fn check_one_domain(
    schedule: &SystemSchedule,
    facts: &[UnitFacts],
) -> Result<(), MeasureError> {
    let mut found = Vec::new();
    for unit in schedule.units() {
        match facts.iter().find(|f| f.unit == unit) {
            None => found.push(GateFinding::NoFacts { unit }),
            Some(f) => g1(schedule, f, &mut found),
        }
    }
    match found.first() {
        None => Ok(()),
        Some(g) => Err(MeasureError::Invalid(format!(
            "these units do not share a clock: {g}"
        ))),
    }
}

fn g1(schedule: &SystemSchedule, f: &UnitFacts, out: &mut Vec<GateFinding>) {
    let unit = f.unit;
    if f.domain != schedule.domain {
        out.push(GateFinding::NotOneDomain {
            unit,
            domain: f.domain,
        });
    }
    if f.matching_entries != 1 {
        out.push(GateFinding::EntryNotUnique {
            unit,
            entries: f.matching_entries,
        });
    }
    if f.t_offset_samples.unsigned_abs() > f.tick_round_trip_samples {
        out.push(GateFinding::TimeDisagrees {
            unit,
            offset_samples: f.t_offset_samples,
            round_trip_samples: f.tick_round_trip_samples,
        });
    }
}

/// G1 to G4 over a finished run: empty when the run may be used.
pub fn check_gate(
    schedule: &SystemSchedule,
    facts: &[UnitFacts],
    proofs: &[SegmentProof],
) -> Vec<GateFinding> {
    let mut out = Vec::new();
    for unit in schedule.units() {
        let Some(f) = facts.iter().find(|f| f.unit == unit) else {
            out.push(GateFinding::NoFacts { unit });
            continue;
        };
        g1(schedule, f, &mut out);
        // G2: locked at both ends, the counter still, the timeline still.
        for (at_end, state) in [(false, f.state_open), (true, f.state_end)] {
            if state != TB_STATE_LOCKED {
                out.push(GateFinding::NotLocked {
                    unit,
                    at_end,
                    state,
                });
            }
        }
        if f.disturbances_open != f.disturbances_end {
            out.push(GateFinding::Disturbed {
                unit,
                before: f.disturbances_open,
                after: f.disturbances_end,
            });
        }
        let by = i64::from(f.t_minus_output_end.wrapping_sub(f.t_minus_output_open) as i32);
        if by.unsigned_abs() > 1 {
            out.push(GateFinding::TimelineMoved { unit, by });
        }
        // G4.
        if f.events != 0 {
            out.push(GateFinding::Events {
                unit,
                count: f.events,
            });
        }
    }
    // G3: every segment proven.
    for (i, seg) in schedule.segments().iter().enumerate() {
        let Some(p) = proofs.iter().find(|p| p.segment == i) else {
            out.push(GateFinding::NoProof { segment: i });
            continue;
        };
        let origin = facts
            .iter()
            .find(|f| f.unit == seg.unit)
            .map_or(0, |f| f.origin);
        // A u64 and an i64 as i128: the difference cannot overflow.
        let want = i128::from(seg.start).saturating_sub(i128::from(origin));
        let mut what = Vec::new();
        if p.run_id != p.expected_run_id {
            what.push(format!(
                "run {} where {} was expected",
                p.run_id, p.expected_run_id
            ));
        }
        if i128::from(p.start) != want {
            what.push(format!(
                "it started at {} where {want} was scheduled",
                p.start
            ));
        }
        if p.last_result != timeline_result::STARTED {
            what.push(format!("its result is {} (not started)", p.last_result));
        }
        if p.discontinuities != 0 {
            what.push(format!("{} discontinuities", p.discontinuities));
        }
        if u64::from(p.samples) != seg.schedule.total_samples() {
            what.push(format!(
                "{} samples where {} were scheduled",
                p.samples,
                seg.schedule.total_samples()
            ));
        }
        if !what.is_empty() {
            out.push(GateFinding::ProofMismatch {
                segment: i,
                what: what.join(", "),
            });
        }
    }
    out
}
