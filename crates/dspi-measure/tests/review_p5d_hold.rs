//! Review P5-D: the timing hold of WP P5TRUST2 (a candidate's relative delay
//! held within a quarter period of the arrival-based one) on crossover
//! families other than the LR4 it was fitted on.
//!
//! The arrivals carry the measured chain's own group delay: the envelope of
//! a steep low-pass peaks late. Through P5SIM's room the woofer's arrival
//! behind an LR8 low-pass at 300 Hz is 1.25 ms (0.38 of the period) later
//! than its acoustic offset says, so the right alignment (+0.13 ms, no flip:
//! every driver is wired right) lies outside the hold, and the best inside it
//! is the half-cycle-off one with the mid flipped. Before the review that was
//! proposed (+1.41 ms and a flip). Now a held choice that flips the polarity
//! or lies on the hold's edge is no timing: the delay and polarity are kept,
//! and said.
//!
//! `review_p5d_probe_hold_by_family` (ignored) prints the same for eight
//! families at two corners:
//! `cargo test -p dspi-measure --all-features --release --test review_p5d_hold -- --ignored --nocapture`.
#![cfg(feature = "sim")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::print_stdout
)]

mod common;

use common::{analyse, walked_run, ways_of};
use dspi_measure::align::{AlignGroup, AlignOptions, AlignResult, Transition, Way, align};
use dspi_measure::ir::{ALIGNMENT_WINDOW_CYCLES, AnalysisOptions};
use dspi_measure::sim::{ProtoModel, World, three_way_chains};
use dspi_measure::stimulus::SweepConfig;
use dspi_measure::target::{Builtin, DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse};
use dspi_measure::types::{LogGrid, Response, WayRole};
use dspi_measure::xover::{XoConstraints, XoProposal, crossover_corner, de_embed, timing_hold_ms};

const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 2_000,
    gap_ms: 1_000,
};
const ROLES: [WayRole; 3] = [WayRole::Low, WayRole::Mid, WayRole::High];

fn transitions() -> Vec<Transition> {
    vec![
        Transition {
            low: 0,
            high: 1,
            corner_hz: None,
        },
        Transition {
            low: 1,
            high: 2,
            corner_hz: None,
        },
    ]
}

fn flat_target() -> Response {
    Target {
        target_use: TargetUse::Speakers,
        coupler: None,
        definition: TargetDefinition::Builtin {
            builtin: Builtin::Flat,
        },
        reference: DEFAULT_REFERENCE_BAND,
    }
    .evaluate(&LogGrid::STANDARD)
    .unwrap()
}

/// P5SIM's three-way room, every driver wired right (acoustic offsets 0.25,
/// 0.12 and 0 ms), measured through a crossover of the family `(lp, hp)`
/// with the lower corner at `f1`; aligned alone, and through the corner
/// search.
fn tuned(lp: u8, hp: u8, f1: f32) -> Option<(Vec<Way>, AlignResult, XoProposal)> {
    let mut world = World::three_way_room(5);
    for o in &mut world.outputs {
        o.driver.inverted = false;
    }
    let mut chains = three_way_chains();
    for side in 0..2 {
        chains[side * 3].crossover[0].filter_type = lp;
        chains[side * 3].crossover[0].freq = f1;
        chains[side * 3 + 1].crossover[0].filter_type = hp;
        chains[side * 3 + 1].crossover[0].freq = f1;
        chains[side * 3 + 1].crossover[1].filter_type = lp;
        chains[side * 3 + 2].crossover[0].filter_type = hp;
    }
    let run = walked_run(&world, &chains, 0b111, 3, &CFG, -12.0, 1_000);
    let a = analyse(&run, &world, &AnalysisOptions::default()).ok()?;
    let ways = ways_of(&a, &chains, &ROLES, ALIGNMENT_WINDOW_CYCLES, None);
    let aligned = align(
        &ways,
        &AlignGroup {
            transitions: transitions(),
        },
        &AlignOptions::default(),
    )
    .ok()?;
    let drivers: Vec<_> = ways
        .iter()
        .map(|w| de_embed(w, &ProtoModel).unwrap())
        .collect();
    let xo = crossover_corner(
        &drivers,
        &transitions(),
        &flat_target(),
        &XoConstraints::default(),
        &ProtoModel,
    )
    .ok()?;
    Some((ways, aligned, xo))
}

#[test]
fn review_p5d_the_hold_never_flips_a_correctly_wired_lr8_pair() {
    // LR4, what the hold was fitted on: the acoustic offset is found (the
    // mid 0.13 ms behind the woofer), nothing flipped, nothing held.
    let (_, lr4, lr4_xo) = tuned(34, 35, 300.0).unwrap();
    let p = &lr4.pairs[0];
    assert!(!p.flipped, "{lr4:?}");
    assert!((p.relative_delay_ms - 0.13).abs() < 0.05, "{lr4:?}");
    assert!(!lr4.notes.iter().any(|n| n.contains("arrivals")), "{lr4:?}");
    assert!(!lr4_xo.transitions[0].flipped, "{lr4_xo:?}");

    // LR8: the woofer's arrival is more than a quarter period late.
    let (ways, lr8, lr8_xo) = tuned(38, 39, 300.0).unwrap();
    let start = ways[0].arrival_ms - ways[1].arrival_ms;
    println!(
        "LR8 start {start:.3} ms, hold {:.3} ms",
        timing_hold_ms(300.0)
    );
    println!("LR8 align {:?} {:?}", lr8.pairs[0], lr8.notes);
    println!("LR8 xo {:?} {:?}", lr8_xo.transitions[0], lr8_xo.notes);
    assert!(start - 0.13 > timing_hold_ms(300.0), "{start}");
    // Alignment alone: no flip, the delays as they were, and said.
    let p = &lr8.pairs[0];
    assert!(!p.flipped, "{lr8:?}");
    assert_eq!(p.relative_delay_ms, 0.0, "{lr8:?}");
    assert!(
        lr8.ways[..2]
            .iter()
            .all(|w| w.inverted == w.inverted_before),
        "{lr8:?}"
    );
    assert!(
        lr8.notes
            .iter()
            .any(|n| n.contains("outputs 0 and 1") && n.contains("are kept")),
        "{lr8:?}"
    );
    // The corner search: whatever corner it takes, the mid is not flipped.
    assert!(!lr8_xo.transitions[0].flipped, "{lr8_xo:?}");
    assert!(
        lr8_xo.outputs[..2].iter().all(|o| !o.chain.feed.inverted),
        "{lr8_xo:?}"
    );
}

#[test]
#[ignore = "a probe: prints, asserts nothing"]
fn review_p5d_probe_hold_by_family() {
    // (name, low-pass type, high-pass type)
    let fams: [(&str, u8, u8); 8] = [
        ("LR2", 32, 33),
        ("LR4", 34, 35),
        ("LR6", 36, 37),
        ("LR8", 38, 39),
        ("BW2", 42, 43),
        ("BW3", 44, 45),
        ("BW4", 46, 47),
        ("BES4", 58, 59),
    ];
    for (name, lp, hp) in fams {
        for f1 in [300.0f32, 120.0] {
            let Some((ways, aligned, xo)) = tuned(lp, hp, f1) else {
                println!("== {name} {f1} Hz: no result");
                continue;
            };
            println!(
                "== {name} f1 {f1} Hz, arrivals {:?} (acoustic offsets 0.25, 0.12, 0 ms)",
                ways.iter()
                    .map(|w| (w.arrival_ms * 1000.0).round() / 1000.0)
                    .collect::<Vec<_>>()
            );
            for (p, t) in aligned.pairs.iter().zip(&transitions()) {
                let start = ways[t.low].arrival_ms - ways[t.high].arrival_ms;
                let per = 1000.0 / p.corner_hz;
                println!(
                    "   align {:.0} Hz: start {start:+.3} ms, chosen {:+.3} ms ({:+.3} T off start, hold {:.3} ms), flipped {}, confidence {:.2}",
                    p.corner_hz,
                    p.relative_delay_ms,
                    (p.relative_delay_ms - start) / per,
                    timing_hold_ms(p.corner_hz),
                    p.flipped,
                    p.confidence
                );
            }
            for n in aligned.notes.iter().filter(|n| n.contains("arrivals")) {
                println!("   align note: {n}");
            }
            for (t, tr) in xo.transitions.iter().zip(&transitions()) {
                let start = ways[tr.low].arrival_ms - ways[tr.high].arrival_ms;
                let per = 1000.0 / t.corner_hz;
                println!(
                    "   xo {:.0} -> {:.0} Hz: start {start:+.3} ms, chosen {:+.3} ms ({:+.3} T off start), flipped {}",
                    t.corner_before_hz,
                    t.corner_hz,
                    t.relative_delay_ms,
                    (t.relative_delay_ms - start) / per,
                    t.flipped
                );
            }
            for n in xo.notes.iter().filter(|n| n.contains("arrivals")) {
                println!("   xo note: {n}");
            }
        }
    }
}
