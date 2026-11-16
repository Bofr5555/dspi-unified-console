//! Tests of WP P5CAL's position and reseat averaging.
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use crate::types::{Coupler, LogGrid, Placement, Response};
use proptest::prelude::*;

const G: LogGrid = LogGrid {
    min_hz: 20.0,
    max_hz: 20_000.0,
    points_per_octave: 3,
};

fn resp(mag: f64, phase: Option<f64>) -> Response {
    let n = G.freqs().unwrap().len();
    Response {
        grid: G,
        mag_db: vec![mag; n],
        phase_deg: phase.map(|p| vec![p; n]),
        valid: None,
        spl: true,
    }
}

#[test]
fn wp_p5cal_power_average_and_spread() {
    // 80 dB and 70 dB: power mean is 10 log10((1e8 + 1e7) / 2) = 77.40 dB.
    let a = average(
        &[
            resp(80.0, Some(10.0)),
            resp(70.0, Some(50.0)),
            resp(80.0, None),
        ],
        AverageKind::Positions,
    )
    .unwrap();
    let expect = 10.0 * ((2.0 * 1e8 + 1e7) / 3.0f64).log10();
    assert!(a.response.mag_db.iter().all(|m| (m - expect).abs() < 1e-9));
    assert!(a.spread_db.iter().all(|s| (s - 10.0).abs() < 1e-12));
    assert!(a.variable_mask().iter().all(|v| *v));
    // Position 1's phase is kept.
    assert!(
        a.response
            .phase_deg
            .as_ref()
            .unwrap()
            .iter()
            .all(|p| *p == 10.0)
    );
    assert!(a.response.spl);
    assert_eq!(a.count, 3);
    // Identical runs: the same level, no spread.
    let same = average(&vec![resp(-3.0, None); 5], AverageKind::Reseats).unwrap();
    assert!(same.response.mag_db.iter().all(|m| (m + 3.0).abs() < 1e-12));
    assert!(same.spread_db.iter().all(|s| *s == 0.0));
    assert!(same.response.phase_deg.is_none());
}

#[test]
fn wp_p5cal_validity_is_the_and_of_the_runs() {
    let mut a = resp(0.0, None);
    let n = a.mag_db.len();
    let mut v = vec![true; n];
    v[2] = false;
    a.valid = Some(v);
    let out = average(
        &[a, resp(0.0, None), resp(0.0, None)],
        AverageKind::Positions,
    )
    .unwrap();
    let valid = out.response.valid.unwrap();
    assert!(!valid[2]);
    assert_eq!(valid.iter().filter(|x| **x).count(), n - 1);
}

#[test]
fn wp_p5cal_placements_are_checked() {
    assert!(validate_placement(&Placement::MultiPosition { count: 2 }).is_err());
    assert!(validate_placement(&Placement::MultiPosition { count: 10 }).is_err());
    assert!(validate_placement(&Placement::MultiPosition { count: 3 }).is_ok());
    assert!(
        validate_placement(&Placement::Headphones {
            coupler: Coupler::Hats,
            reseats: 0
        })
        .is_err()
    );
    assert!(
        validate_placement(&Placement::Headphones {
            coupler: Coupler::Hats,
            reseats: 9
        })
        .is_ok()
    );
    assert_eq!(runs_for(&Placement::NearField), 1);
    let hp = Placement::Headphones {
        coupler: Coupler::Iec60318_4,
        reseats: 3,
    };
    let left = average_for(
        &hp,
        &[resp(90.0, Some(1.0)), resp(91.0, None), resp(89.0, None)],
    )
    .unwrap();
    assert!(left.response.phase_deg.is_none(), "reseats keep no phase");
    assert!(average_for(&hp, &[resp(90.0, None)]).is_err());
    let lp = average_for(&Placement::ListeningPosition, &[resp(70.0, Some(5.0))]).unwrap();
    assert_eq!(lp.response.mag_db, resp(70.0, None).mag_db);
}

#[test]
fn wp_p5cal_bad_inputs_are_typed_errors() {
    assert!(matches!(
        average(&[], AverageKind::Positions),
        Err(MeasureError::Invalid(_))
    ));
    assert!(average(&vec![resp(0.0, None); 10], AverageKind::Positions).is_err());
    let mut other = resp(0.0, None);
    other.grid = LogGrid::STANDARD;
    assert!(average(&[resp(0.0, None), other], AverageKind::Positions).is_err());
    let mut short = resp(0.0, None);
    short.mag_db.pop();
    assert!(average(&[short], AverageKind::Positions).is_err());
    let mut nan = resp(0.0, None);
    nan.mag_db[0] = f64::NAN;
    assert!(average(&[nan], AverageKind::Positions).is_err());
}

proptest! {
    #[test]
    fn proptest_wp_p5cal_average_never_panics_and_lies_between_min_and_max(
        mags in proptest::collection::vec(proptest::collection::vec(any::<f64>(), 0..12), 0..11),
        kind in prop_oneof![Just(AverageKind::Positions), Just(AverageKind::Reseats)],
    ) {
        let n = G.freqs().unwrap().len();
        let runs: Vec<Response> = mags.iter().map(|m| {
            let mut r = resp(0.0, None);
            for (i, v) in m.iter().enumerate() { if i < n { r.mag_db[i] = *v; } }
            r
        }).collect();
        if let Ok(a) = average(&runs, kind) {
            for i in 0..n {
                let lo = runs.iter().map(|r| r.mag_db[i]).fold(f64::INFINITY, f64::min);
                let hi = runs.iter().map(|r| r.mag_db[i]).fold(f64::NEG_INFINITY, f64::max);
                let m = a.response.mag_db[i];
                prop_assert!(m.is_finite() && m >= lo - 1e-9 && m <= hi + 1e-9);
                prop_assert!(a.spread_db[i].is_finite() && a.spread_db[i] >= 0.0);
            }
        }
    }
}
