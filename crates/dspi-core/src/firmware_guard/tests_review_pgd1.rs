//! Batch review PG-D part 1: what PGFLASH's own tests leave open about the
//! core's write gate. A kept target never asks less than no target at all
//! (so a target left over from another board cannot weaken the gate); the
//! 1.20 V rule holds at the gate and the lineage phrase never stands for the
//! voltage phrase; the bootloader entry with its own target read still
//! reaches the reboot on every generation and on the alink sender.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::path::PathBuf;
use std::sync::Arc;

use dspi_transport::fake::FakeGeneration;
use dspi_transport::{FakeDevice, RebootTarget, Transport, paged};

use super::tests_wp_pgfwup::image;
use super::*;
use crate::{ConfirmOp, FirmwareGeneration, FirmwareLineage};

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
        .join(format!("review-pgd1-{name}-{}", std::process::id()));
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

/// Over every target a core could have kept, every record a caller could
/// pass, a set of images and every answer: with nothing typed the kept
/// target lets nothing through that the voltage rule alone would stop; with
/// something typed it lets a write through only when the gate itself named
/// exactly that phrase; a kept board without BOOTSEL lets nothing through.
#[test]
fn review_pgd1_a_kept_target_never_asks_less_than_no_target() {
    use FirmwareGuardBoardSource as S;
    let images = [
        image("1.1.6", None),
        image("1.1.7", None),
        image("1.1.6", Some("1150")),
        image("1.1.7", Some("1200")),
        image("1.1.7", Some("1250")),
        image("1.1.7", Some("1300")),
        Vec::new(),
    ];
    let mvs = [0u16, 1150, 1200, 1300];
    let typed = [
        "",
        LINEAGE_PHRASE,
        CONFIRMATION_PHRASE,
        dspi_session::overvolt::WORD,
        "yes",
    ];
    let mut allowed = 0u32;
    for read_mv in mvs {
        for source in [S::ExtCoreVreg, S::StatusBlock, S::ReadFailed, S::Bootloader] {
            for board_id in [None, Some(0u8), Some(1), Some(9)] {
                for lineage in [
                    FirmwareLineage::Unknown,
                    FirmwareLineage::Upstream,
                    FirmwareLineage::Bofr,
                ] {
                    for generation in [FirmwareGeneration::Legacy, FirmwareGeneration::Paged] {
                        let read = FirmwareTarget {
                            voltage: FirmwareGuardBoard {
                                required_mv: read_mv,
                                source,
                            },
                            board_id,
                            lineage,
                            generation,
                        };
                        for caller_mv in mvs {
                            let board = FirmwareGuardBoard {
                                required_mv: caller_mv,
                                source: S::ExtCoreVreg,
                            };
                            for img in &images {
                                let asked = write_gate(img, Some(&read), board.clone(), "").verdict;
                                let alone = write_gate(img, None, board.clone(), "").verdict;
                                for t in typed {
                                    let kept =
                                        write_gate(img, Some(&read), board.clone(), t).verdict;
                                    let what = format!("{read:?} caller {caller_mv} typed {t:?}");
                                    if !kept.allowed {
                                        continue;
                                    }
                                    allowed += 1;
                                    assert!(read.has_bootsel(), "{what}");
                                    assert!(
                                        (asked.allowed && alone.allowed)
                                            || (!t.is_empty() && asked.confirmation_phrase == t),
                                        "{what}: {kept:?}, alone {alone:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        allowed > 100,
        "the grid did reach allowed writes: {allowed}"
    );
}

/// The K340's rule at the gate, with the update check skipped by the client:
/// our firmware at 1.20 V takes no image that runs lower or does not say,
/// unless the voltage phrase is typed; the lineage phrase is not it.
#[test]
fn review_pgd1_the_120v_rule_holds_at_the_write_gate() {
    for (i, g) in [
        FakeGeneration::Bofr116 { page_guard: true },
        FakeGeneration::Bofr117,
    ]
    .into_iter()
    .enumerate()
    {
        let dev = FakeDevice::demo_generation(g);
        let remote = dev.remote();
        let core = core_on(dev);
        enter(&core).unwrap();
        let read = core.lock().flash_target.clone().unwrap();
        assert_eq!(read.voltage.required_mv, 1200, "{g:?}: the demo is a K340");
        assert_eq!(read.lineage, FirmwareLineage::Bofr, "{g:?}");
        core.disconnect();
        let (dir, d) = drive(&format!("k340-{i}"));
        let file = dir.join("firmware.uf2");
        // Stock (no stamp), and ours built for 1.15 V.
        for low in [image("1.1.7", None), image("1.1.7", Some("1150"))] {
            for typed in ["", LINEAGE_PHRASE, dspi_session::overvolt::WORD] {
                let r = flash(&core, &low, &d, read.voltage.clone(), typed);
                assert!(
                    matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
                    "{g:?} {typed:?}: {r:?}"
                );
                assert!(!file.exists(), "{g:?} {typed:?}");
            }
        }
        // Ours at 1.20 V: nothing asked.
        flash(
            &core,
            &image("1.1.7", Some("1200")),
            &d,
            read.voltage.clone(),
            "",
        )
        .unwrap();
        assert!(file.exists(), "{g:?}");
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    }
}

/// A target kept from board A, then board B arrives in BOOTSEL by hand
/// (never opened by this core): the gate answers every typed text exactly as
/// strictly as a core with no target, or stricter.
#[test]
fn review_pgd1_a_target_left_from_another_board_does_not_weaken_the_gate() {
    let stale = core_on(FakeDevice::demo_generation(FakeGeneration::Stock116));
    let a = stale.firmware_target();
    assert_eq!(a.lineage, FirmwareLineage::Upstream);
    stale.disconnect();
    assert!(stale.lock().flash_target.is_some(), "kept over the leave");
    let fresh = DspiCore::new();
    let (dir, d) = drive("stale");
    let file = dir.join("firmware.uf2");
    let unstamped = image("1.1.7", None);
    let records = [
        firmware_guard_board_in_bootloader(),
        // Board A's own record, passed on by a client that mixed them up
        a.voltage.clone(),
        FirmwareGuardBoard {
            required_mv: 1200,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
    ];
    for board in records {
        for typed in ["", LINEAGE_PHRASE, "yes"] {
            let alone = flash(&fresh, &unstamped, &d, board.clone(), typed).is_ok();
            let _ = std::fs::remove_file(&file);
            let kept = flash(&stale, &unstamped, &d, board.clone(), typed).is_ok();
            let _ = std::fs::remove_file(&file);
            assert!(
                !kept || alone,
                "{board:?} {typed:?}: the kept target let through what no target stops"
            );
            // A write clears the kept target: put it back for the next round
            stale.lock().flash_target = Some(a.clone());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// The entry reads its own target first (a platform read, `EXT_LIST`, the
/// voltage) on every generation and on the alink sender, with no request a
/// generation must not get, and the entry still goes out: the sender's fake
/// models its reboot (the Pico fakes' entry is counted on the wire by
/// `hardware::tests_wp_pgfwup`).
#[test]
fn review_pgd1_the_entry_reads_its_target_on_every_generation() {
    let mut devices: Vec<(String, FakeDevice, bool, FirmwareLineage)> = FakeGeneration::ALL
        .into_iter()
        .map(|g| {
            let ours = matches!(g, FakeGeneration::Bofr116 { .. } | FakeGeneration::Bofr117);
            (
                format!("{g:?}"),
                FakeDevice::demo_generation(g),
                g.is_paged(),
                if ours {
                    FirmwareLineage::Bofr
                } else {
                    FirmwareLineage::Upstream
                },
            )
        })
        .collect();
    devices.push((
        "sender".into(),
        FakeDevice::sender(false),
        false,
        FirmwareLineage::Bofr,
    ));
    for (name, dev, is_paged, lineage) in devices {
        let remote = dev.remote();
        let core = core_on(dev);
        assert!(core.lock().flash_target.is_none(), "{name}");
        enter(&core).unwrap_or_else(|e| panic!("{name}: {e}"));
        if name == "sender" {
            assert_eq!(remote.rebooted_to(), Some(RebootTarget::Bootsel), "{name}");
        }
        let read = core.lock().flash_target.clone().unwrap();
        assert!(read.has_bootsel(), "{name}: {read:?}");
        assert_eq!(read.lineage, lineage, "{name}");
        assert_eq!(
            read.generation,
            if is_paged {
                FirmwareGeneration::Paged
            } else {
                FirmwareGeneration::Legacy
            },
            "{name}"
        );
        assert_eq!(
            remote.violations(),
            0,
            "{name}: {:?}",
            remote.last_violation()
        );
    }
}
