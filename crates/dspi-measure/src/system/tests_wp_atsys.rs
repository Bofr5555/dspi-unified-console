//! WP ATSYS: the pure parts of the system analyser: the run's layout and
//! its refusals, interleaved passes and the lead, the gate's facts G1 to G4,
//! the offer rule, the transition band, the static rules on a generator
//! configuration; `proptest_*` for each public function. The analyser on
//! captures: `ir::tests_wp_atsys` (P5's worlds, bit for bit) and
//! `tests/atsys_system.rs` (several units in P5SIM's world).
#![allow(clippy::arithmetic_side_effects)]

use dspi_proto::ext::timebase::{TB_STATE_HOLDOVER, TB_STATE_LOCKED, timeline_result};
use dspi_proto::packets::SiggenConfig;
use proptest::prelude::*;

use super::analysis::{name_keys, offer};
use super::*;
use crate::ir::AnalysisOptions;
use crate::stimulus::{
    LogSweep, SIGNAL_LOG_SWEEP, SIGNAL_PINK, SweepConfig, WalkSchedule, check_config_bounds,
};
use crate::types::{LogGrid, Response};

const RATE: u32 = 48_000;
const D: DomainId = DomainId {
    time_base: 0,
    id: 7,
};
const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 1_000,
    gap_ms: 300,
};

fn walk(mask: u16, repeat: u16) -> WalkSchedule {
    WalkSchedule::new(LogSweep::new(RATE, &CFG).unwrap(), mask, repeat).unwrap()
}

fn unit(u: u8, mask: u16) -> UnitWalk {
    UnitWalk {
        unit: UnitIndex(u),
        domain: D,
        walk: walk(mask, 1),
    }
}

fn opts(passes: Option<u16>) -> InterleaveOptions {
    InterleaveOptions {
        passes,
        gap_samples: u64::from(LogSweep::new(RATE, &CFG).unwrap().gap_samples),
        lead_samples: 12_000,
        capture_rate_hz: RATE,
        pre_roll_ms: 1_000,
    }
}

// ---------------------------------------------------------------------------
// Interleaving and the lead
// ---------------------------------------------------------------------------

#[test]
fn wp_atsys_interleave_alternates_units_and_keeps_the_lead() {
    let s = interleave(&[unit(0, 0b11), unit(1, 0b1)], &opts(None)).unwrap();
    // Five passes fit one capture: unit 0, unit 1, unit 0, ...
    assert_eq!(s.segments().len(), 10);
    for (i, seg) in s.segments().iter().enumerate() {
        assert_eq!(seg.unit, UnitIndex((i % 2) as u8));
        assert_eq!(seg.schedule.repeat, 1);
    }
    assert_eq!(s.start(), 0);
    check_lead(&s, 12_000).unwrap();
    let gap = u64::from(CFG.gap_ms) * 48;
    for w in s.segments().windows(2) {
        assert!(w[1].start >= w[0].end() + gap);
    }
    // One unit: its own segments are held a lead apart, not only a gap.
    let mut long_lead = opts(Some(3));
    long_lead.lead_samples = 30_000;
    let one = interleave(&[unit(0, 0b1)], &long_lead).unwrap();
    for w in one.segments().windows(2) {
        assert_eq!(w[1].start, w[0].end() + 30_000);
    }
    check_lead(&one, 30_000).unwrap();
    assert!(check_lead(&one, 30_001).is_err());
}

#[test]
fn wp_atsys_interleave_falls_back_to_three_passes_and_refuses_what_cannot_fit() {
    // Two units of 8 outputs, 1.5 s sweeps: a segment is 14.1 s, five
    // passes (10 segments) are past 120 s, three are 86 s.
    let long = SweepConfig {
        duration_ms: 1_500,
        ..CFG
    };
    let w = |u: u8, cfg: &SweepConfig| UnitWalk {
        unit: UnitIndex(u),
        domain: D,
        walk: WalkSchedule::new(LogSweep::new(RATE, cfg).unwrap(), 0xFF, 1).unwrap(),
    };
    let s = interleave(&[w(0, &long), w(1, &long)], &opts(None)).unwrap();
    assert_eq!(s.segments().len(), 6, "three passes");
    assert!(interleave(&[w(0, &long), w(1, &long)], &opts(Some(5))).is_err());
    let huge = SweepConfig {
        duration_ms: 15_000,
        ..CFG
    };
    let e = interleave(&[w(0, &huge), w(1, &huge)], &opts(None)).unwrap_err();
    assert!(e.to_string().contains("anchor output"), "{e}");
}

#[test]
fn wp_atsys_a_schedule_with_two_domains_is_refused() {
    let mut b = unit(1, 0b1);
    b.domain = DomainId {
        time_base: 0,
        id: 8,
    };
    let e = interleave(&[unit(0, 0b1), b], &opts(None)).unwrap_err();
    assert!(e.to_string().contains("do not share a clock"), "{e}");
    // And at open (G1), by the units' facts.
    let s = interleave(&[unit(0, 0b1), unit(1, 0b1)], &opts(Some(3))).unwrap();
    let mut f = facts(&s);
    f[1].domain = DomainId {
        time_base: 3,
        id: 7,
    };
    let e = check_one_domain(&s, &f).unwrap_err();
    assert!(e.to_string().contains("do not share a clock"), "{e}");
    check_one_domain(&s, &facts(&s)).unwrap();
    // Other refusals.
    assert!(interleave(&[], &opts(None)).is_err());
    assert!(interleave(&[unit(0, 1), unit(0, 2)], &opts(None)).is_err());
    let mut two = unit(0, 1);
    two.walk = walk(1, 2);
    assert!(interleave(&[two], &opts(None)).is_err());
    assert!(interleave(&[unit(0, 1)], &opts(Some(1))).is_err());
    assert!(interleave(&[unit(0, 1)], &opts(Some(9))).is_err());
    let mut short = opts(None);
    short.gap_samples = 10;
    assert!(interleave(&[unit(0, 1)], &short).is_err());
}

#[test]
fn wp_atsys_offset_by_moves_every_start() {
    let s = interleave(&[unit(0, 0b1), unit(1, 0b1)], &opts(Some(2))).unwrap();
    let t = s.offset_by(1 << 50).unwrap();
    for (a, b) in s.segments().iter().zip(t.segments()) {
        assert_eq!(b.start, a.start + (1 << 50));
    }
    assert!(s.offset_by(u64::MAX).is_err());
}

// ---------------------------------------------------------------------------
// The run's layout
// ---------------------------------------------------------------------------

#[test]
fn wp_atsys_runs_the_analyser_cannot_lay_out_are_refused() {
    let o = AnalysisOptions::default();
    // Different sweeps.
    let other = SweepConfig {
        duration_ms: 2_000,
        ..CFG
    };
    let a = Segment {
        unit: UnitIndex(0),
        start: 0,
        schedule: walk(1, 1),
    };
    let b = Segment {
        unit: UnitIndex(1),
        start: 200_000,
        schedule: WalkSchedule::new(LogSweep::new(RATE, &other).unwrap(), 1, 1).unwrap(),
    };
    let s = SystemSchedule::new(D, RATE, vec![a.clone(), b]).unwrap();
    assert!(SystemAnalyser::new(&s, RATE, &o).is_err());
    // Uneven passes: unit 0 twice, unit 1 once.
    let mut c = a.clone();
    c.unit = UnitIndex(1);
    c.start = 100_000;
    let mut d = a.clone();
    d.start = 200_000;
    let s = SystemSchedule::new(D, RATE, vec![a.clone(), c.clone(), d]).unwrap();
    assert!(SystemAnalyser::new(&s, RATE, &o).is_err());
    // Closer than a sweep and its gap.
    let mut e = c.clone();
    e.start = a.end() + 10;
    let s = SystemSchedule::new(D, RATE, vec![a.clone(), e]).unwrap();
    assert!(SystemAnalyser::new(&s, RATE, &o).is_err());
    // More than 16 outputs in one run.
    // (review AT-A/AT-B: the planner refuses it too, before anything plays.)
    let units: Vec<UnitWalk> = (0..3).map(|u| unit(u, 0x3F)).collect();
    let err = interleave(&units, &opts(Some(2))).err().unwrap();
    assert!(err.to_string().contains("anchor output"), "{err}");
    let mut at = 0;
    let mut segs = Vec::new();
    for _ in 0..2 {
        for u in 0..3 {
            let seg = Segment {
                unit: UnitIndex(u),
                start: at,
                schedule: walk(0x3F, 1),
            };
            at = seg.end() + 100_000;
            segs.push(seg);
        }
    }
    let s = SystemSchedule::new(D, RATE, segs).unwrap();
    let err = SystemAnalyser::new(&s, RATE, &o).err().unwrap();
    assert!(err.to_string().contains("anchor output"), "{err}");
    let sixteen: Vec<UnitWalk> = (0..4).map(|u| unit(u, 0xF)).collect();
    assert!(interleave(&sixteen, &opts(Some(2))).is_ok());
    // These lay out.
    let ok = interleave(&[unit(0, 0b11), unit(1, 0b101)], &opts(Some(3))).unwrap();
    assert!(SystemAnalyser::new(&ok, RATE, &o).is_ok());
    let summed = SystemSchedule::summed(
        D,
        RATE,
        0,
        vec![(UnitIndex(0), walk(1, 3)), (UnitIndex(1), walk(4, 3))],
    )
    .unwrap();
    assert!(SystemAnalyser::new(&summed, RATE, &o).is_ok());
    let uneven = SystemSchedule::summed(
        D,
        RATE,
        0,
        vec![(UnitIndex(0), walk(1, 3)), (UnitIndex(1), walk(3, 3))],
    )
    .unwrap();
    assert!(SystemAnalyser::new(&uneven, RATE, &o).is_err());
}

// ---------------------------------------------------------------------------
// The gate's facts
// ---------------------------------------------------------------------------

fn facts(s: &SystemSchedule) -> Vec<UnitFacts> {
    s.units()
        .iter()
        .map(|&u| UnitFacts {
            unit: u,
            domain: D,
            matching_entries: 1,
            t_offset_samples: -3,
            tick_round_trip_samples: 96,
            state_open: TB_STATE_LOCKED,
            state_end: TB_STATE_LOCKED,
            disturbances_open: 65_535,
            disturbances_end: 65_535,
            t_minus_output_open: u32::MAX,
            t_minus_output_end: 0,
            origin: if u.0 == 1 { 1_000_000 } else { 0 },
            events: 0,
        })
        .collect()
}

fn proofs(s: &SystemSchedule, f: &[UnitFacts]) -> Vec<SegmentProof> {
    s.segments()
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let origin = f.iter().find(|x| x.unit == seg.unit).unwrap().origin;
            SegmentProof {
                segment: i,
                expected_run_id: 40 + i as u32,
                run_id: 40 + i as u32,
                start: (seg.start as i64 - origin) as u64,
                last_result: timeline_result::STARTED,
                discontinuities: 0,
                samples: seg.schedule.total_samples() as u32,
            }
        })
        .collect()
}

fn placed() -> SystemSchedule {
    interleave(&[unit(0, 0b11), unit(1, 0b1)], &opts(Some(3)))
        .unwrap()
        .offset_by(5_000_000)
        .unwrap()
}

#[test]
fn wp_atsys_the_gate_passes_a_clean_run() {
    let s = placed();
    let f = facts(&s);
    // t - output_frames wrapped by one sample: within the margin.
    assert!(check_gate(&s, &f, &proofs(&s, &f)).is_empty());
}

#[test]
fn wp_atsys_a_start_one_sample_off_is_discarded_by_g3() {
    let s = placed();
    let f = facts(&s);
    for delta in [1i64, -1] {
        let mut p = proofs(&s, &f);
        p[3].start = (p[3].start as i64 + delta) as u64;
        let g = check_gate(&s, &f, &p);
        assert_eq!(g.len(), 1, "{g:?}");
        assert!(
            matches!(&g[0], GateFinding::ProofMismatch { segment: 3, what } if what.contains("scheduled")),
            "{g:?}"
        );
    }
    // Every other read-back fault, each alone.
    type Edit = Box<dyn Fn(&mut SegmentProof)>;
    let cases: Vec<Edit> = vec![
        Box::new(|p| p.run_id += 1),
        Box::new(|p| p.last_result = timeline_result::REFUSED_LATE),
        Box::new(|p| p.last_result = timeline_result::STOPPED_LOSS),
        Box::new(|p| p.discontinuities = 1),
        Box::new(|p| p.samples -= 1),
    ];
    for c in &cases {
        let mut p = proofs(&s, &f);
        c(&mut p[0]);
        let g = check_gate(&s, &f, &p);
        assert!(
            matches!(
                g.as_slice(),
                [GateFinding::ProofMismatch { segment: 0, .. }]
            ),
            "{g:?}"
        );
    }
    // A segment without its read-back.
    let mut p = proofs(&s, &f);
    p.remove(2);
    assert_eq!(
        check_gate(&s, &f, &p),
        vec![GateFinding::NoProof { segment: 2 }]
    );
}

#[test]
fn wp_atsys_g1_g2_g4_findings_are_each_reported() {
    let s = placed();
    let base = facts(&s);
    let p = proofs(&s, &base);
    let one = |edit: &dyn Fn(&mut UnitFacts)| {
        let mut f = base.clone();
        edit(&mut f[1]);
        check_gate(&s, &f, &p)
    };
    let u = UnitIndex(1);
    assert!(matches!(
        one(&|f| f.matching_entries = 2).as_slice(),
        [GateFinding::EntryNotUnique { unit, entries: 2 }] if *unit == u
    ));
    assert!(matches!(
        one(&|f| f.matching_entries = 0).as_slice(),
        [GateFinding::EntryNotUnique { entries: 0, .. }]
    ));
    assert!(matches!(
        one(&|f| f.t_offset_samples = 97).as_slice(),
        [GateFinding::TimeDisagrees { .. }]
    ));
    assert!(one(&|f| f.t_offset_samples = -96).is_empty());
    assert!(matches!(
        one(&|f| f.state_end = TB_STATE_HOLDOVER).as_slice(),
        [GateFinding::NotLocked { at_end: true, .. }]
    ));
    assert!(matches!(
        one(&|f| f.state_open = 1).as_slice(),
        [GateFinding::NotLocked { at_end: false, .. }]
    ));
    assert!(matches!(
        one(&|f| f.disturbances_end = 0).as_slice(),
        [GateFinding::Disturbed { .. }]
    ));
    assert!(matches!(
        one(&|f| f.t_minus_output_end = 1).as_slice(),
        [GateFinding::TimelineMoved { by: 2, .. }]
    ));
    assert!(one(&|f| f.t_minus_output_end = u32::MAX).is_empty());
    assert!(matches!(
        one(&|f| f.events = 1).as_slice(),
        [GateFinding::Events { count: 1, .. }]
    ));
    assert!(matches!(
        one(&|f| f.domain.id = 9).as_slice(),
        [GateFinding::NotOneDomain { .. }]
    ));
    // The origin maps the unit's own numbering: changing it alone moves
    // every one of its read-backs off the schedule.
    let g = one(&|f| f.origin += 1);
    assert!(!g.is_empty());
    assert!(
        g.iter()
            .all(|g| matches!(g, GateFinding::ProofMismatch { .. }))
    );
    // A unit without facts.
    let g = check_gate(&s, &base[..1], &p);
    assert!(g.contains(&GateFinding::NoFacts { unit: u }));
    for g in one(&|f| f.events = 3) {
        assert!(g.to_string().contains("unit 1"), "{g}");
    }
}

// ---------------------------------------------------------------------------
// The offer rule, the names, the transition band
// ---------------------------------------------------------------------------

#[test]
fn wp_atsys_a_difference_above_10_us_is_not_offered() {
    assert_eq!(
        offer(true, Some(9.99), None, "a", "b"),
        (true, String::new())
    );
    assert!(offer(true, Some(10.0), None, "a", "b").0);
    let (ok, why) = offer(
        true,
        Some(10.01),
        None,
        "unit 0 output 0",
        "unit 1 output 0",
    );
    assert!(!ok);
    assert!(
        why.contains("wandered 10.0 µs between unit 0 output 0 and unit 1 output 0"),
        "{why}"
    );
    let (ok, why) = offer(true, None, None, "a", "b");
    assert!(!ok && why.contains("two clear passes"), "{why}");
    let (ok, why) = offer(false, Some(1.0), Some("a capture slip"), "a", "b");
    assert!(!ok && why.contains("a capture slip"), "{why}");
}

fn sys_output(key: u8, unit: u8, output: u8) -> SystemOutput {
    SystemOutput {
        key,
        members: vec![UnitOutput {
            unit: UnitIndex(unit),
            output,
        }],
        intercept_frame: 0.0,
        arrival_ms: 0.0,
        onset_ms: 0.0,
        passes: 3,
        uncertainty_us: None,
        lateness_ms: 0.0, // -- WP ATLATE --
    }
}

#[test]
fn wp_atsys_reasons_name_units_and_outputs() {
    let outs = [sys_output(0, 0, 2), sys_output(1, 3, 0)];
    assert_eq!(
        name_keys("cycle 4 (output 1): its arrival is ambiguous", &outs),
        "cycle 4 (unit 3 output 0): its arrival is ambiguous"
    );
    assert_eq!(
        name_keys("output 0: too quiet; output 7: unknown; output x", &outs),
        "unit 0 output 2: too quiet; output 7: unknown; output x"
    );
    assert_eq!(name_keys("no outputs here", &outs), "no outputs here");
}

/// A response on a coarse grid: `level` dB inside `band`, falling 24 dB per
/// octave outside.
fn shaped(band: (f64, f64), level: f64) -> Response {
    let grid = LogGrid {
        min_hz: 10.0,
        max_hz: 22_000.0,
        points_per_octave: 12,
    };
    let mag_db = grid
        .freqs()
        .unwrap()
        .iter()
        .map(|&f| {
            let below = (band.0 / f).log2().max(0.0);
            let above = (f / band.1).log2().max(0.0);
            level - 24.0 * (below + above)
        })
        .collect();
    Response {
        grid,
        mag_db,
        phase_deg: None,
        valid: None,
        spl: false,
    }
}

#[test]
fn wp_atsys_the_transition_band_is_the_overlap_limited_by_the_uncertainty() {
    let sub = shaped((20.0, 100.0), -10.0);
    let mains = shaped((80.0, 20_000.0), 0.0);
    let (lo, hi) = transition_band(&sub, &mains, None).unwrap();
    // Within 10 dB of each passband: the sub to 100·2^(10/24) Hz, the mains
    // from 80·2^(-10/24) Hz.
    assert!(lo > 55.0 && lo < 65.0, "{lo}");
    assert!(hi > 125.0 && hi < 140.0, "{hi}");
    // Two full-range outputs: the common band, limited to 1/(8u) (6.25 kHz
    // at 20 µs).
    let a = shaped((40.0, 20_000.0), 0.0);
    let b = shaped((30.0, 18_000.0), -3.0);
    let (_, hi) = transition_band(&a, &b, Some(20.0)).unwrap();
    assert!((hi - 6_250.0).abs() < 1e-6, "{hi}");
    // Less than an octave: none.
    let narrow = shaped((1_000.0, 1_200.0), 0.0);
    assert!(transition_band(&narrow, &shaped((4_000.0, 8_000.0), 0.0), None).is_none());
    // A grid mismatch: none.
    let mut g = mains.clone();
    g.grid.points_per_octave = 24;
    assert!(transition_band(&sub, &g, None).is_none());
    assert_eq!(passband(&mains).map(|p| p.1 > 20_000.0), Some(true));
    assert_eq!(passband_level_db(&sub), Some(-10.0));
}

// ---------------------------------------------------------------------------
// The static rules on a generator configuration
// ---------------------------------------------------------------------------

fn sweep_config() -> SiggenConfig {
    SiggenConfig {
        version: 1,
        signal_type: SIGNAL_LOG_SWEEP,
        channel_mask: 0b0110,
        invert_mask: 0b0010,
        flags: SiggenConfig::FLAG_WALK,
        level_db: -30.0,
        duration_ms: 5_500,
        repeat_count: 1,
        gap_ms: 1_000,
        p1: 20.0,
        p2: 20_000.0,
        p3: 0.0,
        p4: 0.0,
    }
}

#[test]
fn wp_atsys_check_config_bounds_refuses_every_broken_rule() {
    check_config_bounds(&sweep_config(), 0).unwrap();
    check_config_bounds(&sweep_config(), 0b1001).unwrap();
    let pink = SiggenConfig {
        signal_type: SIGNAL_PINK,
        channel_mask: 0b1,
        invert_mask: 0,
        flags: 0,
        duration_ms: 60_000,
        ..sweep_config()
    };
    check_config_bounds(&pink, 0).unwrap();
    // (review AT-A/AT-B) What op 9 starts is a sweep: the firmware never
    // starts pink noise on the time base.
    crate::stimulus::check_timed_config(&sweep_config(), 0).unwrap();
    assert!(crate::stimulus::check_timed_config(&sweep_config(), 0b0100).is_err());
    let e = crate::stimulus::check_timed_config(&pink, 0).unwrap_err();
    assert!(e.to_string().contains("log sweep"), "{e}");
    let s = sweep_config;
    let broken: Vec<(&str, SiggenConfig)> = vec![
        ("version", SiggenConfig { version: 2, ..s() }),
        (
            "RAW",
            SiggenConfig {
                flags: SiggenConfig::FLAG_RAW,
                ..s()
            },
        ),
        ("flags", SiggenConfig { flags: 0x80, ..s() }),
        (
            "level",
            SiggenConfig {
                level_db: -5.9,
                ..s()
            },
        ),
        (
            "level",
            SiggenConfig {
                level_db: f32::NAN,
                ..s()
            },
        ),
        (
            "level",
            SiggenConfig {
                level_db: -121.0,
                ..s()
            },
        ),
        (
            "no output",
            SiggenConfig {
                channel_mask: 0,
                invert_mask: 0,
                ..s()
            },
        ),
        (
            "inverted",
            SiggenConfig {
                invert_mask: 0b1000,
                ..s()
            },
        ),
        (
            "sweep",
            SiggenConfig {
                repeat_count: 0,
                ..s()
            },
        ),
        (
            "sweep",
            SiggenConfig {
                repeat_count: 9,
                ..s()
            },
        ),
        (
            "sweep",
            SiggenConfig {
                duration_ms: 999,
                ..s()
            },
        ),
        (
            "sweep",
            SiggenConfig {
                duration_ms: 15_001,
                ..s()
            },
        ),
        ("frequencies", SiggenConfig { p1: 0.5, ..s() }),
        (
            "frequencies",
            SiggenConfig {
                p2: f32::INFINITY,
                ..s()
            },
        ),
        (
            "frequencies",
            SiggenConfig {
                p1: 2_000.0,
                p2: 1_000.0,
                ..s()
            },
        ),
        (
            "pink",
            SiggenConfig {
                duration_ms: 60_001,
                ..pink.clone()
            },
        ),
        (
            "pink",
            SiggenConfig {
                duration_ms: 0,
                ..pink.clone()
            },
        ),
        (
            "pink",
            SiggenConfig {
                channel_mask: 0b11,
                ..pink.clone()
            },
        ),
        (
            "pink",
            SiggenConfig {
                flags: SiggenConfig::FLAG_WALK,
                ..pink.clone()
            },
        ),
        (
            "only",
            SiggenConfig {
                signal_type: 0,
                ..s()
            },
        ),
        (
            "only",
            SiggenConfig {
                signal_type: 5,
                ..s()
            },
        ),
    ];
    for (what, c) in broken {
        let e = check_config_bounds(&c, 0).unwrap_err();
        assert!(e.to_string().contains(what), "{what}: {e}");
    }
    // An S/PDIF source's time-base slot in the mask (S-AT6).
    let e = check_config_bounds(&sweep_config(), 0b0100).unwrap_err();
    assert!(e.to_string().contains("time base"), "{e}");
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

fn arb_config() -> impl Strategy<Value = SiggenConfig> {
    (
        any::<[u8; 36]>(),
        prop_oneof![Just(None), (-200.0f32..10.0).prop_map(Some)],
        prop_oneof![Just(None), (3u8..5).prop_map(Some)],
    )
        .prop_map(|(b, level, kind)| {
            let mut c = SiggenConfig::decode(&b).unwrap();
            if let Some(l) = level {
                c.level_db = l;
            }
            if let Some(k) = kind {
                c.signal_type = k;
                c.version = 1;
            }
            c
        })
}

fn arb_facts() -> impl Strategy<Value = UnitFacts> {
    (
        (0u8..4, any::<u8>(), any::<u32>(), 0u8..3),
        (any::<i64>(), any::<u64>(), 0u8..6, 0u8..6),
        (any::<u16>(), any::<u16>(), any::<u32>(), any::<u32>()),
        (any::<i64>(), 0u32..3),
    )
        .prop_map(
            |((u, tb, id, me), (to, rt, so, se), (d0, d1, m0, m1), (o, ev))| UnitFacts {
                unit: UnitIndex(u),
                domain: DomainId { time_base: tb, id },
                matching_entries: me,
                t_offset_samples: to,
                tick_round_trip_samples: rt,
                state_open: so,
                state_end: se,
                disturbances_open: d0,
                disturbances_end: d1,
                t_minus_output_open: m0,
                t_minus_output_end: m1,
                origin: o,
                events: ev,
            },
        )
}

fn arb_proof() -> impl Strategy<Value = SegmentProof> {
    (
        0usize..12,
        any::<u32>(),
        any::<u32>(),
        any::<u64>(),
        any::<u8>(),
        any::<u16>(),
        any::<u32>(),
    )
        .prop_map(|(segment, e, r, start, lr, dis, samples)| SegmentProof {
            segment,
            expected_run_id: e,
            run_id: r,
            start,
            last_result: lr,
            discontinuities: dis,
            samples,
        })
}

fn arb_schedule() -> impl Strategy<Value = SystemSchedule> {
    (
        1usize..4,
        1u16..8,
        prop_oneof![Just(None), (2u16..6).prop_map(Some)],
        any::<u64>(),
    )
        .prop_map(|(n, mask, passes, at)| {
            let units: Vec<UnitWalk> = (0..n).map(|u| unit(u as u8, mask)).collect();
            let s = interleave(&units, &opts(passes)).unwrap();
            s.offset_by(at / 2).unwrap()
        })
}

fn arb_response() -> impl Strategy<Value = Response> {
    (
        proptest::collection::vec(prop_oneof![-200.0f64..50.0, Just(f64::NAN)], 0..200),
        proptest::option::of(proptest::collection::vec(any::<bool>(), 0..200)),
        1u16..48,
    )
        .prop_map(|(mag_db, valid, ppo)| Response {
            grid: LogGrid {
                min_hz: 20.0,
                max_hz: 20_000.0,
                points_per_octave: ppo,
            },
            mag_db,
            phase_deg: None,
            valid,
            spl: false,
        })
}

proptest! {
    #[test]
    fn proptest_wp_atsys_check_config_bounds_never_panics(c in arb_config(), forbidden in any::<u16>()) {
        if check_config_bounds(&c, forbidden).is_ok() {
            prop_assert!(c.level_db.is_finite() && c.level_db <= -6.0);
            prop_assert!(c.flags & SiggenConfig::FLAG_RAW == 0);
            prop_assert!(c.channel_mask & forbidden == 0);
            prop_assert!(c.signal_type == SIGNAL_LOG_SWEEP || c.signal_type == SIGNAL_PINK);
        }
    }

    #[test]
    fn proptest_wp_atsys_the_gate_never_panics(
        s in arb_schedule(),
        f in proptest::collection::vec(arb_facts(), 0..5),
        p in proptest::collection::vec(arb_proof(), 0..12),
    ) {
        let g = check_gate(&s, &f, &p);
        for x in &g {
            prop_assert!(!x.to_string().is_empty());
        }
        let _ = check_one_domain(&s, &f);
    }

    #[test]
    fn proptest_wp_atsys_interleave_and_check_lead_never_panic(
        n in 0usize..5,
        mask in any::<u16>(),
        passes in proptest::option::of(0u16..12),
        gap in any::<u64>(),
        lead in any::<u64>(),
        cap in prop_oneof![Just(0u32), 8_000u32..200_000],
        pre in any::<u32>(),
        by in any::<u64>(),
    ) {
        let units: Vec<UnitWalk> = (0..n).map(|u| unit(u as u8, mask.max(1))).collect();
        let o = InterleaveOptions {
            passes,
            gap_samples: gap,
            lead_samples: lead,
            capture_rate_hz: cap,
            pre_roll_ms: pre,
        };
        if let Ok(s) = interleave(&units, &o) {
            prop_assert!(check_lead(&s, lead).is_ok());
            prop_assert!(s.segments().len() >= 2 * n);
            let _ = s.offset_by(by);
        }
    }

    #[test]
    fn proptest_wp_atsys_transition_band_and_passband_never_panic(
        a in arb_response(),
        b in arb_response(),
        u in proptest::option::of(prop_oneof![-1.0f64..1e6, Just(f64::NAN), Just(0.0)]),
    ) {
        if let Some((lo, hi)) = transition_band(&a, &b, u) {
            prop_assert!(lo.is_finite() && hi.is_finite() && hi >= 2.0 * lo * 0.999_999);
        }
        if let Some((lo, hi)) = passband(&a) {
            prop_assert!(lo.is_finite() && hi >= lo);
        }
        if let Some(l) = passband_level_db(&a) {
            prop_assert!(l.is_finite());
        }
    }

    #[test]
    fn proptest_wp_atsys_name_keys_and_offer_never_panic(
        text in ".{0,80}",
        keys in proptest::collection::vec((any::<u8>(), any::<u8>(), any::<u8>()), 0..4),
        u in proptest::option::of(any::<f64>()),
        reliable in any::<bool>(),
    ) {
        let outs: Vec<SystemOutput> = keys.iter().map(|&(k, un, o)| sys_output(k, un, o)).collect();
        let _ = name_keys(&text, &outs);
        let _ = name_keys(&format!("output {text}output 0"), &outs);
        let (ok, why) = offer(reliable, u, Some(&text), "a", "b");
        prop_assert!(ok == why.is_empty());
    }
}

/// The analyser on arbitrary captures: never a panic, never a non-finite
/// number in what it returns. Each case analyses a small run of noise,
/// silence or a repeated pattern: 8 cases (at most 24 under
/// `ci/check.sh --step proptest`).
#[test]
fn proptest_wp_atsys_analyse_system_never_panics_or_returns_non_finite() {
    let strategy = (
        1usize..3,
        prop_oneof![Just(None), Just(Some(2u16))],
        proptest::collection::vec(-1.0f32..1.0, 0..64),
        any::<u64>(),
        0u64..100_000,
        any::<bool>(),
        prop_oneof![Just(44_100u32), Just(48_000), Just(1u32)],
    );
    let mut config = proptest::test_runner::contextualize_config(ProptestConfig::with_cases(8));
    config.cases = config.cases.min(24);
    let mut runner = proptest::test_runner::TestRunner::new(config);
    let r = runner.run(&strategy, |(n, passes, pattern, t, rt, summed, rate)| {
        let s = if summed {
            SystemSchedule::summed(
                D,
                RATE,
                t / 2,
                (0..n.max(2))
                    .map(|u| (UnitIndex(u as u8), walk(1, 2)))
                    .collect(),
            )
            .unwrap()
        } else {
            let units: Vec<UnitWalk> = (0..n).map(|u| unit(u as u8, 1)).collect();
            interleave(&units, &opts(passes))
                .unwrap()
                .offset_by(t / 2)
                .unwrap()
        };
        let len = 48_000usize * 12;
        let samples: Vec<f32> = (0..len)
            .map(|i| {
                pattern
                    .get(i % pattern.len().max(1))
                    .copied()
                    .unwrap_or(0.0)
            })
            .collect();
        let pins = [TimePin {
            capture_frame: 48_000,
            t: s.start(),
            round_trip_frames: rt,
        }];
        let cap = SystemCapture {
            samples: &samples,
            first_frame: 0,
            rate_hz: rate,
            schedule: &s,
            pins: &pins,
            status: &[],
            complete: true,
        };
        if let Ok(a) = analyse_system(&cap, &AnalysisOptions::default()) {
            for o in &a.outputs {
                prop_assert!(o.arrival_ms.is_finite() && o.onset_ms.is_finite());
                prop_assert!(o.uncertainty_us.is_none_or(f64::is_finite));
            }
            for d in &a.delays {
                prop_assert!(d.delay_ms.is_finite());
            }
        }
        Ok(())
    });
    if let Err(e) = r {
        panic!("{e}");
    }
}
