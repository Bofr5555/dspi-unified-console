//! WP P0M: the event history's store, file format and watchers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::*;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p0m-events").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn rec(kind: EventKind, t: i64, text: &str) -> EventRecord {
    EventRecord::new(kind, t, text)
}

fn store(d: &Path, limits: Limits) -> EventLog {
    let mut log = EventLog::new(limits);
    log.set_dir(Some(d.to_path_buf())).unwrap();
    log
}

#[test]
fn wp_p0m_records_persist_with_rising_ids() {
    let d = dir("persist");
    let mut log = store(&d, Limits::DEFAULT);
    log.open_device("BB66A44F");
    let mut c = rec(EventKind::Connected, 1000, "Connected");
    c.fw = Some("1.1.6".into());
    c.uptime_s = Some(42);
    log.record(c);
    log.record(rec(EventKind::Clip, 2000, "Clip on OUT 1"));
    log.close_device();

    // A new store (the next run) carries the ids on and remembers the last
    // connection.
    let mut log = store(&d, Limits::DEFAULT);
    let last = log.open_device("BB66A44F");
    assert_eq!(last.fw.as_deref(), Some("1.1.6"));
    assert_eq!(last.uptime, Some((1000, 42)));
    log.record(rec(EventKind::Disconnected, 3000, "Closed"));
    let page = log.read("BB66A44F", 0, 100);
    assert_eq!(
        page.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(page.last_id, 3);
    assert_eq!(page.records[1].serial, "BB66A44F");
    assert_eq!(page.records[1].kind, EventKind::Clip);
    // Paging on from an id.
    let next = log.read("BB66A44F", 2, 100);
    assert_eq!(next.records.len(), 1);
    assert_eq!(log.read("BB66A44F", 3, 100).last_id, 3);
    // A first read with a limit is the newest records.
    let newest = log.read("BB66A44F", 0, 2);
    assert_eq!(
        newest.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(log.devices(), ["BB66A44F"]);
}

#[test]
fn wp_p0m_the_file_format_is_json_lines() {
    let d = dir("format");
    let mut log = store(&d, Limits::DEFAULT);
    log.open_device("SER1");
    let mut r = rec(
        EventKind::Notification,
        5,
        "[  3] [HOST    ] out.gain  -3.0",
    );
    r.source = Some("HOST".into());
    r.session = Some(7);
    log.record(r);
    log.close_device();
    let text = fs::read_to_string(d.join("events-s-SER1.jsonl")).unwrap();
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 1);
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["id"], 1);
    assert_eq!(v["t"], 5);
    assert_eq!(v["kind"], "notification");
    assert_eq!(v["serial"], "SER1");
    assert_eq!(v["source"], "HOST");
    assert_eq!(v["session"], 7);
    // Defaults are left out.
    assert!(v.get("own").is_none() && v.get("count").is_none() && v.get("lines").is_none());
    // The export is the same lines.
    assert_eq!(log.export("SER1"), text);
}

#[test]
fn wp_p0m_odd_serials_get_safe_file_names() {
    assert_eq!(file_stem("ABC-12_x"), "events-s-ABC-12_x");
    assert_eq!(file_stem("a/b"), "events-x-612f62");
    assert_eq!(file_stem(""), "events-x-");
    assert_eq!(file_stem("../x"), "events-x-2e2e2f78");
    assert_eq!(serial_of("events-x-612f62").as_deref(), Some("a/b"));
    assert_eq!(serial_of("events-x-6"), None);
    assert_eq!(serial_of("events-x-zz"), None);
    assert_eq!(serial_of("other"), None);
    let d = dir("names");
    let mut log = store(&d, Limits::DEFAULT);
    log.open_device("a/b é");
    log.record(rec(EventKind::Clip, 1, "x"));
    log.open_device("PLAIN");
    log.record(rec(EventKind::Clip, 1, "x"));
    assert_eq!(log.devices(), ["PLAIN", "a/b é"]);
    assert_eq!(log.read("a/b é", 0, 10).records.len(), 1);
}

#[test]
fn wp_p0m_rotation_keeps_the_history_bounded() {
    let d = dir("rotate");
    let limits = Limits {
        max_file_bytes: 2000,
        rotated_files: 2,
        max_line_bytes: 300,
        memory_records: 10,
    };
    let mut log = store(&d, limits);
    log.open_device("R");
    for i in 0..500 {
        log.record(rec(
            EventKind::Counter,
            i,
            &format!("step {i} {}", "x".repeat(40)),
        ));
    }
    log.close_device();
    let mut files: Vec<String> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "events-s-R.1.jsonl",
            "events-s-R.2.jsonl",
            "events-s-R.jsonl"
        ]
    );
    let mut total = 0;
    for f in &files {
        let len = fs::metadata(d.join(f)).unwrap().len();
        assert!(len <= limits.max_file_bytes, "{f}: {len}");
        total += len;
    }
    assert!(total <= limits.max_file_bytes * 3);
    // What is left is the newest, in order, with no gap.
    let page = log.read("R", 0, usize::MAX);
    let ids: Vec<u64> = page.records.iter().map(|r| r.id).collect();
    assert_eq!(*ids.last().unwrap(), 500);
    assert!(ids.windows(2).all(|w| w[1] == w[0] + 1), "{ids:?}");
    assert!(ids.len() < 500);
    assert_eq!(page.skipped, 0);
}

#[test]
fn wp_p0m_no_rotated_files_and_long_lines() {
    let d = dir("zero");
    let limits = Limits {
        max_file_bytes: 500,
        rotated_files: 0,
        max_line_bytes: 200,
        memory_records: 10,
    };
    let mut log = store(&d, limits);
    log.open_device("Z");
    let mut r = rec(EventKind::Import, 1, &"é\"".repeat(400));
    r.lines = vec!["a change".repeat(20); 30];
    log.record(r);
    for i in 0..20 {
        log.record(rec(EventKind::Clip, i, "clip"));
    }
    let text = fs::read_to_string(d.join("events-s-Z.jsonl")).unwrap();
    assert!(text.len() <= 500);
    for line in text.lines() {
        assert!(line.len() < 200, "{}", line.len());
        assert!(parse_line(line).is_some());
    }
}

#[test]
fn wp_p0m_corrupt_and_partial_files_read_without_panic() {
    let d = dir("corrupt");
    let good =
        |id: u64| format!(r#"{{"v":1,"id":{id},"t":{id},"kind":"clip","serial":"C","text":"ok"}}"#);
    let mut body = String::new();
    body.push_str(&good(2));
    body.push('\n');
    body.push_str("not json at all\n");
    body.push_str("{\"v\":1,\"id\":\n"); // torn
    body.push_str("\u{0}\u{0}\u{0}\n");
    body.push_str(&good(3));
    body.push('\n');
    body.push_str(r#"{"v":9,"id":4,"kind":"from_the_future","text":"new","extra":[1,2]}"#);
    body.push('\n');
    body.push_str(r#"{"id":"three"}"#);
    body.push('\n');
    body.push_str("[]\n\n   \n");
    body.push_str(&good(5)[..20]); // a torn last line, no end
    fs::write(d.join("events-s-C.jsonl"), &body).unwrap();
    // Bytes that are not UTF-8 in a rotated file.
    let mut bad = good(1).into_bytes();
    bad.extend_from_slice(b"\n\xff\xfe\xfd\n");
    fs::write(d.join("events-s-C.1.jsonl"), bad).unwrap();
    // A directory where a rotated file should be.
    fs::create_dir(d.join("events-s-C.2.jsonl")).unwrap();

    let mut log = store(&d, Limits::DEFAULT);
    let page = log.read("C", 0, 100);
    let ids: Vec<u64> = page.records.iter().map(|r| r.id).collect();
    assert_eq!(ids, [1, 2, 3, 4]);
    assert_eq!(page.records[3].kind, EventKind::Unknown);
    assert_eq!(page.skipped, 7);

    // Recording carries on after the highest id, on a line of its own.
    log.open_device("C");
    log.record(rec(EventKind::Clip, 9, "after"));
    let page = log.read("C", 4, 100);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, 5);
    assert_eq!(page.records[0].text, "after");
    let text = fs::read_to_string(d.join("events-s-C.jsonl")).unwrap();
    assert!(text.ends_with("\"text\":\"after\"}\n"), "{text}");
}

#[test]
fn wp_p0m_an_oversized_file_is_read_from_its_tail() {
    let d = dir("huge");
    let limits = Limits {
        max_file_bytes: 1000,
        ..Limits::DEFAULT
    };
    let line = |id: u64| format!(r#"{{"id":{id},"kind":"clip","text":"x"}}"#);
    let body: String = (1..=2000).map(|i| line(i) + "\n").collect();
    assert!(body.len() > 4000 * 4);
    fs::write(d.join("events-s-H.jsonl"), body).unwrap();
    let mut log = store(&d, limits);
    let page = log.read("H", 0, usize::MAX);
    assert_eq!(page.records.last().unwrap().id, 2000);
    assert!(page.records.len() < 200);
    assert_eq!(
        page.skipped, 0,
        "the cut first line is dropped, not counted"
    );
}

#[test]
fn wp_p0m_without_a_directory_the_history_is_in_memory() {
    let mut log = EventLog::new(Limits {
        memory_records: 3,
        ..Limits::DEFAULT
    });
    // Nothing is recorded with no device open.
    log.record(rec(EventKind::Clip, 1, "lost"));
    assert!(log.devices().is_empty());
    log.open_device("M");
    for i in 0..5 {
        log.record(rec(EventKind::Clip, i, "c"));
    }
    let page = log.read("M", 0, 100);
    assert_eq!(
        page.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        [3, 4, 5]
    );
    // Ids carry on across a reopen in the same run.
    log.open_device("M");
    log.record(rec(EventKind::Clip, 9, "c"));
    assert_eq!(log.read("M", 5, 10).records[0].id, 6);
    assert_eq!(log.devices(), ["M"]);
    assert_eq!(log.export("M").lines().count(), 3);
}

#[test]
fn wp_p0m_a_directory_that_cannot_be_made() {
    let d = dir("blocked");
    let file = d.join("a-file");
    fs::write(&file, "x").unwrap();
    let mut log = EventLog::default();
    assert!(log.set_dir(Some(file.join("sub"))).is_err());
    assert!(log.dir().is_none());
    // A directory that goes away later: records fall back to memory.
    let gone = d.join("gone");
    log.set_dir(Some(gone.clone())).unwrap();
    log.open_device("G");
    fs::remove_dir_all(&gone).unwrap();
    fs::write(&gone, "now a file").unwrap();
    log.record(rec(EventKind::Clip, 1, "kept"));
    assert_eq!(log.write_errors(), 1);
    assert_eq!(log.read("G", 0, 10).records[0].text, "kept");
}

#[test]
fn wp_p0m_repeats_coalesce() {
    let mut log = EventLog::default();
    log.open_device("K");
    let drag = |t: i64, v: &str| {
        let mut r = rec(EventKind::Notification, t, &format!("out.gain {v}"));
        r.coalesce = Some("p100:HOST".into());
        r
    };
    log.record(drag(1000, "-1"));
    log.record(drag(1300, "-2"));
    log.record(drag(1900, "-3"));
    // Past the window from the first: a record of its own.
    log.record(drag(2100, "-4"));
    // Another key flushes.
    log.record(rec(EventKind::Clip, 2200, "clip"));
    let page = log.read("K", 0, 10);
    let got: Vec<(i64, &str, u32)> = page
        .records
        .iter()
        .map(|r| (r.t, r.text.as_str(), r.count))
        .collect();
    assert_eq!(
        got,
        [
            (1000, "out.gain -3", 3),
            (2100, "out.gain -4", 1),
            (2200, "clip", 1)
        ]
    );
    // A held record goes out on tick once its window passed.
    log.record(drag(5000, "-5"));
    log.tick(5500);
    // Not yet past its window; a read writes it out anyway.
    assert_eq!(log.read("K", 3, 10).records.len(), 1);
}

#[test]
fn wp_p0m_tick_flushes_a_held_record() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    struct Sink(Arc<Mutex<Vec<String>>>);
    impl EventSink for Sink {
        fn record(&self, r: &EventRecord) {
            self.0.lock().unwrap().push(format!("{}:{}", r.id, r.text));
        }
    }
    let mut log = EventLog::default();
    log.set_sink(Some(Arc::new(Sink(seen.clone()))));
    log.open_device("S");
    let mut r = rec(EventKind::Notification, 100, "held");
    r.coalesce = Some("k".into());
    log.record(r);
    assert!(seen.lock().unwrap().is_empty());
    log.tick(500);
    assert!(seen.lock().unwrap().is_empty());
    log.tick(1100);
    assert_eq!(*seen.lock().unwrap(), ["1:held"]);
    log.record(rec(EventKind::Clip, 1200, "now"));
    assert_eq!(*seen.lock().unwrap(), ["1:held", "2:now"]);
    log.set_sink(None);
    log.record(rec(EventKind::Clip, 1300, "quiet"));
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn wp_p0m_temperature_crossings_have_hysteresis() {
    let mut w = Watch::new();
    assert!(w.temperature(4500, 0).is_none(), "a normal first reading");
    assert!(w.temperature(6400, 1).is_none());
    let r = w.temperature(6500, 2).unwrap();
    assert_eq!(r.kind, EventKind::Temperature);
    assert_eq!(r.source.as_deref(), Some("warm"));
    assert_eq!(r.value, Some(65.0));
    // On the edge: no flapping.
    assert!(w.temperature(6450, 3).is_none());
    assert!(w.temperature(6300, 4).is_none());
    assert_eq!(
        w.temperature(7600, 5).unwrap().source.as_deref(),
        Some("hot")
    );
    assert!(w.temperature(7300, 6).is_none());
    assert_eq!(
        w.temperature(7100, 7).unwrap().source.as_deref(),
        Some("warm")
    );
    assert_eq!(
        w.temperature(3000, 8).unwrap().source.as_deref(),
        Some("normal")
    );
    // A hot first reading is worth a record.
    let mut w = Watch::new();
    assert_eq!(
        w.temperature(8000, 0).unwrap().source.as_deref(),
        Some("hot")
    );
    // Straight from hot to normal.
    assert_eq!(
        w.temperature(2000, 1).unwrap().source.as_deref(),
        Some("normal")
    );
}

#[test]
fn wp_p0m_counters_record_steps_rate_limited() {
    let mut w = Watch::new();
    assert!(
        w.counter("starvation", "Starvation", 5, 0).is_none(),
        "baseline"
    );
    assert!(w.counter("starvation", "Starvation", 5, 1000).is_none());
    let r = w.counter("starvation", "Starvation", 7, 2000).unwrap();
    assert_eq!(r.kind, EventKind::Counter);
    assert_eq!((r.count, r.value), (2, Some(7.0)));
    assert_eq!(r.text, "Starvation +2 (total 7)");
    assert_eq!(r.source.as_deref(), Some("starvation"));
    // Within the interval the steps add up for the next record.
    assert!(w.counter("starvation", "Starvation", 8, 3000).is_none());
    assert!(w.counter("starvation", "Starvation", 10, 20_000).is_none());
    let r = w.counter("starvation", "Starvation", 11, 40_000).unwrap();
    assert_eq!(r.count, 4);
    // A counter that went down was reset: a new baseline, no record.
    assert!(w.counter("starvation", "Starvation", 0, 80_000).is_none());
    assert_eq!(
        w.counter("starvation", "Starvation", 1, 90_000)
            .unwrap()
            .count,
        1
    );
    // Counters are apart.
    assert!(w.counter("ring.underruns", "Underruns", 100, 0).is_none());
}

#[test]
fn wp_p0m_restarts_from_uptime_and_hangs() {
    let mut w = Watch::new();
    assert!(w.uptime(100, Some(0), 0).is_none());
    // 60 s later, up 160 s: the same run.
    assert!(w.uptime(160, Some(0), 60_000).is_none());
    // Up 5 s: it restarted.
    let r = w.uptime(5, Some(0), 120_000).unwrap();
    assert_eq!(r.kind, EventKind::Reset);
    assert_eq!(r.uptime_s, Some(5));
    assert_eq!(r.source.as_deref(), Some("unknown"));
    // A hang count step names the watchdog.
    let r = w.uptime(7, Some(1), 122_000).unwrap();
    assert_eq!(r.source.as_deref(), Some("hang"));
    assert!(w.uptime(9, None, 124_000).is_none());
    // Across connections, from the persisted last uptime.
    assert!(restarted((0, 1000), 20, 60_000));
    assert!(!restarted((0, 1000), 1055, 60_000));
    // A wall clock that went backwards is no restart.
    assert!(!restarted((60_000, 1000), 1000, 0));
}

#[test]
fn wp_p0m_ext_events_are_stubbed_until_fwx() {
    assert_eq!(decode_ext_event(Some(u32::MAX), 0x20, &[1, 2, 3]), None);
    assert_eq!(ResetReason::Brownout.as_str(), "brownout");
}

#[test]
fn proptest_wp_p0m_any_bytes_read_without_panic() {
    use proptest::prelude::*;
    let d = dir("prop");
    proptest!(ProptestConfig::with_cases(64), |(bytes in proptest::collection::vec(any::<u8>(), 0..3000))| {
        fs::write(d.join("events-s-P.jsonl"), &bytes).unwrap();
        let mut log = store(&d, Limits { max_file_bytes: 512, ..Limits::DEFAULT });
        let page = log.read("P", 0, usize::MAX);
        log.open_device("P");
        log.record(rec(EventKind::Clip, 1, "x"));
        let after = log.read("P", page.last_id, usize::MAX);
        prop_assert_eq!(after.records.len(), 1);
    });
}
