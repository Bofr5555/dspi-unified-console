//! The PICOBOOT writer: flash a checked image straight into a board's boot
//! ROM over its PICOBOOT vendor interface, with no bootloader drive mounted
//! (OD-P2b option 3, docs/core-merge/PLAN-P3.md, WP P3PB).
//!
//! The clients' own flash path copies the image to the boot ROM's drive
//! ([`DspiCore::flash_confirmed`]); that needs a mounted file system and a
//! person who can replug a board whose kernel mass-storage probe made the ROM
//! stop answering. The hub flashes with nobody at the rack, so it talks to
//! the ROM's second interface instead, as picotool does:
//!
//! 1. `PICOBOOT_IF_RESET` (clear both endpoints' halts, then the reset
//!    request): a known state.
//! 2. `EXCLUSIVE_ACCESS` with `EXCLUSIVE_AND_EJECT`: the ROM ejects its drive,
//!    so the kernel's mass-storage probe stops.
//! 3. `EXIT_XIP`.
//! 4. Per run of contiguous sectors (at most 64 KiB a command), `FLASH_ERASE`
//!    of exactly the 4 KiB sectors the image's pages cover, then a `WRITE`
//!    of each sector (bytes the image does not cover stay erased, `0xFF`, as
//!    the drive path leaves them).
//! 5. `READ` of every written sector, compared byte for byte. A mismatch ends
//!    the flash here, before any reboot: the board stays in BOOTSEL.
//! 6. `REBOOT` (RP2040: PC 0, SP the end of SRAM, 500 ms) or `REBOOT2` with
//!    the normal boot type (RP2350, 500 ms).
//!
//! The wire is `boot/picoboot.h` of the Pico SDK: a 32-byte little-endian
//! command on the bulk OUT endpoint (`PICOBOOT_MAGIC`, a token, `bCmdId` with
//! the top bit set for IN, `bCmdSize`, `dTransferLength`, 16 argument
//! bytes), the data phase in the command's direction, and an acknowledgement
//! in the opposite direction. A refused command stalls an endpoint; the
//! writer then reads `PICOBOOT_IF_CMD_STATUS` for the ROM's reason, recovers
//! the interface with `PICOBOOT_IF_RESET`, and stops. **Nothing is retried.**
//!
//! In front of the first byte: the image is fully checked (`uf2_inspect`),
//! the firmware guard must say **clear** for the board record read before
//! the board entered BOOTSEL, with nothing typed ([`firmware_guard_check`]
//! with `""`: an image that would ask a person for the phrase or the word is
//! refused here, a second line behind the caller's own check), the image must
//! be for the target's chip, and every erased sector must lie in that chip's
//! XIP window. [`DspiCore::picoboot_flash_confirmed`] also takes a
//! confirmation token ([`ConfirmOp::FirmwareFlash`]) as
//! [`DspiCore::flash_confirmed`] does.
//!
//! Every transfer has a 5 s deadline and the whole flash 60 s; a device that
//! answers garbage is a typed error, never a panic. Only the PICOBOOT
//! interface (class `0xFF`, two bulk endpoints) is ever claimed, never the
//! mass-storage one. The writer talks to the boot ROM only: it never sends a
//! DSPi request and never touches the core-voltage setting.
//!
//! Owned by WP P3PB (docs/core-merge/PLAN-P3.md).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::firmware::{Page, flash_window_of, uf2_pages};
use crate::{
    ConfirmOp, DspiCore, DspiError, FirmwareGuardBoard, FirmwareGuardDecision, Result,
    firmware_guard_check, uf2_inspect,
};

// ── The wire (boot/picoboot.h) ─────────────────────────────────────────

/// `PICOBOOT_MAGIC`.
pub(crate) const PICOBOOT_MAGIC: u32 = 0x431F_D10B;
/// `PICOBOOT_IF_RESET`: size 0 OUT, un-stall and reset the interface.
pub(crate) const IF_RESET: u8 = 0x41;
/// `PICOBOOT_IF_CMD_STATUS`: size 16 IN, the status of the last command.
pub(crate) const IF_CMD_STATUS: u8 = 0x42;

pub(crate) const PC_EXCLUSIVE_ACCESS: u8 = 0x01;
pub(crate) const PC_REBOOT: u8 = 0x02;
pub(crate) const PC_FLASH_ERASE: u8 = 0x03;
pub(crate) const PC_READ: u8 = 0x84;
pub(crate) const PC_WRITE: u8 = 0x05;
pub(crate) const PC_EXIT_XIP: u8 = 0x06;
pub(crate) const PC_REBOOT2: u8 = 0x0A;

/// `picoboot_exclusive_type::EXCLUSIVE_AND_EJECT`.
pub(crate) const EXCLUSIVE_AND_EJECT: u8 = 2;
/// `REBOOT2_FLAG_REBOOT_TYPE_NORMAL`.
pub(crate) const REBOOT2_NORMAL: u32 = 0;
/// picotool's reboot delay.
pub(crate) const REBOOT_DELAY_MS: u32 = 500;
/// `SRAM_END_RP2040` (picotool `addresses.h`): the stack for `REBOOT`.
pub(crate) const RP2040_SRAM_END: u32 = 0x2004_2000;

const CMD_LEN: usize = 32;
const STATUS_LEN: u16 = 16;
/// The flash erase unit.
pub(crate) const SECTOR: u32 = 4096;
const SECTOR_LEN: usize = SECTOR as usize;
/// The most sectors one `FLASH_ERASE` covers (64 KiB: a block erase for the
/// ROM, well inside the transfer deadline).
const ERASE_RUN_MAX: u32 = 16;

/// Each transfer's deadline.
pub(crate) const TRANSFER_DEADLINE: Duration = Duration::from_secs(5);
/// The whole flash's deadline.
pub(crate) const FLASH_DEADLINE: Duration = Duration::from_secs(60);
/// The recovery transfers' deadline (status read, reset): short, so a dead
/// pipe cannot hold the caller long past the failure.
const RECOVERY_DEADLINE: Duration = Duration::from_secs(1);

/// `Uf2Info::platform` values.
const PLATFORM_RP2040: i8 = 0;
const PLATFORM_RP2350: i8 = 1;

/// A 32-byte `picoboot_cmd`, little endian.
pub(crate) fn encode_command(
    token: u32,
    cmd_id: u8,
    args: &[u8],
    transfer_length: u32,
) -> [u8; CMD_LEN] {
    let mut out = [0u8; CMD_LEN];
    let size = u8::try_from(args.len().min(16)).unwrap_or(16);
    let fields = PICOBOOT_MAGIC
        .to_le_bytes()
        .into_iter()
        .chain(token.to_le_bytes())
        .chain([cmd_id, size, 0, 0])
        .chain(transfer_length.to_le_bytes())
        .chain(args.iter().copied().take(16));
    for (slot, b) in out.iter_mut().zip(fields) {
        *slot = b;
    }
    out
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// One command of the writer: what it is, its arguments and data phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    pub(crate) step: Step,
    pub(crate) cmd_id: u8,
    pub(crate) args: Vec<u8>,
    pub(crate) transfer_length: u32,
}

impl Command {
    pub(crate) fn exclusive_and_eject() -> Self {
        Command {
            step: Step::Exclusive,
            cmd_id: PC_EXCLUSIVE_ACCESS,
            args: vec![EXCLUSIVE_AND_EJECT],
            transfer_length: 0,
        }
    }
    pub(crate) fn exit_xip() -> Self {
        Command {
            step: Step::ExitXip,
            cmd_id: PC_EXIT_XIP,
            args: Vec::new(),
            transfer_length: 0,
        }
    }
    pub(crate) fn erase(addr: u32, size: u32) -> Self {
        Command {
            step: Step::Erase { addr },
            cmd_id: PC_FLASH_ERASE,
            args: words(&[addr, size]),
            transfer_length: 0,
        }
    }
    pub(crate) fn write(addr: u32, size: u32) -> Self {
        Command {
            step: Step::Write { addr },
            cmd_id: PC_WRITE,
            args: words(&[addr, size]),
            transfer_length: size,
        }
    }
    pub(crate) fn read(addr: u32, size: u32) -> Self {
        Command {
            step: Step::Read { addr },
            cmd_id: PC_READ,
            args: words(&[addr, size]),
            transfer_length: size,
        }
    }
    pub(crate) fn reboot_rp2040() -> Self {
        Command {
            step: Step::Reboot,
            cmd_id: PC_REBOOT,
            args: words(&[0, RP2040_SRAM_END, REBOOT_DELAY_MS]),
            transfer_length: 0,
        }
    }
    pub(crate) fn reboot2_normal() -> Self {
        Command {
            step: Step::Reboot,
            cmd_id: PC_REBOOT2,
            args: words(&[REBOOT2_NORMAL, REBOOT_DELAY_MS, 0, 0]),
            transfer_length: 0,
        }
    }
    fn is_in(&self) -> bool {
        self.cmd_id & 0x80 != 0
    }
}

/// `picoboot_cmd_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CmdStatus {
    pub(crate) token: u32,
    pub(crate) status_code: u32,
    pub(crate) cmd_id: u8,
    pub(crate) in_progress: bool,
}

impl CmdStatus {
    /// Decode the 16-byte status; `None` for a short answer.
    pub(crate) fn decode(raw: &[u8]) -> Option<Self> {
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                raw.get(at..at.checked_add(4)?)?.try_into().ok()?,
            ))
        };
        if raw.len() < usize::from(STATUS_LEN) {
            return None;
        }
        Some(CmdStatus {
            token: word(0)?,
            status_code: word(4)?,
            cmd_id: *raw.get(8)?,
            in_progress: *raw.get(9)? != 0,
        })
    }
}

/// `picoboot_status`, for the error text.
pub(crate) fn status_name(code: u32) -> &'static str {
    match code {
        0 => "ok",
        1 => "unknown command",
        2 => "invalid command length",
        3 => "invalid transfer length",
        4 => "invalid address",
        5 => "bad alignment",
        6 => "interleaved write",
        7 => "rebooting",
        8 => "unknown error",
        9 => "invalid state",
        10 => "not permitted",
        11 => "invalid argument",
        12 => "buffer too small",
        13 => "precondition not met",
        14 => "modified data",
        15 => "invalid data",
        16 => "not found",
        17 => "unsupported modification",
        _ => "an unknown status",
    }
}

// ── The pipe ───────────────────────────────────────────────────────────

/// Why a transfer on the boot pipe failed.
#[derive(Debug, Clone, PartialEq, Eq)]
// Built by the nusb pipe and the tests' fakes only.
#[cfg_attr(
    not(any(
        test,
        all(
            feature = "usb",
            any(target_os = "linux", target_os = "macos", windows)
        )
    )),
    allow(dead_code)
)]
pub(crate) enum PipeError {
    /// No answer within the deadline.
    Timeout,
    /// The ROM stalled the endpoint (a refused command).
    Stall,
    /// The board left the bus.
    Disconnected,
    /// Anything else the host's USB stack reported.
    Other(String),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Timeout => f.write_str("no answer in time"),
            PipeError::Stall => f.write_str("the boot ROM stalled the transfer"),
            PipeError::Disconnected => f.write_str("the board left the bus"),
            PipeError::Other(e) => write!(f, "USB error: {e}"),
        }
    }
}

/// The boot ROM's PICOBOOT interface: its two bulk endpoints and the
/// interface's vendor control requests (`wIndex` is the interface number,
/// which the pipe knows). The nusb implementation is [`usb::NusbPipe`]; tests
/// use an in-process fake.
pub(crate) trait BootPipe {
    /// Send `data` on the bulk OUT endpoint (all of it, or an error).
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> std::result::Result<(), PipeError>;
    /// Read up to `len` bytes from the bulk IN endpoint (fewer on a short
    /// packet; an empty `Vec` is a zero-length packet).
    fn bulk_in(&mut self, len: usize, timeout: Duration)
    -> std::result::Result<Vec<u8>, PipeError>;
    /// A vendor IN request to the interface, `wValue` 0.
    fn control_in(
        &mut self,
        request: u8,
        len: u16,
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, PipeError>;
    /// A vendor OUT request to the interface with no data, `wValue` 0.
    fn control_out(&mut self, request: u8, timeout: Duration)
    -> std::result::Result<(), PipeError>;
    /// Clear both bulk endpoints' halt (before `PICOBOOT_IF_RESET`).
    fn clear_halts(&mut self) -> std::result::Result<(), PipeError>;
}

// ── Records ────────────────────────────────────────────────────────────

/// A board in BOOTSEL mode with a PICOBOOT interface.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct BootDevice {
    /// 0 RP2040 (`2E8A:0003`), 1 RP2350 (`2E8A:000F`), as `Uf2Info::platform`.
    pub platform: i8,
    /// The boot ROM's USB serial. On an RP2350 it is the chip's unique id,
    /// which is the DSPi's serial; the RP2040's ROM reports a fixed string,
    /// so it is `None` there and only uniqueness identifies the board.
    pub serial: Option<String>,
    /// Where it is on the bus (`<bus>-<port>.<port>...`), stable across a
    /// re-enumeration on the same port.
    pub bus_location: String,
}

/// What a PICOBOOT flash did.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct FlashReport {
    /// The target's platform.
    pub platform: i8,
    /// The target's serial, as [`BootDevice::serial`].
    pub serial: Option<String>,
    /// The image's DSPi version (`Uf2Info::firmware_version`), "" if none.
    pub firmware_version: String,
    /// 4 KiB sectors erased, written and read back.
    pub sectors: u32,
    /// Bytes written (whole sectors).
    pub bytes_written: u64,
    /// How long it took, in ms.
    pub elapsed_ms: u64,
}

// ── Errors ─────────────────────────────────────────────────────────────

/// Which step of the flash, for the error text and the caller's record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Reset,
    Exclusive,
    ExitXip,
    Erase { addr: u32 },
    Write { addr: u32 },
    Read { addr: u32 },
    Reboot,
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Reset => f.write_str("PICOBOOT reset"),
            Step::Exclusive => f.write_str("exclusive access"),
            Step::ExitXip => f.write_str("EXIT_XIP"),
            Step::Erase { addr } => write!(f, "erase at 0x{addr:08x}"),
            Step::Write { addr } => write!(f, "write at 0x{addr:08x}"),
            Step::Read { addr } => write!(f, "read-back at 0x{addr:08x}"),
            Step::Reboot => f.write_str("reboot"),
        }
    }
}

/// Why a PICOBOOT flash stopped. Mapped to [`DspiError`] at the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FlashError {
    /// Not a firmware file, or no pages to write.
    Image(String),
    /// The guard did not clear the image for this board (its message).
    Guard {
        decision: FirmwareGuardDecision,
        message: String,
    },
    /// The image is not for the target's chip, or the target is unknown.
    Target(String),
    /// A transfer failed.
    Pipe { step: Step, error: PipeError },
    /// The ROM refused the command (`picoboot_status`, 0 if unknown).
    Device { step: Step, status: u32 },
    /// The ROM answered something the protocol does not allow.
    Protocol { step: Step, what: String },
    /// The whole flash's deadline passed before this step.
    Deadline { step: Step },
    /// The read-back differed from the image.
    Mismatch { addr: u32 },
}

impl std::fmt::Display for FlashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlashError::Image(m) | FlashError::Target(m) => f.write_str(m),
            FlashError::Guard { message, .. } => f.write_str(message),
            FlashError::Pipe { step, error } => write!(f, "PICOBOOT {step} failed: {error}"),
            FlashError::Device { step, status } => write!(
                f,
                "PICOBOOT {step} refused by the boot ROM: status {status} ({})",
                status_name(*status)
            ),
            FlashError::Protocol { step, what } => {
                write!(f, "PICOBOOT {step}: unexpected answer ({what})")
            }
            FlashError::Deadline { step } => write!(
                f,
                "PICOBOOT flash took longer than {} s; stopped before the {step}",
                FLASH_DEADLINE.as_secs()
            ),
            FlashError::Mismatch { addr } => write!(
                f,
                "PICOBOOT read-back differs from the image at 0x{addr:08x}; the board was not \
                 rebooted and is still in BOOTSEL"
            ),
        }
    }
}

impl From<FlashError> for DspiError {
    fn from(e: FlashError) -> Self {
        match e {
            FlashError::Image(_) | FlashError::Target(_) => {
                DspiError::InvalidArgument { msg: e.to_string() }
            }
            FlashError::Guard {
                decision: FirmwareGuardDecision::Refused,
                message,
            } => DspiError::InvalidArgument { msg: message },
            FlashError::Guard { message, .. } => DspiError::FirmwareGuardBlocked { msg: message },
            _ => DspiError::Transfer { msg: e.to_string() },
        }
    }
}

// ── The plan ───────────────────────────────────────────────────────────

/// What will be written: whole sectors, by address, inside the window.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FlashPlan {
    pub(crate) platform: i8,
    pub(crate) firmware_version: String,
    /// Sector address and its 4096 bytes (`0xFF` where the image has none).
    pub(crate) sectors: Vec<(u32, Vec<u8>)>,
}

impl std::fmt::Debug for FlashPlan {
    // Never the image's bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlashPlan")
            .field("platform", &self.platform)
            .field("firmware_version", &self.firmware_version)
            .field("sectors", &self.sectors.len())
            .finish()
    }
}

impl FlashPlan {
    /// Runs of contiguous sectors, at most [`ERASE_RUN_MAX`] each:
    /// `(first sector address, sector count, index of the first in sectors)`.
    pub(crate) fn erase_runs(&self) -> Vec<(u32, u32, usize)> {
        let mut runs: Vec<(u32, u32, usize)> = Vec::new();
        for (i, (addr, _)) in self.sectors.iter().enumerate() {
            if let Some(last) = runs.last_mut() {
                let next = last
                    .1
                    .checked_mul(SECTOR)
                    .and_then(|n| last.0.checked_add(n));
                if next == Some(*addr) && last.1 < ERASE_RUN_MAX {
                    last.1 = last.1.saturating_add(1);
                    continue;
                }
            }
            runs.push((*addr, 1, i));
        }
        runs
    }
}

/// Group pages into whole sectors, each inside the platform's XIP window.
pub(crate) fn sector_plan(
    pages: &[Page],
    platform: i8,
) -> std::result::Result<Vec<(u32, Vec<u8>)>, FlashError> {
    let Some((lo, hi)) = flash_window_of(platform) else {
        return Err(FlashError::Target(
            "the chip of this image is not known".into(),
        ));
    };
    let mut sectors: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for page in pages {
        let base = page.addr & !(SECTOR - 1);
        let inside = base >= lo && base.checked_add(SECTOR).is_some_and(|end| end <= hi);
        if !inside {
            return Err(FlashError::Image(format!(
                "the image writes 0x{:08x}, outside the chip's flash",
                page.addr
            )));
        }
        let buf = sectors
            .entry(base)
            .or_insert_with(|| vec![0xFF; SECTOR_LEN]);
        let off = usize::try_from(page.addr.saturating_sub(base)).unwrap_or(usize::MAX);
        let Some(slot) = off
            .checked_add(page.data.len())
            .and_then(|end| buf.get_mut(off..end))
        else {
            return Err(FlashError::Image(format!(
                "the page at 0x{:08x} does not fit its sector",
                page.addr
            )));
        };
        slot.copy_from_slice(&page.data);
    }
    Ok(sectors.into_iter().collect())
}

/// Every check before a byte goes out (module docs): the image, the guard
/// with nothing typed, the target's chip, the window. Pure.
pub(crate) fn prepare(
    image: &[u8],
    target: &BootDevice,
    board: &FirmwareGuardBoard,
) -> std::result::Result<FlashPlan, FlashError> {
    let info = uf2_inspect(image.to_vec());
    if !info.valid {
        return Err(FlashError::Image(format!(
            "this is not a firmware file: {}",
            info.problem
        )));
    }
    let verdict = firmware_guard_check(image.to_vec(), board.clone(), String::new());
    if verdict.decision != FirmwareGuardDecision::Clear || !verdict.allowed {
        return Err(FlashError::Guard {
            decision: verdict.decision,
            message: verdict.message,
        });
    }
    if !matches!(target.platform, PLATFORM_RP2040 | PLATFORM_RP2350) {
        return Err(FlashError::Target(
            "the board in BOOTSEL is not an RP2040 or RP2350".into(),
        ));
    }
    if info.platform != target.platform {
        return Err(FlashError::Target(format!(
            "this firmware is for the {}, but the board in BOOTSEL is an {}",
            chip(info.platform),
            chip(target.platform)
        )));
    }
    let pages = uf2_pages(&info, image);
    if pages.is_empty() {
        return Err(FlashError::Image("the image has no flash pages".into()));
    }
    Ok(FlashPlan {
        platform: info.platform,
        firmware_version: info.firmware_version,
        sectors: sector_plan(&pages, info.platform)?,
    })
}

fn chip(platform: i8) -> &'static str {
    match platform {
        PLATFORM_RP2040 => "RP2040",
        PLATFORM_RP2350 => "RP2350",
        _ => "unknown chip",
    }
}

// ── The protocol ───────────────────────────────────────────────────────

/// One flash's conversation with the ROM: the token counter and the clock.
struct Conversation<'a, P: BootPipe + ?Sized> {
    pipe: &'a mut P,
    token: u32,
    /// Time since the flash began.
    elapsed: &'a dyn Fn() -> Duration,
}

impl<P: BootPipe + ?Sized> Conversation<'_, P> {
    /// The transfer deadline, cut to what is left of the whole flash's.
    fn timeout(&self, step: Step) -> std::result::Result<Duration, FlashError> {
        let left = FLASH_DEADLINE.saturating_sub((self.elapsed)());
        if left.is_zero() {
            return Err(FlashError::Deadline { step });
        }
        Ok(left.min(TRANSFER_DEADLINE))
    }

    /// `PICOBOOT_IF_RESET`, with the halts cleared first.
    fn reset(&mut self, timeout: Duration) -> std::result::Result<(), PipeError> {
        self.pipe.clear_halts()?;
        self.pipe.control_out(IF_RESET, timeout)
    }

    /// The interface reset after a failure, so the board is left answering.
    fn recover(&mut self, step: Step) {
        if let Err(e) = self.reset(RECOVERY_DEADLINE) {
            log::warn!("picoboot: the interface reset after the failed {step} failed too: {e}");
        }
    }

    /// After a failed transfer: the ROM's reason when it stalled, then the
    /// interface reset. Never retries.
    fn fail(&mut self, step: Step, error: PipeError) -> FlashError {
        let status = if error == PipeError::Stall {
            self.pipe
                .control_in(IF_CMD_STATUS, STATUS_LEN, RECOVERY_DEADLINE)
                .ok()
                .and_then(|raw| CmdStatus::decode(&raw))
        } else {
            None
        };
        if error != PipeError::Disconnected {
            self.recover(step);
        }
        match status {
            Some(s) => FlashError::Device {
                step,
                status: s.status_code,
            },
            None => FlashError::Pipe { step, error },
        }
    }

    /// Send `cmd` with its data phase (`out` for a write, the returned bytes
    /// for a read) and take the acknowledgement.
    fn run(
        &mut self,
        cmd: &Command,
        out: Option<&[u8]>,
    ) -> std::result::Result<Vec<u8>, FlashError> {
        let step = cmd.step;
        let timeout = self.timeout(step)?;
        let token = self.token;
        self.token = self.token.wrapping_add(1);
        let raw = encode_command(token, cmd.cmd_id, &cmd.args, cmd.transfer_length);
        if let Err(e) = self.pipe.bulk_out(&raw, timeout) {
            return Err(self.fail(step, e));
        }
        let want = usize::try_from(cmd.transfer_length).unwrap_or(usize::MAX);
        let mut data = Vec::new();
        if want > 0 {
            let timeout = self.timeout(step)?;
            if cmd.is_in() {
                match self.pipe.bulk_in(want, timeout) {
                    Ok(d) if d.len() == want => data = d,
                    Ok(d) => {
                        let what = format!("{} of {want} bytes", d.len());
                        self.recover(step);
                        return Err(FlashError::Protocol { step, what });
                    }
                    Err(e) => return Err(self.fail(step, e)),
                }
            } else {
                let payload = out.unwrap_or_default();
                if payload.len() != want {
                    return Err(FlashError::Protocol {
                        step,
                        what: format!("{} bytes to send for {want}", payload.len()),
                    });
                }
                if let Err(e) = self.pipe.bulk_out(payload, timeout) {
                    return Err(self.fail(step, e));
                }
            }
        }
        // The acknowledgement, in the opposite direction (picotool: one byte
        // OUT after an IN command, a zero-length packet IN otherwise).
        let timeout = self.timeout(step)?;
        let ack = if cmd.is_in() {
            self.pipe.bulk_out(&[0], timeout).map(|()| Vec::new())
        } else {
            self.pipe.bulk_in(1, timeout)
        };
        match ack {
            Ok(a) if a.is_empty() => Ok(data),
            Ok(a) => {
                self.recover(step);
                Err(FlashError::Protocol {
                    step,
                    what: format!("an acknowledgement of {} bytes", a.len()),
                })
            }
            // The board rebooting as it acknowledges is the normal ending.
            Err(PipeError::Disconnected) if step == Step::Reboot => Ok(data),
            Err(e) => Err(self.fail(step, e)),
        }
    }
}

/// Run a prepared flash over `pipe` (module docs, steps 1 to 6). `elapsed`
/// is the time since the flash began (the whole-flash deadline).
pub(crate) fn execute<P: BootPipe + ?Sized>(
    pipe: &mut P,
    plan: &FlashPlan,
    target: &BootDevice,
    elapsed: &dyn Fn() -> Duration,
) -> std::result::Result<FlashReport, FlashError> {
    let mut c = Conversation {
        pipe,
        token: 1,
        elapsed,
    };
    let timeout = c.timeout(Step::Reset)?;
    if let Err(error) = c.reset(timeout) {
        return Err(FlashError::Pipe {
            step: Step::Reset,
            error,
        });
    }
    c.run(&Command::exclusive_and_eject(), None)?;
    c.run(&Command::exit_xip(), None)?;
    for (first, count, index) in plan.erase_runs() {
        c.run(&Command::erase(first, count.saturating_mul(SECTOR)), None)?;
        let run = plan
            .sectors
            .iter()
            .skip(index)
            .take(usize::try_from(count).unwrap_or(0));
        for (addr, data) in run {
            c.run(&Command::write(*addr, SECTOR), Some(data))?;
        }
    }
    for (addr, data) in &plan.sectors {
        let back = c.run(&Command::read(*addr, SECTOR), None)?;
        if let Some(i) = back.iter().zip(data).position(|(a, b)| a != b) {
            let at = addr.saturating_add(u32::try_from(i).unwrap_or(0));
            return Err(FlashError::Mismatch { addr: at });
        }
    }
    let reboot = if plan.platform == PLATFORM_RP2040 {
        Command::reboot_rp2040()
    } else {
        Command::reboot2_normal()
    };
    c.run(&reboot, None)?;
    let sectors = u32::try_from(plan.sectors.len()).unwrap_or(u32::MAX);
    Ok(FlashReport {
        platform: plan.platform,
        serial: target.serial.clone(),
        firmware_version: plan.firmware_version.clone(),
        sectors,
        bytes_written: u64::from(sectors).saturating_mul(u64::from(SECTOR)),
        elapsed_ms: u64::try_from(elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// [`prepare`] then [`execute`]: nothing reaches `pipe` unless every check
/// passed. (The API, [`DspiCore::picoboot_flash_with`], runs the two itself
/// with the pipe opened between them.)
#[cfg(test)]
pub(crate) fn picoboot_flash<P: BootPipe + ?Sized>(
    pipe: &mut P,
    image: &[u8],
    target: &BootDevice,
    board: &FirmwareGuardBoard,
    elapsed: &dyn Fn() -> Duration,
) -> std::result::Result<FlashReport, FlashError> {
    let plan = prepare(image, target, board)?;
    execute(pipe, &plan, target, elapsed)
}

// ── The API ────────────────────────────────────────────────────────────

/// Boards in BOOTSEL mode with a PICOBOOT interface (`2E8A:0003` RP2040,
/// `2E8A:000F` RP2350). Empty where the bus cannot be enumerated (Android,
/// and a build without the `usb` feature: iOS, the web).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn picoboot_devices() -> Vec<BootDevice> {
    #[cfg(all(
        feature = "usb",
        any(target_os = "linux", target_os = "macos", windows)
    ))]
    {
        usb::list()
    }
    #[cfg(not(all(
        feature = "usb",
        any(target_os = "linux", target_os = "macos", windows)
    )))]
    {
        Vec::new()
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Flash `image` into the boot ROM of `target` (one of
    /// [`picoboot_devices`]) over PICOBOOT, with no drive mounted (module
    /// docs). `board` is the record [`DspiCore::firmware_guard_board`] read
    /// before the board entered BOOTSEL; the guard must say clear for it with
    /// nothing typed, so an image that would ask a person anything is
    /// refused. Takes a [`ConfirmOp::FirmwareFlash`] token as
    /// [`DspiCore::flash_confirmed`] does (mint it once the board is in
    /// BOOTSEL; it works with no device open), and the installer lock holds.
    /// Blocks for up to 60 s.
    ///
    /// `Ok` means every sector was written and read back identical and the
    /// reboot was acknowledged; the caller confirms by the DSPi coming back
    /// with the image's version. On an error after the first erase the board
    /// stays in BOOTSEL (nothing is retried).
    ///
    /// Errors: [`DspiError::ConfirmationRequired`] without a good token;
    /// [`DspiError::FirmwareGuardBlocked`] or [`DspiError::InvalidArgument`]
    /// for an image the guard does not clear, of another chip, or not
    /// firmware; [`DspiError::Unsupported`] where the bus cannot be reached;
    /// [`DspiError::Transfer`] naming the step for anything the transfer or
    /// the ROM did. In every error before the first command nothing was sent.
    pub fn picoboot_flash_confirmed(
        &self,
        image: Vec<u8>,
        target: BootDevice,
        board: FirmwareGuardBoard,
        token: Option<String>,
    ) -> Result<FlashReport> {
        self.picoboot_flash_with(&image, &target, &board, token.as_deref(), open_pipe)
    }
}

impl DspiCore {
    /// [`DspiCore::picoboot_flash_confirmed`] with the pipe's opener passed
    /// in (tests give a fake). The lock and the token are checked first, then
    /// every check of [`prepare`], and only then is the pipe opened.
    pub(crate) fn picoboot_flash_with<P: BootPipe>(
        &self,
        image: &[u8],
        target: &BootDevice,
        board: &FirmwareGuardBoard,
        token: Option<&str>,
        open: impl FnOnce(&BootDevice) -> Result<P>,
    ) -> Result<FlashReport> {
        crate::limits::lock_check_any(&mut self.lock())?;
        let _ticket = self.confirm_gate(ConfirmOp::FirmwareFlash, token)?;
        let plan = prepare(image, target, board)?;
        let mut pipe = open(target)?;
        let start = dspi_transport::clock::Instant::now();
        let elapsed = move || start.elapsed();
        let report = execute(&mut pipe, &plan, target, &elapsed);
        match &report {
            Ok(r) => log::info!(
                "picoboot: flashed {} sectors of {} into the {} at {} in {} ms",
                r.sectors,
                if r.firmware_version.is_empty() {
                    "an unversioned image"
                } else {
                    &r.firmware_version
                },
                chip(r.platform),
                target.bus_location,
                r.elapsed_ms
            ),
            Err(e) => log::warn!("picoboot: flash at {} failed: {e}", target.bus_location),
        }
        Ok(report?)
    }
}

/// The real pipe for `target`, or `Unsupported` where there is no bus.
#[cfg(all(
    feature = "usb",
    any(target_os = "linux", target_os = "macos", windows)
))]
fn open_pipe(target: &BootDevice) -> Result<usb::NusbPipe> {
    usb::open(target)
}

/// No bus here: nothing can be opened.
#[cfg(not(all(
    feature = "usb",
    any(target_os = "linux", target_os = "macos", windows)
)))]
fn open_pipe(_target: &BootDevice) -> Result<NoPipe> {
    Err(DspiError::Unsupported)
}

/// The pipe type of a build without a bus; never constructed.
#[cfg(not(all(
    feature = "usb",
    any(target_os = "linux", target_os = "macos", windows)
)))]
enum NoPipe {}

#[cfg(not(all(
    feature = "usb",
    any(target_os = "linux", target_os = "macos", windows)
)))]
impl BootPipe for NoPipe {
    fn bulk_out(&mut self, _: &[u8], _: Duration) -> std::result::Result<(), PipeError> {
        match *self {}
    }
    fn bulk_in(&mut self, _: usize, _: Duration) -> std::result::Result<Vec<u8>, PipeError> {
        match *self {}
    }
    fn control_in(
        &mut self,
        _: u8,
        _: u16,
        _: Duration,
    ) -> std::result::Result<Vec<u8>, PipeError> {
        match *self {}
    }
    fn control_out(&mut self, _: u8, _: Duration) -> std::result::Result<(), PipeError> {
        match *self {}
    }
    fn clear_halts(&mut self) -> std::result::Result<(), PipeError> {
        match *self {}
    }
}

// ── nusb ───────────────────────────────────────────────────────────────

#[cfg(all(
    feature = "usb",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub(crate) mod usb {
    //! The PICOBOOT interface over nusb. Claims only the vendor interface
    //! (class `0xFF` with one bulk OUT and one bulk IN endpoint), never the
    //! boot ROM's mass-storage interface.

    use std::time::Duration;

    use nusb::MaybeFuture;
    use nusb::transfer::{
        Buffer, Bulk, ControlIn, ControlOut, ControlType, In, Out, Recipient, TransferError,
    };

    use super::{BootDevice, BootPipe, PLATFORM_RP2040, PLATFORM_RP2350, PipeError};
    use crate::{DspiError, Result};

    const RPI_VID: u16 = 0x2E8A;
    const RP2040_BOOT_PID: u16 = 0x0003;
    const RP2350_BOOT_PID: u16 = 0x000F;
    const VENDOR_CLASS: u8 = 0xFF;
    /// A bulk endpoint's transfer-type bits in `bmAttributes`.
    const BULK: u8 = 0x02;

    fn platform_of(pid: u16) -> Option<i8> {
        match pid {
            RP2040_BOOT_PID => Some(PLATFORM_RP2040),
            RP2350_BOOT_PID => Some(PLATFORM_RP2350),
            _ => None,
        }
    }

    fn location(d: &nusb::DeviceInfo) -> String {
        let ports: Vec<String> = d.port_chain().iter().map(u8::to_string).collect();
        format!("{}-{}", d.bus_id(), ports.join("."))
    }

    fn record(d: &nusb::DeviceInfo, platform: i8) -> BootDevice {
        BootDevice {
            platform,
            // The RP2040 ROM's serial is the same on every board.
            serial: (platform == PLATFORM_RP2350)
                .then(|| d.serial_number().map(str::to_string))
                .flatten(),
            bus_location: location(d),
        }
    }

    fn boot_roms() -> Vec<(nusb::DeviceInfo, i8)> {
        match nusb::list_devices().wait() {
            Ok(devices) => devices
                .filter(|d| d.vendor_id() == RPI_VID)
                .filter_map(|d| platform_of(d.product_id()).map(|p| (d, p)))
                .collect(),
            Err(e) => {
                log::debug!("picoboot: USB enumeration failed: {e}");
                Vec::new()
            }
        }
    }

    /// Every boot ROM on the bus.
    pub(crate) fn list() -> Vec<BootDevice> {
        boot_roms().iter().map(|(d, p)| record(d, *p)).collect()
    }

    fn usb(msg: String) -> DspiError {
        DspiError::Transfer { msg }
    }

    /// Open `target`: exactly one boot ROM at its location, of its chip and
    /// (RP2350) with its serial; its PICOBOOT interface claimed.
    pub(crate) fn open(target: &BootDevice) -> Result<NusbPipe> {
        let matching: Vec<nusb::DeviceInfo> = boot_roms()
            .into_iter()
            .filter(|(d, p)| *p == target.platform && record(d, *p) == *target)
            .map(|(d, _)| d)
            .collect();
        let info = match matching.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(usb(format!(
                    "no board in BOOTSEL at {} any more",
                    target.bus_location
                )));
            }
            _ => {
                return Err(usb(format!(
                    "more than one board in BOOTSEL matches {}",
                    target.bus_location
                )));
            }
        };
        let device = info
            .open()
            .wait()
            .map_err(|e| usb(format!("could not open the board in BOOTSEL: {e}")))?;
        let config = device
            .active_configuration()
            .map_err(|e| usb(format!("could not read the boot ROM's configuration: {e}")))?;
        let mut found = None;
        for iface in config.interfaces() {
            let alt = iface.first_alt_setting();
            if alt.class() != VENDOR_CLASS || alt.num_endpoints() != 2 {
                continue;
            }
            let bulk: Vec<u8> = alt
                .endpoints()
                .filter(|e| e.attributes() & 0x03 == BULK)
                .map(|e| e.address())
                .collect();
            let out = bulk.iter().copied().find(|a| a & 0x80 == 0);
            let inp = bulk.iter().copied().find(|a| a & 0x80 != 0);
            if let (Some(out), Some(inp)) = (out, inp) {
                found = Some((iface.interface_number(), out, inp));
                break;
            }
        }
        let Some((number, out_addr, in_addr)) = found else {
            return Err(usb("the boot ROM has no PICOBOOT interface".into()));
        };
        let interface = device
            .claim_interface(number)
            .wait()
            .map_err(|e| usb(format!("could not claim the PICOBOOT interface: {e}")))?;
        let out = interface
            .endpoint::<Bulk, Out>(out_addr)
            .map_err(|e| usb(format!("PICOBOOT OUT endpoint: {e}")))?;
        let inp = interface
            .endpoint::<Bulk, In>(in_addr)
            .map_err(|e| usb(format!("PICOBOOT IN endpoint: {e}")))?;
        Ok(NusbPipe {
            interface,
            number,
            out,
            inp,
        })
    }

    /// A claimed PICOBOOT interface.
    pub(crate) struct NusbPipe {
        interface: nusb::Interface,
        number: u8,
        out: nusb::Endpoint<Bulk, Out>,
        inp: nusb::Endpoint<Bulk, In>,
    }

    fn map(e: TransferError) -> PipeError {
        match e {
            TransferError::Cancelled => PipeError::Timeout,
            TransferError::Stall => PipeError::Stall,
            TransferError::Disconnected => PipeError::Disconnected,
            other => PipeError::Other(other.to_string()),
        }
    }

    impl BootPipe for NusbPipe {
        fn bulk_out(
            &mut self,
            data: &[u8],
            timeout: Duration,
        ) -> std::result::Result<(), PipeError> {
            let done = self
                .out
                .transfer_blocking(Buffer::from(data.to_vec()), timeout);
            done.status.map_err(map)?;
            if done.actual_len != data.len() {
                return Err(PipeError::Other(format!(
                    "sent {} of {} bytes",
                    done.actual_len,
                    data.len()
                )));
            }
            Ok(())
        }

        fn bulk_in(
            &mut self,
            len: usize,
            timeout: Duration,
        ) -> std::result::Result<Vec<u8>, PipeError> {
            // nusb wants a whole number of packets.
            let mps = self.inp.max_packet_size().max(1);
            let request = len.max(1).div_ceil(mps).saturating_mul(mps);
            let done = self.inp.transfer_blocking(Buffer::new(request), timeout);
            done.status.map_err(map)?;
            let mut v = done.buffer.into_vec();
            v.truncate(done.actual_len);
            Ok(v)
        }

        fn control_in(
            &mut self,
            request: u8,
            len: u16,
            timeout: Duration,
        ) -> std::result::Result<Vec<u8>, PipeError> {
            self.interface
                .control_in(
                    ControlIn {
                        control_type: ControlType::Vendor,
                        recipient: Recipient::Interface,
                        request,
                        value: 0,
                        index: u16::from(self.number),
                        length: len,
                    },
                    timeout,
                )
                .wait()
                .map_err(map)
        }

        fn control_out(
            &mut self,
            request: u8,
            timeout: Duration,
        ) -> std::result::Result<(), PipeError> {
            self.interface
                .control_out(
                    ControlOut {
                        control_type: ControlType::Vendor,
                        recipient: Recipient::Interface,
                        request,
                        value: 0,
                        index: u16::from(self.number),
                        data: &[],
                    },
                    timeout,
                )
                .wait()
                .map_err(map)
        }

        fn clear_halts(&mut self) -> std::result::Result<(), PipeError> {
            let out = self.out.clear_halt().wait();
            let inp = self.inp.clear_halt().wait();
            out.and(inp).map_err(|e| PipeError::Other(e.to_string()))
        }
    }
}

#[cfg(test)]
#[path = "picoboot/tests_wp_p3pb.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p3pb;
