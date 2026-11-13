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

// -- WP QTGAP1 --
/// Only writes inside `dspi_set_dragging(true)` .. `(false)` join one step.
#[test]
fn wp_qtgap1_a_drag_through_c() {
    dspi_set_dragging(ptr::null(), true);
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    assert!(dspi_set_output_gain(h, 0, -1.0));
    assert!(dspi_set_output_gain(h, 0, -2.0));
    dspi_set_dragging(h, true);
    assert!(dspi_set_output_gain(h, 0, -3.0));
    assert!(dspi_set_output_gain(h, 0, -4.0));
    dspi_set_dragging(h, false);
    let r = reply(|b, l| dspi_history(h, 10, b, l));
    assert_eq!(r["value"].as_array().unwrap().len(), 3, "{r}");
    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(r["entry"]["before"], json!({ "float": -2.0 }), "{r}");
    dspi_core_free(h);
}

// -- WP P0W2 --
/// A graph drag mixing one-axis and two-axis moves of one band, sent whole
/// with `dspi_set_filter`, is one step; undo puts the band back as it was.
#[test]
fn wp_p0w2_a_mixed_axis_band_drag_through_c_is_one_step() {
    use crate::fns_device::dspi_set_filter;
    let band = |h: *mut FfiCore| -> FilterParams {
        // SAFETY: the handle is live; the record stays valid until the next call.
        unsafe { (*dspi_get_state(h)).filters[0][1] }
    };
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let start = band(h);
    let mut p = start;
    dspi_set_dragging(h, true);
    for (df, dg) in [(10.0, 0.0), (20.0, 1.0), (0.0, 1.5), (5.0, 0.0)] {
        p.freq += df;
        p.gain += dg;
        assert!(dspi_set_filter(h, 0, 1, p));
    }
    dspi_set_dragging(h, false);
    let r = reply(|b, l| dspi_history(h, 10, b, l));
    assert_eq!(r["value"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["value"][0]["path"], "eq.band");
    let r = reply(|b, l| dspi_undo(h, b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(band(h), start);
    assert!(!dspi_can_undo(h));
    dspi_core_free(h);
}
