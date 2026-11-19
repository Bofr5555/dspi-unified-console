//! `system.measure.run`'s rules (PLAN-AT sections 1.4 and 1.5, OD-ATj, OD-ATn,
//! review R6a), pure: a segment's length from the staged config, the lead,
//! the schedule's checks, when each unit's next op 9 is written, and what a
//! unit's `EXT_SIGGEN_TIMELINE` says of a segment.

use dspi_measure::system::messages::RunSegment;
use dspi_proto::ext::timebase::{SiggenTimeline, TimelineResult, TimelineState};
use dspi_proto::packets::SiggenConfig;

use super::domain::samples_in_us;

/// The longest run, first start to last end (S7).
pub(crate) const MAX_RUN_MS: u64 = 120_000;
/// How far ahead of its T the hub writes an op 9 at most: the firmware's
/// 120 s cap (`SIGGEN_START_MAX_LEAD_MS`, FW-B/FW-C review) less a margin for
/// the time between the hub's read and the unit's check.
pub(crate) const OP9_WRITE_AHEAD_MS: u64 = 110_000;
/// What the lead adds to the largest arm lead (OD-ATn, S-AT3).
pub(crate) const ARM_MARGIN_MS: u64 = 100;
/// What a write's own transfer may take: an op 9 written less than the arm
/// lead plus this ahead is late.
pub(crate) const LATE_MARGIN_MS: u64 = 25;
/// The longest lead a client may ask for.
pub(crate) const MAX_LEAD_MS: u32 = 10_000;
/// How long past its T a written op 9 may still read as pending before the
/// hub takes the start as failed.
pub(crate) const START_GRACE_MS: u64 = 500;

fn samples_ms(ms: u64, rate: u32) -> u64 {
    samples_in_us(ms.saturating_mul(1000), rate)
}

/// The samples one segment of `cfg` plays at `rate`, first to last (no gap
/// after the last cycle, as `WalkSchedule::total_samples`): a walk plays its
/// outputs one after the other, `repeat` passes; otherwise every output
/// together, `repeat` cycles.
///
/// Errors: what [`dspi_measure::stimulus::LogSweep::new`] refuses, in words.
pub(crate) fn segment_samples(cfg: &SiggenConfig, rate: u32) -> Result<u64, String> {
    use dspi_measure::stimulus::{LogSweep, SweepConfig};
    let sweep = LogSweep::new(
        rate,
        &SweepConfig {
            f1_hz: cfg.p1,
            f2_hz: cfg.p2,
            duration_ms: cfg.duration_ms,
            gap_ms: cfg.gap_ms,
        },
    )
    .map_err(|e| e.to_string())?;
    let per_pass = if cfg.flags & SiggenConfig::FLAG_WALK != 0 {
        u64::from(cfg.channel_mask.count_ones())
    } else {
        1
    };
    let cycles = per_pass.saturating_mul(u64::from(cfg.repeat_count));
    let period = u64::from(sweep.cycle_samples).saturating_add(u64::from(sweep.gap_samples));
    Ok(cycles
        .saturating_sub(1)
        .saturating_mul(period)
        .saturating_add(u64::from(sweep.cycle_samples)))
}

/// The run's lead, ms: the client's, or at least the largest arm lead plus
/// [`ARM_MARGIN_MS`] plus the writes' measured time (OD-ATn: 250 ms with a
/// K340's 150 ms arm lead and fast writes).
pub(crate) fn lead_ms(requested: u32, max_arm_lead_ms: u16, writes_ms: u64) -> u64 {
    u64::from(max_arm_lead_ms)
        .saturating_add(ARM_MARGIN_MS)
        .saturating_add(writes_ms)
        .max(u64::from(requested))
}

/// The checks of a run's segments (`bad_schedule`): some, at most
/// `MAX_SEGMENTS`, each of a unit of the session with a staged config
/// (`lens[unit]`), in time order; segments at one offset are a summed run of
/// distinct units; no segment starts before an earlier one has ended (one
/// generator at a time, S-AT4); a unit's next segment starts at least `lead`
/// samples after its previous one ends (R6a); the run is at most
/// [`MAX_RUN_MS`] long.
pub(crate) fn check_segments(
    segments: &[RunSegment],
    lens: &[Option<u64>],
    lead: u64,
    rate: u32,
) -> Result<(), String> {
    if segments.is_empty() {
        return Err("no segment".into());
    }
    if segments.len() > dspi_measure::system::MAX_SEGMENTS {
        return Err(format!(
            "{} segments (at most {})",
            segments.len(),
            dspi_measure::system::MAX_SEGMENTS
        ));
    }
    let len_of = |s: &RunSegment| -> Result<u64, String> {
        match lens.get(usize::from(s.unit)) {
            None => Err(format!("no unit {} in this session", s.unit)),
            Some(None) => Err(format!("unit {} has no config staged", s.unit)),
            Some(Some(l)) => Ok(*l),
        }
    };
    // The end of every segment before the current offset, and each unit's.
    let mut ended: u64 = 0;
    let mut group_offset: Option<u64> = None;
    let mut group_end: u64 = 0;
    let mut group_units: Vec<u8> = Vec::new();
    let mut unit_end: Vec<Option<u64>> = vec![None; lens.len()];
    for s in segments {
        let len = len_of(s)?;
        let end = s.offset.saturating_add(len);
        match group_offset {
            Some(g) if s.offset < g => return Err("segments out of time order".into()),
            Some(g) if s.offset == g => {
                if group_units.contains(&s.unit) {
                    return Err(format!("unit {} twice at one offset", s.unit));
                }
            }
            _ => {
                ended = ended.max(group_end);
                if s.offset < ended {
                    return Err(format!(
                        "unit {}'s segment at {} starts before the one before it ends ({ended})",
                        s.unit, s.offset
                    ));
                }
                group_offset = Some(s.offset);
                group_units.clear();
            }
        }
        if let Some(Some(prev)) = unit_end.get(usize::from(s.unit))
            && s.offset < prev.saturating_add(lead)
        {
            return Err(format!(
                "unit {}'s segment at {} starts less than the lead ({lead} samples) after its \
                 previous one ends ({prev})",
                s.unit, s.offset
            ));
        }
        if let Some(e) = unit_end.get_mut(usize::from(s.unit)) {
            *e = Some(end);
        }
        group_units.push(s.unit);
        group_end = group_end.max(end);
    }
    let span = ended.max(group_end);
    let first = segments.first().map_or(0, |s| s.offset);
    if span.saturating_sub(first) > samples_ms(MAX_RUN_MS, rate) {
        return Err(format!(
            "the run is longer than {} s",
            MAX_RUN_MS.saturating_div(1000)
        ));
    }
    Ok(())
}

// -- WP ATSUMOFF --
/// The checks of a summed run whose parts may start apart
/// (`system.measure.run` with `summed`, `bad_schedule`): two to
/// `MAX_UNITS` segments, each of a distinct unit of the session with a
/// staged config, in time order, none more than [`MAX_SUM_SPREAD_MS`] after
/// the first; the run at most [`MAX_RUN_MS`] long. Each unit has one segment, so
/// one op 9 pending per unit holds (R6a); every start is still `t0 +
/// offset`, at least the lead ahead (S-AT3).
///
/// [`MAX_SUM_SPREAD_MS`]: dspi_measure::system::MAX_SUM_SPREAD_MS
pub(crate) fn check_summed(
    segments: &[RunSegment],
    lens: &[Option<u64>],
    rate: u32,
) -> Result<(), String> {
    use dspi_measure::system::{MAX_SUM_SPREAD_MS, MAX_UNITS, max_sum_spread_samples};
    if segments.len() < 2 || segments.len() > MAX_UNITS {
        return Err(format!(
            "{} segments in a summed run (2 to {MAX_UNITS})",
            segments.len()
        ));
    }
    let mut units: Vec<u8> = Vec::with_capacity(segments.len());
    let mut prev: Option<u64> = None;
    let mut end: u64 = 0;
    for s in segments {
        let len = match lens.get(usize::from(s.unit)) {
            None => return Err(format!("no unit {} in this session", s.unit)),
            Some(None) => return Err(format!("unit {} has no config staged", s.unit)),
            Some(Some(l)) => *l,
        };
        if units.contains(&s.unit) {
            return Err(format!("unit {} twice in a summed run", s.unit));
        }
        if prev.is_some_and(|p| s.offset < p) {
            return Err("segments out of time order".into());
        }
        prev = Some(s.offset);
        units.push(s.unit);
        end = end.max(s.offset.saturating_add(len));
    }
    let first = segments.first().map_or(0, |s| s.offset);
    let last = segments.last().map_or(0, |s| s.offset);
    if last.saturating_sub(first) > max_sum_spread_samples(rate) {
        return Err(format!(
            "a summed run's parts start more than {MAX_SUM_SPREAD_MS} ms apart"
        ));
    }
    if end.saturating_sub(first) > samples_ms(MAX_RUN_MS, rate) {
        return Err(format!(
            "the run is longer than {} s",
            MAX_RUN_MS.saturating_div(1000)
        ));
    }
    Ok(())
}

/// What the scheduler does with a unit's next op 9 now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Due {
    /// Further ahead than [`OP9_WRITE_AHEAD_MS`]: later.
    Wait,
    /// Write it now.
    Write,
    /// Too near (less than the arm lead plus [`LATE_MARGIN_MS`]) or past:
    /// the run cannot keep its timeline.
    Late,
}

/// When a unit's next op 9 (T `t`, unit numbering) is written, with the unit
/// at `now` and its arm lead `arm_lead_ms` (S-AT3, FW-B/FW-C review).
pub(crate) fn op9_due(t: u64, now: u64, arm_lead_ms: u16, rate: u32) -> Due {
    let Some(ahead) = t.checked_sub(now) else {
        return Due::Late;
    };
    let min = samples_ms(u64::from(arm_lead_ms).saturating_add(LATE_MARGIN_MS), rate);
    if ahead < min {
        Due::Late
    } else if ahead > samples_ms(OP9_WRITE_AHEAD_MS, rate) {
        Due::Wait
    } else {
        Due::Write
    }
}

/// What a unit's timeline says of the segment written for T `t`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seen {
    /// Still waiting for its T.
    Pending,
    /// It started on `t`; `clean` while no discontinuity is counted.
    Started {
        /// `discontinuities` is 0.
        clean: bool,
        /// The run id it got.
        run_id: u32,
    },
    /// Refused or cancelled: why, the timeline's word.
    Failed(&'static str),
}

/// [`Seen`] from `tl` for the op 9 at `t`.
pub(crate) fn judge_timeline(tl: &SiggenTimeline, t: u64) -> Seen {
    if tl.start == t && tl.last_result == TimelineResult::Started {
        return Seen::Started {
            clean: tl.discontinuities == 0,
            run_id: tl.run_id,
        };
    }
    // Pending or armed; or `running` the unit's previous segment while this
    // one waits (the next op 9 is written once the previous has started).
    if t != 0
        && tl.pending == t
        && matches!(
            tl.state,
            TimelineState::Pending | TimelineState::Armed | TimelineState::Running
        )
    {
        return Seen::Pending;
    }
    Seen::Failed(match tl.last_result {
        TimelineResult::None | TimelineResult::Started => "not_pending",
        r => r.as_str(),
    })
}

/// Whether op 9 at `t` may still read pending with the unit at `now`.
pub(crate) fn still_in_grace(t: u64, now: u64, rate: u32) -> bool {
    now <= t.saturating_add(samples_ms(START_GRACE_MS, rate))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_athub {
    use super::*;
    use proptest::prelude::*;

    fn sweep(ms: u32, gap: u16, mask: u16, repeat: u16, walk: bool) -> SiggenConfig {
        SiggenConfig {
            signal_type: 4,
            channel_mask: mask,
            flags: if walk { SiggenConfig::FLAG_WALK } else { 0 },
            level_db: -30.0,
            duration_ms: ms,
            repeat_count: repeat,
            gap_ms: gap,
            p1: 20.0,
            p2: 20_000.0,
            ..SiggenConfig::default()
        }
    }

    fn seg(unit: u8, offset: u64) -> RunSegment {
        RunSegment { unit, offset }
    }

    #[test]
    fn wp_athub_segment_length_follows_the_walk() {
        assert_eq!(
            segment_samples(&sweep(1_000, 0, 1, 1, false), 48_000),
            Ok(48_000)
        );
        // Two outputs walked, two passes, 500 ms gaps: 4 cycles, 3 gaps.
        assert_eq!(
            segment_samples(&sweep(1_000, 500, 0b11, 2, true), 48_000),
            Ok(4 * 48_000 + 3 * 24_000)
        );
        // Together: two cycles.
        assert_eq!(
            segment_samples(&sweep(1_000, 500, 0b11, 2, false), 48_000),
            Ok(2 * 48_000 + 24_000)
        );
        assert!(segment_samples(&sweep(500, 0, 1, 1, false), 48_000).is_err());
    }

    #[test]
    fn wp_athub_the_lead_covers_the_arm_lead_and_the_writes() {
        assert_eq!(lead_ms(0, 150, 0), 250, "a K340");
        assert_eq!(lead_ms(250, 150, 12), 262);
        assert_eq!(lead_ms(1_000, 150, 20), 1_000);
        assert_eq!(lead_ms(0, 50, 30), 180);
    }

    #[test]
    fn wp_athub_a_units_segments_closer_than_the_lead_are_refused() {
        let l = [Some(48_000), Some(48_000)];
        let lead = 14_400;
        // A again exactly the lead after its end: fine; a sample less: not.
        assert!(check_segments(&[seg(0, 0), seg(0, 62_400)], &l, lead, 48_000).is_ok());
        let e = check_segments(&[seg(0, 0), seg(0, 62_399)], &l, lead, 48_000);
        assert!(e.unwrap_err().contains("lead"));
        // A, B, A: B in between, A's second after B's end.
        assert!(
            check_segments(
                &[seg(0, 0), seg(1, 48_000), seg(0, 96_000)],
                &l,
                lead,
                48_000
            )
            .is_ok()
        );
        // Overlap across units.
        assert!(check_segments(&[seg(0, 0), seg(1, 47_999)], &l, lead, 48_000).is_err());
        // A summed run: one offset, distinct units.
        assert!(check_segments(&[seg(0, 0), seg(1, 0)], &l, lead, 48_000).is_ok());
        assert!(check_segments(&[seg(0, 0), seg(0, 0)], &l, lead, 48_000).is_err());
        // Order, unknown and unstaged units, empty, too long.
        assert!(check_segments(&[seg(1, 48_000), seg(0, 0)], &l, lead, 48_000).is_err());
        assert!(check_segments(&[seg(2, 0)], &l, lead, 48_000).is_err());
        assert!(check_segments(&[seg(0, 0)], &[None], lead, 48_000).is_err());
        assert!(check_segments(&[], &l, lead, 48_000).is_err());
        assert!(check_segments(&[seg(0, 0), seg(1, 121 * 48_000)], &l, lead, 48_000).is_err());
    }

    // -- WP ATSUMOFF --
    #[test]
    fn wp_atsumoff_a_summed_run_may_start_its_parts_apart_within_the_spread() {
        let l = [Some(48_000), Some(48_000), Some(48_000)];
        // Overlapping starts are a summed run only when asked for.
        assert!(check_segments(&[seg(0, 0), seg(1, 1_000)], &l, 0, 48_000).is_err());
        assert!(check_summed(&[seg(0, 0), seg(1, 1_000)], &l, 48_000).is_ok());
        assert!(check_summed(&[seg(0, 0), seg(1, 0), seg(2, 2_000)], &l, 48_000).is_ok());
        // At most 1 s apart.
        assert!(check_summed(&[seg(0, 0), seg(1, 48_000)], &l, 48_000).is_ok());
        let e = check_summed(&[seg(0, 0), seg(1, 48_001)], &l, 48_000);
        assert!(e.unwrap_err().contains("apart"));
        // Distinct units, in order, staged, two or more.
        assert!(check_summed(&[seg(0, 0), seg(0, 10)], &l, 48_000).is_err());
        assert!(check_summed(&[seg(1, 10), seg(0, 0)], &l, 48_000).is_err());
        assert!(check_summed(&[seg(0, 0)], &l, 48_000).is_err());
        assert!(check_summed(&[seg(0, 0), seg(3, 0)], &l, 48_000).is_err());
        assert!(check_summed(&[seg(0, 0), seg(1, 0)], &[Some(1), None], 48_000).is_err());
        // Too long.
        let long = [Some(121 * 48_000), Some(48_000)];
        assert!(check_summed(&[seg(0, 0), seg(1, 0)], &long, 48_000).is_err());
    }

    #[test]
    fn wp_athub_op9_is_written_within_120_s_and_never_late() {
        let r = 48_000;
        assert_eq!(op9_due(1_000_000, 1_000_000 - 12_000, 150, r), Due::Write);
        assert_eq!(op9_due(1_000_000, 1_000_000 - 8_400, 150, r), Due::Write);
        assert_eq!(op9_due(1_000_000, 1_000_000 - 8_399, 150, r), Due::Late);
        assert_eq!(op9_due(1_000_000, 1_000_001, 150, r), Due::Late);
        let far = 111 * r as u64;
        assert_eq!(op9_due(far + 5, 5, 150, r), Due::Wait);
        assert_eq!(op9_due(110 * r as u64, 0, 150, r), Due::Write);
    }

    #[test]
    fn wp_athub_the_timeline_tells_pending_started_and_refused() {
        let base = SiggenTimeline {
            time_base: 0,
            state: TimelineState::Pending,
            last_result: TimelineResult::None,
            discontinuities: 0,
            arm_lead_ms: 150,
            start: 0,
            pending: 500,
            samples: 0,
            held: 0,
            run_id: 3,
        };
        assert_eq!(judge_timeline(&base, 500), Seen::Pending);
        let started = SiggenTimeline {
            state: TimelineState::Running,
            last_result: TimelineResult::Started,
            start: 500,
            pending: 0,
            run_id: 4,
            ..base
        };
        assert_eq!(
            judge_timeline(&started, 500),
            Seen::Started {
                clean: true,
                run_id: 4
            }
        );
        let broken = SiggenTimeline {
            discontinuities: 1,
            ..started
        };
        assert!(matches!(
            judge_timeline(&broken, 500),
            Seen::Started { clean: false, .. }
        ));
        let refused = SiggenTimeline {
            state: TimelineState::Idle,
            last_result: TimelineResult::RefusedNotReady,
            pending: 0,
            ..base
        };
        assert!(matches!(judge_timeline(&refused, 500), Seen::Failed(_)));
        // The previous segment plays while this one waits.
        let behind = SiggenTimeline {
            pending: 900,
            ..started
        };
        assert_eq!(judge_timeline(&behind, 900), Seen::Pending);
        assert!(still_in_grace(500, 500 + 24_000, 48_000));
        assert!(!still_in_grace(500, 500 + 24_001, 48_000));
    }

    proptest! {
        #[test]
        fn proptest_wp_athub_schedule_checks_never_panic(
            segs in proptest::collection::vec((0u8..5, any::<u64>()), 0..40),
            lens in proptest::collection::vec(proptest::option::of(any::<u64>()), 0..5),
            lead in any::<u64>(), rate in any::<u32>(),
            t in any::<u64>(), now in any::<u64>(), arm in any::<u16>(),
        ) {
            let segs: Vec<RunSegment> = segs.iter().map(|(u, o)| seg(*u, *o)).collect();
            let _ = check_segments(&segs, &lens, lead, rate);
            let _ = check_summed(&segs, &lens, rate);
            let _ = op9_due(t, now, arm, rate);
            let _ = still_in_grace(t, now, rate);
        }

        /// An accepted schedule never overlaps and keeps every unit's lead.
        #[test]
        fn proptest_wp_athub_accepted_schedules_keep_the_rules(
            raw in proptest::collection::vec((0u8..3, 0u64..400_000), 1..12),
            lead in 0u64..50_000,
        ) {
            let lens = [Some(48_000u64), Some(60_000), Some(30_000)];
            let mut raw = raw;
            raw.sort_by_key(|(_, o)| *o);
            let segs: Vec<RunSegment> = raw.iter().map(|(u, o)| seg(*u, *o)).collect();
            if check_segments(&segs, &lens, lead, 48_000).is_ok() {
                for (i, a) in segs.iter().enumerate() {
                    for b in &segs[i + 1..] {
                        let a_end = a.offset + lens[usize::from(a.unit)].unwrap();
                        if a.unit == b.unit {
                            prop_assert!(b.offset >= a_end + lead);
                        } else if a.offset != b.offset {
                            prop_assert!(b.offset >= a_end);
                        }
                    }
                }
            }
        }
    }
}
