//! Solo one output (WP GAP2): a temporary listening state, not a setting.
//!
//! Neither the firmware nor the controllers have an output solo, so the core
//! makes one out of output mutes, in a way that cannot reach saved state:
//!
//! - [`DspiCore::solo_output`] mutes, on the device only, every output but
//!   the one named. It never unmutes anything (an output the user muted stays
//!   muted, the soloed one too) and writes nothing but output mutes, so the
//!   tweeter protection (the high-pass floor, the load guard's holds, a
//!   limits profile) is untouched. Naming another output moves the solo.
//! - The mirror keeps the user's mutes: the outputs the solo muted are
//!   remembered, and every way the device's image comes back into the mirror
//!   (a re-read, a notification's patch, the solo's own echoes) shows them as
//!   the user left them ([`overlay`]). The unsaved list, exports, snapshots,
//!   the agents' reads and every screen see the user's mutes.
//! - A mute change of another output while soloed is the user's (journalled
//!   and undoable as ever), but the device keeps that output muted until the
//!   solo ends ([`holds_write`]).
//! - It ends with [`DspiCore::solo_end`]; before a save writes the live state
//!   to flash ([`lift`]: the mutes go back first, and a failure fails the
//!   save); before or with anything that replaces the whole live state (a
//!   preset load, a bulk image, a factory reset, another writer's image:
//!   [`state_replaced`]); at a measurement's start; and with its connection.
//!   [`DspiCore::disconnect`] puts the mutes back first, best effort.
//! - No undo: starting, moving and ending a solo are not journalled.
//!
//! Known limit: a connection lost without a close (the app killed, a Link
//! session gone) leaves the outputs it muted muted on the device, in the
//! live state only; the next connect reads them as muted.

use dspi_proto::value::Value;

use crate::records::DspState;
use crate::{Connection, DspiCore, DspiError, Inner, Result, state};

/// The solo on a connection; the default is none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Solo {
    /// The output that plays, `None` with no solo.
    output: Option<u8>,
    /// The outputs the solo holds muted on the device that the user left
    /// unmuted: the mirror shows them unmuted, the end unmutes them.
    muted: Vec<u8>,
}

/// Mute or unmute one output on the device, outside the journal and the
/// mirror.
fn write_mute(inner: &mut Inner, output: u8, muted: bool) -> Result<()> {
    let mut w = inner.writable_conn()?;
    w.conn().fresh_image = false;
    w.session()?
        .send("out.mute", &[output], Value::Bool(muted))?;
    Ok(())
}

fn solo_mut(inner: &mut Inner) -> Option<&mut Solo> {
    inner.conn.as_mut().map(|c| &mut c.solo)
}

/// Put back what the solo muted and end it. Each output leaves the solo as
/// its unmute is sent, so a failure leaves the solo holding the rest.
pub(crate) fn lift(inner: &mut Inner) -> Result<()> {
    loop {
        let Some(solo) = solo_mut(inner) else {
            return Ok(());
        };
        let Some(&o) = solo.muted.first() else {
            solo.output = None;
            return Ok(());
        };
        write_mute(inner, o, false)?;
        if let Some(solo) = solo_mut(inner) {
            solo.muted.retain(|&m| m != o);
        }
    }
}

/// The whole live state was replaced (a preset load, a bulk image, a
/// factory reset, another writer's image): the device's mutes are the new
/// state's, and the solo is over without a write.
pub(crate) fn state_replaced(conn: &mut Connection) {
    conn.solo = Solo::default();
}

/// Show the outputs the solo holds muted as the user left them, in a state
/// just read or patched from the device.
pub(crate) fn overlay(solo: &Solo, s: &mut DspState) {
    let mut changed = false;
    for &o in &solo.muted {
        if let Some(m) = s.output_muted.get_mut(usize::from(o))
            && *m
        {
            *m = false;
            changed = true;
        }
    }
    if changed && s.bulk_valid {
        s.bulk_raw = state::encode_bulk(s);
    }
}

/// A write of `path` the solo keeps from the device: a mute change of an
/// output other than the soloed one. The device holds it muted either way;
/// what the user chose is what the end puts back. Called after the write's
/// checks, with the mirror updated by the caller as for any write.
pub(crate) fn holds_write(
    conn: &mut Connection,
    path: &str,
    indices: &[u8],
    value: &Value,
) -> bool {
    let solo = &mut conn.solo;
    let (Some(soloed), "out.mute", [o], Value::Bool(muted)) = (solo.output, path, indices, value)
    else {
        return false;
    };
    if *o == soloed {
        return false;
    }
    solo.muted.retain(|m| m != o);
    if !*muted {
        solo.muted.push(*o);
    }
    true
}

/// The writes that make `output` the only one playing, from the solo `from`
/// (none: the device as the user left it), on `count` outputs with the
/// user's mutes `user`.
fn plan(from: Option<u8>, output: u8, count: u8, user: &[bool]) -> Vec<(u8, bool)> {
    let user_muted = |o: u8| user.get(usize::from(o)).copied().unwrap_or(false);
    let mut writes = Vec::new();
    match from {
        None => {
            for o in (0..count).filter(|&o| o != output && !user_muted(o)) {
                writes.push((o, true));
            }
        }
        Some(prev) => {
            if !user_muted(prev) {
                writes.push((prev, true));
            }
        }
    }
    writes
}

impl DspiCore {
    /// Solo `output` (0-based): only it plays until [`DspiCore::solo_end`]
    /// (module docs). Naming the soloed output again does nothing; naming
    /// another moves the solo. Allowed while an installer lock holds, as a
    /// mute is. Not journalled, never saved.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an output out of range;
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`]; a transfer
    /// error (the solo then ends, best effort, and the error is returned).
    pub fn solo_output(&self, output: u8) -> Result<()> {
        let _lock = crate::limits::LockScope::allow();
        let mut inner = self.writing()?;
        let count = inner.state.num_output_channels;
        if output >= count {
            return Err(DspiError::InvalidArgument {
                msg: format!("output {output} out of range (0..{count})"),
            });
        }
        let from = inner.conn()?.solo.output;
        if from == Some(output) {
            return Ok(());
        }
        let writes = plan(from, output, count, &inner.state.output_muted);
        let user_unmuted = !inner
            .state
            .output_muted
            .get(usize::from(output))
            .copied()
            .unwrap_or(false);
        let mut done = Ok(());
        // The new output first: it plays before the old one stops.
        if from.is_some() && user_unmuted && inner.conn()?.solo.muted.contains(&output) {
            done = write_mute(&mut inner, output, false);
        }
        if done.is_ok() {
            let solo = &mut inner.conn()?.solo;
            solo.output = Some(output);
            solo.muted.retain(|&m| m != output);
            for (o, muted) in writes {
                done = write_mute(&mut inner, o, muted);
                if done.is_err() {
                    break;
                }
                inner.conn()?.solo.muted.push(o);
            }
        }
        if done.is_err() {
            let _ = lift(&mut inner);
        }
        done
    }

    /// End the solo: unmute what it muted. Nothing to do without one (no
    /// I/O, no error, whatever the connection).
    ///
    /// Errors: the connection and transfer errors of a write; the solo then
    /// holds the outputs not yet unmuted.
    pub fn solo_end(&self) -> Result<()> {
        let _lock = crate::limits::LockScope::allow();
        let mut inner = self.lock();
        if inner.conn.as_ref().is_none_or(|c| c.solo.output.is_none()) {
            return Ok(());
        }
        lift(&mut inner)
    }

    /// The soloed output (0-based), `None` with no solo. No I/O.
    pub fn solo_state(&self) -> Option<u8> {
        self.lock().conn.as_ref().and_then(|c| c.solo.output)
    }
}

#[cfg(test)]
#[path = "solo/tests_wp_gap2.rs"]
mod tests_wp_gap2;
