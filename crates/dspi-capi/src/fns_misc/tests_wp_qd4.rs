//! WP QD4 tests: the cached names and a rename elsewhere, through C, over the
//! demo device.

use std::ffi::{CStr, CString};
use std::time::{Duration, Instant};

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};
use crate::types::NotifyResult;

fn cached_device(h: *mut FfiCore) -> Option<String> {
    let mut buf = [0 as c_char; 32];
    dspi_cached_device_name(h, buf.as_mut_ptr(), 32).then(|| {
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_str()
            .unwrap()
            .to_owned()
    })
}

fn cached_source(h: *mut FfiCore, source: u8) -> Option<String> {
    let mut buf = [0 as c_char; 32];
    dspi_cached_input_source_name(h, source, buf.as_mut_ptr(), 32).then(|| {
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_str()
            .unwrap()
            .to_owned()
    })
}

#[test]
fn wp_qd4_names_through_c_on_the_demo_device() {
    let h = dspi_core_new();
    assert_eq!(cached_device(h), None, "no device yet");
    assert!(dspi_open_demo(h));
    assert!(crate::fns_device::dspi_fetch_all(h));
    assert_eq!(cached_device(h).as_deref(), Some("DSPi Demo"));
    assert_eq!(cached_source(h, 2).as_deref(), Some(""));
    assert_eq!(cached_source(h, 7), None);

    let tape = CString::new("Tape").unwrap();
    assert!(dspi_demo_rename_elsewhere(h, 2, tape.as_ptr()));
    assert!(dspi_demo_rename_elsewhere(h, 0xFF, tape.as_ptr()));
    let start = Instant::now();
    let mut r = NotifyResult::default();
    while cached_source(h, 2).as_deref() != Some("Tape")
        || cached_device(h).as_deref() != Some("Tape")
    {
        dspi_process_notifications(h, &mut r);
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!dspi_demo_rename_elsewhere(h, 2, std::ptr::null()));
    dspi_core_free(h);
}

#[test]
fn wp_qd4_null_arguments_are_refused() {
    let h = dspi_core_new();
    let mut buf = [0 as c_char; 1];
    assert!(!dspi_cached_device_name(h, std::ptr::null_mut(), 32));
    assert!(!dspi_cached_input_source_name(h, 0, buf.as_mut_ptr(), 0));
    assert!(!dspi_cached_device_name(
        std::ptr::null_mut(),
        buf.as_mut_ptr(),
        1
    ));
    dspi_core_free(h);
}
