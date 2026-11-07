//! WP CG23 tests: the raw notification tap and the generation through C.

use std::time::{Duration, Instant};

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_disconnect, dspi_set_master_volume};

/// Drain into a buffer of `len` bytes: the length needed and the text.
fn drain(h: *mut FfiCore, len: usize) -> (u32, String) {
    let mut buf = vec![0u8; len.max(1)];
    let needed = dspi_notify_tap_drain(h, buf.as_mut_ptr().cast(), len as u32);
    let end = buf.iter().position(|b| *b == 0).unwrap_or(0);
    (needed, String::from_utf8(buf[..end].to_vec()).unwrap())
}

/// Process notifications until the tap holds something, then drain it.
fn entries(h: *mut FfiCore) -> Vec<serde_json::Value> {
    let start = Instant::now();
    loop {
        dspi_process_notifications(h, ptr::null_mut());
        let (needed, text) = drain(h, 1 << 18);
        assert!(needed as usize <= 1 << 18);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["ok"], true, "{v}");
        let list = v["value"].as_array().unwrap().clone();
        if !list.is_empty() {
            return list;
        }
        assert!(start.elapsed() < Duration::from_secs(8), "nothing tapped");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn wp_cg23_the_tap_and_the_generation_through_c() {
    let h = dspi_core_new();
    assert_eq!(dspi_connection_generation(h), 0);
    assert!(dspi_open_demo(h));
    let first = dspi_connection_generation(h);
    assert_ne!(first, 0);

    dspi_notify_tap_set(h, true);
    assert!(dspi_set_master_volume(h, -14.0));
    let got = entries(h);
    let e = &got[0];
    for key in ["packet", "origin", "own", "lost", "receivedMs"] {
        assert!(e.get(key).is_some(), "{key} in {e}");
    }
    assert!(e["packet"].as_array().is_some_and(|p| !p.is_empty()));

    // A drain the buffer could not hold marks the next entry lost.
    assert!(dspi_set_master_volume(h, -15.0));
    let start = Instant::now();
    loop {
        dspi_process_notifications(h, ptr::null_mut());
        let (needed, text) = drain(h, 1);
        assert!(text.is_empty());
        if needed > 100 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(8), "nothing tapped");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(dspi_set_master_volume(h, -16.0));
    let got = entries(h);
    assert_eq!(got[0]["lost"], true, "{got:?}");

    // Nothing kept: a short buffer loses nothing.
    let (needed, _) = drain(h, 1);
    assert!(needed > 1);
    assert!(dspi_set_master_volume(h, -13.0));
    assert_eq!(entries(h)[0]["lost"], false);

    dspi_disconnect(h);
    assert_eq!(dspi_connection_generation(h), 0);
    assert!(dspi_open_demo(h));
    assert_ne!(dspi_connection_generation(h), first);
    dspi_notify_tap_set(h, false);
    dspi_core_free(h);

    assert_eq!(dspi_connection_generation(ptr::null_mut()), 0);
    assert_eq!(drain(ptr::null_mut(), 64).0, 0);
}
