//! WP CG24 tests: the held device lock through C.

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

/// `dspi_device_lock_acquire`'s JSON reply.
fn acquire(h: *mut FfiCore, timeout_ms: u32) -> serde_json::Value {
    let reason = CString::new("Installing firmware").unwrap();
    let mut buf = vec![0 as c_char; 256];
    let n = dspi_device_lock_acquire(h, reason.as_ptr(), timeout_ms, buf.as_mut_ptr(), 256);
    assert!(n > 0 && n <= 256);
    // SAFETY: the reply was written NUL-terminated into `buf`.
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[test]
fn wp_cg24_acquire_and_release_through_c() {
    let h = dspi_core_new();
    assert_eq!(acquire(h, 1000)["code"], "disconnected");
    assert!(dspi_open_demo(h));
    assert_eq!(acquire(h, 0)["code"], "invalid");
    assert_eq!(acquire(h, 1000), serde_json::json!({ "ok": true }));
    dspi_device_lock_release(h);
    dspi_device_lock_release(h);
    dspi_core_free(h);
    // A null handle is no crash.
    dspi_device_lock_release(ptr::null_mut());
    assert_eq!(
        dspi_device_lock_acquire(ptr::null_mut(), ptr::null(), 1, ptr::null_mut(), 0),
        0
    );
}
