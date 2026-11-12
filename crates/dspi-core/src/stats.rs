//! Signal generator and statistics.
//!
//! Owned by WP C10 (docs/core-merge/PLAN.md).
//!
//! **Signal generator** (firmware siggen, config version 1). The device
//! renders test signals after the matrix mixer, before each output's
//! crossover/PEQ (skipped with `SIGGEN_FLAG_RAW`), delay, trim and volume.
//! Its state is transient: never saved, stopped by a preset load or a
//! factory reset, and it keeps running when the host goes away. Starts and
//! stops arrive as `NOTIFY_EVT_SIGGEN_STATE` (handled with the other
//! notifications, WP C7); fades, gaps and walk steps are visible only by
//! polling [`DspiCore::siggen_get_status`]. The core only sends commands; it
//! never starts the generator by itself.
//!
//! Staging a config and the control actions are writes: they go through
//! `Inner::writable_conn` and are refused ([`DspiError::ReadOnly`]) on a
//! read-only connection.
//!
//! **Statistics** for the System Statistics window: the `REQ_GET_STATUS`
//! `u32` counters, the buffer fill levels with their watermarks, and the
//! S/PDIF receiver. All polled; none is in the bulk image. Every reply is
//! length-checked by the transport (a short reply is an error) and decoded
//! by `dspi_proto`'s bounds-checked codecs; nothing here indexes device
//! data.

use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::{
    BufferStatsPacket, SiggenCapsHeader, SiggenConfig as WireSiggenConfig,
    SiggenStatus as WireSiggenStatus, SiggenTypeDesc, SpdifRxStatusPacket,
};
use dspi_session::ring_health;
use dspi_transport::TransportError;

use crate::consts::{SIGGEN_TYPE_COUNT, SIGGEN_VERSION};
use crate::records::*;
use crate::{DspiCore, DspiError, Result};

/// `REQ_SIGGEN_GET_CAPS` `wValue` for the header (a type index otherwise).
const CAPS_HEADER: u16 = 0xFFFF;
/// `REQ_GET_STATUS` sub-query 9: the meter block (`fetch_status`), not a
/// `u32` counter.
const STATUS_METERS: u16 = 9;
/// `LOOP_PROF_STATUS_BASE` (loop_prof.h): `REQ_GET_STATUS` sub-queries from
/// here up are the loop profiler's in debug builds. Those reads return a
/// value and clear it, and higher ones set the test pacing, so querying them
/// changes state; `fetch_stat` refuses them.
const LOOP_PROF_STATUS_BASE: u16 = 64;
/// S/PDIF receivers a device can have (`SPDIF_RX_NUM_INPUTS`).
const SPDIF_RX_MAX: u8 = 4;
/// IEC 60958 channel status, bytes.
const CHANNEL_STATUS_LEN: u16 = 24;
/// Status byte of an accepted siggen CONTROL or buffer-stats reset.
const ACK: u8 = 1;
/// `REQ_RESET_BUFFER_STATS` `wValue`: reset the watermarks.
const RESET_WATERMARKS: u16 = 0x01;

/// Wire length as a `u16` request length (every size here is a small
/// protocol constant).
fn len16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// A reply that passed the transport's length check but not the codec.
fn malformed(what: &str, e: impl std::fmt::Display) -> DspiError {
    DspiError::Transfer {
        msg: format!("malformed {what} reply: {e}"),
    }
}

/// A siggen write's failure: a stall is the device refusing (an invalid
/// config, START without a valid config, an unknown action), reported as
/// [`DspiError::Refused`] with status 0 (no status byte; its ACK is 1).
fn refusal(e: TransportError) -> DspiError {
    match e {
        TransportError::Stalled { .. } => DspiError::Refused { status: 0 },
        other => other.into(),
    }
}

/// A status byte: `ACK`, or [`DspiError::Refused`] with the byte.
fn acked(reply: &[u8]) -> Result<()> {
    match reply.first() {
        Some(&ACK) => Ok(()),
        Some(&status) => Err(DspiError::Refused { status }),
        None => Err(DspiError::Transfer {
            msg: "empty status reply".into(),
        }),
    }
}

fn caps_type(d: SiggenTypeDesc) -> SiggenTypeInfo {
    SiggenTypeInfo {
        id: d.id,
        name: d.name,
        timing: d.timing_model,
        params: d
            .params
            .iter()
            .map(|p| SiggenParam {
                semantic: p.semantic,
                min: p.min,
                max: p.max,
                def: p.default,
            })
            .collect(),
    }
}

/// An empty type entry: four unused parameters.
fn empty_type() -> SiggenTypeInfo {
    SiggenTypeInfo {
        params: vec![SiggenParam::default(); 4],
        ..Default::default()
    }
}

/// `types` entry for a descriptor that could not be read: id 0xFF, so the
/// host falls back to its own table for that type.
fn unreadable_type() -> SiggenTypeInfo {
    SiggenTypeInfo {
        id: 0xFF,
        ..empty_type()
    }
}

fn config_from_wire(c: WireSiggenConfig) -> SiggenConfig {
    SiggenConfig {
        signal_type: c.signal_type,
        channel_mask: c.channel_mask,
        invert_mask: c.invert_mask,
        flags: c.flags,
        level_db: c.level_db,
        duration_ms: c.duration_ms,
        repeat: c.repeat_count,
        gap_ms: c.gap_ms,
        p: vec![c.p1, c.p2, c.p3, c.p4],
    }
}

/// The wire config, or `InvalidArgument` unless `p` has exactly four
/// parameters.
fn config_to_wire(c: &SiggenConfig) -> Result<WireSiggenConfig> {
    let [p1, p2, p3, p4] =
        <[f32; 4]>::try_from(c.p.as_slice()).map_err(|_| DspiError::InvalidArgument {
            msg: format!(
                "a signal generator config has 4 parameters, not {}",
                c.p.len()
            ),
        })?;
    Ok(WireSiggenConfig {
        version: SIGGEN_VERSION,
        signal_type: c.signal_type,
        channel_mask: c.channel_mask,
        invert_mask: c.invert_mask,
        flags: c.flags,
        level_db: c.level_db,
        duration_ms: c.duration_ms,
        repeat_count: c.repeat,
        gap_ms: c.gap_ms,
        p1,
        p2,
        p3,
        p4,
    })
}

fn buffer_stats(p: BufferStatsPacket) -> BufferStats {
    BufferStats {
        // The firmware has 2 or 4 instances; a larger count is clamped to
        // the 4 entries the packet carries.
        num_spdif: p.num_spdif.min(4),
        pdm_active: p.pdm_active(),
        streaming: p.streaming(),
        sequence: p.sequence,
        spdif: p
            .spdif
            .iter()
            .map(|s| SpdifBufferStats {
                free: s.consumer_free,
                prepared: s.consumer_prepared,
                playing: s.consumer_playing,
                fill_pct: s.consumer_fill_pct,
                min_pct: s.consumer_min_fill_pct,
                max_pct: s.consumer_max_fill_pct,
            })
            .collect(),
        pdm: PdmBufferStats {
            dma_fill_pct: p.pdm.dma_fill_pct,
            dma_min_pct: p.pdm.dma_min_fill_pct,
            dma_max_pct: p.pdm.dma_max_fill_pct,
            ring_fill_pct: p.pdm.ring_fill_pct,
            ring_min_pct: p.pdm.ring_min_fill_pct,
            ring_max_pct: p.pdm.ring_max_fill_pct,
        },
    }
}

fn ring_health_record(h: ring_health::RingHealth) -> RingHealth {
    RingHealth {
        target_frames: h.target.target_frames,
        target_us: h.target.target_us,
        batch_frames: h.target.batch_frames,
        underrun_episodes: h.underrun_episodes,
        refused_blocks: h.refused_blocks,
        lowest_fill: h.lowest_fill,
        worst_input_gap_us: h.worst_input_gap_us,
        worst_usb_gap_us: h.worst_usb_gap_us,
        over_budget_steps: h.over_budget_steps,
    }
}

fn output_fill_record(f: ring_health::OutputFill) -> OutputFill {
    OutputFill {
        fill_pct: f.fill_pct,
        min_pct: f.min_pct,
        max_pct: f.max_pct,
        band: match f.band {
            ring_health::FillBand::Good => FillBand::Good,
            ring_health::FillBand::Warn => FillBand::Warn,
            ring_health::FillBand::Bad => FillBand::Bad,
        },
    }
}

fn spdif_rx_status(p: SpdifRxStatusPacket) -> SpdifRxStatus {
    SpdifRxStatus {
        state: p.state.to_raw(),
        input_source: p.input_source.to_raw(),
        lock_count: p.lock_count,
        loss_count: p.loss_count,
        sample_rate: p.sample_rate,
        parity_errors: p.parity_errors,
        fifo_fill_pct: p.fifo_fill_pct,
        lib_state: p.lib_state,
        // Two 4-bit counters: stable in the high nibble, lost in the low.
        stable_callbacks: p.callback_counts >> 4,
        lost_callbacks: p.callback_counts & 0x0F,
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The generator's capabilities: the `REQ_SIGGEN_GET_CAPS` header and
    /// each signal type's descriptor.
    ///
    /// Ok with `supported = false` when the firmware has no generator (the
    /// header stalls, is short, or has another layout version). `types`
    /// always has [`SIGGEN_TYPE_COUNT`] entries of 4 parameters each; the
    /// first `type_count` (the device's count, clamped to 15) are the
    /// device's, and one whose descriptor could not be read has id `0xFF`
    /// (the host then uses its own table for that type).
    ///
    /// Errors: [`DspiError::Disconnected`]; another transfer failure of the
    /// header (a timeout) as [`DspiError::Transfer`].
    pub fn siggen_fetch_caps(&self) -> Result<SiggenCaps> {
        self.ensure_probed(); // -- WP LK4 --
        let mut guard = self.lock();
        let conn = guard.conn()?;
        let mut caps = SiggenCaps {
            types: vec![empty_type(); SIGGEN_TYPE_COUNT],
            ..Default::default()
        };
        // -- WP LK4 -- the connect probe read the header and every
        // descriptor: answer from it (14 round trips fewer over DSPi Link)
        if let Ok(session) = conn.session()
            && let Some(sg) = session.capabilities().siggen.as_ref()
            && sg.version == SIGGEN_VERSION
            && sg.types.len() == usize::from(sg.type_count)
        {
            let count = usize::from(sg.type_count).min(SIGGEN_TYPE_COUNT);
            caps.type_count = u8::try_from(count).unwrap_or(u8::MAX);
            caps.output_channels = sg.output_channels;
            caps.multitone_max = sg.multitone_max;
            caps.valid_mask = sg.valid_channel_mask;
            for (slot, d) in caps.types.iter_mut().zip(sg.types.iter().take(count)) {
                *slot = caps_type(d.clone());
            }
            caps.supported = true;
            return Ok(caps);
        }
        let t = conn.transport();
        let header = match t.control_in(
            op::REQ_SIGGEN_GET_CAPS,
            CAPS_HEADER,
            len16(SiggenCapsHeader::SIZE),
        ) {
            Ok(b) => match SiggenCapsHeader::decode(&b) {
                Ok(h) if h.version == SIGGEN_VERSION => h,
                _ => return Ok(caps),
            },
            Err(TransportError::Stalled { .. } | TransportError::ShortRead { .. }) => {
                return Ok(caps);
            }
            Err(e) => return Err(e.into()),
        };
        let count = usize::from(header.type_count).min(SIGGEN_TYPE_COUNT);
        // At most SIGGEN_TYPE_COUNT (15), so this never saturates.
        caps.type_count = u8::try_from(count).unwrap_or(u8::MAX);
        caps.output_channels = header.output_channels;
        caps.multitone_max = header.multitone_max;
        caps.valid_mask = header.valid_channel_mask;
        for (index, slot) in (0u16..).zip(caps.types.iter_mut().take(count)) {
            let read = t.control_in(op::REQ_SIGGEN_GET_CAPS, index, len16(SiggenTypeDesc::SIZE));
            *slot = match read {
                Ok(b) => SiggenTypeDesc::decode(&b).map_or_else(|_| unreadable_type(), caps_type),
                Err(TransportError::Disconnected) => return Err(DspiError::Disconnected),
                Err(_) => unreadable_type(),
            };
        }
        caps.supported = true;
        Ok(caps)
    }

    /// The config the generator runs (or last ran), as the device sanitised
    /// it. `p` has 4 entries.
    ///
    /// Errors: [`DspiError::Disconnected`]; a transfer failure, a stall
    /// (no generator) or a short reply as [`DspiError::Transfer`];
    /// [`DspiError::Unsupported`] for a config of another layout version.
    pub fn siggen_get_config(&self) -> Result<SiggenConfig> {
        let b =
            self.lock()
                .conn()?
                .get(op::REQ_SIGGEN_GET_CONFIG, 0, len16(WireSiggenConfig::SIZE))?;
        let c = WireSiggenConfig::decode(&b).map_err(|e| malformed("siggen config", e))?;
        if c.version != SIGGEN_VERSION {
            return Err(DspiError::Unsupported);
        }
        Ok(config_from_wire(c))
    }

    /// Stage a config (a running generator restarts with it, fading). The
    /// device validates type, mask and parameters; it does not play it until
    /// [`SIGGEN_CTL_START`](crate::consts::SIGGEN_CTL_START).
    ///
    /// Current firmware stalls on a refused config; older firmware ACKs even
    /// a refused one over USB, so read it back with
    /// [`siggen_get_config`](Self::siggen_get_config) to be sure.
    ///
    /// Errors: [`DspiError::InvalidArgument`] unless `p` has exactly 4
    /// entries (nothing is sent); [`DspiError::ReadOnly`] on a read-only
    /// connection; [`DspiError::Refused`] (status 0) when the device stalls;
    /// [`DspiError::Disconnected`] or [`DspiError::Transfer`] otherwise.
    pub fn siggen_set_config(&self, config: SiggenConfig) -> Result<()> {
        let bytes = config_to_wire(&config)?.encode();
        let mut guard = self.lock();
        let mut w = guard.writable_conn()?;
        // A write, sent on the Writable (PLAN §1.2); the raw transport only
        // keeps a stall (a refusal) apart from other failures.
        w.conn()
            .transport()
            .control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &bytes)
            .map_err(refusal)?;
        // -- WP P0W -- audible: history only
        let after = dspi_proto::value::Value::Bytes(bytes.to_vec());
        crate::journal::journal(&mut guard, "sig.config", &[], after);
        Ok(())
    }

    /// Run a control action: `SIGGEN_CTL_START` plays the staged config,
    /// `SIGGEN_CTL_STOP` fades out, `SIGGEN_CTL_STOP_NOW` cuts at once
    /// ([`consts`](crate::consts)).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an action above 0xFF (the
    /// firmware reads only the low byte; nothing is sent);
    /// [`DspiError::ReadOnly`] on a read-only connection;
    /// [`DspiError::Refused`] when the device refuses (an unknown action, or
    /// START without a valid config: status 0 for a stall, else the status
    /// byte); [`DspiError::Disconnected`] or [`DspiError::Transfer`]
    /// otherwise.
    pub fn siggen_control(&self, action: u16) -> Result<()> {
        if action > 0xFF {
            return Err(DspiError::InvalidArgument {
                msg: format!("signal generator action {action} is out of range"),
            });
        }
        let mut guard = self.lock();
        let mut w = guard.writable_conn()?;
        // A write-as-read opcode on the Writable (PLAN §1.2); raw so a stall
        // stays a refusal.
        let reply = w
            .conn()
            .transport()
            .control_in(op::REQ_SIGGEN_CONTROL, action, 1)
            .map_err(refusal)?;
        acked(&reply)?;
        // -- WP P0W -- audible: history only, so undo never starts it again
        let after = dspi_proto::value::Value::Choice(u8::try_from(action).unwrap_or(u8::MAX));
        crate::journal::journal(&mut guard, "sig.control", &[], after);
        Ok(())
    }

    /// What the generator is doing now: state (`SIGGEN_STATE_*`: idle, fade
    /// in, run, gap, fade out), elapsed ms, cycles done, the last stop
    /// reason (`SIGGEN_STOP_*`), the walked output (0xFF when not walking)
    /// and the current sweep frequency (Hz, 0 when not sweeping).
    ///
    /// Errors: as [`siggen_get_config`](Self::siggen_get_config).
    pub fn siggen_get_status(&self) -> Result<SiggenStatus> {
        let b =
            self.lock()
                .conn()?
                .get(op::REQ_SIGGEN_GET_STATUS, 0, len16(WireSiggenStatus::SIZE))?;
        let s = WireSiggenStatus::decode(&b).map_err(|e| malformed("siggen status", e))?;
        if s.version != SIGGEN_VERSION {
            return Err(DspiError::Unsupported);
        }
        Ok(SiggenStatus {
            state: s.state,
            signal_type: s.signal_type,
            active_channel: s.active_channel,
            elapsed_ms: s.elapsed_ms,
            cycles_done: s.cycles_done,
            stop_reason: s.stop_reason,
            current_freq: s.current_freq,
        })
    }

    /// One `REQ_GET_STATUS` `u32` counter or reading (`STAT_*` in
    /// [`consts`](crate::consts)); units per constant. Sub-queries the
    /// firmware does not know answer 0.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for sub-query 9 (the meter
    /// block, read by `fetch_status`) and for 64 and above (the debug-build
    /// loop profiler, whose reads change state), nothing sent; [`DspiError::Disconnected`]; a transfer failure
    /// or short reply as [`DspiError::Transfer`].
    pub fn fetch_stat(&self, which: u16) -> Result<u32> {
        if which == STATUS_METERS || which >= LOOP_PROF_STATUS_BASE {
            return Err(DspiError::InvalidArgument {
                msg: format!("status sub-query {which} is not a counter"),
            });
        }
        let b = self.lock().conn()?.get(op::REQ_GET_STATUS, which, 4)?;
        let v = b
            .get(..4)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            .ok_or_else(|| malformed("status counter", "too short"))?;
        let v = u32::from_le_bytes(v);
        self.events.stat(which, v); // -- WP P0M --
        Ok(v)
    }

    /// The output rings' health (Output Ring Health in the Stats window), or
    /// `None` on firmware without `REQ_OUTPUT_LATENCY` or with a short reply to
    /// it: show no section then. Seven transfers.
    ///
    /// Errors: [`DspiError::Disconnected`]; a transfer failure, or a stall or a
    /// short reply on a counter, as [`DspiError::Transfer`]. A counter that
    /// cannot be read fails the call rather than reading as 0.
    pub fn fetch_ring_health(&self) -> Result<Option<RingHealth>> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        // -- WP CG21 -- The re-read that brings in another client's latency change
        let mut cache = conn.ring_target;
        let health = ring_health::read_ring_health_cached(conn.transport(), &mut cache);
        conn.ring_target = cache;
        let health = health?.map(ring_health_record);
        // -- WP P0M -- the event log once `Inner` is released
        drop(inner);
        if let Some(h) = &health {
            self.events.ring_health(h);
        }
        Ok(health)
    }

    /// The live outputs' fill for the Stats rows: to frame resolution, on the
    /// ring's own scale, with the band around the live latency target. Firmware
    /// without `REQ_OUTPUT_LATENCY` keeps its fixed scale and fixed bands. Two
    /// transfers. Use this rather than [`BufferStats`]' percentages, whose
    /// 48-frame steps read as an empty ring at the lowest latency.
    ///
    /// Errors: as for [`DspiCore::fetch_buffer_stats`].
    pub fn fetch_output_fills(&self) -> Result<Vec<OutputFill>> {
        let mut inner = self.lock();
        let fills = ring_health::read_output_fills(inner.conn()?.transport())?;
        Ok(fills.into_iter().map(output_fill_record).collect())
    }

    // -- WP CG21 --
    /// The Stats window's fast poll: the buffer fill levels and the live
    /// outputs' fill, as [`DspiCore::fetch_buffer_stats`] and
    /// [`DspiCore::fetch_output_fills`] give them, from one transfer.
    ///
    /// The fill target is kept on the connection. It is read again, for one more
    /// transfer on the tick after, when this core changed the latency, when a
    /// notification says the settings moved (a preset, a bulk write, a format
    /// change, a lost packet), and after a Link re-attach. The firmware does not
    /// notify a latency change, so another client's reaches this through
    /// [`DspiCore::fetch_ring_health`]: call that on the slow cadence alongside.
    /// While a change has yet to apply the target is read each tick.
    ///
    /// Errors: as for [`DspiCore::fetch_buffer_stats`]. A failed read of the
    /// target is not an error: `fills` is `None`.
    pub fn fetch_stats_tick(&self) -> Result<StatsTick> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        let raw = conn.get(op::REQ_GET_BUFFER_STATS, 0, len16(BufferStatsPacket::SIZE))?;
        let packet = BufferStatsPacket::decode(&raw).map_err(|e| malformed("buffer stats", e))?;
        let mut cache = conn.ring_target;
        let target = cache.target(conn.transport());
        conn.ring_target = cache;
        let fills = target.ok().map(|target| {
            ring_health::decode_output_fills(&raw, target.as_ref())
                .into_iter()
                .map(output_fill_record)
                .collect()
        });
        Ok(StatsTick {
            buffers: buffer_stats(packet),
            fills,
        })
    }

    /// The buffer fill levels (%) and their watermarks
    /// (`REQ_GET_BUFFER_STATS`, 44 bytes). `spdif` has 4 entries; the first
    /// `num_spdif` (at most 4) are live.
    ///
    /// Errors: [`DspiError::Disconnected`]; a transfer failure, a stall or a
    /// short reply as [`DspiError::Transfer`].
    pub fn fetch_buffer_stats(&self) -> Result<BufferStats> {
        let b =
            self.lock()
                .conn()?
                .get(op::REQ_GET_BUFFER_STATS, 0, len16(BufferStatsPacket::SIZE))?;
        let p = BufferStatsPacket::decode(&b).map_err(|e| malformed("buffer stats", e))?;
        Ok(buffer_stats(p))
    }

    /// Start the buffer watermarks again from now
    /// (`REQ_RESET_BUFFER_STATS`). Diagnostics housekeeping that changes no
    /// setting and nothing in the bulk image, so like `clear_clips` it is
    /// allowed on a read-only connection (PLAN §1.2).
    ///
    /// Errors: [`DspiError::Refused`] for a status byte other than ACK (1);
    /// [`DspiError::Disconnected`] or [`DspiError::Transfer`] otherwise.
    pub fn reset_buffer_stats(&self) -> Result<()> {
        let reply = self
            .lock()
            .conn()?
            .get(op::REQ_RESET_BUFFER_STATS, RESET_WATERMARKS, 1)?;
        acked(&reply)
    }

    /// The S/PDIF receiver: lock state (0 inactive, 1 acquiring, 2 locked,
    /// 3 relocking), input source, lock/loss counts, sample rate (Hz when
    /// locked), parity errors, FIFO fill (%), and the receiver library's
    /// debug state and callback counts (4 bits each).
    ///
    /// Errors: [`DspiError::Disconnected`]; a transfer failure, a stall or a
    /// short reply as [`DspiError::Transfer`].
    pub fn fetch_spdif_rx_status(&self) -> Result<SpdifRxStatus> {
        let b = self.lock().conn()?.get(
            op::REQ_GET_SPDIF_RX_STATUS,
            0,
            len16(SpdifRxStatusPacket::SIZE),
        )?;
        let p = SpdifRxStatusPacket::decode(&b).map_err(|e| malformed("S/PDIF RX status", e))?;
        Ok(spdif_rx_status(p))
    }

    /// 24 bytes of IEC 60958 channel status of the locked S/PDIF input.
    ///
    /// Errors: [`DspiError::Disconnected`]; a transfer failure, a stall or a
    /// short reply as [`DspiError::Transfer`].
    pub fn fetch_spdif_rx_channel_status(&self) -> Result<Vec<u8>> {
        self.lock()
            .conn()?
            .get(op::REQ_GET_SPDIF_RX_CH_STATUS, 0, CHANNEL_STATUS_LEN)
    }

    /// GPIO of S/PDIF receiver `index` (0-3).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for `index` 4 and above
    /// (nothing sent); [`DspiError::Unsupported`] when the device has no
    /// receiver `index` (it answers pin 0 for one it lacks; receiver 0
    /// always exists, so its answer is passed on as is);
    /// [`DspiError::Disconnected`]; a transfer failure as
    /// [`DspiError::Transfer`].
    pub fn fetch_spdif_rx_pin(&self, index: u8) -> Result<u8> {
        if index >= SPDIF_RX_MAX {
            return Err(DspiError::InvalidArgument {
                msg: format!("S/PDIF receiver {index} is out of range (0-3)"),
            });
        }
        let b = self
            .lock()
            .conn()?
            .get(op::REQ_GET_SPDIF_RX_PIN, u16::from(index), 1)?;
        match b.first() {
            Some(0) if index > 0 => Err(DspiError::Unsupported),
            Some(&pin) => Ok(pin),
            None => Err(malformed("S/PDIF RX pin", "empty")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c10 {
    use std::sync::Arc;

    use dspi_proto::packets::{
        PdmBufferStats as WirePdm, SiggenParamDesc, SpdifBufferStats as WireSpdif,
    };
    use dspi_proto::wire_compat::WireRead;
    use dspi_transport::mock::{Direction, LogHandle, Reply};
    use dspi_transport::{FakeDevice, MockTransport};
    use proptest::prelude::*;

    use super::*;
    use crate::consts::*;

    /// A core on `t`, attached bare; `writable` marks the connection as
    /// read in this build's wire format (what a full connect would do).
    fn core_on(t: MockTransport, writable: bool) -> (Arc<DspiCore>, LogHandle) {
        let log = t.log_handle();
        let core = DspiCore::new();
        core.attach(Box::new(t), "MOCK".into(), 0);
        if writable {
            core.lock().conn().unwrap().wire = Some(WireRead::Exact);
        }
        (core, log)
    }

    fn sent(log: &LogHandle, opcode: u8) -> Vec<(Direction, u16, Vec<u8>)> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == opcode)
            .map(|e| (e.direction, e.value, e.payload.clone()))
            .collect()
    }

    fn header(type_count: u8) -> Vec<u8> {
        SiggenCapsHeader {
            version: 1,
            type_count,
            output_channels: 9,
            multitone_max: 16,
            valid_channel_mask: 0x01FF,
        }
        .encode()
        .to_vec()
    }

    fn desc(id: u8, name: &str) -> Vec<u8> {
        let mut params = [SiggenParamDesc::default(); 4];
        params[0] = SiggenParamDesc {
            semantic: 1,
            min: 10.0,
            max: 40_000.0,
            default: 1000.0,
        };
        SiggenTypeDesc {
            id,
            name: name.into(),
            timing_model: 2,
            params,
        }
        .encode()
        .to_vec()
    }

    fn sample_config() -> SiggenConfig {
        SiggenConfig {
            signal_type: 6,
            channel_mask: 0x0003,
            invert_mask: 0x0002,
            flags: SIGGEN_FLAG_RAW | SIGGEN_FLAG_WALK,
            level_db: -20.0,
            duration_ms: 5000,
            repeat: 2,
            gap_ms: 250,
            p: vec![20.0, 20_000.0, 3.0, 250.0],
        }
    }

    #[test]
    fn wp_c10_caps_read_the_header_and_every_type() {
        let t = MockTransport::new().reply(
            op::REQ_SIGGEN_GET_CAPS,
            Reply::Sequence(vec![
                Reply::Data(header(3)),
                Reply::Data(desc(0, "sine")),
                Reply::Stall,
                Reply::Data(desc(2, "burstlong")),
            ]),
        );
        let (core, log) = core_on(t, false);
        let caps = core.siggen_fetch_caps().unwrap();
        assert!(caps.supported);
        assert_eq!(
            (
                caps.type_count,
                caps.output_channels,
                caps.multitone_max,
                caps.valid_mask
            ),
            (3, 9, 16, 0x01FF)
        );
        assert_eq!(caps.types.len(), SIGGEN_TYPE_COUNT);
        assert!(caps.types.iter().all(|t| t.params.len() == 4));
        assert_eq!((caps.types[0].id, caps.types[0].name.as_str()), (0, "sine"));
        assert_eq!(caps.types[0].timing, 2);
        assert_eq!(
            caps.types[0].params[0],
            SiggenParam {
                semantic: 1,
                min: 10.0,
                max: 40_000.0,
                def: 1000.0
            }
        );
        assert_eq!(caps.types[1].id, 0xFF, "an unreadable descriptor is marked");
        assert_eq!(caps.types[2].name, "burstlon", "names are 8 bytes");
        assert_eq!(caps.types[3], empty_type());
        let values: Vec<u16> = sent(&log, op::REQ_SIGGEN_GET_CAPS)
            .iter()
            .map(|e| e.1)
            .collect();
        assert_eq!(values, [0xFFFF, 0, 1, 2]);
    }

    #[test]
    fn wp_c10_caps_clamp_the_type_count() {
        let t = MockTransport::new().reply(
            op::REQ_SIGGEN_GET_CAPS,
            Reply::Sequence(vec![Reply::Data(header(200)), Reply::Data(desc(1, "x"))]),
        );
        let (core, log) = core_on(t, false);
        let caps = core.siggen_fetch_caps().unwrap();
        assert_eq!(caps.type_count, 15);
        assert_eq!(caps.types.len(), 15);
        assert_eq!(sent(&log, op::REQ_SIGGEN_GET_CAPS).len(), 16);
    }

    #[test]
    fn wp_c10_caps_without_a_generator_are_unsupported_not_errors() {
        let mut v2 = header(3);
        v2[0] = 2;
        for reply in [Reply::Stall, Reply::Data(vec![1, 2, 3]), Reply::Data(v2)] {
            let t = MockTransport::new().reply(op::REQ_SIGGEN_GET_CAPS, reply);
            let (core, _) = core_on(t, false);
            let caps = core.siggen_fetch_caps().unwrap();
            assert!(!caps.supported);
            assert_eq!(caps.type_count, 0);
            assert_eq!(caps.types.len(), SIGGEN_TYPE_COUNT);
        }
        let t = MockTransport::new().reply(op::REQ_SIGGEN_GET_CAPS, Reply::Disconnect);
        let (core, _) = core_on(t, false);
        assert_eq!(core.siggen_fetch_caps(), Err(DspiError::Disconnected));
    }

    /// Firmware without the generator (it stalls): the caps say so, and
    /// nothing hangs. WP CG22: the demo device has one now, so this is the
    /// demo's capture without the model.
    #[test]
    fn wp_c10_a_device_without_a_generator_says_so() {
        let core = DspiCore::new();
        let options = dspi_transport::FakeOptions {
            extensions: true,
            ..Default::default()
        };
        let fake = FakeDevice::new(dspi_transport::Capture::demo(), options);
        core.attach_probed(Box::new(fake), "DEMO".into(), 0)
            .unwrap();
        assert!(!core.siggen_fetch_caps().unwrap().supported);
        assert!(matches!(
            core.siggen_get_status(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c10_config_reads_back_and_rejects_other_versions() {
        let wire = config_to_wire(&sample_config()).unwrap().encode();
        let t = MockTransport::new().data(op::REQ_SIGGEN_GET_CONFIG, wire.to_vec());
        let (core, _) = core_on(t, false);
        assert_eq!(core.siggen_get_config().unwrap(), sample_config());

        let mut v2 = wire;
        v2[0] = 2;
        let t = MockTransport::new().data(op::REQ_SIGGEN_GET_CONFIG, v2.to_vec());
        let (core, _) = core_on(t, false);
        assert_eq!(core.siggen_get_config(), Err(DspiError::Unsupported));

        let t = MockTransport::new().data(op::REQ_SIGGEN_GET_CONFIG, vec![1u8; 35]);
        let (core, _) = core_on(t, false);
        assert!(matches!(
            core.siggen_get_config(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c10_set_config_sends_the_old_cores_bytes() {
        let (core, log) = core_on(MockTransport::new(), true);
        core.siggen_set_config(sample_config()).unwrap();
        let out = sent(&log, op::REQ_SIGGEN_SET_CONFIG);
        assert_eq!(out.len(), 1);
        let (dir, value, b) = &out[0];
        assert_eq!((*dir, *value), (Direction::Out, 0));
        assert_eq!(b.len(), 36);
        assert_eq!(b[0], 1);
        assert_eq!(&b[1..8], &[6, 3, 0, 2, 0, 5, 0]);
        assert_eq!(&b[8..12], &(-20.0f32).to_le_bytes());
        assert_eq!(&b[12..16], &5000u32.to_le_bytes());
        assert_eq!(&b[16..20], &[2, 0, 250, 0]);
        assert_eq!(&b[32..36], &250.0f32.to_le_bytes());
    }

    #[test]
    fn wp_c10_set_config_checks_before_sending() {
        let (core, log) = core_on(MockTransport::new(), true);
        for n in [0, 3, 5] {
            let mut c = sample_config();
            c.p.resize(n, 0.0);
            assert!(matches!(
                core.siggen_set_config(c),
                Err(DspiError::InvalidArgument { .. })
            ));
        }
        assert!(log.lock().unwrap().is_empty());

        // Read-only: refused, nothing on the wire.
        let (core, log) = core_on(MockTransport::new(), false);
        assert_eq!(
            core.siggen_set_config(sample_config()),
            Err(DspiError::ReadOnly)
        );
        assert!(log.lock().unwrap().is_empty());

        // Current firmware stalls on a refused config.
        let t = MockTransport::new().reply(op::REQ_SIGGEN_SET_CONFIG, Reply::Stall);
        let (core, _) = core_on(t, true);
        assert_eq!(
            core.siggen_set_config(sample_config()),
            Err(DspiError::Refused { status: 0 })
        );
        let t = MockTransport::new().reply(op::REQ_SIGGEN_SET_CONFIG, Reply::Disconnect);
        let (core, _) = core_on(t, true);
        assert_eq!(
            core.siggen_set_config(sample_config()),
            Err(DspiError::Disconnected)
        );
    }

    #[test]
    fn wp_c10_control_is_a_checked_write() {
        let t = MockTransport::new().data(op::REQ_SIGGEN_CONTROL, vec![1u8]);
        let (core, log) = core_on(t, true);
        core.siggen_control(SIGGEN_CTL_STOP_NOW).unwrap();
        assert_eq!(
            sent(&log, op::REQ_SIGGEN_CONTROL),
            [(Direction::In, SIGGEN_CTL_STOP_NOW, vec![])]
        );

        let t = MockTransport::new().data(op::REQ_SIGGEN_CONTROL, vec![0u8]);
        let (core, _) = core_on(t, true);
        assert_eq!(
            core.siggen_control(SIGGEN_CTL_STOP),
            Err(DspiError::Refused { status: 0 })
        );

        // START without a valid config stalls (mock only: no audio).
        let t = MockTransport::new().reply(op::REQ_SIGGEN_CONTROL, Reply::Stall);
        let (core, _) = core_on(t, true);
        assert_eq!(
            core.siggen_control(SIGGEN_CTL_START),
            Err(DspiError::Refused { status: 0 })
        );

        let t = MockTransport::new().data(op::REQ_SIGGEN_CONTROL, vec![1u8]);
        let (core, log) = core_on(t, true);
        assert!(matches!(
            core.siggen_control(0x0101),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert!(log.lock().unwrap().is_empty());

        // Read-only: nothing reaches the device.
        let t = MockTransport::new().data(op::REQ_SIGGEN_CONTROL, vec![1u8]);
        let (core, log) = core_on(t, false);
        assert_eq!(
            core.siggen_control(SIGGEN_CTL_START),
            Err(DspiError::ReadOnly)
        );
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn wp_c10_status_layout() {
        let s = WireSiggenStatus {
            version: 1,
            state: 2,
            signal_type: 4,
            active_channel: 0xFF,
            elapsed_ms: 1500,
            cycles_done: 3,
            stop_reason: 2,
            current_freq: 440.0,
        };
        let t = MockTransport::new().data(op::REQ_SIGGEN_GET_STATUS, s.encode().to_vec());
        let (core, _) = core_on(t, false);
        assert_eq!(
            core.siggen_get_status().unwrap(),
            SiggenStatus {
                state: 2,
                signal_type: 4,
                active_channel: 0xFF,
                elapsed_ms: 1500,
                cycles_done: 3,
                stop_reason: 2,
                current_freq: 440.0,
            }
        );
        let mut v0 = s.encode();
        v0[0] = 0;
        let t = MockTransport::new().data(op::REQ_SIGGEN_GET_STATUS, v0.to_vec());
        let (core, _) = core_on(t, false);
        assert_eq!(core.siggen_get_status(), Err(DspiError::Unsupported));
    }

    #[test]
    fn wp_c10_counters_come_from_the_demo_device() {
        let core = DspiCore::new();
        core.attach_probed(Box::new(FakeDevice::demo()), "DEMO".into(), 0)
            .unwrap();
        assert_eq!(core.fetch_stat(STAT_CLOCK_HZ).unwrap(), 307_200_000);
        assert_eq!(core.fetch_stat(STAT_SAMPLE_RATE).unwrap(), 48_000);
        assert_eq!(core.fetch_stat(STAT_TEMPERATURE).unwrap(), 3_500);
        assert_eq!(core.fetch_stat(STAT_PDM_RING_OVERRUNS).unwrap(), 0);
        // 63 is the last plain sub-query; 64 starts the read-and-reset profiler.
        assert_eq!(core.fetch_stat(LOOP_PROF_STATUS_BASE - 1).unwrap(), 0);
        for bad in [9, LOOP_PROF_STATUS_BASE, 96, 0x100, 0x200, u16::MAX] {
            assert!(matches!(
                core.fetch_stat(bad),
                Err(DspiError::InvalidArgument { .. })
            ));
        }
    }

    #[test]
    fn wp_c10_counter_replies_are_checked() {
        let t = MockTransport::new().data(op::REQ_GET_STATUS, vec![1u8, 2, 3]);
        let (core, _) = core_on(t, false);
        assert!(matches!(
            core.fetch_stat(STAT_CORE_MV),
            Err(DspiError::Transfer { .. })
        ));
        let t = MockTransport::new().data(op::REQ_GET_STATUS, vec![0x34u8, 0x12, 0, 0, 9]);
        let (core, log) = core_on(t, false);
        assert_eq!(core.fetch_stat(STAT_CORE_MV).unwrap(), 0x1234);
        assert_eq!(sent(&log, op::REQ_GET_STATUS)[0].1, STAT_CORE_MV);
    }

    #[test]
    fn wp_c10_buffer_stats_layout() {
        let mut p = BufferStatsPacket {
            num_spdif: 7,
            flags: 3,
            sequence: 0x1234,
            ..Default::default()
        };
        p.spdif[0] = WireSpdif {
            consumer_free: 10,
            consumer_prepared: 4,
            consumer_playing: 1,
            consumer_fill_pct: 37,
            consumer_min_fill_pct: 25,
            consumer_max_fill_pct: 50,
        };
        p.pdm = WirePdm {
            dma_fill_pct: 12,
            dma_min_fill_pct: 5,
            dma_max_fill_pct: 30,
            ring_fill_pct: 7,
            ring_min_fill_pct: 2,
            ring_max_fill_pct: 20,
        };
        let t = MockTransport::new().data(op::REQ_GET_BUFFER_STATS, p.encode().to_vec());
        let (core, _) = core_on(t, false);
        let s = core.fetch_buffer_stats().unwrap();
        assert_eq!(
            (s.num_spdif, s.pdm_active, s.streaming, s.sequence),
            (4, true, true, 0x1234)
        );
        assert_eq!(s.spdif.len(), 4);
        assert_eq!(
            s.spdif[0],
            SpdifBufferStats {
                free: 10,
                prepared: 4,
                playing: 1,
                fill_pct: 37,
                min_pct: 25,
                max_pct: 50
            }
        );
        assert_eq!((s.pdm.dma_fill_pct, s.pdm.ring_max_pct), (12, 20));

        let t = MockTransport::new().data(op::REQ_GET_BUFFER_STATS, vec![0u8; 43]);
        let (core, _) = core_on(t, false);
        assert!(matches!(
            core.fetch_buffer_stats(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c10_reset_buffer_stats_works_on_a_read_only_connection() {
        // Bare attach: not writable (the §1.2 exception, like clear_clips).
        let t = MockTransport::new().data(op::REQ_RESET_BUFFER_STATS, vec![1u8]);
        let (core, log) = core_on(t, false);
        assert!(matches!(
            core.lock().writable_conn(),
            Err(DspiError::ReadOnly)
        ));
        core.reset_buffer_stats().unwrap();
        assert_eq!(
            sent(&log, op::REQ_RESET_BUFFER_STATS),
            [(Direction::In, RESET_WATERMARKS, vec![])]
        );

        let t = MockTransport::new().data(op::REQ_RESET_BUFFER_STATS, vec![0u8]);
        let (core, _) = core_on(t, false);
        assert_eq!(
            core.reset_buffer_stats(),
            Err(DspiError::Refused { status: 0 })
        );
    }

    #[test]
    fn wp_c10_spdif_rx_status_unpacks_debug_bytes() {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&[2, 1, 3, 1]);
        b[4..8].copy_from_slice(&48000u32.to_le_bytes());
        b[8..12].copy_from_slice(&7u32.to_le_bytes());
        b[12..14].copy_from_slice(&52u16.to_le_bytes());
        b[14] = 2;
        b[15] = 0x31;
        let t = MockTransport::new().data(op::REQ_GET_SPDIF_RX_STATUS, b.to_vec());
        let (core, _) = core_on(t, false);
        let s = core.fetch_spdif_rx_status().unwrap();
        assert_eq!(
            (s.state, s.input_source, s.lock_count, s.loss_count),
            (2, 1, 3, 1)
        );
        assert_eq!(
            (s.sample_rate, s.parity_errors, s.fifo_fill_pct),
            (48000, 7, 52)
        );
        assert_eq!(
            (s.lib_state, s.stable_callbacks, s.lost_callbacks),
            (2, 3, 1)
        );
        // Unknown enum values pass through unchanged.
        b[0] = 9;
        b[1] = 200;
        let t = MockTransport::new().data(op::REQ_GET_SPDIF_RX_STATUS, b.to_vec());
        let (core, _) = core_on(t, false);
        let s = core.fetch_spdif_rx_status().unwrap();
        assert_eq!((s.state, s.input_source), (9, 200));
    }

    #[test]
    fn wp_c10_channel_status_is_24_bytes() {
        let cs: Vec<u8> = (0..30).collect();
        let t = MockTransport::new().data(op::REQ_GET_SPDIF_RX_CH_STATUS, cs.clone());
        let (core, _) = core_on(t, false);
        assert_eq!(core.fetch_spdif_rx_channel_status().unwrap(), cs[..24]);
        let t = MockTransport::new().data(op::REQ_GET_SPDIF_RX_CH_STATUS, vec![0u8; 23]);
        let (core, _) = core_on(t, false);
        assert!(core.fetch_spdif_rx_channel_status().is_err());
    }

    #[test]
    fn wp_c10_spdif_rx_pins() {
        let t = MockTransport::new().reply(
            op::REQ_GET_SPDIF_RX_PIN,
            Reply::Sequence(vec![
                Reply::Data(vec![11]),
                Reply::Data(vec![0]),
                Reply::Data(vec![0]),
            ]),
        );
        let (core, log) = core_on(t, false);
        assert_eq!(core.fetch_spdif_rx_pin(0).unwrap(), 11);
        assert_eq!(core.fetch_spdif_rx_pin(2), Err(DspiError::Unsupported));
        assert_eq!(core.fetch_spdif_rx_pin(0).unwrap(), 0);
        assert!(matches!(
            core.fetch_spdif_rx_pin(4),
            Err(DspiError::InvalidArgument { .. })
        ));
        let values: Vec<u16> = sent(&log, op::REQ_GET_SPDIF_RX_PIN)
            .iter()
            .map(|e| e.1)
            .collect();
        assert_eq!(values, [0, 2, 0]);
    }

    #[test]
    fn wp_c10_everything_needs_a_device() {
        let core = DspiCore::new();
        let d = Err::<(), _>(DspiError::Disconnected);
        assert_eq!(core.siggen_fetch_caps().map(|_| ()), d);
        assert_eq!(core.siggen_get_config().map(|_| ()), d);
        assert_eq!(core.siggen_set_config(sample_config()), d);
        assert_eq!(core.siggen_control(SIGGEN_CTL_STOP), d);
        assert_eq!(core.siggen_get_status().map(|_| ()), d);
        assert_eq!(core.fetch_stat(STAT_CLOCK_HZ).map(|_| ()), d);
        assert_eq!(core.fetch_buffer_stats().map(|_| ()), d);
        assert_eq!(core.reset_buffer_stats(), d);
        assert_eq!(core.fetch_spdif_rx_status().map(|_| ()), d);
        assert_eq!(core.fetch_spdif_rx_channel_status().map(|_| ()), d);
        assert_eq!(core.fetch_spdif_rx_pin(0).map(|_| ()), d);
    }

    proptest! {
        /// §1.7: any reply bytes, of any length, give a value or a typed
        /// error, never a panic, on every reader (a mock: no audio).
        #[test]
        fn proptest_wp_c10_any_reply_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..96),
            which in 0u16..32,
            index in 0u8..6,
        ) {
            let (core, _) = core_on(MockTransport::new().answering_everything(bytes), true);
            let _ = core.siggen_get_config();
            let _ = core.siggen_get_status();
            let _ = core.siggen_control(SIGGEN_CTL_STOP_NOW);
            let _ = core.fetch_stat(which);
            let _ = core.fetch_buffer_stats();
            let _ = core.reset_buffer_stats();
            let _ = core.fetch_spdif_rx_status();
            let _ = core.fetch_spdif_rx_channel_status();
            let _ = core.fetch_spdif_rx_pin(index);
            if let Ok(caps) = core.siggen_fetch_caps() {
                prop_assert_eq!(caps.types.len(), SIGGEN_TYPE_COUNT);
                prop_assert!(usize::from(caps.type_count) <= SIGGEN_TYPE_COUNT);
                prop_assert!(caps.types.iter().all(|t| t.params.len() == 4));
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg12 {
    use std::sync::Arc;

    use dspi_transport::mock::Reply;
    use dspi_transport::{FakeDevice, MockTransport};

    use super::*;

    fn core_on(t: MockTransport) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.attach(Box::new(t), "MOCK".into(), 0);
        core
    }

    fn latency() -> Vec<u8> {
        vec![3, 3, 0xD0, 0x07, 96, 0, 48, 0, 0, 7, 0, 0]
    }

    #[test]
    fn wp_cg12_health_record() {
        let replies = [7u32, 6, u32::MAX, 4, 3, 2]
            .iter()
            .map(|v| Reply::Data(v.to_le_bytes().to_vec()))
            .collect();
        let t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .reply(op::REQ_GET_STATUS, Reply::Sequence(replies));
        assert_eq!(
            core_on(t).fetch_ring_health().unwrap(),
            Some(RingHealth {
                target_frames: 96,
                target_us: 2000,
                batch_frames: 48,
                underrun_episodes: 7,
                refused_blocks: 6,
                lowest_fill: None,
                worst_input_gap_us: 4,
                worst_usb_gap_us: 3,
                over_budget_steps: 2,
            })
        );
    }

    #[test]
    fn wp_cg12_no_status_is_no_section() {
        assert_eq!(
            core_on(MockTransport::new()).fetch_ring_health().unwrap(),
            None
        );
        let t = MockTransport::new().data(op::REQ_OUTPUT_LATENCY, vec![0u8; 3]);
        assert_eq!(core_on(t).fetch_ring_health().unwrap(), None);
    }

    #[test]
    fn wp_cg12_a_short_counter_is_a_transfer_error() {
        let t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_STATUS, vec![1u8]);
        assert!(matches!(
            core_on(t).fetch_ring_health(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_cg12_without_a_device() {
        let core = DspiCore::new();
        assert_eq!(core.fetch_ring_health(), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_output_fills(), Err(DspiError::Disconnected));
    }

    #[test]
    fn wp_cg12_fill_record() {
        let mut raw = vec![0u8; BufferStatsPacket::SIZE];
        raw[0] = 1;
        raw[7..12].copy_from_slice(&[6, 13, 25, 0xE8, 0x03]);
        let t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_BUFFER_STATS, raw);
        assert_eq!(
            core_on(t).fetch_output_fills().unwrap(),
            [OutputFill {
                fill_pct: 15.0,
                min_pct: 20,
                max_pct: 38,
                band: FillBand::Good,
            }]
        );
    }

    #[test]
    fn wp_cg12_health_on_the_fake_device() {
        let core = DspiCore::new();
        core.attach(Box::new(FakeDevice::demo()), "FAKE".into(), 0);
        let health = core.fetch_ring_health().unwrap().unwrap();
        assert!(health.target_frames > 0);
        assert_eq!(health.lowest_fill, None);
    }

    #[test]
    fn wp_cg12_fills_on_the_demo_device() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let target = core.fetch_ring_health().unwrap().unwrap().target_frames;
        let fills = core.fetch_output_fills().unwrap();
        assert_eq!(fills.len(), 4);
        let want = f64::from(target) / 512.0 * 100.0;
        for fill in fills {
            assert!((fill.fill_pct - want).abs() < 0.02, "{fill:?}");
            assert_eq!(fill.band, FillBand::Good);
            assert_eq!(fill.min_pct, fill.max_pct);
            assert!(fill.min_pct > 0);
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg21 {
    use std::sync::Arc;

    use dspi_transport::mock::Reply;
    use dspi_transport::{MockTransport, TransportError};

    use super::*;

    fn core_on(t: MockTransport) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.attach(Box::new(t), "MOCK".into(), 0);
        core
    }

    fn latency() -> Vec<u8> {
        vec![3, 3, 0xD0, 0x07, 96, 0, 48, 0, 0, 7, 0, 0]
    }

    /// One live output at 12.5% of the firmware's fixed scale.
    fn buffers() -> Vec<u8> {
        let mut raw = vec![0u8; BufferStatsPacket::SIZE];
        raw[0] = 1;
        raw[7..12].copy_from_slice(&[12, 6, 25, 0xE2, 0x04]);
        raw
    }

    #[test]
    fn wp_cg21_buffers_and_fills_agree_with_the_two_reads() {
        let t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_BUFFER_STATS, buffers());
        let core = core_on(t);
        let tick = core.fetch_stats_tick().unwrap();
        assert_eq!(tick.buffers, core.fetch_buffer_stats().unwrap());
        assert_eq!(tick.fills.unwrap(), core.fetch_output_fills().unwrap());
    }

    #[test]
    fn wp_cg21_a_change_of_device_starts_over() {
        let core = core_on(
            MockTransport::new()
                .data(op::REQ_OUTPUT_LATENCY, latency())
                .data(op::REQ_GET_BUFFER_STATS, buffers()),
        );
        let scaled = core.fetch_stats_tick().unwrap().fills.unwrap();
        assert_eq!(scaled[0].min_pct, 9, "on the ring's scale");

        // Older firmware: no status, so the fixed scale
        core.attach(
            Box::new(MockTransport::new().data(op::REQ_GET_BUFFER_STATS, buffers())),
            "OTHER".into(),
            0,
        );
        let fixed = core.fetch_stats_tick().unwrap().fills.unwrap();
        assert_eq!(fixed[0].min_pct, 6, "the old target stayed behind");
    }

    #[test]
    fn wp_cg21_a_failed_target_read_leaves_the_fills_out() {
        let t = MockTransport::new()
            .reply(
                op::REQ_OUTPUT_LATENCY,
                Reply::Error(TransportError::TimedOut {
                    opcode: 3,
                    retries: 0,
                }),
            )
            .data(op::REQ_GET_BUFFER_STATS, buffers());
        let tick = core_on(t).fetch_stats_tick().unwrap();
        assert_eq!(tick.buffers.num_spdif, 1);
        assert_eq!(tick.fills, None);
    }

    #[test]
    fn wp_cg21_a_failed_buffer_read_fails_the_tick_and_sends_nothing_more() {
        let t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_BUFFER_STATS, vec![0u8; 3]);
        let log = t.log_handle();
        assert!(matches!(
            core_on(t).fetch_stats_tick(),
            Err(DspiError::Transfer { .. })
        ));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .all(|x| x.opcode != op::REQ_OUTPUT_LATENCY)
        );
    }

    #[test]
    fn wp_cg21_without_a_device() {
        assert_eq!(
            DspiCore::new().fetch_stats_tick(),
            Err(DspiError::Disconnected)
        );
    }

    #[test]
    fn wp_cg21_a_raw_latency_write_invalidates() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core.fetch_stats_tick().unwrap();
        let fills = |core: &DspiCore| core.fetch_stats_tick().unwrap().fills.unwrap();
        assert_eq!(fills(&core)[0].band, FillBand::Good);
        // 8 ms: the demo's rings follow it, which at the old target reads as too full
        core.raw_set(op::REQ_OUTPUT_LATENCY, 0, vec![7]).unwrap();
        assert_eq!(fills(&core)[0].band, FillBand::Good);
    }

    #[test]
    fn wp_cg21_an_own_write_is_judged_by_the_new_target_once_applied() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let before = core.fetch_ring_health().unwrap().unwrap().target_frames;
        assert_eq!(
            core.fetch_stats_tick().unwrap().fills.unwrap()[0].band,
            FillBand::Good
        );
        core.demo_latency_timing(1, 3, 0).unwrap();
        core.set_output_latency(7).unwrap();
        let mut last = None;
        for _ in 0..8 {
            last = core.fetch_stats_tick().unwrap().fills;
        }
        let after = core.fetch_ring_health().unwrap().unwrap().target_frames;
        assert!(after > before, "{before} -> {after}");
        assert_eq!(last.unwrap()[0].band, FillBand::Good);
    }
}
