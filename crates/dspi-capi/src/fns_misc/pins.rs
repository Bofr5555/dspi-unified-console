//! C functions: the pin map (WP CG-SWS-b), over `dspi_core::pins`. From the
//! mirror and the last control-surface read: no device I/O.
//!
//! A function is passed as its kind code and index. The codes are
//! `dspi_core::PinFunctionKind`'s: 0 output (index: physical output, the
//! sub last), 1 I2S BCK, 2 the slave pair's BCK, 3 MCK, 4 S/PDIF RX (index:
//! input), 5 I2S RX (index: pair), 6 ADAT out, 7 ADAT in, 8 DAC mute, 9 UART
//! TX, 10 UART RX, 11 I2C SDA, 12 I2C SCL, 13 control surface (index: slot).
//! In JSON a function is `{"kind": "output", "index": 0}`, its kind in
//! camelCase (`i2sBckSlave`, `spdifRx`, ...).

use std::ffi::c_char;

use dspi_core::{PinFunction, PinFunctionKind};

use super::{json_reply, text};
use crate::{FfiCore, with_core};

/// `PIN_CONFIG_*` for "nothing to say": a null handle, a panic or a kind
/// code this library does not know.
const NO_ANSWER: u8 = 0xFF;

/// The pin map, JSON `{ok, platformId, assignable, validPins, functions:
/// [{function, label, pin, holds, choices, i2sAllowed}], holders: [{pin,
/// function, role, label}]}`. Without a device: nothing assignable and no
/// functions. 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_pin_map(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.pin_map())))
}

/// Whether function (`kind`, `index`) may move to GPIO `pin`: 0 when it
/// may, else the `PIN_CONFIG_*` status the firmware would answer (1 invalid
/// pin or wrong mux, 2 in use, 3 no such function, 4 cannot move now).
/// 0xFF for a null handle or an unknown kind. The holder of a pin in use is
/// [`dspi_pin_holder`]'s answer.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_pin_check(core: *mut FfiCore, kind: u8, index: u8, pin: u8) -> u8 {
    let Some(kind) = PinFunctionKind::from_code(kind) else {
        return NO_ANSWER;
    };
    with_core(core, NO_ANSWER, |h| {
        h.core
            .pin_check(PinFunction::new(kind, index), pin)
            .map_or(0, |r| r.status)
    })
}

/// Who holds GPIO `pin`, not counting function (`exclude_kind`,
/// `exclude_index`) (`exclude_kind` 0xFF: counting everyone): JSON `{ok,
/// holder}`, `holder` `{pin, function, role, label}` or null when it is
/// free. An unknown kind code is `{ok: false, code: "invalid"}`. 0 for a null
/// handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_pin_holder(
    core: *mut FfiCore,
    pin: u8,
    exclude_kind: u8,
    exclude_index: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let excluding = match exclude_kind {
            0xFF => Ok(None),
            k => PinFunctionKind::from_code(k)
                .map(|k| Some(PinFunction::new(k, exclude_index)))
                .ok_or_else(|| dspi_core::DspiError::InvalidArgument {
                    msg: format!("no pin function kind {k}"),
                }),
        };
        json_reply(excluding.map(|e| serde_json::json!({ "holder": c.pin_holder(pin, e) })))
    })
}

#[cfg(test)]
mod tests_wp_cgswsb {
    use std::ptr::null_mut;

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 64 * 1024];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32) as usize;
        assert!(n > 0 && n < buf.len(), "reply of {n} bytes");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    #[test]
    fn cgswsb_the_pin_map_over_c() {
        let h = dspi_core_new();
        assert!(super::super::dspi_open_demo(h));
        let map = reply(|b, l| dspi_pin_map(h, b, l));
        assert_eq!(map["ok"], true);
        assert_eq!(map["assignable"], true);
        assert_eq!(map["platformId"], 1);
        assert!(
            map["validPins"]
                .as_array()
                .unwrap()
                .contains(&Value::from(29))
        );
        let out1 = map["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["function"]["kind"] == "output" && f["function"]["index"] == 0)
            .unwrap();
        assert_eq!(out1["label"], "Output 1");
        let own = out1["pin"].as_u64().unwrap() as u8;
        assert_eq!(dspi_pin_check(h, 0, 0, own), 0, "its own pin");
        assert_eq!(dspi_pin_check(h, 0, 0, 24), 1, "power");
        assert_eq!(dspi_pin_check(h, 0, 9, 0), 3, "no output 10");
        assert_eq!(dspi_pin_check(h, 99, 0, 0), 0xFF, "unknown kind");
        // Output 2's pin is Output 1's conflict, named by the holder call.
        let holders = map["holders"].as_array().unwrap();
        let out2 = holders.iter().find(|x| x["label"] == "Output 2").unwrap();
        let pin2 = out2["pin"].as_u64().unwrap() as u8;
        assert_eq!(dspi_pin_check(h, 0, 0, pin2), 2);
        let held = reply(|b, l| dspi_pin_holder(h, pin2, 0, 0, b, l));
        assert_eq!(held["holder"]["label"], "Output 2");
        assert_eq!(held["holder"]["function"]["kind"], "output");
        let none = reply(|b, l| dspi_pin_holder(h, pin2, 0, 1, b, l));
        assert_eq!(none["holder"], Value::Null, "Output 2 itself excluded");
        let bad = reply(|b, l| dspi_pin_holder(h, pin2, 77, 0, b, l));
        assert_eq!(bad["code"], "invalid");
        dspi_core_free(h);
    }

    #[test]
    fn cgswsb_null_handles() {
        assert_eq!(dspi_pin_check(null_mut(), 0, 0, 0), 0xFF);
        assert_eq!(dspi_pin_map(null_mut(), null_mut(), 0), 0);
        assert_eq!(dspi_pin_holder(null_mut(), 0, 0xFF, 0, null_mut(), 0), 0);
    }
}
