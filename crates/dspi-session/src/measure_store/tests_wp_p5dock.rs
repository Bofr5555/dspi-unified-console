//! WP P5DOCK tests: the measurement store's files, ids, index repair, the
//! corrupt-file rule and the bounds (as P1SN's snapshot store tests).
//!
//! The store lives under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).

use std::fs;
use std::path::PathBuf;

use dspi_measure::cal::parse_calibration;
use dspi_measure::doc::{
    CaptureInfo, Curve, Measurement, MeasurementDoc, Meta, MicCal, MicDoc, OutputMeasurement,
    RunKind, Stimulus, Target, TargetDoc,
};
use dspi_measure::target::{Builtin, DEFAULT_REFERENCE_BAND, TargetDefinition, TargetUse};
use dspi_measure::types::{ChannelChain, LogGrid, Placement, ProgrammeFeed, Response, WayRole};
use serde_json::{Map, Value};

use super::*;

const UMIK: &str = include_str!("../../../dspi-measure/testdata/p5/cal/umik1_7000001.txt");

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p5dock-store").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&d);
    d
}

fn flat_curve() -> Curve {
    let n = LogGrid::STANDARD.freqs().unwrap().len();
    Curve::from_response(&Response {
        grid: LogGrid::STANDARD,
        mag_db: vec![80.0; n],
        phase_deg: Some(vec![0.0; n]),
        valid: Some(vec![true; n]),
        spl: true,
    })
}

pub(crate) fn measurement_text(serial: &str, name: &str, saved_ms: i64, id: &str) -> String {
    MeasurementDoc {
        schema_version: 1,
        meta: Meta::new(
            DocKind::Measurement,
            name,
            "2026-11-16T10:00:00Z",
            "test",
        ),
        measurement: Measurement {
            id: id.into(),
            serial: serial.into(),
            platform: "RP2350".into(),
            saved_ms,
            placement: Placement::ListeningPosition,
            mic: None,
            stimulus: Stimulus {
                kind: RunKind::Walked,
                f1_hz: 20.0,
                f2_hz: 20_000.0,
                duration_ms: 5_500,
                repeat: 3,
                gap_ms: 1_000,
                level_db: -30.0,
                device_rate_hz: 48_000,
                extra: Map::new(),
            },
            capture: CaptureInfo {
                rate_hz: 48_000,
                clock_ratio: 1.0,
                timing_reliable: true,
                max_residual_us: Some(2.0),
                extra: Map::new(),
            },
            outputs: vec![OutputMeasurement {
                output: 0,
                role: WayRole::Full,
                speaker: "L".into(),
                chain: ChannelChain {
                    peq: vec![],
                    crossover: vec![],
                    gain_db: 0.0,
                    delay_ms: 0.0,
                    feed: ProgrammeFeed {
                        gain_db: 0.0,
                        inverted: false,
                    },
                }
                .into(),
                arrival_ms: Some(3.0),
                response: flat_curve(),
                extra: Map::new(),
            }],
            positions: 1,
            note: String::new(),
            tunes: vec![],
            extra: Map::new(),
        },
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

fn mic_text(name: &str) -> String {
    let cal = parse_calibration(UMIK, "umik1_7000001.txt").unwrap();
    MicDoc {
        schema_version: 1,
        meta: Meta::new(DocKind::Mic, name, "2026-11-16T10:00:00Z", "test"),
        mic: MicCal::from_calibration(name, &cal, None, &LogGrid::STANDARD).unwrap(),
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

fn target_text(name: &str) -> String {
    let t = dspi_measure::target::Target {
        target_use: TargetUse::Speakers,
        coupler: None,
        definition: TargetDefinition::Builtin {
            builtin: Builtin::Flat,
        },
        reference: DEFAULT_REFERENCE_BAND,
    };
    TargetDoc {
        schema_version: 1,
        meta: Meta::new(DocKind::Target, name, "2026-11-16T10:00:00Z", "test"),
        target: Target::from_target(name, &t, &LogGrid::STANDARD).unwrap(),
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

#[test]
fn wp_p5dock_put_list_get_rename_note_delete() {
    let s = MeasureStore::new(store_dir("crud"));
    assert_eq!(s.list(None, None), vec![]);
    let (m, filed) = s
        .put(&measurement_text("DEMO", "Living room", 1_000, ""), 5)
        .unwrap();
    assert_eq!(m.id, id_for(1_000, 0));
    assert_eq!((m.kind.as_str(), m.serial.as_str()), ("measurement", "DEMO"));
    assert_eq!((m.name.as_str(), m.saved_ms), ("Living room", 1_000));
    assert!(filed.contains(&format!("\"id\": \"{}\"", m.id)), "{filed}");
    let (mic, _) = s.put(&mic_text("UMIK-1 “a” 🎤"), 2_000).unwrap();
    assert_eq!(mic.id, id_for(2_000, 0));
    assert_eq!((mic.kind.as_str(), mic.serial.as_str()), ("mic", ""));
    assert_eq!(mic.saved_ms, 2_000);
    let (t, _) = s.put(&target_text("Flat"), 2_000).unwrap();
    assert_eq!(t.id, id_for(2_000, 1), "unique in the whole store");

    // The folders.
    assert!(s.root().join("s-DEMO").join(format!("{}.dspipreset", m.id)).is_file());
    assert!(s.root().join(MIC_DIR).join(INDEX_FILE).is_file());
    assert!(s.root().join(TARGET_DIR).join(format!("{}.dspipreset", t.id)).is_file());

    let all: Vec<_> = s.list(None, None).into_iter().map(|e| e.id).collect();
    assert_eq!(all, vec![m.id.clone(), mic.id.clone(), t.id.clone()]);
    assert_eq!(s.list(Some(DocKind::Mic), None), vec![mic.clone()]);
    assert_eq!(s.list(Some(DocKind::Measurement), Some("OTHER")), vec![]);
    assert_eq!(s.list(Some(DocKind::Measurement), Some("DEMO")), vec![m.clone()]);
    // A serial is for measurements only.
    assert_eq!(s.list(Some(DocKind::Target), Some("DEMO")), vec![t.clone()]);

    let (e, text) = s.read(&mic.id).unwrap();
    assert_eq!(e, mic);
    assert_eq!(AnyDoc::parse(&text).unwrap().id(), mic.id);

    let r = s.rename(&m.id, "Kväll – 夜").unwrap();
    assert_eq!(r.name, "Kväll – 夜");
    let n = s.set_note(&m.id, "after the rug\nline 2").unwrap();
    assert_eq!((n.name.as_str(), n.note.as_str()), ("Kväll – 夜", "after the rug\nline 2"));
    let (_, text) = s.read(&m.id).unwrap();
    let d = AnyDoc::parse(&text).unwrap();
    assert_eq!((d.name(), d.note()), ("Kväll – 夜", "after the rug\nline 2"));
    // A name past the bound is cut, never refused.
    let long = s.rename(&t.id, &"x".repeat(1000)).unwrap();
    assert_eq!(long.name.chars().count(), dspi_measure::doc::MAX_NAME_CHARS);

    s.delete(&mic.id).unwrap();
    assert!(matches!(s.read(&mic.id), Err(e) if e.kind() == std::io::ErrorKind::NotFound));
    assert_eq!(s.list(Some(DocKind::Mic), None), vec![]);
    assert!(matches!(s.delete(&mic.id), Err(e) if e.kind() == std::io::ErrorKind::NotFound));
}

#[test]
fn wp_p5dock_ids_are_kept_when_free_and_never_paths() {
    let s = MeasureStore::new(store_dir("ids"));
    let (a, _) = s
        .put(&measurement_text("DEMO", "a", 1_000, "1763287200000"), 0)
        .unwrap();
    assert_eq!(a.id, "1763287200000");
    // The same document again: never an overwrite, a new id from its time.
    let (b, _) = s
        .put(&measurement_text("DEMO", "a", 1_000, "1763287200000"), 0)
        .unwrap();
    assert_eq!(b.id, id_for(1_000, 0));
    let (c, _) = s
        .put(&measurement_text("DEMO", "a", 1_000, "1763287200000"), 0)
        .unwrap();
    assert_eq!(c.id, id_for(1_000, 1));
    assert_eq!(s.list(None, None).len(), 3);
    for bad in ["", "../a", "a/b", "A", "-1", "a.b", "index"] {
        assert!(s.read(bad).is_err(), "{bad}");
    }
    assert!(matches!(s.read("../../etc/passwd"), Err(e) if e.kind() == std::io::ErrorKind::NotFound));
}

#[test]
fn wp_p5dock_a_torn_index_is_rebuilt_from_the_documents() {
    let s = MeasureStore::new(store_dir("torn"));
    let (a, _) = s.put(&measurement_text("DEMO", "a", 1_000, ""), 0).unwrap();
    let (b, _) = s.put(&measurement_text("DEMO", "b", 2_000, ""), 0).unwrap();
    let dir = s.root().join("s-DEMO");
    let want = s.list(None, None);
    for torn in [
        "{",
        "",
        "[]",
        "{\"documents\": 5}",
        // wrong kind, bad id, a repeat, a missing document, a wrong type
        r#"{"documents": [{"id": "x/y", "kind": "measurement"},
            {"id": "123", "kind": "mic"},
            {"id": "999", "kind": "measurement", "name": "gone"},
            {"id": 5}]}"#,
    ] {
        fs::write(dir.join(INDEX_FILE), torn).unwrap();
        assert_eq!(s.list(None, None), want, "{torn}");
        // Written back whole.
        let again = fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
        assert_eq!(parse_index(&again, DocKind::Measurement).len(), 2);
    }
    // A missing document drops its entry; a document the index lacks is added.
    fs::remove_file(dir.join(format!("{}.dspipreset", a.id))).unwrap();
    assert_eq!(s.list(None, None).len(), 1);
    fs::remove_file(dir.join(INDEX_FILE)).unwrap();
    assert_eq!(s.list(None, None)[0].id, b.id);
    assert_eq!(s.list(None, None)[0].name, "b");
}

#[test]
fn wp_p5dock_a_damaged_document_is_set_aside() {
    let s = MeasureStore::new(store_dir("corrupt"));
    let (a, _) = s.put(&measurement_text("DEMO", "a", 1_000, ""), 0).unwrap();
    let (b, _) = s.put(&measurement_text("DEMO", "b", 2_000, ""), 0).unwrap();
    let (m, _) = s.put(&mic_text("m"), 3_000).unwrap();
    let dir = s.root().join("s-DEMO");
    // A torn document the index names: refused and set aside on read.
    fs::write(dir.join(format!("{}.dspipreset", a.id)), "{\"schemaVersion\": 1,").unwrap();
    let e = s.read(&a.id).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{e}");
    assert!(e.to_string().contains("set aside"), "{e}");
    assert!(!dir.join(format!("{}.dspipreset", a.id)).exists());
    let aside: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| n.contains(".bad-"))
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(aside[0].starts_with(&format!("{}.dspipreset.bad-", a.id)));
    assert_eq!(s.list(None, None).len(), 2);
    // A document of another kind in a folder, the index not naming it: set
    // aside on the next list.
    fs::remove_file(dir.join(INDEX_FILE)).unwrap();
    fs::write(dir.join("5.dspipreset"), mic_text("stray")).unwrap();
    let ids: Vec<_> = s.list(None, None).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![b.id.clone(), m.id.clone()]);
    assert!(!dir.join("5.dspipreset").exists());
    // A document past its kind's bound.
    fs::write(
        s.root().join(MIC_DIR).join(format!("{}.dspipreset", m.id)),
        " ".repeat(dspi_measure::doc::MAX_MIC_BYTES + 1),
    )
    .unwrap();
    assert_eq!(s.read(&m.id).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(s.list(Some(DocKind::Mic), None), vec![]);
}

#[test]
fn wp_p5dock_an_id_two_folders_hold_is_refused() {
    let s = MeasureStore::new(store_dir("twice"));
    let (a, text) = s.put(&measurement_text("DEMO", "a", 1_000, ""), 0).unwrap();
    let other = s.root().join("s-OTHER");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join(format!("{}.dspipreset", a.id)), text).unwrap();
    for e in [
        s.read(&a.id).unwrap_err(),
        s.rename(&a.id, "x").unwrap_err(),
        s.set_note(&a.id, "x").unwrap_err(),
        s.delete(&a.id).unwrap_err(),
    ] {
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{e}");
    }
    // A put never takes it either.
    let (b, _) = s
        .put(&measurement_text("THIRD", "b", 1_000, &a.id), 0)
        .unwrap();
    assert_ne!(b.id, a.id);
}

#[test]
fn wp_p5dock_a_full_store_refuses_and_keeps_what_it_has() {
    let s = MeasureStore::new(store_dir("full"));
    for i in 0..MAX_MICS {
        s.put(&mic_text(&format!("m{i}")), 1_000 + i as i64).unwrap();
    }
    let e = s.put(&mic_text("one more"), 9_000).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::QuotaExceeded, "{e}");
    assert_eq!(s.list(Some(DocKind::Mic), None).len(), MAX_MICS);
    // Other kinds still file.
    s.put(&target_text("t"), 9_000).unwrap();
    // The measurements' count is per device.
    let dir = s.root().join(dir_name("DEMO"));
    fs::create_dir_all(&dir).unwrap();
    let one = measurement_text("DEMO", "m", 1, "");
    for i in 0..MAX_MEASUREMENTS_PER_DEVICE {
        fs::write(dir.join(format!("{}.dspipreset", id_for(10 + i as i64, 0))), &one).unwrap();
    }
    let e = s.put(&one, 0).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::QuotaExceeded, "{e}");
    s.put(&measurement_text("OTHER", "m", 1, ""), 0).unwrap();
}

#[test]
fn wp_p5dock_the_store_holds_at_most_64_mib() {
    let s = MeasureStore::new(store_dir("bytes"));
    // Filler files count, as documents filed by hand would.
    let dir = s.root().join(TARGET_DIR);
    fs::create_dir_all(&dir).unwrap();
    let f = fs::File::create(dir.join("1.dspipreset")).unwrap();
    f.set_len(MAX_STORE_BYTES - 100).unwrap();
    let e = s.put(&mic_text("m"), 0).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::QuotaExceeded, "{e}");
    assert!(!s.root().join(MIC_DIR).join(format!("{}.dspipreset", id_for(0, 0))).exists());
}

#[test]
fn wp_p5dock_put_refuses_what_is_not_a_measurement_document() {
    let s = MeasureStore::new(store_dir("refuse"));
    let preset = r#"{"schemaVersion": 1, "meta": {"kind": "preset"}, "channels": [{}]}"#;
    let snapshot = r#"{"schemaVersion": 1, "meta": {"kind": "snapshot"}, "snapshot": {}}"#;
    let mut big = measurement_text("DEMO", "a", 1, "");
    big.push_str(&" ".repeat(dspi_measure::doc::MAX_MEASUREMENT_BYTES));
    let mut no_serial: Value = serde_json::from_str(&measurement_text("DEMO", "a", 1, "")).unwrap();
    no_serial["measurement"]["serial"] = Value::from("");
    for bad in [
        "".to_string(),
        "{".into(),
        preset.into(),
        snapshot.into(),
        big,
        no_serial.to_string(),
        measurement_text("DEMO", "a", 1, "").replace("\"schemaVersion\": 1", "\"schemaVersion\": 2"),
    ] {
        let e = s.put(&bad, 0).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{e}");
    }
    assert_eq!(s.list(None, None), vec![]);
    assert!(!s.root().exists() || fs::read_dir(s.root()).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn wp_p5dock_files_are_the_users_alone() {
    use std::os::unix::fs::PermissionsExt;
    let s = MeasureStore::new(store_dir("modes"));
    let (m, _) = s.put(&mic_text("m"), 1).unwrap();
    let dir = s.root().join(MIC_DIR);
    let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(dir.join(format!("{}.dspipreset", m.id))), 0o600);
    assert_eq!(mode(dir.join(INDEX_FILE)), 0o600);
    assert_eq!(mode(dir.clone()), 0o700);
    // No temporary file is left behind.
    assert!(fs::read_dir(&dir).unwrap().flatten().all(|e| !e
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn wp_p5dock_saved_ms_of_an_id() {
    assert_eq!(saved_ms_of_id(&id_for(1_763_287_200_000, 0)), 1_763_287_200_000);
    assert_eq!(saved_ms_of_id(&id_for(5, 3)), 5);
    assert_eq!(saved_ms_of_id("abc"), 0);
    assert_eq!(saved_ms_of_id(&"9".repeat(40)), 999_999_999_999_999_999);
}
