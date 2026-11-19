//! The firmware guard: may this image go on this board without someone
//! accepting a core-voltage risk in writing?
//!
//! DSPi runs the RP2350 at 307.2 MHz, about twice its rating, and some parts
//! hang at the stock 1.15 V core voltage (upstream issue #83; the K340's PCB
//! unit: 5 hangs in about 9 minutes at 1.15 V, none in 41 at 1.20 V). A stock
//! image has no `core_vreg` code at all, so whatever a board has stored does
//! nothing and it runs at 1.15 V. Flashing one to such a board brings the
//! hangs back. Every updater calls this guard (QD11, TM3, AN18, SW-M2).
//!
//! # The rule
//!
//! **An at-risk board plus an unknown or lower image is blocked without typed
//! confirmation.** Each doubt resolves to "blocked":
//!
//! | Board ([`FirmwareGuardBoard::required_mv`]) | Image | Result |
//! |---|---|---|
//! | any | invalid | refused; confirmation does not help |
//! | any | stamped above 1200 and above the board's current voltage, or above 1200 on an unknown board | the word `OVERVOLT` |
//! | exactly 1150: not at risk | any other valid image | clear |
//! | above 1150: at risk | stamped at or above it | clear |
//! | above 1150: at risk | no stamp, or stamped lower | typed confirmation |
//! | anything else: unknown | any other valid image | typed confirmation |
//!
//! # Too high (WP CG14)
//!
//! The second row is `dspi_session::overvolt`, the rule the voltage requests
//! use, with the image's stamp as the target and `required_mv` as the current
//! voltage. It asks on a stock board too: a 1.25 V image on a 1.15 V board asks,
//! a 1.20 V image on a 1.15 V board does not. An image with no stamp counts as
//! 1.15 V, so it never asks; today's images have none. On an unknown board the
//! word takes the place of the phrase: one typed answer per image, and an image
//! above 1.20 V is not the too-low risk the phrase is about.
//!
//! **Accepted limit (owner):** a K340 already on stock firmware reports 1150 mV
//! and is treated as that. There is no record of what it used to be, by design:
//! the guard keeps no stored or per-serial record, and judges what the unit
//! reports now.
//!
//! # How a board is judged
//!
//! [`DspiCore::firmware_guard_board`] reads the device now rather than
//! trusting the connect probe, because `ExtState` turns a stall and a DSPi
//! Link refusal into the same empty state, and only the stall means stock:
//! - `EXT_CORE_VREG` read: `required_mv` is the highest of the build default,
//!   the persisted selection and the live voltage. A trial above the rest
//!   counts: it can only make the guard ask more often.
//! - `EXT_LIST` stalls or does not list `EXT_CORE_VREG`: the firmware has no
//!   `core_vreg`, but a build can still have a fixed higher voltage (the
//!   K340's early builds did), so the status block's live voltage decides.
//! - Anything else, including a board in the bootloader, no device, a refused
//!   or failed read, or a reply with values outside the known ranges, is
//!   unknown.
//!
//! Read the board **before** entering the bootloader, keep the record, and
//! pass it to the check on the way through; the board cannot be asked later.
//! The record speaks only for the board it was read from: for a drive that
//! was already mounted, use [`firmware_guard_board_in_bootloader`].
//!
//! # How an image is judged
//!
//! The only evidence is the build's stamp in the picotool binary info: an
//! `ID_AND_STRING` entry with tag `DS` and id [`STAMP_ID`], whose text is the
//! build's `DSPI_VREG_MV` (`1150`, `1200`, `1250` or `1300`), in an image whose
//! program name is `DSPi`. No stamp, another value, or stamps that disagree
//! all mean unknown, treated as 1.15 V. Today's images carry no stamp, so all
//! count as unknown; nothing here depends on that changing. File names
//! (`-vreg1200`), version strings and loose bytes are never believed. A
//! `core_vreg` marker without a voltage would prove nothing, since such a
//! build's default can still be 1.15 V. The firmware side, in `core_vreg.c`:
//!
//! ```c
//! #define STR2(x) #x
//! #define STR(x) STR2(x)
//! bi_decl(bi_string(BINARY_INFO_MAKE_TAG('D', 'S'), 0x0B7A4C15, STR(DSPI_VREG_MV)))
//! ```
//!
//! A stamped image boots at max(stored, its default) (`core_vreg.c:59-62`), so
//! a stored selection could hold even under a lower stamp. The guard gives no
//! credit for that: it only compares the stamp, and the stored value's survival
//! across an update is the image's business, not something to bet a board on.
//!
//! Owned by WP CG8 (docs/core-merge/PLAN-CLIENTS.md).

use std::collections::BTreeMap;

use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_session::ext::{self as sx, CoreVregStatus, ExtError};
use dspi_session::overvolt;
use dspi_transport::Transport;

use crate::consts::STAT_CORE_MV;
use crate::records::{BootloaderDrive, Uf2Info};
use crate::{DspiCore, DspiError, Result, flash_to_drive, uf2_inspect};

/// What must be typed, exactly, to go ahead with a blocked image. Not
/// localised: the point is that it is typed, and that every client asks for
/// the same words.
const CONFIRMATION_PHRASE: &str = "I ACCEPT THE CORE VOLTAGE RISK";

/// The stock core voltage, and what an image without a stamp is taken to run.
const STOCK_MV: u16 = 1150;
/// Above this a reported voltage is garbage: the regulator's scale ends at
/// 3.30 V, and DSPi's own limit is 1.30 V.
const MAX_BELIEVED_MV: u32 = 2000;

/// `BINARY_INFO_MAKE_TAG('D', 'S')`.
const STAMP_TAG: u16 = 0x5344;
/// The binary-info id of the core-voltage stamp. Picked at random, as the SDK
/// asks of custom ids.
pub const STAMP_ID: u32 = 0x0B7A_4C15;

const UF2_BLOCK: usize = 512;
const UF2_PAGE: u32 = 256;
const BI_MARKER_START: u32 = 0x7188_EBF2;
const BI_MARKER_END: u32 = 0xE71A_A390;
const BI_TYPE_ID_AND_STRING: u16 = 6;
const BI_TAG_RP: u16 = 0x5052;
const BI_ID_PROGRAM_NAME: u32 = 0x0203_1C86;
const BI_SEARCH: u32 = 4096;
const BI_MAX_ENTRIES: u32 = 512;
const BI_MAX_MAPPINGS: u32 = 16;
const BI_MAX_STRING: u32 = 128;

// -- WP PGFWX -- Deserialize: read back inside a FirmwareTarget (C, wasm)
/// Where a [`FirmwareGuardBoard`]'s voltage came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum FirmwareGuardBoardSource {
    /// No device was open.
    #[default]
    NoDevice,
    /// The board is in the bootloader, where it cannot be asked.
    Bootloader,
    /// The device was open, but gave no answer that can be believed.
    ReadFailed,
    /// `EXT_CORE_VREG`.
    ExtCoreVreg,
    /// The status block's live voltage, on firmware without `core_vreg`.
    StatusBlock,
}

// -- WP PGFWX -- Deserialize: read back inside a FirmwareTarget (C, wasm); a
// missing field is the safe default (unknown)
/// What a board needs, read before it enters the bootloader. The default is
/// the safe one: unknown.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct FirmwareGuardBoard {
    /// The lowest core voltage this board may be left on, in mV. 1150: not at
    /// risk. Above that: at risk. Anything else, 0 included: unknown.
    pub required_mv: u16,
    /// Where it came from.
    pub source: FirmwareGuardBoardSource,
}

/// The guard's answer. Flash only on [`FirmwareGuardVerdict::allowed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum FirmwareGuardDecision {
    /// The image is not firmware. Confirmation does not help.
    #[default]
    Refused,
    /// Blocked until the phrase is typed.
    NeedsConfirmation,
    /// There was a risk, and the phrase was typed.
    Confirmed,
    /// No known risk.
    Clear,
}

/// Why, as a stable key for clients that localise the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum FirmwareGuardReason {
    /// The image failed the UF2 check.
    #[default]
    ImageInvalid,
    /// No known risk.
    NoRisk,
    /// At-risk board; the image does not say what voltage it runs at.
    ImageUnknown,
    /// At-risk board; the image is stamped below what the board needs.
    ImageLower,
    /// The board's needs are not known.
    BoardUnknown,
    /// WP CG14: the image runs above 1.20 V and above the board's current
    /// voltage. Its `confirmation_phrase` is the word `OVERVOLT`.
    Overvolt,
}

/// The result of [`firmware_guard_check`]. The default blocks.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct FirmwareGuardVerdict {
    /// True only for `Clear` and `Confirmed`. Anything else must not flash.
    pub allowed: bool,
    /// The decision.
    pub decision: FirmwareGuardDecision,
    /// Why.
    pub reason: FirmwareGuardReason,
    /// For the user, in English.
    pub message: String,
    /// What to ask the user to type when a risk is involved, else "": the
    /// phrase, or for [`FirmwareGuardReason::Overvolt`] the word. Compared
    /// exactly: case and spaces count, and the one does not stand for the other.
    pub confirmation_phrase: String,
    /// The image's stamped voltage in mV; 0 if it has none (taken as 1150).
    pub image_mv: u16,
    /// The board record's `required_mv`, as given.
    pub required_mv: u16,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// What the open board needs, read from the device now. Call before
    /// `enter_bootloader`, and keep the record for [`firmware_guard_check`].
    ///
    /// Never fails: no device, or any answer that cannot be believed, gives
    /// an unknown board, which the check then blocks. Blocks for up to three
    /// control transfers.
    pub fn firmware_guard_board(&self) -> FirmwareGuardBoard {
        let mut inner = self.lock();
        match inner.conn() {
            Ok(conn) => read_board(conn.transport()),
            Err(_) => unknown(FirmwareGuardBoardSource::NoDevice),
        }
    }
}

/// The record for a board already in the bootloader: unknown, so the check
/// asks.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn firmware_guard_board_in_bootloader() -> FirmwareGuardBoard {
    unknown(FirmwareGuardBoardSource::Bootloader)
}

/// The guard (module docs). `typed_confirmation` is what the user typed, ""
/// if nothing yet; call again with it when the decision is
/// `NeedsConfirmation`. Pure and cheap enough for a UI thread's button, but
/// it does parse the whole image. Never panics.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn firmware_guard_check(
    image: Vec<u8>,
    board: FirmwareGuardBoard,
    typed_confirmation: String,
) -> FirmwareGuardVerdict {
    let info = uf2_inspect(image.clone());
    if !info.valid {
        return FirmwareGuardVerdict {
            message: format!("This is not a firmware file: {}", info.problem),
            required_mv: board.required_mv,
            ..FirmwareGuardVerdict::default()
        };
    }
    // -- WP SECFWV -- a damaged or mixed-up copy of a bundled image is refused
    if let Some(refused) = crate::firmware_manifest::refusal(&image, &info, board.required_mv) {
        return refused;
    }
    decide(
        image_stamp(&image, &info),
        board.required_mv,
        &typed_confirmation,
    )
}

/// [`flash_to_drive`] behind the guard. Nothing reaches the drive unless the
/// verdict allows it. Not exported (WP P1TZ): bindings write through
/// [`DspiCore::flash_confirmed`], which takes a confirmation token first.
///
/// Errors: [`DspiError::FirmwareGuardBlocked`] with the guard's message when
/// the image needs confirmation; otherwise those of [`flash_to_drive`].
#[cfg(test)] // -- WP PGFLASH -- the API writes through `flash_to_drive_gated`
pub(crate) fn flash_to_drive_guarded(
    image: Vec<u8>,
    drive: BootloaderDrive,
    board: FirmwareGuardBoard,
    typed_confirmation: String,
) -> Result<()> {
    let verdict = firmware_guard_check(image.clone(), board, typed_confirmation);
    if verdict.allowed {
        return flash_to_drive(image, drive);
    }
    Err(if verdict.decision == FirmwareGuardDecision::Refused {
        DspiError::InvalidArgument {
            msg: verdict.message,
        }
    } else {
        DspiError::FirmwareGuardBlocked {
            msg: verdict.message,
        }
    })
}

// -- WP PGFLASH --
// ── The write gate ─────────────────────────────────────────────────────
//
// The last check before any byte reaches a board (`flash_confirmed`,
// `picoboot_flash_confirmed`): the update check's rules on the target this
// core read itself from the device (`Inner::flash_target`, set by
// `firmware_target` and by the bootloader entry), never on a target a caller
// passes. The caller's voltage record only adds doubt: one that disagrees with
// the core's own makes the voltage unknown, so the voltage phrase is asked.
// With no target read by this core (the board was in BOOTSEL before this core
// saw it, or a fresh core), the voltage rule alone decides, as before.

/// The write gate's verdict for `image` (module comment above).
pub(crate) fn write_gate(
    image: &[u8],
    read: Option<&FirmwareTarget>,
    board: FirmwareGuardBoard,
    typed: &str,
) -> FirmwareUpdateVerdict {
    let Some(read) = read else {
        log::info!(
            "firmware write gate: no target read by this core (the board was in BOOTSEL \
             already): the voltage rule alone decides"
        );
        return FirmwareUpdateVerdict {
            verdict: firmware_guard_check(image.to_vec(), board, typed.to_owned()),
            ..FirmwareUpdateVerdict::default()
        };
    };
    let mut target = read.clone();
    if board.required_mv != read.voltage.required_mv {
        log::warn!(
            "firmware write gate: the caller's voltage record ({} mV) is not the one this core \
             read ({} mV): the voltage is taken as unknown",
            board.required_mv,
            read.voltage.required_mv
        );
        target.voltage.required_mv = 0;
    }
    firmware_update_check(image.to_vec(), &target, typed)
}

/// The gate's verdict as the error the write returns, `None` when it allows.
pub(crate) fn write_gate_error(v: FirmwareUpdateVerdict) -> Option<DspiError> {
    if v.verdict.allowed {
        return None;
    }
    let msg = v.verdict.message;
    Some(
        if v.verdict.decision == FirmwareGuardDecision::Refused
            && v.rule != FirmwareUpdateRule::NoBootsel
        {
            DspiError::InvalidArgument { msg }
        } else {
            DspiError::FirmwareGuardBlocked { msg }
        },
    )
}

/// [`flash_to_drive`] behind the write gate ([`write_gate`]): nothing reaches
/// the drive unless it allows.
pub(crate) fn flash_to_drive_gated(
    image: Vec<u8>,
    drive: BootloaderDrive,
    read: Option<&FirmwareTarget>,
    board: FirmwareGuardBoard,
    typed_confirmation: &str,
) -> Result<()> {
    match write_gate_error(write_gate(&image, read, board, typed_confirmation)) {
        None => flash_to_drive(image, drive),
        Some(e) => Err(e),
    }
}

// ── The decision ───────────────────────────────────────────────────────

fn volts(mv: u16) -> String {
    format!("{}.{:02} V", mv / 1000, mv % 1000 / 10)
}

fn decide(stamp: Option<u16>, required_mv: u16, typed: &str) -> FirmwareGuardVerdict {
    let image_mv = stamp.unwrap_or(0);
    let current = (u32::from(STOCK_MV)..=MAX_BELIEVED_MV)
        .contains(&u32::from(required_mv))
        .then_some(required_mv);
    let risk = if let Some(mv) = stamp.filter(|mv| overvolt::word_required(current, *mv)) {
        Some((
            FirmwareGuardReason::Overvolt,
            match current {
                Some(now) => format!(
                    "This image runs at {}, above this board's current {}, and beyond 1.20 V.",
                    volts(mv),
                    volts(now)
                ),
                None => format!(
                    "This image runs at {}, beyond 1.20 V, and this board's current voltage is \
                     not known.",
                    volts(mv)
                ),
            },
        ))
    } else if required_mv == STOCK_MV {
        None
    } else if required_mv < STOCK_MV {
        Some((
            FirmwareGuardReason::BoardUnknown,
            format!(
                "This board's core voltage needs are not known, and this image runs at {}. It \
                 can hang.",
                volts(stamp.unwrap_or(STOCK_MV))
            ),
        ))
    } else {
        match stamp {
            Some(mv) if mv >= required_mv => None,
            Some(mv) => Some((
                FirmwareGuardReason::ImageLower,
                format!(
                    "This board needs {} and this image runs at {}. It can hang.",
                    volts(required_mv),
                    volts(mv)
                ),
            )),
            None => Some((
                FirmwareGuardReason::ImageUnknown,
                format!(
                    "This board needs {} and this image does not say what it runs at, so it \
                     counts as {}. It can hang.",
                    volts(required_mv),
                    volts(STOCK_MV)
                ),
            )),
        }
    };
    let Some((reason, message)) = risk else {
        return FirmwareGuardVerdict {
            allowed: true,
            decision: FirmwareGuardDecision::Clear,
            reason: FirmwareGuardReason::NoRisk,
            message: String::new(),
            confirmation_phrase: String::new(),
            image_mv,
            required_mv,
        };
    };
    let wanted = if reason == FirmwareGuardReason::Overvolt {
        overvolt::WORD
    } else {
        CONFIRMATION_PHRASE
    };
    let confirmed = typed == wanted;
    if confirmed {
        log::warn!("firmware guard: risk accepted by typed confirmation: {message}");
    }
    FirmwareGuardVerdict {
        allowed: confirmed,
        decision: if confirmed {
            FirmwareGuardDecision::Confirmed
        } else {
            FirmwareGuardDecision::NeedsConfirmation
        },
        reason,
        message,
        confirmation_phrase: wanted.to_owned(),
        image_mv,
        required_mv,
    }
}

// ── The board ──────────────────────────────────────────────────────────

fn unknown(source: FirmwareGuardBoardSource) -> FirmwareGuardBoard {
    FirmwareGuardBoard {
        required_mv: 0,
        source,
    }
}

fn read_board(t: &mut dyn Transport) -> FirmwareGuardBoard {
    let list = sx::list(t);
    board_from_list(t, &list)
}

// -- WP PGFWX -- (review PG-C finding 8) the voltage record from an `EXT_LIST`
// answer already read, so that the update target's lineage and voltage come
// from the same read.
fn board_from_list(
    t: &mut dyn Transport,
    list: &std::result::Result<Vec<u16>, ExtError>,
) -> FirmwareGuardBoard {
    let failed = unknown(FirmwareGuardBoardSource::ReadFailed);
    match list {
        // A stall is the firmware's own answer: no extension channel.
        Err(ExtError::Unsupported { .. }) => from_status(t),
        // A hub refusing a viewer, a timeout, a short reply: no answer at all.
        Err(_) => failed,
        Ok(ids) if !ids.iter().any(|&id| id != g::EXT_LIST) => failed,
        Ok(ids) if ids.contains(&g::EXT_CORE_VREG) => match sx::read_core_vreg(t) {
            Ok(status) => from_core_vreg(&status),
            // Listed and then refused is not "absent" here.
            Err(_) => failed,
        },
        Ok(_) => from_status(t),
    }
}

fn from_core_vreg(s: &CoreVregStatus) -> FirmwareGuardBoard {
    // Selection 0 is the build default.
    let stored = if s.stored_sel == 0 {
        s.default_vreg
    } else {
        s.stored_sel
    };
    let mv = [s.default_vreg, stored, s.live_vreg]
        .into_iter()
        .map(sx::vreg_mv)
        .collect::<Option<Vec<u16>>>()
        .and_then(|all| all.into_iter().max());
    match mv {
        Some(required_mv) if required_mv >= STOCK_MV => FirmwareGuardBoard {
            required_mv,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        },
        _ => unknown(FirmwareGuardBoardSource::ReadFailed),
    }
}

fn from_status(t: &mut dyn Transport) -> FirmwareGuardBoard {
    let live = t
        .control_in(op::REQ_GET_STATUS, STAT_CORE_MV, 4)
        .ok()
        .and_then(|b| b.first_chunk::<4>().map(|v| u32::from_le_bytes(*v)));
    match live.filter(|&mv| (u32::from(STOCK_MV)..=MAX_BELIEVED_MV).contains(&mv)) {
        Some(mv) => FirmwareGuardBoard {
            required_mv: u16::try_from(mv).unwrap_or(0),
            source: FirmwareGuardBoardSource::StatusBlock,
        },
        // 0 is firmware that does not know the sub-query.
        None => unknown(FirmwareGuardBoardSource::ReadFailed),
    }
}

// -- WP PGFWUP --
// ── Per generation and board (PLAN-PG section 1.8) ─────────────────────
//
// On top of the voltage rule above, the update check
// ([`firmware_update_check`]) judges the target as a whole
// ([`FirmwareTarget`], read before the bootloader entry with
// [`DspiCore::firmware_target`]):
//
// | Target | Image | Result |
// |---|---|---|
// | no BOOTSEL (the DSPi-Zero, an unknown board, a board not read) | any | refused: no image is offered |
// | our lineage (bofr: the extension channel answers) | not proven ours | the typed confirmation (lineage loss), on top of the voltage rule |
// | one generation | the other | the change is shown with what works afterwards |
//
// An image is proven ours by the manifest (a bundled bofr image) or by the
// core-voltage stamp, which only our `core_vreg.c` emits. Every other image,
// a stock one included, may be upstream's.

/// What must be typed to put an image that may be upstream's on a board running
/// our firmware, when the voltage rule does not ask already (its phrase then
/// covers both, and the message names both).
pub const LINEAGE_PHRASE: &str = "I ACCEPT LOSING THE EXTENSIONS";

/// The board an update is for, read before it enters the bootloader. The
/// default fails closed: no board known, so no image.
///
/// WP PGFWX: exported (UniFFI record; JSON in the C ABI and wasm, where a
/// missing field is its default: unknown, so it fails closed). Keep the one
/// [`DspiCore::firmware_target`] read and hand it back unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct FirmwareTarget {
    /// The voltage rule's record ([`DspiCore::firmware_guard_board`]).
    pub voltage: FirmwareGuardBoard,
    /// The board id (platform byte 7, PLAN-PG section 1.9; 0 is a Pico, see
    /// [`FirmwareTarget::board`]); `None` when it could not be read.
    pub board_id: Option<u8>,
    /// Whose firmware runs on it: ours when the extension channel answers.
    pub lineage: crate::FirmwareLineage,
    /// The protocol generation of the live connection.
    pub generation: crate::FirmwareGeneration,
}

impl FirmwareTarget {
    /// A board already in BOOTSEL (a mounted drive, a PICOBOOT device): it has
    /// BOOTSEL by being there; nothing else is known.
    pub fn in_bootloader() -> Self {
        Self {
            voltage: firmware_guard_board_in_bootloader(),
            board_id: Some(dspi_proto::packets::Board::Pico.id()),
            ..Self::default()
        }
    }

    /// The board, from [`FirmwareTarget::board_id`]; `None` when it could not
    /// be read.
    pub fn board(&self) -> Option<dspi_proto::packets::Board> {
        self.board_id.map(dspi_proto::packets::Board::from_id)
    }

    /// Whether an image may be offered at all: only a board with BOOTSEL.
    pub fn has_bootsel(&self) -> bool {
        self.board()
            .is_some_and(dspi_proto::packets::Board::has_bootsel)
    }

    // -- WP PGFWX -- (review PG-C finding 8)
    /// Whether the running firmware may be ours: our lineage, or a lineage
    /// that could not be read on a board that was open and answered (not a
    /// board in BOOTSEL and not "no device"). Every doubt asks.
    pub fn may_be_ours(&self) -> bool {
        match self.lineage {
            crate::FirmwareLineage::Bofr => true,
            crate::FirmwareLineage::Upstream => false,
            crate::FirmwareLineage::Unknown => !matches!(
                self.voltage.source,
                FirmwareGuardBoardSource::NoDevice | FirmwareGuardBoardSource::Bootloader
            ),
        }
    }
}

/// Which rule beyond the voltage rule decided an update verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum FirmwareUpdateRule {
    /// None: the voltage rule's verdict stands.
    #[default]
    None,
    /// The board has no BOOTSEL: refused, whatever is typed.
    NoBootsel,
    /// Our firmware would give way to an image that may be upstream's.
    LineageLoss,
}

/// [`firmware_update_check`]'s answer. Flash only on `verdict.allowed`.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct FirmwareUpdateVerdict {
    /// The verdict, as [`firmware_guard_check`]'s with the rules above folded
    /// in (`message` names every risk; `confirmation_phrase` is what to type).
    pub verdict: FirmwareGuardVerdict,
    /// The rule beyond the voltage rule that decided, if any.
    pub rule: FirmwareUpdateRule,
    /// The image's lineage (the manifest, or our stamp; else unknown).
    pub image_lineage: crate::FirmwareLineage,
    /// The image's generation (the manifest, or its version).
    pub image_generation: crate::FirmwareGeneration,
    /// A generation change, shown with what works afterwards; "" when none.
    pub generation_change: String,
}

// -- WP PGFWX -- exported
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The open board as an update target, read now: the voltage record, the
    /// board (8 platform bytes and the descriptor), its lineage and the live
    /// connection's generation. Never fails: what cannot be read is unknown,
    /// and an unknown board is offered no image. Call before the bootloader
    /// entry, off the UI thread (up to five control transfers), and keep it
    /// for [`firmware_update_check`]. `EXT_LIST` is read once, for the
    /// lineage and the voltage record alike.
    pub fn firmware_target(&self) -> FirmwareTarget {
        let mut inner = self.lock();
        // -- WP PGFLASH -- the core keeps what it read, for its own write gate
        let target = target_of(&mut inner);
        if target.voltage.source != FirmwareGuardBoardSource::NoDevice {
            inner.flash_target = Some(target.clone());
        }
        target
    }
}

// -- WP PGFLASH -- the body of `firmware_target`, for the bootloader entry too
/// The open board as an update target, read now through `inner`'s connection
/// ([`DspiCore::firmware_target`]).
pub(crate) fn target_of(inner: &mut crate::Inner) -> FirmwareTarget {
    let inner_state = (
        inner.state.fw_major,
        inner.state.fw_minor,
        inner.state.fw_patch,
    );
    let Ok(conn) = inner.conn() else {
        return FirmwareTarget {
            voltage: unknown(FirmwareGuardBoardSource::NoDevice),
            ..FirmwareTarget::default()
        };
    };
    let board = crate::hardware::read_board(conn).ok();
    let t = conn.transport();
    // The live connection's generation where the wrappers say it, else the
    // generation the running firmware's version names (the mirror's
    // `GET_PLATFORM`, read at connect).
    let generation = match t.generation() {
        Some(dspi_transport::Generation::Legacy) => crate::FirmwareGeneration::Legacy,
        Some(dspi_transport::Generation::Paged(_)) => crate::FirmwareGeneration::Paged,
        None => {
            let s = &inner_state;
            crate::FirmwareGeneration::of_version(&format!("{}.{}.{}", s.0, s.1, s.2))
        }
    };
    // -- WP PGFWX -- one EXT_LIST read for both (review PG-C finding 8)
    let list = sx::list(t);
    let lineage = match &list {
        Ok(ids) if ids.iter().any(|&id| id != g::EXT_LIST) => crate::FirmwareLineage::Bofr,
        // A stall is the firmware's own answer: no extension channel.
        Err(ExtError::Unsupported { .. }) => crate::FirmwareLineage::Upstream,
        _ => crate::FirmwareLineage::Unknown,
    };
    FirmwareTarget {
        voltage: board_from_list(t, &list),
        board_id: board.map(dspi_proto::packets::Board::id),
        lineage,
        generation,
    }
}

/// The image's lineage and generation: the manifest's for a bundled image,
/// else ours when it carries our core-voltage stamp, and the generation from
/// its version.
fn image_facts(
    image: &[u8],
    info: &Uf2Info,
) -> (crate::FirmwareLineage, crate::FirmwareGeneration) {
    if let Some(known) = crate::firmware_manifest::bundled_lineage(image) {
        return known;
    }
    let lineage = if image_stamp(image, info).is_some() {
        crate::FirmwareLineage::Bofr
    } else {
        crate::FirmwareLineage::Unknown
    };
    (
        lineage,
        crate::FirmwareGeneration::of_version(&info.firmware_version),
    )
}

/// What works after a generation change, for the person deciding.
fn generation_change(from: crate::FirmwareGeneration, to: crate::FirmwareGeneration) -> String {
    use crate::FirmwareGeneration as G;
    match (from, to) {
        (G::Legacy, G::Paged) => {
            "This update moves the device to firmware 1.1.7's paged protocol. \
             Afterwards an upstream DSPi Link 1.2 hub (a 1.1.6 console, the bridge 0.1.0) can no \
             longer serve it, and apps older than this one cannot control it."
                .into()
        }
        (G::Paged, G::Legacy) => "This update moves the device back to the 1.1.6 protocol. \
             Afterwards an upstream DSPi Link 1.3 hub does not serve it."
            .into(),
        _ => String::new(),
    }
}

// -- WP PGFWX -- exported
/// The update check (PLAN-PG section 1.8): [`firmware_guard_check`]'s voltage
/// rule on the target's record, plus: no image for a board without BOOTSEL,
/// whatever is typed; the typed confirmation when firmware that may be ours
/// would give way to an image that may be upstream's (a lineage that could
/// not be read on an open board counts as ours); the generation change, with
/// what works afterwards. Pure apart from parsing the image; never panics.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn firmware_update_check(
    image: Vec<u8>,
    target: &FirmwareTarget,
    typed_confirmation: &str,
) -> FirmwareUpdateVerdict {
    let info = uf2_inspect(image.clone());
    let (image_lineage, image_generation) = if info.valid {
        image_facts(&image, &info)
    } else {
        Default::default()
    };
    let mut out = FirmwareUpdateVerdict {
        image_lineage,
        image_generation,
        generation_change: generation_change(target.generation, image_generation),
        ..FirmwareUpdateVerdict::default()
    };
    if !target.has_bootsel() {
        let name = target
            .board()
            .map_or("board could not be identified", crate::board_name);
        out.rule = FirmwareUpdateRule::NoBootsel;
        out.verdict = FirmwareGuardVerdict {
            reason: FirmwareGuardReason::BoardUnknown,
            message: format!(
                "No image is offered for this device ({name}): it has no BOOTSEL bootloader \
                 this app can update it through."
            ),
            required_mv: target.voltage.required_mv,
            ..FirmwareGuardVerdict::default()
        };
        return out;
    }
    let mut v = firmware_guard_check(image, target.voltage.clone(), typed_confirmation.to_owned());
    let loss = target.may_be_ours()
        && image_lineage != crate::FirmwareLineage::Bofr
        && v.decision != FirmwareGuardDecision::Refused;
    if loss {
        out.rule = FirmwareUpdateRule::LineageLoss;
        let text = "This board runs this fork's firmware, and this image may be upstream's: the \
             extension channel, the audio link, the 1.20 V core setting and the stored IO, \
             control-interface and Control Surfaces settings would be lost (upstream firmware \
             reads them as missing and falls back to its defaults).";
        v.message = if v.message.is_empty() {
            text.to_owned()
        } else {
            format!("{} {text}", v.message)
        };
        if v.decision == FirmwareGuardDecision::Clear {
            let confirmed = typed_confirmation == LINEAGE_PHRASE;
            if confirmed {
                log::warn!("firmware guard: lineage loss accepted by typed confirmation");
            }
            v.allowed = confirmed;
            v.decision = if confirmed {
                FirmwareGuardDecision::Confirmed
            } else {
                FirmwareGuardDecision::NeedsConfirmation
            };
            v.reason = FirmwareGuardReason::ImageUnknown;
            v.confirmation_phrase = LINEAGE_PHRASE.to_owned();
        }
    }
    out.verdict = v;
    out
}

// ── The image's stamp ──────────────────────────────────────────────────

/// An image's flash pages by address.
struct Pages<'a>(BTreeMap<u32, &'a [u8]>);

impl<'a> Pages<'a> {
    /// `bytes` already passed the UF2 check. Blocks of another family (the
    /// RP2350-E10 workaround block) are not part of the image.
    fn of(bytes: &'a [u8], family: u32) -> Self {
        let le = |b: &[u8], at: usize| {
            b.get(at..)
                .and_then(<[u8]>::first_chunk::<4>)
                .map(|v| u32::from_le_bytes(*v))
        };
        Self(
            bytes
                .as_chunks::<UF2_BLOCK>()
                .0
                .iter()
                .filter(|b| le(b.as_slice(), 28) == Some(family))
                .filter_map(|b| Some((le(b.as_slice(), 12)?, b.get(32..288)?)))
                .collect(),
        )
    }

    fn byte(&self, addr: u32) -> Option<u8> {
        let page = addr & !(UF2_PAGE - 1);
        let at = usize::try_from(addr.checked_sub(page)?).ok()?;
        self.0.get(&page)?.get(at).copied()
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

    fn c_string(&self, addr: u32) -> Option<String> {
        let mut s = String::new();
        for k in 0..BI_MAX_STRING {
            match self.byte(addr.checked_add(k)?)? {
                0 => return Some(s),
                b @ 0x20..0x7F => s.push(char::from(b)),
                _ => return None,
            }
        }
        None
    }

    /// Every `ID_AND_STRING` binary-info entry: `(tag, id, text)`. Anything
    /// that does not parse is left out, which can only lose a stamp.
    fn strings(&self) -> Vec<(u16, u32, String)> {
        self.walk().unwrap_or_default()
    }

    fn walk(&self) -> Option<Vec<(u16, u32, String)>> {
        let lo = *self.0.keys().next()?;
        let header = (0..BI_SEARCH / 4)
            .filter_map(|k| lo.checked_add(k.checked_mul(4)?))
            .find(|&a| {
                self.u32_at(a) == Some(BI_MARKER_START)
                    && a.checked_add(16).and_then(|e| self.u32_at(e)) == Some(BI_MARKER_END)
            })?;
        let start = self.u32_at(header.checked_add(4)?)?;
        let end = self.u32_at(header.checked_add(8)?)?;
        let map = self.u32_at(header.checked_add(12)?)?;

        // The copy-to-RAM table: (source in flash, dest start, dest end).
        let mut mappings = Vec::new();
        for k in 0..BI_MAX_MAPPINGS {
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
        let entry = |slot: u32| -> Option<(u16, u32, String)> {
            let q = to_flash(self.u32_at(to_flash(slot)?)?)?;
            if self.u16_at(q)? != BI_TYPE_ID_AND_STRING {
                return None;
            }
            let tag = self.u16_at(q.checked_add(2)?)?;
            let id = self.u32_at(q.checked_add(4)?)?;
            let text = self.c_string(to_flash(self.u32_at(q.checked_add(8)?)?)?)?;
            Some((tag, id, text))
        };
        let count = end.saturating_sub(start) / 4;
        Some(
            (0..count.min(BI_MAX_ENTRIES))
                .filter_map(|k| start.checked_add(k.checked_mul(4)?))
                .filter_map(entry)
                .collect(),
        )
    }
}

/// The voltage an image's stamp names (module docs). `None` is "unknown".
fn image_stamp(bytes: &[u8], info: &Uf2Info) -> Option<u16> {
    let strings = Pages::of(bytes, info.family_id).strings();
    let is_dspi = strings
        .iter()
        .any(|(tag, id, text)| *tag == BI_TAG_RP && *id == BI_ID_PROGRAM_NAME && text == "DSPi");
    if !is_dspi {
        return None;
    }
    let mut stamps = strings
        .iter()
        .filter(|(tag, id, _)| *tag == STAMP_TAG && *id == STAMP_ID)
        .map(|(_, _, text)| match text.as_str() {
            "1150" => Some(1150u16),
            "1200" => Some(1200),
            "1250" => Some(1250),
            "1300" => Some(1300),
            _ => None,
        });
    let first = stamps.next()??;
    stamps.all(|s| s == Some(first)).then_some(first)
}

// -- WP PGFWUP --
#[cfg(test)]
mod tests_wp_pgfwup;
// -- WP PGFWX --
#[cfg(test)]
mod tests_wp_pgfwx;
// -- WP PGFLASH --
#[cfg(test)]
mod tests_wp_pgflash;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg8 {
    use super::*;
    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, TransportCaps,
        TransportError,
    };
    use proptest::prelude::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    const XIP: u32 = 0x1000_0000;
    const RP2040: u32 = 0xE48B_FF56;
    const RP2350: u32 = 0xE48B_FF59;
    const ABSOLUTE: u32 = 0xE48B_FF57;
    const FLAG_FAMILY: u32 = 0x2000;
    const FLAG_EXT: u32 = 0x8000;
    const YES: &str = "I ACCEPT THE CORE VOLTAGE RISK";

    // ── Synthetic images ──

    fn block(flags: u32, target: u32, page: &[u8], no: u32, total: u32, family: u32) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        for (off, v) in [
            (0, 0x0A32_4655u32),
            (4, 0x9E5D_5157),
            (8, flags),
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

    fn uf2(flash: &[u8], family: u32) -> Vec<u8> {
        let pages: Vec<&[u8]> = flash.chunks(256).collect();
        let mut out = Vec::new();
        for (i, p) in pages.iter().enumerate() {
            out.extend(block(
                FLAG_FAMILY,
                XIP + 256 * i as u32,
                p,
                i as u32,
                pages.len() as u32,
                family,
            ));
        }
        out
    }

    /// A flash image with a binary-info table holding `entries` as
    /// `(tag, id, text)`. Odd entries are reached through the RAM mapping, as
    /// the SDK lays out its data.
    fn flash_with(entries: &[(u16, u32, &str)]) -> Vec<u8> {
        let mut f = vec![0u8; 0x2000];
        let put =
            |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
        let a = |off: usize| XIP + off as u32;
        put(&mut f, 0x100, BI_MARKER_START);
        put(&mut f, 0x104, a(0x200));
        put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
        put(&mut f, 0x10C, a(0x180));
        put(&mut f, 0x110, BI_MARKER_END);
        // RAM 0x20000000.. is copied from flash 0x1000.
        put(&mut f, 0x180, a(0x1000));
        put(&mut f, 0x184, 0x2000_0000);
        put(&mut f, 0x188, 0x2000_0800);
        for (i, (tag, id, text)) in entries.iter().enumerate() {
            let (at, pointer) = if i % 2 == 0 {
                (0x400 + 16 * i, a(0x400 + 16 * i))
            } else {
                (0x1000 + 16 * i, 0x2000_0000 + 16 * i as u32)
            };
            put(&mut f, 0x200 + 4 * i, pointer);
            f[at..at + 2].copy_from_slice(&BI_TYPE_ID_AND_STRING.to_le_bytes());
            f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
            put(&mut f, at + 4, *id);
            let s = 0x800 + 64 * i;
            put(&mut f, at + 8, a(s));
            f[s..s + text.len()].copy_from_slice(text.as_bytes());
        }
        f
    }

    const NAME: (u16, u32, &str) = (BI_TAG_RP, BI_ID_PROGRAM_NAME, "DSPi");

    fn with_stamps(stamps: &[&'static str]) -> Vec<u8> {
        let mut entries = vec![NAME];
        entries.extend(stamps.iter().map(|s| (STAMP_TAG, STAMP_ID, *s)));
        uf2(&flash_with(&entries), RP2350)
    }

    /// As picotool writes RP2350 images: the E10 block in front.
    fn with_e10(mut image: Vec<u8>) -> Vec<u8> {
        let mut e10 = block(
            FLAG_FAMILY | FLAG_EXT,
            0x10FF_FF00,
            &[0xEF; 256],
            0,
            2,
            ABSOLUTE,
        );
        e10[32 + 256..32 + 260].copy_from_slice(&0x9957_E304u32.to_le_bytes());
        e10.append(&mut image);
        e10
    }

    fn stamped(mv: &'static str) -> Vec<u8> {
        with_stamps(&[mv])
    }

    fn unstamped() -> Vec<u8> {
        with_stamps(&[])
    }

    fn stamp_of(image: &[u8]) -> Option<u16> {
        let info = uf2_inspect(image.to_vec());
        assert!(info.valid, "{}", info.problem);
        image_stamp(image, &info)
    }

    fn check(image: Vec<u8>, required_mv: u16, typed: &str) -> FirmwareGuardVerdict {
        let board = FirmwareGuardBoard {
            required_mv,
            source: FirmwareGuardBoardSource::ExtCoreVreg,
        };
        firmware_guard_check(image, board, typed.to_owned())
    }

    fn assert_clear(v: &FirmwareGuardVerdict) {
        assert!(v.allowed, "{v:?}");
        assert_eq!(v.decision, FirmwareGuardDecision::Clear);
        assert_eq!(v.reason, FirmwareGuardReason::NoRisk);
        assert_eq!(v.confirmation_phrase, "");
    }

    fn assert_blocked(v: &FirmwareGuardVerdict, reason: FirmwareGuardReason) {
        assert!(!v.allowed, "{v:?}");
        assert_eq!(v.decision, FirmwareGuardDecision::NeedsConfirmation);
        assert_eq!(v.reason, reason);
        let wanted = if reason == Why::Overvolt {
            overvolt::WORD
        } else {
            YES
        };
        assert_eq!(v.confirmation_phrase, wanted);
        assert!(!v.message.is_empty());
    }

    // ── A board on a scripted wire ──

    /// A device whose extension channel and status block answer as told, in
    /// front of the demo capture.
    struct Board {
        dev: FakeDevice,
        list: Answer,
        vreg: Answer,
        stat: Answer,
    }

    #[derive(Clone)]
    enum Answer {
        Data(Vec<u8>),
        Stall,
        Denied,
        Gone,
        Timeout,
    }

    impl Answer {
        fn get(&self, opcode: u8) -> dspi_transport::Result<Vec<u8>> {
            match self {
                Answer::Data(d) => Ok(d.clone()),
                Answer::Stall => Err(TransportError::Stalled { opcode }),
                Answer::Denied => Err(TransportError::Denied("config".into())),
                Answer::Gone => Err(TransportError::Disconnected),
                Answer::Timeout => Err(TransportError::TimedOut { opcode, retries: 0 }),
            }
        }
    }

    impl Board {
        fn answer(&self, opcode: u8, value: u16) -> Option<dspi_transport::Result<Vec<u8>>> {
            let a = match (opcode, value) {
                (op::REQ_EXTENSION, g::EXT_LIST) => &self.list,
                (op::REQ_EXTENSION, g::EXT_CORE_VREG) => &self.vreg,
                (op::REQ_GET_STATUS, STAT_CORE_MV) => &self.stat,
                _ => return None,
            };
            Some(a.get(opcode))
        }
    }

    impl Transport for Board {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            match self.answer(opcode, value) {
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
            match self.answer(opcode, value) {
                Some(r) => r,
                None => self.dev.control_in_upto(opcode, value, len),
            }
        }
        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
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

    /// `EXT_CORE_VREG` as 16 bytes: live, stored, default.
    fn vreg_bytes(live: u8, stored: u8, default: u8) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b[0] = live;
        b[1] = stored;
        b[2] = default;
        b
    }

    fn list_of(ids: &[u16]) -> Answer {
        Answer::Data(ids.iter().flat_map(|i| i.to_le_bytes()).collect())
    }

    fn mv_stat(mv: u32) -> Answer {
        Answer::Data(mv.to_le_bytes().to_vec())
    }

    fn board_on(list: Answer, vreg: Answer, stat: Answer) -> (Arc<DspiCore>, FirmwareGuardBoard) {
        let dev = FakeDevice::new(Capture::demo(), FakeOptions::default());
        let core = DspiCore::new();
        core.attach(
            Box::new(Board {
                dev,
                list,
                vreg,
                stat,
            }),
            "CG8".into(),
            0,
        );
        let board = core.firmware_guard_board();
        (core, board)
    }

    fn with_vreg(live: u8, stored: u8, default: u8) -> FirmwareGuardBoard {
        board_on(
            list_of(&[1, 2, 3]),
            Answer::Data(vreg_bytes(live, stored, default)),
            mv_stat(0),
        )
        .1
    }

    const V115: u8 = 0x0C;
    const V120: u8 = 0x0D;
    const V125: u8 = 0x0E;

    fn expect(board: &FirmwareGuardBoard, mv: u16, source: FirmwareGuardBoardSource) {
        assert_eq!((board.required_mv, board.source), (mv, source));
    }

    use FirmwareGuardBoardSource as Src;
    use FirmwareGuardReason as Why;

    // ── Done criteria: K340-like, stock, no-extension, in-bootloader ──

    // ── WP CG14: too high ──

    #[test]
    fn wp_cg14_an_image_above_1v20_and_above_the_board_needs_the_word() {
        // The three cases of the plan.
        let v = check(stamped("1250"), 1150, "");
        assert_blocked(&v, Why::Overvolt);
        assert_eq!(v.confirmation_phrase, "OVERVOLT");
        assert!(check(stamped("1250"), 1150, "OVERVOLT").allowed);
        assert_clear(&check(stamped("1200"), 1150, ""));
        assert_clear(&check(stamped("1250"), 1250, ""));

        // The owner's example, on a board at 1.25 V.
        assert_blocked(&check(stamped("1300"), 1250, ""), Why::Overvolt);
        assert!(check(stamped("1300"), 1250, "OVERVOLT").allowed);
        // A K340 at 1.20 V.
        assert_blocked(&check(stamped("1250"), 1200, ""), Why::Overvolt);
        // Lower than the board stays the older rule, with its phrase.
        let v = check(stamped("1250"), 1300, "");
        assert_blocked(&v, Why::ImageLower);
        assert_eq!(v.confirmation_phrase, YES);
    }

    #[test]
    fn wp_cg14_each_risk_takes_only_its_own_answer() {
        for bad in ["overvolt", "OVERVOLT ", "Overvolt", YES, ""] {
            assert_blocked(&check(stamped("1300"), 1250, bad), Why::Overvolt);
        }
        assert_blocked(&check(stamped("1150"), 1200, "OVERVOLT"), Why::ImageLower);
        assert_blocked(&check(unstamped(), 1200, "OVERVOLT"), Why::ImageUnknown);
    }

    #[test]
    fn wp_cg14_an_unknown_board_asks_for_the_word_above_1v20() {
        for mv in [0, 1, 1149, u16::MAX] {
            assert_blocked(&check(stamped("1250"), mv, ""), Why::Overvolt);
            assert!(check(stamped("1250"), mv, "OVERVOLT").allowed);
        }
        // At or below 1.20 V the older rule answers, with its phrase.
        assert_blocked(&check(stamped("1200"), 0, "OVERVOLT"), Why::BoardUnknown);
    }

    #[test]
    fn wp_cg14_a_stock_k340_is_treated_as_what_it_reports() {
        // The owner's accepted limit: no record of what it used to be.
        assert_clear(&check(stamped("1150"), 1150, ""));
        assert_clear(&check(unstamped(), 1150, ""));
    }

    #[test]
    fn wp_cg14_the_guarded_flash_writes_nothing_without_the_word() {
        let board = FirmwareGuardBoard {
            required_mv: 1150,
            source: FirmwareGuardBoardSource::StatusBlock,
        };
        let (dir, d) = drive("cg14-word");
        let list = || std::fs::read_dir(&dir).unwrap().count();
        let empty = list();
        for bad in ["", "overvolt", "OVERVOLT ", YES] {
            let r = flash_to_drive_guarded(stamped("1250"), d.clone(), board.clone(), bad.into());
            assert!(
                matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
                "{bad:?}: {r:?}"
            );
            assert!(!dir.join("firmware.uf2").exists(), "{bad:?}");
            assert_eq!(list(), empty, "{bad:?}: nothing at all on the drive");
        }
        // The same drive takes it with the word: it was the guard that stopped it.
        flash_to_drive_guarded(stamped("1250"), d, board, overvolt::WORD.into()).unwrap();
        assert!(dir.join("firmware.uf2").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wp_cg8_k340_like_board_blocks_unknown_and_lower_images() {
        // The fake's K340 build: extension channel, build default 1.20 V.
        let core = DspiCore::new();
        let options = FakeOptions {
            extensions: true,
            ..FakeOptions::default()
        };
        let dev = FakeDevice::new(Capture::demo(), options);
        core.attach_probed(Box::new(dev), "K340".into(), 0).unwrap();
        let board = core.firmware_guard_board();
        expect(&board, 1200, Src::ExtCoreVreg);

        let go = |image: Vec<u8>, typed: &str| {
            firmware_guard_check(image, board.clone(), typed.to_owned())
        };
        assert_blocked(&go(unstamped(), ""), Why::ImageUnknown);
        assert_blocked(&go(stamped("1150"), ""), Why::ImageLower);
        // A stock image: real-looking, no stamp, an RP2350 E10 block in front.
        assert_blocked(&go(with_e10(unstamped()), ""), Why::ImageUnknown);

        assert_clear(&go(stamped("1200"), ""));
        assert_blocked(&go(stamped("1250"), ""), Why::Overvolt);
        assert!(go(stamped("1250"), overvolt::WORD).allowed);
        assert_clear(&go(with_e10(stamped("1200")), ""));

        let yes = go(unstamped(), YES);
        assert!(yes.allowed);
        assert_eq!(yes.decision, FirmwareGuardDecision::Confirmed);
        assert_eq!(yes.reason, Why::ImageUnknown);
    }

    #[test]
    fn wp_cg8_stock_board_with_no_extension_is_not_at_risk() {
        // Stock: EXT_LIST stalls, the status block says 1.15 V.
        let (_core, board) = board_on(Answer::Stall, Answer::Stall, mv_stat(1150));
        expect(&board, 1150, Src::StatusBlock);
        for image in [unstamped(), stamped("1150"), stamped("1200")] {
            assert_clear(&firmware_guard_check(image, board.clone(), String::new()));
        }
    }

    #[test]
    fn wp_cg8_core_vreg_firmware_at_stock_voltage_is_not_at_risk() {
        expect(&with_vreg(V115, 0, V115), 1150, Src::ExtCoreVreg);
        assert_clear(&check(
            unstamped(),
            with_vreg(V115, 0, V115).required_mv,
            "",
        ));
    }

    #[test]
    fn wp_cg8_no_extension_build_at_a_fixed_higher_voltage_is_at_risk() {
        // The K340's early builds: 1.20 V fixed, no extension channel.
        let (_core, board) = board_on(Answer::Stall, Answer::Stall, mv_stat(1200));
        expect(&board, 1200, Src::StatusBlock);
        assert_blocked(
            &firmware_guard_check(unstamped(), board, String::new()),
            Why::ImageUnknown,
        );
    }

    #[test]
    fn wp_cg8_board_in_bootloader_always_asks() {
        let board = firmware_guard_board_in_bootloader();
        expect(&board, 0, Src::Bootloader);
        for image in [unstamped(), stamped("1150"), stamped("1200")] {
            let v = firmware_guard_check(image.clone(), board.clone(), String::new());
            assert_blocked(&v, Why::BoardUnknown);
            assert!(firmware_guard_check(image, board.clone(), YES.into()).allowed);
        }
        let v = firmware_guard_check(stamped("1300"), board.clone(), String::new());
        assert_blocked(&v, Why::Overvolt);
        assert!(firmware_guard_check(stamped("1300"), board, overvolt::WORD.into()).allowed);
    }

    // ── Which boards are at risk ──

    #[test]
    fn wp_cg8_highest_of_default_stored_and_live_counts() {
        // Raised automatically or by hand: stored above a 1.15 V default.
        expect(&with_vreg(V120, V120, V115), 1200, Src::ExtCoreVreg);
        // Below the default in a trial: the persisted voltage is what returns.
        expect(&with_vreg(V115, 0, V120), 1200, Src::ExtCoreVreg);
        // Stored below the default: the firmware boots at the default.
        expect(&with_vreg(V120, V115, V120), 1200, Src::ExtCoreVreg);
        // A trial above everything stored asks too.
        expect(&with_vreg(V125, 0, V115), 1250, Src::ExtCoreVreg);
    }

    #[test]
    fn wp_cg8_values_outside_the_known_range_are_unknown() {
        let bad = 0xF0;
        for s in [
            with_vreg(bad, 0, V115),
            with_vreg(V115, bad, V115),
            with_vreg(V115, 0, bad),
            // Coherent but below the floor no firmware goes under.
            with_vreg(0x0B, 0, 0x0B),
        ] {
            expect(&s, 0, Src::ReadFailed);
        }
    }

    // ── The failure direction ──

    #[test]
    fn wp_cg8_every_failed_read_gives_an_unknown_board() {
        let stat = mv_stat(1150);
        let good_vreg = Answer::Data(vreg_bytes(V115, 0, V115));
        let listed = list_of(&[2]);
        let cases: Vec<(&str, Answer, Answer, Answer)> = vec![
            // Denied on EXT_LIST is a Link viewer, not stock.
            (
                "list denied",
                Answer::Denied,
                good_vreg.clone(),
                stat.clone(),
            ),
            ("list gone", Answer::Gone, good_vreg.clone(), stat.clone()),
            (
                "list timeout",
                Answer::Timeout,
                good_vreg.clone(),
                stat.clone(),
            ),
            ("list empty", list_of(&[]), good_vreg.clone(), stat.clone()),
            (
                "list only zero",
                list_of(&[0]),
                good_vreg.clone(),
                stat.clone(),
            ),
            // Listed, then not read: not the same as not listed.
            ("vreg stalled", listed.clone(), Answer::Stall, stat.clone()),
            ("vreg denied", listed.clone(), Answer::Denied, stat.clone()),
            ("vreg gone", listed.clone(), Answer::Gone, stat.clone()),
            (
                "vreg short",
                listed.clone(),
                Answer::Data(vec![V115; 15]),
                stat.clone(),
            ),
            // No core_vreg, and the status block no help.
            ("stat stalled", Answer::Stall, Answer::Stall, Answer::Stall),
            ("stat denied", Answer::Stall, Answer::Stall, Answer::Denied),
            ("stat gone", Answer::Stall, Answer::Stall, Answer::Gone),
            (
                "stat short",
                Answer::Stall,
                Answer::Stall,
                Answer::Data(vec![1, 2]),
            ),
            ("stat zero", Answer::Stall, Answer::Stall, mv_stat(0)),
            ("stat low", Answer::Stall, Answer::Stall, mv_stat(1100)),
            (
                "stat garbage",
                Answer::Stall,
                Answer::Stall,
                mv_stat(0x1234_5678),
            ),
            (
                "stat over u16",
                Answer::Stall,
                Answer::Stall,
                mv_stat(66_350),
            ),
        ];
        for (name, list, vreg, stat) in cases {
            let (_core, board) = board_on(list, vreg, stat);
            assert_eq!(board.required_mv, 0, "{name}");
            assert_eq!(board.source, Src::ReadFailed, "{name}");
            assert_blocked(
                &firmware_guard_check(unstamped(), board.clone(), String::new()),
                Why::BoardUnknown,
            );
            assert_blocked(
                &firmware_guard_check(stamped("1300"), board.clone(), String::new()),
                Why::Overvolt,
            );
        }
    }

    #[test]
    fn wp_cg8_a_link_viewer_is_not_mistaken_for_stock() {
        // The connect probe's state is empty, as on stock firmware. The guard
        // reads for itself, and the refusal comes through as unknown.
        let dev = FakeDevice::new(Capture::demo(), FakeOptions::default());
        let core = DspiCore::new();
        let t = Board {
            dev,
            list: Answer::Denied,
            vreg: Answer::Denied,
            stat: mv_stat(1150),
        };
        core.attach_probed(Box::new(t), "VIEWER".into(), 0).unwrap();
        assert!(core.ext_status().unwrap().supported.is_empty());
        expect(&core.firmware_guard_board(), 0, Src::ReadFailed);
    }

    #[test]
    fn wp_cg8_extension_without_core_vreg_goes_by_the_status_block() {
        let (_c, b) = board_on(list_of(&[1]), Answer::Stall, mv_stat(1150));
        expect(&b, 1150, Src::StatusBlock);
        let (_c, b) = board_on(list_of(&[1]), Answer::Stall, mv_stat(1250));
        expect(&b, 1250, Src::StatusBlock);
    }

    #[test]
    fn wp_cg8_no_device_is_unknown() {
        let core = DspiCore::new();
        expect(&core.firmware_guard_board(), 0, Src::NoDevice);
        assert_eq!(FirmwareGuardBoard::default().required_mv, 0);
    }

    #[test]
    fn wp_cg8_board_values_only_stock_exactly_is_not_at_risk() {
        for mv in [0, 1, 900, 1100, 1149] {
            assert_blocked(&check(stamped("1200"), mv, ""), Why::BoardUnknown);
            assert_blocked(&check(stamped("1300"), mv, ""), Why::Overvolt);
        }
        assert_clear(&check(unstamped(), 1150, ""));
        assert_blocked(&check(unstamped(), 1151, ""), Why::ImageUnknown);
        // Not a voltage to believe, so unreadable, not "lower".
        assert_blocked(&check(stamped("1300"), u16::MAX, ""), Why::Overvolt);
        assert_blocked(&check(stamped("1200"), u16::MAX, ""), Why::ImageLower);
    }

    #[test]
    fn wp_cg8_defaults_do_not_allow() {
        assert!(!FirmwareGuardVerdict::default().allowed);
        assert_blocked(
            &firmware_guard_check(unstamped(), FirmwareGuardBoard::default(), String::new()),
            Why::BoardUnknown,
        );
    }

    // ── The typed confirmation ──

    #[test]
    fn wp_cg8_only_the_exact_phrase_confirms() {
        for wrong in [
            "",
            " ",
            "yes",
            "i accept the core voltage risk",
            "I ACCEPT THE CORE VOLTAGE RISK ",
            " I ACCEPT THE CORE VOLTAGE RISK",
            "I ACCEPT THE CORE VOLTAGE RISK\n",
            "I ACCEPT THE CORE VOLTAGE",
        ] {
            assert_blocked(&check(unstamped(), 1200, wrong), Why::ImageUnknown);
        }
        assert!(check(unstamped(), 1200, YES).allowed);
    }

    #[test]
    fn wp_cg8_confirmation_does_not_rescue_an_invalid_image() {
        for image in [vec![], vec![0u8; 512], vec![0u8; 700]] {
            for required_mv in [0, 1150, 1200] {
                let v = check(image.clone(), required_mv, YES);
                assert!(!v.allowed);
                assert_eq!(v.decision, FirmwareGuardDecision::Refused);
                assert_eq!(v.reason, Why::ImageInvalid);
                assert_eq!(v.confirmation_phrase, "");
            }
        }
    }

    // ── Reading the stamp ──

    #[test]
    fn wp_cg8_stamp_is_read_from_binary_info() {
        assert_eq!(stamp_of(&stamped("1150")), Some(1150));
        assert_eq!(stamp_of(&stamped("1200")), Some(1200));
        assert_eq!(stamp_of(&stamped("1250")), Some(1250));
        assert_eq!(stamp_of(&stamped("1300")), Some(1300));
        assert_eq!(stamp_of(&with_e10(stamped("1200"))), Some(1200));
        assert_eq!(stamp_of(&with_stamps(&["1200", "1200"])), Some(1200));
        let rp2040 = uf2(&flash_with(&[NAME, (STAMP_TAG, STAMP_ID, "1200")]), RP2040);
        assert_eq!(stamp_of(&rp2040), Some(1200));
        assert_eq!(check(stamped("1200"), 1150, "").image_mv, 1200);
        assert_eq!(check(unstamped(), 1150, "").image_mv, 0);
    }

    #[test]
    fn wp_cg8_anything_but_a_clean_stamp_is_unknown() {
        for bad in [
            "", "0", "1100", "1201", "1400", "1200 ", "1.20", "1200mV", "9999",
        ] {
            assert_eq!(stamp_of(&stamped(bad)), None, "{bad:?}");
        }
        // Stamps that disagree, or a bad one beside a good one.
        assert_eq!(stamp_of(&with_stamps(&["1200", "1150"])), None);
        assert_eq!(stamp_of(&with_stamps(&["1200", "junk"])), None);
        assert_eq!(stamp_of(&with_stamps(&["junk", "1200"])), None);
    }

    #[test]
    fn wp_cg8_nothing_else_counts_as_a_stamp() {
        let stamp = (STAMP_TAG, STAMP_ID, "1200");
        // Another program.
        let other = (BI_TAG_RP, BI_ID_PROGRAM_NAME, "blink");
        assert_eq!(stamp_of(&uf2(&flash_with(&[other, stamp]), RP2350)), None);
        // No program name.
        assert_eq!(stamp_of(&uf2(&flash_with(&[stamp]), RP2350)), None);
        // The right id under another tag, the right tag with another id.
        let tag = (BI_TAG_RP, STAMP_ID, "1200");
        assert_eq!(stamp_of(&uf2(&flash_with(&[NAME, tag]), RP2350)), None);
        let id = (STAMP_TAG, STAMP_ID + 1, "1200");
        assert_eq!(stamp_of(&uf2(&flash_with(&[NAME, id]), RP2350)), None);
        // What a K340 release's version string might say.
        let version = (BI_TAG_RP, 0x11A9_BC3A, "v1.1.6-rp2350-midi-vreg1200");
        assert_eq!(stamp_of(&uf2(&flash_with(&[NAME, version]), RP2350)), None);
        // Loose bytes, outside any binary-info table.
        let mut flash = flash_with(&[NAME]);
        flash[0x1800..0x1804].copy_from_slice(b"1200");
        flash[0x1810..0x1814].copy_from_slice(&STAMP_ID.to_le_bytes());
        assert_eq!(stamp_of(&uf2(&flash, RP2350)), None);
        // No binary info at all.
        assert_eq!(stamp_of(&uf2(&[0u8; 0x800], RP2350)), None);
    }

    #[test]
    fn wp_cg8_broken_binary_info_is_unknown_and_does_not_panic() {
        let put =
            |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
        let good = flash_with(&[NAME, (STAMP_TAG, STAMP_ID, "1200")]);
        // Table out of the image, backwards, or wrapping; a broken mapping; a
        // stray entry pointer. Below: an unterminated string, a non-ASCII one.
        for (off, v) in [
            (0x104, 0xFFFF_FFF0),
            (0x108, 0),
            (0x104, 0),
            (0x10C, u32::MAX - 2),
            (0x204, 0),
            (0x204, u32::MAX),
        ] {
            let mut f = good.clone();
            put(&mut f, off, v);
            assert_eq!(stamp_of(&uf2(&f, RP2350)), None, "{off:#x} = {v:#x}");
        }
        let mut f = good.clone();
        let text = 0x800 + 64;
        f[text..text + 200].fill(b'1');
        assert_eq!(stamp_of(&uf2(&f, RP2350)), None);
        let mut f = good;
        f[text] = 0xFF;
        assert_eq!(stamp_of(&uf2(&f, RP2350)), None);
    }

    /// Real SDK layouts: the walk must find the program name in the images
    /// the apps bundle (read only, nothing is flashed), and they carry no
    /// stamp, so a K340 must get the block for them. The version-string
    /// parser in `firmware.rs` proves the same layout for its own walk.
    #[test]
    fn wp_cg8_bundled_stock_images_are_unknown_and_blocked_on_a_k340() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
        for rel in [
            "desktop/qt/firmware/DSPi-RP2040-v1.1.6.uf2",
            "desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2",
            "android/feature-firmware/src/main/assets/firmware/DSPi-RP2040-v1.1.6.uf2",
            "android/feature-firmware/src/main/assets/firmware/DSPi-RP2350-v1.1.6.uf2",
        ] {
            let image = std::fs::read(root.join(rel)).unwrap();
            let info = uf2_inspect(image.clone());
            assert!(info.valid, "{rel}: {}", info.problem);
            let strings = Pages::of(&image, info.family_id).strings();
            assert!(
                strings
                    .iter()
                    .any(|(t, i, s)| *t == BI_TAG_RP && *i == BI_ID_PROGRAM_NAME && s == "DSPi"),
                "{rel}: {strings:?}"
            );
            assert_eq!(image_stamp(&image, &info), None, "{rel}");
            assert_blocked(&check(image.clone(), 1200, ""), Why::ImageUnknown);
            assert_clear(&check(image, 1150, ""));
        }
    }

    // ── The guarded flash ──

    /// A fake bootloader drive beside the test binary: the system's temp dir
    /// may be a tmpfs.
    fn drive(name: &str) -> (PathBuf, BootloaderDrive) {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("wp-cg8-{name}-{}", std::process::id()));
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

    fn k340_record() -> FirmwareGuardBoard {
        FirmwareGuardBoard {
            required_mv: 1200,
            source: Src::ExtCoreVreg,
        }
    }

    #[test]
    fn wp_cg8_guarded_flash_writes_nothing_when_blocked() {
        let (dir, d) = drive("blocked");
        for (image, board) in [
            (unstamped(), k340_record()),
            (stamped("1150"), k340_record()),
            (unstamped(), firmware_guard_board_in_bootloader()),
            (stamped("1200"), FirmwareGuardBoard::default()),
        ] {
            let r = flash_to_drive_guarded(image, d.clone(), board, "yes".into());
            assert!(
                matches!(r, Err(DspiError::FirmwareGuardBlocked { .. })),
                "{r:?}"
            );
            assert!(!dir.join("firmware.uf2").exists());
        }
        let r = flash_to_drive_guarded(vec![0; 512], d, k340_record(), YES.into());
        assert!(matches!(r, Err(DspiError::InvalidArgument { .. })), "{r:?}");
        assert!(!dir.join("firmware.uf2").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wp_cg8_guarded_flash_writes_when_allowed() {
        for (name, image, board, typed) in [
            ("stamped", stamped("1200"), k340_record(), ""),
            ("typed", unstamped(), k340_record(), YES),
            (
                "bootloader",
                unstamped(),
                firmware_guard_board_in_bootloader(),
                YES,
            ),
            (
                "stock",
                unstamped(),
                FirmwareGuardBoard {
                    required_mv: 1150,
                    source: Src::StatusBlock,
                },
                "",
            ),
        ] {
            let (dir, d) = drive(name);
            flash_to_drive_guarded(image.clone(), d, board, typed.into()).unwrap();
            assert_eq!(std::fs::read(dir.join("firmware.uf2")).unwrap(), image);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    // ── The whole rule, over everything ──

    proptest! {
        #[test]
        fn proptest_wp_cg8_nothing_is_allowed_outside_the_rule(
            required_mv in prop_oneof![
                Just(0u16), Just(1150u16), Just(1200u16), Just(1250u16), Just(1300u16), any::<u16>()
            ],
            image_index in 0usize..6,
            typed in prop_oneof![
                Just(YES.to_owned()),
                Just(overvolt::WORD.to_owned()),
                Just(String::new()),
                ".{0,40}"
            ],
        ) {
            let images = [
                (unstamped(), None),
                (stamped("1150"), Some(1150u16)),
                (stamped("1200"), Some(1200)),
                (stamped("1250"), Some(1250)),
                (stamped("1300"), Some(1300)),
                (with_stamps(&["1150", "1300"]), None),
            ];
            let (image, stamp) = images[image_index].clone();
            let v = check(image, required_mv, &typed);
            // WP CG14: too high comes first, and takes only the word.
            let current = (1150..=2000).contains(&required_mv).then_some(required_mv);
            let high = stamp.is_some_and(|mv| overvolt::word_required(current, mv));
            let safe = !high
                && (required_mv == 1150
                    || (required_mv > 1150 && stamp.is_some_and(|mv| mv >= required_mv)));
            let wanted = if high { overvolt::WORD } else { YES };
            prop_assert_eq!(v.allowed, safe || typed == wanted);
            prop_assert_eq!(v.decision == FirmwareGuardDecision::Clear, safe);
        }

        #[test]
        fn proptest_wp_cg8_junk_bytes_are_never_allowed(
            bytes in proptest::collection::vec(any::<u8>(), 0..2048),
            required_mv in any::<u16>(),
        ) {
            let v = check(bytes, required_mv, YES);
            prop_assert!(!v.allowed);
        }

        #[test]
        fn proptest_wp_cg8_a_damaged_table_never_panics(
            at in 0x100usize..0x900,
            value in any::<u32>(),
        ) {
            let mut flash = flash_with(&[NAME, (STAMP_TAG, STAMP_ID, "1200")]);
            flash[at..at + 4].copy_from_slice(&value.to_le_bytes());
            let v = check(uf2(&flash, RP2350), 1200, "");
            if !v.allowed {
                prop_assert_eq!(v.decision, FirmwareGuardDecision::NeedsConfirmation);
            }
        }
    }
}
