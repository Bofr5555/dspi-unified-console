//! WP P2IN: the inventory, as pure state and through the hub (in process, with a data
//! folder under the branch's work folder).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dspi_proto::ext::{Chip, ResetReason};
use dspi_session::events::{EventKind, EventRecord, EventStore, HUB_LIMITS, HUB_TOTAL_BYTES};
use dspi_session::snapshot_store::SnapshotStore;
use dspi_transport::{Capture, FakeDevice, FakeOptions};

use super::*;
use crate::clock::TestClock;
use crate::hub::{Hub, HubConfig};

const WAIT: Duration = Duration::from_secs(20);
const T0: u64 = 1_763_200_000; // 2025-11-15

/// A fresh folder. `ci/run.sh` points `TMPDIR` at the branch's work folder
/// (`~/.cache/dspi-build/work/wp-P2IN/...`), never tmpfs.
fn fresh(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-p2in").join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn seen(serial: &str, handle: u8, fw: &str) -> Seen {
    Seen {
        serial: serial.into(),
        handle,
        name: format!("Board {serial}"),
        platform: 1,
        fw: fw.into(),
        fw_beta: Some(0),
        outputs: 9,
        board: None, // -- WP PGZERO --
    }
}

// ── Pure state ──────────────────────────────────────────────────────────────

#[test]
fn wp_p2in_seen_detached_and_reread_keeps_first_seen_and_moves_last_seen() {
    let mut book = Book::new();
    let c = book.reconcile(&[seen("S1", 0, "1.1.6")], T0);
    assert_eq!(c, vec![Change::Updated("S1".into())]);
    let r = book.get("S1").unwrap();
    assert_eq!((r.first_seen_s, r.last_seen_s), (T0, T0));
    assert_eq!(book.handle("S1"), Some(0));
    // Still online a tick later: nothing changed, nothing to tell.
    assert!(book.reconcile(&[seen("S1", 0, "1.1.6")], T0 + 1).is_empty());

    // Detached.
    let c = book.reconcile(&[], T0 + 100);
    assert_eq!(c, vec![Change::Updated("S1".into())]);
    assert_eq!(book.handle("S1"), None);
    assert_eq!(book.get("S1").unwrap().last_seen_s, T0 + 100);
    let v = book.json("S1", T0 + 200).unwrap();
    assert_eq!(v["online"], false);
    assert_eq!(v["handle"], Value::Null);
    assert_eq!(v["first_seen"], iso8601(T0));
    assert_eq!(v["last_seen"], iso8601(T0 + 100));

    // The hub restarts: the file, read back.
    let (mut again, whole) = Book::from_file(&book.to_file(), MAX_DEVICES);
    assert!(whole);
    assert_eq!(again.get("S1"), book.get("S1"));
    again.reconcile(&[seen("S1", 3, "1.1.6")], T0 + 500);
    let r = again.get("S1").unwrap();
    assert_eq!(r.first_seen_s, T0, "first seen is kept");
    assert_eq!(r.last_seen_s, T0 + 500, "last seen moves");
    assert_eq!(again.handle("S1"), Some(3));
    let v = again.json("S1", T0 + 600).unwrap();
    assert_eq!(v["online"], true);
    assert_eq!(v["last_seen"], iso8601(T0 + 600), "online: seen now");
}

#[test]
fn wp_p2in_a_firmware_change_is_a_change_and_updates_the_record() {
    let mut book = Book::new();
    book.reconcile(&[seen("S1", 0, "1.1.6")], T0);
    book.reconcile(&[], T0 + 1);
    let c = book.reconcile(&[seen("S1", 0, "1.1.7")], T0 + 2);
    assert!(c.contains(&Change::Firmware {
        serial: "S1".into(),
        old: "1.1.6".into(),
        new: "1.1.7".into()
    }));
    assert_eq!(book.get("S1").unwrap().fw, "1.1.7");
    // The first sighting is no change of firmware.
    let c = book.reconcile(&[seen("S1", 0, "1.1.7"), seen("S2", 1, "1.1.6")], T0 + 3);
    assert!(!c.iter().any(|c| matches!(c, Change::Firmware { .. })));
}

#[test]
fn wp_p2in_facts_fill_identify_and_the_last_reset() {
    let mut book = Book::new();
    book.reconcile(&[seen("S1", 0, "1.1.6")], T0);
    let identify = IdentifyStatus {
        remaining_s: 0,
        last_tone: None,
        chip: Chip::Rp2350,
        chip_rev: 2,
        chip_id: 0x1234,
        board: "pico2".into(),
        hw_rev: "K340 rev B".into(),
    };
    let reset = ResetInfo {
        reason: ResetReason::Watchdog,
        hang_count: 1,
        chip: Chip::Rp2350,
        uptime_s: 30,
        prev_uptime_s: Some(3600),
        chip_reset: 0,
        watchdog_reason: 0,
    };
    let c = book.facts("S1", Some(&identify), Some(&reset), "1.1.6", T0 + 30);
    assert_eq!(c, vec![Change::Updated("S1".into())]);
    let v = book.json("S1", T0 + 30).unwrap();
    assert_eq!(v["board"], "pico2");
    assert_eq!(v["hw_rev"], "K340 rev B");
    assert_eq!(v["chip"], "RP2350");
    assert_eq!(v["chip_rev"], 2);
    assert_eq!(v["last_reset"]["reason"], "watchdog");
    assert_eq!(v["last_reset"]["prev_uptime_s"], 3600);
    assert_eq!(v["last_reset"]["at"], iso8601(T0));
    // The same boot read again a minute later is no change.
    let later = ResetInfo {
        uptime_s: 90,
        ..reset
    };
    assert!(
        book.facts("S1", Some(&identify), Some(&later), "1.1.6", T0 + 91)
            .is_empty()
    );
    // A device without the ext IDs: nothing of them, no change.
    book.reconcile(&[seen("S1", 0, "1.1.6"), seen("S2", 1, "1.1.6")], T0 + 92);
    assert!(book.facts("S2", None, None, "1.1.6", T0 + 93).is_empty());
    assert!(book.json("S2", T0).unwrap().get("board").is_none());
    // Unknown serials are ignored.
    assert!(book.facts("S9", None, None, "1.1.6", T0).is_empty());
}

#[test]
fn wp_p2in_past_the_cap_the_oldest_offline_goes() {
    let mut book = Book::with_cap(3);
    for (i, s) in ["A", "B", "C"].iter().enumerate() {
        book.reconcile(&[seen(s, 0, "1")], T0 + i as u64 * 10);
        book.reconcile(&[], T0 + i as u64 * 10 + 1);
    }
    // B seen again later than A.
    book.reconcile(&[seen("A", 0, "1")], T0 + 100);
    book.reconcile(&[], T0 + 101);
    let c = book.reconcile(&[seen("D", 0, "1")], T0 + 200);
    assert!(c.contains(&Change::Removed("B".into())), "{c:?}");
    assert_eq!(book.len(), 3);
    assert!(book.get("B").is_none());

    // The real cap: 1024.
    let mut book = Book::new();
    for i in 0..=MAX_DEVICES {
        let s = format!("S{i:05}");
        book.reconcile(&[seen(&s, 0, "1")], T0 + i as u64);
    }
    book.reconcile(&[], T0 + 5000);
    assert_eq!(book.len(), MAX_DEVICES);
    assert!(book.get("S00000").is_none(), "the oldest went");
    assert!(book.get(&format!("S{MAX_DEVICES:05}")).is_some());
}

#[test]
fn wp_p2in_set_is_bounded_and_never_partial() {
    let mut book = Book::new();
    book.reconcile(&[seen("S1", 0, "1")], T0);
    let ok = Edit {
        location: Some(Some("Stage left — rack 2 ✓".into())),
        note: Some(Some("é".repeat(MAX_TEXT_CHARS))),
        tags: Some(vec!["fohs".into(), " amp ".into(), "fohs".into()]),
    };
    assert_eq!(book.set("S1", ok).unwrap().len(), 1);
    let r = book.get("S1").unwrap().clone();
    assert_eq!(r.tags, vec!["fohs".to_string(), "amp".to_string()]);
    assert_eq!(r.note.as_ref().unwrap().chars().count(), MAX_TEXT_CHARS);

    let refused = [
        Edit {
            location: Some(Some("x".repeat(MAX_TEXT_CHARS + 1))),
            ..Edit::default()
        },
        Edit {
            note: Some(Some("bell\u{7}".into())),
            ..Edit::default()
        },
        Edit {
            location: Some(None),
            tags: Some((0..=MAX_TAGS).map(|i| format!("t{i}")).collect()),
            ..Edit::default()
        },
        Edit {
            location: Some(None),
            tags: Some(vec!["x".repeat(MAX_TAG_CHARS + 1)]),
            ..Edit::default()
        },
        Edit {
            tags: Some(vec!["  ".into()]),
            ..Edit::default()
        },
    ];
    for edit in refused {
        let e = book.set("S1", edit.clone()).unwrap_err();
        assert_eq!(e.code(), "bad_request", "{edit:?}");
        assert_eq!(book.get("S1").unwrap(), &r, "nothing changed: {edit:?}");
    }
    // `null` clears; absent keeps.
    book.set(
        "S1",
        Edit {
            location: Some(None),
            tags: Some(Vec::new()),
            ..Edit::default()
        },
    )
    .unwrap();
    let r2 = book.get("S1").unwrap();
    assert_eq!(r2.location, None);
    assert!(r2.tags.is_empty());
    assert_eq!(r2.note, r.note);
    assert_eq!(
        book.set("S9", Edit::default()).unwrap_err().code(),
        "not_found"
    );
}

#[test]
fn wp_p2in_forget_while_online_is_refused() {
    let mut book = Book::new();
    book.reconcile(&[seen("S1", 0, "1")], T0);
    assert_eq!(book.forget("S1").unwrap_err().code(), "conflict");
    assert_eq!(book.forget("S9").unwrap_err().code(), "not_found");
    book.reconcile(&[], T0 + 1);
    assert_eq!(book.forget("S1").unwrap().serial, "S1");
    assert!(book.is_empty());
}

#[test]
fn wp_p2in_a_torn_file_reads_what_it_can_and_is_kept_aside() {
    let mut book = Book::new();
    for (i, s) in ["A", "B", "C"].iter().enumerate() {
        book.reconcile(&[seen(s, 0, "1.1.6")], T0 + i as u64);
    }
    book.set(
        "A",
        Edit {
            note: Some(Some("line one\nline, two }".into())),
            ..Edit::default()
        },
    )
    .unwrap();
    let text = book.to_file();
    // Torn in the middle of the last record.
    let cut_at = text.rfind("\"serial\":\"C\"").unwrap() + 5;
    let torn = &text[..cut_at];
    let (read, whole) = Book::from_file(torn, MAX_DEVICES);
    assert!(!whole);
    assert_eq!(read.len(), 2);
    assert_eq!(
        read.get("A"),
        book.get("A"),
        "a note with a newline survives"
    );
    // Garbage reads as nothing, never a panic.
    for junk in [
        "",
        "{",
        "[]",
        "\u{0}\u{1}",
        "{\"v\":1,\"devices\":7}",
        "null",
    ] {
        let (b, _) = Book::from_file(junk, MAX_DEVICES);
        assert!(b.is_empty(), "{junk:?}");
    }
    // A bad record among good ones: the others are kept.
    let mixed = text.replacen("\"serial\":\"B\"", "\"serial\":\"\"", 1);
    let (b, whole) = Book::from_file(&mixed, MAX_DEVICES);
    assert!(!whole);
    assert_eq!(b.len(), 2);

    // Through `load`: kept aside, recovered, to be written again.
    let dir = fresh("torn");
    let path = dir.join(FILE_NAME);
    fs::write(&path, torn).unwrap();
    let (loaded, rewrite) = load(&path, T0);
    assert!(rewrite);
    assert_eq!(loaded.len(), 2);
    assert!(!path.exists());
    assert!(dir.join(format!("{FILE_NAME}.bad-{T0}")).exists());
    // A whole file: nothing set aside.
    fs::write(&path, &text).unwrap();
    let (loaded, rewrite) = load(&path, T0);
    assert!(!rewrite);
    assert_eq!(loaded.len(), 3);
}

#[test]
fn wp_p2in_iso8601() {
    assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601(1_763_199_000), "2025-11-15T09:30:00Z");
}

// ── Through the hub ─────────────────────────────────────────────────────────

fn hub_at(dir: &Path, clock: &Arc<TestClock>) -> Hub {
    Hub::start(HubConfig {
        data_dir: Some(dir.to_path_buf()),
        offline_grace: Duration::from_millis(50),
        initial_scan: false,
        services: ServiceConfig {
            clock: Arc::clone(clock) as Arc<dyn HubClock>,
            inventory: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2IN hub")
    })
    .unwrap()
}

fn ask(hub: &HubHandle, session: &Arc<Session>, msg: Value) -> Value {
    let text = hub
        .service_message(session, &msg)
        .expect("an inventory answer");
    serde_json::from_str(&text).unwrap()
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn device(hub: &HubHandle, session: &Arc<Session>, serial: &str) -> Value {
    let v = ask(
        hub,
        session,
        json!({"t": "inventory.list", "id": 1, "serial": serial}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    v["devices"][0].clone()
}

fn wall(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

#[test]
fn wp_p2in_a_device_keeps_first_seen_across_detach_and_a_hub_restart() {
    let dir = fresh("restart");
    let clock = Arc::new(TestClock::at(wall(T0)));
    let hub = hub_at(&dir, &clock);
    let h = hub.handle();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2IN-R")), 0)
        .unwrap();
    let d = device(&h, &viewer, "P2IN-R");
    assert_eq!(d["online"], true);
    assert_eq!(d["handle"], handle);
    assert_eq!(d["first_seen"], iso8601(T0));
    assert_eq!(d["platform"], 1);
    assert_eq!(d["outputs"], 9);

    clock.advance(Duration::from_secs(60));
    hub.detach(handle).unwrap();
    wait_until("offline", || {
        device(&h, &viewer, "P2IN-R")["online"] == false
    });
    let d = device(&h, &viewer, "P2IN-R");
    assert_eq!(d["last_seen"], iso8601(T0 + 60));
    assert!(dir.join(FILE_NAME).exists());
    hub.stop();

    // A new hub on the same folder, later.
    clock.advance(Duration::from_secs(3600));
    let hub = hub_at(&dir, &clock);
    let h = hub.handle();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    let d = device(&h, &viewer, "P2IN-R");
    assert_eq!(d["online"], false, "known, not attached");
    assert_eq!(d["first_seen"], iso8601(T0));
    hub.attach(Box::new(FakeDevice::demo().with_serial("P2IN-R")), 0)
        .unwrap();
    wait_until("online", || device(&h, &viewer, "P2IN-R")["online"] == true);
    let d = device(&h, &viewer, "P2IN-R");
    assert_eq!(d["first_seen"], iso8601(T0), "first seen is kept");
    assert_eq!(d["last_seen"], iso8601(T0 + 3660), "last seen moved on");
    // `device.list` is untouched: still the registry's, by handle.
    assert_eq!(h.devices().len(), 1);
    hub.stop();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// The demo device with firmware patch level `patch` (`GET_PLATFORM`'s byte 5).
fn demo_with_patch(serial: &str, patch: u8) -> FakeDevice {
    let mut capture = Capture::demo();
    capture.platform[5] = patch;
    FakeDevice::new(
        capture,
        FakeOptions {
            extensions: true,
            surfaces: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(serial)
}

#[test]
fn wp_p2in_a_firmware_change_updates_the_record_and_puts_a_firmware_event_on_the_bus() {
    let dir = fresh("firmware");
    let clock = Arc::new(TestClock::at(wall(T0)));
    let hub = hub_at(&dir, &clock);
    let h = hub.handle();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    let sub = h.bus().subscribe("p2in-test", 256);
    let handle = hub
        .attach(Box::new(demo_with_patch("P2IN-F", 6)), 0)
        .unwrap();
    let old = device(&h, &viewer, "P2IN-F")["fw"]
        .as_str()
        .unwrap()
        .to_owned();
    hub.detach(handle).unwrap();
    wait_until("offline", || {
        device(&h, &viewer, "P2IN-F")["online"] == false
    });
    hub.attach(Box::new(demo_with_patch("P2IN-F", 7)), 0)
        .unwrap();
    wait_until("online", || device(&h, &viewer, "P2IN-F")["online"] == true);
    let new = device(&h, &viewer, "P2IN-F")["fw"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(old, new);

    let start = Instant::now();
    let record = loop {
        assert!(start.elapsed() < WAIT, "no firmware event");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(50))
            && let BusMessage::Event { serial, record } = &*m
            && record.kind == EventKind::Firmware
        {
            assert_eq!(serial, "P2IN-F");
            break record.clone();
        }
    };
    assert_eq!(record.fw.as_deref(), Some(new.as_str()));
    assert!(record.text.contains(&old), "{}", record.text);
    assert_eq!(record.t, i64::try_from(T0 * 1000).unwrap());
    hub.stop();
}

#[test]
fn wp_p2in_facts_on_the_bus_reach_the_record() {
    let clock = Arc::new(TestClock::at(wall(T0)));
    let dir = fresh("facts");
    let hub = hub_at(&dir, &clock);
    let h = hub.handle();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    hub.attach(Box::new(FakeDevice::demo().with_serial("P2IN-X")), 0)
        .unwrap();
    let fw = device(&h, &viewer, "P2IN-X")["fw"]
        .as_str()
        .unwrap()
        .to_owned();
    h.bus().publish(BusMessage::Facts {
        serial: "P2IN-X".into(),
        identify: Some(IdentifyStatus {
            remaining_s: 0,
            last_tone: None,
            chip: Chip::Rp2350,
            chip_rev: 2,
            chip_id: 0,
            board: "pico2".into(),
            hw_rev: String::new(),
        }),
        reset_info: Some(ResetInfo {
            reason: ResetReason::Brownout,
            hang_count: 0,
            chip: Chip::Rp2350,
            uptime_s: 10,
            prev_uptime_s: None,
            chip_reset: 0,
            watchdog_reason: 0,
        }),
        fw,
    });
    let d = device(&h, &viewer, "P2IN-X");
    assert_eq!(d["board"], "pico2");
    assert!(d.get("hw_rev").is_none(), "unset is absent: {d}");
    assert_eq!(d["last_reset"]["reason"], "brownout");
    assert_eq!(d["last_reset"]["at"], iso8601(T0 - 10));
    hub.stop();
}

#[test]
fn wp_p2in_admin_sets_control_is_refused_viewer_reads() {
    let clock = Arc::new(TestClock::at(wall(T0)));
    let hub = hub_at(&fresh("roles"), &clock);
    let h = hub.handle();
    let admin = h.open_session(Role::Admin, None).unwrap();
    let control = h.open_session(Role::Control, None).unwrap();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2IN-A")), 0)
        .unwrap();

    let set = json!({"t": "inventory.set", "id": 5, "serial": "P2IN-A",
        "location": "Hall B", "note": "Left array", "tags": ["main", "left"]});
    for s in [&control, &viewer] {
        let v = ask(&h, s, set.clone());
        assert_eq!(v["code"], "denied", "{v}");
        assert_eq!(v["id"], 5);
        let v = ask(
            &h,
            s,
            json!({"t": "inventory.forget", "id": 6, "serial": "P2IN-A"}),
        );
        assert_eq!(v["code"], "denied", "{v}");
    }
    let v = ask(&h, &admin, set);
    assert_eq!(v["t"], "ok", "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["device"]["location"], "Hall B");
    assert_eq!(v["device"]["tags"], json!(["main", "left"]));

    // Every role reads, by serial or handle, or the whole list.
    for s in [&admin, &control, &viewer] {
        let v = ask(&h, s, json!({"t": "inventory.list", "id": 7}));
        assert_eq!(v["t"], "ok", "{v}");
        assert_eq!(v["total"], 1);
        assert_eq!(v["next_offset"], Value::Null);
        assert_eq!(v["devices"][0]["note"], "Left array");
        let v = ask(
            &h,
            s,
            json!({"t": "inventory.list", "id": 8, "handle": handle}),
        );
        assert_eq!(v["devices"][0]["serial"], "P2IN-A", "{v}");
    }
    // Addressing errors.
    let v = ask(
        &h,
        &viewer,
        json!({"t": "inventory.list", "id": 9, "serial": "NOPE"}),
    );
    assert_eq!(v["code"], "not_found");
    let v = ask(
        &h,
        &viewer,
        json!({"t": "inventory.list", "id": 9, "handle": 200}),
    );
    assert_eq!(v["code"], "no_device");
    let v = ask(
        &h,
        &viewer,
        json!({"t": "inventory.list", "id": 9, "handle": handle, "serial": "OTHER"}),
    );
    assert_eq!(v["code"], "bad_request");
    let v = ask(
        &h,
        &admin,
        json!({"t": "inventory.set", "id": 9, "serial": "P2IN-A", "tags": "x"}),
    );
    assert_eq!(v["code"], "bad_request");
    let v = ask(
        &h,
        &viewer,
        json!({"t": "inventory.list", "id": 9, "limit": 501}),
    );
    assert_eq!(v["code"], "bad_request");
    // Not this family's.
    assert!(
        h.service_message(&viewer, &json!({"t": "device.list", "id": 1}))
            .is_none()
    );
    hub.stop();
}

fn client_snapshot(client: &SnapshotStore, serial: &str) -> String {
    let mut doc = json!({
        "meta": {"name": "Venue", "platform": "RP2350", "outputChannelCount": 4, "wireFormatVersion": 3},
        "snapshot": {"image": "AAAA"}
    });
    let e = client.save(serial, &mut doc, 1_700_000_000_000).unwrap();
    client.read(&e.id).unwrap().1
}

#[test]
fn wp_p2in_forget_is_refused_online_and_takes_history_and_asked_for_snapshots() {
    let dir = fresh("forget");
    let clock = Arc::new(TestClock::at(wall(T0)));
    let hub = hub_at(&dir, &clock);
    let h = hub.handle();
    let admin = h.open_session(Role::Admin, None).unwrap();
    let a = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2IN-G1")), 0)
        .unwrap();
    let b = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2IN-G2")), 0)
        .unwrap();
    // Some history and a hub snapshot for each.
    let mut events = EventStore::open(dir.join("events"), HUB_LIMITS, HUB_TOTAL_BYTES).unwrap();
    let snaps = crate::snapshots::HubSnapshots::open(&dir);
    let client = SnapshotStore::new(dir.join("client"));
    for s in ["P2IN-G1", "P2IN-G2"] {
        events
            .append(s, EventRecord::new(EventKind::Connected, 1, "connected"))
            .unwrap();
        snaps.put(s, &client_snapshot(&client, s)).unwrap();
    }

    let v = ask(
        &h,
        &admin,
        json!({"t": "inventory.forget", "id": 1, "serial": "P2IN-G1"}),
    );
    assert_eq!(v["code"], "conflict", "online: {v}");
    assert_eq!(device(&h, &admin, "P2IN-G1")["online"], true);

    hub.detach(a).unwrap();
    hub.detach(b).unwrap();
    wait_until("offline", || {
        device(&h, &admin, "P2IN-G1")["online"] == false
            && device(&h, &admin, "P2IN-G2")["online"] == false
    });
    // Without `with_data`: the record and the history; the snapshots stay.
    let v = ask(
        &h,
        &admin,
        json!({"t": "inventory.forget", "id": 2, "serial": "P2IN-G1"}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    assert_eq!(v["forgot"], json!(["record", "events"]));
    assert!(events.read("P2IN-G1", 0, 10).records.is_empty());
    assert_eq!(snaps.list("P2IN-G1").unwrap().len(), 1);
    let v = ask(
        &h,
        &admin,
        json!({"t": "inventory.list", "id": 3, "serial": "P2IN-G1"}),
    );
    assert_eq!(v["code"], "not_found");
    // With it: the snapshots too.
    let v = ask(
        &h,
        &admin,
        json!({"t": "inventory.forget", "id": 4, "serial": "P2IN-G2", "with_data": true}),
    );
    assert_eq!(v["forgot"], json!(["record", "events", "snapshots"]), "{v}");
    assert!(snaps.list("P2IN-G2").unwrap().is_empty());
    // Forgetting twice: not found.
    let v = ask(
        &h,
        &admin,
        json!({"t": "inventory.forget", "id": 5, "serial": "P2IN-G2"}),
    );
    assert_eq!(v["code"], "not_found");
    // And it stays forgotten in the file.
    hub.stop();
    let (book, rewrite) = load(&dir.join(FILE_NAME), T0);
    assert!(!rewrite, "a whole file");
    assert!(book.is_empty());
}

#[test]
fn wp_p2in_an_in_process_hub_keeps_it_in_memory_and_off_by_default() {
    let clock = Arc::new(TestClock::at(wall(T0)));
    let hub = Hub::start(HubConfig {
        services: ServiceConfig {
            clock: Arc::clone(&clock) as Arc<dyn HubClock>,
            inventory: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2IN mem")
    })
    .unwrap();
    let h = hub.handle();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    hub.attach(Box::new(FakeDevice::demo().with_serial("P2IN-M")), 0)
        .unwrap();
    assert_eq!(device(&h, &viewer, "P2IN-M")["online"], true);
    hub.stop();

    let hub = Hub::start(HubConfig::in_process("P2IN off")).unwrap();
    let h = hub.handle();
    let viewer = h.open_session(Role::Viewer, None).unwrap();
    assert!(
        h.service_message(&viewer, &json!({"t": "inventory.list", "id": 1}))
            .is_none(),
        "off unless asked for"
    );
    assert!(!h.service_caps().contains(&CAP));
    hub.stop();
}

// -- WP PGZERO --
/// The inventory row names the board: a Zero is "DSPi Zero" (`model`) with
/// board id 1; a Pico "DSPi"; a record without a board shows neither, and a
/// later sighting that does not say keeps what was known. The record
/// survives the file.
#[test]
fn wp_pgzero_inventory_row_names_the_zero() {
    use dspi_proto::packets::Board;
    let mut book = Book::new();
    let mut zero = seen("Z1", 0, "1.1.6");
    zero.board = Some(Board::PI_ZERO_2W_ID);
    let mut pico = seen("P1", 1, "1.1.6");
    pico.board = Some(0);
    book.reconcile(&[zero.clone(), pico, seen("U1", 2, "1.1.6")], T0);
    let z = book.json("Z1", T0).unwrap();
    assert_eq!(
        z["model"],
        dspi_transport::board::board_name(Board::PiZero2W)
    );
    assert_eq!(z["model"], "DSPi Zero");
    assert_eq!(z["board_id"], 1);
    assert_eq!(book.json("P1", T0).unwrap()["model"], "DSPi");
    let u = book.json("U1", T0).unwrap();
    assert!(u.get("model").is_none() && u.get("board_id").is_none());
    // A sighting without a board keeps the one known
    zero.board = None;
    book.seen(&zero, T0 + 1);
    assert_eq!(book.get("Z1").unwrap().board_id, Some(1));
    // Through the file format
    let text = serde_json::to_string(book.get("Z1").unwrap()).unwrap();
    let back: Record = serde_json::from_str(&text).unwrap();
    assert_eq!(back.board_id, Some(1));
    // An old file without the field reads as unknown
    let old: Record = serde_json::from_str(r#"{"serial":"O1"}"#).unwrap();
    assert_eq!(old.board_id, None);
}
