//! WP CG11: the calibration flow through the C ABI, on the demo device.

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn view(
    h: *mut FfiCore,
    f: extern "C" fn(*mut FfiCore, *mut c_char, u32) -> u32,
) -> serde_json::Value {
    let mut buf = vec![0u8; 8192];
    let n = f(h, buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
    assert!(n > 0 && n <= buf.len());
    serde_json::from_slice(&buf[..n - 1]).unwrap()
}

#[test]
fn wp_cg11_a_whole_flow_through_c() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));

    let v = view(h, dspi_core_vreg_cal_poll);
    assert_eq!(v["ok"], true);
    assert_eq!(v["available"], true);
    assert_eq!(v["phase"], "idle");
    assert_eq!(v["choices"][0]["needsWord"], false);
    assert_eq!(v["choices"][1]["needsWord"], true);
    assert_eq!(v["confirmationWord"], "OVERVOLT");

    assert_eq!(dspi_core_vreg_cal_open(h), 0);
    assert_eq!(view(h, dspi_core_vreg_cal_view)["phase"], "review");

    // Refused, and the reason is in the view.
    assert_eq!(dspi_core_vreg_cal_start(h, 0x0E, 1800, ptr::null()), 0xFF);
    let v = view(h, dspi_core_vreg_cal_view);
    assert!(v["lastError"].as_str().unwrap().contains("OVERVOLT"));
    assert_eq!(dspi_core_vreg_cal_start(h, 0x0C, 1800, ptr::null()), 0xFF);
    assert_eq!(dspi_core_vreg_cal_start(h, 0x0F, 1800, ptr::null()), 0xFF);

    let phrase = CString::new("OVERVOLT").unwrap();
    assert_eq!(dspi_core_vreg_cal_start(h, 0x0E, 1800, phrase.as_ptr()), 0);
    assert_eq!(view(h, dspi_core_vreg_cal_poll)["phase"], "starting_trial");
    assert_eq!(view(h, dspi_core_vreg_cal_poll)["phase"], "soaking");

    assert_eq!(dspi_core_vreg_demo_event(h, 3, 1800), 0);
    assert_eq!(view(h, dspi_core_vreg_cal_poll)["phase"], "passed");
    // Saving asks again, though the word was typed for the trial.
    let v = view(h, dspi_core_vreg_cal_poll);
    assert_eq!(v["commitNeedsWord"], true);
    assert_eq!(dspi_core_vreg_cal_commit(h, ptr::null()), 0xFF);
    assert_eq!(view(h, dspi_core_vreg_cal_poll)["phase"], "passed");
    assert_eq!(dspi_core_vreg_cal_commit(h, phrase.as_ptr()), 0);
    let v = view(h, dspi_core_vreg_cal_poll);
    assert_eq!(v["phase"], "committed");
    assert_eq!(v["status"]["storedSel"], 0x0E);

    assert_eq!(dspi_core_vreg_manual_set(h, 0, ptr::null()), 0);
    assert_eq!(view(h, dspi_core_vreg_cal_poll)["phase"], "committed");
    assert_eq!(dspi_core_vreg_manual_auto(h, false), 0);
    assert_eq!(dspi_core_vreg_manual_clear(h), 0);

    assert_eq!(dspi_core_vreg_manual_trial(h, 0x0D, ptr::null()), 0);
    let v = view(h, dspi_core_vreg_cal_poll);
    assert_eq!(v["phase"], "trial_found");
    assert_eq!(dspi_core_vreg_cal_keep(h, 1800), 0);
    assert_eq!(dspi_core_vreg_demo_event(h, 0, 0), 0);
    let v = view(h, dspi_core_vreg_cal_poll);
    assert_eq!(v["phase"], "failed");
    assert_eq!(v["nextVreg"], 0x0E);
    assert_eq!(dspi_core_vreg_trial_end(h), 0);
    assert_eq!(dspi_core_vreg_cal_close(h), 0xFF, "still ending");
    assert_eq!(view(h, dspi_core_vreg_cal_poll)["phase"], "ended");
    assert_eq!(dspi_core_vreg_cal_close(h), 0);

    assert_eq!(dspi_core_vreg_demo_event(h, 9, 0), 0xFF);
    dspi_core_free(h);
}

#[test]
fn wp_cg11_null_handles_and_no_device_fail_quietly() {
    let n = ptr::null_mut();
    assert_eq!(dspi_core_vreg_cal_poll(n, ptr::null_mut(), 0), 0);
    assert_eq!(dspi_core_vreg_cal_view(n, ptr::null_mut(), 0), 0);
    assert_eq!(dspi_core_vreg_cal_open(n), 0xFF);
    assert_eq!(dspi_core_vreg_cal_close(n), 0xFF);
    assert_eq!(dspi_core_vreg_cal_start(n, 0x0D, 1800, ptr::null()), 0xFF);
    assert_eq!(dspi_core_vreg_cal_keep(n, 1800), 0xFF);
    assert_eq!(dspi_core_vreg_cal_commit(n, ptr::null()), 0xFF);
    assert_eq!(dspi_core_vreg_trial_end(n), 0xFF);
    assert_eq!(dspi_core_vreg_manual_set(n, 0, ptr::null()), 0xFF);
    assert_eq!(dspi_core_vreg_manual_trial(n, 0x0D, ptr::null()), 0xFF);
    assert_eq!(dspi_core_vreg_manual_auto(n, true), 0xFF);
    assert_eq!(dspi_core_vreg_manual_clear(n), 0xFF);
    assert_eq!(dspi_core_vreg_demo_event(n, 0, 0), 0xFF);

    let h = dspi_core_new();
    let v = view(h, dspi_core_vreg_cal_poll);
    assert_eq!(v["available"], false);
    assert_eq!(dspi_core_vreg_cal_open(h), 0xFF);
    dspi_core_free(h);
}

#[test]
fn wp_cg11_the_functions_are_exported() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
    for name in [
        "dspi_core_vreg_cal_view",
        "dspi_core_vreg_cal_poll",
        "dspi_core_vreg_cal_open",
        "dspi_core_vreg_cal_close",
        "dspi_core_vreg_cal_start",
        "dspi_core_vreg_cal_keep",
        "dspi_core_vreg_cal_commit",
        "dspi_core_vreg_trial_end",
        "dspi_core_vreg_manual_set",
        "dspi_core_vreg_manual_trial",
        "dspi_core_vreg_manual_auto",
        "dspi_core_vreg_manual_clear",
        "dspi_core_vreg_demo_event",
    ] {
        assert!(header.contains(name), "{name} missing");
    }
}

/// WP CG14: the owner's example at the raw C call; what went out is read back
/// from the device.
#[test]
fn wp_cg14_raw_op_takes_the_word_through_c() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let word = CString::new("OVERVOLT").unwrap();
    let lower = CString::new("overvolt").unwrap();
    let stored = |h| view(h, dspi_core_vreg_cal_poll)["status"]["storedSel"].clone();

    assert_eq!(dspi_core_vreg_op(h, 0, 0x0E, word.as_ptr()), 0);
    assert_eq!(stored(h), 0x0E);
    assert_eq!(dspi_core_vreg_op(h, 0, 0x0F, ptr::null()), 0xFF);
    assert_eq!(dspi_core_vreg_op(h, 0, 0x0F, lower.as_ptr()), 0xFF);
    assert_eq!(stored(h), 0x0E, "refused: unchanged");
    assert_eq!(dspi_core_vreg_op(h, 0, 0x0F, word.as_ptr()), 0);
    assert_eq!(stored(h), 0x0F);
    assert_eq!(dspi_core_vreg_op(h, 0, 0x0E, ptr::null()), 0, "lowering");
    assert_eq!(stored(h), 0x0E);
    dspi_core_free(h);
}

#[test]
fn wp_cg14_the_refusal_has_its_own_code() {
    assert_eq!(
        error_code(&DspiError::OvervoltWordRequired { msg: String::new() }),
        "overvolt_word"
    );
}
