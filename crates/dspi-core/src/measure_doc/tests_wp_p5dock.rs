//! WP P5DOCK tests: the core's section model against the curves, the
//! documents through the API (store, imports, export), and the refusal of a
//! measurement document by every configuration path, with nothing sent.
//!
//! Stores live under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dspi_measure::doc::{MeasurementDoc, MicDoc, TargetDefinition, TargetDoc};
use dspi_measure::types::ChannelChain;
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps};
use serde_json::{Value, json};

use super::*;
use crate::curves::{compute_magnitude_curve, compute_phase_curve, compute_response, curve_freq};

macro_rules! cal_fixture {
    ($name:literal) => {
        include_str!(concat!("../../../dspi-measure/testdata/p5/cal/", $name))
    };
}

// ── the section model ─────────────────────────────────────────────────────

/// One case of the curves' reference table (`curves_ref.rs`), as
/// `curves.rs`'s own tests read it.
#[allow(dead_code)]
struct Case {
    name: &'static str,
    bands: &'static [(u8, f32, f32, f32, f32, bool)],
    resp: [f32; 6],
    pts: &'static [(usize, f64, f32, f32)],
}

include!("../curves_ref.rs");

fn band_spec(
    &(filter_type, freq, q, gain, qp, bypass): &(u8, f32, f32, f32, f32, bool),
) -> BandSpec {
    BandSpec {
        filter_type,
        bypass,
        freq,
        q,
        gain,
        qp,
    }
}

fn filter(b: &BandSpec) -> FilterParams {
    FilterParams {
        filter_type: b.filter_type,
        bypass: b.bypass,
        freq: b.freq,
        q: b.q,
        gain: b.gain,
        qp: b.qp,
    }
}

/// `H(e^jw)` of a cascade at `f`, as (dB, degrees): plain complex
/// arithmetic, independent of the curves' `sin²` form.
fn eval(sections: &[Coeffs], f: f64, fs: f64) -> (f64, f64) {
    let w = 2.0 * std::f64::consts::PI * f / fs;
    let (mut re, mut im) = (1.0f64, 0.0f64);
    for c in sections {
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (nr, ni) = (c.b0 + c.b1 * c1 + c.b2 * c2, -(c.b1 * s1 + c.b2 * s2));
        let (dr, di) = (1.0 + c.a1 * c1 + c.a2 * c2, -(c.a1 * s1 + c.a2 * s2));
        let d2 = dr * dr + di * di;
        let (hr, hi) = ((nr * dr + ni * di) / d2, (ni * dr - nr * di) / d2);
        (re, im) = (re * hr - im * hi, re * hi + im * hr);
    }
    (
        10.0 * (re * re + im * im).log10(),
        im.atan2(re).to_degrees(),
    )
}

fn wrap(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

/// The done criterion: the model's sections, evaluated on their own, agree
/// with `compute_magnitude_curve` and `compute_phase_curve` within 0.01 dB
/// and 0.1° on every case of the curves' reference table (where the level is
/// above -100 dB: below it the plain complex form is not the reference's
/// precision, and the phase of a near-zero is not defined).
#[test]
fn wp_p5dock_section_model_agrees_with_the_curves_on_the_reference_table() {
    let m = CoreSectionModel;
    assert_eq!(m.sample_rate(), dspi_proto::dsp::SAMPLE_RATE);
    let mut compared = 0usize;
    for case in REFERENCE {
        let bands: Vec<BandSpec> = case.bands.iter().map(band_spec).collect();
        let filters: Vec<FilterParams> = bands.iter().map(filter).collect();
        let sections: Vec<Coeffs> = bands.iter().flat_map(|b| m.sections(b)).collect();
        let mag = compute_magnitude_curve(filters.clone());
        let phase = compute_phase_curve(filters, false);
        for (i, (want_db, want_deg)) in mag.iter().zip(&phase).enumerate() {
            let (db, deg) = eval(&sections, curve_freq(i as u32), m.sample_rate());
            if *want_db < -100.0 {
                continue;
            }
            assert!(
                (db - want_db).abs() <= 0.01,
                "{} point {i}: {db} dB vs {want_db} dB",
                case.name
            );
            assert!(
                wrap(deg - want_deg).abs() <= 0.1,
                "{} point {i}: {deg}° vs {want_deg}°",
                case.name
            );
            compared += 1;
        }
    }
    assert!(compared > 1000, "{compared}");
}

/// The same agreement through the measurement maths' own path: de-embedding
/// a flat measurement by a chain gives the chain's inverse, on the standard
/// grid, against `compute_response`.
#[test]
fn wp_p5dock_section_model_through_dspi_measure_de_embedding() {
    use dspi_measure::align::Way;
    use dspi_measure::types::{ProgrammeFeed, Response, WayRole};
    let freqs = LogGrid::STANDARD.freqs().unwrap();
    let n = freqs.len();
    let mut checked = 0usize;
    for case in REFERENCE {
        let bands: Vec<BandSpec> = case.bands.iter().map(band_spec).collect();
        if !bands.iter().all(|b| CoreSectionModel.models(b)) {
            continue;
        }
        let way = Way {
            output: 0,
            role: WayRole::Full,
            chain: ChannelChain {
                peq: bands.clone(),
                crossover: vec![],
                gain_db: 0.0,
                delay_ms: 0.0,
                feed: ProgrammeFeed {
                    gain_db: 0.0,
                    inverted: false,
                },
            },
            response: Response {
                grid: LogGrid::STANDARD,
                mag_db: vec![0.0; n],
                phase_deg: Some(vec![0.0; n]),
                valid: Some(vec![true; n]),
                spl: false,
            },
            snr_db: vec![200.0; n],
            arrival_ms: 0.0,
            timing_reliable: true,
        };
        let d = dspi_measure::xover::de_embed(&way, &CoreSectionModel).unwrap();
        let filters: Vec<FilterParams> = bands.iter().map(filter).collect();
        for (i, f) in freqs.iter().enumerate() {
            let want = f64::from(compute_response(filters.clone(), *f as f32));
            if want < -100.0 || *f > 20_000.0 {
                continue;
            }
            let got = -d.response.mag_db[i];
            assert!(
                (got - want).abs() <= 0.01,
                "{} at {f} Hz: {got} vs {want}",
                case.name
            );
            checked += 1;
        }
    }
    assert!(checked > 1000, "{checked}");
}

#[test]
fn wp_p5dock_section_model_flat_bypassed_and_unknown_bands() {
    let m = CoreSectionModel;
    let b = |t: u8, bypass: bool| BandSpec {
        filter_type: t,
        bypass,
        freq: 1000.0,
        q: 0.707,
        gain: 3.0,
        qp: 0.707,
    };
    assert!(m.sections(&b(0, false)).is_empty());
    assert!(m.sections(&b(1, true)).is_empty());
    assert!(m.models(&b(0, false)) && m.models(&b(11, false)) && m.models(&b(63, false)));
    for unknown in [14u8, 31, 64, 200] {
        assert!(!m.models(&b(unknown, false)), "{unknown}");
        assert!(m.sections(&b(unknown, false)).is_empty(), "{unknown}");
    }
    // An LR4 crossover is two sections pairs as the firmware builds it.
    let lr4 = (32..=63u8)
        .find(|t| {
            dspi_proto::xover::meta(*t).is_some_and(|x| {
                x.family == dspi_proto::xover::Family::LinkwitzRiley && x.order == 4
            })
        })
        .unwrap();
    assert_eq!(m.sections(&b(lr4, false)).len(), 2);
    // Non-finite values never panic and never give a non-finite section.
    for v in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1e30] {
        for t in 0..=70u8 {
            let band = BandSpec {
                filter_type: t,
                bypass: false,
                freq: v,
                q: v,
                gain: v,
                qp: v,
            };
            for c in m.sections(&band) {
                assert!(
                    [c.b0, c.b1, c.b2, c.a1, c.a2].iter().all(|x| x.is_finite()),
                    "{t} {v}"
                );
            }
        }
    }
}

// ── the documents ─────────────────────────────────────────────────────────

/// The demo device behind a counter of the OUT transfers that reach it.
struct Counted {
    dev: FakeDevice,
    outs: Arc<AtomicUsize>,
}

impl Transport for Counted {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.dev.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.dev.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.outs.fetch_add(1, Ordering::SeqCst);
        self.dev.control_out(op, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

fn rig() -> (Arc<DspiCore>, Arc<AtomicUsize>) {
    let outs = Arc::new(AtomicUsize::new(0));
    let core = DspiCore::new();
    core.attach_probed(
        Box::new(Counted {
            dev: FakeDevice::demo(),
            outs: Arc::clone(&outs),
        }),
        "P5DOCK-DEMO".into(),
        0,
    )
    .unwrap();
    core.fetch_all().unwrap();
    (core, outs)
}

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p5dock-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A measurement document with no serial: two outputs with phase, a
/// microphone, and a key this build does not know.
fn measurement_text() -> String {
    let n = LogGrid::STANDARD.freqs().unwrap().len();
    let outputs: Vec<Value> = [0u8, 2]
        .iter()
        .map(|o| {
            let response =
                dspi_measure::doc::Curve::from_response(&dspi_measure::types::Response {
                    grid: LogGrid::STANDARD,
                    mag_db: (0..n)
                        .map(|i| 70.0 + f64::from(*o) + (i as f64 * 0.01).sin())
                        .collect(),
                    phase_deg: Some((0..n).map(|i| -(i as f64) * 0.5).collect()),
                    valid: Some((0..n).map(|i| i > 100).collect()),
                    spl: true,
                });
            json!({
                "output": o,
                "role": if *o == 0 { "low" } else { "high" },
                "speaker": "L",
                "chain": {"peq": [], "crossover": [], "gainDb": 0.0, "delayMs": 0.0,
                          "feed": {"gainDb": 0.0, "inverted": false}},
                "arrivalMs": 3.0,
                "response": serde_json::to_value(response).unwrap(),
            })
        })
        .collect();
    let doc = json!({
        "schemaVersion": 1,
        "meta": {"kind": "measurement", "name": "Living room", "savedUtc": "2026-11-16T10:00:00Z", "appVersion": "t"},
        "measurement": {
            "serial": "",
            "platform": "RP2350",
            "savedMs": 1_763_287_200_000i64,
            "placement": {"mode": "listeningPosition"},
            "mic": {"name": "UMIK-1", "approximate": true},
            "stimulus": {"kind": "walked", "f1Hz": 20.0, "f2Hz": 20000.0, "durationMs": 5500,
                         "repeat": 3, "gapMs": 1000, "levelDb": -30.0, "deviceRateHz": 48000},
            "capture": {"rateHz": 48000, "clockRatio": 1.0, "timingReliable": true},
            "outputs": outputs,
            "futureKey": {"kept": true},
        },
    });
    let text = doc.to_string();
    MeasurementDoc::from_text(&text).unwrap();
    text
}

#[test]
fn wp_p5dock_the_store_through_the_api() {
    let (core, outs) = rig();
    // No folder named: empty lists, Io for the rest.
    assert_eq!(
        core.measure_docs(String::new(), String::new()).unwrap(),
        vec![]
    );
    assert!(matches!(
        core.measure_doc_put(measurement_text()),
        Err(DspiError::Io { .. })
    ));
    let dir = store_dir("api");
    core.measure_set_dir(dir.to_string_lossy().into_owned())
        .unwrap();
    let sent = outs.load(Ordering::SeqCst);

    let m = core.measure_doc_put(measurement_text()).unwrap();
    assert_eq!(m.serial, "P5DOCK-DEMO", "the open device's serial");
    assert_eq!(
        (m.kind.as_str(), m.saved_ms),
        ("measurement", 1_763_287_200_000)
    );
    let mic = core
        .measure_doc_put(
            mic_import(
                String::new(),
                "umik1_7000001.txt".into(),
                cal_fixture!("umik1_7000001.txt").into(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(mic.name, "miniDSP 7000001");
    let t = core
        .measure_doc_put(
            target_import(
                "Mine".into(),
                cal_fixture!("target_rew.txt").into(),
                "speakers".into(),
                String::new(),
            )
            .unwrap(),
        )
        .unwrap();
    let kinds: Vec<_> = core
        .measure_docs(String::new(), String::new())
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, vec!["measurement", "mic", "target"]);
    assert_eq!(
        core.measure_docs("measurement".into(), "P5DOCK-DEMO".into())
            .unwrap(),
        vec![m.clone()]
    );
    assert_eq!(
        core.measure_docs("measurement".into(), "OTHER".into())
            .unwrap(),
        vec![]
    );
    assert_eq!(
        core.measure_docs("MIC".into(), String::new()).unwrap(),
        vec![mic.clone()]
    );
    assert!(matches!(
        core.measure_docs("preset".into(), String::new()),
        Err(DspiError::InvalidArgument { .. })
    ));

    let text = core.measure_doc_get(m.id.clone()).unwrap();
    let d = MeasurementDoc::from_text(&text).unwrap();
    assert_eq!(d.measurement.id, m.id);
    assert_eq!(d.measurement.serial, "P5DOCK-DEMO");
    assert_eq!(d.measurement.extra["futureKey"], json!({"kept": true}));
    assert_eq!(measure_doc_info(text.clone()).unwrap().id, m.id);

    let r = core
        .measure_doc_rename(t.id.clone(), "Mein Ziel 🎯".into())
        .unwrap();
    assert_eq!(r.name, "Mein Ziel 🎯");
    let n = core.measure_doc_note(t.id.clone(), "warm".into()).unwrap();
    assert_eq!((n.name.as_str(), n.note.as_str()), ("Mein Ziel 🎯", "warm"));
    let td = TargetDoc::from_text(&core.measure_doc_get(t.id.clone()).unwrap()).unwrap();
    assert_eq!(
        (td.meta.name.as_str(), td.target.note.as_str()),
        ("Mein Ziel 🎯", "warm")
    );

    // The REW export, by id and by text, alike.
    let by_id = core.measurement_export_rew(m.id.clone(), 2).unwrap();
    let by_text = core.measurement_export_rew(text, 2).unwrap();
    assert_eq!(by_id, by_text);
    assert!(
        by_id.starts_with("* Measurement data exported by the DSPi console"),
        "{by_id}"
    );
    assert!(by_id.contains("Out 3 (L high)"), "{by_id}");
    let line = by_id.lines().find(|l| !l.starts_with('*')).unwrap();
    assert_eq!(line.split_whitespace().count(), 3, "{line}");
    assert!(matches!(
        core.measurement_export_rew(m.id.clone(), 1),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.measurement_export_rew(mic.id.clone(), 0),
        Err(DspiError::InvalidArgument { .. })
    ));

    core.measure_doc_delete(mic.id.clone()).unwrap();
    assert!(matches!(
        core.measure_doc_get(mic.id.clone()),
        Err(DspiError::InvalidArgument { .. })
    ));
    for bad in ["../x", "", "A"] {
        assert!(core.measure_doc_get(bad.into()).is_err());
    }
    // None of it touched the device.
    assert_eq!(outs.load(Ordering::SeqCst), sent);
    core.measure_set_dir(String::new()).unwrap();
    assert_eq!(
        core.measure_docs(String::new(), String::new()).unwrap(),
        vec![]
    );
}

#[test]
fn wp_p5dock_a_measurement_without_serial_needs_a_device() {
    let core = DspiCore::new();
    core.measure_set_dir(store_dir("noserial").to_string_lossy().into_owned())
        .unwrap();
    assert!(matches!(
        core.measure_doc_put(measurement_text()),
        Err(DspiError::InvalidArgument { .. })
    ));
}

#[test]
fn wp_p5dock_put_refuses_other_documents() {
    let (core, _) = rig();
    core.measure_set_dir(store_dir("other").to_string_lossy().into_owned())
        .unwrap();
    let preset = core.config_export("p".into(), "t".into(), 0).unwrap();
    let snapshot = core
        .snapshot_capture_document("s".into(), String::new())
        .unwrap();
    for bad in [preset, snapshot, "{".into(), String::new()] {
        assert!(
            matches!(
                core.measure_doc_put(bad.clone()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{bad:.80}"
        );
        assert!(measure_doc_info(bad).is_err());
    }
}

/// The done criterion: a `config_import` of a measurement (and of a `mic`
/// and a `target`) is refused with nothing sent, and so is every other path
/// that reads a configuration.
#[test]
fn wp_p5dock_config_import_of_a_measurement_is_refused_with_nothing_sent() {
    let (core, outs) = rig();
    let mic = mic_import(
        "m".into(),
        "rew_sensitivity.txt".into(),
        cal_fixture!("rew_sensitivity.txt").into(),
    )
    .unwrap();
    let target = target_import(
        "t".into(),
        cal_fixture!("target_rew.txt").into(),
        "speakers".into(),
        String::new(),
    )
    .unwrap();
    for text in [measurement_text(), mic, target] {
        // Even with a channels block added, the kind decides.
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v["channels"] = json!([{"channelId": "x"}]);
        for text in [text.clone(), v.to_string()] {
            let sent = outs.load(Ordering::SeqCst);
            let token = core.confirm_token(crate::ConfirmOp::ConfigImport);
            let r: Value = serde_json::from_str(&core.config_import_confirmed(
                text.clone(),
                0,
                u32::MAX,
                Some(token),
            ))
            .unwrap();
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["error"], MEASURE_DOC_REFUSAL, "{r}");
            assert_eq!(r["noticeKey"], crate::files::NOTICE_CONFIG_UNREADABLE);
            let p: Value =
                serde_json::from_str(&core.config_preview(text.clone(), 0, u32::MAX)).unwrap();
            assert_eq!(p["error"], MEASURE_DOC_REFUSAL, "{p}");
            let i: Value =
                serde_json::from_str(&crate::files::config_inspect(text.clone())).unwrap();
            assert_eq!(i["ok"], false, "{i}");
            assert!(core.snapshot_recall_document(text.clone()).is_err());
            assert_eq!(outs.load(Ordering::SeqCst), sent, "nothing sent");
        }
    }
}

#[test]
fn wp_p5dock_mic_import_reads_every_p5cal_fixture() {
    let cases: &[(&str, &str, &str, u16)] = &[
        (
            "umik1_7000001.txt",
            cal_fixture!("umik1_7000001.txt"),
            "umik",
            0,
        ),
        (
            "umik2_8000002_90deg.txt",
            cal_fixture!("umik2_8000002_90deg.txt"),
            "umik",
            90,
        ),
        (
            "umik_90_comment.txt",
            cal_fixture!("umik_90_comment.txt"),
            "umik",
            90,
        ),
        (
            "rew_sensitivity.txt",
            cal_fixture!("rew_sensitivity.txt"),
            "rew",
            0,
        ),
        ("rew_phase.txt", cal_fixture!("rew_phase.txt"), "rew", 0),
        ("csv_comma.csv", cal_fixture!("csv_comma.csv"), "csv", 0),
        (
            "csv_semicolon.csv",
            cal_fixture!("csv_semicolon.csv"),
            "csv",
            0,
        ),
        ("csv_tab.txt", cal_fixture!("csv_tab.txt"), "csv", 0),
    ];
    for (file, text, format, angle) in cases {
        let doc = mic_import("Mic".into(), (*file).into(), (*text).into())
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let d = MicDoc::from_text(&doc).unwrap();
        assert_eq!(serde_word(&d.mic.format), *format, "{file}");
        assert_eq!(d.mic.angle, *angle, "{file}");
        assert_eq!(d.mic.source_name, *file);
        assert_eq!(d.mic.source_hash.len(), 64);
        assert_eq!(d.mic.response.grid, LogGrid::STANDARD, "{file}");
        assert_eq!(d.meta.kind, "mic");
        assert!(!d.meta.saved_utc.is_empty());
        let info = measure_doc_info(doc).unwrap();
        assert_eq!((info.kind.as_str(), info.name.as_str()), ("mic", "Mic"));
    }
    let umik = MicDoc::from_text(
        &mic_import(
            "U".into(),
            "a.txt".into(),
            cal_fixture!("umik1_7000001.txt").into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(umik.mic.serial, "7000001");
    assert_eq!(umik.mic.vendor, "miniDSP");
    assert!(umik.mic.sensitivity_db.is_some() && umik.mic.approximate);
    // Refusals, in words.
    for bad in ["", "no numbers here", "\u{0}\u{1}", "1 2\nx y\n"] {
        assert!(matches!(
            mic_import("m".into(), "f.txt".into(), bad.into()),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    assert!(mic_import("m".into(), "big.txt".into(), "1 0\n".repeat(400_000)).is_err());
}

#[test]
fn wp_p5dock_target_import_and_builtins() {
    for (file, text) in [
        ("target_rew.txt", cal_fixture!("target_rew.txt")),
        ("compensation.csv", cal_fixture!("compensation.csv")),
        ("csv_comma.csv", cal_fixture!("csv_comma.csv")),
    ] {
        let doc = target_import(
            "T".into(),
            text.into(),
            "headphones".into(),
            "flatPlate".into(),
        )
        .unwrap_or_else(|e| panic!("{file}: {e}"));
        let d = TargetDoc::from_text(&doc).unwrap();
        assert_eq!(serde_word(&d.target.use_for), "headphones");
        assert_eq!(d.target.coupler, Some(Coupler::FlatPlate));
        assert!(matches!(d.target.definition, TargetDefinition::Imported(_)));
        d.target.to_target().unwrap();
    }
    // A speaker target keeps no coupler.
    let s = TargetDoc::from_text(
        &target_import(
            "S".into(),
            cal_fixture!("target_rew.txt").into(),
            "speakers".into(),
            "hats".into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(s.target.coupler, None);
    for (u, c) in [("room", ""), ("speakers", "ear"), ("", "")] {
        assert!(matches!(
            target_import(
                "x".into(),
                cal_fixture!("target_rew.txt").into(),
                u.into(),
                c.into()
            ),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    assert!(target_import("x".into(), "nope".into(), "speakers".into(), String::new()).is_err());

    let b = target_builtins();
    let ids: Vec<_> = b.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "flat",
            "tilt",
            "house",
            "coupler-iec60318-4",
            "coupler-flat-plate",
            "coupler-hats"
        ]
    );
    assert_eq!(b[0].target_use, "speakers");
    assert_eq!(b[3].target_use, "headphones");
    assert_eq!(b[3].coupler, "iec60318_4");
    assert_eq!(b[4].coupler, "flatPlate");
    let house: Vec<_> = b[2].params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        house,
        vec!["shelfCornerHz", "shelfGainDb", "shelfQ", "tiltDbPerOctave"]
    );
    // Each coupler word reads back as a coupler for an import.
    for t in &b[3..] {
        target_import(
            "c".into(),
            cal_fixture!("target_rew.txt").into(),
            "headphones".into(),
            t.coupler.clone(),
        )
        .unwrap();
    }
}

#[test]
fn wp_p5dock_measure_docs_on_a_core_without_a_folder_never_fail() {
    let core = DspiCore::new();
    for k in ["", "measurement", "mic", "target"] {
        assert_eq!(core.measure_docs(k.into(), "x".into()).unwrap(), vec![]);
    }
    assert!(matches!(
        core.measure_doc_get("1".into()),
        Err(DspiError::Io { .. })
    ));
    assert!(matches!(
        core.measure_doc_delete("1".into()),
        Err(DspiError::Io { .. })
    ));
}

mod proptests_wp_p5dock {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn proptest_wp_p5dock_imports_and_info_never_panic(text in ".{0,400}", word in "[a-zA-Z_0-9]{0,12}") {
            let _ = mic_import(word.clone(), word.clone(), text.clone());
            let _ = target_import(word.clone(), text.clone(), word.clone(), word.clone());
            let _ = measure_doc_info(text.clone());
            let _ = export_rew(&text, 0);
        }

        #[test]
        fn proptest_wp_p5dock_section_model_never_panics(t in any::<u8>(), bypass in any::<bool>(),
            freq in any::<f32>(), q in any::<f32>(), gain in any::<f32>(), qp in any::<f32>()) {
            let b = BandSpec { filter_type: t, bypass, freq, q, gain, qp };
            for c in CoreSectionModel.sections(&b) {
                prop_assert!([c.b0, c.b1, c.b2, c.a1, c.a2].iter().all(|x| x.is_finite()));
            }
        }
    }
}

/// WP P5DAYTON: a Dayton Audio file through `mic_import` gives a `dayton`
/// document whose dB SPL scale (what the session's plan, the live view and
/// the passive RTA read through `MicCal::spl_scale`) comes from its 1 kHz
/// sensitivity; a nonsense sensitivity is refused in words.
#[test]
fn wp_p5daytn_mic_import_reads_a_dayton_file() {
    let doc = MicDoc::from_text(
        &mic_import(
            String::new(),
            "dayton.txt".into(),
            cal_fixture!("dayton_imm6c_synthetic.txt").into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(doc.mic.format, dspi_measure::cal::CalFormat::Dayton);
    assert_eq!(doc.mic.vendor, "Dayton Audio");
    assert_eq!(doc.mic.name, "Dayton Audio");
    assert_eq!(doc.mic.sensitivity_db, Some(-38.5));
    assert!(doc.mic.approximate);
    let off = doc.mic.spl_scale().offset_db.unwrap();
    assert!((off - 132.5).abs() < 1e-9);
    assert!(matches!(
        mic_import(
            "m".into(),
            "f.txt".into(),
            "*1000Hz\tnan\r\n\r\n20\t1\r\n".into()
        ),
        Err(DspiError::InvalidArgument { .. })
    ));
}

// ── WP FEAT2 ────────────────────────────────────────────────────────────

#[test]
fn wp_feat2_a_built_in_target_as_a_document_and_a_microphones_spl() {
    for t in target_builtins() {
        let text = crate::target_builtin_document(&t.id).unwrap();
        let info = measure_doc_info(text.clone()).unwrap();
        assert_eq!((info.kind.as_str(), info.name), ("target", t.name));
        let d = dspi_measure::doc::TargetDoc::from_text(&text).unwrap();
        let back = d.target.to_target().unwrap();
        assert!(back.evaluate(&LogGrid::STANDARD).is_ok(), "{}", t.id);
    }
    assert!(crate::target_builtin_document("../flat").is_err());
    let mic = mic_import(
        "m".into(),
        "umik1_7000001.txt".into(),
        cal_fixture!("umik1_7000001.txt").into(),
    )
    .unwrap();
    assert!(crate::mic_document_has_spl(&mic).unwrap());
    assert!(crate::mic_document_has_spl(&crate::target_builtin_document("flat").unwrap()).is_err());
}
