//! WP PGFAKE2 (PLAN-PG sections 1.9 and 4): the boards that share the DSPi
//! VID:PID `2E8B:FEAA` but are not a Pico running DSPi.
//!
//! - **The DSPi-Zero** ([`FakeBoard::Zero`], upstream DSPi-Zero `192df73`): a
//!   bare-metal port to the Raspberry Pi Zero 2 W whose DSPi submodule is the
//!   1.1.6 generation, so for the probe it is a legacy device (stock: no
//!   extensions, no page guard: a paged request runs the same-number opcode and
//!   fails the test unless allowed, PGWIRE's rule). Product string "Weeb Labs
//!   DSPi Zero", `bcdDevice` 0x0300 (`uac2/usb_descriptors.{c,h}`). Asked for 8
//!   or more platform bytes it answers the 7 DSPi bytes plus board id 1 in byte
//!   7; asked for fewer, the 7-byte reply (`z4/main_z4.c:898-913`). Its
//!   bootloader entry reboots "back to the SD card's USB loader"
//!   (`pico_shim.c:664`, VID `CAFE` PID `5A4C`), never an RP2350 BOOTSEL: the
//!   fake counts `REQ_ENTER_BOOTLOADER` as a **violation** (no component may send
//!   it to a board without BOOTSEL, PLAN-PG R7) and panics unless
//!   [`FakeOptions::allow_violation`]; allowed, it answers as the firmware does
//!   and goes away to the foreign loader ([`FakeRemote::rebooted_to`]). A
//!   violation swallowed by a worker thread still fails the test when the device
//!   is dropped.
//! - **The alink sender** ([`FakeBoard::Sender`], `firmware/DSPi-Sender` on
//!   `bofr-main` `8385455`, `sender_vendor.c`): its vendor subset only:
//!   `REQ_EXTENSION` with the sender's alink block (`EXT_LIST`,
//!   `EXT_NOTIFY_FEATURES` 0, the eight sender ids of `fake/alink.rs`),
//!   `REQ_GET_PLATFORM` (RP2350, 0 outputs), `REQ_GET_SERIAL`,
//!   `REQ_GET_BUILD_INFO`, `REQ_SAVE_OUTPUT_CONFIG` and `REQ_ENTER_BOOTLOADER`
//!   (to BOOTSEL: it is a Pico 2). Everything else STALLs, an OUT too (the
//!   sender refuses it at SETUP whatever [`FakeOptions::ack_unknown_sets`]
//!   says). Product "DSPi Audio Link Sender", `bcdDevice` 0x0280.
//!   - `dual_stack: false` is today's build: guarded by PAGEGUARD, so every
//!     paged request STALLs, the probe included (a G1b device).
//!   - `dual_stack: true` is the sender after FWPG4 ("the sender answers
//!     `SYS.PROTOCOL_INFO`, `SYS.PLATFORM`, its vendor subset on the paged
//!     stack and the tunnel, and keeps page 0"). FWPG4 has no spec yet; this
//!     models its done line with upstream's 1.1.7 rules (migration guide 4 and
//!     5): pages `SYS` (discovery, `LAST_RESULT`, `PLATFORM`, `SERIAL`,
//!     `BUILD_INFO`, `ENTER_BOOTLOADER`), `STORE` (`OUTPUT_CONFIG_SAVE`) and our
//!     0xE0 (the tunnel `0xE010`, `wValue` = extension id, refusals as
//!     `VC_ERR_*`), each at the vendored revision (0xE0 at 1). A refusal is a
//!     STALL whose reason `SYS.LAST_RESULT` gives. `SYS.COMMAND_INFO` answers
//!     the registry rows it implements; the tunnel, which the registry does not
//!     have, is `VC_ERR_ADDRESS` there.
//!
//! The board is an axis of its own ([`FakeOptions::board`]), apart from the
//! protocol generation (PGFAKE1's `FakeOptions::generation`): a Zero is legacy
//! today and paged later, and "nothing here assumes either" (PLAN-PG 1.9). The
//! descriptor fields are here for PGZERO, which adds them to `DeviceDescriptor`.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::alink::{self as palink, AlinkRole};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::packets::Board;
use dspi_proto::paged::legacy::{
    VC_ERR_ADDRESS, VC_ERR_DIRECTION, VC_ERR_LENGTH, VC_ERR_UNKNOWN, VC_ERR_VALUE, VC_OK,
};
use dspi_proto::paged::registry::{self, AddrKind, GetLen};
use dspi_proto::paged::{
    CmdId, EXT_PAGE, EXT_TUNNEL, PROTOCOL_MAGIC, PROTOCOL_MAJOR, PageCommands, PagedDir, cmds,
};

use super::{AlinkSetup, Capture, FakeDevice, FakeOptions, FakeRemote, State};
use crate::{Result, TransportError};

/// The DSPi VID:PID every board here shares.
pub const DSPI_VID: u16 = 0x2E8B;
pub const DSPI_PID: u16 = 0xFEAA;

/// The DSPi-Zero's SD-card USB loader (`pico_shim.c:664`, `loader/usb_descriptors.c`).
pub const ZERO_LOADER_VID: u16 = 0xCAFE;
pub const ZERO_LOADER_PID: u16 = 0x5A4C;

/// Which board the fake is. `Pico` is every fake before PGFAKE2.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FakeBoard {
    /// A Pico (2) running DSPi: board id 0, BOOTSEL.
    #[default]
    Pico,
    /// The DSPi-Zero (Raspberry Pi Zero 2 W), board id 1, no BOOTSEL.
    Zero,
    /// The alink sender build, guarded on the 1.1.6 line, dual-stack after FWPG4.
    Sender { dual_stack: bool },
}

impl FakeBoard {
    /// The USB product string.
    pub fn product(self) -> &'static str {
        match self {
            FakeBoard::Pico => "Weeb Labs DSPi",
            FakeBoard::Zero => "Weeb Labs DSPi Zero",
            FakeBoard::Sender { .. } => "DSPi Audio Link Sender",
        }
    }

    /// The USB `bcdDevice` (DSPi's default build 0x0204, the sender 0x0280,
    /// the Zero 0x0300).
    pub fn bcd_device(self) -> u16 {
        match self {
            FakeBoard::Pico => 0x0204,
            FakeBoard::Zero => 0x0300,
            FakeBoard::Sender { .. } => 0x0280,
        }
    }

    /// The board as the platform reply's byte 7 names it.
    pub fn board(self) -> Board {
        match self {
            FakeBoard::Zero => Board::PiZero2W,
            FakeBoard::Pico | FakeBoard::Sender { .. } => Board::Pico,
        }
    }

    /// Where its bootloader entry ends.
    pub fn bootloader(self) -> RebootTarget {
        match self {
            FakeBoard::Zero => RebootTarget::ForeignLoader {
                vid: ZERO_LOADER_VID,
                pid: ZERO_LOADER_PID,
            },
            FakeBoard::Pico | FakeBoard::Sender { .. } => RebootTarget::Bootsel,
        }
    }
}

/// Where a fake went after its bootloader entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebootTarget {
    /// The RP2040/RP2350 ROM's BOOTSEL (PICOBOOT and the UF2 drive).
    Bootsel,
    /// Another loader, which no updater of ours can drive.
    ForeignLoader { vid: u16, pid: u16 },
}

/// A board's state on the fake.
#[derive(Debug, Default)]
pub(super) struct BoardState {
    rebooted: Option<RebootTarget>,
    violations: u32,
    /// The dual-stack sender's `SYS.LAST_RESULT` record.
    last_result: [u8; 8],
}

/// The dual-stack sender's private page 0xE0 revision.
const EXT_PAGE_REVISION: u8 = 1;
/// The sender's buffers (`sender_vendor.c` `resp[64]`, `rx_buf[64]`).
const SENDER_MAX: u16 = 64;

/// What the dual-stack sender implements on its paged stack, by page.
fn sender_paged(id: CmdId, dir: PagedDir) -> bool {
    let get = [
        cmds::SYS_PROTOCOL_INFO,
        cmds::SYS_PAGE_LIST,
        cmds::SYS_PAGE_COMMANDS,
        cmds::SYS_COMMAND_INFO,
        cmds::SYS_LAST_RESULT,
        cmds::SYS_PLATFORM,
        cmds::SYS_SERIAL,
        cmds::SYS_BUILD_INFO,
        EXT_TUNNEL,
    ];
    let set = [cmds::SYS_ENTER_BOOTLOADER, cmds::STORE_OUTPUT_CONFIG_SAVE, EXT_TUNNEL];
    match dir {
        PagedDir::Get => get.contains(&id),
        PagedDir::Set => set.contains(&id),
    }
}

/// The dual-stack sender's pages, ascending: SYS, STORE, 0xE0.
fn sender_pages() -> Vec<(u8, u8)> {
    let rev = |p: u8| registry::page(p).map_or(1, |i| i.revision);
    let (sys, store) = (cmds::SYS_PLATFORM.page(), cmds::STORE_OUTPUT_CONFIG_SAVE.page());
    vec![(sys, rev(sys)), (store, rev(store)), (EXT_PAGE, EXT_PAGE_REVISION)]
}

fn sender_bitmap(page: u8) -> Option<PageCommands> {
    if !sender_pages().iter().any(|(p, _)| *p == page) {
        return None;
    }
    let mut m = PageCommands::EMPTY;
    for cmd in 0x10..=0xFFu8 {
        for dir in [PagedDir::Get, PagedDir::Set] {
            if sender_paged(CmdId::new(page, cmd), dir) {
                m.insert(cmd, dir);
            }
        }
    }
    Some(m)
}

/// The sender's `EXT_LIST` (`sender_vendor.c` `ext_ids`).
fn sender_ext_ids() -> Vec<u16> {
    let mut ids = vec![gext::EXT_NOTIFY_FEATURES];
    ids.extend(palink::SENDER_IDS);
    ids
}

impl FakeDevice {
    /// The DSPi-Zero on its 1.1.6-generation submodule: the demo capture as a
    /// stock device (no extensions), board id 1, no BOOTSEL.
    pub fn zero() -> Self {
        let options = FakeOptions {
            board: FakeBoard::Zero,
            surfaces: true,
            analysis: true,
            ..FakeOptions::default()
        };
        Self::new(Capture::demo(), options)
    }

    /// The alink sender build ([`FakeBoard::Sender`]): guarded by PAGEGUARD, or
    /// dual-stack after FWPG4.
    pub fn sender(dual_stack: bool) -> Self {
        let options = FakeOptions {
            board: FakeBoard::Sender { dual_stack },
            extensions: true,
            alink: true,
            page_guard: !dual_stack,
            ..FakeOptions::default()
        };
        let dev = Self::new(Capture::demo(), options);
        dev.shared.lock().alink.setup = AlinkSetup {
            role: AlinkRole::Sender,
            ..AlinkSetup::default()
        };
        dev
    }

    /// The board this fake is.
    pub fn board(&self) -> FakeBoard {
        self.shared.lock().options.board
    }

    /// A paged IN request on a board with its own paged stack (the dual-stack
    /// sender); `None` for every other fake, whose generation decides.
    pub(super) fn board_paged_in(
        &self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> Option<Result<Vec<u8>>> {
        let mut s = self.shared.lock();
        if s.options.board != (FakeBoard::Sender { dual_stack: true }) {
            return None;
        }
        if s.gone {
            return Some(Err(TransportError::Disconnected));
        }
        s.paged_requests = s.paged_requests.saturating_add(1);
        let r = s.sender_paged_get(id, value, max_len);
        if id != cmds::SYS_LAST_RESULT {
            s.record(id, PagedDir::Get, value, r.as_ref().err().copied());
        }
        Some(match r {
            Ok(mut reply) => {
                reply.truncate(usize::from(max_len));
                Ok(reply)
            }
            Err(_) => Err(TransportError::Stalled { opcode: id.cmd() }),
        })
    }

    /// A paged OUT request, as [`FakeDevice::board_paged_in`].
    pub(super) fn board_paged_out(&self, id: CmdId, value: u16, data: &[u8]) -> Option<Result<()>> {
        let mut s = self.shared.lock();
        if s.options.board != (FakeBoard::Sender { dual_stack: true }) {
            return None;
        }
        if s.gone {
            return Some(Err(TransportError::Disconnected));
        }
        s.paged_requests = s.paged_requests.saturating_add(1);
        let r = s.sender_paged_set(id, value, data);
        s.record(id, PagedDir::Set, value, r.err());
        Some(r.map_err(|_| TransportError::Stalled { opcode: id.cmd() }))
    }

    /// On drop: a board violation that a worker thread swallowed still fails the
    /// test (PLAN-PG R9a).
    pub(super) fn board_check_on_drop(&self) {
        let (violations, allowed) = {
            let s = self.shared.lock();
            (s.board.violations, s.options.allow_violation)
        };
        if violations > 0 && !allowed && !std::thread::panicking() {
            panic!(
                "a FakeDevice was dropped after {violations} board violation(s) (a bootloader \
                 entry on a board without BOOTSEL, PLAN-PG section 1.9)"
            );
        }
    }
}

impl FakeRemote {
    /// Where the device went after a bootloader entry, if it took one.
    pub fn rebooted_to(&self) -> Option<RebootTarget> {
        self.shared.lock().board.rebooted
    }

    /// How many board violations the device saw (allowed or not).
    pub fn board_violations(&self) -> u32 {
        self.shared.lock().board.violations
    }
}

impl State {
    /// A page-0 IN request on a board other than a Pico: `Some` answers or
    /// refuses it, `None` lets the fake answer as it always does.
    pub(super) fn board_get(&mut self, opcode: u8, value: u16, want: usize) -> Option<Result<Vec<u8>>> {
        let stall = Some(Err(TransportError::Stalled { opcode }));
        match self.options.board {
            FakeBoard::Pico => None,
            FakeBoard::Zero => match opcode {
                op::REQ_GET_PLATFORM if want >= dspi_proto::packets::PlatformInfo::FULL_LEN => {
                    let mut p = self.platform.clone();
                    p.resize(7, 0);
                    p.push(Board::PiZero2W.id());
                    Some(Ok(p))
                }
                op::REQ_ENTER_BOOTLOADER => Some(Ok(self.board_bootloader())),
                _ => None,
            },
            FakeBoard::Sender { .. } => match opcode {
                op::REQ_EXTENSION => match value {
                    gext::EXT_LIST => {
                        Some(Ok(sender_ext_ids().iter().flat_map(|id| id.to_le_bytes()).collect()))
                    }
                    // The sender sends no notifications.
                    gext::EXT_NOTIFY_FEATURES => Some(Ok(vec![0; 4])),
                    v if palink::SENDER_IDS.contains(&v) => None,
                    _ => stall,
                },
                op::REQ_GET_PLATFORM => Some(Ok(self.sender_platform())),
                op::REQ_GET_SERIAL | op::REQ_GET_BUILD_INFO | op::REQ_SAVE_OUTPUT_CONFIG => None,
                op::REQ_ENTER_BOOTLOADER => Some(Ok(self.board_bootloader())),
                _ => stall,
            },
        }
    }

    /// A page-0 OUT request, as [`State::board_get`].
    pub(super) fn board_set(&mut self, opcode: u8, value: u16) -> Option<Result<()>> {
        match self.options.board {
            FakeBoard::Pico | FakeBoard::Zero => None,
            // Only the alink block's OUT ids reach a handler; anything else
            // STALLs at SETUP.
            FakeBoard::Sender { .. } => {
                if opcode == op::REQ_EXTENSION && palink::SENDER_IDS.contains(&value) {
                    None
                } else {
                    Some(Err(TransportError::Stalled { opcode }))
                }
            }
        }
    }

    /// `REQ_ENTER_BOOTLOADER` (or `SYS.ENTER_BOOTLOADER`): the reply `[1]`,
    /// then the device leaves the bus for its loader. On a board without
    /// BOOTSEL this is a violation: a panic unless allowed.
    fn board_bootloader(&mut self) -> Vec<u8> {
        let board = self.options.board;
        let target = board.bootloader();
        if !board.board().has_bootsel() {
            self.board.violations = self.board.violations.saturating_add(1);
            if !self.options.allow_violation {
                panic!(
                    "a bootloader entry reached {} (board id {}), which has no BOOTSEL and would \
                     reboot to {target:?}: no component may send one to it (PLAN-PG section \
                     1.9, R7); set FakeOptions::allow_violation to let it through",
                    board.product(),
                    board.board().id(),
                );
            }
        }
        self.board.rebooted = Some(target);
        self.gone = true;
        vec![1]
    }

    /// The sender's 7-byte platform: RP2350, the firmware's version, no outputs.
    fn sender_platform(&self) -> Vec<u8> {
        let mut p = self.platform.clone();
        p.resize(7, 0);
        if let Some(outputs) = p.get_mut(3) {
            *outputs = 0;
        }
        p
    }

    /// `SYS.LAST_RESULT`'s record (guide 5.2): id, direction (1 GET, 0 SET),
    /// result, `wValue`, two reserved bytes.
    fn record(&mut self, id: CmdId, dir: PagedDir, value: u16, refused: Option<u8>) {
        let [i0, i1] = id.0.to_le_bytes();
        let [v0, v1] = value.to_le_bytes();
        let d = u8::from(dir == PagedDir::Get);
        self.board.last_result = [i0, i1, d, refused.unwrap_or(VC_OK), v0, v1, 0, 0];
    }

    /// The 1.1.7 rules a paged request meets before its handler.
    fn sender_paged_check(id: CmdId, dir: PagedDir, value: u16, len: usize) -> std::result::Result<(), u8> {
        if !(sender_paged(id, PagedDir::Get) || sender_paged(id, PagedDir::Set)) {
            return Err(VC_ERR_UNKNOWN);
        }
        if !sender_paged(id, dir) {
            return Err(VC_ERR_DIRECTION);
        }
        if dir == PagedDir::Get && len == 0 {
            return Err(VC_ERR_LENGTH);
        }
        if id == EXT_TUNNEL {
            return Ok(());
        }
        let c = registry::command(id).ok_or(VC_ERR_UNKNOWN)?;
        if dir == PagedDir::Set && !c.set_len_ok(len) {
            return Err(VC_ERR_LENGTH);
        }
        if c.addr == AddrKind::None && value != 0 {
            return Err(VC_ERR_ADDRESS);
        }
        Ok(())
    }

    fn sender_paged_get(&mut self, id: CmdId, value: u16, want: u16) -> std::result::Result<Vec<u8>, u8> {
        if id.is_legacy() {
            // Page 0 through a paged call: the legacy stack, which keeps page 0.
            return self
                .get(id.cmd(), value, usize::from(want))
                .map_err(|_| VC_ERR_UNKNOWN);
        }
        Self::sender_paged_check(id, PagedDir::Get, value, usize::from(want))?;
        match id {
            cmds::SYS_PROTOCOL_INFO => {
                let mut r = PROTOCOL_MAGIC.to_vec();
                let pages = u8::try_from(sender_pages().len()).unwrap_or(u8::MAX);
                r.extend([PROTOCOL_MAJOR, 0, pages, 0]);
                r.extend(SENDER_MAX.to_le_bytes());
                r.extend(SENDER_MAX.to_le_bytes());
                r.resize(16, 0);
                Ok(r)
            }
            cmds::SYS_PAGE_LIST => Ok(sender_pages().iter().flat_map(|(p, r)| [*p, *r]).collect()),
            cmds::SYS_PAGE_COMMANDS => {
                let page = u8::try_from(value).map_err(|_| VC_ERR_ADDRESS)?;
                sender_bitmap(page)
                    .map(|m| m.encode().to_vec())
                    .ok_or(VC_ERR_ADDRESS)
            }
            cmds::SYS_COMMAND_INFO => {
                let target = CmdId(value);
                let c = registry::command(target)
                    .filter(|_| sender_paged(target, PagedDir::Get) || sender_paged(target, PagedDir::Set))
                    .ok_or(VC_ERR_ADDRESS)?;
                let get_len = match c.get_len {
                    GetLen::None => 0,
                    GetLen::Fixed(n) => n,
                    GetLen::Var => 0xFFFF,
                };
                let mut r = c.flags.0.to_le_bytes().to_vec();
                r.extend(get_len.to_le_bytes());
                r.extend([c.set_min, c.set_max, c.addr.code(), u8::from(c.get) | (u8::from(c.set) << 1)]);
                Ok(r)
            }
            cmds::SYS_LAST_RESULT => Ok(self.board.last_result.to_vec()),
            cmds::SYS_PLATFORM => Ok(self.sender_platform()),
            cmds::SYS_SERIAL => self
                .get(op::REQ_GET_SERIAL, 0, usize::from(want))
                .map_err(|_| VC_ERR_UNKNOWN),
            cmds::SYS_BUILD_INFO => self
                .get(op::REQ_GET_BUILD_INFO, 0, usize::from(want))
                .map_err(|_| VC_ERR_UNKNOWN),
            EXT_TUNNEL => {
                let known = value == gext::EXT_LIST || sender_ext_ids().contains(&value);
                if !known {
                    return Err(VC_ERR_ADDRESS);
                }
                self.get(op::REQ_EXTENSION, value, usize::from(want))
                    .map_err(|_| VC_ERR_VALUE)
            }
            _ => Err(VC_ERR_UNKNOWN),
        }
    }

    fn sender_paged_set(&mut self, id: CmdId, value: u16, data: &[u8]) -> std::result::Result<(), u8> {
        if id.is_legacy() {
            return self.set(id.cmd(), value, data).map_err(|_| VC_ERR_UNKNOWN);
        }
        Self::sender_paged_check(id, PagedDir::Set, value, data.len())?;
        match id {
            cmds::SYS_ENTER_BOOTLOADER => {
                self.board_bootloader();
                Ok(())
            }
            cmds::STORE_OUTPUT_CONFIG_SAVE => self
                .get(op::REQ_SAVE_OUTPUT_CONFIG, 0, 1)
                .map(|_| ())
                .map_err(|_| VC_ERR_UNKNOWN),
            EXT_TUNNEL => {
                if !palink::SENDER_IDS.contains(&value) {
                    // EXT_LIST and EXT_NOTIFY_FEATURES are GET only.
                    return Err(if value == gext::EXT_LIST || sender_ext_ids().contains(&value) {
                        VC_ERR_DIRECTION
                    } else {
                        VC_ERR_ADDRESS
                    });
                }
                self.set(op::REQ_EXTENSION, value, data).map_err(|_| VC_ERR_VALUE)
            }
            _ => Err(VC_ERR_UNKNOWN),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_pgfake2 {
    use super::*;
    use crate::Transport;
    use dspi_proto::packets::PlatformInfo;
    use dspi_proto::paged::{LastResult, ProbeReply, ProtocolInfo, SYS_PROTOCOL_INFO, decode_page_list};

    // ------------------------------------------------------------ the Zero

    /// The Zero answers 7 bytes to a 7-byte read and its board id to an 8-byte
    /// one; the decode sees board 1 and no BOOTSEL.
    #[test]
    fn wp_pgfake2_zero_platform_carries_board_one() {
        let mut d = FakeDevice::zero();
        let seven = d.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap();
        assert_eq!(seven.len(), 7);
        let p7 = PlatformInfo::decode(&seven).unwrap();
        assert!(p7.has_bootsel(), "a 7-byte read cannot see the board: ask 8");
        let eight = d.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
        assert_eq!(eight.len(), 8);
        assert_eq!(eight[..7], seven[..]);
        assert_eq!(eight[0], 1, "platform RP2350, so an unpatched console connects");
        let p = PlatformInfo::decode(&eight).unwrap();
        assert_eq!((p.board(), p.has_bootsel()), (Board::PiZero2W, false));
        // A longer read still gets 8.
        assert_eq!(d.control_in_upto(op::REQ_GET_PLATFORM, 0, 64).unwrap(), eight);
        // A Pico fake answers its 7 bytes to the same 8-byte read: board 0.
        let mut pico = FakeDevice::demo();
        let r = pico.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
        assert!(r.len() <= 7);
        assert!(PlatformInfo::decode(&r).unwrap().has_bootsel());
        // Descriptor fields and identity.
        assert_eq!(d.board(), FakeBoard::Zero);
        assert_eq!(FakeBoard::Zero.product(), "Weeb Labs DSPi Zero");
        assert_eq!(FakeBoard::Zero.bcd_device(), 0x0300);
        assert_eq!(FakeBoard::Pico.product(), "Weeb Labs DSPi");
        // A stock 1.1.6-generation device: no extension channel, the probe STALLs.
        assert!(d.control_in(op::REQ_EXTENSION, gext::EXT_LIST, 64).is_err());
    }

    /// The Zero's bootloader entry fails the test by default.
    #[test]
    #[should_panic(expected = "no BOOTSEL")]
    fn wp_pgfake2_zero_bootloader_entry_fails_by_default() {
        let mut d = FakeDevice::zero();
        let _ = d.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1);
    }

    /// Allowed, it answers as the firmware does and goes to the SD loader, never
    /// BOOTSEL; the device is gone.
    #[test]
    fn wp_pgfake2_zero_bootloader_allowed_goes_to_the_sd_loader() {
        let mut d = FakeDevice::zero();
        let remote = d.remote();
        remote.shared.lock().options.allow_violation = true;
        assert_eq!(d.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1).unwrap(), [1]);
        assert_eq!(
            remote.rebooted_to(),
            Some(RebootTarget::ForeignLoader { vid: 0xCAFE, pid: 0x5A4C })
        );
        assert_eq!(remote.board_violations(), 1);
        assert!(matches!(
            d.control_in(op::REQ_GET_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
    }

    /// A violation a worker swallowed still fails when the device is dropped.
    #[test]
    fn wp_pgfake2_zero_swallowed_violation_fails_in_drop() {
        let d = FakeDevice::zero();
        let worker = std::thread::spawn(move || {
            let mut d = d;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = d.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1);
            }));
            assert!(r.is_err(), "the request itself panics");
            // The device is dropped here, outside any unwinding.
            drop(d);
        });
        let r = worker.join();
        assert!(r.is_err(), "dropping the poisoned fake panics");
    }

    /// A paged request to the Zero is PGWIRE's legacy violation; the guard is off
    /// (stock), so the probe is the only paged request it may see.
    #[test]
    fn wp_pgfake2_zero_probe_is_legacy() {
        let mut d = FakeDevice::zero();
        let r = d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16);
        // 1.1.6 has no IN handler for 0x10: a STALL, or at least not a paged reply.
        if let Ok(b) = r {
            assert!(!matches!(ProtocolInfo::decode(&b), ProbeReply::Paged(_)));
        }
        assert_eq!(d.remote().paged_requests(), 1);
    }

    #[test]
    #[should_panic(expected = "1.1.6-generation device")]
    fn wp_pgfake2_zero_paged_bootloader_is_a_violation() {
        let mut d = FakeDevice::zero();
        let _ = d.control_out_paged(cmds::SYS_ENTER_BOOTLOADER, 0, &[]);
    }

    // ---------------------------------------------------------- the sender

    /// Today's sender (G1b, guarded): its vendor subset, the rest STALLs, every
    /// paged request STALLs.
    #[test]
    fn wp_pgfake2_sender_guarded_vendor_subset() {
        let mut d = FakeDevice::sender(false);
        assert_eq!(d.board(), FakeBoard::Sender { dual_stack: false });
        let p = d.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
        assert_eq!(p.len(), 7);
        let info = PlatformInfo::decode(&p).unwrap();
        assert_eq!((info.platform_id, info.num_output_channels), (1, 0));
        assert!(info.has_bootsel());
        assert_eq!(d.control_in(op::REQ_GET_SERIAL, 0, 16).unwrap().len(), 16);
        assert_eq!(d.control_in(op::REQ_GET_BUILD_INFO, 0, 64).unwrap().len(), 64);
        assert_eq!(d.control_in(op::REQ_SAVE_OUTPUT_CONFIG, 0, 1).unwrap(), [0]);
        // EXT_LIST: EXT_NOTIFY_FEATURES and the eight sender ids, nothing else.
        let list = d.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64).unwrap();
        let ids: Vec<u16> = list.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(ids, sender_ext_ids());
        assert_eq!(ids.len(), 9);
        assert_eq!(d.control_in(op::REQ_EXTENSION, gext::EXT_NOTIFY_FEATURES, 4).unwrap(), [0; 4]);
        let info = d.control_in_upto(op::REQ_EXTENSION, palink::EXT_ALINK_INFO, 64).unwrap();
        assert_eq!(info[0], 2, "role 2, a sender");
        // No DSP: the receiver's and the K340's ids, GET_STATUS, bulk all STALL.
        assert!(d.control_in(op::REQ_EXTENSION, gext::EXT_CORE_VREG, 8).is_err());
        assert!(d.control_in(op::REQ_GET_STATUS, 9, 41).is_err());
        assert!(d.control_in(op::REQ_GET_ALL_PARAMS, 0, 64).is_err());
        assert!(d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4]).is_err());
        assert!(d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &[0; 2]).is_err());
        // PAGEGUARD: the probe and any page STALL, nothing runs.
        assert!(matches!(
            d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16),
            Err(TransportError::Stalled { .. })
        ));
        assert_eq!(d.remote().paged_requests(), 1);
        // Its bootloader is BOOTSEL (a Pico 2): no violation.
        let remote = d.remote();
        assert_eq!(d.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1).unwrap(), [1]);
        assert_eq!(remote.rebooted_to(), Some(RebootTarget::Bootsel));
        assert_eq!(remote.board_violations(), 0);
        assert_eq!(FakeBoard::Sender { dual_stack: false }.product(), "DSPi Audio Link Sender");
        assert_eq!(FakeBoard::Sender { dual_stack: true }.bcd_device(), 0x0280);
    }

    /// The dual-stack sender: the probe, discovery, its subset on the paged
    /// stack, the tunnel with `VC_ERR_*` refusals, `LAST_RESULT`, and page 0 kept.
    #[test]
    fn wp_pgfake2_sender_dual_stack() {
        let mut d = FakeDevice::sender(true);
        // The probe answers paged.
        let probe = d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
        let ProbeReply::Paged(info) = ProtocolInfo::decode(&probe) else {
            panic!("not paged: {probe:02X?}")
        };
        assert_eq!((info.pages, info.max_set, info.max_get), (3, 64, 64));
        // Discovery: SYS, STORE, 0xE0, ascending; bitmaps that match.
        let pages = decode_page_list(&d.control_in_paged(cmds::SYS_PAGE_LIST, 0, 6).unwrap());
        let nums: Vec<u8> = pages.iter().map(|p| p.page).collect();
        assert_eq!(nums, [0x01, 0x02, EXT_PAGE]);
        let ext = PageCommands::decode(
            &d.control_in_paged(cmds::SYS_PAGE_COMMANDS, u16::from(EXT_PAGE), 64).unwrap(),
        )
        .unwrap();
        assert!(ext.has(0x10, PagedDir::Get) && ext.has(0x10, PagedDir::Set));
        let sys = PageCommands::decode(&d.control_in_paged(cmds::SYS_PAGE_COMMANDS, 1, 64).unwrap())
            .unwrap();
        assert!(sys.has(cmds::SYS_PLATFORM.cmd(), PagedDir::Get));
        assert!(sys.has(cmds::SYS_ENTER_BOOTLOADER.cmd(), PagedDir::Set));
        assert!(!sys.has(cmds::SYS_SYSTEM_STATUS.cmd(), PagedDir::Get));
        assert!(d.control_in_paged(cmds::SYS_PAGE_COMMANDS, 0x05, 64).is_err());
        let ci = d
            .control_in_paged(cmds::SYS_COMMAND_INFO, cmds::SYS_PLATFORM.0, 8)
            .unwrap();
        assert_eq!(ci, [0, 0, 7, 0, 0, 0, 0, 1]);
        // The subset on the paged stack.
        let plat = d.control_in_paged(cmds::SYS_PLATFORM, 0, 7).unwrap();
        assert_eq!(plat[3], 0);
        assert_eq!(d.control_in_paged(cmds::SYS_SERIAL, 0, 16).unwrap().len(), 16);
        assert_eq!(d.control_in_paged(cmds::SYS_BUILD_INFO, 0, 64).unwrap().len(), 64);
        d.control_out_paged(cmds::STORE_OUTPUT_CONFIG_SAVE, 0, &[]).unwrap();
        // The tunnel: EXT_LIST byte-identical to page 0's.
        let list = d.control_in_paged(EXT_TUNNEL, gext::EXT_LIST, 64).unwrap();
        assert_eq!(list, d.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64).unwrap());
        let info = d.control_in_paged(EXT_TUNNEL, palink::EXT_ALINK_INFO, 64).unwrap();
        assert_eq!(info, d.control_in_upto(op::REQ_EXTENSION, palink::EXT_ALINK_INFO, 64).unwrap());
        // Refusals are VC_ERR_*, with LAST_RESULT naming the request.
        let last = |d: &mut FakeDevice| {
            LastResult::decode(&d.control_in_paged(cmds::SYS_LAST_RESULT, 0, 8).unwrap()).unwrap()
        };
        assert!(d.control_in_paged(EXT_TUNNEL, gext::EXT_CORE_VREG, 8).is_err());
        let r = last(&mut d);
        assert!(r.matches(EXT_TUNNEL, PagedDir::Get, gext::EXT_CORE_VREG));
        assert_eq!(r.result, VC_ERR_ADDRESS);
        assert_eq!(last(&mut d), r, "reading LAST_RESULT never overwrites it");
        assert!(d.control_out_paged(EXT_TUNNEL, gext::EXT_LIST, &[0; 2]).is_err());
        assert_eq!(last(&mut d).result, VC_ERR_DIRECTION);
        assert!(d.control_in_paged(cmds::SYS_SYSTEM_STATUS, 0, 16).is_err());
        assert_eq!(last(&mut d).result, VC_ERR_UNKNOWN);
        assert!(d.control_in_paged(cmds::SYS_PLATFORM, 0, 0).is_err());
        assert_eq!(last(&mut d).result, VC_ERR_LENGTH);
        assert!(d.control_in_paged(cmds::SYS_PLATFORM, 3, 7).is_err());
        assert_eq!(last(&mut d).result, VC_ERR_ADDRESS);
        assert!(d.control_out_paged(cmds::SYS_ENTER_BOOTLOADER, 0, &[1]).is_err());
        assert_eq!(last(&mut d).result, VC_ERR_LENGTH);
        // A success is recorded too.
        d.control_in_paged(cmds::SYS_SERIAL, 0, 16).unwrap();
        let ok = last(&mut d);
        assert!(ok.matches(cmds::SYS_SERIAL, PagedDir::Get, 0));
        assert_eq!(ok.result, VC_OK);
        // Page 0 is kept for legacy hosts.
        assert_eq!(d.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(), plat);
        // The paged bootloader entry: BOOTSEL, no violation.
        let remote = d.remote();
        d.control_out_paged(cmds::SYS_ENTER_BOOTLOADER, 0, &[]).unwrap();
        assert_eq!(remote.rebooted_to(), Some(RebootTarget::Bootsel));
        assert!(matches!(
            d.control_in_paged(cmds::SYS_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
    }

    proptest::proptest! {
        /// The dual-stack sender never panics on a paged request, whatever it is.
        #[test]
        fn proptest_wp_pgfake2_sender_paged_never_panics(
            id in proptest::prelude::any::<u16>(),
            value in proptest::prelude::any::<u16>(),
            len in 0u16..300,
            out in proptest::prelude::any::<bool>(),
            data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..80),
        ) {
            let mut d = FakeDevice::sender(true);
            let id = CmdId(id);
            // Page 0 runs the legacy stack, where a bootloader entry is legal on a sender.
            if out {
                let _ = d.control_out_paged(id, value, &data);
            } else {
                let _ = d.control_in_paged(id, value, len);
            }
        }
    }
}
