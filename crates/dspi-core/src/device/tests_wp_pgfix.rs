//! WP PGFIX (PGMAT F1): the core connects the alink sender with its own
//! commands only: device info, the extension channel, the update check's
//! target; no image, no DSP model, every DSP write `Unsupported`.

use dspi_transport::paged;

use super::*;
use crate::DeviceKind;

/// The guarded sender over USB, opened as every construction site does.
fn sender_core() -> (Arc<DspiCore>, dspi_transport::FakeRemote) {
    let dev = FakeDevice::sender(false);
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let core = DspiCore::new();
    let t = paged::open(Box::new(dev)).unwrap();
    core.open_transport(t, serial, 0).unwrap();
    (core, remote)
}

#[test]
fn wp_pgfix_the_sender_is_told_apart_by_product_and_platform() {
    let sender = [2u8, 1, 0x16, 0, 1, 6, 0];
    let dsp = [2u8, 1, 0x16, 5, 1, 6, 0];
    // Without a product string: the platform names no outputs.
    assert!(is_alink_sender_platform(&sender, None));
    assert!(!is_alink_sender_platform(&dsp, None));
    // The product string decides where there is one.
    assert!(is_alink_sender_platform(
        &dsp,
        Some("DSPi Audio Link Sender")
    ));
    assert!(is_alink_sender_platform(
        &sender,
        Some(" DSPi Link Sender ")
    ));
    assert!(!is_alink_sender_platform(&sender, Some("Weeb Labs DSPi")));
    assert!(is_alink_sender_platform(&sender, Some("")), "blank: none");
    // A reply that does not decode is nobody's.
    assert!(!is_alink_sender_platform(&[2, 1, 0x16], None));
    assert!(!is_alink_sender_platform(&[], None));
}

#[test]
fn wp_pgfix_the_sender_connects_with_its_own_commands() {
    let (core, remote) = sender_core();
    let p = core.device_protocol().unwrap();
    assert_eq!(p.device_kind, DeviceKind::AlinkSender);
    assert_eq!(p.generation, crate::ProtocolGeneration::Legacy);
    assert_eq!(p.ext_envelope, crate::ExtEnvelope::Legacy);
    // The device info: the platform's identity, no image, no channels.
    core.fetch_all().unwrap();
    let s = core.get_state();
    assert!(!s.bulk_valid);
    assert_eq!(
        (s.num_channels, s.num_input_channels, s.num_output_channels),
        (0, 0, 0)
    );
    assert_ne!(s.fw_major, 0, "the firmware version is read");
    // The extension channel and its alink block.
    assert!(!core.ext_status().unwrap().supported.is_empty());
    assert!(core.alink_info().unwrap().is_some());
    core.fetch_alink_status().unwrap();
    core.reset_alink_stats().unwrap();
    // The update check's target: a Pico with BOOTSEL.
    let target = core.firmware_target();
    assert!(target.has_bootsel(), "{target:?}");
    // Nothing of the DSP model: refused before anything is sent.
    assert_eq!(core.set_master_volume(-13.0), Err(DspiError::Unsupported));
    assert_eq!(core.apply_all_params(), Err(DspiError::Unsupported));
    assert_eq!(core.refresh_params(), Err(DspiError::Unsupported));
    assert_eq!(core.fetch_status().map(drop), Err(DspiError::Unsupported));
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    // Still connected.
    assert_eq!(
        core.device_protocol().unwrap().device_kind,
        DeviceKind::AlinkSender
    );
}

#[test]
fn wp_pgfix_a_dsp_is_a_dsp() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    assert_eq!(core.device_protocol().unwrap().device_kind, DeviceKind::Dsp);
}
