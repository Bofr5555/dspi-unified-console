//! WP CG32 tests: a parameter by registry path through C.

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_get_state};

/// Call `dspi_write_param` with `request`: the reply as JSON.
fn write(h: *mut FfiCore, request: &str) -> serde_json::Value {
    let req = CString::new(request).unwrap();
    let mut buf = vec![0u8; 1024];
    let needed = dspi_write_param(h, req.as_ptr(), buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

#[test]
fn wp_cg32_write_param_through_c_reads_back_and_refuses_in_words() {
    let h = dspi_core_new();
    let r = write(h, r#"{"path":"vol.user","value":{"float":-12.5}}"#);
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "disconnected");

    assert!(dspi_open_demo(h));
    let r = write(
        h,
        r#"{"path":"out.gain","indices":[1],"value":{"float":-4.05}}"#,
    );
    assert_eq!(r["ok"], true, "{r}");
    // The setter's quantizing, read back from the mirror.
    assert_eq!(r["value"]["float"].as_f64().unwrap() as f32, -4.1);
    // SAFETY: the handle is live; the record stays valid until the next call.
    let state = unsafe { &*dspi_get_state(h) };
    assert_eq!(state.output_gain_db[1], -4.1);

    let r = write(
        h,
        r#"{"path":"preset.load","indices":[0],"value":"trigger"}"#,
    );
    assert_eq!(r["value"], "trigger", "{r}");

    for (bad, code) in [
        (r#"{"path":"vol.user","value":{"float":3}}"#, "invalid"),
        (r#"{"path":"out.gain","value":{"float":-3}}"#, "invalid"),
        (r#"{"path":"dev.reset","value":"trigger"}"#, "invalid"),
        (r#"{"path":"nope","value":"trigger"}"#, "invalid"),
        ("not json", "invalid"),
    ] {
        let r = write(h, bad);
        assert_eq!(
            (r["ok"].clone(), r["code"].as_str()),
            (false.into(), Some(code)),
            "{bad}: {r}"
        );
    }
    let mut buf = [0u8; 256];
    let n = dspi_write_param(h, ptr::null(), buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(n > 0);
    dspi_core_free(h);
}

#[test]
fn wp_cg32_the_presence_bits_follow_the_image() {
    assert_eq!(dspi_state_presence(ptr::null()), 0);
    let h = dspi_core_new();
    assert_eq!(dspi_state_presence(h), 0, "nothing read yet");
    assert!(dspi_open_demo(h));
    let s = dspi_core::DspiCore::get_state(&unsafe { &*h }.core);
    let want = [
        s.i2s_clock_pin_mode_present,
        s.spdif_inputs_enabled_present,
        s.adat_input_enabled_present,
        s.adat_input_clock_mode_present,
    ]
    .iter()
    .enumerate()
    .fold(0u8, |b, (i, &on)| b | (u8::from(on) << i));
    assert_eq!(dspi_state_presence(h), want);
    // The demo is current firmware: every byte is there.
    assert_eq!(want, 0x0F);
    dspi_core_free(h);
}
