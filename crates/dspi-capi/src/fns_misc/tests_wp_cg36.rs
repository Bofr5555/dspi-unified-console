//! WP CG36 tests: the Core 1 question and the write refusals through C.

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 1024];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn write(h: *mut FfiCore, request: &str) -> serde_json::Value {
    let req = CString::new(request).unwrap();
    reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l))
}

#[test]
fn wp_cg36_core1_question_crossover_types_and_the_delay_line_through_c() {
    assert_eq!(
        dspi_core1_conflict_reason(std::ptr::null_mut(), 0, std::ptr::null_mut(), 0),
        0
    );
    let h = dspi_core_new();
    let r = reply(|b, l| dspi_core1_conflict_reason(h, 0, b, l));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "disconnected");

    assert!(dspi_open_demo(h));
    let r = reply(|b, l| dspi_core1_conflict_reason(h, 0, b, l));
    assert_eq!(
        r,
        serde_json::json!({ "ok": true, "conflict": false, "reason": null })
    );
    let r = reply(|b, l| dspi_core1_conflict_reason(h, 200, b, l));
    assert_eq!(r["ok"], false, "{r}");

    // A crossover type on an output channel's crossover band, read back.
    let r = write(
        h,
        r#"{"path":"eq.type","indices":[9,20],"value":{"choice":44}}"#,
    );
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["value"]["choice"], 44);
    let r = write(
        h,
        r#"{"path":"eq.type","indices":[9,3],"value":{"choice":44}}"#,
    );
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("PEQ band"), "{r}");

    // The delay line: the demo is an RP2350, 42 ms.
    let r = write(
        h,
        r#"{"path":"out.delay","indices":[0],"value":{"float":50}}"#,
    );
    assert_eq!(r["ok"], false, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("at most 42 ms"),
        "{r}"
    );
    assert!(!crate::fns_device::dspi_set_output_delay(h, 0, 50.0));
    dspi_core_free(h);
}
