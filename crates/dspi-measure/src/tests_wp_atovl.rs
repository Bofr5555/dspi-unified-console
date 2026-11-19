//! WP ATOVL (owner 2026-11-19, PLAN-AT 1.4 "overlapped starts"): unit B's
//! sweep a fixed 50-200 ms after unit A's, in one capture and one
//! deconvolution per group, so that the capture clock's wander is common to
//! what plays together. ATSIM renders, ATSYS analyses.
//!
//! - The plan: the offset is the smallest that keeps every unit's impulse
//!   and harmonic images out of every other unit's analysed window, and a
//!   plan where none does is refused (the session then interleaves).
//! - The wander study of WP ATE2E, extended: with overlapped starts
//!   P5SPIKE's wander at x1 and x0.3 gives reliable timing in most seeds,
//!   and everything offered is within 3 of its uncertainties and 10 µs.
//!   The suite runs a few seeds; the table runs in the ignored
//!   `wp_atovl_wander_table` (release build).
//! - Fail closed: a unit heard further from its place than the plan allows
//!   is refused as not separable; two outputs of one unit (never played
//!   together) keep P5's absolute gate.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use crate::ir::{AnalysisOptions, TimingError};
use crate::sim::system::{SimUnit, SystemRender, SystemWorld, WanderBursts, render_system};
use crate::sim::{Microphone, ProtoModel, World};
use crate::stimulus::{LogSweep, SweepConfig, WalkSchedule};
use crate::system::plan::{overlap_conflict, overlap_offset};
use crate::system::{
    DomainId, InterleaveOptions, SystemAnalysis, SystemCapture, SystemSchedule, TimePin, UnitIndex,
    UnitOutput, UnitWalk, analyse_system, check_lead, interleave, overlap_late,
};

const RATE: u32 = 48_000;
const D: DomainId = DomainId {
    time_base: 0,
    id: 0x0E,
};
const ORIGIN: u64 = 2_000_000_000;
const PRE: u64 = 48_000;

fn sweep_of(duration_ms: u32, gap_ms: u16) -> LogSweep {
    LogSweep::new(
        RATE,
        &SweepConfig {
            f1_hz: 20.0,
            f2_hz: 20_000.0,
            duration_ms,
            gap_ms,
        },
    )
    .unwrap()
}

/// ATE2E's sweep: 1 s, 300 ms gaps.
fn sweep() -> LogSweep {
    sweep_of(1_000, 300)
}

fn walks(units: &[(u8, u16)], s: LogSweep) -> Vec<UnitWalk> {
    units
        .iter()
        .map(|&(u, mask)| UnitWalk {
            unit: UnitIndex(u),
            domain: D,
            walk: WalkSchedule::new(s, mask, 1).unwrap(),
        })
        .collect()
}

fn options(s: LogSweep, passes: u16) -> InterleaveOptions {
    InterleaveOptions {
        passes: Some(passes),
        gap_samples: u64::from(s.gap_samples),
        lead_samples: u64::from(RATE) / 4,
        capture_rate_hz: RATE,
        pre_roll_ms: 1_000,
    }
}

fn overlapped(units: &[(u8, u16)], passes: u16, at: u64) -> SystemSchedule {
    let s = sweep();
    overlap_late(&walks(units, s), &options(s, passes), &[])
        .unwrap()
        .offset_by(at)
        .unwrap()
}

fn interleaved(units: &[(u8, u16)], passes: u16, at: u64) -> SystemSchedule {
    let s = sweep();
    interleave(&walks(units, s), &options(s, passes))
        .unwrap()
        .offset_by(at)
        .unwrap()
}

/// ATE2E's room: three_way_room's modes and reflections, a typical
/// microphone 31 ppm fast, noise at -80 dBFS.
fn room(units: Vec<SimUnit>, seed: u64, wander: Option<WanderBursts>) -> SystemWorld {
    let mut w = SystemWorld::free_field(units, D, ORIGIN, RATE);
    w.acoustics = World::three_way_room(seed).acoustics;
    w.mic = Microphone::typical();
    w.noise_dbfs = Some(-80.0);
    w.clock.ppm = 31.0;
    w.seed = seed;
    w.wander = wander;
    w
}

fn unit(name: &str, outputs: usize, playout: f64, delays_ms: &[f64]) -> SimUnit {
    let mut u = SimUnit::flat(name, outputs);
    u.playout_frames = playout;
    for (p, d) in u.outputs.iter_mut().zip(delays_ms) {
        p.delay_ms = *d;
        p.gain_db = -6.0;
    }
    u
}

fn try_analyse(r: &SystemRender, schedule: &SystemSchedule) -> Result<SystemAnalysis, TimingError> {
    let at = |t: u64| r.capture_frame_at_mic(t as f64 - ORIGIN as f64).unwrap();
    let pin = TimePin {
        capture_frame: at(schedule.start()).round() as u64,
        t: schedule.start(),
        round_trip_frames: 48,
    };
    let first = r.capture.blocks.first().map_or(0, |b| b.first_frame);
    let from = (at(schedule.start().saturating_sub(u64::from(RATE))) as u64).max(first);
    let to = (at(schedule.end() + 2 * u64::from(RATE)) as u64)
        .min(first + r.capture.samples.len() as u64);
    let samples = &r.capture.samples[(from - first) as usize..(to - first) as usize];
    analyse_system(
        &SystemCapture {
            samples,
            first_frame: from,
            rate_hz: RATE,
            schedule,
            pins: &[pin],
            status: &[],
            complete: true,
        },
        &AnalysisOptions::default(),
    )
}

/// One two-unit run's cross-unit delay against ATSIM's truth.
#[derive(Debug, Clone, Copy)]
struct Outcome {
    reliable: bool,
    offered: bool,
    err_us: f64,
    u_us: Option<f64>,
    reason_unreliable: bool,
}

/// ATE2E's two units (one output each, 6 µs apart at the microphone).
fn two_units_world(seed: u64, wander: Option<WanderBursts>) -> SystemWorld {
    room(
        vec![
            unit("A", 1, 305.0, &[3.0]),
            unit("B", 1, 257.0 + 48.0, &[5.37]),
        ],
        seed,
        wander,
    )
}

fn outcome(a: &SystemAnalysis, r: &SystemRender) -> Outcome {
    let d = &a.delays[0];
    let at = |o: UnitOutput| r.truth.output(o.unit, o.output).unwrap().arrival_us;
    let truth = at(d.to) - at(d.from);
    Outcome {
        reliable: a.timing_reliable,
        offered: d.offered,
        err_us: d.delay_ms * 1000.0 - truth,
        u_us: d.uncertainty_us,
        reason_unreliable: d.reason.starts_with("timing unreliable"),
    }
}

fn two_units(seed: u64, passes: u16, wander: Option<WanderBursts>, overlap: bool) -> Outcome {
    let s = if overlap {
        overlapped(&[(0, 1), (1, 1)], passes, ORIGIN + PRE)
    } else {
        interleaved(&[(0, 1), (1, 1)], passes, ORIGIN + PRE)
    };
    let r = render_system(&two_units_world(seed, wander), &s, &ProtoModel).unwrap();
    let a = try_analyse(&r, &s).unwrap();
    assert_eq!(a.overlap_ms.is_some(), overlap);
    outcome(&a, &r)
}

/// P5SPIKE's bursts scaled by `k` (rate and bound together).
fn scaled(seed: u64, k: f64) -> WanderBursts {
    let w = WanderBursts::p5spike(seed);
    WanderBursts {
        min_ppm: w.min_ppm * k,
        max_ppm: w.max_ppm * k,
        bound_us: w.bound_us * k,
        ..w
    }
}

/// `(reliable, offered, rms error of the offered µs, worst µs)` over
/// `seeds` at wander scale `k` (0: none); every offered delay held to R8c.
fn study(
    k: f64,
    passes: u16,
    overlap: bool,
    seeds: std::ops::Range<u64>,
) -> (usize, usize, f64, f64) {
    let (mut pass, mut offer, mut sq, mut worst) = (0, 0, 0.0, 0.0f64);
    for seed in seeds {
        let w = (k > 0.0).then(|| scaled(seed, k));
        let o = two_units(seed, passes, w, overlap);
        println!("k {k} overlap {overlap} seed {seed}: {o:?}");
        pass += usize::from(o.reliable);
        if o.offered {
            offer += 1;
            sq += o.err_us * o.err_us;
            worst = worst.max(o.err_us.abs());
            let u = o.u_us.unwrap();
            assert!(
                o.reliable,
                "k {k} seed {seed}: offered from an unreliable run"
            );
            assert!(
                o.err_us.abs() <= 3.0 * u && o.err_us.abs() <= 10.0,
                "k {k} seed {seed}: {o:?}"
            );
        } else {
            assert!(
                o.reason_unreliable,
                "k {k} seed {seed}: not offered, but why: {o:?}"
            );
        }
    }
    let rms = if offer > 0 {
        (sq / offer as f64).sqrt()
    } else {
        0.0
    };
    (pass, offer, rms, worst)
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

#[test]
fn wp_atovl_the_offset_keeps_impulses_and_harmonic_images_apart() {
    // Two units of a 1 s sweep with 300 ms gaps: the smallest offset.
    let s = sweep();
    let gap = u64::from(s.gap_samples);
    assert_eq!(overlap_offset(&s, 2, gap).unwrap(), 2_400, "50 ms");
    // A third unit 100 ms after the first would put its second harmonic's
    // image (100.3 ms early on this sweep) on the first unit's arrival: no
    // offset from 50 to 200 ms keeps three apart; the plan is refused.
    let e = overlap_offset(&s, 3, gap).unwrap_err().to_string();
    assert!(e.contains("overlapped starts do not fit"), "{e}");
    assert!(
        overlap_conflict(&s, 3, 2_880, gap)
            .unwrap()
            .contains("harmonic")
    );
    // A 5.5 s sweep (the speakers' default, 1 s gaps) overlaps four units.
    let long = sweep_of(5_500, 1_000);
    let d = overlap_offset(&long, 4, u64::from(long.gap_samples)).unwrap();
    assert!((2_400..=9_600).contains(&d), "{d}");
    assert!(overlap_conflict(&long, 4, d, u64::from(long.gap_samples)).is_none());
    // Too many units for the gap.
    assert!(overlap_offset(&long, 16, u64::from(long.gap_samples)).is_err());
    // One unit has nothing to overlap.
    assert!(overlap_offset(&s, 1, gap).is_err());
}

#[test]
fn wp_atovl_the_plan_interleaves_its_first_pass_and_keeps_the_hubs_rules() {
    let s = sweep();
    let sch = overlap_late(&walks(&[(0, 0b11), (1, 0b11)], s), &options(s, 3), &[]).unwrap();
    assert_eq!(sch.overlap_samples(), Some(2_400));
    // Two units, 1 interleaved pass and 3 overlapped: 8 segments, starts in
    // order, a unit's segments the lead apart.
    assert_eq!(sch.segments().len(), 8);
    assert!(check_lead(&sch, u64::from(RATE) / 4).is_ok());
    let starts: Vec<(u8, u64)> = sch.segments().iter().map(|g| (g.unit.0, g.start)).collect();
    // Pass 1: B after A's walk and a gap; then A and B 50 ms apart.
    let walk = WalkSchedule::new(s, 0b11, 1).unwrap().total_samples();
    let (_, a1) = starts[0];
    let (_, b1) = starts[1];
    assert_eq!(b1 - a1, walk + u64::from(s.gap_samples));
    let (ua, a2) = starts[2];
    let (ub, b2) = starts[3];
    assert_eq!((ua, ub), (0, 1));
    assert_eq!(b2 - a2, 2_400);
    // Shorter than interleaving the same passes (plus one).
    let il = interleave(&walks(&[(0, 0b11), (1, 0b11)], s), &options(s, 4)).unwrap();
    assert!(sch.end() < il.end(), "{} {}", sch.end(), il.end());
    // Lateness: a unit expected 30 ms late starts that much earlier, so the
    // two are heard the offset apart.
    let late = overlap_late(
        &walks(&[(0, 1), (1, 1)], s),
        &options(s, 3),
        &[(UnitIndex(1), 1_440)],
    )
    .unwrap();
    let g = late.segments();
    let a = g.iter().filter(|x| x.unit.0 == 0).nth(1).unwrap().start;
    let b = g.iter().filter(|x| x.unit.0 == 1).nth(1).unwrap().start;
    assert_eq!(b + 1_440 - a, 2_400);
    assert_eq!(late.lateness_of(UnitIndex(1)), 1_440);
    // Three units of this sweep cannot overlap: refused before anything
    // plays (the session interleaves instead).
    assert!(overlap_late(&walks(&[(0, 1), (1, 1), (2, 1)], s), &options(s, 3), &[]).is_err());
}

#[test]
fn wp_atovl_overlapped_starts_without_wander_measure_as_interleaved() {
    // A steady 31 ppm microphone in the room: the delay within a µs and
    // offered, from the overlapped run; responses kept on the shorter window.
    let o = two_units(3, 3, None, true);
    println!("overlapped, no wander: {o:?}");
    assert!(o.reliable && o.offered, "{o:?}");
    assert!(o.err_us.abs() < 1.0, "{o:?}");
    let i = two_units(3, 3, None, false);
    assert!(
        i.offered && (o.err_us - i.err_us).abs() < 1.0,
        "{o:?} {i:?}"
    );
}

#[test]
fn wp_atovl_p5spike_wander_now_gives_reliable_timing_in_most_seeds() {
    // P5SPIKE's own microphone (x1) and a third of it: overlapped starts
    // pass the gate in most seeds, and what is offered holds (R8c).
    for k in [1.0, 0.3] {
        let (pass, offer, rms, worst) = study(k, 3, true, 200..204);
        println!(
            "k {k}: {pass}/4 reliable, {offer}/4 offered, rms {rms:.2} µs, worst {worst:.2} µs"
        );
        assert!(
            pass >= 3 && offer >= 3,
            "k {k}: {pass} reliable, {offer} offered"
        );
    }
    // The same world interleaved: refused, as WP ATE2E measured.
    let (pass, offer, ..) = study(1.0, 3, false, 200..202);
    assert_eq!((pass, offer), (0, 0));
}

/// The table for the owner (`ci/run.sh 'cargo test --release -p
/// dspi-measure --features sim --lib wp_atovl_wander_table -- --ignored
/// --nocapture'`).
#[test]
#[ignore = "WP ATOVL: the wander table, minutes in a release build"]
fn wp_atovl_wander_table() {
    const N: u64 = 30;
    println!("plan          scale  passes  reliable  offered  rms-err  worst");
    for (overlap, name) in [(true, "overlapped "), (false, "interleaved")] {
        for passes in [3u16, 5] {
            for k in [1.0, 0.5, 0.3, 0.1, 0.0] {
                let (pass, offer, rms, worst) = study(k, passes, overlap, 0..N);
                println!(
                    "{name}  {k:<5}  {passes}       {pass:>2}/{N}     {offer:>2}/{N}   {rms:>5.2}us  {worst:>5.2}us"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fail closed
// ---------------------------------------------------------------------------

#[test]
fn wp_atovl_a_unit_heard_further_from_its_place_than_planned_is_not_separable() {
    // Unit B 30 ms further than planned (its path, not in its terms): the
    // narrow windows refuse the run as not separable; interleaved passes
    // measure it.
    let s = overlapped(&[(0, 1), (1, 1)], 3, ORIGIN + PRE);
    let world = room(
        vec![unit("A", 1, 305.0, &[3.0]), unit("B", 1, 305.0, &[33.0])],
        5,
        None,
    );
    let r = render_system(&world, &s, &ProtoModel).unwrap();
    let e = try_analyse(&r, &s).unwrap_err();
    assert!(matches!(e, TimingError::NotSeparable(_)), "{e:?}");
    let s = interleaved(&[(0, 1), (1, 1)], 3, ORIGIN + PRE);
    let r = render_system(&world, &s, &ProtoModel).unwrap();
    let a = try_analyse(&r, &s).unwrap();
    assert!(a.delays[0].offered, "{:?}", a.delays[0]);
}

#[test]
fn wp_atovl_outputs_that_never_played_together_keep_the_absolute_gate() {
    // Two units of two outputs each under P5SPIKE's wander: the units'
    // first outputs play together, so do their second; what played together
    // is judged on what its group measured, two outputs of one unit (and A's
    // first against B's second) on the absolute gate, which the wander fails.
    let s = overlapped(&[(0, 0b11), (1, 0b11)], 3, ORIGIN + PRE);
    let world = room(
        vec![
            unit("A", 2, 305.0, &[3.0, 3.4]),
            unit("B", 2, 305.0, &[5.37, 5.9]),
        ],
        11,
        Some(WanderBursts::p5spike(11)),
    );
    let r = render_system(&world, &s, &ProtoModel).unwrap();
    let a = try_analyse(&r, &s).unwrap();
    println!(
        "reliable {} absolute {} {:?}",
        a.timing_reliable, a.absolute_reliable, a.absolute_reasons
    );
    assert!(!a.absolute_reliable);
    assert!(!a.same_unit_offer().0);
    let at = |o: UnitOutput| r.truth.output(o.unit, o.output).unwrap().arrival_us;
    for d in &a.delays {
        let together = d.from.output == d.to.output;
        if !together {
            assert!(!d.offered, "{d:?}");
            assert!(d.reason.starts_with("timing unreliable"), "{d:?}");
        } else if d.offered {
            let err = d.delay_ms * 1000.0 - (at(d.to) - at(d.from));
            let u = d.uncertainty_us.unwrap();
            assert!(err.abs() <= 3.0 * u && err.abs() <= 10.0, "{d:?} {err}");
        }
    }
}

#[test]
fn wp_atovl_two_outputs_a_unit_without_wander_offer_every_pair() {
    // Two units of two outputs each, a steady microphone, in the room and
    // in the free field: the overlapped gate and the absolute one pass, so
    // every cross-unit pair (played together or not) and two outputs of one
    // unit may be used, each within 1 µs of the truth.
    let s = overlapped(&[(0, 0b11), (1, 0b11)], 3, ORIGIN + PRE);
    for seed in [1u64, 2] {
        let mut world = room(
            vec![
                unit("A", 2, 0.0, &[5.5, 5.9]),
                unit("B", 2, 0.0, &[4.7, 6.6]),
            ],
            seed,
            None,
        );
        if seed > 1 {
            let ff = SystemWorld::free_field(world.units.clone(), D, ORIGIN, RATE);
            world.acoustics = ff.acoustics;
            world.mic = ff.mic;
            world.clock.ppm = 0.0;
        }
        let r = render_system(&world, &s, &ProtoModel).unwrap();
        let a = try_analyse(&r, &s).unwrap();
        assert!(a.timing_reliable, "seed {seed}: {:?}", a.reasons);
        assert!(a.absolute_reliable, "seed {seed}: {:?}", a.absolute_reasons);
        assert!(a.same_unit_offer().0);
        let at = |o: UnitOutput| r.truth.output(o.unit, o.output).unwrap().arrival_us;
        assert_eq!(a.delays.len(), 4);
        for d in &a.delays {
            let err = d.delay_ms * 1000.0 - (at(d.to) - at(d.from));
            assert!(d.offered && err.abs() < 1.0, "seed {seed}: {d:?} {err}");
        }
    }
}
