//! WP P2SS: the hub's snapshot store and its message handlers.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use dspi_session::snapshot_store::{self, SnapshotStore};
use serde_json::{Value, json};

use super::{Caps, HubSnapshots};
use crate::session::Role;

/// A fresh folder. `ci/run.sh` points `TMPDIR` at the branch's work folder
/// (`~/.cache/dspi-build/work/wp-P2SS/tmp`), never tmpfs.
fn fresh(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-p2ss").join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A snapshot document as a client's own store files it: saved in a client store,
/// then its text read back.
fn client_doc(client: &SnapshotStore, serial: &str, saved_ms: i64, name: &str) -> (String, String) {
    let mut doc = json!({
        "meta": {"name": name, "platform": "RP2350", "outputChannelCount": 4, "wireFormatVersion": 3},
        "snapshot": {"image": "AAAA"}
    });
    let e = client.save(serial, &mut doc, saved_ms).unwrap();
    let (_, text) = client.read(&e.id).unwrap();
    (e.id, text)
}

fn hub_and_client(tag: &str) -> (HubSnapshots, SnapshotStore) {
    let base = fresh(tag);
    (
        HubSnapshots::open(&base.join("data")),
        SnapshotStore::new(base.join("client")),
    )
}

fn ask(h: &HubSnapshots, role: Role, msg: Value) -> Value {
    serde_json::from_str(&h.handle_message(role, &msg).expect("a snapshot message")).unwrap()
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

#[test]
fn wp_p2ss_put_list_get_rename_note_delete_with_roles() {
    let (h, client) = hub_and_client("roles");
    let (sid, text) = client_doc(&client, "SER1", 1_700_000_000_000, "Venue");

    // Viewer may not put; Control may.
    let put = json!({"t": "snapshot.put", "id": 1, "serial": "SER1", "document": text});
    assert_eq!(code(&ask(&h, Role::Viewer, put.clone())), "denied");
    let a = ask(&h, Role::Control, put.clone());
    assert_eq!(a["t"], "ok", "{a}");
    assert_eq!(a["v"], 1);
    assert_eq!(a["created"], true);
    // The client's id is kept.
    assert_eq!(a["snapshot"]["id"], sid.as_str());
    assert_eq!(a["snapshot"]["name"], "Venue");
    assert_eq!(a["snapshot"]["outputChannels"], 4);
    // The same document again: ok, nothing new.
    let a = ask(&h, Role::Admin, put);
    assert_eq!(a["created"], false, "{a}");

    // Every role lists and gets.
    let l = ask(
        &h,
        Role::Viewer,
        json!({"t": "snapshot.list", "id": 2, "serial": "SER1"}),
    );
    assert_eq!(l["snapshots"].as_array().unwrap().len(), 1, "{l}");
    let g = ask(
        &h,
        Role::Viewer,
        json!({"t": "snapshot.get", "id": 3, "snapshot": sid}),
    );
    assert_eq!(g["t"], "ok", "{g}");
    let doc: Value = serde_json::from_str(g["document"].as_str().unwrap()).unwrap();
    assert_eq!(doc["snapshot"]["serial"], "SER1");

    // Rename and note: Control yes, Viewer no.
    let r = json!({"t": "snapshot.rename", "id": 4, "snapshot": sid, "name": "Kväll"});
    assert_eq!(code(&ask(&h, Role::Viewer, r.clone())), "denied");
    assert_eq!(ask(&h, Role::Control, r)["snapshot"]["name"], "Kväll");
    let n = json!({"t": "snapshot.note", "id": 5, "snapshot": sid, "note": "after the gig"});
    assert_eq!(code(&ask(&h, Role::Viewer, n.clone())), "denied");
    assert_eq!(
        ask(&h, Role::Control, n)["snapshot"]["note"],
        "after the gig"
    );

    // Delete: only Admin.
    let d = json!({"t": "snapshot.delete", "id": 6, "snapshot": sid});
    assert_eq!(code(&ask(&h, Role::Viewer, d.clone())), "denied");
    assert_eq!(code(&ask(&h, Role::Control, d.clone())), "denied");
    let a = ask(&h, Role::Admin, d.clone());
    assert_eq!(a["t"], "ok", "{a}");
    assert_eq!(a["serial"], "SER1");
    assert_eq!(code(&ask(&h, Role::Admin, d)), "not_found");
    let l = ask(
        &h,
        Role::Viewer,
        json!({"t": "snapshot.list", "id": 7, "serial": "SER1"}),
    );
    assert!(l["snapshots"].as_array().unwrap().is_empty());

    // One change event per change, in order.
    let ev: Vec<Value> = h
        .take_changed("hub-1")
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let changes: Vec<&str> = ev.iter().map(|e| e["change"].as_str().unwrap()).collect();
    assert_eq!(changes, ["put", "rename", "note", "delete"]);
    assert!(
        ev.iter()
            .all(|e| e["t"] == "snapshot.changed" && e["serial"] == "SER1" && e["hub"] == "hub-1")
    );
    assert!(h.take_changed("hub-1").is_empty());
}

#[test]
fn wp_p2ss_other_messages_and_bad_requests() {
    let (h, _) = hub_and_client("other");
    assert!(
        h.handle_message(Role::Admin, &json!({"t": "snapshot.recall", "id": 1}))
            .is_none()
    );
    assert!(
        h.handle_message(Role::Admin, &json!({"t": "device.list", "id": 1}))
            .is_none()
    );
    assert_eq!(
        code(&ask(&h, Role::Admin, json!({"t": "snapshot.list"}))),
        "bad_request"
    );
    assert_eq!(
        code(&ask(
            &h,
            Role::Admin,
            json!({"t": "snapshot.list", "id": 1})
        )),
        "bad_request"
    );
    assert_eq!(
        code(&ask(
            &h,
            Role::Admin,
            json!({"t": "snapshot.get", "id": 1, "snapshot": "../x"})
        )),
        "not_found"
    );
    let put = |doc: Value| json!({"t": "snapshot.put", "id": 1, "serial": "S", "document": doc});
    for doc in [
        json!("not json"),
        json!("[1,2]"),
        json!({"meta": {"kind": "preset"}, "snapshot": {"id": "1", "serial": "S"}}),
        json!({"meta": {"kind": "snapshot"}}),
        json!({"meta": {"kind": "snapshot"}, "snapshot": {"serial": "S"}}),
        json!({"meta": {"kind": "snapshot"}, "snapshot": {"id": "../evil", "serial": "S"}}),
        json!(42),
    ] {
        assert_eq!(
            code(&ask(&h, Role::Admin, put(doc.clone()))),
            "bad_request",
            "{doc}"
        );
    }
    // An object document is taken as well as its text.
    let a = ask(
        &h,
        Role::Admin,
        put(
            json!({"meta": {"kind": "snapshot"}, "snapshot": {"id": "0000000000001", "serial": "S"}}),
        ),
    );
    assert_eq!(a["t"], "ok", "{a}");
}

#[test]
fn wp_p2ss_put_with_another_devices_serial_refused() {
    let (h, client) = hub_and_client("serial");
    let (_, text) = client_doc(&client, "SER-A", 1_700_000_000_001, "A");
    let a = ask(
        &h,
        Role::Admin,
        json!({"t": "snapshot.put", "id": 1, "serial": "SER-B", "document": text}),
    );
    assert_eq!(code(&a), "bad_request", "{a}");
    assert!(h.list("SER-B").unwrap().is_empty());
    // An id the hub holds for one device is not filed for another.
    let (sid, text_a) = client_doc(&client, "SER-A", 1_700_000_000_002, "A2");
    assert_eq!(h.put("SER-A", &text_a).unwrap().entry.id, sid);
    let mut other: Value = serde_json::from_str(&text_a).unwrap();
    other["snapshot"]["serial"] = json!("SER-B");
    let a = ask(
        &h,
        Role::Admin,
        json!({"t": "snapshot.put", "id": 2, "serial": "SER-B", "document": other}),
    );
    assert_eq!(code(&a), "conflict", "{a}");
}

#[test]
fn wp_p2ss_the_hub_never_overwrites_a_differing_id() {
    let (h, client) = hub_and_client("wins");
    let (sid, text) = client_doc(&client, "S", 1_700_000_000_003, "Hub's");
    h.put("S", &text).unwrap();
    let mut changed: Value = serde_json::from_str(&text).unwrap();
    changed["meta"]["name"] = json!("Local edit");
    let a = ask(
        &h,
        Role::Admin,
        json!({"t": "snapshot.put", "id": 1, "serial": "S", "document": changed}),
    );
    assert_eq!(code(&a), "conflict", "{a}");
    assert_eq!(h.get(&sid).unwrap().0.name, "Hub's");
}

#[test]
fn wp_p2ss_torn_index_lists_from_the_documents() {
    let (h, client) = hub_and_client("torn");
    let (a, ta) = client_doc(&client, "S", 1_700_000_000_010, "first");
    let (b, tb) = client_doc(&client, "S", 1_700_000_000_020, "second");
    h.put("S", &ta).unwrap();
    h.put("S", &tb).unwrap();
    let index = h
        .root()
        .join(snapshot_store::dir_name("S"))
        .join(snapshot_store::INDEX_FILE);
    for torn in [
        "{\"version\":1,\"snapshots\":[{\"id\":",
        "",
        "\u{0}\u{ff}garbage",
        "{\"snapshots\": 7}",
    ] {
        fs::write(&index, torn).unwrap();
        let ids: Vec<String> = h.list("S").unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, [a.clone(), b.clone()], "torn index {torn:?}");
        assert_eq!(h.list("S").unwrap()[1].name, "second");
    }
    fs::remove_file(&index).unwrap();
    assert_eq!(h.list("S").unwrap().len(), 2);
    // The hub's folder is the client store's format: a client store reads it as is.
    let as_client = SnapshotStore::new(h.root());
    assert_eq!(as_client.list("S").len(), 2);
}

#[test]
fn wp_p2ss_names_and_serials_with_any_unicode() {
    let (h, client) = hub_and_client("unicode");
    for (serial, name) in [
        ("SÉR-ü", "Kväll på scenen 🎛️"),
        ("序列号", "名前 \u{200d} \u{202e}rtl"),
        ("s/../x", "\"quoted\" \\ and\nnewline"),
    ] {
        let (sid, text) = client_doc(&client, serial, 1_700_000_100_000, "x");
        let p = h.put(serial, &text).unwrap();
        assert_eq!(p.entry.serial, serial);
        let a = ask(
            &h,
            Role::Control,
            json!({"t": "snapshot.rename", "id": 1, "snapshot": sid, "name": name}),
        );
        assert_eq!(a["snapshot"]["name"], name, "{a}");
        let a = ask(
            &h,
            Role::Control,
            json!({"t": "snapshot.note", "id": 2, "snapshot": sid, "note": name}),
        );
        assert_eq!(a["snapshot"]["note"], name);
        let l = h.list(serial).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name, name);
        assert_eq!(l[0].note, name);
        let (_, doc) = h.get(&sid).unwrap();
        let doc: Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(doc["meta"]["name"], name);
        // Everything stays inside the store's folder.
        assert!(h.root().join(snapshot_store::dir_name(serial)).is_dir());
        let _ = fs::remove_dir_all(h.root().join(snapshot_store::dir_name(serial)));
    }
    // Over-long names and notes are refused, not cut.
    let (sid, text) = client_doc(&client, "S", 1_700_000_200_000, "x");
    h.put("S", &text).unwrap();
    let long = "é".repeat(super::MAX_NAME_CHARS + 1);
    assert_eq!(
        code(&ask(
            &h,
            Role::Control,
            json!({"t": "snapshot.rename", "id": 1, "snapshot": sid, "name": long})
        )),
        "bad_request"
    );
    assert_eq!(
        h.rename(&sid, &"é".repeat(super::MAX_NAME_CHARS))
            .unwrap()
            .name
            .chars()
            .count(),
        super::MAX_NAME_CHARS
    );
    assert!(h.list("").is_err());
    assert!(h.list(&"x".repeat(65)).is_err());
}

#[test]
fn wp_p2ss_caps_enforced_with_conflict() {
    // The defaults are OD-P2k's.
    let d = Caps::default();
    assert_eq!(
        (d.per_device, d.doc_bytes, d.total_bytes),
        (200, 256 * 1024, 64 * 1024 * 1024)
    );

    let base = fresh("caps");
    let client = SnapshotStore::new(base.join("client"));
    let h = HubSnapshots::with_caps(
        base.join("hub"),
        Caps {
            per_device: 3,
            doc_bytes: 2048,
            total_bytes: 4096,
        },
    );
    let put = |serial: &str, text: &str| {
        ask(
            &h,
            Role::Control,
            json!({"t": "snapshot.put", "id": 1, "serial": serial, "document": text}),
        )
    };

    // Per device.
    for i in 0..3 {
        let (_, t) = client_doc(&client, "A", 1_700_000_000_000 + i, "a");
        assert_eq!(put("A", &t)["t"], "ok");
    }
    let (_, t) = client_doc(&client, "A", 1_700_000_000_100, "a");
    let a = put("A", &t);
    assert_eq!(code(&a), "conflict", "{a}");
    assert_eq!(h.list("A").unwrap().len(), 3);

    // Per document.
    let mut big: Value = serde_json::from_str(&t).unwrap();
    big["snapshot"]["image"] = json!("A".repeat(3000));
    big["snapshot"]["serial"] = json!("B");
    let a = put("B", &serde_json::to_string(&big).unwrap());
    assert_eq!(code(&a), "conflict", "{a}");

    // In all: what is filed plus the new document stays under the total.
    // One document per device, so the per-device cap never applies.
    let mut n = 0;
    let refused = loop {
        let serial = format!("C{n}");
        let (_, t) = client_doc(&client, &serial, 1_700_000_001_000 + n, "c");
        let mut doc: Value = serde_json::from_str(&t).unwrap();
        doc["snapshot"]["image"] = json!("A".repeat(1000));
        let a = put(&serial, &serde_json::to_string(&doc).unwrap());
        if a["t"] != "ok" {
            break a;
        }
        n += 1;
        assert!(n < 10, "the total cap never refused");
    };
    assert_eq!(code(&refused), "conflict", "{refused}");
    assert!(
        refused["msg"].as_str().unwrap().contains("in all"),
        "{refused}"
    );
    assert!(n >= 1);

    // A rename that would take a document over its cap.
    let id = h.list("A").unwrap()[0].id.clone();
    let a = ask(
        &h,
        Role::Control,
        json!({"t": "snapshot.rename", "id": 2, "snapshot": id, "name": "é".repeat(256)}),
    );
    assert_eq!(a["t"], "ok", "a 512-byte name still fits: {a}");
    let a = ask(
        &h,
        Role::Control,
        json!({"t": "snapshot.note", "id": 3, "snapshot": id, "note": "x".repeat(2000)}),
    );
    assert_eq!(code(&a), "conflict", "{a}");
}

#[test]
fn wp_p2ss_store_folder_and_files() {
    let base = fresh("files");
    let h = HubSnapshots::open(&base);
    assert_eq!(h.root(), base.join("snapshots"));
    // Nothing is made until the first write.
    assert!(!h.root().exists());
    assert!(h.list("S").unwrap().is_empty());
    let client = SnapshotStore::new(base.join("client"));
    let (sid, text) = client_doc(&client, "S", 1_700_000_000_000, "x");
    h.put("S", &text).unwrap();
    let doc = h.root().join("s-S").join(format!("{sid}.dspipreset"));
    assert!(doc.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&doc), 0o600);
        assert_eq!(mode(&h.root().join("s-S").join("index.json")), 0o600);
        assert_eq!(mode(&h.root().join("s-S")), 0o700);
    }
}

#[test]
fn wp_p2ss_fixup_cap_name_is_not_the_device_cache_cap() {
    // review p2a F1: `snapshot` is spec 7.5's device cache, which every hub announces.
    assert_eq!(super::CAP, "hub_snapshots");
}

#[test]
fn wp_p2ss_fixup_a_snapshot_at_the_cap_survives_two_renames() {
    // review p2a F4: filed pretty-printed, every cap measured on that form.
    let base = fresh("atcap");
    let doc = |pad: usize| {
        json!({
            "meta": {"kind": "snapshot", "name": "abcd"},
            "snapshot": {"id": "0001700000000000", "serial": "S", "image": "A".repeat(pad)}
        })
    };
    let pretty_len = |v: &Value| serde_json::to_string_pretty(v).unwrap().len() as u64;
    let cap = pretty_len(&doc(20_000));
    let h = HubSnapshots::with_caps(
        base.join("hub"),
        Caps {
            per_device: 10,
            doc_bytes: cap,
            total_bytes: 1 << 20,
        },
    );
    // Its compact form (the handler's object form) is far under the cap, its stored form
    // exactly at it; one byte more is refused.
    let a = ask(
        &h,
        Role::Control,
        json!({"t": "snapshot.put", "id": 1, "serial": "S", "document": doc(20_001)}),
    );
    assert_eq!(code(&a), "conflict", "{a}");
    let a = ask(
        &h,
        Role::Control,
        json!({"t": "snapshot.put", "id": 2, "serial": "S", "document": doc(20_000)}),
    );
    assert_eq!(a["t"], "ok", "{a}");
    let path = h.root().join("s-S").join("0001700000000000.dspipreset");
    assert_eq!(fs::metadata(&path).unwrap().len(), cap);
    for (n, name) in ["wxyz", "efgh"].into_iter().enumerate() {
        let a = ask(
            &h,
            Role::Control,
            json!({"t": "snapshot.rename", "id": 3 + n, "snapshot": "0001700000000000", "name": name}),
        );
        assert_eq!(a["t"], "ok", "rename {n}: {a}");
        assert_eq!(fs::metadata(&path).unwrap().len(), cap);
        let g = ask(
            &h,
            Role::Viewer,
            json!({"t": "snapshot.get", "id": 9, "snapshot": "0001700000000000"}),
        );
        assert_eq!(g["snapshot"]["name"], name, "{g}");
    }
    // A longer name would take it over: refused, and the snapshot stays readable.
    let a = ask(
        &h,
        Role::Control,
        json!({"t": "snapshot.rename", "id": 10, "snapshot": "0001700000000000", "name": "abcde"}),
    );
    assert_eq!(code(&a), "conflict", "{a}");
    assert_eq!(h.get("0001700000000000").unwrap().0.name, "efgh");
    // The same document put again is still recognised after the store rewrote it.
    let mut again = doc(20_000);
    again["meta"]["name"] = json!("efgh");
    assert!(
        !h.put("S", &serde_json::to_string(&again).unwrap())
            .unwrap()
            .created
    );
}
