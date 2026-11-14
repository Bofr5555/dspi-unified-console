//! WP AUSB1: the USB-direct block through `DspiCore`, against `FakeDevice`'s
//! USB-direct unit and as firmware without it (no traffic, `Unsupported`), and
//! its 0xE3 in the event log.

use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use dspi_transport::fake::{USB_SIM_GAP, USB_SIM_SOF_LOST, UsbDirectSetup};
use dspi_transport::{FakeDevice, FakeRemote, Transport};

use super::*;
use crate::consts::NOTIFY_INPUT_FORMAT;
use crate::ext::tests_wp_c8::k340;
use crate::{EVENT_READ_MAX, EventEntry};

const WAIT: Duration = Duration::from_secs(8);

fn usb_unit() -> (std::sync::Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), serial, 0).unwrap();
    (core, remote)
}

/// Every USB-direct call, each expected to fail `Unsupported`.
fn every_call(core: &DspiCore) -> Vec<(&'static str, Result<()>)> {
    vec![
        ("config", core.fetch_alink_usb().map(drop)),
        ("set", core.set_alink_usb(2, true, 192)),
        ("status", core.fetch_alink_usb_status().map(drop)),
        ("diag", core.fetch_alink_usb_diag().map(drop)),
        ("clear", core.clear_alink_usb_diag()),
        ("counter", core.fetch_frame_counter_time_base().map(drop)),
        ("apply bus", core.apply_at_time_base(2, 5, 1, 0, 0)),
        ("apply sample", core.apply_at_time_base(1, 5, 1, 0, 0)),
        ("apply alink", core.apply_at_time_base(0, 5, 1, 0, 0)),
    ]
}

#[test]
fn wp_ausb1_without_the_block_every_call_is_unsupported_with_no_traffic() {
    let p = k340();
    assert!(!p.core.usb_direct_supported().unwrap());
    let before = p.traffic();
    for (name, r) in every_call(&p.core) {
        assert!(matches!(r, Err(DspiError::Unsupported)), "{name}: {r:?}");
    }
    assert_eq!(p.traffic(), before, "nothing was sent");
}

#[test]
fn wp_ausb1_the_calls_against_the_fake_unit() {
    let (c, _remote) = usb_unit();
    assert!(c.usb_direct_supported().unwrap());
    // A USB-direct build without the alink: no alink info.
    assert_eq!(c.alink_info().unwrap(), None);
    let cfg = c.fetch_alink_usb().unwrap();
    assert_eq!(
        (
            cfg.mode,
            cfg.mode_name.as_str(),
            cfg.d,
            cfg.auto_reanchor,
            cfg.state.as_str()
        ),
        (2, "phase_lock", 192, true, "phase_locked")
    );
    let s = c.fetch_alink_usb_status().unwrap();
    assert_eq!(
        (s.state.as_str(), s.rate_hz, s.d_min, s.d_max),
        ("phase_locked", 48_000, 170, 399)
    );
    assert!(s.time_base && s.stream && s.hw_timestamp);
    assert!((s.host_ppm + 11.5).abs() < 1e-3);
    assert!(s.stream_start_bus_frame.is_some());

    // A new D: anchoring, then locked again; out of range: refused.
    c.set_alink_usb(2, true, 250).unwrap();
    assert_eq!(c.fetch_alink_usb().unwrap().d, 250);
    assert!(matches!(
        c.set_alink_usb(2, true, 400),
        Err(DspiError::Refused { .. })
    ));
    assert!(matches!(
        c.set_alink_usb(3, true, 192),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        c.set_alink_usb(1, true, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    std::thread::sleep(dspi_transport::fake::USB_DIRECT_ANCHOR + Duration::from_millis(10));

    let d = c.fetch_alink_usb_diag().unwrap();
    assert_eq!(d.ends.len(), 9);
    assert_eq!(d.ends[0].cause, "sof_lost");
    c.clear_alink_usb_diag().unwrap();

    let f = c.fetch_frame_counter_time_base().unwrap();
    assert_eq!((f.time_base.as_str(), f.time_base_code), ("usb_sample", 1));
    assert_eq!(u64::from(f.bus_frame), (f.frame / 48) % 2048);
    // A volume change 40 frames ahead on the bus: queued.
    let bus = (f.bus_frame + 40) % 2048;
    c.apply_at_time_base(TIME_BASE_BUS_FRAME, u64::from(bus), 4, 0, -600)
        .unwrap();
    // Time base 0 is the alink's: this unit has none.
    assert!(matches!(
        c.apply_at_time_base(TIME_BASE_ALINK, 5, 1, 0, 0),
        Err(DspiError::Unsupported)
    ));
    // A bad bus form and a time base 3 are refused before the wire.
    assert!(matches!(
        c.apply_at_time_base(TIME_BASE_BUS_FRAME, 1 << 12, 1, 0, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        c.apply_at_time_base(3, 5, 1, 0, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    // Rate lock: no phase, the unit refuses a bus-frame op.
    c.set_alink_usb(1, true, 192).unwrap();
    assert!(matches!(
        c.apply_at_time_base(TIME_BASE_BUS_FRAME, u64::from(bus), 1, 0, 0),
        Err(DspiError::Refused { .. })
    ));
}

#[test]
fn wp_ausb1_mode_changes_are_journalled_with_a_hazard() {
    let (c, _remote) = usb_unit();
    c.set_alink_usb(2, true, 200).unwrap();
    c.clear_alink_usb_diag().unwrap();
    let h = c.history(10);
    let paths: Vec<_> = h.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"ext.alink.usb"), "{paths:?}");
    assert!(paths.contains(&"ext.alink.usb.diag_clear"), "{paths:?}");
}

/// Push `INPUT_FORMAT` and process until it is seen.
fn settle(core: &DspiCore, remote: &FakeRemote) {
    remote.push(vec![
        2,
        nfy::NOTIFY_EVT_INPUT_FORMAT as u8,
        0,
        0,
        2,
        0,
        0,
        0,
    ]);
    let start = Instant::now();
    while core.process_notifications().flags & NOTIFY_INPUT_FORMAT == 0 {
        assert!(start.elapsed() < WAIT, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn entries(core: &DspiCore, kind: &str) -> Vec<EventEntry> {
    core.event_log_read(String::new(), 0, EVENT_READ_MAX)
        .entries
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect()
}

#[test]
fn wp_ausb1_gaps_and_a_lost_time_base_reach_the_event_log() {
    let (c, remote) = usb_unit();
    settle(&c, &remote);
    assert!(remote.usb_direct_sim(USB_SIM_GAP));
    assert!(remote.usb_direct_sim(USB_SIM_SOF_LOST));
    settle(&c, &remote);
    let log = entries(&c, "audio_link");
    let seen: Vec<_> = log
        .iter()
        .map(|e| (e.source.as_str(), e.text.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                "gap",
                "Audio link USB-direct missed USB packets (a faded gap)"
            ),
            (
                "lost",
                "Audio link USB-direct lost its USB time base: anchoring again"
            ),
        ],
        "{log:?}"
    );
}

#[test]
fn wp_ausb1_usb_events_without_bit_4_are_not_logged() {
    let p = k340();
    settle(&p.core, &p.remote);
    p.remote.push(vec![2, 0xE3, 0, 0, 5, 2, 5, 0, 0, 0, 0, 0]);
    settle(&p.core, &p.remote);
    assert!(entries(&p.core, "audio_link").is_empty());
}

#[test]
fn wp_ausb1_the_demo_unit_opens() {
    let core = DspiCore::new();
    core.open_demo_usb_direct().unwrap();
    assert!(core.usb_direct_supported().unwrap());
    core.open_demo().unwrap();
    assert!(!core.usb_direct_supported().unwrap());
}
