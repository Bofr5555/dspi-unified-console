//! WP FWX: identify and the reset readout through the C ABI, on a fake
//! device as wp/FWX firmware and on the demo (bofr-main 04122d3, without them).

use std::sync::Mutex;

use dspi_core::DspiCore;
use dspi_transport::{Capture, FakeDevice, FakeOptions};

use super::*;
use crate::fns_misc::dspi_open_demo;
use crate::types::DspState;

fn handle(core: std::sync::Arc<DspiCore>) -> *mut FfiCore {
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
    let mut buf = vec![0u8; 1024];
    let n = f(h, buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
    assert!(n > 0 && n <= buf.len());
    serde_json::from_slice(&buf[..n - 1]).unwrap()
}

#[test]
fn wp_fwx_identify_and_reset_info_through_c() {
    let dev = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            diag_ext: true,
            ..FakeOptions::default()
        },
    );
    let core = DspiCore::new();
    core.open_transport(Box::new(dev), "FWX".into(), 0).unwrap();
    let h = handle(core);

    assert_eq!(dspi_identify(h, 30, IDENTIFY_NO_TONE), 0);
    let v = reply(h, dspi_fetch_identify);
    assert_eq!(v["ok"], true);
    assert!(v["remainingS"].as_u64().unwrap() >= 29);
    assert_eq!(v["lastToneOutput"], serde_json::Value::Null);
    assert_eq!(v["board"], "pico2");
    assert_eq!(dspi_identify(h, 0, 2), 0);
    assert_eq!(reply(h, dspi_fetch_identify)["lastToneOutput"], 2);
    // A tone past the outputs is refused.
    assert_eq!(dspi_identify(h, 5, 40), 0xFF);

    let v = reply(h, dspi_fetch_reset_info);
    assert_eq!(v["ok"], true);
    assert_eq!(v["reason"], "power_on");
    assert_eq!(v["reasonCode"], 1);
    assert_eq!(v["prevUptimeS"], serde_json::Value::Null);
    free(h);
}

#[test]
fn wp_fwx_older_firmware_replies_unsupported() {
    let h = handle(DspiCore::new());
    assert_eq!(reply(h, dspi_fetch_reset_info)["code"], "disconnected");
    assert!(dspi_open_demo(h));
    assert_eq!(dspi_identify(h, 5, IDENTIFY_NO_TONE), 0xFF);
    for f in [
        dspi_fetch_identify as extern "C" fn(_, _, _) -> _,
        dspi_fetch_reset_info,
    ] {
        let v = reply(h, f);
        assert_eq!(
            (v["ok"].clone(), v["code"].clone()),
            (false.into(), "unsupported".into())
        );
    }
    // A null handle is 0, not a crash.
    assert_eq!(
        dspi_fetch_identify(std::ptr::null_mut(), std::ptr::null_mut(), 0),
        0
    );
    free(h);
}
