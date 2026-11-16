//! WP P5MDOC: the measurement, mic and target documents.

use super::*;
use crate::types::{LogGrid, MAX_GRID_POINTS, ProgrammeFeed, Response};
use proptest::prelude::*;
use serde_json::json;

fn n_std() -> usize {
    LogGrid::STANDARD.freqs().unwrap().len()
}

/// A standard-grid curve with phase, mask and (optionally) spread.
fn curve(seed: f64, phase: bool, spread: bool) -> Curve {
    let f = LogGrid::STANDARD.freqs().unwrap();
    Curve {
        grid: LogGrid::STANDARD,
        mag_db: f
            .iter()
            .map(|x| 75.0 + 6.0 * (x.ln() * seed).sin() - 0.123_456)
            .collect(),
        phase_deg: phase.then(|| f.iter().map(|x| -360.0 * x * 0.003_217 + seed).collect()),
        valid: Some(f.iter().map(|x| *x > 25.0 && *x < 18_000.0).collect()),
        spread_db: spread.then(|| {
            f.iter()
                .map(|x| (x.ln() * 0.37).sin().abs() * 4.0)
                .collect()
        }),
        spl: true,
        extra: Map::new(),
    }
}

fn band(t: u8, freq: f32) -> BandSpec {
    BandSpec {
        filter_type: t,
        bypass: false,
        freq,
        q: 0.707,
        gain: -3.5,
        qp: 0.0,
    }
}

fn chain(k: usize) -> ChannelChain {
    ChannelChain {
        peq: (0..12)
            .map(|i| band(1 + (i % 7) as u8, 100.0 * (i + 1) as f32))
            .collect(),
        crossover: (0..8)
            .map(|i| band(32 + i as u8, 300.0 * (k + 1) as f32))
            .collect(),
        gain_db: -2.5,
        delay_ms: 0.21 * k as f32,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: k % 2 == 1,
        },
    }
}

fn output(o: u8, role: WayRole, speaker: &str, spread: bool) -> OutputMeasurement {
    OutputMeasurement {
        output: o,
        role,
        speaker: speaker.into(),
        chain: chain(usize::from(o)).into(),
        arrival_ms: Some(3.1 + f64::from(o) * 0.01),
        response: curve(1.0 + f64::from(o), true, spread),
        extra: Map::new(),
    }
}

fn measurement(outputs: usize, spread: bool) -> MeasurementDoc {
    let roles = [WayRole::Low, WayRole::Mid, WayRole::High];
    MeasurementDoc {
        schema_version: 1,
        meta: Meta::new(
            DocKind::Measurement,
            "Living room, 3-way",
            "2026-11-16T10:00:00Z",
            "0.5.0",
        ),
        measurement: Measurement {
            id: "1763287200000".into(),
            serial: "E66164084B3C2E2B".into(),
            platform: "RP2350".into(),
            saved_ms: 1_763_287_200_000,
            placement: if spread {
                Placement::MultiPosition { count: 5 }
            } else {
                Placement::ListeningPosition
            },
            mic: Some(MicRef {
                id: "mic-1".into(),
                name: "UMIK-1".into(),
                serial: "700-1234".into(),
                sensitivity_db: Some(-18.3),
                approximate: true,
                cal_hash: "ab12".into(),
                extra: Map::new(),
            }),
            stimulus: Stimulus {
                kind: RunKind::Walked,
                f1_hz: 20.0,
                f2_hz: 20_000.0,
                duration_ms: 5_500,
                repeat: 3,
                gap_ms: 1_000,
                level_db: -24.0,
                device_rate_hz: 48_000,
                extra: Map::new(),
            },
            capture: CaptureInfo {
                rate_hz: 48_000,
                clock_ratio: 1.000_08,
                timing_reliable: true,
                max_residual_us: Some(3.2),
                extra: Map::new(),
            },
            outputs: (0..outputs)
                .map(|i| {
                    let speaker = if i < 3 { "L" } else { "R" };
                    output(i as u8, roles[i % 3], speaker, spread)
                })
                .collect(),
            positions: if spread { 5 } else { 1 },
            note: "Before the new rug".into(),
            tunes: vec![TuneRecord {
                at: 1_763_287_300_000,
                snapshot_before: "1763287290000".into(),
                snapshot_after: "1763287390000".into(),
                kept: true,
                extra: Map::new(),
            }],
            extra: Map::new(),
        },
        extra: Map::new(),
    }
}

fn mic() -> MicDoc {
    let mut c = curve(0.5, false, false);
    c.spl = false;
    c.valid = None;
    MicDoc {
        schema_version: 1,
        meta: Meta::new(
            DocKind::Mic,
            "UMIK-1 700-1234",
            "2026-11-16T10:00:00Z",
            "0.5.0",
        ),
        mic: MicCal {
            id: "mic-1".into(),
            name: "UMIK-1".into(),
            serial: "700-1234".into(),
            vendor: "miniDSP".into(),
            format: MicFormat::Umik,
            angle: 90,
            sensitivity_db: Some(-18.3),
            gain_db: Some(18.0),
            spl_offset_db: None,
            approximate: true,
            response: c,
            source_name: "700-1234_90deg.txt".into(),
            source_hash: "ab12".into(),
            note: String::new(),
            extra: Map::new(),
        },
        extra: Map::new(),
    }
}

fn target(imported: bool) -> TargetDoc {
    let definition = if imported {
        let mut c = curve(0.2, false, false);
        c.valid = None;
        c.spl = false;
        TargetDefinition::Imported(ImportedTarget {
            curve: c,
            source_name: "my-target.csv".into(),
            extra: Map::new(),
        })
    } else {
        TargetDefinition::Builtin(BuiltinTarget {
            builtin: "house".into(),
            params: [
                ("shelfGainDb".to_string(), 4.0),
                ("tiltDbPerOctave".into(), -0.8),
            ]
            .into_iter()
            .collect(),
            extra: Map::new(),
        })
    };
    TargetDoc {
        schema_version: 1,
        meta: Meta::new(
            DocKind::Target,
            "House curve",
            "2026-11-16T10:00:00Z",
            "0.5.0",
        ),
        target: Target {
            id: String::new(),
            name: "House curve".into(),
            use_for: TargetUse::Headphones,
            coupler: Some(Coupler::Iec60318_4),
            definition,
            compensation: imported.then(|| {
                let mut c = curve(0.9, false, false);
                c.valid = None;
                c.spl = false;
                c
            }),
            reference_band: ReferenceBand {
                low_hz: 500.0,
                high_hz: 2_000.0,
                extra: Map::new(),
            },
            note: "From the review".into(),
            extra: Map::new(),
        },
        extra: Map::new(),
    }
}

fn round_measurement(mut d: MeasurementDoc) -> MeasurementDoc {
    for o in &mut d.measurement.outputs {
        o.response = o.response.rounded();
    }
    d
}

fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

// ── round trips ───────────────────────────────────────────────────────────

#[test]
fn wp_p5mdoc_measurement_round_trips_within_rounding() {
    let d = measurement(6, true);
    let text = d.to_text().unwrap();
    let back = MeasurementDoc::from_text(&text).unwrap();
    assert_eq!(back, round_measurement(d.clone()));
    for (a, b) in d.measurement.outputs.iter().zip(&back.measurement.outputs) {
        assert!(close(&a.response.mag_db, &b.response.mag_db, 0.005 + 1e-9));
        assert!(close(
            a.response.phase_deg.as_ref().unwrap(),
            b.response.phase_deg.as_ref().unwrap(),
            0.05 + 1e-9
        ));
        assert!(close(
            a.response.spread_db.as_ref().unwrap(),
            b.response.spread_db.as_ref().unwrap(),
            0.005 + 1e-9
        ));
        assert_eq!(a.response.valid, b.response.valid);
    }
    // Written again, the text is the same: rounding is idempotent.
    assert_eq!(back.to_text().unwrap(), text);
}

#[test]
fn wp_p5mdoc_mic_and_targets_round_trip() {
    let m = mic();
    let back = MicDoc::from_text(&m.to_text().unwrap()).unwrap();
    let mut want = m.clone();
    want.mic.response = want.mic.response.rounded();
    assert_eq!(back, want);

    for imported in [false, true] {
        let t = target(imported);
        let back = TargetDoc::from_text(&t.to_text().unwrap()).unwrap();
        let mut want = t.clone();
        if let TargetDefinition::Imported(i) = &mut want.target.definition {
            i.curve = i.curve.rounded();
        }
        want.target.compensation = want.target.compensation.map(|c| c.rounded());
        assert_eq!(back, want, "imported {imported}");
    }
}

#[test]
fn wp_p5mdoc_any_kind_reader_dispatches() {
    let texts = [
        (
            measurement(1, false).to_text().unwrap(),
            DocKind::Measurement,
        ),
        (mic().to_text().unwrap(), DocKind::Mic),
        (target(false).to_text().unwrap(), DocKind::Target),
    ];
    for (t, k) in &texts {
        assert_eq!(doc_kind(t), Some(*k));
        let d = MeasureDocument::from_text(t).unwrap();
        assert_eq!(d.kind(), *k);
        assert_eq!(&d.to_text().unwrap(), t);
    }
}

#[test]
fn wp_p5mdoc_response_converts_both_ways() {
    let r = Response {
        grid: LogGrid::STANDARD,
        mag_db: vec![1.234_56; n_std()],
        phase_deg: Some(vec![-12.345; n_std()]),
        valid: Some(vec![true; n_std()]),
        spl: false,
    };
    let c = Curve::from_response(&r);
    assert_eq!(c.to_response(), r);
}

// ── the size bound of RC7 ─────────────────────────────────────────────────

#[test]
fn wp_p5mdoc_three_way_stereo_with_phase_is_under_100_kib() {
    // Six outputs, each with phase, the trusted mask and a position spread,
    // full chains (12 PEQ and 8 crossover bands), pretty-printed as filed.
    let text = measurement(6, true).to_text().unwrap();
    assert!(text.len() < 100 * 1024, "{} bytes", text.len());
    let compact = serde_json::to_string(&measurement(6, true)).unwrap();
    // One output's curve with phase and mask is about 9 KB (PLAN-P5 1.6).
    let one = serde_json::to_string(&curve(1.0, true, false)).unwrap();
    assert!((8_000..10_500).contains(&one.len()), "{}", one.len());
    eprintln!(
        "wp_p5mdoc: 3-way stereo {} bytes pretty, {} compact; one curve {}",
        text.len(),
        compact.len(),
        one.len()
    );
}

#[test]
fn wp_p5mdoc_sixteen_outputs_fit_the_measurement_bound() {
    let mut d = measurement(6, true);
    d.measurement.outputs = (0..16)
        .map(|o| output(o, WayRole::Full, "", true))
        .collect();
    let text = d.to_text().unwrap();
    assert!(text.len() < MAX_MEASUREMENT_BYTES / 2, "{}", text.len());
}

// ── each bound refuses one past it ────────────────────────────────────────

/// `doc`'s text padded with an unknown top-level key to exactly `len` bytes.
fn padded(doc: &Value, len: usize) -> String {
    let mut v = doc.clone();
    v["zzPad"] = json!("");
    let base = serde_json::to_string_pretty(&v).unwrap().len();
    v["zzPad"] = json!("x".repeat(len - base));
    let t = serde_json::to_string_pretty(&v).unwrap();
    assert_eq!(t.len(), len);
    t
}

type Reads = fn(&str) -> bool;

#[test]
fn wp_p5mdoc_size_bounds_refuse_one_byte_past() {
    let cases: [(Value, usize, Reads); 3] = [
        (
            serde_json::to_value(measurement(2, false)).unwrap(),
            MAX_MEASUREMENT_BYTES,
            |t| MeasurementDoc::from_text(t).is_ok(),
        ),
        (serde_json::to_value(mic()).unwrap(), MAX_MIC_BYTES, |t| {
            MicDoc::from_text(t).is_ok()
        }),
        (
            serde_json::to_value(target(true)).unwrap(),
            MAX_TARGET_BYTES,
            |t| TargetDoc::from_text(t).is_ok(),
        ),
    ];
    for (v, max, reads) in cases {
        assert!(reads(&padded(&v, max)), "at {max}");
        assert!(!reads(&padded(&v, max + 1)), "past {max}");
    }
    // A writer refuses to make a document past its bound.
    let mut m = mic();
    m.extra
        .insert("zzPad".into(), json!("x".repeat(MAX_MIC_BYTES)));
    assert!(matches!(m.to_text(), Err(MeasureError::Invalid(_))));
    // The any-kind reader and the kind peek refuse past the largest bound.
    let big = padded(
        &serde_json::to_value(measurement(1, false)).unwrap(),
        MAX_MEASUREMENT_BYTES + 1,
    );
    assert!(MeasureDocument::from_text(&big).is_err());
    assert_eq!(doc_kind(&big), None);
    // A mic text over 256 KiB is not read as a mic even through the any-kind reader.
    let big_mic = padded(&serde_json::to_value(mic()).unwrap(), MAX_MIC_BYTES + 1);
    assert!(MeasureDocument::from_text(&big_mic).is_err());
}

fn refused(d: &MeasurementDoc) -> bool {
    let bad = matches!(d.validate(), Err(MeasureError::Invalid(_)));
    assert_eq!(bad, d.to_text().is_err());
    bad
}

#[test]
fn wp_p5mdoc_output_and_position_bounds() {
    let mut d = measurement(1, false);
    d.measurement.outputs = (0..16)
        .map(|o| output(o, WayRole::Full, "", false))
        .collect();
    assert!(!refused(&d));
    d.measurement
        .outputs
        .push(output(15, WayRole::Full, "", false));
    assert!(refused(&d), "17 outputs");

    let mut d = measurement(1, false);
    d.measurement.outputs[0].output = 15;
    assert!(!refused(&d));
    d.measurement.outputs[0].output = 16;
    assert!(refused(&d), "output 16");

    let mut d = measurement(2, false);
    d.measurement.outputs[1].output = 0;
    assert!(refused(&d), "an output twice");

    let mut d = measurement(1, false);
    d.measurement.outputs.clear();
    assert!(refused(&d), "no output");

    let mut d = measurement(1, false);
    d.measurement.positions = 9;
    assert!(!refused(&d));
    d.measurement.positions = 10;
    assert!(refused(&d), "10 positions");
    d.measurement.positions = 0;
    assert!(refused(&d), "0 positions");

    for (p, ok) in [
        (Placement::MultiPosition { count: 9 }, true),
        (Placement::MultiPosition { count: 10 }, false),
        (Placement::MultiPosition { count: 2 }, false),
        (
            Placement::Headphones {
                coupler: Coupler::FlatPlate,
                reseats: 9,
            },
            true,
        ),
        (
            Placement::Headphones {
                coupler: Coupler::Hats,
                reseats: 10,
            },
            false,
        ),
        (Placement::NearField, true),
    ] {
        let mut d = measurement(1, false);
        d.measurement.placement = p;
        assert_eq!(!refused(&d), ok, "{p:?}");
    }

    let mut d = measurement(1, false);
    d.measurement.outputs[0].chain.chain.peq = vec![band(1, 100.0); MAX_BANDS];
    assert!(!refused(&d));
    d.measurement.outputs[0]
        .chain
        .chain
        .peq
        .push(band(1, 100.0));
    assert!(refused(&d), "33 bands");

    let mut d = measurement(1, false);
    d.measurement.tunes = vec![d.measurement.tunes[0].clone(); MAX_TUNES + 1];
    assert!(refused(&d), "65 tunes");
}

#[test]
fn wp_p5mdoc_name_and_note_bounds() {
    let name_ok = "é".repeat(MAX_NAME_CHARS);
    let name_bad = "é".repeat(MAX_NAME_CHARS + 1);
    let note_ok = "n".repeat(MAX_NOTE_CHARS);
    let note_bad = "n".repeat(MAX_NOTE_CHARS + 1);

    let mut d = measurement(1, false);
    d.meta.name = name_ok.clone();
    d.measurement.note = note_ok.clone();
    assert!(!refused(&d));
    d.meta.name = name_bad.clone();
    assert!(refused(&d), "meta.name");
    d.meta.name = name_ok.clone();
    d.measurement.note = note_bad.clone();
    assert!(refused(&d), "note");

    let mut m = mic();
    m.mic.name = name_ok.clone();
    m.mic.note = note_ok.clone();
    assert!(m.validate().is_ok());
    m.mic.name = name_bad.clone();
    assert!(m.validate().is_err(), "mic.name");
    m.mic.name = name_ok.clone();
    m.mic.note = note_bad.clone();
    assert!(m.validate().is_err(), "mic.note");

    let mut t = target(false);
    t.target.name = name_ok;
    t.target.note = note_ok;
    assert!(t.validate().is_ok());
    t.target.name = name_bad;
    assert!(t.validate().is_err(), "target.name");
    let mut t = target(false);
    t.target.note = note_bad;
    assert!(t.validate().is_err(), "target.note");
}

/// A grid of exactly `n` points: 10 Hz up at 1/192 octave.
fn grid_of(n: usize) -> LogGrid {
    LogGrid {
        min_hz: 10.0,
        max_hz: 10.0 * ((n as f64 - 0.5) / 192.0).exp2(),
        points_per_octave: 192,
    }
}

#[test]
fn wp_p5mdoc_grid_bound() {
    for (n, ok) in [(MAX_GRID_POINTS, true), (MAX_GRID_POINTS + 1, false)] {
        let g = grid_of(n);
        let c = Curve {
            grid: g,
            mag_db: vec![0.0; n],
            phase_deg: None,
            valid: None,
            spread_db: None,
            spl: false,
            extra: Map::new(),
        };
        assert_eq!(c.validate("c").is_ok(), ok, "{n} points");
        let mut m = mic();
        m.mic.response = c.clone();
        assert_eq!(m.to_text().is_ok(), ok, "{n} points");
        if !ok {
            // Read from text too: the decoder refuses the grid before decoding.
            let mut v = serde_json::to_value(mic()).unwrap();
            v["mic"]["response"]["grid"] = serde_json::to_value(g).unwrap();
            v["mic"]["response"]["mag"] = json!(B64.encode(vec![0u8; 2 * n]));
            assert!(MicDoc::from_text(&v.to_string()).is_err());
        }
    }
}

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

type Edit = fn(&mut MeasurementDoc);

#[test]
fn wp_p5mdoc_non_finite_and_out_of_range_values_are_refused() {
    let edits: Vec<(&str, Edit)> = vec![
        ("mag NaN", |d| {
            d.measurement.outputs[0].response.mag_db[3] = f64::NAN
        }),
        ("mag past range", |d| {
            d.measurement.outputs[0].response.mag_db[3] = MAX_MAG_DB + 0.01
        }),
        ("phase inf", |d| {
            d.measurement.outputs[0]
                .response
                .phase_deg
                .as_mut()
                .unwrap()[0] = f64::INFINITY
        }),
        ("spread negative", |d| {
            d.measurement.outputs[0].response.spread_db = Some(vec![-1.0; n_std()])
        }),
        ("mag short", |d| {
            d.measurement.outputs[0].response.mag_db.pop();
        }),
        ("valid long", |d| {
            d.measurement.outputs[0]
                .response
                .valid
                .as_mut()
                .unwrap()
                .push(true)
        }),
        ("chain gain NaN", |d| {
            d.measurement.outputs[0].chain.chain.gain_db = f32::NAN
        }),
        ("band freq inf", |d| {
            d.measurement.outputs[0].chain.chain.peq[0].freq = f32::INFINITY
        }),
        ("level NaN", |d| d.measurement.stimulus.level_db = f64::NAN),
        ("level above 0", |d| d.measurement.stimulus.level_db = 0.5),
        ("clock inf", |d| {
            d.measurement.capture.clock_ratio = f64::INFINITY
        }),
        ("arrival NaN", |d| {
            d.measurement.outputs[0].arrival_ms = Some(f64::NAN)
        }),
        ("sweep too long", |d| {
            d.measurement.stimulus.duration_ms = 15_001
        }),
        ("repeat 9", |d| d.measurement.stimulus.repeat = 9),
        ("rate", |d| d.measurement.stimulus.device_rate_hz = 32_000),
        ("f2 below f1", |d| d.measurement.stimulus.f2_hz = 10.0),
        ("schema 2", |d| d.schema_version = 2),
        ("kind", |d| d.meta.kind = "snapshot".into()),
        ("bad id", |d| d.measurement.id = "../etc".into()),
    ];
    for (what, edit) in edits {
        let mut d = measurement(2, true);
        edit(&mut d);
        assert!(refused(&d), "{what}");
    }
    let mut d = measurement(1, false);
    d.measurement.outputs[0].response.mag_db[3] = MAX_MAG_DB;
    assert!(!refused(&d), "the largest magnitude");

    // A number too large for the band's f32 reads as infinity: refused.
    let mut v = serde_json::to_value(measurement(1, false)).unwrap();
    v["measurement"]["outputs"][0]["chain"]["peq"][0]["freq"] = json!(1e39);
    assert!(MeasurementDoc::from_text(&v.to_string()).is_err());
    // JSON has no NaN: such text is not a document.
    let t = measurement(1, false)
        .to_text()
        .unwrap()
        .replacen("-24.0", "NaN", 1);
    assert!(matches!(
        MeasurementDoc::from_text(&t),
        Err(MeasureError::Parse(_))
    ));

    let mut m = mic();
    m.mic.angle = 45;
    assert!(m.validate().is_err());
    let mut t = target(false);
    t.target.reference_band.high_hz = 400.0;
    assert!(t.validate().is_err(), "band upside down");
    let mut t = target(false);
    t.target.use_for = TargetUse::Speakers;
    assert!(t.validate().is_err(), "a coupler on a speaker target");
    let mut t = target(false);
    if let TargetDefinition::Builtin(b) = &mut t.target.definition {
        b.params = (0..=MAX_BUILTIN_PARAMS)
            .map(|i| (format!("p{i}"), 1.0))
            .collect();
    }
    assert!(t.validate().is_err(), "33 params");
}

// ── other kinds ───────────────────────────────────────────────────────────

#[test]
fn wp_p5mdoc_each_parser_refuses_other_kinds() {
    let m = measurement(1, false).to_text().unwrap();
    let c = mic().to_text().unwrap();
    let t = target(true).to_text().unwrap();
    let mut others: Vec<String> = Vec::new();
    for kind in [
        json!("snapshot"),
        json!("preset"),
        json!("limits"),
        json!(7),
    ] {
        let mut v: Value = serde_json::from_str(&m).unwrap();
        v["meta"]["kind"] = kind;
        others.push(v.to_string());
    }
    let mut v: Value = serde_json::from_str(&m).unwrap();
    v["meta"].as_object_mut().unwrap().remove("kind");
    others.push(v.to_string()); // a preset

    for o in &others {
        assert!(MeasurementDoc::from_text(o).is_err());
        assert!(MicDoc::from_text(o).is_err());
        assert!(TargetDoc::from_text(o).is_err());
        assert!(MeasureDocument::from_text(o).is_err());
        assert_eq!(doc_kind(o), None);
    }
    assert!(matches!(
        MeasurementDoc::from_text(&c),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        MeasurementDoc::from_text(&t),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        MicDoc::from_text(&m),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        MicDoc::from_text(&t),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        TargetDoc::from_text(&m),
        Err(MeasureError::Invalid(_))
    ));
    assert!(matches!(
        TargetDoc::from_text(&c),
        Err(MeasureError::Invalid(_))
    ));

    // A mic block under a measurement's kind is not a measurement either.
    let mut v: Value = serde_json::from_str(&c).unwrap();
    v["meta"]["kind"] = json!("measurement");
    assert!(MeasurementDoc::from_text(&v.to_string()).is_err());

    // The kind word is read leniently, as `limits` is.
    let mut v: Value = serde_json::from_str(&c).unwrap();
    v["meta"]["kind"] = json!(" MIC ");
    assert!(MicDoc::from_text(&v.to_string()).is_ok());

    // Another schema is refused; so is none.
    let mut v: Value = serde_json::from_str(&c).unwrap();
    v["schemaVersion"] = json!(2);
    assert!(MicDoc::from_text(&v.to_string()).is_err());
    v.as_object_mut().unwrap().remove("schemaVersion");
    assert!(MicDoc::from_text(&v.to_string()).is_err());
    // Not an object, not JSON.
    for t in ["[]", "1", "", "{", "\u{feff}"] {
        assert!(MicDoc::from_text(t).is_err(), "{t:?}");
        assert!(MeasureDocument::from_text(t).is_err(), "{t:?}");
    }
    // A byte-order mark is skipped.
    assert!(MicDoc::from_text(&format!("\u{feff}{c}")).is_ok());
}

// ── unknown keys ──────────────────────────────────────────────────────────

#[test]
fn wp_p5mdoc_unknown_keys_are_kept_and_written_back() {
    let mut v = serde_json::to_value(measurement(2, true)).unwrap();
    let paths = [
        "",
        "/meta",
        "/measurement",
        "/measurement/mic",
        "/measurement/stimulus",
        "/measurement/capture",
        "/measurement/outputs/1",
        "/measurement/outputs/1/chain",
        "/measurement/outputs/1/response",
        "/measurement/tunes/0",
    ];
    for p in paths {
        v.pointer_mut(p)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("futureKey".into(), json!({"a": [1, "b"], "p": p}));
    }
    let d = MeasurementDoc::from_text(&v.to_string()).unwrap();
    assert_eq!(d.measurement.outputs[1].chain.extra.len(), 1);
    assert!(!d.measurement.outputs[1].chain.extra.contains_key("gainDb"));
    let back: Value = serde_json::from_str(&d.to_text().unwrap()).unwrap();
    for p in paths {
        assert_eq!(
            back.pointer(&format!("{p}/futureKey")),
            Some(&json!({"a": [1, "b"], "p": p})),
            "{p}"
        );
    }

    let mut v = serde_json::to_value(target(true)).unwrap();
    for p in [
        "",
        "/target",
        "/target/definition",
        "/target/definition/curve",
        "/target/compensation",
        "/target/referenceBand",
    ] {
        v.pointer_mut(p)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("futureKey".into(), json!(p));
    }
    let back: Value = serde_json::from_str(
        &TargetDoc::from_text(&v.to_string())
            .unwrap()
            .to_text()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        back.pointer("/target/definition/futureKey"),
        Some(&json!("/target/definition"))
    );
    assert_eq!(
        back.pointer("/target/definition/type"),
        Some(&json!("imported"))
    );
    assert_eq!(
        back.pointer("/target/compensation/futureKey"),
        Some(&json!("/target/compensation"))
    );

    let mut v = serde_json::to_value(mic()).unwrap();
    v["mic"]["futureKey"] = json!(true);
    let back = MicDoc::from_text(&v.to_string()).unwrap();
    assert_eq!(back.mic.extra.get("futureKey"), Some(&json!(true)));
}

// ── the curve encoding ────────────────────────────────────────────────────

#[test]
fn wp_p5mdoc_curve_encoding_is_the_documented_one() {
    let g = LogGrid {
        min_hz: 100.0,
        max_hz: 1_000.0,
        points_per_octave: 3,
    };
    let n = g.freqs().unwrap().len();
    assert_eq!(n, 10);
    let c = Curve {
        grid: g,
        mag_db: (0..n).map(|i| i as f64 * 0.5 - 1.0).collect(),
        phase_deg: Some((0..n).map(|i| -(i as f64) * 100.06).collect()),
        valid: Some((0..n).map(|i| i % 3 == 0).collect()),
        spread_db: None,
        spl: false,
        extra: Map::new(),
    };
    let v = serde_json::to_value(&c).unwrap();
    let mag = B64.decode(v["mag"].as_str().unwrap()).unwrap();
    assert_eq!(&mag[..4], &[0x9c, 0xff, 0xce, 0xff]); // -100, -50 (0.01 dB steps, LE)
    let phase = B64.decode(v["phase"].as_str().unwrap()).unwrap();
    assert_eq!(&phase[4..8], &(-1001i32).to_le_bytes()); // -100.1° (0.1° steps)
    let bits = B64.decode(v["valid"].as_str().unwrap()).unwrap();
    assert_eq!(bits, vec![0b0100_1001, 0b0000_0010]); // points 0, 3, 6, 9
    assert_eq!(
        v["grid"],
        json!({"minHz": 100.0, "maxHz": 1000.0, "pointsPerOctave": 3})
    );
    assert!(v.get("spread").is_none());

    // Wrong lengths, padding and alphabets are refused, never guessed.
    for (key, bad) in [
        ("mag", B64.encode([0u8; 18])),
        ("mag", B64.encode([0u8; 22])),
        ("phase", B64.encode([0u8; 36])),
        ("valid", B64.encode([0u8; 3])),
        ("valid", B64.encode([0u8; 1])),
        ("mag", "!!!!".to_string()),
        ("mag", "x".repeat(10_000)),
    ] {
        let mut w = v.clone();
        w[key] = json!(bad);
        assert!(
            serde_json::from_value::<Curve>(w).is_err(),
            "{key}: {bad:.20}"
        );
    }
}

// ── properties ────────────────────────────────────────────────────────────

fn small_measurement_text() -> String {
    let g = LogGrid {
        min_hz: 20.0,
        max_hz: 20_000.0,
        points_per_octave: 3,
    };
    let mut d = measurement(2, true);
    for o in &mut d.measurement.outputs {
        let n = g.freqs().unwrap().len();
        o.response = Curve {
            grid: g,
            mag_db: vec![1.0; n],
            phase_deg: Some(vec![-10.0; n]),
            valid: Some(vec![true; n]),
            spread_db: Some(vec![0.5; n]),
            spl: false,
            extra: Map::new(),
        };
        o.chain.chain.peq.truncate(2);
        o.chain.chain.crossover.truncate(1);
    }
    d.to_text().unwrap()
}

/// Every reader on `text`: no panic; whatever reads writes and reads back.
fn every_reader(text: &str) {
    let _ = doc_kind(text);
    if let Ok(d) = MeasurementDoc::from_text(text) {
        let back = MeasurementDoc::from_text(&d.to_text().unwrap()).unwrap();
        assert!(same_doc(&back, &d));
    }
    if let Ok(d) = MicDoc::from_text(text) {
        let back = MicDoc::from_text(&d.to_text().unwrap()).unwrap();
        assert!(same_doc(&back, &d));
    }
    if let Ok(d) = TargetDoc::from_text(text) {
        let back = TargetDoc::from_text(&d.to_text().unwrap()).unwrap();
        assert!(same_doc(&back, &d));
    }
    if let Ok(d) = MeasureDocument::from_text(text) {
        assert!(d.to_text().is_ok());
    }
}

/// Equal as JSON, numbers to 1e-12 relative: serde_json's parser (without
/// its `float_roundtrip` feature) may read an extreme decimal one ulp off.
/// Curves are integers and compare exactly.
fn same_doc<T: Serialize>(a: &T, b: &T) -> bool {
    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
                (Some(x), Some(y)) => x == y || (x - y).abs() <= 1e-12 * x.abs().max(y.abs()),
                _ => x == y,
            },
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
            }
            (Value::Object(x), Value::Object(y)) => {
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
            }
            _ => a == b,
        }
    }
    let (a, b) = (
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap(),
    );
    let ok = same(&a, &b);
    if !ok {
        eprintln!("differ:\n{a}\n{b}");
    }
    ok
}

/// A JSON value proptest picks to put in place of a field.
fn any_json() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        proptest::num::f64::NORMAL.prop_map(Value::from),
        (-2.0f64..2.0).prop_map(Value::from),
        ".{0,12}".prop_map(Value::from),
        "[A-Za-z0-9+/=]{0,40}".prop_map(Value::from),
        Just(json!([])),
        Just(json!({})),
        Just(json!("measurement")),
        Just(json!("mic")),
    ]
}

/// Every object and array path in `v`, depth first.
fn paths(v: &Value, at: String, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let p = format!("{at}/{k}");
                out.push(p.clone());
                paths(x, p, out);
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                let p = format!("{at}/{i}");
                out.push(p.clone());
                paths(x, p, out);
            }
        }
        _ => {}
    }
}

proptest! {
    /// Byte-level mutations of a valid document: replaced bytes, a cut, an
    /// inserted run.
    #[test]
    fn proptest_wp_p5mdoc_mutated_text_never_panics(
        edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..8),
        cut in any::<prop::sample::Index>(),
        do_cut in any::<bool>(),
        insert in proptest::collection::vec(any::<u8>(), 0..16),
        at in any::<prop::sample::Index>(),
    ) {
        let base = small_measurement_text();
        let mut bytes = base.into_bytes();
        for (i, b) in &edits {
            let i = i.index(bytes.len());
            bytes[i] = *b;
        }
        let at = at.index(bytes.len());
        bytes.splice(at..at, insert);
        if do_cut {
            bytes.truncate(cut.index(bytes.len()));
        }
        every_reader(&String::from_utf8_lossy(&bytes));
    }

    /// Structure-level mutations: a field replaced by any JSON, or removed,
    /// at any depth of each kind.
    #[test]
    fn proptest_wp_p5mdoc_mutated_documents_never_panic(
        which in 0usize..3,
        picks in proptest::collection::vec((any::<prop::sample::Index>(), any_json(), any::<bool>()), 1..4),
    ) {
        let text = match which {
            0 => small_measurement_text(),
            1 => mic().to_text().unwrap(),
            _ => target(true).to_text().unwrap(),
        };
        let mut v: Value = serde_json::from_str(&text).unwrap();
        for (pick, new, remove) in picks {
            let mut all = Vec::new();
            paths(&v, String::new(), &mut all);
            let p = &all[pick.index(all.len())];
            let (parent, key) = p.rsplit_once('/').unwrap();
            match v.pointer_mut(parent) {
                Some(Value::Object(m)) if remove => { m.remove(key); }
                Some(Value::Array(a)) if remove => {
                    let i: usize = key.parse().unwrap();
                    a.remove(i);
                }
                _ => { *v.pointer_mut(p).unwrap() = new; }
            }
        }
        every_reader(&v.to_string());
    }

    /// Any value in range comes back within half a step.
    #[test]
    fn proptest_wp_p5mdoc_curve_round_trips_within_half_a_step(
        mag in proptest::collection::vec(-MAX_MAG_DB..MAX_MAG_DB, 10),
        phase in proptest::collection::vec(-1e7f64..1e7, 10),
        spread in proptest::collection::vec(0.0..MAX_MAG_DB, 10),
        valid in proptest::collection::vec(any::<bool>(), 10),
    ) {
        let c = Curve {
            grid: LogGrid { min_hz: 100.0, max_hz: 1_000.0, points_per_octave: 3 },
            mag_db: mag,
            phase_deg: Some(phase),
            valid: Some(valid),
            spread_db: Some(spread),
            spl: true,
            extra: Map::new(),
        };
        prop_assert!(c.validate("c").is_ok());
        let back: Curve = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        prop_assert!(close(&back.mag_db, &c.mag_db, 0.005 + 1e-9));
        prop_assert!(close(back.phase_deg.as_ref().unwrap(), c.phase_deg.as_ref().unwrap(), 0.05 + 1e-6));
        prop_assert!(close(back.spread_db.as_ref().unwrap(), c.spread_db.as_ref().unwrap(), 0.005 + 1e-9));
        prop_assert_eq!(&back.valid, &c.valid);
        prop_assert!(back.mag_db.iter().all(|x| x.is_finite()));
    }

    /// Any text: the readers and the kind peek never panic.
    #[test]
    fn proptest_wp_p5mdoc_any_text_never_panics(text in ".{0,200}") {
        every_reader(&text);
        let wrapped = format!(
            "{{\"schemaVersion\":1,\"meta\":{{\"kind\":\"mic\"}},\"mic\":{text}}}"
        );
        every_reader(&wrapped);
    }

    /// Validation never panics on any numbers, and refuses every non-finite.
    #[test]
    fn proptest_wp_p5mdoc_validate_never_panics(
        level in proptest::num::f64::ANY,
        ratio in proptest::num::f64::ANY,
        arrival in proptest::num::f64::ANY,
        mag in proptest::num::f64::ANY,
        gain in proptest::num::f32::ANY,
        output in any::<u8>(),
        positions in any::<u8>(),
    ) {
        let mut d = measurement(1, false);
        d.measurement.stimulus.level_db = level;
        d.measurement.capture.clock_ratio = ratio;
        d.measurement.outputs[0].arrival_ms = Some(arrival);
        d.measurement.outputs[0].response.mag_db[0] = mag;
        d.measurement.outputs[0].chain.chain.gain_db = gain;
        d.measurement.outputs[0].output = output;
        d.measurement.positions = positions;
        let ok = d.validate().is_ok();
        let finite = level.is_finite() && ratio.is_finite() && arrival.is_finite()
            && mag.is_finite() && gain.is_finite();
        prop_assert!(finite || !ok);
        prop_assert_eq!(ok, d.to_text().is_ok());
    }
}

// ── the contract's examples ───────────────────────────────────────────────

/// Every JSON example in `docs/integrator/measurement.md` reads: the documents
/// as their kind, the bare curve as a curve with the values the page states.
#[test]
fn wp_p5mdoc_the_contract_examples_read() {
    let page = include_str!("../../../../docs/integrator/measurement.md");
    let blocks: Vec<&str> = page
        .split("```json\n")
        .skip(1)
        .filter_map(|b| b.split("```").next())
        .collect();
    assert_eq!(blocks.len(), 4, "the curve and one document of each kind");
    let mut kinds = Vec::new();
    for b in blocks {
        if b.contains("\"schemaVersion\"") {
            let d = MeasureDocument::from_text(b).unwrap_or_else(|e| panic!("{e}: {b}"));
            kinds.push(d.kind());
        } else {
            let c: Curve = serde_json::from_str(b).unwrap();
            assert!(close(&c.mag_db[..3], &[80.0, 82.52, 82.73], 1e-9));
            assert!(close(
                &c.phase_deg.unwrap()[..3],
                &[0.0, -36.0, -72.0],
                1e-9
            ));
            let v = c.valid.unwrap();
            assert!(!v[0] && v[1..].iter().all(|x| *x));
            assert!(c.spl);
        }
    }
    assert_eq!(kinds, DocKind::ALL);
}
