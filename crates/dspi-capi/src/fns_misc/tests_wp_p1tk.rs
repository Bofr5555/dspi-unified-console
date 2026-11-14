//! WP P1TK tests: confirmation tokens through C. The refusals are the core's,
//! as the web binding's are: code and notice key `confirmation_required`.

use std::ffi::{CString, c_char};
use std::ptr;

use serde_json::Value;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};
use crate::fns_misc::{dspi_config_export, dspi_open_demo};

/// A text call's reply as JSON.
fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 16384];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn mint(h: *mut FfiCore, op: &str) -> CString {
    let op = CString::new(op).unwrap();
    let r = reply(|b, l| dspi_confirm_token(h, op.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    CString::new(r["value"].as_str().unwrap()).unwrap()
}

/// A refusal for want of a confirmation, with the reason's words.
fn assert_refused(r: &Value, op: &str, why: &str) {
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "confirmation_required", "{r}");
    assert_eq!(r["noticeKey"], "confirmation_required", "{r}");
    let text = r["error"].as_str().unwrap();
    assert!(text.starts_with(op) && text.contains(why), "{r}");
}

#[test]
fn wp_p1tk_tokens_through_c() {
    // -- WP P1TZ -- tokens are required from the start: no switch to call.
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));

    // No token, another operation's, a good one, the same again.
    let save = |t: *const c_char| reply(|b, l| dspi_save_preset_confirmed(h, 1, t, b, l));
    assert_refused(&save(ptr::null()), "save_preset", "no confirmation token");
    let reset = mint(h, "factory_reset");
    assert_refused(
        &save(reset.as_ptr()),
        "save_preset",
        "for another operation",
    );
    let t = mint(h, "save_preset");
    assert_eq!(save(t.as_ptr())["ok"], true);
    assert_refused(&save(t.as_ptr()), "save_preset", "already used");

    // Every `_confirmed` function takes its own token.
    let ok = |r: Value| assert_eq!(r["ok"], true, "{r}");
    let t = mint(h, "copy_preset");
    ok(reply(|b, l| {
        dspi_copy_preset_confirmed(h, 1, 2, t.as_ptr(), b, l)
    }));
    let t = mint(h, "delete_preset");
    ok(reply(|b, l| {
        dspi_delete_preset_confirmed(h, 2, t.as_ptr(), b, l)
    }));
    let t = mint(h, "save_params");
    ok(reply(|b, l| {
        dspi_save_params_confirmed(h, t.as_ptr(), b, l)
    }));
    let t = mint(h, "save_output_config");
    ok(reply(|b, l| {
        dspi_save_output_config_confirmed(h, t.as_ptr(), b, l)
    }));
    ok(reply(|b, l| {
        dspi_factory_reset_confirmed(h, reset.as_ptr(), b, l)
    }));

    // The import: refused with nothing written, then let through.
    let name = CString::new("P1TK").unwrap();
    let version = CString::new("test").unwrap();
    let mut doc = vec![0u8; 1 << 20];
    let n = dspi_config_export(
        h,
        name.as_ptr(),
        version.as_ptr(),
        0,
        doc.as_mut_ptr().cast(),
        doc.len() as u32,
    );
    doc.truncate(n as usize - 1);
    let doc = CString::new(doc).unwrap();
    let import = |t: *const c_char| {
        reply(|b, l| dspi_config_import_confirmed(h, doc.as_ptr(), 0, u32::MAX, t, b, l))
    };
    let r = import(ptr::null());
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["noticeKey"], "confirmation_required", "{r}");
    let t = mint(h, "config_import");
    ok(import(t.as_ptr()));

    // The core voltage on a device without it: let through, then the device's
    // answer; refused first without a token.
    let r = reply(|b, l| dspi_core_vreg_manual_set_confirmed(h, 0, ptr::null(), ptr::null(), b, l));
    assert_refused(&r, "vreg_save", "no confirmation token");
    let t = mint(h, "vreg_save");
    let r = reply(|b, l| dspi_core_vreg_cal_commit_confirmed(h, ptr::null(), t.as_ptr(), b, l));
    assert_ne!(r["code"], "confirmation_required", "{r}");

    // A token does not cross a reconnect.
    let stale = mint(h, "enter_bootloader");
    crate::fns_device::dspi_disconnect(h);
    assert!(dspi_open_demo(h));
    let r = reply(|b, l| dspi_enter_bootloader_confirmed(h, stale.as_ptr(), b, l));
    assert_refused(&r, "enter_bootloader", "connection changed");

    // The firmware install: refused without a token, nothing reaches a drive.
    crate::fns_device::dspi_disconnect(h);
    let path = CString::new("/nonexistent/p1tk-drive").unwrap();
    let flash = |t: *const c_char| {
        reply(|b, l| {
            dspi_flash_confirmed(
                h,
                [0u8; 4].as_ptr(),
                4,
                path.as_ptr(),
                1,
                0,
                ptr::null(),
                t,
                b,
                l,
            )
        })
    };
    assert_refused(
        &flash(ptr::null()),
        "firmware_flash",
        "no confirmation token",
    );
    let t = mint(h, "firmware_flash");
    let r = flash(t.as_ptr());
    assert_eq!(
        r["code"], "invalid",
        "past the gate, the image is refused: {r}"
    );

    // An operation the core does not have.
    let op = CString::new("format").unwrap();
    assert_eq!(
        reply(|b, l| dspi_confirm_token(h, op.as_ptr(), b, l))["code"],
        "invalid"
    );
    dspi_core_free(h);
}

/// WP P1TZ (OD-P1f): a new core refuses every gated C function without a
/// token, and every gated `dspi_write_param` path, with nothing sent; the
/// functions without a token and the switch are gone from the header.
#[test]
fn wp_p1tz_a_new_core_requires_tokens_through_c() {
    use crate::fns_misc::dspi_write_param;
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let none = ptr::null();
    for (op, r) in [
        (
            "save_preset",
            reply(|b, l| dspi_save_preset_confirmed(h, 1, none, b, l)),
        ),
        (
            "copy_preset",
            reply(|b, l| dspi_copy_preset_confirmed(h, 1, 2, none, b, l)),
        ),
        (
            "delete_preset",
            reply(|b, l| dspi_delete_preset_confirmed(h, 1, none, b, l)),
        ),
        (
            "save_params",
            reply(|b, l| dspi_save_params_confirmed(h, none, b, l)),
        ),
        (
            "factory_reset",
            reply(|b, l| dspi_factory_reset_confirmed(h, none, b, l)),
        ),
        (
            "save_output_config",
            reply(|b, l| dspi_save_output_config_confirmed(h, none, b, l)),
        ),
        (
            "enter_bootloader",
            reply(|b, l| dspi_enter_bootloader_confirmed(h, none, b, l)),
        ),
    ] {
        assert_refused(&r, op, "no confirmation token");
    }
    for req in [
        r#"{"path":"preset.save","indices":[1],"value":"trigger"}"#,
        r#"{"path":"preset.delete","indices":[1],"value":"trigger"}"#,
        r#"{"path":"dev.save","value":"trigger"}"#,
        r#"{"path":"dev.save.io","value":"trigger"}"#,
    ] {
        let req = CString::new(req).unwrap();
        let r = reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l));
        assert_eq!(r["code"], "confirmation_required", "{r}");
    }
    // With a token the same save goes through.
    let t = mint(h, "save_preset");
    let r = reply(|b, l| dspi_save_preset_confirmed(h, 1, t.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    dspi_core_free(h);

    // The header the build writes declares none of the retired functions.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
    for gone in [
        "dspi_save_preset",
        "dspi_copy_preset",
        "dspi_delete_preset",
        "dspi_save_params",
        "dspi_factory_reset",
        "dspi_save_output_config",
        "dspi_enter_bootloader",
        "dspi_config_import",
        "dspi_config_import_selected",
        "dspi_core_vreg_cal_commit",
        "dspi_core_vreg_manual_set",
        "dspi_require_confirmation",
        "dspi_confirmation_required",
        "dspi_flash_to_drive",
        "dspi_flash_to_drive_guarded",
    ] {
        assert!(
            !header.contains(&format!(" {gone}(")),
            "{gone} is still declared"
        );
    }
}

/// WP P1TZ: the drive write without the guard is a core's too, gated like the
/// rest, with no device open (a board in BOOTSEL has no DSPi connection).
#[test]
fn wp_p1tz_the_drive_write_takes_a_firmware_token() {
    let h = dspi_core_new();
    let image = [0u8; 512];
    let path = CString::new("/nonexistent/p1tz-drive").unwrap();
    let write = |t: *const c_char| {
        reply(|b, l| {
            dspi_flash_to_drive_confirmed(h, image.as_ptr(), 512, path.as_ptr(), 1, t, b, l)
        })
    };
    assert_refused(
        &write(ptr::null()),
        "firmware_flash",
        "no confirmation token",
    );
    let other = mint(h, "save_preset");
    assert_refused(
        &write(other.as_ptr()),
        "firmware_flash",
        "for another operation",
    );
    let t = mint(h, "firmware_flash");
    let r = write(t.as_ptr());
    assert_eq!(r["ok"], false, "{r}");
    assert_ne!(
        r["code"], "confirmation_required",
        "let through, then the image: {r}"
    );
    assert_refused(&write(t.as_ptr()), "firmware_flash", "already used");
    dspi_core_free(h);
}
