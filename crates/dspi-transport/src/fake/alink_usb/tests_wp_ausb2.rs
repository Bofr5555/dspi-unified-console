//! WP AUSB2: the fake's USB-direct unit takes the lock-error action (byte 4)
//! and reports lock error as the fork's WP USB3b does; one made as older
//! firmware refuses 1 and answers DIAG version 1.

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

fn config(d: &mut FakeDevice) -> AlinkUsbConfig {
    AlinkUsbConfig::decode(&get(d, usb::EXT_ALINK_USB, 8).unwrap()).unwrap()
}

fn status(d: &mut FakeDevice) -> AlinkUsbStatus {
    AlinkUsbStatus::decode(&get(d, usb::EXT_ALINK_USB_STATUS, 64).unwrap()).unwrap()
}

fn diag(d: &mut FakeDevice) -> AlinkUsbDiag {
    AlinkUsbDiag::decode(&get(d, usb::EXT_ALINK_USB_DIAG, 64).unwrap()).unwrap()
}

fn reasons(src: &mut Box<dyn NotificationSource>) -> Vec<UsbEventReason> {
    let mut out = Vec::new();
    loop {
        let p = src.read(Duration::from_millis(1)).unwrap();
        if p.is_empty() {
            return out;
        }
        if p[1] == usb::NOTIFY_EVT_ALINK_USB {
            out.push(AlinkUsbEvent::decode(&p[4..]).unwrap().reason);
        }
    }
}

#[test]
fn wp_ausb2_the_lock_error_action_is_kept_without_a_reanchor() {
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let mut src = d.notifications().unwrap();
    reasons(&mut src);
    assert_eq!(config(&mut d).on_lock_error, usb::ON_LOCK_ERROR_MUTE);
    let anchors = status(&mut d).anchors;
    set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 192, 0, 1, 0, 0, 0]).unwrap();
    assert_eq!(config(&mut d).on_lock_error, usb::ON_LOCK_ERROR_PLAY);
    let s = status(&mut d);
    assert_eq!((s.state, s.anchors), (UsbState::PhaseLocked, anchors));
    assert!(reasons(&mut src).is_empty(), "no re-anchor, no event");
    assert!(matches!(
        set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 192, 0, 2, 0, 0, 0]),
        Err(TransportError::Stalled { .. })
    ));
    assert_eq!(
        config(&mut d).on_lock_error,
        1,
        "a refused write changes nothing"
    );
    assert_eq!(diag(&mut d).version, usb::DIAG_VERSION);
}

#[test]
fn wp_ausb2_a_rejected_host_is_a_lock_error_in_mode_2() {
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup::default());
    let mut src = d.notifications().unwrap();
    reasons(&mut src);
    let r = d.remote();
    assert!(r.usb_direct_sim(USB_SIM_HOST_RANGE));
    let s = status(&mut d);
    assert_eq!(s.state, UsbState::Rejected);
    assert_ne!(s.flags & usb::USB_FLAG_LOCK_ERROR, 0);
    assert_eq!(
        reasons(&mut src),
        [UsbEventReason::HostRange, UsbEventReason::LockError]
    );
    assert_eq!(diag(&mut d).lock_errors, 1);
    assert!(r.usb_direct_sim(USB_SIM_HOST_RANGE));
    assert_eq!(status(&mut d).flags & usb::USB_FLAG_LOCK_ERROR, 0);
    assert_eq!(
        reasons(&mut src),
        [UsbEventReason::HostRange, UsbEventReason::LockOk]
    );
    // Mode 1 is never in lock error.
    set(&mut d, usb::EXT_ALINK_USB, &[1, 1, 192, 0, 0, 0, 0, 0]).unwrap();
    assert!(r.usb_direct_sim(USB_SIM_HOST_RANGE));
    assert_eq!(status(&mut d).flags & usb::USB_FLAG_LOCK_ERROR, 0);
    // Mode 2 while rejected: in lock error from the write on.
    reasons(&mut src);
    set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 192, 0, 0, 0, 0, 0]).unwrap();
    assert_ne!(status(&mut d).flags & usb::USB_FLAG_LOCK_ERROR, 0);
    assert!(reasons(&mut src).contains(&UsbEventReason::LockError));
    assert_eq!(diag(&mut d).lock_errors, 2);
}

#[test]
fn wp_ausb2_older_firmware_refuses_play_unaligned_and_reads_diag_1() {
    let mut d = FakeDevice::demo_usb_direct(UsbDirectSetup {
        usb3b: false,
        ..UsbDirectSetup::default()
    });
    assert!(matches!(
        set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 192, 0, 1, 0, 0, 0]),
        Err(TransportError::Stalled { .. })
    ));
    set(&mut d, usb::EXT_ALINK_USB, &[2, 1, 192, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(diag(&mut d).version, usb::DIAG_VERSION_1);
    assert!(d.remote().usb_direct_sim(USB_SIM_HOST_RANGE));
    assert_eq!(
        status(&mut d).flags & usb::USB_FLAG_LOCK_ERROR,
        0,
        "no lock error before WP USB3b"
    );
}
