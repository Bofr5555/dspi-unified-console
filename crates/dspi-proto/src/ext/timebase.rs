//! PLAN-AT seam (`docs/core-merge/PLAN-AT.md` section 1): the alink time-base
//! contract's wire constants, transcribed from the plan until the firmware
//! (WP FWTB0, `extension_channel_spec.md` sections 2.10 and 2.11) has a header
//! for them. Constants only: the codecs are WP ATPROTO's, the fake's handlers
//! WP ATFAKE's, and the two are written independently from these numbers and
//! the firmware spec.
//!
//! Every id here is an ordinary extension sub-command: it travels in
//! `REQ_EXTENSION` 0x00 on the 1.1.6 generation and in the tunnel `0xE010` on
//! the 1.1.7 generation, with byte-identical payloads
//! (`extension_channel_spec.md` 1.3), so nothing here depends on the envelope.
//!
//! -- WP ATPROTO -- Below the constants: the codecs, written from the
//! firmware's normative specs (`extension_channel_spec.md` 2.8.7.1, 2.10 and
//! 2.11, `alink_protocol_spec.md` 11 and 12, `test_signals_spec.md` 15; the
//! FWTB1 revision: `EXT_ALINK_INFO.features` 0x63, op 9's lead at most
//! 120 s). Pure: every decoder takes device bytes and never panics, a short
//! reply is `None` and extra bytes are ignored. The I/O is
//! `dspi_session::ext_timebase`.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

// ------------------------------------------------------------ sub-commands

/// `EXT_ALINK_SPDIF` IN/OUT, [`ALINK_SPDIF_LEN`] bytes: the S/PDIF time base's
/// mode (off, follower, source), D, the lock-error action, the source's slots.
pub const EXT_ALINK_SPDIF: u16 = 0x8020;
/// `EXT_ALINK_SPDIF_STATUS` IN, [`ALINK_SPDIF_STATUS_LEN`] bytes.
pub const EXT_ALINK_SPDIF_STATUS: u16 = 0x8021;
/// `EXT_ALINK_SPDIF_CTRL` OUT [`ALINK_SPDIF_CTRL_LEN`] bytes on a source: one
/// apply op carried to every follower in the S/PDIF superframe (the source
/// applies it too). IN: the source's broadcast queue.
pub const EXT_ALINK_SPDIF_CTRL: u16 = 0x8022;
/// `EXT_TIME_BASE` IN, [`TIME_BASE_LEN`] bytes: every time base the unit has
/// now, in the contract's terms (state, T, domain, disturbances, D).
pub const EXT_TIME_BASE: u16 = 0x8030;
/// `EXT_SIGGEN_TIMELINE` IN, [`SIGGEN_TIMELINE_LEN`] bytes: where the
/// generator sits on the time base (the proof a measurement gates on).
pub const EXT_SIGGEN_TIMELINE: u16 = 0x8031;

/// The S/PDIF time-base block, ascending.
pub const ALINK_SPDIF_IDS: [u16; 3] = [
    EXT_ALINK_SPDIF,
    EXT_ALINK_SPDIF_STATUS,
    EXT_ALINK_SPDIF_CTRL,
];
/// The time-base block, ascending.
pub const TIME_BASE_IDS: [u16; 2] = [EXT_TIME_BASE, EXT_SIGGEN_TIMELINE];

/// `EXT_ALINK_SPDIF` length.
pub const ALINK_SPDIF_LEN: usize = 8;
/// `EXT_ALINK_SPDIF_STATUS` length.
pub const ALINK_SPDIF_STATUS_LEN: usize = 48;
/// `EXT_ALINK_SPDIF_CTRL` length, both directions (the `EXT_APPLY_AT_FRAME`
/// layout).
pub const ALINK_SPDIF_CTRL_LEN: usize = 16;
/// `EXT_TIME_BASE` length.
pub const TIME_BASE_LEN: usize = 64;
/// Offset of the first entry in `EXT_TIME_BASE`.
pub const TIME_BASE_ENTRY_OFFSET: usize = 12;
/// One `EXT_TIME_BASE` entry.
pub const TIME_BASE_ENTRY_LEN: usize = 24;
/// Entries `EXT_TIME_BASE` holds (a follower role and a source role).
pub const TIME_BASE_MAX_ENTRIES: usize = 2;
/// `EXT_SIGGEN_TIMELINE` length.
pub const SIGGEN_TIMELINE_LEN: usize = 40;
/// Layout version of `EXT_TIME_BASE` and `EXT_SIGGEN_TIMELINE` (byte 0).
pub const TIME_BASE_LAYOUT_VERSION: u8 = 1;

// ------------------------------------------------------------ time bases

/// Time base 0: the alink line's frame counter F.
pub const TB_ALINK_LINE: u8 = 0;
/// Time base 1: the USB sample index S.
pub const TB_USB_SAMPLE: u8 = 1;
/// Time base 2: a USB bus frame plus a sample in it (apply ops only).
pub const TB_USB_BUS_FRAME: u8 = 2;
/// Time base 3: the S/PDIF time base, the source's frame counter F carried
/// in the user bits.
pub const TB_SPDIF: u8 = 3;
/// No time base.
pub const TB_NONE: u8 = 0xFF;

/// A unit that follows a time base it receives.
pub const TB_ROLE_FOLLOWER: u8 = 0;
/// A unit that emits a time base (an S/PDIF source).
pub const TB_ROLE_SOURCE: u8 = 1;

/// Contract state: no usable time base (off, rate lock only, mode 0).
pub const TB_STATE_NONE: u8 = 0;
/// Searching, pulling in, waiting for a superframe.
pub const TB_STATE_ACQUIRING: u8 = 1;
/// Drain, prefill, start, verify (muted).
pub const TB_STATE_ANCHORING: u8 = 2;
/// Phase-locked: T is valid, play-out is fixed, apply-at-T works.
pub const TB_STATE_LOCKED: u8 = 3;
/// T extrapolated, muted, the timeline kept.
pub const TB_STATE_HOLDOVER: u8 = 4;
/// Configured to lock but cannot (configuration, range, no superframe).
pub const TB_STATE_LOCK_ERROR: u8 = 5;

/// Whether an `EXT_ALINK_SPDIF_CTRL` target `target` is far enough ahead of
/// the frame the source plays now: at least [`SPDIF_CTRL_MIN_LEAD_FRAMES`]
/// (2.10.3, review R2b). A target in the past is not.
pub fn spdif_ctrl_lead_ok(now: u64, target: u64) -> bool {
    target
        .checked_sub(now)
        .is_some_and(|lead| lead >= SPDIF_CTRL_MIN_LEAD_FRAMES)
}

/// Whether an op 9 for sample `t` may be written when the unit's time base
/// reads `now`, at `rate_hz`: at least `arm_lead_ms` ahead and at most
/// [`SIGGEN_START_MAX_LEAD_MS`] (2.8.7.1; the unit refuses either at the
/// write). A host adds its own margin (the lead of PLAN-AT OD-ATn) on top.
pub fn siggen_start_lead_ok(now: u64, t: u64, arm_lead_ms: u16, rate_hz: u32) -> bool {
    let samples = |ms: u32| {
        u64::from(ms)
            .checked_mul(u64::from(rate_hz))?
            .checked_div(1000)
    };
    let (Some(lead), Some(min), Some(max)) = (
        t.checked_sub(now),
        samples(u32::from(arm_lead_ms)),
        samples(SIGGEN_START_MAX_LEAD_MS),
    ) else {
        return false;
    };
    (min..=max).contains(&lead)
}

// ------------------------------------------------------------ event 0xE4

/// `NOTIFY_EVT_TIME_BASE`: one event for every contract transition, on every
/// transport (the transports' own 0xE1 and 0xE3 stay).
pub const NOTIFY_EVT_TIME_BASE: u8 = 0xE4;
/// `EXT_NOTIFY_FEATURES` bit announcing [`NOTIFY_EVT_TIME_BASE`].
pub const NOTIFY_FEAT_TIME_BASE: u32 = 1 << 5;

/// Event and entry reasons.
pub mod reason {
    /// Nothing.
    pub const NONE: u8 = 0;
    /// Phase-locked and playing.
    pub const LOCKED: u8 = 1;
    /// The time base ended.
    pub const LOST: u8 = 2;
    /// Extrapolating, muted, the timeline kept.
    pub const HOLDOVER: u8 = 3;
    /// Back from a holdover on the same timeline.
    pub const RESUMED: u8 = 4;
    /// The timeline moved: drained and anchored again.
    pub const REANCHOR: u8 = 5;
    /// Whole samples dropped or inserted on the timeline.
    pub const CORRECTION: u8 = 6;
    /// Mode off, or the input left the transport.
    pub const DISABLED: u8 = 7;
    /// The lock cannot run at this rate or D.
    pub const CONFIG: u8 = 8;
    /// Entered lock error.
    pub const LOCK_ERROR: u8 = 9;
    /// Left lock error.
    pub const LOCK_OK: u8 = 10;
    /// An S/PDIF source restarted its counter (a new epoch).
    pub const EPOCH: u8 = 11;
}

// ------------------------------------------------------------ apply ops

/// Apply op 9: the generator's sample 0 is the sample carried as T; starts
/// the staged `SiggenConfig`. Never late: a START that cannot start at T
/// exactly is refused.
pub const OP_SIGGEN_START: u8 = 9;
/// Apply op 10: stop the generator at T; `arg` [`SIGGEN_STOP_FADE`] or
/// [`SIGGEN_STOP_NOW`].
pub const OP_SIGGEN_STOP: u8 = 10;
/// `SIGGEN_STOP` arg: fade out.
pub const SIGGEN_STOP_FADE: u8 = 0;
/// `SIGGEN_STOP` arg: stop at once.
pub const SIGGEN_STOP_NOW: u8 = 1;
/// The highest valid apply op in a superframe (the line's and S/PDIF's).
pub const APPLY_OP_MAX: u8 = OP_SIGGEN_STOP;
/// Op 9 is never written in the bus-frame form ([`TB_USB_BUS_FRAME`]): the
/// firmware refuses it there and the hub always writes S (review R5).
pub const OP_SIGGEN_START_REFUSES_BUS_FRAME: bool = true;
/// `EXT_ALINK_INFO.features` bit: the build has ops 9 and 10.
pub const FEATURE_SIGGEN_SYNC: u32 = 1 << 6;

// ------------------------------------------------------------ the generator on the time base

/// `EXT_SIGGEN_TIMELINE.state`: idle.
pub const TL_IDLE: u8 = 0;
/// A START is queued and not yet armed.
pub const TL_PENDING: u8 = 1;
/// Armed: the DAC is opening ahead of T.
pub const TL_ARMED: u8 = 2;
/// Running.
pub const TL_RUNNING: u8 = 3;

/// `EXT_SIGGEN_TIMELINE.last_result`.
pub mod timeline_result {
    /// No timed start yet.
    pub const NONE: u8 = 0;
    /// Started on its sample.
    pub const STARTED: u8 = 1;
    /// Refused: T had passed.
    pub const REFUSED_LATE: u8 = 2;
    /// Refused: not ready at T (DAC still held, muted, not locked, no config).
    pub const REFUSED_NOT_READY: u8 = 3;
    /// Cancelled: the time base left LOCKED.
    pub const CANCELLED_LOSS: u8 = 4;
    /// Cancelled: a `SIGGEN_SET_CONFIG` while pending.
    pub const CANCELLED_CONFIG: u8 = 5;
    /// Cancelled by the host (`CANCEL_ALL`, `STOP`, `STOP_NOW`).
    pub const CANCELLED_HOST: u8 = 6;
    /// Started by op 9, then stopped (as `STOP_NOW`, with its fade) because its
    /// time base left `LOCKED` (review R3).
    pub const STOPPED_LOSS: u8 = 7;
}

// ------------------------------------------------------------ S/PDIF

/// `EXT_ALINK_SPDIF.mode`: off (stock S/PDIF behaviour).
pub const SPDIF_MODE_OFF: u8 = 0;
/// Follow the time base on the active S/PDIF input.
pub const SPDIF_MODE_FOLLOWER: u8 = 1;
/// Stamp the unit's own time base on its S/PDIF outputs.
pub const SPDIF_MODE_SOURCE: u8 = 2;

/// Frames in one IEC 60958 block (the channel-status period); one S/PDIF
/// superframe per block.
pub const SPDIF_BLOCK_FRAMES: u64 = 192;
/// User bits in one block (both subframes).
pub const SPDIF_USER_BITS: usize = 384;
/// Bytes of one S/PDIF superframe (the user bits of one block).
pub const SPDIF_SUPERFRAME_LEN: usize = 48;
/// The superframe's byte 5 on S/PDIF (the line's class is 1..3).
pub const SPDIF_CARRIER_CLASS: u8 = 0x10;
/// Blocks the source's superframe table covers (the 512-frame ring plus the
/// erase-ahead); a block's superframe is a function of its index `F / 192`
/// and is frozen once any of its frames is stamped (review R2).
pub const SPDIF_TX_TABLE_BLOCKS: usize = 4;
/// The transmit ring, frames (`pico_audio_spdif_multi`).
pub const SPDIF_TX_RING_FRAMES: u64 = 512;
/// The shortest lead of a broadcast op through `EXT_ALINK_SPDIF_CTRL`: the
/// ring plus two blocks (896 frames, about 19 ms at 48 kHz).
pub const SPDIF_CTRL_MIN_LEAD_FRAMES: u64 = SPDIF_TX_RING_FRAMES + 2 * SPDIF_BLOCK_FRAMES;
/// Missing or invalid superframes a follower rides through on counter
/// continuity (counted, no disturbance) before it enters `HOLDOVER`.
pub const SPDIF_COAST_BLOCKS: u32 = 8;

// -- WP ATPROTO -- the codecs of the time-base contract (PLAN-AT 1.1-1.3)

use super::alink::{ApplyOp, ApplyRequest, ApplyStatus};

/// `test_signals_spec.md` 15 and `extension_channel_spec.md` 2.8.7.1 (WP
/// FWTB1): an op 9 more than 120 s ahead is refused at the write
/// (`VC_ERR_STATE`), so one pending START never sits for long.
pub const SIGGEN_START_MAX_LEAD_MS: u32 = 120_000;
/// `EXT_ALINK_SPDIF.flags` bit 0 `AUTO_REANCHOR` (the default).
pub const SPDIF_FLAG_AUTO_REANCHOR: u8 = 1 << 0;
/// `EXT_ALINK_SPDIF.d`'s default, samples.
pub const SPDIF_DEFAULT_D: u16 = 192;
/// `EXT_ALINK_SPDIF.on_lock_error` 0: mute (the default).
pub const SPDIF_ON_LOCK_ERROR_MUTE: u8 = 0;
/// `EXT_ALINK_SPDIF.on_lock_error` 1: play unaligned (rate lock).
pub const SPDIF_ON_LOCK_ERROR_PLAY: u8 = 1;

/// `EXT_ALINK_SPDIF_STATUS.flags` (2.10.2).
pub mod spdif_flag {
    /// A valid superframe accepted within the last 8 blocks.
    pub const SUPERFRAME: u8 = 1 << 0;
    /// A usable time base (`LOCKED` or `HOLDOVER`).
    pub const TIME_BASE: u8 = 1 << 1;
    /// In holdover.
    pub const HOLDOVER: u8 = 1 << 2;
    /// In lock error.
    pub const LOCK_ERROR: u8 = 1 << 3;
    /// The unit is a source (mode 2).
    pub const SOURCE: u8 = 1 << 4;
    /// Locked to a stream that carries no valid superframe.
    pub const FOREIGN: u8 = 1 << 5;
}

/// The apply ops an S/PDIF superframe carries and `EXT_ALINK_SPDIF_CTRL`
/// accepts (`alink_protocol_spec.md` 12.2, ext spec 2.10.3): 1 MUTE,
/// 2 UNMUTE, 4 MASTER_VOLUME, 7 CLICK, 9 SIGGEN_START, 10 SIGGEN_STOP. Its
/// own set, not the line's range.
pub const SPDIF_APPLY_OPS: [u8; 6] = [1, 2, 4, 7, OP_SIGGEN_START, OP_SIGGEN_STOP];

/// Whether `op` may travel in an S/PDIF superframe (0, "none", is not an op
/// of the broadcast).
pub fn spdif_op_allowed(op: u8) -> bool {
    SPDIF_APPLY_OPS.contains(&op)
}

/// What a time base id is called (`alink_protocol_spec.md` 11.3).
pub fn time_base_name(tb: u8) -> &'static str {
    match tb {
        TB_ALINK_LINE => "alink",
        TB_USB_SAMPLE => "usb_sample",
        TB_USB_BUS_FRAME => "bus_frame",
        TB_SPDIF => "spdif",
        4 => "adat",
        5 => "i2s",
        6 => "ptp",
        TB_NONE => "none",
        _ => "unknown",
    }
}

/// What a sub-command of the S/PDIF and time-base blocks is called; `None`
/// outside them. "audio link", never "link" alone (that is DSPi Link).
pub fn timebase_name(id: u16) -> Option<&'static str> {
    Some(match id {
        EXT_ALINK_SPDIF => "audio link S/PDIF",
        EXT_ALINK_SPDIF_STATUS => "audio link S/PDIF status",
        EXT_ALINK_SPDIF_CTRL => "audio link S/PDIF broadcast",
        EXT_TIME_BASE => "time base",
        EXT_SIGGEN_TIMELINE => "generator timeline",
        _ => return None,
    })
}

// ------------------------------------------------------------ helpers

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i.checked_add(2)?)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn i16_at(b: &[u8], i: usize) -> Option<i16> {
    u16_at(b, i).map(|v| i16::from_le_bytes(v.to_le_bytes()))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i.checked_add(4)?)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn f32_at(b: &[u8], i: usize) -> Option<f32> {
    u32_at(b, i).map(f32::from_bits)
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    b.get(i..i.checked_add(8)?)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

fn byte(b: &[u8], i: usize) -> Option<u8> {
    b.get(i).copied()
}

/// Write `v` at `at` in `b`, if it fits (it always does for the fixed layouts).
fn put(b: &mut [u8], at: usize, v: &[u8]) {
    if let Some(end) = at.checked_add(v.len())
        && let Some(dst) = b.get_mut(at..end)
    {
        dst.copy_from_slice(v);
    }
}

// ------------------------------------------------------------ enums

/// A time-base contract state (`alink_protocol_spec.md` 11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractState {
    /// 0: no usable time base.
    None,
    /// 1: searching, pulling in, waiting for a superframe.
    Acquiring,
    /// 2: drain, prefill, start, verify (muted).
    Anchoring,
    /// 3: T valid, play-out fixed, apply-at-T works.
    Locked,
    /// 4: T extrapolated, muted, the timeline kept.
    Holdover,
    /// 5: configured to lock but cannot.
    LockError,
    /// A code this build does not know.
    Other(u8),
}

impl ContractState {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            TB_STATE_NONE => Self::None,
            TB_STATE_ACQUIRING => Self::Acquiring,
            TB_STATE_ANCHORING => Self::Anchoring,
            TB_STATE_LOCKED => Self::Locked,
            TB_STATE_HOLDOVER => Self::Holdover,
            TB_STATE_LOCK_ERROR => Self::LockError,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::None => TB_STATE_NONE,
            Self::Acquiring => TB_STATE_ACQUIRING,
            Self::Anchoring => TB_STATE_ANCHORING,
            Self::Locked => TB_STATE_LOCKED,
            Self::Holdover => TB_STATE_HOLDOVER,
            Self::LockError => TB_STATE_LOCK_ERROR,
            Self::Other(o) => o,
        }
    }

    /// `none`, `acquiring`, `anchoring`, `locked`, `holdover`, `lock_error`
    /// or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Acquiring => "acquiring",
            Self::Anchoring => "anchoring",
            Self::Locked => "locked",
            Self::Holdover => "holdover",
            Self::LockError => "lock_error",
            Self::Other(_) => "unknown",
        }
    }

    /// Only a `LOCKED` T may be scheduled on (11.2).
    pub fn is_locked(self) -> bool {
        self == Self::Locked
    }
}

/// Why a time base changed (event 0xE4, `EXT_TIME_BASE.last_reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBaseReason {
    None,
    Locked,
    Lost,
    Holdover,
    Resumed,
    Reanchor,
    Correction,
    Disabled,
    Config,
    LockError,
    LockOk,
    Epoch,
    /// A code this build does not know.
    Other(u8),
}

impl TimeBaseReason {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            reason::NONE => Self::None,
            reason::LOCKED => Self::Locked,
            reason::LOST => Self::Lost,
            reason::HOLDOVER => Self::Holdover,
            reason::RESUMED => Self::Resumed,
            reason::REANCHOR => Self::Reanchor,
            reason::CORRECTION => Self::Correction,
            reason::DISABLED => Self::Disabled,
            reason::CONFIG => Self::Config,
            reason::LOCK_ERROR => Self::LockError,
            reason::LOCK_OK => Self::LockOk,
            reason::EPOCH => Self::Epoch,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::None => reason::NONE,
            Self::Locked => reason::LOCKED,
            Self::Lost => reason::LOST,
            Self::Holdover => reason::HOLDOVER,
            Self::Resumed => reason::RESUMED,
            Self::Reanchor => reason::REANCHOR,
            Self::Correction => reason::CORRECTION,
            Self::Disabled => reason::DISABLED,
            Self::Config => reason::CONFIG,
            Self::LockError => reason::LOCK_ERROR,
            Self::LockOk => reason::LOCK_OK,
            Self::Epoch => reason::EPOCH,
            Self::Other(o) => o,
        }
    }

    /// The reason's name as 2.11.3 spells it, lower case.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Locked => "locked",
            Self::Lost => "lost",
            Self::Holdover => "holdover",
            Self::Resumed => "resumed",
            Self::Reanchor => "reanchor",
            Self::Correction => "correction",
            Self::Disabled => "disabled",
            Self::Config => "config",
            Self::LockError => "lock_error",
            Self::LockOk => "lock_ok",
            Self::Epoch => "epoch",
            Self::Other(_) => "unknown",
        }
    }
}

/// A time-base entry's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBaseRole {
    /// 0: receives the time base.
    Follower,
    /// 1: emits it (an S/PDIF source).
    Source,
    /// A code this build does not know.
    Other(u8),
}

impl TimeBaseRole {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            TB_ROLE_FOLLOWER => Self::Follower,
            TB_ROLE_SOURCE => Self::Source,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::Follower => TB_ROLE_FOLLOWER,
            Self::Source => TB_ROLE_SOURCE,
            Self::Other(o) => o,
        }
    }

    /// `follower`, `source` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Follower => "follower",
            Self::Source => "source",
            Self::Other(_) => "unknown",
        }
    }
}

/// `EXT_SIGGEN_TIMELINE.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineState {
    Idle,
    /// An op 9 queued, not yet armed.
    Pending,
    /// The DAC opening ahead of T.
    Armed,
    Running,
    /// A code this build does not know.
    Other(u8),
}

impl TimelineState {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            TL_IDLE => Self::Idle,
            TL_PENDING => Self::Pending,
            TL_ARMED => Self::Armed,
            TL_RUNNING => Self::Running,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::Idle => TL_IDLE,
            Self::Pending => TL_PENDING,
            Self::Armed => TL_ARMED,
            Self::Running => TL_RUNNING,
            Self::Other(o) => o,
        }
    }

    /// `idle`, `pending`, `armed`, `running` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Pending => "pending",
            Self::Armed => "armed",
            Self::Running => "running",
            Self::Other(_) => "unknown",
        }
    }
}

/// `EXT_SIGGEN_TIMELINE.last_result`: how the last timed START ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineResult {
    None,
    Started,
    RefusedLate,
    RefusedNotReady,
    CancelledLoss,
    CancelledConfig,
    CancelledHost,
    StoppedLoss,
    /// A code this build does not know.
    Other(u8),
}

impl TimelineResult {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        use timeline_result as r;
        match b {
            r::NONE => Self::None,
            r::STARTED => Self::Started,
            r::REFUSED_LATE => Self::RefusedLate,
            r::REFUSED_NOT_READY => Self::RefusedNotReady,
            r::CANCELLED_LOSS => Self::CancelledLoss,
            r::CANCELLED_CONFIG => Self::CancelledConfig,
            r::CANCELLED_HOST => Self::CancelledHost,
            r::STOPPED_LOSS => Self::StoppedLoss,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        use timeline_result as r;
        match self {
            Self::None => r::NONE,
            Self::Started => r::STARTED,
            Self::RefusedLate => r::REFUSED_LATE,
            Self::RefusedNotReady => r::REFUSED_NOT_READY,
            Self::CancelledLoss => r::CANCELLED_LOSS,
            Self::CancelledConfig => r::CANCELLED_CONFIG,
            Self::CancelledHost => r::CANCELLED_HOST,
            Self::StoppedLoss => r::STOPPED_LOSS,
            Self::Other(o) => o,
        }
    }

    /// The result's name, lower case.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Started => "started",
            Self::RefusedLate => "refused_late",
            Self::RefusedNotReady => "refused_not_ready",
            Self::CancelledLoss => "cancelled_loss",
            Self::CancelledConfig => "cancelled_config",
            Self::CancelledHost => "cancelled_host",
            Self::StoppedLoss => "stopped_loss",
            Self::Other(_) => "unknown",
        }
    }
}

/// `EXT_ALINK_SPDIF.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpdifMode {
    /// 0: stock S/PDIF.
    Off,
    /// 1: follow the time base on the active S/PDIF input.
    Follower,
    /// 2: stamp the unit's own counter on its S/PDIF outputs.
    Source,
    /// A code this build does not know (3, the relay, is reserved).
    Other(u8),
}

impl SpdifMode {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            SPDIF_MODE_OFF => Self::Off,
            SPDIF_MODE_FOLLOWER => Self::Follower,
            SPDIF_MODE_SOURCE => Self::Source,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::Off => SPDIF_MODE_OFF,
            Self::Follower => SPDIF_MODE_FOLLOWER,
            Self::Source => SPDIF_MODE_SOURCE,
            Self::Other(o) => o,
        }
    }

    /// `off`, `follower`, `source` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Follower => "follower",
            Self::Source => "source",
            Self::Other(_) => "unknown",
        }
    }
}

// ------------------------------------------------------------ EXT_TIME_BASE

/// One `EXT_TIME_BASE` entry, 24 bytes (2.11.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeBaseEntry {
    /// The time base id (11.3): 0 line, 1 USB S, 3 S/PDIF; never 2.
    pub time_base: u8,
    pub role: TimeBaseRole,
    pub state: ContractState,
    pub last_reason: TimeBaseReason,
    /// The domain identity (C5): byte 3 the time base.
    pub domain: u32,
    /// T at the reply's `time_us`.
    pub t: u64,
    /// Wraps at 2^16: every disturbance of C4.
    pub disturbances: u16,
    /// The play-out delay D, samples (0 for a source).
    pub d: u16,
    /// Samples (frames on the line); 0 unless `LOCKED`.
    pub phase_error: f32,
}

impl TimeBaseEntry {
    pub const SIZE: usize = TIME_BASE_ENTRY_LEN;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            time_base: byte(b, 0)?,
            role: TimeBaseRole::from_raw(byte(b, 1)?),
            state: ContractState::from_raw(byte(b, 2)?),
            last_reason: TimeBaseReason::from_raw(byte(b, 3)?),
            domain: u32_at(b, 4)?,
            t: u64_at(b, 8)?,
            disturbances: u16_at(b, 16)?,
            d: u16_at(b, 18)?,
            phase_error: f32_at(b, 20)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put(
            &mut b,
            0,
            &[
                self.time_base,
                self.role.raw(),
                self.state.raw(),
                self.last_reason.raw(),
            ],
        );
        put(&mut b, 4, &self.domain.to_le_bytes());
        put(&mut b, 8, &self.t.to_le_bytes());
        put(&mut b, 16, &self.disturbances.to_le_bytes());
        put(&mut b, 18, &self.d.to_le_bytes());
        put(&mut b, 20, &self.phase_error.to_bits().to_le_bytes());
        b
    }

    /// The time base the domain names (its byte 3).
    pub fn domain_time_base(&self) -> u8 {
        self.domain.to_le_bytes()[3]
    }

    /// The domain's bytes 0-1: the line's sender id (byte 1 zero) or the
    /// S/PDIF source id.
    pub fn domain_source(&self) -> u16 {
        let d = self.domain.to_le_bytes();
        u16::from_le_bytes([d[0], d[1]])
    }

    /// The domain's byte 2: the S/PDIF source's epoch (0 on the line and USB).
    pub fn domain_epoch(&self) -> u8 {
        self.domain.to_le_bytes()[2]
    }

    /// `t - output_frames` modulo 2^32: constant while the entry stays
    /// `LOCKED` and `disturbances` does not change (C4, PLAN-AT G2).
    pub fn t_minus_output(&self, output_frames: u32) -> u32 {
        // Low 32 bits of `t` by design: the comparison is modulo 2^32.
        (self.t as u32).wrapping_sub(output_frames)
    }
}

/// `EXT_TIME_BASE` IN, 64 bytes: every time base the unit has, in one atomic
/// read (2.11.1).
#[derive(Debug, Clone, PartialEq)]
pub struct TimeBaseRead {
    /// The unit's `time_us_32()` at the read.
    pub time_us: u32,
    /// Output frames played since the outputs started in sync, low 32 bits.
    pub output_frames: u32,
    /// Followers first, then the source; at most [`TIME_BASE_MAX_ENTRIES`].
    pub entries: Vec<TimeBaseEntry>,
}

impl TimeBaseRead {
    pub const SIZE: usize = TIME_BASE_LEN;

    /// A reply of layout version 1 with a `count` of 0..=2; `None` when it is
    /// short, of another version (not this build's to parse) or claims more
    /// entries than the layout holds.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        if byte(b, 0)? != TIME_BASE_LAYOUT_VERSION {
            return None;
        }
        let count = usize::from(byte(b, 1)?);
        if count > TIME_BASE_MAX_ENTRIES {
            return None;
        }
        let entries = (0..count)
            .map(|i| {
                let at = i
                    .checked_mul(TIME_BASE_ENTRY_LEN)?
                    .checked_add(TIME_BASE_ENTRY_OFFSET)?;
                TimeBaseEntry::decode(b.get(at..)?)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            time_us: u32_at(b, 4)?,
            output_frames: u32_at(b, 8)?,
            entries,
        })
    }

    /// The reply bytes; entries past [`TIME_BASE_MAX_ENTRIES`] are dropped.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let n = self.entries.len().min(TIME_BASE_MAX_ENTRIES);
        put(
            &mut b,
            0,
            &[TIME_BASE_LAYOUT_VERSION, u8::try_from(n).unwrap_or(0)],
        );
        put(&mut b, 4, &self.time_us.to_le_bytes());
        put(&mut b, 8, &self.output_frames.to_le_bytes());
        for (i, e) in self.entries.iter().take(n).enumerate() {
            if let Some(at) = i
                .checked_mul(TIME_BASE_ENTRY_LEN)
                .and_then(|o| o.checked_add(TIME_BASE_ENTRY_OFFSET))
            {
                put(&mut b, at, &e.encode());
            }
        }
        b
    }

    /// The entry of time base `tb`, the follower's first.
    pub fn entry(&self, tb: u8) -> Option<&TimeBaseEntry> {
        self.entries.iter().find(|e| e.time_base == tb)
    }

    /// The follower entry (the active input's time base), if any.
    pub fn follower(&self) -> Option<&TimeBaseEntry> {
        self.entries
            .iter()
            .find(|e| e.role == TimeBaseRole::Follower)
    }

    /// The source entry (an S/PDIF source), if any.
    pub fn source(&self) -> Option<&TimeBaseEntry> {
        self.entries.iter().find(|e| e.role == TimeBaseRole::Source)
    }

    /// `t - output_frames` of `entry` (C4), modulo 2^32.
    pub fn t_minus_output(&self, entry: &TimeBaseEntry) -> u32 {
        entry.t_minus_output(self.output_frames)
    }
}

// ------------------------------------------------------------ EXT_SIGGEN_TIMELINE

/// `EXT_SIGGEN_TIMELINE` IN, 40 bytes: where the generator sits on the time
/// base (2.11.2), the proof a measurement gates on (PLAN-AT G3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiggenTimeline {
    /// The time base of `start` and `pending`; [`TB_NONE`] none.
    pub time_base: u8,
    pub state: TimelineState,
    pub last_result: TimelineResult,
    /// Of the current or last run.
    pub discontinuities: u16,
    /// `max(hold_ms, release_ms) + 50`: the shortest op 9 lead the unit takes.
    pub arm_lead_ms: u16,
    /// T of generator sample 0 of the current or last run; 0 none.
    pub start: u64,
    /// T of the pending op 9; 0 none.
    pub pending: u64,
    /// Generator samples rendered in the current or last run.
    pub samples: u32,
    /// Frames of `siggen_hold()` in it.
    pub held: u32,
    /// Increments at every generator start, timed or plain.
    pub run_id: u32,
}

impl SiggenTimeline {
    pub const SIZE: usize = SIGGEN_TIMELINE_LEN;

    /// `None` when short or of another layout version.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        if byte(b, 0)? != TIME_BASE_LAYOUT_VERSION {
            return None;
        }
        Some(Self {
            time_base: byte(b, 1)?,
            state: TimelineState::from_raw(byte(b, 2)?),
            last_result: TimelineResult::from_raw(byte(b, 3)?),
            discontinuities: u16_at(b, 4)?,
            arm_lead_ms: u16_at(b, 6)?,
            start: u64_at(b, 8)?,
            pending: u64_at(b, 16)?,
            samples: u32_at(b, 24)?,
            held: u32_at(b, 28)?,
            run_id: u32_at(b, 32)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put(
            &mut b,
            0,
            &[
                TIME_BASE_LAYOUT_VERSION,
                self.time_base,
                self.state.raw(),
                self.last_result.raw(),
            ],
        );
        put(&mut b, 4, &self.discontinuities.to_le_bytes());
        put(&mut b, 6, &self.arm_lead_ms.to_le_bytes());
        put(&mut b, 8, &self.start.to_le_bytes());
        put(&mut b, 16, &self.pending.to_le_bytes());
        put(&mut b, 24, &self.samples.to_le_bytes());
        put(&mut b, 28, &self.held.to_le_bytes());
        put(&mut b, 32, &self.run_id.to_le_bytes());
        b
    }
}

// ------------------------------------------------------------ the S/PDIF block

/// `EXT_ALINK_SPDIF` IN/OUT, 8 bytes (2.10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkSpdif {
    pub mode: SpdifMode,
    /// Bit 0 [`SPDIF_FLAG_AUTO_REANCHOR`]; the others reserved.
    pub flags: u8,
    /// Follower: samples from the arrival of the sample carried as F to its
    /// output.
    pub d: u16,
    /// [`SPDIF_ON_LOCK_ERROR_MUTE`] or [`SPDIF_ON_LOCK_ERROR_PLAY`].
    pub on_lock_error: u8,
    /// Source: the output slots that carry the superframe; 0 = every
    /// S/PDIF-type slot.
    pub tx_mask: u8,
    /// IN only: the contract state of this role (sent as 0).
    pub state: ContractState,
}

impl AlinkSpdif {
    pub const SIZE: usize = ALINK_SPDIF_LEN;

    /// The firmware's default: mode 0, `AUTO_REANCHOR`, D 192, mute.
    pub const DEFAULT: Self = Self {
        mode: SpdifMode::Off,
        flags: SPDIF_FLAG_AUTO_REANCHOR,
        d: SPDIF_DEFAULT_D,
        on_lock_error: SPDIF_ON_LOCK_ERROR_MUTE,
        tx_mask: 0,
        state: ContractState::None,
    };

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            mode: SpdifMode::from_raw(byte(b, 0)?),
            flags: byte(b, 1)?,
            d: u16_at(b, 2)?,
            on_lock_error: byte(b, 4)?,
            tx_mask: byte(b, 6)?,
            state: ContractState::from_raw(byte(b, 7)?),
        })
    }

    /// The 8 bytes, `state` included (an OUT sends it as 0:
    /// [`AlinkSpdif::encode_out`]).
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put(&mut b, 0, &[self.mode.raw(), self.flags]);
        put(&mut b, 2, &self.d.to_le_bytes());
        put(
            &mut b,
            4,
            &[self.on_lock_error, 0, self.tx_mask, self.state.raw()],
        );
        b
    }

    /// The OUT payload: the IN-only `state` as 0.
    pub fn encode_out(&self) -> [u8; Self::SIZE] {
        Self {
            state: ContractState::None,
            ..*self
        }
        .encode()
    }

    /// What the unit refuses on its own, judged here so nothing is sent: a
    /// mode above 2, a reserved flag bit, an `on_lock_error` above 1, a
    /// `tx_mask` on a mode other than the source. (D against `d_min..d_max`
    /// and the slot types are the unit's to judge.)
    pub fn is_valid(&self) -> bool {
        let mode_ok = matches!(
            self.mode,
            SpdifMode::Off | SpdifMode::Follower | SpdifMode::Source
        );
        let mask_ok = self.tx_mask == 0 || self.mode == SpdifMode::Source;
        mode_ok
            && self.flags & !SPDIF_FLAG_AUTO_REANCHOR == 0
            && self.on_lock_error <= SPDIF_ON_LOCK_ERROR_PLAY
            && mask_ok
    }
}

/// `EXT_ALINK_SPDIF_STATUS` IN, 48 bytes (2.10.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlinkSpdifStatus {
    pub state: ContractState,
    /// Follower: 0 free, 1 anchoring, 2 locked, 3 re-anchor pending.
    pub phase_state: u8,
    /// 0 44.1 kHz, 1 48 kHz, 2 96 kHz, `0xFF` unknown.
    pub rate_code: u8,
    /// [`spdif_flag`] bits.
    pub flags: u8,
    /// The source's serial bytes 0-1; 0 while unknown.
    pub source_id: u16,
    pub epoch: u8,
    /// F now (arriving on a follower, played on a source); 0 without a time
    /// base.
    pub frame: u64,
    /// Samples.
    pub phase_error: f32,
    pub vcxo_ppm: f32,
    pub d_min: u16,
    pub d_max: u16,
    /// Output ring fill of slot 0, samples.
    pub fill: u16,
    pub lock_count: u32,
    pub loss_count: u32,
    pub sf_errors: u32,
    pub reanchors: u32,
}

impl AlinkSpdifStatus {
    pub const SIZE: usize = ALINK_SPDIF_STATUS_LEN;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            state: ContractState::from_raw(byte(b, 0)?),
            phase_state: byte(b, 1)?,
            rate_code: byte(b, 2)?,
            flags: byte(b, 3)?,
            source_id: u16_at(b, 4)?,
            epoch: byte(b, 6)?,
            frame: u64_at(b, 8)?,
            phase_error: f32_at(b, 16)?,
            vcxo_ppm: f32_at(b, 20)?,
            d_min: u16_at(b, 24)?,
            d_max: u16_at(b, 26)?,
            fill: u16_at(b, 28)?,
            lock_count: u32_at(b, 32)?,
            loss_count: u32_at(b, 36)?,
            sf_errors: u32_at(b, 40)?,
            reanchors: u32_at(b, 44)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put(
            &mut b,
            0,
            &[
                self.state.raw(),
                self.phase_state,
                self.rate_code,
                self.flags,
            ],
        );
        put(&mut b, 4, &self.source_id.to_le_bytes());
        put(&mut b, 6, &[self.epoch]);
        put(&mut b, 8, &self.frame.to_le_bytes());
        put(&mut b, 16, &self.phase_error.to_bits().to_le_bytes());
        put(&mut b, 20, &self.vcxo_ppm.to_bits().to_le_bytes());
        put(&mut b, 24, &self.d_min.to_le_bytes());
        put(&mut b, 26, &self.d_max.to_le_bytes());
        put(&mut b, 28, &self.fill.to_le_bytes());
        put(&mut b, 32, &self.lock_count.to_le_bytes());
        put(&mut b, 36, &self.loss_count.to_le_bytes());
        put(&mut b, 40, &self.sf_errors.to_le_bytes());
        put(&mut b, 44, &self.reanchors.to_le_bytes());
        b
    }

    /// Whether flag `bit` ([`spdif_flag`]) is set.
    pub fn has(&self, bit: u8) -> bool {
        self.flags & bit != 0
    }
}

/// `EXT_ALINK_SPDIF_CTRL` OUT, 16 bytes (2.10.3): the `EXT_APPLY_AT_FRAME`
/// layout with byte 12 = 3.
pub fn encode_spdif_ctrl(req: &ApplyRequest) -> [u8; ALINK_SPDIF_CTRL_LEN] {
    encode_apply_tb(req, TB_SPDIF)
}

/// What `EXT_ALINK_SPDIF_CTRL` refuses without looking at its state: an op
/// outside [`SPDIF_APPLY_OPS`] (`CANCEL_ALL` included: a stamped broadcast
/// cannot be taken back) or an argument out of its range (2.8.7).
pub fn spdif_ctrl_valid(req: &ApplyRequest) -> bool {
    spdif_op_allowed(req.op.raw()) && req.args_valid()
}

/// `EXT_ALINK_SPDIF_CTRL` IN: the source's broadcast queue, in the
/// `EXT_APPLY_AT_FRAME` IN layout.
pub fn decode_spdif_ctrl_status(b: &[u8]) -> Option<ApplyStatus> {
    ApplyStatus::decode(b)
}

/// `EXT_APPLY_AT_FRAME` OUT on time base `tb` (byte 12; 2.8.7, 2.9.4,
/// 2.10.4).
pub fn encode_apply_tb(req: &ApplyRequest, tb: u8) -> [u8; ApplyRequest::SIZE] {
    let mut b = req.encode();
    put(&mut b, 12, &[tb]);
    b
}

/// An `EXT_APPLY_AT_FRAME` (or `EXT_ALINK_SPDIF_CTRL`) OUT and its time base.
pub fn decode_apply_tb(b: &[u8]) -> Option<(ApplyRequest, u8)> {
    let b = b.get(..ApplyRequest::SIZE)?;
    Some((ApplyRequest::decode(b)?, byte(b, 12)?))
}

/// Whether `req` on time base `tb` is a write the firmware may accept, judged
/// before the wire: a known op with its arguments in range (2.8.7), a time
/// base 0..=3, never op 6 `RATE` (the sender's), never op 9 in the bus-frame
/// form (review R5; [`OP_SIGGEN_START_REFUSES_BUS_FRAME`]).
pub fn apply_form_valid(req: &ApplyRequest, tb: u8) -> bool {
    let op = req.op.raw();
    let tb_ok = matches!(
        tb,
        TB_ALINK_LINE | TB_USB_SAMPLE | TB_USB_BUS_FRAME | TB_SPDIF
    );
    let bus_ok = !(OP_SIGGEN_START_REFUSES_BUS_FRAME
        && req.op == ApplyOp::SiggenStart
        && tb == TB_USB_BUS_FRAME);
    op <= APPLY_OP_MAX && req.op != ApplyOp::Rate && req.args_valid() && tb_ok && bus_ok
}

// ------------------------------------------------------------ event 0xE4

/// Event 0xE4 `NOTIFY_EVT_TIME_BASE`: `[time_base, state, reason, 0,
/// T_low32]` from byte 4 of the packet (2.11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBaseEvent {
    pub time_base: u8,
    pub state: ContractState,
    pub reason: TimeBaseReason,
    /// T at the transition, low 32 bits.
    pub t_low: u32,
}

impl TimeBaseEvent {
    /// The payload size from byte 4.
    pub const PAYLOAD: usize = 8;

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let p = payload.get(..Self::PAYLOAD)?;
        Some(Self {
            time_base: byte(p, 0)?,
            state: ContractState::from_raw(byte(p, 1)?),
            reason: TimeBaseReason::from_raw(byte(p, 2)?),
            t_low: u32_at(p, 4)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::PAYLOAD] {
        let mut b = [0u8; Self::PAYLOAD];
        put(
            &mut b,
            0,
            &[self.time_base, self.state.raw(), self.reason.raw()],
        );
        put(&mut b, 4, &self.t_low.to_le_bytes());
        b
    }

    /// Whether it says the time base is no longer usable for scheduling.
    pub fn is_loss(&self) -> bool {
        !self.state.is_locked()
    }
}

/// Event 0xE4 behind its feature bit, as [`super::alink::alink_event`]:
/// `notify_features` is the device's `EXT_NOTIFY_FEATURES` (`None` when it
/// lacks or refused it), `id` and `payload` an unknown event's (bytes 4 on).
/// Only a device announcing bit 5 has its 0xE4 read.
pub fn time_base_event(
    notify_features: Option<u32>,
    id: u8,
    payload: &[u8],
) -> Option<TimeBaseEvent> {
    let f = notify_features?;
    (id == NOTIFY_EVT_TIME_BASE && f & NOTIFY_FEAT_TIME_BASE != 0)
        .then(|| TimeBaseEvent::decode(payload))
        .flatten()
}

// ------------------------------------------------------------ the S/PDIF superframe

/// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection, no final
/// XOR), the line's superframe CRC (`alink_protocol_spec.md` 5.5).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &d in data {
        crc ^= u16::from(d).wrapping_shl(8);
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                crc.wrapping_shl(1) ^ 0x1021
            } else {
                crc.wrapping_shl(1)
            };
        }
    }
    crc
}

/// Why an S/PDIF superframe is invalid: the first failing check, with the
/// line's codes (`alink_protocol_spec.md` 6.3, 12.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperframeError {
    /// 1: the CRC does not match (an all-zero block, a corrupted one).
    Crc,
    /// 2: magic is not 0xA5.
    Magic,
    /// 3: version is not 1.
    Version,
    /// 4: `frame mod 192` is not 0.
    Frame,
    /// 5: class is not 0x10, rate not 0..2 or channels not 2.
    Layout,
    /// 6: an op not carried on S/PDIF, or `apply_op = 0` not exactly when
    /// `apply_frame = 0`.
    Apply,
    /// Fewer than 48 bytes.
    Short,
}

impl SuperframeError {
    /// The reference codec's code (`ALINK_SF_ERR_*`); 0 is never returned.
    /// `Short` has no code (the codec takes a full block): 0xFF.
    pub fn code(self) -> u8 {
        match self {
            Self::Crc => 1,
            Self::Magic => 2,
            Self::Version => 3,
            Self::Frame => 4,
            Self::Layout => 5,
            Self::Apply => 6,
            Self::Short => 0xFF,
        }
    }
}

/// The superframe magic (byte 0).
pub const SUPERFRAME_MAGIC: u8 = 0xA5;
/// The protocol version (byte 1).
pub const SUPERFRAME_VERSION: u8 = 1;
/// Flags byte bit 0: system mute.
pub const SF_FLAG_SYSTEM_MUTE: u8 = 1 << 0;
/// Flags byte bit 2: apply pending (derived from `apply_op`).
pub const SF_FLAG_APPLY_PENDING: u8 = 1 << 2;
/// Flags byte bit 3: the source has a live USB stream.
pub const SF_FLAG_USB_STREAM: u8 = 1 << 3;

/// One S/PDIF superframe, the user bits of one IEC 60958 block, 48 bytes
/// (`alink_protocol_spec.md` 12.2). The fixed fields (magic, version,
/// channels 2, class 0x10, the CRC) are not stored: [`SpdifSuperframe::pack`]
/// writes them and [`SpdifSuperframe::unpack`] checks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpdifSuperframe {
    /// Byte 2 as sent; bit 2 is derived on pack (set exactly when an op is
    /// carried).
    pub flags: u8,
    /// Rate code 0..2.
    pub rate: u8,
    /// Byte 6; 0 in this version, ignored by followers.
    pub hops: u8,
    /// The source's serial byte 0 (byte 7).
    pub sender: u8,
    /// F of the block's first frame, a multiple of 192.
    pub frame: u64,
    /// The carried apply: target F (0 none), op, arg, value.
    pub apply: ApplyRequest,
    /// The source's epoch (byte 28).
    pub epoch: u8,
    /// The source's serial byte 1 (byte 29).
    pub sender2: u8,
}

impl SpdifSuperframe {
    pub const SIZE: usize = SPDIF_SUPERFRAME_LEN;

    /// The source id `sender | sender2 << 8` (C5's domain bytes 0-1).
    pub fn source_id(&self) -> u16 {
        u16::from_le_bytes([self.sender, self.sender2])
    }

    /// The 48 bytes: the fixed fields, flags bit 2 derived from the op, the
    /// CRC over bytes 0-29 (low byte first), padding zero.
    pub fn pack(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let op = self.apply.op.raw();
        let flags = if op != 0 {
            self.flags | SF_FLAG_APPLY_PENDING
        } else {
            self.flags & !SF_FLAG_APPLY_PENDING
        };
        put(
            &mut b,
            0,
            &[
                SUPERFRAME_MAGIC,
                SUPERFRAME_VERSION,
                flags,
                self.rate,
                2,
                SPDIF_CARRIER_CLASS,
                self.hops,
                self.sender,
            ],
        );
        put(&mut b, 8, &self.frame.to_le_bytes());
        put(&mut b, 16, &self.apply.frame.to_le_bytes());
        put(&mut b, 24, &[op, self.apply.arg]);
        put(&mut b, 26, &self.apply.value.to_le_bytes());
        put(&mut b, 28, &[self.epoch, self.sender2]);
        let crc = crc16(b.get(..30).unwrap_or(&[]));
        put(&mut b, 30, &crc.to_le_bytes());
        b
    }

    /// Check and decode a block's 48 bytes, in 12.2's order; the first
    /// failing check is the error. `arg` and `value` are not judged (the
    /// unit refuses a bad one when it takes the op).
    pub fn unpack(b: &[u8]) -> Result<Self, SuperframeError> {
        let b = b.get(..Self::SIZE).ok_or(SuperframeError::Short)?;
        let short = SuperframeError::Short;
        let crc = u16_at(b, 30).ok_or(short)?;
        if crc16(b.get(..30).ok_or(short)?) != crc {
            return Err(SuperframeError::Crc);
        }
        if byte(b, 0) != Some(SUPERFRAME_MAGIC) {
            return Err(SuperframeError::Magic);
        }
        if byte(b, 1) != Some(SUPERFRAME_VERSION) {
            return Err(SuperframeError::Version);
        }
        let frame = u64_at(b, 8).ok_or(short)?;
        if frame.checked_rem(SPDIF_BLOCK_FRAMES) != Some(0) {
            return Err(SuperframeError::Frame);
        }
        let rate = byte(b, 3).ok_or(short)?;
        if byte(b, 5) != Some(SPDIF_CARRIER_CLASS) || rate > 2 || byte(b, 4) != Some(2) {
            return Err(SuperframeError::Layout);
        }
        let apply_frame = u64_at(b, 16).ok_or(short)?;
        let op = byte(b, 24).ok_or(short)?;
        let op_ok = op == 0 || spdif_op_allowed(op);
        if !op_ok || (op == 0) != (apply_frame == 0) {
            return Err(SuperframeError::Apply);
        }
        Ok(Self {
            flags: byte(b, 2).ok_or(short)?,
            rate,
            hops: byte(b, 6).ok_or(short)?,
            sender: byte(b, 7).ok_or(short)?,
            frame,
            apply: ApplyRequest {
                frame: apply_frame,
                op: ApplyOp::from_raw(op),
                arg: byte(b, 25).ok_or(short)?,
                value: i16_at(b, 26).ok_or(short)?,
            },
            epoch: byte(b, 28).ok_or(short)?,
            sender2: byte(b, 29).ok_or(short)?,
        })
    }

    /// Whether a superframe of the block at `frame` carries an apply whose
    /// target is `n`: exactly when the block completes before it,
    /// `frame + 192 <= n` (12.2).
    pub fn carries(frame: u64, n: u64) -> bool {
        frame
            .checked_add(SPDIF_BLOCK_FRAMES)
            .is_some_and(|end| end <= n)
    }
}

/// User bit `i` (0..384) of a block whose superframe is `sf`: bit `i mod 8`
/// of byte `i / 8`, least significant first; bit `2k` is channel A of frame
/// `k`, `2k + 1` channel B (12.1). `None` past the block.
pub fn user_bit(sf: &[u8; SPDIF_SUPERFRAME_LEN], i: usize) -> Option<bool> {
    let b = sf.get(i.checked_div(8)?)?;
    let shift = u32::try_from(i.checked_rem(8)?).ok()?;
    Some(b.checked_shr(shift)? & 1 != 0)
}

/// The superframe bytes from a block's 384 user bits in subframe order;
/// `None` unless exactly 384 are given.
pub fn superframe_from_user_bits(bits: &[bool]) -> Option<[u8; SPDIF_SUPERFRAME_LEN]> {
    if bits.len() != SPDIF_USER_BITS {
        return None;
    }
    let mut out = [0u8; SPDIF_SUPERFRAME_LEN];
    for (byte_out, chunk) in out.iter_mut().zip(bits.chunks(8)) {
        for (k, bit) in chunk.iter().enumerate() {
            if *bit {
                *byte_out |= 1u8.checked_shl(u32::try_from(k).ok()?)?;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests_wp_atproto;

#[cfg(test)]
mod tests_wp_planat {
    use super::*;

    #[test]
    fn wp_planat_ids_are_private_and_new() {
        let used: Vec<u16> = super::super::alink::ALINK_IDS
            .iter()
            .chain(super::super::alink_usb::ALINK_USB_IDS.iter())
            .copied()
            .collect();
        for id in ALINK_SPDIF_IDS.iter().chain(TIME_BASE_IDS.iter()) {
            assert!(
                (0x8000..=0x80FF).contains(id),
                "{id:#06x} outside the fork's block range"
            );
            assert!(!used.contains(id), "{id:#06x} already assigned");
        }
    }

    // The layouts fit their lengths (checked at compile time).
    const _: () = assert!(
        TIME_BASE_ENTRY_OFFSET + TIME_BASE_MAX_ENTRIES * TIME_BASE_ENTRY_LEN <= TIME_BASE_LEN
    );
    const _: () = assert!(SPDIF_USER_BITS / 8 == SPDIF_SUPERFRAME_LEN);
    const _: () = assert!(SPDIF_USER_BITS as u64 == 2 * SPDIF_BLOCK_FRAMES);
    const _: () = assert!(APPLY_OP_MAX > 8, "ops 9 and 10 follow the line's 0..8");
}
