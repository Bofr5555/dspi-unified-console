//! Filter files on the web (WP WB6K, M28): Export Filters and Import
//! Filters with the channel picker, through the core's filter file code
//! (`DspiCore::filters_export`, `filters_inspect`, `filters_import`). The page
//! downloads the export and reads the chosen file; the core reads and writes
//! the format (a Console file, or a REW or AutoEQ list). An import is one undo
//! step, and its writes are the ordinary setters: read-only firmware, the
//! lock and the limits apply as they do everywhere.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

/// The undo step an import makes.
pub const IMPORT_LABEL: &str = "Import Filters";

fn need_device(core: &DspiCore) -> Result<(), CallError> {
    if core.is_connected() {
        Ok(())
    } else {
        Err(CallError::new(
            "not_open",
            "No DSPi is open in this session",
        ))
    }
}

/// The core answers JSON text; as a value.
fn parsed(text: &str) -> Result<Value, CallError> {
    serde_json::from_str(text).map_err(|e| CallError::new("internal", e.to_string()))
}

/// `filtersExport`'s parameters: how many inputs the file carries (the live
/// inputs; at most the device's).
#[derive(Debug, Deserialize)]
pub struct ExportParams {
    #[serde(default)]
    pub inputs: u8,
}

/// `filtersExport`: `{text}`, every channel's bands as a filter file.
pub fn export(core: &DspiCore, p: ExportParams) -> Result<Value, CallError> {
    need_device(core)?;
    Ok(json!({ "text": core.filters_export(p.inputs)? }))
}

/// `filtersInspect`'s parameters.
#[derive(Debug, Deserialize)]
pub struct InspectParams {
    pub text: String,
    #[serde(default)]
    pub inputs: u8,
}

/// `filtersInspect`: what the file holds, for the channel picker: `{kind,
/// channels: [{wire, label, bands, checked}], newer}` for a Console file,
/// `{kind: "rew", channels, filters, preamp}` for a list, `{kind: "error",
/// error}` for one too large. Nothing is written.
pub fn inspect(core: &DspiCore, p: InspectParams) -> Result<Value, CallError> {
    need_device(core)?;
    parsed(&core.filters_inspect(p.text, p.inputs))
}

/// `filtersImport`'s parameters: the file and the wire channels chosen.
#[derive(Debug, Deserialize)]
pub struct ImportParams {
    pub text: String,
    pub wires: Vec<u8>,
}

/// `filtersImport`: apply the file to the chosen channels as one undo step
/// (`Import Filters`): `{ok, error, lines}`. `ok` false, with `error`, when
/// nothing applies or the device refused (read-only firmware, the lock, a
/// limit, a failed transfer); channels written before a failure stay
/// written, and stay in the one step.
pub fn import(core: &DspiCore, p: ImportParams) -> Result<Value, CallError> {
    need_device(core)?;
    core.begin_group(IMPORT_LABEL.into());
    let reply = core.filters_import(p.text, p.wires);
    core.end_group();
    parsed(&reply)
}

#[cfg(test)]
mod tests_wp_wb6k {
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

    fn gain(h: u32, wire: usize, band: usize) -> f64 {
        ok(h, "state", Value::Null)["state"]["filters"][wire][band]["gain"]
            .as_f64()
            .unwrap()
    }

    #[test]
    fn wp_wb6k_filter_files_export_inspect_import_one_undo_step() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(
            run(h, "filtersExport", json!({ "inputs": 2 }))["code"],
            "not_open"
        );
        ok(h, "open", json!({ "kind": "demo" }));
        let s = ok(h, "state", Value::Null)["state"].clone();
        let inputs = s["numInputChannels"].as_u64().unwrap();
        let g1 = gain(h, 1, 0);
        // A band changed, exported, then changed back; the import brings it back.
        ok(
            h,
            "filter",
            json!({ "channel": 0, "band": 0, "filterType": 1, "freq": 1000.0, "q": 1.0, "gain": 4.5 }),
        );
        let text = ok(h, "filtersExport", json!({ "inputs": inputs }))["text"]
            .as_str()
            .unwrap()
            .to_string();
        ok(
            h,
            "filter",
            json!({ "channel": 0, "band": 0, "filterType": 1, "freq": 1000.0, "q": 1.0, "gain": 0.0 }),
        );
        ok(
            h,
            "filter",
            json!({ "channel": 1, "band": 0, "filterType": 1, "freq": 1000.0, "q": 1.0, "gain": -2.0 }),
        );
        let seen = ok(
            h,
            "filtersInspect",
            json!({ "text": text, "inputs": inputs }),
        );
        assert_eq!(seen["kind"], "native", "{seen}");
        assert!(seen["channels"].as_array().unwrap().len() >= 2);
        let undo_before = ok(h, "history", json!({ "n": 5 }));
        let done = ok(h, "filtersImport", json!({ "text": text, "wires": [0, 1] }));
        assert_eq!(done["ok"], true, "{done}");
        assert!((gain(h, 0, 0) - 4.5).abs() < 0.01);
        assert!((gain(h, 1, 0) - g1).abs() < 0.01);
        // One step: one undo takes both channels back.
        let after = ok(h, "history", json!({ "n": 5 }));
        assert_ne!(after, undo_before);
        ok(h, "undo", Value::Null);
        assert!(gain(h, 0, 0).abs() < 0.01);
        assert!((gain(h, 1, 0) + 2.0).abs() < 0.01);

        // A REW list goes to the chosen channels; a file with nothing to apply is refused.
        let rew = "Preamp: -3 dB\nFilter 1: ON PK Fc 200 Hz Gain -4 dB Q 1.4\n";
        let seen = ok(
            h,
            "filtersInspect",
            json!({ "text": rew, "inputs": inputs }),
        );
        assert_eq!(seen["kind"], "rew");
        let done = ok(h, "filtersImport", json!({ "text": rew, "wires": [1] }));
        assert_eq!(done["ok"], true, "{done}");
        assert!((gain(h, 1, 0) + 4.0).abs() < 0.01);
        let none = ok(h, "filtersImport", json!({ "text": "hello", "wires": [0] }));
        assert_eq!(none["ok"], false);
        assert!(!none["error"].as_str().unwrap().is_empty());
    }
}
