//! The client read-only mode (WP ROMODE): a client asks the core to refuse
//! every write, for monitoring a unit that must not change and for testing
//! against one.
//!
//! [`DspiCore::set_client_read_only`] turns it on and off; it stays across a
//! disconnect and a reconnect, for every device this core opens, until turned
//! off. While it is on:
//!
//! - every write is refused with [`DspiError::ReadOnly`] before anything is
//!   sent, where every write starts (`Inner::writable_conn`): the typed
//!   setters, `write_param`, control surfaces, imports, snapshot recalls, A/B,
//!   undo and redo, the guarded apply, the core voltage, identify, the signal
//!   generator and the audio link's setters;
//! - the gated calls (`*_confirmed`: preset save, copy and delete, save,
//!   factory reset, the bootloader, a configuration import, a core-voltage
//!   save, a firmware flash) are refused before their token is checked, so a
//!   token is not spent;
//! - the raw path refuses every [`DspiCore::raw_set`], and a
//!   [`DspiCore::raw_get`] of anything but the registry's pure reads
//!   ([`dspi_proto::registry::is_pure_read`]: the `GET` table and the
//!   extension channel's IN side); the write-as-read housekeeping a read-only
//!   connection may still send raw (clear clips, the stats resets) is refused
//!   too; -- WP PGCORE -- likewise every [`DspiCore::raw_set_id`], while a
//!   [`DspiCore::raw_get_id`] passes (every GET of the paged registry and the
//!   extension tunnel reads; a command outside both is refused);
//! - a hub request ([`DspiCore::link_hub_request`], and through it the hub's
//!   admin API, its limits and lock; and the audio link's System view's own
//!   `system.*` requests, -- WP ROMODEF --) goes out only when it reads
//!   ([`HUB_READS`]): `system.set`, `system.align`, `system.scene.apply` are
//!   refused;
//! - -- WP ROMODEF -- the device's counters too, which a read-only
//!   *connection* may still clear (§1.2): typed `clear_clips` and
//!   `reset_buffer_stats` are refused, and the meters' automatic clear after
//!   the clip hold is not sent (the hold then follows the device's sticky
//!   flags, as for a hub's viewer). Another console's clip evidence and soak
//!   watermarks stay as they are.
//!
//! Reads, notifications, the meters and the typed analyser keep working: the
//! analyser's configuration push and STOP change no setting (as the
//! installer's lock lets them through, P1DKFIX F7), so they pass: the one
//! write the mode lets out. Local files (snapshot capture, export) are not
//! the device's and are not refused.
//!
//! A refusal counts in [`DspiCore::client_read_only_refusals`], not in
//! [`DspiCore::read_only_refusals`] (a read-only *connection*'s, which a
//! client's own guard should keep at 0). Its notice is the `read_only.client`
//! row ([`NOTICE_READ_ONLY_CLIENT`]): [`DspiCore::notice_for_error`] gives it
//! for a `ReadOnly` while the mode is on, and the C ABI's and the web
//! binding's failure replies carry it as their `noticeKey`.
//!
//! **This is a client-side diagnostic, not a security boundary.** The client
//! turns it on and can turn it off; another client of the same device (or of
//! the same hub) is not bound by it. What may change a device is decided by
//! the device's firmware, a hub's roles, limits and lock (DSPi Link), and the
//! installer's lock, never by this flag.

use std::cell::Cell;

use crate::notice_text::{Notice, notice_for, notice_for_key};
use crate::{DspiCore, DspiError, Inner, Result};

/// The notice row of a write refused by the client read-only mode.
pub const NOTICE_READ_ONLY_CLIENT: &str = "read_only.client";

/// The notice row of a refusal to turn the mode on while a guarded apply
/// waits ([`DspiCore::set_client_read_only`]).
pub const NOTICE_READ_ONLY_GUARD_PENDING: &str = "read_only.guard_pending";

/// The words of that refusal (an `InvalidArgument`'s message).
pub const GUARD_PENDING: &str = "A safe apply is waiting for Keep or Revert: keep or revert it before turning read-only mode on.";

/// The hub requests that only read, the ones the client read-only mode lets
/// out: the limits and pinned mode (the lock's refresh), the guarded apply's
/// status, and the audio link's System view's reads.
pub const HUB_READS: [&str; 6] = [
    "limits.get",
    "mode.pinned",
    "apply.status",
    "system.get",
    "system.status",
    "system.alarms",
];

thread_local! {
    /// Depth of [`Viewing`] on this thread.
    static VIEWING: Cell<u32> = const { Cell::new(0) };
}

/// While alive, this thread's writes that only serve viewing (the
/// analyser's configuration push and STOP) pass the client read-only mode.
pub(crate) struct Viewing(());

impl Viewing {
    pub(crate) fn allow() -> Self {
        VIEWING.with(|d| d.set(d.get().saturating_add(1)));
        Self(())
    }
}

impl Drop for Viewing {
    fn drop(&mut self) {
        VIEWING.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

fn viewing() -> bool {
    VIEWING.with(|d| d.get() > 0)
}

/// The client read-only mode's state, in `Inner`.
#[derive(Debug, Default)]
pub(crate) struct ClientReadOnly {
    pub(crate) on: bool,
    pub(crate) refusals: u32,
}

/// Refuse a write while the client read-only mode is on: counted, nothing
/// sent. The analyser's own writes ([`Viewing`]) pass.
pub(crate) fn check(inner: &mut Inner) -> Result<()> {
    if !inner.client_read_only.on || viewing() {
        return Ok(());
    }
    refuse(inner)
}

/// Refuse a raw IN transfer of `opcode` unless it is a pure read.
pub(crate) fn check_raw_get(inner: &mut Inner, opcode: u8) -> Result<()> {
    if !inner.client_read_only.on || dspi_proto::registry::is_pure_read(opcode) {
        return Ok(());
    }
    refuse(inner)
}

// -- WP PGCORE --
/// Refuse a raw paged GET of `id` unless it reads (PLAN-PG section 1.6):
/// every GET of a command in this build's registry reads on paged firmware,
/// and so does the extension tunnel's; one outside both is judged as a write,
/// as a hub judges it. `raw_set_id` is refused by [`check_raw_set`].
pub(crate) fn check_raw_get_id(inner: &mut Inner, id: dspi_transport::CmdId) -> Result<()> {
    use dspi_proto::paged::{EXT_TUNNEL, registry};
    let reads = id == EXT_TUNNEL || registry::command(id).is_some_and(|c| c.get);
    if !inner.client_read_only.on || reads {
        return Ok(());
    }
    refuse(inner)
}

/// Refuse a raw OUT transfer: every one is a write.
pub(crate) fn check_raw_set(inner: &mut Inner) -> Result<()> {
    if !inner.client_read_only.on {
        return Ok(());
    }
    refuse(inner)
}

/// Refuse a hub request `t` that is not one of [`HUB_READS`].
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn check_hub_request(inner: &mut Inner, t: &str) -> Result<()> {
    if !inner.client_read_only.on || HUB_READS.contains(&t) {
        return Ok(());
    }
    // -- WP P2CC1 -- the reads of the hub's own families
    if crate::hub_server::HUB_SERVER_READS.contains(&t) {
        return Ok(());
    }
    // -- WP P3WGR -- settling a bulk apply's run (keep or revert what it wrote);
    // -- review p3d -- and stopping a rollout (`fw.rollout.cancel`)
    if crate::hub_server::HUB_SERVER_SETTLES.contains(&t) {
        return Ok(());
    }
    refuse(inner)
}

fn refuse(inner: &mut Inner) -> Result<()> {
    inner.client_read_only.refusals = inner.client_read_only.refusals.saturating_add(1);
    Err(DspiError::ReadOnly)
}

/// `key` as this core answers it: a `read_only` failure is the client
/// mode's row while the mode is on.
pub(crate) fn refine_key(on: bool, key: &str) -> Option<&'static str> {
    (on && key == "read_only").then_some(NOTICE_READ_ONLY_CLIENT)
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Turn the client read-only mode on or off (module docs): while on,
    /// every write is refused with [`DspiError::ReadOnly`] before anything is
    /// sent, and reads, notifications, the meters and the analyser go on. It
    /// applies to every device this core opens until turned off. A client-side
    /// diagnostic, not a security boundary. Waits for a call in flight.
    ///
    /// Turning it on is refused while a guarded apply waits for Keep or
    /// Revert (its revert would be refused too, leaving the device on the
    /// trial): [`DspiError::InvalidArgument`] with [`GUARD_PENDING`]'s words,
    /// the `read_only.guard_pending` row ([`DspiCore::notice_for_error`]),
    /// nothing changed. Keep or revert first. Turning it off always works.
    pub fn set_client_read_only(&self, on: bool) -> Result<()> {
        if !on {
            self.store_client_read_only(false);
            return Ok(());
        }
        // -- WP P5SES -- (review P5-B) A measurement that plays is stopped
        // first, while its one STOP_NOW may still go out: in read-only mode
        // the session's own stop would be refused and the generator would
        // play on to the end of its run.
        self.measure_stop();
        self.with_guard_settled(|waiting| {
            if waiting {
                return Err(DspiError::InvalidArgument {
                    msg: GUARD_PENDING.to_owned(),
                });
            }
            self.store_client_read_only(true);
            Ok(())
        })
    }

    /// Whether the client read-only mode is on.
    pub fn client_read_only(&self) -> bool {
        self.lock().client_read_only.on
    }

    /// How many writes the client read-only mode has refused on this core.
    pub fn client_read_only_refusals(&self) -> u32 {
        self.lock().client_read_only.refusals
    }

    /// What to tell the user about `error`, as [`notice_for`] does, but in
    /// this core's context: [`DspiError::ReadOnly`] while the client
    /// read-only mode is on is the `read_only.client` row.
    pub fn notice_for_error(&self, error: DspiError) -> Notice {
        // -- the refusal to turn the mode on during a guarded apply
        if matches!(&error, DspiError::InvalidArgument { msg } if msg == GUARD_PENDING) {
            return notice_for_key(
                NOTICE_READ_ONLY_GUARD_PENDING.to_owned(),
                error.notice_detail(),
            );
        }
        match refine_key(self.client_read_only(), &error.notice_key()) {
            Some(key) => notice_for_key(key.to_owned(), error.notice_detail()),
            None => notice_for(error),
        }
    }
}

impl DspiCore {
    fn store_client_read_only(&self, on: bool) {
        let mut inner = self.lock();
        if inner.client_read_only.on != on {
            log::info!("client read-only mode {}", if on { "on" } else { "off" });
        }
        inner.client_read_only.on = on;
    }

    /// [`check`] for a call that starts a write without `writable_conn`
    /// (an A/B compare): refused with no device too.
    pub(crate) fn client_write_check(&self) -> Result<()> {
        check(&mut self.lock())
    }
}

/// For the C ABI and the web binding, not in the UniFFI bindings.
impl DspiCore {
    /// A failure reply's `noticeKey` in this core's context: `read_only`
    /// becomes `read_only.client` while the client read-only mode is on
    /// (as [`notice_for_error`](Self::notice_for_error)); `None` when it
    /// stays as it is.
    pub fn client_notice_key(&self, key: &str) -> Option<&'static str> {
        refine_key(self.client_read_only(), key)
    }
}

#[cfg(test)]
#[path = "read_only/tests_wp_romode.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_romode;
// -- WP P3WGR --
#[cfg(test)]
#[path = "read_only/tests_wp_p3wgr.rs"]
mod tests_wp_p3wgr;
