//! WP IMGKIND: the device-kind stamp. Every DSPi image carries `dsp` and the
//! alink sender's carries `alink_sender` (firmware `image_kind.h`, binary
//! info tag `DS`, id [`KIND_ID`]); the update check and the write gate refuse
//! a stamped image whose kind is not the target's, whatever is typed. An image
//! without the stamp is judged as before, except that on a sender it asks the
//! typed confirmation with words saying it may be a DSP image.
//!
//! The fixtures are the K340 build and the sender build of firmware
//! `wp/IMGKIND` `6bffc36`, trimmed as FWSTAMP's (every UF2 block kept, the
//! payload of every flash page the binary-info walk does not read zeroed):
//! - `DSPi-k340-6bffc36-trimmed.uf2`: `-DDSPI_USB_MIDI=ON -DDSPI_VREG_MV=1200`,
//!   UF2 sha256 `3272f91f…de81bd07`, kind `dsp`, voltage stamp `1200`;
//! - `dspi_sender-6bffc36-trimmed.uf2`: `-DDSPI_ROLE_SENDER=ON`, UF2 sha256
//!   `0eb7068b…1dabc72b`, kind `alink_sender`, no voltage stamp (the sender
//!   has no `core_vreg.c`), program name `DSPi-Sender`, copy-to-RAM.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use dspi_proto::packets::Board;
use dspi_transport::{FakeDevice, Transport, paged};

use super::tests_wp_pgfwup::image;
use super::*;
use crate::{DeviceKind, FirmwareGeneration, FirmwareLineage};

const K340: &[u8] = include_bytes!("../../testdata/imgkind/DSPi-k340-6bffc36-trimmed.uf2");
const SENDER: &[u8] = include_bytes!("../../testdata/imgkind/dspi_sender-6bffc36-trimmed.uf2");

/// Every phrase and word the guard knows, and none.
const TYPED: [&str; 5] = [
    "",
    CONFIRMATION_PHRASE,
    LINEAGE_PHRASE,
    KIND_PHRASE,
    overvolt::WORD,
];

fn upstream_rp2350() -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    std::fs::read(root.join("desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2")).unwrap()
}

fn target(
    mv: u16,
    source: FirmwareGuardBoardSource,
    lineage: FirmwareLineage,
    kind: Option<DeviceKind>,
) -> FirmwareTarget {
    FirmwareTarget {
        voltage: FirmwareGuardBoard {
            required_mv: mv,
            source,
        },
        board_id: Some(Board::Pico.id()),
        lineage,
        generation: FirmwareGeneration::Legacy,
        device_kind: kind,
    }
}

/// Our K340: bofr firmware read at 1.20 V.
fn k340() -> FirmwareTarget {
    target(
        1200,
        FirmwareGuardBoardSource::ExtCoreVreg,
        FirmwareLineage::Bofr,
        Some(DeviceKind::Dsp),
    )
}

/// A stock DSPi (upstream firmware, 1.15 V).
fn stock_dsp() -> FirmwareTarget {
    target(
        1150,
        FirmwareGuardBoardSource::StatusBlock,
        FirmwareLineage::Upstream,
        Some(DeviceKind::Dsp),
    )
}

/// A sender as the core reads it: ours, kind `AlinkSender`, the voltage
/// given.
fn sender(mv: u16) -> FirmwareTarget {
    target(
        mv,
        FirmwareGuardBoardSource::StatusBlock,
        FirmwareLineage::Bofr,
        Some(DeviceKind::AlinkSender),
    )
}

/// A core open on the fake dual-stack sender, and the target it read.
fn sender_core() -> (Arc<DspiCore>, FirmwareTarget) {
    let dev = FakeDevice::sender(true);
    let serial = dev.descriptor().serial.clone();
    let core = DspiCore::new();
    core.open_transport(paged::open(Box::new(dev)).unwrap(), serial, 0)
        .unwrap();
    let t = core.firmware_target();
    (core, t)
}

/// `img` with the one occurrence of `from` replaced by `to` (same length).
fn patched(img: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at: Vec<usize> = img
        .windows(from.len())
        .enumerate()
        .filter(|(_, w)| *w == from)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(at.len(), 1, "{from:?} occurs once");
    let mut out = img.to_vec();
    out[at[0]..at[0] + to.len()].copy_from_slice(to);
    out
}

fn assert_refused(v: &FirmwareUpdateVerdict, typed: &str) {
    assert!(!v.verdict.allowed, "{typed:?}: {v:?}");
    assert_eq!(v.rule, FirmwareUpdateRule::KindMismatch, "{typed:?}: {v:?}");
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::Refused);
    assert!(v.verdict.confirmation_phrase.is_empty(), "{v:?}");
    assert!(
        matches!(
            write_gate_error(v.clone()),
            Some(DspiError::InvalidArgument { .. })
        ),
        "{v:?}"
    );
}

#[test]
fn wp_imgkind_the_real_images_carry_their_kind() {
    let info = uf2_inspect(K340.to_vec());
    assert!(info.valid, "{}", info.problem);
    assert_eq!(info.firmware_version, "1.1.6-103-g6bffc36");
    assert_eq!(image_kind(K340, &info), ImageKind::Stamped(DeviceKind::Dsp));
    assert_eq!(image_stamp(K340, &info), Some(1200));
    assert_eq!(image_facts(K340, &info).0, FirmwareLineage::Bofr);

    let info = uf2_inspect(SENDER.to_vec());
    assert!(info.valid, "{}", info.problem);
    assert_eq!(
        image_kind(SENDER, &info),
        ImageKind::Stamped(DeviceKind::AlinkSender)
    );
    // No core_vreg.c, so no voltage stamp; the kind stamp alone proves it ours.
    assert_eq!(image_stamp(SENDER, &info), None);
    assert_eq!(image_facts(SENDER, &info).0, FirmwareLineage::Bofr);

    let up = upstream_rp2350();
    let info = uf2_inspect(up.clone());
    assert_eq!(image_kind(&up, &info), ImageKind::None);
}

#[test]
fn wp_imgkind_dsp_image_on_a_dsp_and_sender_image_on_a_sender_are_not_refused() {
    // Our K340 image on our K340: clear, as FWSTAMP's.
    for typed in ["", "anything"] {
        let v = firmware_update_check(K340.to_vec(), &k340(), typed);
        assert!(v.verdict.allowed, "{v:?}");
        assert_eq!(v.verdict.decision, FirmwareGuardDecision::Clear);
        assert_eq!(v.rule, FirmwareUpdateRule::None);
        assert!(v.verdict.confirmation_phrase.is_empty());
    }
    // The sender image on a sender at the stock voltage: clear, no words.
    let v = firmware_update_check(SENDER.to_vec(), &sender(1150), "");
    assert!(v.verdict.allowed, "{v:?}");
    assert_eq!(v.rule, FirmwareUpdateRule::None);
    assert_eq!(v.image_lineage, FirmwareLineage::Bofr);
    assert!(v.verdict.message.is_empty(), "{v:?}");
    // On the sender as the core reads it (the fake dual-stack sender: no
    // voltage record): -- INT-R1 -- the sender's own stamped image is clear
    // there too, whatever is typed. The sender has no core-voltage setting,
    // so the voltage rule's "needs not known" has nothing to protect; the
    // kind rule adds nothing and the lineage rule nothing.
    let (core, t) = sender_core();
    assert_eq!(t.device_kind, Some(DeviceKind::AlinkSender));
    assert_eq!(t.voltage.required_mv, 0, "no voltage record: {t:?}");
    for typed in TYPED {
        let v = firmware_update_check(SENDER.to_vec(), &t, typed);
        assert!(v.verdict.allowed, "{typed:?}: {v:?}");
        assert_eq!(v.verdict.decision, FirmwareGuardDecision::Clear, "{v:?}");
        assert_eq!(v.rule, FirmwareUpdateRule::None, "{v:?}");
        assert!(v.verdict.confirmation_phrase.is_empty(), "{v:?}");
        assert!(v.verdict.message.is_empty(), "{v:?}");
        let w = core.firmware_write_check(SENDER.to_vec(), t.voltage.clone(), typed.into());
        assert!(w.verdict.allowed, "{typed:?}: {w:?}");
    }
    // Only that image: a sender image whose kind stamp is gone (an unstamped
    // image) still asks the voltage phrase there (the test below), and the
    // DSP image is refused (the test below that).
}

#[test]
fn wp_imgkind_each_image_on_the_other_kind_is_refused_whatever_is_typed() {
    let (core, read) = sender_core();
    for typed in TYPED {
        // The DSP image on a sender, at any voltage the sender reports.
        for t in [sender(1150), sender(1200), sender(0), read.clone()] {
            let v = firmware_update_check(K340.to_vec(), &t, typed);
            assert_refused(&v, typed);
            assert!(v.verdict.message.contains("for a DSPi"), "{v:?}");
        }
        // ... and on the core's own write gate for the sender it read.
        let w = core.firmware_write_check(K340.to_vec(), read.voltage.clone(), typed.into());
        assert_refused(&w, typed);
        // The sender image on a DSP, ours or stock.
        for t in [k340(), stock_dsp()] {
            let v = firmware_update_check(SENDER.to_vec(), &t, typed);
            assert_refused(&v, typed);
            assert!(v.verdict.message.contains("for an alink sender"), "{v:?}");
        }
    }
    // A target whose kind is not known (a board in BOOTSEL): no kind rule;
    // the voltage rule asks as before.
    let v = firmware_update_check(SENDER.to_vec(), &FirmwareTarget::in_bootloader(), "");
    assert_eq!(v.rule, FirmwareUpdateRule::None, "{v:?}");
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::NeedsConfirmation);
}

#[test]
fn wp_imgkind_an_unstamped_upstream_image_on_a_dsp_is_judged_as_before() {
    let up = upstream_rp2350();
    for base in [k340(), stock_dsp()] {
        let before = FirmwareTarget {
            device_kind: None,
            ..base.clone()
        };
        for typed in TYPED {
            assert_eq!(
                firmware_update_check(up.clone(), &base, typed),
                firmware_update_check(up.clone(), &before, typed),
                "{typed:?}"
            );
        }
    }
    // Which is: on our K340 the lineage (and voltage) phrase; on a stock DSPi clear.
    let v = firmware_update_check(up.clone(), &k340(), "");
    assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
    assert!(!v.verdict.allowed);
    assert!(!v.verdict.message.contains("DSP image"), "{v:?}");
    let v = firmware_update_check(up, &stock_dsp(), "");
    assert_eq!(
        (v.rule, v.verdict.decision),
        (FirmwareUpdateRule::None, FirmwareGuardDecision::Clear)
    );
}

#[test]
fn wp_imgkind_an_unstamped_image_on_a_sender_asks_and_says_it_may_be_a_dsp_image() {
    // Ours before IMGKIND (a voltage stamp, no kind stamp) on a sender at the
    // stock voltage: before, clear; now the kind phrase and only it.
    let old_ours = image("1.1.6", Some("1150"));
    for typed in TYPED {
        let v = firmware_update_check(old_ours.clone(), &sender(1150), typed);
        assert_eq!(v.rule, FirmwareUpdateRule::KindUnknown, "{v:?}");
        assert_eq!(v.verdict.confirmation_phrase, KIND_PHRASE);
        assert_eq!(v.verdict.allowed, typed == KIND_PHRASE, "{typed:?}: {v:?}");
        assert!(v.verdict.message.contains("may be a DSP image"), "{v:?}");
    }
    // On the sender as the core reads it: the voltage phrase is asked anyway,
    // and its message names the kind risk too.
    let (core, read) = sender_core();
    for img in [old_ours, upstream_rp2350(), image("1.1.7", None)] {
        let v = firmware_update_check(img.clone(), &read, "");
        assert!(!v.verdict.allowed, "{v:?}");
        assert_eq!(v.verdict.confirmation_phrase, CONFIRMATION_PHRASE, "{v:?}");
        assert!(v.verdict.message.contains("may be a DSP image"), "{v:?}");
        let w = core.firmware_write_check(img, read.voltage.clone(), String::new());
        assert!(!w.verdict.allowed, "{w:?}");
        assert!(w.verdict.message.contains("may be a DSP image"), "{w:?}");
    }
    // An upstream image on a sender: the lineage rule keeps its name and
    // phrase, and the message names both risks.
    for typed in TYPED {
        let v = firmware_update_check(upstream_rp2350(), &sender(1150), typed);
        assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss, "{v:?}");
        assert_eq!(v.verdict.confirmation_phrase, LINEAGE_PHRASE);
        assert_eq!(v.verdict.allowed, typed == LINEAGE_PHRASE, "{typed:?}");
        assert!(v.verdict.message.contains("may be a DSP image"), "{v:?}");
        assert!(v.verdict.message.contains("upstream's"), "{v:?}");
    }
}

#[test]
fn wp_imgkind_a_kind_stamp_that_cannot_be_read_is_refused_everywhere() {
    // Another text, and two kind stamps that disagree (the voltage stamp's
    // id turned into the kind id: kinds `dsp` and `1200`).
    let other = patched(K340, b"dsp\0", b"dsx\0");
    let two = patched(K340, &STAMP_ID.to_le_bytes(), &KIND_ID.to_le_bytes());
    for img in [other, two] {
        let info = uf2_inspect(img.clone());
        assert!(info.valid, "{}", info.problem);
        assert_eq!(image_kind(&img, &info), ImageKind::Broken);
        let unknown_kind = FirmwareTarget {
            device_kind: None,
            ..k340()
        };
        for t in [k340(), stock_dsp(), sender(1150), unknown_kind] {
            for typed in TYPED {
                assert_refused(&firmware_update_check(img.clone(), &t, typed), typed);
            }
        }
    }
}

#[test]
fn wp_imgkind_the_targets_kind_survives_json_and_is_omitted_when_unknown() {
    let t = sender(1150);
    let json = serde_json::to_value(&t).unwrap();
    assert_eq!(json["deviceKind"], "alinkSender");
    assert_eq!(serde_json::from_value::<FirmwareTarget>(json).unwrap(), t);
    let json = serde_json::to_value(FirmwareTarget::in_bootloader()).unwrap();
    assert!(json.get("deviceKind").is_none(), "{json}");
}
