//! WP P3RO: the image store and the records file on their own (no hub): names, the
//! checks an image passes, created/conflict/the fifth image, a changed file, a torn index
//! rebuilt, and a rollout the hub stopped in the middle of read back failed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::atomic::{AtomicU32, Ordering};

use super::images::{ImageStore, MAX_IMAGES, add_file, valid_name};
use super::*;

const NOW: u64 = 1_800_000_000;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-P3RO-unit").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An RP2350 DSPi image of `version`, stamped `stamp` (the firmware guard's layout).
fn image(version: &str, stamp: Option<&str>) -> Vec<u8> {
    const XIP: u32 = 0x1000_0000;
    let mut entries: Vec<(u16, u32, String)> = vec![
        (0x5052, 0x0203_1C86, "DSPi".into()),
        (0x5052, 0x11A9_BC3A, version.into()),
    ];
    if let Some(s) = stamp {
        entries.push((0x5344, 0x0B7A_4C15, s.into()));
    }
    let mut f = vec![0u8; 0x2000];
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, 0x7188_EBF2);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, 0xE71A_A390);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&6u16.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let mut b = vec![0u8; 512];
        for (off, v) in [
            (0, 0x0A32_4655u32),
            (4, 0x9E5D_5157),
            (8, 0x2000),
            (12, XIP + 256 * i as u32),
            (16, 256),
            (20, i as u32),
            (24, pages.len() as u32),
            (28, 0xE48B_FF59),
            (508, 0x0AB1_6F30),
        ] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[32..32 + p.len()].copy_from_slice(p);
        out.extend(b);
    }
    out
}

#[test]
fn wp_p3ro_image_names_are_plain_uf2_file_names() {
    for good in ["dspi-rp2350-1.2.0.uf2", "a.uf2", "K340_v1.1.7+bofr.uf2"] {
        assert!(valid_name(good), "{good}");
    }
    let long = format!("{}.uf2", "a".repeat(61));
    for bad in [
        "",
        ".uf2",
        "x.bin",
        "../x.uf2",
        "a/b.uf2",
        "a b.uf2",
        "index.json",
        ".hidden.uf2",
        "é.uf2",
        long.as_str(),
    ] {
        assert!(!valid_name(bad), "{bad}");
    }
}

#[test]
fn wp_p3ro_the_store_checks_keeps_and_bounds_its_images() {
    let dir = scratch("store");
    let store = ImageStore::open(&dir, NOW);
    let img = image("1.2.0", Some("1200"));
    let added = store.add("dspi-1.2.0.uf2", &img, "Bench", NOW).unwrap();
    assert!(added.created);
    let i = &added.image;
    assert_eq!(
        (i.size, i.platform.as_str(), i.firmware_version.as_str()),
        (img.len() as u64, "RP2350", "1.2.0")
    );
    assert_eq!(i.stamp_mv, Some(1200));
    assert!(!i.verified, "a test image is in no manifest");
    assert_eq!(i.added_by, "Bench");
    assert_eq!(i.sha256, images::sha256_hex(&img));
    // Unstamped images have no stamp.
    let plain = store
        .add("plain.uf2", &image("1.2.0", None), "Bench", NOW)
        .unwrap();
    assert_eq!(plain.image.stamp_mv, None);
    // Again: not created. Other bytes under the name: conflict.
    assert!(
        !store
            .add("dspi-1.2.0.uf2", &img, "Bench", NOW)
            .unwrap()
            .created
    );
    let e = store
        .add(
            "dspi-1.2.0.uf2",
            &image("1.2.1", Some("1200")),
            "Bench",
            NOW,
        )
        .unwrap_err();
    assert_eq!(e.code, "conflict");
    // Not firmware.
    let e = store.add("junk.uf2", b"junk", "Bench", NOW).unwrap_err();
    assert_eq!(e.code, "bad_request");
    assert!(e.msg.contains("not a firmware file"), "{}", e.msg);
    // Too large.
    let e = store
        .add(
            "big.uf2",
            &vec![0u8; images::MAX_IMAGE_BYTES + 1],
            "Bench",
            NOW,
        )
        .unwrap_err();
    assert_eq!(e.code, "bad_request");
    // Four at most.
    store
        .add("c.uf2", &image("1.3.0", Some("1200")), "B", NOW)
        .unwrap();
    store
        .add("d.uf2", &image("1.4.0", Some("1200")), "B", NOW)
        .unwrap();
    assert_eq!(store.list(NOW).len(), MAX_IMAGES);
    let e = store
        .add("e.uf2", &image("1.5.0", Some("1200")), "B", NOW)
        .unwrap_err();
    assert_eq!(e.code, "conflict");
    // A load reads the bytes again; a changed file is refused.
    let (info, bytes) = store.load("dspi-1.2.0.uf2", NOW).unwrap();
    assert_eq!(info.name, "dspi-1.2.0.uf2");
    assert_eq!(bytes, img);
    std::fs::write(dir.join("firmware").join("c.uf2"), b"tampered").unwrap();
    assert_eq!(store.load("c.uf2", NOW).unwrap_err().code, "image_refused");
    assert_eq!(
        store.load("nothing.uf2", NOW).unwrap_err().code,
        "not_found"
    );
    // Opened again, the changed one is dropped from the index.
    let again = ImageStore::open(&dir, NOW);
    let names: Vec<String> = again.list(NOW).into_iter().map(|i| i.name).collect();
    assert_eq!(names, ["dspi-1.2.0.uf2", "plain.uf2", "d.uf2"]);
    // Delete.
    again.delete("plain.uf2", NOW).unwrap();
    assert_eq!(
        again.delete("plain.uf2", NOW).unwrap_err().code,
        "not_found"
    );
    assert!(!dir.join("firmware").join("plain.uf2").exists());
    // From the hub's machine (P3TM): the same checks, `by` the hub console.
    let added = add_file(&dir, "e.uf2", &image("1.5.0", Some("1200")), NOW).unwrap();
    assert_eq!(added.image.added_by, crate::service::LOCAL_LABEL);
    assert_eq!(store.list(NOW).len(), 3, "a running hub sees it");
}

#[test]
fn wp_p3ro_a_torn_index_is_kept_aside_and_rebuilt() {
    let dir = scratch("torn");
    let store = ImageStore::open(&dir, NOW);
    store
        .add("a.uf2", &image("1.2.0", Some("1200")), "B", NOW)
        .unwrap();
    std::fs::write(
        dir.join("firmware").join("index.json"),
        "{\"images\": [ torn",
    )
    .unwrap();
    let names: Vec<String> = ImageStore::open(&dir, NOW)
        .list(NOW)
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert_eq!(names, ["a.uf2"]);
    let aside = std::fs::read_dir(dir.join("firmware"))
        .unwrap()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("index.json.bad-")
        });
    assert!(aside, "the torn index is kept aside");
}

fn record(id: &str, state: &str, devices: &[(&str, &str, &str)]) -> Record {
    Record {
        id: id.into(),
        target: "group:g1".into(),
        image: ImageRef {
            name: "a.uf2".into(),
            sha256: "00".into(),
            firmware_version: "1.2.0".into(),
        },
        by: "Bench".into(),
        while_live: false,
        state: state.into(),
        started_at: iso8601(NOW),
        ended_at: None,
        devices: devices
            .iter()
            .map(|(s, st, step)| DeviceEntry {
                serial: (*s).into(),
                state: (*st).into(),
                step: (*step).into(),
                steps: Vec::new(),
                before: None,
                after: None,
                failed: None,
            })
            .collect(),
    }
}

#[test]
fn wp_p3ro_a_rollout_the_hub_stopped_in_reads_back_failed_and_ids_are_not_reused() {
    let dir = scratch("records");
    let path = dir.join(FILE_NAME);
    let done = record("ro3", "ok", &[("A", "ok", "verify")]);
    let cut_short = record(
        "ro4",
        "running",
        &[
            ("A", "ok", "verify"),
            ("B", "running", "wait"),
            ("C", "pending", ""),
        ],
    );
    let text = serde_json::to_string(&json!({"v": 1, "next_id": 2,
        "rollouts": [done, cut_short, {"id": 7}]}))
    .unwrap();
    std::fs::write(&path, text).unwrap();
    let (records, next_id, ended) = load(&path, NOW);
    assert_eq!(
        records.len(),
        2,
        "the record that does not read is left out"
    );
    assert_eq!(next_id, 5, "past every id the file holds");
    let r = &records[1];
    assert_eq!(r.state, "failed");
    assert!(r.ended_at.is_some());
    let states: Vec<&str> = r.devices.iter().map(|d| d.state.as_str()).collect();
    assert_eq!(states, ["ok", "failed", "not_tried"]);
    assert_eq!(r.devices[1].failed.as_ref().unwrap().step, "wait");
    assert_eq!(
        ended,
        [(
            "B".to_owned(),
            "wait".to_owned(),
            "The hub stopped during this device's rollout".to_owned(),
            "1.2.0".to_owned()
        )]
    );
    // The file as it was is kept beside.
    let aside = std::fs::read_dir(&dir).unwrap().flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with("rollout.json.bad-")
    });
    assert!(aside);
    // A file that does not read at all.
    std::fs::write(&path, "not json").unwrap();
    let (records, next_id, ended) = load(&path, NOW + 1);
    assert!(records.is_empty() && ended.is_empty());
    assert_eq!(next_id, 1);
}
