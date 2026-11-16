//! Tests of WP P5CAL's targets.
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use crate::types::{Coupler, LogGrid};
use proptest::prelude::*;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../testdata/p5/cal/", $name))
    };
}

const EPS: f64 = 1e-9;

#[test]
fn wp_p5cal_flat_and_tilt_are_exact_at_their_defining_points() {
    let flat = Builtin::Flat.evaluate(&LogGrid::STANDARD).unwrap();
    assert!(flat.mag_db.iter().all(|m| *m == 0.0));
    let t = Builtin::Tilt {
        db_per_octave: -0.8,
    }
    .curve()
    .unwrap();
    assert!(t.value_at(TILT_PIVOT_HZ).abs() < EPS);
    assert!((t.value_at(2.0 * TILT_PIVOT_HZ) + 0.8).abs() < EPS);
    assert!((t.value_at(TILT_PIVOT_HZ / 4.0) - 1.6).abs() < EPS);
}

#[test]
fn wp_p5cal_house_curve_is_exact_at_its_defining_points() {
    let b = Builtin::House {
        shelf_gain_db: 6.0,
        shelf_corner_hz: 125.0,
        shelf_q: 0.707,
        tilt_db_per_octave: 0.0,
    };
    let c = b.curve().unwrap();
    // Half the shelf's gain at its corner; the full gain far below; 0 far above.
    assert!((c.value_at(125.0) - 3.0).abs() < EPS);
    assert!((c.value_at(0.01) - 6.0).abs() < 1e-6);
    assert!(c.value_at(1.0e6).abs() < 1e-6);
    // With a tilt the two add exactly.
    let tilted = Builtin::House {
        shelf_gain_db: 6.0,
        shelf_corner_hz: 125.0,
        shelf_q: 0.707,
        tilt_db_per_octave: -0.5,
    }
    .curve()
    .unwrap();
    let expect = 3.0 - 0.5 * (125.0f64 / TILT_PIVOT_HZ).log2();
    assert!((tilted.value_at(125.0) - expect).abs() < EPS);
}

#[test]
fn wp_p5cal_shapes_are_exact_at_their_defining_points() {
    for g in [-12.0, -3.0, 0.0, 4.5, 15.0] {
        for q in [0.5, 0.707, 2.0] {
            let p = Peak {
                gain_db: g,
                center_hz: 2800.0,
                q,
            };
            assert!((peak_db(p, 2800.0) - g).abs() < EPS, "peak {g} {q}");
            let s = Shelf {
                gain_db: g,
                corner_hz: 300.0,
                q: q.min(2.0),
            };
            assert!((low_shelf_db(s, 300.0) - g / 2.0).abs() < EPS);
            assert!((high_shelf_db(s, 300.0) - g / 2.0).abs() < EPS);
            assert!((high_shelf_db(s, 1.0e7) - g).abs() < 1e-4);
            assert!(high_shelf_db(s, 1.0e-3).abs() < 1e-4);
        }
    }
}

#[test]
fn wp_p5cal_coupler_defaults_are_ours_and_exact_at_their_parameters() {
    let all = builtins();
    let ids: Vec<&str> = all.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "flat",
            "tilt",
            "house",
            "coupler-iec60318-4",
            "coupler-flat-plate",
            "coupler-hats"
        ]
    );
    for info in all.iter().filter(|b| b.coupler.is_some()) {
        assert!(info.name.contains("(DSPi)"), "named as ours: {}", info.name);
        assert_eq!(info.target_use, TargetUse::Headphones);
    }
    for c in [Coupler::Iec60318_4, Coupler::FlatPlate, Coupler::Hats] {
        let curve = coupler_default(c);
        curve.validate().unwrap();
        // At a peak's centre the curve is that peak's gain plus the shelves'
        // values there, each computed on its own.
        for p in &curve.peaks {
            let others = curve
                .low_shelf
                .map_or(0.0, |s| low_shelf_db(s, p.center_hz))
                + curve
                    .high_shelf
                    .map_or(0.0, |s| high_shelf_db(s, p.center_hz));
            assert!((curve.value_at(p.center_hz) - (p.gain_db + others)).abs() < EPS);
        }
        let r = Builtin::CouplerDefault { coupler: c }
            .evaluate(&LogGrid::STANDARD)
            .unwrap();
        assert!(r.mag_db.iter().all(|m| m.is_finite()));
        assert_eq!(
            default_target(TargetUse::Headphones, Some(c)),
            Builtin::CouplerDefault { coupler: c }
        );
    }
    assert_eq!(
        coupler_default(Coupler::Hats),
        coupler_default(Coupler::Iec60318_4)
    );
    assert_eq!(default_target(TargetUse::Speakers, None), HOUSE_DEFAULT);
}

#[test]
fn wp_p5cal_normalisation_sets_the_reference_band_mean_to_zero() {
    let t = Target {
        target_use: TargetUse::Speakers,
        coupler: None,
        definition: TargetDefinition::Builtin {
            builtin: HOUSE_DEFAULT,
        },
        reference: DEFAULT_REFERENCE_BAND,
    };
    let r = t.evaluate(&LogGrid::STANDARD).unwrap();
    let f = LogGrid::STANDARD.freqs().unwrap();
    let band: Vec<f64> = f
        .iter()
        .zip(&r.mag_db)
        .filter(|(f, _)| (500.0..=2000.0).contains(*f))
        .map(|(_, m)| *m)
        .collect();
    let mean = band.iter().sum::<f64>() / band.len() as f64;
    assert!(mean.abs() < 1e-9);
    // No point in the band is refused.
    let mut r2 = r.clone();
    let e = normalise(
        &mut r2,
        ReferenceBand {
            low_hz: 30_000.0,
            high_hz: 40_000.0,
        },
    );
    assert!(matches!(e, Err(MeasureError::Invalid(_))));
    assert!(matches!(
        normalise(
            &mut r2,
            ReferenceBand {
                low_hz: 2000.0,
                high_hz: 500.0
            }
        ),
        Err(MeasureError::Invalid(_))
    ));
}

#[test]
fn wp_p5cal_rew_target_and_csv_compensation_import() {
    let t = import_target(
        fixture!("target_rew.txt"),
        TargetUse::Speakers,
        Some(Coupler::Hats),
    )
    .unwrap();
    assert_eq!(t.coupler, None, "a speaker target has no coupler");
    let TargetDefinition::Imported { points } = &t.definition else {
        panic!()
    };
    assert_eq!(points.len(), 5);
    let r = t.evaluate(&LogGrid::STANDARD).unwrap();
    // Imported 500 Hz is 1 dB, 2000 Hz -1 dB, linear in log f between: mean 0.
    let f = LogGrid::STANDARD.freqs().unwrap();
    let i = f
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - 1000.0).abs().total_cmp(&(b.1 - 1000.0).abs()))
        .unwrap()
        .0;
    assert!((f[i] / 1000.0).log2().abs() < 1.0 / 96.0);
    assert!(r.mag_db[i].abs() < 0.05, "{} at {}", r.mag_db[i], f[i]);
    assert!(r.phase_deg.is_none());

    let h = import_target(
        fixture!("csv_tab.txt"),
        TargetUse::Headphones,
        Some(Coupler::FlatPlate),
    )
    .unwrap();
    assert_eq!(h.coupler, Some(Coupler::FlatPlate));

    let c = import_compensation(fixture!("compensation.csv"), Coupler::FlatPlate).unwrap();
    assert_eq!(c.points.len(), 4);
    assert_eq!(c.points[2].db, 3.0);
    let cr = c.to_response(&LogGrid::STANDARD).unwrap();
    assert!(cr.mag_db.iter().all(|m| m.is_finite()));
    assert!(matches!(
        import_target("garbage\nmore", TargetUse::Speakers, None),
        Err(MeasureError::Parse(_))
    ));
    assert!(matches!(
        import_compensation("", Coupler::Hats),
        Err(MeasureError::Parse(_))
    ));
}

#[test]
fn wp_p5cal_out_of_range_parameters_are_refused() {
    for b in [
        Builtin::Tilt { db_per_octave: 5.0 },
        Builtin::Tilt {
            db_per_octave: f64::NAN,
        },
        Builtin::House {
            shelf_gain_db: 30.0,
            shelf_corner_hz: 100.0,
            shelf_q: 0.7,
            tilt_db_per_octave: 0.0,
        },
        Builtin::House {
            shelf_gain_db: 3.0,
            shelf_corner_hz: 0.0,
            shelf_q: 0.7,
            tilt_db_per_octave: 0.0,
        },
        Builtin::House {
            shelf_gain_db: 3.0,
            shelf_corner_hz: 100.0,
            shelf_q: 0.0,
            tilt_db_per_octave: 0.0,
        },
    ] {
        assert!(
            matches!(
                b.evaluate(&LogGrid::STANDARD),
                Err(MeasureError::Invalid(_))
            ),
            "{b:?}"
        );
    }
}

#[test]
fn wp_p5cal_builtin_round_trips_as_json() {
    for info in builtins() {
        let s = serde_json::to_string(&info).unwrap();
        assert_eq!(serde_json::from_str::<BuiltinInfo>(&s).unwrap(), info);
    }
}

proptest! {
    #[test]
    fn proptest_wp_p5cal_builtins_never_panic_and_stay_finite(
        g in any::<f64>(), f in any::<f64>(), q in any::<f64>(), t in any::<f64>(), ppo in 0u16..400,
        band_lo in any::<f64>(), band_hi in any::<f64>(),
    ) {
        let grid = LogGrid { min_hz: 10.0, max_hz: 22_000.0, points_per_octave: ppo };
        for b in [
            Builtin::Tilt { db_per_octave: t },
            Builtin::House { shelf_gain_db: g, shelf_corner_hz: f, shelf_q: q, tilt_db_per_octave: t },
        ] {
            if let Ok(mut r) = b.evaluate(&grid) {
                prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
                if normalise(&mut r, ReferenceBand { low_hz: band_lo, high_hz: band_hi }).is_ok() {
                    prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
                }
            }
        }
    }

    #[test]
    fn proptest_wp_p5cal_target_import_never_panics(s in ".{0,1500}") {
        if let Ok(t) = import_target(&s, TargetUse::Headphones, Some(Coupler::Hats))
            && let Ok(r) = t.evaluate(&LogGrid::STANDARD)
        {
            prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
        }
        if let Ok(c) = import_compensation(&s, Coupler::FlatPlate) {
            prop_assert!(c.to_response(&LogGrid::STANDARD).is_ok());
        }
    }
}
