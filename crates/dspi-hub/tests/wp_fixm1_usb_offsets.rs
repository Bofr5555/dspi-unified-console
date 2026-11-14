//! WP FIXM1 (review ausb1 F3, the coordinator's decision): the USB-direct
//! streams' start offsets move with every stream reopen by the host and every
//! anchor, while the output delays the hub wrote stay. The hub watches them:
//!
//! - after an alignment, a moved start offset raises the issue
//!   `usb_start_offsets_changed` ("USB-direct start offsets changed — Align
//!   again"), which every client shows; the one-click Align clears it;
//! - with the Installer option "Keep USB-direct units aligned" (off by
//!   default, set through the core's edits) the hub writes those units'
//!   output delays again by itself and records it (alarm `usb_realigned`);
//! - a unit whose stream runs at another rate gets an issue, not a silent
//!   drop (`usb_rate_differs`, `align.rs` unit tests).
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_core::link::AlinkSystemEdits;
use dspi_hub::system::SystemStatus;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_transport::FakeDevice;
use dspi_transport::fake::{FakeRemote, USB_SIM_STREAM_RESTART, UsbDirectSetup};
use serde_json::json;

const WAIT: Duration = Duration::from_secs(10);
const CHANGED: &str = "usb_start_offsets_changed";

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-fixm1").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// Two USB-direct units, U1 ("Rear L/R") and U2 ("Height", its stream 2 bus
/// frames after U1's), at the units' own D (192), so an alignment writes only
/// the delays and nothing re-anchors. U1's remote, to restart its stream.
fn hub() -> (Hub, FakeRemote) {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("FIXM1 hub")
    })
    .unwrap();
    let mut u1_remote = None;
    for (serial, lag, wraps) in [("HW-U1", 0, 3), ("HW-U2", 2, 9)] {
        let setup = UsbDirectSetup {
            stream_lag_frames: lag,
            origin_wraps: wraps,
            ..UsbDirectSetup::default()
        };
        let dev = FakeDevice::demo_usb_direct(setup).with_serial(serial);
        if serial == "HW-U1" {
            u1_remote = Some(dev.remote());
        }
        hub.attach(Box::new(dev), 0).unwrap();
    }
    let model = json!({
        "dFrames": 192,
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
    (hub, u1_remote.unwrap())
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-fixm1".into(),
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

fn codes(status: &SystemStatus) -> Vec<String> {
    status
        .alignment
        .issues
        .iter()
        .map(|i| i.code.clone())
        .collect()
}

fn offsets(status: &SystemStatus) -> Vec<(String, Option<i64>)> {
    status
        .alignment
        .units
        .iter()
        .map(|p| (p.unit.clone(), p.start_offset_samples))
        .collect()
}

/// Restart U1's stream (the host reopened it) until its offsets moved: one
/// restart moves them unless it lands a whole 2048 frames on.
fn restart_u1(hub: &Hub, remote: &FakeRemote) -> SystemStatus {
    let before = offsets(&hub.handle().system_status());
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(5));
        assert!(remote.usb_direct_sim(USB_SIM_STREAM_RESTART));
        // Its first packet: the next bus frame (1 ms).
        std::thread::sleep(Duration::from_millis(3));
        let status = hub.handle().system_status();
        if offsets(&status) != before {
            return status;
        }
    }
    panic!("U1's restart never moved the offsets: {before:?}");
}

#[test]
fn wp_fixm1_a_moved_start_offset_asks_for_an_align_and_the_align_clears_it() {
    let (hub, u1) = hub();
    let h = hub.handle();
    // Nothing is judged before the hub wrote an alignment.
    let s = h.system_status();
    assert!(!codes(&s).contains(&CHANGED.into()), "{:?}", codes(&s));
    let r = h.system_align(true);
    let usb: Vec<_> = r
        .writes
        .iter()
        .filter(|w| w.what == "usb_direct")
        .map(|w| w.result.as_str())
        .collect();
    assert_eq!(usb, ["unchanged", "unchanged"], "nothing re-anchors");
    let s = h.system_status();
    assert!(!codes(&s).contains(&CHANGED.into()), "{:?}", codes(&s));
    // The host reopens U1's stream: the delays written no longer match.
    let s = restart_u1(&hub, &u1);
    let issue = s
        .alignment
        .issues
        .iter()
        .find(|i| i.code == CHANGED)
        .unwrap_or_else(|| panic!("{:?}", s.alignment.issues));
    assert!(
        issue
            .message
            .starts_with("USB-direct start offsets changed \u{2014} Align again ("),
        "{}",
        issue.message
    );
    assert!(issue.message.contains("Rear L/R"), "{}", issue.message);
    // Off by default: nothing written by itself, nothing recorded.
    assert!(!s.alarms.iter().any(|a| a.code == "usb_realigned"));
    // Every client sees it, and the one-click Align (Installer) clears it.
    let core = connect(&hub);
    let v = fresh_view(&core);
    assert!(!v.keep_usb_aligned);
    assert!(v.align_needed);
    assert!(
        v.issues
            .iter()
            .any(|i| i.starts_with("USB-direct start offsets changed \u{2014} Align again")),
        "{:?}",
        v.issues
    );
    let a = core.alink_system_align().unwrap();
    assert!(a.applied, "{a:?}");
    let s = h.system_status();
    assert!(!codes(&s).contains(&CHANGED.into()), "{:?}", codes(&s));
    hub.stop();
}

#[test]
fn wp_fixm1_keep_usb_direct_units_aligned_writes_the_delays_again_and_records_it() {
    let (hub, u1) = hub();
    let h = hub.handle();
    let core = connect(&hub);
    let v = fresh_view(&core);
    assert!(!v.keep_usb_aligned, "off by default");
    let edits = AlinkSystemEdits {
        keep_usb_aligned: Some(true),
        ..AlinkSystemEdits::default()
    };
    core.alink_system_save(v.revision, edits).unwrap();
    assert!(h.system_model().0.keep_usb_aligned);
    h.system_align(true);
    let s = restart_u1(&hub, &u1);
    // Written again by the hub itself: no issue, a record of it.
    assert!(!codes(&s).contains(&CHANGED.into()), "{:?}", codes(&s));
    let realigned: Vec<_> = s
        .alarms
        .iter()
        .filter(|a| a.code == "usb_realigned")
        .collect();
    assert_eq!(realigned.len(), 1, "{:?}", s.alarms);
    assert!(
        realigned[0]
            .message
            .starts_with("USB-direct start offsets changed: output delays written again on "),
        "{}",
        realigned[0].message
    );
    // The delays now carry the new offsets: an align has nothing to change.
    let again = h.system_align(false);
    let plan = |id: &str| {
        again
            .alignment
            .units
            .iter()
            .find(|p| p.unit == id)
            .unwrap()
            .clone()
    };
    let core_view = fresh_view(&core);
    assert!(core_view.keep_usb_aligned && !core_view.align_needed);
    let r = h.system_align(true);
    for p in [plan("U1"), plan("U2")] {
        for (output, _) in &p.out_delays {
            let w = r
                .writes
                .iter()
                .find(|w| w.element == p.unit && w.what == format!("out_delay:{output}"))
                .unwrap();
            assert!(
                w.result == "unchanged" || w.result == "clamped (unchanged)",
                "{} {}: {}",
                p.unit,
                w.what,
                w.result
            );
        }
    }
    // Seen again with nothing moved: no second record.
    let s = h.system_status();
    assert_eq!(
        s.alarms
            .iter()
            .filter(|a| a.code == "usb_realigned")
            .count(),
        1
    );
    // Switched off through the core: off is not written into the model.
    let v = fresh_view(&core);
    let off = AlinkSystemEdits {
        keep_usb_aligned: Some(false),
        ..AlinkSystemEdits::default()
    };
    core.alink_system_save(v.revision, off).unwrap();
    assert!(!h.system_model().0.keep_usb_aligned);
    hub.stop();
}
