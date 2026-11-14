//! WP P1BFIX (review p1b F3): snapshot ids are unique across the whole store,
//! not per serial folder, and an id lookup never acts on another serial's
//! snapshot: an id two folders hold (copied in by hand) is refused.
//!
//! The store lives under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).
#![allow(clippy::unwrap_used)]

use std::io::ErrorKind;

use dspi_session::snapshot_store::{SnapshotStore, dir_name};
use serde_json::json;

fn store(name: &str) -> (SnapshotStore, std::path::PathBuf) {
    let dir = std::env::temp_dir()
        .join("wp-p1bfix-session")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (SnapshotStore::new(&dir), dir)
}

#[test]
fn wp_p1bfix_two_serials_in_one_millisecond_get_two_ids() {
    let (store, dir) = store("ids");
    let mut a = json!({"meta": {"name": "on A"}});
    let mut b = json!({"meta": {"name": "on B"}});
    let ea = store.save("AAAA", &mut a, 5000).unwrap();
    let eb = store.save("BBBB", &mut b, 5000).unwrap();
    assert_ne!(ea.id, eb.id, "same capture ms, two devices: two ids");
    assert_eq!(b["snapshot"]["id"], eb.id.as_str());

    let (e, _) = store.read(&eb.id).unwrap();
    assert_eq!((e.serial.as_str(), e.name.as_str()), ("BBBB", "on B"));
    let r = store.rename(&eb.id, "renamed").unwrap();
    assert_eq!(r.serial, "BBBB");
    assert_eq!(store.list("AAAA")[0].name, "on A", "A's untouched");
    assert_eq!(store.list("BBBB")[0].name, "renamed");
    store.delete(&eb.id).unwrap();
    assert!(store.list("BBBB").is_empty());
    assert_eq!(store.list("AAAA").len(), 1, "A's still there");
    assert_eq!(
        store.read(&eb.id).unwrap_err().kind(),
        ErrorKind::NotFound,
        "the id answers for nobody else"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_p1bfix_an_id_two_folders_hold_is_refused_not_guessed() {
    let (store, dir) = store("ambiguous");
    let mut a = json!({"meta": {"name": "on A"}});
    let ea = store.save("AAAA", &mut a, 7000).unwrap();
    let mut b = json!({"meta": {"name": "on B"}});
    store.save("BBBB", &mut b, 9000).unwrap();
    // A copy by hand into B's folder, under A's id.
    let file = format!("{}.dspipreset", ea.id);
    std::fs::copy(
        dir.join(dir_name("AAAA")).join(&file),
        dir.join(dir_name("BBBB")).join(&file),
    )
    .unwrap();

    for e in [
        store.read(&ea.id).map(|_| ()).unwrap_err(),
        store.rename(&ea.id, "x").map(|_| ()).unwrap_err(),
        store.set_note(&ea.id, "x").map(|_| ()).unwrap_err(),
        store.delete(&ea.id).unwrap_err(),
    ] {
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
        assert!(e.to_string().contains("more than one device"), "{e}");
    }
    assert!(
        dir.join(dir_name("AAAA")).join(&file).is_file(),
        "nothing deleted"
    );
    assert_eq!(store.list("AAAA")[0].name, "on A", "nothing renamed");
    // A new snapshot never takes an id any folder holds.
    let mut c = json!({});
    let ec = store.save("CCCC", &mut c, 7000).unwrap();
    assert_ne!(ec.id, ea.id);
    let _ = std::fs::remove_dir_all(&dir);
}
