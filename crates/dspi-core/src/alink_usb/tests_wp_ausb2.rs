//! WP AUSB2: the lock-error action (`EXT_ALINK_USB` byte 4), the lock-error
//! flag and DIAG layout 2 through `DspiCore`, against `FakeDevice`'s
//! USB-direct unit as the fork's WP USB3b and as firmware before it.

use dspi_transport::fake::{USB_SIM_HOST_RANGE, UsbDirectSetup};
use dspi_transport::{FakeDevice, FakeRemote, Transport};

use super::*;
use crate::ext::tests_wp_c8::k340;

fn unit(usb3b: bool) -> (std::sync::Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo_usb_direct(UsbDirectSetup {
        usb3b,
        ..UsbDirectSetup::default()
    });
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), serial, 0).unwrap();
    (core, remote)
}

#[test]
fn wp_ausb2_the_lock_error_action_is_set_and_kept_by_a_mode_write() {
    let (c, _r) = unit(true);
    assert_eq!(c.fetch_alink_usb().unwrap().on_lock_error, "mute");
    let anchors = c.fetch_alink_usb_status().unwrap().anchors;
    c.set_alink_usb_on_lock_error(true).unwrap();
    let cfg = c.fetch_alink_usb().unwrap();
    assert_eq!(
        (cfg.on_lock_error.as_str(), cfg.mode, cfg.d),
        ("play_unaligned", 2, 192)
    );
    assert_eq!(
        c.fetch_alink_usb_status().unwrap().anchors,
        anchors,
        "no re-anchor"
    );
    // Writing the mode and D keeps the action the unit has.
    c.set_alink_usb(2, true, 200).unwrap();
    let cfg = c.fetch_alink_usb().unwrap();
    assert_eq!((cfg.on_lock_error.as_str(), cfg.d), ("play_unaligned", 200));
    c.set_alink_usb_on_lock_error(false).unwrap();
    assert_eq!(c.fetch_alink_usb().unwrap().on_lock_error, "mute");
    // DIAG layout 2.
    assert_eq!(c.fetch_alink_usb_diag().unwrap().version, 2);
}

#[test]
fn wp_ausb2_older_firmware_says_unsupported_for_play_unaligned() {
    let (c, _r) = unit(false);
    assert_eq!(
        c.set_alink_usb_on_lock_error(true),
        Err(DspiError::Unsupported)
    );
    // Mute is what it does anyway: the write of 0 goes through.
    c.set_alink_usb_on_lock_error(false).unwrap();
    let d = c.fetch_alink_usb_diag().unwrap();
    assert_eq!((d.version, d.lock_errors), (1, 0));
}

#[test]
fn wp_ausb2_a_lock_error_shows_in_the_status_and_the_diag() {
    let (c, r) = unit(true);
    assert!(!c.fetch_alink_usb_status().unwrap().lock_error);
    assert!(r.usb_direct_sim(USB_SIM_HOST_RANGE));
    let s = c.fetch_alink_usb_status().unwrap();
    assert_eq!((s.state.as_str(), s.lock_error), ("rejected", true));
    assert_eq!(c.fetch_alink_usb_diag().unwrap().lock_errors, 1);
}

#[test]
fn wp_ausb2_without_the_block_it_is_unsupported_with_no_traffic() {
    let p = k340();
    let before = p.traffic();
    assert_eq!(
        p.core.set_alink_usb_on_lock_error(true),
        Err(DspiError::Unsupported)
    );
    assert_eq!(p.traffic(), before, "nothing was sent");
}
