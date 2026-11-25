//! WP P0M: the meter ballistics in `fetch_status` and the event log the
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
    // A first read with nothing clipped: later clips are fresh.
    remote.set_status(block(n, &[], 0));
    core.fetch_status().unwrap();
    remote.set_status(block(n, &[], out1));
    assert_eq!(core.fetch_status().unwrap().clip_hold, out1);
    // The sticky flag repeats: one clip episode in the log.
    assert_eq!(core.fetch_status().unwrap().clip_hold, out1);
    std::thread::sleep(Duration::from_millis(80));
    // The hold ends; the core sends REQ_CLEAR_CLIPS, which the fake answers.
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0);
    // -- WP P0MFIX -- The fake clears the sticky bit as the firmware does
    // (WB6F): the device reads clear until the channel clips again.
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0);
    // A channel that keeps clipping sets it again: a read after the clear
    // lights the hold again (review p0m F1).
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
    // A user's clear darkens it at once; the device then reads clear.
    remote.set_status(block(n, &[], 0));
    core.clear_clips().unwrap();
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0);
}

/// Review p0m F6: flags set when the connection opens may be old; they are
/// shown (as the Mac does) but logged as sticky, not as fresh clips.
#[test]
fn wp_p0m_review_f6_flags_at_connect_are_logged_as_sticky() {
    let (core, remote) = demo();
    let n = usize::from(core.get_state().num_channels);
    remote.set_status(block(n, &[], 0b1));
    assert_eq!(core.fetch_status().unwrap().clip_hold, 0b1);
    let clip = core
        .event_log_read(String::new(), 0, 100)
        .entries
        .into_iter()
        .find(|e| e.kind == "clip")
        .unwrap();
    assert_eq!(clip.source, "sticky");
    assert_eq!(
        clip.text,
        "Clip on input 1 (flag set before this connection)"
    );
}

/// Review p0m D1: a viewer on a hub may not clear the device. The clip stays
/// shown from the sticky bits instead of going dark after the hold.
#[cfg(feature = "link")]
#[test]
fn wp_p0m_review_d1_a_viewer_on_a_hub_keeps_seeing_the_clip() {
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig, STATUS_WVALUE9};
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed("123456".into()),
        pin_role: "viewer".into(),
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    let hello = core
        .link_connect(
            hub.url(),
            "p0m".into(),
            "test".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
    assert!(hello.needs_pairing);
    core.link_pair("123456".into()).unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(8);
    while core.link_devices().is_empty() {
        assert!(std::time::Instant::now() < until, "no device");
        std::thread::sleep(Duration::from_millis(5));
    }
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.ensure_probed();

    // Channel 0 clips, sticky on the device.
    let mut status = STATUS_WVALUE9.to_vec();
    let channels = (status.len() - 7) / 2;
    status[channels * 2 + 2] |= 1;
    hub.set_status(status);
    core.set_meter_ballistics(MeterBallistics {
        hold_ms: 1000,
        decay_per_s: 0.5,
        clip_hold_ms: 300,
    });
    // Wait for a frame that carries it.
    let until = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let st = core.fetch_status().unwrap();
        if !st.stale && st.clip_flags & 1 != 0 {
            assert_eq!(st.clip_hold & 1, 1);
            break;
        }
        assert!(std::time::Instant::now() < until, "no frame with the clip");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Well past the hold (and the clear the core tries, which is denied):
    // still shown, every read.
    let start = std::time::Instant::now();
    let mut dark = 0;
    while start.elapsed() < Duration::from_millis(2500) {
        let st = core.fetch_status().unwrap();
        if st.clip_hold & 1 == 0 {
            dark += 1;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // At most the reads while the denied clear was in flight.
    assert!(dark <= 3, "dark for {dark} reads");
    assert_eq!(core.fetch_status().unwrap().clip_hold & 1, 1);
    // The viewer may not clear (class `control`), and a user's clear that is
    // denied leaves the clip shown.
    assert!(core.clear_clips().is_err(), "a viewer's clear is denied");
    assert_eq!(core.fetch_status().unwrap().clip_hold & 1, 1);
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
            // -- WP SHOWFIX -- (finding 2) plain words, not the monitor's line
            assert_eq!(n.text, "Delay of In 1 set to 1.5 ms");
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
fn wp_p0m_the_event_log_persists_in_the_data_directory() {
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
    // The watchers themselves, through the event log the core keeps.
    core.events.stat(STAT_TEMPERATURE, 7_700);
    core.events.stat(STAT_STARVATION_TOTAL, 3);
    let k = kinds(&core);
    assert!(k.contains(&"temperature".to_string()), "{k:?}");
    assert!(k.contains(&"counter".to_string()), "{k:?}");
    // An uptime far below the last one is a restart.
    core.events.uptime(1_000_000, 0);
    core.events.uptime(1, 0);
    assert_eq!(kinds(&core).last().map(String::as_str), Some("reset"));
    // A stat the event log does not follow is ignored.
    core.events.stat(STAT_CLOCK_HZ, 1);
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
    core.events.stat(STAT_TEMPERATURE, 9_000);
    assert!(core.event_log_read(String::new(), 0, 10).entries.is_empty());
    assert!(core.event_log_devices().is_empty());
    assert!(core.fetch_status().is_err());
}

/// Review p0m F5: the notification drain never waits on the event log while
/// holding `Inner`. With the log's lock held elsewhere (a slow write), a drain
/// that has records to write still lets every other call through.
#[test]
fn wp_p0m_review_f5_the_drain_does_not_hold_inner_while_logging() {
    let (core, remote) = demo();
    let at = dspi_proto::generated::OFF_DELAYS;
    let log = lock(&core.events.log);
    assert!(remote.poke(at, &2.5f32.to_le_bytes(), nfy::PARAM_SRC_UART as u8));
    let drain = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(8);
            while core.process_notifications().flags == 0 {
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    // The drain runs and blocks on the log; `Inner` stays free meanwhile.
    std::thread::sleep(Duration::from_millis(300));
    for _ in 0..20 {
        assert!(
            core.try_lock().is_some(),
            "Inner held while waiting on the log"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(log);
    drain.join().unwrap();
    assert!(
        core.event_log_read(String::new(), 0, 100)
            .entries
            .iter()
            .any(|e| e.kind == "notification")
    );
}
