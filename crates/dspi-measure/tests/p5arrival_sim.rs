//! WP P5ARRIVAL on P5SIM's world: the microphone clock's smooth wander of 55
//! to 100 ppm (P5SIM's ratio wander; P5SPIKE's iMM-6C showed bursts of that
//! rate) is reported as clock wander inside a sweep, a slip between passes as
//! a capture slip, and the clean world stays reliable with no finding.
#![cfg(feature = "sim")]

mod common;

use common::{analyse, walked_run};
use dspi_measure::ir::{AnalysisOptions, TimingIssue, WalkedAnalysis};
use dspi_measure::sim::{CaptureFault, World, three_way_chains};
use dspi_measure::stimulus::SweepConfig;

const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 1_000,
    gap_ms: 400,
};
/// One cycle and its gap, frames.
const PERIOD: u64 = 67_200;

fn run(faults: Vec<CaptureFault>) -> WalkedAnalysis {
    let mut w = World::three_way_room(11);
    w.clock.ppm = 37.5;
    w.faults = faults;
    let r = walked_run(&w, &three_way_chains(), 0b111, 3, &CFG, -12.0, 1_000);
    analyse(&r, &w, &AnalysisOptions::default()).unwrap()
}

fn wander(a: &WalkedAnalysis) -> Vec<(f64, f64)> {
    a.quality
        .issues
        .iter()
        .filter_map(|i| match i {
            TimingIssue::ClockWander {
                wander_us,
                rate_ppm,
                ..
            } => Some((*wander_us, *rate_ppm)),
            _ => None,
        })
        .collect()
}

#[test]
fn wp_p5arrival_clean_world_has_no_timing_finding() {
    let a = run(Vec::new());
    assert!(a.quality.reliable, "{:?}", a.quality);
    assert!(a.quality.issues.is_empty(), "{:?}", a.quality.issues);
    assert!(a.cycles.iter().all(|c| !c.arrival_ambiguous));
}

#[test]
fn wp_p5arrival_ratio_wander_of_55_to_100_ppm_is_reported_as_wander() {
    for (ppm, period_s) in [(55.0, 1.0), (80.0, 1.2), (100.0, 1.5)] {
        let a = run(vec![CaptureFault::RatioWander {
            amplitude_ppm: ppm,
            period_s,
        }]);
        assert!(!a.quality.reliable, "{ppm} ppm: {:?}", a.quality);
        let w = wander(&a);
        assert!(!w.is_empty(), "{ppm} ppm: {:?}", a.quality);
        let fastest = w.iter().map(|x| x.1).fold(0.0, f64::max);
        assert!(
            fastest > 0.4 * ppm && fastest < 3.0 * ppm,
            "{ppm} ppm: {w:?}"
        );
        assert!(
            a.quality
                .reasons
                .iter()
                .any(|r| r.contains("the capture clock wandered")),
            "{:?}",
            a.quality.reasons
        );
    }
}

#[test]
fn wp_p5arrival_a_slip_between_passes_is_a_capture_slip_not_wander() {
    // +3 frames in the gap between passes 2 and 3 (after cycle 5).
    let a = run(vec![CaptureFault::StepSlip {
        at_frame: 48_000 + 2_400 + 5 * PERIOD + 60_000,
        frames: 3,
    }]);
    assert!(!a.quality.reliable);
    assert!(
        a.quality
            .issues
            .iter()
            .any(|i| matches!(i, TimingIssue::CaptureSlip { .. })),
        "{:?}",
        a.quality
    );
    assert!(wander(&a).is_empty(), "{:?}", a.quality);
}

/// P5TUNE's follow-up: the ramp stops at its first step heard, often the
/// -50 dBFS start; a woofer behind its LR4 low-pass in P5SIM's noise (-85
/// and -90 dBFS) must still be identified by the cycle-identity search (the
/// way's own band, whitened by the pre-roll's noise), and its cycles found.
#[test]
fn wp_p5arrival_a_low_passed_woofer_at_the_ramps_start_level_is_identified() {
    let cfg = SweepConfig {
        f1_hz: 20.0,
        f2_hz: 20_000.0,
        duration_ms: 5_500,
        gap_ms: 1_000,
    };
    let worlds = [
        (
            World::three_way_room(11),
            three_way_chains(),
            0b001u16,
            "three-way room, woofer (LR4 300 Hz)",
        ),
        (
            World::two_way_near_field(5),
            dspi_measure::sim::two_way_chains(),
            0b01,
            "two-way near field, woofer (LR4 2.2 kHz)",
        ),
        (
            World::two_way_near_field(5),
            dspi_measure::sim::two_way_chains(),
            0b11,
            "two-way near field, both ways",
        ),
    ];
    for (w, chains, mask, name) in worlds {
        let r = walked_run(&w, &chains, mask, 3, &cfg, -50.0, 1_000);
        match analyse(&r, &w, &AnalysisOptions::default()) {
            Ok(a) => {
                assert_eq!(a.cycles.len(), 3 * mask.count_ones() as usize, "{name}");
                assert!(a.quality.max_residual_us < 10.0, "{name}: {:?}", a.quality);
            }
            Err(e) => panic!("{name}: {e}"),
        }
        // Still fail-closed: a START noted one cycle off is refused.
        let mut late = walked_run(&w, &chains, mask, 3, &cfg, -50.0, 1_000);
        late.start_frame += 312_000;
        assert!(
            matches!(
                analyse(&late, &w, &AnalysisOptions::default()),
                Err(dspi_measure::ir::TimingError::CycleIdentity(_))
            ),
            "{name}: a START one cycle late was accepted"
        );
    }
}
