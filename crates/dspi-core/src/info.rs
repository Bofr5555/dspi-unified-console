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
use dspi_transport::{InRead, TransportError};
// -- WP QD4 --
use dspi_transport::TransportKind;
use dspi_transport::fake::EXT_FAKE_RENAME_ELSEWHERE;
// -- WP WB8 --
use dspi_transport::fake::EXT_FAKE_CLIP;
// -- WP QD5 --
use dspi_transport::fake::EXT_FAKE_LATENCY_TIMING;

use crate::consts::CHANNEL_NAME_LEN;
use crate::records::*;
use crate::{Connection, DspiCore, DspiError, Result, require_feature};

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

#[cfg_attr(feature = "uniffi", uniffi::export)]
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
        self.ensure_probed(); // -- WP LK4 --
        // -- WP CG13 --
        // The cache is updated under the same lock, so a rename from elsewhere
        // cannot land between the write and the update. `send` refuses a feature
        // the probe did not find (`Unsupported`) and a source the device lacks,
        // then pads the name with NULs to the 32-byte field: "" sends a leading
        // NUL, which clears it.
        check_name(&name)?;
        let mut inner = self.lock();
        let change = inner.journal_begin("in.name", &[source])?; // -- WP P0W --
        let mut w = inner.writable_conn()?;
        w.session()?
            .send("in.name", &[source], Value::Text(name.clone()))?;
        note_source_name(w.conn(), source, name.clone());
        change.done(&mut inner, Value::Text(name));
        Ok(())
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
        self.ensure_probed(); // -- WP LK4 --
        // -- WP QD4 --
        // As `set_input_source_name`: the cache is updated under the same lock.
        check_name(&name)?;
        let mut inner = self.lock();
        let change = inner.journal_begin("dev.name", &[])?; // -- WP P0W --
        let mut w = inner.writable_conn()?;
        w.session()?
            .send("dev.name", &[], Value::Text(name.clone()))?;
        w.conn().device_name.clone_from(&name);
        change.done(&mut inner, Value::Text(name));
        Ok(())
    }

    // -- WP QD4 --
    /// The name of input source `source` as last known, with no traffic: read
    /// on connect, kept current by notifications (from any client) and by
    /// [`DspiCore::set_input_source_name`], and read again after a lost
    /// notification. "" when unnamed. For a client's rows, which would
    /// otherwise read over the wire what connect has just read.
    ///
    /// Errors: `Unsupported` on firmware without source names; `InvalidArgument`
    /// for a source past the table; `Disconnected` without a device or before
    /// it was probed.
    pub fn cached_input_source_name(&self, source: u8) -> Result<String> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        require_feature(conn.session()?, F_SOURCE_NAMES)?;
        conn.source_names
            .get(usize::from(source))
            .cloned()
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: format!("no input source {source}"),
            })
    }

    /// The device's name as last known, with no traffic, kept as
    /// [`DspiCore::cached_input_source_name`] keeps source names. "" when
    /// unnamed. Errors: `Unsupported` on firmware without device names;
    /// `Disconnected`.
    pub fn cached_device_name(&self) -> Result<String> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        require_feature(conn.session()?, F_DEVICE_NAME)?;
        Ok(conn.device_name.clone())
    }

    /// Have someone else rename something on the demo device, for clients'
    /// tests and demo mode: `target` is an input source, or 0xFF for the device
    /// itself. The rename arrives as a notification that is not this client's
    /// own. `Unsupported`, with no traffic, on anything but a simulated device.
    pub fn demo_rename_elsewhere(&self, target: u8, name: String) -> Result<()> {
        check_name(&name)?;
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        if w.conn().caps.kind != TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        let mut payload = vec![target];
        payload.extend_from_slice(name.as_bytes());
        // A leading NUL clears, as on the wire.
        payload.push(0);
        w.conn()
            .transport()
            .control_out(op::REQ_EXTENSION, EXT_FAKE_RENAME_ELSEWHERE, &payload)
            .map_err(DspiError::from)
    }

    // -- WP QD5 --
    /// Set how the demo device applies an output-latency request, for clients'
    /// tests: `mode` 0 at once (and a request held so far applies on the next
    /// status read), 1 after `reads` status reads, 2 never, 3 dropped after
    /// `reads` status reads. `blackout` status reads stall first while one is
    /// pending, as the firmware's flash window does. `Unsupported`, with no
    /// traffic, on anything but a simulated device; `InvalidArgument` for
    /// another `mode`.
    pub fn demo_latency_timing(&self, mode: u8, reads: u8, blackout: u8) -> Result<()> {
        if mode > 3 {
            return Err(DspiError::InvalidArgument {
                msg: format!("latency timing mode {mode}"),
            });
        }
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        if w.conn().caps.kind != TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        w.conn()
            .transport()
            .control_out(
                op::REQ_EXTENSION,
                EXT_FAKE_LATENCY_TIMING,
                &[mode, reads, blackout],
            )
            .map_err(DspiError::from)
    }

    // -- WP QD5 --
    /// How many output-latency requests the demo device has received, for
    /// clients' tests that a wait never sends one again. `Unsupported`, with no
    /// traffic, on anything but a simulated device.
    pub fn demo_latency_sets(&self) -> Result<u32> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        if conn.caps.kind != TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        let raw = conn
            .transport()
            .control_in(op::REQ_EXTENSION, EXT_FAKE_LATENCY_TIMING, 4)?;
        Ok(Cursor(&raw).u32().unwrap_or(0))
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
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        require_feature(conn.session()?, F_LATENCY)?;
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
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let conn = inner.conn()?;
        require_feature(conn.session()?, F_LATENCY)?;
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
        // -- WP CG21 -- Before the send: a send that fails may still have landed
        w.conn().ring_target.invalidate();
        w.session()?.send("out.latency", &[], Value::Choice(id))?;
        // -- WP P0W -- a flash write: history only
        crate::journal::journal(&mut inner, "out.latency", &[], Value::Choice(id));
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
}

impl DspiCore {
    fn read_name(&self, path: &str, indices: &[u8], feature: &str) -> Result<String> {
        self.ensure_probed(); // -- WP LK4 --
        let mut inner = self.lock();
        let session = inner.conn()?.session()?;
        require_feature(session, feature)?;
        match session.read(path, indices)? {
            Value::Text(t) => Ok(decode_name(t.as_bytes())),
            other => Err(DspiError::Transfer {
                msg: format!("{path}: expected a name, read {other:?}"),
            }),
        }
    }
}

// -- WP CG13 --
/// One name per input source, `0..=INPUT_SOURCE_MAX` (config.h).
pub(crate) type SourceNames = [String; 7];

/// Where a `NOTIFY_EVT_INPUT_SOURCE_NAME` payload's name field lies (see
/// `notify::EVT_INPUT_SOURCE_NAME`).
const SOURCE_NAME_EVENT_FIELD: std::ops::Range<usize> = 4..4 + CHANNEL_NAME_LEN;

/// Read every source name into the connection's cache. Firmware without
/// names gets no request at all (it may answer the opcode with a legacy gain).
/// A source whose read fails keeps its last name, "" after a connect, as
/// names are optional and the built-in title stands in.
// -- WP LK4 --
/// The reads of [`read_source_names`] and [`read_device_name`], for a caller
/// that batches them (`fetch_all`'s per-item reads): one exact 32-byte GET per
/// source, then the device name, each only when the probe found it.
pub(crate) fn name_reads(conn: &mut Connection) -> Vec<InRead> {
    let sources = conn.source_names.len();
    match conn.session() {
        Ok(session) => name_reads_for(session.capabilities(), sources),
        Err(_) => Vec::new(),
    }
}

/// [`name_reads`] for a device with `caps` and `sources` input sources.
pub(crate) fn name_reads_for(
    caps: &dspi_session::probe::Capabilities,
    sources: usize,
) -> Vec<InRead> {
    let has = |f: &str| caps.features.iter().any(|x| x.name == f && x.present);
    let mut reads = Vec::new();
    if has(F_SOURCE_NAMES) {
        reads.extend(
            (0..sources)
                .filter_map(|i| u16::try_from(i).ok())
                .map(|i| InRead::exact(op::REQ_GET_INPUT_SOURCE_NAME, i, NAME_LEN)),
        );
    }
    if has(F_DEVICE_NAME) {
        reads.push(InRead::exact(op::REQ_GET_DEVICE_NAME, 0, NAME_LEN));
    }
    reads
}

/// The answers to [`name_reads`]' `reads`, into the caches, as the registry
/// read (`Repr::Text`) and [`read_source_names`] / [`read_device_name`] take
/// them.
pub(crate) fn apply_names(conn: &mut Connection, reads: &[InRead], answers: Vec<Result<Vec<u8>>>) {
    for (r, answer) in reads.iter().zip(answers) {
        let text =
            match answer.map(|b| dspi_proto::value::Repr::Text(usize::from(NAME_LEN)).decode(&b)) {
                Ok(Some(Value::Text(t))) => t,
                Ok(other) => {
                    log::debug!("name 0x{:02X}/{}: read {other:?}", r.opcode, r.value);
                    continue;
                }
                Err(e) => {
                    log::debug!("name 0x{:02X}/{} not read: {e}", r.opcode, r.value);
                    continue;
                }
            };
        if r.opcode == op::REQ_GET_DEVICE_NAME {
            conn.device_name = decode_name(text.as_bytes());
        } else if let Ok(source) = u8::try_from(r.value) {
            note_source_name(conn, source, decode_name(text.as_bytes()));
        }
    }
}

/// `wLength` of a name read (`Text { max: 32 }` in the registry).
const NAME_LEN: u16 = 32;

pub(crate) fn read_source_names(conn: &mut Connection) {
    let supported = conn
        .session()
        .is_ok_and(|s| require_feature(s, F_SOURCE_NAMES).is_ok());
    if !supported {
        return;
    }
    for source in 0..conn.source_names.len() {
        let Ok(index) = u8::try_from(source) else {
            return;
        };
        let read = conn
            .session()
            .and_then(|s| Ok(s.read("in.name", &[index])?));
        match read {
            Ok(Value::Text(t)) => note_source_name(conn, index, decode_name(t.as_bytes())),
            Err(DspiError::Disconnected) => return,
            Ok(other) => log::debug!("source {source} name: read {other:?}"),
            Err(e) => log::debug!("source {source} name not read: {e}"),
        }
    }
}

/// A `NOTIFY_EVT_INPUT_SOURCE_NAME` payload into the cache: the device's word
/// on the name, from any client, our own echo included (the firmware may have
/// stored something other than what was sent). A short packet is ignored.
pub(crate) fn note_source_name_event(conn: &mut Connection, bytes: &[u8]) {
    if let (Some(&source), Some(field)) = (bytes.first(), bytes.get(SOURCE_NAME_EVENT_FIELD)) {
        note_source_name(conn, source, decode_name(field));
    }
}

// -- WP QD4 --
/// Read the device's name into the connection's cache, as
/// [`read_source_names`] reads the source names.
pub(crate) fn read_device_name(conn: &mut Connection) {
    let supported = conn
        .session()
        .is_ok_and(|s| require_feature(s, F_DEVICE_NAME).is_ok());
    if !supported {
        return;
    }
    match conn.session().and_then(|s| Ok(s.read("dev.name", &[])?)) {
        Ok(Value::Text(t)) => conn.device_name = decode_name(t.as_bytes()),
        Ok(other) => log::debug!("device name: read {other:?}"),
        Err(e) => log::debug!("device name not read: {e}"),
    }
}

/// A `NOTIFY_EVT_DEVICE_NAME` payload into the cache, as
/// [`note_source_name_event`]. A short packet is ignored.
pub(crate) fn note_device_name_event(conn: &mut Connection, bytes: &[u8]) {
    if let Some(field) = bytes.get(SOURCE_NAME_EVENT_FIELD) {
        conn.device_name = decode_name(field);
    }
}

/// Cache `name` for `source`; a source past the table is ignored.
fn note_source_name(conn: &mut Connection, source: u8, name: String) {
    if let Some(slot) = conn.source_names.get_mut(usize::from(source)) {
        *slot = name;
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
// -- WP CG13 --
#[cfg(test)]
mod tests_wp_cg13;

// -- WP QD4 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd4;

// -- WP QD5 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd5;

// -- WP WB8 -- not a binding export: the web's demo only (dspi-wasm `demo`)
impl DspiCore {
    /// Have the demo device clip on the channels in `mask` (one bit per wire
    /// channel), for clients' tests and demo mode: the clip shows on the next
    /// meter read and goes into the event log as a real one would.
    /// `Unsupported`, with no traffic, on anything but a simulated device;
    /// `InvalidArgument` for an empty mask.
    pub fn demo_clip(&self, mask: u32) -> Result<()> {
        if mask == 0 {
            return Err(DspiError::InvalidArgument {
                msg: "no channel to clip".into(),
            });
        }
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        if w.conn().caps.kind != TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        w.conn()
            .transport()
            .control_out(op::REQ_EXTENSION, EXT_FAKE_CLIP, &mask.to_le_bytes())
            .map_err(DspiError::from)
    }
}

// -- WP PGCORE --
// The connection's protocol generation for a client's device info (PLAN-PG
// sections 1.1 and 1.3): from the generation wrapper `paged::open` made,
// through every wrapper above it, with no traffic.
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The open device's protocol: its generation, the paged protocol's
    /// version and pages, how the extension channel is reached, and the
    /// board with its name. The board costs one platform read (the rest
    /// sends nothing); a board that cannot be read is an unknown board.
    /// Errors: [`DspiError::Disconnected`] with nothing open or when the
    /// device went.
    pub fn device_protocol(&self) -> Result<DeviceProtocol> {
        let mut inner = self.lock();
        let conn = inner.conn()?;
        let mut p = protocol_of(conn);
        // -- PG-C merge -- the board name from hardware.rs (PGZERO line 5)
        let board = match crate::hardware::read_board(conn) {
            Ok(board) => board,
            Err(DspiError::Disconnected) => return Err(DspiError::Disconnected),
            Err(e) => {
                log::debug!("the board could not be read: {e}");
                dspi_proto::packets::Board::Unknown(crate::UNKNOWN_PRODUCT_BOARD)
            }
        };
        p.board_id = board.id();
        p.board_name = crate::board_name(board).into();
        Ok(p)
    }
}

impl DspiCore {
    // -- WP PGCORE --
    /// The open connection's generation as its transport reports it
    /// (`Transport::generation`), `None` with nothing open or a path that
    /// does not say. For a binding's own transport over the core (the
    /// Terminal's); not exported.
    #[doc(hidden)]
    pub fn connection_protocol_generation(&self) -> Option<dspi_transport::Generation> {
        let mut inner = self.lock();
        inner.conn().ok()?.transport().generation()
    }

    // -- WP PGCORE --
    /// The open connection's paged discovery (`Transport::paged_discovery`),
    /// `None` on a legacy device or with nothing open. Not exported.
    #[doc(hidden)]
    pub fn connection_paged_discovery(&self) -> Option<dspi_transport::paged::Discovery> {
        let mut inner = self.lock();
        inner.conn().ok()?.transport().paged_discovery()
    }
}

// -- WP PGCORE --
/// [`DspiCore::device_protocol`] of `conn`.
pub(crate) fn protocol_of(conn: &mut Connection) -> DeviceProtocol {
    use dspi_transport::Generation;
    let has_ext = !conn.ext.supported.is_empty();
    let t = conn.transport();
    let (generation, protocol_major, protocol_minor) = match t.generation() {
        Some(Generation::Legacy) => (ProtocolGeneration::Legacy, 0, 0),
        Some(Generation::Paged(info)) => (ProtocolGeneration::Paged, info.major, info.minor),
        None => (ProtocolGeneration::Unknown, 0, 0),
    };
    let pages = t
        .paged_discovery()
        .map(|d| {
            d.pages
                .pages()
                .map(|p| DevicePage {
                    page: p.page,
                    revision: p.revision,
                })
                .collect()
        })
        .unwrap_or_default();
    let ext_envelope = match (has_ext, generation) {
        (false, _) => ExtEnvelope::NoChannel,
        (true, ProtocolGeneration::Paged) => ExtEnvelope::Tunnel,
        (true, _) => ExtEnvelope::Legacy,
    };
    DeviceProtocol {
        generation,
        protocol_major,
        protocol_minor,
        pages,
        ext_envelope,
        device_kind: conn.kind, // -- WP PGFIX --
        // -- PG-C merge -- filled by `device_protocol` (one platform read)
        ..DeviceProtocol::default()
    }
}
