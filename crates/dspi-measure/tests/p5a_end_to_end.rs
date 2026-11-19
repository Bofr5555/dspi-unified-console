//! Wave P5-A review, issue 7: the seven work packages on one merged tree.
//!
//! P5SIM's three-way room → P5TIME's analysis → P5RESP's responses and trust
//! mask → P5CAL's microphone correction and target → P5XO's de-embedding,
//! alignment, crossover corner and summed check → P5FIT's fit → P5MDOC's
//! documents. Every step uses the public API only, as `dspi-core`'s `measure`
//! module will (owner requirement R1: one system, not parts).
//!
//! The world is a three-way with **designed** corners of 300 Hz and 3 kHz
//! (each driver resonates 3/4 octave past the corner on the side it must not
//! play, as P5XO's own synthetic drivers), a tweeter wired inverted, acoustic
//! offsets of 0.25, 0.12 and 0 ms, in P5SIM's room (modes, two reflections),
//! heard by P5SIM's typical microphone on a clock 37.5 ppm off with 50 ms of
//! latency and noise. The device starts with corners 1/3 octave high and 1/4
//! octave low.
#![cfg(feature = "sim")]

mod common;

use common::{Run, analyse, output_response, rms_difference_db, summed_run, walked_run, ways_of};
use dspi_measure::align::{AlignGroup, AlignOptions, Transition, align};
use dspi_measure::cal::{parse_calibration, spl_scale};
use dspi_measure::doc::{
    CaptureInfo, Curve, DocKind, Measurement, MeasurementDoc, Meta, OutputMeasurement, RunKind,
    SCHEMA_VERSION, Stimulus as DocStimulus,
};
use dspi_measure::fit::{FitOptions, fit};
use dspi_measure::ir::{
    ALIGNMENT_WINDOW_CYCLES, AnalysisOptions, FIT_WINDOW_CYCLES, WalkedAnalysis,
};
use dspi_measure::sim::{CalFormat, ProtoModel, SimFilter, World, three_way_chains};
use dspi_measure::stimulus::SweepConfig;
use dspi_measure::target::{Builtin, DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse};
use dspi_measure::types::{ChannelChain, LogGrid, Placement, Response, WayRole};
use dspi_measure::xover::{
    SumMode, XoConstraints, crossover_corner, de_embed, predict_sum, summed_check,
};

const CFG: SweepConfig = SweepConfig {
    f1_hz: 20.0,
    f2_hz: 20_000.0,
    duration_ms: 2_000,
    gap_ms: 1_000,
};
const LEVEL_DB: f32 = -12.0;
const PRE_ROLL_MS: u32 = 1_000;
const MASK: u16 = 0b111;
const F1: f64 = 300.0;
const F2: f64 = 3_000.0;
const ROLES: [WayRole; 3] = [WayRole::Low, WayRole::Mid, WayRole::High];

/// The designed three-way in P5SIM's room. `tweeter_extra_ms` moves the
/// tweeter further back.
fn designed_world(tweeter_extra_ms: f64) -> World {
    let mut w = World::three_way_room(5);
    let res = |f: f64| {
        Some(SimFilter::Peak {
            freq: f,
            q: 4.0,
            gain_db: 8.0,
        })
    };
    for side in 0..2 {
        let (lo, mid, hi) = (side * 3, side * 3 + 1, side * 3 + 2);
        // Woofer: flat to far above its band, a resonance 3/4 octave above F1.
        let d = &mut w.outputs[lo].driver;
        d.rolloff_hz = 0.0;
        d.breakup = res(F1 * 0.75f64.exp2());
        // Mid: a resonance 3/4 octave below F1 (its box) and one 3/4 octave
        // above F2 (its breakup).
        let d = &mut w.outputs[mid].driver;
        d.highpass_hz = F1 * (-0.75f64).exp2();
        d.highpass_q = 2.5;
        d.rolloff_hz = 0.0;
        d.breakup = res(F2 * 0.75f64.exp2());
        // Tweeter: a resonance 3/4 octave below F2, wired inverted.
        let d = &mut w.outputs[hi].driver;
        d.highpass_hz = 0.0;
        d.breakup = res(F2 * (-0.75f64).exp2());
        d.inverted = true;
        d.offset_ms += tweeter_extra_ms;
    }
    w
}

/// The device's chains before the tune: LR4, corners off by +1/3 and -1/4
/// octave.
fn start_chains() -> Vec<ChannelChain> {
    let mut c = three_way_chains();
    let f1c = (F1 * (4.0f64 / 12.0).exp2()) as f32;
    let f2c = (F2 * (-3.0f64 / 12.0).exp2()) as f32;
    for side in 0..2 {
        c[side * 3].crossover[0].freq = f1c;
        c[side * 3 + 1].crossover[0].freq = f1c;
        c[side * 3 + 1].crossover[1].freq = f2c;
        c[side * 3 + 2].crossover[0].freq = f2c;
    }
    c
}

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

fn octaves(a: f64, b: f64) -> f64 {
    (a / b).log2().abs()
}

fn mic_correction(world: &World) -> (dspi_measure::cal::MicCalibration, Response) {
    let text = world
        .mic
        .calibration_text(CalFormat::Umik, 48_000.0)
        .unwrap();
    let cal = parse_calibration(&text, "sim_90deg.txt").unwrap();
    let resp = cal.to_response(&LogGrid::STANDARD).unwrap();
    (cal, resp)
}

fn measure(world: &World, chains: &[ChannelChain]) -> (Run, WalkedAnalysis) {
    let run = walked_run(world, chains, MASK, 3, &CFG, LEVEL_DB, PRE_ROLL_MS);
    let a = analyse(&run, world, &AnalysisOptions::default()).unwrap();
    assert!(a.quality.reliable, "{:?}", a.quality);
    (run, a)
}

/// The measured sum of `outputs` (with their polarities) through `chains`.
fn measured_sum(
    world: &World,
    chains: &[ChannelChain],
    outputs: &[(usize, bool)],
    cycles: f64,
    mic: &Response,
) -> Response {
    let run = summed_run(world, chains, outputs, &CFG, LEVEL_DB, PRE_ROLL_MS);
    let a = analyse(&run, world, &AnalysisOptions::default()).unwrap();
    // One pass: magnitudes only, by definition (section 1.1 step 7).
    assert!(!a.quality.reliable);
    output_response(&a.outputs[0], a.band_hz, cycles, Some(mic)).0
}

#[test]
fn wp_p5a_three_way_room_end_to_end() {
    let world = designed_world(0.0);
    let chains = start_chains();
    let (cal, mic) = mic_correction(&world);
    assert_eq!(cal.curve.serial.as_deref(), Some("SIM-7000417"));
    assert!(spl_scale(cal.curve.sensitivity_db, None).approximate);

    // -- Measure (P5SIM, P5TIME, P5RESP, P5CAL) ----------------------------
    let (_run, analysis) = measure(&world, &chains);
    assert!(
        (analysis.clock.ratio_ppm - 37.5).abs() < 0.3,
        "{:?}",
        analysis.clock
    );
    let ways = ways_of(
        &analysis,
        &chains,
        &ROLES,
        ALIGNMENT_WINDOW_CYCLES,
        Some(&mic),
    );
    // The microphone's own response is gone: the correction changed the
    // response by exactly the calibration.
    let (raw, _) = output_response(
        &analysis.outputs[2],
        analysis.band_hz,
        ALIGNMENT_WINDOW_CYCLES,
        None,
    );
    let i10k = LogGrid::STANDARD
        .freqs()
        .unwrap()
        .iter()
        .position(|f| *f >= 11_000.0)
        .unwrap();
    assert!(
        (raw.mag_db[i10k]
            - ways[2].response.mag_db[i10k]
            - world.mic.response_db(11_000.0, 48_000.0))
        .abs()
            < 0.05
    );

    // -- De-embed and the mandatory summed check of the current chain ------
    let drivers: Vec<_> = ways
        .iter()
        .map(|w| de_embed(w, &ProtoModel).unwrap())
        .collect();
    let now: Vec<ChannelChain> = chains[..3].to_vec();
    for t in transitions() {
        let pair = [drivers[t.low].clone(), drivers[t.high].clone()];
        let pair_chains = [now[t.low].clone(), now[t.high].clone()];
        let predicted = predict_sum(
            &pair,
            &pair_chains,
            SumMode::Generator { invert_mask: 0 },
            &ProtoModel,
        )
        .unwrap();
        let measured = measured_sum(
            &world,
            &chains,
            &[(t.low, false), (t.high, false)],
            ALIGNMENT_WINDOW_CYCLES,
            &mic,
        );
        let corner = f64::from(now[t.high].crossover[0].freq);
        let check = summed_check(&measured, &predicted, corner, 0.0).unwrap();
        println!("summed check, current chain, {corner:.0} Hz: {check:?}");
        assert!(check.passed, "{check:?}");
    }

    // -- Alignment alone (T2) ----------------------------------------------
    let aligned = align(
        &ways,
        &AlignGroup {
            transitions: transitions(),
        },
        &AlignOptions::default(),
    )
    .unwrap();
    println!("align: {:?}", aligned.ways);
    assert_eq!(
        aligned.ways.iter().map(|w| w.inverted).collect::<Vec<_>>(),
        vec![false, false, true],
        "the inverted tweeter is flipped: {aligned:?}"
    );

    // -- Crossover corner (T10 step 5) --------------------------------------
    let target = Target {
        target_use: TargetUse::Speakers,
        coupler: None,
        definition: TargetDefinition::Builtin {
            builtin: Builtin::Flat,
        },
        reference: DEFAULT_REFERENCE_BAND,
    }
    .evaluate(&LogGrid::STANDARD)
    .unwrap();
    let proposal = crossover_corner(
        &drivers,
        &transitions(),
        &target,
        &XoConstraints::default(),
        &ProtoModel,
    )
    .unwrap();
    for t in &proposal.transitions {
        println!("xo: {t:?}");
    }
    for o in &proposal.outputs {
        println!(
            "xo: output {} gain {} delay {} inverted {} xo {:?}",
            o.output,
            o.chain.gain_db,
            o.chain.delay_ms,
            o.chain.feed.inverted,
            o.chain.crossover.iter().map(|b| b.freq).collect::<Vec<_>>()
        );
    }
    let (t0, t1) = (&proposal.transitions[0], &proposal.transitions[1]);
    // -- WP P5TRUST -- through the woofer-mid transition this room's sum
    // against the flat target falls all the way to the half-octave window's
    // lower edge (267 Hz); with the trust rules fixed every candidate there
    // is trusted, so the search no longer stops at the trusted range's edge
    // near 300 Hz (what this test read as the design before). The edge is
    // no minimum: the corner is kept, the edge said, the timing proposed.
    assert!(t0.best_at_edge, "{t0:?}");
    assert_eq!(t0.corner_hz, t0.corner_before_hz);
    assert!(t0.kept_reason.is_none(), "{t0:?}");
    // A window wide enough holds the minimum: inside it, below the default
    // window's edge.
    let wide = crossover_corner(
        &drivers,
        &transitions(),
        &target,
        &XoConstraints {
            window_octaves: 1.0,
            ..XoConstraints::default()
        },
        &ProtoModel,
    )
    .unwrap();
    let w0 = &wide.transitions[0];
    println!("xo, one octave: {w0:?}");
    assert!(!w0.best_at_edge, "{w0:?}");
    assert!(w0.corner_hz < 267.3 && w0.corner_hz > 189.0, "{w0:?}");
    assert!(w0.cost_db.unwrap() < w0.cost_before_db.unwrap());
    // -- WP P5TRUST2 -- (review P5TRUST N2) the right cycle: before, 238 Hz
    // won here with a relative delay of -1.74 ms and a flip (delay and
    // polarity standing in for each other at one frequency, 0.47 of a period
    // off the measured arrivals) against -0.06 ms and no flip in the default
    // window. Each candidate's timing is now held within a quarter period of
    // the arrival-based delay, and the note says where it was.
    assert!(!w0.flipped, "{w0:?}");
    let quarter_ms = 0.25 * 1000.0 / w0.corner_hz;
    assert!(
        (w0.relative_delay_ms - t0.relative_delay_ms).abs() < quarter_ms,
        "{w0:?} against {t0:?}"
    );
    assert!(
        wide.notes.iter().any(|n| n.contains("measured arrivals")),
        "{:?}",
        wide.notes
    );
    // -- WP P5TRUST2 -- (review P5TRUST N1) through the mid-tweeter
    // transition every candidate from 2.4 to 3.6 kHz sums alike: 2.67 kHz
    // was 6 % better than the current 2.52 kHz but by 0.023 dB RMS, the
    // room's ripple. Under CORNER_MIN_GAIN_DB the corner is kept (it is a
    // quarter octave from the design), the timing proposed.
    assert_eq!(t1.corner_hz, t1.corner_before_hz, "{t1:?}");
    assert!(octaves(t1.corner_hz, F2) <= 0.25 + 1e-3, "{t1:?}");
    let best_other = t1
        .candidate_costs_db
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    assert!(
        t1.cost_db.unwrap() - best_other < dspi_measure::xover::CORNER_MIN_GAIN_DB,
        "{t1:?}"
    );
    assert!(
        proposal
            .notes
            .iter()
            .any(|n| n.contains("outputs 1 and 2: no corner is clearly better")),
        "{:?}",
        proposal.notes
    );
    assert!(t1.cost_db.unwrap() < t1.cost_before_db.unwrap());
    assert_eq!(
        proposal
            .outputs
            .iter()
            .map(|o| o.chain.feed.inverted)
            .collect::<Vec<_>>(),
        vec![false, false, true]
    );
    // Nothing raised, the smallest delay kept (0), the tweeter (the nearest
    // driver) delayed most.
    assert!(proposal.outputs.iter().all(|o| o.level_change_db <= 0.0));
    let delays: Vec<f32> = proposal.outputs.iter().map(|o| o.chain.delay_ms).collect();
    assert_eq!(delays.iter().copied().fold(f32::MAX, f32::min), 0.0);
    assert!(delays[2] > delays[1] && delays[2] > delays[0], "{delays:?}");

    // -- The proposal in the world: predicted against measured -------------
    let mut tuned = chains.clone();
    for o in &proposal.outputs {
        tuned[usize::from(o.output)] = o.chain.clone();
    }
    let new: Vec<ChannelChain> = tuned[..3].to_vec();
    let invert_mask = proposal
        .outputs
        .iter()
        .filter(|o| o.chain.feed.inverted)
        .fold(0u32, |m, o| m | (1 << o.output));
    for (t, xo) in transitions().iter().zip(&proposal.transitions) {
        let pair = [drivers[t.low].clone(), drivers[t.high].clone()];
        let pair_chains = [new[t.low].clone(), new[t.high].clone()];
        let predicted = predict_sum(
            &pair,
            &pair_chains,
            SumMode::Generator { invert_mask },
            &ProtoModel,
        )
        .unwrap();
        let measured = measured_sum(
            &world,
            &tuned,
            &[
                (t.low, new[t.low].feed.inverted),
                (t.high, new[t.high].feed.inverted),
            ],
            ALIGNMENT_WINDOW_CYCLES,
            &mic,
        );
        let check = summed_check(&measured, &predicted, xo.corner_hz, 0.0).unwrap();
        println!("summed check, proposal, {:.0} Hz: {check:?}", xo.corner_hz);
        assert!(check.passed, "{check:?}");
    }
    // The whole speaker, before and after, against the flat target through
    // both transitions: the tune made the real sum flatter.
    let all = |c: &[ChannelChain]| -> Vec<(usize, bool)> {
        (0..3).map(|i| (i, c[i].feed.inverted)).collect()
    };
    let sum_before = measured_sum(&world, &chains, &all(&chains), FIT_WINDOW_CYCLES, &mic);
    let sum_after = measured_sum(&world, &tuned, &all(&tuned), FIT_WINDOW_CYCLES, &mic);
    let ripple = |r: &Response, lo: f64, hi: f64| -> f64 {
        let f = r.grid.freqs().unwrap();
        let v: Vec<f64> = f
            .iter()
            .zip(&r.mag_db)
            .filter(|(f, _)| **f >= lo && **f <= hi)
            .map(|(_, m)| *m)
            .collect();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        (v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / v.len() as f64).sqrt()
    };
    let (rb, ra) = (
        ripple(&sum_before, 200.0, 6_000.0),
        ripple(&sum_after, 200.0, 6_000.0),
    );
    println!("the speaker's sum 200 Hz to 6 kHz: {rb:.2} dB RMS before, {ra:.2} dB after");
    assert!(ra < rb, "{ra} < {rb}");

    // -- The EQ fit of the bass (P5FIT) -------------------------------------
    let options = FitOptions {
        max_hz: 150.0,
        ..FitOptions::speakers()
    };
    let fitted = fit(&sum_after, &target, &tuned[0], &ProtoModel, &options).unwrap();
    for n in &fitted.notes {
        println!("fit: {n}");
    }
    assert!(
        !fitted.bands.is_empty() && fitted.bands.len() <= 10,
        "{:?}",
        fitted.bands
    );
    assert!(
        fitted.rms_after_db < 0.6 * fitted.rms_before_db,
        "{:?}",
        fitted.notes
    );
    for b in &fitted.bands {
        assert!(
            tuned[0].peq[usize::from(b.index)].filter_type == 0,
            "a free band"
        );
        assert!(b.band.gain <= 3.0 && b.band.gain >= -15.0);
        assert!((20.0..=150.0).contains(&b.band.freq));
    }
    assert!(fitted.gain_cut_db <= 0.0);
    // Applied in the world, the fit's prediction holds.
    let mut eq = tuned.clone();
    for b in &fitted.bands {
        eq[0].peq[usize::from(b.index)] = b.band;
    }
    eq[0].gain_db += fitted.gain_cut_db;
    let sum_eq = measured_sum(&world, &eq, &all(&eq), FIT_WINDOW_CYCLES, &mic);
    let (d, n) = rms_difference_db(&sum_eq, &fitted.predicted, 30.0, 120.0);
    println!("fit predicted against measured, 30 to 120 Hz: {d:.2} dB RMS over {n} points");
    assert!(n > 100 && d < 1.0, "{d} dB over {n}");

    // -- The documents (P5MDOC) ---------------------------------------------
    let doc = MeasurementDoc {
        schema_version: SCHEMA_VERSION,
        meta: Meta::new(
            DocKind::Measurement,
            "Left speaker",
            "2026-11-16T12:00:00Z",
            "0.13.0",
        ),
        measurement: Measurement {
            id: String::new(),
            serial: "SIM".into(),
            platform: "RP2350".into(),
            saved_ms: 1_794_830_400_000,
            placement: Placement::ListeningPosition,
            mic: None,
            stimulus: DocStimulus {
                kind: RunKind::Walked,
                f1_hz: f64::from(CFG.f1_hz),
                f2_hz: f64::from(CFG.f2_hz),
                duration_ms: CFG.duration_ms,
                repeat: 3,
                gap_ms: u32::from(CFG.gap_ms),
                level_db: f64::from(LEVEL_DB),
                device_rate_hz: 48_000,
                extra: Default::default(),
            },
            capture: CaptureInfo {
                rate_hz: analysis.capture_rate_hz,
                clock_ratio: 1.0 + analysis.clock.ratio_ppm * 1e-6,
                timing_reliable: analysis.quality.reliable,
                max_residual_us: Some(analysis.quality.max_residual_us),
                extra: Default::default(),
            },
            outputs: ways
                .iter()
                .zip(&analysis.outputs)
                .map(|(w, o)| {
                    let mut curve = Curve::from_response(&w.response);
                    curve.spread_db = Some(o.spread_db.iter().map(|s| s.min(300.0)).collect());
                    OutputMeasurement {
                        output: w.output,
                        role: w.role,
                        speaker: "L".into(),
                        chain: w.chain.clone().into(),
                        arrival_ms: Some(w.arrival_ms),
                        response: curve,
                        extra: Default::default(),
                    }
                })
                .collect(),
            positions: 1,
            note: String::new(),
            tunes: Vec::new(),
            extra: Default::default(),
        },
        extra: Default::default(),
    };
    let text = doc.to_text().unwrap();
    println!("measurement document: {} bytes", text.len());
    // Three outputs with phase, mask and spread: half of the 100 KiB a
    // three-way stereo measurement may take, far under the 1 MiB bound.
    assert!(text.len() < 50 * 1024, "{}", text.len());
    let back = MeasurementDoc::from_text(&text).unwrap();
    let rewritten = back.to_text().unwrap();
    for (a, b) in rewritten.lines().zip(text.lines()) {
        assert_eq!(a, b, "a second write is identical");
    }
    assert_eq!(rewritten.len(), text.len());
    for (a, b) in back
        .measurement
        .outputs
        .iter()
        .zip(&doc.measurement.outputs)
    {
        assert_eq!(a.response, b.response.rounded());
        assert_eq!(a.chain, b.chain);
    }
    // What was read back still tunes the same: the stored curves carry
    // everything the proposal needs.
    let stored: Vec<_> = back
        .measurement
        .outputs
        .iter()
        .zip(&ways)
        .map(|(o, w)| dspi_measure::align::Way {
            response: o.response.to_response(),
            chain: o.chain.chain.clone(),
            ..w.clone()
        })
        .collect();
    let drivers2: Vec<_> = stored
        .iter()
        .map(|w| de_embed(w, &ProtoModel).unwrap())
        .collect();
    let again = crossover_corner(
        &drivers2,
        &transitions(),
        &target,
        &XoConstraints::default(),
        &ProtoModel,
    )
    .unwrap();
    for (a, b) in again.transitions.iter().zip(&proposal.transitions) {
        assert_eq!(a.corner_hz, b.corner_hz);
        assert_eq!(a.flipped, b.flipped);
        assert!(
            (a.relative_delay_ms - b.relative_delay_ms).abs() < 0.005,
            "{a:?} {b:?}"
        );
    }
}

#[test]
fn wp_p5a_a_0_35_ms_offset_is_recovered_within_10_us_end_to_end() {
    // Two worlds that differ only by the tweeter standing 0.35 ms further
    // back: the proposals' delays must differ by exactly that (P5XO's stated
    // ±10 µs), through the room, the noise and the drifting clock.
    let chains = start_chains();
    let constraints = XoConstraints {
        keep_corners: true,
        ..XoConstraints::default()
    };
    let mut relative = Vec::new();
    let mut aligned_rel = Vec::new();
    for extra in [0.0, 0.35] {
        let world = designed_world(extra);
        let (_, mic) = mic_correction(&world);
        let (_, analysis) = measure(&world, &chains);
        let ways = ways_of(
            &analysis,
            &chains,
            &ROLES,
            ALIGNMENT_WINDOW_CYCLES,
            Some(&mic),
        );
        let drivers: Vec<_> = ways
            .iter()
            .map(|w| de_embed(w, &ProtoModel).unwrap())
            .collect();
        let flat = Builtin::Flat.evaluate(&LogGrid::STANDARD).unwrap();
        let p =
            crossover_corner(&drivers, &transitions(), &flat, &constraints, &ProtoModel).unwrap();
        // Align only: every corner and gain kept.
        for (o, c) in p.outputs.iter().zip(&chains) {
            assert_eq!(o.chain.crossover, c.crossover);
            assert_eq!(o.chain.gain_db, c.gain_db);
        }
        assert!(p.outputs[2].chain.feed.inverted);
        relative.push(p.transitions[1].relative_delay_ms);
        let a = align(
            &ways,
            &AlignGroup {
                transitions: transitions(),
            },
            &AlignOptions::default(),
        )
        .unwrap();
        aligned_rel.push(a.pairs[1].relative_delay_ms);
    }
    // The tweeter is later by 0.35 ms, so it needs 0.35 ms less delay.
    let d = relative[0] - relative[1];
    let da = aligned_rel[0] - aligned_rel[1];
    println!("crossover_corner: {relative:?} -> {d:.4} ms; align: {aligned_rel:?} -> {da:.4} ms");
    assert!((d - 0.35).abs() <= 0.010, "{d}");
    assert!((da - 0.35).abs() <= 0.010, "{da}");
}
