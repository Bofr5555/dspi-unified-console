//! WP PGFWX: the update target as the bindings carry it, and PG-C review
//! finding 8: the target's lineage and voltage record come from one
//! `EXT_LIST` read, and a lineage that could not be read on an open board
//! counts as ours, so the typed confirmation is asked.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dspi_proto::packets::Board;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, TransportCaps,
    TransportError,
};

use super::tests_wp_pgfwup::image;
use super::*;
use crate::{FirmwareGeneration, FirmwareLineage};

/// How `EXT_LIST` answers.
#[derive(Clone, Copy)]
enum List {
    Timeout,
    Stall,
}

/// The demo capture, with `EXT_LIST` answering as told and every read of it
/// counted.
struct Counting {
    dev: FakeDevice,
    list: List,
    reads: Arc<AtomicU32>,
}

impl Counting {
    fn list(&self, opcode: u8, value: u16) -> Option<dspi_transport::Result<Vec<u8>>> {
        if (opcode, value) != (op::REQ_EXTENSION, g::EXT_LIST) {
            return None;
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        Some(Err(match self.list {
            List::Timeout => TransportError::TimedOut { opcode, retries: 0 },
            List::Stall => TransportError::Stalled { opcode },
        }))
    }
}

impl Transport for Counting {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        match self.list(opcode, value) {
            Some(r) => r,
            None => self.dev.control_in(opcode, value, len),
        }
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        match self.list(opcode, value) {
            Some(r) => r,
            None => self.dev.control_in_upto(opcode, value, len),
        }
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.dev.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

/// The target of a demo board whose `EXT_LIST` answers `list`, and how often
/// it was read for it.
fn target_with(list: List) -> (FirmwareTarget, u32) {
    let reads = Arc::new(AtomicU32::new(0));
    let core = DspiCore::new();
    core.attach(
        Box::new(Counting {
            dev: FakeDevice::new(Capture::demo(), FakeOptions::default()),
            list,
            reads: Arc::clone(&reads),
        }),
        "PGFWX".into(),
        0,
    );
    let before = reads.load(Ordering::SeqCst);
    let t = core.firmware_target();
    (t, reads.load(Ordering::SeqCst) - before)
}

#[test]
fn wp_pgfwx_the_target_reads_ext_list_once() {
    let (t, reads) = target_with(List::Timeout);
    assert_eq!(
        reads, 1,
        "one EXT_LIST read for the lineage and the voltage"
    );
    assert_eq!(t.lineage, FirmwareLineage::Unknown);
    assert_eq!(t.voltage.source, FirmwareGuardBoardSource::ReadFailed);
    assert_eq!(t.board(), Some(Board::Pico));
    let (t, reads) = target_with(List::Stall);
    assert_eq!(reads, 1);
    assert_eq!(t.lineage, FirmwareLineage::Upstream);
}

#[test]
fn wp_pgfwx_an_unknown_lineage_on_an_open_board_asks() {
    let stock = || image("1.1.6", None);
    // Read from the wire: a failed EXT_LIST read leaves both unknown, and the
    // message names both risks.
    let (t, _) = target_with(List::Timeout);
    assert!(t.may_be_ours());
    let v = firmware_update_check(stock(), &t, "");
    assert_eq!(v.rule, FirmwareUpdateRule::LineageLoss);
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::NeedsConfirmation);
    assert!(v.verdict.message.contains("this fork's firmware"), "{v:?}");
    // The lineage alone: a board the voltage rule clears still asks.
    let open = FirmwareTarget {
        voltage: FirmwareGuardBoard {
            required_mv: 1150,
            source: FirmwareGuardBoardSource::StatusBlock,
        },
        board_id: Some(0),
        lineage: FirmwareLineage::Unknown,
        generation: FirmwareGeneration::Legacy,
    };
    let v = firmware_update_check(stock(), &open, "");
    assert_eq!(
        (
            v.rule,
            v.verdict.decision,
            v.verdict.confirmation_phrase.as_str()
        ),
        (
            FirmwareUpdateRule::LineageLoss,
            FirmwareGuardDecision::NeedsConfirmation,
            LINEAGE_PHRASE
        )
    );
    let v = firmware_update_check(stock(), &open, LINEAGE_PHRASE);
    assert!(v.verdict.allowed);
    assert_eq!(v.verdict.decision, FirmwareGuardDecision::Confirmed);
    // Known upstream: the voltage rule alone.
    let upstream = FirmwareTarget {
        lineage: FirmwareLineage::Upstream,
        ..open.clone()
    };
    let v = firmware_update_check(stock(), &upstream, "");
    assert_eq!(
        (v.rule, v.verdict.decision),
        (FirmwareUpdateRule::None, FirmwareGuardDecision::Clear)
    );
    // A board already in BOOTSEL was not read: the voltage rule asks on its own.
    let boot = FirmwareTarget::in_bootloader();
    assert!(!boot.may_be_ours());
    let v = firmware_update_check(stock(), &boot, "");
    assert_eq!(v.rule, FirmwareUpdateRule::None);
    assert_eq!(v.verdict.reason, FirmwareGuardReason::BoardUnknown);
}

#[test]
fn wp_pgfwx_the_target_round_trips_as_json_and_fails_closed() {
    let t = FirmwareTarget {
        voltage: FirmwareGuardBoard {
            required_mv: 1200,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
        board_id: Some(Board::PiZero2W.id()),
        lineage: FirmwareLineage::Bofr,
        generation: FirmwareGeneration::Paged,
    };
    let json = serde_json::to_value(&t).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "voltage": {"requiredMv": 1200, "source": "extCoreVreg"},
            "boardId": 1,
            "lineage": "bofr",
            "generation": "paged",
        })
    );
    assert_eq!(serde_json::from_value::<FirmwareTarget>(json).unwrap(), t);
    // Missing fields are the defaults: no board, so no image.
    let empty: FirmwareTarget = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, FirmwareTarget::default());
    assert!(!empty.has_bootsel());
    let v = firmware_update_check(image("1.1.6", None), &empty, LINEAGE_PHRASE);
    assert_eq!(v.rule, FirmwareUpdateRule::NoBootsel);
    assert!(!v.verdict.allowed);
}
