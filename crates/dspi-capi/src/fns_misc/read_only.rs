//! C functions: the client read-only mode (WP ROMODE), over
//! `dspi_core::read_only`.
//!
//! While it is on, every write of this handle is refused before anything is
//! sent: a `bool` setter returns false, a status setter 0xFF, a JSON call
//! `{"ok": false, "code": "read_only", "noticeKey": "read_only.client"}`.
//! Reads, notifications, the meters and the analyser go on. A client-side
//! diagnostic, not a security boundary: the client turns it on and off, and
//! nothing else is bound by it.

use dspi_core::DspiCore;
use serde_json::Value;

use crate::{FfiCore, with_core};

/// Turn the client read-only mode on or off for this handle; it stays for
/// every device the handle opens until turned off. Waits for a call in flight.
/// Nothing for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_client_read_only(core: *mut FfiCore, on: bool) {
    with_core(core, (), |h| h.core.set_client_read_only(on));
}

/// Whether the client read-only mode is on; false for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_client_read_only(core: *mut FfiCore) -> bool {
    with_core(core, false, |h| h.core.client_read_only())
}

/// How many writes the client read-only mode has refused on this handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_client_read_only_refusals(core: *mut FfiCore) -> u32 {
    with_core(core, 0, |h| h.core.client_read_only_refusals())
}

/// A JSON reply of `core`'s with its failure's `noticeKey` as the core
/// answers it now: `read_only.client` for a `read_only` while the mode is on.
/// Anything else is returned as it is.
pub(crate) fn refine_reply(core: &DspiCore, reply: String) -> String {
    if !reply.contains(r#""noticeKey":"read_only""#) {
        return reply;
    }
    let Some(key) = core.client_notice_key("read_only") else {
        return reply;
    };
    match serde_json::from_str::<Value>(&reply) {
        Ok(Value::Object(mut fields)) if fields.get("ok") == Some(&Value::Bool(false)) => {
            fields.insert("noticeKey".to_owned(), Value::String(key.to_owned()));
            Value::Object(fields).to_string()
        }
        _ => reply,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_romode {
    use std::ffi::{CString, c_char};
    use std::ptr;

    use serde_json::Value;

    use super::super::notice_text::dspi_notice_for;
    use super::super::{dspi_open_demo, dspi_write_param};
    use super::*;
    use crate::fns_device::{
        dspi_core_free, dspi_core_new, dspi_fetch_all, dspi_get_state, dspi_set_master_volume,
    };

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let needed = f(ptr::null_mut(), 0);
        assert!(needed > 0);
        let mut buf = vec![0u8; needed as usize];
        assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
        assert_eq!(buf.pop(), Some(0));
        serde_json::from_slice(&buf).unwrap()
    }

    fn write(h: *mut FfiCore, request: &str) -> Value {
        let req = CString::new(request).unwrap();
        reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l))
    }

    #[test]
    fn wp_romode_through_c_writes_are_refused_with_the_client_row_and_reads_go_on() {
        assert!(!dspi_client_read_only(ptr::null_mut()));
        dspi_set_client_read_only(ptr::null_mut(), true); // nothing, no crash
        let h = dspi_core_new();
        assert!(!dspi_client_read_only(h));
        dspi_set_client_read_only(h, true);
        assert!(dspi_client_read_only(h));
        assert!(dspi_open_demo(h));
        // SAFETY: the handle is live; the record stays valid until the next call.
        let before = unsafe { (*dspi_get_state(h)).master_volume_db };

        assert!(!dspi_set_master_volume(h, -7.0));
        let r = write(h, r#"{"path":"vol.master","value":{"float":-7}}"#);
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "read_only", "{r}");
        assert_eq!(r["noticeKey"], "read_only.client", "{r}");
        let key = CString::new(r["noticeKey"].as_str().unwrap()).unwrap();
        let n = reply(|b, l| dspi_notice_for(key.as_ptr(), ptr::null(), b, l));
        assert_eq!(n["title"], "Read-only mode", "{n}");
        assert!(dspi_client_read_only_refusals(h) >= 2);

        // Reads go on.
        assert!(dspi_fetch_all(h));
        // SAFETY: as above.
        assert_eq!(unsafe { (*dspi_get_state(h)).master_volume_db }, before);

        // Off again: writes go out, and a refusal of the firmware's own
        // read-only mode keeps its row.
        dspi_set_client_read_only(h, false);
        assert!(dspi_set_master_volume(h, -7.0));
        // SAFETY: as above.
        assert_eq!(unsafe { (*dspi_get_state(h)).master_volume_db }, -7.0);
        dspi_core_free(h);
    }

    #[test]
    fn wp_romode_only_a_read_only_failure_is_refined() {
        let core = DspiCore::new();
        let failure = r#"{"code":"read_only","error":"x","noticeKey":"read_only","ok":false}"#;
        assert_eq!(refine_reply(&core, failure.to_owned()), failure);
        core.set_client_read_only(true);
        let r: Value = serde_json::from_str(&refine_reply(&core, failure.to_owned())).unwrap();
        assert_eq!(r["noticeKey"], "read_only.client");
        assert_eq!(r["code"], "read_only");
        let other = r#"{"code":"locked","error":"x","noticeKey":"locked","ok":false}"#;
        assert_eq!(refine_reply(&core, other.to_owned()), other);
        let ok = r#"{"ok":true,"value":"noticeKey"}"#;
        assert_eq!(refine_reply(&core, ok.to_owned()), ok);
    }
}
