//! Small stateless facts from the core (WP CG-SWS-a), so the page copies no
//! core rule:
//!
//! - `quantizeMasterVolume {db}`: the master volume as the setter keeps it.
//! - `curveSections {filters}`: the biquad sections of a chain's curve,
//!   `[{b0, b1, b2, a1, a2}]` (`a0` is 1).
//! - `vregCode {code, platformId}`: `{mv, label}` of a core-voltage code;
//!   `mv` null for a code the chip does not have.
//! - `platformShape {platformId}`: the platform's channel counts and
//!   platform-bound sections (`dspi_core::PlatformShape`, camelCase).
//! - `knownFeatureNames`: every name the `features` gate knows.
//! - `defaultChannelNames {numInputs, numOutputs, outputTypes}`: the
//!   firmware's names of unnamed channels, inputs then outputs.
//! - `enumValueLabels {path}`: `[{value, key, label}]` of a choice setting;
//!   code `invalid` for a path without labels.
//! - `imageExtras`: the open device's image bytes `DspState` has no field
//!   for.
//! - `linkParseTxt {serviceName, txt}`: a hub's DNS-SD TXT record (`txt` an
//!   object of strings), code `invalid` when it is no hub's.
//!
//! None needs a device but `imageExtras` (all false without one), and none
//! sends anything.

use dspi_core::{DspiCore, FilterParams, LinkTxtEntry};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};
use crate::curves::Band;

type CallResult = Result<Value, CallError>;

/// More bands than any chain has.
const MAX_BANDS: usize = 64;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

#[derive(Deserialize)]
struct Db {
    db: f32,
}

#[derive(Deserialize)]
struct Filters {
    filters: Vec<Band>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VregCode {
    code: u8,
    platform_id: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Platform {
    platform_id: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Names {
    num_inputs: u8,
    num_outputs: u8,
    #[serde(default)]
    output_types: Vec<u8>,
}

#[derive(Deserialize)]
struct Path {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Txt {
    #[serde(default)]
    service_name: String,
    txt: serde_json::Map<String, Value>,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "quantizeMasterVolume" => {
            params::<Db>(p).map(|d| tidy(&dspi_core::quantize_master_volume(d.db)))
        }
        "curveSections" => params::<Filters>(p).and_then(|f| {
            if f.filters.len() > MAX_BANDS {
                return Err(CallError::new(
                    "invalid",
                    format!("at most {MAX_BANDS} bands"),
                ));
            }
            let chain = f.filters.into_iter().map(FilterParams::from).collect();
            Ok(tidy(&dspi_core::curve_sections(chain)))
        }),
        "vregCode" => params::<VregCode>(p).map(|v| {
            json!({
                "mv": dspi_core::vreg_code_mv(v.code, v.platform_id),
                "label": dspi_core::vreg_code_label(v.code, v.platform_id),
            })
        }),
        "platformShape" => {
            params::<Platform>(p).map(|v| tidy(&dspi_core::platform_shape(v.platform_id)))
        }
        "knownFeatureNames" => Ok(json!(dspi_core::known_feature_names())),
        "defaultChannelNames" => params::<Names>(p).map(|n| {
            json!(dspi_core::default_channel_names(
                n.num_inputs,
                n.num_outputs,
                n.output_types
            ))
        }),
        "enumValueLabels" => params::<Path>(p).and_then(|v| {
            dspi_core::enum_value_labels(v.path)
                .map(|l| tidy(&l))
                .map_err(CallError::from)
        }),
        "imageExtras" => Ok(tidy(&core.image_extras())),
        "linkParseTxt" => params::<Txt>(p).and_then(|t| {
            let entries = t
                .txt
                .into_iter()
                .map(|(key, v)| match v {
                    Value::String(value) => Ok(LinkTxtEntry { key, value }),
                    _ => Err(CallError::new(
                        "invalid",
                        format!("TXT key {key}: not a string"),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            dspi_core::link_parse_txt(t.service_name, entries)
                .map(|r| tidy(&r))
                .map_err(CallError::from)
        }),
        _ => return None,
    })
}

#[cfg(test)]
#[path = "sws_a/tests_wp_cgswsa.rs"]
mod tests_wp_cgswsa;
