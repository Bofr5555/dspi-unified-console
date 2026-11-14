//! WP AUSB2: scene timing with USB-direct units and units without the audio
//! link (review fixm1 F1 and F4), on the reviewer's probe
//! (`review_fixm1_probe.rs`): a sender, A on the audio link, U1 USB-direct
//! (read first), and K1, K2 without the alink, which another client writes
//! every 100 ms (a slider drag), so their reads stall and retry.
//!
//! - F4: a unit last seen without the alink is not read for a frame: a
//!   scene right after a write to it no longer costs 1.2 s per such unit;
//! - F1: when the reads after U1's take longer than a bus frame reaches
//!   (1.92 s), U1 is read again and its op lands at a frame still ahead,
//!   never at the next occurrence of one already past (up to 2 s late).
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_hub::system::SceneReport;
use dspi_hub::{AccessMode, Hub, HubConfig, HubHandle, NetworkConfig, Role};
use dspi_proto::ext::alink::{AlinkRole, ApplyStatus, EXT_APPLY_AT_FRAME};
use dspi_proto::generated::opcodes as op;
use dspi_transport::fake::{AlinkSetup, UsbDirectSetup};
use dspi_transport::{FakeDevice, Transport};
use serde_json::json;

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-ausb2-scene").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn make_hub() -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("AUSB2 scene hub")
    })
    .unwrap();
    // The tests decide when the system is read.
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));
    for (serial, role) in [("HW-TX", AlinkRole::Sender), ("HW-A", AlinkRole::Receiver)] {
        let setup = AlinkSetup {
            role,
            ..AlinkSetup::default()
        };
        hub.attach(
            Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    for k in ["HW-K1", "HW-K2"] {
        hub.attach(Box::new(FakeDevice::demo().with_serial(k)), 0)
            .unwrap();
    }
    let setup = UsbDirectSetup {
        origin_wraps: 3,
        ..UsbDirectSetup::default()
    };
    hub.attach(
        Box::new(FakeDevice::demo_usb_direct(setup).with_serial("HW-U1")),
        0,
    )
    .unwrap();
    let model = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "HW-TX"}],
        "units": [
            {"id": "U1", "name": "Rear L/R", "serial": "HW-U1", "room": "main", "transport": "usb_direct",
             "usbMode": "phase_lock", "speakers": [{"output": 0, "distanceM": 3.0}]},
            {"id": "A", "name": "Main left", "serial": "HW-A", "room": "main", "upstream": "tx"},
            {"id": "K1", "name": "K340 one", "serial": "HW-K1", "room": "main", "upstream": "tx"},
            {"id": "K2", "name": "K340 two", "serial": "HW-K2", "room": "main", "upstream": "tx"}
        ],
        "scenes": [
            {"id": "evening", "name": "Evening", "actions": [{"target": "all", "op": "master_volume", "value": -1200}]}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    hub
}

/// Another client writing K1 and K2 every 100 ms until stopped.
fn drag(h: &HubHandle) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let mut k1 = h.open_local("HW-K1").unwrap();
    let mut k2 = h.open_local("HW-K2").unwrap();
    let t = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut v = 0u8;
            while !stop.load(Ordering::SeqCst) {
                v = v.wrapping_add(1);
                let _ = k1.control_out(op::REQ_SET_USER_MUTE, 0, &[v & 1]);
                let _ = k2.control_out(op::REQ_SET_USER_MUTE, 0, &[v & 1]);
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    std::thread::sleep(Duration::from_millis(300));
    (stop, t)
}

/// U1's op ran (applied, not late) within `within` of the apply returning:
/// not wrapped into the next occurrence of a past frame.
fn u1_applied_soon(h: &HubHandle, within: Duration) {
    let mut u1 = h.open_local("HW-U1").unwrap();
    let start = Instant::now();
    loop {
        let b = u1
            .control_in(op::REQ_EXTENSION, EXT_APPLY_AT_FRAME, 16)
            .unwrap();
        let s = ApplyStatus::decode(&b).unwrap();
        if s.pending == 0 {
            assert_eq!(s.last_result.as_str(), "applied", "{s:?}");
            return;
        }
        assert!(
            start.elapsed() < within,
            "U1's op still pending after {within:?}: a frame already past"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn write<'a>(r: &'a SceneReport, serial: &str) -> &'a dspi_hub::system::WriteResult {
    r.writes.iter().find(|w| w.serial == serial).unwrap()
}

#[test]
fn wp_ausb2_f4_a_unit_last_seen_without_the_alink_costs_no_frame_read() {
    let hub = make_hub();
    let h = hub.handle();
    let _ = h.system_status(); // told K1's and K2's firmware
    let (stop, t) = drag(&h);
    let start = Instant::now();
    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    let took = start.elapsed();
    stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
    assert!(took < Duration::from_millis(600), "apply took {took:?}");
    for k in ["HW-K1", "HW-K2"] {
        let w = write(&r, k);
        assert_eq!(
            (w.result.as_str(), w.reason.as_deref()),
            ("applied_now", Some("no audio link (firmware)"))
        );
    }
    assert_eq!(write(&r, "HW-U1").result, "queued");
    u1_applied_soon(&h, Duration::from_millis(800));
    hub.stop();
}

#[test]
fn wp_ausb2_f1_slow_reads_after_the_usb_unit_read_it_again_never_a_past_frame() {
    let hub = make_hub();
    let h = hub.handle();
    // No status first: K1 and K2 are not known yet, so each read stalls and
    // retries (the probe's 2.4 s), well past the 1.92 s a bus frame reaches.
    let (stop, t) = drag(&h);
    let start = Instant::now();
    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    let took = start.elapsed();
    stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
    assert!(
        took > Duration::from_millis(1920),
        "reads were quick: {took:?}"
    );
    let u1 = write(&r, "HW-U1");
    let lead = r.plan.usb_lead_samples.unwrap();
    assert!(
        lead < 48 * 1000,
        "lead counted from the second read: {lead} samples, {u1:?}"
    );
    assert!(!r.plan.usb_lead_capped);
    assert_eq!(u1.result, "queued", "{u1:?}");
    u1_applied_soon(&h, Duration::from_millis(1000));
    hub.stop();
}

// -- fixup ausb2 (review ausb2 F4, probe B') --
/// Reads of about 1.2-1.8 s (one unit without the alink, unknown yet, under
/// a slider drag) at 48 kHz: under the 1.92 s reach, so the USB-direct op is
/// not said to be cut ("lead over 1.92 s") and the lead is not the cap.
#[test]
fn wp_ausb2_fixup_a_lead_under_the_reach_is_not_said_to_be_cut() {
    let hub = make_hub();
    let h = hub.handle();
    let stop = Arc::new(AtomicBool::new(false));
    let mut k1 = h.open_local("HW-K1").unwrap();
    let t = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut v = 0u8;
            while !stop.load(Ordering::SeqCst) {
                v = v.wrapping_add(1);
                let _ = k1.control_out(op::REQ_SET_USER_MUTE, 0, &[v & 1]);
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    std::thread::sleep(Duration::from_millis(300));
    // Only K1 is slow: K2 is not written, so its refusals come at once.
    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
    assert!(!r.plan.usb_lead_capped, "{:?}", r.plan);
    let lead = r.plan.usb_lead_samples.unwrap();
    assert!(lead < 1920 * 48, "{lead} samples is the cap");
    let u1 = write(&r, "HW-U1");
    assert_eq!((u1.result.as_str(), u1.reason.as_deref()), ("queued", None));
    u1_applied_soon(&h, Duration::from_millis(2500));
    hub.stop();
}
