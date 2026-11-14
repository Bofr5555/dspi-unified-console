//! C functions: limits profiles (WP P1LM, PRO1 S1), over `dspi_core::limits`.
//! Replies as in the parent module's docs: JSON, or the profile document's
//! text itself. A write refused by a limit fails with `{"ok": false, "code":
//! "limit_exceeded", "error", "noticeKey": "limit_exceeded"}`.

use std::ffi::c_char;

use super::{arg, done_reply, json_reply, text};

/// Keep limits profiles in files under `dir`, the app's own data folder, one
/// subfolder per serial ("" forgets the folder). JSON `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_limits_set_dir(
    core: *mut crate::FfiCore,
    dir: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(dir, "dir").and_then(|d| c.limits_set_dir(d.to_owned())))
    })
}

/// The open device's limits profile as a document (`meta.kind = "limits"`;
/// never a PIN), "" when it has none. The text itself.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_limits_profile_document(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.limits_profile_document())
}

/// Set the open device's limits profile from `doc` (a limits document, or
/// any document with `meta.limits`), keep it, and hold the device to it at
/// once. JSON `{ok, lines, held, notes}`, or a failure (`invalid` for a
/// document with no readable profile, `io` when its file could not be
/// written).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_limits_apply_profile(
    core: *mut crate::FfiCore,
    doc: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(doc, "doc").and_then(|d| c.limits_apply_profile(d.to_owned())))
    })
}

/// Remove the open device's limits profile (and its files). JSON `{ok}` or
/// a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_limits_clear(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| done_reply(c.limits_clear()))
}

/// The open device's limits: JSON `{ok, active, serial, state, noticeKey,
/// detail, lastHeld}` (`state`: `none`, `ok`, `fell_back`, `unreadable`;
/// `noticeKey` "" or the standing notice, through `dspi_notice_for_key`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_limits_status(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.limits_status())))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1lm;
