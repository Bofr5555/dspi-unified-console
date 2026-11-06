//! Firmware files and the bootloader: check a `.uf2` before flashing, find
//! the board's bootloader drive, copy the image to it.
//!
//! The Qt app does the drive work in C++ today (`FirmwareUpdater.cpp`) and the
//! Android app in Kotlin; this gives every core-based app (Qt, Terminal,
//! later iOS for the check only) one implementation. Entering the bootloader
//! is `DspiCore::enter_bootloader` (WP C6).
//!
//! # What makes a UF2 acceptable
//!
//! The rules come from the UF2 spec (github.com/microsoft/uf2), the RP2350
//! datasheet §5.5.2/§5.5.3 (family ids, flash windows) and the Android app's
//! validator (`Uf2.kt`). Every block must carry both start magics and the end
//! magic, a family id, and a 256-byte page at a page-aligned address inside
//! the chip's XIP flash window; the blocks must all name the same family and
//! block count, be numbered `0..num_blocks` in file order, and all be there.
//! The one exception is the RP2350-E10 workaround block picotool puts at the
//! front of RP2350 images (absolute family, block 0 of 2, a page of `0xEF`):
//! it is recognised the way picotool's `check_abs_block` does and kept out of
//! the count.
//!
//! The DSPi version is read from the image's picotool binary info
//! (`bi_program_version_string`), not guessed from strings.
//!
//! Owned by WP C11 (docs/core-merge/PLAN.md).

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::hardware::version_matches;
use crate::records::{BootloaderCounts, BootloaderDrive, Uf2Info};
use crate::{DspiCore, DspiError, Result};

// ── UF2 format ─────────────────────────────────────────────────────────

const BLOCK_SIZE: usize = 512;
/// Payload bytes the RP2040/RP2350 boot ROMs take per block.
const PAGE_SIZE: u32 = 256;
/// The most payload a 512-byte UF2 block can hold.
const MAX_PAYLOAD: u32 = 476;
const MAGIC_START0: u32 = 0x0A32_4655;
const MAGIC_START1: u32 = 0x9E5D_5157;
const MAGIC_END: u32 = 0x0AB1_6F30;

const FLAG_NOT_MAIN_FLASH: u32 = 0x0000_0001;
const FLAG_FILE_CONTAINER: u32 = 0x0000_1000;
const FLAG_FAMILY_ID_PRESENT: u32 = 0x0000_2000;
const FLAG_EXTENSION_TAGS_PRESENT: u32 = 0x0000_8000;
/// picotool `UF2_EXTENSION_RP2_IGNORE_BLOCK`: size 4, tag 0x9957e3.
const EXTENSION_RP2_IGNORE_BLOCK: u32 = 0x9957_E304;

const FAMILY_RP2040: u32 = 0xE48B_FF56;
const FAMILY_ABSOLUTE: u32 = 0xE48B_FF57;
const FAMILY_RP2350_ARM_S: u32 = 0xE48B_FF59;
const FAMILY_RP2350_RISCV: u32 = 0xE48B_FF5A;
const FAMILY_RP2350_ARM_NS: u32 = 0xE48B_FF5B;

/// Start of the XIP flash window on both chips.
const XIP_BASE: u32 = 0x1000_0000;
/// RP2040: 16 MB window.
const RP2040_FLASH_WINDOW: u32 = 0x0100_0000;
/// RP2350: 32 MB window.
const RP2350_FLASH_WINDOW: u32 = 0x0200_0000;
/// A UF2 of the whole RP2350 window plus the E10 block; anything bigger is
/// not firmware.
const MAX_FILE_SIZE: usize = (RP2350_FLASH_WINDOW / PAGE_SIZE) as usize * BLOCK_SIZE + BLOCK_SIZE;

/// `Uf2Info::platform` / `BootloaderDrive::platform` values.
const PLATFORM_RP2040: i8 = 0;
const PLATFORM_RP2350: i8 = 1;
const PLATFORM_UNKNOWN: i8 = -1;

/// The bootloader drive's identity file.
const INFO_FILE: &str = "INFO_UF2.TXT";
/// What the image is written as.
const IMAGE_FILE: &str = "firmware.uf2";
/// Volume labels of the two boot ROMs (a second board of the same kind may
/// get a suffix, e.g. `RPI-RP21`).
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
const VOLUME_LABELS: [&str; 2] = ["RPI-RP2", "RP2350"];

/// USB ids of the boot ROMs in BOOTSEL mode.
#[cfg(all(feature = "usb", not(target_os = "android")))]
const RPI_VID: u16 = 0x2E8A;
#[cfg(all(feature = "usb", not(target_os = "android")))]
const RP2040_BOOT_PID: u16 = 0x0003;
#[cfg(all(feature = "usb", not(target_os = "android")))]
const RP2350_BOOT_PID: u16 = 0x000F;

fn family_name(family: u32) -> String {
    match family {
        FAMILY_RP2040 => "rp2040".into(),
        FAMILY_ABSOLUTE => "absolute".into(),
        FAMILY_RP2350_ARM_S => "rp2350-arm-s".into(),
        FAMILY_RP2350_RISCV => "rp2350-riscv".into(),
        FAMILY_RP2350_ARM_NS => "rp2350-arm-ns".into(),
        other => format!("0x{other:08x}"),
    }
}

/// The chip a family boots on. Absolute-family images are RP2350-only: the
/// RP2040 ROM takes nothing but its own family.
fn family_platform(family: u32) -> Option<i8> {
    match family {
        FAMILY_RP2040 => Some(PLATFORM_RP2040),
        FAMILY_ABSOLUTE | FAMILY_RP2350_ARM_S | FAMILY_RP2350_RISCV | FAMILY_RP2350_ARM_NS => {
            Some(PLATFORM_RP2350)
        }
        _ => None,
    }
}

fn platform_name(platform: i8) -> &'static str {
    match platform {
        PLATFORM_RP2040 => "RP2040",
        PLATFORM_RP2350 => "RP2350",
        _ => "unknown chip",
    }
}

fn flash_window(platform: i8) -> u32 {
    if platform == PLATFORM_RP2040 {
        RP2040_FLASH_WINDOW
    } else {
        RP2350_FLASH_WINDOW
    }
}

fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let raw: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// One 512-byte UF2 block, header decoded.
struct Block<'a> {
    flags: u32,
    target: u32,
    payload_size: u32,
    block_no: u32,
    num_blocks: u32,
    family: u32,
    /// The 476-byte data area.
    data: &'a [u8],
}

impl<'a> Block<'a> {
    /// Decode a block; `None` when a magic is wrong (or the slice is short).
    fn parse(raw: &'a [u8]) -> Option<Self> {
        if le_u32(raw, 0)? != MAGIC_START0
            || le_u32(raw, 4)? != MAGIC_START1
            || le_u32(raw, 508)? != MAGIC_END
        {
            return None;
        }
        Some(Block {
            flags: le_u32(raw, 8)?,
            target: le_u32(raw, 12)?,
            payload_size: le_u32(raw, 16)?,
            block_no: le_u32(raw, 20)?,
            num_blocks: le_u32(raw, 24)?,
            family: le_u32(raw, 28)?,
            data: raw.get(32..508)?,
        })
    }

    /// The first `payload_size` bytes of the data area (all of it if the
    /// size is out of range; callers check the size first).
    fn payload(&self) -> &'a [u8] {
        let n = usize::try_from(self.payload_size).unwrap_or(usize::MAX);
        self.data.get(..n).unwrap_or(self.data)
    }

    /// picotool `check_abs_block`: the RP2350-E10 workaround block (absolute
    /// family, a page of `0xEF`, block 0 of 2, optional RP2_IGNORE_BLOCK
    /// extension). The ROM uses it to pick the flash partition; it is not
    /// part of the image.
    fn is_e10_block(&self) -> bool {
        let extension_ok = if self.flags & FLAG_EXTENSION_TAGS_PRESENT != 0 {
            le_u32(self.data, PAGE_SIZE as usize) == Some(EXTENSION_RP2_IGNORE_BLOCK)
        } else {
            true
        };
        self.payload_size == PAGE_SIZE
            && self.flags & !FLAG_EXTENSION_TAGS_PRESENT == FLAG_FAMILY_ID_PRESENT
            && self.num_blocks == 2
            && self.block_no == 0
            && self.family == FAMILY_ABSOLUTE
            && self.payload().iter().all(|&b| b == 0xEF)
            && extension_ok
    }
}

/// A fully checked image: its family and its flash pages by address.
struct Image<'a> {
    family: u32,
    platform: i8,
    num_blocks: u32,
    /// (target address, 256-byte payload), sorted by address.
    pages: Vec<(u32, &'a [u8])>,
}

/// Why an image was refused, and the family if one was seen (0 if not).
struct Refusal {
    problem: String,
    family: u32,
}

fn refuse(family: Option<u32>, problem: String) -> Refusal {
    Refusal {
        problem,
        family: family.unwrap_or(0),
    }
}

/// The full structural check (module docs). Never panics.
fn validate(bytes: &[u8]) -> std::result::Result<Image<'_>, Refusal> {
    if bytes.is_empty() {
        return Err(refuse(None, "the file is empty".into()));
    }
    if bytes.len() > MAX_FILE_SIZE {
        return Err(refuse(
            None,
            "the file is too large to be firmware for an RP2040 or RP2350".into(),
        ));
    }
    if !bytes.len().is_multiple_of(BLOCK_SIZE) {
        return Err(refuse(
            None,
            "the file is truncated: it is not a whole number of 512-byte UF2 blocks".into(),
        ));
    }

    let mut family: Option<u32> = None;
    let mut num_blocks: u32 = 0;
    let mut e10_seen = false;
    let mut pages: Vec<(u32, &[u8])> = Vec::new();
    let mut addresses = BTreeSet::new();

    for (index, raw) in bytes.as_chunks::<BLOCK_SIZE>().0.iter().enumerate() {
        let n = index.saturating_add(1);
        let Some(block) = Block::parse(raw) else {
            return Err(refuse(
                family,
                format!("block {n} is not a UF2 block (wrong magic number)"),
            ));
        };
        if block.flags & FLAG_FILE_CONTAINER != 0 {
            return Err(refuse(
                family,
                format!("block {n} is part of a file container, not firmware"),
            ));
        }
        if block.flags & FLAG_NOT_MAIN_FLASH != 0 {
            return Err(refuse(
                family,
                format!("block {n} is not meant for the main flash"),
            ));
        }
        if block.flags & FLAG_FAMILY_ID_PRESENT == 0 {
            return Err(refuse(
                family,
                format!(
                    "block {n} names no chip family (a legacy UF2); rebuild it with a current SDK"
                ),
            ));
        }
        if block.payload_size > MAX_PAYLOAD {
            return Err(refuse(
                family,
                format!(
                    "block {n} claims {} payload bytes; a UF2 block holds at most {MAX_PAYLOAD}",
                    block.payload_size
                ),
            ));
        }
        if block.payload_size != PAGE_SIZE {
            return Err(refuse(
                family,
                format!(
                    "block {n} carries {} bytes; the boot ROM takes {PAGE_SIZE}",
                    block.payload_size
                ),
            ));
        }
        let Some(block_platform) = family_platform(block.family) else {
            return Err(refuse(
                family.or(Some(block.family)),
                format!(
                    "block {n} is for chip family {}, not an RP2040 or RP2350",
                    family_name(block.family)
                ),
            ));
        };
        let window_end = XIP_BASE.saturating_add(flash_window(block_platform));
        let in_flash = block.target % PAGE_SIZE == 0
            && block.target >= XIP_BASE
            && block
                .target
                .checked_add(PAGE_SIZE)
                .is_some_and(|end| end <= window_end);
        if !in_flash {
            return Err(refuse(
                family.or(Some(block.family)),
                format!(
                    "block {n} targets 0x{:08x}, outside the {} flash",
                    block.target,
                    platform_name(block_platform)
                ),
            ));
        }

        if block.is_e10_block() {
            if e10_seen {
                return Err(refuse(family, "it contains two RP2350-E10 blocks".into()));
            }
            e10_seen = true;
            continue;
        }

        match family {
            None => {
                family = Some(block.family);
                num_blocks = block.num_blocks;
                if num_blocks == 0 {
                    return Err(refuse(
                        family,
                        format!("block {n} says the image has no blocks"),
                    ));
                }
            }
            Some(f) if f != block.family => {
                return Err(refuse(
                    family,
                    format!(
                        "it mixes firmware for different chips ({} and {})",
                        family_name(f),
                        family_name(block.family)
                    ),
                ));
            }
            Some(_) if block.num_blocks != num_blocks => {
                return Err(refuse(
                    family,
                    format!(
                        "its blocks disagree about the image size ({num_blocks} and {})",
                        block.num_blocks
                    ),
                ));
            }
            Some(_) => {}
        }
        if block.block_no >= num_blocks {
            return Err(refuse(
                family,
                format!(
                    "block {n} is numbered {} of {num_blocks}, outside the image",
                    block.block_no
                ),
            ));
        }
        let expected = u32::try_from(pages.len()).unwrap_or(u32::MAX);
        if block.block_no != expected {
            return Err(refuse(
                family,
                format!(
                    "block {n} is out of order (number {} where {expected} was expected)",
                    block.block_no
                ),
            ));
        }
        if !addresses.insert(block.target) {
            return Err(refuse(
                family,
                format!("two blocks write to 0x{:08x}", block.target),
            ));
        }
        pages.push((block.target, block.payload()));
    }

    let Some(family) = family else {
        return Err(refuse(None, "it contains no firmware blocks".into()));
    };
    let platform = family_platform(family).unwrap_or(PLATFORM_UNKNOWN);
    if e10_seen && platform != PLATFORM_RP2350 {
        return Err(refuse(
            Some(family),
            "it puts an RP2350-only block into an RP2040 image".into(),
        ));
    }
    let have = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    if have != num_blocks {
        return Err(refuse(
            Some(family),
            format!("it is incomplete ({have} of {num_blocks} blocks)"),
        ));
    }
    pages.sort_unstable_by_key(|&(addr, _)| addr);
    Ok(Image {
        family,
        platform,
        num_blocks,
        pages,
    })
}

// ── Binary info (the version string) ──────────────────────────────────

const BINARY_INFO_MARKER_START: u32 = 0x7188_EBF2;
const BINARY_INFO_MARKER_END: u32 = 0xE71A_A390;
const BINARY_INFO_TYPE_ID_AND_STRING: u16 = 6;
/// `BINARY_INFO_MAKE_TAG('R', 'P')`.
const BINARY_INFO_TAG_RP: u16 = 0x5052;
const BINARY_INFO_ID_RP_PROGRAM_NAME: u32 = 0x0203_1C86;
const BINARY_INFO_ID_RP_PROGRAM_VERSION_STRING: u32 = 0x11A9_BC3A;
/// The header sits near the start of the image (after boot2 on the RP2040).
const BINARY_INFO_SEARCH: u32 = 4096;
/// Bounds on what an image may make us walk.
const BINARY_INFO_MAX_ENTRIES: u32 = 512;
const BINARY_INFO_MAX_MAPPINGS: u32 = 16;
const BINARY_INFO_MAX_STRING: u32 = 128;

impl Image<'_> {
    fn byte(&self, addr: u32) -> Option<u8> {
        let page = addr & !(PAGE_SIZE - 1);
        let i = self.pages.binary_search_by_key(&page, |&(a, _)| a).ok()?;
        let (_, data) = self.pages.get(i)?;
        data.get(usize::try_from(addr.checked_sub(page)?).ok()?)
            .copied()
    }

    fn u32_at(&self, addr: u32) -> Option<u32> {
        let mut raw = [0u8; 4];
        for (k, b) in (0u32..).zip(raw.iter_mut()) {
            *b = self.byte(addr.checked_add(k)?)?;
        }
        Some(u32::from_le_bytes(raw))
    }

    fn u16_at(&self, addr: u32) -> Option<u16> {
        Some(u16::from_le_bytes([
            self.byte(addr)?,
            self.byte(addr.checked_add(1)?)?,
        ]))
    }

    /// A NUL-terminated printable ASCII string.
    fn c_string(&self, addr: u32) -> Option<String> {
        let mut s = String::new();
        for k in 0..BINARY_INFO_MAX_STRING {
            let b = self.byte(addr.checked_add(k)?)?;
            if b == 0 {
                return Some(s);
            }
            if !(0x20..0x7F).contains(&b) {
                return None;
            }
            s.push(char::from(b));
        }
        None
    }

    /// An `ID_AND_STRING` entry with the `RP` tag at `q`: `(id, text)`.
    fn id_and_string(
        &self,
        q: u32,
        to_flash: &dyn Fn(u32) -> Option<u32>,
    ) -> Option<(u32, String)> {
        if self.u16_at(q)? != BINARY_INFO_TYPE_ID_AND_STRING
            || self.u16_at(q.checked_add(2)?)? != BINARY_INFO_TAG_RP
        {
            return None;
        }
        let id = self.u32_at(q.checked_add(4)?)?;
        let text = self.c_string(to_flash(self.u32_at(q.checked_add(8)?)?)?)?;
        Some((id, text))
    }

    /// `(program name, program version)` from the picotool binary info.
    fn binary_info(&self) -> Option<(Option<String>, Option<String>)> {
        let lo = self.pages.first()?.0;
        let header = (0..BINARY_INFO_SEARCH / 4)
            .filter_map(|k| lo.checked_add(k.checked_mul(4)?))
            .find(|&a| {
                self.u32_at(a) == Some(BINARY_INFO_MARKER_START)
                    && a.checked_add(16).and_then(|e| self.u32_at(e))
                        == Some(BINARY_INFO_MARKER_END)
            })?;
        let start = self.u32_at(header.checked_add(4)?)?;
        let end = self.u32_at(header.checked_add(8)?)?;
        let map = self.u32_at(header.checked_add(12)?)?;

        // The copy-to-RAM table: (source in flash, dest start, dest end).
        let mut mappings = Vec::new();
        for k in 0..BINARY_INFO_MAX_MAPPINGS {
            let entry = k
                .checked_mul(12)
                .and_then(|o| map.checked_add(o))
                .and_then(|at| {
                    Some((
                        self.u32_at(at)?,
                        self.u32_at(at.checked_add(4)?)?,
                        self.u32_at(at.checked_add(8)?)?,
                    ))
                });
            match entry {
                Some((0, _, _)) | None => break,
                Some(m) => mappings.push(m),
            }
        }
        let to_flash = |p: u32| -> Option<u32> {
            if self.byte(p).is_some() {
                return Some(p);
            }
            mappings
                .iter()
                .find(|&&(_, ds, de)| ds <= p && p < de)
                .and_then(|&(src, ds, _)| src.checked_add(p.checked_sub(ds)?))
        };

        let mut name = None;
        let mut version = None;
        let count = end.saturating_sub(start) / 4;
        for k in 0..count.min(BINARY_INFO_MAX_ENTRIES) {
            let entry = k
                .checked_mul(4)
                .and_then(|o| start.checked_add(o))
                .and_then(to_flash)
                .and_then(|slot| self.u32_at(slot))
                .and_then(to_flash)
                .and_then(|q| self.id_and_string(q, &to_flash));
            match entry {
                Some((BINARY_INFO_ID_RP_PROGRAM_NAME, t)) => name = Some(t),
                Some((BINARY_INFO_ID_RP_PROGRAM_VERSION_STRING, t)) => version = Some(t),
                _ => {}
            }
        }
        Some((name, version))
    }

    /// The DSPi version (`1.1.6-beta3-16-g557bce7`, without the leading
    /// `v`), or "" when the image is not DSPi or carries no version.
    fn dspi_version(&self) -> String {
        match self.binary_info() {
            Some((Some(name), Some(version))) if name == "DSPi" => {
                version.strip_prefix('v').unwrap_or(&version).to_string()
            }
            _ => String::new(),
        }
    }
}

fn inspect(bytes: &[u8]) -> Uf2Info {
    match validate(bytes) {
        Ok(image) => Uf2Info {
            valid: true,
            problem: String::new(),
            family_id: image.family,
            platform: image.platform,
            num_blocks: image.num_blocks,
            payload_bytes: u32::try_from(image.pages.len())
                .unwrap_or(u32::MAX)
                .saturating_mul(PAGE_SIZE),
            firmware_version: image.dspi_version(),
        },
        Err(refusal) => Uf2Info {
            valid: false,
            problem: refusal.problem,
            family_id: refusal.family,
            platform: family_platform(refusal.family).unwrap_or(PLATFORM_UNKNOWN),
            num_blocks: 0,
            payload_bytes: 0,
            firmware_version: String::new(),
        },
    }
}

/// Check a UF2 image: block structure, magic numbers, family id, the flash
/// range it covers, and the DSPi version string inside it. Never panics; a
/// refused image comes back with `valid == false` and the reason in
/// `problem` (fit for "This is not a firmware file: <problem>").
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn uf2_inspect(image: Vec<u8>) -> Uf2Info {
    inspect(&image)
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Whether the open device runs firmware `version`: the leading
    /// `major.minor.patch[-betaN]` of a UF2's `Uf2Info::firmware_version`
    /// (`1.1.6-beta3-16-g557bce7` compares as 1.1.6 beta 3), against the
    /// mirror's `fw_*` from `GET_PLATFORM`. The last step of a firmware
    /// update, after the device re-enumerated and was opened again. False
    /// for a string that does not start with a version; an early 1.1.6 beta
    /// that does not report its number matches betas 1 and 2.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open.
    pub fn firmware_version_matches(&self, version: &str) -> Result<bool> {
        let mut guard = self.lock();
        guard.conn()?;
        Ok(version_matches(&guard.state, version))
    }
}

// ── Bootloader on the bus ──────────────────────────────────────────────

/// Boards in BOOTSEL mode on the bus (USB `2E8A:0003` RP2040, `2E8A:000F`
/// RP2350). 0/0 where the bus cannot be enumerated (Android, and a build
/// without the `usb` feature: iOS).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn bootloader_counts() -> BootloaderCounts {
    #[cfg(any(not(feature = "usb"), target_os = "android"))]
    {
        BootloaderCounts::default()
    }
    #[cfg(all(feature = "usb", not(target_os = "android")))]
    {
        use nusb::MaybeFuture;
        let mut counts = BootloaderCounts::default();
        match nusb::list_devices().wait() {
            Ok(devices) => {
                for d in devices.filter(|d| d.vendor_id() == RPI_VID) {
                    match d.product_id() {
                        RP2040_BOOT_PID => counts.rp2040 = counts.rp2040.saturating_add(1),
                        RP2350_BOOT_PID => counts.rp2350 = counts.rp2350.saturating_add(1),
                        _ => {}
                    }
                }
            }
            Err(e) => log::debug!("bootloader_counts: USB enumeration failed: {e}"),
        }
        counts
    }
}

// ── Bootloader drives ──────────────────────────────────────────────────

/// `INFO_UF2.TXT` at `root`, at most 4 KiB of it; `None` if unreadable.
fn read_info(root: &Path) -> Option<String> {
    let mut text = Vec::new();
    File::open(root.join(INFO_FILE))
        .ok()?
        .take(4096)
        .read_to_end(&mut text)
        .ok()?;
    Some(String::from_utf8_lossy(&text).into_owned())
}

/// The chip of a boot ROM's `INFO_UF2.TXT`, from its `Board-ID` and `Model`
/// lines (RP2040: `RPI-RP2` / `Raspberry Pi RP2`; RP2350: `RP2350` /
/// `Raspberry Pi RP2350`). `None` for anything else, including another
/// vendor's UF2 bootloader and a file whose two lines disagree.
fn info_platform(info: &str) -> Option<i8> {
    let mut lines = info.lines().map(str::trim);
    if !lines.next()?.starts_with("UF2 Bootloader") {
        return None;
    }
    let mut board = None;
    let mut model = None;
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            match key.trim() {
                "Board-ID" => board = Some(value.trim()),
                "Model" => model = Some(value.trim()),
                _ => {}
            }
        }
    }
    let by_board = board.map(|b| match b {
        "RPI-RP2" => Some(PLATFORM_RP2040),
        "RP2350" => Some(PLATFORM_RP2350),
        _ => None,
    });
    let by_model = model.map(|m| match m {
        "Raspberry Pi RP2" => Some(PLATFORM_RP2040),
        "Raspberry Pi RP2350" => Some(PLATFORM_RP2350),
        _ => None,
    });
    match (by_board, by_model) {
        (Some(b), Some(m)) if b == m => b,
        (Some(b), None) => b,
        (None, Some(m)) => m,
        _ => None,
    }
}

/// The drive at `root`, if it is an RP2040/RP2350 boot ROM's.
fn drive_at(root: &Path) -> Option<BootloaderDrive> {
    let info = read_info(root)?;
    let platform = info_platform(&info)?;
    Some(BootloaderDrive {
        path: root.to_string_lossy().into_owned(),
        platform,
        info,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn has_volume_label(name: &str) -> bool {
    VOLUME_LABELS.iter().any(|l| name.starts_with(l))
}

/// Undo the octal escapes of `/proc/self/mountinfo` (`\040` for a space).
#[cfg(any(target_os = "linux", test))]
fn unescape_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        let octal = bytes
            .get(i.saturating_add(1)..i.saturating_add(4))
            .filter(|d| b == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c)))
            .and_then(|d| u8::from_str_radix(std::str::from_utf8(d).ok()?, 8).ok());
        match octal {
            Some(v) => {
                out.push(v);
                i = i.saturating_add(4);
            }
            None => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Mount points in a mountinfo table that may be a boot ROM drive: FAT
/// file systems, and anything mounted under a boot ROM's volume label. Read
/// from the kernel's table, which touches no file system (a board that
/// rebooted mid-write can leave a dead mount whose `stat` blocks).
///
/// This deliberately widens the net: the label match is a basename prefix
/// (`RPI-RP2`, `RP2350`, so `RP2350 1` and `RPI-RP21` qualify) and every FAT
/// mount is included. It only picks places to look; the `INFO_UF2.TXT`
/// filter in [`drive_at`] decides what is a boot ROM drive.
#[cfg(any(target_os = "linux", test))]
fn mountinfo_candidates(table: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in table.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(dash) = fields.iter().position(|&f| f == "-") else {
            continue;
        };
        let (Some(mount), Some(fstype)) = (fields.get(4), fields.get(dash.saturating_add(1)))
        else {
            continue;
        };
        let mount = PathBuf::from(unescape_mount_field(mount));
        let labelled = mount
            .file_name()
            .is_some_and(|n| has_volume_label(&n.to_string_lossy()));
        if labelled || matches!(*fstype, "vfat" | "msdos" | "fat") {
            out.push(mount);
        }
    }
    out
}

/// Entries of `dir` named after a boot ROM volume label (a basename-prefix
/// heuristic, like [`mountinfo_candidates`]; `INFO_UF2.TXT` decides).
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn labelled_entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| has_volume_label(&e.file_name().to_string_lossy()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = std::fs::read_to_string("/proc/self/mountinfo")
        .map(|t| mountinfo_candidates(&t))
        .unwrap_or_default();
    // udisks mounts under /run/media/<user> (or /media/<user>); covered by
    // mountinfo unless this process runs in another mount namespace.
    for base in ["/run/media", "/media"] {
        if let Ok(users) = std::fs::read_dir(base) {
            for user in users.filter_map(|e| e.ok()) {
                roots.extend(labelled_entries(&user.path()));
            }
        }
    }
    roots
}

#[cfg(target_os = "macos")]
fn candidate_roots() -> Vec<PathBuf> {
    // macOS mounts the drive itself under /Volumes by its label.
    labelled_entries(Path::new("/Volumes"))
}

#[cfg(windows)]
fn candidate_roots() -> Vec<PathBuf> {
    win::removable_drives_with_labels(&VOLUME_LABELS)
}

/// Android goes through the Storage Access Framework (the Kotlin app);
/// other systems have no known mount convention.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn candidate_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// Mounted bootloader drives (`INFO_UF2.TXT` at the root naming an RP2040 or
/// RP2350 boot ROM): Linux `/proc/self/mountinfo` + `/run/media`, macOS
/// `/Volumes`, Windows removable drives labelled `RPI-RP2` / `RP2350`.
/// Empty on Android.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn bootloader_drives() -> Vec<BootloaderDrive> {
    let mut drives: Vec<BootloaderDrive> = Vec::new();
    for root in candidate_roots() {
        if let Some(d) = drive_at(&root)
            && !drives.iter().any(|o| o.path == d.path)
        {
            drives.push(d);
        }
    }
    drives
}

// ── Writing ────────────────────────────────────────────────────────────

/// Bytes free for an unprivileged writer on the file system holding `path`;
/// `None` where it cannot be asked.
#[cfg(unix)]
fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: an all-zero statvfs is a valid value of this plain C struct.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c_path is NUL-terminated and outlives the call; st is a valid,
    // writable statvfs.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    // The field types differ between platforms (u32/u64); both fit u64.
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    let (avail, frsize) = (st.f_bavail as u64, st.f_frsize as u64);
    avail.checked_mul(frsize)
}

#[cfg(windows)]
fn free_space(path: &Path) -> Option<u64> {
    win::free_space(path)
}

#[cfg(not(any(unix, windows)))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

fn io(msg: String) -> DspiError {
    DspiError::Io { msg }
}

/// Refuse up front when the drive cannot hold the image. Unknown free space
/// does not block the write (the write itself would fail).
fn check_space(free: Option<u64>, needed: usize, drive: &str) -> Result<()> {
    let needed = u64::try_from(needed).unwrap_or(u64::MAX);
    match free {
        Some(free) if free < needed => Err(io(format!(
            "the bootloader drive has {free} bytes free; the image needs {needed}"
        ))),
        Some(_) => Ok(()),
        None => {
            log::warn!("flash_to_drive: free space on {drive} unknown");
            Ok(())
        }
    }
}

/// Write `image` to `drive` as `firmware.uf2`, synced to the medium.
///
/// Before any byte goes out: the image is fully validated, the drive's
/// `INFO_UF2.TXT` is read again and must still name the board `drive`
/// describes, the image's family must be one that board boots, and the
/// drive must have room for the image. The file is then written with one
/// `write_all` and one `sync_all`.
///
/// The board reboots into the new firmware as it takes the last block, so
/// the sync can fail because the drive has gone: that (sync failed and the
/// drive's `INFO_UF2.TXT` is no longer readable) counts as success, logged
/// at warn level. It cannot be told apart from a cable pulled mid-write, so
/// `Ok` means only "every byte was handed to the drive". **The caller must
/// confirm success** by waiting for the DSPi to re-enumerate and report the
/// image's version (`Uf2Info::firmware_version`); a timeout there is the
/// failure.
///
/// The `INFO_UF2.TXT` re-check and the write are separate path-based
/// operations, so a different drive mounted at the same path in between
/// would get the file (an inherent check-then-use race; the Qt updater has
/// the same). The window is a few milliseconds and both boot ROMs reject an
/// image of the other family.
///
/// Errors: [`DspiError::InvalidArgument`] for an invalid image, a drive that
/// is not a boot ROM's, or a family mismatch; [`DspiError::Io`] for a drive
/// that has gone, is short of space, or fails the write.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn flash_to_drive(image: Vec<u8>, drive: BootloaderDrive) -> Result<()> {
    let info = inspect(&image);
    if !info.valid {
        return Err(DspiError::InvalidArgument {
            msg: format!("this is not a firmware file: {}", info.problem),
        });
    }
    let root = Path::new(&drive.path);

    // Checked again right before writing: the drive record may be stale
    // (a different board, or no board, at that path now) or made up.
    let now = read_info(root)
        .ok_or_else(|| io(format!("the bootloader drive at {} is gone", drive.path)))?;
    let board = info_platform(&now).ok_or_else(|| DspiError::InvalidArgument {
        msg: format!(
            "{} is not the drive of an RP2040 or RP2350 bootloader",
            drive.path
        ),
    })?;
    if board != drive.platform {
        return Err(DspiError::InvalidArgument {
            msg: format!(
                "the drive at {} now belongs to an {}, not the {} that was chosen",
                drive.path,
                platform_name(board),
                platform_name(drive.platform)
            ),
        });
    }
    if info.platform != board {
        return Err(DspiError::InvalidArgument {
            msg: format!(
                "this firmware is for the {} ({}), but the board is an {}",
                platform_name(info.platform),
                family_name(info.family_id),
                platform_name(board)
            ),
        });
    }
    check_space(free_space(root), image.len(), &drive.path)?;

    let dest = root.join(IMAGE_FILE);
    let mut file = File::create(&dest).map_err(|e| {
        io(format!(
            "could not open {} for writing: {e}",
            dest.display()
        ))
    })?;
    file.write_all(&image)
        .map_err(|e| io(format!("writing the firmware failed: {e}")))?;
    match file.sync_all() {
        Ok(()) => Ok(()),
        // The board took the last block and rebooted: the normal ending.
        // Indistinguishable from a pulled cable; the caller confirms.
        Err(e) if read_info(root).is_none() => {
            log::warn!(
                "flash_to_drive: {} bytes written to {}, drive gone during sync ({e}); \
                 assuming the board rebooted, the caller must confirm",
                image.len(),
                drive.path
            );
            Ok(())
        }
        Err(e) => Err(io(format!("writing the firmware failed: {e}"))),
    }
}

// ── Windows drive letters ──────────────────────────────────────────────

#[cfg(windows)]
mod win {
    //! The few kernel32 calls the drive search needs, declared here rather
    //! than pulling in windows-sys for four functions. std links kernel32 on
    //! every Windows target.
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    const DRIVE_REMOVABLE: u32 = 2;
    const LABEL_LEN: u32 = 261;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_len: u32,
            serial: *mut u32,
            max_component: *mut u32,
            flags: *mut u32,
            fs_name: *mut u16,
            fs_name_len: u32,
        ) -> i32;
        fn GetDiskFreeSpaceExW(
            dir: *const u16,
            avail_to_caller: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Removable drive roots (`E:\`) whose volume label starts with one of
    /// `labels`. Only the label is asked for, so no file system is read
    /// until `INFO_UF2.TXT` is.
    pub(super) fn removable_drives_with_labels(labels: &[&str]) -> Vec<PathBuf> {
        // SAFETY: no arguments; returns a bit mask.
        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();
        for (bit, letter) in (0u32..).zip(b'A'..=b'Z') {
            if mask.checked_shr(bit).unwrap_or(0) & 1 == 0 {
                continue;
            }
            let root = format!("{}:\\", char::from(letter));
            let root_w = wide(std::ffi::OsStr::new(&root));
            // SAFETY: root_w is a NUL-terminated UTF-16 string.
            if unsafe { GetDriveTypeW(root_w.as_ptr()) } != DRIVE_REMOVABLE {
                continue;
            }
            let mut name = [0u16; LABEL_LEN as usize];
            // SAFETY: root_w is NUL-terminated; name is a writable buffer of
            // LABEL_LEN units; the null out-pointers are optional.
            let ok = unsafe {
                GetVolumeInformationW(
                    root_w.as_ptr(),
                    name.as_mut_ptr(),
                    LABEL_LEN,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            };
            if ok == 0 {
                continue;
            }
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            let label = std::ffi::OsString::from_wide(name.get(..len).unwrap_or(&[]));
            let label = label.to_string_lossy();
            if labels.iter().any(|l| label.starts_with(l)) {
                out.push(PathBuf::from(root));
            }
        }
        out
    }

    pub(super) fn free_space(path: &Path) -> Option<u64> {
        let dir = wide(path.as_os_str());
        let mut avail = 0u64;
        // SAFETY: dir is NUL-terminated; avail is writable; the other
        // out-pointers are optional.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                dir.as_ptr(),
                &mut avail,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(avail)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── Building test images ──

    fn block(
        flags: u32,
        target: u32,
        payload: &[u8],
        block_no: u32,
        num_blocks: u32,
        family: u32,
    ) -> Vec<u8> {
        let mut b = vec![0u8; BLOCK_SIZE];
        for (off, v) in [
            (0, MAGIC_START0),
            (4, MAGIC_START1),
            (8, flags),
            (12, target),
            (16, payload.len() as u32),
            (20, block_no),
            (24, num_blocks),
            (28, family),
            (508, MAGIC_END),
        ] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[32..32 + payload.len()].copy_from_slice(payload);
        b
    }

    /// A flash image: `flash` laid out from XIP_BASE in 256-byte pages.
    fn uf2(flash: &[u8], family: u32) -> Vec<u8> {
        let pages: Vec<&[u8]> = flash.chunks(256).collect();
        let n = pages.len() as u32;
        let mut out = Vec::new();
        for (i, p) in pages.iter().enumerate() {
            let mut page = [0u8; 256];
            page[..p.len()].copy_from_slice(p);
            out.extend(block(
                FLAG_FAMILY_ID_PRESENT,
                XIP_BASE + 256 * i as u32,
                &page,
                i as u32,
                n,
                family,
            ));
        }
        out
    }

    /// picotool's RP2350-E10 block (with the RP2_IGNORE_BLOCK extension).
    fn e10_block() -> Vec<u8> {
        let mut b = block(
            FLAG_FAMILY_ID_PRESENT | FLAG_EXTENSION_TAGS_PRESENT,
            0x10FF_FF00,
            &[0xEF; 256],
            0,
            2,
            FAMILY_ABSOLUTE,
        );
        b[32 + 256..32 + 260].copy_from_slice(&EXTENSION_RP2_IGNORE_BLOCK.to_le_bytes());
        b
    }

    /// A flash image with a binary-info header at 0x100, a table of two
    /// entries, and the program name and version strings; the version
    /// entry lives in "RAM" and is reached through the mapping table, as the
    /// SDK lays out its data.
    fn dspi_flash(name: &str, version: &str) -> Vec<u8> {
        let mut f = vec![0u8; 0x800];
        let put =
            |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
        let a = |off: usize| XIP_BASE + off as u32;
        // header
        put(&mut f, 0x100, BINARY_INFO_MARKER_START);
        put(&mut f, 0x104, a(0x200)); // entries start
        put(&mut f, 0x108, a(0x208)); // entries end (2 entries)
        put(&mut f, 0x10C, a(0x180)); // mapping table
        put(&mut f, 0x110, BINARY_INFO_MARKER_END);
        // mapping: RAM 0x20000000..0x20000100 is copied from flash 0x700
        put(&mut f, 0x180, a(0x700));
        put(&mut f, 0x184, 0x2000_0000);
        put(&mut f, 0x188, 0x2000_0100);
        // entry pointers: name in flash, version in RAM
        put(&mut f, 0x200, a(0x300));
        put(&mut f, 0x204, 0x2000_0010);
        // name entry
        f[0x300..0x302].copy_from_slice(&BINARY_INFO_TYPE_ID_AND_STRING.to_le_bytes());
        f[0x302..0x304].copy_from_slice(&BINARY_INFO_TAG_RP.to_le_bytes());
        put(&mut f, 0x304, BINARY_INFO_ID_RP_PROGRAM_NAME);
        put(&mut f, 0x308, a(0x400));
        // version entry, at RAM 0x20000010 = flash 0x710
        f[0x710..0x712].copy_from_slice(&BINARY_INFO_TYPE_ID_AND_STRING.to_le_bytes());
        f[0x712..0x714].copy_from_slice(&BINARY_INFO_TAG_RP.to_le_bytes());
        put(&mut f, 0x714, BINARY_INFO_ID_RP_PROGRAM_VERSION_STRING);
        put(&mut f, 0x718, a(0x500));
        f[0x400..0x400 + name.len()].copy_from_slice(name.as_bytes());
        f[0x500..0x500 + version.len()].copy_from_slice(version.as_bytes());
        f
    }

    fn rp2040_image() -> Vec<u8> {
        uf2(&dspi_flash("DSPi", "v1.1.6-beta4"), FAMILY_RP2040)
    }

    fn rp2350_image() -> Vec<u8> {
        let mut out = e10_block();
        out.extend(uf2(
            &dspi_flash("DSPi", "v1.1.6-beta4"),
            FAMILY_RP2350_ARM_S,
        ));
        out
    }

    fn set_u32(image: &mut [u8], block: usize, off: usize, v: u32) {
        let at = block * BLOCK_SIZE + off;
        image[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    // ── uf2_inspect ──

    #[test]
    fn a_dspi_image_inspects_as_valid() {
        let info = uf2_inspect(rp2040_image());
        assert_eq!(info.problem, "");
        assert!(info.valid);
        assert_eq!(info.family_id, FAMILY_RP2040);
        assert_eq!(info.platform, 0);
        assert_eq!(info.num_blocks, 8);
        assert_eq!(info.payload_bytes, 8 * 256);
        assert_eq!(info.firmware_version, "1.1.6-beta4");
    }

    #[test]
    fn wp_c11_rp2350_image_with_e10_block_is_valid() {
        let info = uf2_inspect(rp2350_image());
        assert!(info.valid, "{}", info.problem);
        assert_eq!(info.family_id, FAMILY_RP2350_ARM_S);
        assert_eq!(info.platform, 1);
        assert_eq!(info.num_blocks, 8);
        assert_eq!(info.firmware_version, "1.1.6-beta4");
    }

    #[test]
    fn wp_c11_other_rp2350_families_are_rp2350() {
        for fam in [FAMILY_RP2350_RISCV, FAMILY_RP2350_ARM_NS, FAMILY_ABSOLUTE] {
            let info = uf2_inspect(uf2(&[1u8; 512], fam));
            assert!(info.valid, "{}", info.problem);
            assert_eq!(info.platform, 1);
            assert_eq!(info.firmware_version, "");
        }
    }

    #[test]
    fn wp_c11_non_dspi_program_has_no_version() {
        let info = uf2_inspect(uf2(&dspi_flash("blink", "v9.9.9"), FAMILY_RP2040));
        assert!(info.valid);
        assert_eq!(info.firmware_version, "");
    }

    /// The images the Qt app bundles (both must be in the tree).
    #[test]
    fn wp_c11_bundled_images_inspect_as_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/qt/firmware");
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{} is missing: {e}", dir.display()));
        let mut seen = 0;
        for e in entries.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".uf2") {
                continue;
            }
            let info = uf2_inspect(std::fs::read(e.path()).unwrap());
            assert!(info.valid, "{name}: {}", info.problem);
            let expect = if name.contains("RP2350") { 1 } else { 0 };
            assert_eq!(info.platform, expect, "{name}");
            assert!(info.firmware_version.starts_with("1."), "{name}: {info:?}");
            seen += 1;
        }
        assert!(
            seen >= 2,
            "expected the RP2040 and RP2350 images in {}",
            dir.display()
        );
    }

    fn problem(image: Vec<u8>) -> String {
        let info = uf2_inspect(image);
        assert!(!info.valid);
        assert_eq!(info.num_blocks, 0);
        info.problem
    }

    #[test]
    fn wp_c11_truncated_files_are_refused() {
        let image = rp2040_image();
        assert!(problem(image[..image.len() - 100].to_vec()).contains("truncated"));
        // whole blocks, but one missing
        assert!(problem(image[..image.len() - BLOCK_SIZE].to_vec()).contains("incomplete"));
        assert!(problem(Vec::new()).contains("empty"));
    }

    #[test]
    fn wp_c11_wrong_family_is_refused() {
        let p = problem(uf2(&[0u8; 512], 0xADA5_2840)); // nRF52840
        assert!(p.contains("not an RP2040 or RP2350"), "{p}");
        let info = uf2_inspect(uf2(&[0u8; 512], 0xADA5_2840));
        assert_eq!(info.family_id, 0xADA5_2840);
        assert_eq!(info.platform, -1);
        // mixed families
        let mut image = uf2(&[0u8; 512], FAMILY_RP2040);
        set_u32(&mut image, 1, 28, FAMILY_RP2350_ARM_S);
        assert!(problem(image).contains("mixes"));
        // an E10 block in an RP2040 image
        let mut image = e10_block();
        image.extend(rp2040_image());
        assert!(problem(image).contains("RP2350-only"));
    }

    #[test]
    fn wp_c11_block_out_of_order_is_refused() {
        let image = rp2040_image();
        let mut swapped = image.clone();
        swapped[..BLOCK_SIZE].copy_from_slice(&image[BLOCK_SIZE..2 * BLOCK_SIZE]);
        swapped[BLOCK_SIZE..2 * BLOCK_SIZE].copy_from_slice(&image[..BLOCK_SIZE]);
        assert!(problem(swapped).contains("out of order"));
        // numbered past the end
        let mut image = rp2040_image();
        set_u32(&mut image, 2, 20, 8);
        assert!(problem(image).contains("outside the image"));
        // disagreeing counts
        let mut image = rp2040_image();
        set_u32(&mut image, 3, 24, 9);
        assert!(problem(image).contains("disagree"));
    }

    #[test]
    fn wp_c11_bad_magics_are_refused() {
        for off in [0, 4, 508] {
            let mut image = rp2040_image();
            set_u32(&mut image, 3, off, 0);
            assert!(problem(image).contains("block 4 is not a UF2 block"));
        }
    }

    #[test]
    fn wp_c11_bad_payloads_and_addresses_are_refused() {
        let mut image = rp2040_image();
        set_u32(&mut image, 0, 16, 477);
        assert!(problem(image).contains("at most 476"));
        let mut image = rp2040_image();
        set_u32(&mut image, 0, 16, 476);
        assert!(problem(image).contains("the boot ROM takes 256"));
        // RAM image
        let mut image = rp2040_image();
        set_u32(&mut image, 1, 12, 0x2000_0000);
        assert!(problem(image).contains("outside the RP2040 flash"));
        // past the RP2040's 16 MB, fine for the RP2350's 32 MB
        let mut image = uf2(&[0u8; 512], FAMILY_RP2040);
        set_u32(&mut image, 1, 12, 0x1100_0000);
        assert!(problem(image).contains("outside the RP2040 flash"));
        let mut image = uf2(&[0u8; 512], FAMILY_RP2350_ARM_S);
        set_u32(&mut image, 1, 12, 0x1100_0000);
        assert!(uf2_inspect(image).valid);
        // unaligned, duplicate, wrapping
        let mut image = rp2040_image();
        set_u32(&mut image, 1, 12, XIP_BASE + 0x80);
        assert!(problem(image).contains("outside"));
        let mut image = rp2040_image();
        set_u32(&mut image, 1, 12, XIP_BASE);
        assert!(problem(image).contains("two blocks write"));
        let mut image = rp2040_image();
        set_u32(&mut image, 1, 12, 0xFFFF_FF00);
        assert!(problem(image).contains("outside"));
    }

    #[test]
    fn wp_c11_flags_are_checked() {
        let mut image = rp2040_image();
        set_u32(&mut image, 0, 8, 0); // legacy: no family id
        assert!(problem(image).contains("legacy"));
        let mut image = rp2040_image();
        set_u32(
            &mut image,
            0,
            8,
            FLAG_FAMILY_ID_PRESENT | FLAG_NOT_MAIN_FLASH,
        );
        assert!(problem(image).contains("main flash"));
        let mut image = rp2040_image();
        set_u32(
            &mut image,
            0,
            8,
            FLAG_FAMILY_ID_PRESENT | FLAG_FILE_CONTAINER,
        );
        assert!(problem(image).contains("container"));
    }

    #[test]
    fn wp_c11_garbage_is_refused() {
        assert!(problem(vec![0xA5; 4096]).contains("not a UF2 block"));
        assert!(problem(b"hello".to_vec()).contains("truncated"));
    }

    proptest! {
        #[test]
        fn proptest_uf2_inspect_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let _ = uf2_inspect(bytes);
        }

        /// Random corruption of a valid image: never panics, and a refusal
        /// always says why.
        #[test]
        fn proptest_uf2_inspect_never_panics_on_mutated_images(
            edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
            rp2350 in any::<bool>(),
        ) {
            let mut image = if rp2350 { rp2350_image() } else { rp2040_image() };
            for (at, v) in edits {
                let i = at.index(image.len());
                image[i] = v;
            }
            let info = uf2_inspect(image);
            prop_assert!(info.valid || !info.problem.is_empty());
        }
    }

    // ── INFO_UF2.TXT and drives ──

    const INFO_RP2040: &str =
        "UF2 Bootloader v3.0\r\nModel: Raspberry Pi RP2\r\nBoard-ID: RPI-RP2\r\n";
    const INFO_RP2350: &str = "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n";

    #[test]
    fn wp_c11_info_file_names_the_chip() {
        assert_eq!(info_platform(INFO_RP2040), Some(0));
        assert_eq!(info_platform(INFO_RP2350), Some(1));
        assert_eq!(
            info_platform("UF2 Bootloader v1.0\nBoard-ID: RP2350\n"),
            Some(1)
        );
        // another vendor's UF2 bootloader
        assert_eq!(
            info_platform(
                "UF2 Bootloader 0.6.0\nModel: Adafruit Feather nRF52840\nBoard-ID: nRF52840-Feather\n"
            ),
            None
        );
        // Board-ID and Model disagree
        assert_eq!(
            info_platform("UF2 Bootloader v1.0\nModel: Raspberry Pi RP2\nBoard-ID: RP2350\n"),
            None
        );
        assert_eq!(
            info_platform("Model: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n"),
            None
        );
        assert_eq!(info_platform(""), None);
    }

    #[test]
    fn wp_c11_mountinfo_candidates() {
        let table = "\
36 35 98:0 /mnt1 /mnt/parent rw,noatime master:1 - ext3 /dev/root rw,errors=continue
100 29 8:17 / /run/media/bofr/RPI-RP2 rw,nosuid shared:52 - vfat /dev/sdb1 rw,uid=1000
101 29 8:33 / /run/media/bofr/RP2350\\0401 rw,nosuid shared:53 - fuseblk /dev/sdc1 rw
102 29 8:1 / /boot/efi rw,relatime shared:5 - vfat /dev/sda1 rw
garbage line
";
        let got = mountinfo_candidates(table);
        assert_eq!(
            got,
            vec![
                PathBuf::from("/run/media/bofr/RPI-RP2"),
                PathBuf::from("/run/media/bofr/RP2350 1"),
                PathBuf::from("/boot/efi"),
            ]
        );
        assert_eq!(unescape_mount_field("a\\040b\\134c\\9"), "a b\\c\\9");
    }

    #[test]
    fn wp_c11_bootloader_queries_do_not_panic() {
        // No USB devices or boot drives in the build container.
        let _ = bootloader_counts();
        let _ = bootloader_drives();
    }

    /// A directory for a test's files, beside the test binary in the target
    /// directory: the system's temp dir may be a tmpfs. Not created.
    fn test_dir(name: &str) -> PathBuf {
        std::env::current_exe()
            .unwrap()
            .with_file_name("test-scratch")
            .join(format!("{name}-{}", std::process::id()))
    }

    /// A fake drive: a fresh directory beside the test binary with an
    /// INFO_UF2.TXT.
    struct FakeDrive(PathBuf);

    impl FakeDrive {
        fn new(tag: &str, info: Option<&str>) -> Self {
            let dir = test_dir(&format!("wp-c11-{tag}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(info) = info {
                std::fs::write(dir.join(INFO_FILE), info).unwrap();
            }
            FakeDrive(dir)
        }

        fn record(&self, platform: i8) -> BootloaderDrive {
            BootloaderDrive {
                path: self.0.to_string_lossy().into_owned(),
                platform,
                info: String::new(),
            }
        }

        fn written(&self) -> Option<Vec<u8>> {
            std::fs::read(self.0.join(IMAGE_FILE)).ok()
        }
    }

    impl Drop for FakeDrive {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn wp_c11_drive_at_reads_the_info_file() {
        let d = FakeDrive::new("drive-at", Some(INFO_RP2350));
        let found = drive_at(&d.0).unwrap();
        assert_eq!(found.platform, 1);
        assert_eq!(found.info, INFO_RP2350);
        let other = FakeDrive::new("drive-at-other", Some("UF2 Bootloader\nBoard-ID: SAMD21\n"));
        assert!(drive_at(&other.0).is_none());
        let none = FakeDrive::new("drive-at-none", None);
        assert!(drive_at(&none.0).is_none());
    }

    #[test]
    fn wp_c11_labelled_entries() {
        let d = FakeDrive::new("labels", None);
        for name in ["RPI-RP2", "RP2350 1", "USBSTICK"] {
            std::fs::create_dir(d.0.join(name)).unwrap();
        }
        let mut got: Vec<String> = labelled_entries(&d.0)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        got.sort();
        assert_eq!(got, vec!["RP2350 1", "RPI-RP2"]);
    }

    #[test]
    fn wp_c11_flash_writes_the_image() {
        let d = FakeDrive::new("flash-ok", Some(INFO_RP2040));
        let image = rp2040_image();
        flash_to_drive(image.clone(), d.record(0)).unwrap();
        assert_eq!(d.written().unwrap(), image);

        let d = FakeDrive::new("flash-ok-2350", Some(INFO_RP2350));
        flash_to_drive(rp2350_image(), d.record(1)).unwrap();
        assert_eq!(d.written().unwrap(), rp2350_image());
    }

    #[test]
    fn wp_c11_flash_refuses_wrong_family() {
        let d = FakeDrive::new("flash-family", Some(INFO_RP2040));
        let err = flash_to_drive(rp2350_image(), d.record(0)).unwrap_err();
        assert!(
            matches!(err, DspiError::InvalidArgument { ref msg } if msg.contains("board is an RP2040")),
            "{err}"
        );
        assert!(d.written().is_none());
    }

    #[test]
    fn wp_c11_flash_refuses_invalid_image() {
        let d = FakeDrive::new("flash-invalid", Some(INFO_RP2040));
        let mut image = rp2040_image();
        image.truncate(image.len() - BLOCK_SIZE);
        let err = flash_to_drive(image, d.record(0)).unwrap_err();
        assert!(
            matches!(err, DspiError::InvalidArgument { ref msg } if msg.contains("not a firmware file")),
            "{err}"
        );
        assert!(d.written().is_none());
    }

    #[test]
    fn wp_c11_flash_rechecks_the_drive() {
        // The record says RP2040, the drive now says RP2350.
        let d = FakeDrive::new("flash-changed", Some(INFO_RP2350));
        let err = flash_to_drive(rp2040_image(), d.record(0)).unwrap_err();
        assert!(
            matches!(err, DspiError::InvalidArgument { ref msg } if msg.contains("now belongs")),
            "{err}"
        );
        assert!(d.written().is_none());
        // Not a boot ROM drive at all.
        let d = FakeDrive::new("flash-foreign", Some("UF2 Bootloader\nBoard-ID: SAMD21\n"));
        let err = flash_to_drive(rp2040_image(), d.record(0)).unwrap_err();
        assert!(matches!(err, DspiError::InvalidArgument { .. }), "{err}");
        assert!(d.written().is_none());
        // Gone.
        let d = FakeDrive::new("flash-gone", None);
        let err = flash_to_drive(rp2040_image(), d.record(0)).unwrap_err();
        assert!(
            matches!(err, DspiError::Io { ref msg } if msg.contains("gone")),
            "{err}"
        );
        assert!(d.written().is_none());
    }

    #[test]
    fn wp_c11_short_free_space_is_refused() {
        assert!(matches!(
            check_space(Some(1000), 1001, "X"),
            Err(DspiError::Io { ref msg }) if msg.contains("1000 bytes free")
        ));
        assert_eq!(check_space(Some(1001), 1001, "X"), Ok(()));
        assert_eq!(check_space(None, 1001, "X"), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn wp_c11_free_space_is_known_on_unix() {
        let d = FakeDrive::new("free", Some(INFO_RP2040));
        assert!(free_space(&d.0).is_some_and(|f| f > 0));
        assert!(free_space(Path::new("/nonexistent/wp-c11")).is_none());
    }
}
