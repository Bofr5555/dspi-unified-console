//! WP AUSB2: USB-direct units' lock error and the hub's own poll, on a real
//! hub with the fake devices:
//!
//! - the Installer sets each unit's lock-error action (mute or play
//!   unaligned) through the core's revision-guarded edits; the hub's own poll
//!   writes it to the unit, and a unit whose firmware refuses "play
//!   unaligned" (before the fork's WP USB3b) is said once, not retried;
//! - a unit in lock error (STATUS flags bit 6, or 0xE3 `LOCK_ERROR` /
//!   `LOCK_OK`) raises and clears the alarm "USB-direct unit X can't lock —
//!   muted / playing unaligned (cause)", which every client shows with the
//!   unit's row and the DIAG layout 2 counters;
//! - "Keep USB-direct units aligned" runs from the hub's poll, never from a
//!   client's status; a re-align it cannot write is an alarm, not a retry
//!   storm (review fixm1 F2);
//! - an alignment that changes a unit's D reads the start offsets again, so
//!   "Align again" does not follow it (F3);
//! - the start offsets written survive a hub restart (the store).
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_core::link::{AlinkSystemEdits, AlinkUnitUsbMode};
use dspi_hub::system::SystemStatus;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_proto::generated::opcodes as op;
use dspi_transport::FakeDevice;
use dspi_transport::fake::{
    FakeRemote, USB_SIM_HOST_RANGE, USB_SIM_STREAM_RESTART, UsbDirectSetup,
};
use serde_json::json;

const WAIT: Duration = Duration::from_secs(10);
const CHANGED: &str = "usb_start_offsets_changed";

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-ausb2").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// U1 ("Rear L/R") and U2 ("Height", its stream 2 bus frames later), both on
/// firmware with the lock-error setting unless `u2_usb3b` is false; D 192,
/// the units' own. The hub's own poll waits an hour: the tests run it.
fn hub_in(dir: &Path, u2_usb3b: bool, d_frames: u16) -> (Hub, FakeRemote, FakeRemote) {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.to_owned())
        }),
        initial_scan: false,
        ..HubConfig::in_process("AUSB2 hub")
    })
    .unwrap();
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));
    let mut remotes = Vec::new();
    for (serial, lag, wraps, usb3b) in [("HW-U1", 0, 3, true), ("HW-U2", 2, 9, u2_usb3b)] {
        let setup = UsbDirectSetup {
            stream_lag_frames: lag,
            origin_wraps: wraps,
            usb3b,
            ..UsbDirectSetup::default()
        };
        let dev = FakeDevice::demo_usb_direct(setup).with_serial(serial);
        remotes.push(dev.remote());
        hub.attach(Box::new(dev), 0).unwrap();
    }
    let model = json!({
        "dFrames": d_frames,
        "rooms": [{"id": "main", "name": "Main room"}],
        "units": [
            {"id": "U1", "name": "Rear L/R", "serial": "HW-U1", "room": "main", "transport": "usb_direct",
             "usbMode": "phase_lock",
             "speakers": [{"output": 0, "distanceM": 3.0}, {"output": 1, "distanceM": 3.0}]},
            {"id": "U2", "name": "Height", "serial": "HW-U2", "room": "main", "transport": "usb_direct",
             "usbMode": "phase_lock",
             "speakers": [{"output": 0, "distanceM": 3.0}]}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let u2 = remotes.pop().unwrap();
    (hub, remotes.pop().unwrap(), u2)
}

fn hub(u2_usb3b: bool) -> (Hub, FakeRemote, FakeRemote) {
    hub_in(&store_dir(), u2_usb3b, 192)
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-ausb2".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core
}

fn fresh_view(core: &DspiCore) -> dspi_core::link::AlinkSystemView {
    let start = Instant::now();
    loop {
        let v = core.alink_system_view().unwrap();
        if !v.stale {
            return v;
        }
        assert!(start.elapsed() < WAIT, "always stale");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn codes(status: &SystemStatus) -> Vec<(String, String)> {
    status
        .alignment
        .issues
        .iter()
        .map(|i| (i.subject.clone().unwrap_or_default(), i.code.clone()))
        .collect()
}

fn on_lock_error(status: &SystemStatus, serial: &str) -> String {
    status.observed[serial]
        .usb
        .as_ref()
        .unwrap()
        .on_lock_error
        .clone()
}

fn lock_edit(unit: &str, action: &str) -> AlinkSystemEdits {
    AlinkSystemEdits {
        usb_modes: vec![AlinkUnitUsbMode {
            unit_id: unit.into(),
            mode: Some("phase_lock".into()),
            on_lock_error: Some(action.into()),
        }],
        ..AlinkSystemEdits::default()
    }
}

#[test]
fn wp_ausb2_the_lock_error_action_is_set_written_and_refused_once_by_older_firmware() {
    let (hub, _u1, _u2) = hub(false);
    let h = hub.handle();
    let core = connect(&hub);
    let v = fresh_view(&core);
    let row = |v: &dspi_core::link::AlinkSystemView, id: &str| {
        v.units.iter().find(|u| u.id == id).unwrap().clone()
    };
    assert_eq!(row(&v, "U1").usb_on_lock_error_wanted, "mute");
    assert_eq!(row(&v, "U1").usb_on_lock_error.as_deref(), Some("mute"));
    // Installer: revision-guarded, only known words, only USB-direct units.
    let rev = core
        .alink_system_save(v.revision, lock_edit("U1", "play_unaligned"))
        .unwrap();
    assert!(matches!(
        core.alink_system_save(v.revision, lock_edit("U1", "mute")),
        Err(dspi_core::DspiError::Link { ref code, .. }) if code == "conflict"
    ));
    assert!(matches!(
        core.alink_system_save(rev, lock_edit("U1", "loud")),
        Err(dspi_core::DspiError::InvalidArgument { .. })
    ));
    let rev = core
        .alink_system_save(rev, lock_edit("U2", "play_unaligned"))
        .unwrap();
    let (m, _) = h.system_model();
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(json["units"][0]["onLockError"], "play_unaligned");
    // A client's status writes nothing.
    assert_eq!(on_lock_error(&h.system_status(), "HW-U1"), "mute");
    // The hub's own poll writes it; the older unit refuses it.
    h.system_poll_now();
    let s = h.system_status();
    assert_eq!(on_lock_error(&s, "HW-U1"), "play_unaligned");
    assert_eq!(on_lock_error(&s, "HW-U2"), "mute");
    assert!(
        codes(&s).contains(&("U2".into(), "usb_lock_error_unsupported".into())),
        "{:?}",
        codes(&s)
    );
    let u2 = s
        .alignment
        .issues
        .iter()
        .find(|i| i.code == "usb_lock_error_unsupported");
    assert!(
        u2.unwrap()
            .message
            .starts_with("Height: its firmware cannot play unaligned"),
        "{u2:?}"
    );
    // Said once, not tried again: an alignment has nothing to write for it.
    let r = h.system_align(true);
    assert!(
        !r.writes.iter().any(|w| w.what == "on_lock_error"),
        "{:?}",
        r.writes
    );
    let v = fresh_view(&core);
    assert_eq!(
        row(&v, "U1").usb_on_lock_error.as_deref(),
        Some("play_unaligned")
    );
    assert!(row(&v, "U2").usb_play_unaligned_unsupported);
    assert_eq!(row(&v, "U2").usb_on_lock_error_wanted, "play_unaligned");
    // Back to mute (the default: left out of the model) is written too.
    core.alink_system_save(rev, lock_edit("U1", "mute"))
        .unwrap();
    let r = h.system_align(true);
    let w = r.writes.iter().find(|w| w.what == "on_lock_error").unwrap();
    assert_eq!((w.element.as_str(), w.result.as_str()), ("U1", "ok"));
    assert_eq!(on_lock_error(&h.system_status(), "HW-U1"), "mute");
    let json = serde_json::to_value(&h.system_model().0).unwrap();
    assert!(json["units"][0].get("onLockError").is_none(), "{json}");
    hub.stop();
}

fn lock_alarms(h: &dspi_hub::HubHandle) -> Vec<dspi_hub::system::alarms::Alarm> {
    h.system_alarms()
        .into_iter()
        .filter(|a| {
            a.code == "usb_lock_error" && a.state != dspi_hub::system::alarms::AlarmState::Cleared
        })
        .collect()
}

#[test]
fn wp_ausb2_a_lock_error_raises_the_alarm_from_a_read_and_from_its_events() {
    let (hub, u1, _u2) = hub(true);
    let h = hub.handle();
    let core = connect(&hub);
    // From the unit's event alone (no read): raised, then cleared.
    assert!(u1.usb_direct_sim(USB_SIM_HOST_RANGE));
    let start = Instant::now();
    while lock_alarms(&h).is_empty() {
        assert!(start.elapsed() < WAIT, "no alarm from the 0xE3");
        std::thread::sleep(Duration::from_millis(20));
    }
    let a = &lock_alarms(&h)[0];
    assert_eq!(
        (a.subject.as_str(), a.severity.as_str(), a.message.as_str()),
        (
            "U1",
            "critical",
            "USB-direct unit Rear L/R can't lock \u{2014} muted (the host's rate is out of range)"
        )
    );
    assert!(u1.usb_direct_sim(USB_SIM_HOST_RANGE));
    let start = Instant::now();
    while !lock_alarms(&h).is_empty() {
        assert!(start.elapsed() < WAIT, "not cleared by LOCK_OK");
        std::thread::sleep(Duration::from_millis(20));
    }
    // From a read, with "play unaligned": a warning that says so; every client shows it.
    let v = fresh_view(&core);
    core.alink_system_save(v.revision, lock_edit("U1", "play_unaligned"))
        .unwrap();
    h.system_poll_now();
    assert!(u1.usb_direct_sim(USB_SIM_HOST_RANGE));
    let s = h.system_poll_now();
    let a = lock_alarms(&h);
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0].severity, "warning");
    assert!(
        a[0].message.contains("playing unaligned"),
        "{}",
        a[0].message
    );
    assert!(
        codes(&s).contains(&("U1".into(), "usb_lock_error".into())),
        "{:?}",
        codes(&s)
    );
    let v = fresh_view(&core);
    let row = v.units.iter().find(|u| u.id == "U1").unwrap();
    assert!(row.usb_lock_error, "{row:?}");
    assert_eq!(row.state_word, "Can't lock (USB): playing unaligned");
    assert_eq!(row.usb_lock_error_cause, "the host's rate is out of range");
    assert!(row.usb_stats.contains("lock errors 2"), "{}", row.usb_stats);
    assert!(
        v.alarms
            .iter()
            .any(|a| a.message.starts_with("USB-direct unit Rear L/R can't lock")),
        "{:?}",
        v.alarms
    );
    assert_eq!(v.health, "problem");
    hub.stop();
}

/// Restart U1's stream until its start offset moved.
fn restart_u1(h: &dspi_hub::HubHandle, remote: &FakeRemote) -> SystemStatus {
    let offsets = |s: &SystemStatus| {
        s.alignment
            .units
            .iter()
            .map(|p| (p.unit.clone(), p.start_offset_samples))
            .collect::<Vec<_>>()
    };
    let before = offsets(&h.system_status());
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(5));
        assert!(remote.usb_direct_sim(USB_SIM_STREAM_RESTART));
        std::thread::sleep(Duration::from_millis(3));
        let s = h.system_status();
        if offsets(&s) != before {
            return s;
        }
    }
    panic!("U1's restart never moved the offsets");
}

fn has_changed(s: &SystemStatus) -> bool {
    s.alignment.issues.iter().any(|i| i.code == CHANGED)
}

#[test]
fn wp_ausb2_keep_aligned_runs_from_the_hubs_poll_and_a_failure_is_an_alarm_once() {
    let (hub, u1, u2) = hub(true);
    let h = hub.handle();
    let mut m = h.system_model().0;
    m.keep_usb_aligned = true;
    h.system_set_model(m).unwrap();
    h.system_align(true);
    // A client's status does not write: the issue stands until the poll.
    let s = restart_u1(&h, &u1);
    assert!(has_changed(&s));
    assert!(has_changed(&h.system_status()), "still: nothing written");
    let s = h.system_poll_now();
    assert!(!has_changed(&s), "{:?}", codes(&s));
    assert_eq!(
        s.alarms
            .iter()
            .filter(|a| a.code == "usb_realigned")
            .count(),
        1
    );
    // The units refuse their delays now (whichever moved; read too, as a
    // delay cut to the longest may already be there): the poll records it
    // once, and does not retry.
    for u in [&u1, &u2] {
        u.stall(op::REQ_SET_OUTPUT_DELAY);
        u.stall(op::REQ_GET_OUTPUT_DELAY);
    }
    restart_u1(&h, &u1);
    let s = h.system_poll_now();
    assert!(has_changed(&s), "still moved");
    let failed: Vec<_> = s
        .alarms
        .iter()
        .filter(|a| a.code == "usb_realign_failed")
        .collect();
    assert_eq!(failed.len(), 1, "{:?}", s.alarms);
    assert!(
        failed[0].message.starts_with(
            "Keep USB-direct units aligned: the output delays could not be written on "
        ) && failed[0].message.contains("out_delay:0 refused"),
        "{}",
        failed[0].message
    );
    assert_eq!(failed[0].count, 1);
    for _ in 0..3 {
        h.system_poll_now();
    }
    let a = h.system_alarms();
    let failed = a.iter().find(|a| a.code == "usb_realign_failed").unwrap();
    assert_eq!(failed.count, 1, "held off: not tried again");
    assert_eq!(
        a.iter().filter(|a| a.code == "usb_realigned").count(),
        1,
        "no record of a re-align that did not happen"
    );
    hub.stop();
}

#[test]
fn wp_ausb2_an_align_that_changes_d_reads_the_offsets_again() {
    // D 200: both units re-anchor when it is written.
    let (hub, _u1, _u2) = hub_in(&store_dir(), true, 200);
    let h = hub.handle();
    let r = h.system_align(true);
    let usb: Vec<_> = r
        .writes
        .iter()
        .filter(|w| w.what == "usb_direct")
        .map(|w| w.result.as_str())
        .collect();
    assert_eq!(usb, ["ok", "ok"], "{:?}", r.writes);
    // The report's alignment is the one read after the re-anchor ...
    let after: Vec<_> = r
        .alignment
        .units
        .iter()
        .map(|p| (p.unit.clone(), p.start_offset_samples))
        .collect();
    let s = h.system_status();
    let now: Vec<_> = s
        .alignment
        .units
        .iter()
        .map(|p| (p.unit.clone(), p.start_offset_samples))
        .collect();
    assert_eq!(after, now);
    // ... so "Align again" does not follow the Align.
    assert!(!has_changed(&s), "{:?}", codes(&s));
    hub.stop();
}

#[test]
fn wp_ausb2_the_offsets_written_survive_a_hub_restart() {
    let dir = store_dir();
    let (hub, u1, _u2) = hub_in(&dir, true, 192);
    hub.handle().system_align(true);
    hub.stop();
    drop(hub);
    let text = fs::read_to_string(dir.join("hub-store.json")).unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(file["system_usb_offsets"].is_object(), "{text}");
    drop(u1);
    // A new run on the same store: no alignment written in it, yet a moved
    // offset is judged against what the last run wrote.
    let (hub, u1, _u2) = hub_in(&dir, true, 192);
    let h = hub.handle();
    assert!(
        !has_changed(&h.system_status()),
        "same placement: nothing moved"
    );
    let s = restart_u1(&h, &u1);
    assert!(has_changed(&s), "{:?}", codes(&s));
    hub.stop();
}

/// Review fixm1 F5: the Align's own report no longer counts the "Align again"
/// issue it has just cleared (the web's Alignment panel counts these).
#[test]
fn wp_ausb2_the_align_report_does_not_count_the_issue_it_fixed() {
    let (hub, u1, _u2) = hub(true);
    let h = hub.handle();
    h.system_align(true);
    let s = restart_u1(&h, &u1);
    assert!(has_changed(&s));
    let r = h.system_align(true);
    assert!(
        !r.alignment.issues.iter().any(|i| i.code == CHANGED),
        "{:?}",
        r.alignment.issues
    );
    assert!(!has_changed(&h.system_status()));
    hub.stop();
}
