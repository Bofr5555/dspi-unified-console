//! C functions: the speaker layout (WP ROLE0, MPLAN section 2.2), over
//! `dspi_core`'s `speaker_layout` module.
//!
//! The layout is JSON (`SpeakerLayout`, camelCase: `{serial, outputs:
//! [{output, role ("full", "sub", "low", "mid", "high"), speaker, group}],
//! confirmedByUser, updatedMs, notes}`). Every reply is JSON: the layout,
//! `{ok: true, value: null}` when none is stored, `{ok}`, or a failure.
//!
//! `dspi_speaker_layout_set` is for the user's own screens only (Getting
//! Started, Settings › Speakers, the Measure tables). -- WP ROLES -- An agent
//! sets roles only from the user's words, through the MCP server
//! (`dspi_set_speaker_roles`), never through this call.

use std::ffi::c_char;

use dspi_core::{DspiError, SpeakerLayout};

use super::{arg, done_reply, json_reply, text};
use crate::FfiCore;

/// Keep speaker layouts in files under `dir`, the app's own data folder, one
/// subfolder per serial ("" forgets the folder). `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_speaker_layout_set_dir(
    core: *mut FfiCore,
    dir: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(dir, "dir").and_then(|d| c.speaker_layout_set_dir(d.to_owned())))
    })
}

/// The open device's stored layout, `{ok: true, value: null}` when none was
/// set, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_speaker_layout(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.speaker_layout()))
}

/// What the device's filters and routing suggest (never confirmed, never
/// saved): the layout, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_speaker_layout_propose(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.speaker_layout_propose()))
}

/// Keep `layout_json` as the open device's layout (the user's screens only):
/// the layout kept, or a failure (`invalid` naming why).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_speaker_layout_set(
    core: *mut FfiCore,
    layout_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let r = arg(layout_json, "layout").and_then(|t| {
            let layout: SpeakerLayout =
                serde_json::from_str(t).map_err(|e| DspiError::InvalidArgument {
                    msg: format!("layout: {e}"),
                })?;
            c.speaker_layout_set(layout)
        });
        json_reply(r)
    })
}

#[cfg(test)]
mod tests_wp_role0 {
    use std::ffi::CString;

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    fn read(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 1 << 16];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len());
        let s = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        serde_json::from_str(s.to_str().unwrap()).unwrap()
    }

    #[test]
    fn wp_role0_the_c_calls_carry_the_speaker_layout() {
        let h = dspi_core_new();
        // No device: refused.
        let r = read(|b, l| dspi_speaker_layout(h, b, l));
        assert_eq!(r["code"], "disconnected", "{r}");
        assert!(dspi_open_demo(h));
        let r = read(|b, l| dspi_speaker_layout(h, b, l));
        assert_eq!(
            (r["ok"].clone(), r["value"].clone()),
            (Value::Bool(true), Value::Null)
        );
        let mut p = read(|b, l| dspi_speaker_layout_propose(h, b, l));
        assert_eq!(p["ok"], true, "{p}");
        assert_eq!(p["confirmedByUser"], false);
        assert!(
            p["outputs"].as_array().is_some_and(|o| !o.is_empty()),
            "{p}"
        );
        // The user confirms it: kept, stamped, read back.
        p["confirmedByUser"] = Value::Bool(true);
        p["outputs"][0]["role"] = Value::from("high");
        let t = CString::new(p.to_string()).unwrap();
        let k = read(|b, l| dspi_speaker_layout_set(h, t.as_ptr(), b, l));
        assert_eq!(k["ok"], true, "{k}");
        assert!(k["updatedMs"].as_i64().is_some_and(|ms| ms > 0));
        let r = read(|b, l| dspi_speaker_layout(h, b, l));
        assert_eq!(r["outputs"][0]["role"], "high", "{r}");
        assert_eq!(r["confirmedByUser"], true);
        // Not a layout, an output the device lacks: refused in words.
        let bad = CString::new("{\"outputs\": 3}").unwrap();
        let r = read(|b, l| dspi_speaker_layout_set(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let past = CString::new("{\"outputs\": [{\"output\": 99, \"role\": \"low\"}]}").unwrap();
        let r = read(|b, l| dspi_speaker_layout_set(h, past.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = read(|b, l| dspi_speaker_layout_set(h, std::ptr::null(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        dspi_core_free(h);
    }
}
