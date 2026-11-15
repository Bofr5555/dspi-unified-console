//! WP P2EV: the history's pure state, without threads or sockets: what each bus message
//! records, coalescing, the watchdog and restart records, lost messages, paging and the
//! export.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_proto::ext::{Chip, ResetInfo, ResetReason};
use dspi_session::notify::{PROTOCOL_VERSION, evt};

use super::*;
use crate::bus::TargetResult;

const S: &str = "P2EV-UNIT";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/wp-P2EV, not tmpfs.
    let dir = std::env::temp_dir().join("wp-P2EV").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn history(name: &str) -> (History, PathBuf) {
    let dir = scratch(name);
    (
        History::open(dir.clone(), &HistoryConfig::default()).unwrap(),
        dir,
    )
}

struct Time {
    ms: i64,
    mono: Instant,
}

impl Time {
    fn new() -> Self {
        Self {
            ms: 1_763_200_000_000,
            mono: Instant::now(),
        }
    }

    fn now(&self) -> Now {
        Now {
            ms: self.ms,
            mono: self.mono,
        }
    }

    fn advance(&mut self, ms: i64) {
        self.ms += ms;
        self.mono += Duration::from_millis(u64::try_from(ms).unwrap());
    }
}

fn online(h: &mut History, t: &Time, serial: &str) -> Vec<EventRecord> {
    h.on_message(
        &BusMessage::DeviceOnline {
            serial: serial.into(),
            handle: 0,
        },
        t.now(),
    )
}

/// A `PARAM_CHANGED` of 4 bytes at `offset` from `source`.
fn param_changed(offset: u16, source: u8, seq: u8) -> Vec<u8> {
    let mut p = vec![PROTOCOL_VERSION, evt::PARAM_CHANGED, 0, seq];
    p.extend(offset.to_le_bytes());
    p.extend(4u16.to_le_bytes());
    p.extend([source, 0, 0, 0]);
    p.extend((-12.0f32).to_le_bytes());
    p
}

fn note(session: u16, by: Option<&str>, packet: Vec<u8>) -> BusMessage {
    BusMessage::Notification {
        serial: S.into(),
        session,
        by: by.map(str::to_owned),
        packet: Arc::from(packet),
    }
}

fn all(h: &mut History, serial: &str) -> Vec<EventRecord> {
    let mut written = Vec::new();
    let list = h.list(serial, 0, PAGE_MAX, &mut written);
    serde_json::from_value(list["records"].clone()).unwrap()
}

fn facts(reason: ResetReason, uptime_s: u32, fw: &str) -> BusMessage {
    BusMessage::Facts {
        serial: S.into(),
        identify: None,
        reset_info: Some(ResetInfo {
            reason,
            hang_count: 1,
            chip: Chip::Rp2350,
            uptime_s,
            prev_uptime_s: Some(3600),
            chip_reset: 0,
            watchdog_reason: 0,
        }),
        fw: fw.into(),
    }
}

fn health(temp_c: Option<f32>, clip_mask: u32, uptime_s: Option<u32>) -> BusMessage {
    BusMessage::Health {
        serial: S.into(),
        cpu: Some(20),
        clip_mask,
        temp_c,
        uptime_s,
        hang_count: Some(0),
    }
}

#[test]
fn wp_p2ev_attach_and_detach_record_connected_and_disconnected() {
    let (mut h, _dir) = history("attach");
    let t = Time::new();
    let w = online(&mut h, &t, S);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].kind, EventKind::Connected);
    assert_eq!(w[0].id, 1);
    let w = h.on_message(
        &BusMessage::DeviceOffline {
            serial: S.into(),
            handle: 0,
        },
        t.now(),
    );
    assert_eq!(w[0].kind, EventKind::Disconnected);
    online(&mut h, &t, S);
    let kinds: Vec<EventKind> = all(&mut h, S).iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [
            EventKind::Connected,
            EventKind::Disconnected,
            EventKind::Connected
        ]
    );
}

#[test]
fn wp_p2ev_a_slider_sweep_is_one_record_with_its_writer() {
    let (mut h, _dir) = history("sweep");
    let mut t = Time::new();
    online(&mut h, &t, S);
    for seq in 0..20u8 {
        let w = h.on_message(
            &note(7, Some("Front of house"), param_changed(4, 1, seq)),
            t.now(),
        );
        assert!(w.is_empty(), "held back while it coalesces");
        t.advance(30);
    }
    // Another session's change, between the sweep's, is a record of its own and leaves
    // the sweep one record.
    let w = h.on_message(
        &note(9, Some("Back office"), param_changed(4, 1, 21)),
        t.now(),
    );
    assert!(w.is_empty());
    h.on_message(
        &note(7, Some("Front of house"), param_changed(4, 1, 22)),
        t.now(),
    );
    // Both are written by their window passing.
    t.advance(COALESCE_MS);
    let w = h.tick(t.now());
    assert_eq!(w.len(), 2);
    let sweep = &w[0];
    assert_eq!(sweep.kind, EventKind::Notification);
    assert_eq!(sweep.count, 21);
    assert_eq!(sweep.session, Some(7));
    assert_eq!(sweep.by.as_deref(), Some("Front of house"));
    assert_eq!(sweep.source.as_deref(), Some("HOST"));
    assert!(sweep.text.contains("changed"), "{}", sweep.text);
    assert_eq!(w[1].by.as_deref(), Some("Back office"));
    assert_eq!(w[1].count, 1);
    // A record that does not coalesce writes out what is held first.
    h.on_message(&note(7, None, param_changed(4, 1, 23)), t.now());
    let w = h.on_message(
        &BusMessage::DeviceOffline {
            serial: S.into(),
            handle: 0,
        },
        t.now(),
    );
    let kinds: Vec<EventKind> = w.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, [EventKind::Notification, EventKind::Disconnected]);
    // At most MAX_HELD keys are held: the oldest goes out first.
    let mut written = 0;
    for off in 0..=MAX_HELD {
        written += h
            .on_message(
                &note(
                    0,
                    None,
                    param_changed(u16::try_from(off).unwrap() * 4, 1, 0),
                ),
                t.now(),
            )
            .len();
    }
    assert_eq!(written, 1);
    // A device's own change (session 0) has no session and no `by`.
    h.on_message(&note(0, None, param_changed(8, 1, 22)), t.now());
    let last = all(&mut h, S).pop().unwrap();
    assert_eq!(last.session, None);
    assert_eq!(last.by, None);
}

#[test]
fn wp_p2ev_idle_and_legacy_packets_record_nothing_and_junk_is_kept_readable() {
    let (mut h, _dir) = history("junk");
    let t = Time::new();
    online(&mut h, &t, S);
    assert!(h.on_message(&note(0, None, vec![0]), t.now()).is_empty());
    let mut legacy = vec![evt::MASTER_VOLUME, 0, 0, 0];
    legacy.extend((-6.0f32).to_le_bytes());
    assert!(h.on_message(&note(0, None, legacy), t.now()).is_empty());
    // A wrong version, a short packet: one readable record each, never a panic.
    h.on_message(&note(0, None, vec![9, 2, 0, 0, 1]), t.now());
    h.on_message(
        &note(0, None, vec![PROTOCOL_VERSION, evt::PARAM_CHANGED, 0, 0, 1]),
        t.now(),
    );
    let records = all(&mut h, S);
    assert_eq!(records.len(), 2, "connected, then the coalesced junk");
    assert_eq!(records[1].count, 2);
    assert!(
        records[1].text.contains("could not read"),
        "{}",
        records[1].text
    );
}

#[test]
fn wp_p2ev_clip_events_are_read_only_on_extension_firmware() {
    let (mut h, _dir) = history("clip");
    let t = Time::new();
    online(&mut h, &t, S);
    let mut clip = vec![PROTOCOL_VERSION, 0xE0, 0, 1, 0, 1, 0, 0];
    clip.extend(0b101u32.to_le_bytes());
    // Before its facts: an unknown event, plainly.
    let w = h.on_message(&note(0, None, clip.clone()), t.now());
    assert_eq!(w[0].kind, EventKind::Notification);
    assert!(w[0].text.contains("0xE0"), "{}", w[0].text);
    // On extension firmware: one clip record per channel.
    h.on_message(&facts(ResetReason::PowerOn, 100, "1.1.7"), t.now());
    let w = h.on_message(&note(0, None, clip), t.now());
    let clips: Vec<Option<u8>> = w
        .iter()
        .filter(|r| r.kind == EventKind::Clip)
        .map(|r| r.channel)
        .collect();
    assert_eq!(clips, [Some(0), Some(2)]);
    // And the latch is then left to the events: no second record for the same clip.
    let w = h.on_message(&health(None, 0b101, None), t.now());
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn wp_p2ev_the_clip_latch_records_each_new_clip_once() {
    let (mut h, _dir) = history("latch");
    let t = Time::new();
    online(&mut h, &t, S);
    let w = h.on_message(&health(None, 0b10, None), t.now());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].channel, Some(1));
    assert!(w[0].text.contains("before the hub looked"), "{}", w[0].text);
    assert!(h.on_message(&health(None, 0b10, None), t.now()).is_empty());
    let w = h.on_message(&health(None, 0b11, None), t.now());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].channel, Some(0));
    assert_eq!(w[0].source.as_deref(), Some("latch"));
    // Someone cleared it; a clip after that is new again.
    assert!(h.on_message(&health(None, 0, None), t.now()).is_empty());
    assert_eq!(h.on_message(&health(None, 0b1, None), t.now()).len(), 1);
}

#[test]
fn wp_p2ev_temperature_records_crossing_5_degree_bands_without_flapping() {
    let (mut h, _dir) = history("temp");
    let t = Time::new();
    online(&mut h, &t, S);
    let mut temps = |c: f32| -> Vec<EventRecord> {
        h.on_message(&health(Some(c), 0, None), t.now())
            .into_iter()
            .filter(|r| r.kind == EventKind::Temperature)
            .collect()
    };
    assert!(temps(42.0).is_empty(), "the first reading is the baseline");
    assert!(temps(44.9).is_empty());
    assert!(temps(45.5).is_empty(), "inside the hysteresis");
    let up = temps(46.2);
    assert_eq!(up.len(), 1);
    assert_eq!(up[0].source.as_deref(), Some("rising"));
    assert_eq!(up[0].value, Some(f64::from(46.2f32)));
    assert!(temps(44.5).is_empty(), "back on the edge: stays");
    let down = temps(43.8);
    assert_eq!(down.len(), 1);
    assert_eq!(down[0].source.as_deref(), Some("falling"));
    assert!(temps(f32::NAN).is_empty(), "a bad reading is no reading");
    assert_eq!(temps(61.0).len(), 1, "a jump is one record");
}

#[test]
fn wp_p2ev_a_watchdog_reset_is_recorded_once_with_its_reason() {
    let (mut h, dir) = history("reset");
    let mut t = Time::new();
    online(&mut h, &t, S);
    let w = h.on_message(&facts(ResetReason::Watchdog, 12, "1.1.7"), t.now());
    let reset = w.iter().find(|r| r.kind == EventKind::Reset).unwrap();
    assert_eq!(reset.source.as_deref(), Some("watchdog"));
    assert_eq!(reset.uptime_s, Some(12));
    assert!(reset.text.contains("watchdog"), "{}", reset.text);
    assert!(reset.text.contains("3600"), "{}", reset.text);
    assert_eq!(reset.lines, ["hang count 1"]);
    assert!(
        w.iter().all(|r| r.kind != EventKind::Firmware),
        "a firmware change is the inventory's to record"
    );

    // The same facts a minute on (the device kept running): nothing new.
    t.advance(60_000);
    assert!(
        h.on_message(&facts(ResetReason::Watchdog, 72, "1.1.7"), t.now())
            .is_empty()
    );
    // A new hub on the same folder, the device still running: nothing either.
    drop(h);
    let mut h = History::open(dir.clone(), &HistoryConfig::default()).unwrap();
    t.advance(60_000);
    online(&mut h, &t, S);
    assert!(
        h.on_message(&facts(ResetReason::Watchdog, 132, "1.1.7"), t.now())
            .is_empty()
    );
    // It restarted (a power cycle): recorded.
    t.advance(60_000);
    let w = h.on_message(&facts(ResetReason::PowerOn, 3, "1.1.8"), t.now());
    let kinds: Vec<EventKind> = w.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, [EventKind::Reset]);
    assert_eq!(w[0].source.as_deref(), Some("power_on"));
}

#[test]
fn wp_p2ev_the_health_poll_sees_a_restart_and_a_hang() {
    let (mut h, _dir) = history("uptime");
    let mut t = Time::new();
    online(&mut h, &t, S);
    assert!(
        h.on_message(&health(None, 0, Some(500)), t.now())
            .is_empty()
    );
    t.advance(60_000);
    assert!(
        h.on_message(&health(None, 0, Some(560)), t.now())
            .is_empty()
    );
    t.advance(60_000);
    let w = h.on_message(&health(None, 0, Some(5)), t.now());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].kind, EventKind::Reset);
    // The hang count stepping is a watchdog restart.
    t.advance(60_000);
    let hung = BusMessage::Health {
        serial: S.into(),
        cpu: None,
        clip_mask: 0,
        temp_c: None,
        uptime_s: Some(3),
        hang_count: Some(1),
    };
    let w = h.on_message(&hung, t.now());
    assert_eq!(w[0].kind, EventKind::Reset);
    assert_eq!(w[0].source.as_deref(), Some("hang"));
}

#[test]
fn wp_p2ev_unresponsive_alarms_and_schedules_are_recorded() {
    let (mut h, _dir) = history("other");
    let t = Time::new();
    online(&mut h, &t, S);
    let since = u64::try_from(t.ms / 1000).unwrap() - 12;
    let w = h.on_message(
        &BusMessage::Unresponsive {
            serial: S.into(),
            since,
        },
        t.now(),
    );
    assert_eq!(w[0].kind, EventKind::Unresponsive);
    assert_eq!(w[0].value, Some(12.0));

    let alarm = |state: AlarmState, serial: Option<&str>| BusMessage::Alarm {
        id: "device:1".into(),
        source: "device".into(),
        code: "device_restarted".into(),
        subject: S.into(),
        serial: serial.map(str::to_owned),
        severity: "critical".into(),
        state,
        message: "Restarted (watchdog)".into(),
        at: 0,
        raised_at: 0,
        cleared_at: None,
        acknowledged_by: Some("Front of house".into()),
        count: 1,
    };
    let w = h.on_message(&alarm(AlarmState::Raised, Some(S)), t.now());
    assert_eq!(w[0].kind, EventKind::Alarm);
    assert_eq!(w[0].source.as_deref(), Some("device_restarted"));
    assert_eq!(w[0].by, None);
    let w = h.on_message(&alarm(AlarmState::Acknowledged, Some(S)), t.now());
    assert_eq!(w[0].by.as_deref(), Some("Front of house"));
    assert!(
        h.on_message(&alarm(AlarmState::Raised, None), t.now())
            .is_empty(),
        "an audio link alarm is no device's"
    );

    let row = |serial: &str, ok: bool| TargetResult {
        serial: serial.into(),
        action: Some("0:preset.load".into()),
        ok,
        code: (!ok).then(|| "offline".into()),
        msg: None,
    };
    let w = h.on_message(
        &BusMessage::ScheduleRan {
            rule: "r1".into(),
            name: "Morning".into(),
            at: 0,
            by: None,
            results: vec![row(S, true), row("P2EV-OTHER", false)],
        },
        t.now(),
    );
    assert_eq!(w.len(), 2);
    let mine = w.iter().find(|r| r.serial == S).unwrap();
    assert_eq!(mine.kind, EventKind::Schedule);
    assert_eq!(mine.by.as_deref(), Some("Schedule: Morning"));
    assert_eq!(mine.lines, ["0:preset.load: ok"]);
    let other = w.iter().find(|r| r.serial == "P2EV-OTHER").unwrap();
    assert_eq!(other.text, "Schedule Morning failed");
    assert_eq!(other.lines, ["0:preset.load: offline"]);
}

#[test]
fn wp_p2ev_another_services_record_is_written_once() {
    let (mut h, _dir) = history("event");
    let t = Time::new();
    online(&mut h, &t, S);
    let mut r = EventRecord::new(EventKind::Notification, 0, "Identify: blinking 10 s");
    r.by = Some("Front of house".into());
    let w = h.on_message(
        &BusMessage::Event {
            serial: S.into(),
            record: r,
        },
        t.now(),
    );
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].by.as_deref(), Some("Front of house"));
    assert_eq!(w[0].t, t.ms, "stamped when it has no time");
    // What the history published itself (it has an id) is not written again.
    let again = BusMessage::Event {
        serial: S.into(),
        record: w[0].clone(),
    };
    assert!(h.on_message(&again, t.now()).is_empty());
    // The inventory's firmware change (P2IN) is written once; the same version again
    // (a restarted inventory) is not.
    let firmware = |v: &str| {
        let mut fw = EventRecord::new(EventKind::Firmware, 0, format!("Firmware {v}"));
        fw.fw = Some(v.into());
        BusMessage::Event {
            serial: S.into(),
            record: fw,
        }
    };
    assert_eq!(h.on_message(&firmware("1.2.0"), t.now()).len(), 1);
    assert!(h.on_message(&firmware("1.2.0"), t.now()).is_empty());
    assert_eq!(h.on_message(&firmware("1.2.1"), t.now()).len(), 1);
}

#[test]
fn wp_p2ev_lost_messages_become_one_events_dropped_record_per_device() {
    let (mut h, _dir) = history("lost");
    let mut t = Time::new();
    // Nobody known yet: the count waits.
    assert!(h.lost(5, t.now()).is_empty());
    online(&mut h, &t, S);
    online(&mut h, &t, "P2EV-B");
    h.on_message(
        &BusMessage::DeviceOffline {
            serial: "P2EV-B".into(),
            handle: 1,
        },
        t.now(),
    );
    t.advance(LOST_RETRY_MS);
    let w = h.tick(t.now());
    assert_eq!(w.len(), 1, "the online device only: {w:?}");
    assert_eq!(w[0].kind, EventKind::EventsDropped);
    assert_eq!(w[0].serial, S);
    assert_eq!(w[0].count, 5);
    assert!(h.tick(t.now()).is_empty(), "written once");

    // A record the store refuses (a serial too long for a file name) is lost too.
    let long = "X".repeat(events::store::MAX_SERIAL_BYTES + 1);
    let w = h.on_message(
        &BusMessage::Unresponsive {
            serial: long,
            since: 0,
        },
        t.now(),
    );
    assert!(w.is_empty());
    t.advance(LOST_RETRY_MS);
    let w = h.lost(3, t.now());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].count, 4, "three dropped and one refused");
}

#[test]
fn wp_p2ev_list_pages_by_id_and_answers_the_newest_id() {
    let (mut h, _dir) = history("page");
    let t = Time::new();
    for _ in 0..30 {
        online(&mut h, &t, S);
    }
    let mut written = Vec::new();
    let first = h.list(S, 0, 10, &mut written);
    let ids = |v: &Value| -> Vec<u64> {
        v["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(ids(&first), (21..=30).collect::<Vec<_>>(), "the newest");
    assert_eq!(first["more"], false);
    assert_eq!(first["newest_id"], 30);
    let page = h.list(S, 5, 10, &mut written);
    assert_eq!(ids(&page), (6..=15).collect::<Vec<_>>());
    assert_eq!(page["more"], true);
    let end = h.list(S, 25, 10, &mut written);
    assert_eq!(ids(&end), (26..=30).collect::<Vec<_>>());
    assert_eq!(end["more"], false);
    assert_eq!(end["records"][0]["serial"], S, "the file format's record");

    // Forgotten by the inventory's own store on the folder (`inventory.forget`): the
    // ids start again at 1, and a client paging from 30 sees the newest id below its
    // own and reads from 0.
    let mut other =
        EventStore::open(h.store.dir().to_path_buf(), HUB_LIMITS, HUB_TOTAL_BYTES).unwrap();
    other.forget(S).unwrap();
    online(&mut h, &t, S);
    let after = h.list(S, 30, 10, &mut written);
    assert!(ids(&after).is_empty());
    assert_eq!(after["newest_id"], 1);
    assert_eq!(all(&mut h, S)[0].id, 1);
    assert!(written.is_empty(), "nothing was held back");
}

#[test]
fn wp_p2ev_export_pages_whole_records_and_reads_as_the_file() {
    let (mut h, dir) = history("export");
    let t = Time::new();
    online(&mut h, &t, S);
    for n in 0..600 {
        let mut r = EventRecord::new(
            EventKind::Notification,
            0,
            format!("{n}: {}", "x".repeat(200)),
        );
        r.by = Some("Front of house".into());
        h.on_message(
            &BusMessage::Event {
                serial: S.into(),
                record: r,
            },
            t.now(),
        );
    }
    let mut written = Vec::new();
    let mut text = String::new();
    let mut after = 0;
    let mut pages = 0;
    loop {
        let page = h.export(S, after, &mut written);
        let chunk = page["text"].as_str().unwrap();
        assert!(chunk.len() <= EXPORT_PAGE_BYTES, "{}", chunk.len());
        assert!(chunk.ends_with('\n'), "whole records");
        assert_eq!(page["newest_id"], 601);
        text.push_str(chunk);
        pages += 1;
        match page["next_after_id"].as_u64() {
            Some(n) => after = n,
            None => break,
        }
    }
    assert!(pages > 1, "{pages}");
    assert_eq!(text, dspi_session::events::EventLog::export_in(&dir, S));
    assert_eq!(text.lines().count(), 601);
}
