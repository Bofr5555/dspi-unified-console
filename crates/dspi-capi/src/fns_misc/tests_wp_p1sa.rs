//! WP P1SA tests: the guarded apply through the C ABI on the demo device.

use std::ffi::{CStr, CString};

use super::guarded::*;
use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> String {
    let mut buf = vec![0 as c_char; 256 * 1024];
    let needed = f(buf.as_mut_ptr(), buf.len() as u32);
    assert!(
        needed > 0 && needed as usize <= buf.len(),
        "needed {needed}"
    );
    // SAFETY: the call wrote a NUL-terminated reply into `buf`.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    serde_json::from_str(&reply(f)).unwrap()
}

#[test]
fn wp_p1sa_guarded_apply_through_the_c_abi() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let core = with_core(h, None, |f| Some(Arc::clone(&f.core))).unwrap();
    assert!(!dspi_apply_guard_default(h));
    assert!(!dspi_apply_guard_default(std::ptr::null_mut()));
    let idle = read_json(|b, l| dspi_apply_status(h, b, l));
    assert_eq!((idle["ok"].clone(), idle["phase"].clone()), (json!(true), json!("idle")));

    // A document with another preamp; the device left as it was.
    let back = core
        .snapshot_capture_document("back".into(), String::new())
        .unwrap();
    core.set_input_preamp(0, -6.0).unwrap();
    let doc = core
        .config_export("Kväll".into(), "wp-p1sa".into(), 0)
        .unwrap();
    core.snapshot_recall_document(back).unwrap();
    let doc = CString::new(doc).unwrap();
    let all = dspi_core::consts::IMPORT_BY_BLOCK | dspi_core::consts::IMPORT_ALL_BLOCKS;

    let s = read_json(|b, l| dspi_apply_guarded_import(h, doc.as_ptr(), all, u32::MAX, 0, b, l));
    assert_eq!(s["ok"], true, "{s}");
    assert_eq!(s["phase"], "waiting");
    assert_eq!(s["label"], "Kväll");
    assert_eq!(s["timeoutMs"], 30_000);
    assert_eq!(s["hub"], false);
    assert!(!s["changes"].as_array().unwrap().is_empty(), "{s}");
    assert_eq!(core.get_state().input_preamp_db[0], -6.0);
    let k = read_json(|b, l| dspi_apply_confirm(h, b, l));
    assert_eq!((k["phase"].clone(), k["reason"].clone()), (json!("kept"), json!("confirmed")));
    let again = read_json(|b, l| dspi_apply_confirm(h, b, l));
    assert_eq!((again["ok"].clone(), again["code"].clone()), (json!(false), json!("invalid")));

    // A flow, then Revert.
    let label = CString::new("Flow").unwrap();
    let s = read_json(|b, l| dspi_apply_guard_begin(h, label.as_ptr(), 10, b, l));
    assert_eq!((s["phase"].clone(), s["timeoutMs"].clone()), (json!("waiting"), json!(10_000)));
    core.set_input_preamp(0, -3.0).unwrap();
    let r = read_json(|b, l| dspi_apply_revert(h, b, l));
    assert_eq!((r["phase"].clone(), r["reason"].clone()), (json!("reverted"), json!("reverted")));
    assert_eq!(core.get_state().input_preamp_db[0], -6.0);
    let p = read_json(|b, l| dspi_apply_poll(h, b, l));
    assert_eq!(p, r);

    // Not a snapshot: refused, nothing waits.
    let r = read_json(|b, l| dspi_apply_guarded_snapshot_document(h, doc.as_ptr(), 0, b, l));
    assert_eq!((r["ok"].clone(), r["code"].clone()), (json!(false), json!("invalid")), "{r}");
    let id = CString::new("nope").unwrap();
    let r = read_json(|b, l| dspi_apply_guarded_snapshot(h, id.as_ptr(), 0, b, l));
    assert_eq!(r["ok"], false, "{r}");
    dspi_core_free(h);
}
