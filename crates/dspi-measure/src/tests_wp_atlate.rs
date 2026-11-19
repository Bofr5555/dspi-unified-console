//! WP ATLATE: a walked run planned with each unit's **expected lateness**
//! (`interleave_late`, `SystemSchedule::with_lateness`). The AT-D1 review
//! found that the walked analysis keeps each response only from 100 ms
//! before the run's first arrival to 100 ms after it (the 300 ms gap less
//! twice the search span), so a unit heard later than that after its START
//! (USB-direct units carry their streams' start offsets, some 21 ms a unit,
//! in `out.delay`) was cut, or not found. The planner knows each unit's
//! play-out and output delays before the run: the starts now leave room for
//! them and the analysis looks for each unit where it is expected, so such
//! a unit is measured whole. A wrong expectation fails closed.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use crate::ir::AnalysisOptions;
use crate::sim::system::{SimUnit, SystemRender, SystemWorld, render_system};
use crate::sim::{Microphone, ProtoModel, World};
use crate::stimulus::{LogSweep, SweepConfig, WalkSchedule};
use crate::system::{
    DomainId, InterleaveOptions, MAX_LATENESS_MS, SystemAnalysis, SystemCapture, SystemSchedule,
    TimePin, UnitIndex, UnitOutput, UnitWalk, analyse_system, interleave, interleave_late,
};
use crate::types::MeasureError;

const RATE: u32 = 48_000;
const D: DomainId = DomainId {
    time_base: 0,
    id: 0x1A7E,
};
/// The domain sample at reference device frame 0.
const ORIGIN: u64 = 2_000_000_000;
/// Silence before the first segment, frames.
const PRE: u64 = 48_000;
/// 150 ms at the units' rate: later than the analysis keeps without the
/// lateness (100 ms), within what a run plans for.
const LATE_150_MS: u64 = 7_200;

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

fn walks(units: &[(u8, u16)]) -> Vec<UnitWalk> {
    let s = sweep();
    units
        .iter()
        .map(|&(u, mask)| UnitWalk {
            unit: UnitIndex(u),
            domain: D,
            walk: WalkSchedule::new(s, mask, 1).unwrap(),
        })
        .collect()
}

fn options(passes: u16) -> InterleaveOptions {
    InterleaveOptions {
        passes: Some(passes),
        gap_samples: u64::from(sweep().gap_samples),
        lead_samples: u64::from(RATE) / 4,
        capture_rate_hz: RATE,
        pre_roll_ms: 1_000,
    }
}

fn unit(name: &str, playout: f64, delay_ms: f64) -> SimUnit {
    let mut u = SimUnit::flat(name, 1);
    u.playout_frames = playout;
    for p in &mut u.outputs {
        p.delay_ms = delay_ms;
        p.gain_db = -6.0;
    }
    u
}

/// A room (three_way_room's modes and reflections, a typical microphone),
/// the capture 31 ppm fast, noise at -80 dBFS.
fn room(units: Vec<SimUnit>, seed: u64) -> SystemWorld {
    let mut w = SystemWorld::free_field(units, D, ORIGIN, RATE);
    w.acoustics = World::three_way_room(seed).acoustics;
    w.mic = Microphone::typical();
    w.noise_dbfs = Some(-80.0);
    w.clock.ppm = 31.0;
    w.seed = seed;
    w
}

/// The analysis of `schedule` in a capture, pinned at its start, from a
/// second before it to two seconds after its end.
fn analyse(
    r: &SystemRender,
    schedule: &SystemSchedule,
) -> Result<SystemAnalysis, crate::ir::TimingError> {
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

/// Unit B heard `late` frames of play-out after unit A (A at 305 frames,
/// 3.0 ms acoustic; B 5.37 ms acoustic), measured over three passes with
/// `lateness` told to the planner: the analysis (or its refusal), and the
/// world's truth of B's arrival less A's, µs.
fn late_pair(late: f64, lateness: &[(UnitIndex, u64)]) -> (Result<SystemAnalysis, String>, f64) {
    late_pair_of(1, late, lateness)
}

/// [`late_pair`] with unit `which` (0: the first to play, 1) the late one.
fn late_pair_of(
    which: u8,
    late: f64,
    lateness: &[(UnitIndex, u64)],
) -> (Result<SystemAnalysis, String>, f64) {
    let s = interleave_late(&walks(&[(0, 1), (1, 1)]), &options(3), lateness)
        .unwrap()
        .offset_by(ORIGIN + PRE)
        .unwrap();
    let world = room(
        vec![
            unit("A", 305.0 + if which == 0 { late } else { 0.0 }, 3.0),
            unit("B", 305.0 + if which == 1 { late } else { 0.0 }, 5.37),
        ],
        11,
    );
    let r = render_system(&world, &s, &ProtoModel).unwrap();
    let at = |o: UnitOutput| r.truth.output(o.unit, o.output).unwrap().arrival_us;
    let a = UnitOutput {
        unit: UnitIndex(0),
        output: 0,
    };
    let b = UnitOutput {
        unit: UnitIndex(1),
        output: 0,
    };
    (analyse(&r, &s).map_err(|e| e.to_string()), at(b) - at(a))
}

#[test]
fn wp_atlate_with_no_lateness_the_schedule_is_unchanged() {
    let w = walks(&[(0, 0b11), (1, 0b1), (2, 0b101)]);
    let plain = interleave(&w, &options(3)).unwrap();
    assert_eq!(interleave_late(&w, &options(3), &[]).unwrap(), plain);
    // Every unit equally late: nothing to plan around.
    let same: Vec<(UnitIndex, u64)> = (0..3).map(|u| (UnitIndex(u), 960)).collect();
    let s = interleave_late(&w, &options(3), &same).unwrap();
    assert_eq!(s.segments(), plain.segments());
    assert!(s.lateness().is_empty());
}

#[test]
fn wp_atlate_starts_leave_room_for_a_late_unit() {
    let w = walks(&[(0, 1), (1, 1), (2, 1)]);
    let gap = u64::from(sweep().gap_samples);
    // Unit 1 heard 150 ms after unit 0, unit 2 50 ms after it.
    let late = [
        (UnitIndex(0), 1_000),
        (UnitIndex(1), 1_000 + LATE_150_MS),
        (UnitIndex(2), 1_000 + 2_400),
    ];
    let s = interleave_late(&w, &options(3), &late).unwrap();
    // Relative to the earliest.
    assert_eq!(s.lateness_of(UnitIndex(0)), 0);
    assert_eq!(s.lateness_of(UnitIndex(1)), LATE_150_MS);
    assert_eq!(s.lateness_of(UnitIndex(2)), 2_400);
    let seg = s.segments();
    for pair in seg.windows(2) {
        let (p, n) = (&pair[0], &pair[1]);
        let extra = s.lateness_of(p.unit).saturating_sub(s.lateness_of(n.unit));
        // Each start as early as the one before allows: its end, the gap,
        // and how much later the one before is heard (unit 1 -> 2: 100 ms;
        // unit 2 -> 0 of the next pass: 50 ms; 0 -> 1: none).
        assert_eq!(n.start, p.end() + gap + extra, "{p:?} -> {n:?}");
        // Heard at least a sweep and its gap apart.
        let heard = |x: &crate::system::Segment| x.start + s.lateness_of(x.unit);
        assert!(heard(n) >= heard(p) + p.schedule.period_samples());
    }
    // The hub's rule holds as before.
    crate::system::check_lead(&s, u64::from(RATE) / 4).unwrap();
}

#[test]
fn wp_atlate_lateness_is_relative_bounded_and_not_for_a_summed_run() {
    let w = walks(&[(0, 1), (1, 1)]);
    let s = interleave(&w, &options(2)).unwrap();
    // A unit not in the run is ignored; one not named is 0.
    let t = s
        .clone()
        .with_lateness(&[(UnitIndex(1), 480), (UnitIndex(7), 9_999)])
        .unwrap();
    assert_eq!(t.lateness(), &[(UnitIndex(1), 480)]);
    // Twice: refused.
    let e = s
        .clone()
        .with_lateness(&[(UnitIndex(1), 480), (UnitIndex(1), 0)])
        .unwrap_err();
    assert!(matches!(e, MeasureError::Invalid(_)), "{e}");
    // More than a run plans for: refused, the unit named, before anything
    // plays (the planner's error).
    let bound = u64::from(RATE) * u64::from(MAX_LATENESS_MS) / 1000;
    let e = interleave_late(&w, &options(2), &[(UnitIndex(1), bound + 1)]).unwrap_err();
    assert!(e.to_string().contains("unit 1 is expected"), "{e}");
    assert!(
        interleave_late(&w, &options(2), &[(UnitIndex(1), bound)]).is_ok(),
        "at the bound"
    );
    // A summed run starts its parts apart instead.
    let sweep = sweep();
    let summed = SystemSchedule::summed(
        D,
        RATE,
        0,
        vec![
            (UnitIndex(0), WalkSchedule::new(sweep, 1, 1).unwrap()),
            (UnitIndex(1), WalkSchedule::new(sweep, 1, 1).unwrap()),
        ],
    )
    .unwrap();
    assert!(
        summed
            .clone()
            .with_lateness(&[(UnitIndex(1), 480)])
            .is_err()
    );
    assert!(summed.with_lateness(&[]).is_ok());
}

/// The review's finding, end to end in ATSIM's room: unit B heard 150 ms
/// after unit A. Without the lateness nothing correct is offered (its
/// arrival is outside what the analysis keeps); told the lateness, its
/// delay is offered within 2 µs of the world's and within three of its own
/// uncertainties.
#[test]
fn wp_atlate_a_unit_heard_150_ms_late_is_measured_with_its_lateness() {
    let late = LATE_150_MS as f64;
    let (plain, truth) = late_pair(late, &[]);
    match &plain {
        Err(e) => eprintln!("without lateness: {e}"),
        Ok(a) => {
            let d = &a.delays[0];
            eprintln!(
                "without lateness: offered {} at {:.3} ms ({})",
                d.offered, d.delay_ms, d.reason
            );
            assert!(
                !d.offered || (d.delay_ms * 1000.0 - truth).abs() > 2.0 || !a.timing_reliable,
                "measured right without the lateness: the test does not show the case"
            );
        }
    }
    let (told, truth) = late_pair(late, &[(UnitIndex(1), LATE_150_MS)]);
    let a = told.unwrap();
    assert!(a.timing_reliable, "{:?}", a.reasons);
    let d = &a.delays[0];
    let err = d.delay_ms * 1000.0 - truth;
    let u = d.uncertainty_us.unwrap();
    eprintln!(
        "with lateness: {:.3} ms, truth {:.3} ms, error {err:.3} µs, u {u:.3} µs",
        d.delay_ms,
        truth / 1000.0
    );
    assert!(d.offered, "{d:?}");
    assert!(err.abs() <= 2.0, "{err} µs off");
    assert!(err.abs() <= 3.0 * u.max(0.25), "{err} µs, u {u}");
    // The arrivals stay on the one timeline, the earliest at 0, and the
    // response (on the analysis' timeline) is the lateness early.
    let b = a.output(UnitIndex(1), 0).unwrap();
    let a0 = a.output(UnitIndex(0), 0).unwrap();
    assert!(a0.arrival_ms.abs() < 1e-9, "{a0:?}");
    assert!((b.lateness_ms - 150.0).abs() < 1e-9, "{b:?}");
    assert!((b.arrival_ms - truth / 1000.0).abs() < 0.002, "{b:?}");
}

/// A wrong expectation fails closed: told 150 ms where B is on time, the
/// analysis looks for it 150 ms late and does not offer a wrong delay.
#[test]
fn wp_atlate_a_wrong_lateness_offers_nothing_wrong() {
    let (told, truth) = late_pair(0.0, &[(UnitIndex(1), LATE_150_MS)]);
    match told {
        Err(e) => eprintln!("refused: {e}"),
        Ok(a) => {
            let d = &a.delays[0];
            eprintln!(
                "offered {} at {:.3} ms ({}), truth {:.3} ms",
                d.offered,
                d.delay_ms,
                d.reason,
                truth / 1000.0
            );
            assert!(
                !(d.offered && a.timing_reliable) || (d.delay_ms * 1000.0 - truth).abs() <= 2.0,
                "a wrong delay offered: {d:?}"
            );
        }
    }
}

/// The same with the first unit to play the late one: the run's first
/// cycle is then looked for its lateness after its START.
#[test]
fn wp_atlate_the_first_unit_heard_150_ms_late_is_measured_with_its_lateness() {
    let (told, truth) = late_pair_of(0, LATE_150_MS as f64, &[(UnitIndex(0), LATE_150_MS)]);
    let a = told.unwrap();
    assert!(a.timing_reliable, "{:?}", a.reasons);
    let d = &a.delays[0];
    let err = d.delay_ms * 1000.0 - truth;
    eprintln!(
        "first late: {:.3} ms, truth {:.3} ms, error {err:.3} µs",
        d.delay_ms,
        truth / 1000.0
    );
    assert!(d.offered, "{d:?}");
    assert!(err.abs() <= 2.0, "{err} µs off");
}

// -- review AT-E --
/// [`analyse`] with every unit's status polls, one each 250 ms of the run:
/// what its generator says then (its own segments' walk), noted at the
/// capture frame that sound made at that moment is heard at.
fn analyse_polled(
    r: &SystemRender,
    schedule: &SystemSchedule,
) -> Result<SystemAnalysis, crate::ir::TimingError> {
    use crate::ir::StatusObservation;
    use crate::system::UnitStatusObservation;
    let at = |t: u64| r.capture_frame_at_mic(t as f64 - ORIGIN as f64).unwrap();
    let mut status = Vec::new();
    // 100 ms after a cycle's start and so, with 1.0 s sweeps, 100 ms after
    // one's end too, where the generator has moved on to its next output.
    let mut t = schedule.start() + u64::from(RATE) / 10;
    while t < schedule.end() {
        for u in schedule.units() {
            // The unit's segment that plays (or last played) at `t`.
            let Some(seg) = schedule
                .segments()
                .iter()
                .rfind(|s| s.unit == u && s.start <= t)
            else {
                continue;
            };
            let st = seg.schedule.status_at((t - seg.start) as f64);
            if st.active_channel == crate::stimulus::NO_CHANNEL {
                continue;
            }
            status.push(UnitStatusObservation {
                unit: u,
                observation: StatusObservation {
                    capture_frame: at(t).round() as u64,
                    status: st,
                },
            });
        }
        t += u64::from(RATE) / 4;
    }
    assert!(status.len() > 20);
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
            status: &status,
            complete: true,
        },
        &AnalysisOptions::default(),
    )
}

/// The run's first unit the late one, by more than the first cycle's search
/// window (500 ms after its START) and than the status polls' slack
/// (250 ms): 400 ms and 900 ms, with every unit's polls. Before this review
/// 900 ms failed "the first cycle was not heard in the window the START
/// implies" and 400 ms failed the status cross-check (the polls were judged
/// against where the first cycle was heard, 400 ms late); now the first
/// cycle is looked for, and the polls are judged, its lateness later, and
/// the delay is offered within 2 us of the world's.
#[test]
fn review_ate_the_first_unit_late_past_the_search_window_is_found_and_its_polls_agree() {
    for late in [19_200u64, 43_200] {
        eprintln!("first unit {late} samples late");
        let lateness = [(UnitIndex(0), late)];
        // Two outputs a unit: its status changes within its segments.
        let s = interleave_late(&walks(&[(0, 3), (1, 3)]), &options(3), &lateness)
            .unwrap()
            .offset_by(ORIGIN + PRE)
            .unwrap();
        let two = |name: &str, playout: f64, delay_ms: f64| {
            let mut u = SimUnit::flat(name, 2);
            u.playout_frames = playout;
            for p in &mut u.outputs {
                p.delay_ms = delay_ms;
                p.gain_db = -6.0;
            }
            u
        };
        let world = room(
            vec![two("A", 305.0 + late as f64, 3.0), two("B", 305.0, 5.37)],
            11,
        );
        let r = render_system(&world, &s, &ProtoModel).unwrap();
        let truth = |u: u8| {
            r.truth
                .output(UnitIndex(u), 0)
                .map(|o| o.arrival_us)
                .unwrap()
        };
        let a = analyse_polled(&r, &s).unwrap_or_else(|e| panic!("{late} samples late: {e}"));
        assert!(a.timing_reliable, "{late}: {:?}", a.reasons);
        let d = a
            .delays
            .iter()
            .find(|d| d.from.output == 0 && d.to.output == 0)
            .unwrap();
        let err = d.delay_ms * 1000.0 - (truth(1) - truth(0));
        eprintln!(
            "{late} samples late: {:.3} ms, error {err:.3} us",
            d.delay_ms
        );
        assert!(a.delays.iter().all(|d| d.offered), "{late}: {:?}", a.delays);
        assert!(err.abs() <= 2.0, "{late}: {err} us off");
        assert_eq!(a.walked.cycles.len(), 12);
    }
}

/// A wrong lateness of the first unit is still refused. Told 900 ms late
/// where it is on time, with one output a unit nothing is heard where its
/// first cycle is looked for; with two, its second output's cycle (1.3 s
/// after its START) lies in that window and would be taken for the first, a
/// cycle late: the sweep heard before the window refuses it.
#[test]
fn review_ate_a_first_unit_whose_lateness_is_wrong_is_refused() {
    let told = [(UnitIndex(0), 43_200u64)];
    for outputs in [1usize, 2] {
        let mask = if outputs == 2 { 3 } else { 1 };
        let s = interleave_late(&walks(&[(0, mask), (1, mask)]), &options(3), &told)
            .unwrap()
            .offset_by(ORIGIN + PRE)
            .unwrap();
        let on_time = |name: &str, delay_ms: f64| {
            let mut u = SimUnit::flat(name, outputs);
            u.playout_frames = 305.0;
            for p in &mut u.outputs {
                p.delay_ms = delay_ms;
                p.gain_db = -6.0;
            }
            u
        };
        let world = room(vec![on_time("A", 3.0), on_time("B", 5.37)], 11);
        let r = render_system(&world, &s, &ProtoModel).unwrap();
        match analyse_polled(&r, &s) {
            Err(e) => eprintln!("{outputs} output(s): refused: {e}"),
            Ok(a) => {
                eprintln!("{outputs} output(s): {:?} {:?}", a.reasons, a.delays);
                assert!(
                    !a.timing_reliable || a.delays.iter().all(|d| !d.offered),
                    "offered: {:?}",
                    a.delays
                );
            }
        }
    }
}
