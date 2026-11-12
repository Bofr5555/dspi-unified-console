//! WP QP16 tests: the last failure on the calling thread, as the delay-refusal
//! banner takes it after a refused delay.

use std::ffi::CStr;
use std::ptr::null_mut;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_max_delay_ms, dspi_set_output_delay};

/// `dspi_take_failure` into a buffer of `len` bytes, as text ("" when it did
/// not fit).
fn take(len: u32) -> (u32, String) {
    let mut buf = vec![0 as c_char; len as usize];
    let needed = dspi_take_failure(buf.as_mut_ptr(), len);
    let text = if needed <= len {
        // SAFETY: the call wrote a NUL-terminated reply into `buf`.
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };
    (needed, text)
}

#[test]
fn wp_qp16_a_refused_delay_leaves_the_cores_sentence() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let _ = take(512);
    assert_eq!(
        take(512).1,
        r#"{"ok":true}"#,
        "nothing failed since the take"
    );

    let max = dspi_max_delay_ms(h);
    assert!(max > 0.0);
    assert!(!dspi_set_output_delay(h, 0, max + 5.0));
    let (needed, text) = take(4);
    assert!(needed > 4 && text.is_empty(), "too short: nothing written");
    let reply: Value = serde_json::from_str(&take(512).1).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["code"], "invalid");
    let sentence = reply["error"].as_str().unwrap();
    assert!(sentence.contains("out of range"), "{sentence}");
    assert!(sentence.contains(&format!("0 to {max} ms")), "{sentence}");

    // Taken: the next take is clean, and an accepted delay leaves none.
    assert_eq!(take(512).1, r#"{"ok":true}"#);
    assert!(dspi_set_output_delay(h, 0, 1.5));
    assert_eq!(take(512).1, r#"{"ok":true}"#);
    dspi_core_free(h);
}

#[test]
fn wp_qp16_the_failure_belongs_to_its_thread() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    assert!(!dspi_set_output_delay(h, 0, f32::NAN));
    let other = std::thread::spawn(|| take(512).1).join().unwrap();
    assert_eq!(other, r#"{"ok":true}"#, "another thread has none");
    assert!(take(512).1.contains(r#""ok":false"#));
    // A null buffer only asks for the length, and takes nothing.
    assert!(!dspi_set_output_delay(h, 0, 1.0e6));
    let needed = dspi_take_failure(null_mut(), 0);
    assert!(needed > 1);
    assert_eq!(take(needed).0, needed);
    dspi_core_free(h);
}
