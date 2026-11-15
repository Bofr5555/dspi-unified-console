//! The pin map for the web (WP CG-SWS-b), over `dspi_core::pins`: before
//! this the page could list pins but not judge them (WB6I). From the
//! mirror: no device I/O.
//!
//! - `pinMap`: `{platformId, assignable, validPins, functions: [{function,
//!   label, pin, holds, choices, i2sAllowed}], holders: [{pin, function,
//!   role, label}]}`. A function is `{kind, index}`, `kind` one of `output`,
//!   `i2sBck`, `i2sBckSlave`, `mck`, `spdifRx`, `i2sRx`, `adatOut`,
//!   `adatIn`, `dacMute`, `uartTx`, `uartRx`, `i2cSda`, `i2cScl`,
//!   `controlSurface`.
//! - `pinCheck {function, pin}`: `null` when the move is accepted, else
//!   `{reason, status, holder}` (`reason` `notAssignable`, `noSuchFunction`,
//!   `invalidPin`, `wrongMux`, `inUse`, `outputActive`, `clearWhileEnabled`;
//!   `status` the firmware's `PIN_CONFIG_*`).
//! - `pinHolder {pin, excluding?}`: the holder of `pin` (not counting
//!   `excluding`), or `null`.
//!
//! The moves themselves are `write`s (`out.pin`, `i2s.bck`, `in.spdif.pin`,
//! ...), which run the same check before they send.

use dspi_core::{DspiCore, PinFunction};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

#[derive(Deserialize)]
struct Check {
    function: PinFunction,
    pin: u8,
}

#[derive(Deserialize)]
struct Holder {
    pin: u8,
    #[serde(default)]
    excluding: Option<PinFunction>,
}

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "pinMap" => Ok(tidy(&core.pin_map())),
        "pinCheck" => params::<Check>(p).map(|c| tidy(&core.pin_check(c.function, c.pin))),
        "pinHolder" => params::<Holder>(p).map(|h| tidy(&core.pin_holder(h.pin, h.excluding))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_cgswsb {
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn cgswsb_the_web_judges_a_pin_and_moves_it() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let map = run(h, "pinMap", Value::Null);
        assert_eq!(map["ok"], true);
        let map = &map["value"];
        assert_eq!(map["assignable"], true);
        let out1 = map["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["function"] == json!({"kind": "output", "index": 0}))
            .unwrap()
            .clone();
        let own = out1["pin"].as_u64().unwrap();
        let to = out1["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_u64().unwrap())
            .find(|p| *p != own)
            .unwrap();
        // Accepted: null. The BCK pin: in use, by "I2S BCK".
        let ok = run(
            h,
            "pinCheck",
            json!({"function": {"kind": "output", "index": 0}, "pin": to}),
        );
        assert_eq!(ok["value"], Value::Null);
        let bck = map["holders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["label"] == "I2S BCK")
            .unwrap()["pin"]
            .clone();
        let no = run(
            h,
            "pinCheck",
            json!({"function": {"kind": "output", "index": 0}, "pin": bck}),
        );
        assert_eq!(no["value"]["reason"], "inUse");
        assert_eq!(no["value"]["status"], 2);
        assert_eq!(no["value"]["holder"]["label"], "I2S BCK");
        // The move itself, a write, now answered by the demo (WB6I).
        let w = run(
            h,
            "write",
            json!({"path": "out.pin", "indices": [0], "value": {"int": to}}),
        );
        assert_eq!(w["ok"], true, "{w}");
        let held = run(h, "pinHolder", json!({"pin": to}));
        assert_eq!(held["value"]["label"], "Output 1");
        let mine = run(
            h,
            "pinHolder",
            json!({"pin": to, "excluding": {"kind": "output", "index": 0}}),
        );
        assert_eq!(mine["value"], Value::Null);
        // The write of a held pin is refused before it is sent.
        let w = run(
            h,
            "write",
            json!({"path": "out.pin", "indices": [0], "value": {"int": bck}}),
        );
        assert_eq!(w["ok"], false, "{w}");
        let bad = run(
            h,
            "pinCheck",
            json!({"function": {"kind": "nope"}, "pin": 1}),
        );
        assert_eq!(bad["code"], "invalid");
    }
}
