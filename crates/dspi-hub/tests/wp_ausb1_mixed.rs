//! WP AUSB1: USB-direct units (`extension_channel_spec.md` 2.9) next to the
//! audio link's units and a plain unit, on a real hub with the fake devices:
//!
//! - the hub reads a USB-direct unit's `EXT_ALINK_USB`, `_STATUS` and `_DIAG`
//!   (not the alink's), and every client (the core's view) shows its state,
//!   host ppm, VCXO, phase error and counters;
//! - the alignment puts both transports on one system delay, equalises the
//!   USB-direct streams' start offsets through the outputs' delays, and writes
//!   each USB-direct unit's mode and D;
//! - a scene goes at a bus frame to the phase-locked USB-direct units, at an
//!   audio link frame to the others, at once to the plain unit, and the recall
//!   says each;
//! - the mode is set from the "Clock and audio links" edits, guarded by the
//!   revision; a unit that is not phase-locked takes scenes at once.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_core::link::{AlinkSystemEdits, AlinkUnitUsbMode};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::{AlinkSetup, UsbDirectSetup};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(10);

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-ausb1").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// Sender, fan-out, units A (direct) and B (behind the fan-out), the plain
/// unit K, and the USB-direct units U1 ("Rear L/R") and U2 ("Height"), whose
/// stream began 2 bus frames after U1's.
fn model(extra: Option<Value>) -> Value {
    let mut units = vec![
        json!({"id": "A", "name": "Main left", "serial": "HW-A", "room": "main", "upstream": "tx"}),
        json!({"id": "B", "name": "Main right", "serial": "HW-B", "room": "main", "upstream": "fan"}),
        json!({"id": "K", "name": "K340", "serial": "HW-K", "room": "main", "upstream": "tx"}),
        json!({"id": "U1", "name": "Rear L/R", "serial": "HW-U1", "room": "main", "transport": "usb_direct",
               "usbMode": "phase_lock",
               "speakers": [{"output": 0, "distanceM": 3.0}, {"output": 1, "distanceM": 3.0}]}),
        json!({"id": "U2", "name": "Height", "serial": "HW-U2", "room": "main", "transport": "usb_direct",
               "speakers": [{"output": 0, "distanceM": 3.0}]}),
    ];
    units.extend(extra);
    json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "HW-TX"}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "HW-FAN", "upstream": "tx", "output": 1}],
        "units": units,
        "scenes": [
            {"id": "evening", "name": "Evening", "actions": [{"target": "all", "op": "master_volume", "value": -1200}]},
            {"id": "rear", "name": "Rear quiet", "actions": [{"target": "unit:U1", "op": "mute"}]}
        ]
    })
}

fn hub(extra: Option<Value>) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("AUSB1 hub")
    })
    .unwrap();
    let at = |role, hops, hop_delay_frames| AlinkSetup {
        role,
        hops,
        hop_delay_frames,
        ..AlinkSetup::default()
    };
    for (serial, setup) in [
        ("HW-TX", at(AlinkRole::Sender, 0, 0)),
        ("HW-FAN", at(AlinkRole::FanOut, 0, 3)),
        ("HW-A", at(AlinkRole::Receiver, 0, 0)),
        ("HW-B", at(AlinkRole::Receiver, 1, 0)),
    ] {
        hub.attach(
            Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    hub.attach(Box::new(FakeDevice::demo().with_serial("HW-K")), 0)
        .unwrap();
    // A plain demo too, declared as USB-direct by the extra unit when given.
    hub.attach(Box::new(FakeDevice::demo().with_serial("HW-X")), 0)
        .unwrap();
    for (serial, lag, wraps) in [("HW-U1", 0, 3), ("HW-U2", 2, 9)] {
        let setup = UsbDirectSetup {
            stream_lag_frames: lag,
            origin_wraps: wraps,
            ..UsbDirectSetup::default()
        };
        hub.attach(
            Box::new(FakeDevice::demo_usb_direct(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    hub.handle()
        .system_set_model(serde_json::from_value(model(extra)).unwrap())
        .unwrap();
    hub
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-ausb1".into(),
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

fn codes(status: &dspi_hub::system::SystemStatus) -> Vec<(String, String)> {
    status
        .alignment
        .issues
        .iter()
        .map(|i| (i.subject.clone().unwrap_or_default(), i.code.clone()))
        .collect()
}

#[test]
fn wp_ausb1_usb_direct_units_are_read_and_shown() {
    let hub = hub(None);
    let status = hub.handle().system_status();
    let u1 = serde_json::to_value(&status.observed["HW-U1"]).unwrap();
    assert_eq!(u1["usbDirect"], "present", "{u1}");
    assert_eq!(u1["usb"]["state"], "phase_locked");
    assert_eq!(u1["usb"]["mode"], "phase_lock");
    assert_eq!(
        (u1["usb"]["dMin"].as_u64(), u1["usb"]["dMax"].as_u64()),
        (Some(170), Some(399))
    );
    assert!(
        u1["alink"].is_null() && u1["role"].is_null() && u1["state"].is_null(),
        "no alink read: {u1}"
    );
    assert!(u1["usb"]["diag"].is_object());
    let c = codes(&status);
    assert!(
        c.contains(&(String::new(), "mixed_time_bases".into())),
        "{c:?}"
    );
    assert!(
        c.contains(&(String::new(), "usb_start_offsets".into())),
        "{c:?}"
    );
    assert!(!c.iter().any(|(s, _)| s == "U1" || s == "U2"), "{c:?}");
    // One system delay: the alink's need (257 + the fan-out's 3) is U1's D too.
    let plan = |id: &str| {
        status
            .alignment
            .units
            .iter()
            .find(|p| p.unit == id)
            .unwrap()
            .clone()
    };
    assert_eq!(status.alignment.d_frames, 260);
    let (p1, p2) = (plan("U1"), plan("U2"));
    assert_eq!(
        (p1.transport.as_str(), p1.usb_d, p1.usb_mode.as_deref()),
        ("usb_direct", Some(260), Some("phase_lock"))
    );
    assert_eq!(
        p2.usb_mode.as_deref(),
        Some("phase_lock"),
        "the unit's own mode"
    );
    // U1's stream began 2 frames earlier: 96 samples, 2 ms more on its outputs.
    assert_eq!(
        (p1.start_offset_samples, p2.start_offset_samples),
        (Some(96), Some(0))
    );
    assert_eq!(p1.out_delays, vec![(0, 2.0), (1, 2.0)]);
    assert_eq!(p2.out_delays, vec![(0, 0.0)]);
    assert!(p1.writable && p2.writable);
    assert_eq!(plan("A").transport, "alink");

    let core = connect(&hub);
    let v = fresh_view(&core);
    let row = v.units.iter().find(|u| u.id == "U1").unwrap();
    assert_eq!(
        (
            row.transport.as_str(),
            row.role.as_str(),
            row.state.as_str(),
            row.state_word.as_str()
        ),
        (
            "usb_direct",
            "USB-direct",
            "phase_locked",
            "Phase-locked (USB)"
        )
    );
    assert_eq!(
        (row.usb_delay_samples, row.unit_usb_delay_samples),
        (Some(260), Some(192))
    );
    assert!(row.pending, "D to write");
    assert!((row.host_ppm.unwrap() + 11.5).abs() < 0.01);
    assert!(row.vcxo_ppm.is_some() && row.phase_error_samples.is_some());
    assert_eq!(
        (row.gaps, row.missed_samples, row.late_samples),
        (Some(0), Some(0), Some(0))
    );
    assert_eq!(row.start_offset_samples, Some(96));
    assert!(
        row.usb_stats.starts_with("host -11.5 ppm · VCXO "),
        "{}",
        row.usb_stats
    );
    assert!(
        row.usb_stats.contains("stream 96 samples early"),
        "{}",
        row.usb_stats
    );
    let a = v.units.iter().find(|u| u.id == "A").unwrap();
    assert_eq!(a.transport, "alink");
    assert_eq!((v.usb_unit_count, v.usb_locked, v.unit_count), (2, 2, 2));
    assert!(
        v.summary.starts_with(
            "Audio link: 2 of 2 locked, USB-direct 2 of 2 locked, 1 without the audio link (firmware)"
        ),
        "{}",
        v.summary
    );
    hub.stop();
}

#[test]
fn wp_ausb1_align_writes_the_usb_mode_d_and_the_offsets() {
    let hub = hub(None);
    let h = hub.handle();
    let r = h.system_align(true);
    let w = |e: &str, what: &str| {
        r.writes
            .iter()
            .find(|w| w.element == e && w.what == what)
            .unwrap_or_else(|| panic!("{e} {what}: {:?}", r.writes))
            .result
            .clone()
    };
    assert_eq!(w("U1", "usb_direct"), "ok");
    assert_eq!(w("U2", "usb_direct"), "ok");
    assert_eq!(w("U1", "out_delay:0"), "ok");
    assert_eq!(w("A", "phase"), "ok", "{:?} {:?}", r.writes, r.alignment);
    assert!(
        !r.writes
            .iter()
            .any(|w| w.element == "U1" && w.what == "phase")
    );
    std::thread::sleep(dspi_transport::fake::USB_DIRECT_ANCHOR + Duration::from_millis(30));
    let s = h.system_status();
    let u1 = s.observed["HW-U1"].usb.as_ref().unwrap();
    assert_eq!(
        (u1.mode.as_str(), u1.d, u1.state.as_str()),
        ("phase_lock", 260, "phase_locked")
    );
    // Again: nothing changes, nothing re-anchors.
    let again = h.system_align(true);
    let u1w = again
        .writes
        .iter()
        .find(|w| w.element == "U1" && w.what == "usb_direct")
        .unwrap();
    assert_eq!(u1w.result, "unchanged");
    let core = connect(&hub);
    fresh_view(&core);
    let a = core.alink_system_align().unwrap();
    let words: Vec<_> = a
        .writes
        .iter()
        .map(|w| (w.unit.as_str(), w.what.as_str()))
        .collect();
    assert!(
        words.contains(&("Rear L/R", "USB-direct mode and delay")),
        "{words:?}"
    );
    hub.stop();
}

#[test]
fn wp_ausb1_a_mixed_scene_goes_at_a_usb_frame_an_alink_frame_and_at_once() {
    let hub = hub(None);
    let h = hub.handle();
    h.system_status();
    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    assert_eq!(r.plan.path, "superframe", "{:?}", r.plan);
    let bus = r.plan.usb_frame.expect("a bus frame");
    assert!(bus < 2048);
    let usb: Vec<_> = r.plan.usb_ops.iter().map(|o| o.element.as_str()).collect();
    assert_eq!(usb, ["U1", "U2"]);
    assert_eq!(r.plan.direct.len(), 1, "K at once");
    let res = |e: &str| {
        r.writes
            .iter()
            .find(|w| w.element == e)
            .map(|w| w.result.clone())
    };
    assert_eq!(res("U1").as_deref(), Some("queued"), "{:?}", r.writes);
    assert_eq!(res("U2").as_deref(), Some("queued"));
    assert_eq!(res("K").as_deref(), Some("applied_now"));
    // The ops ran on both USB-direct units at that bus frame.
    std::thread::sleep(Duration::from_millis(
        r.plan.usb_lead_samples.unwrap() / 48 + 100,
    ));
    let core = connect(&hub);
    fresh_view(&core);
    let rc = core.alink_system_recall("evening".into()).unwrap();
    assert!(rc.taken);
    assert!(
        rc.message.starts_with("Evening recalled: at USB frame ")
            && rc
                .message
                .contains(" on Rear L/R, Height; at audio link frame ")
            && rc.message.contains(
                " through the sender on the others; applied now on K340 (no audio link)."
            ),
        "{}",
        rc.message
    );
    let u1 = rc.writes.iter().find(|w| w.unit == "Rear L/R").unwrap();
    assert_eq!(
        (u1.what.as_str(), u1.result_word.as_str()),
        ("Master volume", "Queued")
    );
    // Again before the first one's frame: the sender holds it and refuses; the
    // USB-direct units are held back with it, so no room plays half a scene.
    let again = h.system_apply_scene("evening", Role::Admin).unwrap();
    let tx = again.writes.iter().find(|w| w.element == "tx").unwrap();
    if tx.result == "refused" {
        for e in ["U1", "U2"] {
            let w = again.writes.iter().find(|w| w.element == e).unwrap();
            assert_eq!(
                (w.result.as_str(), w.reason.as_deref()),
                ("skipped", Some(dspi_hub::system::USB_HELD_BACK))
            );
        }
    }
    std::thread::sleep(Duration::from_millis(200));
    // A scene of one USB-direct unit alone: the bus frame only.
    let r = h.system_apply_scene("rear", Role::Admin).unwrap();
    assert_eq!(
        (r.plan.path.as_str(), r.plan.usb_ops.len()),
        ("usb_frame", 1)
    );
    assert!(r.on_time, "{:?}", r.writes);
    hub.stop();
}

#[test]
fn wp_ausb1_the_mode_is_set_from_the_editor_and_a_rate_locked_unit_takes_scenes_at_once() {
    let hub = hub(None);
    let core = connect(&hub);
    let v = fresh_view(&core);
    let edits = AlinkSystemEdits {
        usb_modes: vec![AlinkUnitUsbMode {
            unit_id: "U2".into(),
            mode: Some("rate_lock".into()),
        }],
        ..AlinkSystemEdits::default()
    };
    let rev = core.alink_system_save(v.revision, edits.clone()).unwrap();
    assert_eq!(rev, v.revision + 1);
    // Saved on the old revision again: refused, nothing saved.
    let stale = core.alink_system_save(v.revision, edits);
    assert!(
        matches!(stale, Err(dspi_core::DspiError::Link { ref code, .. }) if code == "conflict"),
        "{stale:?}"
    );
    // Only a USB-direct unit takes a mode; only known words.
    let bad = AlinkSystemEdits {
        usb_modes: vec![AlinkUnitUsbMode {
            unit_id: "A".into(),
            mode: Some("rate_lock".into()),
        }],
        ..AlinkSystemEdits::default()
    };
    assert!(matches!(
        core.alink_system_save(rev, bad),
        Err(dspi_core::DspiError::InvalidArgument { .. })
    ));
    let bad = AlinkSystemEdits {
        usb_modes: vec![AlinkUnitUsbMode {
            unit_id: "U2".into(),
            mode: Some("fast".into()),
        }],
        ..AlinkSystemEdits::default()
    };
    assert!(matches!(
        core.alink_system_save(rev, bad),
        Err(dspi_core::DspiError::InvalidArgument { .. })
    ));
    let (m, _) = hub.handle().system_model();
    assert_eq!(m.unit("U2").unwrap().usb_mode.as_deref(), Some("rate_lock"));

    let r = hub.handle().system_align(true);
    let u2 = r
        .writes
        .iter()
        .find(|w| w.element == "U2" && w.what == "usb_direct")
        .unwrap();
    assert_eq!(u2.result, "ok");
    let s = hub.handle().system_status();
    assert_eq!(
        s.observed["HW-U2"].usb.as_ref().unwrap().state,
        "rate_locked"
    );
    assert!(codes(&s).contains(&("U2".into(), "usb_not_phase_locked".into())));
    let v = fresh_view(&core);
    let row = v.units.iter().find(|u| u.id == "U2").unwrap();
    assert_eq!(
        (row.state.as_str(), row.usb_mode_wanted.as_deref()),
        ("rate_locked", Some("rate_lock"))
    );
    assert_eq!(v.usb_locked, 2, "rate-locked as asked");

    let r = hub
        .handle()
        .system_apply_scene("evening", Role::Admin)
        .unwrap();
    let u2 = r.writes.iter().find(|w| w.element == "U2").unwrap();
    assert_eq!(
        (u2.result.as_str(), u2.reason.as_deref()),
        ("applied_now", Some("not phase-locked (USB-direct)"))
    );
    let rc = core.alink_system_recall("evening".into()).unwrap();
    assert!(
        rc.message
            .contains("applied now on K340 (no audio link), Height (USB-direct, not phase-locked)"),
        "{}",
        rc.message
    );
    assert!(
        rc.message.contains("at USB frame ") && rc.message.contains(" on Rear L/R;"),
        "{}",
        rc.message
    );
    hub.stop();
}

#[test]
fn wp_ausb1_a_unit_without_the_block_and_bad_models() {
    let extra =
        json!({"id": "X", "name": "Old firmware", "serial": "HW-X", "transport": "usb_direct"});
    let hub = hub(Some(extra));
    let s = hub.handle().system_status();
    let x = serde_json::to_value(&s.observed["HW-X"]).unwrap();
    assert_eq!(x["usbDirect"], "none", "{x}");
    assert!(codes(&s).contains(&("X".into(), "no_usb_direct".into())));
    let warnings = hub.handle().system_alink_warnings();
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("Old firmware (HW-X): its firmware has no USB-direct block")),
        "{warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("Rear L/R")),
        "{warnings:?}"
    );
    let core = connect(&hub);
    let v = fresh_view(&core);
    let row = v.units.iter().find(|u| u.id == "X").unwrap();
    assert_eq!(
        (row.state.as_str(), row.state_word.as_str()),
        ("no_usb_direct", "No USB-direct (firmware)")
    );
    let r = hub.handle().system_align(true);
    let xw = r.writes.iter().find(|w| w.element == "X").unwrap();
    assert_eq!(
        (xw.result.as_str(), xw.reason.as_deref()),
        ("skipped", Some("no USB-direct block (firmware)"))
    );

    // The model's rules for USB-direct units.
    let refuse = |unit: Value| {
        let mut m = model(None);
        m["units"].as_array_mut().unwrap().push(unit);
        let m: dspi_hub::system::model::SystemModel = serde_json::from_value(m).unwrap();
        m.validate().is_err()
    };
    assert!(refuse(
        json!({"id": "Y", "serial": "S-Y", "transport": "usb_direct", "upstream": "tx"})
    ));
    assert!(refuse(
        json!({"id": "Y", "serial": "S-Y", "transport": "usb_direct", "usbMode": "fast"})
    ));
    assert!(refuse(
        json!({"id": "Y", "serial": "S-Y", "upstream": "tx", "usbMode": "off"})
    ));
    assert!(refuse(
        json!({"id": "Y", "serial": "S-Y", "transport": "usb_direct", "slots": [0, 1, 255, 255, 255, 255, 255, 255]})
    ));
    assert!(!refuse(
        json!({"id": "Y", "serial": "S-Y", "transport": "usb_direct", "usbMode": "off"})
    ));
    // And a USB-direct unit is never a feed.
    assert!(refuse(
        json!({"id": "Y", "serial": "S-Y", "upstream": "U1"})
    ));
    hub.stop();
}
