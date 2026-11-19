//! Batch review PGS-DOC: the update check and the bootloader entry on the
//! dual-stack alink sender (FWPG4) on its paged stack. It is judged as the
//! guarded sender is: a Pico running our firmware, so an image that may be
//! upstream's asks for the lineage phrase, and the entry reads its target and
//! reboots with nothing on the wire but the sender's own commands.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use dspi_transport::{FakeDevice, RebootTarget, Transport, paged};

use super::tests_wp_pgfwup::image;
use super::*;
use crate::{ConfirmOp, DeviceKind, FirmwareGeneration, FirmwareLineage};

#[test]
fn review_pgsdoc_the_paged_sender_is_an_update_target_of_our_lineage() {
    let dev = FakeDevice::sender(true);
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let core = DspiCore::new();
    core.open_transport(paged::open(Box::new(dev)).unwrap(), serial, 0)
        .unwrap();
    assert_eq!(
        core.device_protocol().unwrap().device_kind,
        DeviceKind::AlinkSender
    );
    let target = core.firmware_target();
    assert!(target.has_bootsel(), "{target:?}");
    assert_eq!(target.lineage, FirmwareLineage::Bofr, "{target:?}");
    assert_eq!(target.generation, FirmwareGeneration::Paged, "{target:?}");
    // An image not proven ours: never without the phrase the check names,
    // on the check and on the core's own write gate alike, and exactly as on
    // the guarded (legacy) sender. The sender has no voltage record, so the
    // voltage phrase is asked and its message names the lineage loss too.
    let legacy = {
        let dev = FakeDevice::sender(false);
        let serial = dev.descriptor().serial.clone();
        let core = DspiCore::new();
        core.open_transport(paged::open(Box::new(dev)).unwrap(), serial, 0)
            .unwrap();
        core.firmware_target()
    };
    assert_eq!(legacy.generation, FirmwareGeneration::Legacy);
    assert_eq!(
        (legacy.board_id, legacy.lineage),
        (target.board_id, target.lineage)
    );
    assert_eq!(legacy.voltage, target.voltage);
    let unknown = image("1.1.7", None);
    let phrase = firmware_update_check(unknown.clone(), &target, "")
        .verdict
        .confirmation_phrase;
    assert!(!phrase.is_empty());
    for typed in ["", "yes", LINEAGE_PHRASE, CONFIRMATION_PHRASE] {
        let v = firmware_update_check(unknown.clone(), &target, typed);
        assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss, "{typed:?}");
        assert_eq!(v.verdict.allowed, typed == phrase, "{typed:?}: {v:?}");
        assert_eq!(
            v.verdict,
            firmware_update_check(unknown.clone(), &legacy, typed).verdict,
            "{typed:?}"
        );
        let w = core.firmware_write_check(unknown.clone(), target.voltage.clone(), typed.into());
        assert_eq!(w.verdict.allowed, typed == phrase, "{typed:?}: {w:?}");
    }
    // The entry: its own target read, then the reboot, paged only.
    core.lock().flash_target = None;
    core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)))
        .unwrap();
    assert_eq!(remote.rebooted_to(), Some(RebootTarget::Bootsel));
    let read = core.lock().flash_target.clone().unwrap();
    assert_eq!(read.lineage, FirmwareLineage::Bofr, "{read:?}");
    assert_eq!(read.generation, FirmwareGeneration::Paged, "{read:?}");
    assert!(read.has_bootsel(), "{read:?}");
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}
