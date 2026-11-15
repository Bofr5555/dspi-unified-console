//! WP CG-SWS-a: `quantize_master_volume` and `curve_sections`, with parity
//! against the Mac's rules and against the core's own setter and curves.

use super::*;
use crate::DspiCore;
use dspi_proto::xover::Family;

/// The Mac's `keptMasterVolume` with a core attached
/// (`apps/apple/DSPi Console/Commands.swift:396-401`), written out.
fn swift_kept_master_volume(db: f32) -> f32 {
    if db <= -128.0 {
        return -128.0;
    }
    let val = ((db * 2.0).round() / 2.0).clamp(-127.0, 0.0);
    if val == 0.0 { 0.0 } else { val }
}

#[test]
fn cgswsa_quantize_master_volume_matches_the_swift_rule() {
    let mut db = -200.0_f32;
    while db < 10.0 {
        let got = quantize_master_volume(db);
        let want = swift_kept_master_volume(db);
        assert_eq!(got.to_bits(), want.to_bits(), "{db}");
        db += 0.01;
    }
    for (db, want) in [
        (-128.0, -128.0),
        (-127.9, -127.0),
        (-500.0, -128.0),
        (-0.2, 0.0),
        (-0.25, -0.5),
        (-3.26, -3.5),
        (-3.24, -3.0),
        (6.0, 0.0),
        (f32::NEG_INFINITY, -128.0),
        (f32::INFINITY, 0.0),
    ] {
        assert_eq!(quantize_master_volume(db), want, "{db}");
    }
    assert!(quantize_master_volume(-0.1).is_sign_positive());
    assert!(quantize_master_volume(f32::NAN).is_nan());
}

#[test]
fn cgswsa_quantize_master_volume_is_what_the_setter_keeps() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    for db in [-128.0, -127.6, -60.26, -12.74, -3.25, -0.2, 0.0, 3.0] {
        core.set_master_volume(db).unwrap();
        // == so that the setter's -0.0 equals the quantizer's +0.0.
        assert_eq!(
            core.get_state().master_volume_db,
            quantize_master_volume(db),
            "{db}"
        );
    }
}

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

/// `|H(e^jw)|` of a cascade, in dB, by complex arithmetic: the way a shader
/// evaluates the sections, independent of the core's `sin^2` form.
fn level(sections: &[BiquadSection], freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE;
    let mut power = 1.0;
    for s in sections {
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let nr = s.b0 + s.b1 * c1 + s.b2 * c2;
        let ni = -(s.b1 * s1 + s.b2 * s2);
        let dr = 1.0 + s.a1 * c1 + s.a2 * c2;
        let di = -(s.a1 * s1 + s.a2 * s2);
        power *= (nr * nr + ni * ni) / (dr * dr + di * di);
    }
    10.0 * power.max(1e-30).log10()
}

#[test]
fn cgswsa_curve_sections_give_the_magnitude_curve() {
    let lr4_low = (32u8..=63)
        .find(|t| {
            xover::meta(*t)
                .is_some_and(|m| m.family == Family::LinkwitzRiley && m.order == 4 && !m.high_pass)
        })
        .unwrap();
    let chain = vec![
        band(1, 1000.0, 1.4, 6.0),
        band(2, 120.0, 0.7, -3.0),
        band(lr4_low, 2500.0, 0.7, 0.0),
        FilterParams {
            bypass: true,
            ..band(1, 400.0, 2.0, 9.0)
        },
    ];
    let sections = curve_sections(chain.clone());
    // Peaking 1 + shelf 1 + LR4 (two Butterworth pairs) 2; the bypassed band none.
    assert_eq!(sections.len(), 4);
    let curve = compute_magnitude_curve(chain);
    for (i, want) in curve.iter().enumerate() {
        let got = level(&sections, curve_freq(i as u32));
        assert!((got - want).abs() < 1e-6, "point {i}: {got} vs {want}");
    }
}

/// RBJ peaking as the Mac's `DSPMath.calculateCoefficients` computes it
/// (`apps/apple/DSPi Console/DSPMath.swift:614-622`, `:633-635`), normalised.
#[test]
fn cgswsa_curve_sections_match_the_swift_peaking_coefficients() {
    let (freq, q, gain) = (1234.0_f32, 2.5_f32, -7.5_f32);
    let omega = 2.0 * std::f64::consts::PI * f64::from(freq) / 48000.0;
    let (sn, cs) = (omega.sin(), omega.cos());
    let alpha = sn / (2.0 * f64::from(q));
    let a = 10f64.powf(f64::from(gain) / 40.0);
    let a0 = 1.0 + alpha / a;
    let want = [
        (1.0 + alpha * a) / a0,
        (-2.0 * cs) / a0,
        (1.0 - alpha * a) / a0,
        (-2.0 * cs) / a0,
        (1.0 - alpha / a) / a0,
    ];
    let s = curve_sections(vec![band(1, freq, q, gain)]);
    assert_eq!(s.len(), 1);
    let got = [s[0].b0, s[0].b1, s[0].b2, s[0].a1, s[0].a2];
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-9, "{got:?} vs {want:?}");
    }
}

#[test]
fn cgswsa_curve_sections_skip_flat_and_bypassed_and_keep_bad_bands_unity() {
    assert!(curve_sections(vec![]).is_empty());
    assert!(curve_sections(vec![band(0, 1000.0, 1.0, 6.0)]).is_empty());
    let bypassed = FilterParams {
        bypass: true,
        ..band(1, 1000.0, 1.0, 6.0)
    };
    assert!(curve_sections(vec![bypassed]).is_empty());
    let unity = BiquadSection {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };
    assert_eq!(
        curve_sections(vec![band(1, f32::NAN, 1.0, 6.0)]),
        vec![unity]
    );
    // A type this build does not know is drawn flat, with one unity section.
    assert_eq!(curve_sections(vec![band(200, 1000.0, 1.0, 6.0)]).len(), 1);
}
