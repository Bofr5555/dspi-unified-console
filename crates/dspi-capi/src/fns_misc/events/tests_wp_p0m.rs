//! WP P0M: the meter ballistics and the event history over the C ABI.

use std::ffi::{CStr, CString};
use std::ptr::{null, null_mut};

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_disconnect, dspi_fetch_status};
use crate::fns_misc::dspi_open_demo;
use crate::types::SystemStatus;

fn demo() -> *mut FfiCore {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    h
}

fn text_of(f: impl Fn(*mut c_char, u32) -> u32) -> String {
    let need = f(null_mut(), 0);
    let mut buf = vec![0 as c_char; need as usize];
    assert_eq!(f(buf.as_mut_ptr(), need), need);
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn wp_p0m_status_and_ballistics_cross_the_c_abi() {
    let h = demo();
    let mut b = MeterBallistics {
        hold_ms: 0,
        decay_per_s: 0.0,
        clip_hold_ms: 0,
    };
    assert!(dspi_meter_ballistics(h, &raw mut b));
    assert_eq!(
        (b.hold_ms, b.decay_per_s, b.clip_hold_ms),
        (1000, 0.5, 10_000)
    );
    dspi_set_meter_ballistics(h, 250, 2.0, 0);
    assert!(dspi_meter_ballistics(h, &raw mut b));
    assert_eq!((b.hold_ms, b.decay_per_s, b.clip_hold_ms), (250, 2.0, 0));
    assert!(!dspi_meter_ballistics(h, null_mut()));
    assert!(!dspi_meter_ballistics(null_mut(), &raw mut b));
    dspi_set_meter_ballistics(null_mut(), 1, 1.0, 1);
    dspi_reset_meter_max(null_mut());

    let mut st = SystemStatus::from(&dspi_core::SystemStatus::default());
    assert!(dspi_fetch_status(h, &raw mut st));
    // The hold is at least the live level; the maximum at least the hold.
    for i in 0..st.peaks.len() {
        assert!(st.peak_hold[i] >= st.peaks[i]);
        assert!(st.peak_max[i] >= st.peak_hold[i]);
    }
    dspi_reset_meter_max(h);
    dspi_core_free(h);
}

#[test]
fn wp_p0m_the_history_reads_and_exports_as_json_lines() {
    let h = demo();
    // Paging with the id handed back.
    let mut buf = vec![0 as c_char; 64 * 1024];
    let last = dspi_event_log_read(h, null(), 0, 100, buf.as_mut_ptr(), buf.len() as u32);
    assert!(last >= 1);
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_str()
        .unwrap()
        .to_owned();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["kind"], "connected");
    assert_eq!(first["v"], 1);
    let serial = first["serial"].as_str().unwrap().to_owned();
    assert_eq!(
        dspi_event_log_read(h, null(), last, 100, buf.as_mut_ptr(), buf.len() as u32),
        last
    );
    // A buffer too small for any record: the record is skipped past, nothing written.
    let mut tiny = [0x55 as c_char; 8];
    assert_eq!(
        dspi_event_log_read(h, null(), 0, 1, tiny.as_mut_ptr(), 8),
        last
    );
    assert_eq!(tiny[0], 0);
    // Unusable buffers and handles give `after_id` back.
    assert_eq!(dspi_event_log_read(h, null(), 7, 1, null_mut(), 8), 7);
    assert_eq!(dspi_event_log_read(h, null(), 7, 1, buf.as_mut_ptr(), 0), 7);
    assert_eq!(
        dspi_event_log_read(null_mut(), null(), 7, 1, buf.as_mut_ptr(), 8),
        7
    );

    dspi_disconnect(h);
    let c = CString::new(serial.clone()).unwrap();
    let export = text_of(|b, n| dspi_event_log_export(h, c.as_ptr(), b, n));
    let kinds: Vec<String> = export
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds.first().map(String::as_str), Some("connected"));
    assert_eq!(kinds.last().map(String::as_str), Some("disconnected"));
    let devices = text_of(|b, n| dspi_event_log_devices(h, b, n));
    assert_eq!(devices, serde_json::json!([serial]).to_string());
    // No open device and no serial: nothing.
    assert_eq!(text_of(|b, n| dspi_event_log_export(h, null(), b, n)), "");
    dspi_core_free(h);
}

#[test]
fn wp_p0m_set_dir_reports_failure() {
    let h = dspi_core_new();
    let base = std::env::temp_dir().join(format!("wp-p0m-capi-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("file");
    std::fs::write(&file, "x").unwrap();
    let bad = CString::new(file.join("sub").to_string_lossy().into_owned()).unwrap();
    assert!(!dspi_event_log_set_dir(h, bad.as_ptr()));
    let good = CString::new(base.join("events").to_string_lossy().into_owned()).unwrap();
    assert!(dspi_event_log_set_dir(h, good.as_ptr()));
    assert!(base.join("events").is_dir());
    // Null is memory only.
    assert!(dspi_event_log_set_dir(h, null()));
    assert!(!dspi_event_log_set_dir(null_mut(), null()));
    dspi_core_free(h);
    let _ = std::fs::remove_dir_all(&base);
}
