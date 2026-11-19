//! WP ATPROTO (PLAN-AT 1.3): the I/O of the time-base contract over the
//! extension channel: `EXT_TIME_BASE` (0x8030), `EXT_SIGGEN_TIMELINE`
//! (0x8031), the S/PDIF block (0x8020-0x8022), `EXT_APPLY_AT_FRAME` with ops
//! 9 and 10 on every time base, and event 0xE4 behind `EXT_NOTIFY_FEATURES`
//! bit 5. The codecs are `dspi_proto::ext::timebase`.
//!
//! **Both envelopes.** Every call is a `REQ_EXTENSION` 0x00 request; on the
//! 1.1.7 generation the paged adapter (`dspi_transport::paged`) carries it
//! in the tunnel `0xE010` with the same `wValue` and bytes, so the helpers do
//! not know the envelope.
//!
//! **Listed first.** Each helper takes the unit's `EXT_LIST`
//! (`ExtState::supported`) and refuses locally, with nothing sent, when the id
//! it needs is not named there ([`ExtError::Unsupported`]): stock firmware
//! (G1s, G2s), RP2040 builds and the fork without these blocks see no extra
//! traffic (`extension_channel_spec.md` 2.10, 2.11). A request the unit would
//! refuse by its arguments alone is refused here too
//! ([`ExtError::BadArgument`]), also with nothing sent.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::alink;
use dspi_proto::ext::alink_usb;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{Transport, TransportError};

pub use dspi_proto::ext::alink::{ApplyOp, ApplyRequest, ApplyStatus};
pub use dspi_proto::ext::timebase::{
    self, AlinkSpdif, AlinkSpdifStatus, ContractState, SiggenTimeline, SpdifMode, SpdifSuperframe,
    TimeBaseEntry, TimeBaseEvent, TimeBaseRead, TimeBaseReason, TimeBaseRole, TimelineResult,
    TimelineState, time_base_event,
};

use crate::ext::{ExtError, Result};

/// [`ExtError::Unsupported`] unless `supported` names `id`.
fn listed(supported: &[u16], id: u16) -> Result<()> {
    if supported.contains(&id) {
        Ok(())
    } else {
        Err(ExtError::Unsupported { id })
    }
}

/// One fixed-size read of a listed id, decoded: a stall is
/// [`ExtError::Unsupported`], a short reply a `ShortRead`, a reply of a layout
/// this build does not parse `Malformed`.
fn read_listed<T>(
    t: &mut dyn Transport,
    supported: &[u16],
    id: u16,
    size: usize,
    decode: impl Fn(&[u8]) -> Option<T>,
) -> Result<T> {
    listed(supported, id)?;
    let len = u16::try_from(size).unwrap_or(u16::MAX);
    let b = t
        .control_in(op::REQ_EXTENSION, id, len)
        .map_err(|e| match e {
            TransportError::Stalled { .. } => ExtError::Unsupported { id },
            e => ExtError::Transport(e),
        })?;
    if b.len() < size {
        return Err(ExtError::Transport(TransportError::ShortRead {
            opcode: op::REQ_EXTENSION,
            wanted: size,
            got: b.len(),
        }));
    }
    decode(&b).ok_or_else(|| {
        ExtError::Transport(TransportError::Malformed(format!(
            "extension 0x{id:04X} {}: a layout this build does not read",
            dspi_proto::ext::name(id)
        )))
    })
}

/// A write of a listed id; a stall is [`ExtError::Refused`].
fn write_listed(t: &mut dyn Transport, supported: &[u16], id: u16, payload: &[u8]) -> Result<()> {
    listed(supported, id)?;
    t.control_out(op::REQ_EXTENSION, id, payload)
        .map_err(|e| match e {
            TransportError::Stalled { .. } => ExtError::Refused { id },
            e => ExtError::Transport(e),
        })
}

// ------------------------------------------------------------ the contract

/// `EXT_TIME_BASE` IN: every time base the unit has, in one atomic read
/// (2.11.1).
pub fn read_time_base(t: &mut dyn Transport, supported: &[u16]) -> Result<TimeBaseRead> {
    read_listed(
        t,
        supported,
        timebase::EXT_TIME_BASE,
        TimeBaseRead::SIZE,
        TimeBaseRead::decode,
    )
}

/// `EXT_SIGGEN_TIMELINE` IN: where the generator sits on the time base
/// (2.11.2).
pub fn read_siggen_timeline(t: &mut dyn Transport, supported: &[u16]) -> Result<SiggenTimeline> {
    read_listed(
        t,
        supported,
        timebase::EXT_SIGGEN_TIMELINE,
        SiggenTimeline::SIZE,
        SiggenTimeline::decode,
    )
}

// ------------------------------------------------------------ the S/PDIF block

/// `EXT_ALINK_SPDIF` IN (2.10.1).
pub fn read_alink_spdif(t: &mut dyn Transport, supported: &[u16]) -> Result<AlinkSpdif> {
    read_listed(
        t,
        supported,
        timebase::EXT_ALINK_SPDIF,
        AlinkSpdif::SIZE,
        AlinkSpdif::decode,
    )
}

/// `EXT_ALINK_SPDIF_STATUS` IN (2.10.2).
pub fn read_alink_spdif_status(
    t: &mut dyn Transport,
    supported: &[u16],
) -> Result<AlinkSpdifStatus> {
    read_listed(
        t,
        supported,
        timebase::EXT_ALINK_SPDIF_STATUS,
        AlinkSpdifStatus::SIZE,
        AlinkSpdifStatus::decode,
    )
}

/// `EXT_ALINK_SPDIF_CTRL` IN: a source's broadcast queue (2.10.3).
pub fn read_alink_spdif_ctrl(t: &mut dyn Transport, supported: &[u16]) -> Result<ApplyStatus> {
    read_listed(
        t,
        supported,
        timebase::EXT_ALINK_SPDIF_CTRL,
        ApplyStatus::SIZE,
        timebase::decode_spdif_ctrl_status,
    )
}

/// `EXT_ALINK_SPDIF` OUT (the IN-only state sent as 0). Takes effect at once;
/// `REQ_SAVE_OUTPUT_CONFIG` persists it. [`ExtError::BadArgument`] before the
/// wire for what [`AlinkSpdif::is_valid`] refuses; [`ExtError::Refused`] when
/// the unit stalls (D outside `d_min..d_max`, a `tx_mask` slot that is not
/// S/PDIF, mode 2 without an S/PDIF output).
pub fn write_alink_spdif(t: &mut dyn Transport, supported: &[u16], cfg: AlinkSpdif) -> Result<()> {
    listed(supported, timebase::EXT_ALINK_SPDIF)?;
    if !cfg.is_valid() {
        return Err(ExtError::BadArgument(format!(
            "audio link S/PDIF: mode {}, flags 0x{:02X}, on_lock_error {}, tx_mask 0x{:02X} \
             (modes 0-2, flag bit 0 only, on_lock_error 0-1, tx_mask on a source only; \
             extension_channel_spec.md 2.10.1)",
            cfg.mode.raw(),
            cfg.flags,
            cfg.on_lock_error,
            cfg.tx_mask
        )));
    }
    write_listed(t, supported, timebase::EXT_ALINK_SPDIF, &cfg.encode_out())
}

/// `EXT_ALINK_SPDIF_CTRL` OUT on a source: one op carried in the S/PDIF
/// superframe to every follower and applied by the source at the same F
/// (2.10.3). [`ExtError::BadArgument`] before the wire for an op the S/PDIF
/// superframe never carries (0, 3, 5, 6, 8) or an argument out of range;
/// [`ExtError::Refused`] when the unit stalls (a lead under 896 frames, a
/// broadcast already pending, not a locked source).
pub fn write_spdif_ctrl(t: &mut dyn Transport, supported: &[u16], req: ApplyRequest) -> Result<()> {
    listed(supported, timebase::EXT_ALINK_SPDIF_CTRL)?;
    if !timebase::spdif_ctrl_valid(&req) {
        return Err(ExtError::BadArgument(format!(
            "audio link S/PDIF broadcast: op {} with arg {} and value {} (ops 1, 2, 4, 7, 9, 10; \
             extension_channel_spec.md 2.10.3)",
            req.op.raw(),
            req.arg,
            req.value
        )));
    }
    write_listed(
        t,
        supported,
        timebase::EXT_ALINK_SPDIF_CTRL,
        &timebase::encode_spdif_ctrl(&req),
    )
}

// ------------------------------------------------------------ apply at T

/// `EXT_APPLY_AT_FRAME` OUT on time base `time_base` (byte 12): 0 the line's
/// F, 1 the USB sample S, 2 a USB bus frame, 3 the S/PDIF F; ops 0-10
/// (2.8.7, 2.8.7.1, 2.9.4, 2.10.4).
///
/// Before the wire: [`ExtError::BadArgument`] for an op or argument out of
/// range, op 6, a time base above 3, and **op 9 in the bus-frame form**
/// (review R5: write it as S); [`ExtError::Unsupported`] unless `EXT_LIST`
/// names `EXT_APPLY_AT_FRAME`, the time base's own block (`0x8000` the line,
/// `0x8010` USB-direct, `0x8020` S/PDIF) and, for ops 9 and 10,
/// `EXT_SIGGEN_TIMELINE`. [`ExtError::Refused`] when the unit stalls (not
/// locked, a full queue, op 9 with less than `arm_lead` or more than 120 s
/// of lead, or while another op 9 is pending).
pub fn apply_at(
    t: &mut dyn Transport,
    supported: &[u16],
    req: ApplyRequest,
    time_base: u8,
) -> Result<()> {
    if !timebase::apply_form_valid(&req, time_base) {
        return Err(ExtError::BadArgument(format!(
            "apply at frame: time base {time_base}, frame {:#x}, op {} with arg {} and value {} \
             (time bases 0-3, ops 0-10 but 6, op 9 never on time base 2; \
             extension_channel_spec.md 2.8.7, 2.8.7.1)",
            req.frame,
            req.op.raw(),
            req.arg,
            req.value
        )));
    }
    listed(supported, alink::EXT_APPLY_AT_FRAME)?;
    let block = match time_base {
        timebase::TB_ALINK_LINE => alink::EXT_ALINK_INFO,
        timebase::TB_SPDIF => timebase::EXT_ALINK_SPDIF,
        _ => alink_usb::EXT_ALINK_USB,
    };
    listed(supported, block)?;
    if matches!(req.op, ApplyOp::SiggenStart | ApplyOp::SiggenStop) {
        listed(supported, timebase::EXT_SIGGEN_TIMELINE)?;
    }
    write_listed(
        t,
        supported,
        alink::EXT_APPLY_AT_FRAME,
        &timebase::encode_apply_tb(&req, time_base),
    )
}

/// Op 9 `SIGGEN_START` at sample `t_start` of time base `time_base` (never
/// 2): the staged generator config starts on exactly that sample, or not at
/// all (`test_signals_spec.md` 15). As [`apply_at`].
pub fn siggen_start_at(
    t: &mut dyn Transport,
    supported: &[u16],
    time_base: u8,
    t_start: u64,
) -> Result<()> {
    let req = ApplyRequest {
        frame: t_start,
        op: ApplyOp::SiggenStart,
        arg: 0,
        value: 0,
    };
    apply_at(t, supported, req, time_base)
}

/// Op 10 `SIGGEN_STOP` at sample `t_stop`: with a fade, or at once when
/// `now` is set. As [`apply_at`]; a late op 10 is applied at once.
pub fn siggen_stop_at(
    t: &mut dyn Transport,
    supported: &[u16],
    time_base: u8,
    t_stop: u64,
    now: bool,
) -> Result<()> {
    let req = ApplyRequest {
        frame: t_stop,
        op: ApplyOp::SiggenStop,
        arg: if now {
            timebase::SIGGEN_STOP_NOW
        } else {
            timebase::SIGGEN_STOP_FADE
        },
        value: 0,
    };
    apply_at(t, supported, req, time_base)
}

#[cfg(test)]
mod tests_wp_atproto;

// -- review AT-A/AT-B -- ATPROTO's helpers against ATFAKE's real fakes
#[cfg(test)]
mod tests_review_atab;
