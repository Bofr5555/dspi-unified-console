//! C functions: confirmation tokens for dangerous operations (WP P1TK), over
//! `dspi_core::confirm`.
//!
//! `dspi_confirm_token` mints a single-use token for one operation, valid for
//! 10 s on the open connection; the `_confirmed` functions take it. Tokens
//! are always required (WP P1TZ, OD-P1f): a call without a good token, a null
//! one included, is refused before anything is sent: `{"ok": false, "code":
//! "confirmation_required", "error", "noticeKey": "confirmation_required"}`.
//! The forms without a token (`dspi_save_preset`, `dspi_config_import`, ...)
//! and the switch (`dspi_require_confirmation`, `dspi_confirmation_required`)
//! are removed; `dspi_write_param` refuses the gated paths. Each `_confirmed`
//! call has happened (and spent its token) whatever the buffer, so do not call
//! again for a short one: its reply is `{"ok": true}` or a failure of a few
//! hundred bytes, except the import's (the document's report).
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
/// `_confirmed` function gets a good token from here. The functions without
/// a token (`dspi_save_preset`, `dspi_copy_preset`, `dspi_delete_preset`,
/// `dspi_save_params`, `dspi_factory_reset`, `dspi_save_output_config`,
/// `dspi_enter_bootloader`, `dspi_config_import`,
/// `dspi_config_import_selected`, `dspi_core_vreg_cal_commit`,
/// `dspi_core_vreg_manual_set`) are removed; there is no switch.
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

/// Save the live state to preset `slot`. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Copy preset `source` to `dest`. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Delete preset `slot`. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Save the live parameters to flash. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_params_confirmed(
    core: *mut FfiCore,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| c.save_params_confirmed(t))
}

/// Restore the factory defaults, writing flash, and read the device again. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
#[unsafe(no_mangle)]
pub extern "C" fn dspi_factory_reset_confirmed(
    core: *mut FfiCore,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| c.factory_reset_confirmed(t))
}

/// Store the output configuration in flash. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Reboot the device into its USB bootloader; `{"ok": true}` means only
/// that the request went out (no answer comes back). With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Apply a configuration document to the device. `options`: bit 0 the
/// volumes too, bit 1 the hardware configuration too, or with
/// `DSPI_IMPORT_BY_BLOCK` the `DSPI_IMPORT_*` block bits; `eq_channels` the
/// wire channels whose EQ is applied (bit k: channel k; all bits: every one).
/// With a token from `dspi_confirm_token` (required; WP P1TZ retired
/// `dspi_config_import` and `dspi_config_import_selected`). JSON reply; the
/// device has changed whatever the buffer. Without a good token: a refusal
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

/// Store the passed trial's voltage; `typed` as for the start. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Store VREG code `value` directly (0: the build's default); `typed` as
/// for the start. With a token from `dspi_confirm_token` (required; WP P1TZ
/// retired the form without one). JSON reply: `{"ok": true}`, or a
/// failure: `code` `refused` with the device's status in `error`,
/// `confirmation_required` without a good token (nothing sent), ...
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

/// Write a UF2 image to the bootloader drive at `drive_path` (`platform`: 0
/// RP2040, 1 RP2350) behind the firmware guard (`board_required_mv`, `typed`
/// as for `dspi_firmware_guard_check`), with a token minted while the board
/// waits in its bootloader (`firmware_flash`; `core` needs no device open,
/// a board already in BOOTSEL has none). Blocks for the copy. JSON reply;
/// nothing reaches the drive on a refusal (`confirmation_required`,
/// `firmware_guard`, `invalid`). The only drive write of this ABI: WP P1TZ
/// retired `dspi_flash_to_drive` and `dspi_flash_to_drive_guarded`, WP P1TZF
/// the unguarded `dspi_flash_to_drive_confirmed`. A board in BOOTSEL: pass
/// `board_required_mv` 0 (unknown) and the phrase the guard's verdict asks for.
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

// -- WP P1TZF -- the raw core-voltage operation: a SET stores it, so it takes a token
/// One core voltage operation (`op` 0 set, 1 trial, 2 end trial, 3 auto, 4
/// clear). `word` is what the user typed (null: nothing); raising above
/// 1.20 V needs `OVERVOLT`. A SET takes a token minted for `vreg_save`
/// (null for the others; one given is checked). JSON `{ok}` or a failure
/// (`confirmation_required` for a SET without a good token, nothing sent).
/// WP P1TZF retired `dspi_core_vreg_op`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_op_confirmed(
    core: *mut FfiCore,
    op: u8,
    value: u8,
    word: *const c_char,
    token: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    confirmed(core, token, buf, len, |c, t| {
        c.core_vreg_op_confirmed(op, value, optional_text(word, "word")?, t)
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
