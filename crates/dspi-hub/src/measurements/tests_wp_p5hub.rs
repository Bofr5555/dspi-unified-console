//! WP P5HUB: the hub's measurement store and its messages, without a network (the
//! messages over the real hub with the core's calls are `tests/wp_p5hub_measurements.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_measure::doc::{
    CaptureInfo, Curve, DocKind, Measurement, MeasurementDoc, Meta, MicDoc, OutputMeasurement,
    RunKind, Stimulus,
};
use dspi_measure::types::{ChannelChain, LogGrid, Placement, ProgrammeFeed, Response, WayRole};
use dspi_session::measure_store::{MAX_MICS, dir_name, id_for};
use serde_json::{Map, Value, json};

use super::*;

fn store_dir(name: &str) -> PathBuf {
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("wp-p5hub-unit").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&d);
    d
}

fn curve() -> Curve {
    let n = LogGrid::STANDARD.freqs().unwrap().len();
    Curve::from_response(&Response {
        grid: LogGrid::STANDARD,
        mag_db: (0..n).map(|i| 80.0 + (i as f64 * 0.01).sin()).collect(),
        phase_deg: Some((0..n).map(|i| -(i as f64) * 0.3).collect()),
        valid: Some(vec![true; n]),
        spl: true,
    })
}

pub(crate) fn measurement_text(serial: &str, id: &str, name: &str) -> String {
    MeasurementDoc {
        schema_version: 1,
        meta: Meta::new(DocKind::Measurement, name, "2026-11-17T10:00:00Z", "test"),
        measurement: Measurement {
            id: id.into(),
            serial: serial.into(),
            platform: "RP2350".into(),
            saved_ms: 1_763_373_600_000,
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
                response: curve(),
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

fn mic_text(id: &str, name: &str) -> String {
    let umik = include_str!("../../../dspi-measure/testdata/p5/cal/umik1_7000001.txt");
    let cal = dspi_measure::cal::parse_calibration(umik, "umik1_7000001.txt").unwrap();
    let mut mic =
        dspi_measure::doc::MicCal::from_calibration(name, &cal, None, &LogGrid::STANDARD).unwrap();
    mic.id = id.into();
    MicDoc {
        schema_version: 1,
        meta: Meta::new(DocKind::Mic, name, "2026-11-17T10:00:00Z", "test"),
        mic,
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

fn session(role: Role, id: SessionId) -> Arc<Session> {
    Session::new(id, role, None, false)
}

fn ask(h: &HubMeasurements, s: &Arc<Session>, msg: Value) -> Value {
    serde_json::from_str(&h.handle_message(s, Instant::now(), &msg).unwrap()).unwrap()
}

#[test]
fn wp_p5hub_split_parts_cuts_at_characters_within_the_budget() {
    let text = "a\"b\\cé€😀\n".repeat(1000);
    for budget in [6usize, 7, 64, 1000, 32_768] {
        let parts = split_parts(&text, budget);
        let mut joined = String::new();
        let mut end = 0;
        for &(a, b) in &parts {
            assert_eq!(a, end, "contiguous");
            assert!(b > a, "never empty");
            let p = &text[a..b];
            assert!(serde_json::to_string(p).unwrap().len() - 2 <= budget);
            joined.push_str(p);
            end = b;
        }
        assert_eq!(joined, text);
    }
    assert_eq!(split_parts("", 10), vec![(0, 0)]);
}

#[test]
fn wp_p5hub_put_list_get_rename_note_delete_and_events() {
    let h = HubMeasurements::with_root(store_dir("crud"));
    let text = measurement_text("DEV1", "1763373600000-0", "Living room");
    let p = h.put(&text).unwrap();
    assert!(p.created);
    assert_eq!(p.entry.id, "1763373600000-0");
    assert_eq!(p.entry.kind, "measurement");
    assert_eq!(p.entry.serial, "DEV1");
    // The same document again.
    let again = h.put(&text).unwrap();
    assert!(!again.created);
    assert_eq!(again.entry, p.entry);
    // Listed by kind and serial only.
    assert_eq!(h.list(DocKind::Measurement, Some("DEV1")).unwrap().len(), 1);
    assert!(
        h.list(DocKind::Measurement, Some("DEV2"))
            .unwrap()
            .is_empty()
    );
    assert!(h.list(DocKind::Mic, None).unwrap().is_empty());
    assert_eq!(
        h.list(DocKind::Measurement, None).unwrap_err().code(),
        "bad_request"
    );
    // Compact on the wire, the same document.
    let (e, compact) = h.get(&p.entry.id).unwrap();
    assert_eq!(e, p.entry);
    assert!(!compact.contains('\n'));
    let a: Value = serde_json::from_str(&compact).unwrap();
    let b: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(a, b);

    let r = h.rename(&p.entry.id, "Kitchen ☕").unwrap();
    assert_eq!(r.name, "Kitchen ☕");
    let n = h.set_note(&p.entry.id, "after the rug").unwrap();
    assert_eq!(n.note, "after the rug");
    // The hub's copy changed: the client's old one is now a conflict.
    assert_eq!(h.put(&text).unwrap_err().code(), "conflict");
    assert_eq!(
        h.rename(&p.entry.id, &"x".repeat(257)).unwrap_err().code(),
        "bad_request"
    );
    assert_eq!(
        h.set_note(&p.entry.id, &"x".repeat(4097))
            .unwrap_err()
            .code(),
        "bad_request"
    );
    let d = h.delete(&p.entry.id).unwrap();
    assert_eq!(d.serial, "DEV1");
    assert_eq!(h.get(&p.entry.id).unwrap_err().code(), "not_found");
    assert_eq!(h.delete(&p.entry.id).unwrap_err().code(), "not_found");

    let events: Vec<Value> = h
        .take_changed("HUBID")
        .iter()
        .map(|t| serde_json::from_str(t).unwrap())
        .collect();
    let changes: Vec<&str> = events
        .iter()
        .map(|e| e["change"].as_str().unwrap())
        .collect();
    assert_eq!(changes, ["put", "rename", "note", "delete"]);
    for e in &events {
        assert_eq!(e["t"], CHANGED_EVENT);
        assert_eq!(e["v"], 1);
        assert_eq!(e["hub"], "HUBID");
        assert_eq!(e["kind"], "measurement");
        assert_eq!(e["serial"], "DEV1");
        assert_eq!(e["doc"], "1763373600000-0");
    }
    assert!(h.take_changed("HUBID").is_empty());
}

#[test]
fn wp_p5hub_the_hub_never_makes_an_id_and_never_overwrites() {
    let h = HubMeasurements::with_root(store_dir("ids"));
    // No id, or one the store would not make.
    for id in ["", "Not An Id", "../x"] {
        let mut v: Value = serde_json::from_str(&mic_text("", "UMIK")).unwrap();
        v["mic"]["id"] = json!(id);
        let e = h.put(&v.to_string()).unwrap_err();
        assert_eq!(e.code(), "bad_request", "{id}: {e}");
    }
    // A measurement names its device.
    let e = h.put(&measurement_text("", "1-0", "m")).unwrap_err();
    assert_eq!(e.code(), "bad_request", "{e}");
    // An id held as another kind, or for another device: conflict.
    h.put(&measurement_text("DEV1", "5-0", "m")).unwrap();
    assert_eq!(
        h.put(&mic_text("5-0", "UMIK")).unwrap_err().code(),
        "conflict"
    );
    assert_eq!(
        h.put(&measurement_text("DEV2", "5-0", "m"))
            .unwrap_err()
            .code(),
        "conflict"
    );
    // A different document with the same id: conflict, the hub's kept.
    assert_eq!(
        h.put(&measurement_text("DEV1", "5-0", "other name"))
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(h.get("5-0").unwrap().0.name, "m");
    // Not of the family at all.
    for text in [
        "not json",
        "{}",
        r#"{"schemaVersion":1,"meta":{"kind":"snapshot","name":"s"},"snapshot":{}}"#,
        r#"{"schemaVersion":1,"meta":{"kind":"preset","name":"p"}}"#,
    ] {
        assert_eq!(h.put(text).unwrap_err().code(), "bad_request", "{text}");
    }
}

#[test]
fn wp_p5hub_bounds_and_caps() {
    let h = HubMeasurements::with_root(store_dir("caps"));
    for i in 0..MAX_MICS {
        h.put(&mic_text(&id_for(1000 + i as i64, 0), &format!("m{i}")))
            .unwrap();
    }
    let e = h.put(&mic_text(&id_for(9000, 0), "one more")).unwrap_err();
    assert_eq!(e.code(), "conflict", "{e}");
    // A measurement past 1 MiB (padded with a key the reader keeps).
    let mut v: Value = serde_json::from_str(&measurement_text("DEV1", "7-0", "big")).unwrap();
    v["measurement"]["pad"] = json!("x".repeat(1 << 20));
    let e = h.put(&v.to_string()).unwrap_err();
    assert_eq!(e.code(), "bad_request", "{e}");
    // A text far past it is refused before it is parsed.
    let e = h.put(&" ".repeat((1 << 22) + 1)).unwrap_err();
    assert_eq!(e.code(), "bad_request", "{e}");
}

#[test]
fn wp_p5hub_a_damaged_document_is_set_aside_and_not_found() {
    let dir = store_dir("corrupt");
    let h = HubMeasurements::with_root(&dir);
    h.put(&measurement_text("DEV1", "8-0", "m")).unwrap();
    let path = dir.join(dir_name("DEV1")).join("8-0.dspipreset");
    fs::write(&path, "{ torn").unwrap();
    assert_eq!(h.get("8-0").unwrap_err().code(), "not_found");
    assert!(!path.exists());
    let aside = fs::read_dir(dir.join(dir_name("DEV1")))
        .unwrap()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("8-0.dspipreset.bad-")
        });
    assert!(aside);
    assert!(
        h.list(DocKind::Measurement, Some("DEV1"))
            .unwrap()
            .is_empty()
    );
    // The id is free again: the client's copy can be filed anew.
    assert!(
        h.put(&measurement_text("DEV1", "8-0", "m"))
            .unwrap()
            .created
    );
}

#[test]
fn wp_p5hub_messages_by_role() {
    let h = HubMeasurements::with_root(store_dir("roles"));
    let viewer = session(Role::Viewer, 0x8001);
    let control = session(Role::Control, 0x8002);
    let admin = session(Role::Admin, 0x8003);
    let doc = mic_text("11-0", "UMIK");
    let put = json!({"t": "measure.doc.put", "id": 1, "document": doc});
    assert_eq!(ask(&h, &viewer, put.clone())["code"], "denied");
    let a = ask(&h, &control, put.clone());
    assert_eq!(a["t"], "ok", "{a}");
    assert_eq!(a["v"], 1);
    assert_eq!(a["created"], true);
    assert_eq!(a["doc"]["id"], "11-0");
    assert_eq!(a["doc"]["kind"], "mic");
    // The document as the JSON object.
    let obj: Value = serde_json::from_str(&doc).unwrap();
    let a = ask(
        &h,
        &control,
        json!({"t": "measure.doc.put", "id": 2, "document": obj}),
    );
    assert_eq!(a["created"], false, "{a}");

    let a = ask(
        &h,
        &viewer,
        json!({"t": "measure.doc.list", "id": 3, "kind": "mic"}),
    );
    assert_eq!(a["docs"].as_array().unwrap().len(), 1, "{a}");
    assert_eq!(a["kind"], "mic");
    let a = ask(
        &h,
        &viewer,
        json!({"t": "measure.doc.list", "id": 4, "kind": "nope"}),
    );
    assert_eq!(a["code"], "bad_request");
    let a = ask(
        &h,
        &viewer,
        json!({"t": "measure.doc.get", "id": 5, "doc": "11-0"}),
    );
    assert_eq!(a["doc"]["name"], "UMIK", "{a}");
    assert!(a["document"].as_str().unwrap().contains("\"mic\""));

    for t in ["measure.doc.rename", "measure.doc.note"] {
        let msg = json!({"t": t, "id": 6, "doc": "11-0", "name": "n", "note": "n"});
        assert_eq!(ask(&h, &viewer, msg.clone())["code"], "denied");
        assert_eq!(ask(&h, &control, msg)["t"], "ok");
    }
    let del = json!({"t": "measure.doc.delete", "id": 7, "doc": "11-0"});
    assert_eq!(ask(&h, &viewer, del.clone())["code"], "denied");
    assert_eq!(ask(&h, &control, del.clone())["code"], "denied");
    let a = ask(&h, &admin, del.clone());
    assert_eq!(a["doc"], "11-0", "{a}");
    assert_eq!(a["kind"], "mic");
    assert_eq!(ask(&h, &admin, del)["code"], "not_found");
    // Not ours, and a missing id.
    assert!(
        h.handle_message(
            &admin,
            Instant::now(),
            &json!({"t": "snapshot.list", "id": 1})
        )
        .is_none()
    );
    let a = ask(&h, &admin, json!({"t": "measure.doc.list", "kind": "mic"}));
    assert_eq!(a["code"], "bad_request");
}

#[test]
fn wp_p5hub_parts_round_trip_and_their_refusals() {
    let h = HubMeasurements::with_root(store_dir("parts"));
    let control = session(Role::Control, 0x8100);
    let mut v: Value = serde_json::from_str(&measurement_text("DEV1", "21-0", "big")).unwrap();
    v["measurement"]["pad"] = json!("y".repeat(200_000));
    let compact = v.to_string();
    let parts = split_parts(&compact, PART_BYTES);
    assert!(parts.len() >= 7, "{}", parts.len());
    let now = Instant::now();
    let mut last_answer = Value::Null;
    for (i, &(a, b)) in parts.iter().enumerate() {
        let msg = json!({"t": "measure.doc.put", "id": i, "seq": i,
            "last": i + 1 == parts.len(), "part": &compact[a..b]});
        last_answer =
            serde_json::from_str(&h.handle_message(&control, now, &msg).unwrap()).unwrap();
        assert_eq!(last_answer["t"], "ok", "{last_answer}");
        assert_eq!(last_answer["received"], b);
    }
    assert_eq!(last_answer["created"], true);
    assert_eq!(h.uploads_open(), 0);

    // Get: too large for one frame, so parts, with one hash and size.
    let first = ask(
        &h,
        &control,
        json!({"t": "measure.doc.get", "id": 1, "doc": "21-0"}),
    );
    assert!(first.get("document").is_none());
    assert_eq!(first["seq"], 0);
    let mut text = String::new();
    let mut seq = 0u64;
    loop {
        let a = ask(
            &h,
            &control,
            json!({"t": "measure.doc.get", "id": 2, "doc": "21-0", "seq": seq}),
        );
        assert!(a.to_string().len() <= MAX_FRAME);
        assert_eq!(a["sha256"], first["sha256"]);
        text.push_str(a["part"].as_str().unwrap());
        if a["last"] == true {
            break;
        }
        seq += 1;
    }
    assert_eq!(text, compact);
    assert_eq!(first["size"], compact.len());
    assert_eq!(first["sha256"], sha256_hex(&compact));
    let a = ask(
        &h,
        &control,
        json!({"t": "measure.doc.get", "id": 3, "doc": "21-0", "seq": seq + 1}),
    );
    assert_eq!(a["code"], "bad_request");

    // Out of order: dropped.
    let p = |seq: u64, last: bool| json!({"t": "measure.doc.put", "id": 9, "seq": seq, "last": last, "part": "{"});
    assert_eq!(ask(&h, &control, p(0, false))["t"], "ok");
    assert_eq!(ask(&h, &control, p(2, false))["code"], "bad_request");
    assert_eq!(h.uploads_open(), 0);
    assert_eq!(ask(&h, &control, p(1, false))["code"], "bad_request");
    // A whole text that is not a document: checked only when whole.
    assert_eq!(ask(&h, &control, p(0, false))["t"], "ok");
    assert_eq!(ask(&h, &control, p(1, true))["code"], "bad_request");
    // Past 1 MiB.
    let big = "z".repeat(600_000);
    let part = |seq: u64| json!({"t": "measure.doc.put", "id": 9, "seq": seq, "part": big});
    assert_eq!(ask(&h, &control, part(0))["t"], "ok");
    assert_eq!(ask(&h, &control, part(1))["code"], "bad_request");
    assert_eq!(h.uploads_open(), 0);
    // A viewer may not upload.
    let viewer = session(Role::Viewer, 0x8101);
    assert_eq!(ask(&h, &viewer, p(0, false))["code"], "denied");
}

#[test]
fn wp_p5hub_uploads_are_bounded_and_expire() {
    let h = HubMeasurements::with_root(store_dir("uploads"));
    let now = Instant::now();
    let sessions: Vec<Arc<Session>> = (0..=MAX_UPLOADS as u16)
        .map(|i| session(Role::Control, 0x8200 + i))
        .collect();
    let msg = json!({"t": "measure.doc.put", "id": 1, "seq": 0, "part": "{"});
    for s in &sessions[..MAX_UPLOADS] {
        let a: Value = serde_json::from_str(&h.handle_message(s, now, &msg).unwrap()).unwrap();
        assert_eq!(a["t"], "ok");
    }
    let a: Value =
        serde_json::from_str(&h.handle_message(&sessions[MAX_UPLOADS], now, &msg).unwrap())
            .unwrap();
    assert_eq!(a["code"], "busy");
    // A session's upload starts again with seq 0 and stays one.
    let a: Value =
        serde_json::from_str(&h.handle_message(&sessions[0], now, &msg).unwrap()).unwrap();
    assert_eq!(a["t"], "ok");
    assert_eq!(h.uploads_open(), MAX_UPLOADS);
    // After the timeout every one is gone.
    let later = now + UPLOAD_TIMEOUT + Duration::from_secs(1);
    let a: Value = serde_json::from_str(
        &h.handle_message(&sessions[MAX_UPLOADS], later, &msg)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(a["t"], "ok");
    assert_eq!(h.uploads_open(), 1);
    // A session that ended leaves nothing behind.
    let gone = session(Role::Control, 0x8300);
    let _ = h.handle_message(&gone, later, &msg);
    assert_eq!(h.uploads_open(), 2);
    drop(gone);
    let _ = h.handle_message(&sessions[1], later, &msg);
    assert_eq!(h.uploads_open(), 2);
}

proptest::proptest! {
    #[test]
    fn proptest_wp_p5hub_messages_never_panic(
        t in proptest::sample::select(MESSAGES.to_vec()),
        doc in ".{0,40}",
        part in ".{0,200}",
        seq in proptest::option::of(0u64..5),
        kind in "[a-z]{0,12}",
    ) {
        let h = HubMeasurements::with_root(store_dir("prop"));
        let s = session(Role::Admin, 0x8400);
        let msg = json!({"t": t, "id": 1, "doc": doc, "kind": kind, "serial": doc,
            "seq": seq, "part": part, "name": part, "note": part, "document": part});
        let a: Value = serde_json::from_str(&h.handle_message(&s, Instant::now(), &msg).unwrap()).unwrap();
        proptest::prop_assert!(a["t"] == "ok" || a["t"] == "err");
    }
}
