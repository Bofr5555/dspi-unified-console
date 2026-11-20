//! WP FWSTAMP (owner 2026-11-19): our own firmware images carry the
//! core-voltage stamp (`core_vreg.c`, bofr-main from `c6d82b8`), so the update
//! check judges a real bofr build as ours, at its build's voltage, without the
//! lineage phrase or the voltage words. An image without the stamp, with
//! another text in it, or upstream's, is judged as before (fail closed).
//!
//! The fixture is the K340 build of `wp/FWSTAMP` `c6d82b8`
//! (`-DDSPI_USB_MIDI=ON -DDSPI_VREG_MV=1200`, UF2 sha256 `4b1ff2bc…31fe6d`)
//! with every UF2 block kept and the payload of every flash page the
//! binary-info walk does not read zeroed (22 of 1530 pages kept), so the real
//! SDK layout (the copy-to-RAM mapping, the E10 block, the entry order) is
//! what the parser walks, at 15 KB compressed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::packets::Board;

use super::*;
use crate::{FirmwareGeneration, FirmwareLineage};

const K340: &[u8] = include_bytes!("../../testdata/fwstamp/DSPi-k340-c6d82b8-vreg1200-trimmed.uf2");

fn target(mv: u16, lineage: FirmwareLineage) -> FirmwareTarget {
    FirmwareTarget {
        voltage: FirmwareGuardBoard {
            required_mv: mv,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
        board_id: Some(Board::Pico.id()),
        lineage,
        generation: FirmwareGeneration::Legacy,
        device_kind: None,
    }
}

/// The fixture with its stamp's text replaced (it occurs once).
fn restamped(text: &[u8; 4]) -> Vec<u8> {
    let mut img = K340.to_vec();
    let at: Vec<usize> = img
        .windows(5)
        .enumerate()
        .filter(|(_, w)| *w == b"1200\0")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(at.len(), 1, "the stamp text occurs once");
    img[at[0]..at[0] + 4].copy_from_slice(text);
    img
}

#[test]
fn wp_fwstamp_our_k340_image_is_stamped_1200() {
    let info = uf2_inspect(K340.to_vec());
    assert!(info.valid, "{}", info.problem);
    assert_eq!(info.firmware_version, "1.1.6-102-gc6d82b8");
    assert_eq!(image_stamp(K340, &info), Some(1200));
    assert_eq!(
        image_facts(K340, &info),
        (FirmwareLineage::Bofr, FirmwareGeneration::Legacy)
    );
}

#[test]
fn wp_fwstamp_our_image_on_our_k340_needs_no_words() {
    for typed in ["", "anything"] {
        let v = firmware_update_check(K340.to_vec(), &target(1200, FirmwareLineage::Bofr), typed);
        assert!(v.verdict.allowed, "{v:?}");
        assert_eq!(v.verdict.decision, FirmwareGuardDecision::Clear);
        assert_eq!(v.verdict.reason, FirmwareGuardReason::NoRisk);
        assert_eq!(v.rule, FirmwareUpdateRule::None);
        assert_eq!(v.image_lineage, FirmwareLineage::Bofr);
        assert_eq!((v.verdict.image_mv, v.verdict.required_mv), (1200, 1200));
        assert!(v.verdict.confirmation_phrase.is_empty());
        assert!(v.generation_change.is_empty());
    }
    // A bofr board at the stock voltage, and an upstream one: clear too (a
    // 1.20 V image on a 1.15 V board asks nothing).
    for t in [
        target(1150, FirmwareLineage::Bofr),
        target(1150, FirmwareLineage::Upstream),
    ] {
        let v = firmware_update_check(K340.to_vec(), &t, "");
        assert!(v.verdict.allowed, "{t:?} {v:?}");
        assert_eq!(v.rule, FirmwareUpdateRule::None);
        assert_eq!(v.verdict.image_mv, 1200);
    }
    // A board that auto-raised itself to 1.25 V: the 1.20 V image is lower.
    let v = firmware_update_check(K340.to_vec(), &target(1250, FirmwareLineage::Bofr), "");
    assert!(!v.verdict.allowed);
    assert_eq!(v.verdict.reason, FirmwareGuardReason::ImageLower);
    assert_eq!(v.image_lineage, FirmwareLineage::Bofr);
    assert_eq!(v.rule, FirmwareUpdateRule::None);
}

#[test]
fn wp_fwstamp_an_image_without_a_clean_stamp_is_judged_as_before() {
    let ours = target(1200, FirmwareLineage::Bofr);
    // Another text: no stamp, so unknown lineage and 1.15 V: both rules ask,
    // the voltage phrase covers both.
    for text in [b"12x0", b"1201", b"0000"] {
        let img = restamped(text);
        let info = uf2_inspect(img.clone());
        assert!(info.valid);
        assert_eq!(image_stamp(&img, &info), None, "{text:?}");
        let v = firmware_update_check(img.clone(), &ours, "");
        assert!(!v.verdict.allowed, "{text:?} {v:?}");
        assert_eq!(v.image_lineage, FirmwareLineage::Unknown);
        assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
        assert_eq!(v.verdict.reason, FirmwareGuardReason::ImageUnknown);
        assert_eq!(v.verdict.confirmation_phrase, CONFIRMATION_PHRASE);
        assert!(
            firmware_update_check(img, &ours, CONFIRMATION_PHRASE)
                .verdict
                .allowed
        );
    }
    // A clean stamp of a lower voltage: ours, but lower than the board.
    let img = restamped(b"1150");
    let v = firmware_update_check(img, &ours, "");
    assert!(!v.verdict.allowed);
    assert_eq!(v.image_lineage, FirmwareLineage::Bofr);
    assert_eq!(v.verdict.reason, FirmwareGuardReason::ImageLower);
    assert_eq!(v.verdict.image_mv, 1150);
}

#[test]
fn wp_fwstamp_upstream_and_unknown_boards_still_ask() {
    // The bundled upstream image on our K340: the lineage loss and the voltage.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    let stock = std::fs::read(root.join("desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2")).unwrap();
    let v = firmware_update_check(stock, &target(1200, FirmwareLineage::Bofr), "");
    assert!(!v.verdict.allowed, "{v:?}");
    assert_eq!(v.image_lineage, FirmwareLineage::Upstream);
    assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
    assert_eq!(v.verdict.reason, FirmwareGuardReason::ImageUnknown);
    // Our stamped image on a board already in BOOTSEL (not read): its voltage
    // needs are unknown, so the phrase is still asked (no lineage rule there).
    let v = firmware_update_check(K340.to_vec(), &FirmwareTarget::in_bootloader(), "");
    assert!(!v.verdict.allowed, "{v:?}");
    assert_eq!(v.verdict.reason, FirmwareGuardReason::BoardUnknown);
    assert_eq!(v.rule, FirmwareUpdateRule::None);
    assert_eq!(v.image_lineage, FirmwareLineage::Bofr);
    let v = firmware_update_check(
        K340.to_vec(),
        &FirmwareTarget::in_bootloader(),
        CONFIRMATION_PHRASE,
    );
    assert!(v.verdict.allowed, "{v:?}");
}
