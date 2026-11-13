//! The bulk-SET gain repair's I/O (WP BULKFIX), shared by the core's
//! `apply_all_params` and the hub's relay of a client's bulk SET.
//!
//! The plan and the reason are in [`dspi_proto::bulk_gain`]. A caller:
//! 1. asks [`fix_reported`] (or [`fix_reported_in`] with the probed
//!    `EXT_LIST`); with the fix, it sends the image as it is;
//! 2. otherwise sends [`GainRepair::muted`] with its own bulk writer, then
//!    calls [`repair`] (or its steps: [`wait_applied`], [`check_readback`],
//!    the gains, [`GainGuard::clear_all`], the unmutes).
//!
//! A connection's transport is wrapped by [`GainGuard::wrap`], so an unmute by
//! any path after a failed repair re-sends the gains first (fixup bf F1).
//!
//! Every step fails safe: if anything goes wrong after the muted image was
//! sent, the outputs stay muted and the caller gets the error.

use std::time::Duration;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dspi_proto::bulk_gain::{
    EXT_FIRMWARE_FIXES, FIX_BULK_DB_POWF, GainRepair, muted_mask, output_gain_writes,
    readback_matches,
};
use dspi_proto::generated::opcodes as op;
use dspi_transport::clock::{self, Instant};
use dspi_transport::{
    DeviceDescriptor, InRead, NotificationSource, Transport, TransportCaps, TransportError,
};

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
/// check it did, then send the gain SETs and, last, the unmutes. With a
/// `guard`, every output is suspect if anything fails before the gains are
/// all re-sent, and none is once they are ([`GainGuard`]).
///
/// The wait is a bulk read: the firmware refuses one (USB STALL, or
/// `BULK_LOCKED`) while a received bulk SET waits for its apply, so a read
/// that succeeds comes after `bulk_params_apply`. Single SETs sent earlier
/// would be overwritten by the apply.
///
/// Errors: the transport's error; [`TransportError::Malformed`] when the
/// read-back is not the image that was sent ([`check_readback`]), in which
/// case nothing more is sent.
pub fn repair(
    t: &mut dyn Transport,
    r: &GainRepair,
    guard: Option<&GainGuard>,
) -> Result<(), TransportError> {
    let gains = (|| {
        let back = wait_applied(t)?;
        check_readback(&back, r)?;
        for w in &r.gains {
            t.control_out(w.opcode, w.value, &w.payload)?;
        }
        Ok(())
    })();
    if let Some(g) = guard {
        match &gains {
            Ok(()) => g.clear_all(),
            Err(_) => g.suspect(all_outputs(r.outputs)),
        }
    }
    gains?;
    for w in &r.unmutes {
        t.control_out(w.opcode, w.value, &w.payload)?;
    }
    Ok(())
}

// -- fixup bf (F3) --
/// [`dspi_proto::bulk_gain::readback_matches`] as an error: a read-back that is
/// not the sent image (one the firmware refused for its platform or channel
/// counts, say, so the old state is still in) stops the repair before any
/// write, so nothing is unmuted that the firmware would not have unmuted.
pub fn check_readback(back: &[u8], r: &GainRepair) -> Result<(), TransportError> {
    if readback_matches(back, r) {
        Ok(())
    } else {
        Err(TransportError::Malformed(
            "the device did not confirm the bulk write: its read-back differs from the image \
             sent, so its outputs were left as they are"
                .into(),
        ))
    }
}

/// Bits `0..outputs` (outputs past 32 are not tracked; the wire has 9).
pub fn all_outputs(outputs: usize) -> u32 {
    match outputs {
        0 => 0,
        n if n >= 32 => u32::MAX,
        n => u32::MAX >> (32usize.saturating_sub(n)),
    }
}

/// The image, read once the device takes bulk reads again (the barrier of
/// [`repair`]).
pub fn wait_applied(t: &mut dyn Transport) -> Result<Vec<u8>, TransportError> {
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

// -- fixup bf (F1, F4) --
/// Re-send every gain feeding output `o` as the device stores it (read back),
/// through the single SETs that convert with `powf`.
pub fn reconvert_output(t: &mut dyn Transport, o: usize) -> Result<(), TransportError> {
    let back = wait_applied(t)?;
    let writes = output_gain_writes(&back, o).ok_or_else(|| {
        TransportError::Malformed(format!(
            "output {} is not in the device's image",
            o.saturating_add(1)
        ))
    })?;
    for w in writes {
        t.control_out(w.opcode, w.value, &w.payload)?;
    }
    Ok(())
}

/// What one connection knows about the bulk dB bug: whether the firmware
/// reports the fix (asked once, F4), and which outputs may still hold the
/// series' gains behind their mute.
///
/// An output is suspect after a repair that failed before all its gains were
/// re-sent, and, on firmware without the fix, if it was muted when the
/// connection opened (a repair cut short by a lost connection leaves exactly
/// that). Unmuting a suspect output through [`GainGuard::wrap`]'s transport
/// first re-sends the gains feeding it ([`reconvert_output`]) and refuses the
/// unmute if that fails. A successful repair, a preset load and a factory
/// reset (both convert every gain with `powf`) clear every output.
#[derive(Clone, Default)]
pub struct GainGuard {
    state: Arc<Mutex<GuardState>>,
}

#[derive(Default)]
struct GuardState {
    fixed: Option<bool>,
    suspect: u32,
    /// Find the suspect outputs at the first unmute (the hub, which reads no
    /// image at open); else [`GainGuard::seed`] says.
    lazy: bool,
    seeded: bool,
}

impl GainGuard {
    fn state(&self) -> MutexGuard<'_, GuardState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wrap `t`: every unmute through the result is checked as above. With
    /// `lazy`, the first unmute on firmware without the fix reads the image
    /// and takes its muted outputs as suspect; without, [`GainGuard::seed`]
    /// does that from the connect's image.
    pub fn wrap(t: Box<dyn Transport>, lazy: bool) -> (Box<dyn Transport>, GainGuard) {
        let guard = GainGuard::default();
        guard.state().lazy = lazy;
        let wrapped = Guarded {
            inner: t,
            guard: guard.clone(),
        };
        (Box::new(wrapped), guard)
    }

    /// Whether the firmware reports the fix, asked once per connection: from
    /// `supported` (the connect probe's `EXT_LIST`) when given, which costs no
    /// traffic unless it names `EXT_FIRMWARE_FIXES`, else by reading
    /// `EXT_LIST`. A read that failed for another reason than "no extension
    /// channel" is "not fixed" this time and asked again next time.
    pub fn fixed(&self, t: &mut dyn Transport, supported: Option<&[u16]>) -> bool {
        if let Some(f) = self.state().fixed {
            return f;
        }
        let f = match supported {
            Some(ids) => Some(fix_reported_in(t, ids)),
            None => match crate::ext::list(t) {
                Ok(ids) => Some(fix_reported_in(t, &ids)),
                // Stock firmware: no extension channel, so no fix, for good.
                Err(crate::ext::ExtError::Unsupported { .. }) => Some(false),
                Err(_) => None,
            },
        };
        if f.is_some() {
            self.state().fixed = f;
        }
        f.unwrap_or(false)
    }

    /// The connect's word: the fix (or not), and the outputs muted in the
    /// image just read, which are suspect without the fix.
    pub fn seed(&self, fixed: bool, muted: u32) {
        let mut s = self.state();
        s.fixed = Some(fixed);
        s.seeded = true;
        if !fixed {
            s.suspect |= muted;
        }
    }

    /// Mark outputs (bits) suspect. What is known now needs no seeding.
    pub fn suspect(&self, outputs: u32) {
        let mut s = self.state();
        s.suspect |= outputs;
        s.seeded = true;
    }

    /// The suspect outputs, one bit each.
    pub fn suspects(&self) -> u32 {
        self.state().suspect
    }

    /// Every gain is known to convert right again (nothing left to seed: a
    /// repair's own unmutes must not read its muted image as suspect).
    pub fn clear_all(&self) {
        let mut s = self.state();
        s.suspect = 0;
        s.seeded = true;
    }
}

// -- WP SW2 --
/// `REQ_EXTENSION` `EXT_APPLY_AT_FRAME` OUT with op 2 `UNMUTE`.
fn is_apply_unmute(opcode: u8, value: u16, data: &[u8]) -> bool {
    use dspi_proto::ext::audio_link::{ApplyOp, EXT_APPLY_AT_FRAME};
    opcode == op::REQ_EXTENSION
        && value == EXT_APPLY_AT_FRAME
        && data.get(8) == Some(&ApplyOp::Unmute.raw())
}

/// The transport behind [`GainGuard::wrap`].
struct Guarded {
    inner: Box<dyn Transport>,
    guard: GainGuard,
}

fn bit(o: usize) -> u32 {
    u32::try_from(o)
        .ok()
        .and_then(|o| 1u32.checked_shl(o))
        .unwrap_or(0)
}

impl Guarded {
    /// Before output `o` is unmuted: re-send its gains if they are suspect.
    fn before_unmute(&mut self, o: usize) -> Result<(), TransportError> {
        let seed = {
            let mut s = self.guard.state();
            let seed = s.lazy && !s.seeded;
            s.seeded = true;
            seed
        };
        if seed && !self.guard.fixed(&mut *self.inner, None) {
            let back = wait_applied(&mut *self.inner)?;
            let outputs = back.get(3).copied().map_or(0, usize::from);
            self.guard.suspect(muted_mask(&back, outputs));
        }
        if self.guard.suspects() & bit(o) == 0 {
            return Ok(());
        }
        reconvert_output(&mut *self.inner, o).map_err(|e| {
            TransportError::Malformed(format!(
                "output {} stays muted: a bulk write did not finish, so its gains may still be \
                 the firmware's wrong ones, and re-sending them failed ({e}); load a preset or \
                 apply the configuration again",
                o.saturating_add(1)
            ))
        })?;
        self.guard.state().suspect &= !bit(o);
        Ok(())
    }

    // -- WP SW2 --
    /// Before an unmute of the whole unit: [`Self::before_unmute`] for every
    /// suspect output (the first call also seeds a lazy guard).
    fn before_unmute_all(&mut self) -> Result<(), TransportError> {
        self.before_unmute(0)?;
        let suspects = self.guard.suspects();
        for o in 1..32 {
            if suspects & bit(o) != 0 {
                self.before_unmute(o)?;
            }
        }
        Ok(())
    }

    /// A preset load or factory reset that the device answered with 0
    /// (`PRESET_OK` / `FLASH_OK`) converted every gain with `powf`.
    fn after_in(&self, opcode: u8, reply: &Result<Vec<u8>, TransportError>) {
        if matches!(opcode, op::REQ_PRESET_LOAD | op::REQ_FACTORY_RESET)
            && reply.as_ref().is_ok_and(|b| b.first() == Some(&0))
        {
            self.guard.clear_all();
        }
    }
}

impl Transport for Guarded {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>, TransportError> {
        let r = self.inner.control_in(opcode, value, len);
        self.after_in(opcode, &r);
        r
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> Result<Vec<u8>, TransportError> {
        let r = self.inner.control_in_upto(opcode, value, max_len);
        self.after_in(opcode, &r);
        r
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<(), TransportError> {
        if opcode == op::REQ_SET_OUTPUT_MUTE && data.first() == Some(&0) {
            self.before_unmute(usize::from(value))?;
        }
        // -- WP SW2 -- the audio link's apply-at-frame UNMUTE (op 2,
        // `extension_channel_spec.md` 2.8.7) unmutes the unit at a later frame:
        // the suspect outputs' gains are re-sent now, or the op is refused, as
        // for an unmute of each.
        if is_apply_unmute(opcode, value, data) {
            self.before_unmute_all()?;
        }
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.inner.session_id()
    }

    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<(), TransportError> {
        self.inner.lock(reason, timeout)
    }

    fn unlock(&mut self) {
        self.inner.unlock();
    }

    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>, TransportError>> {
        let replies = self.inner.control_in_batch(reads);
        for (r, reply) in reads.iter().zip(&replies) {
            self.after_in(r.opcode, reply);
        }
        replies
    }
}
