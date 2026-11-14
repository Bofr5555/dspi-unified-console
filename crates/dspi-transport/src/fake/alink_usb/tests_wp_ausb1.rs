//! WP AUSB1: the fake's USB-direct unit answers as 2.9 specifies; the demo
//! device and an alink receiver do not list the block.

use super::super::FakeDevice;
use super::*;
use crate::{NotificationSource, Transport, TransportError};
use dspi_proto::generated::opcodes as op;

fn get(d: &mut FakeDevice, id: u16, len: u16) -> crate::Result<Vec<u8>> {
    d.control_in(op::REQ_EXTENSION, id, len)
}

fn set(d: &mut FakeDevice, id: u16, data: &[u8]) -> crate::Result<()> {
    d.control_out(op::REQ_EXTENSION, id, data)
}

fn list(d: &mut FakeDevice) -> Vec<u16> {
    let b = d
        .control_in_upto(op::REQ_EXTENSION, dspi_proto::generated::ext::EXT_LIST, 64)
        .unwrap();
    dspi_proto::ext::decode_list(&b)
}

fn status(d: &mut FakeDevice) -> AlinkUsbStatus {
    AlinkUsbStatus::decode(&get(d, usb::EXT_ALINK_USB_STATUS, 64).unwrap()).unwrap()
}

fn drain(src: &mut Box<dyn NotificationSource>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let p = src.read(Duration::from_millis(1)).unwrap();
        if p.is_empty() {
            return out;
        }
        out.push(p);
    }
}

fn usb_events(src: &mut Box<dyn NotificationSource>) -> Vec<AlinkUsbEvent> {
    drain(src)
        .iter()
        .filter(|p| p[1] == usb::NOTIFY_EVT_ALINK_USB)
        .map(|p| AlinkUsbEvent::decode(&p[4..]).unwrap())
        .collect()
}

#[test]
fn wp_ausb1_only_a_usb_direct_unit_lists_the_block() {
    let mut demo = FakeDevice::demo();
    assert!(!list(&mut demo).contains(&usb::EXT_ALINK_USB));
    assert!(matches!(
        get(&mut demo, usb::EXT_ALINK_USB_STATUS, 64),
        Err(TransportError::Stalled { .. })
    ));
    let mut rx = FakeDevice::demo_alink();
    assert!(!list(&mut rx).contains(&usb::EXT_ALINK_USB));
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let ids = list(&mut d);
    for id in [0x8005, 0x8006, 0x8010, 0x8011, 0x8012] {
        assert!(ids.contains(&id), "{id:#06x} in {ids:x?}");
    }
    assert!(
        !ids.contains(&alink::EXT_ALINK_INFO),
        "no alink in this build"
    );
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ascending");
    let f = get(&mut d, dspi_proto::generated::ext::EXT_NOTIFY_FEATURES, 4).unwrap();
    let f = u32::from_le_bytes(f.try_into().unwrap());
    assert_eq!(
        f & (usb::NOTIFY_FEAT_ALINK_USB | alink::NOTIFY_FEAT_APPLY_RESULT),
        usb::NOTIFY_FEAT_ALINK_USB | alink::NOTIFY_FEAT_APPLY_RESULT
    );
}

#[test]
fn wp_ausb1_a_demo_unit_is_phase_locked_with_its_stream_placed() {
    let setup = UsbDirectSetup {
        stream_lag_frames: 3,
        stream_shift: -2,
        ..UsbDirectSetup::default()
    };
    let mut d = FakeDevice::demo_usb_direct(setup);
    std::thread::sleep(Duration::from_millis(5));
    let s = status(&mut d);
    assert_eq!(
        (s.state, s.phase_state, s.rate_code),
        (UsbState::PhaseLocked, 2, 1)
    );
    assert_eq!((s.d_min, s.d_max), (170, 399));
    assert!((s.host_ppm + 11.5).abs() < 1e-3);
    assert!(s.phase_error.abs() <= 0.1);
    assert_eq!(s.stream_shift, -2);
    // The content began 3 frames after the host's first SOF.
    let epoch = u64::from(setup.origin_wraps) * 2048;
    assert_eq!(
        s.stream_start_bus_frame(),
        Some(((epoch - 512 + 3) % 2048) as u16)
    );
    let c = AlinkUsbConfig::decode(&get(&mut d, usb::EXT_ALINK_USB, 8).unwrap()).unwrap();
    assert_eq!((c.mode, c.d, c.state), (2, 192, 5));
}

#[test]
fn wp_ausb1_two_units_on_one_host_see_the_same_bus_frame() {
    let mut a = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let mut b = FakeDevice::demo_usb_direct(UsbDirectSetup {
        origin_wraps: 7,
        ..UsbDirectSetup::default()
    });
    for _ in 0..3 {
        let ca = FrameCounterTb::decode(&get(&mut a, 0x8005, 20).unwrap()).unwrap();
        let cb = FrameCounterTb::decode(&get(&mut b, 0x8005, 20).unwrap()).unwrap();
        assert_eq!(
            (ca.time_base, cb.time_base),
            (TimeBase::UsbSample, TimeBase::UsbSample)
        );
        // Read a moment apart: within a frame or two of each other, mod 2048.
        let diff = (i32::from(cb.bus_frame) - i32::from(ca.bus_frame)).rem_euclid(2048);
        assert!(
            diff <= 2 || diff >= 2046,
            "{} vs {}",
            ca.bus_frame,
            cb.bus_frame
        );
        // Their S differ by whole wraps of 2048 frames.
        let ds = cb.counter.frame.abs_diff(ca.counter.frame);
        assert!((ds as f64 / (48.0 * 2048.0) - 4.0).abs() < 0.01, "{ds}");
    }
}

#[test]
fn wp_ausb1_the_config_refusals_and_a_mode_change() {
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let mut src = d.notifications().unwrap();
    drain(&mut src);
    let refused = |d: &mut FakeDevice, b: &[u8]| {
        matches!(
            set(d, usb::EXT_ALINK_USB, b),
            Err(TransportError::Stalled { .. })
        )
    };
    assert!(refused(&mut d, &[3, 1, 192, 0, 0, 0, 0, 0]), "mode 3");
    assert!(refused(&mut d, &[2, 2, 192, 0, 0, 0, 0, 0]), "flag bit 1");
    assert!(refused(&mut d, &[2, 1, 0, 0, 0, 0, 0, 0]), "D 0");
    assert!(refused(&mut d, &[2, 1, 192, 0, 1, 0, 0, 0]), "reserved");
    assert!(
        refused(&mut d, &[2, 1, 0x90, 0x01, 0, 0, 0, 0]),
        "D 400 in mode 2"
    );
    assert!(refused(&mut d, &[2, 1, 192, 0, 0, 0, 0]), "short");
    // Mode 1 takes D 400: it is not used.
    set(&mut d, usb::EXT_ALINK_USB, &[1, 1, 0x90, 0x01, 0, 0, 0, 0]).unwrap();
    assert_eq!(status(&mut d).state, UsbState::RateLocked);
    // Back to phase lock with D 250: anchoring, then locked; REANCHOR sent.
    set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 250, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(status(&mut d).state, UsbState::Anchoring);
    std::thread::sleep(ANCHOR + Duration::from_millis(10));
    let s = status(&mut d);
    assert_eq!((s.state, s.anchors), (UsbState::PhaseLocked, 2));
    // Off: parked, DISABLED.
    set(&mut d, usb::EXT_ALINK_USB, &[0, 1, 250, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(status(&mut d).state, UsbState::Off);
    let e = usb_events(&mut src);
    let reasons: Vec<_> = e.iter().map(|e| e.reason).collect();
    assert!(reasons.contains(&UsbEventReason::Reanchor), "{reasons:?}");
    assert_eq!(reasons.last(), Some(&UsbEventReason::Disabled));
    // From off: pull-in first.
    set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 192, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(status(&mut d).state, UsbState::PullIn);
    assert!(d.remote().usb_direct_sim(USB_SIM_SETTLE));
    assert_eq!(status(&mut d).state, UsbState::PhaseLocked);
    // The counter answers time base 0xFF in mode 0.
    set(&mut d, usb::EXT_ALINK_USB, &[0, 1, 192, 0, 0, 0, 0, 0]).unwrap();
    let c = FrameCounterTb::decode(&get(&mut d, 0x8005, 20).unwrap()).unwrap();
    assert_eq!(c.time_base, TimeBase::None);
}

#[test]
fn wp_ausb1_ops_on_the_bus_frame_run_and_report_the_bus_form() {
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let mut src = d.notifications().unwrap();
    drain(&mut src);
    let c = FrameCounterTb::decode(&get(&mut d, 0x8005, 20).unwrap()).unwrap();
    let bus = (c.bus_frame + 30) % 2048;
    let req = ApplyRequest {
        frame: usb::bus_target(bus, 5, 48).unwrap(),
        op: ApplyOp::MasterVolume,
        arg: 0,
        value: -1200,
    };
    set(&mut d, 0x8006, &usb::encode_apply(&req, TimeBase::BusFrame)).unwrap();
    let q = ApplyStatus::decode(&get(&mut d, 0x8006, 16).unwrap()).unwrap();
    assert_eq!(q.pending, 1);
    std::thread::sleep(Duration::from_millis(45));
    let q = ApplyStatus::decode(&get(&mut d, 0x8006, 16).unwrap()).unwrap();
    assert_eq!(
        (q.pending, q.last_op, q.last_result),
        (0, ApplyOp::MasterVolume, ApplyResult::Applied)
    );
    assert_eq!(usb::bus_of_sample(q.last_frame, 48), Some((bus, 5)));
    let e: Vec<_> = drain(&mut src)
        .into_iter()
        .filter(|p| p[1] == alink::NOTIFY_EVT_APPLY_RESULT)
        .collect();
    let r = ApplyResultEvent::decode(&e[0][4..]).unwrap();
    assert_eq!(r.frame_low, u32::from(bus) | (5 << 16));
    // A bus frame of the last 64 is late; a bad form and time base 0 are refused.
    let now = FrameCounterTb::decode(&get(&mut d, 0x8005, 20).unwrap()).unwrap();
    let past = ApplyRequest {
        frame: usb::bus_target((now.bus_frame + 2048 - 10) % 2048, 0, 48).unwrap(),
        ..req
    };
    set(
        &mut d,
        0x8006,
        &usb::encode_apply(&past, TimeBase::BusFrame),
    )
    .unwrap();
    let q = ApplyStatus::decode(&get(&mut d, 0x8006, 16).unwrap()).unwrap();
    assert_eq!(q.last_result, ApplyResult::Late);
    let bad = ApplyRequest {
        frame: 1 << 12,
        ..req
    };
    assert!(set(&mut d, 0x8006, &usb::encode_apply(&bad, TimeBase::BusFrame)).is_err());
    assert!(set(&mut d, 0x8006, &usb::encode_apply(&req, TimeBase::Alink)).is_err());
    // Not phase-locked: refused.
    set(&mut d, usb::EXT_ALINK_USB, &[1, 1, 192, 0, 0, 0, 0, 0]).unwrap();
    assert!(set(&mut d, 0x8006, &usb::encode_apply(&req, TimeBase::BusFrame)).is_err());
}

#[test]
fn wp_ausb1_the_bus_sims_count_in_status_and_diag() {
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let r = d.remote();
    assert!(r.usb_direct_sim(USB_SIM_GAP));
    let s = status(&mut d);
    assert_eq!((s.gap_count, s.missed_samples), (1, 96));
    assert!(r.usb_direct_sim(USB_SIM_HOLDOVER));
    assert!(r.usb_direct_sim(USB_SIM_SOF_LOST));
    assert_eq!(status(&mut d).state, UsbState::NoSof);
    let g = AlinkUsbDiag::decode(&get(&mut d, usb::EXT_ALINK_USB_DIAG, 64).unwrap()).unwrap();
    assert_eq!(
        (g.holdovers, g.rides, g.ends[0], g.bus_resets),
        (1, 1, 1, 1)
    );
    assert_eq!(g.last_cause_name(), Some("sof_lost"));
    assert!(r.usb_direct_sim(USB_SIM_SETTLE));
    assert_eq!(status(&mut d).state, UsbState::PhaseLocked);
    assert!(r.usb_direct_sim(USB_SIM_HOST_RANGE));
    assert_eq!(status(&mut d).state, UsbState::Rejected);
    assert!(r.usb_direct_sim(USB_SIM_HOST_RANGE));
    set(&mut d, usb::EXT_ALINK_USB_DIAG, &[1]).unwrap();
    let g = AlinkUsbDiag::decode(&get(&mut d, usb::EXT_ALINK_USB_DIAG, 64).unwrap()).unwrap();
    assert_eq!((g.total_ends(), g.last_cause), (0, None));
    assert!(set(&mut d, usb::EXT_ALINK_USB_DIAG, &[2]).is_err());
    assert!(!r.usb_direct_sim(42));
    assert!(!FakeDevice::demo().remote().usb_direct_sim(USB_SIM_GAP));
}
