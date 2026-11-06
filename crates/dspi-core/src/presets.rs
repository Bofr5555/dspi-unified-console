//! Presets, flash, baseline and diff.
//!
//! - Save, load and delete are deferred on the device (they run on its main
//!   loop, bracketed by a pipeline reset), so each one polls the device until
//!   it reports the change (the directory's occupied mask, or the active
//!   slot) before the mirror is updated, as the old desktop core did.
//! - The flows that take more than one request (save, load, copy, delete,
//!   save params, revert, factory reset) run under the transport's device
//!   lock (`Transport::lock`: a no-op except on DSPi Link, where it holds off
//!   the other consoles; PLAN "Link device lock"). The lock is released on
//!   every path, failures included.
//! - The baseline is a copy of `bulk_raw` taken when the live state matched
//!   the active preset. [`DspiCore::preset_changes`] decodes the baseline and
//!   the live `bulk_raw` with the same codec onto the same template (the
//!   live mirror, so the persistence modes are the live ones) and words the
//!   differences as the old core and the macOS Console do. Comparing image
//!   to image keeps a codec that is decode-stable but not byte-identical
//!   from reporting phantom changes.
//!
//! Owned by WP C5 (docs/core-merge/PLAN.md).

use std::thread;
use std::time::{Duration, Instant};

use dspi_proto::enums::FilterType;
use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::status as st;
use dspi_proto::generated::tube as tb;

use crate::connect::{read_preset_directory, read_preset_name, refresh_params_locked};
use crate::consts::*;
use crate::records::*;
use crate::{Connection, DspiCore, DspiError, Inner, Result, error, state};

/// How long a deferred save, load or delete may take to show (the old
/// core's value).
const DEFERRED_TIMEOUT: Duration = Duration::from_millis(1500);
/// Pause between two polls of a deferred operation.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Settling time after a factory reset before the device is read again (the
/// old core's value).
const FACTORY_RESET_SETTLE: Duration = Duration::from_millis(250);
/// How long the multi-request flows wait for the Link device lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

const PRESET_ERR_INVALID_SLOT: u8 = st::PRESET_ERR_INVALID_SLOT as u8;
const PRESET_ERR_CRC: u8 = st::PRESET_ERR_CRC as u8;

/// Preset name buffer, bytes, NUL included. The same size as
/// `CHANNEL_NAME_LEN` (bulk_params.h: `WIRE_NAME_LEN` must match it), but the
/// preset's own constant.
const PRESET_NAME_LEN: usize = st::PRESET_NAME_LEN as usize;

/// `set_preset_startup` mode: load `default_slot` at boot.
const STARTUP_SPECIFIED: u8 = st::PRESET_STARTUP_SPECIFIED as u8;
/// `set_preset_startup` mode: load the last active slot at boot.
const STARTUP_LAST_ACTIVE: u8 = st::PRESET_STARTUP_LAST_ACTIVE as u8;

/// `output_config_mode` / `master_volume_mode`: travels with the preset.
const WITH_PRESET: u8 = 1;

impl DspiCore {
    /// Save the live state to preset `slot` (0..10) and make it the active
    /// slot. **Writes flash.**
    ///
    /// Sends `REQ_PRESET_SAVE`, then waits up to 1.5 s for the deferred save
    /// to show in the directory. The mirror's active slot and occupied mask
    /// follow, and the baseline becomes the live state (it now matches the
    /// active preset). Runs under the Link device lock.
    ///
    /// Errors: [`DspiError::Refused`] with the `PRESET_*` status
    /// (`PRESET_ERR_INVALID_SLOT` for a slot past 9, without sending);
    /// [`DspiError::ReadOnly`] on a read-only connection;
    /// [`DspiError::Disconnected`]; [`DspiError::Locked`] /
    /// [`DspiError::Denied`] when the Link hub refuses its lock;
    /// [`DspiError::Transfer`] when the device does not confirm in time (the
    /// mirror then follows the directory, if it answers, and the baseline
    /// stays); the transport's typed error when a transfer fails.
    pub fn save_preset(&self, slot: u8) -> Result<()> {
        let bit = slot_bit(slot)?;
        self.with_device_lock("Saving preset", |inner| {
            save_locked(inner, slot, bit)?;
            capture_baseline_locked(inner);
            Ok(())
        })
    }

    /// Load preset `slot` (0..10) and re-read every parameter into the
    /// mirror.
    ///
    /// Sends `REQ_PRESET_LOAD`, waits up to 1.5 s for the device to report
    /// the slot active, then re-reads the bulk image ([`DspiCore::refresh_params`]) and the
    /// directory. The baseline becomes the loaded state. Runs under the Link
    /// device lock. The device also notifies `PRESET_LOADED` and then
    /// `BULK_INVALIDATED` (source `PRESET`); `process_notifications`
    /// re-reads on those as on any foreign change.
    ///
    /// Errors: [`DspiError::Refused`] with the `PRESET_*` status;
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`];
    /// [`DspiError::Locked`] / [`DspiError::Denied`];
    /// [`DspiError::Transfer`] when the device does not confirm the load in
    /// time (a load that failed on the device looks the same: the image is
    /// not re-read, the active slot follows the directory if it answers, and
    /// the baseline stays); the transport's typed error when a transfer, or
    /// the re-read, fails.
    pub fn load_preset(&self, slot: u8) -> Result<()> {
        slot_bit(slot)?;
        device_lock(&mut self.lock(), "Loading preset")?;
        let result = self.load_flow(slot);
        self.device_unlock();
        result
    }

    /// Copy the live state into preset `dest`, then save it back to
    /// `source` so `source` stays the active slot (as the macOS Console
    /// does). **Writes flash twice.**
    ///
    /// Each deferred save must finish (the device reports that slot active)
    /// before the next is sent, or the second would replace the first's
    /// pending save. The baseline becomes the live state. Runs under the
    /// Link device lock.
    ///
    /// Errors: as [`DspiCore::save_preset`] for either slot, plus
    /// [`DspiError::Transfer`] when the device does not confirm either save
    /// in time (after the first, nothing is saved to `source`; `dest` may
    /// hold the live state and the directory says which slot is active).
    pub fn copy_preset(&self, source: u8, dest: u8) -> Result<()> {
        let source_bit = slot_bit(source)?;
        let dest_bit = slot_bit(dest)?;
        self.with_device_lock("Copying preset", |inner| {
            save_locked(inner, dest, dest_bit)?;
            if !wait_for(inner.conn()?, |c| Ok(read_active(c)? == dest)) {
                return Err(unconfirmed(inner, "saving", dest));
            }
            save_locked(inner, source, source_bit)?;
            if !wait_for(inner.conn()?, |c| Ok(read_active(c)? == source)) {
                return Err(unconfirmed(inner, "saving", source));
            }
            capture_baseline_locked(inner);
            Ok(())
        })
    }

    /// Delete preset `slot` (0..10) and its name. **Writes flash.**
    ///
    /// Waits up to 1.5 s for the slot to leave the directory. Deleting the
    /// active slot makes the device apply its factory defaults, so the
    /// image is then re-read (the baseline is kept: the live state matches
    /// no preset). Runs under the Link device lock.
    ///
    /// Errors: [`DspiError::Refused`] with the `PRESET_*` status;
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`];
    /// [`DspiError::Locked`] / [`DspiError::Denied`];
    /// [`DspiError::Transfer`] when the device does not confirm in time (the
    /// mirror's directory fields then follow the directory, if it answers,
    /// and the name is kept); the transport's typed error when a transfer or
    /// the re-read fails.
    pub fn delete_preset(&self, slot: u8) -> Result<()> {
        let bit = slot_bit(slot)?;
        device_lock(&mut self.lock(), "Deleting preset")?;
        let result = self.delete_flow(slot, bit);
        self.device_unlock();
        result
    }

    /// Name preset `slot` (0..10): at most 31 bytes of UTF-8, cut at a
    /// character boundary (and at an embedded NUL); an empty name clears it.
    /// The mirror's `preset_names[slot]` follows.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a slot past 9;
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`]; the transport's
    /// typed error when the transfer fails.
    pub fn set_preset_name(&self, slot: u8, name: String) -> Result<()> {
        let index = slot_index(slot)?;
        let payload = name_payload(&name);
        let mut guard = self.lock();
        let inner = &mut *guard;
        inner
            .writable_conn()?
            .set(op::REQ_PRESET_SET_NAME, u16::from(slot), &payload)?;
        if let Some(n) = inner.state.preset_names.get_mut(index) {
            let text = payload.split_last().map_or(&[][..], |(_, b)| b);
            *n = String::from_utf8_lossy(text).into_owned();
        }
        Ok(())
    }

    /// Read the name of preset `slot` (0..10) into the mirror and return it.
    /// Works on a read-only connection.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a slot past 9;
    /// [`DspiError::Disconnected`]; the transport's typed error when the
    /// read fails (an empty slot stalls; its mirrored name is cleared).
    pub fn get_preset_name(&self, slot: u8) -> Result<String> {
        let mut guard = self.lock();
        let Inner { conn, state, .. } = &mut *guard;
        let conn = conn.as_mut().ok_or(DspiError::Disconnected)?;
        read_preset_name(conn, state, slot)
    }

    /// Read the preset directory (occupied slots, startup mode and slot,
    /// active slot, the output-config and master-volume persistence modes)
    /// into the mirror and return it. Works on a read-only connection.
    ///
    /// Errors: [`DspiError::Disconnected`]; the transport's typed error when
    /// the read fails or the reply is shorter than 5 bytes.
    pub fn get_preset_directory(&self) -> Result<PresetDirectory> {
        let mut guard = self.lock();
        let Inner { conn, state, .. } = &mut *guard;
        let conn = conn.as_mut().ok_or(DspiError::Disconnected)?;
        read_preset_directory(conn, state)
    }

    /// Choose the preset loaded at boot: `mode` 0 = load `default_slot`,
    /// 1 = the last active slot. The mirror follows.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a mode other than 0 or 1
    /// or a slot past 9; [`DspiError::ReadOnly`];
    /// [`DspiError::Disconnected`]; the transport's typed error when the
    /// transfer fails.
    pub fn set_preset_startup(&self, mode: u8, default_slot: u8) -> Result<()> {
        if mode != STARTUP_SPECIFIED && mode != STARTUP_LAST_ACTIVE {
            return Err(DspiError::InvalidArgument {
                msg: format!("preset startup mode {mode} is not 0 or 1"),
            });
        }
        slot_index(default_slot)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        inner
            .writable_conn()?
            .set(op::REQ_PRESET_SET_STARTUP, 0, &[mode, default_slot])?;
        inner.state.preset_startup_mode = mode;
        inner.state.preset_default_slot = default_slot;
        Ok(())
    }

    /// Save the live state to the active preset slot (`REQ_SAVE_PARAMS`,
    /// deferred on the device). **Writes flash.** The baseline becomes the
    /// live state. Runs under the Link device lock.
    ///
    /// Errors: [`DspiError::Refused`] with the `FLASH_*` status;
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`];
    /// [`DspiError::Locked`] / [`DspiError::Denied`]; the transport's typed
    /// error when the transfer fails.
    pub fn save_params(&self) -> Result<()> {
        self.with_device_lock("Saving to flash", |inner| {
            let code = inner.writable_conn()?.set_as_read(op::REQ_SAVE_PARAMS, 0)?;
            error::status(code)?;
            capture_baseline_locked(inner);
            Ok(())
        })
    }

    /// Revert to saved: reload the active preset through the deferred,
    /// S/PDIF-safe preset load ([`DspiCore::load_preset`] of the mirror's
    /// `active_preset_slot`).
    ///
    /// Errors: as [`DspiCore::load_preset`], except that a refusal is
    /// reported as [`DspiError::Refused`] with a `FLASH_*` status:
    /// `FLASH_ERR_CRC` for a corrupt slot, `FLASH_ERR_WRITE` otherwise.
    pub fn load_params(&self) -> Result<()> {
        let slot = self.lock().state.active_preset_slot;
        self.load_preset(slot).map_err(|e| match e {
            DspiError::Refused {
                status: PRESET_ERR_CRC,
            } => DspiError::Refused {
                status: FLASH_ERR_CRC,
            },
            DspiError::Refused { .. } => DspiError::Refused {
                status: FLASH_ERR_WRITE,
            },
            other => other,
        })
    }

    /// Restore the factory defaults (`REQ_FACTORY_RESET`, deferred on the
    /// device). **Writes flash.** Waits 250 ms, then re-reads everything
    /// ([`DspiCore::fetch_all`]); the baseline becomes the reset state.
    /// Runs under the Link device lock.
    ///
    /// Errors: [`DspiError::Refused`] with the `FLASH_*` status;
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`];
    /// [`DspiError::Locked`] / [`DspiError::Denied`]; the transport's typed
    /// error when the transfer or the re-read fails.
    pub fn factory_reset(&self) -> Result<()> {
        device_lock(&mut self.lock(), "Factory reset")?;
        let result = self.factory_reset_flow();
        self.device_unlock();
        result
    }

    /// The live state now matches the active preset: remember its image as
    /// the baseline for [`DspiCore::preset_dirty`] and
    /// [`DspiCore::preset_changes`]. Without a device, or before an image
    /// has been read, the baseline is cleared. Never fails.
    pub fn capture_baseline(&self) {
        capture_baseline_locked(&mut self.lock());
    }

    /// Whether the live state differs from the baseline in anything a
    /// preset holds: exactly when [`DspiCore::preset_changes`] is not
    /// empty. False before any baseline. Never fails.
    pub fn preset_dirty(&self) -> bool {
        let guard = self.lock();
        match &guard.baseline {
            Some(b) if *b != guard.state.bulk_raw => !changes(b, &guard.state).is_empty(),
            _ => false,
        }
    }

    /// What differs from the baseline (the active preset), one sentence
    /// each, worded as the macOS Console words it ("Preamp L: 0.0 dB →
    /// -3.0 dB", "2 bands changed on Output 1"). The master volume counts
    /// only in WITH_PRESET master-volume mode, the hardware wiring and the
    /// limiters only in WITH_PRESET output-config mode, both judged by the
    /// live mode. Empty before any baseline. Never fails.
    pub fn preset_changes(&self) -> Vec<String> {
        let guard = self.lock();
        match &guard.baseline {
            Some(b) if *b != guard.state.bulk_raw => changes(b, &guard.state),
            _ => Vec::new(),
        }
    }

    // ── Flows ─────────────────────────────────────────────────────────

    /// Take the device lock ([`device_lock`]), run `f` under the core's
    /// lock and release the device lock whatever `f` returned.
    fn with_device_lock<T>(
        &self,
        reason: &str,
        f: impl FnOnce(&mut Inner) -> Result<T>,
    ) -> Result<T> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        device_lock(inner, reason)?;
        let result = f(inner);
        if let Some(c) = inner.conn.as_mut() {
            c.transport().unlock();
        }
        result
    }

    /// Release the device lock taken by a flow that has to drop the core's
    /// lock on the way (to call `refresh_params` / `fetch_all`, which take
    /// it themselves).
    ///
    /// Unlocks the current connection: if the connection was replaced
    /// mid-flow, this is a stray unlock on the new one (the old one's lock
    /// went with its transport).
    fn device_unlock(&self) {
        if let Some(c) = self.lock().conn.as_mut() {
            c.transport().unlock();
        }
    }

    /// `load_preset` once the device lock is held.
    fn load_flow(&self, slot: u8) -> Result<()> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let code = inner
            .writable_conn()?
            .set_as_read(op::REQ_PRESET_LOAD, u16::from(slot))?;
        error::status(code)?;
        if !wait_for(inner.conn()?, |c| Ok(read_active(c)? == slot)) {
            return Err(unconfirmed(inner, "loading", slot));
        }
        // Kept across the re-read, so no notification or setter lands between
        // the device's state and the mirror.
        refresh_params_locked(inner)?;
        let directory = inner
            .conn
            .as_mut()
            .map(|conn| read_preset_directory(conn, &mut inner.state));
        if !matches!(directory, Some(Ok(_))) {
            // The load was confirmed; only the directory read failed.
            inner.state.active_preset_slot = slot;
        }
        // The directory is trusted over the poll: the baseline must not be
        // taken against a slot the device says is not the active one.
        if inner.state.active_preset_slot != slot {
            return Err(unconfirmed(inner, "loading", slot));
        }
        capture_baseline_locked(inner);
        Ok(())
    }

    /// `delete_preset` once the device lock is held.
    fn delete_flow(&self, slot: u8, bit: u16) -> Result<()> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let code = inner
            .writable_conn()?
            .set_as_read(op::REQ_PRESET_DELETE, u16::from(slot))?;
        error::status(code)?;
        if !wait_for(inner.conn()?, |c| Ok(read_occupied(c)? & bit == 0)) {
            return Err(unconfirmed(inner, "deleting", slot));
        }
        let s = &mut inner.state;
        s.preset_occupied &= !bit;
        // The device clears the slot's name with it.
        if let Some(n) = s.preset_names.get_mut(usize::from(slot)) {
            n.clear();
        }
        if s.active_preset_slot == slot {
            // Deleting the active slot applies the factory defaults.
            refresh_params_locked(inner)?;
        }
        Ok(())
    }

    /// `factory_reset` once the device lock is held.
    fn factory_reset_flow(&self) -> Result<()> {
        {
            let mut guard = self.lock();
            let code = guard
                .writable_conn()?
                .set_as_read(op::REQ_FACTORY_RESET, 0)?;
            error::status(code)?;
        }
        thread::sleep(FACTORY_RESET_SETTLE);
        // Ends with the baseline.
        self.fetch_all()
    }
}

// ── Helpers (called with the core's lock held, or without state) ──────

/// Remember the live image as the baseline. The hook for every flow that
/// leaves the live state equal to the active preset, and for
/// `process_notifications` (WP C7) once it has re-read a `BULK_INVALIDATED`
/// from a preset load or factory reset. Cleared without a device, or when
/// no image has been read (the mirror's image does not decode).
pub(crate) fn capture_baseline_locked(inner: &mut Inner) {
    let readable = inner.conn.is_some() && {
        let mut probe = inner.state.clone();
        state::decode_bulk(&inner.state.bulk_raw, &mut probe).is_ok()
    };
    inner.baseline = readable.then(|| inner.state.bulk_raw.clone());
}

/// Check the connection is writable (a read-only one is `ReadOnly`, before
/// any lock request) and take the transport's device lock for `reason`.
/// The caller releases it on every path.
fn device_lock(inner: &mut Inner, reason: &str) -> Result<()> {
    inner
        .writable_conn()?
        .conn()
        .transport()
        .lock(reason, LOCK_TIMEOUT)?;
    Ok(())
}

/// The occupied-mask bit of `slot`, or `Refused { PRESET_ERR_INVALID_SLOT }`
/// (what the firmware answers) for a slot past `MAX_PRESETS`.
fn slot_bit(slot: u8) -> Result<u16> {
    let refused = DspiError::Refused {
        status: PRESET_ERR_INVALID_SLOT,
    };
    if usize::from(slot) >= MAX_PRESETS {
        return Err(refused);
    }
    1u16.checked_shl(u32::from(slot)).ok_or(refused)
}

/// `slot` as an index into `preset_names`, or `InvalidArgument`.
fn slot_index(slot: u8) -> Result<usize> {
    let index = usize::from(slot);
    if index >= MAX_PRESETS {
        return Err(DspiError::InvalidArgument {
            msg: format!("preset slot {slot} is out of range (0..{MAX_PRESETS})"),
        });
    }
    Ok(index)
}

/// `REQ_PRESET_SAVE` of `slot`, then wait for it to show in the directory;
/// the mirror's active slot and occupied mask follow.
fn save_locked(inner: &mut Inner, slot: u8, bit: u16) -> Result<()> {
    let code = inner
        .writable_conn()?
        .set_as_read(op::REQ_PRESET_SAVE, u16::from(slot))?;
    error::status(code)?;
    if !wait_for(inner.conn()?, |c| Ok(read_occupied(c)? & bit != 0)) {
        return Err(unconfirmed(inner, "saving", slot));
    }
    inner.state.active_preset_slot = slot;
    inner.state.preset_occupied |= bit;
    Ok(())
}

/// A deferred operation the device did not confirm in time, which may also
/// have failed on the device: the mirror's directory fields follow the
/// directory, if it answers, instead of assuming the operation's outcome.
fn unconfirmed(inner: &mut Inner, doing: &str, slot: u8) -> DspiError {
    if let Some(conn) = inner.conn.as_mut() {
        let _ = read_preset_directory(conn, &mut inner.state);
    }
    DspiError::Transfer {
        msg: format!("the device did not confirm {doing} preset {slot} in time"),
    }
}

/// Poll `done` until it holds or the deferred-operation timeout passes. A
/// failed poll counts as "not yet".
fn wait_for(conn: &mut Connection, mut done: impl FnMut(&mut Connection) -> Result<bool>) -> bool {
    let start = Instant::now();
    loop {
        if let Ok(true) = done(conn) {
            return true;
        }
        if start.elapsed() >= DEFERRED_TIMEOUT {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// `REQ_PRESET_GET_ACTIVE`: the active slot.
fn read_active(conn: &mut Connection) -> Result<u8> {
    let d = conn.get_upto(op::REQ_PRESET_GET_ACTIVE, 0, 1)?;
    d.first().copied().ok_or(DspiError::Transfer {
        msg: "empty reply to REQ_PRESET_GET_ACTIVE".into(),
    })
}

/// The directory's occupied mask, read into a scratch state (the caller
/// updates the mirror once the operation is confirmed).
fn read_occupied(conn: &mut Connection) -> Result<u16> {
    let mut scratch = state::default_state();
    Ok(read_preset_directory(conn, &mut scratch)?.occupied_mask)
}

/// The `REQ_PRESET_SET_NAME` payload: `name` up to an embedded NUL, at most
/// 31 bytes cut at a character boundary, then the NUL. An empty name is a
/// lone NUL, which clears the name.
fn name_payload(name: &str) -> Vec<u8> {
    let name = name.split('\0').next().unwrap_or("");
    let mut len = name.len().min(PRESET_NAME_LEN.saturating_sub(1));
    while !name.is_char_boundary(len) {
        len = len.saturating_sub(1);
    }
    let mut out = name.as_bytes().get(..len).unwrap_or(&[]).to_vec();
    out.push(0);
    out
}

/// The changes from the `baseline` image to the live one, both decoded onto
/// the live mirror (so the fields the image does not carry, such as the
/// persistence modes, are the live ones). Empty when either does not decode.
fn changes(baseline: &[u8], live: &DspState) -> Vec<String> {
    let mut old = live.clone();
    let mut new = live.clone();
    if state::decode_bulk(baseline, &mut old).is_err()
        || state::decode_bulk(&live.bulk_raw, &mut new).is_err()
    {
        return Vec::new();
    }
    diff(&old, &new)
}

// ── The diff (the old core's presetdiff.rs, on checked access) ────────

/// `v[i]`, or the default past the end.
fn at<T: Copy + Default>(v: &[T], i: usize) -> T {
    v.get(i).copied().unwrap_or_default()
}

/// `v[i][j]`, or the default past either end.
fn at2<T: Copy + Default>(v: &[Vec<T>], i: usize, j: usize) -> T {
    v.get(i).map(|r| at(r, j)).unwrap_or_default()
}

/// Two decoded values differ (two NaNs are the same value).
fn ne(a: f32, b: f32) -> bool {
    a != b && !(a.is_nan() && b.is_nan())
}

fn db(v: f32) -> String {
    format!("{v:.1} dB")
}

/// Whole numbers without decimals, others with one.
fn val(v: f32) -> String {
    if v == v.round() && v.abs() < 100_000.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// A delay to its stored 0.0001 ms, without trailing zeros ("1", "1.25").
fn ms(v: f32) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn on(b: bool) -> &'static str {
    if b { "enabled" } else { "disabled" }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A wire channel's name, or its default label.
fn channel_label(s: &DspState, wire: usize) -> String {
    if let Some(name) = s.channel_names.get(wire).filter(|n| !n.is_empty()) {
        return name.clone();
    }
    let ins = usize::from(s.num_input_channels);
    if wire < ins {
        return match wire {
            0 => "USB L".into(),
            1 => "USB R".into(),
            n => format!("Input {}", n.saturating_add(1)),
        };
    }
    format!("Output {}", wire.saturating_sub(ins).saturating_add(1))
}

fn band(v: &[Vec<FilterParams>], ch: usize, b: usize) -> Option<FilterParams> {
    v.get(ch).and_then(|r| r.get(b)).copied()
}

fn same_filter(a: Option<FilterParams>, b: Option<FilterParams>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    // A Linkwitz Transform's target Q counts; for every other type it is
    // ignored.
    let qp = |f: &FilterParams| {
        if FilterType::from_raw(f.filter_type).is_linkwitz() {
            f.qp
        } else {
            0.0
        }
    };
    a.filter_type == b.filter_type
        && a.bypass == b.bypass
        && !ne(a.freq, b.freq)
        && !ne(a.q, b.q)
        && !ne(a.gain, b.gain)
        && !ne(qp(&a), qp(&b))
}

/// What differs between the baseline `o` and the live state `n`, worded as
/// the macOS Console words it. The persistence modes are `n`'s.
fn diff(o: &DspState, n: &DspState) -> Vec<String> {
    let mut c = Vec::new();
    let ins = usize::from(n.num_input_channels).min(MAX_INPUTS);
    let outs = usize::from(n.num_output_channels).min(MAX_OUTPUTS);
    let all = ins.saturating_add(outs).min(MAX_CHANNELS);
    let hardware = n.output_config_mode == WITH_PRESET;
    diff_global(o, n, ins, &mut c);
    diff_effects(o, n, &mut c);
    if hardware {
        diff_limiters(o, n, ins, outs, &mut c);
    }
    diff_upmix_leveller(o, n, &mut c);
    diff_channels(o, n, ins, outs, all, &mut c);
    if hardware {
        diff_hardware(o, n, &mut c);
    }
    if o.input_source != n.input_source {
        let src = |v: u8| match v {
            0 => "USB".to_string(),
            1 => "S/PDIF".into(),
            2 => "I2S".into(),
            3 => "ADAT".into(),
            4..=6 => format!("S/PDIF {}", v.saturating_sub(2)),
            _ => format!("Source {v}"),
        };
        c.push(format!(
            "Input source: {} → {}",
            src(o.input_source),
            src(n.input_source)
        ));
    }
    if o.lg_sound_sync_enabled != n.lg_sound_sync_enabled {
        c.push(format!("LG Sound Sync: {}", on(n.lg_sound_sync_enabled)));
    }
    c
}

fn diff_global(o: &DspState, n: &DspState, ins: usize, c: &mut Vec<String>) {
    let labels = ["L", "R", "FC", "LFE", "BL", "BR", "SL", "SR"];
    for i in 0..ins {
        let (a, b) = (at(&o.input_preamp_db, i), at(&n.input_preamp_db, i));
        if ne(a, b) {
            let l = labels
                .get(i)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("In {}", i.saturating_add(1)));
            c.push(format!("Preamp {l}: {} → {}", db(a), db(b)));
        }
    }
    if n.master_volume_mode == WITH_PRESET && ne(o.master_volume_db, n.master_volume_db) {
        let mv = |v: f32| {
            if v <= -128.0 {
                "-∞ dB".to_string()
            } else {
                db(v)
            }
        };
        c.push(format!(
            "Master Volume: {} → {}",
            mv(o.master_volume_db),
            mv(n.master_volume_db)
        ));
    }
    if o.bypass != n.bypass {
        let w = |b: bool| if b { "on" } else { "off" };
        c.push(format!(
            "Master EQ bypass: {} → {}",
            w(o.bypass),
            w(n.bypass)
        ));
    }
}

fn diff_effects(o: &DspState, n: &DspState, c: &mut Vec<String>) {
    // Loudness
    if o.loudness_enabled != n.loudness_enabled {
        c.push(format!("Loudness: {}", on(n.loudness_enabled)));
    }
    if o.loudness_output_mask != n.loudness_output_mask {
        c.push(format!(
            "Loudness outputs: 0x{:04X} → 0x{:04X}",
            o.loudness_output_mask, n.loudness_output_mask
        ));
    }
    if ne(o.loudness_ref_spl, n.loudness_ref_spl) {
        c.push(format!(
            "Loudness ref SPL: {} → {}",
            val(o.loudness_ref_spl),
            val(n.loudness_ref_spl)
        ));
    }
    if ne(o.loudness_intensity, n.loudness_intensity) {
        c.push(format!(
            "Loudness intensity: {}% → {}%",
            val(o.loudness_intensity),
            val(n.loudness_intensity)
        ));
    }

    // Crossfeed
    if o.crossfeed_enabled != n.crossfeed_enabled {
        c.push(format!("Crossfeed: {}", on(n.crossfeed_enabled)));
    }
    if o.crossfeed_preset != n.crossfeed_preset {
        c.push(format!(
            "Crossfeed preset: {} → {}",
            o.crossfeed_preset, n.crossfeed_preset
        ));
    }
    if ne(o.crossfeed_freq, n.crossfeed_freq) {
        c.push(format!(
            "Crossfeed frequency: {} → {} Hz",
            val(o.crossfeed_freq),
            val(n.crossfeed_freq)
        ));
    }
    if ne(o.crossfeed_feed, n.crossfeed_feed) {
        c.push(format!(
            "Crossfeed feed: {} → {}",
            val(o.crossfeed_feed),
            val(n.crossfeed_feed)
        ));
    }
    if o.crossfeed_itd != n.crossfeed_itd {
        c.push(format!("Crossfeed ITD: {}", on(n.crossfeed_itd)));
    }
    if o.crossfeed_output_pair_mask != n.crossfeed_output_pair_mask {
        c.push(format!(
            "Crossfeed output pairs: 0x{:02X} → 0x{:02X}",
            o.crossfeed_output_pair_mask, n.crossfeed_output_pair_mask
        ));
    }

    // Psychoacoustic bass
    if o.psybass_enabled != n.psybass_enabled {
        c.push(format!("Psychoacoustic Bass: {}", on(n.psybass_enabled)));
    }
    if o.psybass_output_mask != n.psybass_output_mask {
        c.push(format!(
            "Psybass outputs: 0x{:04X} → 0x{:04X}",
            o.psybass_output_mask, n.psybass_output_mask
        ));
    }
    for (name, a, b, unit) in [
        (
            "Psybass cutoff",
            o.psybass_cutoff_hz,
            n.psybass_cutoff_hz,
            " Hz",
        ),
        (
            "Psybass harmonics",
            o.psybass_harmonics_db,
            n.psybass_harmonics_db,
            " dB",
        ),
        (
            "Psybass drive",
            o.psybass_drive_db,
            n.psybass_drive_db,
            " dB",
        ),
        (
            "Psybass character",
            o.psybass_character_pct,
            n.psybass_character_pct,
            "%",
        ),
        (
            "Psybass original bass",
            o.psybass_original_db,
            n.psybass_original_db,
            " dB",
        ),
    ] {
        if ne(a, b) {
            c.push(format!("{name}: {}{unit} → {}{unit}", val(a), val(b)));
        }
    }

    diff_subharm(o, n, c);
    diff_tube(o, n, c);
}

fn diff_subharm(o: &DspState, n: &DspState, c: &mut Vec<String>) {
    let sh = |i: u8| {
        (
            at(&o.subharm, usize::from(i)),
            at(&n.subharm, usize::from(i)),
        )
    };
    let (a, b) = sh(SUBHARM_PARAM_ENABLED);
    if ne(a, b) {
        c.push(format!("Subharmonic Synthesizer: {}", on(b != 0.0)));
    }
    let (a, b) = sh(SUBHARM_PARAM_MASK);
    if ne(a, b) {
        c.push(format!(
            "Subharm outputs: 0x{:04X} → 0x{:04X}",
            a as u32, b as u32
        ));
    }
    let level = |v: f32| {
        if v <= -30.0 { "Off".to_string() } else { db(v) }
    };
    for (name, i) in [
        ("Subharm 24-36 Hz", SUBHARM_PARAM_LOW),
        ("Subharm 36-56 Hz", SUBHARM_PARAM_HIGH),
        ("Subharm 56-80 Hz", SUBHARM_PARAM_TOP),
    ] {
        let (a, b) = sh(i);
        if ne(a, b) {
            c.push(format!("{name}: {} → {}", level(a), level(b)));
        }
    }
    let select = |v: f32| match v as i32 {
        1 => "Percussive",
        2 => "Sustained",
        _ => "All material",
    };
    let (a, b) = sh(SUBHARM_PARAM_SELECT);
    if ne(a, b) {
        c.push(format!(
            "Subharm selectivity: {} → {}",
            select(a),
            select(b)
        ));
    }
    let (a, b) = sh(SUBHARM_PARAM_DEPTH);
    if ne(a, b) {
        c.push(format!(
            "Subharm selectivity depth: {}% → {}%",
            val(a),
            val(b)
        ));
    }
    let (a, b) = sh(SUBHARM_PARAM_HOLD);
    if ne(a, b) {
        c.push(format!(
            "Subharm selectivity hold: {} ms → {} ms",
            val(a),
            val(b)
        ));
    }
    let ceiling = |v: f32| {
        if v >= 0.0 { "Off".to_string() } else { db(v) }
    };
    let (a, b) = sh(SUBHARM_PARAM_CEILING);
    if ne(a, b) {
        c.push(format!(
            "Subharm sub ceiling: {} → {}",
            ceiling(a),
            ceiling(b)
        ));
    }
    let (a, b) = sh(SUBHARM_PARAM_LINK);
    if ne(a, b) {
        let l = if b != 0.0 { "linked" } else { "independent" };
        c.push(format!("Subharm pair link: {l}"));
    }
    let (a, b) = sh(SUBHARM_PARAM_BOOST);
    if ne(a, b) {
        c.push(format!("Subharm LF boost: {} dB → {} dB", val(a), val(b)));
    }
    // Solo is for listening, not part of the sound: not a preset change (as
    // on macOS).
}

fn diff_tube(o: &DspState, n: &DspState, c: &mut Vec<String>) {
    const TUBE_NAMES: [&str; 17] = [
        "Custom", "12AX7", "5751", "12AT7", "12AY7", "12AU7", "6SN7", "6SL7", "6DJ8", "EF86",
        "6SJ7", "EL84", "EL34", "6L6", "6V6", "KT88", "300B",
    ];
    let t = |i: u16| (at(&o.tube, usize::from(i)), at(&n.tube, usize::from(i)));
    let (a, b) = t(tb::TUBE_PARAM_ENABLED);
    if ne(a, b) {
        c.push(format!("Tube Modeller: {}", on(b != 0.0)));
    }
    let (a, b) = t(tb::TUBE_PARAM_OUTPUT_MASK);
    if ne(a, b) {
        c.push(format!(
            "Tube outputs: 0x{:04X} → 0x{:04X}",
            a as u32, b as u32
        ));
    }
    let tube = |v: f32| TUBE_NAMES.get(v as usize).copied().unwrap_or("Custom");
    let (a, b) = t(tb::TUBE_PARAM_TUBE_TYPE);
    if ne(a, b) {
        c.push(format!("Tube type: {} → {}", tube(a), tube(b)));
    }
    for (name, i, unit) in [
        ("Tube drive", tb::TUBE_PARAM_DRIVE_DB, " dB"),
        ("Tube bias", tb::TUBE_PARAM_BIAS_PCT, "%"),
        ("Tube asymmetry", tb::TUBE_PARAM_ASYM_DB, " dB"),
        ("Tube knee hardness", tb::TUBE_PARAM_HARDNESS_PCT, "%"),
        ("Tube sag", tb::TUBE_PARAM_SAG_PCT, "%"),
    ] {
        let (a, b) = t(i);
        if ne(a, b) {
            c.push(format!("{name}: {}{unit} → {}{unit}", val(a), val(b)));
        }
    }
    let rect = |v: f32| match v as i32 {
        0 => "Solid state",
        2 => "Tube (slow)",
        3 => "Tube (heavy)",
        _ => "Tube",
    };
    let (a, b) = t(tb::TUBE_PARAM_RECTIFIER);
    if ne(a, b) {
        c.push(format!("Tube rectifier: {} → {}", rect(a), rect(b)));
    }
    let (a, b) = t(tb::TUBE_PARAM_XFMR_ENABLED);
    if ne(a, b) {
        c.push(format!("Tube output stage: {}", on(b != 0.0)));
    }
    for (name, i, unit) in [
        ("Tube damping factor", tb::TUBE_PARAM_XFMR_DAMPING, ""),
        ("Tube speaker resonance", tb::TUBE_PARAM_XFMR_RES_HZ, " Hz"),
        ("Tube mix", tb::TUBE_PARAM_MIX_PCT, "%"),
        ("Tube output trim", tb::TUBE_PARAM_TRIM_DB, " dB"),
    ] {
        let (a, b) = t(i);
        if ne(a, b) {
            c.push(format!("{name}: {}{unit} → {}{unit}", val(a), val(b)));
        }
    }
}

/// The output limiters (part of the preset only with the hardware config).
fn diff_limiters(o: &DspState, n: &DspState, ins: usize, outs: usize, c: &mut Vec<String>) {
    for i in 0..outs {
        let name = channel_label(n, ins.saturating_add(i));
        let (a, b) = (at(&o.limiter_enabled, i), at(&n.limiter_enabled, i));
        if a != b {
            c.push(format!("{name} limiter: {}", on(b)));
        }
        let (a, b) = (
            at(&o.limiter_threshold_db, i),
            at(&n.limiter_threshold_db, i),
        );
        if ne(a, b) {
            c.push(format!("{name} limiter threshold: {a:.1} → {b:.1} dBFS"));
        }
        let (a, b) = (at(&o.limiter_release_ms, i), at(&n.limiter_release_ms, i));
        if ne(a, b) {
            c.push(format!(
                "{name} limiter release: {} ms → {} ms",
                val(a),
                val(b)
            ));
        }
        let (a, b) = (at(&o.limiter_link_group, i), at(&n.limiter_link_group, i));
        if a != b {
            let g = |v: u8| {
                if v == 0 {
                    "Unlinked".to_string()
                } else {
                    format!("Group {v}")
                }
            };
            c.push(format!("{name} limiter link: {} → {}", g(a), g(b)));
        }
    }
}

fn diff_upmix_leveller(o: &DspState, n: &DspState, c: &mut Vec<String>) {
    let up = |i: u8| (at(&o.upmix, usize::from(i)), at(&n.upmix, usize::from(i)));
    let (a, b) = up(UPMIX_PARAM_ENABLED);
    if ne(a, b) {
        c.push(format!("Stereo Upmixer: {}", on(b != 0.0)));
    }
    let centre = |v: f32| match v as i32 {
        0 => "Sinner",
        1 => "Logician",
        _ => "Off",
    };
    let surround = |v: f32| match v as i32 {
        1 => "Sinner",
        2 => "Logician",
        _ => "Off",
    };
    let (a, b) = up(UPMIX_PARAM_CENTER_MODE);
    if ne(a, b) {
        c.push(format!("Centre mode: {} → {}", centre(a), centre(b)));
    }
    let (a, b) = up(UPMIX_PARAM_SURROUND_MODE);
    if ne(a, b) {
        c.push(format!("Surround mode: {} → {}", surround(a), surround(b)));
    }
    for (name, i, unit) in [
        ("Centre strength", UPMIX_PARAM_STRENGTH, "%"),
        ("Centre width", UPMIX_PARAM_WIDTH, "%"),
        ("Correlation threshold", UPMIX_PARAM_THRESHOLD, "%"),
        ("Centre attack", UPMIX_PARAM_ATTACK, " ms"),
        ("Centre release", UPMIX_PARAM_RELEASE, " ms"),
        ("Detector HPF", UPMIX_PARAM_DETECTOR_HPF, " Hz"),
        ("Surround delay", UPMIX_PARAM_SURROUND_DELAY, " ms"),
        ("Surround HPF", UPMIX_PARAM_SURROUND_HPF, " Hz"),
        ("Surround LPF", UPMIX_PARAM_SURROUND_LPF, " Hz"),
        ("Decorrelation", UPMIX_PARAM_DECORRELATION, "%"),
        ("Centre presence", UPMIX_PARAM_PRESENCE, " dB"),
    ] {
        let (a, b) = up(i);
        if ne(a, b) {
            c.push(format!("{name}: {}{unit} → {}{unit}", val(a), val(b)));
        }
    }

    // Volume leveller
    if o.leveller_enabled != n.leveller_enabled {
        c.push(format!("Volume Leveller: {}", on(n.leveller_enabled)));
    }
    if ne(o.leveller_amount, n.leveller_amount) {
        c.push(format!(
            "Leveller amount: {}% → {}%",
            val(o.leveller_amount),
            val(n.leveller_amount)
        ));
    }
    if o.leveller_speed != n.leveller_speed {
        let sp = |v: u8| {
            ["Slow", "Medium", "Fast"]
                .get(usize::from(v))
                .copied()
                .unwrap_or("Medium")
        };
        c.push(format!(
            "Leveller speed: {} → {}",
            sp(o.leveller_speed),
            sp(n.leveller_speed)
        ));
    }
    if ne(o.leveller_max_gain_db, n.leveller_max_gain_db) {
        c.push(format!(
            "Leveller max gain: {} dB → {} dB",
            val(o.leveller_max_gain_db),
            val(n.leveller_max_gain_db)
        ));
    }
    if o.leveller_lookahead != n.leveller_lookahead {
        c.push(format!("Leveller lookahead: {}", on(n.leveller_lookahead)));
    }
    if ne(o.leveller_gate_db, n.leveller_gate_db) {
        c.push(format!(
            "Leveller gate: {} dB → {} dB",
            val(o.leveller_gate_db),
            val(n.leveller_gate_db)
        ));
    }
    if o.leveller_detector_mask != n.leveller_detector_mask {
        c.push("Leveller detector channels changed".into());
    }
    if o.leveller_apply_mask != n.leveller_apply_mask {
        c.push("Leveller apply channels changed".into());
    }
}

/// Delays, the matrix, the outputs, the bands and the channel names.
fn diff_channels(
    o: &DspState,
    n: &DspState,
    ins: usize,
    outs: usize,
    all: usize,
    c: &mut Vec<String>,
) {
    // Input channel delays (outputs have their own below)
    for w in 0..ins {
        let (a, b) = (at(&o.channel_delays, w), at(&n.channel_delays, w));
        if ne(a, b) {
            c.push(format!(
                "{} delay: {} ms → {} ms",
                channel_label(n, w),
                ms(a),
                ms(b)
            ));
        }
    }

    // Matrix crosspoints
    let points = (0..ins)
        .flat_map(|i| (0..outs).map(move |j| (i, j)))
        .filter(|&(i, j)| {
            at2(&o.matrix_routing, i, j) != at2(&n.matrix_routing, i, j)
                || ne(at2(&o.matrix_gain, i, j), at2(&n.matrix_gain, i, j))
                || at2(&o.matrix_invert, i, j) != at2(&n.matrix_invert, i, j)
        })
        .count();
    if points > 0 {
        c.push(format!("{points} crosspoint{} changed", plural(points)));
    }

    // Outputs
    for i in 0..outs {
        let name = channel_label(n, ins.saturating_add(i));
        let (a, b) = (at(&o.output_enabled, i), at(&n.output_enabled, i));
        if a != b {
            c.push(format!("{name} {}", on(b)));
        }
        let (a, b) = (at(&o.output_muted, i), at(&n.output_muted, i));
        if a != b {
            c.push(format!("{name} {}", if b { "muted" } else { "unmuted" }));
        }
        let (a, b) = (at(&o.output_gain_db, i), at(&n.output_gain_db, i));
        if ne(a, b) {
            c.push(format!("{name} gain: {} → {}", db(a), db(b)));
        }
        let (a, b) = (at(&o.output_delay_ms, i), at(&n.output_delay_ms, i));
        if ne(a, b) {
            c.push(format!("{name} delay: {} ms → {} ms", ms(a), ms(b)));
        }
    }

    // EQ and crossover bands
    for w in 0..all {
        let changed = (0..BANDS_PER_CHANNEL)
            .filter(|&b| !same_filter(band(&o.filters, w, b), band(&n.filters, w, b)))
            .count();
        if changed > 0 {
            c.push(format!(
                "{changed} band{} changed on {}",
                plural(changed),
                channel_label(n, w)
            ));
        }
    }
    for w in ins..all {
        let changed = (0..MAX_XOVER_BANDS)
            .filter(|&b| !same_filter(band(&o.xover, w, b), band(&n.xover, w, b)))
            .count();
        if changed > 0 {
            c.push(format!(
                "{changed} crossover band{} changed on {}",
                plural(changed),
                channel_label(n, w)
            ));
        }
    }

    // Channel names. Changing an output's type makes the firmware rename its
    // channels that are still at their default: a consequence, not an edit.
    let types_changed = o.output_types != n.output_types;
    for w in 0..all {
        let (a, b) = (o.channel_names.get(w), n.channel_names.get(w));
        if a != b && !(types_changed && w >= ins) {
            c.push(format!(
                "'{}' → '{}'",
                a.map_or("", String::as_str),
                b.map_or("", String::as_str)
            ));
        }
    }
}

/// The hardware configuration, when it travels with the preset.
fn diff_hardware(o: &DspState, n: &DspState, c: &mut Vec<String>) {
    let pins = (0..MAX_PHYSICAL_OUTPUTS)
        .filter(|&i| at(&o.output_pins, i) != at(&n.output_pins, i))
        .count();
    if pins > 0 {
        c.push(format!("{pins} pin assignment{} changed", plural(pins)));
    }
    let types = (0..4)
        .filter(|&i| at(&o.output_types, i) != at(&n.output_types, i))
        .count();
    if types > 0 {
        c.push(format!("{types} output type{} changed", plural(types)));
    }
    if o.i2s_bck_pin != n.i2s_bck_pin {
        c.push(format!(
            "BCK pin: GPIO {} → GPIO {}",
            o.i2s_bck_pin, n.i2s_bck_pin
        ));
    }
    if o.mck_enabled != n.mck_enabled {
        c.push(format!("MCK: {}", on(n.mck_enabled)));
    }
    if o.mck_pin != n.mck_pin {
        c.push(format!("MCK pin: GPIO {} → GPIO {}", o.mck_pin, n.mck_pin));
    }
    if o.mck_multiplier != n.mck_multiplier {
        let mult = |m: u8| 128u32.checked_shl(u32::from(m.min(1))).unwrap_or(128);
        c.push(format!(
            "MCK multiplier: {}x → {}x",
            mult(o.mck_multiplier),
            mult(n.mck_multiplier)
        ));
    }
    let (a, b) = (at(&o.spdif_rx_pins, 0), at(&n.spdif_rx_pins, 0));
    if a != b {
        c.push(format!("S/PDIF RX pin: GPIO {a} → GPIO {b}"));
    }
    for k in 1..4usize {
        let bit = |m: u8| m.checked_shr(k as u32).is_some_and(|v| v & 1 == 1);
        let (was, now) = (bit(o.spdif_inputs_enabled), bit(n.spdif_inputs_enabled));
        let nth = k.saturating_add(1);
        if was != now {
            c.push(format!("S/PDIF {nth} input: {}", on(now)));
        }
        let (a, b) = (at(&o.spdif_rx_pins, k), at(&n.spdif_rx_pins, k));
        if a != b {
            c.push(format!("S/PDIF {nth} RX pin: GPIO {a} → GPIO {b}"));
        }
    }
    if o.i2s_input_channels != n.i2s_input_channels {
        c.push(format!(
            "I2S input channels: {} → {}",
            o.i2s_input_channels, n.i2s_input_channels
        ));
    }
    for p in 0..4usize {
        let (a, b) = (at(&o.i2s_rx_pins, p), at(&n.i2s_rx_pins, p));
        if a != b {
            c.push(format!(
                "I2S RX pin (pair {}): GPIO {a} → GPIO {b}",
                p.saturating_add(1)
            ));
        }
    }
    if o.i2s_input_rate != n.i2s_input_rate {
        c.push("I2S rate changed".into());
    }
    let mode = |v: u8| if v == 1 { "Slave" } else { "Master" };
    if o.i2s_clock_mode != n.i2s_clock_mode {
        c.push(format!(
            "I2S clock mode: {} → {}",
            mode(o.i2s_clock_mode),
            mode(n.i2s_clock_mode)
        ));
    }
    if o.adat_out_enabled != n.adat_out_enabled {
        c.push(format!("ADAT output: {}", on(n.adat_out_enabled)));
    }
    if o.adat_out_pin != n.adat_out_pin {
        c.push(format!(
            "ADAT pin: GPIO {} → GPIO {}",
            o.adat_out_pin, n.adat_out_pin
        ));
    }
    if o.adat_input_enabled != n.adat_input_enabled {
        c.push(format!("ADAT input: {}", on(n.adat_input_enabled)));
    }
    if o.adat_input_pin != n.adat_input_pin {
        let pin = |v: u8| {
            if v == 0xFF {
                "Not set".to_string()
            } else {
                format!("GPIO {v}")
            }
        };
        c.push(format!(
            "ADAT input pin: {} → {}",
            pin(o.adat_input_pin),
            pin(n.adat_input_pin)
        ));
    }
    if o.adat_input_clock_mode != n.adat_input_clock_mode {
        c.push(format!(
            "ADAT input clock mode: {} → {}",
            mode(o.adat_input_clock_mode),
            mode(n.adat_input_clock_mode)
        ));
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c5 {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource,
        Transport, TransportCaps, TransportError,
    };

    use super::*;

    type Log = Arc<Mutex<Vec<String>>>;

    /// A demo device that logs every request and lock call, answers some
    /// IN opcodes itself (`overrides`), can refuse the lock, and on a
    /// factory reset loads `reset_image` into the device.
    struct Spy {
        inner: FakeDevice,
        remote: FakeRemote,
        log: Log,
        overrides: HashMap<u8, Vec<u8>>,
        stalls: Vec<u8>,
        refuse_lock: Option<TransportError>,
        reset_image: Option<Vec<u8>>,
    }

    impl Spy {
        fn demo() -> Self {
            Self::with(FakeDevice::demo())
        }
        fn with(inner: FakeDevice) -> Self {
            let remote = inner.remote();
            Self {
                inner,
                remote,
                log: Arc::default(),
                overrides: HashMap::new(),
                stalls: Vec::new(),
                refuse_lock: None,
                reset_image: None,
            }
        }
        fn answer(mut self, opcode: u8, reply: &[u8]) -> Self {
            self.overrides.insert(opcode, reply.to_vec());
            self
        }
        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }
        fn reply(
            &mut self,
            op: u8,
            value: u16,
            len: u16,
            upto: bool,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.note(format!("in {op:02X} {value}"));
            if self.stalls.contains(&op) {
                return Err(TransportError::Stalled { opcode: op });
            }
            if let Some(r) = self.overrides.get(&op) {
                if op == op::REQ_FACTORY_RESET
                    && let Some(img) = self.reset_image.take()
                {
                    self.remote.load_image(img, 0);
                }
                return Ok(r.clone());
            }
            if upto {
                self.inner.control_in_upto(op, value, len)
            } else {
                self.inner.control_in(op, value, len)
            }
        }
    }

    impl Transport for Spy {
        fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
            self.reply(op, value, len, false)
        }
        fn control_in_upto(
            &mut self,
            op: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.reply(op, value, len, true)
        }
        fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
            self.note(format!("out {op:02X} {value}"));
            self.inner.control_out(op, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.inner.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.inner.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            None
        }
        fn caps(&self) -> TransportCaps {
            self.inner.caps()
        }
        fn lock(&mut self, reason: &str, timeout: Duration) -> dspi_transport::Result<()> {
            self.note(format!("lock {reason} {}", timeout.as_secs()));
            match &self.refuse_lock {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
        fn unlock(&mut self) {
            self.note("unlock".into());
        }
    }

    /// `spy` attached and read as a connect does.
    fn open(spy: Spy) -> (Arc<DspiCore>, FakeRemote, Log) {
        let remote = spy.remote.clone();
        let log = Arc::clone(&spy.log);
        let core = DspiCore::new();
        core.attach_probed(Box::new(spy), "DEMO".into(), 0).unwrap();
        core.fetch_all().unwrap();
        // `fetch_all` leaves a baseline; these tests start without one, to see
        // which flows take it.
        core.lock().baseline = None;
        log.lock().unwrap().clear();
        (core, remote, log)
    }

    fn entries(log: &Log) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    fn sent(log: &Log, dir: &str, opcode: u8) -> bool {
        let prefix = format!("{dir} {opcode:02X} ");
        entries(log).iter().any(|e| e.starts_with(&prefix))
    }

    /// The first and last log entries: the lock taken, then released.
    fn bracketed(log: &Log, reason: &str) {
        let e = entries(log);
        assert_eq!(
            e.first().map(String::as_str),
            Some(&*format!("lock {reason} 60")),
            "{e:?}"
        );
        assert_eq!(e.last().map(String::as_str), Some("unlock"), "{e:?}");
        assert_eq!(
            e.iter().filter(|x| x.starts_with("lock")).count(),
            1,
            "{e:?}"
        );
        assert_eq!(e.iter().filter(|x| *x == "unlock").count(), 1, "{e:?}");
    }

    /// Change the mirror as a setter does: the field, then `bulk_raw`.
    fn edit(core: &DspiCore, f: impl FnOnce(&mut DspState)) {
        let mut g = core.lock();
        f(&mut g.state);
        g.state.bulk_raw = state::encode_bulk(&g.state);
    }

    fn decoded(image: &[u8]) -> DspState {
        let mut s = state::default_state();
        state::decode_bulk(image, &mut s).unwrap();
        s
    }

    #[test]
    fn wp_c5_save_preset_fills_the_slot_under_the_lock() {
        let (core, remote, log) = open(Spy::demo());
        edit(&core, |s| s.input_preamp_db[0] = -3.0);
        core.apply_all_params().unwrap();
        log.lock().unwrap().clear();

        core.save_preset(3).unwrap();
        bracketed(&log, "Saving preset");
        let s = core.get_state();
        assert_eq!(s.active_preset_slot, 3);
        assert_eq!(s.preset_occupied, 0b1001);
        let dir = core.get_preset_directory().unwrap();
        assert_eq!((dir.occupied_mask, dir.last_active), (0b1001, 3));
        // The live state is the active preset now.
        assert!(!core.preset_dirty());
        assert_eq!(core.lock().baseline.as_deref(), Some(&*s.bulk_raw));
        assert_eq!(decoded(&remote.image()).input_preamp_db[0], -3.0);
    }

    #[test]
    fn wp_c5_load_preset_rereads_and_rebaselines() {
        let (core, remote, log) = open(Spy::demo());
        let saved = core.get_state().output_gain_db[0];
        core.save_preset(2).unwrap();
        // Edit and send it to the device: dirty against slot 2.
        edit(&core, |s| s.output_gain_db[0] = -9.0);
        core.apply_all_params().unwrap();
        assert!(core.preset_dirty());
        let s = core.get_state();
        let out1 = channel_label(&s, usize::from(s.num_input_channels));
        assert_eq!(
            core.preset_changes(),
            vec![format!("{out1} gain: {} → -9.0 dB", db(saved))]
        );
        log.lock().unwrap().clear();

        core.load_preset(2).unwrap();
        bracketed(&log, "Loading preset");
        assert!(sent(&log, "in", op::REQ_PRESET_LOAD));
        let s = core.get_state();
        assert_eq!(s.output_gain_db[0], saved);
        assert_eq!(
            s.bulk_raw,
            remote.image(),
            "the mirror is the device's image"
        );
        assert_eq!(s.active_preset_slot, 2);
        assert!(!core.preset_dirty());
        assert!(core.preset_changes().is_empty());
    }

    #[test]
    fn wp_c5_load_params_reverts_to_the_active_slot() {
        let (core, _remote, log) = open(Spy::demo());
        let saved = core.get_state().output_gain_db[1];
        core.capture_baseline();
        edit(&core, |s| s.output_gain_db[1] = -20.0);
        core.apply_all_params().unwrap();
        log.lock().unwrap().clear();
        core.load_params().unwrap();
        assert!(entries(&log).contains(&format!("in {:02X} 0", op::REQ_PRESET_LOAD)));
        assert_eq!(core.get_state().output_gain_db[1], saved);
        assert!(!core.preset_dirty());
    }

    #[test]
    fn wp_c5_load_params_reports_flash_codes() {
        let crc = Spy::demo().answer(op::REQ_PRESET_LOAD, &[PRESET_ERR_CRC]);
        let (core, _, log) = open(crc);
        assert_eq!(
            core.load_params(),
            Err(DspiError::Refused {
                status: FLASH_ERR_CRC
            })
        );
        bracketed(&log, "Loading preset");
        let empty = Spy::demo().answer(op::REQ_PRESET_LOAD, &[st::PRESET_ERR_SLOT_EMPTY as u8]);
        let (core, _, _) = open(empty);
        assert_eq!(
            core.load_params(),
            Err(DspiError::Refused {
                status: FLASH_ERR_WRITE
            })
        );
        // load_preset itself keeps the PRESET_* code.
        assert_eq!(
            core.load_preset(1),
            Err(DspiError::Refused {
                status: st::PRESET_ERR_SLOT_EMPTY as u8
            })
        );
    }

    #[test]
    fn wp_c5_copy_preset_keeps_the_source_active() {
        let (core, remote, log) = open(Spy::demo());
        core.copy_preset(0, 4).unwrap();
        bracketed(&log, "Copying preset");
        let saves: Vec<_> = entries(&log)
            .into_iter()
            .filter(|e| e.starts_with(&format!("in {:02X} ", op::REQ_PRESET_SAVE)))
            .collect();
        assert_eq!(
            saves,
            vec![
                format!("in {:02X} 4", op::REQ_PRESET_SAVE),
                format!("in {:02X} 0", op::REQ_PRESET_SAVE)
            ]
        );
        let s = core.get_state();
        assert_eq!(s.active_preset_slot, 0);
        assert_eq!(s.preset_occupied, 0b1_0001);
        let dir = core.get_preset_directory().unwrap();
        assert_eq!((dir.occupied_mask, dir.last_active), (0b1_0001, 0));
        assert!(!core.preset_dirty());
        assert_eq!(s.bulk_raw, remote.image());
    }

    #[test]
    fn wp_c5_delete_preset_clears_the_slot_and_its_name() {
        let (core, _, log) = open(Spy::demo());
        core.save_preset(5).unwrap();
        core.set_preset_name(5, "Night".into()).unwrap();
        core.save_preset(0).unwrap();
        log.lock().unwrap().clear();
        core.delete_preset(5).unwrap();
        bracketed(&log, "Deleting preset");
        // Not the active slot: no re-read.
        assert!(!sent(&log, "in", op::REQ_GET_ALL_PARAMS_CHUNK));
        let s = core.get_state();
        assert_eq!(s.preset_occupied, 0b1);
        assert_eq!(s.preset_names[5], "");
        // Firmware stalls on an empty slot; the fake answers an empty name.
        assert_eq!(core.get_preset_name(5).unwrap_or_default(), "");
    }

    #[test]
    fn wp_c5_deleting_the_active_slot_rereads_the_image() {
        let (core, _, log) = open(Spy::demo());
        core.delete_preset(0).unwrap();
        assert!(
            sent(&log, "in", op::REQ_GET_ALL_PARAMS_CHUNK)
                || sent(&log, "in", op::REQ_GET_ALL_PARAMS)
        );
        assert_eq!(entries(&log).last().map(String::as_str), Some("unlock"));
        assert_eq!(core.get_state().preset_occupied, 0);
    }

    #[test]
    fn wp_c5_preset_names_round_trip() {
        let (core, _, _) = open(Spy::demo());
        assert_eq!(core.get_preset_name(0).unwrap(), "Demo");
        core.set_preset_name(0, "Living room".into()).unwrap();
        assert_eq!(core.get_state().preset_names[0], "Living room");
        core.lock().state.preset_names[0].clear();
        assert_eq!(core.get_preset_name(0).unwrap(), "Living room");
        assert_eq!(core.get_state().preset_names[0], "Living room");
        // Cut at 31 bytes, on a character boundary.
        core.set_preset_name(0, "é".repeat(16)).unwrap();
        assert_eq!(core.get_preset_name(0).unwrap(), "é".repeat(15));
        assert_eq!(core.get_state().preset_names[0], "é".repeat(15));
        // Empty clears.
        core.set_preset_name(0, String::new()).unwrap();
        assert_eq!(core.get_preset_name(0).unwrap(), "");
        assert!(matches!(
            core.set_preset_name(10, "x".into()),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert!(matches!(
            core.get_preset_name(10),
            Err(DspiError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn wp_c5_name_payload_is_nul_terminated_and_bounded() {
        assert_eq!(name_payload(""), vec![0]);
        assert_eq!(name_payload("ab\0cd"), b"ab\0".to_vec());
        let long = name_payload(&"x".repeat(40));
        assert_eq!(long.len(), PRESET_NAME_LEN);
        assert_eq!(long.last(), Some(&0));
    }

    #[test]
    fn wp_c5_startup_mode_is_validated_and_stored() {
        let (core, _, _) = open(Spy::demo());
        core.set_preset_startup(1, 3).unwrap();
        let s = core.get_state();
        assert_eq!((s.preset_startup_mode, s.preset_default_slot), (1, 3));
        let dir = core.get_preset_directory().unwrap();
        assert_eq!((dir.startup_mode, dir.default_slot), (1, 3));
        assert!(matches!(
            core.set_preset_startup(2, 0),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert!(matches!(
            core.set_preset_startup(0, 10),
            Err(DspiError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn wp_c5_slots_past_the_last_are_refused_without_sending() {
        let (core, _, log) = open(Spy::demo());
        let invalid = Err(DspiError::Refused {
            status: PRESET_ERR_INVALID_SLOT,
        });
        assert_eq!(core.save_preset(10), invalid);
        assert_eq!(core.load_preset(200), invalid);
        assert_eq!(core.delete_preset(10), invalid);
        assert_eq!(core.copy_preset(0, 10), invalid);
        assert_eq!(core.copy_preset(10, 0), invalid);
        assert!(entries(&log).is_empty(), "{:?}", entries(&log));
    }

    #[test]
    fn wp_c5_a_refused_save_still_releases_the_lock() {
        let spy = Spy::demo().answer(op::REQ_PRESET_SAVE, &[st::PRESET_ERR_FLASH_WRITE as u8]);
        let (core, _, log) = open(spy);
        let before = core.get_state();
        assert_eq!(
            core.save_preset(1),
            Err(DspiError::Refused {
                status: st::PRESET_ERR_FLASH_WRITE as u8
            })
        );
        bracketed(&log, "Saving preset");
        assert_eq!(core.get_state(), before);
        assert_eq!(core.lock().baseline, None);
    }

    #[test]
    fn wp_c5_a_refused_lock_sends_nothing() {
        for (refusal, want) in [
            (TransportError::Locked, DspiError::Locked),
            (TransportError::Denied("viewer".into()), DspiError::Denied),
        ] {
            let mut spy = Spy::demo();
            spy.refuse_lock = Some(refusal);
            let (core, remote, log) = open(spy);
            let image = remote.image();
            assert_eq!(core.save_preset(1), Err(want.clone()));
            assert_eq!(core.load_preset(0), Err(want.clone()));
            assert_eq!(core.copy_preset(0, 1), Err(want.clone()));
            assert_eq!(core.delete_preset(0), Err(want.clone()));
            assert_eq!(core.save_params(), Err(want.clone()));
            assert_eq!(core.factory_reset(), Err(want.clone()));
            let e = entries(&log);
            assert!(e.iter().all(|x| x.starts_with("lock ")), "{e:?}");
            assert_eq!(remote.image(), image);
        }
    }

    #[test]
    fn wp_c5_save_params_saves_and_rebaselines() {
        let spy = Spy::demo().answer(op::REQ_SAVE_PARAMS, &[FLASH_OK]);
        let (core, _, log) = open(spy);
        edit(&core, |s| s.bypass = !s.bypass);
        core.save_params().unwrap();
        bracketed(&log, "Saving to flash");
        assert!(sent(&log, "in", op::REQ_SAVE_PARAMS));
        assert!(!core.preset_dirty());

        let spy = Spy::demo().answer(op::REQ_SAVE_PARAMS, &[FLASH_ERR_WRITE]);
        let (core, _, log) = open(spy);
        assert_eq!(
            core.save_params(),
            Err(DspiError::Refused {
                status: FLASH_ERR_WRITE
            })
        );
        bracketed(&log, "Saving to flash");
        assert_eq!(core.lock().baseline, None);
    }

    #[test]
    fn wp_c5_factory_reset_rereads_everything() {
        let mut reset = Capture::demo().bulk;
        let out = dspi_proto::generated::OFF_OUTPUTS;
        let mut changed = decoded(&reset);
        changed.output_gain_db[0] = -42.0;
        reset = state::encode_bulk(&changed);
        assert_ne!(out, 0);

        let mut spy = Spy::demo().answer(op::REQ_FACTORY_RESET, &[FLASH_OK]);
        spy.reset_image = Some(reset.clone());
        let (core, _, log) = open(spy);
        core.factory_reset().unwrap();
        bracketed(&log, "Factory reset");
        assert!(sent(&log, "in", op::REQ_PRESET_GET_DIR), "fetch_all ran");
        let s = core.get_state();
        assert_eq!(s.output_gain_db[0], -42.0);
        assert_eq!(s.bulk_raw, reset);
        assert!(!core.preset_dirty());
        assert_eq!(core.lock().baseline.as_deref(), Some(&*reset));

        let spy = Spy::demo().answer(op::REQ_FACTORY_RESET, &[FLASH_ERR_WRITE]);
        let (core, _, log) = open(spy);
        assert_eq!(
            core.factory_reset(),
            Err(DspiError::Refused {
                status: FLASH_ERR_WRITE
            })
        );
        bracketed(&log, "Factory reset");
    }

    #[test]
    fn wp_c5_read_only_connections_write_nothing() {
        // A newer firmware's image: shown through its V32 prefix, read-only.
        let mut bulk = Capture::demo().bulk;
        bulk.resize(6200, 0);
        bulk[0] = 33;
        bulk[6..8].copy_from_slice(&6200u16.to_le_bytes());
        let cap = Capture {
            platform: Capture::demo().platform,
            bulk,
        };
        let spy = Spy::with(FakeDevice::new(cap, FakeOptions::default()));
        let (log, remote) = (Arc::clone(&spy.log), spy.remote.clone());
        let core = DspiCore::new();
        core.attach(Box::new(spy), "NEW".into(), 0);
        core.fetch_all().unwrap();
        assert_eq!(core.get_state().compat, COMPAT_FIRMWARE_TOO_NEW);
        log.lock().unwrap().clear();
        let image = remote.image();

        let ro = Err(DspiError::ReadOnly);
        assert_eq!(core.save_preset(1), ro);
        assert_eq!(core.load_preset(0), ro);
        assert_eq!(core.copy_preset(0, 1), ro);
        assert_eq!(core.delete_preset(0), ro);
        assert_eq!(core.set_preset_name(0, "x".into()), ro);
        assert_eq!(core.set_preset_startup(0, 0), ro);
        assert_eq!(core.save_params(), ro);
        assert_eq!(core.load_params(), ro);
        assert_eq!(core.factory_reset(), ro);
        assert!(entries(&log).is_empty(), "{:?}", entries(&log));
        assert_eq!(remote.image(), image);
        // Reads work.
        assert_eq!(core.get_preset_name(0).unwrap(), "Demo");
        assert_eq!(core.get_preset_directory().unwrap().occupied_mask, 1);
        // The baseline and diff work on the shown image.
        core.capture_baseline();
        assert!(!core.preset_dirty());
        edit(&core, |s| s.input_preamp_db[1] = 2.0);
        assert!(core.preset_dirty());
    }

    #[test]
    fn wp_c5_without_a_device() {
        let core = DspiCore::new();
        assert_eq!(core.save_preset(0), Err(DspiError::Disconnected));
        assert_eq!(core.load_preset(0), Err(DspiError::Disconnected));
        assert_eq!(core.get_preset_name(0), Err(DspiError::Disconnected));
        assert_eq!(core.get_preset_directory(), Err(DspiError::Disconnected));
        assert_eq!(core.factory_reset(), Err(DspiError::Disconnected));
        core.capture_baseline();
        assert_eq!(core.lock().baseline, None);
        assert!(!core.preset_dirty());
        assert!(core.preset_changes().is_empty());
    }

    #[test]
    fn wp_c5_no_baseline_means_no_changes() {
        let (core, _, _) = open(Spy::demo());
        edit(&core, |s| s.input_preamp_db[0] = -6.0);
        assert!(!core.preset_dirty());
        assert!(core.preset_changes().is_empty());
    }

    #[test]
    fn wp_c5_dirty_follows_the_image_and_the_live_modes() {
        let (core, _, _) = open(Spy::demo());
        core.capture_baseline();
        assert!(!core.preset_dirty());

        // Master volume: not part of the preset in the demo's INDEPENDENT
        // mode, part of it in WITH_PRESET mode (judged by the live mode).
        let mv = core.get_state().master_volume_db;
        edit(&core, |s| s.master_volume_db = mv - 7.5);
        assert!(!core.preset_dirty());
        core.lock().state.master_volume_mode = WITH_PRESET;
        assert!(core.preset_dirty());
        assert!(
            core.preset_changes()[0].starts_with("Master Volume: "),
            "{:?}",
            core.preset_changes()
        );
        core.lock().state.master_volume_mode = 0;

        // An edit and its undo: clean again.
        edit(&core, |s| s.master_volume_db = mv);
        let before = core.get_state().input_preamp_db[0];
        edit(&core, |s| s.input_preamp_db[0] = before - 3.0);
        assert!(core.preset_dirty());
        assert_eq!(
            core.preset_changes(),
            vec![format!("Preamp L: {} → {}", db(before), db(before - 3.0))]
        );
        edit(&core, |s| s.input_preamp_db[0] = before);
        assert!(!core.preset_dirty());
    }

    #[test]
    fn wp_c5_a_reencoded_image_is_not_a_change() {
        // The codec is decode-stable, not byte-identical: an image written
        // back from the mirror may differ from the device's in bytes no
        // field reads. That is no edit.
        let (core, _, _) = open(Spy::demo());
        core.capture_baseline();
        {
            let mut g = core.lock();
            let mut s = g.state.clone();
            s.bulk_raw.iter_mut().for_each(|b| *b = 0);
            let raw = g.state.bulk_raw.clone();
            s.bulk_raw[..8].copy_from_slice(&raw[..8]);
            g.state.bulk_raw = state::encode_bulk(&s);
        }
        assert!(
            core.preset_changes().is_empty(),
            "{:?}",
            core.preset_changes()
        );
        assert!(!core.preset_dirty());
    }

    #[test]
    fn wp_c5_diff_words_edits_as_the_console_does() {
        let mut a = decoded(&Capture::demo().bulk);
        a.num_input_channels = 2;
        a.num_output_channels = 4;
        a.output_config_mode = 0;
        let mut b = a.clone();
        assert!(diff(&a, &b).is_empty());

        b.filters[2][1].gain += 4.0;
        b.output_muted[1] = !a.output_muted[1];
        b.output_pins[0] = b.output_pins[0].wrapping_add(1);
        b.channel_names[0] = "Left".into();
        let lines = diff(&a, &b);
        let (out1, out2) = (channel_label(&b, 2), channel_label(&b, 3));
        assert!(
            lines.contains(&format!("1 band changed on {out1}")),
            "{lines:?}"
        );
        let muted = if b.output_muted[1] {
            "muted"
        } else {
            "unmuted"
        };
        assert!(lines.contains(&format!("{out2} {muted}")), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| *l == format!("'{}' → 'Left'", a.channel_names[0])),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("pin assignment")),
            "{lines:?}"
        );

        b.output_config_mode = WITH_PRESET;
        let lines = diff(&a, &b);
        assert!(
            lines.iter().any(|l| l == "1 pin assignment changed"),
            "{lines:?}"
        );

        // NaN is not a change; a Linkwitz Transform's Qp is.
        let mut c = a.clone();
        let mut d = a.clone();
        c.filters[0][0].freq = f32::NAN;
        d.filters[0][0].freq = f32::NAN;
        assert!(diff(&c, &d).is_empty());
        d.filters[0][0].qp = 3.0;
        assert!(
            diff(&c, &d).is_empty(),
            "Qp counts only for a Linkwitz Transform"
        );
        c.filters[0][0].filter_type = 11;
        d.filters[0][0].filter_type = 11;
        assert_eq!(diff(&c, &d).len(), 1);
    }

    /// A directory reply: occupied mask and active slot, WITH_PRESET output
    /// config, independent master volume.
    fn dir(mask: u8, active: u8) -> [u8; 7] {
        [mask, 0, 0, 0, active, WITH_PRESET, 0]
    }

    fn is_transfer<T: std::fmt::Debug>(r: Result<T>) {
        assert!(matches!(r, Err(DspiError::Transfer { .. })), "{r:?}");
    }

    #[test]
    fn wp_c5_an_unconfirmed_load_trusts_the_directory() {
        // Accepted, then the device stays on slot 0 (a CRC failure on its
        // main loop looks like this).
        let spy = Spy::demo()
            .answer(op::REQ_PRESET_LOAD, &[0])
            .answer(op::REQ_PRESET_GET_ACTIVE, &[0])
            .answer(op::REQ_PRESET_GET_DIR, &dir(0b101, 0));
        let (core, _, log) = open(spy);
        core.capture_baseline();
        let baseline = core.lock().baseline.clone();
        edit(&core, |s| s.bypass = !s.bypass);
        log.lock().unwrap().clear();

        is_transfer(core.load_preset(2));
        bracketed(&log, "Loading preset");
        assert!(
            !sent(&log, "in", op::REQ_GET_ALL_PARAMS_CHUNK),
            "no re-read"
        );
        assert_eq!(core.get_state().active_preset_slot, 0);
        assert_eq!(core.lock().baseline, baseline);
        assert!(core.preset_dirty(), "the edit is still unsaved");
    }

    #[test]
    fn wp_c5_a_confirmed_load_without_a_directory_takes_the_slot() {
        let mut spy = Spy::demo()
            .answer(op::REQ_PRESET_LOAD, &[0])
            .answer(op::REQ_PRESET_GET_ACTIVE, &[3]);
        spy.stalls.push(op::REQ_PRESET_GET_DIR);
        let (core, _, _) = open(spy);
        core.load_preset(3).unwrap();
        assert_eq!(core.get_state().active_preset_slot, 3);
        assert!(core.lock().baseline.is_some());
    }

    #[test]
    fn wp_c5_a_directory_naming_another_slot_wins_over_the_poll() {
        let spy = Spy::demo()
            .answer(op::REQ_PRESET_LOAD, &[0])
            .answer(op::REQ_PRESET_GET_ACTIVE, &[3])
            .answer(op::REQ_PRESET_GET_DIR, &dir(0b1001, 0));
        let (core, _, log) = open(spy);
        is_transfer(core.load_preset(3));
        assert_eq!(entries(&log).last().map(String::as_str), Some("unlock"));
        assert_eq!(core.get_state().active_preset_slot, 0);
        assert_eq!(core.lock().baseline, None);
    }

    #[test]
    fn wp_c5_an_unconfirmed_save_changes_nothing_but_the_directory() {
        let spy = Spy::demo()
            .answer(op::REQ_PRESET_SAVE, &[0])
            .answer(op::REQ_PRESET_GET_DIR, &dir(0b1, 0));
        let (core, _, log) = open(spy);
        is_transfer(core.save_preset(3));
        bracketed(&log, "Saving preset");
        let s = core.get_state();
        assert_eq!((s.active_preset_slot, s.preset_occupied), (0, 0b1));
        assert_eq!(core.lock().baseline, None);
    }

    #[test]
    fn wp_c5_an_unconfirmed_delete_keeps_the_slot_and_its_name() {
        let spy = Spy::demo()
            .answer(op::REQ_PRESET_DELETE, &[0])
            .answer(op::REQ_PRESET_GET_DIR, &dir(0b1, 0));
        let (core, _, log) = open(spy);
        let name = core.get_state().preset_names[0].clone();
        assert!(!name.is_empty());
        is_transfer(core.delete_preset(0));
        bracketed(&log, "Deleting preset");
        assert!(
            !sent(&log, "in", op::REQ_GET_ALL_PARAMS_CHUNK),
            "no re-read"
        );
        let s = core.get_state();
        assert_eq!(s.preset_occupied, 0b1);
        assert_eq!(s.preset_names[0], name);
    }

    #[test]
    fn wp_c5_an_unconfirmed_copy_stops_and_keeps_the_baseline() {
        // The first save never reports active: the source is not re-saved.
        let spy = Spy::demo().answer(op::REQ_PRESET_GET_ACTIVE, &[0]);
        let (core, _, log) = open(spy);
        is_transfer(core.copy_preset(0, 4));
        bracketed(&log, "Copying preset");
        let saves = entries(&log)
            .into_iter()
            .filter(|e| e.starts_with(&format!("in {:02X} ", op::REQ_PRESET_SAVE)))
            .count();
        assert_eq!(saves, 1);
        assert_eq!(core.lock().baseline, None);

        // The second save never reports active.
        let spy = Spy::demo().answer(op::REQ_PRESET_GET_ACTIVE, &[4]);
        let (core, _, log) = open(spy);
        is_transfer(core.copy_preset(0, 4));
        bracketed(&log, "Copying preset");
        assert_eq!(core.lock().baseline, None);
    }

    #[test]
    fn wp_c5_formatting_helpers() {
        assert_eq!(ms(0.0), "0");
        assert_eq!(ms(10.0), "10");
        assert_eq!(ms(1.02), "1.02");
        assert_eq!(ms(1.2346), "1.2346");
        assert_eq!(val(3.0), "3");
        assert_eq!(val(3.25), "3.2");
        assert_eq!(db(-3.0), "-3.0 dB");
    }
}
