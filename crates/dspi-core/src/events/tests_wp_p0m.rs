//! WP P0M: the meter ballistics in `fetch_status` and the event history the
//! core writes, against the demo device.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_proto::generated::notify as nfy;
use dspi_transport::{FakeDevice, FakeRemote};

use super::*;

fn demo() -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "P0M".into(), 0).unwrap();
    core.process_notifications();
    (core, remote)
}

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p0m-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A meter block for `n` channels: `peaks` (linear) then cpu, clip flags,
/// active inputs.
fn block(n: usize, peaks: &[f32], clips: u32) -> Vec<u8> {
    let mut b = Vec::new();
    for i in 0..n {
        let p = peaks.get(i).copied().unwrap_or(0.0);
        b.extend_from_slice(&((p * 32767.0) as u16).to_le_bytes());
    }
    b.extend_from_slice(&[10, 20]);
    b.extend_from_slice(&clips.to_le_bytes());
    b.push(2);
    b
}

fn kinds(core: &DspiCore) -> Vec<String> {
    core.event_log_read(String::new(), 0, EVENT_READ_MAX)
        .entries
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

#[test]
fn wp_p0m_fetch_status_carries_the_ballistics() {
    let (core, remote) = demo();
    let n = usize::from(core.get_state().num_channels);
    remote.set_status(block(n, &[0.5, 0.25], 0));
    let s = core.fetch_status().unwrap();
    assert_eq!(s.peak_hold.len(), 17);
    assert_eq!(s.peak_max.len(), 17);
    assert!((s.peak_hold[0] - 0.5).abs() < 1e-3);
    assert!((s.peak_max[1] - 0.25).abs() < 1e-3);
    // Quieter: the hold and the maximum stay.
    remote.set_status(block(n, &[0.1, 0.0], 0));
    let s = core.fetch_status().unwrap();
    assert!((s.peaks[0] - 0.1).abs() < 1e-3);
    assert!((s.peak_hold[0] - 0.5).abs() < 1e-3);
    assert!((s.peak_max[0] - 0.5).abs() < 1e-3);
    core.reset_meter_max();
    let s = core.fetch_status().unwrap();
    assert!((s.peak_max[0] - 0.1).abs() < 1e-3);
    assert_eq!(s.peak_max[1], 0.0);
}

#[test]
fn wp_p0m_the_clip_hold_ends_and_clears_the_device() {
    let (core, remote) = demo();
    let n = usize::from(core.get_state().num_channels);
    core.set_meter_ballistics(MeterBallistics {
        hold_ms: 1000,
        decay_per_s: 0.5,
        clip_hold_ms: 50,
    });
    assert_eq!(core.meter_ballistics().clip_hold_ms, 50);
    let inputs = core.get_state().num_input_channels;
    let out1 = 1u32 << inputs;
    remote.set_status(block(n, &[], out1));
    assert_eq!(core.fetch_status().unwrap().clip_hold, out1);
    // The sticky flag repeats: one clip episode in the history.
    assert_eq!(core.fetch_status().unwrap().clip_hold, out1);
    std::thread::sleep(Duration::from_millis(80));
    // The hold ends; the core sends REQ_CLEAR_CLIPS. The fake keeps its
    // block, so the bit still reads set: not lit again until it reads clear.
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0);
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0);
    remote.set_status(block(n, &[], 0));
    core.fetch_status().unwrap();
    remote.set_status(block(n, &[], out1));
    assert_eq!(core.fetch_status().unwrap().clip_hold, out1);
    let clips: Vec<EventEntry> = core
        .event_log_read(String::new(), 0, 100)
        .entries
        .into_iter()
        .filter(|e| e.kind == "clip")
        .collect();
    assert_eq!(clips.len(), 2, "{clips:?}");
    assert_eq!(clips[0].channel, Some(inputs));
    assert_eq!(clips[0].text, "Clip on output 1");
    assert_eq!(clips[0].source, "meter");
    // A user's clear darkens it at once.
    core.clear_clips().unwrap();
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0);
}

#[test]
fn wp_p0m_connects_disconnects_and_notifications_are_recorded() {
    let (core, remote) = demo();
    // Another client's change, from UART.
    let at = dspi_proto::generated::OFF_DELAYS;
    assert!(remote.poke(at, &1.5f32.to_le_bytes(), nfy::PARAM_SRC_UART as u8));
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        core.process_notifications();
        let page = core.event_log_read(String::new(), 0, 100);
        if page.entries.iter().any(|e| e.kind == "notification") {
            let n = page
                .entries
                .iter()
                .find(|e| e.kind == "notification")
                .unwrap();
            assert_eq!(n.source, "UART");
            assert!(!n.own);
            assert!(n.text.contains("delays"), "{}", n.text);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no notification record"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let first = core.event_log_read(String::new(), 0, 100).entries;
    assert_eq!(first[0].kind, "connected");
    assert_eq!(first[0].serial, "P0M");
    assert_eq!(first[0].source, "fake");
    assert!(first[0].fw.is_some());
    core.disconnect();
    // The open device is gone: read it by serial.
    let all = core.event_log_read("P0M".into(), 0, 100).entries;
    assert_eq!(all.last().unwrap().kind, "disconnected");
    assert_eq!(all.last().unwrap().source, "closed");
    assert_eq!(core.event_log_devices(), ["P0M"]);
}

#[test]
fn wp_p0m_the_history_persists_in_the_data_directory() {
    let d = dir("persist");
    let core = DspiCore::new();
    core.event_log_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    core.attach_probed(Box::new(FakeDevice::demo()), "PERSIST".into(), 0)
        .unwrap();
    core.load_preset(0).unwrap();
    core.disconnect();
    let file = d.join("events-s-PERSIST.jsonl");
    let text = std::fs::read_to_string(&file).unwrap();
    let kinds: Vec<String> = text
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(kinds.starts_with(&["connected".to_string()]), "{kinds:?}");
    assert!(kinds.contains(&"recall".to_string()), "{kinds:?}");
    assert_eq!(kinds.last().unwrap(), "disconnected");

    // A new core (the next run) reads it, carries the ids on, and exports it.
    let core = DspiCore::new();
    core.event_log_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    let before = core.event_log_read("PERSIST".into(), 0, 1000);
    assert_eq!(before.entries.len(), kinds.len());
    core.attach_probed(Box::new(FakeDevice::demo()), "PERSIST".into(), 0)
        .unwrap();
    let after = core.event_log_read(String::new(), before.last_id, 1000);
    assert_eq!(after.entries[0].id, before.last_id + 1);
    assert_eq!(after.entries[0].kind, "connected");
    // Same firmware: no firmware record.
    assert!(after.entries.iter().all(|e| e.kind != "firmware"));
    let export = core.event_log_export(String::new());
    assert_eq!(export.lines().count(), kinds.len() + 1);
    assert_eq!(core.event_log_export("NOBODY".into()), "");
}

#[test]
fn wp_p0m_a_recall_records_what_changed() {
    let (core, remote) = demo();
    // Make the live state differ from slot 0 so the recall changes something.
    core.set_master_volume(-20.0).ok();
    let _ = remote;
    core.load_preset(0).unwrap();
    let recall = core
        .event_log_read(String::new(), 0, 100)
        .entries
        .into_iter()
        .find(|e| e.kind == "recall")
        .unwrap();
    assert_eq!(recall.channel, Some(0));
    assert!(
        recall.text.starts_with("Recalled preset 1: "),
        "{}",
        recall.text
    );
    assert_eq!(recall.count as usize, recall.lines.len().max(1));
}

#[test]
fn wp_p0m_polled_values_feed_the_watchers() {
    let (core, _remote) = demo();
    // The fake reads 35.00 °C and quiet counters: baselines, no records.
    assert_eq!(core.fetch_stat(STAT_TEMPERATURE).unwrap(), 3500);
    core.fetch_stat(STAT_STARVATION_TOTAL).unwrap();
    core.fetch_stat(STAT_STARVATION_TOTAL).unwrap();
    let _ = core.fetch_ring_health();
    let _ = core.fetch_core_vreg();
    let _ = core.fetch_core_vreg();
    let k = kinds(&core);
    assert!(
        k.iter()
            .all(|k| !matches!(k.as_str(), "temperature" | "counter" | "reset")),
        "{k:?}"
    );
    // The watchers themselves, through the history the core keeps.
    core.history.stat(STAT_TEMPERATURE, 7_700);
    core.history.stat(STAT_STARVATION_TOTAL, 3);
    let k = kinds(&core);
    assert!(k.contains(&"temperature".to_string()), "{k:?}");
    assert!(k.contains(&"counter".to_string()), "{k:?}");
    // An uptime far below the last one is a restart.
    core.history.uptime(1_000_000, 0);
    core.history.uptime(1, 0);
    assert_eq!(kinds(&core).last().map(String::as_str), Some("reset"));
    // A stat the history does not follow is ignored.
    core.history.stat(STAT_CLOCK_HZ, 1);
}

#[test]
fn wp_p0m_a_sink_sees_every_record() {
    struct Sink(Mutex<Vec<String>>);
    impl EventSink for Sink {
        fn record(&self, r: &EventRecord) {
            self.0.lock().unwrap().push(r.kind.as_str().to_string());
        }
    }
    let sink = Arc::new(Sink(Mutex::new(Vec::new())));
    let core = DspiCore::new();
    core.set_event_sink(Some(sink.clone()));
    core.attach_probed(Box::new(FakeDevice::demo()), "SINK".into(), 0)
        .unwrap();
    core.disconnect();
    assert_eq!(*sink.0.lock().unwrap(), ["connected", "disconnected"]);
}

#[test]
fn wp_p0m_a_directory_that_cannot_be_made_is_an_io_error() {
    let d = dir("blocked");
    std::fs::create_dir_all(&d).unwrap();
    let file = d.join("file");
    std::fs::write(&file, "x").unwrap();
    let core = DspiCore::new();
    let e = core
        .event_log_set_dir(file.join("sub").to_string_lossy().into_owned())
        .unwrap_err();
    assert!(matches!(e, DspiError::Io { .. }), "{e:?}");
    // Back to memory only.
    core.event_log_set_dir(String::new()).unwrap();
}

#[test]
fn wp_p0m_without_a_device_nothing_is_recorded() {
    let core = DspiCore::new();
    core.history.stat(STAT_TEMPERATURE, 9_000);
    assert!(core.event_log_read(String::new(), 0, 10).entries.is_empty());
    assert!(core.event_log_devices().is_empty());
    assert!(core.fetch_status().is_err());
}
