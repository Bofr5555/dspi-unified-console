//! WP Q2b tests: failure values on null and short buffers, the old signatures
//! in the generated header, and the functions over the demo device.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::types::DspState;

fn handle() -> *mut FfiCore {
    let core = DspiCore::new();
    let state = DspState::from(&core.get_state());
    Box::into_raw(Box::new(FfiCore {
        core,
        state: Mutex::new(Box::new(state)),
    }))
}

fn free(h: *mut FfiCore) {
    drop(unsafe { Box::from_raw(h) });
}

fn demo() -> *mut FfiCore {
    let h = handle();
    assert!(dspi_open_demo(h));
    h
}

/// Call a text function with a large buffer and give back the text.
fn read_text(f: impl FnOnce(*mut c_char, u32) -> u32) -> String {
    let mut buf = vec![0u8; 256 * 1024];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(
        needed > 0 && needed as usize <= buf.len(),
        "needed {needed}"
    );
    assert_eq!(buf[needed as usize - 1], 0);
    String::from_utf8(buf[..needed as usize - 1].to_vec()).unwrap()
}

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    serde_json::from_str(&read_text(f)).unwrap()
}

#[test]
fn wp_q2b_a_null_handle_gives_the_failure_value() {
    let n = null_mut::<FfiCore>();
    let mut b = [7u8; 16];
    let p = b.as_mut_ptr();
    let t = p.cast::<c_char>();

    for v in [
        dspi_set_output_pin(n, 0, 0),
        dspi_fetch_output_pin(n, 0),
        dspi_save_output_config(n),
        dspi_set_output_type(n, 0, 0),
        dspi_set_i2s_bck_pin(n, 0, 0),
        dspi_set_i2s_clock_pin_mode(n, 0),
        dspi_set_mck_enabled(n, false),
        dspi_set_mck_pin(n, 0),
        dspi_set_mck_multiplier(n, 0),
        dspi_set_adat_out_enabled(n, false),
        dspi_set_adat_out_pin(n, 0),
        dspi_set_spdif_rx_pin(n, 0, 0),
        dspi_set_spdif_input_enabled(n, 1, false),
        dspi_set_i2s_rx_pin(n, 0, 0),
        dspi_set_i2s_input_channels(n, 2),
        dspi_set_adat_input_enabled(n, false),
        dspi_set_adat_input_pin(n, 0),
        dspi_set_adat_input_clock_mode(n, 0),
        dspi_test_dac_mute(n),
        dspi_set_crossfeed_pair_map(n, p, 4),
        dspi_core_vreg_op(n, 0, 0),
    ] {
        assert_eq!(v, 0xFF);
    }
    for v in [
        dspi_set_output_config_mode(n, 0),
        dspi_enter_bootloader(n),
        dspi_set_hotplug_callback(n, None, null_mut()),
        dspi_set_input_rate(n, 0),
        dspi_set_i2s_clock_mode(n, 0),
        dspi_set_dac_mute(n, false, false, 0, 0, 0),
        dspi_fetch_dac_mute(n),
        dspi_set_lg_sound_sync(n, false),
        dspi_fetch_ctrl_ifaces(n),
        dspi_set_uart(n, UartConfig::default()),
        dspi_set_i2c(n, I2cConfig::default()),
        dspi_siggen_control(n, 0),
        dspi_reset_buffer_stats(n),
        dspi_cs_fetch(n),
        dspi_open_demo(n),
        dspi_fetch_crossfeed_pair_map(n, p, 4),
        dspi_get_input_source_name(n, 0, t, 16),
        dspi_get_device_name(n, t, 16),
        dspi_set_output_latency(n, 1),
    ] {
        assert!(!v);
    }
    for v in [
        dspi_fetch_input_rate(n),
        dspi_cs_snapshot(n, t, 16),
        dspi_cs_apply(n, null(), t, 16),
        dspi_cs_pin_uses(n, p, p, 1),
        dspi_config_export(n, null(), null(), 0, t, 16),
        dspi_config_import(n, null(), 0, t, 16),
        dspi_filters_export(n, 0, t, 16),
        dspi_filters_inspect(n, null(), 0, t, 16),
        dspi_filters_import(n, null(), null(), 0, t, 16),
        dspi_ext_status(n, t, 16),
        dspi_fetch_core_vreg(n, t, 16),
        dspi_fetch_output_latency(n, t, 16),
        dspi_fetch_output_latency_options(n, t, 16),
        dspi_fetch_clock_diag(n, t, 16),
    ] {
        assert_eq!(v, 0);
    }
    assert_eq!(dspi_fetch_core1_mode(n), -1);
    assert_eq!(dspi_check_core1_conflict(n, 0), -1);
    assert_eq!(dspi_monitor_read(n, 42, t, 16), 42);
    // A failure leaves the caller's buffer alone.
    assert_eq!(b, [7u8; 16]);

    // void functions on a null handle do nothing, or fill in the zero value.
    dspi_poll_hotplug(n);
    dspi_set_notify_callback(n, None, null_mut());
    dspi_rta_set_callback(n, None, null_mut());
    dspi_rta_start(n);
    dspi_monitor_clear(n);
    let mut note = NotifyResult {
        flags: 9,
        filter_channels: 9,
    };
    dspi_process_notifications(n, &mut note);
    assert_eq!(note, NotifyResult::default());
    let mut caps = RtaCapsInfo {
        supported: true,
        ..Default::default()
    };
    dspi_rta_get_caps(n, &mut caps);
    assert!(!caps.supported);
    let mut snapshot = Box::<RtaSnapshot>::default();
    dspi_rta_get_snapshot(n, &mut *snapshot);
}

#[test]
fn wp_q2b_a_null_out_pointer_gives_the_failure_value() {
    let h = demo();
    let n = null_mut::<u8>();
    assert!(!dspi_fetch_input_lock(h, 0, n.cast()));
    assert!(!dspi_fetch_adat_out_status(h, n.cast()));
    assert!(!dspi_fetch_lg_status(h, n.cast()));
    assert!(!dspi_siggen_fetch_caps(h, n.cast()));
    assert!(!dspi_siggen_get_config(h, n.cast()));
    assert!(!dspi_siggen_set_config(h, null()));
    assert!(!dspi_siggen_get_status(h, n.cast()));
    assert!(!dspi_fetch_stat(h, 0, n.cast()));
    assert!(!dspi_fetch_buffer_stats(h, n.cast()));
    assert!(!dspi_fetch_spdif_rx_status(h, n.cast()));
    assert!(!dspi_fetch_spdif_rx_channel_status(h, n.cast()));
    assert!(!dspi_fetch_spdif_rx_pin(h, 0, n.cast()));
    assert!(!dspi_fetch_crossfeed_pair_map(h, n.cast(), 4));
    assert!(!dspi_get_input_source_name(h, 0, n.cast(), 32));
    assert!(!dspi_get_device_name(h, n.cast(), 32));
    assert!(!dspi_set_device_name(h, null()));
    assert!(!dspi_set_input_source_name(h, 0, null()));
    assert_eq!(dspi_firmware_version_matches(h, null()), -1);
    assert_eq!(dspi_monitor_read(h, 5, n.cast(), 64), 5);
    dspi_rta_set_request(h, null());
    dspi_rta_get_caps(h, n.cast());
    dspi_rta_get_snapshot(h, n.cast());
    dspi_process_notifications(h, n.cast());
    dspi_bootloader_counts(n.cast(), n.cast());
    free(h);
}

#[test]
fn wp_q2b_invalid_utf8_is_refused_not_a_panic() {
    let h = demo();
    let bad = [0xFFu8, 0xFE, 0];
    let bad = bad.as_ptr().cast::<c_char>();
    assert!(!dspi_set_device_name(h, bad));
    assert!(!dspi_set_input_source_name(h, 0, bad));
    assert_eq!(dspi_firmware_version_matches(h, bad), -1);
    assert_eq!(read_json(|b, l| dspi_cs_apply(h, bad, b, l))["ok"], false);
    assert_eq!(
        read_json(|b, l| dspi_config_inspect(bad, b, l))["ok"],
        false
    );
    assert_eq!(
        read_json(|b, l| dspi_config_import(h, bad, 0, b, l))["ok"],
        false
    );
    free(h);
}

#[test]
fn wp_q2b_copy_text_never_overruns_a_short_buffer() {
    for text in ["", "a", "abc", "åäö€", "name with spaces"] {
        for len in 0..24u32 {
            let mut buf = vec![0x55u8; 32];
            let needed = copy_text(text, buf.as_mut_ptr().cast(), len);
            assert_eq!(needed as usize, text.len() + 1);
            if needed <= len {
                assert_eq!(&buf[..text.len()], text.as_bytes());
                assert_eq!(buf[text.len()], 0);
                assert!(buf[needed as usize..].iter().all(|&b| b == 0x55));
            } else {
                assert!(buf.iter().all(|&b| b == 0x55), "{text:?} in {len}");
            }
        }
        assert_eq!(copy_text(text, null_mut(), 64) as usize, text.len() + 1);
    }
}

#[test]
fn wp_q2b_copy_out_cuts_at_a_character_boundary() {
    for len in 1..12u32 {
        let mut buf = vec![0x55u8; 16];
        assert!(copy_out("åäö€", buf.as_mut_ptr().cast(), len));
        let end = buf.iter().position(|&b| b == 0).unwrap();
        assert!(end < len as usize);
        assert!(buf[len as usize..].iter().all(|&b| b == 0x55));
        assert!("åäö€".starts_with(std::str::from_utf8(&buf[..end]).unwrap()));
    }
    assert!(!copy_out("x", null_mut(), 8));
    let mut one = [1u8];
    assert!(!copy_out("x", one.as_mut_ptr().cast(), 0));
    assert_eq!(one, [1]);
}

#[test]
fn wp_q2b_a_short_buffer_writes_nothing_and_reports_the_length() {
    let h = demo();
    let full = read_text(|b, l| dspi_cs_snapshot(h, b, l));
    let mut small = [0x55u8; 4];
    let needed = dspi_cs_snapshot(h, small.as_mut_ptr().cast(), small.len() as u32);
    assert_eq!(needed as usize, full.len() + 1);
    assert_eq!(small, [0x55; 4]);
    free(h);
}

#[test]
fn wp_q2b_hardware_status_bytes() {
    let h = demo();
    assert_eq!(dspi_set_mck_multiplier(h, 1), 0);
    // The device's refusal comes through as its PIN_CONFIG_* byte.
    let refused = dspi_set_i2s_input_channels(h, 3);
    assert!(refused != 0 && refused != 0xFF, "status {refused}");
    assert!(!dspi_set_output_config_mode(h, 9));
    assert!(dspi_set_output_config_mode(h, 1));
    // The fake stalls what it does not model: -1, or a mode.
    assert!((-1..=2).contains(&dspi_fetch_core1_mode(h)));
    assert!((0..=1).contains(&dspi_check_core1_conflict(h, 0)));
    assert_eq!(dspi_check_core1_conflict(h, 0xF0), -1);
    dspi_fetch_dac_mute(h);
    dspi_fetch_ctrl_ifaces(h);
    let mut lock = InputLockStatus::default();
    dspi_fetch_input_lock(h, 0, &mut lock);
    assert!(!dspi_fetch_input_lock(h, 2, &mut lock), "no such input");
    let mut adat = AdatOutStatus::default();
    dspi_fetch_adat_out_status(h, &mut adat);
    let mut lg = LgStatus::default();
    dspi_fetch_lg_status(h, &mut lg);
    free(h);
}

#[test]
fn wp_q2b_errors_map_onto_the_old_conventions() {
    let h = handle();
    assert_eq!(status(h, |_| Ok(())), 0);
    let refused = status(h, |_| Err(DspiError::Refused { status: 2 }));
    assert_eq!(refused, 2, "a refusal is the device's own byte");
    assert_eq!(status(h, |_| Err(DspiError::Disconnected)), 0xFF);
    assert_eq!(status(h, |_| panic!("boom")), 0xFF);
    assert!(flag(h, |_| Ok(())));
    assert!(!flag(h, |_| Err(DspiError::ReadOnly)));
    assert!(!flag(h, |_| panic!("boom")));
    assert_eq!(text(h, null_mut(), 0, |_| panic!("boom")), 0);
    let mut v = 7u32;
    assert!(!fetch_value(h, &mut v, |_| Err(DspiError::Unsupported)));
    assert_eq!(v, 7, "a failed fetch leaves out untouched");
    assert!(fetch_value(h, &mut v, |_| Ok(3)));
    assert_eq!(v, 3);
    free(h);
}

#[test]
fn wp_q2b_disconnected_calls_fail() {
    let h = handle();
    assert_eq!(dspi_set_output_type(h, 0, 0), 0xFF);
    assert_eq!(dspi_fetch_output_pin(h, 0), 0xFF);
    assert_eq!(dspi_fetch_input_rate(h), 0);
    assert_eq!(dspi_fetch_core1_mode(h), -1);
    let mut v = 0u32;
    assert!(!dspi_fetch_stat(h, 0, &mut v));
    assert!(!dspi_cs_fetch(h));
    let reply = read_json(|b, l| dspi_ext_status(h, b, l));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["code"], "disconnected");
    assert!(reply["error"].is_string());
    let reply = read_json(|b, l| dspi_fetch_clock_diag(h, b, l));
    assert_eq!(reply["code"], "disconnected");
    free(h);
}

#[test]
fn wp_q2b_siggen_and_stats_on_the_demo_device() {
    let h = demo();
    // The demo device has no generator: true with `supported` false, and
    // the rest are failures that leave the records alone.
    let mut caps = Box::<SiggenCaps>::default();
    assert!(dspi_siggen_fetch_caps(h, &mut *caps));
    assert!(!caps.supported);
    let cfg = SiggenConfig::default();
    let mut out = cfg;
    assert!(!dspi_siggen_get_config(h, &mut out));
    assert_eq!(out, cfg);
    // The fake stalls what it does not model: either way, `out` holds a
    // record only on true.
    let mut stats = BufferStats::default();
    if !dspi_fetch_buffer_stats(h, &mut stats) {
        assert_eq!(stats, BufferStats::default());
    }
    let mut rx = SpdifRxStatus::default();
    if !dspi_fetch_spdif_rx_status(h, &mut rx) {
        assert_eq!(rx, SpdifRxStatus::default());
    }
    let mut chan = [0x55u8; 25];
    if !dspi_fetch_spdif_rx_channel_status(h, chan.as_mut_ptr()) {
        assert_eq!(chan[0], 0x55);
    }
    assert_eq!(chan[24], 0x55, "at most 24 bytes");
    let mut count = 0xFFFF_FFFFu32;
    if !dspi_fetch_stat(h, crate::types::STAT_CLOCK_HZ, &mut count) {
        assert_eq!(count, 0xFFFF_FFFF);
    }
    let mut pin = 0u8;
    assert!(!dspi_fetch_spdif_rx_pin(h, 9, &mut pin), "no receiver 9");
    assert_eq!(pin, 0);
    dspi_reset_buffer_stats(h);
    free(h);
}

#[test]
fn wp_q2b_control_surfaces_carry_the_error_field() {
    let h = demo();
    assert!(dspi_cs_fetch(h));
    let snapshot = read_json(|b, l| dspi_cs_snapshot(h, b, l));
    assert!(snapshot.get("supported").is_some());
    // The Qt note (PLAN wave 3): a reply's `error` reaches the C caller.
    let op = CString::new("not json").unwrap();
    let reply = read_json(|b, l| dspi_cs_apply(h, op.as_ptr(), b, l));
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().is_some_and(|e| !e.is_empty()));
    let reply = read_json(|b, l| dspi_cs_apply(h, null(), b, l));
    assert_eq!(reply["ok"], false);
    let mut pins = [0u8; 4];
    let mut slots = [0u8; 4];
    let total = dspi_cs_pin_uses(h, pins.as_mut_ptr(), slots.as_mut_ptr(), 4);
    assert_eq!(dspi_cs_pin_uses(h, null_mut(), null_mut(), 0), total);
    free(h);
}

#[test]
fn wp_q2b_files_round_trip_on_the_demo_device() {
    let h = demo();
    let name = CString::new("Q2b").unwrap();
    let version = CString::new("1.0").unwrap();
    let doc = read_text(|b, l| dspi_config_export(h, name.as_ptr(), version.as_ptr(), 0, b, l));
    let doc = CString::new(doc).unwrap();
    let info = read_json(|b, l| dspi_config_inspect(doc.as_ptr(), b, l));
    assert_eq!(info["ok"], true);
    assert_eq!(info["name"], "Q2b");
    let applied = read_json(|b, l| dspi_config_import(h, doc.as_ptr(), 0, b, l));
    assert_eq!(applied["ok"], true);
    assert_eq!(
        read_json(|b, l| dspi_config_inspect(null(), b, l))["ok"],
        false
    );

    let filters = read_text(|b, l| dspi_filters_export(h, 2, b, l));
    let filters = CString::new(filters).unwrap();
    let shown = read_json(|b, l| dspi_filters_inspect(h, filters.as_ptr(), 2, b, l));
    assert!(shown.is_object());
    let wires = [0u8];
    let reply = read_json(|b, l| dspi_filters_import(h, filters.as_ptr(), wires.as_ptr(), 1, b, l));
    assert_eq!(reply["ok"], true);
    free(h);
}

#[test]
fn wp_q2b_monitor_cuts_at_whole_lines() {
    let h = demo();
    // A change that notifies, drained into the log.
    assert!(dspi_set_input_rate(h, 1));
    let name = CString::new("x").unwrap();
    assert!(dspi_set_device_name(h, name.as_ptr()));
    for _ in 0..1000 {
        dspi_process_notifications(h, null_mut());
        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut probe = vec![0u8; 4096];
        if dspi_monitor_read(h, 0, probe.as_mut_ptr().cast(), 4096) != 0 {
            break;
        }
    }
    let mut buf = vec![0u8; 64 * 1024];
    let last = dspi_monitor_read(h, 0, buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(last > 0, "the demo device notified");
    let end = buf.iter().position(|&b| b == 0).unwrap();
    let all = std::str::from_utf8(&buf[..end]).unwrap();
    let first = all.split_inclusive('\n').next().unwrap();
    let (ms, text) = first.trim_end().split_once('\t').unwrap();
    assert!(ms.parse::<i64>().unwrap() > 0);
    assert!(!text.is_empty());

    // Room for exactly the first line: only it comes back.
    let mut small = vec![0x55u8; first.len() + 1];
    let id = dspi_monitor_read(h, 0, small.as_mut_ptr().cast(), small.len() as u32);
    assert!(id > 0 && id <= last);
    assert_eq!(&small[..first.len()], first.as_bytes());
    // Lines too long for the buffer are dropped and the id moves past them,
    // so a caller looping until the id stops moving cannot get stuck.
    let mut tiny = [0x55u8; 3];
    assert_eq!(dspi_monitor_read(h, 0, tiny.as_mut_ptr().cast(), 3), last);
    assert_eq!(tiny[0], 0);
    // No buffer: nothing is read, so the id does not move.
    assert_eq!(dspi_monitor_read(h, 0, tiny.as_mut_ptr().cast(), 0), 0);

    dspi_monitor_clear(h);
    assert_eq!(
        dspi_monitor_read(h, last, buf.as_mut_ptr().cast(), buf.len() as u32),
        last
    );
    free(h);
}

extern "C" fn count_wake(user_data: *mut c_void) {
    let n = unsafe { &*user_data.cast::<AtomicU32>() };
    n.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn wp_q2b_rta_on_a_device_without_an_analyser() {
    let h = demo();
    let wakes = Box::leak(Box::new(AtomicU32::new(0)));
    let user = std::ptr::from_ref(wakes).cast_mut().cast::<c_void>();
    dspi_rta_set_callback(h, Some(count_wake), user);
    dspi_rta_start(h);
    let request = RtaRequest {
        mask: 1,
        ..Default::default()
    };
    dspi_rta_set_request(h, &request);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut caps = RtaCapsInfo {
        supported: true,
        ..Default::default()
    };
    dspi_rta_get_caps(h, &mut caps);
    assert!(!caps.supported, "the demo device has no analyser");
    let mut snapshot = Box::<RtaSnapshot>::default();
    dspi_rta_get_snapshot(h, &mut *snapshot);
    dspi_rta_set_callback(h, None, null_mut());
    let after = wakes.load(Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(wakes.load(Ordering::SeqCst), after, "NULL stops the calls");
    free(h);
}

#[test]
fn wp_q2b_notify_callback_wakes_and_clears() {
    let h = handle();
    let wakes = Box::leak(Box::new(AtomicU32::new(0)));
    let user = std::ptr::from_ref(wakes).cast_mut().cast::<c_void>();
    dspi_set_notify_callback(h, Some(count_wake), user);
    assert!(dspi_open_demo(h));
    let name = CString::new("q2b").unwrap();
    assert!(dspi_set_device_name(h, name.as_ptr()));
    for _ in 0..1000 {
        if wakes.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(wakes.load(Ordering::SeqCst) > 0, "the listener ran");
    let mut r = NotifyResult::default();
    dspi_process_notifications(h, &mut r);
    dspi_set_notify_callback(h, None, null_mut());
    free(h);
}

extern "C" fn record_hotplug(event: u8, serial: *const c_char, user_data: *mut c_void) {
    let seen = unsafe { &*user_data.cast::<Mutex<Vec<(u8, String)>>>() };
    let serial = unsafe { CStr::from_ptr(serial) }
        .to_str()
        .unwrap()
        .to_owned();
    seen.lock().unwrap().push((event, serial));
}

#[test]
fn wp_q2b_hotplug_callback_gets_nul_terminated_serials() {
    let seen: Mutex<Vec<(u8, String)>> = Mutex::new(Vec::new());
    let wrapped = CHotplug {
        callback: record_hotplug,
        user_data: std::ptr::from_ref(&seen).cast_mut().cast(),
    };
    wrapped.on_arrived("SER1".to_owned());
    wrapped.on_departed("SE\0R2".to_owned());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(0, "SER1".to_owned()), (1, "SE".to_owned())]
    );
    let h = handle();
    assert!(dspi_set_hotplug_callback(
        h,
        Some(record_hotplug),
        wrapped.user_data
    ));
    assert!(dspi_set_hotplug_callback(h, None, null_mut()));
    dspi_poll_hotplug(h);
    free(h);
}

#[test]
fn wp_q2b_extension_channel_replies() {
    let h = demo();
    let status = read_json(|b, l| dspi_ext_status(h, b, l));
    assert_eq!(status["ok"], true);
    assert!(status["supported"].is_array());
    assert!(status["hasPairMap"].is_boolean());
    let has_map = status["hasPairMap"] == true;
    let has_vreg = status["hasCoreVreg"] == true;

    let mut map = [9u8; 4];
    assert!(
        !dspi_fetch_crossfeed_pair_map(h, map.as_mut_ptr(), 3),
        "short"
    );
    assert_eq!(map, [9u8; 4]);
    assert_eq!(
        dspi_fetch_crossfeed_pair_map(h, map.as_mut_ptr(), 4),
        has_map
    );
    // Too short a map never gets as far as a device status.
    assert_eq!(dspi_set_crossfeed_pair_map(h, map.as_ptr(), 2), 0xFF);
    assert_eq!(dspi_set_crossfeed_pair_map(h, null(), 4), 0xFF);

    let vreg = read_json(|b, l| dspi_fetch_core_vreg(h, b, l));
    assert_eq!(vreg["ok"], has_vreg);
    if has_vreg {
        assert!(vreg["millivolts"].is_number());
    } else {
        assert_eq!(vreg["code"], "unsupported");
    }
    assert_eq!(dspi_core_vreg_op(h, 99, 0), 0xFF);
    free(h);
}

#[test]
fn wp_q2b_names_round_trip() {
    let h = demo();
    let name = CString::new("Livingroom").unwrap();
    if dspi_set_device_name(h, name.as_ptr()) {
        let mut buf = [0x55u8; 32];
        assert!(dspi_get_device_name(h, buf.as_mut_ptr().cast(), 32));
        assert_eq!(&buf[..11], b"Livingroom\0");
        // A short buffer cuts the name.
        let mut short = [0x55u8; 5];
        assert!(dspi_get_device_name(h, short.as_mut_ptr().cast(), 5));
        assert_eq!(&short, b"Livi\0");
    }
    let long = CString::new("x".repeat(40)).unwrap();
    assert!(!dspi_set_device_name(h, long.as_ptr()));
    let source = CString::new("Turntable").unwrap();
    if dspi_set_input_source_name(h, 0, source.as_ptr()) {
        let mut buf = [0u8; 32];
        assert!(dspi_get_input_source_name(
            h,
            0,
            buf.as_mut_ptr().cast(),
            32
        ));
        assert_eq!(&buf[..10], b"Turntable\0");
    }
    free(h);
}

#[test]
fn wp_q2b_diagnostics_replies() {
    let h = demo();
    let latency = read_json(|b, l| dspi_fetch_output_latency(h, b, l));
    if latency["ok"] == true {
        assert!(latency["fields"].is_array());
        let options = read_json(|b, l| dspi_fetch_output_latency_options(h, b, l));
        assert!(options["value"].is_array());
    } else {
        assert_eq!(latency["code"], "unsupported");
    }
    let diag = read_json(|b, l| dspi_fetch_clock_diag(h, b, l));
    assert_eq!(diag["ok"], true);
    assert!(diag["available"].is_boolean());
    assert!(!dspi_set_output_latency(h, 0));
    free(h);
}

#[test]
fn wp_q2b_firmware_functions() {
    let info = read_json(|b, l| dspi_uf2_inspect(null(), 0, b, l));
    assert_eq!(info["ok"], true);
    assert_eq!(info["valid"], false);
    assert!(info["problem"].is_string());
    let junk = [0u8; 600];
    let info = read_json(|b, l| dspi_uf2_inspect(junk.as_ptr(), 600, b, l));
    assert_eq!(info["valid"], false);

    let drives = read_json(|b, l| dspi_bootloader_drives(b, l));
    assert!(drives["value"].is_array());

    // An invalid image is refused before the drive is looked at.
    let path = CString::new("/nonexistent/drive").unwrap();
    let reply = read_json(|b, l| dspi_flash_to_drive(junk.as_ptr(), 600, path.as_ptr(), 0, b, l));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["code"], "invalid");
    let reply = read_json(|b, l| dspi_flash_to_drive(null(), 0, null(), 0, b, l));
    assert_eq!(reply["ok"], false);

    let (mut a, mut b) = (9u32, 9u32);
    dspi_bootloader_counts(&mut a, &mut b);
    dspi_bootloader_counts(&mut a, null_mut());
    assert!(a < 9 && b < 9);

    let h = handle();
    let version = CString::new("1.1.6").unwrap();
    assert_eq!(dspi_firmware_version_matches(h, version.as_ptr()), -1);
    assert!(dspi_open_demo(h));
    assert!(dspi_firmware_version_matches(h, version.as_ptr()) >= 0);
    free(h);
}

#[test]
fn wp_q2b_a_misaligned_pointer_gives_the_failure_value() {
    let h = demo();
    let mut words = [0x5555_5555u32; 4];
    let bytes = words.as_mut_ptr().cast::<u8>();
    // One byte in: misaligned for everything wider than a byte.
    let odd = bytes.wrapping_add(1);
    assert!(!dspi_fetch_stat(h, 0, odd.cast()));
    assert!(!dspi_fetch_adat_out_status(h, odd.cast()));
    assert!(!dspi_siggen_set_config(
        h,
        odd.cast::<SiggenConfig>().cast_const()
    ));
    dspi_process_notifications(h, odd.cast());
    dspi_rta_set_request(h, odd.cast::<RtaRequest>().cast_const());
    dspi_rta_get_caps(h, odd.cast());
    dspi_rta_get_snapshot(h, odd.cast());
    dspi_bootloader_counts(odd.cast(), odd.cast());
    assert_eq!(words, [0x5555_5555u32; 4]);
    free(h);
}

#[test]
fn wp_q2b_no_reply_record_has_a_field_named_ok() {
    use dspi_core::{
        BootloaderDrive, ClockDiag, CoreVregInfo, ExtStatus, OutputLatency, OutputLatencyOption,
        Uf2Info,
    };
    let records = [
        serde_json::to_value(ExtStatus::default()).unwrap(),
        serde_json::to_value(CoreVregInfo::default()).unwrap(),
        serde_json::to_value(OutputLatency::default()).unwrap(),
        serde_json::to_value(OutputLatencyOption::default()).unwrap(),
        serde_json::to_value(ClockDiag::default()).unwrap(),
        serde_json::to_value(Uf2Info::default()).unwrap(),
        serde_json::to_value(BootloaderDrive::default()).unwrap(),
    ];
    for record in records {
        let fields = record.as_object().unwrap();
        assert!(!fields.is_empty());
        assert!(!fields.contains_key("ok"), "{record}");
    }
}

#[test]
fn wp_q2b_json_reply_shapes() {
    let r: Result<Vec<u8>> = Ok(vec![1, 2]);
    assert_eq!(json_reply(r), r#"{"ok":true,"value":[1,2]}"#);
    let r: Result<serde_json::Value> = Ok(serde_json::json!({}));
    assert_eq!(json_reply(r), r#"{"ok":true}"#);
    let r: Result<u8> = Err(DspiError::Unsupported);
    let v: serde_json::Value = serde_json::from_str(&json_reply(r)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["code"], "unsupported");
}

// ── The generated header ─────────────────────────────────────────────

/// `name -> prototype`, whitespace normalised, of every function in a header.
fn prototypes(header: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rest = header;
    while let Some(i) = rest.find("dspi_") {
        let after = &rest[i..];
        let Some(open) = after.find(|c: char| !(c.is_alphanumeric() || c == '_')) else {
            break;
        };
        let name = &after[..open];
        if !after[open..].starts_with('(') {
            rest = &after[open..];
            continue;
        }
        let start = rest[..i].rfind(['\n', ';', '/']).map_or(0, |s| s + 1);
        let Some(close) = after.find(");") else { break };
        let proto = rest[start..i].to_owned() + &after[..close + 1];
        let proto = proto.split_whitespace().collect::<Vec<_>>().join(" ");
        if proto.split_whitespace().count() >= 2 && !proto.starts_with('*') {
            out.insert(name.to_owned(), proto);
        }
        rest = &after[close..];
    }
    out
}

/// The old functions Q2b took over, with the old signature (old header).
const OLD_FUNCTIONS: &[&str] = &[
    "dspi_set_output_pin",
    "dspi_fetch_output_pin",
    "dspi_set_output_config_mode",
    "dspi_save_output_config",
    "dspi_enter_bootloader",
    "dspi_fetch_core1_mode",
    "dspi_check_core1_conflict",
    "dspi_set_hotplug_callback",
    "dspi_poll_hotplug",
    "dspi_set_output_type",
    "dspi_set_i2s_bck_pin",
    "dspi_set_i2s_clock_pin_mode",
    "dspi_set_mck_enabled",
    "dspi_set_mck_pin",
    "dspi_set_mck_multiplier",
    "dspi_set_adat_out_enabled",
    "dspi_set_adat_out_pin",
    "dspi_set_spdif_rx_pin",
    "dspi_set_spdif_input_enabled",
    "dspi_set_i2s_rx_pin",
    "dspi_set_i2s_input_channels",
    "dspi_set_adat_input_enabled",
    "dspi_set_adat_input_pin",
    "dspi_set_adat_input_clock_mode",
    "dspi_set_input_rate",
    "dspi_fetch_input_rate",
    "dspi_set_i2s_clock_mode",
    "dspi_fetch_input_lock",
    "dspi_fetch_adat_out_status",
    "dspi_set_dac_mute",
    "dspi_fetch_dac_mute",
    "dspi_test_dac_mute",
    "dspi_set_lg_sound_sync",
    "dspi_fetch_lg_status",
    "dspi_fetch_ctrl_ifaces",
    "dspi_set_uart",
    "dspi_set_i2c",
    "dspi_set_notify_callback",
    "dspi_process_notifications",
    "dspi_rta_set_callback",
    "dspi_rta_start",
    "dspi_rta_set_request",
    "dspi_rta_get_caps",
    "dspi_rta_get_snapshot",
    "dspi_siggen_fetch_caps",
    "dspi_siggen_get_config",
    "dspi_siggen_set_config",
    "dspi_siggen_control",
    "dspi_siggen_get_status",
    "dspi_fetch_stat",
    "dspi_fetch_buffer_stats",
    "dspi_reset_buffer_stats",
    "dspi_fetch_spdif_rx_status",
    "dspi_fetch_spdif_rx_channel_status",
    "dspi_fetch_spdif_rx_pin",
    "dspi_monitor_read",
    "dspi_monitor_clear",
    "dspi_config_export",
    "dspi_config_inspect",
    "dspi_config_import",
    "dspi_filters_export",
    "dspi_filters_inspect",
    "dspi_filters_import",
    "dspi_cs_fetch",
    "dspi_cs_snapshot",
    "dspi_cs_apply",
    "dspi_cs_pin_uses",
    "dspi_bootloader_counts",
];

#[test]
fn wp_q2b_old_functions_keep_their_signatures_in_the_header() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let new = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
    let old = std::fs::read_to_string(dir.join("tests/data/dspi_core_old.h")).unwrap();
    let (new, old) = (prototypes(&new), prototypes(&old));
    for name in OLD_FUNCTIONS {
        assert!(new.contains_key(*name), "{name} missing");
        // These two take the typedef's function type spelled out, so that
        // NULL is defined on the Rust side: the same type to a C compiler.
        let want = match *name {
            "dspi_set_hotplug_callback" => Some(
                "bool dspi_set_hotplug_callback(struct FfiCore *core, void (*callback)(uint8_t \
                 event, const char *serial, void *user_data), void *user_data)"
                    .to_owned(),
            ),
            "dspi_set_notify_callback" => Some(
                "void dspi_set_notify_callback(struct FfiCore *core, void \
                 (*callback)(void *user_data), void *user_data)"
                    .to_owned(),
            ),
            _ => old.get(*name).cloned(),
        };
        assert_eq!(new.get(*name), want.as_ref(), "{name}");
    }
}

#[test]
fn wp_q2b_added_functions_are_in_the_header() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let new = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
    let new = prototypes(&new);
    for name in [
        "dspi_open_demo",
        "dspi_ext_status",
        "dspi_fetch_crossfeed_pair_map",
        "dspi_set_crossfeed_pair_map",
        "dspi_fetch_core_vreg",
        "dspi_core_vreg_op",
        "dspi_get_input_source_name",
        "dspi_set_input_source_name",
        "dspi_get_device_name",
        "dspi_set_device_name",
        "dspi_fetch_output_latency",
        "dspi_fetch_output_latency_options",
        "dspi_set_output_latency",
        "dspi_fetch_clock_diag",
        "dspi_uf2_inspect",
        "dspi_bootloader_drives",
        "dspi_flash_to_drive",
        "dspi_firmware_version_matches",
    ] {
        assert!(new.contains_key(name), "{name} missing");
    }
}

// -- WP CG5 --
#[test]
fn wp_cg5_ownership_and_take_control_are_exported() {
    let mut o = RtaOwnership {
        following: true,
        ..Default::default()
    };
    dspi_rta_get_ownership(null_mut(), &mut o);
    assert!(!o.following, "the null handle reads as driving");
    dspi_rta_take_control(null_mut());
    let h = handle();
    dspi_rta_get_ownership(h, null_mut());
    o.following = true;
    dspi_rta_get_ownership(h, &mut o);
    assert_eq!(o, RtaOwnership::default());
    dspi_rta_take_control(h);
    free(h);
    let header = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/dspi_core.h"),
    )
    .unwrap();
    assert!(
        header.contains(
            "void dspi_rta_get_ownership(struct FfiCore *core, struct RtaOwnership *out);"
        )
    );
    assert!(header.contains("void dspi_rta_take_control(struct FfiCore *core);"));
}
