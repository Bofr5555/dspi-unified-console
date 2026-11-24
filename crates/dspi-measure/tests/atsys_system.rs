//! WP ATSYS on P5SIM's world: several units on one timeline. The world's
//! outputs stand for the units' outputs; each segment plays as the
//! generator plays an op-9 START (its first cycle with the start fade), at
//! its domain sample, through P5SIM's renderer. Also: P5SIM's single-unit
//! goldens through the system analyser, bit for bit.
#![cfg(feature = "sim")]
#![allow(clippy::arithmetic_side_effects)]

mod common;

use std::sync::OnceLock;

use common::{analyse, output_response, walked_run};
use dspi_measure::ir::{AnalysisOptions, StatusObservation, TimingError};
use dspi_measure::sim::{
    Burst, CaptureFault, ProtoModel, Stimulus, World, flat_chains, render, three_way_chains,
    two_way_chains,
};
use dspi_measure::stimulus::{GeneratorStatus, LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_measure::system::{
    ARRIVAL_RESOLUTION_US, AnchorOutput, ChainOptions, ChainRefusal, ChainRun, DomainId,
    InterleaveOptions, SystemAnalysis, SystemCapture, SystemSchedule, TimePin, UnitIndex,
    UnitStatusObservation, UnitWalk, analyse_system, chain_runs, interleave, transition_band,
};
use dspi_measure::types::{Coupler, Response};
use proptest::prelude::*;

const RATE: u32 = 48_000;
const D: DomainId = DomainId {
    time_base: 0,
    id: 0x51,
};
const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 1_000,
    gap_ms: 300,
};
const PRE_MS: u32 = 1_000;
/// The run's first domain sample (any).
const T0: u64 = 1 << 40;

/// (unit, output) → the world's output.
type Map = Vec<(UnitIndex, u8, usize)>;

fn sweep() -> LogSweep {
    LogSweep::new(RATE, &CFG).unwrap()
}

fn walk(mask: u16) -> WalkSchedule {
    WalkSchedule::new(sweep(), mask, 1).unwrap()
}

/// The interleaved schedule of `units` (unit, mask), from `T0`.
fn plan(units: &[(u8, u16)], passes: Option<u16>) -> SystemSchedule {
    let walks: Vec<UnitWalk> = units
        .iter()
        .map(|&(u, m)| UnitWalk {
            unit: UnitIndex(u),
            domain: D,
            walk: walk(m),
        })
        .collect();
    interleave(
        &walks,
        &InterleaveOptions {
            passes,
            gap_samples: u64::from(sweep().gap_samples),
            lead_samples: RATE as u64 / 4,
            capture_rate_hz: RATE,
            pre_roll_ms: PRE_MS,
        },
    )
    .unwrap()
    .offset_by(T0)
    .unwrap()
}

struct SysRun {
    samples: Vec<f32>,
    first_frame: u64,
    pin: TimePin,
}

/// Plays `schedule` in `world`: every segment's cycles on its unit's
/// outputs, each segment's first cycle with the start fade, a unit's starts
/// moved by `start_error` samples; a unit in `silent` never starts.
fn play(
    world: &World,
    map: &Map,
    schedule: &SystemSchedule,
    start_error: &[(UnitIndex, i64)],
    silent: &[UnitIndex],
) -> SysRun {
    let s = sweep();
    let amp = peak_linear(-20.0);
    let scaled = |first: bool| -> Vec<f32> {
        s.render_cycle(first)
            .iter()
            .map(|v| *v as f32 * amp)
            .collect()
    };
    let (first, regular) = (scaled(true), scaled(false));
    let pre = u64::from(RATE) * u64::from(PRE_MS) / 1000;
    let t_ref = schedule.start();
    let mut bursts = Vec::new();
    for seg in schedule.segments() {
        if silent.contains(&seg.unit) {
            continue;
        }
        let err = start_error
            .iter()
            .find(|(u, _)| *u == seg.unit)
            .map_or(0, |e| e.1);
        for c in seg.schedule.cycles() {
            let out = map
                .iter()
                .find(|(u, o, _)| *u == seg.unit && *o == c.output)
                .unwrap()
                .2;
            let at = (pre + (seg.start - t_ref) + c.start) as i64 + err;
            bursts.push(Burst {
                output: out,
                start_frame: at as u64,
                samples: if c.index == 0 { &first } else { &regular },
                gain: 1.0,
            });
        }
    }
    let stimulus = Stimulus {
        bursts,
        frames: pre + (schedule.end() - t_ref) + u64::from(s.gap_samples),
    };
    let chains = flat_chains(world.outputs.len());
    let capture = render(world, &stimulus, &chains, &ProtoModel, RATE).unwrap();
    let at = world.capture_position(pre as f64, RATE).unwrap();
    SysRun {
        samples: capture.samples,
        first_frame: world.clock.first_frame,
        pin: TimePin {
            capture_frame: at.round() as u64,
            t: t_ref,
            round_trip_frames: 48,
        },
    }
}

fn run_analysis(
    run: &SysRun,
    schedule: &SystemSchedule,
    status: &[UnitStatusObservation],
    opts: &AnalysisOptions,
) -> Result<SystemAnalysis, TimingError> {
    analyse_system(
        &SystemCapture {
            samples: &run.samples,
            first_frame: run.first_frame,
            rate_hz: RATE,
            schedule,
            pins: &[run.pin],
            status,
            complete: true,
        },
        opts,
    )
}

/// Two units of two outputs, flat drivers, each output at its own delay (a
/// unit's play-out offset and its distance together), a microphone 37.5 ppm
/// fast with 50 ms of latency, noise at -80 dBFS.
fn two_unit_world(seed: u64) -> (World, Map, [f64; 4]) {
    let mut w = World::flat(4, RATE);
    let delays = [1.0, 1.25, 2.137_3, 0.811_7];
    for (p, d) in w.outputs.iter_mut().zip(delays) {
        p.delay_ms = d;
    }
    w.clock.ppm = 37.5;
    w.clock.latency_ms = 50.0;
    w.clock.first_frame = 12_345;
    w.noise_dbfs = Some(-80.0);
    w.seed = seed;
    let map = vec![
        (UnitIndex(0), 0, 0),
        (UnitIndex(0), 1, 1),
        (UnitIndex(1), 0, 2),
        (UnitIndex(1), 1, 3),
    ];
    (w, map, delays)
}

fn truth_ms(world: &World, map: &Map, a: (UnitIndex, u8), b: (UnitIndex, u8)) -> f64 {
    let idx = |x: (UnitIndex, u8)| map.iter().find(|m| m.0 == x.0 && m.1 == x.1).unwrap().2;
    world.acoustic_delay_ms(idx(b)).unwrap() - world.acoustic_delay_ms(idx(a)).unwrap()
}

#[test]
fn wp_atsys_two_units_recover_their_delays_within_2_us_and_the_uncertainty_covers_it() {
    let sch = plan(&[(0, 0b11), (1, 0b11)], None);
    // 5 passes fit one capture.
    assert_eq!(sch.segments().len(), 10);
    for (seed, noise) in [(1u64, -80.0), (2, -80.0), (7, -50.0)] {
        let (mut world, map, _) = two_unit_world(seed);
        world.noise_dbfs = Some(noise);
        let run = play(&world, &map, &sch, &[], &[]);
        let a = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap();
        assert!(a.timing_reliable, "{:?}", a.reasons);
        assert!(
            (a.clock.ratio_ppm - 37.5).abs() < 0.5,
            "{}",
            a.clock.ratio_ppm
        );
        assert_eq!(a.outputs.len(), 4);
        assert_eq!(a.delays.len(), 4, "the pairs across the two units");
        for d in &a.delays {
            let truth = truth_ms(
                &world,
                &map,
                (d.from.unit, d.from.output),
                (d.to.unit, d.to.output),
            );
            let err_us = (d.delay_ms - truth) * 1000.0;
            let u = d.uncertainty_us.unwrap();
            println!(
                "seed {seed}: {:?} -> {:?}: {:.4} ms (truth {truth:.4}), error {err_us:.3} µs, u {u:.3} µs",
                d.from, d.to, d.delay_ms
            );
            assert!(err_us.abs() < 2.0, "{err_us} µs");
            assert!(d.offered, "{}", d.reason);
            assert!(err_us.abs() <= 3.0 * u, "{err_us} µs against u {u} µs");
        }
    }
}

#[test]
fn wp_atsys_a_wandering_capture_clock_widens_the_uncertainty_and_nothing_is_offered() {
    // P5SPIKE's bursts: 80 ppm over seconds, ±25 µs. P5's own gate is opened
    // wide here so that the uncertainty rule alone decides.
    let sch = plan(&[(0, 0b11), (1, 0b11)], None);
    let (mut world, map, _) = two_unit_world(3);
    world.faults = vec![CaptureFault::RatioWander {
        amplitude_ppm: 80.0,
        period_s: 2.0,
    }];
    let run = play(&world, &map, &sch, &[], &[]);
    let open = AnalysisOptions {
        max_residual_us: 1e5,
        max_phase_spread_deg: 180.0,
        max_phase_spread_us: 1e5,
        ..AnalysisOptions::default()
    };
    let a = run_analysis(&run, &sch, &[], &open).unwrap();
    let mut over = 0;
    for d in &a.delays {
        let u = d.uncertainty_us.unwrap();
        let truth = truth_ms(
            &world,
            &map,
            (d.from.unit, d.from.output),
            (d.to.unit, d.to.output),
        );
        let err = (d.delay_ms - truth) * 1000.0;
        println!(
            "wander: {:?} -> {:?} error {err:.1} µs u {u:.1} µs offered {}",
            d.from, d.to, d.offered
        );
        // The reported uncertainty is of the size of the error (R8c: within
        // three of it).
        assert!(err.abs() <= 3.0 * u, "{err} µs against {u} µs");
        if u > 10.0 {
            over += 1;
            assert!(!d.offered);
            assert!(d.reason.contains("wandered"), "{}", d.reason);
        } else {
            assert!(d.offered);
        }
    }
    assert!(over > 0, "the wander shows in the uncertainty");
    assert!(
        a.reasons.iter().any(|r| r.contains("wandered")),
        "{:?}",
        a.reasons
    );
    // With P5's gate as it is, the run is timing unreliable and offers nothing.
    let b = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap();
    assert!(!b.timing_reliable);
    assert_eq!(b.offered().count(), 0);
    assert!(
        b.reasons.iter().any(|r| r.contains("unit ")),
        "{:?}",
        b.reasons
    );
}

#[test]
fn wp_atsys_a_unit_that_never_starts_or_a_wrong_pin_is_a_cycle_identity_failure() {
    let sch = plan(&[(0, 0b11), (1, 0b11)], Some(3));
    let (world, map, _) = two_unit_world(4);
    let run = play(&world, &map, &sch, &[], &[UnitIndex(1)]);
    let e = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap_err();
    assert!(matches!(e, TimingError::CycleIdentity(_)), "{e:?}");
    // The pin one segment late.
    let mut run = play(&world, &map, &sch, &[], &[]);
    let seg = sch.segments()[1].start - sch.start();
    run.pin.t -= seg;
    let e = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap_err();
    assert!(matches!(e, TimingError::CycleIdentity(_)), "{e:?}");
    // No usable pin (a round trip past the capture latency allowed).
    run.pin.t += seg;
    run.pin.round_trip_frames = RATE as u64;
    let e = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap_err();
    assert!(matches!(e, TimingError::CycleIdentity(_)), "{e:?}");
}

#[test]
fn wp_atsys_per_unit_status_polls_are_checked_against_their_own_segments() {
    let sch = plan(&[(0, 0b11), (1, 0b11)], Some(3));
    let (world, map, _) = two_unit_world(5);
    let run = play(&world, &map, &sch, &[], &[]);
    let pre = u64::from(RATE) * u64::from(PRE_MS) / 1000;
    // The capture frame of a domain sample.
    let at = |t: u64| {
        world
            .capture_position((pre + t - sch.start()) as f64, RATE)
            .unwrap() as u64
    };
    let s1 = &sch.segments()[1]; // unit 1, pass 0
    let half = u64::from(sweep().cycle_samples) / 2;
    let obs = |unit: u8, t: u64, ch: u8, done: u16| UnitStatusObservation {
        unit: UnitIndex(unit),
        observation: StatusObservation {
            capture_frame: at(t),
            status: GeneratorStatus {
                active_channel: ch,
                cycles_done: done,
            },
        },
    };
    let period = sch.segments()[0].schedule.period_samples();
    let good = [
        // Unit 0 in its first segment's second cycle.
        obs(0, sch.start() + period + half, 1, 1),
        // Unit 1 before its first segment: not checked.
        obs(1, sch.start() + half, 0xFF, 0),
        // Unit 1 in its first segment's first cycle.
        obs(1, s1.start + half, 0, 0),
        // Unit 0 idle between its segments.
        obs(0, s1.start + half, 0xFF, 2),
    ];
    let a = run_analysis(&run, &sch, &good, &AnalysisOptions::default());
    assert!(a.is_ok(), "{a:?}");
    // Unit 1 says it plays output 1 while its first cycle (output 0) plays.
    let bad = [obs(1, s1.start + half, 1, 1)];
    let e = run_analysis(&run, &sch, &bad, &AnalysisOptions::default()).unwrap_err();
    assert!(matches!(e, TimingError::CycleIdentity(_)), "{e:?}");
}

#[test]
fn wp_atsys_sliced_analysis_waits_for_capture_and_pins_and_matches_the_whole() {
    let sch = plan(&[(0, 0b1), (1, 0b1)], Some(3));
    let (world, map, _) = two_unit_world(6);
    let run = play(&world, &map, &sch, &[], &[]);
    let whole = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap();
    let mut an =
        dspi_measure::system::SystemAnalyser::new(&sch, RATE, &AnalysisOptions::default()).unwrap();
    fn cap<'a>(
        run: &'a SysRun,
        sch: &'a SystemSchedule,
        n: usize,
        pins: &'a [TimePin],
    ) -> SystemCapture<'a> {
        SystemCapture {
            samples: &run.samples[..n.min(run.samples.len())],
            first_frame: run.first_frame,
            rate_hz: RATE,
            schedule: sch,
            pins,
            status: &[],
            complete: n >= run.samples.len(),
        }
    }
    // No pin yet: wait.
    assert_eq!(
        an.step(&cap(&run, &sch, RATE as usize, &[])).unwrap(),
        dspi_measure::system::SystemStep::NeedCapture
    );
    let pins = [run.pin];
    let mut have = 2 * RATE as usize;
    let mut waits = 0;
    let done = loop {
        match an.step(&cap(&run, &sch, have, &pins)).unwrap() {
            dspi_measure::system::SystemStep::Done(r) => break *r,
            dspi_measure::system::SystemStep::NeedCapture => {
                waits += 1;
                have += RATE as usize;
            }
            dspi_measure::system::SystemStep::Progress { .. } => {}
        }
        assert!(waits < 1000);
    };
    assert!(waits > 0);
    assert_eq!(format!("{done:?}"), format!("{whole:?}"));
}

// ---------------------------------------------------------------------------
// Summed runs and the transition band
// ---------------------------------------------------------------------------

/// A two-unit world with a subwoofer (unit 0, low-passed at 100 Hz) and a
/// mains output (unit 1, high-passed at 80 Hz).
fn sub_and_mains() -> (World, Map) {
    let mut w = World::flat(2, RATE);
    w.outputs[0].driver.rolloff_hz = 100.0;
    w.outputs[0].driver.rolloff_q = 0.707;
    w.outputs[0].delay_ms = 3.0;
    w.outputs[1].driver.highpass_hz = 80.0;
    w.outputs[1].driver.highpass_q = 0.707;
    w.outputs[1].delay_ms = 2.0;
    w.clock.latency_ms = 30.0;
    w.noise_dbfs = Some(-90.0);
    (w, vec![(UnitIndex(0), 0, 0), (UnitIndex(1), 0, 1)])
}

fn response_of(a: &SystemAnalysis, key: u8) -> Response {
    let o = a.output_ir(key).unwrap();
    output_response(o, a.walked.band_hz, 15.0, None).0
}

#[test]
fn wp_atsys_a_summed_run_is_one_key_of_two_members_and_the_transition_band_is_the_overlap() {
    let (world, map) = sub_and_mains();
    let walked = plan(&[(0, 0b1), (1, 0b1)], Some(3));
    let run = play(&world, &map, &walked, &[], &[]);
    let a = run_analysis(&run, &walked, &[], &AnalysisOptions::default()).unwrap();
    let sub = response_of(&a, a.output(UnitIndex(0), 0).unwrap().key);
    let mains = response_of(&a, a.output(UnitIndex(1), 0).unwrap().key);
    let band = transition_band(&sub, &mains, None).unwrap();
    println!("transition band {band:?}");
    assert!(
        band.0 > 25.0 && band.0 < 90.0 && band.1 > 100.0 && band.1 < 400.0,
        "{band:?}"
    );
    // Limited above by the uncertainty: 1 / (8 u).
    let narrow = transition_band(&sub, &mains, Some(2_000.0));
    assert!(narrow.is_none_or(|b| b.1 <= 62.5 + 1e-9), "{narrow:?}");
    // The summed run: both at one T.
    let s = sweep();
    let three = WalkSchedule::new(s, 0b1, 3).unwrap();
    let summed = SystemSchedule::summed(
        D,
        RATE,
        T0,
        vec![(UnitIndex(0), three.clone()), (UnitIndex(1), three)],
    )
    .unwrap();
    let run = play(&world, &map, &summed, &[], &[]);
    let b = run_analysis(&run, &summed, &[], &AnalysisOptions::default()).unwrap();
    assert!(b.summed);
    assert_eq!(b.outputs.len(), 1);
    assert_eq!(b.outputs[0].members.len(), 2);
    assert!(b.delays.is_empty());
    assert!(b.timing_reliable, "{:?}", b.reasons);
    assert!(b.outputs[0].name().contains(" + "));
}

// ---------------------------------------------------------------------------
// Anchor chaining
// ---------------------------------------------------------------------------

/// Three units, three runs each repeating unit 0's output 0 (the anchor).
type Runs = (World, Map, Vec<(SystemSchedule, SystemAnalysis)>);

fn three_runs() -> &'static Runs {
    static RUNS: OnceLock<Runs> = OnceLock::new();
    RUNS.get_or_init(|| {
        let mut w = World::flat(5, RATE);
        let delays = [1.0, 1.4, 2.25, 0.7, 3.1];
        for (p, d) in w.outputs.iter_mut().zip(delays) {
            p.delay_ms = d;
        }
        w.clock.ppm = -12.0;
        w.clock.latency_ms = 40.0;
        w.noise_dbfs = Some(-80.0);
        w.seed = 9;
        let map = vec![
            (UnitIndex(0), 0, 0),
            (UnitIndex(0), 1, 1),
            (UnitIndex(1), 0, 2),
            (UnitIndex(1), 1, 3),
            (UnitIndex(2), 0, 4),
        ];
        let plans = [
            vec![(0u8, 0b11u16)],
            vec![(0, 0b1), (1, 0b1)],
            vec![(0, 0b1), (1, 0b10), (2, 0b1)],
        ];
        let runs = plans
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let sch = plan(p, Some(3));
                let mut world = w.clone();
                world.seed = 9 + i as u64;
                let run = play(&world, &map, &sch, &[], &[]);
                let a = run_analysis(&run, &sch, &[], &AnalysisOptions::default()).unwrap();
                (sch, a)
            })
            .collect();
        (w, map, runs)
    })
}

const ANCHOR: AnchorOutput = AnchorOutput {
    unit: UnitIndex(0),
    output: 0,
};

#[test]
fn wp_atsys_three_runs_chained_through_an_anchor_recover_the_delays() {
    let (world, map, runs) = three_runs();
    let responses: Vec<Response> = runs
        .iter()
        .map(|(_, a)| response_of(a, a.output(ANCHOR.unit, ANCHOR.output).unwrap().key))
        .collect();
    let chain_in: Vec<ChainRun<'_>> = runs
        .iter()
        .zip(&responses)
        .map(|((_, a), r)| ChainRun {
            analysis: a,
            anchor_response: r,
        })
        .collect();
    let c = chain_runs(ANCHOR, &chain_in, &ChainOptions::default()).unwrap();
    assert!(c.anchor_spread_db <= 1.0, "{}", c.anchor_spread_db);
    // The bound: the residuals of every run plus one arrival's resolution
    // per run (a clean world's residuals are nanoseconds).
    let residuals: f64 = runs
        .iter()
        .map(|(_, a)| a.walked.quality.max_residual_us + ARRIVAL_RESOLUTION_US)
        .sum();
    for o in &c.outputs {
        let truth = truth_ms(
            world,
            map,
            (ANCHOR.unit, ANCHOR.output),
            (o.output.unit, o.output.output),
        );
        let err = (o.relative_ms - truth) * 1000.0;
        println!(
            "run {} {:?}: {:.4} ms (truth {truth:.4}) error {err:.3} µs, bound {residuals:.3}",
            o.run, o.output, o.relative_ms
        );
        assert!(err.abs() <= residuals, "{err} µs");
        assert!(o.offered);
    }
    // Every output of the group is reached.
    for m in map {
        assert!(
            c.outputs
                .iter()
                .any(|o| o.output.unit == m.0 && o.output.output == m.1)
        );
    }
}

#[test]
fn wp_atsys_a_moved_microphone_refuses_the_chain() {
    let (_, _, runs) = three_runs();
    let mut responses: Vec<Response> = runs
        .iter()
        .map(|(_, a)| response_of(a, a.output(ANCHOR.unit, ANCHOR.output).unwrap().key))
        .collect();
    // The anchor 3 dB louder in the second run (nearer).
    responses[1].mag_db.iter_mut().for_each(|m| *m += 3.0);
    let chain_in: Vec<ChainRun<'_>> = runs
        .iter()
        .zip(&responses)
        .map(|((_, a), r)| ChainRun {
            analysis: a,
            anchor_response: r,
        })
        .collect();
    let e = chain_runs(ANCHOR, &chain_in, &ChainOptions::default()).unwrap_err();
    match e {
        ChainRefusal::Moved { spread_db, .. } => assert!((spread_db - 3.0).abs() < 0.5),
        other => panic!("{other:?}"),
    }
    // A run without the anchor.
    let e = chain_runs(
        AnchorOutput {
            unit: UnitIndex(2),
            output: 0,
        },
        &chain_in,
        &ChainOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(e, ChainRefusal::AnchorMissing { run: 0 }), "{e:?}");
    assert!(matches!(
        chain_runs(ANCHOR, &chain_in[..1], &ChainOptions::default()),
        Err(ChainRefusal::Runs(1))
    ));
}

/// At most this many cases, whatever `PROPTEST_CASES` says: each case runs
/// the whole chain (about 66 ms), and the gate's 20000 cases made this one
/// property the full CI's critical path (about 22 min) for a small input
/// space (owner 2026-11-24). 1000 cases cover every NaN position about once.
const CHAIN_CASES_MAX: u32 = 1000;

#[test]
fn proptest_wp_atsys_chain_runs_never_panics() {
    let mut config = proptest::test_runner::contextualize_config(ProptestConfig::with_cases(24));
    config.cases = config.cases.min(CHAIN_CASES_MAX);
    config.source_file = Some(file!());
    config.test_name = Some("proptest_wp_atsys_chain_runs_never_panics");
    let strategy = (
        proptest::collection::vec(-100.0f64..100.0, 3),
        proptest::option::of(0usize..1066),
        proptest::option::of((1.0f64..30_000.0, 1.0f64..30_000.0)),
        0.0f64..10.0,
        any::<bool>(),
    );
    let mut runner = proptest::test_runner::TestRunner::new(config);
    let result = runner.run(&strategy, |(offsets, nan_at, band, spread, one_clock)| {
        let (_, _, runs) = three_runs();
        let base = response_of(
            &runs[0].1,
            runs[0].1.output(ANCHOR.unit, ANCHOR.output).unwrap().key,
        );
        let responses: Vec<Response> = offsets
            .iter()
            .map(|o| {
                let mut r = base.clone();
                r.mag_db.iter_mut().for_each(|m| *m += o);
                if let Some(i) = nan_at
                    && let Some(m) = r.mag_db.get_mut(i)
                {
                    *m = f64::NAN;
                }
                r
            })
            .collect();
        let chain_in: Vec<ChainRun<'_>> = runs
            .iter()
            .zip(&responses)
            .map(|((_, a), r)| ChainRun {
                analysis: a,
                anchor_response: r,
            })
            .collect();
        let opts = ChainOptions {
            band_hz: band,
            max_spread_db: spread,
            one_capture_clock: one_clock,
            ..ChainOptions::default()
        };
        if let Ok(c) = chain_runs(ANCHOR, &chain_in, &opts) {
            prop_assert!(c.anchor_spread_db.is_finite());
            for o in &c.outputs {
                prop_assert!(o.relative_ms.is_finite());
            }
        }
        Ok(())
    });
    if let Err(e) = result {
        panic!("proptest_wp_atsys_chain_runs_never_panics: {e}");
    }
}

// ---------------------------------------------------------------------------
// P5SIM's single-unit worlds through the system analyser
// ---------------------------------------------------------------------------

fn single_equal(world: &World, run: &common::Run, opts: &AnalysisOptions) {
    let p5 = analyse(run, world, opts);
    let sch = SystemSchedule::single(D, UnitIndex(0), T0, run.schedule.clone()).unwrap();
    let pin = TimePin {
        capture_frame: run.start_frame,
        t: T0,
        round_trip_frames: 0,
    };
    let sys = analyse_system(
        &SystemCapture {
            samples: &run.capture.samples,
            first_frame: world.clock.first_frame,
            rate_hz: run.capture.rate_hz,
            schedule: &sch,
            pins: &[pin],
            status: &[],
            complete: true,
        },
        opts,
    );
    match (p5, sys) {
        (Ok(w), Ok(s)) => assert_eq!(format!("{w:?}"), format!("{:?}", s.walked)),
        (Err(a), Err(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}")),
        (a, b) => panic!("{:?} against {:?}", a.is_ok(), b.is_ok()),
    }
}

#[test]
fn wp_atsys_p5sim_goldens_give_the_same_result_through_the_system_analyser() {
    let chains = three_way_chains();
    let cfg = SweepConfig { gap_ms: 400, ..CFG };
    let opts = AnalysisOptions::default();
    // The three-way room (its golden world), clean and with each fault kind
    // of P5ARRIVAL's and P5A's fault tests.
    let base = World::three_way_room(1);
    let faults = [
        vec![],
        vec![CaptureFault::RatioWander {
            amplitude_ppm: 75.0,
            period_s: 1.5,
        }],
        vec![CaptureFault::StepSlip {
            at_frame: 48_000 * 9,
            frames: 2,
        }],
        vec![CaptureFault::DeviceSlip {
            at_device_frame: 48_000 * 6,
            frames: -1,
        }],
    ];
    for f in faults {
        let mut w = base.clone();
        w.faults = f;
        let run = walked_run(&w, &chains, 0b111, 3, &cfg, -20.0, 1_000);
        single_equal(&w, &run, &opts);
    }
    // The golden files' worlds (`testdata/p5/sim`: P5SIM's goldens render
    // these): walked through their own chains.
    let goldens = [
        (World::three_way_room(1), three_way_chains(), 0b111u16),
        (World::two_way_near_field(1), two_way_chains(), 0b11),
        (
            World::headphones(Coupler::Iec60318_4, 1),
            flat_chains(2),
            0b11,
        ),
    ];
    for (w, chains, mask) in goldens {
        let run = walked_run(&w, &chains, mask, 3, &cfg, -20.0, 1_000);
        single_equal(&w, &run, &opts);
    }
}
