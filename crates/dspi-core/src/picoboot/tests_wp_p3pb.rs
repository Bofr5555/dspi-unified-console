//! WP P3PB: the PICOBOOT writer against an in-process fake boot ROM.
//!
//! The fake speaks the protocol of `boot/picoboot.h` as the ROM does: a
//! 32-byte command, the data phase, the acknowledgement the other way, a
//! stall and a status for a refused command, NOR flash semantics (erase to
//! `0xFF`, a write can only clear bits). No USB is touched.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use proptest::prelude::*;

use super::*;
use crate::firmware::Page;
use crate::{FirmwareGuardBoardSource, STAMP_ID};

// ── Images ─────────────────────────────────────────────────────────────

const XIP: u32 = 0x1000_0000;
const FAMILY_RP2040: u32 = 0xE48B_FF56;
const FAMILY_RP2350: u32 = 0xE48B_FF59;

fn block(target: u32, page: &[u8], no: u32, total: u32, family: u32) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    for (off, v) in [
        (0, 0x0A32_4655u32),
        (4, 0x9E5D_5157),
        (8, 0x2000),
        (12, target),
        (16, 256),
        (20, no),
        (24, total),
        (28, family),
        (508, 0x0AB1_6F30),
    ] {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[32..32 + page.len()].copy_from_slice(page);
    b
}

/// A UF2 of `pages` (address, fill byte).
fn uf2_at(pages: &[(u32, u8)], family: u32) -> Vec<u8> {
    let n = pages.len() as u32;
    let mut out = Vec::new();
    for (i, (addr, fill)) in pages.iter().enumerate() {
        out.extend(block(*addr, &[*fill; 256], i as u32, n, family));
    }
    out
}

/// A UF2 of `flash` laid out from XIP in 256-byte pages.
fn uf2(flash: &[u8], family: u32) -> Vec<u8> {
    let pages: Vec<&[u8]> = flash.chunks(256).collect();
    let n = pages.len() as u32;
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let mut page = [0u8; 256];
        page[..p.len()].copy_from_slice(p);
        out.extend(block(XIP + 256 * i as u32, &page, i as u32, n, family));
    }
    out
}

/// A DSPi image whose binary info carries the core-voltage stamp `mv`
/// (`None`: no stamp), `len` bytes of flash.
fn dspi(mv: Option<&str>, len: usize, family: u32) -> Vec<u8> {
    let mut f = vec![0u8; len.max(0x1000)];
    for (i, b) in f.iter_mut().enumerate().skip(0x1000) {
        *b = (i * 7 % 251) as u8;
    }
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    let mut entries: Vec<(u16, u32, &str)> = vec![(0x5052, 0x0203_1C86, "DSPi")];
    if let Some(mv) = mv {
        entries.push((0x5344, STAMP_ID, mv));
    }
    put(&mut f, 0x100, 0x7188_EBF2);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, 0xE71A_A390);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&6u16.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    uf2(&f, family)
}

fn stock() -> FirmwareGuardBoard {
    FirmwareGuardBoard {
        required_mv: 1150,
        source: FirmwareGuardBoardSource::ExtCoreVreg,
    }
}

fn k340() -> FirmwareGuardBoard {
    FirmwareGuardBoard {
        required_mv: 1200,
        source: FirmwareGuardBoardSource::ExtCoreVreg,
    }
}

fn rp2040() -> BootDevice {
    BootDevice {
        platform: 0,
        serial: None,
        bus_location: "1-2".into(),
    }
}

fn rp2350() -> BootDevice {
    BootDevice {
        platform: 1,
        serial: Some("A1B2C3D4E5F60718".into()),
        bus_location: "1-3.1".into(),
    }
}

fn bundled(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/qt/firmware")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// ── The fake boot ROM ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Cmd([u8; 32]),
    DataOut(usize),
    DataIn(usize),
    AckOut,
    AckIn,
    Status,
    Reset,
    ClearHalts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Command,
    DataOut,
    DataIn,
    /// The host reads a zero-length packet (an OUT command's ack).
    AckIn,
    /// The host writes the ack (an IN command's).
    AckOut,
    /// Stalled until the interface is reset.
    Stalled,
}

#[derive(Debug, Clone, Copy)]
struct Parsed {
    id: u8,
    a0: u32,
    a1: u32,
    len: u32,
}

struct FakeRom {
    ops: Vec<Op>,
    phase: Phase,
    cmd: Option<Parsed>,
    /// Flash by sector; absent sectors hold the old firmware (0x5A).
    flash: BTreeMap<u32, Vec<u8>>,
    status: u32,
    exclusive: u8,
    rebooted: bool,
    clock: Rc<Cell<Duration>>,
    /// Clock cost of every transfer.
    cost: Duration,
    /// Stall this command id's acknowledgement with this status.
    refuse: Option<(u8, u32)>,
    /// Never answer this command id (every transfer of it times out).
    hang: Option<u8>,
    /// Flip a byte of what READ returns at this address.
    corrupt: Option<u32>,
}

impl FakeRom {
    fn new() -> Self {
        FakeRom {
            ops: Vec::new(),
            phase: Phase::Command,
            cmd: None,
            flash: BTreeMap::new(),
            status: 0,
            exclusive: 0,
            rebooted: false,
            clock: Rc::new(Cell::new(Duration::ZERO)),
            cost: Duration::from_millis(1),
            refuse: None,
            hang: None,
            corrupt: None,
        }
    }

    fn tick(&self, d: Duration) {
        self.clock.set(self.clock.get() + d);
    }

    fn sector(&mut self, base: u32) -> &mut Vec<u8> {
        self.flash.entry(base).or_insert_with(|| vec![0x5A; 4096])
    }

    fn byte(&self, addr: u32) -> u8 {
        let base = addr & !0xFFF;
        self.flash
            .get(&base)
            .map(|s| s[(addr - base) as usize])
            .unwrap_or(0x5A)
    }

    fn commands(&self) -> Vec<[u8; 32]> {
        self.ops
            .iter()
            .filter_map(|o| match o {
                Op::Cmd(c) => Some(*c),
                _ => None,
            })
            .collect()
    }

    fn command_ids(&self) -> Vec<u8> {
        self.commands().iter().map(|c| c[8]).collect()
    }

    fn hung(&self, timeout: Duration) -> bool {
        if let (Some(h), Some(c)) = (self.hang, self.cmd)
            && h == c.id
        {
            self.tick(timeout);
            return true;
        }
        false
    }

    /// The effect of a finished command (at its acknowledgement).
    fn finish(&mut self) -> std::result::Result<(), PipeError> {
        let Some(c) = self.cmd else {
            return Err(PipeError::Other("no command".into()));
        };
        if let Some((id, status)) = self.refuse
            && id == c.id
        {
            self.status = status;
            self.phase = Phase::Stalled;
            return Err(PipeError::Stall);
        }
        match c.id {
            PC_EXCLUSIVE_ACCESS => self.exclusive = c.a0 as u8,
            PC_FLASH_ERASE => {
                assert_eq!(c.a0 % 4096, 0, "erase of a partial sector");
                assert_eq!(c.a1 % 4096, 0, "erase of a partial sector");
                for base in (c.a0..c.a0 + c.a1).step_by(4096) {
                    *self.sector(base) = vec![0xFF; 4096];
                }
            }
            PC_REBOOT | PC_REBOOT2 => self.rebooted = true,
            _ => {}
        }
        self.status = 0;
        self.phase = Phase::Command;
        Ok(())
    }
}

impl BootPipe for FakeRom {
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> std::result::Result<(), PipeError> {
        self.tick(self.cost);
        match self.phase {
            Phase::Command => {
                let raw: [u8; 32] = data
                    .try_into()
                    .map_err(|_| PipeError::Other("bad command".into()))?;
                self.ops.push(Op::Cmd(raw));
                let w = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
                assert_eq!(w(0), PICOBOOT_MAGIC);
                let c = Parsed {
                    id: raw[8],
                    a0: w(16),
                    a1: w(20),
                    len: w(12),
                };
                self.cmd = Some(c);
                if self.hung(timeout) {
                    return Err(PipeError::Timeout);
                }
                self.phase = match (c.len, c.id & 0x80 != 0) {
                    (0, true) => Phase::AckOut,
                    (0, false) => Phase::AckIn,
                    (_, true) => Phase::DataIn,
                    (_, false) => Phase::DataOut,
                };
                Ok(())
            }
            Phase::DataOut => {
                self.ops.push(Op::DataOut(data.len()));
                if self.hung(timeout) {
                    return Err(PipeError::Timeout);
                }
                let c = self.cmd.unwrap();
                assert_eq!(c.id, PC_WRITE);
                assert_eq!(data.len() as u32, c.len);
                for (i, b) in data.iter().enumerate() {
                    let addr = c.a0 + i as u32;
                    let base = addr & !0xFFF;
                    let s = self.sector(base);
                    s[(addr - base) as usize] &= *b; // NOR: bits only clear
                }
                self.phase = Phase::AckIn;
                Ok(())
            }
            Phase::AckOut => {
                self.ops.push(Op::AckOut);
                if self.hung(timeout) {
                    return Err(PipeError::Timeout);
                }
                self.finish()
            }
            Phase::Stalled => Err(PipeError::Stall),
            _ => Err(PipeError::Other(format!("OUT in phase {:?}", self.phase))),
        }
    }

    fn bulk_in(
        &mut self,
        len: usize,
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, PipeError> {
        self.tick(self.cost);
        match self.phase {
            Phase::DataIn => {
                self.ops.push(Op::DataIn(len));
                if self.hung(timeout) {
                    return Err(PipeError::Timeout);
                }
                let c = self.cmd.unwrap();
                let mut v: Vec<u8> = (0..c.len).map(|i| self.byte(c.a0 + i)).collect();
                if let Some(at) = self.corrupt
                    && (c.a0..c.a0 + c.len).contains(&at)
                {
                    v[(at - c.a0) as usize] ^= 0x01;
                }
                self.phase = Phase::AckOut;
                Ok(v)
            }
            Phase::AckIn => {
                self.ops.push(Op::AckIn);
                if self.hung(timeout) {
                    return Err(PipeError::Timeout);
                }
                self.finish().map(|()| Vec::new())
            }
            Phase::Stalled => Err(PipeError::Stall),
            _ => Err(PipeError::Other(format!("IN in phase {:?}", self.phase))),
        }
    }

    fn control_in(
        &mut self,
        request: u8,
        len: u16,
        _timeout: Duration,
    ) -> std::result::Result<Vec<u8>, PipeError> {
        assert_eq!(request, IF_CMD_STATUS);
        assert_eq!(len, 16);
        self.ops.push(Op::Status);
        let mut v = vec![0u8; 16];
        v[4..8].copy_from_slice(&self.status.to_le_bytes());
        v[8] = self.cmd.map(|c| c.id).unwrap_or(0);
        Ok(v)
    }

    fn control_out(
        &mut self,
        request: u8,
        _timeout: Duration,
    ) -> std::result::Result<(), PipeError> {
        assert_eq!(request, IF_RESET);
        self.ops.push(Op::Reset);
        self.phase = Phase::Command;
        self.exclusive = 0;
        Ok(())
    }

    fn clear_halts(&mut self) -> std::result::Result<(), PipeError> {
        self.ops.push(Op::ClearHalts);
        Ok(())
    }
}

fn flash(
    rom: &mut FakeRom,
    image: &[u8],
    target: &BootDevice,
    board: &FirmwareGuardBoard,
) -> std::result::Result<FlashReport, FlashError> {
    let clock = rom.clock.clone();
    let elapsed = move || clock.get();
    picoboot_flash(rom, image, target, board, &elapsed)
}

// ── The command bytes ──────────────────────────────────────────────────

/// Every command against a vector written from the header's layout
/// (`picoboot_cmd`: magic, token, id, size, 2 unused, transfer length, 16
/// argument bytes; little endian), in the order of a one-sector RP2040 flash.
#[test]
fn wp_p3pb_command_bytes_match_the_header_layout() {
    let image = uf2_at(&[(XIP, 0x11)], FAMILY_RP2040);
    let mut rom = FakeRom::new();
    flash(&mut rom, &image, &rp2040(), &stock()).unwrap();
    #[rustfmt::skip]
    let expected: [[u8; 32]; 6] = [
        // EXCLUSIVE_ACCESS, token 1, size 1, EXCLUSIVE_AND_EJECT
        [0x0b,0xd1,0x1f,0x43, 1,0,0,0, 0x01,1,0,0, 0,0,0,0, 2,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0],
        // EXIT_XIP, token 2, size 0
        [0x0b,0xd1,0x1f,0x43, 2,0,0,0, 0x06,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0],
        // FLASH_ERASE 0x10000000 + 0x1000, size 8
        [0x0b,0xd1,0x1f,0x43, 3,0,0,0, 0x03,8,0,0, 0,0,0,0, 0,0,0,0x10, 0,0x10,0,0, 0,0,0,0, 0,0,0,0],
        // WRITE 0x10000000 + 0x1000, transfer 0x1000
        [0x0b,0xd1,0x1f,0x43, 4,0,0,0, 0x05,8,0,0, 0,0x10,0,0, 0,0,0,0x10, 0,0x10,0,0, 0,0,0,0, 0,0,0,0],
        // READ 0x10000000 + 0x1000 (IN: top bit), transfer 0x1000
        [0x0b,0xd1,0x1f,0x43, 5,0,0,0, 0x84,8,0,0, 0,0x10,0,0, 0,0,0,0x10, 0,0x10,0,0, 0,0,0,0, 0,0,0,0],
        // REBOOT pc 0, sp 0x20042000, delay 500 ms, size 12
        [0x0b,0xd1,0x1f,0x43, 6,0,0,0, 0x02,12,0,0, 0,0,0,0, 0,0,0,0, 0x00,0x20,0x04,0x20, 0xf4,0x01,0,0, 0,0,0,0],
    ];
    assert_eq!(rom.commands(), expected.to_vec());
    // The interface was reset first, and each command was acknowledged the
    // other way round.
    assert_eq!(&rom.ops[..2], &[Op::ClearHalts, Op::Reset]);
    assert!(rom.ops.contains(&Op::DataOut(4096)));
    assert!(rom.ops.contains(&Op::DataIn(4096)));
    assert!(rom.rebooted);
    assert_eq!(rom.exclusive, EXCLUSIVE_AND_EJECT);
}

/// An RP2350 ends with REBOOT2, normal boot, 500 ms (size 16).
#[test]
fn wp_p3pb_rp2350_reboots_with_reboot2_normal() {
    let image = uf2_at(&[(XIP, 0x22)], FAMILY_RP2350);
    let mut rom = FakeRom::new();
    let report = flash(&mut rom, &image, &rp2350(), &stock()).unwrap();
    #[rustfmt::skip]
    let reboot2: [u8; 32] =
        [0x0b,0xd1,0x1f,0x43, 6,0,0,0, 0x0a,16,0,0, 0,0,0,0, 0,0,0,0, 0xf4,0x01,0,0, 0,0,0,0, 0,0,0,0];
    assert_eq!(rom.commands().last(), Some(&reboot2));
    assert_eq!(report.serial.as_deref(), Some("A1B2C3D4E5F60718"));
    assert_eq!(report.sectors, 1);
    assert_eq!(report.bytes_written, 4096);
}

#[test]
fn wp_p3pb_status_decodes_the_header_struct() {
    let mut raw = [0u8; 16];
    raw[0..4].copy_from_slice(&7u32.to_le_bytes());
    raw[4..8].copy_from_slice(&10u32.to_le_bytes());
    raw[8] = PC_FLASH_ERASE;
    raw[9] = 1;
    let s = CmdStatus::decode(&raw).unwrap();
    assert_eq!(
        s,
        CmdStatus {
            token: 7,
            status_code: 10,
            cmd_id: PC_FLASH_ERASE,
            in_progress: true
        }
    );
    assert_eq!(status_name(10), "not permitted");
    assert!(CmdStatus::decode(&raw[..15]).is_none());
}

// ── The page plan ──────────────────────────────────────────────────────

fn assert_plan_sound(image: &[u8], platform: i8) -> FlashPlan {
    let info = uf2_inspect(image.to_vec());
    assert!(info.valid, "{}", info.problem);
    let target = if platform == 0 { rp2040() } else { rp2350() };
    let plan = prepare(image, &target, &stock()).unwrap();
    let pages = uf2_pages(&info, image);
    assert_eq!(pages.len() as u32, info.num_blocks);
    let (lo, hi) = flash_window_of(platform).unwrap();
    // Every sector is whole, inside the window, and covered by a page.
    for (addr, data) in &plan.sectors {
        assert_eq!(addr % 4096, 0);
        assert!(*addr >= lo && addr + 4096 <= hi, "0x{addr:08x}");
        assert_eq!(data.len(), 4096);
        assert!(pages.iter().any(|p| p.addr & !0xFFF == *addr));
    }
    // Every page is in its sector, byte for byte.
    for p in &pages {
        let base = p.addr & !0xFFF;
        let (_, data) = plan.sectors.iter().find(|(a, _)| *a == base).unwrap();
        let off = (p.addr - base) as usize;
        assert_eq!(&data[off..off + 256], &p.data[..]);
    }
    // The erase runs cover exactly the planned sectors, whole sectors only.
    let mut erased = Vec::new();
    for (first, count, _) in plan.erase_runs() {
        assert!((1..=16).contains(&count));
        for k in 0..count {
            erased.push(first + k * 4096);
        }
    }
    let planned: Vec<u32> = plan.sectors.iter().map(|(a, _)| *a).collect();
    assert_eq!(erased, planned);
    plan
}

/// The released images: whole sectors only, never outside the window; the
/// RP2350's E10 block (0x10FFFF00) is not part of the plan.
#[test]
fn wp_p3pb_real_images_plan_whole_sectors_inside_the_window() {
    let plan = assert_plan_sound(&bundled("DSPi-RP2040-v1.1.6.uf2"), 0);
    assert!(plan.firmware_version.starts_with("1.1.6"));
    let plan = assert_plan_sound(&bundled("DSPi-RP2350-v1.1.6.uf2"), 1);
    assert!(plan.sectors.iter().all(|(a, _)| *a < 0x10FF_F000));
    // A 622 KiB UF2 carries 256 bytes a 512-byte block: about 78 sectors.
    assert!(
        (70..90).contains(&plan.sectors.len()),
        "{}",
        plan.sectors.len()
    );
}

/// A real image flashed end to end into the fake ROM: the flash then holds
/// the image, the untouched sectors keep the old contents.
#[test]
fn wp_p3pb_real_rp2350_image_flashes_end_to_end() {
    let image = bundled("DSPi-RP2350-v1.1.6.uf2");
    let info = uf2_inspect(image.clone());
    let mut rom = FakeRom::new();
    // Old firmware, and a settings sector at the top that must survive.
    rom.sector(0x101F_F000).fill(0x33);
    let report = flash(&mut rom, &image, &rp2350(), &stock()).unwrap();
    for p in uf2_pages(&info, &image) {
        for (i, b) in p.data.iter().enumerate() {
            assert_eq!(rom.byte(p.addr + i as u32), *b);
        }
    }
    assert_eq!(
        rom.byte(0x101F_F000),
        0x33,
        "a sector outside the image was erased"
    );
    assert!(rom.rebooted);
    assert_eq!(report.firmware_version, info.firmware_version);
    assert!(report.sectors > 60);
}

/// A partly covered sector is erased whole and the rest left erased; a gap
/// splits the erase runs; a run is at most 64 KiB.
#[test]
fn wp_p3pb_holes_and_runs() {
    let mut pages = vec![(XIP + 0x100, 0x01), (XIP + 0x3000, 0x02)];
    for k in 0..20u32 {
        pages.push((XIP + 0x10_0000 + k * 4096, 0x03));
    }
    let image = uf2_at(&pages, FAMILY_RP2040);
    let plan = assert_plan_sound(&image, 0);
    assert_eq!(plan.sectors.len(), 22);
    let first = &plan.sectors[0].1;
    assert!(first[..0x100].iter().all(|&b| b == 0xFF));
    assert!(first[0x100..0x200].iter().all(|&b| b == 0x01));
    assert!(first[0x200..].iter().all(|&b| b == 0xFF));
    let runs: Vec<(u32, u32)> = plan.erase_runs().iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(
        runs,
        vec![
            (XIP, 1),
            (XIP + 0x3000, 1),
            (XIP + 0x10_0000, 16),
            (XIP + 0x11_0000, 4)
        ]
    );
    let mut rom = FakeRom::new();
    flash(&mut rom, &image, &rp2040(), &stock()).unwrap();
    assert_eq!(rom.byte(XIP), 0xFF);
    assert_eq!(rom.byte(XIP + 0x100), 0x01);
    assert_eq!(
        rom.byte(XIP + 0x1000),
        0x5A,
        "an uncovered sector was erased"
    );
}

/// A page outside the window, or straddling its end, is never planned.
#[test]
fn wp_p3pb_nothing_outside_the_window() {
    let page = |addr| Page {
        addr,
        data: [0u8; 256],
    };
    // The RP2040's window ends at 0x11000000.
    assert!(sector_plan(&[page(0x10FF_FF00)], 0).is_ok());
    for bad in [0x1100_0000, 0x0FFF_FF00, 0x2000_0000, 0xFFFF_FF00] {
        assert!(
            matches!(sector_plan(&[page(bad)], 0), Err(FlashError::Image(_))),
            "0x{bad:08x}"
        );
    }
    // The RP2350's is 32 MB.
    assert!(sector_plan(&[page(0x11FF_FF00)], 1).is_ok());
    assert!(sector_plan(&[page(0x1200_0000)], 1).is_err());
    assert!(matches!(
        sector_plan(&[page(XIP)], -1),
        Err(FlashError::Target(_))
    ));
    // An image the inspection refuses has no pages at all.
    let image = uf2_at(&[(XIP, 1)], FAMILY_RP2040);
    let mut other = uf2_inspect(image.clone());
    other.num_blocks = 2;
    assert!(uf2_pages(&other, &image).is_empty());
}

// ── Failures ───────────────────────────────────────────────────────────

/// A read-back that differs fails before REBOOT: the board stays in BOOTSEL.
#[test]
fn wp_p3pb_mismatching_read_back_fails_before_reboot() {
    let image = dspi(None, 0x3000, FAMILY_RP2040);
    let mut rom = FakeRom::new();
    rom.corrupt = Some(XIP + 0x1234);
    let e = flash(&mut rom, &image, &rp2040(), &stock()).unwrap_err();
    assert_eq!(e, FlashError::Mismatch { addr: XIP + 0x1234 });
    assert!(!rom.rebooted);
    assert!(!rom.command_ids().contains(&PC_REBOOT));
    let msg = DspiError::from(e).to_string();
    assert!(
        msg.contains("0x10001234") && msg.contains("BOOTSEL"),
        "{msg}"
    );
}

/// A stalled pipe hits the transfer deadline and the error names the step;
/// nothing after it is sent.
#[test]
fn wp_p3pb_stalled_pipe_hits_the_deadline_and_names_the_step() {
    let image = dspi(None, 0x3000, FAMILY_RP2040);
    let mut rom = FakeRom::new();
    rom.hang = Some(PC_WRITE);
    let e = flash(&mut rom, &image, &rp2040(), &stock()).unwrap_err();
    assert_eq!(
        e,
        FlashError::Pipe {
            step: Step::Write { addr: XIP },
            error: PipeError::Timeout
        }
    );
    // One transfer deadline, not more.
    assert!(rom.clock.get() >= TRANSFER_DEADLINE);
    assert!(rom.clock.get() < TRANSFER_DEADLINE + Duration::from_secs(1));
    let msg = DspiError::from(e).to_string();
    assert!(msg.contains("write at 0x10000000"), "{msg}");
    let ids = rom.command_ids();
    assert_eq!(ids.iter().filter(|&&i| i == PC_WRITE).count(), 1, "retried");
    assert!(!ids.contains(&PC_READ) && !rom.rebooted);
    // The interface was reset after the failure.
    assert_eq!(rom.ops.last(), Some(&Op::Reset));
}

/// The whole flash's deadline stops a slow board between steps.
#[test]
fn wp_p3pb_whole_flash_deadline() {
    let image = dspi(None, 0x40000, FAMILY_RP2040); // 64 sectors
    let mut rom = FakeRom::new();
    rom.cost = Duration::from_millis(400);
    let e = flash(&mut rom, &image, &rp2040(), &stock()).unwrap_err();
    assert!(matches!(e, FlashError::Deadline { .. }), "{e:?}");
    assert!(rom.clock.get() <= FLASH_DEADLINE + Duration::from_secs(1));
    assert!(!rom.rebooted);
    assert!(DspiError::from(e).to_string().contains("60 s"));
}

/// An EXCLUSIVE_ACCESS refusal is reported with the ROM's status, once.
#[test]
fn wp_p3pb_exclusive_refusal_is_reported_not_retried() {
    let image = uf2_at(&[(XIP, 0x11)], FAMILY_RP2040);
    let mut rom = FakeRom::new();
    rom.refuse = Some((PC_EXCLUSIVE_ACCESS, 10));
    let e = flash(&mut rom, &image, &rp2040(), &stock()).unwrap_err();
    assert_eq!(
        e,
        FlashError::Device {
            step: Step::Exclusive,
            status: 10
        }
    );
    assert_eq!(rom.command_ids(), vec![PC_EXCLUSIVE_ACCESS]);
    assert_eq!(
        &rom.ops[rom.ops.len() - 3..],
        &[Op::Status, Op::ClearHalts, Op::Reset]
    );
    let msg = DspiError::from(e).to_string();
    assert!(
        msg.contains("exclusive access") && msg.contains("not permitted"),
        "{msg}"
    );
}

/// An erase the ROM refuses stops the flash there.
#[test]
fn wp_p3pb_refused_erase_stops() {
    let image = dspi(None, 0x3000, FAMILY_RP2040);
    let mut rom = FakeRom::new();
    rom.refuse = Some((PC_FLASH_ERASE, 4));
    let e = flash(&mut rom, &image, &rp2040(), &stock()).unwrap_err();
    assert_eq!(
        e,
        FlashError::Device {
            step: Step::Erase { addr: XIP },
            status: 4
        }
    );
    assert!(!rom.command_ids().contains(&PC_WRITE));
}

// ── Guard, image and target: nothing sent ──────────────────────────────

/// An image the guard does not clear outright sends nothing: an unstamped
/// image on a 1.20 V board (the phrase), a 1.25 V image on a 1.20 V board
/// (the word), any image on an unknown board, and a board in BOOTSEL.
#[test]
fn wp_p3pb_an_image_the_guard_does_not_clear_sends_nothing() {
    let unknown = FirmwareGuardBoard::default();
    let cases = [
        (dspi(None, 0x2000, FAMILY_RP2350), k340()),
        (dspi(Some("1250"), 0x2000, FAMILY_RP2350), k340()),
        (dspi(Some("1200"), 0x2000, FAMILY_RP2350), unknown),
        (
            dspi(Some("1200"), 0x2000, FAMILY_RP2350),
            crate::firmware_guard_board_in_bootloader(),
        ),
    ];
    for (image, board) in cases {
        let mut rom = FakeRom::new();
        let e = flash(&mut rom, &image, &rp2350(), &board).unwrap_err();
        let expected = firmware_guard_check(image.clone(), board.clone(), String::new());
        assert_eq!(
            e,
            FlashError::Guard {
                decision: expected.decision,
                message: expected.message.clone()
            }
        );
        assert!(rom.ops.is_empty(), "sent {:?}", rom.ops);
        assert!(matches!(
            DspiError::from(e),
            DspiError::FirmwareGuardBlocked { .. }
        ));
    }
}

/// The K340 case: a 1.20 V board takes a 1.20 V image.
#[test]
fn wp_p3pb_a_k340_takes_a_1200_image() {
    let image = dspi(Some("1200"), 0x2000, FAMILY_RP2350);
    let mut rom = FakeRom::new();
    flash(&mut rom, &image, &rp2350(), &k340()).unwrap();
    assert!(rom.rebooted);
}

/// Not firmware, another chip's image, or an unknown target: nothing sent.
#[test]
fn wp_p3pb_bad_image_or_target_sends_nothing() {
    let rp40 = uf2_at(&[(XIP, 1)], FAMILY_RP2040);
    let mut garbage = rp40.clone();
    garbage[0] ^= 0xFF;
    let unknown_chip = BootDevice {
        platform: 7,
        ..rp2040()
    };
    let cases: [(&[u8], BootDevice); 4] = [
        (&garbage, rp2040()),
        (&[], rp2040()),
        (&rp40, rp2350()),
        (&rp40, unknown_chip),
    ];
    for (image, target) in cases {
        let mut rom = FakeRom::new();
        let e = flash(&mut rom, image, &target, &stock()).unwrap_err();
        assert!(
            matches!(e, FlashError::Image(_) | FlashError::Target(_)),
            "{e:?}"
        );
        assert!(rom.ops.is_empty());
        assert!(matches!(
            DspiError::from(e),
            DspiError::InvalidArgument { .. }
        ));
    }
}

// ── The token ──────────────────────────────────────────────────────────

/// A missing or wrong token is `ConfirmationRequired` and the pipe is never
/// opened; a guard refusal with a good token opens nothing either; a good
/// token flashes, with no device open (the board is in BOOTSEL).
#[test]
fn wp_p3pb_the_token_comes_first() {
    let core = DspiCore::new();
    assert!(!core.is_connected());
    let image = uf2_at(&[(XIP, 0x11)], FAMILY_RP2040);
    let opened = Cell::new(0);
    let opener = |_: &BootDevice| {
        opened.set(opened.get() + 1);
        Ok(FakeRom::new())
    };
    let code = |r: Result<FlashReport>| match r {
        Err(DspiError::ConfirmationRequired { code, .. }) => code,
        other => format!("{other:?}"),
    };
    let r = core.picoboot_flash_with(&image, &rp2040(), &stock(), None, opener);
    assert_eq!(code(r), "missing");
    let other = core.confirm_token(ConfirmOp::EnterBootloader);
    let r = core.picoboot_flash_with(&image, &rp2040(), &stock(), Some(&other), opener);
    assert_eq!(code(r), "wrong_op");
    let r = core.picoboot_flash_with(&image, &rp2040(), &stock(), Some("forged"), opener);
    assert!(matches!(r, Err(DspiError::ConfirmationRequired { .. })));
    assert_eq!(opened.get(), 0, "the pipe was opened without a token");

    let t = core.confirm_token(ConfirmOp::FirmwareFlash);
    let r = core.picoboot_flash_with(&image, &rp2040(), &k340(), Some(&t), opener);
    assert!(
        matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
        "{r:?}"
    );
    assert_eq!(opened.get(), 0, "the pipe was opened for a blocked image");

    let t = core.confirm_token(ConfirmOp::FirmwareFlash);
    let report = core
        .picoboot_flash_with(&image, &rp2040(), &stock(), Some(&t), opener)
        .unwrap();
    assert_eq!(opened.get(), 1);
    assert_eq!(report.sectors, 1);
    // The token is spent.
    let r = core.picoboot_flash_with(&image, &rp2040(), &stock(), Some(&t), opener);
    assert!(matches!(r, Err(DspiError::ConfirmationRequired { .. })));
    assert_eq!(opened.get(), 1);
}

/// Without a bus (a container has none) the listing is empty and does not
/// panic; the real path with a good token answers an error, never a flash.
#[test]
fn wp_p3pb_no_bus_no_panic() {
    let devices = picoboot_devices();
    assert!(devices.iter().all(|d| d.platform == 0 || d.platform == 1));
    let core = DspiCore::new();
    let t = core.confirm_token(ConfirmOp::FirmwareFlash);
    let target = BootDevice {
        bus_location: "250-9.9.9".into(),
        ..rp2040()
    };
    let r =
        core.picoboot_flash_confirmed(uf2_at(&[(XIP, 1)], FAMILY_RP2040), target, stock(), Some(t));
    assert!(r.is_err());
}

/// The plan's and the report's Debug never carry image bytes.
#[test]
fn wp_p3pb_debug_has_no_image_bytes() {
    let image = uf2_at(&[(XIP, 0xAB)], FAMILY_RP2040);
    let plan = prepare(&image, &rp2040(), &stock()).unwrap();
    let text = format!("{plan:?}");
    assert!(
        !text.contains("171") && !text.to_lowercase().contains("ab, "),
        "{text}"
    );
    let info = uf2_inspect(image.clone());
    let pages = uf2_pages(&info, &image);
    assert_eq!(format!("{:?}", pages[0]), "Page(0x10000000)");
}

// ── Never panics ───────────────────────────────────────────────────────

/// A pipe that answers anything at all.
struct ChaosPipe {
    answers: Vec<u8>,
    at: usize,
}

impl ChaosPipe {
    fn next(&mut self) -> u8 {
        let b = self.answers.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        b
    }
    fn result<T>(&mut self, ok: impl FnOnce(&mut Self) -> T) -> std::result::Result<T, PipeError> {
        match self.next() % 6 {
            0 => Err(PipeError::Timeout),
            1 => Err(PipeError::Stall),
            2 => Err(PipeError::Disconnected),
            3 => Err(PipeError::Other("x".into())),
            _ => Ok(ok(self)),
        }
    }
}

impl BootPipe for ChaosPipe {
    fn bulk_out(&mut self, _: &[u8], _: Duration) -> std::result::Result<(), PipeError> {
        self.result(|_| ())
    }
    fn bulk_in(&mut self, len: usize, _: Duration) -> std::result::Result<Vec<u8>, PipeError> {
        self.result(|s| {
            let n = match s.next() % 3 {
                0 => 0,
                1 => len,
                _ => usize::from(s.next()),
            };
            (0..n).map(|_| s.next()).collect()
        })
    }
    fn control_in(
        &mut self,
        _: u8,
        _: u16,
        _: Duration,
    ) -> std::result::Result<Vec<u8>, PipeError> {
        self.result(|s| {
            let n = usize::from(s.next() % 20);
            (0..n).map(|_| s.next()).collect()
        })
    }
    fn control_out(&mut self, _: u8, _: Duration) -> std::result::Result<(), PipeError> {
        self.result(|_| ())
    }
    fn clear_halts(&mut self) -> std::result::Result<(), PipeError> {
        self.result(|_| ())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Mutated images never panic the writer, and an image that no longer
    /// inspects valid sends nothing.
    #[test]
    fn proptest_wp_p3pb_mutated_images_never_panic(
        flips in proptest::collection::vec((0usize..0x2000 * 2, any::<u8>()), 1..12),
        truncate in proptest::option::of(0usize..0x4000),
    ) {
        let mut image = dspi(None, 0x2000, FAMILY_RP2040);
        for (at, x) in flips {
            let at = at % image.len();
            image[at] ^= x;
        }
        if let Some(t) = truncate {
            image.truncate(t);
        }
        let valid = uf2_inspect(image.clone()).valid;
        let mut rom = FakeRom::new();
        let r = flash(&mut rom, &image, &rp2040(), &stock());
        if !valid {
            prop_assert!(r.is_err());
            prop_assert!(rom.ops.is_empty());
        }
    }

    /// Any device answers never panic the writer, and it always ends.
    #[test]
    fn proptest_wp_p3pb_any_device_answer_never_panics(
        answers in proptest::collection::vec(any::<u8>(), 0..400),
    ) {
        let image = dspi(None, 0x2000, FAMILY_RP2040);
        let mut pipe = ChaosPipe { answers, at: 0 };
        let now = Cell::new(Duration::ZERO);
        let elapsed = || {
            now.set(now.get() + Duration::from_millis(10));
            now.get()
        };
        let _ = picoboot_flash(&mut pipe, &image, &rp2040(), &stock(), &elapsed);
    }

    /// The status decode never panics.
    #[test]
    fn proptest_wp_p3pb_status_decode(raw in proptest::collection::vec(any::<u8>(), 0..40)) {
        let _ = CmdStatus::decode(&raw);
    }
}
