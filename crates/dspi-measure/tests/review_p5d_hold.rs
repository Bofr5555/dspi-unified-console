//! Review P5-D: the timing hold of WP P5TRUST2 (a candidate's relative delay
//! held within a quarter period of the arrival-based one) on crossover
//! families other than the LR4 it was fitted on; WP P5HOLD: its anchor.
//!
//! The arrivals carry the measured chain's own onset: a steep low-pass rises
//! slowly. Through P5SIM's room the woofer's arrival behind an LR8 low-pass
//! at 300 Hz is 1.25 ms (0.38 of the period) later than its acoustic offset
//! says, so the right alignment lay outside the hold, and the best inside it
//! was the half-cycle-off one with the mid flipped (proposed before the
//! review: +1.41 ms and a flip). The review made such a held choice no
//! timing (kept, and said: `held_unclear`). WP P5HOLD anchors the hold on
//! the drivers' own arrivals instead (each arrival less its measured chain's
//! onset), and the steep pairs get their timing again.
//!
//! **The right timing** here is the truth of the simulated speaker: the
//! anechoic ways (P5SIM's drivers, their offsets, the chains; no room, no
//! noise, no window) and `align`'s own rule, the phase agreement through one
//! octave each side of the corner (quadrature for BW3). It is not always the
//! drivers' offset with the family's polarity: the room's midrange has its
//! own second-order high-pass at 120 Hz, so at a 120 Hz corner the right
//! timing lies about a third of a period earlier, and an LR2 or BW2 pair
//! there needs no inversion while an LR4, BW4 or LR8 pair does.
//!
//! `review_p5d_probe_hold_by_family` (ignored) prints every family at both
//! corners against the truth:
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
use dspi_measure::ir::{AnalysisOptions, C64, FIT_WINDOW_CYCLES};
use dspi_measure::sim::{ProtoModel, SimFilter, World, three_way_chains};
use dspi_measure::stimulus::SweepConfig;
use dspi_measure::target::{Builtin, DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse};
use dspi_measure::types::{ChannelChain, Coeffs, LogGrid, Response, SectionModel, WayRole};
use dspi_measure::xover::{XoConstraints, XoProposal, crossover_corner, de_embed, timing_hold_ms};

const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 2_000,
    gap_ms: 1_000,
};
const ROLES: [WayRole; 3] = [WayRole::Low, WayRole::Mid, WayRole::High];

/// The families of the review's probe: (name, low-pass type, high-pass type).
const FAMILIES: [(&str, u8, u8); 8] = [
    ("LR2", 32, 33),
    ("LR4", 34, 35),
    ("LR6", 36, 37),
    ("LR8", 38, 39),
    ("BW2", 42, 43),
    ("BW3", 44, 45),
    ("BW4", 46, 47),
    ("BES4", 58, 59),
];
/// The odd-order Butterworth family: aligned to quadrature, never flipped.
const BW3_LP: u8 = 44;

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

/// P5SIM's three-way room, every driver wired right.
fn room() -> World {
    let mut world = World::three_way_room(5);
    for o in &mut world.outputs {
        o.driver.inverted = false;
    }
    world
}

/// P5SIM's three-way room, every driver wired right (acoustic offsets 0.25,
/// 0.12 and 0 ms), measured through a crossover of the family `(lp, hp)`
/// with the lower corner at `f1`; aligned alone, and through the corner
/// search, on the responses the core gives them (`FIT_WINDOW_CYCLES`).
fn tuned(lp: u8, hp: u8, f1: f32) -> Option<(Vec<Way>, AlignResult, XoProposal)> {
    let world = room();
    let chains = family_chains(lp, hp, f1);
    let run = walked_run(&world, &chains, 0b111, 3, &CFG, -12.0, 1_000);
    let a = analyse(&run, &world, &AnalysisOptions::default()).ok()?;
    let ways = ways_of(&a, &chains, &ROLES, FIT_WINDOW_CYCLES, None);
    let aligned = align(
        &ways,
        &AlignGroup {
            transitions: transitions(),
        },
        &AlignOptions::default(),
        &ProtoModel,
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
    // WP P5HOLD: the hold is anchored on the drivers' own arrivals, so the
    // pair now gets its timing (before: kept, review P5-D F1): no flip, the
    // truth's delay within a tenth of a period, nothing held.
    let world = room();
    let chains = family_chains(38, 39, 300.0);
    let (td, tf, _, _) = truth(&world, &chains, 0, 1, 300.0, false);
    assert!(!tf, "the truth flips nothing");
    let p = &lr8.pairs[0];
    assert!(!p.flipped, "{lr8:?}");
    assert!(
        (p.relative_delay_ms - td).abs() < 0.1 * 1000.0 / 300.0,
        "{lr8:?} truth {td}"
    );
    assert!(
        lr8.ways[..2]
            .iter()
            .all(|w| w.inverted == w.inverted_before),
        "{lr8:?}"
    );
    assert!(!lr8.notes.iter().any(|n| n.contains("arrivals")), "{lr8:?}");
    // The corner search: whatever corner it takes, the mid is not flipped.
    assert!(!lr8_xo.transitions[0].flipped, "{lr8_xo:?}");
    assert!(
        lr8_xo.outputs[..2].iter().all(|o| !o.chain.feed.inverted),
        "{lr8_xo:?}"
    );
}

// -- WP P5HOLD -- the truth the probe is judged against.

/// One section's complex response at `f` and `fs` (z = e^{jw}).
fn biquad_at(c: &Coeffs, f: f64, fs: f64) -> C64 {
    let w = std::f64::consts::TAU * f / fs;
    let z1 = C64::from_polar(1.0, -w);
    let z2 = C64::from_polar(1.0, -2.0 * w);
    (c.b0 + c.b1 * z1 + c.b2 * z2) / (1.0 + c.a1 * z1 + c.a2 * z2)
}

/// P5SIM's driver filter (the RBJ section its renderer runs).
fn sim_section(f: &SimFilter, fs: f64) -> Coeffs {
    let (freq, q, gain_db) = match *f {
        SimFilter::Peak { freq, q, gain_db }
        | SimFilter::LowShelf { freq, q, gain_db }
        | SimFilter::HighShelf { freq, q, gain_db } => (freq, q, gain_db),
        SimFilter::HighPass { freq, q } | SimFilter::LowPass { freq, q } => (freq, q, 0.0),
    };
    let w = std::f64::consts::TAU * freq / fs;
    let (sn, cs) = w.sin_cos();
    let alpha = sn / (2.0 * q);
    let a = 10f64.powf(gain_db / 40.0);
    let (b0, b1, b2, a0, a1, a2) = match f {
        SimFilter::Peak { .. } => (
            1.0 + alpha * a,
            -2.0 * cs,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cs,
            1.0 - alpha / a,
        ),
        SimFilter::HighPass { .. } => (
            (1.0 + cs) / 2.0,
            -(1.0 + cs),
            (1.0 + cs) / 2.0,
            1.0 + alpha,
            -2.0 * cs,
            1.0 - alpha,
        ),
        SimFilter::LowPass { .. } => (
            (1.0 - cs) / 2.0,
            1.0 - cs,
            (1.0 - cs) / 2.0,
            1.0 + alpha,
            -2.0 * cs,
            1.0 - alpha,
        ),
        _ => unreachable!("no shelf in the three-way room's drivers"),
    };
    Coeffs {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

/// A way's programme contribution in free field, no room, no noise: the
/// driver (its filters, polarity and acoustic offset) through its chain.
fn anechoic(world: &World, chains: &[ChannelChain], output: usize, f: f64) -> C64 {
    let fs = ProtoModel.sample_rate();
    let d = &world.outputs[output].driver;
    let c = &chains[output];
    let level = 10f64.powf((d.sensitivity_db + world.outputs[output].gain_db) / 20.0);
    let mut h = C64::new(if d.inverted { -level } else { level }, 0.0);
    for s in d.filters() {
        h *= biquad_at(&sim_section(&s, fs), f, fs);
    }
    for b in c.peq.iter().chain(&c.crossover) {
        for s in ProtoModel.sections(b) {
            h *= biquad_at(&s, f, fs);
        }
    }
    if c.feed.inverted {
        h = -h;
    }
    h * C64::from_polar(1.0, -std::f64::consts::TAU * f * d.offset_ms / 1000.0)
}

/// The right timing of transition `low`-`high` at `corner`: the anechoic
/// speaker's phase agreement through one octave each side (align's rule),
/// searched over a period each side of the acoustic offsets. `(delay, flip,
/// cost, best cost of the other polarity)`.
fn truth(
    world: &World,
    chains: &[ChannelChain],
    low: usize,
    high: usize,
    corner: f64,
    quadrature: bool,
) -> (f64, bool, f64, f64) {
    let pts: Vec<(f64, C64, C64)> = LogGrid::STANDARD
        .freqs()
        .unwrap()
        .into_iter()
        .filter(|f| *f >= corner / 2.0 && *f <= corner * 2.0)
        .map(|f| {
            (
                f,
                anechoic(world, chains, low, f),
                anechoic(world, chains, high, f),
            )
        })
        .collect();
    let cost = |delta: f64, flip: bool| {
        let (mut num, mut den) = (0.0, 0.0);
        for (f, a, b) in &pts {
            let b = *b
                * if flip { -1.0 } else { 1.0 }
                * C64::from_polar(1.0, -std::f64::consts::TAU * f * delta / 1000.0);
            let w = a.norm() * b.norm() / (a.norm_sqr() + b.norm_sqr());
            let cos = (a * b.conj()).re / (a.norm() * b.norm());
            num += w * if quadrature { cos * cos } else { 1.0 - cos };
            den += w;
        }
        (num / den).sqrt()
    };
    let offset = world.outputs[low].driver.offset_ms - world.outputs[high].driver.offset_ms;
    let period = 1000.0 / corner;
    let mut best = [(0.0, f64::INFINITY); 2];
    for k in -2000..=2000 {
        let d = offset + period * f64::from(k) / 2000.0;
        for flip in [false, true] {
            if quadrature && flip {
                continue;
            }
            let c = cost(d, flip);
            if c < best[usize::from(flip)].1 {
                best[usize::from(flip)] = (d, c);
            }
        }
    }
    let flip = best[1].1 < best[0].1 * 0.9;
    let (d, c) = best[usize::from(flip)];
    (d, flip, c, best[usize::from(!flip)].1)
}

/// The three-way room's chains with the family `(lp, hp)`, lower corner `f1`.
fn family_chains(lp: u8, hp: u8, f1: f32) -> Vec<ChannelChain> {
    let mut chains = three_way_chains();
    for side in 0..2 {
        chains[side * 3].crossover[0].filter_type = lp;
        chains[side * 3].crossover[0].freq = f1;
        chains[side * 3 + 1].crossover[0].filter_type = hp;
        chains[side * 3 + 1].crossover[0].freq = f1;
        chains[side * 3 + 1].crossover[1].filter_type = lp;
        chains[side * 3 + 2].crossover[0].filter_type = hp;
    }
    chains
}

/// The chains of a proposal's corners with the polarities and delays the
/// speaker was measured with: what the truth of its transitions is judged on.
fn proposal_corners(xo: &XoProposal, before: &[ChannelChain]) -> Vec<ChannelChain> {
    let mut c = before.to_vec();
    for (i, o) in xo.outputs.iter().enumerate() {
        c[i].crossover.clone_from(&o.chain.crossover);
    }
    c
}

/// Whether a proposed pair timing is the truth's: the same polarity, the
/// delay within a tenth of the corner's period.
fn agrees(delay: f64, flip: bool, truth: (f64, bool, f64, f64), corner: f64) -> bool {
    flip == truth.1 && (delay - truth.0).abs() <= 0.1 * 1000.0 / corner
}

/// One family of the review's probe (review P5-D D1) as a test: the lower
/// corner at 300 and at 120 Hz, alignment alone and the corner search, on
/// the responses the core gives them (`FIT_WINDOW_CYCLES`, as
/// `dspi-core`'s `runs::output_response`).
///
/// - At 300 Hz and 3 kHz every pair's timing is the truth's (polarity, and
///   the delay within a tenth of a period), and nothing is held (the review
///   saw LR6 and LR8 kept, and before it flipped).
/// - At 120 Hz the midrange's own high-pass sits on the corner and moves the
///   right timing about a third of a period earlier, which the arrivals do
///   not see: a pair there gets the truth's timing or keeps its timing with
///   the note (`held_unclear`: review P5-D F1 and P5HOLD's polarity guard).
///   Never a timing that disagrees with the truth.
fn wp_p5hold_family(name: &str, lp: u8, hp: u8) {
    family_check(name, lp, hp, true);
}

// -- WP P5FOLLOW -- (review P5-E's mutants)
/// [`wp_p5hold_family`] with the hold anchored on the raw arrivals (as
/// before WP P5HOLD; `xover::sim_without_onset_anchor`): the guards alone
/// must then keep what they cannot settle, at any corner, and never propose
/// a timing that disagrees with the truth. The review saw BW3 at 120 Hz
/// proposed (+1.03 ms, the truth -2.32 ms) for want of an odd-order guard;
/// LR8 at 300 Hz is kept by the polarity guard.
fn wp_p5follow_family_without_the_anchor(name: &str, lp: u8, hp: u8) {
    dspi_measure::xover::sim_without_onset_anchor(|| family_check(name, lp, hp, false));
}

fn family_check(name: &str, lp: u8, hp: u8, anchored: bool) {
    let world = room();
    let q = lp == BW3_LP;
    let mut failures = Vec::new();
    for f1 in [300.0f32, 120.0] {
        let chains = family_chains(lp, hp, f1);
        let (_, aligned, xo) = tuned(lp, hp, f1).unwrap();
        let may_keep = f1 < 200.0 || !anchored;
        // -- WP UFW -- the notes count the outputs from 1
        let held_note = |notes: &[String], low: u8, high: u8| {
            notes.iter().any(|n| {
                n.contains(&format!("outputs {} and {}", low + 1, high + 1))
                    && n.contains("could not be settled")
            })
        };
        for (p, t) in aligned.pairs.iter().zip(transitions()) {
            let tr = truth(&world, &chains, t.low, t.high, p.corner_hz, q);
            let kept = p.relative_delay_ms == 0.0
                && !p.flipped
                && held_note(&aligned.notes, p.low_output, p.high_output);
            let ok = agrees(p.relative_delay_ms, p.flipped, tr, p.corner_hz) || (may_keep && kept);
            println!(
                "{name} {f1} Hz align {:.0} Hz: {:+.3} ms flip {} | truth {:+.3} ms flip {} | {}",
                p.corner_hz,
                p.relative_delay_ms,
                p.flipped,
                tr.0,
                tr.1,
                if !ok {
                    "WRONG"
                } else if kept {
                    "kept"
                } else {
                    "ok"
                }
            );
            if !ok {
                failures.push(format!("{name} {f1} Hz align {:.0} Hz", p.corner_hz));
            }
        }
        let corners = proposal_corners(&xo, &chains);
        for (x, t) in xo.transitions.iter().zip(transitions()) {
            let tr = truth(&world, &corners, t.low, t.high, x.corner_hz, q);
            let kept = x.relative_delay_ms == 0.0
                && !x.flipped
                && held_note(&xo.notes, x.low_output, x.high_output);
            let ok = (x.kept_reason.is_none()
                && agrees(x.relative_delay_ms, x.flipped, tr, x.corner_hz))
                || (may_keep && kept);
            println!(
                "{name} {f1} Hz xo {:.0} -> {:.0} Hz: {:+.3} ms flip {} | truth {:+.3} ms flip {} | {}",
                x.corner_before_hz,
                x.corner_hz,
                x.relative_delay_ms,
                x.flipped,
                tr.0,
                tr.1,
                if !ok {
                    "WRONG"
                } else if kept {
                    "kept"
                } else {
                    "ok"
                }
            );
            if !ok {
                failures.push(format!("{name} {f1} Hz xo {:.0} Hz", x.corner_hz));
            }
        }
        if anchored && f1 >= 200.0 {
            for n in aligned.notes.iter().chain(&xo.notes) {
                if n.contains("arrivals") {
                    failures.push(format!("{name} {f1} Hz held: {n}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

macro_rules! family_tests {
    ($($test:ident: $name:literal, $lp:literal, $hp:literal;)*) => {
        $(
            #[test]
            fn $test() {
                wp_p5hold_family($name, $lp, $hp);
            }
        )*
    };
}

family_tests! {
    wp_p5hold_lr2_gets_the_right_timing: "LR2", 32, 33;
    wp_p5hold_lr4_gets_the_right_timing: "LR4", 34, 35;
    wp_p5hold_lr6_gets_the_right_timing: "LR6", 36, 37;
    wp_p5hold_lr8_gets_the_right_timing: "LR8", 38, 39;
    wp_p5hold_bw2_gets_the_right_timing: "BW2", 42, 43;
    wp_p5hold_bw3_gets_the_right_timing: "BW3", 44, 45;
    wp_p5hold_bw4_gets_the_right_timing: "BW4", 46, 47;
    wp_p5hold_bessel4_gets_the_right_timing: "BES4", 58, 59;
}

// -- WP P5FOLLOW --
macro_rules! unanchored_tests {
    ($($test:ident: $name:literal, $lp:literal, $hp:literal;)*) => {
        $(
            #[test]
            fn $test() {
                wp_p5follow_family_without_the_anchor($name, $lp, $hp);
            }
        )*
    };
}

unanchored_tests! {
    wp_p5follow_lr2_without_the_anchor_is_right_or_kept: "LR2", 32, 33;
    wp_p5follow_lr4_without_the_anchor_is_right_or_kept: "LR4", 34, 35;
    wp_p5follow_lr6_without_the_anchor_is_right_or_kept: "LR6", 36, 37;
    wp_p5follow_lr8_without_the_anchor_is_right_or_kept: "LR8", 38, 39;
    wp_p5follow_bw2_without_the_anchor_is_right_or_kept: "BW2", 42, 43;
    wp_p5follow_bw3_without_the_anchor_is_right_or_kept: "BW3", 44, 45;
    wp_p5follow_bw4_without_the_anchor_is_right_or_kept: "BW4", 46, 47;
    wp_p5follow_bessel4_without_the_anchor_is_right_or_kept: "BES4", 58, 59;
}

// -- WP P5FOLLOW --
/// The review's case by itself: BW3 at 120 Hz without the anchor is kept
/// with the note, alone and through the corner search, not proposed.
#[test]
fn wp_p5follow_bw3_at_120_hz_without_the_anchor_is_kept() {
    let (_, aligned, xo) =
        dspi_measure::xover::sim_without_onset_anchor(|| tuned(44, 45, 120.0)).unwrap();
    let p = &aligned.pairs[0];
    assert_eq!((p.relative_delay_ms, p.flipped), (0.0, false), "{p:?}");
    assert!(
        aligned
            .notes
            .iter()
            .any(|n| n.contains("outputs 1 and 2") && n.contains("could not be settled")), // -- WP UFW -- from 1
        "{:#?}",
        aligned.notes
    );
    let x = &xo.transitions[0];
    assert_eq!((x.relative_delay_ms, x.flipped), (0.0, false), "{x:?}");
    assert!(
        xo.notes
            .iter()
            .any(|n| n.contains("outputs 1 and 2") && n.contains("could not be settled")), // -- WP UFW -- from 1
        "{:#?}",
        xo.notes
    );
}

#[test]
#[ignore = "a probe: prints, asserts nothing"]
fn review_p5d_probe_hold_by_family() {
    let world = room();
    for (name, lp, hp) in FAMILIES {
        for f1 in [300.0f32, 120.0] {
            let chains = family_chains(lp, hp, f1);
            let q = lp == BW3_LP;
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
                let (td, tf, tc, to) = truth(&world, &chains, t.low, t.high, p.corner_hz, q);
                println!(
                    "   align {:.0} Hz: truth {td:+.3} ms flip {tf} (cost {tc:.3}, other {to:.3}); arrivals {start:+.3} ms; chosen {:+.3} ms ({:+.3} T off the truth, hold {:.3} ms), flipped {}, confidence {:.2}",
                    p.corner_hz,
                    p.relative_delay_ms,
                    (p.relative_delay_ms - td) / per,
                    timing_hold_ms(p.corner_hz),
                    p.flipped,
                    p.confidence
                );
            }
            for n in aligned.notes.iter().filter(|n| n.contains("arrivals")) {
                println!("   align note: {n}");
            }
            let corners = proposal_corners(&xo, &chains);
            for (x, t) in xo.transitions.iter().zip(&transitions()) {
                let per = 1000.0 / x.corner_hz;
                let (td, tf, _, _) = truth(&world, &corners, t.low, t.high, x.corner_hz, q);
                println!(
                    "   xo {:.0} -> {:.0} Hz: truth {td:+.3} ms flip {tf}; chosen {:+.3} ms ({:+.3} T off the truth), flipped {}",
                    x.corner_before_hz,
                    x.corner_hz,
                    x.relative_delay_ms,
                    (x.relative_delay_ms - td) / per,
                    x.flipped
                );
            }
            for n in xo.notes.iter().filter(|n| n.contains("arrivals")) {
                println!("   xo note: {n}");
            }
        }
    }
}
