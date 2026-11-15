//! Presets, the device lock and the unsaved-changes baseline.
//!
//! # Preset flows
//!
//! The firmware defers every flash operation to its main loop: `PRESET_SAVE`,
//! `PRESET_LOAD`, `PRESET_DELETE`, `SAVE_PARAMS` and `FACTORY_RESET` answer
//! "accepted" at once and do the work a little later. A flow that must know
//! the work is done asks the device until it says so, for at most
//! [`CONFIRM_WITHIN`]:
//!
//! - a save or a load is done when `PRESET_GET_ACTIVE` names the slot;
//! - a delete is done when the directory no longer marks the slot occupied.
//!
//! What a flow does when the answer never comes is part of its contract (see
//! each method). The multi-request flows run under the device lock, so no
//! other DSPi Link session writes in the middle of them.
//!
//! # The device lock
//!
//! [`DeviceLock`] is one flow's hold on the transport's lock
//! (`Transport::lock`, a no-op except over DSPi Link), or on the Link side
//! channel's ([`SideChannel::lock`]), which is taken with the core lock
//! released. A lock is tied to the connection it was taken on by that
//! connection's generation: releasing it never touches a connection that
//! replaced the one it locked.
//!
//! A lock may also be kept past the call that took it ([`HeldLock`] on the
//! connection): the caller's own (`device_lock_acquire`), an IR learn
//! window, the bootloader restart. While one is held, the core's own flows
//! take nothing of their own: their lock is *nested*, and its release gives
//! nothing back. A held lock ends when its holder ends it, when it runs out
//! (it is given back before the next flow takes the lock), or with its
//! connection.
//!
//! # Unsaved changes
//!
//! The baseline is the bulk image the active preset holds as far as this
//! client knows: captured after a connect, a load, a save, a revert or a
//! factory reset. [`DspiCore::preset_changes`] decodes it and lists what the
//! live mirror does differently, in the Mac console's words and order
//! (`PresetSnapshot.diff`); the preset is dirty exactly when that list is
//! not empty.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use dspi_proto::FilterType;
use dspi_proto::generated::{self as g, opcodes as op, status as st};
use dspi_transport::clock::{self, Instant};

use crate::confirm::ConfirmOp; // -- WP P1TK --
use crate::consts::{
    BANDS_PER_CHANNEL, CHANNEL_NAME_LEN, CROSSFEED_MAP_PAIRS, FLASH_ERR_CRC, FLASH_ERR_WRITE,
    MAX_PRESETS, MAX_XOVER_BANDS, SUBHARM_PARAM_BOOST, SUBHARM_PARAM_CEILING, SUBHARM_PARAM_DEPTH,
    SUBHARM_PARAM_ENABLED, SUBHARM_PARAM_HIGH, SUBHARM_PARAM_HOLD, SUBHARM_PARAM_LINK,
    SUBHARM_PARAM_LOW, SUBHARM_PARAM_MASK, SUBHARM_PARAM_SELECT, SUBHARM_PARAM_TOP,
    UPMIX_PARAM_ATTACK, UPMIX_PARAM_CENTER_MODE, UPMIX_PARAM_DECORRELATION,
    UPMIX_PARAM_DETECTOR_HPF, UPMIX_PARAM_ENABLED, UPMIX_PARAM_PRESENCE, UPMIX_PARAM_RELEASE,
    UPMIX_PARAM_STRENGTH, UPMIX_PARAM_SURROUND_DELAY, UPMIX_PARAM_SURROUND_HPF,
    UPMIX_PARAM_SURROUND_LPF, UPMIX_PARAM_SURROUND_MODE, UPMIX_PARAM_THRESHOLD, UPMIX_PARAM_WIDTH,
};
use crate::error::status;
use crate::journal::journal;
use crate::records::{DspState, FilterParams, PresetDirectory};
use crate::{Connection, DspiCore, DspiError, Inner, Result, SideChannel, connect, state};
use dspi_proto::value::Value;

/// How long a flow waits for the device to finish a deferred flash operation.
const CONFIRM_WITHIN: Duration = Duration::from_millis(1500);
/// The pause between two questions while it waits.
const CONFIRM_EVERY: Duration = Duration::from_millis(20);

/// The lease a flow's own lock asks for (a hub ends it sooner on release).
const FLOW_LEASE: Duration = Duration::from_secs(60);
/// The longest a caller may hold the lock (`device_lock_acquire`).
const CALLER_LEASE_MAX_MS: u32 = 60_000;

const SAVE_REASON: &str = "Saving preset";
const LOAD_REASON: &str = "Loading preset";
const COPY_REASON: &str = "Copying preset";
const DELETE_REASON: &str = "Deleting preset";
const SAVE_PARAMS_REASON: &str = "Saving to flash";
const RESET_REASON: &str = "Restoring factory defaults";
// -- WP CLEAN1 fixup cl1 --
/// How long the firmware may take to apply a factory reset: it answers the
/// request at once and resets in its main loop, so a read straight after the
/// answer can still see the old state.
const FACTORY_RESET_SETTLE: Duration = Duration::from_millis(250);

/// `PRESET_GET_DIR`'s reply length (the master-volume mode is its last byte).
const DIRECTORY_LEN: u16 = 7;

// ── The device lock ───────────────────────────────────────────────────

/// One flow's hold on the device lock. Give it back with
/// [`DspiCore::release_device_lock`] on every path, or keep it with
/// [`DspiCore::hold_device_lock`].
#[must_use = "release it on every path"]
pub(crate) struct DeviceLock {
    /// The connection it was taken on.
    generation: u64,
    /// The Link side channel it was taken through; `None` for the
    /// transport's own lock (or a nested one).
    side: Option<Arc<dyn SideChannel>>,
    /// Taken inside a held lock: nothing was asked, nothing goes back.
    nested: bool,
}

/// Who keeps a [`HeldLock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holder {
    /// `device_lock_acquire`; `device_lock_release` ends it.
    Caller,
    /// An IR learn window.
    Learn,
    /// The bootloader restart: it lasts until the device leaves.
    Bootloader,
}

/// A device lock kept on its connection past the call that took it.
pub(crate) struct HeldLock {
    /// `None` once given back.
    lock: Option<DeviceLock>,
    /// When it runs out.
    until: Instant,
    pub(crate) holder: Holder,
}

impl HeldLock {
    /// Still in force at `now`.
    fn in_force(&self, now: Instant) -> bool {
        now < self.until
    }

    /// Give back a transport lock now, on `conn` (its own connection). A side
    /// channel's goes back when this is dropped, which the caller does with
    /// the core lock released.
    fn give_back_transport(&mut self, conn: &mut Connection) {
        if self.lock.as_ref().is_some_and(|l| l.side.is_none()) {
            self.lock = None;
            conn.transport().unlock();
        }
    }
}

impl Drop for HeldLock {
    /// A side channel's lock goes back with its holder; a transport's goes
    /// with the transport (a connection that closes gives it back).
    fn drop(&mut self) {
        if let Some(DeviceLock {
            side: Some(side),
            nested: false,
            ..
        }) = self.lock.take()
        {
            side.unlock();
        }
    }
}

impl DeviceLock {
    /// Whether `inner`'s connection is still the one this lock was taken on.
    pub(crate) fn is_current(&self, inner: &Inner) -> bool {
        inner
            .conn
            .as_ref()
            .is_some_and(|c| c.generation == self.generation)
    }
}

// ── Presets ───────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    // -- WP P1TK --
    /// Save the live state to `slot` (0..10) and wait until the device has
    /// written it (it names the slot active). Runs under the device lock.
    /// The slot becomes the active one and the live state its baseline.
    ///
    /// Without that confirmation the mirror only takes the directory as the
    /// device reports it, the baseline stays, and the call fails.
    ///
    /// Errors: [`DspiError::Refused`] with `PRESET_ERR_INVALID_SLOT` for a
    /// slot past the last (nothing is sent) or with the device's `PRESET_*`
    /// status; [`DspiError::Transfer`] when the save was not confirmed;
    /// [`DspiError::Locked`] / [`DspiError::Denied`] (Link);
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`].
    ///
    /// Takes a confirmation token ([`crate::confirm`], WP P1TK; required since
    /// WP P1TZ): [`DspiError::ConfirmationRequired`], with nothing sent,
    /// without a good one. There is no form without a token.
    pub fn save_preset_confirmed(&self, slot: u8, token: Option<String>) -> Result<()> {
        self.save_preset_with(slot, token.as_deref())
    }

    /// Load preset `slot`, wait until the device runs it, then read the
    /// image and the directory again; the loaded state becomes the baseline.
    /// Runs under the device lock.
    ///
    /// The active slot is the directory's after the re-read. When the
    /// directory cannot be read, it is `slot` if the device confirmed the
    /// load, and stays as it was otherwise. A load that is never confirmed
    /// (an empty slot changes nothing) still re-reads and succeeds.
    ///
    /// Errors: as [`DspiCore::save_preset_confirmed`], and the re-read's.
    pub fn load_preset(&self, slot: u8) -> Result<()> {
        let _lock = crate::limits::LockScope::allow(); // -- WP P1LK -- allowed while locked
        slot_in_range(slot)?;
        let began = Instant::now(); // -- WP ECHO1 --
        let lines = self.with_device_lock(LOAD_REASON, |inner| {
            let before = inner.state.clone(); // -- WP P0M --
            let confirmed = {
                let mut w = inner.writable_conn()?;
                w.conn().fresh_image = false;
                // -- WP ECHO1 -- a whole image: an own echo of a write sent
                // before it is older than the preset
                w.conn().echo.whole_image_sent(Instant::now());
                let sent = status(w.set_as_read(op::REQ_PRESET_LOAD, u16::from(slot))?)
                    .and_then(|()| wait_for(w.conn(), |c| names_active(c, slot)));
                w.conn().echo.whole_image_done(Instant::now());
                sent?
            };
            // -- WP P0W -- the history before a load is gone with the state
            journal(inner, "preset.load", &[slot], Value::Trigger);
            connect::refresh_params_locked(inner)?;
            if !refresh_directory(inner) && confirmed {
                inner.state.active_preset_slot = slot;
            }
            capture_baseline_locked(inner);
            // -- WP P0M -- what changed, for the event log
            let names = inner.conn.as_ref().map(|c| c.source_names.to_vec());
            Ok(changes(
                &before,
                &inner.state,
                names.as_deref().unwrap_or(&[]),
            ))
        })?;
        // -- WP P0M -- written once `Inner` is released
        self.events.applied(
            dspi_session::events::EventKind::Recall,
            Some(slot),
            &format!("Recalled preset {}", u16::from(slot).saturating_add(1)),
            lines,
        );
        // -- WP P1LM -- the slot's stored values, read back, held to the
        // limits profile (OD-P1j); reported in `limits_status`
        self.limits_after_load();
        self.whole_image_ended(began); // -- WP ECHO1 --
        Ok(())
    }

    // -- WP P1TK --
    /// Copy the live state to `dest` and save it to `source` again, so
    /// `source` stays the active preset (the Mac console's "Copy to"). Each
    /// save waits for the device; one device lock covers both.
    ///
    /// If the save to `dest` is not confirmed, nothing more is sent and the
    /// baseline stays. After the second save, the live state is the
    /// baseline.
    ///
    /// Errors: the first failing save's (`Refused` with its `PRESET_*`
    /// status); [`DspiError::Transfer`] when `dest`'s save was not
    /// confirmed; otherwise as [`DspiCore::save_preset_confirmed`].
    ///
    /// Takes a confirmation token ([`crate::confirm`], WP P1TK; required since
    /// WP P1TZ): [`DspiError::ConfirmationRequired`], with nothing sent,
    /// without a good one. There is no form without a token.
    pub fn copy_preset_confirmed(&self, source: u8, dest: u8, token: Option<String>) -> Result<()> {
        self.copy_preset_with(source, dest, token.as_deref())
    }

    // -- WP P1TK --
    /// Delete preset `slot` and wait until the directory no longer holds
    /// it; its name goes with it. Runs under the device lock.
    ///
    /// Deleting the active slot makes the device load its factory defaults:
    /// the image is read again and becomes the baseline. A deletion that is
    /// never confirmed leaves the slot and its name in the mirror and fails.
    ///
    /// Errors: as [`DspiCore::save_preset_confirmed`].
    ///
    /// Takes a confirmation token ([`crate::confirm`], WP P1TK; required since
    /// WP P1TZ): [`DspiError::ConfirmationRequired`], with nothing sent,
    /// without a good one. There is no form without a token.
    pub fn delete_preset_confirmed(&self, slot: u8, token: Option<String>) -> Result<()> {
        self.delete_preset_with(slot, token.as_deref())
    }

    /// Name preset `slot`: at most 31 bytes of `name` (cut at a character
    /// boundary), sent NUL-terminated. The mirror takes the name as sent.
    ///
    /// Errors: `Refused` with `PRESET_ERR_INVALID_SLOT` for a slot past the
    /// last (nothing is sent); [`DspiError::ReadOnly`];
    /// [`DspiError::Disconnected`]; a transfer error.
    pub fn set_preset_name(&self, slot: u8, name: String) -> Result<()> {
        slot_in_range(slot)?;
        let name = name_within(&name);
        let mut payload = Vec::with_capacity(name.len().saturating_add(1));
        payload.extend_from_slice(name.as_bytes());
        payload.push(0);
        let mut inner = self.lock();
        let change = inner.journal_begin("preset.name", &[slot])?; // -- WP P0W --
        inner
            .writable_conn()?
            .set(op::REQ_PRESET_SET_NAME, u16::from(slot), &payload)?;
        let names = &mut inner.state.preset_names;
        if names.len() < MAX_PRESETS {
            names.resize(MAX_PRESETS, String::new());
        }
        if let Some(n) = names.get_mut(usize::from(slot)) {
            *n = name.to_owned();
        }
        change.done(&mut inner, Value::Text(name.to_owned()));
        Ok(())
    }

    /// Read preset `slot`'s name from the device (also into the mirror). An
    /// empty slot has no name: the device stalls, and that is the error.
    ///
    /// Errors: `Refused` with `PRESET_ERR_INVALID_SLOT` for a slot past the
    /// last (nothing is sent); [`DspiError::Disconnected`]; a transfer error.
    pub fn get_preset_name(&self, slot: u8) -> Result<String> {
        slot_in_range(slot)?;
        let mut guard = self.lock();
        let Inner { conn, state, .. } = &mut *guard;
        let conn = conn.as_mut().ok_or(DspiError::Disconnected)?;
        connect::read_preset_name(conn, state, slot)
    }

    /// Read the preset directory (also into the mirror: occupied slots,
    /// startup mode, default and active slot, both persistence modes).
    ///
    /// Errors: [`DspiError::Disconnected`]; a transfer error or a short reply.
    pub fn get_preset_directory(&self) -> Result<PresetDirectory> {
        let mut guard = self.lock();
        let Inner { conn, state, .. } = &mut *guard;
        let conn = conn.as_mut().ok_or(DspiError::Disconnected)?;
        connect::read_preset_directory(conn, state)
    }

    /// What the device loads at boot: `mode` 0 (`PRESET_STARTUP_SPECIFIED`)
    /// loads `default_slot`, 1 (`PRESET_STARTUP_LAST_ACTIVE`) the slot last
    /// active. The mirror takes both.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another mode or a slot
    /// past the last (nothing is sent); [`DspiError::ReadOnly`];
    /// [`DspiError::Disconnected`]; a transfer error.
    pub fn set_preset_startup(&self, mode: u8, default_slot: u8) -> Result<()> {
        if u16::from(mode) > st::PRESET_STARTUP_LAST_ACTIVE {
            return Err(DspiError::InvalidArgument {
                msg: format!("preset startup mode {mode} is neither 0 (a slot) nor 1 (the last)"),
            });
        }
        if usize::from(default_slot) >= MAX_PRESETS {
            return Err(DspiError::InvalidArgument {
                msg: format!("default preset slot {default_slot} is past the last ({MAX_PRESETS})"),
            });
        }
        let mut inner = self.lock();
        let change = inner.journal_begin("preset.startup", &[])?; // -- WP P0W --
        {
            let mut w = inner.writable_conn()?;
            w.conn().fresh_image = false;
            w.set(op::REQ_PRESET_SET_STARTUP, 0, &[mode, default_slot])?;
        }
        inner.state.preset_startup_mode = mode;
        inner.state.preset_default_slot = default_slot;
        change.done(&mut inner, Value::Bytes(vec![mode, default_slot]));
        Ok(())
    }

    // -- WP P1TK --
    /// Save the live state to the active slot (`REQ_SAVE_PARAMS`), under the
    /// device lock; the live state becomes the baseline.
    ///
    /// Errors: `Refused` with the device's `FLASH_*` status;
    /// [`DspiError::Locked`] / [`DspiError::Denied`] (Link);
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`]; a transfer
    /// error.
    ///
    /// Takes a confirmation token ([`crate::confirm`], WP P1TK; required since
    /// WP P1TZ): [`DspiError::ConfirmationRequired`], with nothing sent,
    /// without a good one. There is no form without a token.
    pub fn save_params_confirmed(&self, token: Option<String>) -> Result<()> {
        self.save_params_with(token.as_deref())
    }

    /// Revert to saved: load the active slot again ([`DspiCore::load_preset`],
    /// the device's deferred load, safe on every input).
    ///
    /// Errors: as `load_preset`, but a refusal carries a `FLASH_*` code:
    /// `FLASH_ERR_CRC` for a slot that failed its check, `FLASH_ERR_WRITE`
    /// for any other.
    pub fn load_params(&self) -> Result<()> {
        let slot = self.lock().state.active_preset_slot;
        self.load_preset(slot).map_err(|e| match e {
            DspiError::Refused { status } => DspiError::Refused {
                status: if u16::from(status) == st::PRESET_ERR_CRC {
                    FLASH_ERR_CRC
                } else {
                    FLASH_ERR_WRITE
                },
            },
            other => other,
        })
    }

    // -- WP P1TK --
    /// Restore the factory defaults on the live state (stored presets stay),
    /// give the device [`FACTORY_RESET_SETTLE`] to apply them (it defers the
    /// reset to its main loop), then read everything again as
    /// [`DspiCore::fetch_all`] does; the result is the baseline. Runs under the
    /// device lock; the core lock is not held while it waits.
    ///
    /// Errors: `Refused` with the device's `FLASH_*` status;
    /// [`DspiError::Locked`] / [`DspiError::Denied`] (Link);
    /// [`DspiError::ReadOnly`]; [`DspiError::Disconnected`]; a transfer
    /// error, of the reset or the re-read.
    ///
    /// Takes a confirmation token ([`crate::confirm`], WP P1TK; required since
    /// WP P1TZ): [`DspiError::ConfirmationRequired`], with nothing sent,
    /// without a good one. There is no form without a token.
    pub fn factory_reset_confirmed(&self, token: Option<String>) -> Result<()> {
        self.factory_reset_with(token.as_deref())
    }

    /// Take the live state as what the active preset holds: from now on
    /// [`DspiCore::preset_changes`] lists what differs from it. The core
    /// does this itself after a connect, a load, a save, a revert and a
    /// factory reset. Without a device, or without an image in this build's
    /// format, there is no baseline.
    pub fn capture_baseline(&self) {
        capture_baseline_locked(&mut self.lock());
    }

    /// Whether the live state differs from the active preset: exactly when
    /// [`DspiCore::preset_changes`] has something to say. No I/O.
    pub fn preset_dirty(&self) -> bool {
        !self.preset_changes().is_empty()
    }

    /// What differs from the active preset, one sentence each, in the Mac
    /// console's words and order. Empty without a device or a baseline. No
    /// I/O: input source titles come from the names the core keeps.
    pub fn preset_changes(&self) -> Vec<String> {
        let inner = self.lock();
        let (Some(conn), Some(base)) = (inner.conn.as_ref(), inner.baseline.as_deref()) else {
            return Vec::new();
        };
        if !inner.state.bulk_valid {
            return Vec::new();
        }
        // Start from the live state, so what the image does not carry (the
        // persistence modes) is the live one, as the console gates on it.
        let mut saved = inner.state.clone();
        if let Err(e) = state::decode_bulk(base, &mut saved) {
            log::debug!("the preset baseline does not decode: {e}");
            return Vec::new();
        }
        changes(&saved, &inner.state, &conn.source_names)
    }
}

// ── The caller's lock ─────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Hold the device lock across a flow of several calls (a restore, a
    /// firmware install), for at most `timeout_ms` (capped at 60 000), until
    /// [`DspiCore::device_lock_release`]: another DSPi Link session's
    /// commands answer locked meanwhile. The core's own flows nest inside it.
    /// Blocking over Link: call it off the UI thread.
    ///
    /// An acquire while a lock is held asks nothing more; one held by an IR
    /// learn window or the bootloader restart becomes the caller's. Elsewhere
    /// than over Link it succeeds without asking anyone.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a zero timeout;
    /// [`DspiError::Disconnected`]; [`DspiError::Locked`] while another
    /// session holds it; [`DspiError::Denied`].
    pub fn device_lock_acquire(&self, reason: String, timeout_ms: u32) -> Result<()> {
        if timeout_ms == 0 {
            return Err(DspiError::InvalidArgument {
                msg: "a device lock needs a timeout".into(),
            });
        }
        let lease = Duration::from_millis(u64::from(timeout_ms.min(CALLER_LEASE_MAX_MS)));
        {
            let mut inner = self.lock();
            let conn = inner.conn()?;
            if let Some(held) = conn.held_lock.as_mut()
                && held.in_force(Instant::now())
            {
                held.holder = Holder::Caller;
                return Ok(());
            }
        }
        let lock = self.take_device_lock(&reason, lease, false)?;
        self.hold_device_lock(lock, lease, Holder::Caller);
        Ok(())
    }

    /// Give back the lock [`DspiCore::device_lock_acquire`] took, and only
    /// that one: an IR learn window or the bootloader restart ends on its own
    /// terms. Idempotent; never blocks.
    pub fn device_lock_release(&self) {
        self.end_held_lock(Some(Holder::Caller));
    }
}

impl DspiCore {
    // -- WP P1TK --
    /// The body of [`DspiCore::save_preset_confirmed`] (WP P1TK).
    pub(crate) fn save_preset_with(&self, slot: u8, token: Option<&str>) -> Result<()> {
        slot_in_range(slot)?;
        let ticket = self.confirm_gate(ConfirmOp::SavePreset, token)?;
        self.with_device_lock(SAVE_REASON, |inner| {
            ticket.still_on(inner)?;
            let confirmed = save_to_slot(inner, slot)?;
            refresh_directory(inner);
            if !confirmed {
                return Err(unconfirmed("save to", slot));
            }
            mark_saved(&mut inner.state, slot);
            capture_baseline_locked(inner);
            journal(inner, "preset.save", &[slot], Value::Trigger); // -- WP P0W --
            Ok(())
        })
    }

    /// The body of [`DspiCore::copy_preset_confirmed`] (WP P1TK).
    pub(crate) fn copy_preset_with(&self, source: u8, dest: u8, token: Option<&str>) -> Result<()> {
        slot_in_range(source)?;
        slot_in_range(dest)?;
        let ticket = self.confirm_gate(ConfirmOp::CopyPreset, token)?;
        self.with_device_lock(COPY_REASON, |inner| {
            ticket.still_on(inner)?;
            if !save_to_slot(inner, dest)? {
                refresh_directory(inner);
                return Err(unconfirmed("copy to", dest));
            }
            mark_saved(&mut inner.state, dest);
            let back = save_to_slot(inner, source)?;
            refresh_directory(inner);
            if back {
                mark_saved(&mut inner.state, source);
            } else {
                log::warn!("the device did not confirm saving preset {source} again");
            }
            capture_baseline_locked(inner);
            Ok(())
        })
    }

    /// The body of [`DspiCore::delete_preset_confirmed`] (WP P1TK).
    pub(crate) fn delete_preset_with(&self, slot: u8, token: Option<&str>) -> Result<()> {
        slot_in_range(slot)?;
        let ticket = self.confirm_gate(ConfirmOp::DeletePreset, token)?;
        self.with_device_lock(DELETE_REASON, |inner| {
            ticket.still_on(inner)?;
            let was_active = inner.state.active_preset_slot == slot;
            let confirmed = {
                let mut w = inner.writable_conn()?;
                w.conn().fresh_image = false;
                status(w.set_as_read(op::REQ_PRESET_DELETE, u16::from(slot))?)?;
                wait_for(w.conn(), |c| slot_cleared(c, slot))?
            };
            if !confirmed {
                return Err(unconfirmed("delete", slot));
            }
            journal(inner, "preset.delete", &[slot], Value::Trigger); // -- WP P0W --
            inner.state.preset_occupied &= !bit(slot);
            if let Some(name) = inner.state.preset_names.get_mut(usize::from(slot)) {
                name.clear();
            }
            refresh_directory(inner);
            if was_active {
                connect::refresh_params_locked(inner)?;
                capture_baseline_locked(inner);
            }
            Ok(())
        })
    }

    /// The body of [`DspiCore::save_params_confirmed`] (WP P1TK).
    pub(crate) fn save_params_with(&self, token: Option<&str>) -> Result<()> {
        let ticket = self.confirm_gate(ConfirmOp::SaveParams, token)?;
        self.with_device_lock(SAVE_PARAMS_REASON, |inner| {
            ticket.still_on(inner)?;
            status(inner.writable_conn()?.set_as_read(op::REQ_SAVE_PARAMS, 0)?)?;
            journal(inner, "dev.save", &[], Value::Trigger); // -- WP P0W --
            let active = inner.state.active_preset_slot;
            if !refresh_directory(inner) {
                mark_saved(&mut inner.state, active);
            }
            capture_baseline_locked(inner);
            Ok(())
        })
    }

    /// The body of [`DspiCore::factory_reset_confirmed`] (WP P1TK).
    pub(crate) fn factory_reset_with(&self, token: Option<&str>) -> Result<()> {
        let ticket = self.confirm_gate(ConfirmOp::FactoryReset, token)?;
        let lock = self.acquire_device_lock(RESET_REASON)?;
        let began = Instant::now(); // -- WP ECHO1 --
        let sent = {
            let mut guard = self.lock();
            if lock.is_current(&guard) {
                ticket
                    .still_on(&guard)
                    .and_then(|()| guard.writable_conn())
                    .and_then(|mut w| {
                        w.conn().fresh_image = false;
                        // -- WP ECHO1 -- a whole image, as a preset load; its
                        // late echoes are expected until the re-read below
                        // and LATE_ECHO after the reply
                        w.conn().echo.whole_image_sent(Instant::now());
                        let sent = w.set_as_read(op::REQ_FACTORY_RESET, 0);
                        w.conn().echo.whole_image_done(Instant::now());
                        status(sent?)
                    })
                    // -- WP P0W -- the history before a reset is gone with the state
                    .map(|()| journal(&mut guard, "dev.reset", &[], Value::Trigger))
            } else {
                Err(DspiError::Disconnected)
            }
        };
        let result = sent.and_then(|()| {
            clock::sleep(FACTORY_RESET_SETTLE);
            self.fetch_all()?;
            // -- WP SWAPFIX -- the re-read keeps a baseline; the reset's is new
            self.capture_baseline();
            Ok(())
        });
        self.whole_image_ended(began); // -- WP ECHO1 --
        self.release_device_lock(lock);
        result
    }

    /// Run `f` with the core lock held, inside the device lock (`reason`
    /// tells other sessions why). `f` runs only if the connection is still
    /// the one locked; the lock goes back whatever `f` returns.
    ///
    /// Errors: the lock's ([`DspiCore::acquire_device_lock`]), `f`'s, or
    /// [`DspiError::Disconnected`] when the connection changed meanwhile.
    pub(crate) fn with_device_lock<T>(
        &self,
        reason: &str,
        f: impl FnOnce(&mut Inner) -> Result<T>,
    ) -> Result<T> {
        let lock = self.acquire_device_lock(reason)?;
        let result = {
            let mut guard = self.lock();
            if lock.is_current(&guard) {
                f(&mut guard)
            } else {
                Err(DspiError::Disconnected)
            }
        };
        self.release_device_lock(lock);
        result
    }

    /// The device lock for one flow that writes. Call with the core lock
    /// released.
    ///
    /// Errors: [`DspiError::ReadOnly`] (nothing is asked);
    /// [`DspiError::Disconnected`]; the lock's refusal.
    pub(crate) fn acquire_device_lock(&self, reason: &str) -> Result<DeviceLock> {
        self.take_device_lock(reason, FLOW_LEASE, true)
    }

    /// The device lock for `timeout` (the lease a hub grants), on the open
    /// connection; with `writable`, only on one whose state may be written.
    /// Inside a held lock it is nested: nothing is asked. A held lock that
    /// ran out is given back first. Over DSPi Link the hub is asked with the
    /// core lock released. Call with the core lock released.
    ///
    /// Errors: [`DspiError::ReadOnly`] (with `writable`);
    /// [`DspiError::Disconnected`]; [`DspiError::Locked`] /
    /// [`DspiError::Denied`] and the transport's other refusals.
    pub(crate) fn take_device_lock(
        &self,
        reason: &str,
        timeout: Duration,
        writable: bool,
    ) -> Result<DeviceLock> {
        let (generation, side, transport_locked, run_out) = {
            let mut guard = self.lock();
            let inner = &mut *guard;
            if writable {
                inner.writable_conn()?;
            }
            let conn = inner.conn()?;
            let generation = conn.generation;
            let mut run_out = None;
            if let Some(held) = conn.held_lock.as_ref() {
                if held.in_force(Instant::now()) {
                    return Ok(DeviceLock {
                        generation,
                        side: None,
                        nested: true,
                    });
                }
                run_out = conn.held_lock.take();
            }
            if let Some(held) = run_out.as_mut() {
                held.give_back_transport(conn);
            }
            match conn.side.clone() {
                Some(side) => (generation, Some(side), Ok(()), run_out),
                None => {
                    let locked = conn.transport().lock(reason, timeout);
                    (generation, None, locked, run_out)
                }
            }
        };
        // A side channel's expired lock goes back before it is asked again.
        drop(run_out);
        match side {
            Some(side) => {
                side.lock(reason, timeout)?;
                Ok(DeviceLock {
                    generation,
                    side: Some(side),
                    nested: false,
                })
            }
            None => {
                transport_locked?;
                Ok(DeviceLock {
                    generation,
                    side: None,
                    nested: false,
                })
            }
        }
    }

    /// Give `lock` back: through its side channel, or to its connection's
    /// transport if that connection is still open (never to one that
    /// replaced it). A nested lock gives nothing back. Call with the core
    /// lock released.
    pub(crate) fn release_device_lock(&self, lock: DeviceLock) {
        let DeviceLock {
            generation,
            side,
            nested,
        } = lock;
        if nested {
            return;
        }
        if let Some(side) = side {
            side.unlock();
            return;
        }
        let mut inner = self.lock();
        if let Some(conn) = inner.conn.as_mut()
            && conn.generation == generation
        {
            conn.transport().unlock();
        }
    }

    /// Keep `lock` on its connection for `hold`, for `holder`. A nested lock
    /// is the holder's already: nothing changes. A lock whose connection is
    /// gone is given back at once. Call with the core lock released.
    pub(crate) fn hold_device_lock(&self, lock: DeviceLock, hold: Duration, holder: Holder) {
        if lock.nested {
            return;
        }
        let now = Instant::now();
        let (unused, replaced) = {
            let mut guard = self.lock();
            match guard.conn.as_mut() {
                Some(conn) if conn.generation == lock.generation => {
                    let mut replaced = conn.held_lock.replace(HeldLock {
                        lock: Some(lock),
                        until: now.checked_add(hold).unwrap_or(now),
                        holder,
                    });
                    if let Some(old) = replaced.as_mut() {
                        old.give_back_transport(conn);
                    }
                    (None, replaced)
                }
                _ => (Some(lock), None),
            }
        };
        drop(replaced);
        if let Some(lock) = unused {
            self.release_device_lock(lock);
        }
    }

    /// End the held lock if `holder` keeps it (`None`: whoever does), giving
    /// it back. Call with the core lock released.
    pub(crate) fn end_held_lock(&self, holder: Option<Holder>) {
        let ended = {
            let mut guard = self.lock();
            let Some(conn) = guard.conn.as_mut() else {
                return;
            };
            let ours = conn
                .held_lock
                .as_ref()
                .is_some_and(|h| holder.is_none_or(|want| h.holder == want));
            if !ours {
                return;
            }
            let mut ended = conn.held_lock.take();
            if let Some(h) = ended.as_mut() {
                h.give_back_transport(conn);
            }
            ended
        };
        drop(ended);
    }

    /// Let the held lock run out now, as its lease would (its holder stays
    /// until the next flow gives it back).
    #[cfg(test)]
    pub(crate) fn expire_held_lock(&self) {
        let now = Instant::now();
        if let Some(held) = self.lock().conn.as_mut().and_then(|c| c.held_lock.as_mut()) {
            held.until = now;
        }
    }

    /// Who keeps the lock in force on the open connection, if anyone.
    #[cfg(test)]
    pub(crate) fn held_lock_holder(&self) -> Option<Holder> {
        let now = Instant::now();
        self.lock()
            .conn
            .as_ref()
            .and_then(|c| c.held_lock.as_ref())
            .filter(|h| h.in_force(now))
            .map(|h| h.holder)
    }
}

// ── Flow steps ────────────────────────────────────────────────────────

/// `Refused` with `PRESET_ERR_INVALID_SLOT` for a slot past the last.
fn slot_in_range(slot: u8) -> Result<()> {
    if usize::from(slot) < MAX_PRESETS {
        Ok(())
    } else {
        Err(DspiError::Refused {
            status: st::PRESET_ERR_INVALID_SLOT as u8,
        })
    }
}

/// The directory's bit for `slot`.
fn bit(slot: u8) -> u16 {
    1u16.checked_shl(u32::from(slot)).unwrap_or(0)
}

fn unconfirmed(what: &str, slot: u8) -> DspiError {
    DspiError::Transfer {
        msg: format!(
            "the device did not confirm the {what} preset {slot} within {} ms",
            CONFIRM_WITHIN.as_millis()
        ),
    }
}

/// Send `PRESET_SAVE` for `slot` and wait until the device names it active:
/// whether it did.
fn save_to_slot(inner: &mut Inner, slot: u8) -> Result<bool> {
    let mut w = inner.writable_conn()?;
    status(w.set_as_read(op::REQ_PRESET_SAVE, u16::from(slot))?)?;
    wait_for(w.conn(), |c| names_active(c, slot))
}

/// Ask `done` until it says yes or [`CONFIRM_WITHIN`] has passed: whether it
/// did. An answer that cannot be read is a "not yet", but a device that went
/// away ends the wait with [`DspiError::Disconnected`].
fn wait_for(
    conn: &mut Connection,
    mut done: impl FnMut(&mut Connection) -> Result<bool>,
) -> Result<bool> {
    let start = Instant::now();
    loop {
        match done(conn) {
            Ok(true) => return Ok(true),
            Err(DspiError::Disconnected) => return Err(DspiError::Disconnected),
            Ok(false) | Err(_) => {}
        }
        if start.elapsed() >= CONFIRM_WITHIN {
            return Ok(false);
        }
        clock::sleep(CONFIRM_EVERY);
    }
}

/// Whether `PRESET_GET_ACTIVE` names `slot`.
fn names_active(conn: &mut Connection, slot: u8) -> Result<bool> {
    let reply = conn.get_upto(op::REQ_PRESET_GET_ACTIVE, 0, 1)?;
    Ok(reply.first() == Some(&slot))
}

/// Whether the directory no longer marks `slot` occupied.
fn slot_cleared(conn: &mut Connection, slot: u8) -> Result<bool> {
    Ok(occupied(conn)?.is_some_and(|mask| mask & bit(slot) == 0))
}

/// The directory's occupied mask; `None` for a reply too short to hold it.
fn occupied(conn: &mut Connection) -> Result<Option<u16>> {
    let d = conn.get_upto(op::REQ_PRESET_GET_DIR, 0, DIRECTORY_LEN)?;
    Ok(match d.as_slice() {
        [lo, hi, ..] => Some(u16::from_le_bytes([*lo, *hi])),
        _ => None,
    })
}

/// Read the directory into the mirror; whether that worked. A failure is
/// logged and leaves the mirror as it was.
fn refresh_directory(inner: &mut Inner) -> bool {
    let Inner { conn, state, .. } = inner;
    let Some(conn) = conn.as_mut() else {
        return false;
    };
    match connect::read_preset_directory(conn, state) {
        Ok(_) => true,
        Err(e) => {
            log::debug!("preset directory not read again: {e}");
            false
        }
    }
}

/// `slot` holds a preset and is the active one.
fn mark_saved(s: &mut DspState, slot: u8) {
    s.preset_occupied |= bit(slot);
    s.active_preset_slot = slot;
}

/// At most 31 bytes of `name`, ending on a character boundary.
fn name_within(name: &str) -> &str {
    let max = CHANNEL_NAME_LEN.saturating_sub(1);
    let mut end = name.len().min(max);
    while !name.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    name.get(..end).unwrap_or_default()
}

// ── The baseline ──────────────────────────────────────────────────────

/// [`DspiCore::capture_baseline`] with the core lock held.
pub(crate) fn capture_baseline_locked(inner: &mut Inner) {
    inner.baseline =
        (inner.conn.is_some() && inner.state.bulk_valid).then(|| inner.state.bulk_raw.clone());
    // -- WP SWAPFIX --
    inner.baseline_of = inner
        .baseline
        .as_ref()
        .and(inner.conn.as_ref())
        .map(|c| (c.serial.clone(), inner.state.active_preset_slot));
}

// -- WP SWAPFIX --
/// After a re-read ([`DspiCore::fetch_all`]): keep the baseline when it was
/// captured for this device on this active preset and still has the image's
/// size; capture it otherwise (none yet, another device, another preset, a
/// firmware with another layout).
pub(crate) fn keep_or_capture_baseline_locked(inner: &mut Inner) {
    let same = match (&inner.baseline, &inner.baseline_of, &inner.conn) {
        (Some(base), Some((serial, slot)), Some(conn)) => {
            inner.state.bulk_valid
                && *serial == conn.serial
                && *slot == inner.state.active_preset_slot
                && base.len() == inner.state.bulk_raw.len()
        }
        _ => false,
    };
    if !same {
        capture_baseline_locked(inner);
    }
}

/// After a re-read caused by a preset load (this client's or anyone's): the
/// re-read image is the new baseline, except where an edit made after the
/// load reached the device first. `old` is the baseline before the re-read
/// and `mirror` the image the mirror held then.
pub(crate) fn rebaseline_after_load_locked(inner: &mut Inner, old: Option<&[u8]>, mirror: &[u8]) {
    capture_baseline_locked(inner);
    if let (Some(base), Some(old)) = (inner.baseline.as_mut(), old) {
        keep_edits_after_load(base, old, mirror);
    }
}

/// Keep unsaved what was edited after a load. `baseline` holds the re-read
/// image; `old` the baseline before it; `mirror` the mirror's image before it.
/// Where the mirror had changed a value since `old` and the device now holds
/// that same value, the device applied the edit on top of the loaded preset:
/// the value goes back to `old`'s, so the edit still shows. Where the device
/// holds something else, the load replaced it, and the re-read stands.
///
/// A full bulk image is compared field by field ([`image_fields`]), so a
/// value is never a mix of two; any other image by runs of changed bytes.
/// Images of different lengths are left as read.
pub(crate) fn keep_edits_after_load(baseline: &mut [u8], old: &[u8], mirror: &[u8]) {
    if baseline.len() != old.len() || old.len() != mirror.len() {
        return;
    }
    let spans = if old.len() == g::BULK_SIZE {
        image_fields()
    } else {
        changed_runs(old, mirror)
    };
    for span in spans {
        let (Some(was), Some(edited)) = (old.get(span.clone()), mirror.get(span.clone())) else {
            continue;
        };
        if was == edited {
            continue;
        }
        if let Some(now) = baseline.get_mut(span)
            && *now == *edited
        {
            now.copy_from_slice(was);
        }
    }
}

/// The maximal runs of bytes where `a` and `b` differ.
fn changed_runs(a: &[u8], b: &[u8]) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut open: Option<usize> = None;
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        match (x != y, open) {
            (true, None) => open = Some(i),
            (false, Some(start)) => {
                runs.push(start..i);
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        runs.push(start..a.len().min(b.len()));
    }
    runs
}

/// `WireBandParams`: type, bypass, the Linkwitz target Q pair, freq, Q, gain.
const BAND_FIELDS: &[usize] = &[1, 1, 2, 4, 4, 4];

/// The bulk image's sections as `(offset, length, field widths of one
/// record)`; a section is a run of such records (`bulk_params.h`).
pub(crate) const IMAGE_FIELDS: [(usize, usize, &[usize]); 25] = [
    (
        g::OFF_HEADER,
        g::LEN_HEADER,
        &[1, 1, 1, 1, 1, 1, 2, 2, 2, 4],
    ),
    (g::OFF_GLOBAL, g::LEN_GLOBAL, &[4, 1, 1, 2, 4, 4]),
    (
        g::OFF_CROSSFEED,
        g::LEN_CROSSFEED,
        &[1, 1, 1, 1, 4, 4, 1, 1, 1, 1],
    ),
    (g::OFF_LEGACY, g::LEN_LEGACY, &[4, 4, 4, 1, 1, 1, 1]),
    (g::OFF_DELAYS, g::LEN_DELAYS, &[4]),
    (g::OFF_CROSSPOINTS, g::LEN_CROSSPOINTS, &[1, 1, 1, 1, 4]),
    (g::OFF_OUTPUTS, g::LEN_OUTPUTS, &[1, 1, 1, 1, 4, 4]),
    (g::OFF_PINS, g::LEN_PINS, &[1]),
    (g::OFF_EQ, g::LEN_EQ, BAND_FIELDS),
    (
        g::OFF_CHANNEL_NAMES,
        g::LEN_CHANNEL_NAMES,
        &[CHANNEL_NAME_LEN],
    ),
    (g::OFF_I2S_CONFIG, g::LEN_I2S_CONFIG, &[1]),
    (
        g::OFF_LEVELLER,
        g::LEN_LEVELLER,
        &[1, 1, 1, 1, 4, 4, 4, 1, 1, 1, 1],
    ),
    (g::OFF_PREAMP, g::LEN_PREAMP, &[4]),
    (
        g::OFF_MASTER_VOLUME,
        g::LEN_MASTER_VOLUME,
        &[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    ),
    (g::OFF_INPUT_CONFIG, g::LEN_INPUT_CONFIG, &[1]),
    (g::OFF_LG_SOUND_SYNC, g::LEN_LG_SOUND_SYNC, &[1]),
    (
        g::OFF_USER_VOLUME,
        g::LEN_USER_VOLUME,
        &[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    ),
    (
        g::OFF_DAC_HW_MUTE,
        g::LEN_DAC_HW_MUTE,
        &[1, 1, 1, 1, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1],
    ),
    (g::OFF_CROSSOVERS, g::LEN_CROSSOVERS, BAND_FIELDS),
    (g::OFF_ADAT_CONFIG, g::LEN_ADAT_CONFIG, &[1]),
    (g::OFF_PSYBASS, g::LEN_PSYBASS, &[1, 1, 2, 4, 4, 4, 4, 4]),
    (
        g::OFF_UPMIX,
        g::LEN_UPMIX,
        &[1, 1, 1, 1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    ),
    (
        g::OFF_SUBHARM,
        g::LEN_SUBHARM,
        &[1, 1, 2, 4, 4, 4, 4, 4, 4, 4, 1, 1, 1, 1],
    ),
    (
        g::OFF_TUBE,
        g::LEN_TUBE,
        &[1, 1, 1, 1, 2, 1, 1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    ),
    (g::OFF_LIMITER, g::LEN_LIMITER, &[1, 1, 1, 1, 4, 4]),
];

/// Every field of the bulk image, in order, as byte ranges: [`IMAGE_FIELDS`]
/// with each section's records spelled out.
pub(crate) fn image_fields() -> Vec<Range<usize>> {
    let mut fields = Vec::new();
    for &(offset, len, widths) in &IMAGE_FIELDS {
        let end = offset.saturating_add(len);
        let mut at = offset;
        'records: while at < end {
            for &w in widths {
                let next = at.saturating_add(w);
                if w == 0 || next > end {
                    break 'records;
                }
                fields.push(at..next);
                at = next;
            }
        }
    }
    fields
}

// ── The change list ───────────────────────────────────────────────────

/// `OUTPUT_CONFIG_MODE_WITH_PRESET` and `MASTER_VOLUME_MODE_WITH_PRESET`.
pub(crate) const WITH_PRESET: u8 = 1;
/// The console's floor of a subharmonic band level: the band is off.
const SUBHARM_BAND_OFF_DB: f32 = g::ranges::SUBHARM_LEVEL_MIN;
/// The console's top of the sub ceiling: no ceiling.
const SUBHARM_NO_CEILING_DB: f32 = g::ranges::SUBHARM_CEILING_MAX;
/// I2S input rate codes, in Hz.
const I2S_RATES_HZ: [u32; 3] = [44_100, 48_000, 96_000];
/// The ADAT input pin's "unset" value in the mirror.
pub(crate) const PIN_UNSET: u8 = 0xFF;

/// The rows for going from `saved` to `live`, in the console's order.
/// `sources`: the user's input source names ("" for none).
pub(crate) fn changes(saved: &DspState, live: &DspState, sources: &[String]) -> Vec<String> {
    let mut d = Rows {
        out: Vec::new(),
        live,
    };
    d.global(saved);
    d.loudness_and_crossfeed(saved);
    d.psybass(saved);
    d.subharm(saved);
    d.tube(saved);
    if live.output_config_mode == WITH_PRESET {
        d.limiters(saved);
    }
    d.upmix(saved);
    d.leveller(saved);
    d.channels(saved);
    d.names(saved);
    if live.output_config_mode == WITH_PRESET {
        d.output_config(saved);
    }
    if saved.input_source != live.input_source {
        let others_on = live.spdif_inputs_enabled & 0x0E != 0;
        let title = |s: u8| source_title(s, sources, others_on);
        d.out.push(format!(
            "Input source: {} → {}",
            title(saved.input_source),
            title(live.input_source)
        ));
    }
    d.toggle(
        saved.lg_sound_sync_enabled,
        live.lg_sound_sync_enabled,
        "LG Sound Sync",
    );
    d.out
}

/// The rows so far, and the live state they compare with.
struct Rows<'a> {
    out: Vec<String>,
    live: &'a DspState,
}

impl Rows<'_> {
    /// A row from `text` when the values differ.
    fn value(&mut self, a: f32, b: f32, text: impl FnOnce(f32, f32) -> String) {
        if !same(a, b) {
            self.out.push(text(a, b));
        }
    }

    /// "`what`: enabled" (or disabled) when the switch moved.
    fn toggle(&mut self, a: bool, b: bool, what: &str) {
        if a != b {
            self.out.push(format!("{what}: {}", enabled(b)));
        }
    }

    /// "`what`: 0x…. → 0x…." for a 16-bit mask.
    fn mask16(&mut self, a: u16, b: u16, what: &str) {
        if a != b {
            self.out.push(format!("{what}: 0x{a:04X} → 0x{b:04X}"));
        }
    }

    /// "`what`: a`unit` → b`unit`", the values as the console prints them.
    fn plain(&mut self, a: f32, b: f32, what: &str, unit: &str) {
        self.value(a, b, |a, b| {
            format!("{what}: {}{unit} → {}{unit}", plain(a), plain(b))
        });
    }

    /// The live label of wire channel `ch`, or `fallback`.
    fn label(&self, ch: usize, fallback: impl FnOnce() -> String) -> String {
        channel_label(self.live, ch).unwrap_or_else(fallback)
    }

    fn global(&mut self, s: &DspState) {
        const LABELS: [&str; 8] = ["L", "R", "FC", "LFE", "BL", "BR", "SL", "SR"];
        for (ch, (&a, &b)) in s
            .input_preamp_db
            .iter()
            .zip(&self.live.input_preamp_db)
            .enumerate()
        {
            let label = LABELS.get(ch).map_or_else(
                || format!("In {}", ch.saturating_add(1)),
                |l| (*l).to_owned(),
            );
            self.value(a, b, |a, b| {
                format!("Preamp {label}: {} → {}", db(a), db(b))
            });
        }
        // The master volume belongs to a preset only in that mode, and the
        // live mode decides, so a switch to it shows a level that differs.
        if self.live.master_volume_mode == WITH_PRESET {
            self.value(s.master_volume_db, self.live.master_volume_db, |a, b| {
                format!("Master Volume: {} → {}", volume(a), volume(b))
            });
        }
        if s.bypass != self.live.bypass {
            self.out.push(format!(
                "Master EQ bypass: {} → {}",
                on_off(s.bypass),
                on_off(self.live.bypass)
            ));
        }
    }

    fn loudness_and_crossfeed(&mut self, s: &DspState) {
        let l = self.live;
        self.toggle(s.loudness_enabled, l.loudness_enabled, "Loudness");
        self.mask16(
            s.loudness_output_mask,
            l.loudness_output_mask,
            "Loudness outputs",
        );
        self.plain(
            s.loudness_ref_spl,
            l.loudness_ref_spl,
            "Loudness ref SPL",
            "",
        );
        self.plain(
            s.loudness_intensity,
            l.loudness_intensity,
            "Loudness intensity",
            "%",
        );

        self.toggle(s.crossfeed_enabled, l.crossfeed_enabled, "Crossfeed");
        if s.crossfeed_preset != l.crossfeed_preset {
            self.out.push(format!(
                "Crossfeed preset: {} → {}",
                s.crossfeed_preset, l.crossfeed_preset
            ));
        }
        self.value(s.crossfeed_freq, l.crossfeed_freq, |a, b| {
            format!("Crossfeed frequency: {} → {} Hz", plain(a), plain(b))
        });
        self.plain(s.crossfeed_feed, l.crossfeed_feed, "Crossfeed feed", "");
        self.toggle(s.crossfeed_itd, l.crossfeed_itd, "Crossfeed ITD");
        if s.crossfeed_output_pair_mask != l.crossfeed_output_pair_mask {
            self.out.push(format!(
                "Crossfeed output pairs: 0x{:02X} → 0x{:02X}",
                s.crossfeed_output_pair_mask, l.crossfeed_output_pair_mask
            ));
        }
        for p in 0..CROSSFEED_MAP_PAIRS {
            let (a, b) = (
                pair_outputs(&s.crossfeed_pair_map, p),
                pair_outputs(&l.crossfeed_pair_map, p),
            );
            if a != b {
                self.out.push(format!(
                    "Crossfeed pair {} outputs: {}/{} → {}/{}",
                    p.saturating_add(1),
                    a.0.saturating_add(1),
                    a.1.saturating_add(1),
                    b.0.saturating_add(1),
                    b.1.saturating_add(1)
                ));
            }
        }
    }

    fn psybass(&mut self, s: &DspState) {
        let l = self.live;
        self.toggle(s.psybass_enabled, l.psybass_enabled, "Psychoacoustic Bass");
        self.mask16(
            s.psybass_output_mask,
            l.psybass_output_mask,
            "Psybass outputs",
        );
        self.value(s.psybass_cutoff_hz, l.psybass_cutoff_hz, |a, b| {
            format!("Psybass cutoff: {} → {} Hz", plain(a), plain(b))
        });
        self.plain(
            s.psybass_harmonics_db,
            l.psybass_harmonics_db,
            "Psybass harmonics",
            " dB",
        );
        self.plain(
            s.psybass_drive_db,
            l.psybass_drive_db,
            "Psybass drive",
            " dB",
        );
        self.plain(
            s.psybass_character_pct,
            l.psybass_character_pct,
            "Psybass character",
            "%",
        );
        self.plain(
            s.psybass_original_db,
            l.psybass_original_db,
            "Psybass original bass",
            " dB",
        );
    }

    fn subharm(&mut self, s: &DspState) {
        let (a, b) = (&s.subharm, &self.live.subharm);
        let at = |v: &[f32], id: u8| param(v, id);
        self.toggle(
            at(a, SUBHARM_PARAM_ENABLED) != 0.0,
            at(b, SUBHARM_PARAM_ENABLED) != 0.0,
            "Subharmonic Synthesizer",
        );
        self.mask16(
            at(a, SUBHARM_PARAM_MASK) as u16,
            at(b, SUBHARM_PARAM_MASK) as u16,
            "Subharm outputs",
        );
        for (id, band) in [
            (SUBHARM_PARAM_LOW, "24-36 Hz"),
            (SUBHARM_PARAM_HIGH, "36-56 Hz"),
            (SUBHARM_PARAM_TOP, "56-80 Hz"),
        ] {
            self.value(at(a, id), at(b, id), |x, y| {
                format!("Subharm {band}: {} → {}", band_level(x), band_level(y))
            });
        }
        let (ma, mb) = (
            at(a, SUBHARM_PARAM_SELECT) as i64,
            at(b, SUBHARM_PARAM_SELECT) as i64,
        );
        if ma != mb {
            self.out.push(format!(
                "Subharm selectivity: {} → {}",
                selectivity(ma),
                selectivity(mb)
            ));
        }
        self.plain(
            at(a, SUBHARM_PARAM_DEPTH),
            at(b, SUBHARM_PARAM_DEPTH),
            "Subharm selectivity depth",
            "%",
        );
        self.plain(
            at(a, SUBHARM_PARAM_HOLD),
            at(b, SUBHARM_PARAM_HOLD),
            "Subharm selectivity hold",
            " ms",
        );
        self.value(
            at(a, SUBHARM_PARAM_CEILING),
            at(b, SUBHARM_PARAM_CEILING),
            |x, y| format!("Subharm sub ceiling: {} → {}", ceiling(x), ceiling(y)),
        );
        let (la, lb) = (
            at(a, SUBHARM_PARAM_LINK) != 0.0,
            at(b, SUBHARM_PARAM_LINK) != 0.0,
        );
        if la != lb {
            let word = if lb { "linked" } else { "independent" };
            self.out.push(format!("Subharm pair link: {word}"));
        }
        self.plain(
            at(a, SUBHARM_PARAM_BOOST),
            at(b, SUBHARM_PARAM_BOOST),
            "Subharm LF boost",
            " dB",
        );
    }

    fn tube(&mut self, s: &DspState) {
        use g::tube as t;
        let (a, b) = (&s.tube, &self.live.tube);
        let at = |v: &[f32], id: u16| param(v, u8::try_from(id).unwrap_or(u8::MAX));
        self.toggle(
            at(a, t::TUBE_PARAM_ENABLED) != 0.0,
            at(b, t::TUBE_PARAM_ENABLED) != 0.0,
            "Tube Modeller",
        );
        self.mask16(
            at(a, t::TUBE_PARAM_OUTPUT_MASK) as u16,
            at(b, t::TUBE_PARAM_OUTPUT_MASK) as u16,
            "Tube outputs",
        );
        let (ta, tb) = (
            at(a, t::TUBE_PARAM_TUBE_TYPE) as u8,
            at(b, t::TUBE_PARAM_TUBE_TYPE) as u8,
        );
        if ta != tb {
            self.out.push(format!(
                "Tube type: {} → {}",
                dspi_session::tube::type_name(ta),
                dspi_session::tube::type_name(tb)
            ));
        }
        for (id, what, unit) in [
            (t::TUBE_PARAM_DRIVE_DB, "Tube drive", " dB"),
            (t::TUBE_PARAM_BIAS_PCT, "Tube bias", "%"),
            (t::TUBE_PARAM_ASYM_DB, "Tube asymmetry", " dB"),
            (t::TUBE_PARAM_HARDNESS_PCT, "Tube knee hardness", "%"),
            (t::TUBE_PARAM_SAG_PCT, "Tube sag", "%"),
        ] {
            self.plain(at(a, id), at(b, id), what, unit);
        }
        let (ra, rb) = (
            at(a, t::TUBE_PARAM_RECTIFIER) as u8,
            at(b, t::TUBE_PARAM_RECTIFIER) as u8,
        );
        if ra != rb {
            self.out.push(format!(
                "Tube rectifier: {} → {}",
                dspi_session::tube::rectifier_name(ra),
                dspi_session::tube::rectifier_name(rb)
            ));
        }
        self.toggle(
            at(a, t::TUBE_PARAM_XFMR_ENABLED) != 0.0,
            at(b, t::TUBE_PARAM_XFMR_ENABLED) != 0.0,
            "Tube output stage",
        );
        self.plain(
            at(a, t::TUBE_PARAM_XFMR_DAMPING),
            at(b, t::TUBE_PARAM_XFMR_DAMPING),
            "Tube damping factor",
            "",
        );
        self.value(
            at(a, t::TUBE_PARAM_XFMR_RES_HZ),
            at(b, t::TUBE_PARAM_XFMR_RES_HZ),
            |x, y| format!("Tube speaker resonance: {} → {} Hz", plain(x), plain(y)),
        );
        self.plain(
            at(a, t::TUBE_PARAM_MIX_PCT),
            at(b, t::TUBE_PARAM_MIX_PCT),
            "Tube mix",
            "%",
        );
        self.plain(
            at(a, t::TUBE_PARAM_TRIM_DB),
            at(b, t::TUBE_PARAM_TRIM_DB),
            "Tube output trim",
            " dB",
        );
    }

    fn limiters(&mut self, s: &DspState) {
        let l = self.live;
        let first_output = usize::from(l.num_input_channels);
        for i in 0..usize::from(l.num_output_channels) {
            let name = self.label(first_output.saturating_add(i), || format!("Output {i}"));
            let flag = |v: &[bool]| v.get(i).copied().unwrap_or(false);
            let num = |v: &[f32]| v.get(i).copied().unwrap_or(0.0);
            let group = |v: &[u8]| v.get(i).copied().unwrap_or(0);
            if flag(&s.limiter_enabled) != flag(&l.limiter_enabled) {
                self.out.push(format!(
                    "{name} limiter: {}",
                    enabled(flag(&l.limiter_enabled))
                ));
            }
            self.value(
                num(&s.limiter_threshold_db),
                num(&l.limiter_threshold_db),
                |a, b| format!("{name} limiter threshold: {a:.1} → {b:.1} dBFS"),
            );
            self.value(
                num(&s.limiter_release_ms),
                num(&l.limiter_release_ms),
                |a, b| format!("{name} limiter release: {} ms → {} ms", plain(a), plain(b)),
            );
            let (ga, gb) = (group(&s.limiter_link_group), group(&l.limiter_link_group));
            if ga != gb {
                self.out.push(format!(
                    "{name} limiter link: {} → {}",
                    dspi_session::state::limiter_link_group_name(ga),
                    dspi_session::state::limiter_link_group_name(gb)
                ));
            }
        }
    }

    fn upmix(&mut self, s: &DspState) {
        let (a, b) = (&s.upmix, &self.live.upmix);
        let at = |v: &[f32], id: u8| param(v, id);
        self.toggle(
            at(a, UPMIX_PARAM_ENABLED) != 0.0,
            at(b, UPMIX_PARAM_ENABLED) != 0.0,
            "Stereo Upmixer",
        );
        // The console's names, by wire value.
        let modes: [(u8, &str, [&str; 3]); 2] = [
            (
                UPMIX_PARAM_CENTER_MODE,
                "Centre mode",
                ["Sinner", "Logician", "Off"],
            ),
            (
                UPMIX_PARAM_SURROUND_MODE,
                "Surround mode",
                ["Off", "Sinner", "Logician"],
            ),
        ];
        for (id, what, names) in modes {
            let (x, y) = (at(a, id) as i64, at(b, id) as i64);
            if x != y {
                let name = |m: i64| {
                    usize::try_from(m)
                        .ok()
                        .and_then(|i| names.get(i))
                        .map_or_else(|| m.to_string(), |n| (*n).to_owned())
                };
                self.out.push(format!("{what}: {} → {}", name(x), name(y)));
            }
        }
        for (id, what, unit) in [
            (UPMIX_PARAM_STRENGTH, "Centre strength", "%"),
            (UPMIX_PARAM_WIDTH, "Centre width", "%"),
            (UPMIX_PARAM_THRESHOLD, "Correlation threshold", "%"),
            (UPMIX_PARAM_ATTACK, "Centre attack", " ms"),
            (UPMIX_PARAM_RELEASE, "Centre release", " ms"),
            (UPMIX_PARAM_DETECTOR_HPF, "Detector HPF", " Hz"),
            (UPMIX_PARAM_SURROUND_DELAY, "Surround delay", " ms"),
            (UPMIX_PARAM_SURROUND_HPF, "Surround HPF", " Hz"),
            (UPMIX_PARAM_SURROUND_LPF, "Surround LPF", " Hz"),
            (UPMIX_PARAM_DECORRELATION, "Decorrelation", "%"),
            (UPMIX_PARAM_PRESENCE, "Centre presence", " dB"),
        ] {
            self.plain(at(a, id), at(b, id), what, unit);
        }
    }

    fn leveller(&mut self, s: &DspState) {
        let l = self.live;
        self.toggle(s.leveller_enabled, l.leveller_enabled, "Volume Leveller");
        self.plain(s.leveller_amount, l.leveller_amount, "Leveller amount", "%");
        if s.leveller_speed != l.leveller_speed {
            let name = |v: u8| {
                ["Slow", "Medium", "Fast"]
                    .get(usize::from(v))
                    .map_or_else(|| v.to_string(), |n| (*n).to_owned())
            };
            self.out.push(format!(
                "Leveller speed: {} → {}",
                name(s.leveller_speed),
                name(l.leveller_speed)
            ));
        }
        self.plain(
            s.leveller_max_gain_db,
            l.leveller_max_gain_db,
            "Leveller max gain",
            " dB",
        );
        self.toggle(
            s.leveller_lookahead,
            l.leveller_lookahead,
            "Leveller lookahead",
        );
        self.plain(
            s.leveller_gate_db,
            l.leveller_gate_db,
            "Leveller gate",
            " dB",
        );
        if s.leveller_detector_mask != l.leveller_detector_mask {
            self.out
                .push("Leveller detector channels changed".to_owned());
        }
        if s.leveller_apply_mask != l.leveller_apply_mask {
            self.out.push("Leveller apply channels changed".to_owned());
        }
    }

    /// Delays, the matrix, the outputs, the EQ and the crossovers.
    fn channels(&mut self, s: &DspState) {
        let l = self.live;
        let inputs = usize::from(l.num_input_channels);
        let outputs = usize::from(l.num_output_channels);
        let total = usize::from(l.num_channels);

        for ch in 0..total {
            let (a, b) = (
                s.channel_delays.get(ch).copied().unwrap_or(0.0),
                l.channel_delays.get(ch).copied().unwrap_or(0.0),
            );
            // -- WP SWAPFIX -- an output's delay is in the image twice (the
            // channel's and the output's): the output's line below says it
            let listed = ch
                .checked_sub(inputs)
                .filter(|&o| o < outputs)
                .is_some_and(|o| {
                    let num = |v: &[f32]| v.get(o).copied().unwrap_or(0.0);
                    !same(num(&s.output_delay_ms), num(&l.output_delay_ms))
                });
            if !same(a, b) && !listed {
                let name = self.label(ch, || format!("Ch {ch}"));
                self.out
                    .push(format!("{name} delay: {} ms → {} ms", delay(a), delay(b)));
            }
        }

        let crosspoints = (0..inputs)
            .flat_map(|i| (0..outputs).map(move |o| (i, o)))
            .filter(|&(i, o)| {
                let cell = |st: &DspState| {
                    (
                        cell(&st.matrix_routing, i, o).copied().unwrap_or(false),
                        cell(&st.matrix_gain, i, o).copied().unwrap_or(0.0),
                        cell(&st.matrix_invert, i, o).copied().unwrap_or(false),
                    )
                };
                let (x, y) = (cell(s), cell(l));
                x.0 != y.0 || !same(x.1, y.1) || x.2 != y.2
            })
            .count();
        if crosspoints > 0 {
            self.out.push(format!(
                "{crosspoints} crosspoint{} changed",
                plural(crosspoints)
            ));
        }

        for i in 0..outputs {
            let name = self.label(inputs.saturating_add(i), || format!("Output {i}"));
            let flag = |v: &[bool]| v.get(i).copied().unwrap_or(false);
            let num = |v: &[f32]| v.get(i).copied().unwrap_or(0.0);
            let mut what = Vec::new();
            if flag(&s.output_enabled) != flag(&l.output_enabled) {
                what.push(
                    if flag(&l.output_enabled) {
                        "enabled"
                    } else {
                        "disabled"
                    }
                    .to_owned(),
                );
            }
            if flag(&s.output_muted) != flag(&l.output_muted) {
                what.push(
                    if flag(&l.output_muted) {
                        "muted"
                    } else {
                        "unmuted"
                    }
                    .to_owned(),
                );
            }
            let (ga, gb) = (num(&s.output_gain_db), num(&l.output_gain_db));
            if !same(ga, gb) {
                what.push(format!("gain: {} → {}", db(ga), db(gb)));
            }
            let (da, dbl) = (num(&s.output_delay_ms), num(&l.output_delay_ms));
            if !same(da, dbl) {
                what.push(format!("delay: {} ms → {} ms", delay(da), delay(dbl)));
            }
            for w in what {
                self.out.push(format!("{name} {w}"));
            }
        }

        for ch in 0..total {
            let n = bands_changed(&s.filters, &l.filters, ch, BANDS_PER_CHANNEL);
            if n > 0 {
                let name = self.label(ch, || format!("Ch {ch}"));
                self.out
                    .push(format!("{n} band{} changed on {name}", plural(n)));
            }
        }
        for ch in inputs..total {
            let n = bands_changed(&s.xover, &l.xover, ch, MAX_XOVER_BANDS);
            if n > 0 {
                let name = self.label(ch, || format!("Ch {ch}"));
                self.out
                    .push(format!("{n} crossover band{} changed on {name}", plural(n)));
            }
        }
    }

    /// Renames. One output default name to another is the firmware following
    /// an output type change, not an edit: skipped.
    fn names(&mut self, s: &DspState) {
        let l = self.live;
        for (ch, (a, b)) in s.channel_names.iter().zip(&l.channel_names).enumerate() {
            if a == b {
                continue;
            }
            let was_default = firmware_name(s, ch).is_some_and(|n| n == *a);
            let is_default = firmware_name(l, ch).is_some_and(|n| n == *b);
            if was_default && is_default {
                continue;
            }
            self.out.push(format!("'{a}' → '{b}'"));
        }
    }

    /// The output configuration: what a preset holds only in that mode.
    fn output_config(&mut self, s: &DspState) {
        let l = self.live;
        let pins = differing(&s.output_pins, &l.output_pins);
        if pins > 0 {
            self.out
                .push(format!("{pins} pin assignment{} changed", plural(pins)));
        }
        let types = differing(&s.output_types, &l.output_types);
        if types > 0 {
            self.out
                .push(format!("{types} output type{} changed", plural(types)));
        }
        self.gpio(s.i2s_bck_pin, l.i2s_bck_pin, "BCK pin");
        self.toggle(s.mck_enabled, l.mck_enabled, "MCK");
        self.gpio(s.mck_pin, l.mck_pin, "MCK pin");
        if s.mck_multiplier != l.mck_multiplier {
            self.out.push(format!(
                "MCK multiplier: {}x → {}x",
                mck_multiplier(s.mck_multiplier),
                mck_multiplier(l.mck_multiplier)
            ));
        }
        let first = |v: &[u8]| v.first().copied().unwrap_or(0);
        self.gpio(
            first(&s.spdif_rx_pins),
            first(&l.spdif_rx_pins),
            "S/PDIF RX pin",
        );
        if s.spdif_inputs_enabled_present && l.spdif_inputs_enabled_present {
            // Bit k is S/PDIF input k + 1; the first is always on.
            for k in 1..4u32 {
                let on = |m: u8| m & 1u8.checked_shl(k).unwrap_or(0) != 0;
                let (a, b) = (on(s.spdif_inputs_enabled), on(l.spdif_inputs_enabled));
                if a != b {
                    self.out.push(format!(
                        "S/PDIF {} input: {}",
                        k.saturating_add(1),
                        enabled(b)
                    ));
                }
            }
            for (i, (a, b)) in s
                .spdif_rx_pins
                .iter()
                .zip(&l.spdif_rx_pins)
                .enumerate()
                .skip(1)
            {
                self.gpio(*a, *b, &format!("S/PDIF {} RX pin", i.saturating_add(1)));
            }
        }
        if s.i2s_input_channels != l.i2s_input_channels {
            self.out.push(format!(
                "I2S input channels: {} → {}",
                s.i2s_input_channels, l.i2s_input_channels
            ));
        }
        let pairs = if l.platform_id == 0 { 1 } else { 4 };
        for (p, (a, b)) in s
            .i2s_rx_pins
            .iter()
            .zip(&l.i2s_rx_pins)
            .take(pairs)
            .enumerate()
        {
            self.gpio(
                *a,
                *b,
                &format!("I2S RX pin (pair {})", p.saturating_add(1)),
            );
        }
        let (ra, rb) = (rate_hz(s.i2s_input_rate), rate_hz(l.i2s_input_rate));
        if ra != rb {
            self.out.push(format!("I2S rate: {ra} Hz → {rb} Hz"));
        }
        if s.i2s_clock_mode != l.i2s_clock_mode {
            self.out.push(format!(
                "I2S clock mode: {} → {}",
                clock_mode(s.i2s_clock_mode),
                clock_mode(l.i2s_clock_mode)
            ));
        }
        // ADAT is an RP2350 feature.
        if l.platform_id == 1 {
            self.toggle(s.adat_out_enabled, l.adat_out_enabled, "ADAT output");
            self.gpio(s.adat_out_pin, l.adat_out_pin, "ADAT pin");
            if s.adat_input_enabled_present && l.adat_input_enabled_present {
                self.toggle(s.adat_input_enabled, l.adat_input_enabled, "ADAT input");
            }
            if s.adat_input_pin != l.adat_input_pin {
                let pin = |p: u8| {
                    if p == PIN_UNSET {
                        "unset".to_owned()
                    } else {
                        format!("GPIO {p}")
                    }
                };
                self.out.push(format!(
                    "ADAT input pin: {} → {}",
                    pin(s.adat_input_pin),
                    pin(l.adat_input_pin)
                ));
            }
            if s.adat_input_clock_mode_present
                && l.adat_input_clock_mode_present
                && s.adat_input_clock_mode != l.adat_input_clock_mode
            {
                self.out.push(format!(
                    "ADAT input clock mode: {} → {}",
                    clock_mode(s.adat_input_clock_mode),
                    clock_mode(l.adat_input_clock_mode)
                ));
            }
        }
    }

    /// "`what`: GPIO a → GPIO b".
    fn gpio(&mut self, a: u8, b: u8, what: &str) {
        if a != b {
            self.out.push(format!("{what}: GPIO {a} → GPIO {b}"));
        }
    }
}

// -- WP P1DF -- the helpers below are crate-visible: `doc_diff` words its
// rows with them.

/// Equal as the console compares (`==`), but a NaN equals a NaN, so one
/// stored in the image never reads as a change.
pub(crate) fn same(a: f32, b: f32) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

pub(crate) fn enabled(on: bool) -> &'static str {
    if on { "enabled" } else { "disabled" }
}

pub(crate) fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// One decimal and the unit: "-3.0 dB".
pub(crate) fn db(v: f32) -> String {
    format!("{v:.1} dB")
}

/// A master volume: at or below -128 dB it is off.
pub(crate) fn volume(v: f32) -> String {
    if v <= -128.0 {
        "-∞ dB".to_owned()
    } else {
        db(v)
    }
}

/// A whole number as such, anything else with one decimal.
pub(crate) fn plain(v: f32) -> String {
    if v == v.round() && v.abs() < 100_000.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// A delay in ms: up to four decimals, trailing zeros dropped.
pub(crate) fn delay(ms: f32) -> String {
    let text = format!("{ms:.4}");
    match text.split_once('.') {
        Some((whole, frac)) => {
            let frac = frac.trim_end_matches('0');
            if frac.is_empty() {
                whole.to_owned()
            } else {
                format!("{whole}.{frac}")
            }
        }
        None => text,
    }
}

/// A subharmonic band level; the floor switches the band off.
pub(crate) fn band_level(v: f32) -> String {
    if v <= SUBHARM_BAND_OFF_DB {
        "off".to_owned()
    } else {
        format!("{} dB", plain(v))
    }
}

/// The sub ceiling; at full scale it is off.
pub(crate) fn ceiling(v: f32) -> String {
    if v >= SUBHARM_NO_CEILING_DB {
        "off".to_owned()
    } else {
        format!("{} dBFS", plain(v))
    }
}

pub(crate) fn selectivity(mode: i64) -> &'static str {
    match u16::try_from(mode) {
        Ok(g::subharm::SUBHARM_SELECT_PERCUSSIVE) => "percussive",
        Ok(g::subharm::SUBHARM_SELECT_SUSTAINED) => "sustained",
        _ => "all material",
    }
}

pub(crate) fn mck_multiplier(code: u8) -> u16 {
    if code == 1 { 256 } else { 128 }
}

pub(crate) fn rate_hz(code: u8) -> u32 {
    I2S_RATES_HZ
        .get(usize::from(code))
        .copied()
        .unwrap_or(48_000)
}

pub(crate) fn clock_mode(mode: u8) -> &'static str {
    if mode == 1 { "Slave" } else { "Master" }
}

/// Parameter `id` of a `*_PARAM_*`-indexed vector, 0 when missing.
pub(crate) fn param(v: &[f32], id: u8) -> f32 {
    v.get(usize::from(id)).copied().unwrap_or(0.0)
}

pub(crate) fn cell<T>(m: &[Vec<T>], i: usize, o: usize) -> Option<&T> {
    m.get(i).and_then(|row| row.get(o))
}

/// How many entries differ, over the shorter of the two.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// The outputs (left, right) of crossfeed pair `p`; a missing or zero byte
/// is the pair's own outputs.
pub(crate) fn pair_outputs(map: &[u8], p: usize) -> (usize, usize) {
    match map.get(p).copied().unwrap_or(0) {
        0 => {
            let left = p.saturating_mul(2);
            (left, left.saturating_add(1))
        }
        b => (usize::from(b & 0x0F), usize::from(b >> 4)),
    }
}

/// How many of the first `bands` bands of channel `ch` differ.
fn bands_changed(
    a: &[Vec<FilterParams>],
    b: &[Vec<FilterParams>],
    ch: usize,
    bands: usize,
) -> usize {
    let empty = Vec::new();
    let (x, y) = (a.get(ch).unwrap_or(&empty), b.get(ch).unwrap_or(&empty));
    (0..bands)
        .filter(|&i| match (x.get(i), y.get(i)) {
            (Some(p), Some(q)) => !same_band(p, q),
            (None, None) => false,
            _ => true,
        })
        .count()
}

/// The console's band comparison: type, frequency, Q, gain, bypass, and the
/// Linkwitz target Q only on a Linkwitz band.
pub(crate) fn same_band(a: &FilterParams, b: &FilterParams) -> bool {
    let target_q = |p: &FilterParams| {
        if FilterType::from_raw(p.filter_type).is_linkwitz() {
            p.qp
        } else {
            0.0
        }
    };
    a.filter_type == b.filter_type
        && a.bypass == b.bypass
        && same(a.freq, b.freq)
        && same(a.q, b.q)
        && same(a.gain, b.gain)
        && same(target_q(a), target_q(b))
}

/// What a channel is called in a row: its name, or the firmware's default
/// name when it has none.
pub(crate) fn channel_label(s: &DspState, ch: usize) -> Option<String> {
    match s.channel_names.get(ch) {
        Some(n) if !n.is_empty() => Some(n.clone()),
        _ => firmware_name(s, ch),
    }
}

/// The firmware's default name of wire channel `ch` (`get_default_channel_name`).
/// -- WP CG-SWS-a -- the rule is [`crate::default_channel_names`]'.
fn firmware_name(s: &DspState, ch: usize) -> Option<String> {
    crate::platform::default_channel_name(
        s.num_input_channels,
        s.num_output_channels,
        s.input_source,
        &s.output_types,
        ch,
    )
}

/// An input source's title: the user's name for it, else the built-in one.
/// The first S/PDIF input is numbered only while another one is on.
pub(crate) fn source_title(source: u8, names: &[String], other_spdif_on: bool) -> String {
    if let Some(n) = names.get(usize::from(source))
        && !n.is_empty()
    {
        return n.clone();
    }
    match source {
        0 => "USB".to_owned(),
        1 if other_spdif_on => "S/PDIF 1".to_owned(),
        1 => "S/PDIF".to_owned(),
        2 => "I2S".to_owned(),
        3 => "ADAT".to_owned(),
        4..=6 => format!("S/PDIF {}", source.saturating_sub(2)),
        // -- WP SW1 (fixup sw1, F4) --
        crate::alink::INPUT_SOURCE_ALINK => "Audio link".to_owned(),
        n => format!("Source {n}"),
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps,
        TransportError,
    };

    use super::*;
    use crate::consts::{FLASH_ERR_CRC, FLASH_ERR_WRITE};
    use crate::{MeterRead, SideChannel};

    const INVALID_SLOT: u8 = st::PRESET_ERR_INVALID_SLOT as u8;

    /// What the test sees of a [`Spy`]: its log, its scripted replies, and
    /// whether it refuses the device lock.
    #[derive(Default)]
    struct Probe {
        /// `"in OP value"`, `"out OP value hex"`, `"lock reason ms"`,
        /// `"unlock"`, `"side lock reason"`, `"side unlock"`.
        log: Mutex<Vec<String>>,
        /// IN replies by opcode instead of the device's; `None` stalls.
        script: Mutex<HashMap<u8, Option<Vec<u8>>>>,
        refuse_lock: AtomicBool,
        /// -- WP CLEAN1 fixup cl1 -- Answer a factory reset at once and apply
        /// it only on a transfer this long after, as the firmware's main loop does.
        defer_reset: Mutex<Option<Duration>>,
        /// When the deferred reset was asked for.
        reset_asked: Mutex<Option<std::time::Instant>>,
    }

    impl Probe {
        fn note(&self, line: String) {
            self.log.lock().unwrap().push(line);
        }
        fn mark(&self) -> usize {
            self.log.lock().unwrap().len()
        }
        fn since(&self, mark: usize) -> Vec<String> {
            self.log.lock().unwrap()[mark..].to_vec()
        }
        fn reply(&self, opcode: u8, reply: Option<Vec<u8>>) {
            self.script.lock().unwrap().insert(opcode, reply);
        }
        fn forget(&self, opcode: u8) {
            self.script.lock().unwrap().remove(&opcode);
        }
        fn scripted(&self, opcode: u8, len: u16) -> Option<dspi_transport::Result<Vec<u8>>> {
            let script = self.script.lock().unwrap();
            script.get(&opcode).map(|r| match r {
                Some(b) => Ok(b.iter().copied().take(usize::from(len)).collect()),
                None => Err(TransportError::Stalled { opcode }),
            })
        }
    }

    /// The demo device, watched through a [`Probe`].
    struct Spy {
        dev: FakeDevice,
        probe: Arc<Probe>,
    }

    impl Spy {
        /// Apply a deferred factory reset once its delay has passed.
        fn catch_up(&mut self) {
            let due = {
                let delay = *self.probe.defer_reset.lock().unwrap();
                let mut asked = self.probe.reset_asked.lock().unwrap();
                match (delay, *asked) {
                    (Some(d), Some(t)) if t.elapsed() >= d => asked.take().is_some(),
                    _ => false,
                }
            };
            if due {
                self.dev.control_in(op::REQ_FACTORY_RESET, 0, 1).unwrap();
            }
        }
    }

    impl Transport for Spy {
        fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
            self.probe.note(format!("in {op:02X} {value}"));
            self.catch_up();
            if op == op::REQ_FACTORY_RESET && self.probe.defer_reset.lock().unwrap().is_some() {
                *self.probe.reset_asked.lock().unwrap() = Some(std::time::Instant::now());
                return Ok(vec![0]);
            }
            match self.probe.scripted(op, len) {
                Some(r) => r,
                None => self.dev.control_in(op, value, len),
            }
        }
        fn control_in_upto(
            &mut self,
            op: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.probe.note(format!("in {op:02X} {value}"));
            self.catch_up();
            match self.probe.scripted(op, len) {
                Some(r) => r,
                None => self.dev.control_in_upto(op, value, len),
            }
        }
        fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            self.probe.note(format!("out {op:02X} {value} {hex}"));
            self.catch_up();
            self.dev.control_out(op, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.dev.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.dev.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            None
        }
        fn caps(&self) -> TransportCaps {
            self.dev.caps()
        }
        fn lock(&mut self, reason: &str, timeout: Duration) -> dspi_transport::Result<()> {
            self.probe
                .note(format!("lock {reason} {}", timeout.as_millis()));
            if self.probe.refuse_lock.load(Ordering::SeqCst) {
                Err(TransportError::Locked)
            } else {
                Ok(())
            }
        }
        fn unlock(&mut self) {
            self.probe.note("unlock".into());
        }
    }

    /// A Link side channel that only logs its lock calls.
    struct Side(Arc<Probe>);

    impl SideChannel for Side {
        fn lock(&self, reason: &str, _timeout: Duration) -> Result<()> {
            self.0.note(format!("side lock {reason}"));
            Ok(())
        }
        fn unlock(&self) {
            self.0.note("side unlock".into());
        }
        fn rta_transport(self: Arc<Self>, _rta_mask: u16) -> Box<dyn Transport> {
            Box::new(FakeDevice::demo())
        }
        fn meters(&self, _len: u16) -> Result<MeterRead> {
            Ok(MeterRead::default())
        }
    }

    fn spy_on(dev: FakeDevice) -> (Spy, Arc<Probe>) {
        let probe = Arc::new(Probe::default());
        (
            Spy {
                dev,
                probe: Arc::clone(&probe),
            },
            probe,
        )
    }

    /// The demo device behind a spy, probed but not fetched (no baseline).
    fn probed() -> (Arc<DspiCore>, Arc<Probe>) {
        let (spy, probe) = spy_on(FakeDevice::demo());
        let core = DspiCore::new();
        core.attach_probed(Box::new(spy), "DEMO".into(), 0).unwrap();
        (core, probe)
    }

    /// The demo device behind a spy, fetched: the baseline is its state.
    fn fetched() -> (Arc<DspiCore>, Arc<Probe>) {
        let (core, probe) = probed();
        core.fetch_all().unwrap();
        (core, probe)
    }

    fn sent(log: &[String], opcode: u8) -> bool {
        let tag = format!(" {opcode:02X} ");
        log.iter().any(|l| l.contains(&tag))
    }

    fn count(log: &[String], prefix: &str) -> usize {
        log.iter().filter(|l| l.starts_with(prefix)).count()
    }

    fn preamp(core: &DspiCore) -> f32 {
        core.get_state().input_preamp_db[0]
    }

    /// The preset directory as the device would report it.
    fn directory(occupied: u16, last_active: u8) -> Vec<u8> {
        let [lo, hi] = occupied.to_le_bytes();
        vec![lo, hi, 0, 0, last_active, 1, 0]
    }

    // ── Flows ─────────────────────────────────────────────────────────

    #[test]
    fn clean1_a_save_fills_the_slot_inside_one_lock() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -4.0).unwrap();
        assert!(core.preset_dirty());
        let mark = probe.mark();
        core.save_preset_tok(3).unwrap();
        let log = probe.since(mark);
        assert_eq!(
            log.first().map(String::as_str),
            Some("lock Saving preset 60000")
        );
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
        assert_eq!(count(&log, "lock"), 1, "{log:?}");
        assert!(log.contains(&format!("in {:02X} 3", op::REQ_PRESET_SAVE)));
        assert!(sent(&log, op::REQ_PRESET_GET_ACTIVE), "{log:?}");
        let s = core.get_state();
        assert_eq!(s.active_preset_slot, 3);
        assert_ne!(s.preset_occupied & (1 << 3), 0);
        assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
        assert_eq!(core.get_preset_directory().unwrap().last_active, 3);
    }

    // -- WP FIXM1 --
    #[test]
    fn wp_fixm1_the_master_volume_save_runs_inside_one_lock() {
        let (core, probe) = fetched();
        let mark = probe.mark();
        core.save_master_volume().unwrap();
        let log = probe.since(mark);
        assert_eq!(
            log.first().map(String::as_str),
            Some("lock Saving master volume 60000"),
            "{log:?}"
        );
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
        assert_eq!(count(&log, "lock"), 1, "{log:?}");
        assert!(sent(&log, op::REQ_SAVE_MASTER_VOLUME), "{log:?}");
        // A lock held elsewhere: refused, nothing sent.
        probe.refuse_lock.store(true, Ordering::SeqCst);
        let mark = probe.mark();
        assert_eq!(core.save_master_volume(), Err(DspiError::Locked));
        assert!(!sent(&probe.since(mark), op::REQ_SAVE_MASTER_VOLUME));
    }

    #[test]
    fn clean1_a_load_reads_the_slot_back_and_is_clean() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -4.0).unwrap();
        core.save_preset_tok(1).unwrap();
        core.set_input_preamp(0, -2.0).unwrap();
        core.save_preset_tok(0).unwrap();
        core.set_input_preamp(0, 5.0).unwrap();
        let mark = probe.mark();
        core.load_preset(1).unwrap();
        let log = probe.since(mark);
        assert!(log.contains(&format!("in {:02X} 1", op::REQ_PRESET_LOAD)));
        assert!(
            sent(&log, op::REQ_GET_ALL_PARAMS) || sent(&log, op::REQ_GET_ALL_PARAMS_CHUNK),
            "{log:?}"
        );
        assert_eq!(preamp(&core), -4.0);
        assert_eq!(core.get_state().active_preset_slot, 1);
        assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
    }

    #[test]
    fn clean1_revert_loads_the_active_slot_again() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -4.0).unwrap();
        core.save_preset_tok(2).unwrap();
        core.set_input_preamp(0, -1.0).unwrap();
        let mark = probe.mark();
        core.load_params().unwrap();
        assert!(
            probe
                .since(mark)
                .contains(&format!("in {:02X} 2", op::REQ_PRESET_LOAD))
        );
        assert_eq!(preamp(&core), -4.0);
        assert!(!core.preset_dirty());
    }

    #[test]
    fn clean1_a_refused_revert_answers_in_flash_codes() {
        let (core, probe) = fetched();
        for (preset, flash) in [
            (st::PRESET_ERR_CRC as u8, FLASH_ERR_CRC),
            (st::PRESET_ERR_SLOT_EMPTY as u8, FLASH_ERR_WRITE),
            (st::PRESET_ERR_FLASH_WRITE as u8, FLASH_ERR_WRITE),
        ] {
            probe.reply(op::REQ_PRESET_LOAD, Some(vec![preset]));
            assert_eq!(
                core.load_params(),
                Err(DspiError::Refused { status: flash }),
                "preset status {preset}"
            );
        }
        probe.forget(op::REQ_PRESET_LOAD);
        core.load_params().unwrap();
        assert_eq!(DspiCore::new().load_params(), Err(DspiError::Disconnected));
    }

    #[test]
    fn clean1_copy_to_keeps_the_source_active() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -6.0).unwrap();
        let mark = probe.mark();
        core.copy_preset_tok(0, 5).unwrap();
        let log = probe.since(mark);
        let saves: Vec<_> = log
            .iter()
            .filter(|l| l.starts_with(&format!("in {:02X} ", op::REQ_PRESET_SAVE)))
            .collect();
        assert_eq!(saves, ["in 90 5", "in 90 0"]);
        assert_eq!(count(&log, "lock"), 1);
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
        let s = core.get_state();
        assert_eq!(s.active_preset_slot, 0);
        assert_eq!(s.preset_occupied & 0b10_0001, 0b10_0001);
        assert!(!core.preset_dirty());
        core.set_input_preamp(0, 0.0).unwrap();
        core.load_preset(5).unwrap();
        assert_eq!(preamp(&core), -6.0);
    }

    #[test]
    fn clean1_delete_clears_the_slot_and_its_name() {
        let (core, probe) = fetched();
        core.save_preset_tok(4).unwrap();
        core.set_preset_name(4, "Gone".into()).unwrap();
        core.save_preset_tok(0).unwrap();
        let mark = probe.mark();
        core.delete_preset_tok(4).unwrap();
        let log = probe.since(mark);
        assert!(log.contains(&format!("in {:02X} 4", op::REQ_PRESET_DELETE)));
        assert_eq!(count(&log, "lock"), 1);
        let s = core.get_state();
        assert_eq!(s.preset_occupied & (1 << 4), 0);
        assert_eq!(s.preset_names[4], "");
        assert_eq!(
            core.get_preset_directory().unwrap().occupied_mask & (1 << 4),
            0
        );
        // Not the active slot: the image is not read again.
        assert!(!sent(&log, op::REQ_GET_ALL_PARAMS) && !sent(&log, op::REQ_GET_ALL_PARAMS_CHUNK));
    }

    #[test]
    fn clean1_deleting_the_active_slot_reads_the_image_again() {
        let (core, probe) = fetched();
        core.save_preset_tok(2).unwrap();
        core.set_input_preamp(0, 2.0).unwrap();
        let mark = probe.mark();
        core.delete_preset_tok(2).unwrap();
        let log = probe.since(mark);
        assert!(
            sent(&log, op::REQ_GET_ALL_PARAMS) || sent(&log, op::REQ_GET_ALL_PARAMS_CHUNK),
            "{log:?}"
        );
        assert!(
            !core.preset_dirty(),
            "what the device runs now is the baseline"
        );
    }

    #[test]
    fn clean1_names_go_out_and_come_back() {
        let (core, _) = fetched();
        core.save_preset_tok(3).unwrap();
        core.set_preset_name(3, "Night".into()).unwrap();
        assert_eq!(core.get_state().preset_names[3], "Night");
        assert_eq!(core.get_preset_name(3).unwrap(), "Night");
    }

    #[test]
    fn clean1_a_name_is_cut_to_31_bytes_and_ends_in_nul() {
        let (core, probe) = fetched();
        let mark = probe.mark();
        core.set_preset_name(1, "x".repeat(40)).unwrap();
        let hex = format!("{}00", "78".repeat(31));
        assert_eq!(
            probe.since(mark),
            [format!("out {:02X} 1 {hex}", op::REQ_PRESET_SET_NAME)]
        );
        assert_eq!(core.get_state().preset_names[1], "x".repeat(31));
        // Never through a character: 15 two-byte characters fit.
        core.set_preset_name(1, "é".repeat(20)).unwrap();
        assert_eq!(core.get_state().preset_names[1], "é".repeat(15));
        assert_eq!(name_within(""), "");
    }

    #[test]
    fn clean1_the_startup_choice_is_checked_then_stored() {
        let (core, probe) = fetched();
        let mark = probe.mark();
        assert!(matches!(
            core.set_preset_startup(2, 0),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert!(matches!(
            core.set_preset_startup(0, MAX_PRESETS as u8),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert!(probe.since(mark).is_empty());
        core.set_preset_startup(1, 4).unwrap();
        let s = core.get_state();
        assert_eq!((s.preset_startup_mode, s.preset_default_slot), (1, 4));
        let dir = core.get_preset_directory().unwrap();
        assert_eq!((dir.startup_mode, dir.default_slot), (1, 4));
    }

    #[test]
    fn clean1_slots_past_the_last_send_nothing() {
        let (core, probe) = fetched();
        let refused = Err(DspiError::Refused {
            status: INVALID_SLOT,
        });
        let mark = probe.mark();
        let past = MAX_PRESETS as u8;
        assert_eq!(core.save_preset_tok(past), refused);
        assert_eq!(core.load_preset(past), refused);
        assert_eq!(core.delete_preset_tok(past), refused);
        assert_eq!(core.copy_preset_tok(0, past), refused);
        assert_eq!(core.copy_preset_tok(past, 0), refused);
        assert_eq!(core.set_preset_name(past, "x".into()), refused);
        assert_eq!(
            core.get_preset_name(255),
            Err(DspiError::Refused {
                status: INVALID_SLOT
            })
        );
        assert!(probe.since(mark).is_empty());
        // Before the connection is even looked at.
        assert_eq!(DspiCore::new().save_preset_tok(past), refused);
    }

    #[test]
    fn clean1_a_refused_save_gives_the_lock_back() {
        let (core, probe) = fetched();
        let flash = st::PRESET_ERR_FLASH_WRITE as u8;
        probe.reply(op::REQ_PRESET_SAVE, Some(vec![flash]));
        let mark = probe.mark();
        assert_eq!(
            core.save_preset_tok(1),
            Err(DspiError::Refused { status: flash })
        );
        let log = probe.since(mark);
        assert_eq!(log.last().map(String::as_str), Some("unlock"), "{log:?}");
        assert!(
            !sent(&log, op::REQ_PRESET_GET_ACTIVE),
            "no wait after a refusal"
        );
    }

    #[test]
    fn clean1_a_refused_lock_sends_nothing() {
        let (core, probe) = fetched();
        probe.refuse_lock.store(true, Ordering::SeqCst);
        let mark = probe.mark();
        assert_eq!(core.save_preset_tok(1), Err(DspiError::Locked));
        assert_eq!(core.load_preset(1), Err(DspiError::Locked));
        assert_eq!(core.copy_preset_tok(0, 1), Err(DspiError::Locked));
        assert_eq!(core.delete_preset_tok(1), Err(DspiError::Locked));
        assert_eq!(core.save_params_tok(), Err(DspiError::Locked));
        assert_eq!(core.factory_reset_tok(), Err(DspiError::Locked));
        let log = probe.since(mark);
        assert!(log.iter().all(|l| l.starts_with("lock ")), "{log:?}");
        assert_eq!(log.len(), 6);
    }

    #[test]
    fn clean1_save_params_writes_the_active_slot_and_rebaselines() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -3.5).unwrap();
        assert!(core.preset_dirty());
        let mark = probe.mark();
        core.save_params_tok().unwrap();
        let log = probe.since(mark);
        assert!(log.contains(&format!("in {:02X} 0", op::REQ_SAVE_PARAMS)));
        assert_eq!(
            log.first().map(String::as_str),
            Some("lock Saving to flash 60000")
        );
        assert!(!core.preset_dirty());
        probe.reply(op::REQ_SAVE_PARAMS, Some(vec![FLASH_ERR_WRITE]));
        assert_eq!(
            core.save_params_tok(),
            Err(DspiError::Refused {
                status: FLASH_ERR_WRITE
            })
        );
    }

    #[test]
    fn clean1_factory_reset_reads_everything_again() {
        let (core, probe) = fetched();
        let factory = preamp(&core);
        core.set_input_preamp(0, factory - 7.0).unwrap();
        let mark = probe.mark();
        core.factory_reset_tok().unwrap();
        let log = probe.since(mark);
        let reset = log
            .iter()
            .position(|l| *l == format!("in {:02X} 0", op::REQ_FACTORY_RESET))
            .unwrap();
        let after = &log[reset..];
        assert!(sent(after, op::REQ_GET_PLATFORM), "{log:?}");
        assert!(sent(after, op::REQ_PRESET_GET_DIR), "{log:?}");
        assert_eq!(preamp(&core), factory);
        assert!(!core.preset_dirty());
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
    }

    /// -- WP CLEAN1 fixup cl1 -- A device that applies the reset a little after
    /// answering still ends up read as reset, with nothing unsaved.
    #[test]
    fn clean1_factory_reset_waits_for_a_deferring_device() {
        let (core, probe) = fetched();
        let factory = preamp(&core);
        core.set_input_preamp(0, factory - 7.0).unwrap();
        *probe.defer_reset.lock().unwrap() = Some(Duration::from_millis(100));
        core.factory_reset_tok().unwrap();
        assert_eq!(preamp(&core), factory);
        assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
        assert!(
            probe.reset_asked.lock().unwrap().is_none(),
            "the reset was applied"
        );
    }

    #[test]
    fn clean1_a_read_only_device_is_never_written() {
        let (spy, probe) = spy_on(FakeDevice::demo_as_wire(31).unwrap());
        let core = DspiCore::new();
        core.attach_probed(Box::new(spy), "OLD".into(), 0).unwrap();
        core.fetch_all().unwrap();
        let mark = probe.mark();
        assert_eq!(core.save_preset_tok(1), Err(DspiError::ReadOnly));
        assert_eq!(core.load_preset(1), Err(DspiError::ReadOnly));
        assert_eq!(core.copy_preset_tok(0, 1), Err(DspiError::ReadOnly));
        assert_eq!(core.delete_preset_tok(1), Err(DspiError::ReadOnly));
        assert_eq!(
            core.set_preset_name(1, "x".into()),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(core.set_preset_startup(1, 1), Err(DspiError::ReadOnly));
        assert_eq!(core.save_params_tok(), Err(DspiError::ReadOnly));
        assert_eq!(core.load_params(), Err(DspiError::ReadOnly));
        assert_eq!(core.factory_reset_tok(), Err(DspiError::ReadOnly));
        assert!(probe.since(mark).is_empty(), "{:?}", probe.since(mark));
        // Reading still works, and an image of another format has no baseline.
        core.get_preset_directory().unwrap();
        assert!(core.preset_changes().is_empty());
    }

    #[test]
    fn clean1_with_no_device() {
        let core = DspiCore::new();
        assert_eq!(core.save_preset_tok(0), Err(DspiError::Disconnected));
        assert_eq!(core.load_preset(0), Err(DspiError::Disconnected));
        assert_eq!(core.copy_preset_tok(0, 1), Err(DspiError::Disconnected));
        assert_eq!(core.delete_preset_tok(0), Err(DspiError::Disconnected));
        assert_eq!(
            core.set_preset_name(0, "x".into()),
            Err(DspiError::Disconnected)
        );
        assert_eq!(core.get_preset_name(0), Err(DspiError::Disconnected));
        assert_eq!(core.get_preset_directory(), Err(DspiError::Disconnected));
        assert_eq!(core.set_preset_startup(0, 0), Err(DspiError::Disconnected));
        assert_eq!(core.save_params_tok(), Err(DspiError::Disconnected));
        assert_eq!(core.factory_reset_tok(), Err(DspiError::Disconnected));
        assert_eq!(
            core.device_lock_acquire("x".into(), 100),
            Err(DspiError::Disconnected)
        );
        core.device_lock_release();
        core.capture_baseline();
        assert!(core.lock().baseline.is_none());
        assert!(!core.preset_dirty());
        assert!(core.preset_changes().is_empty());
    }

    // ── When the device does not confirm ──────────────────────────────

    #[test]
    fn clean1_a_load_never_confirmed_goes_by_the_directory() {
        let (core, probe) = fetched();
        core.save_preset_tok(1).unwrap();
        probe.reply(op::REQ_PRESET_GET_ACTIVE, Some(vec![7]));
        probe.reply(op::REQ_PRESET_GET_DIR, Some(directory(0b11, 5)));
        core.set_input_preamp(0, 1.0).unwrap();
        core.load_preset(1).unwrap();
        assert_eq!(core.get_state().active_preset_slot, 5);
        assert!(!core.preset_dirty());
    }

    #[test]
    fn clean1_a_confirmed_load_without_a_directory_is_on_its_slot() {
        let (core, probe) = fetched();
        core.save_preset_tok(1).unwrap();
        core.save_preset_tok(0).unwrap();
        probe.reply(op::REQ_PRESET_GET_DIR, None);
        core.load_preset(1).unwrap();
        assert_eq!(core.get_state().active_preset_slot, 1);
        // Neither confirmed nor read: it stays where it was.
        probe.reply(op::REQ_PRESET_GET_ACTIVE, Some(vec![9]));
        core.load_preset(0).unwrap();
        assert_eq!(core.get_state().active_preset_slot, 1);
    }

    #[test]
    fn clean1_the_directory_has_the_last_word_on_the_active_slot() {
        let (core, probe) = fetched();
        core.save_preset_tok(1).unwrap();
        probe.reply(op::REQ_PRESET_GET_DIR, Some(directory(0b11, 4)));
        core.load_preset(1).unwrap();
        assert_eq!(core.get_state().active_preset_slot, 4);
    }

    #[test]
    fn clean1_a_save_never_confirmed_takes_only_the_directory() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -9.0).unwrap();
        let changes = core.preset_changes();
        probe.reply(op::REQ_PRESET_GET_ACTIVE, Some(vec![9]));
        probe.reply(op::REQ_PRESET_GET_DIR, Some(directory(0b101, 0)));
        assert!(matches!(
            core.save_preset_tok(2),
            Err(DspiError::Transfer { .. })
        ));
        let s = core.get_state();
        assert_eq!((s.preset_occupied, s.active_preset_slot), (0b101, 0));
        assert_eq!(core.preset_changes(), changes, "the baseline stays");
    }

    #[test]
    fn clean1_a_delete_never_confirmed_keeps_the_slot_and_its_name() {
        let (core, probe) = fetched();
        core.save_preset_tok(3).unwrap();
        core.set_preset_name(3, "Keep".into()).unwrap();
        probe.reply(op::REQ_PRESET_GET_DIR, Some(directory(0x3FF, 3)));
        assert!(matches!(
            core.delete_preset_tok(3),
            Err(DspiError::Transfer { .. })
        ));
        let s = core.get_state();
        assert_ne!(s.preset_occupied & (1 << 3), 0);
        assert_eq!(s.preset_names[3], "Keep");
    }

    #[test]
    fn clean1_a_copy_never_confirmed_stops_and_keeps_the_baseline() {
        let (core, probe) = fetched();
        core.set_input_preamp(0, -9.0).unwrap();
        probe.reply(op::REQ_PRESET_GET_ACTIVE, Some(vec![9]));
        let mark = probe.mark();
        assert!(matches!(
            core.copy_preset_tok(0, 4),
            Err(DspiError::Transfer { .. })
        ));
        let log = probe.since(mark);
        assert_eq!(count(&log, &format!("in {:02X} ", op::REQ_PRESET_SAVE)), 1);
        assert_eq!(log.last().map(String::as_str), Some("unlock"));
        assert!(core.preset_dirty());
    }

    #[test]
    fn clean1_a_device_that_leaves_ends_the_wait() {
        let mut calls = 0;
        let (spy, _) = spy_on(FakeDevice::demo());
        let mut conn = Connection::new(Box::new(spy), "X".into(), 0);
        let r = wait_for(&mut conn, |_| {
            calls += 1;
            Err(DspiError::Disconnected)
        });
        assert_eq!(r, Err(DspiError::Disconnected));
        assert_eq!(calls, 1);
        let r = wait_for(&mut conn, |_| Ok(true));
        assert_eq!(r, Ok(true));
    }

    // ── The device lock ───────────────────────────────────────────────

    #[test]
    fn clean1_flows_inside_a_held_lock_ask_for_nothing() {
        let (core, probe) = fetched();
        let mark = probe.mark();
        core.device_lock_acquire("Restore".into(), 5000).unwrap();
        assert_eq!(core.held_lock_holder(), Some(Holder::Caller));
        core.save_preset_tok(1).unwrap();
        core.load_preset(1).unwrap();
        core.apply_all_params().unwrap();
        // Again: nothing more is asked.
        core.device_lock_acquire("Again".into(), 5000).unwrap();
        core.device_lock_release();
        let log = probe.since(mark);
        let locks: Vec<_> = log
            .iter()
            .filter(|l| l.starts_with("lock") || *l == "unlock")
            .collect();
        assert_eq!(locks, ["lock Restore 5000", "unlock"]);
        assert_eq!(core.held_lock_holder(), None);
        core.device_lock_release();
        assert_eq!(count(&probe.since(mark), "unlock"), 1, "idempotent");
    }

    #[test]
    fn clean1_a_lease_is_capped_and_never_zero() {
        let (core, probe) = fetched();
        assert_eq!(
            core.device_lock_acquire("x".into(), 0),
            Err(DspiError::InvalidArgument {
                msg: "a device lock needs a timeout".into()
            })
        );
        let mark = probe.mark();
        core.device_lock_acquire("Install".into(), u32::MAX)
            .unwrap();
        assert_eq!(probe.since(mark), ["lock Install 60000"]);
    }

    #[test]
    fn clean1_a_refused_acquire_holds_nothing() {
        let (core, probe) = fetched();
        probe.refuse_lock.store(true, Ordering::SeqCst);
        assert_eq!(
            core.device_lock_acquire("x".into(), 1000),
            Err(DspiError::Locked)
        );
        assert_eq!(core.held_lock_holder(), None);
        probe.refuse_lock.store(false, Ordering::SeqCst);
        let mark = probe.mark();
        core.save_preset_tok(1).unwrap();
        let log = probe.since(mark);
        assert_eq!(count(&log, "lock "), 1);
        assert_eq!(count(&log, "unlock"), 1);
    }

    #[test]
    fn clean1_a_lease_that_ran_out_goes_back_before_the_next_flow() {
        let (core, probe) = fetched();
        core.device_lock_acquire("x".into(), 5000).unwrap();
        core.expire_held_lock();
        assert_eq!(core.held_lock_holder(), None);
        let mark = probe.mark();
        core.save_preset_tok(1).unwrap();
        let log = probe.since(mark);
        let locks: Vec<_> = log
            .iter()
            .filter(|l| l.starts_with("lock") || *l == "unlock")
            .collect();
        assert_eq!(locks, ["unlock", "lock Saving preset 60000", "unlock"]);
    }

    #[test]
    fn clean1_another_holder_becomes_the_callers() {
        let (core, probe) = fetched();
        let lock = core
            .take_device_lock("Learning", Duration::from_secs(15), true)
            .unwrap();
        core.hold_device_lock(lock, Duration::from_secs(15), Holder::Learn);
        core.device_lock_release();
        assert_eq!(core.held_lock_holder(), Some(Holder::Learn));
        core.device_lock_acquire("Flow".into(), 1000).unwrap();
        assert_eq!(core.held_lock_holder(), Some(Holder::Caller));
        core.end_held_lock(Some(Holder::Learn));
        assert_eq!(core.held_lock_holder(), Some(Holder::Caller));
        core.end_held_lock(None);
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(count(&probe.log.lock().unwrap(), "unlock"), 1);
    }

    #[test]
    fn clean1_a_new_connection_holds_nothing() {
        let (core, _) = fetched();
        core.device_lock_acquire("x".into(), 5000).unwrap();
        let (spy, probe) = spy_on(FakeDevice::demo());
        core.attach_probed(Box::new(spy), "NEW".into(), 0).unwrap();
        assert_eq!(core.held_lock_holder(), None);
        core.fetch_all().unwrap();
        let mark = probe.mark();
        core.save_preset_tok(1).unwrap();
        assert_eq!(count(&probe.since(mark), "lock "), 1);
    }

    #[test]
    fn clean1_a_lock_on_a_replaced_connection_leaves_the_new_one_locked() {
        let core = DspiCore::new();
        let (a, first) = spy_on(FakeDevice::demo());
        core.attach(Box::new(a), "A".into(), 0);
        let old = core
            .take_device_lock("Old", Duration::from_secs(1), false)
            .unwrap();
        let (b, second) = spy_on(FakeDevice::demo());
        core.attach(Box::new(b), "B".into(), 0);
        let new = core
            .take_device_lock("New", Duration::from_secs(1), false)
            .unwrap();
        assert!(!old.is_current(&core.lock()));
        assert!(new.is_current(&core.lock()));
        core.release_device_lock(old);
        assert_eq!(count(&second.log.lock().unwrap(), "unlock"), 0);
        assert_eq!(count(&first.log.lock().unwrap(), "unlock"), 0);
        // Its own connection's lock goes back.
        core.release_device_lock(new);
        assert_eq!(*second.log.lock().unwrap(), ["lock New 1000", "unlock"]);
    }

    #[test]
    fn clean1_a_held_lock_on_a_gone_connection_is_given_back() {
        let core = DspiCore::new();
        let (a, first) = spy_on(FakeDevice::demo());
        core.attach(Box::new(a), "A".into(), 0);
        let lock = core
            .take_device_lock("x", Duration::from_secs(1), false)
            .unwrap();
        let (b, _) = spy_on(FakeDevice::demo());
        core.attach(Box::new(b), "B".into(), 0);
        // Not kept on the new connection, and never given to it.
        core.hold_device_lock(lock, Duration::from_secs(1), Holder::Caller);
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(count(&first.log.lock().unwrap(), "unlock"), 0);
    }

    #[test]
    fn clean1_over_a_side_channel_the_lock_goes_there() {
        let (core, probe) = fetched();
        core.lock().conn().unwrap().side = Some(Arc::new(Side(Arc::clone(&probe))));
        let mark = probe.mark();
        core.save_preset_tok(1).unwrap();
        let log = probe.since(mark);
        assert_eq!(
            log.first().map(String::as_str),
            Some("side lock Saving preset")
        );
        assert_eq!(log.last().map(String::as_str), Some("side unlock"));
        assert_eq!(count(&log, "lock "), 0, "not the transport's");

        // A held one goes back when its connection closes.
        core.device_lock_acquire("Hold".into(), 5000).unwrap();
        let mark = probe.mark();
        core.disconnect();
        assert_eq!(probe.since(mark), ["side unlock"]);
    }

    // ── The baseline ──────────────────────────────────────────────────

    #[test]
    fn clean1_without_a_baseline_nothing_has_changed() {
        let (core, _) = probed();
        assert!(core.lock().baseline.is_none());
        core.set_input_preamp(0, -3.0).unwrap();
        assert!(core.preset_changes().is_empty());
        assert!(!core.preset_dirty());
        core.capture_baseline();
        core.set_input_preamp(0, -4.0).unwrap();
        assert_eq!(core.preset_changes().len(), 1);
    }

    #[test]
    fn clean1_dirty_follows_the_image_and_the_live_modes() {
        let (core, _) = fetched();
        assert!(!core.preset_dirty());
        // The demo keeps the master volume apart from presets.
        core.lock().state.master_volume_mode = 0;
        core.set_master_volume(-33.0).unwrap();
        assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
        core.lock().state.master_volume_mode = WITH_PRESET;
        assert!(core.preset_dirty());
        assert!(core.preset_changes()[0].starts_with("Master Volume: "));
        core.lock().state.master_volume_mode = 0;
        core.set_bypass(true).unwrap();
        assert_eq!(core.preset_changes(), ["Master EQ bypass: off → on"]);
        core.set_bypass(false).unwrap();
        assert!(!core.preset_dirty());
    }

    #[test]
    fn clean1_the_same_values_in_other_bytes_are_no_change() {
        let (core, _) = fetched();
        core.apply_all_params().unwrap();
        assert!(!core.preset_dirty());
        // Bytes no value is read from, and the header, differ: still clean.
        let mut inner = core.lock();
        let mut base = inner.state.bulk_raw.clone();
        base[g::OFF_LEGACY] ^= 0x5A;
        base[g::OFF_HEADER + 15] ^= 0x01;
        inner.baseline = Some(base);
        drop(inner);
        assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
        // A baseline that does not decode has nothing to say.
        core.lock().baseline = Some(vec![0; 10]);
        assert!(core.preset_changes().is_empty());
    }

    #[test]
    fn clean1_a_load_elsewhere_keeps_later_edits_unsaved() {
        let (core, _) = fetched();
        let mut inner = core.lock();
        let old = inner.baseline.clone();
        assert!(old.is_some());
        // Nothing was edited: the re-read is the baseline.
        let mirror = inner.state.bulk_raw.clone();
        rebaseline_after_load_locked(&mut inner, old.as_deref(), &mirror);
        assert_eq!(
            inner.baseline.as_deref(),
            Some(inner.state.bulk_raw.as_slice())
        );
        // No baseline before: the re-read becomes it.
        inner.baseline = None;
        rebaseline_after_load_locked(&mut inner, None, &mirror);
        assert_eq!(inner.baseline.as_deref(), Some(mirror.as_slice()));
    }

    #[test]
    fn clean1_changed_runs_and_fields() {
        assert_eq!(changed_runs(&[1, 2, 3, 4], &[1, 9, 9, 4]), vec![1..3]);
        assert_eq!(changed_runs(&[1, 2], &[3, 2]), vec![0..1]);
        assert_eq!(changed_runs(&[1, 2], &[1, 3]), vec![1..2]);
        assert!(changed_runs(&[1], &[1]).is_empty());
        let fields = image_fields();
        assert_eq!(fields.first(), Some(&(0..1)));
        assert_eq!(fields.last().map(|f| f.end), Some(g::BULK_SIZE));
    }

    // ── The rows ──────────────────────────────────────────────────────

    /// The demo's state with a known layout: two inputs, five outputs, no
    /// names (so every label is the firmware's default name), every
    /// persistence mode with the preset, and known values where rows test.
    fn base() -> DspState {
        let mut s = state::default_state();
        state::decode_bulk(&Capture::demo().bulk, &mut s).unwrap();
        s.platform_id = 1;
        s.num_input_channels = 2;
        s.num_output_channels = 5;
        s.num_channels = 7;
        s.output_types = vec![0; 4];
        s.channel_names = vec![String::new(); 17];
        s.output_config_mode = WITH_PRESET;
        s.master_volume_mode = WITH_PRESET;
        s.input_preamp_db = vec![0.0; 8];
        s.master_volume_db = -20.0;
        s.bypass = false;
        s.loudness_enabled = false;
        s.loudness_output_mask = 0;
        s.loudness_ref_spl = 83.0;
        s.crossfeed_freq = 700.0;
        s.crossfeed_pair_map = vec![0; 4];
        s.psybass_cutoff_hz = 80.0;
        s.subharm[usize::from(SUBHARM_PARAM_LOW)] = -6.0;
        s.subharm[usize::from(SUBHARM_PARAM_CEILING)] = -3.0;
        s.subharm[usize::from(SUBHARM_PARAM_SELECT)] = 0.0;
        s.tube[g::tube::TUBE_PARAM_TUBE_TYPE as usize] = 1.0;
        s.limiter_threshold_db = vec![-1.0; 9];
        s.upmix[usize::from(UPMIX_PARAM_CENTER_MODE)] = 0.0;
        s.leveller_speed = 1;
        s.channel_delays = vec![0.0; 17];
        s.output_muted = vec![false; 9];
        s.output_gain_db = vec![0.0; 9];
        s.input_source = 0;
        s.lg_sound_sync_enabled = false;
        s.spdif_inputs_enabled = 0x01;
        s.spdif_inputs_enabled_present = true;
        s
    }

    fn rows(saved: &DspState, live: &DspState) -> Vec<String> {
        changes(saved, live, &[])
    }

    #[test]
    fn clean1_every_kind_of_row_in_the_consoles_words_and_order() {
        let saved = base();
        let mut l = saved.clone();
        l.lg_sound_sync_enabled = true;
        l.input_source = 2;
        l.output_pins[1] ^= 1;
        l.channel_names[4] = "Sub".into();
        l.xover[6][0].freq += 10.0;
        l.xover[6][1].q += 0.5;
        l.filters[1][0].gain += 1.0;
        l.output_gain_db[1] = -1.0;
        l.output_muted[1] = true;
        l.matrix_gain[0][0] += 1.0;
        l.matrix_routing[1][1] = !l.matrix_routing[1][1];
        l.channel_delays[0] = 1.25;
        l.leveller_speed = 2;
        l.upmix[usize::from(UPMIX_PARAM_CENTER_MODE)] = 2.0;
        l.limiter_threshold_db[0] = -3.0;
        l.tube[g::tube::TUBE_PARAM_TUBE_TYPE as usize] = 0.0;
        l.subharm[usize::from(SUBHARM_PARAM_SELECT)] = 1.0;
        l.subharm[usize::from(SUBHARM_PARAM_CEILING)] = 0.0;
        l.subharm[usize::from(SUBHARM_PARAM_LOW)] = -30.0;
        l.psybass_cutoff_hz = 100.0;
        l.crossfeed_pair_map = vec![0x02, 0, 0, 0];
        l.crossfeed_freq = 650.0;
        l.loudness_ref_spl = 85.5;
        l.loudness_output_mask = 3;
        l.loudness_enabled = true;
        l.bypass = true;
        l.master_volume_db = -200.0;
        l.input_preamp_db[1] = -3.0;
        let tube_was = dspi_session::tube::type_name(1);
        assert_eq!(
            rows(&saved, &l),
            [
                "Preamp R: 0.0 dB → -3.0 dB".to_owned(),
                "Master Volume: -20.0 dB → -∞ dB".into(),
                "Master EQ bypass: off → on".into(),
                "Loudness: enabled".into(),
                "Loudness outputs: 0x0000 → 0x0003".into(),
                "Loudness ref SPL: 83 → 85.5".into(),
                "Crossfeed frequency: 700 → 650 Hz".into(),
                "Crossfeed pair 1 outputs: 1/2 → 3/1".into(),
                "Psybass cutoff: 80 → 100 Hz".into(),
                "Subharm 24-36 Hz: -6 dB → off".into(),
                "Subharm selectivity: all material → percussive".into(),
                "Subharm sub ceiling: -3 dBFS → off".into(),
                format!("Tube type: {tube_was} → Custom"),
                "SPDIF 1 L limiter threshold: -1.0 → -3.0 dBFS".into(),
                "Centre mode: Sinner → Off".into(),
                "Leveller speed: Medium → Fast".into(),
                // -- WP CG-SWS-a -- the firmware names inputs by the (new) source
                "I2S 1 L delay: 0 ms → 1.25 ms".into(),
                "2 crosspoints changed".into(),
                "SPDIF 1 R muted".into(),
                "SPDIF 1 R gain: 0.0 dB → -1.0 dB".into(),
                "1 band changed on I2S 1 R".into(),
                "2 crossover bands changed on PDM".into(),
                "'' → 'Sub'".into(),
                "1 pin assignment changed".into(),
                "Input source: USB → I2S".into(),
                "LG Sound Sync: enabled".into(),
            ]
        );
        assert!(rows(&saved, &saved).is_empty());
    }

    #[test]
    fn clean1_the_live_modes_gate_their_rows() {
        let saved = base();
        let mut l = saved.clone();
        l.master_volume_db = -30.0;
        l.limiter_enabled[2] = !l.limiter_enabled[2];
        l.i2s_bck_pin ^= 1;
        assert_eq!(
            rows(&saved, &l),
            [
                "Master Volume: -20.0 dB → -30.0 dB",
                &format!("SPDIF 2 L limiter: {}", enabled(l.limiter_enabled[2])),
                &format!(
                    "BCK pin: GPIO {} → GPIO {}",
                    saved.i2s_bck_pin, l.i2s_bck_pin
                ),
            ]
        );
        // The saved state's own modes do not matter; the live ones do.
        let mut s = saved.clone();
        s.master_volume_mode = 0;
        s.output_config_mode = 0;
        assert_eq!(rows(&s, &l).len(), 3);
        l.master_volume_mode = 0;
        l.output_config_mode = 0;
        assert!(rows(&saved, &l).is_empty());
    }

    #[test]
    fn clean1_pairs_on_their_own_outputs_are_no_change() {
        let saved = base();
        let mut l = saved.clone();
        // Pair 2 named explicitly on its default outputs 3 and 4.
        l.crossfeed_pair_map = vec![0, 0x32, 0, 0];
        assert!(rows(&saved, &l).is_empty());
        // A map cut short reads as default.
        l.crossfeed_pair_map = vec![];
        assert!(rows(&saved, &l).is_empty());
        l.crossfeed_pair_map = vec![0, 0x23];
        assert_eq!(rows(&saved, &l), ["Crossfeed pair 2 outputs: 3/4 → 4/3"]);
        assert_eq!(pair_outputs(&[], 3), (6, 7));
    }

    #[test]
    fn clean1_volume_and_mute_outside_the_preset_are_no_change() {
        let saved = base();
        let mut l = saved.clone();
        l.user_volume_db -= 6.0;
        l.user_mute = !l.user_mute;
        l.dac_mute_enabled = !l.dac_mute_enabled;
        l.dac_mute_pin ^= 1;
        l.dac_mute_hold_ms += 3;
        l.subharm_solo = !l.subharm_solo;
        assert!(rows(&saved, &l).is_empty());
        // The pair map is part of the preset.
        l.crossfeed_pair_map = vec![0x02, 0, 0, 0];
        assert_eq!(rows(&saved, &l).len(), 1);
    }

    #[test]
    fn clean1_a_default_name_to_another_is_no_rename() {
        let mut saved = base();
        saved.output_config_mode = 0;
        saved.channel_names[2] = "SPDIF 1 L".into();
        let mut l = saved.clone();
        // The slot became I2S: the firmware renamed the channel.
        l.output_types[0] = 1;
        l.channel_names[2] = "I2S 1 L".into();
        assert!(rows(&saved, &l).is_empty());
        // A name the user chose, or another channel's default, is a rename.
        l.channel_names[2] = "Left".into();
        assert_eq!(rows(&saved, &l), ["'SPDIF 1 L' → 'Left'"]);
        l.channel_names[2] = "I2S 1 R".into();
        assert_eq!(rows(&saved, &l), ["'SPDIF 1 L' → 'I2S 1 R'"]);
    }

    #[test]
    fn clean1_an_unnamed_channel_reads_as_its_firmware_name() {
        let saved = base();
        let mut l = saved.clone();
        for ch in 0..7 {
            l.channel_delays[ch] = 1.0;
        }
        let names: Vec<String> = rows(&saved, &l)
            .iter()
            .map(|r| r.trim_end_matches(" delay: 0 ms → 1 ms").to_owned())
            .collect();
        assert_eq!(
            names,
            [
                "USB 1",
                "USB 2",
                "SPDIF 1 L",
                "SPDIF 1 R",
                "SPDIF 2 L",
                "SPDIF 2 R",
                "PDM"
            ]
        );
        // A name, when there is one.
        l.channel_names[6] = "Sub".into();
        let mut s = saved.clone();
        s.channel_names[6] = "Sub".into();
        assert_eq!(rows(&s, &l).last().unwrap(), "Sub delay: 0 ms → 1 ms");
        // More inputs: their numbers go on.
        assert_eq!(
            firmware_name(
                &{
                    let mut x = base();
                    x.num_input_channels = 8;
                    x
                },
                5
            )
            .unwrap(),
            "USB 6"
        );
        assert_eq!(firmware_name(&base(), 7), None);
    }

    #[test]
    fn clean1_the_first_spdif_is_numbered_while_another_is_on() {
        let mut saved = base();
        saved.input_source = 1;
        let mut l = saved.clone();
        l.input_source = 0;
        assert_eq!(rows(&saved, &l), ["Input source: S/PDIF → USB"]);
        l.spdif_inputs_enabled = 0x03;
        let mut s = saved.clone();
        s.spdif_inputs_enabled = 0x03;
        assert_eq!(rows(&s, &l), ["Input source: S/PDIF 1 → USB"]);
        assert_eq!(source_title(5, &[], false), "S/PDIF 3");
        assert_eq!(source_title(3, &[], false), "ADAT");
        // -- WP SW1 (fixup sw1, F4) --
        assert_eq!(source_title(7, &[], false), "Audio link");
        assert_eq!(source_title(9, &[], false), "Source 9");
    }

    #[test]
    fn clean1_the_users_source_name_comes_first() {
        let saved = base();
        let mut l = saved.clone();
        l.input_source = 2;
        let names: Vec<String> = ["Mac", "", "Turntable", "", "", "", ""]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(
            changes(&saved, &l, &names),
            ["Input source: Mac → Turntable"]
        );
    }

    #[test]
    fn clean1_spdif_switches_come_before_their_pins() {
        let saved = base();
        let mut l = saved.clone();
        l.spdif_inputs_enabled = 0x05;
        l.spdif_rx_pins[2] = saved.spdif_rx_pins[2].wrapping_add(1);
        assert_eq!(
            rows(&saved, &l),
            [
                "S/PDIF 3 input: enabled".to_owned(),
                format!(
                    "S/PDIF 3 RX pin: GPIO {} → GPIO {}",
                    saved.spdif_rx_pins[2], l.spdif_rx_pins[2]
                ),
            ]
        );
        // Firmware without the optional inputs has no such rows.
        l.spdif_inputs_enabled_present = false;
        assert!(rows(&saved, &l).is_empty());
    }

    #[test]
    fn clean1_values_print_as_the_console_prints_them() {
        assert_eq!(db(-3.0), "-3.0 dB");
        assert_eq!(db(0.04), "0.0 dB");
        assert_eq!(volume(-128.0), "-∞ dB");
        assert_eq!(volume(-127.9), "-127.9 dB");
        assert_eq!(plain(83.0), "83");
        assert_eq!(plain(85.5), "85.5");
        assert_eq!(plain(-0.25), "-0.2");
        assert_eq!(plain(250_000.0), "250000.0");
        assert_eq!(delay(0.0), "0");
        assert_eq!(delay(1.5), "1.5");
        assert_eq!(delay(2.0), "2");
        assert_eq!(delay(0.123_456), "0.1235");
        assert_eq!(band_level(-30.0), "off");
        assert_eq!(band_level(-12.5), "-12.5 dB");
        assert_eq!(ceiling(0.0), "off");
        assert_eq!(ceiling(-6.0), "-6 dBFS");
        assert_eq!(selectivity(2), "sustained");
        assert_eq!(selectivity(-1), "all material");
        assert_eq!(rate_hz(0), 44_100);
        assert_eq!(rate_hz(7), 48_000);
        assert_eq!(mck_multiplier(1), 256);
        assert_eq!(clock_mode(1), "Slave");
        assert!(same(f32::NAN, f32::NAN));
        assert!(same(0.0, -0.0));
        assert_eq!(plural(1), "");
        assert_eq!(plural(2), "s");
    }
}
