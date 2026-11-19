//! Wave P5-A review: what the seven work packages must agree on. Each test
//! names the review's cross-WP issue it settles.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use crate::align::tests_wp_p5xo::{Drv, ProtoModel, band, chain, peak, way};
use crate::align::{AlignGroup, AlignOptions, Transition, align};
use crate::cal::{
    CalFormat, CalibratorStep, MicAngle, level_dbfs, parse_calibration, spl_offset_from_tone,
    spl_scale,
};
use crate::doc::{DocKind, Meta, MicCal, MicDoc, SCHEMA_VERSION, Target as DocTarget, TargetDoc};
use crate::fit::{FitOptions, fit};
use crate::ir::ImpulseResponse;
use crate::response::{SmoothingDomain, from_grid_values, smooth, smooth_as};
use crate::rta::{Averaging, MicRta, RtaConfig};
use crate::target::{
    Builtin, DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse, builtins, import_target,
};
use crate::types::{
    BandSpec, ChannelChain, Coupler, FULL_SCALE_SINE_DB, LogGrid, MeasureError, ProgrammeFeed,
    Response, SectionModel, WayRole,
};
use crate::xover::{SumMode, XoConstraints, crossover_corner, de_embed, predict_sum};

fn tone(amp: f64, seconds: f64) -> Vec<f32> {
    (0..(48_000.0 * seconds) as usize)
        .map(|i| (amp * (std::f64::consts::TAU * 1000.0 * i as f64 / 48_000.0).sin()) as f32)
        .collect()
}

fn rta_at_1k(x: &[f32], spl_offset_db: Option<f64>) -> (f64, bool) {
    let cfg = RtaConfig {
        averaging: Averaging::INFINITE,
        ..RtaConfig::DEFAULT
    };
    let mut rta = MicRta::new(cfg, None, spl_offset_db).unwrap();
    rta.push(x).unwrap();
    let s = rta.spectrum().unwrap();
    let k = rta
        .grid()
        .freqs()
        .unwrap()
        .iter()
        .position(|f| (f - 1000.0).abs() < 1e-6)
        .unwrap();
    (s.mag_db[k], s.spl)
}

// -- Issue 1: one dBFS and dB SPL convention ---------------------------------

#[test]
fn wp_p5a_a_full_scale_sine_reads_0_dbfs_in_cal_and_in_the_rta() {
    assert!((FULL_SCALE_SINE_DB - 10.0 * 2f64.log10()).abs() < 1e-12);
    let full = tone(1.0, 1.0);
    assert!(level_dbfs(&full).unwrap().abs() < 0.01);
    let (band, spl) = rta_at_1k(&full, None);
    assert!(band.abs() < 0.05, "the RTA's 1 kHz band: {band} dBFS");
    assert!(!spl);
    // And 20 dB down both read -20.
    let quiet = tone(0.1, 1.0);
    assert!((level_dbfs(&quiet).unwrap() + 20.0).abs() < 0.01);
    assert!((rta_at_1k(&quiet, None).0 + 20.0).abs() < 0.05);
}

#[test]
fn wp_p5a_a_94_db_calibrator_reads_94_db_spl_across_cal_rta_and_the_mic_document() {
    for (amp, reference) in [(0.05, 94.0), (0.4, 114.0)] {
        let x = tone(amp, 1.5);
        let offset = spl_offset_from_tone(&x, 48_000, reference).unwrap();
        let step = CalibratorStep {
            offset_db: offset,
            reference_db: reference,
        };
        // P5CAL's scale on P5CAL's level.
        let scale = spl_scale(Some(-1.5), Some(step));
        assert!(!scale.approximate);
        let spl = scale.to_spl(level_dbfs(&x).unwrap()).unwrap();
        assert!((spl - reference).abs() < 0.05, "{spl}");
        // P5RESP's RTA with the same offset.
        let (band, is_spl) = rta_at_1k(&x, scale.offset_db);
        assert!(is_spl);
        assert!(
            (band - reference).abs() < 0.05,
            "the RTA reads {band} dB SPL"
        );
        // The same tone 6 dB quieter reads 6 dB less in both.
        let y = tone(amp / 2.0, 1.5);
        let l = scale.to_spl(level_dbfs(&y).unwrap()).unwrap();
        let (b, _) = rta_at_1k(&y, scale.offset_db);
        assert!((l - (reference - 6.02)).abs() < 0.05 && (b - l).abs() < 0.05);
        // The `mic` document carries that offset and gives the same scale back.
        let cal = parse_calibration(
            "\"Sens Factor =-1.5dB, SERNO: 7000001\"\n20 0.1\n20000 -0.2\n",
            "m.txt",
        )
        .unwrap();
        let block = MicCal::from_calibration("mic", &cal, Some(step), &LogGrid::STANDARD).unwrap();
        assert_eq!(block.spl_offset_db, Some(offset));
        assert!(!block.approximate);
        let again = block.spl_scale().to_spl(level_dbfs(&x).unwrap()).unwrap();
        assert!((again - reference).abs() < 0.05);
    }
}

// -- Issue 2: one smoothing, two domains --------------------------------------

fn flat_with(f0: f64, width_oct: f64, depth_db: f64) -> Response {
    let grid = LogGrid::STANDARD;
    let mag = grid
        .freqs()
        .unwrap()
        .iter()
        .map(|f| {
            if (f / f0).log2().abs() <= width_oct / 2.0 {
                depth_db
            } else {
                0.0
            }
        })
        .collect();
    Response {
        grid,
        mag_db: mag,
        phase_deg: None,
        valid: None,
        spl: false,
    }
}

fn at(r: &Response, hz: f64) -> f64 {
    let f = r.grid.freqs().unwrap();
    r.mag_db[f.iter().position(|x| *x >= hz).unwrap()]
}

#[test]
fn wp_p5a_smoothing_is_one_window_in_db_for_the_graph_and_in_power_for_the_fit() {
    // The graph's smoothing is the dB domain.
    let peak = flat_with(1000.0, 1.0 / 3.0, 12.0);
    assert_eq!(
        smooth(&peak, 3).unwrap(),
        smooth_as(&peak, 3, SmoothingDomain::Db).unwrap()
    );
    // A 30 dB null 1/12 octave wide under 1/3-octave smoothing: the dB mean
    // draws a 7.5 dB dip (a quarter of the window at -30 dB); as energy it is
    // a quarter of the window missing, 1.25 dB.
    let null = flat_with(1000.0, 1.0 / 12.0, -30.0);
    let db = at(&smooth_as(&null, 3, SmoothingDomain::Db).unwrap(), 1000.0);
    let power = at(
        &smooth_as(&null, 3, SmoothingDomain::Power).unwrap(),
        1000.0,
    );
    assert!((db + 7.5).abs() < 0.6, "{db}");
    assert!((power + 1.25).abs() < 0.3, "{power}");
    // A peak keeps its energy: the power mean of a 12 dB peak as wide as the
    // window is the peak, and the dB mean too.
    for d in [SmoothingDomain::Db, SmoothingDomain::Power] {
        let v = at(&smooth_as(&peak, 3, d).unwrap(), 1000.0);
        assert!((v - 12.0).abs() < 0.8, "{d:?}: {v}");
        // Flat stays flat, exactly enough.
        let flat = flat_with(1000.0, 1.0, 0.0);
        assert!(
            smooth_as(&flat, 6, d)
                .unwrap()
                .mag_db
                .iter()
                .all(|m| m.abs() < 1e-9)
        );
    }
    // Untrusted points never leak into their neighbours, in either domain.
    let mut holed = flat_with(1000.0, 1.0 / 12.0, 40.0);
    let f = holed.grid.freqs().unwrap();
    holed.valid = Some(
        f.iter()
            .map(|f| (f / 1000.0).log2().abs() > 1.0 / 24.0)
            .collect(),
    );
    for d in [SmoothingDomain::Db, SmoothingDomain::Power] {
        assert!(at(&smooth_as(&holed, 3, d).unwrap(), 1100.0).abs() < 1e-9);
    }
}

const FLAT_BAND: BandSpec = BandSpec {
    filter_type: 0,
    bypass: false,
    freq: 1000.0,
    q: 0.707,
    gain: 0.0,
    qp: 0.0,
};

fn peq_chain() -> ChannelChain {
    ChannelChain {
        peq: vec![FLAT_BAND; 10],
        crossover: Vec::new(),
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

#[test]
fn wp_p5a_the_fit_smooths_as_energy_so_a_narrow_null_is_not_a_dip_to_fill() {
    // A 25 dB null 1/24 octave wide at 80 Hz and nothing else: with the
    // default 1/24-octave power smoothing the fit sees a dip of a few dB at
    // most and never proposes more than its boost cap; with a dB mean it
    // would see the full depth. Nothing is boosted past +3 dB either way,
    // and the proposal is identical on a second run.
    let m = flat_with(80.0, 1.0 / 24.0, -25.0);
    let target = flat_with(80.0, 1.0, 0.0);
    let r = fit(
        &m,
        &target,
        &peq_chain(),
        &ProtoModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert!(r.bands.iter().all(|b| b.band.gain <= 3.0), "{:?}", r.bands);
    assert!(r.rms_after_db <= r.rms_before_db);
    let again = fit(
        &m,
        &target,
        &peq_chain(),
        &ProtoModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert_eq!(r.bands, again.bands);
}

// -- Issue 3: one filter response --------------------------------------------

#[test]
fn wp_p5a_the_fits_and_the_crossovers_filter_evaluators_agree_with_the_graphs() {
    // fit.rs evaluates |H|² from precomputed sines (its inner loop);
    // align.rs/xover.rs evaluate the complex response. They are two pieces
    // of code on purpose (the fit's must be fast), so they are pinned to each
    // other and to `dspi_proto::dsp::response_at`, which draws the graph.
    let bands = [
        peak(63.0, 4.0, -7.5),
        peak(2500.0, 0.7, 4.0),
        BandSpec {
            filter_type: 2,
            freq: 120.0,
            q: 0.7,
            gain: 3.0,
            ..FLAT_BAND
        },
        BandSpec {
            filter_type: 3,
            freq: 8000.0,
            q: 0.9,
            gain: -5.0,
            ..FLAT_BAND
        },
    ];
    let secs = crate::xover::sections(&bands, &ProtoModel).unwrap();
    let freqs = LogGrid::STANDARD.freqs().unwrap();
    let trig: Vec<crate::fit::Trig> = freqs
        .iter()
        .map(|f| crate::fit::Trig::at(*f, 48_000.0))
        .collect();
    let mut fit_db = Vec::new();
    assert!(crate::fit::cascade_db(&secs, &trig, &mut fit_db));
    let proto: Vec<dspi_proto::dsp::Band> = bands
        .iter()
        .map(|b| dspi_proto::dsp::Band {
            filter_type: dspi_proto::FilterType::from_raw(b.filter_type),
            freq: b.freq,
            q: b.q,
            gain_db: b.gain,
            bypass: false,
        })
        .collect();
    for (f, a) in freqs.iter().zip(&fit_db) {
        let b = crate::xover::cascade_at(&secs, *f, 48_000.0).unwrap().db();
        let c = dspi_proto::dsp::response_at(*f, &proto);
        assert!((a - b).abs() < 1e-9, "{f} Hz: fit {a}, xover {b}");
        assert!((a - c).abs() < 1e-6, "{f} Hz: fit {a}, graph {c}");
    }
    // A crossover pair too (the graph's cascade of sections).
    let xo = crate::xover::sections(&[band(34, 300.0), band(35, 3000.0)], &ProtoModel).unwrap();
    assert!(crate::fit::cascade_db(&xo, &trig, &mut fit_db));
    for (f, a) in freqs.iter().zip(&fit_db) {
        let b = crate::xover::cascade_at(&xo, *f, 48_000.0).unwrap().db();
        assert!((a - b).abs() < 1e-8 || *a < -190.0, "{f} Hz: {a} vs {b}");
    }
}

// -- Issue 4: cal and target line up with the documents -----------------------

#[test]
fn wp_p5a_every_builtin_target_round_trips_through_a_target_document() {
    let grid = LogGrid::STANDARD;
    for info in builtins() {
        assert_eq!(info.builtin.id(), info.id);
        let target = Target {
            target_use: info.target_use,
            coupler: info.coupler,
            definition: TargetDefinition::Builtin {
                builtin: info.builtin,
            },
            reference: DEFAULT_REFERENCE_BAND,
        };
        let doc = TargetDoc {
            schema_version: SCHEMA_VERSION,
            meta: Meta::new(
                DocKind::Target,
                &info.name,
                "2026-11-16T12:00:00Z",
                "0.13.0",
            ),
            target: DocTarget::from_target(&info.name, &target, &grid).unwrap(),
            extra: Default::default(),
        };
        let text = doc.to_text().unwrap();
        // The contract's spelling: {"type": "builtin", "builtin": id, "params": {..}}.
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["target"]["definition"]["type"], "builtin");
        assert_eq!(v["target"]["definition"]["builtin"], info.id.as_str());
        let back = TargetDoc::from_text(&text)
            .unwrap()
            .target
            .to_target()
            .unwrap();
        assert_eq!(back.target_use, target.target_use);
        assert_eq!(back.coupler, target.coupler);
        let (a, b) = (
            target.evaluate(&grid).unwrap(),
            back.evaluate(&grid).unwrap(),
        );
        for (x, y) in a.mag_db.iter().zip(&b.mag_db) {
            assert!((x - y).abs() < 1e-9, "{}: {x} vs {y}", info.id);
        }
    }
    // Parameters by name; one left out takes its default.
    let house =
        Builtin::from_id("house", &BTreeMap::from([("shelfGainDb".to_string(), 4.0)])).unwrap();
    assert_eq!(
        house,
        Builtin::House {
            shelf_gain_db: 4.0,
            shelf_corner_hz: 120.0,
            shelf_q: 0.707,
            tilt_db_per_octave: -0.3
        }
    );
    // An unknown id, a misspelt parameter and a value out of range are refused.
    for (id, params) in [
        ("harman", BTreeMap::new()),
        ("house", BTreeMap::from([("bassShelfDb".to_string(), 4.0)])),
        ("tilt", BTreeMap::from([("dbPerOctave".to_string(), 30.0)])),
        ("flat", BTreeMap::from([("dbPerOctave".to_string(), 1.0)])),
    ] {
        assert!(
            matches!(Builtin::from_id(id, &params), Err(MeasureError::Invalid(_))),
            "{id} {params:?}"
        );
    }
}

#[test]
fn wp_p5a_an_imported_target_and_a_compensation_round_trip_through_a_document() {
    let grid = LogGrid::STANDARD;
    let target = import_target(
        "20 3.0\n200 0.0\n2000 0.0\n20000 -6.0\n",
        TargetUse::Headphones,
        Some(Coupler::Hats),
    )
    .unwrap();
    let mut block = DocTarget::from_target("mine", &target, &grid).unwrap();
    block.compensation = Some(crate::doc::Curve::from_response(
        &crate::cal::curve_to_grid(
            &crate::target::import_compensation("100,1.0\n10000,2.0\n", Coupler::Hats)
                .unwrap()
                .points,
            &grid,
        )
        .unwrap(),
    ));
    let doc = TargetDoc {
        schema_version: SCHEMA_VERSION,
        meta: Meta::new(DocKind::Target, "mine", "2026-11-16T12:00:00Z", "0.13.0"),
        target: block,
        extra: Default::default(),
    };
    let read = TargetDoc::from_text(&doc.to_text().unwrap()).unwrap();
    let back = read.target.to_target().unwrap();
    let (a, b) = (
        target.evaluate(&grid).unwrap(),
        back.evaluate(&grid).unwrap(),
    );
    let va = a.valid.as_ref().unwrap();
    let vb = b.valid.as_ref().unwrap();
    for i in 0..a.mag_db.len() {
        // Trusted where the file had points; within the curve's 0.01 dB step.
        assert_eq!(va[i], vb[i]);
        if va[i] {
            assert!((a.mag_db[i] - b.mag_db[i]).abs() < 0.011, "{i}");
        }
    }
    let comp = read.target.to_compensation().unwrap().unwrap();
    assert_eq!(comp.coupler, Coupler::Hats);
    assert!(
        comp.points.first().unwrap().freq_hz >= 100.0
            && comp.points.last().unwrap().freq_hz <= 10_000.0
    );
}

#[test]
fn wp_p5a_a_calibration_file_becomes_a_mic_document_with_one_spelling() {
    let text = "\"Sens Factor =-1.378dB, AGain =18dB, SERNO: 8000002\"\n20 -0.5 3\n1000 0.0 0\n20000 1.5 -9\n";
    let cal = parse_calibration(text, "8000002_90deg.txt").unwrap();
    // cal's own JSON (what a binding carries) and the document's agree.
    let parsed = serde_json::to_value(&cal.curve).unwrap();
    assert_eq!(parsed["format"], "umik");
    assert_eq!(parsed["angle"], 90);
    assert_eq!(
        serde_json::from_value::<MicAngle>(serde_json::json!(0)).unwrap(),
        MicAngle::Deg0
    );
    assert!(serde_json::from_value::<MicAngle>(serde_json::json!(45)).is_err());
    let doc = MicDoc {
        schema_version: SCHEMA_VERSION,
        meta: Meta::new(DocKind::Mic, "UMIK-2", "2026-11-16T12:00:00Z", "0.13.0"),
        mic: MicCal::from_calibration("UMIK-2", &cal, None, &LogGrid::STANDARD).unwrap(),
        extra: Default::default(),
    };
    let out = doc.to_text().unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["mic"]["format"], "umik");
    assert_eq!(v["mic"]["angle"], 90);
    assert_eq!(v["mic"]["serial"], "8000002");
    assert_eq!(v["mic"]["vendor"], "miniDSP");
    assert_eq!(v["mic"]["approximate"], true);
    let back = MicDoc::from_text(&out).unwrap().mic;
    assert_eq!(back.format, CalFormat::Umik);
    assert_eq!(back.mic_angle(), Some(MicAngle::Deg90));
    assert_eq!(back.sensitivity_db, Some(-1.378));
    assert_eq!(back.gain_db, Some(18.0));
    // The sensitivity's scale, approximate: dB SPL = dBFS + 94 - sensitivity.
    assert_eq!(back.spl_scale(), cal.spl_scale());
    // The correction is the file's curve on the grid, phase kept.
    let want = cal.to_response(&LogGrid::STANDARD).unwrap();
    let got = back.correction();
    assert_eq!(got.valid, want.valid);
    for (a, b) in got.mag_db.iter().zip(&want.mag_db) {
        assert!((a - b).abs() < 0.0051);
    }
    assert!(got.phase_deg.is_some());
}

// -- Issue 5: odd-order Butterworth pairs -------------------------------------

/// A two-way with a third-order Butterworth pair at 2 kHz, the woofer
/// 0.35 ms later than the tweeter.
fn bw3_two_way(tweeter_inverted: bool) -> Vec<crate::align::Way> {
    const BW3_LP: u8 = 44;
    const BW3_HP: u8 = 45;
    let mut t = Drv::flat(50.5);
    t.inverted = tweeter_inverted;
    vec![
        way(
            0,
            WayRole::Low,
            chain(vec![band(BW3_LP, 2000.0)]),
            &Drv::flat(50.85),
            60.0,
        ),
        way(
            1,
            WayRole::High,
            chain(vec![band(BW3_HP, 2000.0)]),
            &t,
            60.0,
        ),
    ]
}

fn pair() -> Vec<Transition> {
    vec![Transition {
        low: 0,
        high: 1,
        corner_hz: None,
    }]
}

#[test]
fn wp_p5a_an_odd_order_butterworth_pair_is_aligned_to_quadrature_and_keeps_its_polarity() {
    let flat = Builtin::Flat.evaluate(&LogGrid::STANDARD).unwrap();
    for inverted in [false, true] {
        let ways = bw3_two_way(inverted);
        // align(): the tweeter is 0.35 ms early; no polarity is proposed.
        let a = align(
            &ways,
            &AlignGroup {
                transitions: pair(),
            },
            &AlignOptions::default(),
            &ProtoModel,
        )
        .unwrap();
        assert!(
            (f64::from(a.ways[1].delay_ms) - 0.35).abs() <= 0.010,
            "{a:?}"
        );
        assert_eq!(a.ways[0].delay_ms, 0.0);
        assert!(!a.pairs[0].flipped && a.pairs[0].confidence == 0.0, "{a:?}");
        assert!(a.ways.iter().all(|w| w.inverted == w.inverted_before));
        assert!(a.notes.iter().any(|n| n.contains("odd-order Butterworth")));
        // crossover_corner(): the same, with and without a corner search.
        let drivers: Vec<_> = ways
            .iter()
            .map(|w| de_embed(w, &ProtoModel).unwrap())
            .collect();
        for keep_corners in [true, false] {
            let c = XoConstraints {
                keep_corners,
                ..XoConstraints::default()
            };
            let p = crossover_corner(&drivers, &pair(), &flat, &c, &ProtoModel).unwrap();
            let t = &p.transitions[0];
            assert!(
                (t.relative_delay_ms - 0.35).abs() <= 0.010,
                "{inverted} {keep_corners}: {t:?}"
            );
            assert!(!t.flipped);
            assert!(p.outputs.iter().all(|o| !o.chain.feed.inverted));
            assert!(p.notes.iter().any(|n| n.contains("odd-order Butterworth")));
            // The proposal sums flat through the transition: no +3 dB bump
            // from pulling a quadrature pair into phase.
            let chains: Vec<ChannelChain> = p.outputs.iter().map(|o| o.chain.clone()).collect();
            let sum = predict_sum(&drivers, &chains, SumMode::Programme, &ProtoModel).unwrap();
            let f = LogGrid::STANDARD.freqs().unwrap();
            for (f, m) in f.iter().zip(&sum.mag_db) {
                if (1000.0..=4000.0).contains(f) {
                    assert!(m.abs() < 0.3, "{inverted} {keep_corners}: {m} dB at {f} Hz");
                }
            }
        }
    }
}

#[test]
fn wp_p5a_align_and_align_only_agree_and_a_near_tie_keeps_the_polarity() {
    // The LR4 three-way of P5XO's tests through both entries: the same
    // relative delays and flips (one rule in both).
    let w = Drv::flat(50.6);
    let m = Drv::flat(50.3);
    let mut t = Drv::flat(50.2);
    t.inverted = true;
    let ways = vec![
        way(0, WayRole::Low, chain(vec![band(34, 300.0)]), &w, 60.0),
        way(
            1,
            WayRole::Mid,
            chain(vec![band(35, 300.0), band(34, 3000.0)]),
            &m,
            60.0,
        ),
        way(2, WayRole::High, chain(vec![band(35, 3000.0)]), &t, 60.0),
    ];
    let tr = vec![
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
    ];
    let a = align(
        &ways,
        &AlignGroup {
            transitions: tr.clone(),
        },
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    let drivers: Vec<_> = ways
        .iter()
        .map(|w| de_embed(w, &ProtoModel).unwrap())
        .collect();
    let flat = Builtin::Flat.evaluate(&LogGrid::STANDARD).unwrap();
    let c = XoConstraints {
        keep_corners: true,
        ..XoConstraints::default()
    };
    let p = crossover_corner(&drivers, &tr, &flat, &c, &ProtoModel).unwrap();
    for (x, y) in a.pairs.iter().zip(&p.transitions) {
        assert_eq!(x.flipped, y.flipped);
        assert!(
            (x.relative_delay_ms - y.relative_delay_ms).abs() < 0.002,
            "{x:?} {y:?}"
        );
    }
    assert_eq!(
        a.ways.iter().map(|w| w.inverted).collect::<Vec<_>>(),
        vec![false, false, true]
    );
    // The mid arrives 0.3 ms before the woofer, but its own 3 kHz LR4
    // low-pass delays it by 0.150 ms where it meets the woofer (2·√2/ω0):
    // phase agreement asks for the rest. The tweeter is 0.1 ms before the mid.
    let (mid, high) = (f64::from(a.ways[1].delay_ms), f64::from(a.ways[2].delay_ms));
    assert!(
        (mid - 0.150).abs() < 0.01 && (high - mid - 0.1).abs() < 0.02,
        "{a:?}"
    );
    assert!(
        a.pairs[1].confidence > 0.5 && a.pairs[1].cost_db < 0.1,
        "{a:?}"
    );
}

// -- Review findings ----------------------------------------------------------

#[test]
fn wp_p5a_a_released_band_in_use_is_cleared_when_the_fit_does_not_reuse_it() {
    // F1: bands 2 (free) and 3 (the user's +6 dB at 80 Hz) are released. The
    // prediction takes band 3 out, so the proposal must write band 3 too,
    // whichever index the fitted bands land on: applied to the chain, the
    // proposal gives exactly the predicted curve.
    let user = peak(80.0, 2.0, 6.0);
    let mut c = peq_chain();
    c.peq[3] = user;
    let grid = LogGrid::STANDARD;
    let freqs = grid.freqs().unwrap();
    let model = ProtoModel;
    let db_of = |bands: &[BandSpec]| -> Vec<f64> {
        let secs = crate::xover::sections(bands, &model).unwrap();
        freqs
            .iter()
            .map(|f| {
                crate::xover::cascade_at(&secs, *f, model.sample_rate()).map_or(0.0, |h| h.db())
            })
            .collect()
    };
    // A room with one mode at 45 Hz, measured through the chain.
    let mode = db_of(&[peak(45.0, 6.0, 7.0)]);
    let through = db_of(&c.peq);
    let measured = Response {
        grid,
        mag_db: mode
            .iter()
            .zip(&through)
            .map(|(a, b)| 70.0 + a + b)
            .collect(),
        phase_deg: None,
        valid: None,
        spl: false,
    };
    let target = flat_with(100.0, 1.0, 0.0);
    for released in [vec![2u8, 3], vec![3, 2], vec![3]] {
        let opts = FitOptions {
            bands: Some(released.clone()),
            max_bands: Some(1),
            headroom: false,
            ..FitOptions::speakers()
        };
        let r = fit(&measured, &target, &c, &model, &opts).unwrap();
        assert!(!r.bands.is_empty(), "{:?}", r.notes);
        let mut applied = c.clone();
        for b in &r.bands {
            applied.peq[usize::from(b.index)] = b.band;
        }
        assert!(
            applied.peq[3] != user,
            "{released:?}: the band in use is overwritten or cleared: {:?}",
            r.bands
        );
        let after = db_of(&applied.peq);
        for (i, f) in freqs.iter().enumerate() {
            let want = measured.mag_db[i] - through[i] + after[i];
            assert!(
                (r.predicted.mag_db[i] - want).abs() < 1e-6,
                "{released:?} at {f} Hz: predicted {} but the device would give {want}",
                r.predicted.mag_db[i]
            );
        }
    }
}

#[test]
fn wp_p5a_the_onset_does_not_jump_between_the_lobes_of_a_band_limited_response() {
    // Two responses that differ only in which lobe is a hair higher: the
    // peak moves by half a period, the onset stays.
    let lobes = |a: f64, b: f64| -> ImpulseResponse {
        let mut s = vec![0.0; 400];
        for (i, v) in s.iter_mut().enumerate().skip(100).take(48) {
            let x = (i - 100) as f64 / 24.0;
            let lobe = (std::f64::consts::PI * x).sin();
            *v = if x < 1.0 { a * lobe } else { b * lobe };
        }
        ImpulseResponse {
            rate_hz: 48_000,
            samples: s,
            zero_index: 100,
        }
    };
    let (first, second) = (lobes(1.0, 0.98), lobes(0.98, 1.0));
    let (p1, p2) = (first.peak_index().unwrap(), second.peak_index().unwrap());
    assert!(
        (p2 - p1 - 24.0).abs() < 1.0,
        "the peak jumps a lobe: {p1} {p2}"
    );
    let (o1, o2) = (first.onset_index().unwrap(), second.onset_index().unwrap());
    assert!((o1 - o2).abs() < 0.2, "the onset stays: {o1} {o2}");
    assert!(o1 > 100.0 && o1 < 106.0, "{o1}");
    // Silence and nothing have no onset; a response that starts loud has 0.
    let silent = ImpulseResponse {
        rate_hz: 48_000,
        samples: vec![0.0; 16],
        zero_index: 0,
    };
    assert_eq!(silent.onset_index(), None);
    let empty = ImpulseResponse {
        rate_hz: 48_000,
        samples: Vec::new(),
        zero_index: 0,
    };
    assert_eq!(empty.onset_index(), None);
    let step = ImpulseResponse {
        rate_hz: 48_000,
        samples: vec![1.0, 0.5],
        zero_index: 0,
    };
    assert_eq!(step.onset_index(), Some(0.0));
}

#[test]
fn wp_p5a_values_at_grid_points_unwrap_with_the_arrival_as_reference() {
    // A way 5 ms after the timing origin: 145 Hz between grid points at
    // 20 kHz is 0.72 of a turn a point. With its arrival as the reference the
    // phase is the straight line it must be.
    let grid = LogGrid::STANDARD;
    let freqs = grid.freqs().unwrap();
    let tau = 0.005;
    let values: Vec<[f64; 2]> = freqs
        .iter()
        .map(|f| {
            let p = -std::f64::consts::TAU * f * tau;
            [2.0 * p.cos(), 2.0 * p.sin()]
        })
        .collect();
    for hint in [tau, tau - 0.000_2, tau + 0.000_3] {
        let r = from_grid_values(&values, &grid, hint).unwrap();
        let ph = r.phase_deg.as_ref().unwrap();
        for (f, p) in freqs.iter().zip(ph) {
            // Right to a whole number of turns at the first point, then
            // continuous: the same whole number everywhere.
            let err = p + 360.0 * f * tau;
            let turns = (err / 360.0).round();
            assert!((err - turns * 360.0).abs() < 1e-6, "{hint}: {f} Hz: {err}");
            let first = ph[0] + 360.0 * freqs[0] * tau;
            assert!((err - first).abs() < 1e-6, "{hint}: continuous at {f} Hz");
        }
        assert!(
            r.mag_db
                .iter()
                .all(|m| (m - 20.0 * 2f64.log10()).abs() < 1e-9)
        );
        assert!(r.valid.as_ref().unwrap().iter().all(|v| *v));
    }
    // Without the reference the same values cannot be unwrapped.
    let wrong = from_grid_values(&values, &grid, 0.0).unwrap();
    let last = freqs.len() - 1;
    assert!((wrong.phase_deg.unwrap()[last] + 360.0 * freqs[last] * tau).abs() > 360.0);
    // A zero value is untrusted; bad input is a typed error.
    let mut holed = values.clone();
    holed[10] = [0.0, 0.0];
    let r = from_grid_values(&holed, &grid, tau).unwrap();
    assert!(!r.valid.as_ref().unwrap()[10] && r.mag_db[10] == crate::response::FLOOR_DB);
    assert!(from_grid_values(&values[..5], &grid, 0.0).is_err());
    assert!(from_grid_values(&values, &grid, f64::NAN).is_err());
    assert!(from_grid_values(&values, &grid, 11.0).is_err());
    holed[3] = [f64::INFINITY, 0.0];
    assert!(from_grid_values(&holed, &grid, 0.0).is_err());
}

#[test]
fn wp_p5a_a_document_read_and_written_again_is_the_same_text() {
    // F3: numbers of 17 digits (a measured clock ratio, a residual, an
    // arrival, a band grid's irrational edge) used to come back one unit in
    // the last place off, so the second write differed from the first.
    use crate::doc::{
        CaptureInfo, Curve, Measurement, MeasurementDoc, OutputMeasurement, RunKind, Stimulus,
    };
    let grid = crate::rta::band_grid(48_000.0, 3).unwrap();
    let n = grid.freqs().unwrap().len();
    let mut worst = 0usize;
    for k in 0..400u32 {
        // Awkward decimals from a simple generator.
        let x = |i: u32| -> f64 { f64::from(k * 7919 + i * 104_729 + 1).sqrt().fract() };
        let doc = MeasurementDoc {
            schema_version: SCHEMA_VERSION,
            meta: Meta::new(DocKind::Measurement, "m", "2026-11-16T12:00:00Z", "0.13.0"),
            measurement: Measurement {
                id: String::new(),
                serial: "S".into(),
                platform: "RP2350".into(),
                saved_ms: 1,
                placement: crate::types::Placement::ListeningPosition,
                mic: None,
                stimulus: Stimulus {
                    kind: RunKind::Walked,
                    f1_hz: 20.0 + x(1),
                    f2_hz: 20_000.0 + x(2),
                    duration_ms: 5_500,
                    repeat: 3,
                    gap_ms: 1_000,
                    level_db: -12.0 - x(3),
                    device_rate_hz: 48_000,
                    extra: Default::default(),
                },
                capture: CaptureInfo {
                    rate_hz: 48_000,
                    clock_ratio: 1.0 + 1e-4 * (x(4) - 0.5),
                    timing_reliable: true,
                    max_residual_us: Some(x(5) * 9.0),
                    extra: Default::default(),
                },
                outputs: vec![OutputMeasurement {
                    output: 0,
                    role: WayRole::Full,
                    speaker: "L".into(),
                    chain: peq_chain().into(),
                    arrival_ms: Some(x(6) * 3.0),
                    response: Curve::from_response(&Response {
                        grid,
                        mag_db: vec![1.0; n],
                        phase_deg: None,
                        valid: None,
                        spl: false,
                    }),
                    extra: Default::default(),
                }],
                positions: 1,
                note: String::new(),
                tunes: Vec::new(),
                extra: Default::default(),
            },
            extra: Default::default(),
        };
        let first = doc.to_text().unwrap();
        let second = MeasurementDoc::from_text(&first)
            .unwrap()
            .to_text()
            .unwrap();
        assert_eq!(first, second, "case {k}");
        // What the canonical form costs: nothing a measurement can see.
        let back = MeasurementDoc::from_text(&first).unwrap();
        let (a, b) = (
            back.measurement.capture.clock_ratio,
            doc.measurement.capture.clock_ratio,
        );
        assert!((a - b).abs() < 1e-14);
        worst = worst.max(first.len());
    }
    assert!(worst < 16 * 1024);
}

#[test]
fn wp_p5a_a_q_range_narrower_than_the_grid_step_still_holds_its_bounds() {
    // F4 (found by P5FIT's own property test on another seed): Q from 1.5953
    // to 1.5953 holds no value of the 0.01 grid; the band came out at 1.59,
    // below the range.
    let grid = LogGrid::STANDARD;
    let secs = crate::xover::sections(&[peak(226.0, 0.5, 4.4)], &ProtoModel).unwrap();
    let measured = Response {
        grid,
        mag_db: grid
            .freqs()
            .unwrap()
            .iter()
            .map(|f| 70.0 + crate::xover::cascade_at(&secs, *f, 48_000.0).map_or(0.0, |h| h.db()))
            .collect(),
        phase_deg: None,
        valid: None,
        spl: false,
    };
    let q = 1.595_297_625_708_042_7;
    for (q_min, q_max) in [(q, q), (q, q + 0.001), (0.301, 0.309), (9.991, 9.999)] {
        let opts = FitOptions {
            min_hz: 15.0,
            max_hz: 200.0,
            max_boost_db: 0.0,
            max_cut_db: 6.0,
            q_min,
            q_max,
            shelves: false,
            smoothing: None,
            ..FitOptions::speakers()
        };
        let r = fit(
            &measured,
            &flat_with(100.0, 1.0, 0.0),
            &peq_chain(),
            &ProtoModel,
            &opts,
        )
        .unwrap();
        assert!(!r.bands.is_empty(), "{q_min}..{q_max}: {:?}", r.notes);
        for b in &r.bands {
            let got = f64::from(b.band.q);
            assert!(
                got >= q_min - 1e-6 && got <= q_max + 1e-6,
                "Q {got} outside {q_min}..{q_max}"
            );
        }
    }
}
