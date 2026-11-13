//! WP P1UG tests: undo groups through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_get_state, dspi_set_output_gain};

/// A text call's reply as JSON.
fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 16384];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn gains(h: *mut FfiCore) -> (f32, f32) {
    // SAFETY: the handle is live; the record stays valid until the next call.
    unsafe {
        let s = &*dspi_get_state(h);
        (s.output_gain_db[0], s.output_gain_db[1])
    }
}

/// Two writes between `dspi_undo_group_begin` and `_end` are one step with
/// the label; one undo takes both back, one redo puts both again.
#[test]
fn wp_p1ug_an_undo_group_through_c() {
    let label = CString::new("Linked pair").unwrap();
    dspi_undo_group_begin(ptr::null(), label.as_ptr());
    dspi_undo_group_end(ptr::null());
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let start = gains(h);
    dspi_undo_group_begin(h, label.as_ptr());
    assert!(dspi_set_output_gain(h, 0, -5.0));
    assert!(dspi_set_output_gain(h, 1, -5.0));
    dspi_undo_group_end(h);

    let r = reply(|b, l| dspi_history(h, 10, b, l));
    let list = r["value"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{r}");
    assert_eq!(list[0]["label"], "Linked pair");

    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(r["entry"]["label"], "Linked pair", "{r}");
    assert_eq!(gains(h), start);
    assert!(!dspi_can_undo(h));
    reply(|b, l| dspi_redo(h, b, l));
    assert_eq!(gains(h), (-5.0, -5.0));

    // A null label: still one step, no label.
    dspi_undo_group_begin(h, ptr::null());
    assert!(dspi_set_output_gain(h, 0, -6.0));
    assert!(dspi_set_output_gain(h, 1, -6.0));
    dspi_undo_group_end(h);
    let r = reply(|b, l| dspi_history(h, 1, b, l));
    assert_eq!(r["value"][0]["label"], Value::Null);
    reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(gains(h), (-5.0, -5.0));
    dspi_core_free(h);
}
