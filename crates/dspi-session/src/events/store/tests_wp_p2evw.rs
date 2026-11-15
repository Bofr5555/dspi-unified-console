//! WP P2EVW: the multi-device event writer, the new kinds and `by`.
//!
//! Files go under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs); a run that would put
//! them on `/tmp` or `/dev/shm` fails instead.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::super::{EventKind, EventLog, EventRecord, Limits, file_stem, parse_line};
use super::*;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p2evw-events").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    assert!(
        !d.starts_with("/tmp") && !d.starts_with("/dev/shm"),
        "WP P2EVW tests must not write on tmpfs: run them through ci/run.sh ({})",
        d.display()
    );
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn rec(kind: EventKind, t: i64, text: &str) -> EventRecord {
    EventRecord::new(kind, t, text)
}

fn ids(store: &EventStore, serial: &str) -> Vec<u64> {
    store
        .read(serial, 0, usize::MAX)
        .records
        .iter()
        .map(|r| r.id)
        .collect()
}

fn small(max_file_bytes: u64) -> Limits {
    Limits {
        max_file_bytes,
        rotated_files: 3,
        max_line_bytes: 4096,
        memory_records: 0,
    }
}

fn file(d: &Path, serial: &str, n: u32) -> PathBuf {
    let stem = file_stem(serial);
    if n == 0 {
        d.join(format!("{stem}.jsonl"))
    } else {
        d.join(format!("{stem}.{n}.jsonl"))
    }
}

#[test]
fn wp_p2evw_hub_limits_are_od_p2k() {
    assert_eq!(HUB_LIMITS.max_file_bytes, 512 * 1024);
    assert_eq!(HUB_LIMITS.rotated_files, 3);
    assert_eq!(HUB_TOTAL_BYTES, 128 * 1024 * 1024);
    // 2 MiB per device.
    assert_eq!(
        HUB_LIMITS.max_file_bytes * u64::from(HUB_LIMITS.rotated_files + 1),
        2 * 1024 * 1024
    );
}

#[test]
fn wp_p2evw_serials_interleaved_each_rising() {
    let d = dir("interleave");
    let mut store = EventStore::open(d.clone(), small(2048), HUB_TOTAL_BYTES).unwrap();
    let serials = ["BB66A44F", "CC77B550", "weird serial/é"];
    for i in 0..120 {
        let s = serials[i % 3];
        let r = store
            .append(s, rec(EventKind::Notification, i as i64, &format!("n {i}")))
            .unwrap();
        assert_eq!(r.serial, s);
    }
    for s in serials {
        let got = ids(&store, s);
        assert_eq!(got, (1..=40).collect::<Vec<u64>>(), "{s}");
        // Rotated: 40 lines of some 90 bytes do not fit 2 KiB.
        assert!(file(&d, s, 1).exists(), "{s} rotated");
        // Each file's ids rise, and the older file holds the lower ones.
        let mut last = 0;
        for n in (0..=3).rev() {
            let Ok(text) = fs::read_to_string(file(&d, s, n)) else {
                continue;
            };
            for line in text.lines() {
                let r = parse_line(line).unwrap();
                assert!(r.id > last, "{s} file {n}: {} after {last}", r.id);
                last = r.id;
            }
        }
    }
    assert_eq!(store.devices().len(), 3);
    // Ids carry on in a new store (the next run), from the files.
    let mut store = EventStore::open(d.clone(), small(2048), HUB_TOTAL_BYTES).unwrap();
    let r = store
        .append("CC77B550", rec(EventKind::Alarm, 1, "next run"))
        .unwrap();
    assert_eq!(r.id, 41);
    // The client's read-only export reads the hub's folder unchanged.
    let out = EventLog::export_in(&d, "CC77B550");
    assert_eq!(out.lines().count(), 41);
}

#[test]
fn wp_p2evw_client_writer_on_the_same_folder_never_repeats_an_id() {
    let d = dir("two-writers");
    let mut hub = EventStore::open(d.clone(), HUB_LIMITS, HUB_TOTAL_BYTES).unwrap();
    // A client with its own (default) bounds, on the same folder.
    let mut client = EventLog::new(Limits::DEFAULT);
    client.set_dir(Some(d.clone())).unwrap();
    client.open_device("BB66A44F");
    for i in 0..50 {
        hub.append("BB66A44F", rec(EventKind::Notification, i, "hub"))
            .unwrap();
        client.record(rec(EventKind::Clip, i, "client"));
        if i % 7 == 0 {
            hub.append("BB66A44F", rec(EventKind::Access, i, "hub again"))
                .unwrap();
        }
        // The hub's other devices in between.
        hub.append("CC77B550", rec(EventKind::Notification, i, "other"))
            .unwrap();
    }
    client.close_device();
    let all = ids(&hub, "BB66A44F");
    let mut unique = all.clone();
    unique.dedup();
    assert_eq!(all.len(), 50 + 50 + 8);
    assert_eq!(unique, all, "no id twice, and rising");
    assert_eq!(all, (1..=108).collect::<Vec<u64>>());
    assert_eq!(client.write_errors(), 0);
    assert_eq!(hub.write_errors(), 0);
}

#[test]
fn wp_p2evw_total_cap_removes_the_oldest_rotated_file_first() {
    let d = dir("total-cap");
    let limits = small(1024);
    // No cap while filling.
    let mut store = EventStore::open(d.clone(), limits, u64::MAX).unwrap();
    for i in 0..60 {
        for s in ["AAAA", "BBBB"] {
            store
                .append(
                    s,
                    rec(EventKind::Notification, i, &format!("record {i:03}")),
                )
                .unwrap();
        }
    }
    for s in ["AAAA", "BBBB"] {
        for n in 0..=3 {
            assert!(file(&d, s, n).exists(), "{s}.{n}");
        }
    }
    // Make the ages unambiguous: BBBB's .3 is the oldest file of all, then
    // AAAA's .3; current files are the newest.
    let base = SystemTime::now() - Duration::from_secs(10_000);
    let age = |s: &str, n: u32, secs: u64| {
        let f = fs::OpenOptions::new()
            .append(true)
            .open(file(&d, s, n))
            .unwrap();
        f.set_modified(base + Duration::from_secs(secs)).unwrap();
    };
    age("BBBB", 3, 0);
    age("AAAA", 3, 10);
    age("BBBB", 2, 20);
    age("AAAA", 2, 30);
    age("BBBB", 1, 40);
    age("AAAA", 1, 50);
    let size = |s: &str, n: u32| fs::metadata(file(&d, s, n)).unwrap().len();
    let total: u64 = ["AAAA", "BBBB"]
        .iter()
        .flat_map(|s| (0..=3).map(move |n| (*s, n)))
        .map(|(s, n)| size(s, n))
        .sum();
    // Room for all but a little: one file (the oldest rotated) has to go.
    let cap = total - 1;
    let mut store = EventStore::open(d.clone(), limits, cap).unwrap();
    assert_eq!(store.evicted_files(), 1);
    assert!(!file(&d, "BBBB", 3).exists(), "the oldest went");
    assert!(file(&d, "AAAA", 3).exists());
    assert!(store.used_bytes() <= cap);

    // A tighter cap: rotated files go oldest first, current files stay.
    let rotated_rest =
        size("AAAA", 3) + size("BBBB", 2) + size("AAAA", 2) + size("BBBB", 1) + size("AAAA", 1);
    let current = size("AAAA", 0) + size("BBBB", 0);
    let cap = current + size("AAAA", 1);
    store = EventStore::open(d.clone(), limits, cap).unwrap();
    assert!(rotated_rest > size("AAAA", 1));
    for (s, n) in [("AAAA", 3), ("BBBB", 2), ("AAAA", 2), ("BBBB", 1)] {
        assert!(!file(&d, s, n).exists(), "{s}.{n} went");
    }
    assert!(file(&d, "AAAA", 1).exists(), "the newest rotated stays");
    assert!(file(&d, "AAAA", 0).exists() && file(&d, "BBBB", 0).exists());
    assert_eq!(store.evicted_files(), 4);

    // Ids still rise after the trim (the current file holds the newest).
    let r = store
        .append("BBBB", rec(EventKind::Notification, 99, "after"))
        .unwrap();
    assert_eq!(r.id, 61);
}

#[test]
fn wp_p2evw_appends_keep_the_folder_under_the_total() {
    let d = dir("cap-on-append");
    let limits = small(1024);
    let cap = 6 * 1024;
    let mut store = EventStore::open(d.clone(), limits, cap).unwrap();
    for i in 0..400 {
        let s = ["AAAA", "BBBB", "CCCC"][i % 3];
        store
            .append(
                s,
                rec(EventKind::Notification, i as i64, &format!("record {i:04}")),
            )
            .unwrap();
        assert!(store.used_bytes() <= cap + 4096, "{}", store.used_bytes());
    }
    let on_disk: u64 = fs::read_dir(&d)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
        .map(|e| e.metadata().unwrap().len())
        .sum();
    assert!(on_disk <= cap, "{on_disk}");
    assert!(store.evicted_files() > 0);
    // Every device keeps its current file and its newest records.
    for s in ["AAAA", "BBBB", "CCCC"] {
        assert!(file(&d, s, 0).exists(), "{s}");
        assert_eq!(
            ids(&store, s).last().copied(),
            Some(400 / 3 + 1 - u64::from(s != "AAAA"))
        );
    }
}

#[test]
fn wp_p2evw_forget_removes_a_device_and_only_it() {
    let d = dir("forget");
    let mut store = EventStore::open(d.clone(), small(1024), HUB_TOTAL_BYTES).unwrap();
    for i in 0..40 {
        store
            .append("AAAA", rec(EventKind::Notification, i, "a"))
            .unwrap();
        store
            .append("BBBB", rec(EventKind::Notification, i, "b"))
            .unwrap();
    }
    fs::write(d.join("notes.txt"), "not ours").unwrap();
    let removed = store.forget("AAAA").unwrap();
    assert!(removed >= 2, "{removed}");
    for n in 0..=3 {
        assert!(!file(&d, "AAAA", n).exists());
    }
    assert!(!d.join(format!("{}.lock", file_stem("AAAA"))).exists());
    assert_eq!(store.devices(), ["BBBB"]);
    assert_eq!(ids(&store, "BBBB").len(), 40);
    assert!(d.join("notes.txt").exists());
    // A forgotten device starts again.
    let r = store
        .append("AAAA", rec(EventKind::Connected, 1, "back"))
        .unwrap();
    assert_eq!(r.id, 1);
    // Forgetting nothing is fine.
    assert_eq!(store.forget("ZZZZ").unwrap(), 0);
}

#[test]
fn wp_p2evw_bad_serials_are_refused() {
    let d = dir("bad-serial");
    let mut store = EventStore::open(d, HUB_LIMITS, HUB_TOTAL_BYTES).unwrap();
    for s in [String::new(), "x".repeat(MAX_SERIAL_BYTES + 1)] {
        assert!(matches!(
            store.append(&s, rec(EventKind::Alarm, 1, "x")),
            Err(EventStoreError::BadSerial)
        ));
        assert!(matches!(store.forget(&s), Err(EventStoreError::BadSerial)));
    }
    // The longest one fits a file name, hex included.
    let s = "é".repeat(MAX_SERIAL_BYTES / 2);
    store.append(&s, rec(EventKind::Alarm, 1, "x")).unwrap();
    assert_eq!(store.devices(), [s]);
}

#[test]
fn wp_p2evw_corrupt_files_do_not_stop_the_store() {
    let d = dir("corrupt");
    fs::write(
        file(&d, "AAAA", 0),
        "garbage\n{\"id\":99999999999999999999}\n[1,2]\n{\"id\":5,\"kind\":\"x\",\"text\":\"ok\"}\n\u{0}\u{0}torn",
    )
    .unwrap();
    fs::write(d.join("events-s-AAAA.zz.jsonl"), "not a rotation").unwrap();
    fs::write(d.join("events-x-zz.jsonl"), "bad hex").unwrap();
    let mut store = EventStore::open(d.clone(), HUB_LIMITS, HUB_TOTAL_BYTES).unwrap();
    let r = store
        .append("AAAA", rec(EventKind::Unresponsive, 1, "silent"))
        .unwrap();
    assert_eq!(r.id, 6);
    let page = store.read("AAAA", 0, 100);
    assert_eq!(
        page.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        [5, 6]
    );
    assert_eq!(page.records[0].kind, EventKind::Unknown);
}

/// The record as a build before WP P2EVW read it: the same kinds list up to
/// `audio_link`, an unknown kind as `unknown`, and no `by`.
#[derive(Debug, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum OldKind {
    Notification,
    Connected,
    Disconnected,
    Clip,
    Temperature,
    Counter,
    Reset,
    Firmware,
    Import,
    Recall,
    #[serde(rename = "audio_link", alias = "alink")]
    Alink,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, serde::Deserialize)]
struct OldRecord {
    id: u64,
    kind: OldKind,
    text: String,
    #[serde(default)]
    session: Option<u32>,
}

#[test]
fn wp_p2evw_new_kinds_and_by_round_trip_and_read_as_unknown_in_an_old_build() {
    let d = dir("kinds");
    let mut store = EventStore::open(d.clone(), HUB_LIMITS, HUB_TOTAL_BYTES).unwrap();
    let kinds = [
        (EventKind::Unresponsive, "unresponsive"),
        (EventKind::Alarm, "alarm"),
        (EventKind::Schedule, "schedule"),
        (EventKind::Access, "access"),
        (EventKind::EventsDropped, "events_dropped"),
    ];
    for (i, (k, name)) in kinds.iter().enumerate() {
        assert_eq!(k.as_str(), *name);
        let mut r = rec(*k, i as i64, name);
        r.by = Some("Schedule: Morning".into());
        r.session = Some(7);
        store.append("AAAA", r).unwrap();
    }
    // A record with no `by` writes no `by`.
    store
        .append("AAAA", rec(EventKind::Clip, 9, "plain"))
        .unwrap();
    let export = store.export("AAAA");
    let lines: Vec<&str> = export.lines().collect();
    assert_eq!(lines.len(), 6);
    for (line, (k, name)) in lines.iter().zip(kinds.iter()) {
        assert!(line.contains(&format!("\"kind\":\"{name}\"")), "{line}");
        let now = parse_line(line).unwrap();
        assert_eq!(now.kind, *k);
        assert_eq!(now.by.as_deref(), Some("Schedule: Morning"));
        let old: OldRecord = serde_json::from_str(line).unwrap();
        assert_eq!(old.kind, OldKind::Unknown, "{line}");
        assert_eq!(old.text, *name);
        assert_eq!(old.session, Some(7));
        assert_eq!(old.id, now.id);
    }
    assert!(!lines[5].contains("\"by\""));
    let old: OldRecord = serde_json::from_str(lines[5]).unwrap();
    assert_eq!(old.kind, OldKind::Clip);
}
