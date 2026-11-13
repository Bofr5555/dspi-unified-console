//! WP SW1 (SYS2 sections 3, 5 and 7): the audio link, the optical or SFP
//! stream from a sender that a unit plays with a fixed delay. Not DSPi Link,
//! the network control protocol of [`crate::link`].
//!
//! The calls read and write the link's extension sub-commands (0x8000-0x800A,
//! the fork's `extension_channel_spec.md` 2.8, `dspi_proto::ext::audio_link`):
//! the unit's role, its lock status and
//! counters, the slots it takes from the frame, the phase-lock mode and its
//! delay, the frame counter, and "apply at frame N".
//!
//! Everything is gated on what the connect probe found (`ExtState`): a unit
//! whose `EXT_LIST` did not name `EXT_LINK_INFO` (stock firmware, the fork
//! without the link) answers [`DspiError::Unsupported`] with no traffic, as
//! does a sub-command the list did not name. The link's events (0xE1 lock,
//! loss and re-anchor, 0xE2 apply results) go into the event log
//! (`events.rs`), only from a device that announces them.
//!
//! History (P0W): slot changes are undoable; a phase or delay change is
//! recorded with an audible hazard (it re-anchors with a short silence), so
//! undo steps over it; apply-at-frame and the counter reset are actions.
//!
//! Notes for SW2 (the hub), from review sw1, not fixed here:
//! - The link's writes send no `PARAM_CHANGED` (2.8). The hub's router
//!   (`dspi-hub` `router.rs` `is_silent`) knows only `EXT_CORE_VREG` as a
//!   silent extension write, so a link write leaves a `Writer` entry waiting,
//!   and the next unrelated scalar notification can take its attribution:
//!   add 0x8000-0x800A to the silent list.
//! - Apply-at-frame `UNMUTE` (op 2) arrives as `REQ_EXTENSION 0x8006`, past
//!   BULKFIX's `GainGuard`, which intercepts `REQ_SET_OUTPUT_MUTE` only. No
//!   effect today (every link build carries `FIX_BULK_DB_POWF`, so the guard
//!   gates nothing there); keep it in mind if that changes.

use dspi_session::ext::alink::{self, LINK_SLOT_SILENT, LINK_SLOTS};
use dspi_session::ext::{
    self as sx, ApplyOp, ApplyRequest, ApplyStatus, FrameCounter, LinkInfo, LinkPhase, LinkSlots,
    LinkStatus,
};

use dspi_transport::Transport;

use dspi_proto::value::Value;

use crate::ext::{read_error, write_error};
use crate::journal;
use crate::{Connection, DspiCore, DspiError, Inner, Result};

/// A slot value that feeds silence to its matrix input
/// ([`DspiCore::set_audio_link_slots`]).
pub const AUDIO_LINK_SLOT_SILENT: u8 = LINK_SLOT_SILENT;

/// `REQ_SET_INPUT_SOURCE`'s value for the audio link input.
pub const INPUT_SOURCE_AUDIO_LINK: u8 = alink::INPUT_SOURCE_LINK;

/// What `EXT_LINK_INFO` reports, read once at connect.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLinkInfo {
    /// `none`, `receiver`, `sender`, `repeater`, `fan_out` or `unknown`.
    pub role: String,
    /// The firmware's role code.
    pub role_code: u8,
    /// The link protocol version the unit speaks.
    pub protocol_version: u8,
    /// Line-rate classes the unit handles: bit 0 L1, bit 1 L2, bit 2 L3.
    pub class_mask: u8,
    /// A sender's link outputs; 1 on a receiver.
    pub max_links: u8,
    /// The most channels in a frame the unit decodes.
    pub max_channels: u8,
    /// Matrix inputs the unit takes from the link (8).
    pub slots_per_unit: u8,
    /// The receive GPIO, 0xFF for none.
    pub rx_pin: u8,
    /// The repeater or sender GPIO, 0xFF for none.
    pub tx_pin: u8,
    /// The firmware's link feature bits: 0 phase lock, 1 apply at frame, 2
    /// repeater, 3 fan-out, 4 test patterns, 5 LOS pin.
    pub feature_bits: u32,
    /// The smallest fixed delay (`d_local + hops_offset`, frames) the unit
    /// accepts under its current output latency option.
    pub d_local_min: u16,
    /// The largest.
    pub d_local_max: u16,
    /// Whether the unit takes audio from the link, so input source 7 is the
    /// link input.
    pub link_input: bool,
    /// Whether the unit sends link lock, loss and re-anchor events.
    pub state_events: bool,
    /// Whether the unit sends apply-at-frame results.
    pub apply_events: bool,
}

/// What `EXT_LINK_STATUS` reports.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLinkStatus {
    /// `off`, `searching`, `locked`, `holdover` (lost: muted, clock held) or
    /// `unknown`.
    pub state: String,
    /// The firmware's state code.
    pub state_code: u8,
    /// The line-rate class, 1-3 (L1-L3); 0 while unknown.
    pub link_class: u8,
    /// 0 = 44.1, 1 = 48, 2 = 96 kHz.
    pub rate_code: u8,
    /// Channels in the frame.
    pub channels: u8,
    /// Repeaters between the sender and this unit.
    pub hops: u8,
    /// The LOS input's level, 0xFF without one.
    pub los_pin: u8,
    /// The sender's id (the low byte of its serial).
    pub sender_id: u8,
    /// The superframe's flags.
    pub flags: u8,
    /// The link frame counter.
    pub frame: u64,
    /// Times the link locked since the counters were cleared.
    pub lock_count: u32,
    /// Times the link was lost.
    pub loss_count: u32,
    /// Frame header check errors.
    pub crc8_errors: u32,
    /// Audio bank check errors.
    pub crc16_errors: u32,
    /// Bit slips.
    pub bit_slips: u32,
    /// Superframe check errors.
    pub superframe_errors: u32,
    /// The clock trim, ppm.
    pub vcxo_ppm: f32,
    /// The phase error, frames (phase lock).
    pub phase_error_frames: f32,
    /// The output ring's fill, frames.
    pub fill_frames: u16,
}

/// What `EXT_LINK_PHASE` reports: the phase-lock mode and the fixed delay.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLinkPhase {
    /// 0 fill servo (as S/PDIF and ADAT), 1 phase lock.
    pub mode: u8,
    /// The unit's fixed delay in link frames (the system's D less its hops).
    pub d_local: u16,
    /// Normally 0: `d_local` already includes the hops.
    pub hops_offset: i16,
    /// The firmware's phase flags.
    pub flags: u8,
    /// 0 free, 1 anchoring, 2 locked, 3 re-anchor pending.
    pub state: u8,
}

/// What `EXT_FRAME_COUNTER` reports.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLinkFrameCounter {
    /// The link frame at the read.
    pub frame: u64,
    /// The unit's clock at the read, µs (wraps).
    pub time_us: u32,
    /// Output frames played, low 32 bits.
    pub output_frames: u32,
}

/// What `EXT_APPLY_AT_FRAME` reports: the unit's queue of ops.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyAtFrameStatus {
    /// Ops waiting for their frame (at most 4).
    pub pending: u8,
    /// The last op: `cancel_all`, `mute`, `unmute`, `preset_load`,
    /// `master_volume`, `out_delay`, `rate`, `click`, `sync_pulse`.
    pub last_op: String,
    /// Its code.
    pub last_op_code: u8,
    /// How it ended: `applied`, `late`, `cancelled`, `refused`.
    pub last_result: String,
    /// Its code.
    pub last_result_code: u8,
    /// The frame it ran at.
    pub last_frame: u64,
}

/// The highest apply-at-frame op code ([`DspiCore::apply_at_frame`]): 0
/// cancel all, 1 mute, 2 unmute, 3 preset load (`arg` slot), 4 master volume
/// (`value` dB x 100), 5 output delay (`arg` output, `value` samples), 6 rate
/// (`arg` rate code), 7 click on output `arg`, 8 sync pulse.
pub const APPLY_OP_MAX: u8 = ApplyOp::MAX;

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The unit's audio link as the connect probe found it, without
    /// traffic: `None` when the device has no link (stock firmware, the fork
    /// without it, role 0, a protocol version this build does not speak, or
    /// a hub that refused the read). `Disconnected` without a device.
    pub fn audio_link_info(&self) -> Result<Option<AudioLinkInfo>> {
        self.ensure_probed();
        let mut inner = self.lock();
        Ok(info_of(&inner.conn()?.ext))
    }

    /// Read `EXT_LINK_INFO` again and keep it: its delay limits follow the
    /// output latency option, so re-read after changing that. Errors as
    /// [`DspiCore::fetch_audio_link_status`].
    pub fn fetch_audio_link_info(&self) -> Result<AudioLinkInfo> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = link_conn(&mut inner, alink::EXT_LINK_INFO)?;
        let i = sx::read_link_info(conn.transport()).map_err(read_error)?;
        conn.ext.link_info = Some(i);
        info_of(&conn.ext).ok_or(DspiError::Unsupported)
    }

    /// Read `EXT_LINK_STATUS`: lock state, frame counter, error counters,
    /// clock trim and phase error.
    ///
    /// `Unsupported`, with no traffic, without the link; also when the read
    /// stalls or a DSPi Link hub refuses it. `Disconnected` without a device;
    /// `Transfer` for a short or failed read.
    pub fn fetch_audio_link_status(&self) -> Result<AudioLinkStatus> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = link_conn(&mut inner, alink::EXT_LINK_STATUS)?;
        let s = sx::read_link_status(conn.transport()).map_err(read_error)?;
        Ok(status(&s))
    }

    /// Read `EXT_LINK_SLOTS`: for each of the 8 matrix inputs, the link slot
    /// it takes, or [`AUDIO_LINK_SLOT_SILENT`]. Errors as
    /// [`DspiCore::fetch_audio_link_status`].
    pub fn fetch_audio_link_slots(&self) -> Result<Vec<u8>> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = link_conn(&mut inner, alink::EXT_LINK_SLOTS)?;
        let s = sx::read_link_slots(conn.transport()).map_err(read_error)?;
        Ok(s.0.to_vec())
    }

    /// Write `EXT_LINK_SLOTS`: which link channel (0-63) feeds each of the 8
    /// matrix inputs, [`AUDIO_LINK_SLOT_SILENT`] for none. A channel the
    /// stream does not carry yet plays silence until it does.
    ///
    /// Errors, with no traffic: `Disconnected`; `Unsupported` without the
    /// link; `InvalidArgument` for a length other than 8 or a value from 64
    /// to 254; `ReadOnly`. From the device: `Refused` when it stalls;
    /// `Denied`, `Locked` or `Transfer` from the link.
    pub fn set_audio_link_slots(&self, slots: Vec<u8>) -> Result<()> {
        self.ensure_probed();
        let mut inner = self.lock();
        link_conn(&mut inner, alink::EXT_LINK_SLOTS)?;
        let slots = <[u8; LINK_SLOTS]>::try_from(slots.as_slice())
            .map(LinkSlots)
            .map_err(|_| DspiError::InvalidArgument {
                msg: format!(
                    "audio link slots: {} given, {LINK_SLOTS} needed",
                    slots.len()
                ),
            })?;
        if !slots.is_valid() {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "audio link slots: each is a channel below {} or {LINK_SLOT_SILENT} for silence",
                    alink::LINK_MAX_CHANNELS
                ),
            });
        }
        // -- fixup sw1 (F1) -- journalled, undoable: live routing, as the
        // crossfeed pair map. The value before is read from the device.
        journal::check_replay(&inner)?;
        let mut w = inner.writable_conn()?;
        let t = w.conn().transport();
        let before = sx::read_link_slots(t)
            .ok()
            .map(|s| Value::Bytes(s.0.to_vec()));
        sx::write_link_slots(t, slots).map_err(write_error)?;
        journal::Pending::of(journal::AUDIO_LINK_SLOTS, &[], before)
            .done_with(&mut inner, Value::Bytes(slots.0.to_vec()));
        Ok(())
    }

    /// Read `EXT_LINK_PHASE`: the phase-lock mode, the fixed delay and the
    /// lock's state. Errors as [`DspiCore::fetch_audio_link_status`].
    pub fn fetch_audio_link_phase(&self) -> Result<AudioLinkPhase> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = link_conn(&mut inner, alink::EXT_LINK_PHASE)?;
        let p = sx::read_link_phase(conn.transport()).map_err(read_error)?;
        Ok(phase(&p))
    }

    /// Write `EXT_LINK_PHASE`: `mode` 0 (fill servo) or 1 (phase lock), the
    /// fixed delay `d_local` in link frames, and `hops_offset` (normally 0, a
    /// trim of -64..64). A change of mode or delay while phase-locked
    /// re-anchors: 2-6 ms of silence.
    ///
    /// Errors as [`DspiCore::set_audio_link_slots`]; `InvalidArgument` for a
    /// mode above 1 or an offset past 64. `Refused` when the effective delay
    /// is outside the unit's limits ([`DspiCore::fetch_audio_link_info`] has
    /// them as they are now).
    pub fn set_audio_link_phase(&self, mode: u8, d_local: u16, hops_offset: i16) -> Result<()> {
        self.ensure_probed();
        if mode > alink::PHASE_MODE_LOCK {
            return Err(DspiError::InvalidArgument {
                msg: format!("audio link phase mode {mode} (0 fill servo, 1 phase lock)"),
            });
        }
        if hops_offset.unsigned_abs() > alink::HOPS_OFFSET_MAX.unsigned_abs() {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "audio link hops offset {hops_offset} (-{m}..={m})",
                    m = alink::HOPS_OFFSET_MAX
                ),
            });
        }
        let mut inner = self.lock();
        link_conn(&mut inner, alink::EXT_LINK_PHASE)?;
        let p = LinkPhase {
            mode,
            d_local,
            hops_offset,
            flags: 0,
            state: 0,
        };
        // -- fixup sw1 (F1) -- journalled, never undone: a change re-anchors
        // with a short silence (an audible hazard, so undo steps over it).
        let mut w = inner.writable_conn()?;
        let t = w.conn().transport();
        let before = sx::read_link_phase(t).ok().map(|b| phase_value(&b));
        sx::write_link_phase(t, p).map_err(write_error)?;
        journal::Pending::of(journal::AUDIO_LINK_PHASE, &[], before)
            .done_with(&mut inner, phase_value(&p));
        Ok(())
    }

    /// Read `EXT_FRAME_COUNTER`: the link frame, the unit's clock and its
    /// output frames at one instant. Errors as
    /// [`DspiCore::fetch_audio_link_status`].
    pub fn fetch_audio_link_frame_counter(&self) -> Result<AudioLinkFrameCounter> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = link_conn(&mut inner, alink::EXT_FRAME_COUNTER)?;
        let c = sx::read_frame_counter(conn.transport()).map_err(read_error)?;
        Ok(frame_counter(&c))
    }

    /// `EXT_APPLY_AT_FRAME`: run `op` ([`APPLY_OP_MAX`] lists them) on the
    /// samples of link frame `frame`, heard `d_local` frames later on every
    /// phase-locked unit at once; a frame already passed runs at once and
    /// reports "late". Op 0 cancels every pending op (`frame` ignored). The
    /// unit queues up to four.
    ///
    /// Errors as [`DspiCore::set_audio_link_slots`]; `InvalidArgument` for an
    /// op above [`APPLY_OP_MAX`] or an `arg` or `value` outside its range
    /// (unused ones 0; a preset slot 0-9, a master volume of -12800..0
    /// hundredths of a dB, a rate code 0-2, a delay of 0 samples or more).
    /// `Refused` when the queue is full, the link is not locked, or an
    /// output or delay is past the device's.
    pub fn apply_at_frame(&self, frame: u64, op: u8, arg: u8, value: i16) -> Result<()> {
        self.ensure_probed();
        let req = ApplyRequest {
            frame,
            op: ApplyOp::from_raw(op),
            arg,
            value,
        };
        if op > APPLY_OP_MAX || !req.args_valid() {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "apply at frame: op {op} with arg {arg} and value {value} (ops 0..={APPLY_OP_MAX}; extension_channel_spec.md 2.8.7)"
                ),
            });
        }
        let mut inner = self.lock();
        link_conn(&mut inner, alink::EXT_APPLY_AT_FRAME)?;
        let mut w = inner.writable_conn()?;
        sx::apply_at_frame(w.conn().transport(), req).map_err(write_error)?;
        // -- fixup sw1 (F1) -- an action: history, never undone
        journal::journal(
            &mut inner,
            journal::AUDIO_LINK_APPLY,
            &[op, arg],
            Value::Bytes(req.encode().to_vec()),
        );
        Ok(())
    }

    /// Read `EXT_APPLY_AT_FRAME`: ops pending and how the last one ended.
    /// Errors as [`DspiCore::fetch_audio_link_status`].
    pub fn fetch_apply_at_frame_status(&self) -> Result<ApplyAtFrameStatus> {
        self.ensure_probed();
        let mut inner = self.lock();
        let conn = link_conn(&mut inner, alink::EXT_APPLY_AT_FRAME)?;
        let s = sx::read_apply_status(conn.transport()).map_err(read_error)?;
        Ok(apply_status(&s))
    }

    /// `EXT_LINK_STATS_RESET`: clear the link's error and lock counters.
    /// Errors as [`DspiCore::set_audio_link_slots`].
    pub fn reset_audio_link_stats(&self) -> Result<()> {
        self.ensure_probed();
        let mut inner = self.lock();
        link_conn(&mut inner, alink::EXT_LINK_STATS_RESET)?;
        let mut w = inner.writable_conn()?;
        sx::reset_link_stats(w.conn().transport()).map_err(write_error)?;
        // -- fixup sw1 (F1) -- an action: history, never undone
        journal::journal(
            &mut inner,
            journal::AUDIO_LINK_STATS_RESET,
            &[],
            Value::Trigger,
        );
        Ok(())
    }

    /// Open the demo device as a unit receiving the audio link (the fake
    /// device's link model, `extension_channel_spec.md` 2.8's example
    /// receiver: locked on L1, 8 channels at 48 kHz, its frame counter
    /// running), for clients and the hub without hardware. Replaces any open
    /// device.
    pub fn open_demo_audio_link(&self) -> Result<()> {
        let demo = dspi_transport::FakeDevice::demo_audio_link();
        let serial = demo.descriptor().serial.clone();
        self.attach_probed(Box::new(demo), serial, 0)
    }
}

/// The open connection when it has the audio link and `EXT_LIST` named
/// `id`; else `Unsupported` (no traffic) or `Disconnected`.
fn link_conn(inner: &mut Inner, id: u16) -> Result<&mut Connection> {
    let conn = inner.conn()?;
    if !conn.ext.audio_link() || !conn.ext.has(id) {
        return Err(DspiError::Unsupported);
    }
    Ok(conn)
}

/// Whether input source `source` may be selected on `conn`: the link input
/// (7) only on a unit that receives the link. Used by `set_input_source`.
pub(crate) fn source_selectable(conn: &Connection, source: u8) -> bool {
    source != INPUT_SOURCE_AUDIO_LINK || conn.ext.link_input()
}

/// The link's info as the connection holds it, when usable.
fn info_of(ext: &dspi_session::ext_state::ExtState) -> Option<AudioLinkInfo> {
    let i = ext.link_info.filter(|_| ext.audio_link())?;
    Some(AudioLinkInfo {
        link_input: ext.link_input(),
        state_events: ext.link_state_events(),
        apply_events: ext.apply_result_events(),
        ..info(&i)
    })
}

fn info(i: &LinkInfo) -> AudioLinkInfo {
    AudioLinkInfo {
        role: i.role.as_str().into(),
        role_code: i.role.raw(),
        protocol_version: i.protocol_version,
        class_mask: i.class_mask,
        max_links: i.max_links,
        max_channels: i.max_channels,
        slots_per_unit: i.slots_per_unit,
        rx_pin: i.rx_pin,
        tx_pin: i.tx_pin,
        feature_bits: i.feature_bits,
        d_local_min: i.d_local_min,
        d_local_max: i.d_local_max,
        link_input: false,
        state_events: false,
        apply_events: false,
    }
}

fn status(s: &LinkStatus) -> AudioLinkStatus {
    AudioLinkStatus {
        state: s.state.as_str().into(),
        state_code: s.state.raw(),
        link_class: s.class,
        rate_code: s.rate_code,
        channels: s.channels,
        hops: s.hops,
        los_pin: s.los_pin,
        sender_id: s.sender_id,
        flags: s.flags,
        frame: s.frame,
        lock_count: s.lock_count,
        loss_count: s.loss_count,
        crc8_errors: s.crc8_errors,
        crc16_errors: s.crc16_errors,
        bit_slips: s.bit_slips,
        superframe_errors: s.superframe_errors,
        vcxo_ppm: s.vcxo_ppm,
        phase_error_frames: s.phase_error_frames,
        fill_frames: s.fill_frames,
    }
}

fn phase(p: &LinkPhase) -> AudioLinkPhase {
    AudioLinkPhase {
        mode: p.mode,
        d_local: p.d_local,
        hops_offset: p.hops_offset,
        flags: p.flags,
        state: p.state,
    }
}

/// The journal's value of a phase setting: `[mode, d_local LE, hops_offset
/// LE]`, without the device's flags and state.
fn phase_value(p: &LinkPhase) -> Value {
    let mut b = vec![p.mode];
    b.extend(p.d_local.to_le_bytes());
    b.extend(p.hops_offset.to_le_bytes());
    Value::Bytes(b)
}

fn frame_counter(c: &FrameCounter) -> AudioLinkFrameCounter {
    AudioLinkFrameCounter {
        frame: c.frame,
        time_us: c.time_us,
        output_frames: c.output_frames,
    }
}

fn apply_status(s: &ApplyStatus) -> ApplyAtFrameStatus {
    ApplyAtFrameStatus {
        pending: s.pending,
        last_op: s.last_op.as_str().into(),
        last_op_code: s.last_op.raw(),
        last_result: s.last_result.as_str().into(),
        last_result_code: s.last_result.raw(),
        last_frame: s.last_frame,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_sw1;
