//! WP Q2a tests: failure values for null handles, null pointers, bad strings
//! and short buffers; the status conventions; the state pointer; the
//! signatures against the old header. Device behaviour itself is dspi-core's
//! to test: here the demo device only shows that the calls reach it.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use proptest::prelude::*;

use super::*;
use crate::types::{FLASH_OK, PRESET_ERR_INVALID_SLOT, PRESET_OK};

const NULL: *mut FfiCore = ptr::null_mut();

/// A handle freed at the end of the test.
struct Handle(*mut FfiCore);

impl Handle {
    fn new() -> Self {
        let h = dspi_core_new();
        assert!(!h.is_null());
        Self(h)
    }

    /// With the demo device open.
    fn demo() -> Self {
        let h = Self::new();
        with_core(h.0, (), |c| c.core.open_demo().unwrap());
        h
    }

    fn state(&self) -> DspState {
        // SAFETY: the pointer is valid until the handle is freed.
        unsafe { *dspi_get_state(self.0) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        dspi_core_free(self.0);
    }
}

fn band(freq: f32) -> FilterParams {
    FilterParams {
        filter_type: 1,
        freq,
        q: 1.0,
        qp: 0.0,
        gain: 6.0,
        bypass: false,
    }
}

#[test]
fn wp_q2a_null_handle_gives_the_failure_value() {
    assert!(dspi_get_state(NULL).is_null());
    assert_eq!(dspi_scan_devices(NULL, ptr::null_mut(), 4), 0);
    assert_eq!(dspi_preamp_max_db(NULL), 0.0);
    assert_eq!(dspi_inaccessible_devices(NULL), 0);
    assert!(!dspi_select_device(NULL, c"x".as_ptr()));
    dspi_disconnect(NULL);
    assert!(!dspi_is_connected(NULL));
    assert!(!dspi_fetch_all(NULL));
    assert!(!dspi_refresh_params(NULL));
    assert!(!dspi_apply_all_params(NULL));
    assert!(!dspi_fetch_status(NULL, ptr::null_mut()));
    assert_eq!(dspi_max_delay_ms(NULL), 0.0);
    assert!(!dspi_clear_clips(NULL));
    assert!(!dspi_set_filter(NULL, 0, 0, band(100.0)));
    assert!(!dspi_set_crossover(NULL, 0, 0, band(100.0)));
    assert!(!dspi_set_band_bypass(NULL, 0, 0, true));
    assert!(!dspi_set_master_volume(NULL, -3.0));
    assert!(!dspi_set_matrix_route(NULL, 0, 0, true, 0.0, false));
    assert_eq!(dspi_set_output_enable(NULL, 0, true), -1);
    assert!(!dspi_set_limiter_param(NULL, 0, 0, 0.0));
    let mut meter = [0f32; MAX_OUTPUTS];
    assert!(!dspi_fetch_limiter_meter(NULL, meter.as_mut_ptr()));
    assert!(!dspi_fetch_subharm_meter(NULL, meter.as_mut_ptr()));
    assert!(!dspi_fetch_subharm_headroom(NULL, meter.as_mut_ptr()));
    assert!(!dspi_set_channel_name(NULL, 0, c"x".as_ptr()));
    let mut buf = [1 as c_char; 8];
    assert!(!dspi_get_channel_name(NULL, 0, buf.as_mut_ptr(), 8));
    assert!(!dspi_get_preset_name(NULL, 0, buf.as_mut_ptr(), 8));
    assert_eq!(buf, [1 as c_char; 8], "nothing is written");
    assert_eq!(dspi_save_master_volume(NULL), 0xFF);
    assert_eq!(dspi_save_preset(NULL, 0), 0xFF);
    assert_eq!(dspi_load_preset(NULL, 0), 0xFF);
    assert_eq!(dspi_copy_preset(NULL, 0, 1), 0xFF);
    assert_eq!(dspi_delete_preset(NULL, 0), 0xFF);
    assert_eq!(dspi_save_params(NULL), FLASH_ERR_WRITE);
    assert_eq!(dspi_load_params(NULL), FLASH_ERR_WRITE);
    assert_eq!(dspi_factory_reset(NULL), FLASH_ERR_WRITE);
    dspi_capture_baseline(NULL);
    assert!(!dspi_preset_dirty(NULL));
    assert_eq!(dspi_preset_changes(NULL, ptr::null_mut(), 0), 0);
    dspi_core_free(NULL);
}

#[test]
fn wp_q2a_null_arguments_give_the_failure_value() {
    let h = Handle::demo();
    assert!(!dspi_select_device(h.0, ptr::null()));
    assert!(!dspi_set_channel_name(h.0, 0, ptr::null()));
    assert!(!dspi_set_preset_name(h.0, 0, ptr::null()));
    assert!(!dspi_get_channel_name(h.0, 0, ptr::null_mut(), 8));
    assert!(!dspi_get_preset_name(h.0, 0, ptr::null_mut(), 8));
    assert!(!dspi_get_preset_directory(h.0, ptr::null_mut()));
    assert!(!dspi_fetch_upmix_status(h.0, ptr::null_mut()));
    assert!(!dspi_fetch_limiter_meter(h.0, ptr::null_mut()));
    assert!(!dspi_fetch_subharm_meter(h.0, ptr::null_mut()));
    assert!(!dspi_fetch_subharm_headroom(h.0, ptr::null_mut()));
    assert!(!dspi_compute_magnitude_curve(
        ptr::null(),
        0,
        ptr::null_mut()
    ));
    assert!(!dspi_compute_phase_curve(
        ptr::null(),
        0,
        false,
        ptr::null_mut()
    ));
    // These two take an optional output, as the old ABI's did.
    assert!(dspi_fetch_status(h.0, ptr::null_mut()));
    assert_eq!(dspi_scan_devices(h.0, ptr::null_mut(), 0), 0);
}

#[test]
fn wp_q2a_bad_utf8_is_refused() {
    let h = Handle::demo();
    let bad = [0xFFu8, 0xFE, 0];
    let bad = bad.as_ptr().cast::<c_char>();
    assert!(!dspi_select_device(h.0, bad));
    assert!(!dspi_set_channel_name(h.0, 0, bad));
    assert!(!dspi_set_preset_name(h.0, 0, bad));
}

#[test]
fn wp_q2a_disconnected_calls_use_the_old_conventions() {
    let h = Handle::new();
    assert!(!dspi_is_connected(h.0));
    assert!(!dspi_fetch_all(h.0));
    assert!(!dspi_set_master_volume(h.0, -3.0));
    assert_eq!(dspi_set_output_enable(h.0, 0, true), -1);
    assert_eq!(dspi_save_preset(h.0, 0), 0xFF);
    assert_eq!(dspi_save_master_volume(h.0), 0xFF);
    assert_eq!(dspi_save_params(h.0), FLASH_ERR_WRITE);
    assert_eq!(dspi_factory_reset(h.0), FLASH_ERR_WRITE);
    assert!(!dspi_preset_dirty(h.0));
    assert_eq!(dspi_preamp_max_db(h.0), 18.0);
}

#[test]
fn wp_q2a_status_passes_the_device_status_through() {
    assert_eq!(status_of(Ok(()), 0xFF), 0);
    assert_eq!(status_of(Err(DspiError::Refused { status: 3 }), 0xFF), 3);
    assert_eq!(status_of(Err(DspiError::Disconnected), 0xFF), 0xFF);
    assert_eq!(
        status_of(Err(DspiError::ReadOnly), FLASH_ERR_WRITE),
        FLASH_ERR_WRITE
    );
    let h = Handle::demo();
    assert_eq!(dspi_save_preset(h.0, 99), PRESET_ERR_INVALID_SLOT);
    assert_eq!(dspi_load_preset(h.0, 99), PRESET_ERR_INVALID_SLOT);
}

#[test]
fn wp_q2a_state_pointer_is_stable_and_refreshed_by_each_call() {
    let h = Handle::demo();
    let p = dspi_get_state(h.0);
    assert!(!p.is_null());
    assert!(dspi_set_master_volume(h.0, -7.0));
    let q = dspi_get_state(h.0);
    assert_eq!(p, q, "the address does not move");
    // SAFETY: valid until the handle is freed; this test has one thread.
    assert_eq!(unsafe { (*q).master_volume_db }, -7.0);
    assert!(dspi_set_master_volume(h.0, -9.0));
    // SAFETY: as above. The old pointer reads the last refresh.
    assert_eq!(unsafe { (*p).master_volume_db }, -7.0);
    assert_eq!(h.state().master_volume_db, -9.0);
}

#[test]
fn wp_q2a_state_resets_on_disconnect() {
    let h = Handle::demo();
    assert_ne!(h.state().num_channels, 0);
    assert!(dspi_is_connected(h.0));
    dspi_disconnect(h.0);
    assert!(!dspi_is_connected(h.0));
    assert!(!dspi_fetch_status(h.0, ptr::null_mut()));
}

/// Counts the callbacks that run once `dspi_core_free` has begun.
#[derive(Default)]
struct Late {
    freeing: AtomicBool,
    calls: AtomicU32,
}

impl Late {
    fn fired(&self) {
        if self.freeing.load(Ordering::SeqCst) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl dspi_core::NotifyListener for Late {
    fn on_notifications(&self) {
        self.fired();
    }
}

impl dspi_core::RtaListener for Late {
    fn on_frame(&self) {
        self.fired();
    }
}

impl dspi_core::DeviceEvents for Late {
    fn on_arrived(&self, _: String) {
        self.fired();
    }

    fn on_departed(&self, _: String) {
        self.fired();
    }
}

/// Closing a device with a running analyser worker fires the RTA listener: with
/// the caller's `user_data`, about to be freed, if it were still set.
#[test]
fn wp_q2a_no_callback_fires_after_free_begins() {
    let late = Arc::new(Late::default());
    let h = dspi_core_new();
    with_core(h, (), |c| {
        c.core.open_demo().unwrap();
        c.core.set_notify_listener(Some(late.clone()));
        c.core.rta_set_listener(Some(late.clone()));
        c.core.set_hotplug_callback(Some(late.clone()));
        c.core.rta_start();
    });
    // The worker's own first wake (the demo has no analyser, so it is its last)
    // comes before `freeing`: only the teardown can count.
    std::thread::sleep(std::time::Duration::from_millis(300));
    late.freeing.store(true, Ordering::SeqCst);
    dspi_core_free(h);
    assert_eq!(late.calls.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(&late), 1, "the core let go of them all");
}

#[test]
fn wp_q2a_setters_reach_the_demo_device() {
    let h = Handle::demo();
    assert!(dspi_fetch_all(h.0));
    let mut st = SystemStatus::from(&r::SystemStatus::default());
    assert!(dspi_fetch_status(h.0, &raw mut st));

    assert!(dspi_set_filter(h.0, 0, 2, band(1234.0)));
    assert_eq!(h.state().filters[0][2].freq, 1234.0);
    assert!(dspi_set_band_bypass(h.0, 0, 2, true));
    assert!(h.state().filters[0][2].bypass);

    let out = h.state().num_output_channels - 1;
    assert_eq!(dspi_set_output_enable(h.0, 0, false), 0);
    assert!(!h.state().output_enabled[0]);
    assert_eq!(dspi_set_output_enable(h.0, 0, true), 1);
    assert!(dspi_set_output_gain(h.0, 0, -2.0));
    assert_eq!(h.state().output_gain_db[0], -2.0);
    assert!(!dspi_set_output_gain(h.0, out + 40, -2.0));

    assert!(dspi_set_channel_name(h.0, 0, c"Left".as_ptr()));
    let mut name = [0 as c_char; 32];
    assert!(dspi_get_channel_name(h.0, 0, name.as_mut_ptr(), 32));
    // SAFETY: copy_out terminates the buffer.
    assert_eq!(unsafe { CStr::from_ptr(name.as_ptr()) }, c"Left");
    let mut short = [0x55 as c_char; 3];
    assert!(dspi_get_channel_name(h.0, 0, short.as_mut_ptr(), 3));
    // SAFETY: as above.
    assert_eq!(unsafe { CStr::from_ptr(short.as_ptr()) }, c"Le");
}

#[test]
fn wp_q2a_meters_fill_every_output() {
    let h = Handle::demo();
    let mut meter = [f32::NAN; MAX_OUTPUTS + 1];
    if dspi_fetch_limiter_meter(h.0, meter.as_mut_ptr()) {
        assert!(meter[..MAX_OUTPUTS].iter().all(|v| !v.is_nan()));
    }
    assert!(meter[MAX_OUTPUTS].is_nan(), "one past the end is untouched");
    assert!(fetch_floats(h.0, meter.as_mut_ptr(), |_| Ok(vec![1.0; 2])));
    assert_eq!(meter[1], 1.0);
    assert_eq!(meter[2], 0.0);
    assert!(fetch_floats(h.0, meter.as_mut_ptr(), |_| Ok(vec![2.0; 40])));
    assert_eq!(meter[MAX_OUTPUTS - 1], 2.0);
    assert!(meter[MAX_OUTPUTS].is_nan());
    assert!(!fetch_floats(h.0, meter.as_mut_ptr(), |_| Err(
        DspiError::Disconnected
    )));
}

#[test]
fn wp_q2a_presets_round_trip_and_report_changes() {
    let h = Handle::demo();
    assert_eq!(dspi_save_preset(h.0, 1), PRESET_OK);
    assert!(dspi_set_preset_name(h.0, 1, c"Room".as_ptr()));
    let mut buf = [0 as c_char; 32];
    assert!(dspi_get_preset_name(h.0, 1, buf.as_mut_ptr(), 32));
    // SAFETY: copy_out terminates the buffer.
    assert_eq!(unsafe { CStr::from_ptr(buf.as_ptr()) }, c"Room");
    let mut dir = PresetDirectory::from(&r::PresetDirectory::default());
    assert!(dspi_get_preset_directory(h.0, &raw mut dir));
    assert_eq!(dir.occupied_mask & 0b10, 0b10);

    dspi_capture_baseline(h.0);
    assert!(!dspi_preset_dirty(h.0));
    assert_eq!(
        dspi_preset_changes(h.0, ptr::null_mut(), 0),
        3,
        "[] and NUL"
    );
    assert!(dspi_set_input_preamp(h.0, 0, -3.0));
    assert!(dspi_preset_dirty(h.0));

    // The size-then-fill protocol: a short buffer is left alone.
    let needed = dspi_preset_changes(h.0, ptr::null_mut(), 0);
    assert!(needed > 3);
    let mut short = [0x55 as c_char; 2];
    assert_eq!(dspi_preset_changes(h.0, short.as_mut_ptr(), 2), needed);
    assert_eq!(short, [0x55 as c_char; 2]);
    let mut text = vec![0 as c_char; needed as usize];
    assert_eq!(dspi_preset_changes(h.0, text.as_mut_ptr(), needed), needed);
    // SAFETY: copy_out terminates the buffer.
    let text = unsafe { CStr::from_ptr(text.as_ptr()) }.to_str().unwrap();
    let list: Vec<String> = serde_json::from_str(text).unwrap();
    assert!(!list.is_empty());

    assert_eq!(dspi_load_preset(h.0, 1), PRESET_OK);
    assert_eq!(dspi_save_params(h.0), FLASH_OK);
    assert_eq!(dspi_delete_preset(h.0, 1), PRESET_OK);
}

#[test]
fn wp_q2a_curves_match_dspi_core() {
    let chain = [band(1000.0), band(2000.0)];
    let core_chain: Vec<r::FilterParams> = chain.iter().map(r::FilterParams::from).collect();
    assert_eq!(
        dspi_compute_response(chain.as_ptr(), 2, 1000.0),
        dspi_core::compute_response(core_chain.clone(), 1000.0)
    );
    assert_eq!(dspi_compute_response(ptr::null(), 2, 1000.0), 0.0);
    assert_eq!(dspi_compute_response(chain.as_ptr(), 0, 1000.0), 0.0);

    let mut out = [f64::NAN; MAGNITUDE_POINTS + 1];
    assert!(dspi_compute_magnitude_curve(
        chain.as_ptr(),
        2,
        out.as_mut_ptr()
    ));
    assert_eq!(
        out[..MAGNITUDE_POINTS],
        dspi_core::compute_magnitude_curve(core_chain.clone())[..]
    );
    assert!(out[MAGNITUDE_POINTS].is_nan());
    assert!(dspi_compute_phase_curve(
        chain.as_ptr(),
        2,
        true,
        out.as_mut_ptr()
    ));
    assert_eq!(
        out[..MAGNITUDE_POINTS],
        dspi_core::compute_phase_curve(core_chain, true)[..]
    );
    // A null chain is an empty one: flat.
    assert!(dspi_compute_magnitude_curve(
        ptr::null(),
        5,
        out.as_mut_ptr()
    ));
    assert!(out[..MAGNITUDE_POINTS].iter().all(|v| *v == 0.0));
    // A curve of the wrong length neither overruns nor leaves garbage.
    assert!(write_curve(vec![1.0; 3], out.as_mut_ptr()));
    assert_eq!((out[2], out[3]), (1.0, 0.0));
    assert!(write_curve(vec![2.0; 500], out.as_mut_ptr()));
    assert!(out[MAGNITUDE_POINTS].is_nan());
}

#[test]
fn wp_q2a_devices_are_written_up_to_the_limit() {
    let devices: Vec<r::DeviceInfo> = (0..3)
        .map(|i| r::DeviceInfo {
            serial: format!("S{i}"),
            location_id: i,
        })
        .collect();
    let mut out = [DeviceInfo::default(); 3];
    assert_eq!(write_devices(&devices, out.as_mut_ptr(), 2), 2);
    assert_eq!(out[1].location_id, 1);
    assert_eq!(out[2].serial_len, 0, "past max_devices is untouched");
    assert_eq!(write_devices(&devices, ptr::null_mut(), 2), 2);
    assert_eq!(write_devices(&devices, out.as_mut_ptr(), 9), 3);
    assert_eq!(out[2].serial_len, 2);
}

/// `name(args)` -> the prototype, comments gone and whitespace collapsed, for
/// each function in a cbindgen header.
fn prototypes(h: &str) -> BTreeMap<String, String> {
    let mut s = String::new();
    let mut rest = h;
    while let Some(i) = rest.find("/*") {
        s.push_str(&rest[..i]);
        rest = rest[i..].find("*/").map_or("", |j| &rest[i + j + 2..]);
    }
    s.push_str(rest);
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = BTreeMap::new();
    for proto in flat.split(';') {
        let proto = proto.rsplit('}').next().unwrap_or("").trim();
        let Some(open) = proto.find("dspi_") else {
            continue;
        };
        let Some(end) = proto[open..].find('(') else {
            continue;
        };
        let name = proto[open..open + end].to_owned();
        out.insert(name, proto.to_owned());
    }
    out
}

#[test]
fn wp_q2a_signatures_match_the_old_header() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let old = std::fs::read_to_string(manifest.join("tests/data/dspi_core_old.h")).unwrap();
    let new = std::fs::read_to_string(manifest.join("include/dspi_core.h")).unwrap();
    let (old, new) = (prototypes(&old), prototypes(&new));
    assert!(old.len() > 140, "the parser found {} functions", old.len());
    let source = include_str!("../fns_device.rs");
    let mine: Vec<&str> = source
        .split("pub extern \"C\" fn ")
        .skip(1)
        .map(|s| s.split('(').next().unwrap())
        .collect();
    assert!(mine.len() > 70, "{}", mine.len());
    for name in mine {
        let o = old.get(name).unwrap_or_else(|| panic!("{name}: not old"));
        let n = new.get(name).unwrap_or_else(|| panic!("{name}: no header"));
        assert_eq!(o, n, "{name}");
    }
}

proptest! {
    /// However short the buffer, `copy_out` stays inside it, terminates it and
    /// leaves valid UTF-8 that is a prefix of the string.
    #[test]
    fn proptest_wp_q2a_copy_out_stays_in_the_buffer(s in "[^\\x00]{0,64}", len in 0u32..80) {
        let mut buf = vec![0x55 as c_char; len as usize + 1];
        let written = copy_out(&s, buf.as_mut_ptr(), len);
        prop_assert_eq!(written, len > 0);
        prop_assert_eq!(buf[len as usize], 0x55, "the guard byte");
        if written {
            // SAFETY: copy_out terminated the buffer.
            let got = unsafe { CStr::from_ptr(buf.as_ptr()) };
            let got = got.to_str().unwrap();
            prop_assert!(s.starts_with(got));
            prop_assert!(got.len() < len as usize);
        }
    }

    /// `copy_text` writes all of the string or nothing, and always says how
    /// much it needs.
    #[test]
    fn proptest_wp_q2a_copy_text_is_all_or_nothing(s in "[^\\x00]{0,64}", len in 0u32..80) {
        let mut buf = vec![0x55 as c_char; len as usize + 1];
        let needed = copy_text(&s, buf.as_mut_ptr(), len);
        prop_assert_eq!(needed as usize, s.len() + 1);
        if len >= needed {
            // SAFETY: copy_out terminated the buffer.
            let got = unsafe { CStr::from_ptr(buf.as_ptr()) };
            prop_assert_eq!(got.to_str().unwrap(), s.as_str());
        } else {
            prop_assert!(buf.iter().all(|b| *b == 0x55));
        }
        prop_assert_eq!(copy_text(&s, ptr::null_mut(), len), needed);
    }

    /// Any arguments on a null or a disconnected handle give the failure value.
    #[test]
    fn proptest_wp_q2a_no_device_gives_the_failure_value(
        a in any::<u8>(), b in any::<u8>(), v in any::<f32>(), on in any::<bool>(),
    ) {
        let h = Handle::new();
        for core in [NULL, h.0] {
            prop_assert!(!dspi_set_band_bypass(core, a, b, on));
            prop_assert!(!dspi_set_input_preamp(core, a, v));
            prop_assert!(!dspi_set_upmix_param(core, a, v));
            prop_assert!(!dspi_set_tube_param(core, a, v));
            prop_assert_eq!(dspi_set_output_enable(core, a, on), -1);
            prop_assert_eq!(dspi_save_preset(core, a), status_for_slot(core, a));
        }
    }

    /// Any name, on a real connection: the call answers and the connection
    /// stays usable. One demo device per test thread: opening one takes 250 ms.
    #[test]
    fn proptest_wp_q2a_any_name_is_handled(name in "[^\\x00]{0,80}", ch in any::<u8>()) {
        thread_local! {
            static DEMO: Handle = Handle::demo();
        }
        let name = CString::new(name).unwrap();
        DEMO.with(|h| {
            let _ = dspi_set_channel_name(h.0, ch, name.as_ptr());
            assert!(dspi_is_connected(h.0));
            assert!(dspi_set_master_volume(h.0, -3.0));
        });
    }
}

/// What `dspi_save_preset` gives with no device: an invalid slot is refused
/// before the connection is looked at (on a real handle).
fn status_for_slot(core: *mut FfiCore, slot: u8) -> u8 {
    if !core.is_null() && slot >= 10 {
        PRESET_ERR_INVALID_SLOT
    } else {
        0xFF
    }
}
