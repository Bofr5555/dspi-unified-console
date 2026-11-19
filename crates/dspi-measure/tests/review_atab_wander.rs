//! Review AT-A/AT-B: WP ATSIM's world through WP ATSYS's analyser (the two
//! were written apart; ATE2E joins them for good). What is judged here is
//! the offer rule of review R8: a cross-unit delay that is offered must be
//! right within what it claims, and P5SPIKE's microphone must never get one.
//!
//! What the review's longer runs showed (two units, one output each):
//! P5SPIKE's bursts (55-100 ppm, +-50 us): P5's gate passed none of 30
//! seeds with five passes (and none of the seeds below with three); with the
//! gate opened, three passes, 24 seeds, nothing is offered and the fit's
//! error is 22 us RMS against an uncertainty of 31 us RMS. Bursts of
//! 5-15 ppm within +-5 us, three passes: 19 runs of 24 pass the gate, every
//! one is offered, RMS error 2.7 us against an RMS uncertainty of 3.1 us,
//! the largest offered error 6.7 us.
#![cfg(feature = "sim")]
#![allow(clippy::arithmetic_side_effects)]

use dspi_measure::ir::AnalysisOptions;
use dspi_measure::sim::ProtoModel;
use dspi_measure::sim::system::{SimUnit, SystemWorld, WanderBursts, render_system};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule};
use dspi_measure::system::{
    DomainId, InterleaveOptions, SystemAnalysis, SystemCapture, TimePin, UnitIndex, UnitWalk,
    analyse_system, interleave,
};

const RATE: u32 = 48_000;
const D: DomainId = DomainId {
    time_base: 0,
    id: 0x42,
};
const ORIGIN: u64 = 1_000_000_000;
const PRE: u64 = 48_000;

struct Outcome {
    analysis: SystemAnalysis,
    /// The truth of every delay, in `analysis.delays`' order, µs.
    truth_us: Vec<f64>,
}

fn run(seed: u64, passes: u16, wander: Option<WanderBursts>, opts: &AnalysisOptions) -> Outcome {
    let sweep = LogSweep::new(
        RATE,
        &SweepConfig {
            f1_hz: 20.0,
            f2_hz: 20_000.0,
            duration_ms: 1_000,
            gap_ms: 300,
        },
    )
    .unwrap();
    let walks: Vec<UnitWalk> = (0..2u8)
        .map(|u| UnitWalk {
            unit: UnitIndex(u),
            domain: D,
            walk: WalkSchedule::new(sweep, 0b1, 1).unwrap(),
        })
        .collect();
    let schedule = interleave(
        &walks,
        &InterleaveOptions {
            passes: Some(passes),
            gap_samples: u64::from(sweep.gap_samples),
            lead_samples: u64::from(RATE) / 4,
            capture_rate_hz: RATE,
            pre_roll_ms: 1_000,
        },
    )
    .unwrap()
    .offset_by(ORIGIN + PRE)
    .unwrap();
    let mut a = SimUnit::flat("A", 1);
    a.playout_frames = 96.0;
    a.outputs[0].delay_ms = 2.0;
    let mut b = SimUnit::flat("B", 1);
    b.playout_frames = 96.0;
    b.outputs[0].delay_ms = 3.3;
    let mut world = SystemWorld::free_field(vec![a, b], D, ORIGIN, RATE);
    world.noise_dbfs = Some(-70.0);
    world.clock.ppm = 23.0;
    world.seed = seed;
    world.wander = wander;
    let r = render_system(&world, &schedule, &ProtoModel).unwrap();
    let pin = TimePin {
        capture_frame: r.capture_frame_at_mic(PRE as f64).unwrap().round() as u64,
        t: schedule.start(),
        round_trip_frames: 48,
    };
    let analysis = analyse_system(
        &SystemCapture {
            samples: &r.capture.samples,
            first_frame: r.capture.blocks.first().map_or(0, |b| b.first_frame),
            rate_hz: RATE,
            schedule: &schedule,
            pins: &[pin],
            status: &[],
            complete: true,
        },
        opts,
    )
    .unwrap_or_else(|e| panic!("seed {seed}: {e:?}"));
    let truth_us = analysis
        .delays
        .iter()
        .map(|d| {
            let at = |o: dspi_measure::system::UnitOutput| {
                r.truth.output(o.unit, o.output).unwrap().arrival_us
            };
            at(d.to) - at(d.from)
        })
        .collect();
    Outcome { analysis, truth_us }
}

fn open_gate() -> AnalysisOptions {
    AnalysisOptions {
        max_residual_us: 1e5,
        max_phase_spread_deg: 180.0,
        max_phase_spread_us: 1e5,
        ..AnalysisOptions::default()
    }
}

/// `(error of the offered delay, error of the intercepts' difference,
/// uncertainty)` of the run's one cross-unit delay, µs.
fn errors(o: &Outcome) -> (f64, f64, f64) {
    let d = &o.analysis.delays[0];
    let at = |u: dspi_measure::system::UnitOutput| {
        o.analysis.output(u.unit, u.output).unwrap().intercept_frame
    };
    let by_fit = (at(d.to) - at(d.from)) * 1e6 / f64::from(RATE);
    (
        d.delay_ms * 1000.0 - o.truth_us[0],
        by_fit - o.truth_us[0],
        d.uncertainty_us.unwrap(),
    )
}

/// ATSIM's truth and ATSYS's result agree on a steady clock: two oracles
/// written apart.
#[test]
fn wp_review_atab_atsim_and_atsys_agree_on_a_steady_clock() {
    let o = run(1, 3, None, &AnalysisOptions::default());
    let (err, by_fit, u) = errors(&o);
    println!("steady: error {err:.3} us, by the fit {by_fit:.3} us, u {u:.3} us");
    assert!(o.analysis.timing_reliable, "{:?}", o.analysis.reasons);
    assert!(o.analysis.delays[0].offered);
    assert!(
        err.abs() < 2.0 && err.abs() <= 3.0 * u,
        "{err} us, u {u} us"
    );
    assert!((o.analysis.clock.ratio_ppm - 23.0).abs() < 0.5);
}

/// P5SPIKE's microphone gets no cross-unit delay: P5's gate refuses the run,
/// and with that gate opened wide the uncertainty rule alone refuses it too,
/// with an uncertainty that covers the fit's error.
#[test]
fn wp_review_atab_p5spike_wander_is_never_offered_a_cross_unit_delay() {
    for seed in 0..4u64 {
        let w = Some(WanderBursts::p5spike(seed));
        let o = run(seed, 3, w, &AnalysisOptions::default());
        assert!(!o.analysis.timing_reliable, "seed {seed}");
        assert_eq!(o.analysis.offered().count(), 0, "seed {seed}");
        assert!(
            o.analysis.delays[0].reason.starts_with("timing unreliable"),
            "{}",
            o.analysis.delays[0].reason
        );

        let o = run(seed, 3, w, &open_gate());
        let (err, by_fit, u) = errors(&o);
        println!("p5spike seed {seed}: error {err:.1} us, by the fit {by_fit:.1} us, u {u:.1} us");
        assert!(u > 10.0, "seed {seed}: u {u} us");
        assert!(!o.analysis.delays[0].offered);
        assert!(
            o.analysis.delays[0].reason.contains("wandered"),
            "{}",
            o.analysis.delays[0].reason
        );
        assert!(
            by_fit.abs() <= 3.0 * u,
            "seed {seed}: {by_fit} us, u {u} us"
        );
    }
}

/// A microphone that wanders a tenth as much (5-15 ppm within +-5 us) passes
/// the gate, and what is offered then is right within its uncertainty and
/// within the 10 us the gate stands for.
#[test]
fn wp_review_atab_an_offered_delay_is_within_its_uncertainty() {
    let mut offered = 0;
    for seed in 1..=5u64 {
        let w = WanderBursts {
            seed,
            min_ppm: 5.0,
            max_ppm: 15.0,
            bound_us: 5.0,
            ..WanderBursts::p5spike(seed)
        };
        let o = run(seed, 3, Some(w), &AnalysisOptions::default());
        let (err, _, u) = errors(&o);
        println!(
            "mild seed {seed}: reliable {} offered {} error {err:.2} us u {u:.2} us",
            o.analysis.timing_reliable, o.analysis.delays[0].offered
        );
        if o.analysis.delays[0].offered {
            offered += 1;
            assert!(o.analysis.timing_reliable);
            assert!(err.abs() <= 3.0 * u, "seed {seed}: {err} us, u {u} us");
            assert!(err.abs() <= 10.0, "seed {seed}: {err} us");
        }
    }
    assert!(offered >= 3, "{offered} of 5 offered");
}
