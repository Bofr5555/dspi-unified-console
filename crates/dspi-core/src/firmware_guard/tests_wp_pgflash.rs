//! WP PGFLASH: the core's write gate (`flash_confirmed`,
//! `picoboot_flash_confirmed`) applies the update check's rules to the
//! target the core read itself from the device: no BOOTSEL is refused, the
//! lineage rule and the voltage rule each need their typed phrase, even when
//! the client skipped the update check; a caller's voltage record only adds
//! doubt. With no target read (a board in BOOTSEL before the core saw it),
//! the voltage rule alone decides, as before.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::path::PathBuf;
use std::sync::Arc;

use dspi_transport::fake::FakeGeneration;
use dspi_transport::{Capture, FakeBoard, FakeDevice, FakeOptions, FakeRemote, Transport, paged};

use super::tests_wp_pgfwup::image;
use super::*;
use crate::picoboot::{BootDevice, prepare_gated};
use crate::{ConfirmOp, FirmwareLineage};

/// A core on `dev`, opened as every construction site opens a device.
fn core_on(dev: FakeDevice) -> Arc<DspiCore> {
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).unwrap();
    let core = DspiCore::new();
    core.open_transport(t, serial, 0).unwrap();
    core
}

/// A fake bootloader drive beside the test binary (never a tmpfs).
fn drive(name: &str) -> (PathBuf, BootloaderDrive) {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("wp-pgflash-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("INFO_UF2.TXT"),
        "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n",
    )
    .unwrap();
    let d = BootloaderDrive {
        path: dir.to_string_lossy().into_owned(),
        platform: 1,
        info: String::new(),
    };
    (dir, d)
}

fn stock_board() -> FirmwareGuardBoard {
    FirmwareGuardBoard {
        required_mv: 1150,
        source: FirmwareGuardBoardSource::ExtCoreVreg,
    }
}

fn flash(
    core: &DspiCore,
    image: &[u8],
    d: &BootloaderDrive,
    board: FirmwareGuardBoard,
    typed: &str,
) -> crate::Result<()> {
    let token = core.confirm_token(ConfirmOp::FirmwareFlash);
    core.flash_confirmed(image.to_vec(), d.clone(), board, typed.into(), Some(token))
}

fn enter(core: &DspiCore) -> crate::Result<()> {
    core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)))
}

/// Our firmware of generation `g` at the stock 1.15 V (VREG code 0x0C), so
/// the voltage rule is clear and only the lineage rule can ask.
fn ours_at_stock(g: FakeGeneration) -> FakeDevice {
    let dev = FakeDevice::demo_generation(g);
    dev.remote().vreg_set_build_default(0x0C);
    dev
}

fn no_violation(remote: &FakeRemote) {
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}

/// A pipe that cannot exist: the refusals come before any is opened.
enum NeverPipe {}

impl crate::picoboot::BootPipe for NeverPipe {
    fn bulk_out(
        &mut self,
        _: &[u8],
        _: std::time::Duration,
    ) -> std::result::Result<(), crate::picoboot::PipeError> {
        match *self {}
    }
    fn bulk_in(
        &mut self,
        _: usize,
        _: std::time::Duration,
    ) -> std::result::Result<Vec<u8>, crate::picoboot::PipeError> {
        match *self {}
    }
    fn control_in(
        &mut self,
        _: u8,
        _: u16,
        _: std::time::Duration,
    ) -> std::result::Result<Vec<u8>, crate::picoboot::PipeError> {
        match *self {}
    }
    fn control_out(
        &mut self,
        _: u8,
        _: std::time::Duration,
    ) -> std::result::Result<(), crate::picoboot::PipeError> {
        match *self {}
    }
    fn clear_halts(&mut self) -> std::result::Result<(), crate::picoboot::PipeError> {
        match *self {}
    }
}

const BOFR: [FakeGeneration; 3] = [
    FakeGeneration::Bofr116 { page_guard: false },
    FakeGeneration::Bofr116 { page_guard: true },
    FakeGeneration::Bofr117,
];

#[test]
fn wp_pgflash_an_unstamped_image_on_our_firmware_needs_the_phrase_at_the_write_gate() {
    for (i, g) in BOFR.into_iter().enumerate() {
        let dev = ours_at_stock(g);
        let remote = dev.remote();
        let core = core_on(dev);
        // The client skips the update check: only the bootloader entry, which
        // reads the target on its own.
        assert!(core.lock().flash_target.is_none());
        enter(&core).unwrap();
        let read = core.lock().flash_target.clone().unwrap();
        assert_eq!(read.lineage, FirmwareLineage::Bofr, "{g:?}");
        assert_eq!(
            read.voltage.required_mv, 1150,
            "{g:?}: the voltage rule is clear"
        );
        core.disconnect();
        let (dir, d) = drive(&format!("lineage-{i}"));
        let unstamped = image("1.1.6", None);
        // The voltage rule alone would let this through with nothing typed.
        assert!(firmware_guard_check(unstamped.clone(), stock_board(), String::new()).allowed);
        for typed in ["", CONFIRMATION_PHRASE, "yes"] {
            let r = flash(&core, &unstamped, &d, stock_board(), typed);
            assert!(
                matches!(&r, Err(DspiError::FirmwareGuardBlocked { msg }) if msg.contains("extension")),
                "{g:?} {typed:?}: {r:?}"
            );
            assert!(!dir.join("firmware.uf2").exists(), "{g:?}");
        }
        // The lineage phrase lets it through.
        flash(&core, &unstamped, &d, stock_board(), LINEAGE_PHRASE).unwrap();
        assert_eq!(std::fs::read(dir.join("firmware.uf2")).unwrap(), unstamped);
        // The target is forgotten after the write.
        assert!(core.lock().flash_target.is_none(), "{g:?}");
        std::fs::remove_dir_all(dir).unwrap();
        no_violation(&remote);
    }
}

#[test]
fn wp_pgflash_our_stamped_image_needs_nothing_and_stock_firmware_loses_nothing() {
    // Ours (stamped) onto ours: clear.
    let core = core_on(ours_at_stock(FakeGeneration::Bofr117));
    let _ = core.firmware_target();
    let (dir, d) = drive("ours");
    let ours = image("1.1.7", Some("1150"));
    flash(&core, &ours, &d, stock_board(), "").unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    // Anything onto stock firmware: no lineage to lose.
    for (i, g) in [FakeGeneration::Stock116, FakeGeneration::STOCK117]
        .into_iter()
        .enumerate()
    {
        let core = core_on(FakeDevice::demo_generation(g));
        let read = core.firmware_target();
        assert_eq!(read.lineage, FirmwareLineage::Upstream, "{g:?}");
        let (dir, d) = drive(&format!("stock-{i}"));
        let unstamped = image("1.1.6", None);
        let v = firmware_update_check(unstamped.clone(), &read, "");
        assert_eq!(v.rule, FirmwareUpdateRule::None, "{g:?}");
        let typed = v.verdict.confirmation_phrase;
        flash(&core, &unstamped, &d, read.voltage.clone(), &typed).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn wp_pgflash_a_zero_is_refused_at_the_write_gate() {
    for generation in [None, Some(FakeGeneration::STOCK117)] {
        let dev = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                board: FakeBoard::Zero,
                generation,
                surfaces: true,
                analysis: true,
                ..FakeOptions::default()
            },
        );
        let core = core_on(dev);
        let read = core.firmware_target();
        assert!(!read.has_bootsel(), "{generation:?}: {read:?}");
        // Its entry is refused (PGFWUP); a write is refused all the same, with
        // anything typed and any voltage record the caller passes.
        assert_eq!(enter(&core), Err(DspiError::Unsupported));
        let (dir, d) = drive(&format!("zero-{}", generation.is_some()));
        for (board, typed) in [
            (read.voltage.clone(), ""),
            (stock_board(), LINEAGE_PHRASE),
            (firmware_guard_board_in_bootloader(), CONFIRMATION_PHRASE),
        ] {
            let r = flash(&core, &image("1.1.7", Some("1150")), &d, board, typed);
            assert!(
                matches!(&r, Err(DspiError::FirmwareGuardBlocked { msg }) if msg.contains("BOOTSEL")),
                "{generation:?}: {r:?}"
            );
            assert!(!dir.join("firmware.uf2").exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn wp_pgflash_a_callers_voltage_record_only_adds_doubt() {
    let core = core_on(ours_at_stock(FakeGeneration::Bofr117));
    let read = core.firmware_target();
    assert_eq!(read.voltage.required_mv, 1150);
    let (dir, d) = drive("doubt");
    let unstamped = image("1.1.7", None);
    // A record the core did not read (a board "in BOOTSEL", a forged higher
    // one): the voltage is unknown, so the voltage phrase is asked, and the
    // lineage phrase does not stand for it.
    for board in [
        firmware_guard_board_in_bootloader(),
        FirmwareGuardBoard {
            required_mv: 1300,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
    ] {
        let r = flash(&core, &unstamped, &d, board.clone(), LINEAGE_PHRASE);
        assert!(
            matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
            "{r:?}"
        );
        assert!(!dir.join("firmware.uf2").exists());
    }
    // A forged 1.30 V record does not let a 1.30 V image past the word.
    let high = image("1.1.7", Some("1300"));
    let forged = FirmwareGuardBoard {
        required_mv: 1300,
        source: FirmwareGuardBoardSource::ExtCoreVreg,
    };
    assert!(firmware_guard_check(high.clone(), forged.clone(), String::new()).allowed);
    let r = flash(&core, &high, &d, forged, "");
    assert!(
        matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
        "{r:?}"
    );
    assert!(!dir.join("firmware.uf2").exists());
    // The voltage phrase covers both risks.
    flash(
        &core,
        &unstamped,
        &d,
        firmware_guard_board_in_bootloader(),
        CONFIRMATION_PHRASE,
    )
    .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wp_pgflash_with_no_target_read_the_voltage_rule_alone_decides() {
    // A board in BOOTSEL before this core saw it: a fresh core, as before.
    let core = DspiCore::new();
    assert!(core.lock().flash_target.is_none());
    let (dir, d) = drive("fallback");
    let unstamped = image("1.1.6", None);
    flash(&core, &unstamped, &d, stock_board(), "").unwrap();
    assert_eq!(std::fs::read(dir.join("firmware.uf2")).unwrap(), unstamped);
    std::fs::remove_file(dir.join("firmware.uf2")).unwrap();
    // The voltage rule still asks where it asks.
    let r = flash(
        &core,
        &unstamped,
        &d,
        firmware_guard_board_in_bootloader(),
        "",
    );
    assert!(
        matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
        "{r:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wp_pgflash_another_device_opened_forgets_the_target() {
    let core = core_on(ours_at_stock(FakeGeneration::Bofr117));
    let _ = core.firmware_target();
    assert!(core.lock().flash_target.is_some());
    // With no device the read is not kept (and does not replace the record).
    core.disconnect();
    let none = core.firmware_target();
    assert_eq!(none.voltage.source, FirmwareGuardBoardSource::NoDevice);
    assert_eq!(
        core.lock().flash_target.as_ref().map(|t| t.lineage),
        Some(FirmwareLineage::Bofr)
    );
    // Another device: the old board's target is gone.
    let dev = FakeDevice::demo_generation(FakeGeneration::Stock116);
    let serial = dev.descriptor().serial.clone();
    core.open_transport(paged::open(Box::new(dev)).unwrap(), serial, 0)
        .unwrap();
    assert!(core.lock().flash_target.is_none());
    // The bootloader entry reads it once, unless already read on this
    // connection.
    enter(&core).unwrap();
    assert_eq!(
        core.lock().flash_target.as_ref().map(|t| t.lineage),
        Some(FirmwareLineage::Upstream)
    );
}

#[test]
fn wp_pgflash_picoboot_judges_by_the_target_the_core_read() {
    let target = BootDevice {
        platform: 1,
        serial: Some("A1B2C3D4E5F60718".into()),
        bus_location: "1-3.1".into(),
    };
    let unstamped = image("1.1.6", None);
    // No target read (the hub rollout's writer core): the voltage rule.
    assert!(prepare_gated(&unstamped, &target, None, &stock_board()).is_ok());
    // Read from our firmware: the lineage loss would ask, so it is refused
    // (a PICOBOOT write asks nobody anything).
    let core = core_on(ours_at_stock(FakeGeneration::Bofr117));
    let ours = core.firmware_target();
    let r = prepare_gated(&unstamped, &target, Some(&ours), &stock_board());
    assert!(
        matches!(
            &r,
            Err(crate::picoboot::FlashError::Guard {
                decision: FirmwareGuardDecision::NeedsConfirmation,
                ..
            })
        ),
        "{:?}",
        r.err()
    );
    assert!(
        prepare_gated(
            &image("1.1.7", Some("1150")),
            &target,
            Some(&ours),
            &stock_board()
        )
        .is_ok()
    );
    // A Zero: refused.
    let zero = FirmwareTarget {
        board_id: Some(dspi_proto::packets::Board::PiZero2W.id()),
        ..ours.clone()
    };
    assert!(
        prepare_gated(
            &image("1.1.7", Some("1150")),
            &target,
            Some(&zero),
            &stock_board()
        )
        .is_err()
    );
    // Through the API: the core's own record is used (the pipe is never
    // opened on a refusal).
    let token = core.confirm_token(ConfirmOp::FirmwareFlash);
    let opened = std::cell::Cell::new(false);
    let r = core.picoboot_flash_with::<NeverPipe>(
        &unstamped,
        &target,
        &stock_board(),
        Some(&token),
        |_| {
            opened.set(true);
            Err(DspiError::Unsupported)
        },
    );
    assert!(!opened.get(), "the pipe is not opened on a refusal");
    assert!(
        matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
        "{r:?}"
    );
}
