//! Tests of WP P5CAL's calibration parser (fixtures written by hand in
//! `testdata/p5/cal/`, never a vendor's file).
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use crate::types::LogGrid;
use proptest::prelude::*;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../testdata/p5/cal/", $name))
    };
}

fn pts(c: &ParsedCurve) -> Vec<(f64, f64, Option<f64>)> {
    c.points
        .iter()
        .map(|p| (p.freq_hz, p.db, p.phase_deg))
        .collect()
}

fn is_parse_err(r: Result<MicCalibration, MeasureError>) -> bool {
    matches!(r, Err(MeasureError::Parse(_)))
}

#[test]
fn wp_p5cal_umik1_fixture_parses_with_sensitivity_and_range() {
    let c = parse_calibration(fixture!("umik1_7000001.txt"), "umik1_7000001.txt").unwrap();
    assert_eq!(c.curve.format, CalFormat::Umik);
    assert_eq!(c.vendor.as_deref(), Some("miniDSP"));
    assert_eq!(c.curve.sensitivity_db, Some(-1.378));
    assert_eq!(c.curve.gain_db, None);
    assert_eq!(c.curve.serial.as_deref(), Some("7000001"));
    assert_eq!(c.curve.angle, MicAngle::Deg0);
    assert!(!c.curve.has_phase);
    // 5 Hz and 24 kHz are outside 10 Hz..22 kHz.
    assert_eq!(c.curve.ignored_out_of_range, 2);
    assert_eq!(
        pts(&c.curve),
        vec![
            (10.0, -2.5, None),
            (20.0, -1.2, None),
            (100.0, -0.3, None),
            (1000.0, 0.0, None),
            (10000.0, 0.8, None),
            (20000.0, -1.5, None),
        ]
    );
    let s = c.spl_scale();
    assert!(s.approximate);
    assert_eq!(s.basis, SplBasis::Sensitivity);
    assert!((s.offset_db.unwrap() - (94.0 + 1.378)).abs() < 1e-12);
}

#[test]
fn wp_p5cal_umik2_90deg_file_is_recognised_with_again() {
    let c = parse_calibration(
        fixture!("umik2_8000002_90deg.txt"),
        "umik2_8000002_90deg.txt",
    )
    .unwrap();
    assert_eq!(c.curve.format, CalFormat::Umik);
    assert_eq!(c.curve.angle, MicAngle::Deg90);
    assert_eq!(c.curve.angle.degrees(), 90);
    assert_eq!(c.curve.sensitivity_db, Some(-12.42));
    assert_eq!(c.curve.gain_db, Some(18.0));
    assert_eq!(c.curve.serial.as_deref(), Some("8000002"));
    assert_eq!(c.curve.points.len(), 5);
    // The same text under a 0° name is 0°; a comment saying 90deg is 90°.
    let c0 = parse_calibration(fixture!("umik2_8000002_90deg.txt"), "umik2_8000002.txt").unwrap();
    assert_eq!(c0.curve.angle, MicAngle::Deg0);
    let cc = parse_calibration(fixture!("umik_90_comment.txt"), "mic.txt").unwrap();
    assert_eq!(cc.curve.angle, MicAngle::Deg90);
    // Not fooled by 190deg or a serial containing 90.
    assert!(!mentions_90("7090123.txt"));
    assert!(!mentions_90("x_190deg.txt"));
    assert!(mentions_90("7012345_90deg.txt"));
    assert!(mentions_90("UMIK 90 Degree.txt"));
    assert!(mentions_90("mic 90°.txt"));
}

#[test]
fn wp_p5cal_rew_fixture_keeps_phase_sorts_and_deduplicates() {
    let c = parse_calibration(fixture!("rew_phase.txt"), "rew_phase.txt").unwrap();
    assert_eq!(c.curve.format, CalFormat::Rew);
    assert_eq!(c.vendor, None);
    assert!(c.curve.has_phase);
    assert_eq!(c.curve.duplicates_dropped, 1);
    assert_eq!(
        pts(&c.curve),
        vec![
            (20.0, -0.5, Some(10.0)),
            (200.0, 0.0, Some(2.0)),
            (1000.0, 0.1, Some(-1.0)),
            (20000.0, 1.25, Some(-40.0)),
        ]
    );
    let r = c.to_response(&LogGrid::STANDARD).unwrap();
    assert!(r.phase_deg.is_some());
}

#[test]
fn wp_p5cal_rew_sensitivity_line_is_read() {
    let c = parse_calibration(fixture!("rew_sensitivity.txt"), "a.txt").unwrap();
    assert_eq!(c.curve.format, CalFormat::Rew);
    assert_eq!(c.curve.sensitivity_db, Some(-12.34));
    assert!((c.spl_scale().offset_db.unwrap() - 106.34).abs() < 1e-9);
    assert_eq!(c.curve.points.len(), 3);
}

#[test]
fn wp_p5cal_csv_fixtures_parse_comma_semicolon_and_tab() {
    let c = parse_calibration(fixture!("csv_comma.csv"), "c.csv").unwrap();
    assert_eq!(c.curve.format, CalFormat::Csv);
    assert_eq!(
        pts(&c.curve),
        vec![
            (20.0, -0.4, None),
            (1000.0, 0.0, None),
            (5000.0, 0.6, None),
            (20000.0, -2.0, None)
        ]
    );
    let s = parse_calibration(fixture!("csv_semicolon.csv"), "s.csv").unwrap();
    assert_eq!(s.curve.format, CalFormat::Csv);
    assert_eq!(
        pts(&s.curve),
        vec![
            (20.0, -0.25, None),
            (1000.0, 0.0, None),
            (10000.0, 1.5, None)
        ]
    );
    let t = parse_calibration(fixture!("csv_tab.txt"), "t.txt").unwrap();
    assert_eq!(t.curve.format, CalFormat::Csv);
    assert!(t.curve.has_phase);
    assert_eq!(t.curve.points.len(), 3);
    assert_eq!(s.curve.sensitivity_db, None);
    assert_eq!(s.spl_scale().basis, SplBasis::Relative);
}

#[test]
fn wp_p5cal_to_grid_interpolates_in_log_frequency_and_marks_outside() {
    let c = parse_calibration(fixture!("umik1_7000001.txt"), "u.txt").unwrap();
    let g = LogGrid {
        min_hz: 10.0,
        max_hz: 22_000.0,
        points_per_octave: 3,
    };
    let r = c.to_response(&g).unwrap();
    let f = g.freqs().unwrap();
    for (i, &fi) in f.iter().enumerate() {
        let v = r.valid.as_ref().unwrap()[i];
        assert_eq!(v, (10.0..=20000.0).contains(&fi), "{fi}");
    }
    // Exactly at a point: its value. 1000 Hz is not on this grid; 10 Hz is.
    assert!((r.mag_db[0] + 2.5).abs() < 1e-12);
    // Half way in log f between 100 Hz (-0.3) and 1000 Hz (0.0).
    let pts = [
        CurvePoint {
            freq_hz: 100.0,
            db: -0.3,
            phase_deg: None,
        },
        CurvePoint {
            freq_hz: 1000.0,
            db: 0.0,
            phase_deg: None,
        },
    ];
    let (m, _, inside) = interpolate(&pts, (100f64 * 1000.0).sqrt());
    assert!(inside);
    assert!((m + 0.15).abs() < 1e-12);
    // Above the last point the end value is held, not valid.
    assert_eq!(*r.mag_db.last().unwrap(), -1.5);
}

#[test]
fn wp_p5cal_apply_correction_subtracts_the_microphone() {
    let g = LogGrid {
        min_hz: 20.0,
        max_hz: 20_000.0,
        points_per_octave: 1,
    };
    let n = g.freqs().unwrap().len();
    let measured = Response {
        grid: g,
        mag_db: vec![80.0; n],
        phase_deg: Some(vec![10.0; n]),
        valid: None,
        spl: true,
    };
    let cal = curve_to_grid(
        &[
            CurvePoint {
                freq_hz: 20.0,
                db: 2.0,
                phase_deg: Some(4.0),
            },
            CurvePoint {
                freq_hz: 20000.0,
                db: 2.0,
                phase_deg: Some(4.0),
            },
        ],
        &g,
    )
    .unwrap();
    let out = apply_correction(&measured, &cal).unwrap();
    assert!(out.mag_db.iter().all(|m| (*m - 78.0).abs() < 1e-12));
    assert!(
        out.phase_deg
            .unwrap()
            .iter()
            .all(|p| (*p - 6.0).abs() < 1e-12)
    );
    assert!(out.spl);
    let other = Response {
        grid: LogGrid::STANDARD,
        ..measured.clone()
    };
    assert!(matches!(
        apply_correction(&other, &cal),
        Err(MeasureError::Invalid(_))
    ));
}

#[test]
fn wp_p5cal_garbage_empty_long_binary_and_non_utf8_are_typed_errors() {
    // Garbage: text that is not numbers after the one allowed header line.
    assert!(is_parse_err(parse_calibration("hello\nworld\n", "g.txt")));
    assert!(is_parse_err(parse_calibration(
        "20 0\nthis is not data\n",
        "g.txt"
    )));
    assert!(is_parse_err(parse_calibration("20 abc\n", "g.txt")));
    assert!(is_parse_err(parse_calibration("20 0 phase?\n", "g.txt")));
    // Empty and comment-only.
    assert!(is_parse_err(parse_calibration("", "e.txt")));
    assert!(is_parse_err(parse_calibration(
        "* only a comment\n\n",
        "e.txt"
    )));
    // Everything out of range.
    assert!(is_parse_err(parse_calibration("5 0\n30000 0\n", "e.txt")));
    // 10 000 lines.
    let long: String = (0..10_000).map(|i| format!("{} 0\n", 20 + i)).collect();
    let e = parse_calibration(&long, "l.txt").unwrap_err();
    assert!(
        matches!(&e, MeasureError::Parse(s) if s.contains("8192")),
        "{e:?}"
    );
    // Over 1 MiB.
    let big = "*".repeat(MAX_FILE_BYTES + 1);
    assert!(is_parse_err(parse_calibration(&big, "b.txt")));
    assert!(is_parse_err(parse_calibration_bytes(
        big.as_bytes(),
        "b.txt"
    )));
    // Binary: NUL bytes, other control bytes, UTF-16.
    assert!(is_parse_err(parse_calibration_bytes(
        b"20 0\n\x00\x01\x02\x03",
        "x.bin"
    )));
    assert!(is_parse_err(parse_calibration_bytes(
        b"\x7fELF\x02\x01\x01",
        "x.bin"
    )));
    let utf16: Vec<u8> = "20 0\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert!(is_parse_err(parse_calibration_bytes(&utf16, "w.txt")));
    assert!(is_parse_err(parse_calibration("20 0\n\u{1}\n", "x.txt")));
    // Non-UTF-8 text (Latin-1 degree sign).
    let latin1 = b"* 90\xb0 file\n20 0\n1000 0\n";
    let e = parse_calibration_bytes(latin1, "l1.txt").unwrap_err();
    assert!(
        matches!(&e, MeasureError::Parse(s) if s.contains("UTF-8")),
        "{e:?}"
    );
    // The same as UTF-8 bytes with a BOM parses.
    let ok = "\u{feff}* 90° file\r\n20 0\r\n1000 0\r\n";
    let c = parse_calibration_bytes(ok.as_bytes(), "ok.txt").unwrap();
    assert_eq!(c.curve.angle, MicAngle::Deg90);
    assert_eq!(c.curve.points.len(), 2);
}

#[test]
fn wp_p5cal_non_finite_values_are_refused_with_the_line_number() {
    for (text, line) in [
        ("20 0\n1000 NaN\n", 2),
        ("* c\n\n20 inf\n", 3),
        ("20 0 -inf\n", 1),
        ("\"Sens Factor =nan dB, SERNO: 1\"\n20 0\n", 1),
        ("20 1e999\n", 1),
    ] {
        let e = parse_calibration(text, "n.txt").unwrap_err();
        match e {
            MeasureError::Parse(s) => {
                assert!(s.contains(&format!("line {line}:")), "{text:?}: {s}");
                assert!(!s.contains("NaN") && !s.contains("inf "), "{s}");
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn wp_p5cal_mixed_phase_columns_drop_phase() {
    let c = parse_calibration("20 0 1\n1000 0\n", "m.txt").unwrap();
    assert!(!c.curve.has_phase);
    assert!(c.curve.points.iter().all(|p| p.phase_deg.is_none()));
}

fn tone(rate: u32, freq: f64, amp: f64, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin()) as f32
        })
        .collect()
}

#[test]
fn wp_p5cal_calibrator_tone_gives_the_offset() {
    // A sine of peak 0.1 is -20 dBFS (full-scale sine = 0 dBFS).
    let x = tone(48_000, 1000.0, 0.1, 48_000);
    assert!((level_dbfs(&x).unwrap() + 20.0).abs() < 0.01);
    let off = spl_offset_from_tone(&x, 48_000, 94.0).unwrap();
    assert!((off - 114.0).abs() < 0.05, "{off}");
    // A calibrator 1.5 % off frequency, at 114 dB, with a little noise.
    let mut y = tone(44_100, 1015.0, 0.5, 30_000);
    for (i, s) in y.iter_mut().enumerate() {
        *s += 0.001 * (((i * 7919) % 1000) as f32 / 500.0 - 1.0);
    }
    let off = spl_offset_from_tone(&y, 44_100, 114.0).unwrap();
    assert!((off - (114.0 + 6.0206)).abs() < 0.05, "{off}");
    let scale = spl_scale(
        Some(-1.0),
        Some(CalibratorStep {
            offset_db: off,
            reference_db: 114.0,
        }),
    );
    assert!(!scale.approximate);
    assert_eq!(
        scale.basis,
        SplBasis::Calibrator {
            reference_db: 114.0
        }
    );
    assert!((scale.to_spl(-6.0206).unwrap() - 114.0).abs() < 0.05);
}

#[test]
fn wp_p5cal_calibrator_refusals_are_typed() {
    let x = tone(48_000, 1000.0, 0.1, 48_000);
    assert!(matches!(
        spl_offset_from_tone(&x, 48_000, 100.0),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        spl_offset_from_tone(&x, 0, 94.0),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        spl_offset_from_tone(&x[..100], 48_000, 94.0),
        Err(MeasureError::NoSignal(_))
    ));
    assert!(matches!(
        spl_offset_from_tone(&[0.0; 8000], 48_000, 94.0),
        Err(MeasureError::NoSignal(_))
    ));
    let wrong = tone(48_000, 250.0, 0.1, 48_000);
    assert!(matches!(
        spl_offset_from_tone(&wrong, 48_000, 94.0),
        Err(MeasureError::NoSignal(_))
    ));
    let clipped = tone(48_000, 1000.0, 1.5, 48_000);
    assert!(matches!(
        spl_offset_from_tone(&clipped, 48_000, 94.0),
        Err(MeasureError::NoSignal(_))
    ));
    let mut nan = x.clone();
    nan[10] = f32::NAN;
    assert!(matches!(
        spl_offset_from_tone(&nan, 48_000, 94.0),
        Err(MeasureError::Invalid(_))
    ));
    // A relative scale when there is neither; a bad calibrator step is ignored.
    let s = spl_scale(
        None,
        Some(CalibratorStep {
            offset_db: f64::NAN,
            reference_db: 94.0,
        }),
    );
    assert_eq!(s.basis, SplBasis::Relative);
    assert_eq!(s.to_spl(-20.0), None);
}

proptest! {
    #[test]
    fn proptest_wp_p5cal_parse_curve_never_panics_on_any_text(s in ".{0,2000}") {
        if let Ok(c) = parse_curve(&s, "p.txt") {
            prop_assert!(!c.points.is_empty());
            prop_assert!(c.points.windows(2).all(|w| w[0].freq_hz < w[1].freq_hz));
            prop_assert!(c.points.iter().all(|p| p.db.is_finite()
                && (CAL_MIN_HZ..=CAL_MAX_HZ).contains(&p.freq_hz)));
        }
    }

    #[test]
    fn proptest_wp_p5cal_parse_bytes_never_panics(b in proptest::collection::vec(any::<u8>(), 0..3000)) {
        let _ = parse_calibration_bytes(&b, "b");
    }

    #[test]
    fn proptest_wp_p5cal_numeric_lines_parse_finite_and_to_grid(
        rows in proptest::collection::vec((any::<f64>(), any::<f64>(), proptest::option::of(any::<f64>())), 0..200),
        sep in prop_oneof![Just(" "), Just("\t"), Just(","), Just(";")],
        header in any::<bool>(),
        sens in proptest::option::of(-200.0f64..200.0),
    ) {
        let mut text = String::new();
        if let Some(s) = sens { text.push_str(&format!("\"Sens Factor ={s}dB, AGain =18dB, SERNO: 1\"\n")); }
        if header { text.push_str(&format!("Freq{sep}dB\n")); }
        for (f, d, p) in &rows {
            match p {
                Some(p) => text.push_str(&format!("{f}{sep}{d}{sep}{p}\n")),
                None => text.push_str(&format!("{f}{sep}{d}\n")),
            }
        }
        match parse_curve(&text, "n") {
            Ok(c) => {
                let r = curve_to_grid(&c.points, &LogGrid::STANDARD).unwrap();
                prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
                if let Some(p) = &r.phase_deg { prop_assert!(p.iter().all(|v| v.is_finite())); }
                if let Some(s) = sens { prop_assert_eq!(c.sensitivity_db, Some(s)); }
            }
            Err(e) => prop_assert!(matches!(e, MeasureError::Parse(_))),
        }
    }

    #[test]
    fn proptest_wp_p5cal_curve_to_grid_and_correction_never_panic(
        pts in proptest::collection::vec((any::<f64>(), any::<f64>()), 0..50),
        ppo in 0u16..400,
    ) {
        let points: Vec<CurvePoint> = pts.iter().map(|(f, d)| CurvePoint { freq_hz: *f, db: *d, phase_deg: None }).collect();
        let g = LogGrid { min_hz: 10.0, max_hz: 22_000.0, points_per_octave: ppo };
        if let Ok(r) = curve_to_grid(&points, &g) {
            prop_assert!(r.mag_db.iter().all(|v| v.is_finite()));
            if let Ok(c) = apply_correction(&r, &r) {
                prop_assert!(c.mag_db.iter().all(|v| v.is_finite() && v.abs() < 1e-9));
            }
        }
    }

    #[test]
    fn proptest_wp_p5cal_tone_and_scale_never_panic(
        samples in proptest::collection::vec(any::<f32>(), 0..6000),
        rate in any::<u32>(),
        reference in prop_oneof![Just(94.0), Just(114.0), any::<f64>()],
        sens in proptest::option::of(any::<f64>()),
    ) {
        if let Ok(off) = spl_offset_from_tone(&samples, rate, reference) {
            prop_assert!(off.is_finite());
        }
        let s = spl_scale(sens, None);
        if let Some(o) = s.offset_db { prop_assert!(o.is_finite()); }
        if let Some(l) = level_dbfs(&samples) { prop_assert!(l.is_finite()); }
    }
}
