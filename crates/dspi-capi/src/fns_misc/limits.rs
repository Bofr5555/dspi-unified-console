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

// -- WP P1EFIX --
/// Every limits field's range, the one table for a limits editor: JSON `{ok,
/// value:[{key, perOutput, unit, min, max, step}]}` (`dspi_core::limits_field_ranges`).
/// Takes no core handle and does not block. Returns the bytes needed including
/// the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_limits_field_ranges(buf: *mut c_char, len: u32) -> u32 {
    super::guard(0, || {
        super::copy_text(&json_reply(Ok(dspi_core::limits_field_ranges())), buf, len)
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1lm;

// -- WP P1EFIX --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1efix {
    use std::ptr;

    use serde_json::Value;

    use super::*;

    #[test]
    fn wp_p1efix_the_range_table_in_c() {
        let needed = dspi_limits_field_ranges(ptr::null_mut(), 0);
        assert!(needed > 0);
        let mut buf = vec![0u8; needed as usize];
        assert_eq!(
            dspi_limits_field_ranges(buf.as_mut_ptr().cast(), needed),
            needed
        );
        let text = std::ffi::CStr::from_bytes_until_nul(&buf)
            .unwrap()
            .to_str()
            .unwrap();
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["ok"], true);
        let rows = v["value"].as_array().unwrap();
        assert_eq!(rows.len(), dspi_core::limits_field_ranges().len());
        assert_eq!(rows[0]["key"], "maxGainDb");
        assert_eq!(rows[0]["perOutput"], true);
        assert_eq!(rows[0]["max"], 20.0);
    }
}
