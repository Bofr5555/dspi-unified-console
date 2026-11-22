//! WP MDQT1: the main core through C.

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_is_connected, dspi_set_master_volume};
use crate::fns_misc::{dspi_open_demo, dspi_set_hotplug_callback};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 65536];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn device_of(set: *mut FfiCoreSet, core: *const FfiCore) -> Value {
    reply(|b, l| dspi_core_set_list(set, b, l));
    reply(|b, l| dspi_core_set_device_of(set, core, b, l))["value"].clone()
}

extern "C" fn no_event(_: u8, _: *const c_char, _: *mut c_void) {}

#[test]
fn wp_mdqt1_the_main_core_through_c() {
    let set = dspi_core_set_new();
    let main = dspi_core_set_main(set);
    assert!(!main.is_null());
    assert_eq!(main, dspi_core_set_main(set), "one handle");
    assert_eq!(device_of(set, main), Value::Null, "nothing open");
    dspi_set_hotplug_callback(main, Some(no_event), ptr::null_mut());

    assert!(dspi_open_demo(main));
    assert_eq!(device_of(set, main)["id"], "demo:1");
    let id = CString::new("demo:1").unwrap();
    assert_eq!(
        dspi_core_set_core(set, id.as_ptr()),
        main,
        "its id gives the same handle"
    );
    let demo = CString::new(r#"{"kind":"demo"}"#).unwrap();
    let other = reply(|b, l| dspi_core_set_open(set, demo.as_ptr(), b, l));
    assert_eq!(other, json!({"ok": true, "id": "demo:2"}));

    // Closed: the handle and its callbacks stay, and it opens again.
    assert_eq!(
        reply(|b, l| dspi_core_set_close(set, id.as_ptr(), b, l)),
        json!({"ok": true})
    );
    assert!(!dspi_is_connected(main));
    assert_eq!(dspi_core_set_main(set), main);
    assert!(dspi_core_set_core(set, id.as_ptr()).is_null());
    assert!(dspi_open_demo(main));
    assert_eq!(device_of(set, main)["id"], "demo:3");
    assert!(dspi_set_master_volume(main, -12.0));

    // A foreign core is no device of the set.
    let alone = crate::fns_device::dspi_core_new();
    assert_eq!(device_of(set, alone), Value::Null);
    crate::fns_device::dspi_core_free(alone);
    assert!(dspi_core_set_main(ptr::null_mut()).is_null());
    assert_eq!(
        dspi_core_set_device_of(ptr::null_mut(), main, ptr::null_mut(), 0),
        0
    );
    dspi_core_set_free(set);
}

/// The short-buffer rule of `_open`: the device is open, its id in the list.
#[test]
fn wp_mdqt1_an_open_with_a_short_buffer_has_happened() {
    let set = dspi_core_set_new();
    let demo = CString::new(r#"{"kind":"demo"}"#).unwrap();
    let mut small = [0u8; 4];
    let needed = dspi_core_set_open(set, demo.as_ptr(), small.as_mut_ptr().cast(), 4);
    assert!(needed > 4);
    let list = reply(|b, l| dspi_core_set_list(set, b, l));
    assert_eq!(list["value"][0]["id"], "demo:1");
    dspi_core_set_free(set);
}
