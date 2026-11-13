//! WP SW1: the audio link through the C ABI, on the demo with the link and on
//! the plain demo (no link: `unsupported`).

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
    let mut buf = vec![0u8; 2048];
    let n = f(h, buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
    assert!(n > 0 && n <= buf.len());
    serde_json::from_slice(&buf[..n - 1]).unwrap()
}

#[test]
fn wp_sw1_the_link_through_c() {
    let h = handle();
    assert!(dspi_open_demo_audio_link(h));
    let info = reply(h, dspi_audio_link_info);
    assert_eq!(info["ok"], true);
    assert_eq!(info["role"], "receiver");
    assert_eq!(info["dLocalMin"], 112);
    assert_eq!(info["linkInput"], true);
    assert_eq!(reply(h, dspi_fetch_audio_link_info)["maxChannels"], 64);

    let s = reply(h, dspi_fetch_audio_link_status);
    assert_eq!(s["state"], "locked");
    assert!(s["frame"].as_u64().unwrap() > 0);

    let slots = [3u8, 2, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(dspi_set_audio_link_slots(h, slots.as_ptr(), 8), 0);
    let mut out = [0u8; 8];
    assert!(dspi_fetch_audio_link_slots(h, out.as_mut_ptr(), 8));
    assert_eq!(out, slots);
    assert!(!dspi_fetch_audio_link_slots(h, out.as_mut_ptr(), 7));
    assert_eq!(dspi_set_audio_link_slots(h, slots.as_ptr(), 7), 0xFF);
    assert_eq!(dspi_set_audio_link_slots(h, std::ptr::null(), 8), 0xFF);

    assert_eq!(dspi_set_audio_link_phase(h, 1, 200, -1), 0);
    let p = reply(h, dspi_fetch_audio_link_phase);
    assert_eq!(
        (p["dLocal"].as_u64(), p["hopsOffset"].as_i64()),
        (Some(200), Some(-1))
    );

    let f = reply(h, dspi_fetch_audio_link_frame_counter)["frame"]
        .as_u64()
        .unwrap();
    assert_eq!(dspi_apply_at_frame(h, f + 48_000_000, 7, 1, 0), 0);
    assert_eq!(reply(h, dspi_fetch_apply_at_frame_status)["pending"], 1);
    assert_eq!(dspi_apply_at_frame(h, 0, 0, 0, 0), 0);
    let st = reply(h, dspi_fetch_apply_at_frame_status);
    assert_eq!(
        (st["lastOp"].as_str(), st["lastResult"].as_str()),
        (Some("click"), Some("cancelled"))
    );
    assert_eq!(dspi_apply_at_frame(h, 0, 9, 0, 0), 0xFF);
    assert_eq!(dspi_reset_audio_link_stats(h), 0);
    free(h);
}

#[test]
fn wp_sw1_without_the_link_c_says_unsupported() {
    let h = handle();
    assert!(dspi_open_demo(h));
    let info = reply(h, dspi_audio_link_info);
    assert_eq!(
        (info["ok"].as_bool(), info["value"].is_null()),
        (Some(true), true)
    );
    for f in [
        dspi_fetch_audio_link_status as extern "C" fn(*mut FfiCore, *mut c_char, u32) -> u32,
        dspi_fetch_audio_link_info,
        dspi_fetch_audio_link_phase,
        dspi_fetch_audio_link_frame_counter,
        dspi_fetch_apply_at_frame_status,
    ] {
        let v = reply(h, f);
        assert_eq!(v["code"], "unsupported", "{v}");
    }
    assert_eq!(dspi_reset_audio_link_stats(h), 0xFF);
    // Null handles never crash.
    assert_eq!(dspi_reset_audio_link_stats(std::ptr::null_mut()), 0xFF);
    assert_eq!(
        dspi_fetch_audio_link_status(std::ptr::null_mut(), std::ptr::null_mut(), 0),
        0
    );
    free(h);
}
