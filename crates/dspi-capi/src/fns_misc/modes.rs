//! C functions: the mode manifest and the installer's lock (WP P1LK, PRO1 S4
//! and U7), over `dspi_core::modes` and `dspi_core::limits`. Replies as in the
//! parent module's docs: JSON. A write refused by the lock fails with `{"ok":
//! false, "code": "installer_lock", "error", "noticeKey":
//! "installer_lock.locked"}`; a wrong PIN with `noticeKey`
//! `installer_lock.wrong_pin`, the refusal window with `installer_lock.wait`.
//!
//! The PIN calls hash (0.1 to 1 s): call them off the UI thread.

use std::ffi::c_char;

use super::{arg, done_reply, free_text, json_reply, text};

/// The mode manifest, JSON `{ok, version, platforms, modes, lockedUser,
/// screens, operations}` (the shape of `assets/modes/modes.json`). No handle,
/// no I/O; 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mode_manifest(buf: *mut c_char, len: u32) -> u32 {
    free_text(buf, len, || json_reply(Ok(dspi_core::mode_manifest())))
}

/// The mode to show on a client of `platform` (`phone`, `desktop`, `web`)
/// when the user chose `chosen` ("" or null for none): JSON `{ok, mode,
/// reason, switchable, needsPin, openModes, screens, firstScreen,
/// operations}`, or a failure (`invalid` for a name the manifest lacks).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mode_resolve(
    core: *mut crate::FfiCore,
    platform: *const c_char,
    chosen: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let chosen = if chosen.is_null() {
            Ok("")
        } else {
            arg(chosen, "chosen")
        };
        json_reply(
            arg(platform, "platform")
                .and_then(|p| chosen.map(|ch| (p, ch)))
                .and_then(|(p, ch)| c.mode_resolve(p.to_owned(), ch.to_owned())),
        )
    })
}

/// Pin the open device's mode as the hub says (`""` or null clears it):
/// JSON `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mode_set_pinned(
    core: *mut crate::FfiCore,
    mode: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let mode = if mode.is_null() {
            Ok("")
        } else {
            arg(mode, "mode")
        };
        done_reply(mode.and_then(|m| c.mode_set_pinned(m.to_owned())))
    })
}

/// The open device's installer lock: JSON `{ok, serial, pinSet, unlocked,
/// locked, installerOpen, covers, allowedWrites, wrongTries, retryAfterMs,
/// recovered, note, contact, scopeNoticeKey, detail}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_lock_state(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.installer_lock_state())))
}

/// Unlock the open device on this console with the PIN or the recovery
/// code: the lock state's JSON, or a failure (`installer_lock`: `wrong_pin`,
/// `wait`). Slow: off the UI thread.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_unlock(
    core: *mut crate::FfiCore,
    pin: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(pin, "pin").and_then(|p| c.installer_unlock(p.to_owned())))
    })
}

/// Lock the open device again on this console: the lock state's JSON, or a
/// failure (`installer_lock` `no_pin`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_lock(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.installer_lock()))
}

/// Set the open device's installer PIN: JSON `{ok, recoveryCode, state}`
/// (the code shown this once), or a failure (`invalid`, `installer_lock`
/// `pin_required`, `io`). Slow: off the UI thread.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_pin_set(
    core: *mut crate::FfiCore,
    pin: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(pin, "pin").and_then(|p| c.installer_pin_set(p.to_owned())))
    })
}

/// Remove the open device's installer PIN: the lock state's JSON, or a
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_pin_clear(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.installer_pin_clear()))
}

/// Choose what the open device's lock covers: `pages_json` a JSON array of
/// the manifest's screen ids. The lock state's JSON, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installer_lock_cover(
    core: *mut crate::FfiCore,
    pages_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let pages = arg(pages_json, "pages_json").and_then(|t| {
            serde_json::from_str::<Vec<String>>(t).map_err(|e| {
                dspi_core::DspiError::InvalidArgument {
                    msg: format!("pages_json is not a JSON array of strings: {e}"),
                }
            })
        });
        json_reply(pages.and_then(|p| c.installer_lock_cover(p)))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests_wp_p1lk {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::{dspi_open_demo, dspi_write_param};

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    #[test]
    fn wp_p1lk_the_manifest_round_trips_through_c() {
        let r = reply(|b, l| dspi_mode_manifest(b, l));
        assert_eq!(r["ok"], true, "{r}");
        let mut fields = r.as_object().unwrap().clone();
        fields.remove("ok");
        let back: dspi_core::ModeManifest = serde_json::from_value(Value::Object(fields)).unwrap();
        assert_eq!(back, dspi_core::mode_manifest());
    }

    #[test]
    fn wp_p1lk_the_lock_through_c() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let web = CString::new("web").unwrap();
        let inst = CString::new("installer").unwrap();
        let c = reply(|b, l| dspi_mode_resolve(h, web.as_ptr(), inst.as_ptr(), b, l));
        assert_eq!(
            (c["mode"].clone(), c["needsPin"].clone()),
            ("installer".into(), false.into()),
            "{c}"
        );
        let st = reply(|b, l| dspi_installer_lock_state(h, b, l));
        assert_eq!(st["pinSet"], false, "{st}");

        let pin = CString::new("2468").unwrap();
        let set = reply(|b, l| dspi_installer_pin_set(h, pin.as_ptr(), b, l));
        assert_eq!(set["ok"], true, "{set}");
        assert_eq!(set["recoveryCode"].as_str().unwrap().len(), 23, "{set}");
        let cover = CString::new(r#"["front"]"#).unwrap();
        assert_eq!(
            reply(|b, l| dspi_installer_lock_cover(h, cover.as_ptr(), b, l))["ok"],
            true
        );
        let st = reply(|b, l| dspi_installer_lock(h, b, l));
        assert_eq!(st["locked"], true, "{st}");

        let gain =
            CString::new(r#"{"path":"out.gain","indices":[1],"value":{"float":1}}"#).unwrap();
        let r = reply(|b, l| dspi_write_param(h, gain.as_ptr(), b, l));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "installer_lock", "{r}");
        assert_eq!(r["noticeKey"], "installer_lock.locked", "{r}");
        let vol =
            CString::new(r#"{"path":"vol.master","indices":[],"value":{"float":-20}}"#).unwrap();
        assert_eq!(
            reply(|b, l| dspi_write_param(h, vol.as_ptr(), b, l))["ok"],
            true
        );

        let wrong = CString::new("0000").unwrap();
        let r = reply(|b, l| dspi_installer_unlock(h, wrong.as_ptr(), b, l));
        assert_eq!(r["noticeKey"], "installer_lock.wrong_pin", "{r}");
        let r = reply(|b, l| dspi_installer_unlock(h, pin.as_ptr(), b, l));
        assert_eq!(r["unlocked"], true, "{r}");
        assert_eq!(
            reply(|b, l| dspi_write_param(h, gain.as_ptr(), b, l))["ok"],
            true
        );

        let simple = CString::new("simple").unwrap();
        assert_eq!(
            reply(|b, l| dspi_mode_set_pinned(h, simple.as_ptr(), b, l))["ok"],
            true
        );
        let c = reply(|b, l| dspi_mode_resolve(h, web.as_ptr(), std::ptr::null(), b, l));
        assert_eq!(c["reason"], "pinned", "{c}");
        let st = reply(|b, l| dspi_installer_pin_clear(h, b, l));
        assert_eq!(st["pinSet"], false, "{st}");
        dspi_core_free(h);
    }
}
