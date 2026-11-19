//! WP ATE2E (PLAN-AT, wave AT-D): WP ATSIM's world through WP ATSYS's
//! analyser at scale, the parts of the end-to-end list that are statistics
//! rather than wiring (the wiring, through the real hub and WP ATCORE's
//! session, is `crates/dspi-hub/tests/wp_ate2e_system.rs`).
//!
//! - **R8c, the offer rule over many seeds:** with P5SPIKE's microphone
//!   (55-100 ppm bursts of 0.25-1.5 s within ±50 µs) every delay that is
//!   offered is within three of its reported uncertainties and within 10 µs
//!   of the truth, and every run that offers nothing says "timing
//!   unreliable". The suite runs [`SUITE_SEEDS`] seeds; the full 100 run in
//!   the ignored `wp_ate2e_r8c_p5spike_100_seeds` (release build).
//! - **The gate as the wander falls** (the AT-A/AT-B review's open case:
//!   with the iMM-6C-class wander every run is refused): the ignored
//!   `wp_ate2e_gate_pass_rate_as_the_wander_falls` prints how often a run
//!   passes P5's gate when P5SPIKE's bursts are scaled down (rate and bound
//!   together), for the owner's decision on "overlapped starts". The suite
//!   keeps the two ends of that table.
//! - **Three runs chained through an anchor in one capture:** the microphone
//!   stays open across runs (WP ATCORE's decision), so ATSIM renders the three
//!   runs as one capture on one clock, each run is analysed on its own, and
//!   the chain's arrival check (20 µs of one drift fit) runs; the same three
//!   runs with every unit 3 samples later in the middle one (the microphone
//!   moved 2 cm between runs, the magnitudes unchanged) are refused by it.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use crate::ir::{AnalysisOptions, fdw_response};
use crate::response::from_grid_values;
use crate::sim::system::{
    SimUnit, SystemRender, SystemWorld, UnitFault, WanderBursts, render_system,
};
use crate::sim::{Microphone, ProtoModel, World};
use crate::stimulus::{LogSweep, SweepConfig, WalkSchedule};
use crate::system::chain::{Chain, ChainOptions, ChainRefusal, ChainRun, chain_runs};
use crate::system::{
    AnchorOutput, DomainId, InterleaveOptions, Segment, SystemAnalysis, SystemCapture,
    SystemSchedule, TimePin, UnitIndex, UnitOutput, UnitWalk, analyse_system, interleave,
};
use crate::types::{LogGrid, Response};

const RATE: u32 = 48_000;
const D: DomainId = DomainId {
    time_base: 0,
    id: 0x5E,
};
/// The domain sample at reference device frame 0.
const ORIGIN: u64 = 2_000_000_000;
/// Silence before the first segment, frames (the pre-roll).
const PRE: u64 = 48_000;
/// Seeds the suite runs for R8c (the full 100 run in the ignored test).
const SUITE_SEEDS: u64 = 8;

fn sweep() -> LogSweep {
    LogSweep::new(
        RATE,
        &SweepConfig {
            f1_hz: 20.0,
            f2_hz: 20_000.0,
            duration_ms: 1_000,
            gap_ms: 300,
        },
    )
    .unwrap()
}

/// `units` (`(unit, mask)`) interleaved over `passes` from domain sample `at`.
fn schedule(units: &[(u8, u16)], passes: u16, at: u64) -> SystemSchedule {
    let s = sweep();
    let walks: Vec<UnitWalk> = units
        .iter()
        .map(|&(u, mask)| UnitWalk {
            unit: UnitIndex(u),
            domain: D,
            walk: WalkSchedule::new(s, mask, 1).unwrap(),
        })
        .collect();
    interleave(
        &walks,
        &InterleaveOptions {
            passes: Some(passes),
            gap_samples: u64::from(s.gap_samples),
            lead_samples: u64::from(RATE) / 4,
            capture_rate_hz: RATE,
            pre_roll_ms: 1_000,
        },
    )
    .unwrap()
    .offset_by(at)
    .unwrap()
}

/// A room (three_way_room's modes and reflections, a typical microphone),
/// the capture 31 ppm fast, noise at -80 dBFS.
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

/// The analysis of `schedule` in a capture, pinned at its start, on the
/// capture from a second before it (its pre-roll, as WP ATCORE's session
/// keeps one per run) to two seconds after its end.
fn analyse(r: &SystemRender, schedule: &SystemSchedule, opts: &AnalysisOptions) -> SystemAnalysis {
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
        opts,
    )
    .unwrap()
}

/// One two-unit run's one cross-unit delay against ATSIM's truth.
#[derive(Debug, Clone, Copy)]
struct Outcome {
    /// P5's gate (G5) passed.
    reliable: bool,
    offered: bool,
    err_us: f64,
    u_us: Option<f64>,
    /// The reason says "timing unreliable".
    reason_unreliable: bool,
}

fn two_units(seed: u64, passes: u16, wander: Option<WanderBursts>) -> Outcome {
    let s = schedule(&[(0, 1), (1, 1)], passes, ORIGIN + PRE);
    let world = room(
        vec![
            unit("A", 1, 305.0, &[3.0]),
            unit("B", 1, 257.0 + 48.0, &[5.37]),
        ],
        seed,
        wander,
    );
    let r = render_system(&world, &s, &ProtoModel).unwrap();
    let a = analyse(&r, &s, &AnalysisOptions::default());
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

/// R8c over `seeds`: what is offered holds, what is not says why. Returns
/// how many were offered.
fn r8c(seeds: std::ops::Range<u64>) -> usize {
    let mut offered = 0;
    for seed in seeds {
        let o = two_units(seed, 3, Some(WanderBursts::p5spike(seed)));
        println!("p5spike seed {seed}: {o:?}");
        if o.offered {
            offered += 1;
            let u = o.u_us.unwrap();
            assert!(o.reliable, "seed {seed}: offered from an unreliable run");
            assert!(
                o.err_us.abs() <= 3.0 * u,
                "seed {seed}: {} µs off, u {u} µs",
                o.err_us
            );
            assert!(o.err_us.abs() <= 10.0, "seed {seed}: {} µs off", o.err_us);
        } else {
            assert!(
                o.reason_unreliable,
                "seed {seed}: not offered, but why: {o:?}"
            );
        }
    }
    offered
}

#[test]
fn wp_ate2e_r8c_p5spike_seeds_offer_only_what_holds() {
    let offered = r8c(0..SUITE_SEEDS);
    println!("p5spike: {offered} of {SUITE_SEEDS} offered");
}

/// The plan's 100 seeds (`ci/run.sh 'cargo test --release -p dspi-measure
/// --features sim --lib wp_ate2e_r8c_p5spike_100 -- --ignored'`).
#[test]
#[ignore = "WP ATE2E: 100 seeds, minutes in a debug build; run with --release --ignored"]
fn wp_ate2e_r8c_p5spike_100_seeds() {
    let offered = r8c(0..100);
    println!("p5spike: {offered} of 100 offered");
}

/// P5SPIKE's bursts scaled by `k` (rate and bound together; the burst and
/// gap lengths as measured).
fn scaled(seed: u64, k: f64) -> WanderBursts {
    let w = WanderBursts::p5spike(seed);
    WanderBursts {
        min_ppm: w.min_ppm * k,
        max_ppm: w.max_ppm * k,
        bound_us: w.bound_us * k,
        ..w
    }
}

/// `(runs that passed P5's gate, runs that offered the delay, RMS error of
/// the offered ones µs, largest offered error µs)` over `seeds` at scale `k`
/// (0: no wander).
fn pass_rate(k: f64, passes: u16, seeds: std::ops::Range<u64>) -> (usize, usize, f64, f64) {
    let (mut pass, mut offer, mut sq, mut worst) = (0, 0, 0.0, 0.0f64);
    for seed in seeds {
        let w = (k > 0.0).then(|| scaled(seed, k));
        let o = two_units(seed, passes, w);
        pass += usize::from(o.reliable);
        if o.offered {
            offer += 1;
            sq += o.err_us * o.err_us;
            worst = worst.max(o.err_us.abs());
            assert!(
                o.err_us.abs() <= 3.0 * o.u_us.unwrap().max(0.25) && o.err_us.abs() <= 10.0,
                "k {k} seed {seed}: {o:?}"
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

/// The table for the owner (`ci/run.sh 'cargo test --release -p
/// dspi-measure --features sim --lib wp_ate2e_gate_pass_rate -- --ignored
/// --nocapture'`).
#[test]
#[ignore = "WP ATE2E: the wander table, minutes in a release build"]
fn wp_ate2e_gate_pass_rate_as_the_wander_falls() {
    const N: u64 = 30;
    println!("scale  bursts ppm  bound    passes  gate-pass  offered  rms-err  worst");
    for passes in [3u16, 5] {
        for k in [1.0, 0.5, 0.3, 0.2, 0.1, 0.05, 0.02, 0.0] {
            let (pass, offer, rms, worst) = pass_rate(k, passes, 0..N);
            println!(
                "{k:<5}  {:>5.1}-{:<5.1} {:>5.2}us  {passes}       {pass:>2}/{N}      {offer:>2}/{N}   {rms:>5.2}us  {worst:>5.2}us",
                55.0 * k,
                100.0 * k,
                50.0 * k
            );
        }
    }
}

/// The two ends of the table, in the suite: P5SPIKE's own wander passes no
/// run; a twentieth of it passes most, and what they offer holds.
#[test]
fn wp_ate2e_the_gate_opens_only_as_the_wander_falls() {
    let (pass, offer, ..) = pass_rate(1.0, 3, 100..103);
    assert_eq!((pass, offer), (0, 0), "P5SPIKE's wander");
    let (pass, offer, rms, worst) = pass_rate(0.05, 3, 100..103);
    println!("a twentieth: {pass}/3 pass, {offer}/3 offered, rms {rms:.2} µs, worst {worst:.2} µs");
    assert!(pass >= 2 && offer >= 2, "{pass} {offer}");
}

// ---------------------------------------------------------------------------
// Three runs chained through the anchor, in one capture
// ---------------------------------------------------------------------------

const ANCHOR: AnchorOutput = AnchorOutput {
    unit: UnitIndex(0),
    output: 0,
};

/// The anchor's response in `a` (the onset-centred window P5 uses).
fn anchor_response(a: &SystemAnalysis) -> Response {
    let o = a
        .output_ir(a.output(ANCHOR.unit, ANCHOR.output).unwrap().key)
        .unwrap();
    let grid = LogGrid::STANDARD;
    let freqs = grid.freqs().unwrap();
    let w = fdw_response(&o.ir, o.onset_index, &freqs, 15.0).unwrap();
    let values: Vec<[f64; 2]> = w.values.iter().map(|c| [c.re, c.im]).collect();
    let delay_s = (o.onset_index - o.ir.zero_index as f64) / f64::from(o.ir.rate_hz);
    from_grid_values(&values, &grid, delay_s).unwrap()
}

/// Three units; run 1 walks unit 0's outputs 0 and 1, run 2 the anchor and
/// unit 1's output 0, run 3 the anchor, unit 1's output 1 and unit 2's
/// output 0: back to back in one schedule, rendered as one capture.
/// `moved`: every unit plays 3 samples later during run 2 (the microphone
/// 2 cm farther), back at run 3.
fn chained(moved: bool) -> (Vec<SystemAnalysis>, SystemRender) {
    let parts: [&[(u8, u16)]; 3] = [
        &[(0, 0b11)],
        &[(0, 0b1), (1, 0b1)],
        &[(0, 0b1), (1, 0b10), (2, 0b1)],
    ];
    let mut runs: Vec<SystemSchedule> = Vec::new();
    let mut at = ORIGIN + PRE;
    for p in parts {
        let s = schedule(p, 3, at);
        // The next run three seconds later (the session analyses the run
        // and captures the next one's pre-roll in between).
        at = s.end() + 3 * u64::from(RATE);
        runs.push(s);
    }
    let all: Vec<Segment> = runs.iter().flat_map(|s| s.segments().to_vec()).collect();
    let whole = SystemSchedule::new(D, RATE, all).unwrap();
    let mut units = vec![
        unit("A", 2, 305.0, &[3.0, 3.4]),
        unit("B", 2, 305.0, &[4.25, 2.7]),
        unit("C", 1, 305.0, &[5.1]),
    ];
    if moved {
        let (t2, t3) = (runs[1].start() - 4_800, runs[2].start() - 4_800);
        for u in &mut units {
            u.faults.push(UnitFault::Correction {
                at_t: t2,
                frames: 3,
                event: false,
            });
            u.faults.push(UnitFault::Correction {
                at_t: t3,
                frames: -3,
                event: false,
            });
        }
    }
    let world = room(units, 5, None);
    let r = render_system(&world, &whole, &ProtoModel).unwrap();
    let analyses = runs
        .iter()
        .map(|s| analyse(&r, s, &AnalysisOptions::default()))
        .collect();
    (analyses, r)
}

fn chain_of(analyses: &[SystemAnalysis]) -> Result<Chain, ChainRefusal> {
    let responses: Vec<Response> = analyses.iter().map(anchor_response).collect();
    let input: Vec<ChainRun<'_>> = analyses
        .iter()
        .zip(&responses)
        .map(|(a, r)| ChainRun {
            analysis: a,
            anchor_response: r,
        })
        .collect();
    chain_runs(
        ANCHOR,
        &input,
        &ChainOptions {
            one_capture_clock: true,
            ..ChainOptions::default()
        },
    )
}

#[test]
fn wp_ate2e_three_runs_chained_through_the_anchor_in_one_capture() {
    let (analyses, r) = chained(false);
    for a in &analyses {
        assert!(a.timing_reliable, "{:?}", a.reasons);
    }
    let c = chain_of(&analyses).unwrap();
    println!(
        "spread {:.3} dB, anchor off the drift fit {:?} µs",
        c.anchor_spread_db, c.anchor_arrival_us
    );
    // One capture clock: the arrival check ran, and it holds.
    let off = c.anchor_arrival_us.expect("the arrival check ran");
    assert!(off <= 2.0, "{off} µs");
    let truth = |u: UnitIndex, o: u8| r.truth.output(u, o).unwrap().arrival_us;
    let anchor = truth(ANCHOR.unit, ANCHOR.output);
    for o in &c.outputs {
        let err = o.relative_ms * 1000.0 - (truth(o.output.unit, o.output.output) - anchor);
        println!(
            "run {} {:?}: {:.4} ms, error {err:.3} µs, u {:?}",
            o.run, o.output, o.relative_ms, o.uncertainty_us
        );
        assert!(err.abs() <= 2.0, "{err} µs");
        if o.output.unit != ANCHOR.unit {
            assert!(o.offered, "{o:?}");
            assert!(err.abs() <= 3.0 * o.uncertainty_us.unwrap().max(0.25));
        }
    }
    assert_eq!(
        c.outputs.len(),
        5,
        "every output of the group, the anchor once"
    );
}

#[test]
fn wp_ate2e_a_microphone_moved_between_chained_runs_is_refused_by_the_arrival_check() {
    let (analyses, _) = chained(true);
    let e = chain_of(&analyses).unwrap_err();
    println!("{e}");
    match e {
        // The middle run 62.5 µs late: two thirds of it off a line through
        // three runs.
        ChainRefusal::ArrivalMoved { us } => assert!(us > 20.0 && us < 62.5, "{us}"),
        other => panic!("{other:?}"),
    }
}
