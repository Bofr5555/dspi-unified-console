//! WP P1LM tests: limits profiles through C. A refusal is the core's: code
//! and notice key `limit_exceeded`, as the web binding's.

use std::ffi::{CString, c_char};

use serde_json::{Value, json};

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};
use crate::fns_misc::{dspi_open_demo, dspi_write_param};

/// A text call's reply as JSON, or the text itself when it is not JSON.
fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 65536];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end])
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&buf[..end]).into()))
}

#[test]
fn wp_p1lm_limits_through_c() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let st = reply(|b, l| dspi_limits_status(h, b, l));
    assert_eq!(st["ok"], true, "{st}");
    assert_eq!(st["active"], false);
    assert_eq!(st["state"], "none");

    // Output 1 above the limit to begin with: applying clamps and reports.
    let req = CString::new(r#"{"path":"out.gain","indices":[1],"value":{"float":8}}"#).unwrap();
    assert_eq!(
        reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l))["ok"],
        true
    );
    let doc = json!({
        "schemaVersion": 1,
        "meta": {"kind": "limits"},
        "limits": {"outputs": [{"output": 1, "maxGainDb": 2.0}]}
    })
    .to_string();
    let doc = CString::new(doc).unwrap();
    let r = reply(|b, l| dspi_limits_apply_profile(h, doc.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["held"][0], "out.gain[1] 8 → 2 dB", "{r}");

    // A write past the limit: refused, with its notice key.
    let req = CString::new(r#"{"path":"out.gain","indices":[1],"value":{"float":5}}"#).unwrap();
    let r = reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "limit_exceeded", "{r}");
    assert_eq!(r["noticeKey"], "limit_exceeded", "{r}");

    let st = reply(|b, l| dspi_limits_status(h, b, l));
    assert_eq!(st["active"], true, "{st}");
    assert_eq!(st["lastHeld"][0], "out.gain[1] 8 → 2 dB", "{st}");
    // The document's text itself (JSON, so it reads as such).
    let text = reply(|b, l| dspi_limits_profile_document(h, b, l));
    assert_eq!(text["meta"]["kind"], "limits", "{text}");
    assert!(text.get("ok").is_none());

    // Not a profile: invalid.
    let bad = CString::new("{}").unwrap();
    let r = reply(|b, l| dspi_limits_apply_profile(h, bad.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid", "{r}");

    assert_eq!(reply(|b, l| dspi_limits_clear(h, b, l))["ok"], true);
    assert_eq!(
        reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l))["ok"],
        true
    );
    dspi_core_free(h);
}
