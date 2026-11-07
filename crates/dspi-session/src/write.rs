//! The single write path.
//!
//! Every parameter change in the application goes through [`Session::write`],
//! because every subtlety of this protocol lives in one of its steps and none of
//! them are optional:
//!
//! - Some mutating opcodes are dispatched on the **IN** path and stall if issued
//!   as an OUT transfer.
//! - Deferred writes return "accepted" **before** validation runs, so the status
//!   byte is not evidence the value took effect. A bad DAC mute config is
//!   dropped silently.
//! - Flash writes disable the control endpoint for tens of milliseconds, so the
//!   confirming read has to survive a blackout.
//!
//! Spreading this across call sites would guarantee that some of them get it
//! wrong. There is one implementation, and the outcome type makes a silent
//! rejection impossible to ignore.

use dspi_proto::FilterType;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::SiggenStatus;
use dspi_proto::registry::{
    ALL_OUTPUTS, Hazard, Kind, ParamDesc, Requires, Target, WValue, by_path,
};
use dspi_proto::value::{EqParamPacket, Repr, Value, ValueError, decode_qp};
use dspi_proto::wire::BulkPacket;
use dspi_proto::{ChannelMap, Dir, Platform};
use dspi_transport::{Transport, TransportError, TransportKind, with_busy_retry};
use std::time::Duration;

use dspi_transport::clock::Instant;

use crate::probe::Capabilities;

/// How often the `out.latency` confirmation polls, and how long it keeps at it:
/// the Mac's 40 x 100 ms (`Commands.swift:757-762`), held to a clock so slow
/// reads cannot stretch it.
const LATENCY_POLL: Duration = Duration::from_millis(100);
const LATENCY_SETTLE: Duration = Duration::from_secs(4);
/// `OUTPUT_LATENCY_GET_STATUS`, the `REQ_OUTPUT_LATENCY` IN `wValue` for
/// `OutputLatencyStatus` (config.h:261, 1197-1206).
const LATENCY_STATUS: u16 = 0;
const LATENCY_STATUS_LEN: u16 = 12;
/// `OutputLatencyStatus.pending`: 1 until a SET has fully applied.
const LATENCY_PENDING_AT: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("no parameter called `{0}`")]
    UnknownParam(String),

    #[error("`{path}` is limited to {max} on this platform")]
    PlatformRange { path: String, max: String },

    #[error("`{path}` cannot be changed; it is read-only")]
    ReadOnly { path: String },

    // -- WP TM26 -- An action has a SET and no GET: reading it is the mistake.
    #[error("`{path}` cannot be read; it is an action (write-only), so `set` it instead")]
    WriteOnly { path: String },

    #[error("`{path}` needs {want} index(es), got {got}")]
    WrongArity {
        path: String,
        want: usize,
        got: usize,
    },

    #[error("{0} is not a valid {1} on this device")]
    BadTarget(u8, &'static str),

    #[error("`{path}` is not available: {why}")]
    Unavailable { path: String, why: String },

    /// Enabling this output would collide with the other side of Core 1.
    ///
    /// The firmware silently skips a blocked enable (survey-firmware 6.8), so
    /// this is refused before the wire with the reason the Console shows,
    /// rather than returning success for a write that did nothing.
    #[error("output {output} cannot be enabled yet: {body}")]
    Core1Conflict {
        output: u8,
        title: String,
        body: String,
        confirm: String,
    },

    #[error("{0}")]
    Value(#[from] ValueError),

    #[error(transparent)]
    Transport(#[from] TransportError),

    /// The write went out, and the device did not say it had finished applying
    /// it in the time allowed. It may still land, so this is neither success
    /// nor a refusal, and sending it again is the caller's decision.
    #[error(
        "`{path}` was sent, but {why} after {waited_ms} ms; it may still apply, so read it back before sending it again"
    )]
    Unsettled {
        path: String,
        waited_ms: u64,
        why: String,
    },
}

/// What actually happened, which is not always what was asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Written and confirmed by reading it back.
    Confirmed(Value),

    /// The device accepted the request but the readback shows a different value.
    ///
    /// This is the case the protocol makes easy to miss and expensive to get
    /// wrong: the write returned success, and the device did something else.
    Rejected { sent: Value, actual: Value },

    /// Written; the parameter has no readback, so this is the device's word for it.
    Accepted,

    /// An action with no value to confirm.
    Triggered,
}

impl Outcome {
    pub fn is_confirmed(&self) -> bool {
        matches!(
            self,
            Outcome::Confirmed(_) | Outcome::Accepted | Outcome::Triggered
        )
    }
}

/// One recorded change, for undo and for the command echo.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub path: &'static str,
    pub indices: Vec<u8>,
    pub before: Option<Value>,
    pub after: Value,
    pub outcome: Outcome,
    /// Whether this can be undone. Flash writes, pin moves and the bootloader
    /// jump cannot, so they are confirmed up front instead.
    pub undoable: bool,
    /// What the device also changed on its own because of this write, as it
    /// was before: see [`side_effects`]. Undo puts these back after `before`,
    /// in order; redo needs only `after`, since writing it again makes the
    /// device do the same thing again.
    pub companions: Vec<Companion>,
}

/// One value a write moved besides its own, as it was before the write.
#[derive(Debug, Clone, PartialEq)]
pub struct Companion {
    pub path: &'static str,
    pub indices: Vec<u8>,
    pub before: Value,
}

/// The tube's voicing: the type and the four character values its rows set.
/// Choosing a type loads its row into the four (tube.c:138-150); moving any
/// of the four sets the type back to Custom (tube.c:113-119, 154-165).
const TUBE_VOICING: [&str; 5] = [
    "tube.type",
    "tube.bias",
    "tube.asym",
    "tube.hardness",
    "tube.sag",
];

/// The settings a limiter link group shares (limiter.c:151-155).
/// Whether undo and redo may ever send this parameter again.
///
/// Only one with no hazard. An audible one is an action rather than a value to
/// put back: stepping back through `sig.control` would start a signal generator
/// the person had stopped, and `cs.macro.fire` would fire a macro again.
/// [`Session::undo`] steps over these and names them. The exception is what a
/// reversible write moved as a side effect (see [`side_effects`]): a limiter
/// link change puts `limit.on` back, which is state and starts nothing.
fn is_reversible(d: &ParamDesc) -> bool {
    matches!(d.hazard, Hazard::None)
}

const LIMITER_SHARED: [&str; 3] = ["limit.on", "limit.threshold", "limit.release"];

/// The values a write to `path` at `indices` can change besides its own, in
/// the order undo should put them back.
///
/// - A tube voicing write moves the rest of the voicing (see
///   [`TUBE_VOICING`]).
/// - A limiter link change on one output adopts the group's enable,
///   threshold and release (limiter.c:180-186).
/// - A limiter write to every output (`LIMITER_ALL_OUTPUTS`, limiter.h:21)
///   has no single value to read before it, so each output's own value is
///   kept; a link write to every output also re-gangs the groups
///   (limiter.c:177-179, 159-170), so the shared settings are kept too.
pub fn side_effects(path: &str, indices: &[u8], num_outputs: u8) -> Vec<(&'static str, Vec<u8>)> {
    if TUBE_VOICING.contains(&path) {
        return TUBE_VOICING
            .iter()
            .filter(|p| **p != path)
            .map(|p| (*p, Vec::new()))
            .collect();
    }
    let Some(d) = by_path(path) else {
        return Vec::new();
    };
    if d.target != Target::OutputOrAll {
        return Vec::new();
    }
    let outputs: Vec<u8> = match indices.first() {
        Some(&ALL_OUTPUTS) => (0..num_outputs).collect(),
        Some(&o) if path == "limit.link" => vec![o],
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    if indices.first() == Some(&ALL_OUTPUTS) {
        out.extend(outputs.iter().map(|&o| (d.path, vec![o])));
    }
    if path == "limit.link" {
        for p in LIMITER_SHARED {
            out.extend(outputs.iter().map(|&o| (p, vec![o])));
        }
    }
    out
}

pub struct Session {
    transport: Box<dyn Transport>,
    caps: Capabilities,
    map: ChannelMap,
    pub journal: Vec<JournalEntry>,
    /// When set, nothing is written; the intended transfer is recorded instead.
    pub dry_run: bool,
    /// Changes that can still be stepped back through, oldest first.
    ///
    /// Separate from the journal because the journal is a log of everything
    /// that happened, including the replays undo itself issues; undoing those
    /// again would just toggle a value back and forth.
    pub(crate) undo_stack: Vec<JournalEntry>,
    /// Changes stepped back through and not yet re-applied, oldest first.
    pub(crate) redo_stack: Vec<JournalEntry>,
    /// Set while undo or redo is replaying, so the replay neither becomes a new
    /// undo step nor discards the redo stack it is walking.
    pub(crate) replaying: bool,
    /// Set while [`Session::enable_output`] drives the Core 1 interlock itself,
    /// so the check inside [`Session::write`] does not run it twice.
    pub(crate) core1_checked: bool,
    /// The status byte the last write-as-read answered.
    ///
    /// Pin and clock setters report `PIN_CONFIG_*` here (config.h:607-613) and
    /// then quietly keep the old value, so the caller that wants to say *why*
    /// a step was refused needs the code, not just the readback.
    pub(crate) last_status: Option<u8>,
}

impl Session {
    pub fn new(transport: Box<dyn Transport>, caps: Capabilities) -> Option<Self> {
        let map = ChannelMap::new(caps.num_inputs, caps.num_outputs, caps.num_channels)?
            .with_names(caps.channels.iter().map(|c| c.name.clone()).collect());
        Some(Self {
            transport,
            caps,
            map,
            journal: Vec::new(),
            dry_run: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            replaying: false,
            core1_checked: false,
            last_status: None,
        })
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    /// The status byte the last write-as-read answered, if there was one.
    ///
    /// Every pin and clock setter is a write-as-read that returns a
    /// `PIN_CONFIG_*` code (config.h:607-613). A refusal leaves the old value
    /// in place, so the readback alone says only that nothing changed; this
    /// says why.
    pub fn last_write_status(&self) -> Option<u8> {
        self.last_status
    }

    /// Run something against the raw transport.
    ///
    /// Used by the subsystem helpers that speak their own packet formats rather
    /// than going through the registry, such as control surfaces.
    pub fn with_transport<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Transport) -> dspi_transport::Result<T>,
    ) -> dspi_transport::Result<T> {
        f(&mut *self.transport)
    }

    /// The transport, for callers that keep their own driver loop (the
    /// embedder API in `dspi-core`). Prefer [`Session::with_transport`].
    pub fn transport_mut(&mut self) -> &mut dyn Transport {
        &mut *self.transport
    }

    pub fn channel_map(&self) -> &ChannelMap {
        &self.map
    }

    /// Change one parameter, and confirm it actually changed.
    pub fn write(
        &mut self,
        path: &str,
        indices: &[u8],
        value: Value,
    ) -> Result<Outcome, WriteError> {
        let (d, set) = writable_param(path)?;

        // Belongs to this write alone; a stale code from the previous one would
        // be read as this one's refusal.
        self.last_status = None;

        let value = self.validate_write(d, indices, &value)?;

        // PDM and the Core 1 EQ workers cannot both run, and the firmware skips
        // a blocked enable without saying so (survey-firmware 6.8). Ask first,
        // so a plain `out.enable` write fails loudly with the reason instead of
        // reporting success for nothing.
        if d.path == "out.enable"
            && !self.core1_checked
            && value.as_bool() == Some(true)
            && let Some(c) = self.core1_conflict_alert(indices[0])
        {
            return Err(WriteError::Core1Conflict {
                output: indices[0],
                title: c.title,
                body: c.body,
                confirm: c.confirm,
            });
        }

        // A write that cannot be reversed has no use for what it replaces, and
        // reading it first would keep a Stop waiting on a busy device.
        let reversible = is_reversible(d);
        let before = if reversible {
            self.read(path, indices).ok()
        } else {
            None
        };
        // A replay is not an undo step, so what it moves need not be kept.
        let companions = if self.replaying || !reversible {
            Vec::new()
        } else {
            self.read_companions(path, indices)
        };

        if self.dry_run {
            let outcome = Outcome::Accepted;
            self.record(d, indices, before, companions, value, outcome.clone());
            return Ok(outcome);
        }

        // An EQ band is written as one whole packet, never field by field: the
        // firmware's SET_EQ_PARAM takes a 16-byte descriptor and carries nothing
        // in wValue. Changing one field therefore means reading the band,
        // editing it, and writing it back.
        if matches!(d.wvalue, WValue::EqScalar(_)) {
            let outcome = self.write_eq_field(d, indices[0], indices[1], &value)?;
            self.record(d, indices, before, companions, value, outcome.clone());
            return Ok(outcome);
        }

        let wvalue = self.build_wvalue(d, indices, &value)?;
        let repr = d.repr();

        match d.dir {
            Dir::Out => {
                let payload = repr.encode(&value)?;
                self.transport
                    .control_out(set, wvalue, &payload)
                    .map_err(|e| self.explain_aux_stall(d, indices, e))?;
            }
            // Mutating commands on the IN path. Not a mistake: they carry their
            // parameters in wValue and answer with a status byte.
            Dir::WriteAsRead | Dir::In => {
                let reply = self.transport.control_in(set, wvalue, 1)?;
                self.last_status = reply.first().copied();
            }
        }

        let outcome = self.confirm(d, indices, &value)?;
        self.record(d, indices, before, companions, value, outcome.clone());
        Ok(outcome)
    }

    /// Check a [`write`](Self::write) without making it: the parameter
    /// exists and is writable, is available on this device, takes these
    /// indices, and accepts this value (range, type and platform limits).
    /// Nothing goes on the wire. A write that passes can still be refused by
    /// the device, or fail on the transport.
    pub fn check(&self, path: &str, indices: &[u8], value: &Value) -> Result<(), WriteError> {
        let (d, _) = writable_param(path)?;
        self.validate_write(d, indices, value).map(|_| ())
    }

    /// The checks a write makes before the wire, shared by [`write`](Self::write)
    /// and [`check`](Self::check). Returns the value as the registry
    /// normalises it.
    fn validate_write(
        &self,
        d: &ParamDesc,
        indices: &[u8],
        value: &Value,
    ) -> Result<Value, WriteError> {
        self.check_available(d)?;
        self.check_indices(d, indices)?;

        // Validate before the wire, so the user gets a message naming the limits
        // rather than a bare stall from the device.
        let value = d.kind.validate(value)?;
        // The delay buffers are 1024 samples on an RP2040 and 2048 on an
        // RP2350 (config.h:94-99): 42 ms against 85 ms at 48 kHz. The registry
        // carries the larger; the smaller is a platform fact, checked here.
        self.check_platform_range(d, &value)?;
        Ok(value)
    }

    /// Poll every meter in one transfer.
    pub fn meters(&mut self) -> Result<crate::Meters, WriteError> {
        Ok(crate::read_meters(
            &mut *self.transport,
            self.caps.num_channels,
        )?)
    }

    /// Read the whole device state in one chunked transfer.
    ///
    /// Everything the bulk packet covers can be decoded from here instead of
    /// asked for one scalar at a time. Prefer this whenever more than a couple
    /// of values are wanted: it is six transfers for all 6136 bytes, against
    /// five transfers per EQ band alone.
    pub fn snapshot(&mut self) -> Result<BulkPacket, WriteError> {
        Ok(crate::probe::read_bulk_from(
            &mut *self.transport,
            Some(&self.caps.firmware_version),
        )?)
    }

    /// Read a whole EQ band.
    ///
    /// Five transfers, plus a sixth for the Linkwitz Transform's `Qp`. For more
    /// than a band or two, [`Session::snapshot`] carries the same table for the
    /// cost of one read.
    pub fn read_band(&mut self, channel: u8, band: u8) -> Result<EqParamPacket, WriteError> {
        let scalar = |s: &mut Self, param: u8, len: u16| -> Result<Vec<u8>, WriteError> {
            let wvalue = ((channel as u16) << 8) | ((band as u16) << 3) | param as u16;
            let kind = s.transport.caps().kind;
            Ok(with_busy_retry(
                kind,
                || s.transport.control_in(op::REQ_GET_EQ_PARAM, wvalue, len),
                op::REQ_GET_EQ_PARAM,
            )?)
        };
        let f32_at = |s: &mut Self, param: u8| -> Result<f32, WriteError> {
            let b = scalar(s, param, 4)?;
            Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };

        let filter_type = FilterType::from_raw(scalar(self, 0, 4)?[0]);
        let freq = f32_at(self, 1)?;
        let q = f32_at(self, 2)?;
        let gain_db = f32_at(self, 3)?;
        let bypass = scalar(self, 4, 4)?[0] != 0;

        // Only fetch the sidecar when it means something; on other types the
        // firmware stores zero there and reading it would just cost a transfer.
        let qp = if filter_type.is_linkwitz() {
            let raw = scalar(self, 5, 4)?;
            Some(decode_qp(u16::from_le_bytes([raw[0], raw[1]])))
        } else {
            None
        };

        Ok(EqParamPacket {
            channel,
            band,
            filter_type,
            bypass,
            freq,
            q,
            gain_db,
            qp,
        })
    }

    /// Write a whole EQ band in one transfer.
    ///
    /// Prefer this over setting fields one at a time: it is one transfer rather
    /// than seven, and it cannot leave the band in a half-updated state if the
    /// device disappears midway.
    pub fn write_band(&mut self, packet: &EqParamPacket) -> Result<Outcome, WriteError> {
        if self.dry_run {
            return Ok(Outcome::Accepted);
        }
        self.transport
            .control_out(op::REQ_SET_EQ_PARAM, 0, &packet.encode())?;

        let actual = self.read_band(packet.channel, packet.band)?;
        Ok(if bands_agree(packet, &actual) {
            Outcome::Confirmed(Value::Bytes(packet.encode()))
        } else {
            Outcome::Rejected {
                sent: Value::Bytes(packet.encode()),
                actual: Value::Bytes(actual.encode()),
            }
        })
    }

    /// Change one field of a band, preserving the rest.
    fn write_eq_field(
        &mut self,
        d: &ParamDesc,
        channel: u8,
        band: u8,
        value: &Value,
    ) -> Result<Outcome, WriteError> {
        let mut packet = self.read_band(channel, band)?;

        crate::fast::apply_eq_field(&mut packet, d, value)?;

        self.write_band(&packet)
    }

    /// Read one parameter back.
    pub fn read(&mut self, path: &str, indices: &[u8]) -> Result<Value, WriteError> {
        let d = by_path(path).ok_or_else(|| WriteError::UnknownParam(path.into()))?;
        let get = d
            .get
            .ok_or_else(|| WriteError::WriteOnly { path: path.into() })?;

        self.check_indices(d, indices)?;
        // "Every output" exists only for a write; the device stalls a read of
        // it (limiter.h:21).
        if d.target == Target::OutputOrAll && indices.first() == Some(&ALL_OUTPUTS) {
            return Err(WriteError::BadTarget(ALL_OUTPUTS, "single output"));
        }
        if d.path == SIG_CONTROL {
            return self.read_siggen_state().map(siggen_control_of);
        }
        let wvalue = self.build_read_wvalue(d, indices);
        let repr = d.repr();
        // EQ scalars always answer four bytes regardless of the field's
        // width; a crosspoint answers its whole 8-byte MatrixRoutePacket
        // (config.h:845-851).
        let len = if matches!(d.wvalue, WValue::EqScalar(_)) {
            4
        } else if matches!(d.wvalue, WValue::Crosspoint) {
            8
        } else {
            repr.len().max(1)
        };

        let kind = self.transport.caps().kind;
        let bytes = with_busy_retry(
            kind,
            || self.transport.control_in(get, wvalue, len as u16),
            get,
        )
        .map_err(|e| self.explain_aux_stall(d, indices, e))?;

        // A crosspoint reads back in the words the grammar writes it:
        // `on -6.0 dB inv`, `off`.
        if matches!(d.wvalue, WValue::Crosspoint) && bytes.len() >= 8 {
            let enabled = bytes[2] != 0;
            let invert = bytes[3] != 0;
            let gain = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let mut text = if enabled {
                format!("on {gain:+.1} dB")
            } else {
                "off".to_string()
            };
            if invert {
                text.push_str(" inv");
            }
            return Ok(Value::Text(text));
        }

        Ok(decode_read(d, &bytes))
    }

    // -- WP TM26 --
    /// The firmware stalls `cs.aux` and `cs.aux.level` on a slot that holds no
    /// auxiliary output of the right kind (control_surfaces.h:284-285), which
    /// reads as a fault. Ask the slot's binding what it holds, and say so when
    /// that explains the stall. Any other stall, or a binding that cannot be
    /// read, is passed on as it was.
    fn explain_aux_stall(
        &mut self,
        d: &ParamDesc,
        indices: &[u8],
        e: TransportError,
    ) -> WriteError {
        use crate::cs_model::ty;
        use dspi_proto::packets::CsBinding;
        let (TransportError::Stalled { .. }, Some(&slot)) = (&e, indices.first()) else {
            return e.into();
        };
        let level = match d.path {
            "cs.aux" => false,
            "cs.aux.level" => true,
            _ => return e.into(),
        };
        let binding = self
            .transport
            .control_in(
                op::REQ_GET_CS_BINDING,
                u16::from(slot),
                CsBinding::SIZE as u16,
            )
            .ok()
            .and_then(|b| CsBinding::decode(&b).ok());
        let Some(b) = binding else {
            return e.into();
        };
        let why = match b.component {
            ty::AUX_PWM => return e.into(),
            ty::AUX_OUT if !level => return e.into(),
            ty::AUX_OUT => {
                format!("slot {slot} holds an on/off auxiliary output, which has no level")
            }
            ty::NONE => format!("slot {slot} is empty; it holds no auxiliary output"),
            _ => format!("slot {slot} holds no auxiliary output"),
        };
        WriteError::Unavailable {
            path: d.path.into(),
            why,
        }
    }

    /// Confirm a write landed, for the parameters where the status byte is not
    /// proof. Everything else is taken at the device's word.
    fn confirm(
        &mut self,
        d: &ParamDesc,
        indices: &[u8],
        sent: &Value,
    ) -> Result<Outcome, WriteError> {
        if matches!(d.kind, Kind::Trigger) {
            return Ok(Outcome::Triggered);
        }
        if d.path == SIG_CONTROL {
            return Ok(self.confirm_siggen_control(sent));
        }
        // A packet has no scalar readback: `read` fetches one byte of it,
        // which is not evidence either way. The typed helpers (surfaces.rs)
        // verify packets by their own status protocol.
        if matches!(d.kind, Kind::Packet) {
            return Ok(Outcome::Accepted);
        }
        if !d.needs_readback() {
            return Ok(match d.get {
                // Cheap to verify, so verify.
                Some(_) => match self.read(d.path, indices) {
                    Ok(actual) if values_agree(sent, &actual) => Outcome::Confirmed(actual),
                    Ok(actual) => Outcome::Rejected {
                        sent: sent.clone(),
                        actual,
                    },
                    Err(_) => Outcome::Accepted,
                },
                None => Outcome::Accepted,
            });
        }

        if d.path == "out.latency" {
            return self.confirm_output_latency(d, sent);
        }

        // A deferred write may still be applying, and the control endpoint may
        // be dead mid-flash. The retry inside read() covers the blackout.
        match self.read(d.path, indices) {
            Ok(actual) if values_agree(sent, &actual) => Ok(Outcome::Confirmed(actual)),
            Ok(actual) => Ok(Outcome::Rejected {
                sent: sent.clone(),
                actual,
            }),
            Err(e) => Err(e),
        }
    }

    /// The generator's state, from `SiggenStatus` (siggen.h:110-120).
    fn read_siggen_state(&mut self) -> Result<u8, WriteError> {
        let kind = self.transport.caps().kind;
        let get = op::REQ_SIGGEN_GET_STATUS;
        let wanted = SiggenStatus::SIZE;
        let bytes = with_busy_retry(
            kind,
            || self.transport.control_in(get, 0, wanted as u16),
            get,
        )?;
        let status = SiggenStatus::decode(&bytes).map_err(|_| TransportError::ShortRead {
            opcode: get,
            got: bytes.len(),
            wanted,
        })?;
        Ok(status.state)
    }

    /// Check a `sig.control` against the state it leaves (siggen.c:1240-1276).
    /// The action has been sent by now, so a status that cannot be read is no
    /// reason to report anything but that.
    fn confirm_siggen_control(&mut self, sent: &Value) -> Outcome {
        match self.read_siggen_state() {
            Ok(state) if siggen_control_took(sent, state) => Outcome::Confirmed(sent.clone()),
            Ok(state) => Outcome::Rejected {
                sent: sent.clone(),
                actual: siggen_control_of(state),
            },
            Err(_) => Outcome::Accepted,
        }
    }

    /// Wait out the deferred `out.latency` apply, then compare.
    ///
    /// The firmware only notes the request; its main loop does the flash write
    /// and the output restart (vendor_commands.c:1474-1488, main.c:2298-2309).
    /// Until then the status reports the old id with `pending` set, so an
    /// immediate read-back would call a good write rejected. The status is
    /// polled as `Commands.swift:757-762` does, from 100 ms after the write.
    ///
    /// Blocks the calling thread for one interval when the device applies
    /// promptly, and at most [`LATENCY_SETTLE`] plus one transfer's timeout
    /// when it does not. The poll does not use the busy retry, which would
    /// stretch that bound by seconds per read.
    ///
    /// The write is never sent again from here: a failure either ends the wait
    /// or returns.
    fn confirm_output_latency(
        &mut self,
        d: &ParamDesc,
        sent: &Value,
    ) -> Result<Outcome, WriteError> {
        let get = d.get.ok_or_else(|| WriteError::ReadOnly {
            path: d.path.into(),
        })?;
        let stall_is_busy = matches!(
            self.transport.caps().kind,
            TransportKind::Usb | TransportKind::Fake
        );
        let start = Instant::now();
        loop {
            dspi_transport::clock::sleep(LATENCY_POLL);
            let why = match self
                .transport
                .control_in(get, LATENCY_STATUS, LATENCY_STATUS_LEN)
            {
                Ok(status) => match status.get(LATENCY_PENDING_AT) {
                    // The requested id is the status's first byte, so this one
                    // read answers both questions.
                    Some(0) => {
                        let actual = decode_read(d, &status);
                        return Ok(if values_agree(sent, &actual) {
                            Outcome::Confirmed(actual)
                        } else {
                            Outcome::Rejected {
                                sent: sent.clone(),
                                actual,
                            }
                        });
                    }
                    Some(_) => "the device still reports it pending".to_string(),
                    None => "the device's status reply was too short".to_string(),
                },
                // The flash write takes the control endpoint away for a moment.
                Err(e) if latency_poll_can_continue(&e, stall_is_busy) => {
                    format!("the device last answered: {e}")
                }
                Err(e) => return Err(e.into()),
            };
            if start.elapsed() >= LATENCY_SETTLE {
                return Err(WriteError::Unsettled {
                    path: d.path.into(),
                    waited_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    why,
                });
            }
        }
    }

    fn record(
        &mut self,
        d: &ParamDesc,
        indices: &[u8],
        before: Option<Value>,
        companions: Vec<Companion>,
        after: Value,
        outcome: Outcome,
    ) {
        let entry = JournalEntry {
            path: d.path,
            indices: indices.to_vec(),
            before,
            after,
            outcome,
            undoable: is_reversible(d),
            companions,
        };
        self.journal.push(entry.clone());

        // A replay is already accounted for by the stack it came from. A fresh
        // write is a new branch of history, so anything that had been undone is
        // no longer reachable.
        if !self.replaying {
            self.redo_stack.clear();
            self.undo_stack.push(entry);
        }
    }

    /// Read the values a write is about to move besides its own (see
    /// [`side_effects`]). One that cannot be read is left out: undo then
    /// restores what it can.
    fn read_companions(&mut self, path: &str, indices: &[u8]) -> Vec<Companion> {
        side_effects(path, indices, self.caps.num_outputs)
            .into_iter()
            .filter_map(|(path, indices)| {
                let before = self.read(path, &indices).ok()?;
                Some(Companion {
                    path,
                    indices,
                    before,
                })
            })
            .collect()
    }

    /// Refuse a parameter this device does not have, with the reason.
    pub(crate) fn check_available(&self, d: &ParamDesc) -> Result<(), WriteError> {
        let why = match d.requires {
            Requires::Always => return Ok(()),
            Requires::Rp2350 => {
                if self.caps.platform == Platform::Rp2350 {
                    return Ok(());
                }
                format!("it needs an RP2350; this is {}", self.caps.platform.name())
            }
            Requires::Feature(name) => {
                let present = self
                    .caps
                    .features
                    .iter()
                    .any(|f| f.name == name && f.present);
                if present {
                    return Ok(());
                }
                format!("this firmware does not provide `{name}`")
            }
        };
        Err(WriteError::Unavailable {
            path: d.path.into(),
            why,
        })
    }

    /// Range-check indices against the device's actual topology, never a constant.
    pub(crate) fn check_indices(&self, d: &ParamDesc, indices: &[u8]) -> Result<(), WriteError> {
        let want = d.target.arity();
        if indices.len() != want {
            return Err(WriteError::WrongArity {
                path: d.path.into(),
                want,
                got: indices.len(),
            });
        }

        let ok = match d.target {
            Target::None => true,
            Target::Channel => indices[0] < self.map.num_channels(),
            Target::Input => indices[0] < self.map.num_inputs(),
            Target::Output => indices[0] < self.map.num_outputs(),
            // The limiter numbers outputs from 0 like every other output
            // command, and 0xFF writes them all (limiter.h:11-21).
            Target::OutputOrAll => indices[0] < self.map.num_outputs() || indices[0] == ALL_OUTPUTS,
            Target::ChannelBand => {
                indices[0] < self.map.num_channels()
                    && is_valid_band(indices[1], self.caps.max_bands)
            }
            Target::Crosspoint => {
                indices[0] < self.map.num_inputs() && indices[1] < self.map.num_outputs()
            }
            Target::PresetSlot => indices[0] < 10,
            Target::CsSlot => indices[0] < self.caps.cs.as_ref().map_or(0, |c| c.max_bindings),
            // `CS_MAX_IR_COMMANDS` doubled from 8 to 16 at caps v6, so this must
            // come from the caps header rather than a constant.
            Target::CsIrSlot => indices[0] < self.caps.cs.as_ref().map_or(0, |c| c.max_ir_commands),
            Target::CsGroup => indices[0] < self.caps.cs.as_ref().map_or(0, |c| c.max_groups),
            Target::CsMacro => indices[0] < self.caps.cs.as_ref().map_or(0, |c| c.max_macros),
            Target::CsMacroStep => {
                let cs = self.caps.cs.as_ref();
                indices[0] < cs.map_or(0, |c| c.max_macros)
                    && indices[1] < cs.map_or(0, |c| c.max_macro_steps)
            }
            // `CS_MAX_DISPLAY_PAGES` is 16 and is not reported by the caps
            // header; `REQ_GET_CS_DISPLAY_CFG` answers it as `max_pages`, which
            // no probe reads yet, so the firmware's own range check applies.
            Target::CsDisplayPage => true,
            // config.h:448-453: four S/PDIF inputs, indexed 0..3.
            Target::SpdifInput => indices[0] < 4,
            // config.h:454: only the optional inputs can be switched.
            Target::SpdifExtraInput => (1..4).contains(&indices[0]),
            // audio_input.h: INPUT_SOURCE_MAX is 6 (S/PDIF 4).
            Target::InputSource => indices[0] <= 6,
            // Bounded by the device but not separately reported; the firmware
            // range-checks these and answers with a status code.
            _ => true,
        };

        if ok {
            Ok(())
        } else {
            Err(WriteError::BadTarget(indices[0], target_name(d.target)))
        }
    }

    pub(crate) fn build_wvalue(
        &self,
        d: &ParamDesc,
        indices: &[u8],
        value: &Value,
    ) -> Result<u16, WriteError> {
        Ok(wvalue_for(d, indices, value))
    }

    /// Reads address the same parameter but never carry a value in `wValue`.
    fn build_read_wvalue(&self, d: &ParamDesc, indices: &[u8]) -> u16 {
        read_wvalue_for(d, indices)
    }
}

/// Pack the target and value into `wValue` for a write.
///
/// Free rather than a method so the packing rules can be tested directly; they
/// are the fiddliest part of the protocol and the easiest to get subtly wrong.
pub(crate) fn wvalue_for(d: &ParamDesc, indices: &[u8], value: &Value) -> u16 {
    {
        match d.wvalue {
            WValue::Zero => 0,
            WValue::Fixed(v) => v,
            // `(output << 8) | index` (limiter.h:11, config.h:233-236).
            WValue::OutputIndex(index) => {
                ((indices.first().copied().unwrap_or(0) as u16) << 8) | index as u16
            }
            WValue::Target => indices.first().copied().unwrap_or(0) as u16,
            WValue::ChannelBand => ((indices[0] as u16) << 8) | indices[1] as u16,
            WValue::Crosspoint => ((indices[0] as u16) << 8) | indices[1] as u16,
            // `(step << 8) | macro`: the only command that puts the second
            // index in the high byte (config.h:141).
            WValue::MacroStep => ((indices[1] as u16) << 8) | indices[0] as u16,
            // The band field is five bits wide, so crossover bands 20-23 fit.
            WValue::EqScalar(param) => {
                ((indices[0] as u16) << 8) | ((indices[1] as u16) << 3) | param as u16
            }
            WValue::ValueSlot => {
                let v = value.as_u8().unwrap_or(0) as u16;
                (v << 8) | indices.first().copied().unwrap_or(0) as u16
            }
            WValue::ValueIndex => {
                let v = value.as_f32().unwrap_or(0.0) as u16;
                (v << 8) | indices.first().copied().unwrap_or(0) as u16
            }
            WValue::ValueOnly => value.as_f32().unwrap_or(0.0) as u16,
            // `(role << 8) | value`: the role picks which of the opcode's two
            // parameters this write is for (config.h:334).
            WValue::RoleValue(role) => {
                ((role as u16) << 8) | (value.as_f32().unwrap_or(0.0) as u16 & 0xFF)
            }
        }
    }
}

pub(crate) fn read_wvalue_for(d: &ParamDesc, indices: &[u8]) -> u16 {
    {
        match d.wvalue {
            WValue::Fixed(v) => v,
            WValue::OutputIndex(index) => {
                ((indices.first().copied().unwrap_or(0) as u16) << 8) | index as u16
            }
            WValue::ChannelBand | WValue::Crosspoint => {
                ((indices[0] as u16) << 8) | indices[1] as u16
            }
            WValue::EqScalar(param) => {
                ((indices[0] as u16) << 8) | ((indices[1] as u16) << 3) | param as u16
            }
            // A macro step reads back inside the whole macro, addressed by the
            // macro index alone; the step number has no place in that wValue.
            WValue::Target | WValue::ValueSlot | WValue::ValueIndex | WValue::MacroStep => {
                indices.first().copied().unwrap_or(0) as u16
            }
            // The read takes the bare role, not the packed pair
            // (`fetchI2SBckPin(role:)`, config.h:336).
            WValue::RoleValue(role) => role as u16,
            _ => 0,
        }
    }
}

fn is_valid_band(band: u8, max_peq: u8) -> bool {
    // PEQ bands sit at 0..max, crossover bands at 20..23. The gap between is
    // deliberately rejected by the firmware.
    band < max_peq || (20..24).contains(&band)
}

fn target_name(t: Target) -> &'static str {
    match t {
        Target::Channel => "channel",
        Target::Input => "input",
        Target::Output | Target::OutputOrAll => "output",
        Target::TapChannel => "analyser channel",
        Target::ChannelBand => "channel or band",
        Target::Crosspoint => "crosspoint",
        Target::PresetSlot => "preset slot",
        Target::CsSlot => "binding slot",
        Target::CsIrSlot => "IR command slot",
        Target::CsGroup => "channel group",
        Target::CsMacro => "macro",
        Target::CsMacroStep => "macro or step",
        Target::CsDisplayPage => "display page",
        Target::SpdifInput => "S/PDIF input",
        Target::SpdifExtraInput => "optional S/PDIF input",
        Target::InputSource => "input source",
        _ => "index",
    }
}

/// Interpret a read according to the parameter's kind.
/// Whether a failed status read during [`Session::write`]'s `out.latency` wait
/// is the flash window rather than an answer. A device that has gone, or a
/// link that refuses, will not come back within the wait, so those fail now.
/// A stall counts only where a stall can mean busy (see
/// [`dspi_transport::with_busy_retry`]).
fn latency_poll_can_continue(e: &TransportError, stall_is_busy: bool) -> bool {
    match e {
        TransportError::Stalled { .. } => stall_is_busy,
        TransportError::TimedOut { .. }
        | TransportError::ShortRead { .. }
        | TransportError::Usb(_) => true,
        _ => false,
    }
}

/// Answers from `SiggenStatus` rather than from a scalar: see
/// [`Session::read_siggen_state`].
const SIG_CONTROL: &str = "sig.control";
/// `SIGGEN_STATE_*` (siggen.h:102-106).
const SIGGEN_STATE_IDLE: u8 = 0;
const SIGGEN_STATE_FADE_OUT: u8 = 4;
/// `sig.control`'s choices, which are `SIGGEN_CTL_*`.
const SIGGEN_CTL_STOP: u8 = 0;
const SIGGEN_CTL_START: u8 = 1;
const SIGGEN_CTL_STOP_NOW: u8 = 2;

/// The state as the `sig.control` choice it amounts to: fading out is stopping.
fn siggen_control_of(state: u8) -> Value {
    Value::Choice(match state {
        SIGGEN_STATE_IDLE | SIGGEN_STATE_FADE_OUT => SIGGEN_CTL_STOP,
        _ => SIGGEN_CTL_START,
    })
}

/// Whether a state is what this action leaves. A plain Stop fades unless the
/// generator is in its silent gap. A Start on a running generator fades that
/// signal out first and relaunches, so any state but idle is a Start that took.
fn siggen_control_took(sent: &Value, state: u8) -> bool {
    match sent.as_u8() {
        Some(SIGGEN_CTL_START) => state != SIGGEN_STATE_IDLE,
        Some(SIGGEN_CTL_STOP_NOW) => state == SIGGEN_STATE_IDLE,
        _ => matches!(state, SIGGEN_STATE_IDLE | SIGGEN_STATE_FADE_OUT),
    }
}

fn decode_read(d: &ParamDesc, bytes: &[u8]) -> Value {
    let repr = d.repr();

    // EQ scalars always answer four bytes: an f32 for freq/Q/gain, and a small
    // integer in the low byte for type and bypass.
    if let WValue::EqScalar(param) = d.wvalue {
        return match param {
            1..=3 => Repr::F32.decode(bytes).unwrap_or(Value::Float(0.0)),
            _ => Value::Int(bytes.first().copied().unwrap_or(0) as i64),
        };
    }

    // The indexed opcodes answer a float whatever the parameter is; the
    // firmware rounds enums and masks to the nearest integer on the way in
    // (tube.c:122-212), so round them back the same way here.
    if repr == Repr::F32 && !matches!(d.kind, Kind::Float { .. }) {
        let f = Repr::F32
            .decode(bytes)
            .and_then(|v| v.as_f32())
            .unwrap_or(0.0);
        return match d.kind {
            Kind::Bool => Value::Bool(f != 0.0),
            Kind::Choice(_) => Value::Choice(f.round().clamp(0.0, 255.0) as u8),
            Kind::Mask => Value::Mask(f.round().max(0.0) as u32),
            _ => Value::Int(f.round() as i64),
        };
    }

    match d.kind {
        Kind::Bool => Value::Bool(bytes.first().copied().unwrap_or(0) != 0),
        Kind::Choice(_) => Value::Choice(bytes.first().copied().unwrap_or(0)),
        Kind::Mask => Value::Mask(
            bytes
                .first()
                .copied()
                .map(|lo| lo as u32 | ((bytes.get(1).copied().unwrap_or(0) as u32) << 8))
                .unwrap_or(0),
        ),
        _ => repr.decode(bytes).unwrap_or(Value::Bytes(bytes.to_vec())),
    }
}

/// Compare a whole band, field by field, with the same float tolerance scalars
/// use. Comparing encoded bytes would report spurious rejections, because the
/// device re-quantises what it stores.
/// A parameter by path, with its SET opcode; refused when unknown or
/// read-only.
fn writable_param(path: &str) -> Result<(&'static ParamDesc, u8), WriteError> {
    let d = by_path(path).ok_or_else(|| WriteError::UnknownParam(path.into()))?;
    let set = d
        .set
        .ok_or_else(|| WriteError::ReadOnly { path: path.into() })?;
    Ok((d, set))
}

fn bands_agree(sent: &EqParamPacket, actual: &EqParamPacket) -> bool {
    let close = |a: f32, b: f32| (a - b).abs() <= a.abs().max(1.0) * 1e-3;
    sent.filter_type == actual.filter_type
        && sent.bypass == actual.bypass
        && close(sent.freq, actual.freq)
        && close(sent.q, actual.q)
        && close(sent.gain_db, actual.gain_db)
}

/// Compare what we sent against what came back.
///
/// Floats need a tolerance: the device stores single precision and several
/// parameters are clamped or quantised on the way in, so exact equality would
/// report spurious rejections on values that were in fact accepted.
fn values_agree(sent: &Value, actual: &Value) -> bool {
    match (sent, actual) {
        (Value::Float(a), Value::Float(b)) => (a - b).abs() <= a.abs().max(1.0) * 1e-3,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Bool(a), Value::Int(b)) => *a == (*b != 0),
        (Value::Choice(a), Value::Int(b)) => *a as i64 == *b,
        (Value::Choice(a), Value::Choice(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Int(b)) => (*a as i64) == *b,
        (Value::Mask(a), Value::Mask(b)) => a == b,
        (Value::Mask(a), Value::Int(b)) => *a as i64 == *b,
        (Value::Text(a), Value::Text(b)) => a == b,
        _ => sent == actual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{Capabilities, ChannelInfo, ControlSurfaceCaps, Feature};
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, Reply};

    pub(super) fn caps(platform: Platform, features: &[(&str, bool)]) -> Capabilities {
        Capabilities {
            serial: "TEST".into(),
            platform,
            firmware: "1.1.5".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 5, 0),
            build_info: None,
            wire_format: 26,
            num_channels: 17,
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
            band_storage: 12,
            channels: (0..17)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= 8,
                })
                .collect(),
            features: features
                .iter()
                .map(|(n, p)| Feature {
                    name: (*n).into(),
                    present: *p,
                    evidence: "test".into(),
                })
                .collect(),
            cs: Some(ControlSurfaceCaps {
                caps_version: 13,
                max_bindings: 16,
                type_count: 9,
                noun_count: 57,
                max_ir_commands: 16,
                max_groups: 8,
                max_macros: 8,
                max_macro_steps: 8,
                max_pages: 16,
                display_models: 8,
                types: Vec::new(),
            }),
            siggen: None,
            active_preset: Some(0),
        }
    }

    /// Fixed-width NUL-padded text, as the device actually returns it.
    pub(super) fn padded(s: &str) -> Vec<u8> {
        let mut v = vec![0u8; 32];
        v[..s.len()].copy_from_slice(s.as_bytes());
        v
    }

    fn session(t: MockTransport, features: &[(&str, bool)]) -> Session {
        Session::new(Box::new(t), caps(Platform::Rp2350, features)).unwrap()
    }

    #[test]
    fn writes_a_float_and_confirms_it() {
        let t =
            MockTransport::new().data(op::REQ_GET_USER_VOLUME, (-18.0f32).to_le_bytes().to_vec());
        let mut s = session(t, &[]);

        let out = s.write("vol.user", &[], Value::Float(-18.0)).unwrap();
        assert_eq!(out, Outcome::Confirmed(Value::Float(-18.0)));
        assert!(out.is_confirmed());
    }

    /// The failure mode the protocol makes easy to miss: the write succeeds and
    /// the device quietly keeps a different value.
    #[test]
    fn a_silent_rejection_is_surfaced_not_swallowed() {
        let t = MockTransport::new()
            // The device clamps to -60 but reports success on the write.
            .data(op::REQ_GET_USER_VOLUME, (-60.0f32).to_le_bytes().to_vec());
        let mut s = session(t, &[]);

        let out = s.write("vol.user", &[], Value::Float(-50.0)).unwrap();
        match out {
            Outcome::Rejected { sent, actual } => {
                assert_eq!(sent, Value::Float(-50.0));
                assert_eq!(actual, Value::Float(-60.0));
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_is_refused_before_the_wire() {
        let t = MockTransport::new();
        let mut s = session(t, &[("psychoacoustic_bass", true)]);

        let e = s.write("bass.drive", &[], Value::Float(99.0)).unwrap_err();
        assert!(e.to_string().contains("18"), "should name the limit: {e}");
    }

    #[test]
    fn a_missing_feature_is_explained_not_stalled() {
        let t = MockTransport::new();
        let mut s = session(t, &[("psychoacoustic_bass", false)]);

        let e = s.write("bass.on", &[], Value::Bool(true)).unwrap_err();
        assert!(e.to_string().contains("psychoacoustic_bass"), "{e}");
    }

    #[test]
    fn rp2350_only_parameters_are_refused_on_rp2040() {
        let mut caps = caps(Platform::Rp2040, &[]);
        caps.num_inputs = 2;
        caps.num_outputs = 5;
        caps.num_channels = 7;
        caps.channels.truncate(7);
        let mut s = Session::new(Box::new(MockTransport::new()), caps).unwrap();

        let e = s
            .write("up.config", &[], Value::Bytes(vec![0; 44]))
            .unwrap_err();
        assert!(e.to_string().contains("RP2350"), "{e}");
    }

    /// Topology comes from the device, so an index valid on one platform must be
    /// rejected on another.
    #[test]
    fn indices_are_checked_against_the_actual_topology() {
        // RP2350 has nine outputs, so 8 is the last valid index and 9 is not.
        let t = MockTransport::new().data(op::REQ_GET_OUTPUT_GAIN, 0.0f32.to_le_bytes().to_vec());
        let mut s = session(t, &[]);
        assert!(s.write("out.gain", &[8], Value::Float(0.0)).is_ok());
        assert!(s.write("out.gain", &[9], Value::Float(0.0)).is_err());

        let mut caps40 = caps(Platform::Rp2040, &[]);
        caps40.num_inputs = 2;
        caps40.num_outputs = 5;
        caps40.num_channels = 7;
        caps40.channels.truncate(7);
        let mut s40 = Session::new(Box::new(MockTransport::new()), caps40).unwrap();
        let e = s40.write("out.gain", &[6], Value::Float(0.0)).unwrap_err();
        assert!(e.to_string().contains("output"), "{e}");
    }

    #[test]
    fn missing_indices_are_caught_with_a_useful_message() {
        let t = MockTransport::new();
        let mut s = session(t, &[]);
        let e = s.write("eq.freq", &[8], Value::Float(1000.0)).unwrap_err();
        assert!(e.to_string().contains("2 index"), "{e}");
    }

    /// The band field is five bits, so 20-23 must be addressable; band 15 sits
    /// in the reserved gap the firmware rejects.
    #[test]
    fn crossover_bands_are_addressable_and_the_gap_is_not() {
        let t = MockTransport::new().data(op::REQ_GET_EQ_PARAM, 80.0f32.to_le_bytes().to_vec());
        let mut s = session(t, &[]);
        assert!(s.write("eq.freq", &[8, 20], Value::Float(80.0)).is_ok());
        assert!(s.write("eq.freq", &[8, 15], Value::Float(80.0)).is_err());
    }

    /// The band field is five bits wide. A four-bit shift, which older host code
    /// used, silently aliases band 20 onto band 2 of the wrong parameter.
    #[test]
    fn eq_wvalue_packs_the_band_into_five_bits() {
        let d = by_path("eq.freq").unwrap();

        // (channel << 8) | (band << 3) | param, param 1 = freq.
        assert_eq!(wvalue_for(d, &[8, 23], &Value::Float(0.0)), 0x08B9);
        assert_eq!(wvalue_for(d, &[0, 0], &Value::Float(0.0)), 0x0001);
        assert_eq!(wvalue_for(d, &[8, 20], &Value::Float(0.0)), 0x08A1);

        // Bands above 15 must not collide with the next channel.
        let band20 = wvalue_for(d, &[1, 20], &Value::Float(0.0));
        assert_eq!(band20 >> 8, 1, "band 20 must stay within channel 1");

        // Each scalar gets its own param index.
        assert_eq!(
            wvalue_for(by_path("eq.type").unwrap(), &[8, 3], &Value::Choice(0)) & 7,
            0
        );
        assert_eq!(
            wvalue_for(by_path("eq.q").unwrap(), &[8, 3], &Value::Float(0.0)) & 7,
            2
        );
        assert_eq!(
            wvalue_for(by_path("eq.gain").unwrap(), &[8, 3], &Value::Float(0.0)) & 7,
            3
        );
    }

    #[test]
    fn wvalue_packs_targets_and_values_per_rule() {
        // (new_type << 8) | slot
        let ty = by_path("out.type").unwrap();
        assert_eq!(wvalue_for(ty, &[2], &Value::Choice(1)), 0x0102);

        // (gpio << 8) | index
        let pin = by_path("out.pin").unwrap();
        assert_eq!(wvalue_for(pin, &[4], &Value::Int(21)), 0x1504);

        // plain target
        assert_eq!(
            wvalue_for(by_path("out.gain").unwrap(), &[3], &Value::Float(0.0)),
            3
        );

        // value only, no data stage
        assert_eq!(
            wvalue_for(by_path("i2s.mck").unwrap(), &[], &Value::Bool(true)),
            1
        );

        // fixed selector
        assert_eq!(
            wvalue_for(by_path("cs.caps").unwrap(), &[], &Value::Trigger),
            0xFFFF
        );

        // (role << 8) | gpio, and the bare role on the way back
        // (config.h:334-336). Master and slave share one opcode, so the role
        // is the only thing telling the two clock pairs apart.
        let master = by_path("i2s.bck").unwrap();
        let slave = by_path("i2s.bck.slave").unwrap();
        assert_eq!(master.set, slave.set, "one opcode, two roles");
        assert_eq!(wvalue_for(master, &[], &Value::Int(14)), 0x000E);
        assert_eq!(wvalue_for(slave, &[], &Value::Int(26)), 0x011A);
        assert_eq!(read_wvalue_for(master, &[]), 0);
        assert_eq!(read_wvalue_for(slave, &[]), 1);
    }

    /// The limiter packs `(output << 8) | index` both ways (limiter.h:11), and
    /// output 0xFF is every output (limiter.h:21).
    #[test]
    fn the_limiter_packs_output_then_index() {
        let threshold = by_path("limit.threshold").unwrap();
        assert_eq!(wvalue_for(threshold, &[3], &Value::Float(-3.0)), 0x0301);
        assert_eq!(read_wvalue_for(threshold, &[3]), 0x0301);
        let link = by_path("limit.link").unwrap();
        assert_eq!(wvalue_for(link, &[0xFF], &Value::Int(2)), 0xFF03);
        assert_eq!(
            read_wvalue_for(by_path("limit.meter").unwrap(), &[]),
            0x0080
        );
    }

    /// A tube or limiter boolean is still a float on the wire: a one-byte
    /// payload is a short payload, which the firmware ignores (tube.c:122-212).
    #[test]
    fn indexed_booleans_are_sent_as_floats_and_read_back_as_booleans() {
        let t = MockTransport::new().data(op::REQ_GET_TUBE_PARAM, 1.0f32.to_le_bytes().to_vec());
        let log = t.log_handle();
        let mut s = session(t, &[("tube_preamp", true)]);

        let out = s.write("tube.on", &[], Value::Bool(true)).unwrap();
        assert_eq!(out, Outcome::Confirmed(Value::Bool(true)));
        let sent = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.opcode == op::REQ_SET_TUBE_PARAM)
            .cloned()
            .unwrap();
        assert_eq!(sent.value, 0, "TUBE_PARAM_ENABLED");
        assert_eq!(sent.payload, 1.0f32.to_le_bytes().to_vec());

        // A mask rounds back from the float the device answers.
        let t = MockTransport::new().data(op::REQ_GET_TUBE_PARAM, 3.0f32.to_le_bytes().to_vec());
        let mut s = session(t, &[("tube_preamp", true)]);
        assert_eq!(s.read("tube.mask", &[]).unwrap(), Value::Mask(3));
        assert_eq!(s.read("tube.rectifier", &[]).unwrap(), Value::Choice(3));
    }

    /// The upmixer's indexed opcode takes a float for its switches too, and
    /// stalls on anything shorter (upmix.h:185-186, vendor_commands.c:1829).
    #[test]
    fn the_upmixer_switch_is_sent_as_a_float() {
        let t = MockTransport::new().data(op::REQ_UPMIX_GET_PARAM, 0.0f32.to_le_bytes().to_vec());
        let log = t.log_handle();
        let mut s = session(t, &[]);
        s.write("up.on", &[], Value::Bool(false)).unwrap();
        let sent = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.opcode == op::REQ_UPMIX_SET_PARAM)
            .cloned()
            .unwrap();
        assert_eq!(sent.payload.len(), 4);
    }

    /// Every output at once is a write, never a read: the device stalls a
    /// limiter GET of output 0xFF (limiter.h:21).
    #[test]
    fn every_output_is_writable_but_not_readable() {
        let t = MockTransport::new().data(op::REQ_LIMITER, (-6.0f32).to_le_bytes().to_vec());
        let log = t.log_handle();
        let mut s = session(t, &[("output_limiter", true)]);

        s.write("limit.threshold", &[0xFF], Value::Float(-6.0))
            .unwrap();
        let wrote = log
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.opcode == op::REQ_LIMITER && e.value == 0xFF01 && !e.payload.is_empty());
        assert!(wrote, "the all-outputs write went out as 0xFF01");

        assert!(matches!(
            s.read("limit.threshold", &[0xFF]),
            Err(WriteError::BadTarget(0xFF, _))
        ));
        // One past the last output is still refused on either path.
        assert!(s.write("limit.on", &[9], Value::Bool(true)).is_err());
        assert!(s.read("limit.threshold", &[8]).is_ok());
    }

    /// The aux level is an 8.8 percentage (config.h:141-145).
    #[test]
    fn the_aux_level_travels_as_eight_dot_eight() {
        let t = MockTransport::new().data(op::REQ_GET_CS_AUX_LEVEL, vec![0x00, 0x32]);
        let log = t.log_handle();
        let mut s = session(t, &[]);
        let out = s.write("cs.aux.level", &[2], Value::Float(50.0)).unwrap();
        assert_eq!(out, Outcome::Confirmed(Value::Float(50.0)));
        let sent = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.opcode == op::REQ_SET_CS_AUX_LEVEL)
            .cloned()
            .unwrap();
        assert_eq!((sent.value, sent.payload), (2, vec![0x00, 0x32]));
    }

    /// A read addresses the parameter but must never smuggle a value into wValue.
    #[test]
    fn reads_do_not_carry_a_value() {
        assert_eq!(read_wvalue_for(by_path("out.type").unwrap(), &[2]), 2);
        assert_eq!(read_wvalue_for(by_path("out.pin").unwrap(), &[4]), 4);
    }

    /// Write-as-read opcodes must go out on the IN path or they stall.
    #[test]
    fn a_flash_write_survives_the_blackout() {
        let t = MockTransport::new()
            .data(op::REQ_PRESET_SET_NAME, vec![0])
            .reply(
                op::REQ_PRESET_GET_NAME,
                Reply::BusyThen(3, Box::new(Reply::Data(padded("Living Room")))),
            );
        let mut s = session(t, &[]);

        let out = s
            .write("preset.name", &[3], Value::Text("Living Room".into()))
            .unwrap();
        assert!(matches!(out, Outcome::Confirmed(_)));
    }

    #[test]
    fn dry_run_writes_nothing() {
        let t = MockTransport::new();
        let mut s = session(t, &[]);
        s.dry_run = true;

        let out = s.write("vol.user", &[], Value::Float(-20.0)).unwrap();
        assert_eq!(out, Outcome::Accepted);
        assert_eq!(s.journal.len(), 1);
        // Nothing left the host.
        assert!(!s.transport.descriptor().serial.is_empty());
    }

    #[test]
    fn the_journal_records_before_and_after() {
        let t =
            MockTransport::new().data(op::REQ_GET_USER_VOLUME, (-18.0f32).to_le_bytes().to_vec());
        let mut s = session(t, &[]);
        s.write("vol.user", &[], Value::Float(-18.0)).unwrap();

        let e = s.journal.last().unwrap();
        assert_eq!(e.path, "vol.user");
        assert_eq!(e.after, Value::Float(-18.0));
        assert!(e.undoable);
    }

    /// Flash writes and reconfiguration cannot be undone, so they must not be
    /// offered as undoable; they are confirmed before the fact instead.
    #[test]
    fn irreversible_changes_are_not_marked_undoable() {
        let t = MockTransport::new()
            .data(op::REQ_PRESET_SET_NAME, vec![0])
            .data(op::REQ_PRESET_GET_NAME, padded("Test"));
        let mut s = session(t, &[]);
        s.write("preset.name", &[0], Value::Text("Test".into()))
            .unwrap();
        assert!(!s.journal.last().unwrap().undoable);
    }

    // -- WP TM18 --
    mod tm18_siggen {
        use super::*;
        use dspi_proto::registry::REGISTRY;
        use dspi_transport::mock::{Direction, Reply};

        fn with_status(state: u8) -> MockTransport {
            let status = SiggenStatus {
                state,
                ..Default::default()
            };
            MockTransport::new()
                .data(op::REQ_SIGGEN_CONTROL, vec![1])
                .data(op::REQ_SIGGEN_GET_STATUS, status.encode().to_vec())
        }

        fn control(t: MockTransport, choice: u8) -> Outcome {
            let mut s = session(t, &[("test_signals", true)]);
            s.write("sig.control", &[], Value::Choice(choice)).unwrap()
        }

        /// The version byte comes first in the status and is always 1, which
        /// is what made every readback say "start".
        #[test]
        fn tm18_the_readback_is_the_state_not_the_version() {
            for (state, choice) in [(0, 0), (1, 1), (2, 1), (3, 1), (4, 0)] {
                let mut s = session(with_status(state), &[("test_signals", true)]);
                assert_eq!(
                    s.read("sig.control", &[]).unwrap(),
                    Value::Choice(choice),
                    "state {state}"
                );
            }
        }

        #[test]
        fn tm18_stop_confirms_while_the_signal_fades_or_once_idle() {
            for state in [0, 4] {
                assert_eq!(
                    control(with_status(state), 0),
                    Outcome::Confirmed(Value::Choice(0)),
                    "state {state}"
                );
            }
            assert_eq!(
                control(with_status(0), 2),
                Outcome::Confirmed(Value::Choice(2))
            );
        }

        #[test]
        fn tm18_a_stop_the_device_did_not_take_is_reported() {
            for choice in [0, 2] {
                assert_eq!(
                    control(with_status(2), choice),
                    Outcome::Rejected {
                        sent: Value::Choice(choice),
                        actual: Value::Choice(1),
                    }
                );
            }
        }

        /// A Start on a running generator fades it out and relaunches
        /// (siggen.c:1244-1250), so fading out is a Start that took.
        #[test]
        fn tm18_start_confirms_on_anything_but_idle() {
            for state in [1, 2, 3, 4] {
                assert_eq!(
                    control(with_status(state), 1),
                    Outcome::Confirmed(Value::Choice(1)),
                    "state {state}"
                );
            }
            assert_eq!(
                control(with_status(0), 1),
                Outcome::Rejected {
                    sent: Value::Choice(1),
                    actual: Value::Choice(0),
                }
            );
        }

        /// Stop goes out first, with nothing read ahead of it, and a status
        /// that will not answer does not turn it into a failure.
        #[test]
        fn tm18_stop_is_not_held_up_by_the_status() {
            for choice in [0, 2] {
                let t = MockTransport::new()
                    .data(op::REQ_SIGGEN_CONTROL, vec![1])
                    .reply(op::REQ_SIGGEN_GET_STATUS, Reply::Stall);
                let log = t.log_handle();
                assert_eq!(control(t, choice), Outcome::Accepted);
                let log = log.lock().unwrap();
                let first = log.first().expect("something on the wire");
                assert_eq!(first.opcode, op::REQ_SIGGEN_CONTROL);
                assert_eq!(first.direction, Direction::In);
                assert_eq!(first.value, choice as u16);
            }
        }

        /// Every audible parameter, not only the generator's.
        #[test]
        fn tm18_nothing_audible_can_be_undone() {
            let audible: Vec<_> = REGISTRY
                .iter()
                .filter(|d| d.hazard == Hazard::Audible)
                .collect();
            assert!(audible.iter().any(|d| d.path == "sig.config"));
            assert!(audible.iter().any(|d| d.path == "sig.control"));
            let mut s = session(MockTransport::new(), &[]);
            for d in audible {
                s.record(
                    d,
                    &[],
                    Some(Value::Choice(1)),
                    Vec::new(),
                    Value::Choice(0),
                    Outcome::Accepted,
                );
                assert!(!s.journal.last().unwrap().undoable, "{}", d.path);
            }
            #[cfg(feature = "undo")]
            assert!(!s.can_undo());
        }

        /// What a write moves besides itself is restored by undo, so the
        /// generator must not be among it for any parameter. The one audible
        /// companion is `limit.on` under a link change: that puts a limiter's
        /// state back and starts nothing.
        #[test]
        fn tm18_undo_cannot_reach_the_generator_as_a_companion() {
            for d in REGISTRY {
                for at in [vec![0], vec![ALL_OUTPUTS], Vec::new()] {
                    for (path, _) in side_effects(d.path, &at, 9) {
                        let hazard = by_path(path).map(|c| c.hazard);
                        assert!(
                            hazard == Some(Hazard::None) || path == "limit.on",
                            "{} moves {path}",
                            d.path
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_parameters_are_named_in_the_error() {
        let t = MockTransport::new();
        let mut s = session(t, &[]);
        let e = s
            .write("bass.nonsense", &[], Value::Bool(true))
            .unwrap_err();
        assert!(e.to_string().contains("bass.nonsense"));
    }

    #[test]
    fn read_only_parameters_refuse_writes() {
        let t = MockTransport::new();
        let mut s = session(t, &[]);
        let e = s
            .write("dev.serial", &[], Value::Text("x".into()))
            .unwrap_err();
        assert!(e.to_string().contains("read-only"), "{e}");
    }

    // -- WP TM26 -- hw-1 D2: `dspi get rta.control` said "read-only".
    #[test]
    fn tm26_reading_an_action_says_write_only() {
        let t = MockTransport::new();
        let mut s = session(t, &[]);
        let e = s.read("rta.control", &[]).unwrap_err();
        assert!(matches!(e, WriteError::WriteOnly { .. }), "{e:?}");
        let text = e.to_string();
        assert!(text.contains("write-only"), "{text}");
        assert!(!text.contains("read-only"), "{text}");
        let e = s.read_whole("rta.control", &[]).err();
        assert!(
            e.as_ref()
                .is_none_or(|e| matches!(e, WriteError::WriteOnly { .. })),
            "{e:?}"
        );
    }

    // -- WP TM26 -- hw-1 M1: `cs.aux 0` stalled on a slot with no aux output.
    fn tm26_with_binding(component: u8) -> Session {
        use dspi_proto::packets::CsBinding;
        let b = CsBinding {
            component,
            ..CsBinding::default()
        };
        // The aux opcodes stall, as on the K340.
        let t = MockTransport::new()
            .data(op::REQ_GET_CS_BINDING, b.encode().to_vec())
            .reply(op::REQ_SET_CS_AUX_STATE, Reply::Stall);
        session(t, &[])
    }

    #[test]
    fn tm26_aux_stall_on_an_empty_slot_says_so() {
        use crate::cs_model::ty;
        let mut s = tm26_with_binding(ty::NONE);
        for path in ["cs.aux", "cs.aux.level"] {
            let e = s.read(path, &[0]).unwrap_err();
            assert!(matches!(e, WriteError::Unavailable { .. }), "{e:?}");
            assert!(e.to_string().contains("slot 0 is empty"), "{e}");
        }
        let e = s.write("cs.aux", &[0], Value::Bool(true)).unwrap_err();
        assert!(e.to_string().contains("no auxiliary output"), "{e}");

        let mut s = tm26_with_binding(ty::BUTTON);
        let e = s.read("cs.aux", &[0]).unwrap_err();
        assert!(
            e.to_string().contains("slot 0 holds no auxiliary output"),
            "{e}"
        );
    }

    #[test]
    fn tm26_aux_level_on_an_on_off_output_says_it_has_no_level() {
        use crate::cs_model::ty;
        let mut s = tm26_with_binding(ty::AUX_OUT);
        let e = s.read("cs.aux.level", &[0]).unwrap_err();
        assert!(e.to_string().contains("on/off auxiliary output"), "{e}");
        // An on/off output that stalls its state is a real stall, passed on.
        let e = s.read("cs.aux", &[0]).unwrap_err();
        assert!(
            matches!(e, WriteError::Transport(TransportError::Stalled { .. })),
            "{e:?}"
        );
    }

    #[test]
    fn tm26_aux_stall_with_an_unreadable_binding_stays_a_stall() {
        let mut s = session(MockTransport::new(), &[]);
        let e = s.read("cs.aux", &[0]).unwrap_err();
        assert!(
            matches!(e, WriteError::Transport(TransportError::Stalled { .. })),
            "{e:?}"
        );
    }

    #[test]
    fn float_comparison_tolerates_single_precision() {
        assert!(values_agree(
            &Value::Float(2856.0),
            &Value::Float(2856.0001)
        ));
        assert!(!values_agree(&Value::Float(2856.0), &Value::Float(2900.0)));
        // Small values compare absolutely rather than relatively.
        assert!(values_agree(&Value::Float(0.0), &Value::Float(0.0002)));
    }

    #[test]
    fn mock_log_shows_direction_for_out_writes() {
        let mut t = MockTransport::new();
        let _ = t.control_out(op::REQ_SET_USER_VOLUME, 0, &[0, 0, 0, 0]);
        assert_eq!(t.log()[0].direction, Direction::Out);
    }
}

#[cfg(test)]
mod eq_tests {
    use super::tests::caps;
    use super::*;
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::Direction;

    /// The bug this replaced: a scalar write to 0x42 sends four bytes with a
    /// packed wValue, but the firmware wants a 16-byte descriptor and reads
    /// nothing from wValue. It would have been rejected on hardware.
    /// Build a session over a mock, keeping a handle on the wire log.
    fn rig(reply: [u8; 4]) -> (Session, dspi_transport::mock::LogHandle) {
        let t = MockTransport::new().data(op::REQ_GET_EQ_PARAM, reply.to_vec());
        let log = t.log_handle();
        let s = Session::new(Box::new(t), caps(Platform::Rp2350, &[])).unwrap();
        (s, log)
    }

    #[test]
    fn changing_one_field_sends_a_whole_band_packet() {
        let (mut s, log) = rig(1000.0f32.to_le_bytes());

        s.write("eq.freq", &[8, 3], Value::Float(2856.0)).unwrap();

        let writes: Vec<_> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out && e.opcode == op::REQ_SET_EQ_PARAM)
            .cloned()
            .collect();

        assert_eq!(writes.len(), 1, "exactly one band write");
        let w = &writes[0];
        assert_eq!(
            w.payload.len(),
            16,
            "the firmware wants a 16-byte descriptor"
        );
        assert_eq!(w.value, 0, "wValue carries nothing for this opcode");
        assert_eq!(w.payload[0], 8, "channel travels in the payload");
        assert_eq!(w.payload[1], 3, "so does the band");
        assert_eq!(
            &w.payload[4..8],
            &2856.0f32.to_le_bytes(),
            "the new frequency"
        );
    }

    /// Fields the user did not touch must survive the round trip.
    #[test]
    fn untouched_fields_are_preserved() {
        let (mut s, log) = rig(3.5f32.to_le_bytes());

        s.write("eq.gain", &[8, 3], Value::Float(-6.0)).unwrap();

        let w = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .cloned()
            .unwrap();
        // Freq and Q came from the read, not from defaults.
        assert_eq!(&w.payload[4..8], &3.5f32.to_le_bytes());
        assert_eq!(&w.payload[8..12], &3.5f32.to_le_bytes());
        assert_eq!(&w.payload[12..16], &(-6.0f32).to_le_bytes());
    }

    /// A plain band write must stay 16 bytes, or it would clobber the stored Qp.
    #[test]
    fn non_linkwitz_writes_never_grow_to_eighteen_bytes() {
        let (mut s, log) = rig(1000.0f32.to_le_bytes());
        s.write("eq.q", &[8, 3], Value::Float(2.0)).unwrap();

        let w = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .cloned()
            .unwrap();
        assert_eq!(w.payload.len(), 16);
    }
}

/// One matrix crosspoint.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Crosspoint {
    pub enabled: bool,
    pub phase_invert: bool,
    pub gain_db: f32,
}

/// One output's strip settings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OutputStrip {
    pub enabled: bool,
    pub mute: bool,
    pub gain_db: f32,
    pub delay_ms: f32,
}

impl Session {
    /// Read the whole matrix in one transfer.
    ///
    /// Reading it a crosspoint at a time would be 72 transfers on an RP2350; the
    /// bulk snapshot carries the same data in one, so that is what this uses.
    /// The returned grid is indexed `[input][output]`.
    pub fn read_matrix(&mut self) -> Result<(Vec<Vec<Crosspoint>>, Vec<OutputStrip>), WriteError> {
        let packet = crate::read_bulk(&mut *self.transport)?;

        // The wire array is always sized at the platform maximum and
        // zero-padded, so the header counts say how much of it is real.
        const WIRE_OUTPUTS: usize = 9;
        let n_in = self.caps.num_inputs as usize;
        let n_out = self.caps.num_outputs as usize;

        let cross = packet.section("crosspoints").unwrap_or(&[]);
        let mut grid = vec![vec![Crosspoint::default(); n_out]; n_in];
        for (i, row) in grid.iter_mut().enumerate() {
            for (o, cell) in row.iter_mut().enumerate() {
                let off = (i * WIRE_OUTPUTS + o) * 8;
                if off + 8 > cross.len() {
                    continue;
                }
                *cell = Crosspoint {
                    enabled: cross[off] != 0,
                    phase_invert: cross[off + 1] != 0,
                    gain_db: f32::from_le_bytes([
                        cross[off + 4],
                        cross[off + 5],
                        cross[off + 6],
                        cross[off + 7],
                    ]),
                };
            }
        }

        let outs = packet.section("outputs").unwrap_or(&[]);
        let strips = (0..n_out)
            .map(|o| {
                let off = o * 12;
                if off + 12 > outs.len() {
                    return OutputStrip::default();
                }
                OutputStrip {
                    enabled: outs[off] != 0,
                    mute: outs[off + 1] != 0,
                    gain_db: f32::from_le_bytes([
                        outs[off + 4],
                        outs[off + 5],
                        outs[off + 6],
                        outs[off + 7],
                    ]),
                    delay_ms: f32::from_le_bytes([
                        outs[off + 8],
                        outs[off + 9],
                        outs[off + 10],
                        outs[off + 11],
                    ]),
                }
            })
            .collect();

        Ok((grid, strips))
    }

    /// Would enabling this output collide with what Core 1 is already doing?
    ///
    /// PDM and the Core 1 EQ worker are mutually exclusive, and the firmware
    /// silently ignores an enable that would break that. Asking first is the
    /// difference between a greyed-out option with a reason and a control that
    /// appears to do nothing.
    pub fn core1_conflict(&mut self, output: u8) -> bool {
        self.try_core1_conflict(output).unwrap_or(false)
    }

    /// [`Session::core1_conflict`] with the read's failure kept: a caller
    /// about to write must not read a lost device as "no conflict". An empty
    /// reply is [`TransportError::ShortRead`] too.
    pub fn try_core1_conflict(&mut self, output: u8) -> Result<bool, TransportError> {
        let d = self
            .transport
            .control_in(op::REQ_GET_CORE1_CONFLICT, output as u16, 1)?;
        d.first().map(|b| *b != 0).ok_or(TransportError::ShortRead {
            opcode: op::REQ_GET_CORE1_CONFLICT,
            wanted: 1,
            got: 0,
        })
    }

    /// The PDM sub, which is always the last output
    /// (`DSPViewModel.swift:2124`).
    pub fn pdm_output(&self) -> Option<u8> {
        self.map.num_outputs().checked_sub(1)
    }

    /// The outputs Core 1 runs EQ for, which is the other half of the
    /// interlock.
    ///
    /// `CORE1_EQ_FIRST_OUTPUT` is 2 on both platforms and
    /// `CORE1_EQ_LAST_OUTPUT` is the output below PDM (config.h:764-770: 7 of
    /// nine outputs on RP2350, 3 of five on RP2040), so the range follows the
    /// discovered output count rather than a compiled-in platform test.
    pub fn eq_worker_outputs(&self) -> std::ops::RangeInclusive<u8> {
        const FIRST: u8 = 2;
        match self.map.num_outputs().checked_sub(2) {
            Some(last) if last >= FIRST => FIRST..=last,
            // Nothing to conflict with on a part this small, so an empty range
            // rather than a made-up one.
            _ => std::ops::RangeInclusive::new(1, 0),
        }
    }

    /// The dialog the Console shows before an enable that would take the other
    /// side of Core 1 down, or `None` when there is no collision.
    pub fn core1_conflict_alert(&mut self, output: u8) -> Option<Core1Conflict> {
        if !self.core1_conflict(output) {
            return None;
        }
        // MatrixMixerView.swift:193-217. Both alerts are titled "Warning"; which
        // one shows depends on which side of the interlock is being asked for.
        let pdm = self.pdm_output();
        Some(if Some(output) == pdm {
            let eq = self.eq_worker_outputs();
            Core1Conflict {
                title: "Warning".into(),
                // 1-based for display, as the Console counts them.
                body: format!(
                    "Outputs {}-{} will be disabled. Are you sure?",
                    eq.start() + 1,
                    eq.end() + 1
                ),
                confirm: "Enable PDM".into(),
            }
        } else {
            Core1Conflict {
                title: "Warning".into(),
                body: "The PDM output will be disabled. Are you sure?".into(),
                confirm: "Disable PDM".into(),
            }
        })
    }

    /// Turn an output on or off, consulting the Core 1 interlock first.
    ///
    /// Disabling always lands. Enabling may need the person to agree to the
    /// other side being switched off, which is what
    /// [`EnableOutcome::NeedsConfirm`] carries.
    pub fn enable_output(&mut self, index: u8, enable: bool) -> Result<EnableOutcome, WriteError> {
        if enable && let Some(c) = self.core1_conflict_alert(index) {
            return Ok(EnableOutcome::NeedsConfirm(c));
        }
        self.set_enable(index, enable)
    }

    /// Enable an output after the person has agreed to the conflict.
    ///
    /// Frees the other side of Core 1 first, in the order the Console uses
    /// (`Commands.swift:1453-1474`): enabling PDM disables every EQ-worker
    /// output; enabling an EQ-worker output disables PDM.
    pub fn enable_output_confirmed(&mut self, index: u8) -> Result<EnableOutcome, WriteError> {
        let pdm = self.pdm_output();
        if Some(index) == pdm {
            for o in self.eq_worker_outputs() {
                self.set_enable(o, false)?;
            }
        } else if let Some(pdm) = pdm {
            self.set_enable(pdm, false)?;
        }
        self.set_enable(index, true)
    }

    /// The write itself, with the interlock already decided.
    fn set_enable(&mut self, index: u8, enable: bool) -> Result<EnableOutcome, WriteError> {
        self.core1_checked = true;
        let written = self.write("out.enable", &[index], Value::Bool(enable));
        self.core1_checked = false;

        // A blocked enable is skipped in silence (survey-firmware 6.8), so the
        // readback is the only evidence that it took. `write` already compares
        // it, and reports the mismatch as a rejection.
        Ok(match written? {
            Outcome::Rejected { .. } => EnableOutcome::Rejected,
            _ => EnableOutcome::Done,
        })
    }
}

/// The Console's confirmation dialog for a Core 1 collision, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core1Conflict {
    pub title: String,
    pub body: String,
    /// The label on the button that goes ahead.
    pub confirm: String,
}

/// What happened when an output was asked to turn on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnableOutcome {
    /// The output is now in the state that was asked for.
    Done,
    /// Someone has to agree to the other side of Core 1 being switched off.
    NeedsConfirm(Core1Conflict),
    /// The device took the request and kept the old state anyway.
    Rejected,
}

#[cfg(test)]
mod core1_tests {
    use super::tests::caps;
    use super::*;
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, LogHandle};

    /// A device that reports a conflict for `output`, and answers the enable
    /// readback with `enabled`.
    fn rig(conflict: bool, enabled: bool) -> (Session, LogHandle) {
        let t = MockTransport::new()
            .data(op::REQ_GET_CORE1_CONFLICT, vec![conflict as u8])
            .data(op::REQ_SET_OUTPUT_ENABLE, vec![0])
            .data(op::REQ_GET_OUTPUT_ENABLE, vec![enabled as u8]);
        let log = t.log_handle();
        let s = Session::new(Box::new(t), caps(Platform::Rp2350, &[])).unwrap();
        (s, log)
    }

    /// The PDM alert names the outputs it is about to take down, 1-based, from
    /// the discovered output count rather than a platform test.
    #[test]
    fn enabling_pdm_warns_about_the_eq_worker_outputs() {
        let (mut s, _) = rig(true, false);
        let pdm = s.pdm_output().unwrap();
        assert_eq!(pdm, 8, "the last of nine outputs");

        match s.enable_output(pdm, true).unwrap() {
            EnableOutcome::NeedsConfirm(c) => {
                assert_eq!(c.title, "Warning");
                assert_eq!(c.body, "Outputs 3-8 will be disabled. Are you sure?");
                assert_eq!(c.confirm, "Enable PDM");
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
    }

    #[test]
    fn the_eq_worker_range_follows_the_output_count() {
        let mut small = caps(Platform::Rp2040, &[]);
        small.num_inputs = 2;
        small.num_outputs = 5;
        small.num_channels = 7;
        small.channels.truncate(7);
        let t = MockTransport::new().data(op::REQ_GET_CORE1_CONFLICT, vec![1]);
        let mut s = Session::new(Box::new(t), small).unwrap();

        // config.h:764-770: outputs 2-3 on RP2040, shown 1-based.
        assert_eq!(s.eq_worker_outputs(), 2..=3);
        let pdm = s.pdm_output().unwrap();
        match s.enable_output(pdm, true).unwrap() {
            EnableOutcome::NeedsConfirm(c) => {
                assert_eq!(c.body, "Outputs 3-4 will be disabled. Are you sure?");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn enabling_an_eq_worker_output_warns_about_pdm() {
        let (mut s, _) = rig(true, false);
        match s.enable_output(3, true).unwrap() {
            EnableOutcome::NeedsConfirm(c) => {
                assert_eq!(c.title, "Warning");
                assert_eq!(c.body, "The PDM output will be disabled. Are you sure?");
                assert_eq!(c.confirm, "Disable PDM");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Disabling never collides, so it must not stop to ask.
    #[test]
    fn disabling_never_asks() {
        let (mut s, _) = rig(true, false);
        assert_eq!(s.enable_output(8, false).unwrap(), EnableOutcome::Done);
    }

    /// The plain write path has to refuse too, or a script gets a success for a
    /// write the firmware quietly dropped.
    #[test]
    fn a_plain_write_is_refused_with_the_reason() {
        let (mut s, log) = rig(true, false);
        let e = s.write("out.enable", &[3], Value::Bool(true)).unwrap_err();
        match &e {
            WriteError::Core1Conflict {
                output, confirm, ..
            } => {
                assert_eq!(*output, 3);
                assert_eq!(confirm, "Disable PDM");
            }
            other => panic!("{other:?}"),
        }
        assert!(e.to_string().contains("PDM output will be disabled"), "{e}");
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|x| x.direction == Direction::Out),
            "nothing should have gone out"
        );
    }

    /// With no conflict the write goes through as it always did.
    #[test]
    fn without_a_conflict_the_write_is_untouched() {
        let (mut s, _) = rig(false, true);
        assert!(s.write("out.enable", &[3], Value::Bool(true)).is_ok());
    }

    /// After confirming, the other side is freed first and in the Console's
    /// order: every EQ-worker output off, then PDM on.
    #[test]
    fn confirming_pdm_frees_the_eq_workers_first() {
        let (mut s, log) = rig(false, true);
        assert_eq!(s.enable_output_confirmed(8).unwrap(), EnableOutcome::Done);

        let sent: Vec<(u16, Vec<u8>)> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out && e.opcode == op::REQ_SET_OUTPUT_ENABLE)
            .map(|e| (e.value, e.payload.clone()))
            .collect();

        assert_eq!(sent.len(), 7, "six EQ-worker outputs, then PDM");
        for (i, (target, payload)) in sent.iter().take(6).enumerate() {
            assert_eq!(*target, 2 + i as u16);
            assert_eq!(payload[0], 0, "the EQ workers go off first");
        }
        assert_eq!(sent[6], (8, vec![1]));
    }

    #[test]
    fn confirming_an_eq_worker_output_drops_pdm_first() {
        let (mut s, log) = rig(false, true);
        s.enable_output_confirmed(3).unwrap();

        let sent: Vec<(u16, u8)> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out && e.opcode == op::REQ_SET_OUTPUT_ENABLE)
            .map(|e| (e.value, e.payload[0]))
            .collect();
        assert_eq!(sent, vec![(8, 0), (3, 1)]);
    }

    /// The firmware skips a blocked enable in silence (survey-firmware 6.8), so
    /// a readback that still says "off" has to be reported as a rejection.
    #[test]
    fn a_silently_skipped_enable_comes_back_as_rejected() {
        let (mut s, _) = rig(false, false);
        assert_eq!(s.enable_output(3, true).unwrap(), EnableOutcome::Rejected);
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::*;
    use crate::probe::Capabilities;
    use dspi_proto::generated;
    use dspi_transport::MockTransport;

    fn bulk_with_matrix() -> Vec<u8> {
        let mut b = vec![0u8; generated::BULK_SIZE];
        b[0] = generated::wire::WIRE_FORMAT_VERSION as u8;
        b[1] = 1;
        b[2] = 17;
        b[3] = 9;
        b[4] = 8;
        b[5] = 12;
        b[6..8].copy_from_slice(&(generated::BULK_SIZE as u16).to_le_bytes());

        // Input 0 -> output 0, enabled, -3 dB.
        let c = generated::OFF_CROSSPOINTS;
        b[c] = 1;
        b[c + 4..c + 8].copy_from_slice(&(-3.0f32).to_le_bytes());
        // Input 1 -> output 1, enabled and phase inverted.
        // Input 1, output 1: the wire stride is 9, not the device's output count.
        let c1 = generated::OFF_CROSSPOINTS + (9 + 1) * 8;
        b[c1] = 1;
        b[c1 + 1] = 1;

        // Output 0: enabled, -6 dB, 4.2 ms.
        let o = generated::OFF_OUTPUTS;
        b[o] = 1;
        b[o + 4..o + 8].copy_from_slice(&(-6.0f32).to_le_bytes());
        b[o + 8..o + 12].copy_from_slice(&4.2f32.to_le_bytes());
        b
    }

    fn rig() -> (Session, dspi_transport::mock::LogHandle) {
        let t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, bulk_with_matrix());
        let log = t.log_handle();
        (session_over(Box::new(t)), log)
    }

    fn session() -> Session {
        let t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, bulk_with_matrix());
        session_over(Box::new(t))
    }

    fn session_over(t: Box<dyn Transport>) -> Session {
        let caps = Capabilities {
            serial: "T".into(),
            platform: Platform::Rp2350,
            firmware: "1.1.5".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 5, 0),
            build_info: None,
            wire_format: 26,
            num_channels: 17,
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
            band_storage: 12,
            channels: Vec::new(),
            features: Vec::new(),
            cs: None,
            siggen: None,
            active_preset: None,
        };
        Session::new(t, caps).unwrap()
    }

    #[test]
    fn the_matrix_is_sized_from_the_device_not_a_constant() {
        let mut s = session();
        let (grid, strips) = s.read_matrix().unwrap();
        assert_eq!(grid.len(), 8, "eight inputs on RP2350");
        assert_eq!(grid[0].len(), 9, "nine outputs");
        assert_eq!(strips.len(), 9);
    }

    #[test]
    fn crosspoints_decode_gain_and_polarity() {
        let mut s = session();
        let (grid, _) = s.read_matrix().unwrap();
        assert!(grid[0][0].enabled);
        assert!((grid[0][0].gain_db + 3.0).abs() < 1e-5);
        assert!(!grid[0][0].phase_invert);

        assert!(grid[1][1].enabled && grid[1][1].phase_invert);
        assert!(!grid[2][2].enabled, "an unset crosspoint reads as off");
    }

    /// The wire array is always nine wide even on a seven-channel part, so the
    /// row stride must come from the wire, not from the device's output count.
    #[test]
    fn row_stride_follows_the_wire_not_the_device() {
        let mut s = session();
        let (grid, _) = s.read_matrix().unwrap();
        // Input 1's second crosspoint is at wire offset (1*9 + 1); reading with a
        // stride of the device's own count would land somewhere else entirely.
        assert!(grid[1][1].enabled);
        assert!(!grid[1][0].enabled);
    }

    #[test]
    fn output_strips_decode_gain_and_delay() {
        let mut s = session();
        let (_, strips) = s.read_matrix().unwrap();
        assert!(strips[0].enabled);
        assert!((strips[0].gain_db + 6.0).abs() < 1e-5);
        assert!((strips[0].delay_ms - 4.2).abs() < 1e-5);
        assert!(!strips[1].enabled);
    }

    #[test]
    fn a_crosspoint_reads_back_in_the_grammars_own_words() {
        let t = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, bulk_with_matrix())
            .data(
                op::REQ_GET_MATRIX_ROUTE,
                [
                    0u8,
                    4,
                    1,
                    1,
                    (-6.0f32).to_le_bytes()[0],
                    (-6.0f32).to_le_bytes()[1],
                    (-6.0f32).to_le_bytes()[2],
                    (-6.0f32).to_le_bytes()[3],
                ],
            );
        let log = t.log_handle();
        let mut s = session_over(Box::new(t));
        let v = s.read("mix", &[0, 4]).unwrap();
        assert_eq!(v, Value::Text("on -6.0 dB inv".into()));
        let seen = log.lock().unwrap();
        let read = seen
            .iter()
            .find(|e| e.opcode == op::REQ_GET_MATRIX_ROUTE)
            .expect("the read went out");
        assert_eq!(read.value, 0x0004, "input 0, output 4");
    }

    /// Reading the matrix a crosspoint at a time would be 72 transfers on an
    /// RP2350; the bulk snapshot carries the same data in one read.
    #[test]
    fn the_matrix_comes_from_the_bulk_snapshot_not_per_crosspoint_reads() {
        let (mut s, log) = rig();
        let _ = s.read_matrix().unwrap();
        let seen = log.lock().unwrap();
        assert!(
            seen.iter()
                .all(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK),
            "something other than the bulk read went out"
        );
        assert!(
            !seen.iter().any(|e| e.opcode == op::REQ_GET_MATRIX_ROUTE),
            "per-crosspoint reads would be 72 transfers"
        );
    }
}
