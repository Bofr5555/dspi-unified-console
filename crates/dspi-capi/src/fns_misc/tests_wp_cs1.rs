//! WP CS1 tests: the USB error counters through the C ABI.

use std::ffi::CStr;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn usb_stats(h: *mut FfiCore) -> serde_json::Value {
    let mut buf = vec![0 as c_char; 512];
    let needed = dspi_fetch_usb_error_stats(h, buf.as_mut_ptr(), 512);
    assert!(needed <= 512);
    // SAFETY: the call wrote a NUL-terminated reply into `buf`.
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn wp_cs1_usb_error_stats_reply_the_six_counters() {
    let h = dspi_core_new();
    assert_eq!(usb_stats(h)["code"], "disconnected");
    assert!(dspi_open_demo(h));
    let v = usb_stats(h);
    assert_eq!(v["ok"], true, "{v}");
    for key in [
        "total",
        "crc",
        "bitstuff",
        "rxOverflow",
        "rxTimeout",
        "dataSeq",
    ] {
        assert_eq!(v[key], 0, "{key}: {v}");
    }
    dspi_core_free(h);
    // A null handle writes nothing.
    let mut buf = [0 as c_char; 8];
    assert_eq!(
        dspi_fetch_usb_error_stats(std::ptr::null_mut(), buf.as_mut_ptr(), 8),
        0
    );
}
