//! C functions: the demo's simulated microphone (WP P5DEMO, PLAN-P5 OD-P5q),
//! over `dspi_core`'s `measure_sim_*` calls.
//!
//! On the demo device only: turned on, P5SIM's three-way speaker in a room
//! answers the generator and feeds the measurement session from
//! `dspi_measure_tick`; the app pushes no capture (its pushes are ignored)
//! and labels the microphone with `label` ("Simulated microphone"). Every
//! reply is JSON: `{ok, available, on, label, world, seed, issue}` or a
//! failure (`unsupported` over a real DSPi or in a build without it,
//! `disconnected`, `invalid` while a measurement runs).

use std::ffi::c_char;

use super::{json_reply, text};
use crate::FfiCore;

/// Turn the simulated microphone on (`on`, its world drawn from `seed`) or
/// off: the info, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_sim_set(
    core: *mut FfiCore,
    on: bool,
    seed: u64,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.measure_sim_set(on, seed)))
}

/// The simulated microphone's state, without I/O: the info.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_sim_info(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.measure_sim_info())))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5demo {
    use std::ffi::CString;
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;
    use crate::fns_misc::measure::{
        dspi_measure_capture_push, dspi_measure_result, dspi_measure_start, dspi_measure_tick,
    };

    fn read(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 1 << 20];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n <= buf.len());
        let s = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        serde_json::from_str(s.to_str().unwrap()).unwrap()
    }

    /// The done criterion through the C ABI: a demo-device session produces
    /// a measurement without a microphone, with a timing verdict.
    #[test]
    fn wp_p5demo_the_c_calls_measure_with_the_simulated_microphone() {
        let h = dspi_core_new();
        let r = read(|b, l| dspi_measure_sim_set(h, true, 7, b, l));
        assert_eq!(r["code"], "disconnected", "{r}");
        assert!(dspi_open_demo(h));
        assert!(crate::fns_device::dspi_set_user_volume(h, -20.0));
        let info = read(|b, l| dspi_measure_sim_info(h, b, l));
        assert_eq!(info["available"], true, "{info}");
        assert_eq!(info["on"], false);
        let info = read(|b, l| dspi_measure_sim_set(h, true, 7, b, l));
        assert_eq!(info["ok"], true, "{info}");
        assert_eq!(info["on"], true);
        assert_eq!(info["label"], "Simulated microphone");
        let setup = CString::new(
            r#"{"kind":"walked","outputs":[{"output":0,"role":"full"},{"output":1,"role":"full"}],
               "placement":"headphones","coupler":"flatPlate","sweepMs":1000,"gapMs":300,
               "startLevelOnly":true}"#,
        )
        .unwrap();
        let s = read(|b, l| dspi_measure_start(h, setup.as_ptr(), b, l));
        assert_eq!(s["ok"], true, "{s}");
        // Refused while it runs.
        let r = read(|b, l| dspi_measure_sim_set(h, false, 0, b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        // The app's own push is ignored.
        let samples = [0.9f32; 960];
        let s =
            read(|b, l| dspi_measure_capture_push(h, 5, samples.as_ptr(), 960, 48_000, true, b, l));
        assert_eq!(s["active"], true, "{s}");
        let end = Instant::now() + Duration::from_secs(120);
        let mut st = Value::Null;
        while Instant::now() < end {
            st = read(|b, l| dspi_measure_tick(h, b, l));
            if st["active"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(st["phase"], "done", "{st}");
        assert_eq!(st["timing"]["reliable"], true, "{st}");
        assert_eq!(
            st["timing"]["cycleResidualsUs"].as_array().unwrap().len(),
            6
        );
        let doc = read(|b, l| dspi_measure_result(h, b, l));
        assert_eq!(doc["meta"]["kind"], "measurement", "{doc}");
        assert_eq!(doc["measurement"]["outputs"].as_array().unwrap().len(), 2);
        dspi_core_free(h);
    }
}
