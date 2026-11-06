//! Names and diagnostics the old desktop core lacked: input-source names
//! and the device name (0x54..0x57), output latency (0x03) and clock
//! diagnostics (0x82).
//!
//! Names and the latency setting go through the Terminal's registry paths
//! (`in.name`, `dev.name`, `out.latency`), so they get its index, value and
//! feature checks: a feature the connect probe did not find is
//! [`DspiError::Unsupported`] and nothing is sent (old firmware answers
//! 0x55/0x57 with a legacy gain or mute, so a name SET must never reach it).
//! The latency status and options and the clock diagnostics are packets the
//! registry reads only one byte of, so they are read whole here.
//!
//! Names: up to 31 bytes of UTF-8 in a 32-byte NUL-terminated field
//! (`input_source_names_spec.md`, `device_name_spec.md`); a leading NUL
//! clears the name. A rename is reported by `NOTIFY_EVT_INPUT_SOURCE_NAME`
//! (0x0F) / `NOTIFY_EVT_DEVICE_NAME` (0x10), whose name field
//! [`decode_name`] reads (WP C7).
//!
//! Owned by WP C8 (docs/core-merge/PLAN.md).

use dspi_proto::generated::opcodes as op;
use dspi_proto::value::Value;
use dspi_session::Session;
use dspi_transport::TransportError;

use crate::consts::CHANNEL_NAME_LEN;
use crate::records::*;
use crate::{DspiCore, DspiError, Result};

/// Longest name, bytes: the 32-byte field keeps a NUL terminator.
const NAME_MAX: usize = CHANNEL_NAME_LEN.saturating_sub(1);
/// `OUTPUT_LATENCY_GET_STATUS` / `OUTPUT_LATENCY_GET_OPTIONS`, the IN
/// `wValue`s of `REQ_OUTPUT_LATENCY` (config.h:261-262).
const LATENCY_GET_STATUS: u16 = 0;
const LATENCY_GET_OPTIONS: u16 = 1;
/// `sizeof(OutputLatencyStatus)` (config.h).
const LATENCY_STATUS_LEN: u16 = 12;
/// `sizeof(OutputLatencyOptionWire)` (config.h).
const LATENCY_OPTION_LEN: usize = 8;
/// Room for 32 options; the firmware has 7 and truncates to `wLength`.
const LATENCY_OPTIONS_READ: u16 = 256;
/// `OUTPUT_LATENCY_FLAG_STREAM_ONLY` (output_latency.h).
const LATENCY_FLAG_STREAM_ONLY: u8 = 0x01;
/// `OUTPUT_LATENCY_WIRE_AVAILABLE` (config.h).
const LATENCY_FLAG_AVAILABLE: u8 = 0x80;
/// Room for a `ClockDiagPacket` (248 bytes with four slots at version 2).
const CLOCK_DIAG_READ: u16 = 512;

/// Probe names (`dspi_session::probe`) of the features read here.
const F_SOURCE_NAMES: &str = "input_source_names";
const F_DEVICE_NAME: &str = "device_name";
const F_LATENCY: &str = "output_latency";

impl DspiCore {
    /// The user's name for input source `source` (`REQ_GET_INPUT_SOURCE_NAME`,
    /// registry `in.name`); "" when unnamed.
    ///
    /// Errors: `Unsupported` on firmware without source names (nothing is
    /// sent); `InvalidArgument` for a source the device does not have;
    /// `Disconnected` without a device or before it was probed; `Transfer`
    /// (or `Denied` / `Locked` over DSPi Link) when the read fails.
    pub fn get_input_source_name(&self, source: u8) -> Result<String> {
        self.read_name("in.name", &[source], F_SOURCE_NAMES)
    }

    /// Name input source `source` (`REQ_SET_INPUT_SOURCE_NAME`, registry
    /// `in.name`); "" clears it. At most 31 bytes of UTF-8, no NUL. The
    /// device stores it in flash from its main loop with a brief output
    /// restart, then notifies `NOTIFY_EVT_INPUT_SOURCE_NAME`; a read right
    /// after the write already returns the new name.
    ///
    /// Errors: `InvalidArgument` for a name that is too long or holds a NUL,
    /// or a source the device does not have (nothing is sent); `ReadOnly` on
    /// a read-only connection; `Unsupported` on firmware without source
    /// names; `Disconnected`; `Transfer`, `Denied` or `Locked` from the link.
    pub fn set_input_source_name(&self, source: u8, name: String) -> Result<()> {
        self.write_name("in.name", &[source], name)
    }

    /// The device's own name (`REQ_GET_DEVICE_NAME`, registry `dev.name`), ""
    /// when unnamed. Errors as for [`DspiCore::get_input_source_name`]
    /// (`Unsupported` on firmware without device names).
    pub fn get_device_name(&self) -> Result<String> {
        self.read_name("dev.name", &[], F_DEVICE_NAME)
    }

    /// Rename the device (`REQ_SET_DEVICE_NAME`, registry `dev.name`); ""
    /// clears it. At most 31 bytes of UTF-8, no NUL; the front panel shows
    /// only printable ASCII. Applied and notified (`NOTIFY_EVT_DEVICE_NAME`)
    /// as a source name. Errors as for [`DspiCore::set_input_source_name`].
    pub fn set_device_name(&self, name: String) -> Result<()> {
        self.write_name("dev.name", &[], name)
    }

    /// `REQ_OUTPUT_LATENCY` status (`wValue` 0, 12 bytes): the requested
    /// and the effective option (a stream-only option requested while USB is
    /// the source falls back to the first option without that flag), the
    /// ring target in µs and frames, the batch in frames, `pending` (1 until
    /// a SET has fully applied) and the option count, as `fields` in the
    /// firmware's order.
    ///
    /// Errors: `Unsupported` on firmware without the setting (nothing is
    /// sent); `Disconnected`; `Transfer` for a short or failed read.
    pub fn fetch_output_latency(&self) -> Result<OutputLatency> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        require(conn.session()?, F_LATENCY)?;
        let raw = conn.get(
            op::REQ_OUTPUT_LATENCY,
            LATENCY_GET_STATUS,
            LATENCY_STATUS_LEN,
        )?;
        let fields = latency_fields(&raw).ok_or_else(|| short(op::REQ_OUTPUT_LATENCY, &raw))?;
        Ok(OutputLatency { raw, fields })
    }

    /// The output-latency options (`REQ_OUTPUT_LATENCY` `wValue` 1), in the
    /// firmware's order, with what choosing each gives on the active source
    /// and rate (`effective_us`; a stream-only option on USB gives the
    /// fallback's latency). A trailing partial entry is ignored.
    ///
    /// Errors as for [`DspiCore::fetch_output_latency`].
    pub fn fetch_output_latency_options(&self) -> Result<Vec<OutputLatencyOption>> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        require(conn.session()?, F_LATENCY)?;
        let raw = conn.get_upto(
            op::REQ_OUTPUT_LATENCY,
            LATENCY_GET_OPTIONS,
            LATENCY_OPTIONS_READ,
        )?;
        Ok(latency_options(&raw))
    }

    /// Request output-latency option `id` (`OutputLatencyId`, 1..=7: 0.67,
    /// 1, 2, 3, 4, 6 and 8 ms; registry `out.latency`). The device applies
    /// it from its main loop (a directory write and a synchronized output
    /// restart): until then [`DspiCore::fetch_output_latency`] reports the
    /// old `requested_id` with `pending` 1. A stream-only option (0.67 and
    /// 1 ms) is stored as asked, but USB input runs the fallback.
    ///
    /// Errors: `InvalidArgument` for an id that is not an option (nothing is
    /// sent); `ReadOnly` on a read-only connection; `Unsupported` on
    /// firmware without the setting; `Disconnected`; `Transfer`, `Denied`
    /// or `Locked` from the link.
    pub fn set_output_latency(&self, id: u8) -> Result<()> {
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        w.session()?.send("out.latency", &[], Value::Choice(id))?;
        Ok(())
    }

    /// `REQ_GET_CLOCK_DIAG` (0x82): the `ClockDiagPacket` (clock_diag.h)
    /// latched by the firmware's 1 Hz poll, decoded into `fields` in packet
    /// order (`slot_*[i]` per S/PDIF slot; `_cppm` are ppm × 100, `_mhz`
    /// Hz × 1000, `_q16` Q16.16 samples per frame). Decoding stops at the
    /// end of the reply, so an older packet version simply has fewer fields.
    ///
    /// `available` is false, with `raw` and `fields` empty, on firmware
    /// built without `DSPI_CLOCK_DIAG` (a zero-length reply), when the
    /// packet does not fit the transport, or on firmware without the
    /// opcode (it stalls). Errors: `Disconnected`; `Transfer`, `Denied` or
    /// `Locked` from the link.
    pub fn fetch_clock_diag(&self) -> Result<ClockDiag> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        // Straight to the transport, not `Connection::get_upto`: the stall
        // must stay a `TransportError` to read as "not built in". No busy
        // retry either: on USB `with_busy_retry` treats a stall as busy and
        // retries it, which would make every read of a build without the
        // opcode slow, and this is a diagnostic poll the caller repeats.
        let raw = match conn
            .transport()
            .control_in_upto(op::REQ_GET_CLOCK_DIAG, 0, CLOCK_DIAG_READ)
        {
            Ok(raw) => raw,
            Err(TransportError::Stalled { .. }) => return Ok(ClockDiag::default()),
            Err(e) => return Err(e.into()),
        };
        if raw.is_empty() {
            return Ok(ClockDiag::default());
        }
        let fields = clock_diag_fields(&raw);
        Ok(ClockDiag {
            available: true,
            raw,
            fields,
        })
    }

    fn read_name(&self, path: &str, indices: &[u8], feature: &str) -> Result<String> {
        let mut inner = self.lock();
        let session = inner.conn()?.session()?;
        require(session, feature)?;
        match session.read(path, indices)? {
            Value::Text(t) => Ok(decode_name(t.as_bytes())),
            other => Err(DspiError::Transfer {
                msg: format!("{path}: expected a name, read {other:?}"),
            }),
        }
    }

    fn write_name(&self, path: &str, indices: &[u8], name: String) -> Result<()> {
        check_name(&name)?;
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        // `send` refuses a feature the probe did not find (`Unsupported`) and
        // a source the device lacks, then pads the name with NULs to the
        // 32-byte field: "" sends a leading NUL, which clears it.
        w.session()?.send(path, indices, Value::Text(name))?;
        Ok(())
    }
}

/// A name from the device (a GET reply or a 0x0F/0x10 notification's name
/// field): UTF-8, decoded lossily, up to the first NUL and at most 31 bytes.
pub(crate) fn decode_name(field: &[u8]) -> String {
    let field = field.get(..field.len().min(NAME_MAX)).unwrap_or(&[]);
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(field.get(..end).unwrap_or(&[])).into_owned()
}

/// A name the device can hold: at most 31 bytes, no NUL (it would cut the
/// name short, or clear it when leading).
fn check_name(name: &str) -> Result<()> {
    if name.len() > NAME_MAX {
        return Err(DspiError::InvalidArgument {
            msg: format!(
                "a name holds at most {NAME_MAX} bytes of UTF-8; this one has {}",
                name.len()
            ),
        });
    }
    if name.contains('\0') {
        return Err(DspiError::InvalidArgument {
            msg: "a name cannot contain a NUL character".into(),
        });
    }
    Ok(())
}

/// `Unsupported` unless the connect probe found `feature`.
fn require(session: &Session, feature: &str) -> Result<()> {
    let present = session
        .capabilities()
        .features
        .iter()
        .any(|f| f.name == feature && f.present);
    if present {
        Ok(())
    } else {
        Err(DspiError::Unsupported)
    }
}

fn short(opcode: u8, raw: &[u8]) -> DspiError {
    DspiError::Transfer {
        msg: format!("short reply to opcode 0x{opcode:02X} ({} bytes)", raw.len()),
    }
}

/// A little-endian reader over a device reply: every read is checked, and
/// `None` once the reply runs out.
struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[b]| b)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.take().map(i32::from_le_bytes)
    }
}

fn named(name: impl Into<String>, value: impl ToString) -> NamedValue {
    NamedValue {
        name: name.into(),
        value: value.to_string(),
    }
}

/// `OutputLatencyStatus` (config.h) as named fields; `None` when shorter
/// than 12 bytes.
fn latency_fields(b: &[u8]) -> Option<Vec<NamedValue>> {
    let mut c = Cursor(b);
    let v = vec![
        named("requested_id", c.u8()?),
        named("effective_id", c.u8()?),
        named("effective_us", c.u16()?),
        named("target_frames", c.u16()?),
        named("batch_frames", c.u16()?),
        named("pending", c.u8()?),
        named("option_count", c.u8()?),
    ];
    // The reserved u16 completes the 12 bytes.
    c.u16()?;
    Some(v)
}

/// `OutputLatencyOptionWire` entries (config.h), 8 bytes each.
fn latency_options(b: &[u8]) -> Vec<OutputLatencyOption> {
    b.as_chunks::<LATENCY_OPTION_LEN>()
        .0
        .iter()
        .filter_map(|e| {
            let mut c = Cursor(e);
            let id = c.u8()?;
            let flags = c.u8()?;
            Some(OutputLatencyOption {
                id,
                stream_only: flags & LATENCY_FLAG_STREAM_ONLY != 0,
                available: flags & LATENCY_FLAG_AVAILABLE != 0,
                latency_us: c.u16()?,
                batch_us: c.u16()?,
                effective_us: c.u16()?,
            })
        })
        .collect()
}

/// One field of `ClockDiagPacket` (clock_diag.h), in packet order.
#[derive(Clone, Copy)]
enum Diag {
    U8(&'static str),
    U16(&'static str),
    U32(&'static str),
    I32(&'static str),
    /// `uint8_t name[num_slots]`.
    SlotsU8(&'static str),
    /// `uint32_t name[num_slots]`.
    SlotsU32(&'static str),
}

/// `ClockDiagPacket`, version 2 (version 1 ends before `fb_q16`).
const CLOCK_DIAG: &[Diag] = {
    use Diag::*;
    &[
        U8("version"),
        U8("platform"),
        U8("num_slots"),
        U8("pll_lock"),
        U32("sysclk_last_hz"),
        U32("sysclk_min_hz"),
        U32("sysclk_max_hz"),
        U32("sysclk_count"),
        I32("vcxo_last_cppm"),
        I32("vcxo_min_cppm"),
        I32("vcxo_max_cppm"),
        U32("vcxo_pulse_hz"),
        U32("vcxo_pwm_wrap"),
        U16("vcxo_pwm_div"),
        U16("vcxo_fbdiv_live"),
        U32("vcxo_set_calls"),
        U32("vcxo_clamp_sats"),
        U32("vcxo_sign_changes"),
        U8("vcxo_fbdiv0"),
        U8("vcxo_parked"),
        U8("loop_pulse_busy"),
        U8("loop_pulse_en"),
        U8("loop_pace_busy"),
        U8("loop_pace_en"),
        U32("loop_pulse_tcr"),
        U32("loop_pace_tcr"),
        U32("loop_pace_ctdreq"),
        U32("loop_pwm_moves"),
        U16("loop_pwm_ctr"),
        U32("servo_actual_mhz"),
        I32("servo_ff_cppm"),
        I32("servo_total_cppm"),
        I32("servo_min_cppm"),
        I32("servo_max_cppm"),
        I32("servo_int_cppm"),
        I32("servo_fill_error"),
        I32("servo_fill_slot"),
        U32("servo_ticks"),
        SlotsU8("slot_fill"),
        SlotsU8("slot_min_fill"),
        SlotsU8("slot_is_i2s"),
        SlotsU32("slot_starv"),
        SlotsU32("slot_dwc"),
        U8("pdm_enabled"),
        U8("pdm_ring_fill"),
        U8("pdm_ring_min"),
        U8("pdm_dma_fill"),
        U32("pdm_ring_ur"),
        U32("pdm_dma_ur"),
        U8("in_src"),
        U8("in_state"),
        U8("in_lock"),
        U8("in_loss"),
        U32("in_rate"),
        U32("in_parity"),
        U16("in_fifo_pct"),
        U8("in_which_long"),
        U8("_rsv"),
        U32("in_hz_long_mhz"),
        U32("in_span_ms"),
        U32("loop_gapmax_us"),
        U32("fb_q16"),
        U32("fb_min_q16"),
        U32("fb_max_q16"),
        U32("fb_rate_iir_q16"),
        U32("fb_rate_long_q16"),
        U32("fb_long_span_ms"),
        I32("fb_servo_q16"),
        I32("fb_fill_err_q16"),
        U32("fb_updates"),
        U32("fb_merged"),
        U32("fb_rejected"),
        U32("fb_reanchors"),
    ]
};

/// Most S/PDIF slots a packet may claim (`NUM_SPDIF_INSTANCES` is 4 on an
/// RP2350): a larger `num_slots` is malformed, and decoding stops before
/// the per-slot arrays.
const MAX_DIAG_SLOTS: u8 = 8;

/// `ClockDiagPacket` as named fields, in packet order, as far as the reply
/// goes. The `_rsv` padding is skipped.
fn clock_diag_fields(b: &[u8]) -> Vec<NamedValue> {
    let mut c = Cursor(b);
    let mut out = Vec::new();
    let mut slots: Option<u8> = None;
    for f in CLOCK_DIAG {
        let ok = match *f {
            Diag::U8(n) => c.u8().map(|v| {
                if n == "num_slots" {
                    slots = Some(v);
                }
                if n != "_rsv" {
                    out.push(named(n, v));
                }
            }),
            Diag::U16(n) => c.u16().map(|v| out.push(named(n, v))),
            Diag::U32(n) => c.u32().map(|v| out.push(named(n, v))),
            Diag::I32(n) => c.i32().map(|v| out.push(named(n, v))),
            Diag::SlotsU8(n) | Diag::SlotsU32(n) => {
                let Some(count) = slots.filter(|s| *s <= MAX_DIAG_SLOTS) else {
                    break;
                };
                let wide = matches!(f, Diag::SlotsU32(_));
                (0..count)
                    .map(|i| {
                        let v = if wide { c.u32() } else { c.u8().map(u32::from) };
                        v.map(|v| out.push(named(format!("{n}[{i}]"), v)))
                    })
                    .collect::<Option<()>>()
            }
        };
        if ok.is_none() {
            break;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c8;
