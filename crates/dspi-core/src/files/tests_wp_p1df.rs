//! P1DF tests: the document diff (PRO1 W5), import per block (W8) and
//! `meta.kind`, on fixtures and on the demo device.

use std::sync::Arc;

use super::*;
use crate::state::{apply_platform, decode_bulk, default_state};

const PLATFORM: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/platform.bin");
const BULK: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/bulk_v32.bin");
/// The demo device as `config_export` writes it: the code the Qt console (on
/// Windows and Linux) runs.
const QT: &str = include_str!("../../../../tests/fixtures/preset/qt-core-v32.dspipreset");
/// The Mac console's document of the demo device, with a tuning on top.
const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
/// The owner's K340, exported by the Terminal.
const K340: &str = include_str!("../../../../tests/fixtures/preset/k340-bug1.dspipreset");

type RowT = (String, String, String, String, String);

/// The demo device's state (RP2350, V32), without a connection.
fn capture() -> DspState {
    let mut s = default_state();
    apply_platform(PLATFORM, &mut s).unwrap();
    decode_bulk(BULK, &mut s).unwrap();
    s
}

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    core
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn export(core: &DspiCore) -> String {
    core.config_export("x".into(), "1".into(), 0).unwrap()
}

/// `(block, section, item, before, after)` of each row.
fn rows(reply: &Value) -> Vec<RowT> {
    assert_eq!(reply["ok"], json!(true), "{reply}");
    reply["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let f = |k: &str| r[k].as_str().unwrap().to_owned();
            (f("block"), f("section"), f("item"), f("before"), f("after"))
        })
        .collect()
}

fn has_row(rows: &[RowT], item: &str, before: &str, after: &str) -> bool {
    rows.iter()
        .any(|r| r.2 == item && r.3 == before && r.4 == after)
}

/// The Mac document with a change in the two blocks it leaves as the demo
/// has them: a channel name and a hardware value.
fn tuned() -> String {
    let mut v = json_of(MAC);
    v["channels"][0]["name"] = json!("Turntable");
    v["io"]["mckPin"] = json!(21);
    v.to_string()
}

const BLOCK_BITS: [(u32, &str); 9] = [
    (IMPORT_EQ, "eq"),
    (IMPORT_CROSSOVER, "crossover"),
    (IMPORT_ROUTING, "routing"),
    (IMPORT_OUTPUTS, "outputs"),
    (IMPORT_LIMITERS, "limiters"),
    (IMPORT_EFFECTS, "effects"),
    (IMPORT_VOLUMES, "volumes"),
    (IMPORT_HARDWARE, "hardware"),
    (IMPORT_NAMES, "names"),
];

// ── W5: the diff ──

#[test]
fn p1df_a_document_against_itself_differs_in_nothing() {
    for doc in [QT, MAC, K340] {
        let r = json_of(&config_diff(doc.into(), doc.into()));
        assert_eq!(rows(&r), vec![], "{r}");
        assert_eq!(r["count"], json!(0));
        assert_eq!(r["summary"], json!([]));
        for (_, key) in BLOCK_BITS {
            assert_eq!(r["blocks"][key], json!(0), "{key}");
        }
    }
}

#[test]
fn p1df_doc_vs_doc_names_each_changed_value_in_the_preset_words() {
    let r = json_of(&config_diff(QT.into(), MAC.into()));
    let rows = rows(&r);
    // The Mac fixture's tuning (gen_mac_doc.py), one row per value.
    for (item, before, after) in [
        ("Master Volume", "-20.0 dB", "-18.0 dB"),
        ("Loudness", "disabled", "enabled"),
        ("Loudness intensity", "100%", "60%"),
        ("Preamp L", "0.0 dB", "-3.0 dB"),
        ("Crossfeed", "disabled", "enabled"),
        ("Psybass cutoff", "80 Hz", "90 Hz"),
    ] {
        assert!(has_row(&rows, item, before, after), "{item}: {rows:#?}");
    }
    let block_of = |item: &str| rows.iter().find(|r| r.2 == item).map(|r| r.0.clone());
    assert_eq!(block_of("Master Volume").as_deref(), Some("volumes"));
    assert_eq!(block_of("Preamp L").as_deref(), Some("routing"));
    assert_eq!(block_of("Loudness").as_deref(), Some("effects"));
    // Bands one row each, with the band in words.
    assert!(
        rows.iter().any(|r| r.0 == "eq"
            && r.2.ends_with("band 1")
            && r.4.starts_with("Peaking 100 Hz, Q 1.25, 3.0 dB")),
        "{rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.0 == "eq" && r.4.ends_with("(bypassed)"))
    );
    assert!(
        rows.iter()
            .any(|r| r.0 == "crossover" && r.2.ends_with("band 1"))
    );
    assert!(
        rows.iter()
            .any(|r| r.0 == "limiters" && r.2.ends_with("limiter threshold"))
    );
    assert!(
        rows.iter()
            .any(|r| r.0 == "routing" && r.1 == "Matrix" && r.4 == "on, -6.0 dB, inverted")
    );
    assert!(
        rows.iter()
            .any(|r| r.0 == "outputs" && r.2.ends_with("gain"))
    );
    // The summary is the unsaved-changes list's sentences.
    let summary: Vec<String> = serde_json::from_value(r["summary"].clone()).unwrap();
    assert!(
        summary.contains(&"Loudness: enabled".to_owned()),
        "{summary:?}"
    );
    assert!(
        summary.iter().any(|s| s.contains("crosspoint")),
        "{summary:?}"
    );
    // The counts per block add up.
    let total: u64 = BLOCK_BITS
        .iter()
        .map(|(_, k)| r["blocks"][k].as_u64().unwrap())
        .sum();
    assert_eq!(total, rows.len() as u64);
    assert_eq!(r["count"], json!(rows.len()));
}

#[test]
fn p1df_doc_vs_demo_is_the_doc_vs_doc_diff() {
    let core = demo();
    let all = IMPORT_VOLUMES | IMPORT_HARDWARE;
    let preview = json_of(&core.config_preview(MAC.into(), all, u32::MAX));
    let docs = json_of(&config_diff(QT.into(), MAC.into()));
    let mut on_device = rows(&preview);
    let mut between = rows(&docs);
    on_device.sort();
    between.sort();
    assert_eq!(on_device, between);
    // The demo's own document changes nothing on it.
    let same = json_of(&core.config_preview(QT.into(), all, u32::MAX));
    assert_eq!(rows(&same), vec![]);
}

#[test]
fn p1df_a_preview_writes_nothing() {
    let core = demo();
    let before = export(&core);
    let all = IMPORT_VOLUMES | IMPORT_HARDWARE;
    let r = json_of(&core.config_preview(MAC.into(), all, u32::MAX));
    assert!(r["count"].as_u64().unwrap() > 0);
    let d = json_of(&config_diff(before, export(&core)));
    assert_eq!(rows(&d), vec![]);
    assert!(core.preset_changes().is_empty());
}

#[test]
fn p1df_a_preview_says_what_the_import_then_does() {
    let doc = tuned();
    for (bit, key) in BLOCK_BITS {
        let core = demo();
        let options = IMPORT_BY_BLOCK | bit;
        let preview = json_of(&core.config_preview(doc.clone(), options, u32::MAX));
        let before = export(&core);
        let r = json_of(&core.config_import_selected(doc.clone(), options, u32::MAX));
        assert_eq!(r["ok"], json!(true), "{key}: {r}");
        let real = json_of(&config_diff(before, export(&core)));
        let (mut p, mut d) = (rows(&preview), rows(&real));
        p.sort();
        d.sort();
        assert_eq!(p, d, "{key}");
    }
}

#[test]
fn p1df_preview_and_diff_refuse_what_is_no_document() {
    let r = json_of(&config_diff("{".into(), QT.into()));
    assert_eq!(r["ok"], json!(false));
    assert!(r["error"].as_str().unwrap().starts_with("First document"));
    let r = json_of(&config_diff(QT.into(), "[]".into()));
    assert!(r["error"].as_str().unwrap().starts_with("Second document"));
    let core = DspiCore::new();
    let r = json_of(&core.config_preview(QT.into(), 0, u32::MAX));
    assert_eq!(r["ok"], json!(false));
    assert_eq!(r["error"], json!("No device connected."));
}

// ── W8: per block ──

#[test]
fn p1df_each_block_alone_changes_only_its_values() {
    let doc = tuned();
    for (bit, key) in BLOCK_BITS {
        let core = demo();
        let before = export(&core);
        let r = json_of(&core.config_import_selected(doc.clone(), IMPORT_BY_BLOCK | bit, u32::MAX));
        assert_eq!(r["ok"], json!(true), "{key}: {r}");
        assert_eq!(r["blocks"], json!(bit), "{key}");
        let d = json_of(&config_diff(before, export(&core)));
        let rows = rows(&d);
        assert!(!rows.is_empty(), "{key}: the document changes this block");
        for row in &rows {
            assert_eq!(row.0, key, "{key} applied alone changed {row:?}");
        }
        assert_eq!(d["blocks"][key], json!(rows.len()), "{key}");
    }
}

#[test]
fn p1df_no_block_changes_nothing() {
    let core = demo();
    let before = export(&core);
    let r = json_of(&core.config_import_selected(tuned(), IMPORT_BY_BLOCK, u32::MAX));
    assert_eq!(r["ok"], json!(true));
    let d = json_of(&config_diff(before, export(&core)));
    assert_eq!(rows(&d), vec![]);
}

#[test]
fn p1df_eq_per_channel() {
    let core = demo();
    let was = json_of(&export(&core));
    // Wire channels 1 and 9 only.
    let mask = (1 << 1) | (1 << 9);
    let options = IMPORT_BY_BLOCK | IMPORT_EQ;
    let r = json_of(&core.config_import_selected(MAC.into(), options, mask));
    assert_eq!(r["ok"], json!(true));
    let after = json_of(&export(&core));
    let mac = json_of(MAC);
    let eq_of = |v: &Value, wire: usize| {
        v["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["eqChannel"] == json!(wire))
            .unwrap()["eq"]
            .clone()
    };
    for wire in 0..17 {
        if wire == 1 || wire == 9 {
            assert_eq!(
                eq_of(&after, wire)[0]["freqHz"].as_f64(),
                eq_of(&mac, wire)[0]["freqHz"].as_f64(),
                "{wire}"
            );
            assert_ne!(eq_of(&after, wire), eq_of(&was, wire), "{wire}");
        } else {
            assert_eq!(eq_of(&after, wire), eq_of(&was, wire), "{wire}");
        }
    }
    // The preview says the same.
    let core = demo();
    let p = json_of(&core.config_preview(MAC.into(), options, mask));
    let rows = rows(&p);
    assert_eq!(
        rows.len(),
        4,
        "two bands on each of two channels: {rows:#?}"
    );
    assert!(rows.iter().all(|r| r.0 == "eq"));
}

#[test]
fn p1df_old_option_bits_keep_their_meaning() {
    let base = capture();
    let doc = json_of(&tuned());
    let audio = IMPORT_EQ
        | IMPORT_CROSSOVER
        | IMPORT_ROUTING
        | IMPORT_OUTPUTS
        | IMPORT_EFFECTS
        | IMPORT_NAMES;
    for (old, blocks) in [
        (0, audio),
        (IMPORT_VOLUMES, audio | IMPORT_VOLUMES),
        (IMPORT_HARDWARE, audio | IMPORT_HARDWARE | IMPORT_LIMITERS),
        (IMPORT_VOLUMES | IMPORT_HARDWARE, IMPORT_ALL_BLOCKS),
    ] {
        assert_eq!(Selection::new(old, u32::MAX).blocks(), blocks, "{old}");
        let a = build_config(&base, &doc, old);
        let b = build_selected(
            &base,
            &doc,
            Selection::new(IMPORT_BY_BLOCK | blocks, u32::MAX),
        );
        assert_eq!(a.state, b.state, "{old}");
        assert_eq!(a.left_alone, b.left_alone, "{old}");
        assert_eq!(config_report(&a), config_report(&b), "{old}");
    }
    // Without the volumes bit the volumes stay; without the hardware bit the
    // limiters and the I/O stay.
    let a = build_config(&base, &doc, 0);
    assert_eq!(a.state.master_volume_db, base.master_volume_db);
    assert_eq!(a.state.limiter_threshold_db, base.limiter_threshold_db);
    assert_eq!(a.state.mck_pin, base.mck_pin);
    let all = build_config(&base, &doc, IMPORT_VOLUMES | IMPORT_HARDWARE);
    assert_eq!(all.state.master_volume_db, -18.0);
    assert_eq!(all.state.mck_pin, 21);
    // The constants: the old two unchanged, the block bits apart from the
    // mode bit.
    assert_eq!((IMPORT_VOLUMES, IMPORT_HARDWARE), (1, 2));
    assert_eq!(IMPORT_ALL_BLOCKS & IMPORT_BY_BLOCK, 0);
    let bits: u32 = BLOCK_BITS.iter().map(|(b, _)| b).sum();
    assert_eq!(bits, IMPORT_ALL_BLOCKS);
}

#[test]
fn p1df_old_option_bits_through_config_import() {
    let core = demo();
    let r = json_of(&core.config_import(MAC.into(), 0));
    assert_eq!(r["ok"], json!(true));
    let audio = IMPORT_EQ
        | IMPORT_CROSSOVER
        | IMPORT_ROUTING
        | IMPORT_OUTPUTS
        | IMPORT_EFFECTS
        | IMPORT_NAMES;
    assert_eq!(r["blocks"], json!(audio));
    let after = json_of(&export(&core));
    assert_eq!(after["global"]["masterVolumeDb"], json!(-20.0));
    assert_eq!(after["loudness"]["enabled"], json!(true));
}

// ── meta.kind ──

#[test]
fn p1df_kind_is_read_and_unknown_reads_as_preset() {
    let with = |kind: Value| {
        let mut v = json_of(QT);
        v["meta"]["kind"] = kind;
        v.to_string()
    };
    for (kind, read) in [
        (json!("snapshot"), "snapshot"),
        (json!("template"), "template"),
        (json!("limits"), "limits"),
        (json!("Preset"), "preset"),
        (json!("scene-v9"), "preset"),
        (json!(7), "preset"),
        (Value::Null, "preset"),
    ] {
        let text = with(kind.clone());
        let i = json_of(&config_inspect(text.clone()));
        assert_eq!(i["ok"], json!(true));
        assert_eq!(i["kind"], json!(read), "{kind}");
        // It imports, as any preset does.
        let core = demo();
        let r = json_of(&core.config_import(text.clone(), IMPORT_VOLUMES | IMPORT_HARDWARE));
        assert_eq!(r["ok"], json!(true), "{kind}: {r}");
        assert_eq!(json_of(&config_diff(QT.into(), text))["kind"], json!(read));
    }
    // The Terminal's reader too.
    let doc = preset_file::parse(&with(json!("scene-v9"))).unwrap();
    assert_eq!(doc.meta.doc_kind(), MetaKind::Preset);
    let doc = preset_file::parse(&with(json!("snapshot"))).unwrap();
    assert_eq!(doc.meta.doc_kind(), MetaKind::Snapshot);
}

#[test]
fn p1df_kind_is_written_for_every_kind_but_a_preset() {
    let s = capture();
    let mut preset = json_of(&export_document(&s, "x", "1", 0, None, MetaKind::Preset));
    assert!(preset["meta"].get("kind").is_none());
    assert!(
        json_of(&export_config(&s, "x", "1", 0, None))["meta"]
            .get("kind")
            .is_none()
    );
    preset["meta"]["savedUtc"] = json!("");
    for kind in [MetaKind::Snapshot, MetaKind::Template, MetaKind::Limits] {
        let text = export_document(&s, "x", "1", 0, None, kind);
        let mut v = json_of(&text);
        assert_eq!(v["meta"]["kind"], json!(kind.word()));
        assert_eq!(
            json_of(&config_inspect(text.clone()))["kind"],
            json!(kind.word())
        );
        // The rest is the preset's.
        v["meta"].as_object_mut().unwrap().remove("kind");
        v["meta"]["savedUtc"] = json!("");
        assert_eq!(v, preset);
    }
}

// ── Other consoles' documents ──

#[test]
fn p1df_mac_and_qt_documents_still_import() {
    let every = IMPORT_BY_BLOCK | IMPORT_ALL_BLOCKS;
    for (what, doc) in [
        ("Mac", MAC),
        ("Qt (Windows, Linux)", QT),
        ("Terminal", K340),
    ] {
        for options in [0, IMPORT_VOLUMES | IMPORT_HARDWARE, every] {
            let core = demo();
            let r = json_of(&core.config_import(doc.into(), options));
            assert_eq!(r["ok"], json!(true), "{what} {options}: {r}");
        }
        // After an import of everything, the device holds what it says.
        let core = demo();
        core.config_import(doc.into(), every);
        let left = json_of(&core.config_preview(doc.into(), every, u32::MAX));
        assert_eq!(rows(&left), vec![], "{what}");
    }
}
