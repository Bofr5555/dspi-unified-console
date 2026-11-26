//! WP GAP1: `compute_group_delay_curve`, the web EQ graph's group delay
//! beside its phase: zero without bands, the analog prototypes' textbook
//! delays, and the slope of the core's own phase everywhere.

use super::*;

fn band(filter_type: u8, freq: f32, q: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type,
        bypass: false,
        freq,
        q,
        gain,
        qp: 0.0,
    }
}

/// `-d(phase)/d(omega)` in ms from the core's phase, by a central difference.
fn delay_from_phase(sections: &[Coeffs], hz: f64) -> f64 {
    let d = hz * 1e-5;
    let step = phase_deg(sections, hz + d) - phase_deg(sections, hz - d);
    // A wrap between the two points is a whole turn
    let step = (step + 540.0).rem_euclid(360.0) - 180.0;
    -step / 360.0 / (2.0 * d) * 1000.0
}

#[test]
fn gap1_no_bands_or_flat_bands_have_no_delay() {
    assert_eq!(compute_group_delay_curve(vec![]).len(), CURVE_POINTS);
    assert!(compute_group_delay_curve(vec![]).iter().all(|d| *d == 0.0));
    let flat = vec![band(1, 1000.0, 1.0, 0.0), band(0, 100.0, 0.7, 6.0)];
    assert!(
        compute_group_delay_curve(flat)
            .iter()
            .all(|d| d.abs() < 1e-12)
    );
    let mut off = band(34, 1000.0, 0.707, 0.0);
    off.bypass = true;
    assert!(
        compute_group_delay_curve(vec![off])
            .iter()
            .all(|d| *d == 0.0)
    );
}

#[test]
fn gap1_low_passes_delay_as_their_prototypes_at_dc() {
    // Butterworth 2nd order: sqrt(2)/w0; Linkwitz-Riley 4th (two of them):
    // twice that. At 10 Hz, far below a 1 kHz corner.
    let w0 = 2.0 * std::f64::consts::PI * 1000.0;
    for (ty, want) in [(42u8, 2f64.sqrt() / w0), (34, 2.0 * 2f64.sqrt() / w0)] {
        let got = compute_group_delay_curve(vec![band(ty, 1000.0, 0.707, 0.0)])[0];
        let want = want * 1000.0;
        assert!(
            (got - want).abs() < 0.01 * want,
            "type {ty}: {got} ms vs {want} ms"
        );
    }
}

#[test]
fn gap1_an_all_pass_delays_4q_over_w0_at_its_centre() {
    // 2nd-order all-pass: 4Q/w0 at w0 (the bilinear map adds sec^2(wT/2),
    // under half a percent at 1 kHz).
    let w0 = 2.0 * std::f64::consts::PI * 1000.0;
    for q in [0.5f32, 1.0, 2.0] {
        let s = cascade(&[band(7, 1000.0, q, 0.0)]);
        let got = delay_ms(&s, 1000.0);
        let want = 4.0 * f64::from(q) / w0 * 1000.0;
        assert!((got - want).abs() < 0.01 * want, "Q {q}: {got} vs {want}");
    }
}

#[test]
fn gap1_the_delay_is_the_slope_of_the_cores_phase() {
    let sets = [
        vec![band(1, 1000.0, 2.0, 9.0)],
        vec![band(2, 120.0, 0.707, -6.0), band(3, 6000.0, 0.9, 4.0)],
        vec![band(34, 2500.0, 0.707, 0.0), band(5, 40.0, 0.707, 0.0)],
        vec![band(35, 80.0, 0.707, 0.0), band(8, 300.0, 1.0, 0.0)],
        vec![band(56, 800.0, 0.707, 0.0)],
        vec![band(10, 2000.0, 0.707, 3.0), band(12, 15_000.0, 0.707, 0.0)],
    ];
    for filters in sets {
        let curve = compute_group_delay_curve(filters.clone());
        let s = cascade(&filters);
        for i in (0..CURVE_POINTS as u32).step_by(5) {
            let hz = curve_freq(i);
            let want = delay_from_phase(&s, hz);
            let got = curve[i as usize];
            assert!(
                (got - want).abs() < 1e-4 + 1e-3 * want.abs(),
                "{filters:?} at {hz} Hz: {got} vs {want}"
            );
        }
    }
}

#[test]
fn gap1_the_delay_never_panics_and_is_always_a_number() {
    for ty in 0..=63u8 {
        for (f, q, g) in [
            (0.0, 0.0, 0.0),
            (1.0, 0.01, 40.0),
            (23_999.0, 50.0, -40.0),
            (f32::NAN, f32::INFINITY, f32::NAN),
        ] {
            let c = compute_group_delay_curve(vec![band(ty, f, q, g)]);
            assert!(c.iter().all(|d| d.is_finite()), "type {ty} at {f}");
        }
    }
    // A notch's zero sits on the unit circle: no number there, so 0
    let s = cascade(&[band(6, 1000.0, 1.0, 0.0)]);
    assert!(delay_ms(&s, 1000.0).is_finite());
}
