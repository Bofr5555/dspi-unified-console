//! C functions: the board as an update target and the update check (WP
//! PGFWX, PLAN-PG sections 1.8 and 1.9), over `dspi_core`'s
//! `firmware_target` and `firmware_update_check`.
//!
//! Read the target with `dspi_firmware_target` before the bootloader entry,
//! keep its JSON unchanged and pass it to `dspi_firmware_update_check` for
//! the image the user picked. The check adds to `dspi_firmware_guard_check`'s
//! voltage rule: no image for a board without BOOTSEL (a DSPi-Zero, a board
//! not identified), the typed confirmation when this fork's firmware (or a
//! lineage that could not be read) would give way to an image that may be
//! upstream's, and the generation change. Both are reads: nothing is sent to
//! the device but the target's reads.

use std::ffi::c_char;

use dspi_core::{DspiError, FirmwareTarget};

use super::{c_bytes, c_str, free_text, json_reply};
use crate::{FfiCore, fns_device::text};

/// The open board as an update target, in JSON: `{ok, voltage: {requiredMv,
/// source}, boardId (null: not read), lineage ("unknown", "upstream" or
/// "bofr"), generation ("unknown", "legacy" or "paged")}`. Never a failure
/// for the device: what cannot be read is unknown, and an unknown board is
/// offered no image. Blocks for up to five control transfers: call it off
/// the UI thread. Returns the bytes needed including the NUL.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_firmware_target(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.firmware_target())))
}

/// The update check on `image_len` bytes of `image` for `target` (the JSON
/// `dspi_firmware_target` gave; null: nothing known, so no image), with what
/// the user `typed` (null: nothing), in JSON: `{ok, verdict: {allowed,
/// decision, reason, message, confirmationPhrase, imageMv, requiredMv}, rule
/// ("none", "noBootsel" or "lineageLoss"), imageLineage, imageGeneration,
/// generationChange ("" when none)}`. Flash only when `verdict.allowed`. A
/// target that is not that JSON is `invalid`. Pure apart from parsing the
/// image.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_firmware_update_check(
    image: *const u8,
    image_len: u32,
    target: *const c_char,
    typed: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    free_text(buf, len, || {
        let target = parse_target(target);
        json_reply(target.map(|t| {
            dspi_core::firmware_update_check(
                c_bytes(image, image_len),
                &t,
                c_str(typed).unwrap_or_default(),
            )
        }))
    })
}

/// The target argument: null is the default (nothing known); text that is
/// not UTF-8 or not the target's JSON is refused.
fn parse_target(p: *const c_char) -> dspi_core::Result<FirmwareTarget> {
    if p.is_null() {
        return Ok(FirmwareTarget::default());
    }
    let text = c_str(p).ok_or_else(|| DspiError::InvalidArgument {
        msg: "the target is not UTF-8".into(),
    })?;
    serde_json::from_str(text).map_err(|e| DspiError::InvalidArgument {
        msg: format!("the target is not a firmware target: {e}"),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_pgfwx {
    use std::ffi::CString;
    use std::ptr::{null, null_mut};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    fn stock_image() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2");
        std::fs::read(path).unwrap()
    }

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let needed = f(null_mut(), 0);
        assert!(needed > 0);
        let mut buf = vec![0u8; needed as usize];
        assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
        serde_json::from_slice(&buf[..needed as usize - 1]).unwrap()
    }

    fn check(image: &[u8], target: *const c_char, typed: *const c_char) -> Value {
        reply(|b, l| {
            dspi_firmware_update_check(image.as_ptr(), image.len() as u32, target, typed, b, l)
        })
    }

    #[test]
    fn wp_pgfwx_the_target_and_the_update_check_over_the_c_abi() {
        // No device: nothing known; the check offers that target no image.
        let h = dspi_core_new();
        let none = reply(|b, l| dspi_firmware_target(h, b, l));
        assert_eq!(none["ok"], true, "{none}");
        assert_eq!(none["boardId"], Value::Null, "{none}");
        assert_eq!(none["voltage"]["source"], "noDevice", "{none}");
        let image = stock_image();
        let json = CString::new(serde_json::to_string(&none).unwrap()).unwrap();
        let refused = check(&image, json.as_ptr(), null());
        assert_eq!(refused["rule"], "noBootsel", "{refused}");
        assert_eq!(refused["verdict"]["allowed"], false, "{refused}");
        // A null target is the same: nothing known.
        assert_eq!(check(&image, null(), null())["rule"], "noBootsel");
        // The demo board is a Pico.
        assert!(dspi_open_demo(h));
        let demo = reply(|b, l| dspi_firmware_target(h, b, l));
        assert_eq!(demo["boardId"], 0, "{demo}");
        // A Pico on our firmware at the stock voltage: the lineage rule asks.
        let ours = CString::new(
            r#"{"voltage":{"requiredMv":1150,"source":"statusBlock"},"boardId":0,
                "lineage":"bofr","generation":"legacy"}"#,
        )
        .unwrap();
        let asks = check(&image, ours.as_ptr(), null());
        assert_eq!(asks["rule"], "lineageLoss", "{asks}");
        assert_eq!(asks["verdict"]["decision"], "needsConfirmation", "{asks}");
        assert_eq!(asks["imageLineage"], "upstream", "{asks}");
        let phrase = CString::new(dspi_core::LINEAGE_PHRASE).unwrap();
        let typed = check(&image, ours.as_ptr(), phrase.as_ptr());
        assert_eq!(typed["verdict"]["allowed"], true, "{typed}");
        // Not a target: invalid.
        let junk = CString::new("{\"boardId\": \"x\"}").unwrap();
        let bad = check(&image, junk.as_ptr(), null());
        assert_eq!(
            (bad["ok"].clone(), bad["code"].clone()),
            (Value::Bool(false), "invalid".into())
        );
        dspi_core_free(h);
    }
}
