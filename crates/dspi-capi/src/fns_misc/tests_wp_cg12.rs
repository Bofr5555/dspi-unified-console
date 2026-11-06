//! WP CG12 tests: the output ring health functions, over the demo device.

use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::Mutex;

use super::*;
use crate::types::DspState;

fn demo() -> *mut FfiCore {
    let core = DspiCore::new();
    let state = DspState::from(&core.get_state());
    let h = Box::into_raw(Box::new(FfiCore {
        core,
        state: Mutex::new(Box::new(state)),
    }));
    assert!(crate::fns_misc::dspi_open_demo(h));
    h
}

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 16 * 1024];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(
        needed > 0 && needed as usize <= buf.len(),
        "needed {needed}"
    );
    let text = std::str::from_utf8(&buf[..needed as usize - 1]).unwrap();
    serde_json::from_str(text).unwrap()
}

#[test]
fn wp_cg12_health_on_the_demo_device() {
    let h = demo();
    let reply = read_json(|b, l| dspi_fetch_ring_health(h, b, l));
    assert_eq!(reply["ok"], true);
    assert!(reply["targetFrames"].as_u64().unwrap() > 0);
    for field in [
        "targetUs",
        "batchFrames",
        "underrunEpisodes",
        "refusedBlocks",
        "lowestFill",
        "worstInputGapUs",
        "worstUsbGapUs",
        "overBudgetSteps",
    ] {
        assert!(reply.get(field).is_some(), "{field}");
    }
    drop(unsafe { Box::from_raw(h) });
}

#[test]
fn wp_cg12_fills_on_the_demo_device() {
    let h = demo();
    let reply = read_json(|b, l| dspi_fetch_output_fills(h, b, l));
    assert_eq!(reply["ok"], true, "{reply}");
    let fills = reply["value"].as_array().unwrap();
    assert_eq!(fills.len(), 4);
    assert_eq!(fills[0]["band"], "good");
    assert!(fills[0]["fillPct"].as_f64().unwrap() > 0.0);
    drop(unsafe { Box::from_raw(h) });
}

#[test]
fn wp_cg12_a_null_handle_gives_the_failure_value() {
    let n = null_mut::<FfiCore>();
    let mut b = [7u8; 16];
    let t = b.as_mut_ptr().cast::<c_char>();
    assert_eq!(dspi_fetch_ring_health(n, t, 16), 0);
    assert_eq!(dspi_fetch_output_fills(n, t, 16), 0);
}

#[test]
fn wp_cg12_reply_shapes() {
    let absent: Result<Option<dspi_core::RingHealth>> = Ok(None);
    assert_eq!(json_reply(absent), r#"{"ok":true,"value":null}"#);

    let v: serde_json::Value =
        serde_json::from_str(&json_reply(Ok(Some(dspi_core::RingHealth::default())))).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v["lowestFill"].is_null());

    let fills = Ok(vec![dspi_core::OutputFill {
        band: dspi_core::FillBand::Warn,
        ..Default::default()
    }]);
    let v: serde_json::Value = serde_json::from_str(&json_reply(fills)).unwrap();
    assert_eq!(v["value"][0]["band"], "warn");
    assert_eq!(v["value"][0]["fillPct"], 0.0);
}

#[test]
fn wp_cg12_functions_are_in_the_header() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
    assert!(
        header.contains("dspi_fetch_ring_health(struct FfiCore *core, char *buf, uint32_t len)")
    );
    assert!(
        header.contains("dspi_fetch_output_fills(struct FfiCore *core, char *buf, uint32_t len)")
    );
}
