//! Raw vendor commands (WP CG4, `docs/core-merge/PLAN.md` §1.2).
//!
//! For a binding whose code has not moved onto the typed API yet: the Swift
//! console's `CoreBackedTransport` (SW4) sends its existing commands through
//! these two methods, so it shares the core's one link instead of opening a
//! second one. New code uses the typed API.
//!
//! Three rules keep the shortcut from undoing what the typed API guarantees:
//!
//! - **Writes are typed.** [`DspiCore::raw_set`], and [`DspiCore::raw_get`] for
//!   any opcode that is not a known plain read, go through
//!   [`Inner::writable_conn`](crate::Inner::writable_conn), so a read-only
//!   connection (§1.5) refuses them with no traffic. The list of plain reads
//!   ([`is_plain_read`]) fails closed: an opcode this build does not know
//!   counts as a write.
//! - **Bulk sessions stay the core's.** Each call takes the core's lock for
//!   its one transfer, so it cannot land inside a chunked session the core is
//!   running, which any other request would abort on the device. The chunk
//!   opcodes themselves are refused ([`check_not_bulk_session`]): a session
//!   spanning several raw calls could not hold the lock between them. So is
//!   the single-shot bulk write, whose checks and echo bracket are
//!   `apply_all_params`'s. The single-shot bulk read stays available.
//! - **Nothing is retried or queued.** One call is one transfer; a failure is
//!   the caller's to see, as for any write on a lost link (`PLAN-CLIENTS.md` §1).

use std::collections::VecDeque;
use std::time::UNIX_EPOCH;

use dspi_proto::generated::{ALL_OPCODES, opcodes as op};
use dspi_session::notify::{Event, Notification, PROTOCOL_VERSION, evt};
use dspi_transport::TransportError;
use dspi_transport::clock::{self, Instant};

use crate::{DspiCore, DspiError, Result};

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// One IN transfer: `opcode`, `value` as `wValue`, at most `len` bytes
    /// back. The reply is returned as the device sent it, so it may be
    /// shorter than `len`, and a status byte in it is not interpreted.
    ///
    /// An opcode that is not a plain read (`REQ_PRESET_LOAD`,
    /// `REQ_RTA_CONTROL`, ...) is a write-as-read and is treated as a write.
    /// A plain read is any `*_GET_*` opcode, `REQ_EXTENSION`, `REQ_LIMITER`,
    /// `REQ_OUTPUT_LATENCY`, and the §1.2 exceptions `REQ_CLEAR_CLIPS` and
    /// `REQ_RESET_BUFFER_STATS` (`REQ_ENTER_BOOTLOADER`, the third, is refused
    /// here since WP P1TZF: it takes a token, through
    /// `enter_bootloader_confirmed`).
    ///
    /// The mirror does not change here. A device change reaches it through
    /// the notification, like anyone else's. Over DSPi Link the
    /// `BULK_INVALIDATED` of a command of ours counts as our own echo and
    /// does not re-read, so after a raw command that replaces many settings
    /// (a preset load) call [`refresh_params`](Self::refresh_params) there.
    ///
    /// Blocks for the transfer with the core's lock held: call it off the UI
    /// thread. `REQ_GET_ALL_PARAMS` for a whole image is the slow case.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for `REQ_SET_ALL_PARAMS` and
    /// the chunk opcodes, and for an opcode behind a confirmation token
    /// ([`raw_gated_call`]: preset save and delete, parameter and output
    /// configuration save, factory reset, the bootloader), naming the
    /// `_confirmed` call to use (nothing is sent; WP P1TZF);
    /// [`DspiError::Disconnected`];
    /// [`DspiError::ReadOnly`] for a write-as-read on a read-only connection
    /// (nothing is sent); [`DspiError::Refused`] with status 0 when the
    /// device stalls; [`DspiError::Denied`], [`DspiError::Locked`] or
    /// [`DspiError::Transfer`] from the link.
    pub fn raw_get(&self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        check_not_bulk_session(opcode)?;
        check_not_gated(opcode)?; // -- WP P1TZF --
        let mut guard = self.lock();
        // -- WP ROMODE -- the client read-only mode: the registry's pure reads only
        if guard.conn.is_some() {
            crate::read_only::check_raw_get(&mut guard, opcode)?;
        }
        if is_plain_read(opcode) {
            guard
                .conn()?
                .transport()
                .control_in_upto(opcode, value, len)
                .map_err(refusal)
        } else {
            // -- WP TMCORE -- as in `raw_set`
            let _analyser = is_analyser(opcode).then(crate::limits::LockScope::allow);
            // A write-as-read opcode on the Writable (§1.2); raw for any
            // reply length, and so a stall stays a refusal.
            let mut w = guard.writable_conn()?;
            let conn = w.conn();
            conn.fresh_image = false;
            conn.transport()
                .control_in_upto(opcode, value, len)
                .map_err(refusal)
        }
    }

    /// One OUT transfer: `opcode`, `value` as `wValue`, `data` as the
    /// payload. Success means the device took the transfer, not that it
    /// applied the value; the firmware ignores some invalid ones.
    ///
    /// The mirror follows as for [`raw_get`](Self::raw_get). Blocks for the
    /// transfer with the core's lock held: call it off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for `REQ_SET_ALL_PARAMS`, the
    /// chunk opcodes, an opcode behind a confirmation token ([`raw_gated_call`],
    /// as for `raw_get`), an `EXT_CORE_VREG` request, or more than 65535
    /// bytes of `data` (nothing is sent);
    /// [`DspiError::Disconnected`]; [`DspiError::ReadOnly`] on a read-only
    /// connection (nothing is sent); [`DspiError::Refused`] with status 0
    /// when the device stalls; [`DspiError::Denied`], [`DspiError::Locked`]
    /// or [`DspiError::Transfer`] from the link.
    pub fn raw_set(&self, opcode: u8, value: u16, data: Vec<u8>) -> Result<()> {
        check_not_bulk_session(opcode)?;
        check_not_gated(opcode)?; // -- WP P1TZF --
        // -- WP CG14 --
        // It could not carry the confirmation word: `core_vreg_op` can.
        if dspi_session::overvolt::is_core_vreg_write(opcode, value) {
            return Err(DspiError::InvalidArgument {
                msg: "core-voltage requests go through core_vreg_op_confirmed, not a raw write"
                    .into(),
            });
        }
        if u16::try_from(data.len()).is_err() {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "{} bytes for opcode 0x{opcode:02X} exceed a vendor request",
                    data.len()
                ),
            });
        }
        // -- WP TMCORE -- the analyser's requests pass the locked-user state,
        // as the core's own analyser calls do (P1-D: viewing is not a write)
        let _analyser = is_analyser(opcode).then(crate::limits::LockScope::allow);
        let mut guard = self.lock();
        // -- WP ROMODE -- the client read-only mode: no raw write at all
        if guard.conn.is_some() {
            crate::read_only::check_raw_set(&mut guard)?;
        }
        let mut w = guard.writable_conn()?;
        // Raw on the Writable, so a stall stays a refusal.
        let conn = w.conn();
        conn.fresh_image = false;
        // -- WP CG21 --
        if opcode == op::REQ_OUTPUT_LATENCY {
            conn.ring_target.invalidate();
        }
        conn.transport()
            .control_out(opcode, value, &data)
            .map_err(refusal)?;
        // -- WP P0W -- history only: nobody knows what a raw write moved
        let [lo, hi] = value.to_le_bytes();
        let after = dspi_proto::value::Value::Bytes(data);
        crate::journal::journal(&mut guard, "raw", &[opcode, lo, hi], after);
        Ok(())
    }
}

// -- WP CG23 --
// The raw notification tap and the connection generation, for a binding that
// still hands its own code the packets (`docs/SWIFT-SEAMS.md` §6: the Swift
// console's `InterruptMonitor` and `onX` hooks, until SW14 moves them to
// `monitor_read`).

/// One notification as the tap saw it (WP CG23).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawNotification {
    /// The packet in the firmware's v2 layout (`notify.h`), or the 8-byte v1
    /// volume packet. Rebuilt from the decoded event, with the flags and the
    /// padding zero as the firmware sends them: byte for byte what the
    /// device sent for every event this build knows. A packet the reader
    /// could not decode is kept as it came.
    pub packet: Vec<u8>,
    /// The link's attribution: the DSPi Link session id of the writer, 0
    /// for the device itself; `None` over USB and MIDI.
    pub origin: Option<u16>,
    /// The echo filter's verdict: the echo of a write of this core's own.
    /// Only a `PARAM_CHANGED` or a `BULK_INVALIDATED` can be one; every
    /// other event is `false`.
    pub own: bool,
    /// Something before this entry is missing: the tap's ring overflowed,
    /// or the reader saw a gap in the device's sequence numbers.
    pub lost: bool,
    /// When the reader received it, ms since the Unix epoch.
    pub received_ms: i64,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Start or stop the notification tap. While on, every notification
    /// [`process_notifications`](Self::process_notifications) drains is also
    /// kept for [`notify_tap_drain`](Self::notify_tap_drain), in the order
    /// the mirror applied them. Off by default; turning it off forgets what
    /// it held. Never fails.
    pub fn notify_tap_set(&self, enabled: bool) {
        self.lock().notify_tap.set(enabled);
    }

    /// The notifications kept since the last drain, oldest first. The tap
    /// keeps the last 512; when older ones were dropped, the oldest returned
    /// has `lost` set. Empty while the tap is off. Never fails.
    pub fn notify_tap_drain(&self) -> Vec<RawNotification> {
        self.lock().notify_tap.drain()
    }

    /// The open connection's generation: unique per connection and never
    /// reused while the process runs, so it changes on every reopen and on
    /// a Link re-bind. 0 when nothing is open.
    pub fn connection_generation(&self) -> u64 {
        self.lock().conn.as_ref().map_or(0, |c| c.generation)
    }
}

impl DspiCore {
    // -- WP TMCORE --
    /// The device's bulk image as it sends it (any wire format), read the
    /// way this connection reads it (chunked over USB, single-shot over DSPi
    /// Link and MIDI), with the core's lock held for the whole read: for a
    /// client whose own code reads the image through the core's link (the
    /// Terminal's `Session` over the core), since the chunk opcodes are not
    /// [`raw_get`](Self::raw_get)'s. The mirror does not change. Not
    /// exported to UniFFI.
    ///
    /// Errors: [`DspiError::Disconnected`]; the transfer's.
    #[doc(hidden)]
    pub fn read_image_bytes(&self) -> Result<Vec<u8>> {
        let mut guard = self.lock();
        let conn = guard.conn()?;
        dspi_session::compat::read_bulk_bytes(conn.transport()).map_err(DspiError::from)
    }

    /// Set `lost` on the next entry the tap keeps: a binding that drained
    /// entries and could not deliver them (the C ABI's too-small buffer)
    /// says so. Not exported to UniFFI.
    #[doc(hidden)]
    pub fn notify_tap_note_dropped(&self) {
        self.lock().notify_tap.dropped = true;
    }
}

/// The tap's ring (WP CG23), behind the core lock.
#[derive(Default)]
pub(crate) struct NotifyTap {
    on: bool,
    ring: VecDeque<RawNotification>,
    /// Entries were dropped since the last one kept.
    dropped: bool,
}

impl NotifyTap {
    /// The most entries kept; the oldest go first.
    pub(crate) const LIMIT: usize = 512;

    fn set(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.ring = VecDeque::new();
            self.dropped = false;
        }
    }

    fn drain(&mut self) -> Vec<RawNotification> {
        self.ring.drain(..).collect()
    }

    /// Keep `note`, with the echo filter's verdict `own`. Nothing when off.
    pub(crate) fn record(&mut self, note: &Notification, own: bool) {
        if !self.on {
            return;
        }
        if self.ring.len() >= Self::LIMIT {
            self.ring.pop_front();
            if let Some(first) = self.ring.front_mut() {
                first.lost = true;
            }
        }
        let lost = note.lost || std::mem::take(&mut self.dropped);
        self.ring.push_back(RawNotification {
            packet: packet(note),
            origin: note.origin,
            own,
            lost,
            received_ms: epoch_ms(note.received),
        });
    }
}

/// `note`'s packet in the firmware's layout (`notify.h:24-112`).
fn packet(note: &Notification) -> Vec<u8> {
    let v2 = |id: u8, body: &[u8]| {
        let mut p = vec![PROTOCOL_VERSION, id, 0, note.seq];
        p.extend_from_slice(body);
        p
    };
    match &note.event {
        Event::Idle => vec![evt::IDLE],
        Event::MasterVolume(db) => {
            let mut p = vec![evt::MASTER_VOLUME, 0, 0, 0];
            p.extend_from_slice(&db.to_le_bytes());
            p
        }
        Event::ParamChanged {
            offset,
            source,
            bytes,
        } => {
            // The decoder took `bytes` from the packet's own size field.
            let size = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
            let mut body = Vec::with_capacity(bytes.len().saturating_add(8));
            body.extend_from_slice(&offset.to_le_bytes());
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&[source.raw(), 0, 0, 0]);
            body.extend_from_slice(bytes);
            v2(evt::PARAM_CHANGED, &body)
        }
        Event::BulkInvalidated { source } => v2(evt::BULK_INVALIDATED, &[source.raw(), 0, 0, 0]),
        Event::PresetLoaded { slot } => v2(evt::PRESET_LOADED, &[*slot, 0, 0, 0]),
        Event::InputFormat { channels } => v2(evt::INPUT_FORMAT, &[*channels, 0, 0, 0]),
        Event::SiggenState {
            state,
            reason,
            signal_type,
            channel,
        } => v2(
            evt::SIGGEN_STATE,
            &[*state, *reason, *signal_type, *channel],
        ),
        Event::AdatState {
            enabled,
            active,
            pin,
        } => v2(
            evt::ADAT_STATE,
            &[u8::from(*enabled), u8::from(*active), *pin, 0],
        ),
        Event::I2sSlaveState { state, rate_hz } => {
            let r = rate_hz.to_le_bytes();
            v2(evt::I2S_SLAVE_STATE, &[*state, r[0], r[1], r[2], r[3]])
        }
        Event::IrLearn {
            state,
            protocol,
            code,
        } => {
            let c = code.to_le_bytes();
            v2(
                evt::CS_IR_LEARN,
                &[*state, *protocol, 0, 0, c[0], c[1], c[2], c[3]],
            )
        }
        Event::AdatInputState {
            state,
            rate_hz,
            clock_mode,
        } => {
            let r = rate_hz.to_le_bytes();
            v2(
                evt::ADAT_INPUT_STATE,
                &[*state, r[0], r[1], r[2], r[3], *clock_mode],
            )
        }
        Event::CsAux {
            slot,
            state,
            level_q8,
            source,
        } => {
            let l = level_q8.to_le_bytes();
            v2(evt::CS_AUX, &[*slot, *state, l[0], l[1], source.raw()])
        }
        // The reader's stand-in for a packet it could not decode holds that
        // packet whole (`dspi_session::notify::Notifications::start`).
        Event::Unknown { id: 0xFF, bytes } if note.lost && note.seq == 0 => bytes.clone(),
        Event::Unknown { id, bytes } => v2(*id, bytes),
    }
}

/// `at` as ms since the Unix epoch (0 if the clock is before it), as the
/// monitor stamps its lines.
pub(crate) fn epoch_ms(at: Instant) -> i64 {
    let age = Instant::now().saturating_duration_since(at);
    clock::system_time()
        .checked_sub(age)
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// [`DspiError::InvalidArgument`] for the opcodes only the core may send:
/// the chunk opcodes, because a chunked session spans transfers and any
/// other request aborts it on the device, and the single-shot bulk write,
/// because `apply_all_params` owns its checks, device lock and echo bracket.
// -- WP TMCORE --
/// The analyser's own requests (its configuration push and its start and
/// stop), which the locked-user state lets through.
fn is_analyser(opcode: u8) -> bool {
    matches!(opcode, op::REQ_RTA_SET_CONFIG | op::REQ_RTA_CONTROL)
}

fn check_not_bulk_session(opcode: u8) -> Result<()> {
    if matches!(
        opcode,
        op::REQ_SET_ALL_PARAMS | op::REQ_GET_ALL_PARAMS_CHUNK | op::REQ_SET_ALL_PARAMS_CHUNK
    ) {
        return Err(DspiError::InvalidArgument {
            msg: format!("opcode 0x{opcode:02X} is a bulk transfer the core runs itself"),
        });
    }
    Ok(())
}

// -- WP P1TZF --
/// The typed call that takes a confirmation token for `opcode`, when the
/// opcode is one of the dangerous operations (`crate::confirm`): those never
/// go out through a raw request, which carries no token. Public (not in the
/// bindings) for the Terminal's `raw`, which also opens devices without a
/// core.
pub fn raw_gated_call(opcode: u8) -> Option<&'static str> {
    match opcode {
        op::REQ_PRESET_SAVE => Some("save_preset_confirmed (or copy_preset_confirmed)"),
        op::REQ_PRESET_DELETE => Some("delete_preset_confirmed"),
        op::REQ_SAVE_PARAMS => Some("save_params_confirmed"),
        op::REQ_SAVE_OUTPUT_CONFIG => Some("save_output_config_confirmed"),
        op::REQ_FACTORY_RESET => Some("factory_reset_confirmed"),
        op::REQ_ENTER_BOOTLOADER => Some("enter_bootloader_confirmed"),
        _ => None,
    }
}

/// [`DspiError::InvalidArgument`], with nothing sent, for an opcode behind a
/// confirmation token: the refusal names the `_confirmed` call to use.
fn check_not_gated(opcode: u8) -> Result<()> {
    match raw_gated_call(opcode) {
        Some(call) => Err(DspiError::InvalidArgument {
            msg: format!(
                "opcode 0x{opcode:02X} needs a confirmation token: use {call}, not a raw request"
            ),
        }),
        None => Ok(()),
    }
}

/// Whether an IN transfer of `opcode` may go out on a read-only connection.
///
/// Checked against the firmware's IN handlers (`vendor_commands.c`):
/// `REQ_EXTENSION`, `REQ_LIMITER` and `REQ_OUTPUT_LATENCY` have no `GET` in
/// their names because they share one opcode with their OUT write, and their
/// IN side only reads. The last three are the named exceptions of §1.2.
/// Everything else without `_GET_` in its name, or not in this build's table,
/// counts as a write.
fn is_plain_read(opcode: u8) -> bool {
    matches!(
        opcode,
        op::REQ_EXTENSION
            | op::REQ_LIMITER
            | op::REQ_OUTPUT_LATENCY
            | op::REQ_CLEAR_CLIPS
            | op::REQ_RESET_BUFFER_STATS
            | op::REQ_ENTER_BOOTLOADER
    ) || ALL_OPCODES
        .iter()
        .any(|(name, code)| *code == opcode && name.contains("_GET_"))
}

/// A transfer's failure: a stall is the device refusing, reported as
/// [`DspiError::Refused`] with status 0 (there is no status byte), so a
/// binding can tell it from a lost link or a timeout.
fn refusal(e: TransportError) -> DspiError {
    match e {
        TransportError::Stalled { .. } => DspiError::Refused { status: 0 },
        other => other.into(),
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg4 {
    use std::sync::atomic::{AtomicU16, Ordering};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use dspi_proto::generated::BULK_SIZE;
    use dspi_proto::wire_compat::{self, WireRead};
    use dspi_transport::mock::{LogHandle, Reply};
    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, FakeOptions, MockTransport, NotificationSource,
        Transport, TransportCaps,
    };

    use super::*;

    const REFUSED_BULK: [u8; 3] = [
        op::REQ_SET_ALL_PARAMS,
        op::REQ_GET_ALL_PARAMS_CHUNK,
        op::REQ_SET_ALL_PARAMS_CHUNK,
    ];

    /// A core on `t`; `writable` as if it had read a V32 image.
    fn core_on(t: MockTransport, writable: bool) -> (Arc<DspiCore>, LogHandle) {
        let log = t.log_handle();
        let core = DspiCore::new();
        core.attach(Box::new(t), "MOCK".into(), 0);
        if writable {
            core.lock().conn().unwrap().wire = Some(WireRead::Exact);
        }
        (core, log)
    }

    fn opcodes(log: &LogHandle) -> Vec<u8> {
        log.lock().unwrap().iter().map(|e| e.opcode).collect()
    }

    fn volume(db: f32) -> Vec<u8> {
        db.to_le_bytes().to_vec()
    }

    #[test]
    fn wp_cg4_without_a_device_both_are_disconnected() {
        let core = DspiCore::new();
        assert_eq!(
            core.raw_get(op::REQ_GET_PLATFORM, 0, 7),
            Err(DspiError::Disconnected)
        );
        assert_eq!(
            core.raw_get(op::REQ_PRESET_LOAD, 1, 1),
            Err(DspiError::Disconnected)
        );
        assert_eq!(
            core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, volume(-6.0)),
            Err(DspiError::Disconnected)
        );
    }

    #[test]
    fn wp_cg4_read_only_refuses_every_write_with_no_traffic() {
        let (core, log) = core_on(MockTransport::new().answering_everything(vec![1u8]), false);
        assert!(matches!(
            core.lock().writable_conn(),
            Err(DspiError::ReadOnly)
        ));
        for (name, code) in ALL_OPCODES {
            // -- WP P1TZF -- the gated opcodes are refused before the connection
            if REFUSED_BULK.contains(&code) || raw_gated_call(code).is_some() {
                continue;
            }
            assert_eq!(
                core.raw_set(code, 0, vec![1]),
                Err(DspiError::ReadOnly),
                "raw_set {name}"
            );
        }
        assert_eq!(opcodes(&log), [] as [u8; 0]);

        // The write-as-read opcodes the firmware dispatches on the IN path,
        // by name, so the name rule cannot quietly let one through.
        for code in [
            op::REQ_SAVE_PARAMS,
            op::REQ_SAVE_OUTPUT_CONFIG,
            op::REQ_SAVE_MASTER_VOLUME,
            op::REQ_FACTORY_RESET,
            op::REQ_PRESET_SAVE,
            op::REQ_PRESET_LOAD,
            op::REQ_PRESET_DELETE,
            op::REQ_CS_SAVE,
            op::REQ_CS_REVERT,
            op::REQ_CS_MACRO_FIRE,
            op::REQ_CS_IR_LEARN,
            op::REQ_RTA_CONTROL,
            op::REQ_SIGGEN_CONTROL,
            op::REQ_TEST_DAC_HW_MUTE,
            op::REQ_RESET_USB_ERROR_STATS,
            op::REQ_SET_SPDIF_INPUT_ENABLE,
            // No such opcode in this build: fails closed.
            0x01,
        ] {
            // -- WP P1TZF -- a gated opcode is refused as such, before the connection
            let refused = core.raw_get(code, 0, 1);
            if raw_gated_call(code).is_some() {
                assert!(
                    matches!(refused, Err(DspiError::InvalidArgument { .. })),
                    "raw_get 0x{code:02X}: {refused:?}"
                );
            } else {
                assert_eq!(refused, Err(DspiError::ReadOnly), "raw_get 0x{code:02X}");
            }
        }
        assert_eq!(opcodes(&log), [] as [u8; 0]);
    }

    #[test]
    fn wp_cg4_read_only_classifies_the_whole_opcode_table() {
        let named_reads = [
            op::REQ_EXTENSION,
            op::REQ_LIMITER,
            op::REQ_OUTPUT_LATENCY,
            op::REQ_CLEAR_CLIPS,
            op::REQ_RESET_BUFFER_STATS,
            op::REQ_ENTER_BOOTLOADER,
        ];
        for (name, code) in ALL_OPCODES {
            // -- WP P1TZF -- the gated opcodes are refused before this (tests_wp_p1tzf)
            if REFUSED_BULK.contains(&code) || raw_gated_call(code).is_some() {
                continue;
            }
            let (core, log) = core_on(MockTransport::new().answering_everything(vec![1u8]), false);
            let got = core.raw_get(code, 0, 1);
            if name.contains("_GET_") || named_reads.contains(&code) {
                assert_eq!(got, Ok(vec![1]), "{name} is a read");
                assert_eq!(opcodes(&log), [code], "{name}");
            } else {
                assert_eq!(got, Err(DspiError::ReadOnly), "{name} is a write");
                assert_eq!(opcodes(&log), [] as [u8; 0], "{name}");
            }
        }
    }

    #[test]
    fn wp_cg4_an_older_image_is_read_only_through_raw_too() {
        let size = wire_compat::bulk_size(31).unwrap();
        let mut bulk = Capture::demo().bulk;
        bulk.resize(size, 0);
        bulk[0] = 31;
        bulk[6..8].copy_from_slice(&(size as u16).to_le_bytes());
        let cap = Capture {
            platform: Capture::demo().platform,
            bulk,
        };
        let dev = FakeDevice::new(cap, FakeOptions::default());
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "OLD".into(), 0).unwrap();
        let before = remote.image();

        assert_eq!(
            core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, volume(-6.0)),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(
            core.raw_get(op::REQ_PRESET_LOAD, 1, 1),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(remote.image(), before);
        assert!(!core.raw_get(op::REQ_GET_PLATFORM, 0, 7).unwrap().is_empty());
    }

    #[test]
    fn wp_cg4_writes_go_out_on_a_writable_connection() {
        let (core, log) = core_on(MockTransport::new().answering_everything(vec![0u8]), true);
        core.raw_set(op::REQ_SET_MASTER_VOLUME, 0x1234, volume(-6.0))
            .unwrap();
        assert_eq!(core.raw_get(op::REQ_PRESET_LOAD, 2, 1), Ok(vec![0]));

        let log = log.lock().unwrap();
        assert_eq!(log[0].opcode, op::REQ_SET_MASTER_VOLUME);
        assert_eq!(log[0].value, 0x1234);
        assert_eq!(log[0].payload, volume(-6.0));
        assert_eq!((log[1].opcode, log[1].value), (op::REQ_PRESET_LOAD, 2));
    }

    #[test]
    fn wp_cg4_a_write_makes_the_connect_image_stale() {
        for write_as_read in [false, true] {
            let (core, _) = core_on(MockTransport::new().answering_everything(vec![0u8]), true);
            core.lock().conn().unwrap().fresh_image = true;
            core.raw_get(op::REQ_GET_PLATFORM, 0, 1).unwrap();
            assert!(core.lock().conn().unwrap().fresh_image, "a read keeps it");
            if write_as_read {
                core.raw_get(op::REQ_PRESET_LOAD, 1, 1).unwrap();
            } else {
                core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, volume(-6.0))
                    .unwrap();
            }
            assert!(!core.lock().conn().unwrap().fresh_image);
        }
    }

    #[test]
    fn wp_cg4_bulk_session_opcodes_are_refused_with_no_traffic() {
        for writable in [false, true] {
            let (core, log) = core_on(
                MockTransport::new().answering_everything(vec![1u8]),
                writable,
            );
            for code in REFUSED_BULK {
                assert!(matches!(
                    core.raw_get(code, 0, 64),
                    Err(DspiError::InvalidArgument { .. })
                ));
                assert!(matches!(
                    core.raw_set(code, 0, vec![0; 64]),
                    Err(DspiError::InvalidArgument { .. })
                ));
            }
            assert_eq!(opcodes(&log), [] as [u8; 0]);
        }
    }

    #[test]
    fn wp_cg4_the_single_shot_bulk_read_stays_available() {
        // Commands.swift:3715 reads the image this way until SW8.
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let image = core
            .raw_get(op::REQ_GET_ALL_PARAMS, 0, BULK_SIZE as u16)
            .unwrap();
        assert_eq!(image, core.get_state().bulk_raw);
    }

    /// [`Dawdling::armed`] before the test arms it: no opcode.
    const DISARMED: u16 = 0x100;

    /// A fake device that logs each transfer and, while armed for an opcode,
    /// lets the test know its first transfer began and then dawdles on each
    /// one, so a racing raw call has every chance to get in between.
    struct Dawdling {
        dev: FakeDevice,
        log: Arc<Mutex<Vec<u8>>>,
        /// The armed opcode, or [`DISARMED`] (no opcode is 0x100).
        armed: Arc<AtomicU16>,
        started: Sender<()>,
    }

    impl Dawdling {
        fn note(&self, opcode: u8) {
            self.log.lock().unwrap().push(opcode);
            if self.armed.load(Ordering::SeqCst) == u16::from(opcode) {
                let _ = self.started.send(());
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    impl Transport for Dawdling {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.note(opcode);
            self.dev.control_in(opcode, value, len)
        }

        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            max_len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.note(opcode);
            self.dev.control_in_upto(opcode, value, max_len)
        }

        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            self.note(opcode);
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

    /// Raw calls racing a chunked session of `chunk` run by `session` wait
    /// for its end: no other opcode between its first and last chunk, and
    /// both raw calls did reach the device afterwards.
    fn raw_calls_wait_for(chunk: u8, session: impl FnOnce(&DspiCore) -> Result<()>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        // -- WP FLAKE3 -- disarmed until the session: armed at 0 matched the connect's
        // extension-channel requests (opcode 0x00), whose signals sat in the channel and
        // let the racer go before the session had started
        let armed = Arc::new(AtomicU16::new(DISARMED));
        let (started, wait_started) = channel();
        let dev = Dawdling {
            dev: FakeDevice::demo(),
            log: Arc::clone(&log),
            armed: Arc::clone(&armed),
            started,
        };
        assert!(dev.caps().chunked_bulk);
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "FAKE".into(), 0).unwrap();
        // The connect's own image is spent: the session below really reads.
        core.lock().conn().unwrap().fresh_image = false;
        log.lock().unwrap().clear();
        armed.store(u16::from(chunk), Ordering::SeqCst);

        let racer = {
            let core = Arc::clone(&core);
            std::thread::spawn(move || {
                wait_started
                    .recv_timeout(Duration::from_secs(10))
                    .expect("the session started");
                let read = core.raw_get(op::REQ_GET_PLATFORM, 0, 7).map(|_| ());
                let write = core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, volume(-6.0));
                (read, write)
            })
        };
        session(&core).unwrap();
        let (read, write) = racer.join().unwrap();
        read.unwrap();
        write.unwrap();

        let log = log.lock().unwrap();
        let first = log.iter().position(|o| *o == chunk).unwrap();
        let last = log.iter().rposition(|o| *o == chunk).unwrap();
        assert!(last > first, "a session of several chunks: {log:02X?}");
        assert!(
            log[first..=last].iter().all(|o| *o == chunk),
            "another request inside the session: {log:02X?}"
        );
        for raw in [op::REQ_GET_PLATFORM, op::REQ_SET_MASTER_VOLUME] {
            let at = log.iter().rposition(|o| *o == raw).unwrap();
            assert!(at > last, "0x{raw:02X} reached the device: {log:02X?}");
        }
    }

    #[test]
    fn wp_cg4_raw_calls_cannot_land_inside_a_chunked_write() {
        raw_calls_wait_for(op::REQ_SET_ALL_PARAMS_CHUNK, |core| core.apply_all_params());
    }

    #[test]
    fn wp_cg4_raw_calls_cannot_land_inside_a_chunked_read() {
        raw_calls_wait_for(op::REQ_GET_ALL_PARAMS_CHUNK, |core| core.fetch_all());
    }

    #[test]
    fn wp_cg4_a_short_reply_is_the_answer() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let reply = core.raw_get(op::REQ_GET_PLATFORM, 0, 512).unwrap();
        assert!(!reply.is_empty() && reply.len() < 512, "{}", reply.len());
    }

    #[test]
    fn wp_cg4_oversized_data_is_refused_with_no_traffic() {
        let (core, log) = core_on(MockTransport::new().answering_everything(vec![0u8]), true);
        assert!(matches!(
            core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, vec![0; 65536]),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert_eq!(opcodes(&log), [] as [u8; 0]);
    }

    #[test]
    fn wp_cg4_a_stall_is_a_refusal_and_is_not_retried() {
        let t = MockTransport::new()
            .reply(op::REQ_SET_MASTER_VOLUME, Reply::Stall)
            .reply(op::REQ_GET_PLATFORM, Reply::Stall)
            .reply(op::REQ_PRESET_LOAD, Reply::Stall);
        let (core, log) = core_on(t, true);
        let refused = Err(DspiError::Refused { status: 0 });
        assert_eq!(
            core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, volume(-6.0)),
            refused.clone().map(|_: ()| ())
        );
        assert_eq!(
            core.raw_get(op::REQ_GET_PLATFORM, 0, 7),
            refused.clone().map(|_: ()| vec![])
        );
        assert_eq!(
            core.raw_get(op::REQ_PRESET_LOAD, 1, 1),
            refused.map(|_: ()| vec![])
        );
        assert_eq!(
            opcodes(&log),
            [
                op::REQ_SET_MASTER_VOLUME,
                op::REQ_GET_PLATFORM,
                op::REQ_PRESET_LOAD
            ]
        );
    }

    #[test]
    fn wp_cg4_the_mirror_follows_a_raw_write_through_its_echo() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core.raw_set(op::REQ_SET_MASTER_VOLUME, 0, volume(-17.0))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while core.get_state().master_volume_db != -17.0 {
            assert!(
                Instant::now() < deadline,
                "the echo never reached the mirror"
            );
            core.process_notifications();
            std::thread::sleep(Duration::from_millis(5));
        }
        // And the device agrees.
        let read = core.raw_get(op::REQ_GET_MASTER_VOLUME, 0, 4).unwrap();
        assert_eq!(read, volume(-17.0));
    }
}

// -- WP CG23 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "raw/tests_wp_cg23.rs"]
mod tests_wp_cg23;

// -- WP P1TZF --
#[cfg(test)]
#[path = "raw/tests_wp_p1tzf.rs"]
mod tests_wp_p1tzf;
