//! The measurement documents on the web (WP P5DOCK, PLAN-P5 T11): no files
//! there, so the page keeps document text (downloads, uploads, and the hub's
//! store from P5HUB on) and these calls make and read it, as the snapshot
//! document calls do. None touches the device.
//!
//! - `measureDocInfo {text}`: a measurement, mic or target document's row,
//!   validated: `{id, kind, name, note, serial, savedMs, bytes}`.
//! - `micImport {name, fileName, text}`: a `mic` document from a calibration
//!   file's text: `{text}`.
//! - `targetImport {name, text, use, coupler}`: a `target` document from a
//!   target file's text (`use` `speakers` or `headphones`; `coupler`
//!   `iec60318_4`, `flatPlate`, `hats` or ""): `{text}`.
//! - `targetBuiltins`: the built-in targets, `[{id, name, targetUse, coupler,
//!   params: [{name, value}]}]`.
//! - `measurementExportRew {text, output}`: output `output`'s response of a
//!   measurement document as REW text: `{text}`.
//!
//! A refusal is code `invalid` with the reason in words.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MicFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    file_name: String,
    text: String,
}

#[derive(Deserialize)]
struct TargetFile {
    #[serde(default)]
    name: String,
    text: String,
    #[serde(rename = "use")]
    target_use: String,
    #[serde(default)]
    coupler: String,
}

#[derive(Deserialize)]
struct Export {
    text: String,
    output: u8,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "measureDocInfo" => params::<Text>(p).and_then(|t| {
            dspi_core::measure_doc_info(t.text)
                .map(|i| tidy(&i))
                .map_err(CallError::from)
        }),
        "micImport" => params::<MicFile>(p).and_then(|m| {
            dspi_core::mic_import(m.name, m.file_name, m.text)
                .map(|text| json!({ "text": text }))
                .map_err(CallError::from)
        }),
        "targetImport" => params::<TargetFile>(p).and_then(|t| {
            dspi_core::target_import(t.name, t.text, t.target_use, t.coupler)
                .map(|text| json!({ "text": text }))
                .map_err(CallError::from)
        }),
        "targetBuiltins" => Ok(tidy(&dspi_core::target_builtins())),
        "measurementExportRew" => params::<Export>(p).and_then(|e| {
            // Text only: an id names a file, and the web has none.
            if dspi_core::measure_doc_info(e.text.clone()).is_err() {
                return Err(CallError::new(
                    "invalid",
                    "not a measurement document".to_owned(),
                ));
            }
            core.measurement_export_rew(e.text, e.output)
                .map(|text| json!({ "text": text }))
                .map_err(CallError::from)
        }),
        _ => return None,
    })
}

#[cfg(test)]
#[path = "measure_docs/tests_wp_p5dock.rs"]
mod tests_wp_p5dock;
