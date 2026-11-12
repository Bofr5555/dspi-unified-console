//! The bulk-SET gain repair's I/O (WP BULKFIX), shared by the core's
//! `apply_all_params` and the hub's relay of a client's bulk SET.
//!
//! The plan and the reason are in [`dspi_proto::bulk_gain`]. A caller:
//! 1. asks [`fix_reported`] (or [`fix_reported_in`] with the probed
//!    `EXT_LIST`); with the fix, it sends the image as it is;
//! 2. otherwise sends [`GainRepair::muted`] with its own bulk writer, then
//!    calls [`repair`].
//!
//! Every step fails safe: if anything goes wrong after the muted image was
//! sent, the outputs stay muted and the caller gets the error.

use std::time::Duration;

use dspi_proto::bulk_gain::{EXT_FIRMWARE_FIXES, FIX_BULK_DB_POWF, GainRepair, outputs_muted};
use dspi_proto::generated::opcodes as op;
use dspi_transport::clock::{self, Instant};
use dspi_transport::{Transport, TransportError};

use crate::WriteError;

/// How long the device may take to apply a bulk SET it has received. The
/// main loop waits for the pipeline to be ready (a mute hold of the DAC
/// mute, typically well under a second) before `bulk_params_apply`.
const APPLY_WAIT: Duration = Duration::from_secs(3);
/// Between two read-backs refused because the apply is still pending.
const APPLY_POLL: Duration = Duration::from_millis(50);

/// The `EXT_FIRMWARE_FIXES` word, `None` when the firmware does not answer
/// it (stock firmware, or a fork build without FW2).
pub fn read_fixes(t: &mut dyn Transport) -> Option<u32> {
    let b = t
        .control_in(op::REQ_EXTENSION, EXT_FIRMWARE_FIXES, 4)
        .ok()?;
    let v: [u8; 4] = b.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(v))
}

/// Whether the firmware reports that its bulk SET converts dB right, given
/// the sub-commands its `EXT_LIST` named (the connect probe's). No traffic
/// unless `EXT_FIRMWARE_FIXES` is listed. Any failure reads as "not fixed".
pub fn fix_reported_in(t: &mut dyn Transport, supported: &[u16]) -> bool {
    supported.contains(&EXT_FIRMWARE_FIXES)
        && read_fixes(t).is_some_and(|f| f & FIX_BULK_DB_POWF != 0)
}

/// [`fix_reported_in`], reading `EXT_LIST` first (one transfer on stock
/// firmware, which stalls it).
pub fn fix_reported(t: &mut dyn Transport) -> bool {
    match crate::ext::list(t) {
        Ok(ids) => fix_reported_in(t, &ids),
        Err(_) => false,
    }
}

/// After [`GainRepair::muted`] was sent: wait until the device has applied it,
/// check it did, then send the gain SETs and, last, the unmutes.
///
/// The wait is a bulk read: the firmware refuses one (USB STALL, or
/// `BULK_LOCKED`) while a received bulk SET waits for its apply, so a read
/// that succeeds comes after `bulk_params_apply`. Single SETs sent earlier
/// would be overwritten by the apply.
///
/// Errors: the transport's error; [`TransportError::Malformed`] when the
/// read-back does not have the outputs muted (the device did not apply this
/// image), in which case nothing more is sent.
pub fn repair(t: &mut dyn Transport, r: &GainRepair) -> Result<(), TransportError> {
    let back = read_back_applied(t)?;
    if !outputs_muted(&back, r.outputs) {
        return Err(TransportError::Malformed(
            "the device did not confirm the bulk write: its outputs are not muted on read-back"
                .into(),
        ));
    }
    for w in r.gains.iter().chain(&r.unmutes) {
        t.control_out(w.opcode, w.value, &w.payload)?;
    }
    Ok(())
}

/// The image, read once the device takes bulk reads again.
fn read_back_applied(t: &mut dyn Transport) -> Result<Vec<u8>, TransportError> {
    let start = Instant::now();
    loop {
        let err = match crate::compat::read_bulk_bytes(t) {
            Ok(b) => return Ok(b),
            Err(WriteError::Transport(e)) => e,
            Err(e) => return Err(TransportError::Malformed(e.to_string())),
        };
        let pending = matches!(
            err,
            TransportError::Stalled { .. }
                | TransportError::Locked
                | TransportError::TimedOut { retries: 0, .. }
        );
        if !pending || start.elapsed() >= APPLY_WAIT {
            return Err(err);
        }
        clock::sleep(APPLY_POLL);
    }
}
