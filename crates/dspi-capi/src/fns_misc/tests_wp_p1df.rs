//! WP P1DF tests: the document diff, the preview and the import by block
//! through the C ABI.

use std::ffi::{CStr, CString};
use std::ptr::null;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};
use dspi_core::consts::{IMPORT_BY_BLOCK, IMPORT_VOLUMES};

const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
const QT: &str = include_str!("../../../../tests/fixtures/preset/qt-core-v32.dspipreset");

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0 as c_char; 512 * 1024];
    let needed = f(buf.as_mut_ptr(), buf.len() as u32);
    assert!(
        needed > 0 && needed as usize <= buf.len(),
        "needed {needed}"
    );
    // SAFETY: the call wrote a NUL-terminated reply into `buf`.
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn wp_p1df_diff_preview_and_import_by_block() {
    let (mac, qt) = (CString::new(MAC).unwrap(), CString::new(QT).unwrap());
    let d = read_json(|b, l| dspi_config_diff(qt.as_ptr(), mac.as_ptr(), b, l));
    assert_eq!(d["ok"], true, "{d}");
    assert!(d["count"].as_u64().unwrap() > 0);
    assert_eq!(
        read_json(|b, l| dspi_config_diff(null(), mac.as_ptr(), b, l))["ok"],
        false
    );

    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let options = IMPORT_BY_BLOCK | IMPORT_VOLUMES;
    let p = read_json(|b, l| dspi_config_preview(h, mac.as_ptr(), options, u32::MAX, b, l));
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["count"], p["blocks"]["volumes"]);
    assert_eq!(p["count"], 2, "master and user volume: {p}");
    let r = read_json(|b, l| dspi_config_import_selected(h, mac.as_ptr(), options, u32::MAX, b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["blocks"], IMPORT_VOLUMES);
    let left = read_json(|b, l| dspi_config_preview(h, mac.as_ptr(), options, u32::MAX, b, l));
    assert_eq!(left["count"], 0, "{left}");
    dspi_core_free(h);
}
