//! WP P1SN tests: the snapshot store's files, index repair and names.
//!
//! The store lives under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1sn-store").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&d);
    d
}

fn doc(name: &str) -> Value {
    json!({
        "schemaVersion": 1,
        "meta": { "name": name, "platform": "RP2350", "inputChannelCount": 2,
                  "outputChannelCount": 9, "wireFormatVersion": 32 },
        "channels": [{ "channelId": "x" }],
        "snapshot": { "note": "n", "wireFormat": 32 },
    })
}

#[test]
fn wp_p1sn_dir_names_round_trip_like_the_event_log() {
    assert_eq!(dir_name("DSPI-01_a"), "s-DSPI-01_a");
    assert_eq!(serial_of_dir("s-DSPI-01_a").as_deref(), Some("DSPI-01_a"));
    for serial in ["", "a/b", "../x", "Ünï 🎛", "a b"] {
        let d = dir_name(serial);
        assert!(d.starts_with("x-"), "{d}");
        assert!(d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        if !serial.is_empty() {
            assert_eq!(serial_of_dir(&d).as_deref(), Some(serial));
        }
    }
    assert_eq!(serial_of_dir("x-zz"), None);
    assert_eq!(serial_of_dir("index.json"), None);
}

#[test]
fn wp_p1sn_ids_are_never_paths() {
    for bad in ["", "../a", "a/b", "A", "-1", "a.b", &"1".repeat(65)] {
        assert!(!valid_id(bad), "{bad}");
    }
    assert!(valid_id("0001731600000000"));
    assert!(valid_id("0001731600000000-2"));
}

#[test]
fn wp_p1sn_save_list_rename_note_delete() {
    let root = store_dir("crud");
    let s = SnapshotStore::new(&root);
    assert_eq!(s.list("DEMO"), vec![]);
    let mut d = doc("Évening 🎛 “quoted” \\ / \u{202e}rtl\n");
    let a = s.save("DEMO", &mut d, 1_000).unwrap();
    assert_eq!(d["meta"]["kind"], "snapshot");
    assert_eq!(d["snapshot"]["id"], json!(a.id));
    assert_eq!(a.name, "Évening 🎛 “quoted” \\ / \u{202e}rtl\n");
    assert_eq!(a.note, "n");
    assert_eq!((a.serial.as_str(), a.saved_ms), ("DEMO", 1_000));
    assert_eq!((a.input_channels, a.output_channels), (2, 9));
    assert_eq!((a.platform.as_str(), a.wire_format), ("RP2350", 32));
    // Same millisecond: a second id.
    let b = s.save("DEMO", &mut doc("b"), 1_000).unwrap();
    assert_ne!(a.id, b.id);
    let c = s.save("DEMO", &mut doc("c"), 500).unwrap();
    let ids: Vec<_> = s.list("DEMO").into_iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![c.id.clone(), a.id.clone(), b.id.clone()]);
    assert_eq!(s.serials(), vec!["DEMO".to_string()]);

    let r = s.rename(&a.id, "Nacht – 夜").unwrap();
    assert_eq!(r.name, "Nacht – 夜");
    let n = s.set_note(&a.id, "für später").unwrap();
    assert_eq!(
        (n.name.as_str(), n.note.as_str()),
        ("Nacht – 夜", "für später")
    );
    let (e, text) = s.read(&a.id).unwrap();
    assert_eq!(e, n);
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["meta"]["name"], "Nacht – 夜");
    assert_eq!(v["snapshot"]["note"], "für später");

    s.delete(&b.id).unwrap();
    assert_eq!(s.list("DEMO").len(), 2);
    assert_eq!(
        s.read(&b.id).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        s.rename("../x", "y").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    // Another serial, with characters a folder name cannot hold.
    let o = s.save("dev/ü", &mut doc("o"), 2).unwrap();
    assert_eq!(s.list("dev/ü"), vec![o]);
    assert_eq!(s.serials().len(), 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn wp_p1sn_a_corrupt_index_never_fails_a_list() {
    let root = store_dir("corrupt");
    let s = SnapshotStore::new(&root);
    let a = s.save("K340", &mut doc("a"), 10).unwrap();
    let b = s.save("K340", &mut doc("b"), 20).unwrap();
    let dir = root.join(dir_name("K340"));
    let index = dir.join(INDEX_FILE);
    let good = fs::read_to_string(&index).unwrap();
    // Torn mid-write, empty, not JSON, the wrong shape, hand edits.
    let cuts = [
        good[..good.len() / 2].to_string(),
        String::new(),
        "\u{feff}garbage".into(),
        "[]".into(),
        r#"{"snapshots": 5}"#.into(),
        format!(
            r#"{{"version": 9, "snapshots": [
                {{"id": "{}", "name": "edited by hand", "savedMs": "soon"}},
                {{"id": "../../etc", "name": "evil"}},
                {{"id": "{}", "name": "hand name", "savedMs": 20}},
                {{"id": "{}", "name": "twice"}},
                {{"id": "99999", "name": "gone"}},
                42
            ]}}"#,
            a.id, b.id, b.id
        ),
    ];
    for (k, text) in cuts.iter().enumerate() {
        fs::write(&index, text).unwrap();
        let l = s.list("K340");
        let ids: Vec<_> = l.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec![a.id.as_str(), b.id.as_str()], "case {k}");
        if k == 5 {
            // A readable hand edit is kept; the bad entry's twin is rebuilt.
            assert_eq!(l[1].name, "hand name");
            assert_eq!(l[0].name, "a");
        }
        // Repaired on disk.
        assert_eq!(parse_index(&fs::read_to_string(&index).unwrap()), l);
    }
    // No index at all, and a torn document no index names.
    fs::remove_file(&index).unwrap();
    fs::write(dir.join("123.dspipreset"), "{\"schemaVer").unwrap();
    fs::write(dir.join("notes.txt"), "x").unwrap();
    assert_eq!(s.list("K340").len(), 2);
    // An index that is a folder.
    fs::remove_file(&index).unwrap();
    fs::create_dir(&index).unwrap();
    assert_eq!(s.list("K340").len(), 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn wp_p1sn_entry_from_a_document_is_lenient() {
    let e = entry_from_document("1", &json!({ "meta": { "name": 5 }, "snapshot": [] }));
    assert_eq!(e.id, "1");
    assert_eq!(e.name, "");
    assert_eq!(e.wire_format, 0);
    let e = entry_from_document("2", &doc("x"));
    assert_eq!(e.wire_format, 32);
}
