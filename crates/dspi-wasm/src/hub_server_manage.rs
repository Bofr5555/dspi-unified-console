//! The hub's device and name writes on the web (WP P2CC3), over
//! `dspi_core::hub_server::manage` (`docs/integrator/hub-api.md` sections 3,
//! 4 and 15), through this session's DSPi Link. Records out are the core's,
//! camelCase:
//!
//! - `hubInventorySet {serial?, location?, note?, tags?}` (Admin): a field
//!   left out (or null) keeps its value, "" or `[]` clears it; the record.
//! - `hubInventoryForget {serial, withData?}` (Admin): `{serial, forgot}`.
//! - `hubRename {name}` (Admin): the name the hub now uses.
//! - `hubIdentifySerial {serial?, seconds, toneOutput?}`: any device on the
//!   hub ("" or left out: the open one); the state read after, or null.
//!
//! Without the family's cap every call answers `unsupported` and sends
//! nothing; in the client read-only mode `read_only`, nothing sent.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    // No parameters is every field's default.
    let v = if v.is_null() {
        Value::Object(Default::default())
    } else {
        v
    };
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    Ok(tidy(&r?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct InventorySet {
    serial: String,
    location: Option<String>,
    note: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Forget {
    serial: String,
    #[serde(default)]
    with_data: bool,
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentifySerial {
    #[serde(default)]
    serial: String,
    seconds: u8,
    #[serde(default)]
    tone_output: Option<u8>,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "hubInventorySet" => params::<InventorySet>(p)
            .and_then(|x| record(core.hub_inventory_set(x.serial, x.location, x.note, x.tags))),
        "hubInventoryForget" => params::<Forget>(p)
            .and_then(|x| record(core.hub_inventory_forget(x.serial, x.with_data))),
        "hubRename" => params::<Rename>(p).and_then(|x| record(core.hub_rename(x.name))),
        "hubIdentifySerial" => params::<IdentifySerial>(p)
            .and_then(|x| record(core.hub_identify_serial(x.serial, x.seconds, x.tone_output))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p2cc3 {
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

    /// The same words as the core's own calls, on the web.
    #[test]
    fn wp_p2cc3_the_hub_writes_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        for (m, p) in [
            (
                "hubInventorySet",
                json!({"serial": "S1", "location": "Hall", "tags": ["main"]}),
            ),
            (
                "hubInventoryForget",
                json!({"serial": "S1", "withData": true}),
            ),
            ("hubRename", json!({"name": "Studio"})),
            (
                "hubIdentifySerial",
                json!({"serial": "S1", "seconds": 5, "toneOutput": 1}),
            ),
        ] {
            let r = run(h, m, p);
            assert_eq!(r["ok"], false, "{m}: {r}");
            assert_eq!(r["code"], "unsupported", "{m}: {r}");
            assert_eq!(r["noticeKey"], "unsupported", "{m}: {r}");
        }
        // Parameters that do not read, and forget without its device.
        let r = run(h, "hubRename", Value::Null);
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "hubInventoryForget", json!({"serial": ""}));
        assert_eq!(r["code"], "invalid", "{r}");
        let core_says = dspi_core::DspiCore::new()
            .hub_inventory_forget(String::new(), false)
            .unwrap_err();
        assert_eq!(r["error"], core_says.to_string(), "{r}");
    }
}
