//! WP FWX: identify, the reset readout and the clip event through
//! `DspiCore`, against `FakeDevice` as wp/FWX firmware and as bofr-main
//! 04122d3 (which has none of them).

use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use dspi_transport::fake::FAKE_HW_REV;
use dspi_transport::{Capture, FakeDevice, FakeOptions, FakeRemote};

use super::*;
use crate::consts::NOTIFY_INPUT_FORMAT;
use crate::ext::tests_wp_c8::{Faults, k340, k340_options, probed};

const WAIT: Duration = Duration::from_secs(8);

fn fwx_options() -> FakeOptions {
    FakeOptions {
        diag_ext: true,
        ..k340_options()
    }
}

#[test]
fn wp_fwx_identify_and_reset_info_on_fwx_firmware() {
    let p = probed(Capture::demo().bulk, fwx_options(), Faults::default());
    assert!(p.core.has_clip_events().unwrap());
    let at_connect = p.core.reset_info_at_connect().unwrap().unwrap();
    assert_eq!(at_connect.reason, "power_on");
    assert_eq!(at_connect.prev_uptime_s, None);

    p.core.identify(10, None).unwrap();
    let id = p.core.fetch_identify().unwrap();
    assert!((9..=10).contains(&id.remaining_s), "{}", id.remaining_s);
    assert_eq!(id.last_tone_output, None);
    assert_eq!((id.chip.as_str(), id.board.as_str()), ("RP2350", "pico2"));
    assert_eq!(id.hw_rev, FAKE_HW_REV);

    p.core.identify(0, Some(3)).unwrap();
    let id = p.core.fetch_identify().unwrap();
    assert_eq!((id.remaining_s, id.last_tone_output), (0, Some(3)));

    // A tone past the 9 outputs: refused before anything is sent.
    let before = p.traffic();
    assert!(matches!(
        p.core.identify(5, Some(9)),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(p.traffic(), before);

    let r = p.core.fetch_reset_info().unwrap();
    assert_eq!(
        (r.reason.as_str(), r.reason_code, r.chip.as_str()),
        ("power_on", 1, "RP2350")
    );
    assert!(!r.description.is_empty());
    assert_eq!(r.hang_count, 0);
}

#[test]
fn wp_fwx_older_firmware_is_unsupported_and_nothing_is_sent() {
    // bofr-main 04122d3: extensions 1 and 2 (3 with bulk-source), none of these.
    let p = k340();
    let before = p.traffic();
    assert_eq!(p.core.identify(10, None), Err(DspiError::Unsupported));
    assert_eq!(p.core.fetch_identify(), Err(DspiError::Unsupported));
    assert_eq!(p.core.fetch_reset_info(), Err(DspiError::Unsupported));
    assert_eq!(p.core.reset_info_at_connect(), Ok(None));
    assert_eq!(p.core.has_clip_events(), Ok(false));
    assert_eq!(p.core.take_clip_mask(), 0);
    assert_eq!(p.traffic(), before);

    let core = DspiCore::new();
    assert_eq!(core.identify(1, None), Err(DspiError::Disconnected));
    assert_eq!(core.fetch_identify(), Err(DspiError::Disconnected));
    assert_eq!(core.fetch_reset_info(), Err(DspiError::Disconnected));
    assert_eq!(core.reset_info_at_connect(), Err(DspiError::Disconnected));
    assert_eq!(core.has_clip_events(), Err(DspiError::Disconnected));
    assert_eq!(core.take_clip_mask(), 0);
}

#[test]
fn wp_fwx_a_refused_identify_is_a_refusal() {
    let p = probed(
        Capture::demo().bulk,
        fwx_options(),
        Faults {
            stall_ext_writes: true,
            ..Faults::default()
        },
    );
    assert!(matches!(
        p.core.identify(10, Some(0)),
        Err(DspiError::Refused { .. })
    ));
}

fn attach(diag_ext: bool) -> (std::sync::Arc<DspiCore>, FakeRemote) {
    let options = FakeOptions {
        diag_ext,
        bulk_source: true,
        ..k340_options()
    };
    let dev = FakeDevice::new(Capture::demo(), options);
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "FWX".into(), 0).unwrap();
    core.process_notifications();
    (core, remote)
}

/// A clip event as wp/FWX firmware sends it (the fake's `EXT_FAKE_CLIP`
/// sends the same, dspi-transport's tests show).
fn clip(mask: u32) -> Vec<u8> {
    let mut p = vec![
        2,
        0xE0,
        0,
        0,
        mask.trailing_zeros() as u8,
        mask.count_ones() as u8,
        0,
        0,
    ];
    p.extend(mask.to_le_bytes());
    p
}

/// Push `INPUT_FORMAT` and process until it is seen: everything pushed
/// before it has been processed too.
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

#[test]
fn wp_fwx_clip_events_reach_take_clip_mask_behind_the_feature_bit() {
    let (core, remote) = attach(true);
    // Two events: outputs 1 and 4 of the RP2350 (channels 9, 12), then 9 again.
    remote.push(clip(0x1200));
    remote.push(clip(0x0200));
    settle(&core, &remote);
    assert_eq!(core.take_clip_mask(), 0x1200);
    assert_eq!(core.take_clip_mask(), 0, "taken");

    // The same packet from a device that does not announce clip events is
    // not a clip.
    let (core, remote) = attach(false);
    remote.push(clip(0x0200));
    settle(&core, &remote);
    assert_eq!(core.take_clip_mask(), 0);
}
