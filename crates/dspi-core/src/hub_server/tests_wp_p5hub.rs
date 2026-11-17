//! WP P5HUB: the core's side of the hub's measurement documents without a hub
//! (the calls against the real hub are `crates/dspi-hub/tests/wp_p5hub_measurements.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use dspi_measure::doc::{MicCal, MicDoc};
use dspi_measure::types::LogGrid;
use serde_json::{Map, json};

use super::*;
use crate::hub_server::{
    HUB_SERVER_READS, HUB_SERVER_SETTLES, HubServerEvent, WHERE_BOTH, WHERE_HUB, WHERE_LOCAL,
};

fn info(id: &str, kind: &str, name: &str, saved_ms: i64) -> MeasureDocInfo {
    MeasureDocInfo {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        note: String::new(),
        serial: if kind == "measurement" {
            "DEV".into()
        } else {
            String::new()
        },
        saved_ms,
        bytes: 100,
    }
}

fn mic_text(id: &str) -> String {
    let umik = include_str!("../../../dspi-measure/testdata/p5/cal/umik1_7000001.txt");
    let cal = dspi_measure::cal::parse_calibration(umik, "umik1_7000001.txt").unwrap();
    let mut mic = MicCal::from_calibration("UMIK", &cal, None, &LogGrid::STANDARD).unwrap();
    mic.id = id.into();
    MicDoc {
        schema_version: 1,
        meta: dspi_measure::doc::Meta::new(DocKind::Mic, "UMIK", "2026-11-17T10:00:00Z", "test"),
        mic,
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

#[test]
fn wp_p5hub_split_parts_keeps_every_character_within_the_budget() {
    let text = "{\"a\":\"é😀\\n\"}".repeat(3000);
    let parts = split_parts(&text, MEASURE_WIRE_PART_BYTES);
    assert!(parts.len() > 1);
    assert_eq!(parts.concat(), text);
    for p in &parts {
        assert!(!p.is_empty());
        assert!(serde_json::to_string(p).unwrap().len() - 2 <= MEASURE_WIRE_PART_BYTES);
    }
    assert_eq!(split_parts("", 10), vec![""]);
}

#[test]
fn wp_p5hub_merge_prefers_the_hubs_copy() {
    let local = vec![
        info("1-0", "measurement", "same", 1),
        info("2-0", "measurement", "mine", 2),
        info("3-0", "mic", "only here", 3),
    ];
    let hub = vec![
        info("1-0", "measurement", "same", 1),
        info("2-0", "measurement", "renamed on the hub", 2),
        info("4-0", "target", "only on the hub", 4),
    ];
    let m = merge(local, hub);
    let rows: Vec<(&str, &str, &str, bool)> = m
        .iter()
        .map(|r| {
            (
                r.doc.id.as_str(),
                r.doc.name.as_str(),
                r.location.as_str(),
                r.local_copy,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("1-0", "same", WHERE_BOTH, false),
            ("2-0", "renamed on the hub", WHERE_HUB, false),
            ("2-0", "mine", WHERE_LOCAL, true),
            ("3-0", "only here", WHERE_LOCAL, false),
            ("4-0", "only on the hub", WHERE_HUB, false),
        ]
    );
    let v = serde_json::to_value(&m[0]).unwrap();
    assert_eq!(v["where"], "both");
    assert_eq!(v["doc"]["savedMs"], 1);
}

#[test]
fn wp_p5hub_hub_ready_gives_an_id_and_a_serial() {
    let (kind, v) = hub_ready(&mic_text(""), "DEV", 1_763_373_600_000).unwrap();
    assert_eq!(kind, DocKind::Mic);
    assert_eq!(v["mic"]["id"], "1763373600000");
    // One that has an id keeps it.
    let (_, v) = hub_ready(&mic_text("5-0"), "DEV", 9).unwrap();
    assert_eq!(v["mic"]["id"], "5-0");
    // Not a document of the family.
    for t in [
        "",
        "{}",
        r#"{"schemaVersion":1,"meta":{"kind":"snapshot"}}"#,
    ] {
        assert!(matches!(
            hub_ready(t, "DEV", 0),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
}

#[test]
fn wp_p5hub_doc_from_is_bounded() {
    let d = doc_from(
        &json!({"id": "1-0", "kind": "mic", "name": "n".repeat(1000),
        "note": "x".repeat(10_000), "savedMs": 5, "bytes": 7}),
    );
    assert_eq!(d.id, "1-0");
    assert!(d.name.chars().count() <= 300);
    assert_eq!(d.note.chars().count(), 4096);
    assert_eq!((d.saved_ms, d.bytes), (5, 7));
    let empty = doc_from(&serde_json::Value::Null);
    assert!(empty.id.is_empty());
}

#[test]
fn wp_p5hub_calls_on_no_hub_are_unsupported_and_send_nothing() {
    let core = DspiCore::new();
    assert!(!core.hub_caps().hub_measure);
    assert_eq!(
        core.hub_measure_docs(String::new(), String::new())
            .unwrap_err(),
        DspiError::Unsupported
    );
    assert_eq!(
        core.hub_measure_doc_get("1-0".into()).unwrap_err(),
        DspiError::Unsupported
    );
    assert_eq!(
        core.hub_measure_doc_put(mic_text("1-0")).unwrap_err(),
        DspiError::Unsupported
    );
    // What is not a document is refused before the cap is looked at.
    assert!(matches!(
        core.hub_measure_doc_put("{}".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(
        core.hub_measure_doc_rename("1-0".into(), "n".into())
            .unwrap_err(),
        DspiError::Unsupported
    );
    assert_eq!(
        core.hub_measure_doc_note("1-0".into(), "n".into())
            .unwrap_err(),
        DspiError::Unsupported
    );
    assert_eq!(
        core.hub_measure_doc_delete("1-0".into()).unwrap_err(),
        DspiError::Unsupported
    );
    assert!(matches!(
        core.hub_measure_docs("nope".into(), String::new()),
        Err(DspiError::InvalidArgument { .. })
    ));
    // The merged list without a hub is the local one, not listed, no notice.
    let m = core
        .measure_docs_merged(String::new(), String::new())
        .unwrap();
    assert!(m.entries.is_empty() && !m.hub_listed && m.hub_notice_key.is_empty());
    assert!(
        core.measure_docs_merged("nope".into(), String::new())
            .is_err()
    );
}

#[test]
fn wp_p5hub_the_family_reads_pass_the_read_only_mode() {
    for t in ["measure.doc.list", "measure.doc.get"] {
        assert!(HUB_SERVER_READS.contains(&t), "{t}");
    }
    for t in [
        "measure.doc.put",
        "measure.doc.rename",
        "measure.doc.note",
        "measure.doc.delete",
    ] {
        assert!(!HUB_SERVER_READS.contains(&t), "{t}");
        assert!(!HUB_SERVER_SETTLES.contains(&t), "{t}");
    }
}

#[test]
fn wp_p5hub_the_changed_event_is_read() {
    let e = HubServerEvent::from_hub(
        "measure.doc.changed",
        &json!({"v": 1, "hub": "H", "kind": "measurement", "serial": "DEV", "doc": "1-0",
                "change": "rename"}),
    )
    .unwrap();
    assert_eq!(e.kind, "measure.doc.changed");
    assert_eq!(e.measure_doc, "1-0");
    assert_eq!(e.measure_kind, "measurement");
    assert_eq!(e.serial, "DEV");
    assert_eq!(e.change, "rename");
    let s = HubServerEvent::from_hub(
        "snapshot.changed",
        &json!({"snapshot": "1-0", "kind": "x", "doc": "y", "change": "put"}),
    )
    .unwrap();
    assert!(s.measure_doc.is_empty() && s.measure_kind.is_empty());
}
