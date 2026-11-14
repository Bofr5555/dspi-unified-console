//! C functions: confirmation tokens for dangerous operations (WP P1TK), over
//! `dspi_core::confirm`.
//!
//! `dspi_confirm_token` mints a single-use token for one operation, valid for
//! 10 s on the open connection; the `_confirmed` functions take it. Tokens
//! are always required (WP P1TZ, OD-P1f): a call without a good token, a null
//! one included, and the plain functions (`dspi_save_preset`,
//! `dspi_config_import`, ...), are refused before anything is sent:
//! `{"ok": false, "code": "confirmation_required", "error", "noticeKey":
//! "confirmation_required"}` (a status function answers its failure status).
//! There is no switch: `dspi_require_confirmation` and
//! `dspi_confirmation_required` are gone. Each `_confirmed` call has happened (and spent its
//! token) whatever the buffer, so do not call again for a short one: its reply
//! is `{"ok": true}` or a failure of a few hundred bytes, except the import's,
//! which is `dspi_config_import_selected`'s.
//!
//! Operations, by key: `save_preset`, `copy_preset`, `delete_preset`,
//! `save_params`, `factory_reset`, `save_output_config`, `enter_bootloader`,
//! `config_import`, `vreg_save`, `firmware_flash`.

use std::ffi::c_char;

use dspi_core::{ConfirmOp, DspiError};

use super::{
    arg, board_needing, c_bytes, c_str, done_reply, drive_at, json_reply, optional_text, text,
};
use crate::FfiCore;

/// A token argument: null is none; bytes that are not UTF-8 are refused.
fn token(p: *const c_char) -> dspi_core::Result<Option<String>> {
    optional_text(p, "token")
}

/// A `_confirmed` call's JSON reply: `{"ok": true}` or its failure.
fn confirmed(
    core: *mut FfiCore,
    tok: *const c_char,
    buf: *mut c_char,
    len: u32,
    f: impl FnOnce(&dspi_core::DspiCore, Option<String>) -> dspi_core::Result<()>,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(token(tok).and_then(|t| f(c, t)))
    })
}

/// Mint a token for the operation `op` (a key above) once the user has
/// confirmed it. JSON `{ok, value: <token>}`, or `invalid` for an operation
/// the core does not have. Each call mints a new token. No I/O.
///
/// Tokens are always required (WP P1TZ, OD-P1f; a behaviour change of this
/// ABI): every gated operation (`save_preset`, `copy_preset`,
/// `delete_preset`, `save_params`, `factory_reset`, `save_output_config`,
/// `enter_bootloader`, `config_import`, `vreg_save`, `firmware_flash`) is
/// refused with code `confirmation_required` and nothing sent unless its
/// `_confirmed` function gets a good token from here. The plain functions
/// (`dspi_save_preset`, `dspi_copy_preset`, `dspi_delete_preset`,
/// `dspi_config_import`, `dspi_config_import_selected`,
/// `dspi_core_vreg_cal_commit`, `dspi_core_vreg_manual_set`) are always
/// refused so; there is no switch to turn this off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_confirm_token(
    core: *mut FfiCore,
    op: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let op = arg(op, "op").and_then(|k| {
            ConfirmOp::from_key(k).ok_or_else(|| DspiError::InvalidArgument {
                msg: format!("{k} is not an operation that is confirmed"),
            })
        });
        json_reply(op.map(|op| c.confirm_token(op)))
    })
}

/// `dspi_save_preset` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_preset_confirmed(
    core: *mut FfiCore,
    slot: u8,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.save_preset_confirmed(slot, t)
    })
}

/// `dspi_copy_preset` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_copy_preset_confirmed(
    core: *mut FfiCore,
    source: u8,
    dest: u8,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.copy_preset_confirmed(source, dest, t)
    })
}

/// `dspi_delete_preset` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_delete_preset_confirmed(
    core: *mut FfiCore,
    slot: u8,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.delete_preset_confirmed(slot, t)
    })
}

/// `dspi_save_params` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_params_confirmed(
    core: *mut FfiCore,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| c.save_params_confirmed(t))
}

/// `dspi_factory_reset` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_factory_reset_confirmed(
    core: *mut FfiCore,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| c.factory_reset_confirmed(t))
}

/// `dspi_save_output_config` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_output_config_confirmed(
    core: *mut FfiCore,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.save_output_config_confirmed(t)
    })
}

/// `dspi_enter_bootloader` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_enter_bootloader_confirmed(
    core: *mut FfiCore,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.enter_bootloader_confirmed(t)
    })
}

/// `dspi_config_import_selected` with a token: its JSON reply, or a refusal
/// with `noticeKey` `confirmation_required` and nothing written.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_config_import_confirmed(
    core: *mut FfiCore,
    text: *const c_char,
    options: u32,
    eq_channels: u32,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    super::text(core, buf, len, |c| {
        match (arg(text, "text"), self::token(token)) {
            (Ok(doc), Ok(t)) => c.config_import_confirmed(doc.to_owned(), options, eq_channels, t),
            (Err(e), _) | (_, Err(e)) => json_reply::<()>(Err(e)),
        }
    })
}

/// `dspi_core_vreg_cal_commit` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_commit_confirmed(
    core: *mut FfiCore,
    typed: *const c_char,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.core_vreg_cal_commit_confirmed(optional_text(typed, "typed")?, t)
    })
}

/// `dspi_core_vreg_manual_set` with a token. JSON reply.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_manual_set_confirmed(
    core: *mut FfiCore,
    value: u8,
    typed: *const c_char,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.core_vreg_manual_set_confirmed(value, optional_text(typed, "typed")?, t)
    })
}

/// `dspi_flash_to_drive_guarded` with a token minted while the board waits
/// in its bootloader (`firmware_flash`). JSON reply; nothing reaches the
/// drive on a refusal.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_flash_confirmed(
    core: *mut FfiCore,
    image: *const u8,
    image_len: u32,
    drive_path: *const c_char,
    platform: i8,
    board_required_mv: u16,
    typed: *const c_char,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        let drive = drive_at(drive_path, platform)?;
        c.flash_confirmed(
            c_bytes(image, image_len),
            drive,
            board_needing(board_required_mv),
            c_str(typed).unwrap_or_default().to_owned(),
            t,
        )
    })
}

#[cfg(test)]
#[path = "tests_wp_p1tk.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1tk;

// -- WP P1TZ -- tokens are always required: other modules' tests call a gated
// `_confirmed` function with a token minted for it, as a client does.
#[cfg(test)]
pub(crate) fn with_token_for_test(
    h: *mut FfiCore,
    op: &str,
    f: impl FnOnce(*const c_char, *mut c_char, u32) -> u32,
) -> serde_json::Value {
    let read = |g: &mut dyn FnMut(*mut c_char, u32) -> u32| {
        let mut buf = vec![0u8; 1 << 20];
        let n = g(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len(), "{n}");
        serde_json::from_slice::<serde_json::Value>(&buf[..n.saturating_sub(1)]).unwrap_or_default()
    };
    let key = std::ffi::CString::new(op).unwrap_or_default();
    let minted = read(&mut |b, l| dspi_confirm_token(h, key.as_ptr(), b, l));
    let token =
        std::ffi::CString::new(minted["value"].as_str().unwrap_or_default()).unwrap_or_default();
    let mut f = Some(f);
    read(&mut |b, l| f.take().map_or(0, |f| f(token.as_ptr(), b, l)))
}
