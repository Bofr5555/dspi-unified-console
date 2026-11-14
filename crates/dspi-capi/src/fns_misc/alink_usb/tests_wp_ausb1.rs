//! WP AUSB1: the USB-direct block through the C ABI, on the USB-direct demo
//! and on the plain demo (`unsupported`).

use std::sync::Mutex;

use crate::fns_misc::dspi_open_demo;
use crate::types::DspState;

use super::*;

fn handle() -> *mut FfiCore {
    let core = DspiCore::new();
    let state = DspState::from(&core.get_state());
    Box::into_raw(Box::new(FfiCore {
        core,
        state: Mutex::new(Box::new(state)),
    }))
}

fn free(h: *mut FfiCore) {
    // SAFETY: `h` came from `handle`, and is freed once.
    drop(unsafe { Box::from_raw(h) });
}

fn reply(
    h: *mut FfiCore,
    f: extern "C" fn(*mut FfiCore, *mut c_char, u32) -> u32,
) -> serde_json::Value {
    let mut buf = vec![0u8; 4096];
    let n = f(h, buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
    assert!(n > 0 && n <= buf.len());
    serde_json::from_slice(&buf[..n - 1]).unwrap()
}

#[test]
fn wp_ausb1_usb_direct_through_c() {
    let h = handle();
    assert!(dspi_open_demo_usb_direct(h));
    assert!(dspi_usb_direct_supported(h));
    let c = reply(h, dspi_fetch_alink_usb);
    assert_eq!(
        (c["ok"].as_bool(), c["mode"].as_u64(), c["d"].as_u64()),
        (Some(true), Some(2), Some(192))
    );
    let s = reply(h, dspi_fetch_alink_usb_status);
    assert_eq!(s["state"], "phase_locked");
    assert_eq!(
        (s["dMin"].as_u64(), s["dMax"].as_u64()),
        (Some(170), Some(399))
    );
    assert_eq!(dspi_set_alink_usb(h, 2, true, 250), 0);
    assert_eq!(reply(h, dspi_fetch_alink_usb)["d"], 250);
    assert_eq!(dspi_set_alink_usb(h, 3, true, 250), 0xFF);
    // -- WP AUSB2 -- the lock-error action, and the fields that come with it
    assert_eq!(reply(h, dspi_fetch_alink_usb)["onLockError"], "mute");
    assert_eq!(dspi_set_alink_usb_on_lock_error(h, true), 0);
    assert_eq!(
        reply(h, dspi_fetch_alink_usb)["onLockError"],
        "play_unaligned"
    );
    assert_eq!(reply(h, dspi_fetch_alink_usb_status)["lockError"], false);
    let d = reply(h, dspi_fetch_alink_usb_diag);
    assert_eq!(d["ends"].as_array().map(Vec::len), Some(9));
    assert_eq!(dspi_clear_alink_usb_diag(h), 0);
    std::thread::sleep(std::time::Duration::from_millis(60));
    let f = reply(h, dspi_fetch_frame_counter_time_base);
    assert_eq!(f["timeBase"], "usb_sample");
    let bus = (f["busFrame"].as_u64().unwrap() + 40) % 2048;
    assert_eq!(dspi_apply_at_time_base(h, 2, bus, 1, 0, 0), 0);
    assert_eq!(dspi_apply_at_time_base(h, 2, 1 << 12, 1, 0, 0), 0xFF);

    // The plain demo: no block, nothing sent.
    assert!(dspi_open_demo(h));
    assert!(!dspi_usb_direct_supported(h));
    assert_eq!(reply(h, dspi_fetch_alink_usb_status)["code"], "unsupported");
    assert_eq!(dspi_set_alink_usb(h, 2, true, 192), 0xFF);
    assert_eq!(dspi_set_alink_usb_on_lock_error(h, false), 0xFF);
    free(h);
}
