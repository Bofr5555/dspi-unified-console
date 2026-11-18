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
