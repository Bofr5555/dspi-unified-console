//! WP WB6g: what the web's input pages read that `state` does not carry.
//!
//! `inputSourceNames`: the user's names for the input sources, as the core
//! last knew them (`DspiCore::cached_input_source_name`: read on connect,
//! kept current by the device's rename notifications from any client), with
//! no traffic. A rename is a `write` of the registry's `in.name`, which goes
//! through the core's typed setter and its journal.

use dspi_core::{DspiCore, DspiError};
use serde_json::{Value, json};

use crate::calls::CallError;

/// The sources a name table can hold: USB, S/PDIF, I2S, ADAT, S/PDIF 2..4
/// and the audio link (`in.source` 0..7).
const SOURCES: u8 = 8;

/// `inputSourceNames`: `{supported, names}`. `names[s]` is source `s`'s name
/// ("" when unnamed), as far as the device's table goes. Firmware without
/// source names answers `supported: false` and no names, nothing sent.
pub fn source_names(core: &DspiCore) -> Result<Value, CallError> {
    let mut names = Vec::new();
    for s in 0..SOURCES {
        match core.cached_input_source_name(s) {
            Ok(n) => names.push(n),
            Err(DspiError::Unsupported) => return Ok(json!({ "supported": false, "names": [] })),
            // Past the device's table: the rest are not its sources.
            Err(DspiError::InvalidArgument { .. }) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(json!({ "supported": true, "names": names }))
}

#[cfg(test)]
mod tests_wp_wb6g {
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

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    #[test]
    fn wb6g_source_names_read_from_the_cache_and_renamed_by_write() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        // Nothing open: the core's own refusal.
        let r = run(h, "inputSourceNames", Value::Null);
        assert_eq!(r["ok"], false, "{r}");

        ok(h, "open", json!({ "kind": "demo" }));
        let n = ok(h, "inputSourceNames", Value::Null);
        assert_eq!(n["supported"], true, "{n}");
        let names = n["names"].as_array().unwrap();
        assert!(names.len() >= 7, "{n}");
        assert!(
            names.iter().all(|x| x == ""),
            "the demo starts unnamed: {n}"
        );

        // A rename through `write` (the core's setter and journal), then the cache.
        let w = ok(
            h,
            "write",
            json!({ "path": "in.name", "indices": [1], "value": { "text": "Turntable" } }),
        );
        assert_eq!(w, json!({ "text": "Turntable" }));
        let n = ok(h, "inputSourceNames", Value::Null);
        assert_eq!(n["names"][1], "Turntable", "{n}");
        assert_eq!(n["names"][0], "");
        // The journal records it, and undo steps over it (a name is a flash
        // write, as the registry classes it): the name stays.
        let undone = ok(h, "undo", Value::Null);
        assert_eq!(undone["skipped"][0]["path"], "in.name", "{undone}");
        assert_eq!(undone["skipped"][0]["undoable"], false, "{undone}");
        assert_eq!(
            ok(h, "inputSourceNames", Value::Null)["names"][1],
            "Turntable"
        );
        // "" clears; a name past 31 bytes is refused before anything is sent.
        let long = "x".repeat(32);
        let r = run(
            h,
            "write",
            json!({ "path": "in.name", "indices": [0], "value": { "text": long } }),
        );
        assert_eq!(r["ok"], false, "{r}");
    }
}
