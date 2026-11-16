//! Wave P5-A review, issue 6: P5TIME's fault matrix on P5SIM's world.
//!
//! The two WPs were written in parallel; P5TIME tested against a small world
//! of its own. Here P5SIM's three-way room (drivers, room modes, reflections,
//! a microphone 37.5 ppm off with 50 ms of latency, noise) is driven with
//! P5TIME's model of the generator and every capture fault of RC5e is
//! rendered by P5SIM. The claim proven: **no fault is ever marked "timing
//! reliable" with a delay between two outputs off by more than 10 µs**; a
//! fault the frame indices show is caught there.
#![cfg(feature = "sim")]

mod common;

use common::{Run, analyse, lag_us, walked_run};
use dspi_measure::ir::{AnalysisOptions, FrameContinuity, FrameFault, TimingError, WalkedAnalysis};
use dspi_measure::sim::{CaptureFault, World, three_way_chains};
use dspi_measure::stimulus::SweepConfig;

/// A short run keeps the matrix affordable in a debug build: 1 s sweeps,
/// 400 ms gaps, the left speaker's three ways, three passes.
const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 1_000,
    gap_ms: 400,
};
const MASK: u16 = 0b111;
const PASSES: u16 = 3;
const LEVEL_DB: f32 = -12.0;
const PRE_ROLL_MS: u32 = 1_000;
/// One cycle and its gap, frames.
const PERIOD: u64 = 67_200;
/// Where each way is loud enough to compare phases, Hz.
const BANDS: [(f64, f64); 3] = [(60.0, 250.0), (400.0, 2_500.0), (4_000.0, 12_000.0)];
/// The bound of the claim, µs.
const BOUND_US: f64 = 10.0;

fn world(ppm: f64, faults: Vec<CaptureFault>) -> World {
    let mut w = World::three_way_room(11);
    w.clock.ppm = ppm;
    w.faults = faults;
    w
}

fn run(world: &World) -> Run {
    walked_run(
        world,
        &three_way_chains(),
        MASK,
        PASSES,
        &CFG,
        LEVEL_DB,
        PRE_ROLL_MS,
    )
}

/// The largest error of a delay between two outputs against the reference
/// run, µs.
fn delay_error_us(got: &WalkedAnalysis, reference: &WalkedAnalysis) -> f64 {
    let lags: Vec<f64> = got
        .outputs
        .iter()
        .zip(&reference.outputs)
        .zip(BANDS)
        .map(|((a, b), band)| lag_us(a, b, band))
        .collect();
    let lo = lags.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = lags.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    hi - lo
}

/// The capture frame (counted from the capture's first frame) of a point
/// `offset` frames into cycle `k`.
fn in_cycle(k: u64, offset: u64) -> u64 {
    // Pre-roll, 50 ms of latency, then the cycles.
    48_000 + 2_400 + k * PERIOD + offset
}

#[derive(Debug)]
struct Outcome {
    name: String,
    reliable: bool,
    error_us: Option<f64>,
    why: String,
}

fn judge(name: &str, w: &World, reference: &WalkedAnalysis) -> Outcome {
    let r = run(w);
    match analyse(&r, w, &AnalysisOptions::default()) {
        Ok(a) => Outcome {
            name: name.to_string(),
            reliable: a.quality.reliable,
            error_us: Some(delay_error_us(&a, reference)),
            why: format!(
                "residual {:.2} µs (bound {:.2}), phase {:.1}°, {:?}",
                a.quality.max_residual_us,
                a.quality.residual_bound_us,
                a.quality.max_phase_spread_deg,
                a.quality.reasons
            ),
        },
        Err(TimingError::CycleIdentity(why)) => Outcome {
            name: name.to_string(),
            reliable: false,
            error_us: None,
            why: format!("cycle identity: {why}"),
        },
        Err(e) => panic!("{name}: {e}"),
    }
}

#[test]
fn wp_p5a_no_fault_of_p5sims_world_is_reliable_with_a_delay_off_by_10_us() {
    // The reference: the same world with an exact clock and no fault.
    let w0 = world(0.0, Vec::new());
    let reference = analyse(&run(&w0), &w0, &AnalysisOptions::default()).unwrap();
    assert!(reference.quality.reliable, "{:?}", reference.quality);

    let slip = |k: u64, offset: u64, frames: i32| CaptureFault::StepSlip {
        at_frame: in_cycle(k, offset),
        frames,
    };
    let device = |k: u64, offset: u64, frames: i32| CaptureFault::DeviceSlip {
        at_device_frame: 48_000 + k * PERIOD + offset,
        frames,
    };
    let wander = |amplitude_ppm: f64, period_s: f64| CaptureFault::RatioWander {
        amplitude_ppm,
        period_s,
    };
    // (name, clock offset ppm, faults, must be flagged)
    let cases: Vec<(&str, f64, Vec<CaptureFault>, bool)> = vec![
        ("clean, 37.5 ppm", 37.5, vec![], false),
        ("clean, 50 ppm", 50.0, vec![], false),
        ("clean, -80 ppm", -80.0, vec![], false),
        (
            "step slip +1 in the gap after cycle 2",
            37.5,
            vec![slip(2, 58_000, 1)],
            true,
        ),
        (
            "step slip +1 in the middle of cycle 4",
            37.5,
            vec![slip(4, 24_000, 1)],
            true,
        ),
        (
            "step slip -2 in the middle of cycle 0",
            37.5,
            vec![slip(0, 30_000, -2)],
            true,
        ),
        (
            "step slip +3 between passes 2 and 3",
            50.0,
            vec![slip(5, 60_000, 3)],
            true,
        ),
        (
            "step slip +128 in the last cycle",
            37.5,
            vec![slip(8, 20_000, 128)],
            true,
        ),
        (
            "device slip +32 in the gap after cycle 3",
            37.5,
            vec![device(3, 56_000, 32)],
            true,
        ),
        (
            "device slip -48 in the middle of cycle 5",
            50.0,
            vec![device(5, 20_000, -48)],
            true,
        ),
        (
            "wander 20 ppm over 7 s",
            37.5,
            vec![wander(20.0, 7.0)],
            true,
        ),
        ("wander 5 ppm over 3 s", 50.0, vec![wander(5.0, 3.0)], false),
    ];
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for (name, ppm, faults, must_flag) in cases {
        let clean = faults.is_empty();
        let w = world(ppm, faults);
        let o = judge(name, &w, &reference);
        if o.reliable && o.error_us.is_some_and(|e| e > BOUND_US) {
            failures.push(format!("RELIABLE WITH A WRONG DELAY: {o:?}"));
        }
        if must_flag && o.reliable {
            failures.push(format!("not flagged: {o:?}"));
        }
        if clean && !(o.reliable && o.error_us.is_some_and(|e| e <= 5.0)) {
            failures.push(format!(
                "a clean run must be reliable and within 5 µs: {o:?}"
            ));
        }
        report.push(o);
    }
    for o in &report {
        println!(
            "{:<45} reliable={:<5} error={:>8} µs  {}",
            o.name,
            o.reliable,
            o.error_us.map_or("-".to_string(), |e| format!("{e:.2}")),
            o.why
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn wp_p5a_the_clock_of_p5sims_world_is_measured() {
    // 50 ppm over a 12.6 s baseline (the plan's 0.2 ppm is for 18 s).
    let w = world(50.0, Vec::new());
    let a = analyse(&run(&w), &w, &AnalysisOptions::default()).unwrap();
    assert!(a.clock.fitted);
    assert!((a.clock.ratio_ppm - 50.0).abs() < 0.3, "{:?}", a.clock);
    // The walk's outputs come out in order with their repeats.
    assert_eq!(
        a.outputs.iter().map(|o| o.output).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(a.outputs.iter().all(|o| o.repeats == PASSES));
}

#[test]
fn wp_p5a_dropped_and_repeated_blocks_are_caught_by_the_frame_indices() {
    let w0 = world(0.0, Vec::new());
    let reference = analyse(&run(&w0), &w0, &AnalysisOptions::default()).unwrap();
    for (fault, want_gap) in [
        (
            CaptureFault::DroppedBlock {
                at_frame: in_cycle(3, 10_000),
            },
            true,
        ),
        (
            CaptureFault::RepeatedBlock {
                at_frame: in_cycle(4, 30_000),
            },
            false,
        ),
    ] {
        let w = world(37.5, vec![fault]);
        let r = run(&w);
        let event = r.capture.events[0];
        assert!(
            event.visible_in_frame_indices && event.in_capture,
            "{event:?}"
        );
        // The session's push path: every block through the continuity check.
        let mut track = FrameContinuity::new();
        let mut caught = None;
        for (first_frame, samples, _) in r.capture.pushes() {
            if let Err(e) = track.push(first_frame, samples.len()) {
                caught = Some(e);
                break;
            }
        }
        match caught {
            Some(FrameFault::Gap { expected, got }) if want_gap => {
                assert_eq!(got - expected, 128, "{fault:?}");
            }
            Some(FrameFault::Repeat { expected, got }) if !want_gap => {
                assert_eq!(expected - got, 128, "{fault:?}");
            }
            other => panic!("{fault:?}: {other:?}"),
        }
        // And if a client ignored its indices and analysed the samples as
        // they were pushed, the gate still does not call the timing reliable.
        match analyse(&r, &w, &AnalysisOptions::default()) {
            Ok(a) => assert!(
                !a.quality.reliable || delay_error_us(&a, &reference) <= BOUND_US,
                "{fault:?}: {:?}",
                a.quality
            ),
            Err(TimingError::CycleIdentity(_)) => {}
            Err(e) => panic!("{fault:?}: {e}"),
        }
    }
}

#[test]
fn wp_p5a_a_start_noted_one_cycle_off_is_a_cycle_identity_failure_in_p5sims_world() {
    let w = world(37.5, Vec::new());
    let mut r = run(&w);
    let honest = r.start_frame;
    for off in [-1i64, 1] {
        r.start_frame = (honest as i64 + off * PERIOD as i64) as u64;
        match analyse(&r, &w, &AnalysisOptions::default()) {
            Err(TimingError::CycleIdentity(_)) => {}
            other => panic!("START {off} cycle off: {:?}", other.map(|a| a.quality)),
        }
    }
}
