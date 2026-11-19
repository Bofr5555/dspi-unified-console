//! The open device's protocol generation and raw paged requests by command
//! id (WP PGCORE, PLAN-PG sections 1.1, 1.2 and 1.6), over `dspi_core`'s
//! `device_protocol`, `raw_get_id` and `raw_set_id`:
//!
//! - `deviceProtocol`: `{generation ("unknown", "legacy" or "paged"),
//!   protocolMajor, protocolMinor, pages: [{page, revision}], extEnvelope
//!   ("noChannel", "legacy" or "tunnel")}`. No traffic; code `not_open`
//!   with nothing open.
//! - `rawGetId {id, value, len}`: `{data: [bytes]}`, a paged GET of command
//!   `id` (`page << 8 | command`).
//! - `rawSetId {id, value, data}`: `{}`, a paged SET. Refused with code
//!   `invalid` and nothing sent for what the core gates (a request behind a
//!   confirmation token, a core-voltage request, a flash or restart command,
//!   a page-0 id, one with no 1.1.6 equivalent that is not a plain value
//!   SET).
//!
//! Refusals by the firmware are code `refused`; a 1.1.6-generation device
//! answers `unsupported`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

#[derive(Deserialize)]
struct Get {
    id: u16,
    #[serde(default)]
    value: u16,
    len: u16,
}

#[derive(Deserialize)]
struct Set {
    id: u16,
    #[serde(default)]
    value: u16,
    #[serde(default)]
    data: Vec<u8>,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "deviceProtocol" => core
            .device_protocol()
            .map(|d| tidy(&d))
            .map_err(CallError::from),
        "rawGetId" => params::<Get>(p).and_then(|g| {
            core.raw_get_id(g.id, g.value, g.len)
                .map(|data| json!({ "data": data }))
                .map_err(CallError::from)
        }),
        "rawSetId" => params::<Set>(p).and_then(|s| {
            core.raw_set_id(s.id, s.value, s.data)
                .map(|()| json!({}))
                .map_err(CallError::from)
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_pgcore {
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
    fn wp_pgcore_the_demo_is_legacy_and_the_raw_id_calls_are_gated() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "deviceProtocol", Value::Null)["code"], "not_open");
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let r = run(h, "deviceProtocol", Value::Null);
        assert_eq!(r["value"]["generation"], "legacy", "{r}");
        assert_eq!(r["value"]["pages"], json!([]));
        // A 1.1.6-generation device has no pages: nothing is sent
        let r = run(h, "rawGetId", json!({ "id": 0x0550, "len": 4 }));
        assert_eq!(r["code"], "unsupported", "{r}");
        // The bootloader by its paged id is gated before the device
        let r = run(h, "rawSetId", json!({ "id": 0x0130 }));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "rawGetId", json!({ "id": 0x0050, "len": 4 }));
        assert_eq!(r["code"], "invalid", "{r}");
    }
}
