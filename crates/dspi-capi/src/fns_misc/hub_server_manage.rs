//! C functions: the hub's device and name writes (WP P2CC3), over
//! `dspi_core::hub_server::manage` (`docs/integrator/hub-api.md` sections 3,
//! 4 and 15): an Admin's inventory edits and forget, renaming the hub, and
//! Identify for any device on the hub. Replies as in the parent module's
//! docs: JSON, the records' fields camelCase beside `"ok": true`.
//!
//! Without the family's cap (or off a hub) a call fails with `code`
//! `unsupported` and sends nothing; the hub's refusals as in
//! `hub_server.rs`; in the client read-only mode each is `read_only`, nothing
//! sent. Every call is a round trip: off the UI thread.

use std::ffi::c_char;

use dspi_core::{DspiError, Result};

use super::{arg, json_reply, optional_text, text};

/// The `tone_output` of [`dspi_hub_identify_serial`] for no tone.
const IDENTIFY_NO_TONE: u8 = 0xFF;

/// `json` (null: `None`) as a JSON list of strings.
fn tags_arg(json: *const c_char) -> Result<Option<Vec<String>>> {
    let Some(text) = optional_text(json, "tags_json")? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| DspiError::InvalidArgument {
            msg: format!("tags_json is a JSON list of strings: {e}"),
        })
}

/// Set device `serial`'s (null or "": the open device's) `location`, `note`
/// and tags (`tags_json`, a JSON list of strings) in the hub's inventory
/// (Admin). Each null keeps its field; "" (or `[]`) clears it. The record as
/// the hub now has it: JSON `{ok, serial, handle, online, name, ..., location,
/// note, tags, ...}` as `dspi_hub_inventory`'s, or a failure (`invalid` past a
/// bound, `link` with `dlink.not_found`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_inventory_set(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    location: *const c_char,
    note: *const c_char,
    tags_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let args = || -> Result<_> {
            Ok((
                optional_text(serial, "serial")?.unwrap_or_default(),
                optional_text(location, "location")?,
                optional_text(note, "note")?,
                tags_arg(tags_json)?,
            ))
        };
        json_reply(args().and_then(|(s, l, n, t)| c.hub_inventory_set(s, l, n, t)))
    })
}

/// Forget offline device `serial` on the hub (Admin): its record, its history
/// and, with `with_data`, its snapshots on the hub. JSON `{ok, serial,
/// forgot: ["record", "events"?, "snapshots"?]}`, or a failure (`link` with
/// `dlink.conflict` while it is online, `dlink.not_found`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_inventory_forget(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    with_data: bool,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(serial, "serial").and_then(|s| c.hub_inventory_forget(s.to_owned(), with_data)),
        )
    })
}

/// Rename the hub to `name` (Admin): JSON `{ok, value: "<the name the hub
/// now uses>"}`, or a failure (`invalid` for an empty or too long name).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_rename(
    core: *mut crate::FfiCore,
    name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(name, "name").and_then(|n| c.hub_rename(n.to_owned())))
    })
}

/// Identify device `serial` on the hub (null or "": the open device) for
/// `seconds` (0 stops), with the quiet tone on `tone_output` (0xFF: none;
/// pass 0xFF unless the user asked for sound). Any device on the hub. JSON
/// as `dspi_hub_identify`'s: `{ok, remainingS, lastToneOutput, chip, chipRev,
/// chipId, board, hwRev}`, `{ok, value: null}` when the state could not be
/// read after the write, or a failure (`unsupported` off a hub; `link` with
/// `dlink.rate_limited`, `dlink.offline`, `dlink.not_found`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_identify_serial(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    seconds: u8,
    tone_output: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let tone = (tone_output != IDENTIFY_NO_TONE).then_some(tone_output);
    text(core, buf, len, |c| {
        json_reply(
            optional_text(serial, "serial")
                .and_then(|s| c.hub_identify_serial(s.unwrap_or_default(), seconds, tone)),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p2cc3 {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    /// The same words as the core's own calls, through C.
    #[test]
    fn wp_p2cc3_the_hub_writes_through_c() {
        let h = dspi_core_new();
        let serial = CString::new("S1").unwrap();
        let hall = CString::new("Hall").unwrap();
        let tags = CString::new(r#"["main"]"#).unwrap();
        let name = CString::new("Studio").unwrap();
        for r in [
            reply(|b, l| {
                dspi_hub_inventory_set(
                    h,
                    serial.as_ptr(),
                    hall.as_ptr(),
                    std::ptr::null(),
                    tags.as_ptr(),
                    b,
                    l,
                )
            }),
            reply(|b, l| dspi_hub_inventory_forget(h, serial.as_ptr(), true, b, l)),
            reply(|b, l| dspi_hub_rename(h, name.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_identify_serial(h, serial.as_ptr(), 5, 0xFF, b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["code"], "unsupported", "{r}");
            assert_eq!(r["noticeKey"], "unsupported", "{r}");
        }
        // Arguments that do not read are the caller's error, nothing sent.
        let bad = CString::new("not a list").unwrap();
        let r = reply(|b, l| {
            dspi_hub_inventory_set(
                h,
                serial.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                bad.as_ptr(),
                b,
                l,
            )
        });
        assert_eq!(r["code"], "invalid", "{r}");
        let r = reply(|b, l| dspi_hub_inventory_forget(h, std::ptr::null(), false, b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let core_says = dspi_core::DspiCore::new()
            .hub_inventory_forget(String::new(), false)
            .unwrap_err();
        let empty = CString::new("").unwrap();
        let r = reply(|b, l| dspi_hub_inventory_forget(h, empty.as_ptr(), false, b, l));
        assert_eq!(r["error"], core_says.to_string(), "{r}");
        dspi_core_free(h);
    }
}
