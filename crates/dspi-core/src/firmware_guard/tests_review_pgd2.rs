//! Batch review PG-D part 2: `firmware_write_check`, the write gate's verdict
//! for a client whose write is its own (Android). It is the gate itself: the
//! core's own target decides, a target the client kept from another board
//! cannot stand in for it, and the K340's 1.20 V rule holds.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use dspi_transport::fake::FakeGeneration;
use dspi_transport::{FakeDevice, Transport, paged};

use super::tests_wp_pgfwup::image;
use super::*;
use crate::{ConfirmOp, FirmwareLineage};

fn open(core: &DspiCore, dev: FakeDevice) {
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).unwrap();
    core.open_transport(t, serial, 0).unwrap();
}

fn core_on(dev: FakeDevice) -> Arc<DspiCore> {
    let core = DspiCore::new();
    open(&core, dev);
    core
}

fn check(
    core: &DspiCore,
    image: &[u8],
    board: &FirmwareGuardBoard,
    typed: &str,
) -> FirmwareUpdateVerdict {
    core.firmware_write_check(image.to_vec(), board.clone(), typed.into())
}

/// A Pico on our firmware at the stock 1.15 V (the voltage rule alone clears
/// it) and an image that may be upstream's: the write check asks the lineage
/// phrase, takes only that, and answers the same when asked again.
#[test]
fn review_pgd2_the_write_check_asks_the_lineage_phrase_on_the_cores_own_target() {
    let dev = FakeDevice::demo_generation(FakeGeneration::Bofr116 { page_guard: true });
    dev.remote().vreg_set_build_default(0x0C);
    let core = core_on(dev);
    let unstamped = image("1.1.7", None);
    let read = core.firmware_target();
    assert_eq!(read.lineage, FirmwareLineage::Bofr);
    assert_eq!(read.voltage.required_mv, 1150);
    for _ in 0..2 {
        let v = check(&core, &unstamped, &read.voltage, "");
        assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
        assert!(!v.verdict.allowed);
        assert_eq!(v.verdict.confirmation_phrase, LINEAGE_PHRASE);
    }
    assert!(
        !check(&core, &unstamped, &read.voltage, "yes")
            .verdict
            .allowed
    );
    assert!(
        check(&core, &unstamped, &read.voltage, LINEAGE_PHRASE)
            .verdict
            .allowed
    );
    // It is the update check on that target, and the write's own answer
    assert_eq!(
        check(&core, &unstamped, &read.voltage, ""),
        firmware_update_check(unstamped.clone(), &read, "")
    );
    // The board gone to its bootloader: the target is kept for the write
    core.disconnect();
    assert_eq!(
        check(&core, &unstamped, &read.voltage, "").rule,
        FirmwareUpdateRule::LineageLoss
    );
    // A client that kept no record (a board in BOOTSEL nobody asked): never less
    let v = check(&core, &unstamped, &firmware_guard_board_in_bootloader(), "");
    assert!(!v.verdict.allowed);
    assert!(
        !check(
            &core,
            &unstamped,
            &firmware_guard_board_in_bootloader(),
            LINEAGE_PHRASE
        )
        .verdict
        .allowed,
        "an unknown voltage asks the voltage phrase, which the lineage phrase is not"
    );
}

/// The client judged a stock board (clear, nothing asked) and kept its
/// record; then another board, a K340-like one on our firmware at 1.20 V, is
/// the one this core opens and sends to its bootloader. A check on the
/// client's kept target would clear a stock image for it. The core's check
/// does not: its own target is the K340's.
#[test]
fn review_pgd2_a_target_the_client_kept_from_another_board_does_not_clear_a_k340() {
    let pico = FakeDevice::demo_generation(FakeGeneration::Stock116);
    pico.remote().vreg_set_build_default(0x0C); // 1.15 V
    let core = core_on(pico);
    let stock = image("1.1.6", None);
    let a = core.firmware_target();
    assert_eq!(a.lineage, FirmwareLineage::Upstream);
    assert_eq!(a.voltage.required_mv, 1150);
    // What the client's own check on its kept target says, then and later
    let kept = firmware_update_check(stock.clone(), &a, "");
    assert!(kept.verdict.allowed, "{kept:?}");
    assert!(check(&core, &stock, &a.voltage, "").verdict.allowed);

    // The demo's own core voltage is 1.20 V, as a K340's
    let k340 = FakeDevice::demo_generation(FakeGeneration::Bofr116 { page_guard: true });
    open(&core, k340);
    assert!(core.lock().flash_target.is_none(), "dropped at the open");
    core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)))
        .unwrap();
    let own = core.lock().flash_target.clone().unwrap();
    assert_eq!(own.voltage.required_mv, 1200);
    for typed in ["", LINEAGE_PHRASE, dspi_session::overvolt::WORD, "yes"] {
        // Board A's record, the K340's own, and none at all
        for board in [
            a.voltage.clone(),
            own.voltage.clone(),
            firmware_guard_board_in_bootloader(),
        ] {
            let v = check(&core, &stock, &board, typed);
            assert!(!v.verdict.allowed, "{board:?} {typed:?}: {v:?}");
            assert_eq!(v.verdict.confirmation_phrase, CONFIRMATION_PHRASE);
        }
    }
    // Only the voltage phrase, on the K340's own record or on doubt
    assert!(
        check(&core, &stock, &own.voltage, CONFIRMATION_PHRASE)
            .verdict
            .allowed
    );
    // Our stamped 1.20 V image is clear on its own record
    let ours = image("1.1.6", Some("1200"));
    assert!(check(&core, &ours, &own.voltage, "").verdict.allowed);
}

/// A board without BOOTSEL is refused whatever is typed and whatever record
/// is passed; a core that read nothing judges by the voltage rule alone, so
/// a board in BOOTSEL nobody asked is asked the voltage phrase.
#[test]
fn review_pgd2_the_write_check_refuses_a_zero_and_asks_for_a_board_nobody_read() {
    let core = core_on(FakeDevice::zero());
    let read = core.firmware_target();
    assert!(!read.has_bootsel());
    let stock = image("1.1.6", None);
    for typed in ["", LINEAGE_PHRASE, CONFIRMATION_PHRASE] {
        for board in [read.voltage.clone(), firmware_guard_board_in_bootloader()] {
            let v = check(&core, &stock, &board, typed);
            assert_eq!(v.rule, FirmwareUpdateRule::NoBootsel);
            assert!(!v.verdict.allowed);
        }
    }

    let fresh = DspiCore::new();
    let hand = firmware_guard_board_in_bootloader();
    for img in [stock, image("1.1.6", Some("1200"))] {
        let v = check(&fresh, &img, &hand, "");
        assert_eq!(
            v.verdict,
            firmware_guard_check(img.clone(), hand.clone(), String::new())
        );
        assert!(!v.verdict.allowed);
        assert_eq!(v.verdict.confirmation_phrase, CONFIRMATION_PHRASE);
        assert!(!check(&fresh, &img, &hand, LINEAGE_PHRASE).verdict.allowed);
        assert!(
            check(&fresh, &img, &hand, CONFIRMATION_PHRASE)
                .verdict
                .allowed
        );
    }
}
