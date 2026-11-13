//! WP P1AFIX tests through the C ABI: the configuration calls' refusals carry
//! a notice key (review p1a N3), a by-block import with no block is refused
//! (F4), and the crossfeed's custom pair reads typed (N1).

use std::ffi::{CStr, CString};

use super::*;
use crate::fns_device::{
    dspi_core_free, dspi_core_new, dspi_get_crossfeed_custom_pair, dspi_set_crossfeed_freq,
    dspi_set_crossfeed_preset,
};
use crate::types::CrossfeedPair;
use dspi_core::consts::IMPORT_BY_BLOCK;

const QT: &str = include_str!("../../../../tests/fixtures/preset/qt-core-v32.dspipreset");

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0 as c_char; 512 * 1024];
    let needed = f(buf.as_mut_ptr(), buf.len() as u32);
    assert!(
        needed > 0 && needed as usize <= buf.len(),
        "needed {needed}"
    );
    // SAFETY: the call wrote a NUL-terminated reply into `buf`.
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn wp_p1afix_config_refusals_carry_a_notice_key_through_c() {
    let (bad, qt) = (CString::new("{").unwrap(), CString::new(QT).unwrap());
    let h = dspi_core_new();
    let unreadable = [
        read_json(|b, l| dspi_config_diff(bad.as_ptr(), qt.as_ptr(), b, l)),
        read_json(|b, l| dspi_config_import(h, bad.as_ptr(), 0, b, l)),
        read_json(|b, l| dspi_config_import_selected(h, bad.as_ptr(), 0, u32::MAX, b, l)),
        read_json(|b, l| dspi_config_preview(h, bad.as_ptr(), 0, u32::MAX, b, l)),
    ];
    for r in unreadable {
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["noticeKey"], "config.unreadable", "{r}");
    }
    // No device.
    for r in [
        read_json(|b, l| dspi_config_import(h, qt.as_ptr(), 0, b, l)),
        read_json(|b, l| dspi_config_preview(h, qt.as_ptr(), 0, u32::MAX, b, l)),
    ] {
        assert_eq!(r["noticeKey"], "disconnected", "{r}");
    }
    assert!(dspi_open_demo(h));
    let r = read_json(|b, l| {
        dspi_config_import_selected(h, qt.as_ptr(), IMPORT_BY_BLOCK, u32::MAX, b, l)
    });
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["noticeKey"], "config.nothing_selected", "{r}");
    dspi_core_free(h);
}

#[test]
fn wp_p1afix_the_crossfeed_custom_pair_through_c() {
    let h = dspi_core_new();
    let mut out = CrossfeedPair::default();
    assert!(!dspi_get_crossfeed_custom_pair(h, &mut out), "no device");
    assert!(!dspi_get_crossfeed_custom_pair(h, std::ptr::null_mut()));
    assert!(dspi_open_demo(h));
    assert!(dspi_set_crossfeed_preset(h, 3));
    assert!(dspi_set_crossfeed_freq(h, 820.0));
    assert!(dspi_set_crossfeed_preset(h, 2));
    assert!(dspi_get_crossfeed_custom_pair(h, &mut out));
    assert_eq!(out.freq_hz, 820.0);
    dspi_core_free(h);
}
