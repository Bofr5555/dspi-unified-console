//! WP P5LIVE: the live view's maths and its setup, without a device.

use dspi_measure::fit::FittedBand;
use dspi_measure::types::ProgrammeFeed;
use proptest::prelude::*;

use super::*;

fn flat_band() -> BandSpec {
    BandSpec {
        filter_type: 0,
        bypass: false,
        freq: 1_000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

fn peak(freq: f32, q: f32, gain: f32) -> BandSpec {
    BandSpec {
        filter_type: 1,
        bypass: false,
        freq,
        q,
        gain,
        qp: 0.0,
    }
}

fn chain(peq: Vec<BandSpec>, gain_db: f32) -> ChannelChain {
    ChannelChain {
        peq,
        crossover: Vec::new(),
        gain_db,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

/// A flat response at `db`, trusted from `from_hz` up.
fn flat_resp(db: f64, from_hz: f64) -> Response {
    let g = LogGrid::STANDARD;
    let f = g.freqs().unwrap();
    Response {
        grid: g,
        mag_db: vec![db; f.len()],
        phase_deg: None,
        valid: Some(f.iter().map(|x| *x >= from_hz).collect()),
        spl: false,
    }
}

fn nearest(freqs: &[f64], f: f64) -> usize {
    freqs
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 / f).ln().abs().total_cmp(&(b.1 / f).ln().abs()))
        .unwrap()
        .0
}

#[test]
fn wp_p5live_the_prediction_takes_the_measured_chain_out_and_puts_the_new_one_in() {
    let measured_with = chain(vec![peak(100.0, 1.0, 4.0), flat_band()], 0.0);
    // The measurement: 0 dB in the room, plus what the chain did.
    let freqs = LogGrid::STANDARD.freqs().unwrap();
    let mut m = flat_resp(0.0, 10.0);
    m.mag_db = chain_db(&measured_with, &CoreSectionModel, &freqs);
    // The new chain: the bass peak gone, a cut at 2 kHz, the gain -1 dB.
    let next = chain(vec![flat_band(), peak(2_000.0, 2.0, -6.0)], -1.0);
    let p = repredict(&m, &measured_with, &next, &CoreSectionModel).unwrap();
    let k100 = nearest(&freqs, 100.0);
    let k2k = nearest(&freqs, 2_000.0);
    assert!((p.mag_db[k100] - -1.0).abs() < 0.05, "{}", p.mag_db[k100]);
    assert!((p.mag_db[k2k] - -7.0).abs() < 0.05, "{}", p.mag_db[k2k]);
    assert!(p.phase_deg.is_none(), "magnitude only");
    assert_eq!(p.valid, m.valid);
    // An unchanged chain predicts the measurement.
    let same = repredict(&m, &measured_with, &measured_with, &CoreSectionModel).unwrap();
    for (a, b) in same.mag_db.iter().zip(&m.mag_db) {
        assert!((a - b).abs() < 1e-9);
    }
}

#[test]
fn wp_p5live_a_rise_lowers_the_next_ramp_and_a_cut_does_not() {
    let a = chain(vec![flat_band()], 0.0);
    let b = chain(vec![peak(1_000.0, 2.0, 4.0)], 0.0);
    let rise = max_rise_db(&a, &b, &CoreSectionModel);
    assert!((rise - 4.0).abs() < 0.05, "{rise}");
    assert!(max_rise_db(&b, &a, &CoreSectionModel).abs() < 1e-9);
    let louder = chain(vec![flat_band()], 2.0);
    assert!((max_rise_db(&a, &louder, &CoreSectionModel) - 2.0).abs() < 1e-6);
}

#[test]
fn wp_p5live_a_fit_starts_at_the_lowest_trusted_point() {
    // Review P5-A N11: never into a roll-off the measurement cannot see.
    let r = flat_resp(0.0, 45.0);
    let (lo, hi) = fit_span(&r, 20.0, 300.0).unwrap();
    // A third of an octave above the first trusted point (45.1 Hz).
    assert!((56.0..58.0).contains(&lo), "{lo}");
    assert!((hi - 300.0).abs() < 1e-9);
    // Its top end the same way below the last (22 kHz).
    let (_, top) = fit_span(&r, 1_000.0, 40_000.0).unwrap();
    assert!((17_000.0..17_600.0).contains(&top), "{top}");
    // Given either way round.
    assert_eq!(fit_span(&r, 300.0, 20.0), Some((lo, hi)));
    // Nothing trusted there.
    assert_eq!(fit_span(&r, 10.0, 50.0), None);
    assert_eq!(fit_span(&r, f64::NAN, 40.0), None);
    let mut none = r.clone();
    none.valid = Some(vec![false; none.mag_db.len()]);
    assert_eq!(fit_span(&none, 20.0, 300.0), None);
}

#[test]
fn wp_p5live_every_band_a_fit_lists_is_written_a_flat_one_too() {
    // Review P5-A: a released band the fit does not need comes back flat and
    // must be written.
    let c = chain(
        vec![flat_band(), peak(80.0, 3.0, 5.0), flat_band(), flat_band()],
        0.0,
    );
    let r = FitResult {
        bands: vec![
            FittedBand {
                index: 1,
                band: flat_band(),
            },
            FittedBand {
                index: 3,
                band: peak(55.0, 4.1, -5.8),
            },
        ],
        predicted: flat_resp(0.0, 10.0),
        rms_before_db: 2.0,
        rms_after_db: 1.0,
        max_error_after_db: 1.0,
        target_offset_db: 0.0,
        gain_cut_db: 0.0,
        evaluations: 1,
        notes: Vec::new(),
    };
    let w = fit_writes(&r, &c);
    assert_eq!(w.len(), 2);
    assert_eq!((w[0].band, w[0].after.filter_type), (1, 0));
    assert_eq!(w[0].before.filter_type, 1, "the band in use, before");
    assert_eq!(
        w[0].words,
        "PEQ band 2: peak +5.0 dB at 80 Hz, Q 3.00 → flat"
    );
    assert_eq!(
        w[1].after.qp, w[1].before.qp,
        "the target Q kept for a peak"
    );
    assert_eq!((w[1].band, w[1].after.filter_type), (3, 1));
    assert_eq!(
        w[1].words,
        "PEQ band 4: flat → peak -5.8 dB at 55 Hz, Q 4.10"
    );
    // Applied to the chain, both land.
    let after = chain_with(&c, &w, Some(-1.5));
    assert_eq!(after.peq[1].filter_type, 0);
    assert_eq!(after.peq[3], peak(55.0, 4.1, -5.8));
    assert_eq!(after.gain_db, -1.5);
}

#[test]
fn wp_p5live_bands_in_words() {
    let p = |t: u8, f: f32, g: f32| FilterParams {
        filter_type: t,
        bypass: false,
        freq: f,
        q: 0.71,
        gain: g,
        qp: 0.0,
    };
    assert_eq!(band_words(&p(0, 1.0, 0.0)), "flat");
    assert_eq!(
        band_words(&p(2, 120.0, 3.0)),
        "low shelf +3.0 dB at 120 Hz, Q 0.71"
    );
    assert_eq!(
        band_words(&p(3, 9_500.0, -2.0)),
        "high shelf -2.0 dB at 9.50 kHz, Q 0.71"
    );
    assert_eq!(band_words(&p(40, 300.0, 0.0)), "type 40 at 300 Hz");
    let mut b = p(1, 1_000.0, -4.0);
    b.bypass = true;
    assert!(band_words(&b).ends_with("(bypassed)"));
}

#[test]
fn wp_p5live_the_target_is_levelled_at_the_reference_band() {
    let measured = flat_resp(-23.0, 20.0);
    let target = flat_resp(0.0, 10.0);
    let t = levelled_target(&target, &measured);
    assert!(t.mag_db.iter().all(|m| (m + 23.0).abs() < 1e-9));
    // Nothing trusted in 500 Hz to 2 kHz: every trusted point decides.
    let mut m = flat_resp(-10.0, 3_000.0);
    m.spl = true;
    let t = levelled_target(&target, &m);
    assert!(t.mag_db.iter().all(|v| (v + 10.0).abs() < 1e-9));
    assert!(t.spl);
}

#[test]
fn wp_p5live_curves_are_on_the_display_grid() {
    let c = curve(&flat_resp(-3.0, 50.0), 6).unwrap();
    assert_eq!(c.freqs_hz.len(), DISPLAY_GRID.freqs().unwrap().len());
    assert_eq!(c.mag_db.len(), c.freqs_hz.len());
    assert_eq!(c.valid.len(), c.freqs_hz.len());
    assert!(!c.valid[0] && *c.valid.last().unwrap());
    assert!(c.mag_db.iter().all(|m| (m + 3.0).abs() < 1e-9));
}

#[test]
fn wp_p5live_the_setup_is_checked() {
    let ok = MeasureLiveSetup::default();
    let (s, target, headphones) = check_setup(&ok).unwrap();
    assert_eq!(s.kind, MeasureRunKind::LiveSweep);
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.pink_ms, PINK_REARM_MS);
    assert!(!headphones);
    assert_eq!(
        target.mag_db.len(),
        LogGrid::STANDARD.freqs().unwrap().len()
    );
    // Pink noise: finite, at most 60 s.
    let pink = MeasureLiveSetup {
        mode: MeasureLiveMode::PinkRta,
        pink_ms: 60_000,
        ..ok.clone()
    };
    assert_eq!(check_setup(&pink).unwrap().0.kind, MeasureRunKind::Pink);
    for bad in [
        MeasureLiveSetup {
            pink_ms: 60_001,
            ..pink.clone()
        },
        MeasureLiveSetup {
            placement: MeasurePlacement::MultiPosition,
            ..ok.clone()
        },
        MeasureLiveSetup {
            smoothing: 49,
            ..ok.clone()
        },
        MeasureLiveSetup {
            bands_per_octave: 49,
            ..ok.clone()
        },
        MeasureLiveSetup {
            output: 16,
            ..ok.clone()
        },
        MeasureLiveSetup {
            sweep_ms: 100,
            ..ok.clone()
        },
        MeasureLiveSetup {
            target_document: "{\"schemaVersion\": 1}".into(),
            ..ok.clone()
        },
        MeasureLiveSetup {
            ceiling_db: Some(0.0),
            ..ok.clone()
        },
    ] {
        assert!(
            matches!(check_setup(&bad), Err(DspiError::InvalidArgument { .. })),
            "{bad:?}"
        );
    }
    // Headphones: one seat, the coupler's own default target.
    let hp = MeasureLiveSetup {
        placement: MeasurePlacement::Headphones,
        coupler: Some(MeasureCoupler::FlatPlate),
        ..ok.clone()
    };
    let (s, t_hp, headphones) = check_setup(&hp).unwrap();
    assert!(headphones);
    assert_eq!(s.count, 1);
    assert_ne!(t_hp.mag_db, target.mag_db);
    // A target document is read.
    let doc = crate::target_import(
        "Flat".into(),
        "20 0\n1000 0\n20000 -3\n".into(),
        "speakers".into(),
        String::new(),
    )
    .unwrap();
    let (_, t_doc, _) = check_setup(&MeasureLiveSetup {
        target_document: doc,
        ..ok
    })
    .unwrap();
    assert!(t_doc.mag_db.iter().all(|m| m.is_finite()));
}

#[test]
fn wp_p5live_the_tee_keeps_a_pink_run_s_playing_part_only() {
    let slot = MeasureSlot::default();
    let st = MeasureState {
        kind: MeasureRunKind::Pink,
        phase: MeasurePhase::Run,
        level_db: Some(-40.0),
        generator_start_frame: Some(1_000),
        ..MeasureState::default()
    };
    {
        let mut t = slot.tee_lock();
        t.on = true;
        t.skip_frames = 100;
        t.pink_frames = 500;
    }
    let block = vec![0.5f32; 1_000];
    // Frames 600..1600: only 1100..1500 kept.
    tee_push(&slot, &st, 600, &block);
    assert_eq!(slot.tee_lock().buf.len(), 400);
    // Not while ramping, not off.
    let ramp = MeasureState {
        phase: MeasurePhase::Ramp,
        ..st.clone()
    };
    tee_push(&slot, &ramp, 1_100, &block);
    assert_eq!(slot.tee_lock().buf.len(), 400);
    slot.tee_lock().on = false;
    tee_push(&slot, &st, 1_100, &block);
    assert_eq!(slot.tee_lock().buf.len(), 400);
    // Bounded: an undrained copy starts again.
    {
        let mut t = slot.tee_lock();
        t.on = true;
        t.pink_frames = u64::MAX;
        t.buf = vec![0.0; TEE_MAX];
    }
    tee_push(&slot, &st, 1_200, &block);
    assert_eq!(slot.tee_lock().buf.len(), 1_000);
}

#[test]
fn wp_p5live_calls_without_a_live_view() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    assert!(!core.measure_live_tick().active);
    assert!(!core.measure_live_stop().active);
    assert!(!core.measure_live_frame().active);
    assert!(core.measure_live_cut_peak(0, 1_000.0).is_err());
    assert!(core.measure_live_fit_range(0, 20.0, 300.0).is_err());
    assert!(core.measure_live_predict(0, Vec::new()).is_err());
    assert!(core.measure_live_apply(1).is_err());
    assert!(core.measure_live_hold(true).is_err());
    assert!(
        core.measure_live_averaging(MeasureRtaAveraging::Fast)
            .is_err()
    );
    assert!(core.measure_live_discard().pending.is_none());
}

#[test]
fn wp_p5live_notice_keys_have_rows() {
    for k in [NOTICE_NO_FRAME, NOTICE_NOTHING, NOTICE_CHAIN_CHANGED] {
        assert!(super::super::NOTICE_KEYS.contains(&k), "{k}");
        let n = crate::notice_for_key(k.to_string(), String::new());
        assert_eq!(n.key, k, "a row for {k}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn proptest_wp_p5live_the_prediction_is_finite_for_any_chain(
        mags in proptest::collection::vec(-200.0f64..200.0, 1066),
        bands in proptest::collection::vec(
            (0u8..64, any::<bool>(), 1.0f32..30_000.0, 0.05f32..30.0, -30.0f32..30.0),
            0..12,
        ),
        gain in -60.0f32..20.0,
    ) {
        let mut m = flat_resp(0.0, 10.0);
        m.mag_db = mags;
        let peq: Vec<BandSpec> = bands
            .iter()
            .map(|(t, b, f, q, g)| BandSpec {
                filter_type: *t,
                bypass: *b,
                freq: *f,
                q: *q,
                gain: *g,
                qp: 0.707,
            })
            .collect();
        let a = chain(peq.clone(), 0.0);
        let b = chain(peq.into_iter().rev().collect(), gain);
        let p = repredict(&m, &a, &b, &CoreSectionModel).unwrap();
        prop_assert!(p.mag_db.iter().all(|v| v.is_finite()));
        prop_assert!(max_rise_db(&a, &b, &CoreSectionModel).is_finite());
        if let Some(c) = curve(&p, 12) {
            prop_assert!(c.mag_db.iter().all(|v| v.is_finite()));
        }
        let t = levelled_target(&flat_resp(0.0, 10.0), &p);
        prop_assert!(t.mag_db.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn proptest_wp_p5live_fit_spans_stay_inside_the_trusted_range(
        from in 10.0f64..20_000.0,
        f1 in -100.0f64..40_000.0,
        f2 in -100.0f64..40_000.0,
    ) {
        let r = flat_resp(0.0, from);
        if let Some((lo, hi)) = fit_span(&r, f1, f2) {
            prop_assert!(lo >= from * EDGE_MARGIN * 0.999 && hi >= lo && lo > 0.0);
            prop_assert!(lo >= f1.min(f2) && hi <= f1.max(f2));
        }
    }
}

// -- WP P5TRUST -- "fit this range" anchors its level in the reference band
// where the output plays there.
#[test]
fn wp_p5trust_the_live_fit_anchors_only_where_the_output_plays_the_reference_band() {
    let flat_target = flat_resp(0.0, 20.0);
    let m = flat_resp(-30.0, 20.0);
    // A full-range output: the measurement's level in 500 Hz to 2 kHz.
    let full = chain(vec![flat_band(); 10], 0.0);
    let a = live_anchor(&m, &flat_target, &full).unwrap();
    assert!((a + 30.0).abs() < 1e-9, "{a}");
    // A woofer crossed at 300 Hz measures its stopband there: no anchor.
    let mut woofer = full.clone();
    woofer.crossover = vec![BandSpec {
        filter_type: 34,
        bypass: false,
        freq: 300.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }];
    assert!(live_anchor(&m, &flat_target, &woofer).is_none());
    // Nothing trusted in the band: no anchor.
    assert!(live_anchor(&flat_resp(-30.0, 5_000.0), &flat_target, &full).is_none());
}
