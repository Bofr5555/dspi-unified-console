//! WP PGFWUP (PLAN-PG sections 1.8 and 1.9): the core's gated bootloader
//! entry per generation and board. The entry reaches each fake generation's
//! reboot (0xF0 on page 0 on a 1.1.6-generation device, `SYS.ENTER_BOOTLOADER`
//! 0x0130 on a paged one, through the adapter); on the DSPi-Zero, legacy and
//! paged alike, it refuses with nothing but the platform read sent (the raw
//! path included); an unknown board id, an unknown product string and a
//! failed board read fail closed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::{Board, PlatformInfo};
use dspi_proto::paged::cmds;
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{
    Capture, CmdId, DeviceDescriptor, FakeBoard, FakeDevice, FakeOptions, FakeRemote, Generation,
    NotificationSource, RebootTarget, Result as TResult, Transport, TransportCaps, TransportError,
    paged,
};

use super::{
    PICO_PRODUCTS, UNKNOWN_PRODUCT_BOARD, ZERO_BCD_DEVICE, ZERO_PRODUCT, board_from_descriptor,
    board_name, identify_board,
};
use crate::{ConfirmOp, DspiCore, DspiError};

// ── The descriptor seam (PGZERO fills the real fields) ──────────────────────

thread_local! {
    static DESCRIPTOR: Cell<Option<Board>> = const { Cell::new(None) };
}

/// What `descriptor_board` answers on this thread in a test: the stand-in for
/// PGZERO's `DeviceDescriptor` product string and `bcdDevice`.
pub(super) fn descriptor_override() -> Option<Board> {
    DESCRIPTOR.with(Cell::get)
}

/// Sets the descriptor's board for the rest of a test, cleared on drop.
struct DescriptorSays;

impl DescriptorSays {
    fn board(b: Option<Board>) -> Self {
        DESCRIPTOR.with(|d| d.set(b));
        Self
    }
}

impl Drop for DescriptorSays {
    fn drop(&mut self) {
        DESCRIPTOR.with(|d| d.set(None));
    }
}

// ── A raw wire that notes what reaches it ───────────────────────────────────

/// What a [`Wire`] does with a bootloader entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Entry {
    /// Answer it as a Pico does and leave the bus (the demo fakes model no
    /// reboot of their own).
    Reboot,
    /// Hand it to the fake (the sender and the Zero model their own).
    Pass,
}

#[derive(Default)]
struct Seen {
    /// `0xF0` IN on page 0.
    f0: u32,
    /// `SYS.ENTER_BOOTLOADER` SET.
    sys_entry: u32,
    /// `REQ_GET_PLATFORM` reads with their `wLength`.
    platform_reads: Vec<u16>,
    /// The wire left the bus after an entry.
    gone: bool,
}

/// The raw wire under `paged::open`: a fake, its paged calls forwarded, the
/// bootloader entry noted, and optionally a board id put into byte 7 of an
/// 8-byte platform reply or the platform read made to fail.
struct Wire {
    inner: FakeDevice,
    entry: Entry,
    seen: Arc<Mutex<Seen>>,
    board_id: Option<u8>,
    platform_fails: Arc<Mutex<bool>>,
}

impl Wire {
    fn new(inner: FakeDevice, entry: Entry) -> Self {
        Self {
            inner,
            entry,
            seen: Arc::default(),
            board_id: None,
            platform_fails: Arc::default(),
        }
    }

    fn gone(&self) -> TResult<()> {
        if self.seen.lock().unwrap().gone {
            Err(TransportError::Disconnected)
        } else {
            Ok(())
        }
    }

    fn platform(&mut self, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.seen.lock().unwrap().platform_reads.push(len);
        if *self.platform_fails.lock().unwrap() {
            return Err(TransportError::TimedOut {
                opcode: op::REQ_GET_PLATFORM,
                retries: 3,
            });
        }
        let mut reply = self
            .inner
            .control_in_upto(op::REQ_GET_PLATFORM, value, len)?;
        if let Some(id) = self.board_id
            && usize::from(len) >= PlatformInfo::FULL_LEN
        {
            reply.resize(7, 0);
            reply.push(id);
        }
        Ok(reply)
    }

    fn in_(&mut self, opcode: u8, value: u16, len: u16, upto: bool) -> TResult<Vec<u8>> {
        self.gone()?;
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.seen.lock().unwrap().f0 += 1;
            if self.entry == Entry::Reboot {
                self.seen.lock().unwrap().gone = true;
                return Ok(vec![1]);
            }
        }
        if opcode == op::REQ_GET_PLATFORM {
            return self.platform(value, len);
        }
        if upto {
            self.inner.control_in_upto(opcode, value, len)
        } else {
            self.inner.control_in(opcode, value, len)
        }
    }
}

impl Transport for Wire {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.in_(opcode, value, len, false)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        self.in_(opcode, value, max_len, true)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        self.gone()?;
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn carries_pages(&self) -> bool {
        self.inner.carries_pages()
    }

    fn control_in_paged(&mut self, id: CmdId, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        self.gone()?;
        if id == cmds::SYS_PLATFORM {
            self.seen.lock().unwrap().platform_reads.push(max_len);
            if *self.platform_fails.lock().unwrap() {
                return Err(TransportError::TimedOut {
                    opcode: op::REQ_GET_PLATFORM,
                    retries: 3,
                });
            }
        }
        self.inner.control_in_paged(id, value, max_len)
    }

    fn control_out_paged(&mut self, id: CmdId, value: u16, data: &[u8]) -> TResult<()> {
        self.gone()?;
        if id == cmds::SYS_ENTER_BOOTLOADER {
            self.seen.lock().unwrap().sys_entry += 1;
            if self.entry == Entry::Reboot {
                self.seen.lock().unwrap().gone = true;
                return Ok(());
            }
        }
        self.inner.control_out_paged(id, value, data)
    }
}

/// A core on `wire` opened as every construction site opens a device.
fn open(wire: Wire) -> (Arc<DspiCore>, Option<Generation>) {
    let serial = wire.descriptor().serial.clone();
    let t = paged::open(Box::new(wire)).expect("open");
    let generation = t.generation();
    let core = DspiCore::new();
    core.open_transport(t, serial, 0).expect("the connect");
    (core, generation)
}

fn enter(core: &DspiCore) -> crate::Result<()> {
    core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)))
}

fn no_violation(remote: &FakeRemote) {
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}

/// A paged DSPi-Zero (its submodule after a move to 1.1.7): the board axis
/// apart from the generation (PGFAKE2).
fn paged_zero() -> FakeDevice {
    FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            board: FakeBoard::Zero,
            generation: Some(FakeGeneration::STOCK117),
            surfaces: true,
            analysis: true,
            ..FakeOptions::default()
        },
    )
}

/// What the Zero fake's descriptor says (PGFAKE2's fields for PGZERO).
fn zero_descriptor() -> Option<Board> {
    board_from_descriptor(
        Some(FakeBoard::Zero.product()),
        Some(FakeBoard::Zero.bcd_device()),
    )
}

// ── The entry per generation ────────────────────────────────────────────────

#[test]
fn wp_pgfwup_the_entry_reaches_each_generations_reboot() {
    for g in FakeGeneration::ALL {
        let dev = FakeDevice::demo_generation(g);
        let remote = dev.remote();
        let wire = Wire::new(dev, Entry::Reboot);
        let seen = Arc::clone(&wire.seen);
        let (core, generation) = open(wire);
        assert_eq!(
            matches!(generation, Some(Generation::Paged(_))),
            g.is_paged(),
            "{g:?}"
        );
        seen.lock().unwrap().platform_reads.clear();
        enter(&core).unwrap_or_else(|e| panic!("{g:?}: {e}"));
        let s = seen.lock().unwrap();
        assert!(s.gone, "{g:?}: the device left for its bootloader");
        if g.is_paged() {
            assert_eq!(
                (s.f0, s.sys_entry),
                (0, 1),
                "{g:?}: SYS.ENTER_BOOTLOADER 0x0130"
            );
        } else {
            assert_eq!(
                (s.f0, s.sys_entry),
                (1, 0),
                "{g:?}: REQ_ENTER_BOOTLOADER 0xF0"
            );
        }
        // The board read first, 8 bytes asked; on a paged device the adapter
        // answers it from the `SYS.PLATFORM` its discovery read (8 asked) on
        // this connection, so nothing more is sent.
        let want: &[u16] = if g.is_paged() { &[] } else { &[8] };
        assert_eq!(s.platform_reads, want, "{g:?}");
        no_violation(&remote);
    }
}

/// The alink sender models its own reboot: to BOOTSEL by 0xF0 on today's
/// guarded build, by 0x0130 on the dual-stack one.
#[test]
fn wp_pgfwup_the_sender_reboots_to_bootsel_on_both_stacks() {
    for dual_stack in [false, true] {
        let dev = FakeDevice::sender(dual_stack);
        let remote = dev.remote();
        let t = paged::open(Box::new(Wire::new(dev, Entry::Pass))).unwrap();
        assert_eq!(
            matches!(t.generation(), Some(Generation::Paged(_))),
            dual_stack,
            "{dual_stack}"
        );
        // The core's connection on the sender is the gated call's path: the
        // board read, then the entry (`attach_probed` needs a full DSPi; the
        // sender has its vendor subset only, so the call runs on the wire).
        let mut conn = crate::Connection::new(t, "SENDER".into(), 0);
        assert_eq!(super::read_board(&mut conn).unwrap(), Board::Pico);
        assert!(super::bootloader_allowed(&mut conn).is_ok());
        let _ = conn.get_upto(op::REQ_ENTER_BOOTLOADER, 0, 1);
        assert_eq!(
            remote.rebooted_to(),
            Some(RebootTarget::Bootsel),
            "{dual_stack}"
        );
        no_violation(&remote);
    }
}

// ── The Zero ────────────────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_a_legacy_zero_never_enters_its_bootloader() {
    let dev = FakeDevice::zero();
    let remote = dev.remote();
    let wire = Wire::new(dev, Entry::Pass);
    let seen = Arc::clone(&wire.seen);
    let (core, generation) = open(wire);
    assert_eq!(generation, Some(Generation::Legacy));
    assert_eq!(core.board(), Ok(Board::PiZero2W));
    assert_eq!(enter(&core), Err(DspiError::Unsupported));
    // The raw path: gated, it names the `_confirmed` call, which refuses.
    assert!(matches!(
        core.raw_get(op::REQ_ENTER_BOOTLOADER, 0, 1),
        Err(DspiError::InvalidArgument { .. })
    ));
    let s = seen.lock().unwrap();
    assert_eq!(
        (s.f0, s.sys_entry),
        (0, 0),
        "nothing but the board read was sent"
    );
    assert_eq!(remote.rebooted_to(), None);
    no_violation(&remote);
    // The device lock the entry took is released again.
    assert_eq!(core.held_lock_holder(), None);
}

#[test]
fn wp_pgfwup_a_paged_zero_never_enters_its_bootloader() {
    // SYS.PLATFORM has no byte 7: by the platform alone it is board 0.
    let dev = paged_zero();
    let remote = dev.remote();
    let wire = Wire::new(dev, Entry::Pass);
    let seen = Arc::clone(&wire.seen);
    let (core, generation) = open(wire);
    assert!(
        matches!(generation, Some(Generation::Paged(_))),
        "{generation:?}"
    );
    // -- PG-C merge -- the fake's own descriptor (PGZERO's product string and
    // bcdDevice) names it now, with no stand-in: the PARTIAL point closed
    assert_eq!(core.board(), Ok(Board::PiZero2W), "by its descriptor");
    // The seam still decides when set (as before the merge).
    let _d = DescriptorSays::board(zero_descriptor());
    assert_eq!(core.board(), Ok(Board::PiZero2W));
    assert_eq!(enter(&core), Err(DspiError::Unsupported));
    assert!(matches!(
        core.raw_get(op::REQ_ENTER_BOOTLOADER, 0, 1),
        Err(DspiError::InvalidArgument { .. })
    ));
    let s = seen.lock().unwrap();
    assert_eq!((s.f0, s.sys_entry), (0, 0));
    assert_eq!(remote.rebooted_to(), None);
    no_violation(&remote);
}

// ── Fail closed ─────────────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_an_unknown_board_fails_closed() {
    // A board id this build does not know, in byte 7.
    for id in [2u8, 7, 0xFE] {
        let mut wire = Wire::new(FakeDevice::demo(), Entry::Reboot);
        wire.board_id = Some(id);
        let seen = Arc::clone(&wire.seen);
        let (core, _) = open(wire);
        assert_eq!(core.board(), Ok(Board::Unknown(id)));
        assert_eq!(enter(&core), Err(DspiError::Unsupported), "{id}");
        assert_eq!(seen.lock().unwrap().f0, 0, "{id}");
    }
    // A product string this build does not know.
    let wire = Wire::new(FakeDevice::demo(), Entry::Reboot);
    let seen = Arc::clone(&wire.seen);
    let (core, _) = open(wire);
    let _d = DescriptorSays::board(board_from_descriptor(Some("Someone's DSP"), Some(0x0204)));
    assert_eq!(core.board(), Ok(Board::Unknown(UNKNOWN_PRODUCT_BOARD)));
    assert_eq!(enter(&core), Err(DspiError::Unsupported));
    assert_eq!(seen.lock().unwrap().f0, 0);
}

#[test]
fn wp_pgfwup_a_failed_board_read_fails_closed() {
    // On a 1.1.6-generation device (a paged one answers from its discovery).
    for g in [
        FakeGeneration::Stock116,
        FakeGeneration::Bofr116 { page_guard: true },
    ] {
        let wire = Wire::new(FakeDevice::demo_generation(g), Entry::Reboot);
        let seen = Arc::clone(&wire.seen);
        let fails = Arc::clone(&wire.platform_fails);
        let (core, _) = open(wire);
        *fails.lock().unwrap() = true;
        let e = enter(&core).unwrap_err();
        assert!(matches!(e, DspiError::Transfer { .. }), "{g:?}: {e:?}");
        let s = seen.lock().unwrap();
        assert_eq!((s.f0, s.sys_entry), (0, 0), "{g:?}");
        assert!(!s.gone);
        drop(s);
        assert_eq!(core.held_lock_holder(), None, "{g:?}");
    }
}

#[test]
fn wp_pgfwup_a_pico_with_its_descriptor_still_enters() {
    let wire = Wire::new(FakeDevice::demo(), Entry::Reboot);
    let seen = Arc::clone(&wire.seen);
    let (core, _) = open(wire);
    let _d = DescriptorSays::board(board_from_descriptor(Some("Weeb Labs DSPi"), Some(0x0204)));
    enter(&core).unwrap();
    assert_eq!(seen.lock().unwrap().f0, 1);
    std::thread::sleep(Duration::from_millis(1));
}

// ── The rules on their own ──────────────────────────────────────────────────

#[test]
fn wp_pgfwup_the_descriptor_rule() {
    assert_eq!(zero_descriptor(), Some(Board::PiZero2W));
    assert_eq!(
        board_from_descriptor(Some(ZERO_PRODUCT), None),
        Some(Board::PiZero2W)
    );
    assert_eq!(
        board_from_descriptor(None, Some(ZERO_BCD_DEVICE)),
        Some(Board::PiZero2W)
    );
    // A Zero's bcdDevice wins over a Pico's product string: doubt blocks.
    assert_eq!(
        board_from_descriptor(Some("Weeb Labs DSPi"), Some(ZERO_BCD_DEVICE)),
        Some(Board::PiZero2W)
    );
    for p in PICO_PRODUCTS {
        assert_eq!(
            board_from_descriptor(Some(p), Some(0x0204)),
            Some(Board::Pico),
            "{p}"
        );
    }
    assert_eq!(
        board_from_descriptor(Some(FakeBoard::Sender { dual_stack: true }.product()), None),
        Some(Board::Pico)
    );
    assert_eq!(
        board_from_descriptor(Some("Pico DSP 2.1 (WinUSB)"), None),
        Some(Board::Pico)
    );
    assert_eq!(
        board_from_descriptor(Some("Weeb Labs DSPi Max"), None),
        Some(Board::Unknown(UNKNOWN_PRODUCT_BOARD))
    );
    // Nothing said: nothing known.
    assert_eq!(board_from_descriptor(None, Some(0x0204)), None);
    assert_eq!(board_from_descriptor(Some("  "), None), None);
}

#[test]
fn wp_pgfwup_the_platform_rule_and_the_names() {
    let base = [1u8, 1, 0x16, 4, 1, 6, 0];
    for len in [4usize, 6, 7] {
        let p = PlatformInfo::decode(&base[..len]).unwrap();
        assert_eq!(
            identify_board(&p, None),
            Board::Pico,
            "{len}: byte 7 absent"
        );
        assert_eq!(
            identify_board(&p, zero_descriptor()),
            Board::PiZero2W,
            "{len}: the descriptor decides on board 0"
        );
    }
    let mut eight = base.to_vec();
    eight.push(Board::PI_ZERO_2W_ID);
    let p = PlatformInfo::decode(&eight).unwrap();
    assert_eq!(identify_board(&p, None), Board::PiZero2W);
    // The platform's own board wins over a descriptor that says Pico.
    assert_eq!(identify_board(&p, Some(Board::Pico)), Board::PiZero2W);
    eight[7] = 9;
    let p = PlatformInfo::decode(&eight).unwrap();
    assert_eq!(identify_board(&p, Some(Board::Pico)), Board::Unknown(9));
    assert!(!Board::Unknown(9).has_bootsel());

    assert_eq!(board_name(Board::Pico), "DSPi");
    assert_eq!(board_name(Board::PiZero2W), "DSPi Zero");
    assert_eq!(board_name(Board::Unknown(3)), "Unknown board");
}
