//! WP P1AB tests: A/B compare through the C ABI on the demo device.

use std::ffi::{CStr, CString};
use std::ptr::null;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> String {
    let mut buf = vec![0 as c_char; 512 * 1024];
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

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    serde_json::from_str(&reply(f)).unwrap()
}

#[test]
fn wp_p1ab_compare_through_the_c_abi() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));

    // No compare: a typed failure; the state says none.
    let r = read_json(|b, l| dspi_ab_toggle(h, b, l));
    assert_eq!(
        (r["ok"].clone(), r["code"].clone()),
        (json!(false), json!("invalid")),
        "{r}"
    );
    let r = read_json(|b, l| dspi_ab_state(h, b, l));
    assert_eq!(
        (r["ok"].clone(), r["active"].clone()),
        (json!(true), json!(false)),
        "{r}"
    );

    // B: the demo with a changed preamp; then back to A.
    let name = CString::new("Bee").unwrap();
    let a = reply(|b, l| dspi_snapshot_capture_document(h, name.as_ptr(), null(), b, l));
    let a_state = with_core(h, None, |c| Some(c.core.get_state())).unwrap();
    with_core(h, (), |c| c.core.set_input_preamp(0, -6.0).unwrap());
    let b_doc = reply(|b, l| dspi_snapshot_capture_document(h, name.as_ptr(), null(), b, l));
    let b_state = with_core(h, None, |c| Some(c.core.get_state())).unwrap();
    let a_c = CString::new(a).unwrap();
    let b_c = CString::new(b_doc).unwrap();
    let r = read_json(|b, l| dspi_snapshot_recall_document(h, a_c.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");

    let s = read_json(|b, l| dspi_ab_start_documents(h, null(), b_c.as_ptr(), b, l));
    assert_eq!(s["ok"], true, "{s}");
    assert_eq!(
        (s["active"].clone(), s["side"].clone()),
        (json!(true), json!("a"))
    );
    assert_eq!(s["aName"], "A");
    assert_eq!(s["recall"], Value::Null);

    let t = read_json(|b, l| dspi_ab_toggle(h, b, l));
    assert_eq!(t["side"], "b", "{t}");
    assert_eq!(t["recall"]["image"], true);
    assert_eq!(
        with_core(h, None, |c| Some(c.core.get_state())).unwrap(),
        b_state
    );
    let t = read_json(|b, l| dspi_ab_toggle(h, b, l));
    assert_eq!(t["side"], "a", "{t}");
    assert_eq!(
        with_core(h, None, |c| Some(c.core.get_state())).unwrap(),
        a_state
    );

    // The diff is config_diff's for the pair.
    let d: Value = serde_json::from_str(&reply(|b, l| dspi_ab_diff(h, b, l))).unwrap();
    let want: Value = serde_json::from_str(&dspi_core::files::config_diff(
        a_c.to_string_lossy().into_owned(),
        b_c.to_string_lossy().into_owned(),
    ))
    .unwrap();
    assert_eq!(d, want);
    assert_eq!(d["ok"], true);

    // A bad side is refused; keep B ends it on B.
    let bad = CString::new("c").unwrap();
    let r = read_json(|b, l| dspi_ab_keep(h, bad.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid", "{r}");
    let side = CString::new("b").unwrap();
    let k = read_json(|b, l| dspi_ab_keep(h, side.as_ptr(), b, l));
    assert_eq!(
        (k["active"].clone(), k["side"].clone()),
        (json!(false), json!("b")),
        "{k}"
    );
    assert_eq!(
        with_core(h, None, |c| Some(c.core.get_state())).unwrap(),
        b_state
    );

    // A disconnect ends a compare.
    let s = read_json(|b, l| dspi_ab_start_documents(h, null(), a_c.as_ptr(), b, l));
    assert_eq!(s["active"], true, "{s}");
    with_core(h, (), |c| c.core.disconnect());
    let r = read_json(|b, l| dspi_ab_state(h, b, l));
    assert_eq!(r["active"], false, "{r}");
    let r = read_json(|b, l| dspi_ab_diff(h, b, l));
    assert_eq!(r["ok"], false, "{r}");
    dspi_core_free(h);
}
