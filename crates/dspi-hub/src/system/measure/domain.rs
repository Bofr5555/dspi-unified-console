//! Gate G1 at `system.measure.open` (PLAN-AT section 1.4, review R7a and R7b),
//! pure: which `EXT_TIME_BASE` entry each unit's group uses, whether the units'
//! T agree as one numbering, the per-unit USB origins, and the rate.
//!
//! The counters and reads are what is trusted (OD-ATi): a short domain id can
//! collide, a `t` disagreement cannot hide.

use dspi_proto::ext::timebase::{TB_USB_SAMPLE, TimeBaseEntry};

/// Samples the `t` agreement allows on top of the reads' round trips: the
/// whole-sample rounding of each `t`.
pub(crate) const AGREE_MARGIN: u64 = 2;

/// The shortest span between two reads of one unit the rate is judged from,
/// in the unit's own microseconds: long enough that a read's `t` and
/// `time_us` a few milliseconds apart (a busy host, a fake) still name the
/// rate within [`RATE_TOLERANCE_PCT`].
pub(crate) const RATE_SPAN_US: u32 = 100_000;
/// How far an estimate may be from a rate, percent (44.1 and 48 kHz are 8.5 %
/// apart).
pub(crate) const RATE_TOLERANCE_PCT: u128 = 4;

// -- review AT-D1 --
/// How many more rounds of reads `open` makes, each doubling the span since
/// the first reads, when a unit's two reads were not timed well enough to
/// tell its rate ([`rate_reads_timed`]): 0.1 s, then 0.2, 0.4, 0.8 and 1.6 s.
pub(crate) const RATE_EXTRA_ROUNDS: u32 = 4;
/// How much of the span between two reads their round trips may be, percent,
/// for the pair to tell the rate: the estimate is then off by at most that,
/// inside [`RATE_TOLERANCE_PCT`] with a margin for the unit's own clock.
pub(crate) const RATE_JITTER_PCT: u128 = 3;

/// Whether two reads of one unit pin its rate, from the hub's own clock:
/// `rtt_*_us` each read's round trip, `span_us` from the first answer to the
/// second request. A read's `t` and `time_us` are one instant on firmware
/// (R7c), but through a busy host, a bridge or a fake they are only known to
/// lie within the read's round trip, so [`rate_of`]'s estimate may be off by
/// about `(rtt_first + rtt_second) / span`. [`RATE_TOLERANCE_PCT`] is that
/// allowance: within it a real rate is named and never taken for its
/// neighbour (44.1 and 48 kHz are 8.5 % apart); beyond it the estimate says
/// nothing (15 ms over 100 ms names no rate, 8 ms reads 48 kHz as 44.1 kHz)
/// and `open` reads again over a longer span rather than judging by it. The
/// domain check is not loosened: a pair that is not timed is never used, and
/// a timed pair is judged by [`rate_of`] as before.
pub(crate) fn rate_reads_timed(rtt_first_us: u64, rtt_second_us: u64, span_us: u64) -> bool {
    let rtt = u128::from(rtt_first_us).saturating_add(u128::from(rtt_second_us));
    span_us >= u64::from(RATE_SPAN_US)
        && rtt.saturating_mul(100) <= u128::from(span_us).saturating_mul(RATE_JITTER_PCT)
}

/// The rates a measurement runs at (the generator's, `stimulus::SUPPORTED_RATES`).
pub(crate) const RATES: [u32; 3] = dspi_measure::stimulus::SUPPORTED_RATES;

/// Why the units are not one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DomainRefusal {
    /// The unit at this index has no `LOCKED` entry the group could use.
    NotLocked(usize),
    /// Not one numbering: why, in words.
    NotOneDomain(String),
}

/// The group's time base and, per unit, the index of the entry it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    /// The time base id.
    pub time_base: u8,
    /// The domain id (C5).
    pub domain: u32,
    /// Per unit, its entry's index in its `EXT_TIME_BASE` read.
    pub entry: Vec<usize>,
}

/// G1's "one entry with the group's `(time_base, domain)`" (R7b): the key
/// every unit has, and exactly one entry for it on each unit. A unit with two
/// entries (a USB-direct follower that is also an S/PDIF source) takes part
/// by the one the others share.
pub(crate) fn choose_group(units: &[&[TimeBaseEntry]]) -> Result<Group, DomainRefusal> {
    let key = |e: &TimeBaseEntry| (e.time_base, e.domain);
    let Some(first) = units.first() else {
        return Err(DomainRefusal::NotOneDomain("no unit".into()));
    };
    let mut common: Vec<(u8, u32)> = first.iter().map(key).collect();
    common.sort_unstable();
    common.dedup();
    for u in units.iter().skip(1) {
        common.retain(|k| u.iter().any(|e| key(e) == *k));
    }
    let unlocked = || {
        units
            .iter()
            .position(|u| !u.iter().any(|e| e.state.is_locked()))
    };
    let (time_base, domain) = match common.as_slice() {
        [one] => *one,
        [] => {
            if let Some(i) = unlocked() {
                return Err(DomainRefusal::NotLocked(i));
            }
            return Err(DomainRefusal::NotOneDomain(
                "the units report no time base and domain in common".into(),
            ));
        }
        _ => {
            return Err(DomainRefusal::NotOneDomain(
                "the units share more than one time base; the group is not one domain".into(),
            ));
        }
    };
    let mut entry = Vec::with_capacity(units.len());
    for (i, u) in units.iter().enumerate() {
        let mut found = u
            .iter()
            .enumerate()
            .filter(|(_, e)| key(e) == (time_base, domain));
        match (found.next(), found.next()) {
            (Some((at, e)), None) => {
                if !e.state.is_locked() {
                    return Err(DomainRefusal::NotLocked(i));
                }
                entry.push(at);
            }
            _ => {
                return Err(DomainRefusal::NotOneDomain(format!(
                    "unit {i} lists the group's time base more than once"
                )));
            }
        }
    }
    Ok(Group {
        time_base,
        domain,
        entry,
    })
}

/// The unit's rate from two reads of one entry: `(t, time_us)` each, the
/// second at least [`RATE_SPAN_US`] after the first on the unit's clock
/// (`time_us` wraps). The one of [`RATES`] within [`RATE_TOLERANCE_PCT`],
/// else `None`.
pub(crate) fn rate_of(first: (u64, u32), second: (u64, u32)) -> Option<u32> {
    let dt = second.0.checked_sub(first.0)?;
    let dus = second.1.wrapping_sub(first.1);
    if dus < RATE_SPAN_US {
        return None;
    }
    let est = u128::from(dt)
        .checked_mul(1_000_000)?
        .checked_div(u128::from(dus))?;
    RATES.into_iter().find(|r| {
        let r = u128::from(*r);
        est.abs_diff(r).saturating_mul(100) <= r.saturating_mul(RATE_TOLERANCE_PCT)
    })
}

/// Samples in `us` microseconds at `rate`, rounded up.
pub(crate) fn samples_in_us(us: u64, rate: u32) -> u64 {
    let s = u128::from(us)
        .saturating_mul(u128::from(rate))
        .div_ceil(1_000_000);
    u64::try_from(s).unwrap_or(u64::MAX)
}

/// One unit's read for the agreement: its entry's `t`, the middle of the
/// read on the hub's clock and its round trip, microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reading {
    /// T of the group's entry.
    pub t: u64,
    /// The middle of the read, hub microseconds.
    pub mid_us: u64,
    /// The read's round trip, microseconds.
    pub rtt_us: u64,
}

/// USB's origin step at `rate`: units count S from origins `2048 * spf`
/// apart (`extension_channel_spec.md` 2.9.1).
pub(crate) fn usb_origin_step(rate: u32) -> u64 {
    2048u64.saturating_mul(u64::from(rate / 1000))
}

/// The units' origins against the first (`t_unit - t_domain`, the domain
/// numbering being the first unit's), checked: each `t` corrected by the
/// hub's timestamps of its read agrees with the first unit's within the two
/// reads' round trips plus [`AGREE_MARGIN`] (R7a); USB origins are whole
/// multiples of [`usb_origin_step`], every other transport has one numbering.
pub(crate) fn origins(
    time_base: u8,
    rate: u32,
    readings: &[Reading],
) -> Result<Vec<i64>, DomainRefusal> {
    let Some(r0) = readings.first() else {
        return Ok(Vec::new());
    };
    let step = i128::from(usb_origin_step(rate));
    let mut out = Vec::with_capacity(readings.len());
    for (i, r) in readings.iter().enumerate() {
        // Every value here is below 2^65 in size: no i128 operation overflows,
        // saturating keeps the lint's promise all the same.
        let since = i128::from(r.mid_us).saturating_sub(i128::from(r0.mid_us));
        let moved = since
            .saturating_mul(i128::from(rate))
            .checked_div(1_000_000)
            .unwrap_or(0);
        let predicted = i128::from(r0.t).saturating_add(moved);
        let diff = i128::from(r.t).saturating_sub(predicted);
        let origin = if time_base == TB_USB_SAMPLE && step > 0 {
            // The nearest whole step, rounding half away from zero.
            let half = diff
                .signum()
                .saturating_mul(step.checked_div(2).unwrap_or(0));
            let k = diff.saturating_add(half).checked_div(step).unwrap_or(0);
            k.saturating_mul(step)
        } else {
            0
        };
        let residual = diff.saturating_sub(origin).unsigned_abs();
        let tol =
            samples_in_us(r.rtt_us.saturating_add(r0.rtt_us), rate).saturating_add(AGREE_MARGIN);
        if residual > u128::from(tol) {
            return Err(DomainRefusal::NotOneDomain(format!(
                "unit {i}'s time base is {residual} samples away from the first unit's \
                 (at most {tol} for one clock): these units do not share a clock"
            )));
        }
        out.push(i64::try_from(origin).unwrap_or(i64::MAX));
    }
    Ok(out)
}

/// The USB bus check of G1: the units' host buses, where the platform gives
/// one (`nusb`'s `bus_id`), all the same. A unit whose bus is not known is
/// judged by the `t` agreement alone.
pub(crate) fn same_bus(buses: &[Option<String>]) -> bool {
    let mut known = buses.iter().flatten();
    match known.next() {
        Some(first) => known.all(|b| b == first),
        None => true,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_athub {
    use super::*;
    use dspi_proto::ext::timebase::{
        ContractState, TB_ALINK_LINE, TB_SPDIF, TimeBaseReason, TimeBaseRole,
    };
    use proptest::prelude::*;

    fn entry(tb: u8, domain: u32, state: ContractState, role: TimeBaseRole) -> TimeBaseEntry {
        TimeBaseEntry {
            time_base: tb,
            role,
            state,
            last_reason: TimeBaseReason::None,
            domain,
            t: 1_000,
            disturbances: 0,
            d: 0,
            phase_error: 0.0,
        }
    }

    const USB: u32 = 0x0100_0000;
    const SP: u32 = 0x0301_1234;

    #[test]
    fn wp_athub_a_two_entry_unit_takes_part_by_the_shared_entry() {
        // R7b: a USB-direct follower that is also an S/PDIF source.
        let both = [
            entry(
                TB_USB_SAMPLE,
                USB,
                ContractState::Locked,
                TimeBaseRole::Follower,
            ),
            entry(TB_SPDIF, SP, ContractState::Locked, TimeBaseRole::Source),
        ];
        let usb = [entry(
            TB_USB_SAMPLE,
            USB,
            ContractState::Locked,
            TimeBaseRole::Follower,
        )];
        let g = choose_group(&[&both, &usb]).unwrap();
        assert_eq!(
            (g.time_base, g.domain, g.entry.clone()),
            (TB_USB_SAMPLE, USB, vec![0, 0])
        );
        // With an S/PDIF follower of its source instead, the S/PDIF entry is used.
        let follower = [entry(
            TB_SPDIF,
            SP,
            ContractState::Locked,
            TimeBaseRole::Follower,
        )];
        let g = choose_group(&[&both, &follower]).unwrap();
        assert_eq!((g.time_base, g.entry.clone()), (TB_SPDIF, vec![1, 0]));
        // Two such units share both: not one group.
        assert!(matches!(
            choose_group(&[&both, &both]),
            Err(DomainRefusal::NotOneDomain(_))
        ));
    }

    #[test]
    fn wp_athub_an_unlocked_or_foreign_unit_is_named() {
        let a = [entry(
            TB_ALINK_LINE,
            7,
            ContractState::Locked,
            TimeBaseRole::Follower,
        )];
        let held = [entry(
            TB_ALINK_LINE,
            7,
            ContractState::Holdover,
            TimeBaseRole::Follower,
        )];
        let other = [entry(
            TB_ALINK_LINE,
            8,
            ContractState::Locked,
            TimeBaseRole::Follower,
        )];
        let none: [TimeBaseEntry; 0] = [];
        assert_eq!(choose_group(&[&a, &held]), Err(DomainRefusal::NotLocked(1)));
        assert!(matches!(
            choose_group(&[&a, &other]),
            Err(DomainRefusal::NotOneDomain(_))
        ));
        assert_eq!(choose_group(&[&a, &none]), Err(DomainRefusal::NotLocked(1)));
    }

    #[test]
    fn wp_athub_the_rate_is_read_from_two_reads() {
        assert_eq!(rate_of((1_000, 0), (1_000 + 4_800, 100_000)), Some(48_000));
        assert_eq!(
            rate_of((0, u32::MAX - 99_999), (9_600, 100_000)),
            Some(48_000)
        );
        assert_eq!(rate_of((0, 0), (4_410, 100_000)), Some(44_100));
        assert_eq!(rate_of((0, 0), (9_600, 100_000)), Some(96_000));
        // A read whose `t` came 3 ms after its `time_us`.
        assert_eq!(rate_of((0, 0), (4_944, 100_000)), Some(48_000));
        assert_eq!(rate_of((0, 0), (100, 5_000)), None, "too short a span");
        assert_eq!(rate_of((0, 0), (6_500, 100_000)), None, "65 kHz");
        assert_eq!(rate_of((5, 0), (4, 200_000)), None, "backwards");
    }

    // -- review AT-D1 --
    /// The flake under host load: a read whose `t` and `time_us` were taken
    /// some ms apart. 15 ms over 100 ms names no rate, 8 ms names the wrong
    /// one; neither pair counts as timed, and over a longer span the same
    /// jitter is inside the tolerance and the rate is right.
    #[test]
    fn review_atd1_a_jittered_read_is_not_timed_and_a_longer_span_names_the_rate() {
        // 48 kHz; the second read's `time_us` taken `j` us after its `t`.
        let pair = |span: u32, j: u32| ((0u64, 0u32), (u64::from(span) * 48 / 1000, span + j));
        let (a, b) = pair(100_000, 15_000);
        assert_eq!(rate_of(a, b), None, "the flake");
        let (a, b) = pair(100_000, 8_000);
        assert_eq!(rate_of(a, b), Some(44_100), "worse: the wrong rate");
        // The hub sees such a read's round trip (the jitter is inside it).
        assert!(!rate_reads_timed(300, 15_300, 100_000));
        assert!(!rate_reads_timed(300, 8_300, 100_000));
        assert!(!rate_reads_timed(2_000, 2_000, 100_000));
        assert!(rate_reads_timed(300, 300, 100_000));
        assert!(rate_reads_timed(1_500, 1_500, 100_000));
        assert!(!rate_reads_timed(0, 0, 99_999), "too short a span");
        // Whatever is timed names the rate right, at the worst jitter it
        // allows, in either read and in either direction.
        for span in [100_000u32, 200_000, 400_000, 800_000, 1_600_000] {
            let worst = span * 3 / 100;
            assert!(rate_reads_timed(0, u64::from(worst), u64::from(span)));
            assert!(!rate_reads_timed(1, u64::from(worst), u64::from(span)));
            for rate in RATES {
                // Each read's `t` and `time_us` lie somewhere in its round
                // trip: the samples and the unit's microseconds between the
                // reads are each the span to the span plus the round trips.
                for (t_us, own_us) in [
                    (span, span),
                    (span, span + worst),
                    (span + worst, span),
                    (span + worst, span + worst),
                ] {
                    let dt = u64::from(t_us) * u64::from(rate) / 1_000_000;
                    assert_eq!(rate_of((0, 7), (dt, 7 + own_us)), Some(rate), "{span}");
                }
            }
        }
        assert!(
            rate_reads_timed(15_300, 300, 800_000),
            "the same jitter, a longer span"
        );
        assert!(!rate_reads_timed(u64::MAX, u64::MAX, u64::MAX));
    }

    #[test]
    fn wp_athub_usb_origins_are_whole_wraps_and_a_collision_is_caught() {
        let step = usb_origin_step(48_000);
        assert_eq!(step, 98_304);
        let r = |t, mid_us| Reading {
            t,
            mid_us,
            rtt_us: 200,
        };
        // Unit 1 read 1 ms later, three wraps behind.
        let o = origins(
            TB_USB_SAMPLE,
            48_000,
            &[r(10 * step, 0), r(7 * step + 48, 1_000)],
        )
        .unwrap();
        assert_eq!(o, vec![0, -3 * step as i64]);
        // The line: no origins; 40 samples apart is not one clock.
        assert_eq!(
            origins(
                TB_ALINK_LINE,
                48_000,
                &[r(1_000_000, 0), r(1_000_048, 1_000)]
            )
            .unwrap(),
            vec![0, 0]
        );
        assert!(matches!(
            origins(
                TB_ALINK_LINE,
                48_000,
                &[r(1_000_000, 0), r(1_000_088, 1_000)]
            ),
            Err(DomainRefusal::NotOneDomain(_))
        ));
        // USB: half a wrap off is no origin either.
        assert!(origins(TB_USB_SAMPLE, 48_000, &[r(0, 0), r(step / 2, 0)]).is_err());
    }

    #[test]
    fn wp_athub_bus_check() {
        assert!(same_bus(&[None, Some("1".into()), Some("1".into())]));
        assert!(!same_bus(&[Some("1".into()), Some("2".into())]));
        assert!(same_bus(&[None, None]));
    }

    proptest! {
        #[test]
        fn proptest_wp_athub_origins_recover_any_wrap_and_never_panic(
            base in 0u64..(1 << 50), wraps in proptest::collection::vec(-20i64..20, 1..16),
            jitter in proptest::collection::vec(0u64..2, 16), rate_i in 0usize..3,
            garbage in proptest::collection::vec((any::<u64>(), any::<u64>(), any::<u64>()), 0..8),
        ) {
            let rate = [48_000u32, 96_000, 48_000][rate_i];
            let step = usb_origin_step(rate) as i64;
            let readings: Vec<Reading> = wraps
                .iter()
                .zip(&jitter)
                .map(|(w, j)| Reading {
                    t: (base as i64 + 40 * step + w * step + *j as i64) as u64,
                    mid_us: 0,
                    rtt_us: 0,
                })
                .collect();
            let o = origins(TB_USB_SAMPLE, rate, &readings).unwrap();
            for (i, w) in wraps.iter().enumerate() {
                prop_assert_eq!(o[i], (w - wraps[0]) * step);
            }
            let g: Vec<Reading> = garbage
                .iter()
                .map(|(t, m, r)| Reading { t: *t, mid_us: *m, rtt_us: *r })
                .collect();
            let _ = origins(TB_USB_SAMPLE, rate, &g);
            let _ = origins(TB_SPDIF, 0, &g);
        }

        #[test]
        fn proptest_wp_athub_rate_of_never_panics(
            a in any::<u64>(), b in any::<u64>(), c in any::<u32>(), d in any::<u32>()
        ) {
            if let Some(r) = rate_of((a, c), (b, d)) {
                prop_assert!(RATES.contains(&r));
            }
        }
    }
}
