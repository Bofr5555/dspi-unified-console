//! C functions: safe apply with rollback (WP P1SA), over `dspi_core`'s
//! guarded apply (`guarded.rs`).
//!
//! A guarded apply captures a snapshot, applies, and waits for
//! `dspi_apply_confirm`; the timeout (on `dspi_apply_poll`), `dspi_apply_revert`
//! or a failed apply put the snapshot back. Over DSPi Link the hub guards it
//! too. Every reply is JSON: the state `{ok, id, phase, label, hub, timeoutMs,
//! remainingMs, reason, message, changes, notes}` (`phase`: `idle`, `waiting`,
//! `kept`, `reverted`, `revert_failed`, `not_written`, `lost`), or a failure.
//!
//! A guarded call has happened whatever the buffer: for a short one, read the
//! state with `dspi_apply_status` rather than calling again.

use std::ffi::c_char;

use super::{arg, json_reply, text};
use crate::{FfiCore, with_core};

/// Import the `.dspipreset` document `doc` with a way back: `options` and
/// `eq_channels` as `dspi_config_import_confirmed` takes them, `timeout_s`
/// seconds for the Keep (0: 30; at most 590). No confirmation token. JSON:
/// the state (`waiting`, with the changes), or a failure (`invalid` for no
/// document, an empty selection, or another apply waiting; the hub's refusal
/// as `link`; the import's own failure after the snapshot was put back).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_guarded_import(
    core: *mut FfiCore,
    doc: *const c_char,
    options: u32,
    eq_channels: u32,
    timeout_s: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(doc, "doc").and_then(|d| {
                c.apply_guarded_import(d.to_owned(), options, eq_channels, timeout_s)
            }),
        )
    })
}

/// Recall snapshot `id` from the store with a way back. JSON as
/// `dspi_apply_guarded_import`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_guarded_snapshot(
    core: *mut FfiCore,
    id: *const c_char,
    timeout_s: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.apply_guarded_snapshot(i.to_owned(), timeout_s)))
    })
}

/// Recall a snapshot document's text with a way back (only a snapshot).
/// JSON as `dspi_apply_guarded_import`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_guarded_snapshot_document(
    core: *mut FfiCore,
    doc: *const c_char,
    timeout_s: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(doc, "doc")
                .and_then(|d| c.apply_guarded_snapshot_document(d.to_owned(), timeout_s)),
        )
    })
}

/// Begin a guard around a flow the client writes itself, named `label`; then
/// Keep or Revert. JSON: the state, or a failure as for an import.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_guard_begin(
    core: *mut FfiCore,
    label: *const c_char,
    timeout_s: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(label, "label").and_then(|l| c.apply_guard_begin(l.to_owned(), timeout_s)))
    })
}

/// Keep the change. JSON: the state (`kept`; past the timeout the revert's
/// end; `lost` when the hub could not be told), or `invalid` when nothing
/// waits, or for the first Keep of an auto-tune whose verify did not pass
/// (`measure.tune.keep_unverified`, as `dspi_measure_tune_keep`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_confirm(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.apply_confirm()))
}

/// Put the settings from before back now (after a lost USB connection: once
/// the same device is open again). JSON: the state, or `invalid` when there
/// is nothing to revert.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_revert(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.apply_revert()))
}

/// Drive the guard from the client's poll, off the UI thread: the timeout's
/// revert, a lost connection, the hub's word after a reconnect. JSON: the
/// state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_poll(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.apply_poll())))
}

/// The state, for the countdown bar. No I/O. JSON: the state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_status(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.apply_status())))
}

/// Whether imports and recalls are guarded by default here (over DSPi Link
/// to a hub with the guard). False for a null handle. No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_guard_default(core: *mut FfiCore) -> bool {
    with_core(core, false, |h| h.core.apply_guard_default())
}
