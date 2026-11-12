//! WP P0W tests: undo, redo and the change history through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_get_state, dspi_set_output_gain};

/// A text call's reply as JSON.
fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 8192];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn gain(h: *mut FfiCore) -> f32 {
    // SAFETY: the handle is live; the record stays valid until the next call.
    unsafe { (*dspi_get_state(h)).output_gain_db[0] }
}

#[test]
fn wp_p0w_undo_redo_and_history_through_c() {
    assert!(!dspi_can_undo(ptr::null()));
    assert!(!dspi_can_redo(ptr::null()));
    let h = dspi_core_new();
    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(
        (r["ok"].clone(), r["code"].clone()),
        (json!(false), json!("disconnected"))
    );

    assert!(dspi_open_demo(h));
    let start = gain(h);
    assert!(dspi_set_output_gain(h, 0, -7.5));
    assert!(dspi_can_undo(h) && !dspi_can_redo(h));

    let r = reply(|b, l| dspi_history(h, 5, b, l));
    assert_eq!(r["ok"], true, "{r}");
    let e = &r["value"][0];
    assert_eq!(e["path"], "out.gain");
    assert_eq!(e["indices"], json!([0]));
    assert_eq!(e["after"], json!({ "float": -7.5 }));
    assert_eq!(e["source"], "local");
    assert_eq!(e["undoable"], true);
    assert!(e["atMs"].as_u64().unwrap() > 0);

    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["entry"]["path"], "out.gain");
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(gain(h), start);
    assert!(dspi_can_redo(h));

    let r = reply(|b, l| dspi_redo(h, b, l));
    assert_eq!(r["entry"]["after"], json!({ "float": -7.5 }));
    assert_eq!(gain(h), -7.5);

    let r = reply(|b, l| dspi_history(h, 1, b, l));
    assert_eq!(r["value"][0]["source"], "redo");
    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(r["ok"], true);
    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(r["entry"], Value::Null, "nothing left: {r}");
    dspi_core_free(h);
}

#[test]
fn wp_p0w_the_history_flag_is_in_the_header_constants() {
    assert_eq!(
        crate::types::NOTIFY_HISTORY,
        dspi_core::consts::NOTIFY_HISTORY
    );
}
