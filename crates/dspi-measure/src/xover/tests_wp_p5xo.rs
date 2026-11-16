//! Tests of WP P5XO's de-embedding, crossover search and summed check, on the
//! synthetic world of `align`'s tests.

use super::*;
use crate::align::tests_wp_p5xo::{
    Drv, LR4_HP, LR4_LP, ProtoModel, arb_ways, band, chain, chain_at, peak, to_response, way,
};
use crate::types::LogGrid;
use proptest::prelude::*;

fn flat_target() -> Response {
    let n = LogGrid::STANDARD.freqs().unwrap().len();
    Response {
        grid: LogGrid::STANDARD,
        mag_db: vec![0.0; n],
        phase_deg: None,
        valid: None,
        spl: false,
    }
}

const F1: f64 = 300.0;
const F2: f64 = 3000.0;

/// A three-way whose designed corners are 300 Hz and 3 kHz: each driver has
/// a +8 dB resonance 3/4 octave past the designed corner on the side it must
/// not play, mirrored on its partner, so the flattest sum is at the designed
/// corner. The current corners are off by +1/3 and -1/4 octave; the tweeter
/// is wired inverted; the ways arrive 0.3 and 0.4 ms apart.
fn three_way_drivers(output0: u8) -> Vec<Driver> {
    let f1c = F1 * (4.0f64 / 12.0).exp2();
    let f2c = F2 * (-3.0f64 / 12.0).exp2();
    let mut w = Drv::flat(50.6);
    w.shape = vec![peak(F1 * 0.75f64.exp2(), 4.0, 8.0)];
    let mut m = Drv::flat(50.3);
    m.shape = vec![
        peak(F1 * (-0.75f64).exp2(), 4.0, 8.0),
        peak(F2 * 0.75f64.exp2(), 4.0, 8.0),
    ];
    let mut t = Drv::flat(50.2);
    t.shape = vec![peak(F2 * (-0.75f64).exp2(), 4.0, 8.0)];
    t.inverted = true;
    let ways = [
        way(
            output0,
            WayRole::Low,
            chain(vec![band(LR4_LP, f1c)]),
            &w,
            60.0,
        ),
        way(
            output0 + 1,
            WayRole::Mid,
            chain(vec![band(LR4_HP, f1c), band(LR4_LP, f2c)]),
            &m,
            60.0,
        ),
        way(
            output0 + 2,
            WayRole::High,
            chain(vec![band(LR4_HP, f2c)]),
            &t,
            60.0,
        ),
    ];
    ways.iter()
        .map(|w| de_embed(w, &ProtoModel).unwrap())
        .collect()
}

fn three_way_transitions(base: usize) -> Vec<Transition> {
    vec![
        Transition {
            low: base,
            high: base + 1,
            corner_hz: None,
        },
        Transition {
            low: base + 1,
            high: base + 2,
            corner_hz: None,
        },
    ]
}

fn octaves(a: f64, b: f64) -> f64 {
    (a / b).log2().abs()
}

#[test]
fn wp_p5xo_de_embedding_recovers_the_driver_and_masks_by_snr_minus_attenuation() {
    let mut d = Drv::flat(50.0);
    d.shape = vec![peak(800.0, 2.0, 6.0)];
    let w = way(3, WayRole::Low, chain(vec![band(LR4_LP, 1000.0)]), &d, 40.0);
    let drv = de_embed(&w, &ProtoModel).unwrap();
    assert_eq!(drv.output, 3);
    assert!((drv.arrival_ms - 50.05).abs() < 1e-9);
    let f = LogGrid::STANDARD.freqs().unwrap();
    let valid = drv.response.valid.as_ref().unwrap();
    let ph = drv.response.phase_deg.as_ref().unwrap();
    let xo = sections(&w.chain.crossover, &ProtoModel).unwrap();
    for (i, fr) in f.iter().enumerate() {
        let att = -cascade_at(&xo, *fr, 48_000.0).unwrap().db();
        let att = att.max(0.0);
        assert_eq!(
            valid[i],
            40.0 - att >= DEEMBED_MIN_SNR_DB,
            "{fr} Hz att {att}"
        );
        if valid[i] {
            let truth = d.at(*fr);
            let got = Cx::polar(db_to_lin(drv.response.mag_db[i]), ph[i].to_radians());
            let err = got.add(truth.scale(-1.0)).norm_sqr().sqrt() / truth.norm_sqr().sqrt();
            assert!(err < 1e-6, "{fr} Hz: {err}");
        }
    }
    // Trusted to above the corner, untrusted an octave and more above it.
    let at = |hz: f64| valid[f.iter().position(|x| *x >= hz).unwrap()];
    assert!(at(500.0) && at(1000.0) && !at(2000.0) && !at(8000.0));
}

#[test]
fn wp_p5xo_three_way_corners_come_within_a_sixth_octave_of_the_designed_optimum() {
    let drivers = three_way_drivers(0);
    let p = crossover_corner(
        &drivers,
        &three_way_transitions(0),
        &flat_target(),
        &XoConstraints::default(),
        &ProtoModel,
    )
    .unwrap();
    let t0 = &p.transitions[0];
    let t1 = &p.transitions[1];
    assert!(octaves(t0.corner_hz, F1) <= 1.0 / 6.0 + 1e-9, "{t0:?}");
    assert!(octaves(t1.corner_hz, F2) <= 1.0 / 6.0 + 1e-9, "{t1:?}");
    // The proposal is better than what is there.
    assert!(t0.cost_db.unwrap() < t0.cost_before_db.unwrap(), "{t0:?}");
    assert!(t1.cost_db.unwrap() < t1.cost_before_db.unwrap(), "{t1:?}");
    // Family and order kept.
    for (o, d) in p.outputs.iter().zip(&drivers) {
        for (a, b) in o.chain.crossover.iter().zip(&d.chain.crossover) {
            assert_eq!(a.filter_type, b.filter_type);
        }
    }
    // The inverted tweeter is flipped, the others are not.
    assert_eq!(
        p.outputs
            .iter()
            .map(|o| o.chain.feed.inverted)
            .collect::<Vec<_>>(),
        vec![false, false, true],
        "{p:?}"
    );
    // The smallest delay keeps its value (0); the tweeter, the earliest
    // driver, gets the most.
    let d: Vec<f32> = p.outputs.iter().map(|o| o.chain.delay_ms).collect();
    assert_eq!(d.iter().copied().fold(f32::MAX, f32::min), 0.0, "{d:?}");
    assert!(d[2] > d[0] && d[2] > d[1] && d[2] < 1.0, "{d:?}");
    // Drivers of equal sensitivity: no level change past the resonances'
    // skirts.
    assert!(p.outputs.iter().all(|o| o.level_change_db >= -0.5), "{p:?}");
    for t in &p.transitions {
        assert!(t.evaluations <= MAX_EVALUATIONS_PER_TRANSITION);
    }
}

#[test]
fn wp_p5xo_align_only_keeps_every_corner() {
    let drivers = three_way_drivers(0);
    let c = XoConstraints {
        keep_corners: true,
        ..XoConstraints::default()
    };
    let p = crossover_corner(
        &drivers,
        &three_way_transitions(0),
        &flat_target(),
        &c,
        &ProtoModel,
    )
    .unwrap();
    for (o, d) in p.outputs.iter().zip(&drivers) {
        assert_eq!(o.chain.crossover, d.chain.crossover);
        assert_eq!(o.chain.gain_db, d.chain.gain_db);
    }
    for t in &p.transitions {
        assert_eq!(t.candidates_hz.len(), 1);
    }
    assert!(p.outputs[2].chain.feed.inverted, "still aligned: {p:?}");
}

#[test]
fn wp_p5xo_a_candidate_outside_either_drivers_trusted_range_is_never_evaluated() {
    // The tweeter is trusted only from 2.2 kHz: a candidate at c needs it
    // trusted down to c/2, so only corners of 4.4 kHz and above qualify; the
    // window around 3 kHz reaches 3 * 2^(1/2) = 4.24 kHz: none does.
    let mut d = Drv::flat(50.0);
    d.shape = Vec::new();
    let mut tw = way(
        1,
        WayRole::High,
        chain(vec![band(LR4_HP, 3000.0)]),
        &d,
        60.0,
    );
    let f = LogGrid::STANDARD.freqs().unwrap();
    for (i, fr) in f.iter().enumerate() {
        if *fr < 2200.0 {
            tw.snr_db[i] = 0.0;
        }
    }
    let wf = way(0, WayRole::Low, chain(vec![band(LR4_LP, 3000.0)]), &d, 80.0);
    let drivers = vec![
        de_embed(&wf, &ProtoModel).unwrap(),
        de_embed(&tw, &ProtoModel).unwrap(),
    ];
    let ts = [Transition {
        low: 0,
        high: 1,
        corner_hz: None,
    }];
    let p = crossover_corner(
        &drivers,
        &ts,
        &flat_target(),
        &XoConstraints::default(),
        &ProtoModel,
    )
    .unwrap();
    assert!(p.transitions[0].candidates_hz.is_empty(), "{p:?}");
    assert_eq!(p.transitions[0].evaluations, 0);
    assert!(p.transitions[0].kept_reason.is_some());
    assert_eq!(p.outputs[1].chain, drivers[1].chain);
    // A wider window reaches qualifying corners: every one evaluated keeps
    // both drivers trusted one octave each side.
    let c = XoConstraints {
        window_octaves: 1.5,
        ..XoConstraints::default()
    };
    let p = crossover_corner(&drivers, &ts, &flat_target(), &c, &ProtoModel).unwrap();
    let t = &p.transitions[0];
    assert!(!t.candidates_hz.is_empty(), "{p:?}");
    let tv = drivers[1].response.valid.as_ref().unwrap();
    for c in &t.candidates_hz {
        for (i, fr) in f.iter().enumerate() {
            if *fr >= c / 2.0 && *fr <= c * 2.0 {
                assert!(tv[i], "candidate {c} Hz uses an untrusted {fr} Hz");
            }
        }
    }
    assert!(t.evaluations <= MAX_EVALUATIONS_PER_TRANSITION);
}

#[test]
fn wp_p5xo_the_protective_high_pass_is_never_lowered_below_its_rule() {
    // The designed optimum is 3 kHz; the limits profile asks the tweeter for
    // a high-pass at 3.3 kHz or above, so no lower corner is offered.
    let drivers = three_way_drivers(0);
    let c = XoConstraints {
        window_octaves: 1.0,
        highpass: vec![HighpassOverride {
            output: 2,
            rule: HighpassRule {
                min_hz: 3300.0,
                min_slope_db: 24,
            },
        }],
        ..XoConstraints::default()
    };
    let p = crossover_corner(
        &drivers,
        &three_way_transitions(0),
        &flat_target(),
        &c,
        &ProtoModel,
    )
    .unwrap();
    let hp = p.outputs[2].chain.crossover[0].freq;
    assert!(hp >= 3300.0, "{p:?}");
    assert!(p.transitions[1].candidates_hz.iter().all(|c| *c >= 3299.9));
    // The default tweeter rule (1 kHz) holds for every candidate as well.
    let c = XoConstraints {
        window_octaves: 2.0,
        ..XoConstraints::default()
    };
    let p = crossover_corner(
        &drivers,
        &three_way_transitions(0),
        &flat_target(),
        &c,
        &ProtoModel,
    )
    .unwrap();
    assert!(p.transitions[1].candidates_hz.iter().all(|c| *c >= 999.9));
    assert!(p.outputs[2].chain.crossover[0].freq >= 1000.0);
    // A rule no slope can meet: nothing is evaluated, nothing is changed.
    let c = XoConstraints {
        highpass: vec![HighpassOverride {
            output: 2,
            rule: HighpassRule {
                min_hz: 1000.0,
                min_slope_db: 48,
            },
        }],
        ..XoConstraints::default()
    };
    let p = crossover_corner(
        &drivers,
        &three_way_transitions(0),
        &flat_target(),
        &c,
        &ProtoModel,
    )
    .unwrap();
    assert!(p.transitions[1].candidates_hz.is_empty());
    assert_eq!(p.outputs[2].chain.crossover, drivers[2].chain.crossover);
}

#[test]
fn wp_p5xo_levels_are_cut_only_and_the_quietest_way_keeps_its_gain() {
    // The tweeter is 6 dB hotter than the woofer; both outputs carry gains
    // that are not on the 0.1 dB grid.
    let mut t = Drv::flat(50.0);
    t.level_db = 6.0;
    let mut cw = chain(vec![band(LR4_LP, 2000.0)]);
    cw.gain_db = -1.03;
    let mut ct = chain(vec![band(LR4_HP, 2000.0)]);
    ct.gain_db = -2.07;
    let drivers = vec![
        de_embed(
            &way(0, WayRole::Low, cw, &Drv::flat(50.0), 60.0),
            &ProtoModel,
        )
        .unwrap(),
        de_embed(&way(1, WayRole::High, ct, &t, 60.0), &ProtoModel).unwrap(),
    ];
    let ts = [Transition {
        low: 0,
        high: 1,
        corner_hz: None,
    }];
    let p = crossover_corner(
        &drivers,
        &ts,
        &flat_target(),
        &XoConstraints::default(),
        &ProtoModel,
    )
    .unwrap();
    assert_eq!(
        p.outputs[0].chain.gain_db, -1.03,
        "the quietest is untouched"
    );
    assert_eq!(p.outputs[0].level_change_db, 0.0);
    // Levels include today's gains: 6 - 2.07 against -1.03, a cut of 4.96.
    assert!((p.outputs[1].level_change_db + 4.96).abs() < 0.06, "{p:?}");
    for o in &p.outputs {
        assert!(o.chain.gain_db <= o.chain_before.gain_db);
    }
    // A cut past the cap is held to it.
    let c = XoConstraints {
        max_level_cut_db: 3.0,
        ..XoConstraints::default()
    };
    let p = crossover_corner(&drivers, &ts, &flat_target(), &c, &ProtoModel).unwrap();
    assert!(p.outputs[1].level_change_db >= -3.0 - 1e-9, "{p:?}");
    assert!(!p.notes.is_empty());
    // keep_levels keeps every gain.
    let c = XoConstraints {
        keep_levels: true,
        ..XoConstraints::default()
    };
    let p = crossover_corner(&drivers, &ts, &flat_target(), &c, &ProtoModel).unwrap();
    assert_eq!(p.outputs[1].chain.gain_db, -2.07);
}

#[test]
fn wp_p5xo_identical_input_gives_identical_quantised_proposals() {
    let drivers = three_way_drivers(0);
    let run = || {
        crossover_corner(
            &drivers,
            &three_way_transitions(0),
            &flat_target(),
            &XoConstraints::default(),
            &ProtoModel,
        )
        .unwrap()
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b);
    // Quantised: delays on the 0.0001 ms grid, corners on 0.1 Hz.
    for o in &a.outputs {
        let d = f64::from(o.chain.delay_ms) * 10_000.0;
        assert!((d - d.round()).abs() < 1e-3, "{}", o.chain.delay_ms);
        for b in &o.chain.crossover {
            let f = f64::from(b.freq) * 10.0;
            assert!((f - f.round()).abs() < 1e-2, "{}", b.freq);
        }
    }
}

#[test]
fn wp_p5xo_a_stereo_three_way_is_searched_within_the_time_and_evaluation_bounds() {
    let mut drivers = three_way_drivers(0);
    drivers.extend(three_way_drivers(3));
    let mut ts = three_way_transitions(0);
    ts.extend(three_way_transitions(3));
    let c = XoConstraints {
        window_octaves: MAX_WINDOW_OCTAVES,
        ..XoConstraints::default()
    };
    let t0 = std::time::Instant::now();
    let p = crossover_corner(&drivers, &ts, &flat_target(), &c, &ProtoModel).unwrap();
    let took = t0.elapsed();
    for t in &p.transitions {
        assert!(
            t.evaluations <= MAX_EVALUATIONS_PER_TRANSITION,
            "{}",
            t.evaluations
        );
        assert!(
            t.evaluations > 1000,
            "the budget is used: {}",
            t.evaluations
        );
    }
    // Under 2 s native release for a three-way pair (here: both sides);
    // unoptimised test builds get more room.
    let limit = if cfg!(debug_assertions) { 60.0 } else { 2.0 };
    assert!(took.as_secs_f64() < limit, "{took:?}");
}

fn walked_and_summed(real_tweeter: &Drv) -> (Vec<Driver>, Vec<ChannelChain>, Response) {
    // The walked run sees a flat, non-inverted tweeter...
    let cw = chain(vec![band(LR4_LP, 2000.0)]);
    let mut ct = chain(vec![band(LR4_HP, 2000.0)]);
    ct.delay_ms = 0.35;
    let wf = Drv::flat(50.85);
    let drivers = vec![
        de_embed(&way(0, WayRole::Low, cw.clone(), &wf, 60.0), &ProtoModel).unwrap(),
        de_embed(
            &way(1, WayRole::High, ct.clone(), &Drv::flat(50.5), 60.0),
            &ProtoModel,
        )
        .unwrap(),
    ];
    // ... and the summed run plays both through the same chains, the tweeter
    // being what it really is.
    let f = LogGrid::STANDARD.freqs().unwrap();
    let h: Vec<Cx> = f
        .iter()
        .map(|f| {
            wf.at(*f)
                .mul(chain_at(&cw, *f))
                .add(real_tweeter.at(*f).mul(chain_at(&ct, *f)))
        })
        .collect();
    (drivers, vec![cw, ct], to_response(LogGrid::STANDARD, &h))
}

#[test]
fn wp_p5xo_the_summed_check_confirms_a_true_prediction_and_fails_a_wrong_one() {
    let (drivers, chains, measured) = walked_and_summed(&Drv::flat(50.5));
    let pred = predict_sum(
        &drivers,
        &chains,
        SumMode::Generator { invert_mask: 0 },
        &ProtoModel,
    )
    .unwrap();
    let ok = summed_check(&measured, &pred, 2000.0, 0.0).unwrap();
    assert!(ok.passed && ok.rms_db < 0.01, "{ok:?}");
    assert!(ok.points >= 190);
    // A level offset between the runs is the caller's to give.
    let off = summed_check(&measured, &pred, 2000.0, 2.0).unwrap();
    assert!(!off.passed);
    // The walked run's timing was wrong by 0.2 ms: the check fails.
    let (_, _, late) = walked_and_summed(&Drv::flat(50.7));
    let bad = summed_check(&late, &pred, 2000.0, 0.0).unwrap();
    assert!(
        !bad.passed && bad.rms_db > SUMMED_CHECK_MAX_RMS_DB,
        "{bad:?}"
    );
    // The generator's invert mask is what the summed run reproduces: with the
    // tweeter's bit set the prediction has the notch.
    let inv = predict_sum(
        &drivers,
        &chains,
        SumMode::Generator { invert_mask: 0b10 },
        &ProtoModel,
    )
    .unwrap();
    let mut tw = Drv::flat(50.5);
    tw.inverted = true;
    let (_, _, measured_inv) = walked_and_summed(&tw);
    assert!(
        summed_check(&measured_inv, &inv, 2000.0, 0.0)
            .unwrap()
            .passed
    );
    assert!(!summed_check(&measured, &inv, 2000.0, 0.0).unwrap().passed);
}

#[test]
fn wp_p5xo_the_programme_sum_adds_the_feed() {
    let (drivers, mut chains, _) = walked_and_summed(&Drv::flat(50.5));
    chains[1].feed.inverted = true;
    let prog = predict_sum(&drivers, &chains, SumMode::Programme, &ProtoModel).unwrap();
    let generator = predict_sum(
        &drivers,
        &chains,
        SumMode::Generator { invert_mask: 0b10 },
        &ProtoModel,
    )
    .unwrap();
    for (a, b) in prog.mag_db.iter().zip(&generator.mag_db) {
        assert!((a - b).abs() < 1e-9);
    }
    chains[1].feed.gain_db = -6.0;
    let prog = predict_sum(&drivers, &chains, SumMode::Programme, &ProtoModel).unwrap();
    assert!(prog != generator);
}

#[test]
fn wp_p5xo_bad_input_to_the_sum_and_check_is_refused() {
    let (drivers, chains, measured) = walked_and_summed(&Drv::flat(50.5));
    assert!(predict_sum(&drivers, &chains[..1], SumMode::Programme, &ProtoModel).is_err());
    let mut far = drivers.clone();
    far[1].output = 40;
    assert!(
        predict_sum(
            &far,
            &chains,
            SumMode::Generator { invert_mask: 0 },
            &ProtoModel
        )
        .is_err()
    );
    assert!(summed_check(&measured, &measured, f64::NAN, 0.0).is_err());
    let mut other = measured.clone();
    other.grid.points_per_octave = 48;
    assert!(summed_check(&measured, &other, 2000.0, 0.0).is_err());
    let mut none = measured.clone();
    none.valid = Some(vec![false; none.mag_db.len()]);
    assert!(matches!(
        summed_check(&none, &measured, 2000.0, 0.0),
        Err(MeasureError::NoSignal(_))
    ));
    // A band the model cannot model.
    let mut lt = chains.clone();
    lt[0].peq.push(BandSpec {
        filter_type: 11,
        bypass: false,
        freq: 40.0,
        q: 0.7,
        gain: 20.0,
        qp: 0.5,
    });
    assert!(predict_sum(&drivers, &lt, SumMode::Programme, &ProtoModel).is_err());
}

fn arb_drivers() -> impl Strategy<Value = (Vec<Driver>, Vec<Transition>)> {
    arb_ways().prop_map(|(ways, ts)| {
        let d: Vec<Driver> = ways
            .iter()
            .filter_map(|w| de_embed(w, &ProtoModel).ok())
            .collect();
        (d, ts)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn proptest_wp_p5xo_de_embed_never_panics_and_is_finite((ways, _) in arb_ways()) {
        for w in &ways {
            if let Ok(d) = de_embed(w, &ProtoModel) {
                prop_assert!(d.arrival_ms.is_finite());
                prop_assert!(d.response.mag_db.iter().all(|x| x.is_finite()));
                prop_assert!(d.response.phase_deg.as_ref().unwrap().iter().all(|x| x.is_finite()));
            }
        }
    }

    #[test]
    fn proptest_wp_p5xo_crossover_corner_never_panics_and_keeps_its_bounds(
        (drivers, ts) in arb_drivers(),
        win in 0.0..2.0f64,
        keep in any::<bool>(),
        tilt in -3.0..3.0f64,
    ) {
        let n = LogGrid::STANDARD.freqs().unwrap().len();
        let mut target = flat_target();
        target.mag_db = (0..n).map(|i| tilt * i as f64 / 96.0).collect();
        let c = XoConstraints { window_octaves: win, keep_corners: keep, ..XoConstraints::default() };
        if let Ok(p) = crossover_corner(&drivers, &ts, &target, &c, &ProtoModel) {
            for o in &p.outputs {
                prop_assert!(o.chain.gain_db <= o.chain_before.gain_db);
                prop_assert!(o.level_change_db.is_finite() && o.level_change_db <= 0.0);
                prop_assert!(o.chain.delay_ms.is_finite());
                prop_assert!(o.chain.crossover.iter().all(|b| b.freq.is_finite()));
            }
            for t in &p.transitions {
                prop_assert!(t.evaluations <= MAX_EVALUATIONS_PER_TRANSITION);
                prop_assert!(t.relative_delay_ms.is_finite() && t.corner_hz.is_finite());
                prop_assert!(t.cost_db.is_none_or(f64::is_finite));
            }
        }
    }

    #[test]
    fn proptest_wp_p5xo_predict_sum_and_summed_check_never_panic(
        (drivers, _) in arb_drivers(),
        mask in any::<u32>(),
        prog in any::<bool>(),
        corner in 1.0..30_000.0f64,
        offset in -20.0..20.0f64,
    ) {
        let chains: Vec<ChannelChain> = drivers.iter().map(|d| d.chain.clone()).collect();
        let mode = if prog { SumMode::Programme } else { SumMode::Generator { invert_mask: mask } };
        if let Ok(r) = predict_sum(&drivers, &chains, mode, &ProtoModel) {
            prop_assert!(r.mag_db.iter().all(|x| x.is_finite()));
            prop_assert!(r.phase_deg.as_ref().unwrap().iter().all(|x| x.is_finite()));
            if let Ok(c) = summed_check(&r, &r, corner, offset) {
                prop_assert!(c.rms_db.is_finite());
            }
        }
    }
}
