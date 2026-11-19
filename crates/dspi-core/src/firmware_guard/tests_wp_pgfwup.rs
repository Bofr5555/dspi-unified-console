//! WP PGFWUP (PLAN-PG section 1.8): the update check per generation and
//! board. A board without BOOTSEL (the DSPi-Zero, an unknown or unread board)
//! is offered no image whatever is typed; our firmware giving way to an image
//! that may be upstream's asks the typed confirmation, on top of the voltage
//! rule; a generation change is shown with what works afterwards; the target
//! is read the same on every fake generation.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use dspi_proto::packets::Board;
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{FakeDevice, Transport, paged};

use super::*;
use crate::{FirmwareGeneration, FirmwareLineage};

// ── Images (the layout the hub's rollout tests build) ───────────────────────

const XIP: u32 = 0x1000_0000;

/// An RP2350 DSPi image of `version`, stamped `stamp` mV (`None`: unstamped).
fn image(version: &str, stamp: Option<&str>) -> Vec<u8> {
    let mut entries: Vec<(u16, u32, String)> = vec![
        (BI_TAG_RP, BI_ID_PROGRAM_NAME, "DSPi".into()),
        (BI_TAG_RP, 0x11A9_BC3A, version.into()),
    ];
    if let Some(s) = stamp {
        entries.push((STAMP_TAG, STAMP_ID, s.into()));
    }
    let mut f = vec![0u8; 0x2000];
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, BI_MARKER_START);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, BI_MARKER_END);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&BI_TYPE_ID_AND_STRING.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let mut b = vec![0u8; 512];
        for (off, v) in [
            (0, 0x0A32_4655u32),
            (4, 0x9E5D_5157),
            (8, 0x2000),
            (12, XIP + 256 * i as u32),
            (16, 256),
            (20, i as u32),
            (24, pages.len() as u32),
            (28, 0xE48B_FF59),
            (508, 0x0AB1_6F30),
        ] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[32..32 + p.len()].copy_from_slice(p);
        out.extend(b);
    }
    out
}

fn target(mv: u16, lineage: FirmwareLineage, generation: FirmwareGeneration) -> FirmwareTarget {
    FirmwareTarget {
        voltage: FirmwareGuardBoard {
            required_mv: mv,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
        board: Some(Board::Pico),
        lineage,
        generation,
    }
}

fn bofr(mv: u16) -> FirmwareTarget {
    target(mv, FirmwareLineage::Bofr, FirmwareGeneration::Legacy)
}

/// A core on `dev`, opened as every construction site opens a device.
fn core_on(dev: FakeDevice) -> Arc<DspiCore> {
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).unwrap();
    let core = DspiCore::new();
    core.open_transport(t, serial, 0).unwrap();
    core
}

// ── The target, read ────────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_the_target_is_read_on_every_generation() {
    for g in FakeGeneration::ALL {
        let dev = FakeDevice::demo_generation(g);
        let remote = dev.remote();
        if g.has_extensions() {
            remote.vreg_set_build_default(0x0D);
        }
        let core = core_on(dev);
        let t = core.firmware_target();
        assert_eq!(t.board, Some(Board::Pico), "{g:?}");
        assert!(t.has_bootsel(), "{g:?}");
        let lineage = if g.has_extensions() {
            FirmwareLineage::Bofr
        } else {
            FirmwareLineage::Upstream
        };
        let generation = if g.is_paged() {
            FirmwareGeneration::Paged
        } else {
            FirmwareGeneration::Legacy
        };
        assert_eq!((t.lineage, t.generation), (lineage, generation), "{g:?}");
        // The voltage record through whichever envelope the device has (the
        // tunnel on the dual-stack port).
        if g.has_extensions() {
            assert_eq!(
                (t.voltage.required_mv, t.voltage.source),
                (1200, FirmwareGuardBoardSource::ExtCoreVreg),
                "{g:?}"
            );
        }
        assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    }
}

#[test]
fn wp_pgfwup_the_zero_is_a_target_without_bootsel() {
    let dev = FakeDevice::zero();
    let remote = dev.remote();
    let core = core_on(dev);
    let t = core.firmware_target();
    assert_eq!(t.board, Some(Board::PiZero2W));
    assert!(!t.has_bootsel());
    assert_eq!(remote.violations(), 0);
    // No device: nothing known, nothing offered.
    let t = DspiCore::new().firmware_target();
    assert_eq!(t.board, None);
    assert_eq!(t.voltage.source, FirmwareGuardBoardSource::NoDevice);
    assert!(!t.has_bootsel());
}

// ── No BOOTSEL: no image ────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_a_board_without_bootsel_is_offered_no_image() {
    let img = image("1.1.6-3-gabc", Some("1200"));
    let zero = FirmwareTarget {
        board: Some(Board::PiZero2W),
        ..bofr(1200)
    };
    for t in [
        zero.clone(),
        FirmwareTarget {
            board: Some(Board::Unknown(7)),
            ..bofr(1150)
        },
        FirmwareTarget {
            board: None,
            ..bofr(1150)
        },
        FirmwareTarget::default(),
    ] {
        for typed in ["", CONFIRMATION_PHRASE, LINEAGE_PHRASE, overvolt::WORD] {
            let v = firmware_update_check(img.clone(), &t, typed);
            assert!(!v.verdict.allowed, "{t:?} {typed}");
            assert_eq!(v.verdict.decision, FirmwareGuardDecision::Refused);
            assert_eq!(v.rule, FirmwareUpdateRule::NoBootsel);
            assert!(v.verdict.message.contains("BOOTSEL"), "{v:?}");
        }
    }
    let v = firmware_update_check(img.clone(), &zero, "");
    assert!(v.verdict.message.contains("DSPi Zero"), "{v:?}");
    // A board in BOOTSEL has one.
    let v = firmware_update_check(img, &FirmwareTarget::in_bootloader(), CONFIRMATION_PHRASE);
    assert_ne!(v.rule, FirmwareUpdateRule::NoBootsel);
    assert!(v.verdict.allowed, "{v:?}");
}

// ── bofr to upstream ────────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_bofr_to_upstream_asks_the_typed_confirmation() {
    let stock = image("1.1.6-1-g9ffce34", None);
    // A stock board would take it as before.
    let up = target(1150, FirmwareLineage::Upstream, FirmwareGeneration::Legacy);
    let v = firmware_update_check(stock.clone(), &up, "");
    assert_eq!(
        (v.verdict.decision, v.rule),
        (FirmwareGuardDecision::Clear, FirmwareUpdateRule::None)
    );
    // Our firmware at 1.15 V: the voltage rule would let it through, the
    // lineage rule asks.
    let v = firmware_update_check(stock.clone(), &bofr(1150), "");
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::NeedsConfirmation);
    assert!(!v.verdict.allowed);
    assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
    assert_eq!(v.verdict.confirmation_phrase, LINEAGE_PHRASE);
    assert_eq!(v.image_lineage, FirmwareLineage::Unknown);
    for word in ["extension channel", "1.20 V", "Control Surfaces"] {
        assert!(v.verdict.message.contains(word), "{word}: {v:?}");
    }
    // The other phrase does not stand for it.
    let v = firmware_update_check(stock.clone(), &bofr(1150), CONFIRMATION_PHRASE);
    assert!(!v.verdict.allowed);
    let v = firmware_update_check(stock.clone(), &bofr(1150), LINEAGE_PHRASE);
    assert!(v.verdict.allowed);
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::Confirmed);
    // At risk (the K340 at 1.20 V): the voltage rule asks already; its phrase
    // covers both, and the message names both.
    let v = firmware_update_check(stock.clone(), &bofr(1200), "");
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::NeedsConfirmation);
    assert_eq!(v.verdict.confirmation_phrase, CONFIRMATION_PHRASE);
    assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
    assert!(v.verdict.message.contains("It can hang"), "{v:?}");
    assert!(v.verdict.message.contains("extension channel"), "{v:?}");
    assert!(
        !firmware_update_check(stock.clone(), &bofr(1200), LINEAGE_PHRASE)
            .verdict
            .allowed
    );
    assert!(
        firmware_update_check(stock, &bofr(1200), CONFIRMATION_PHRASE)
            .verdict
            .allowed
    );
    // An image proven ours (our core-voltage stamp) does not ask.
    let ours = image("1.1.6-30-gdeadbee", Some("1200"));
    let v = firmware_update_check(ours, &bofr(1200), "");
    assert_eq!(
        (v.verdict.decision, v.rule, v.image_lineage),
        (
            FirmwareGuardDecision::Clear,
            FirmwareUpdateRule::None,
            FirmwareLineage::Bofr
        )
    );
    // An unknown lineage on the board is the voltage rule's to judge.
    let unknown_board = target(1150, FirmwareLineage::Unknown, FirmwareGeneration::Unknown);
    let v = firmware_update_check(image("1.1.6", None), &unknown_board, "");
    assert_eq!(v.rule, FirmwareUpdateRule::None);
    // A file that is not firmware stays refused.
    let v = firmware_update_check(vec![0; 64], &bofr(1150), LINEAGE_PHRASE);
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::Refused);
    assert_eq!(v.rule, FirmwareUpdateRule::None);
}

// ── A generation change ─────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_a_generation_change_is_shown_with_what_works_afterwards() {
    let to_paged = firmware_update_check(image("1.1.7-beta1", Some("1200")), &bofr(1200), "");
    assert!(to_paged.verdict.allowed);
    assert_eq!(to_paged.image_generation, FirmwareGeneration::Paged);
    assert!(
        to_paged.generation_change.contains("DSPi Link 1.2 hub"),
        "{to_paged:?}"
    );
    let paged = target(1200, FirmwareLineage::Bofr, FirmwareGeneration::Paged);
    let back = firmware_update_check(image("1.1.6-40-gabc", Some("1200")), &paged, "");
    assert!(
        back.generation_change.contains("DSPi Link 1.3 hub"),
        "{back:?}"
    );
    let same = firmware_update_check(image("1.1.6-41-gabc", Some("1200")), &bofr(1200), "");
    assert_eq!(same.generation_change, "");
    let unknown = firmware_update_check(
        image("1.1.7", Some("1200")),
        &FirmwareTarget::in_bootloader(),
        "",
    );
    assert_eq!(unknown.generation_change, "");
}
