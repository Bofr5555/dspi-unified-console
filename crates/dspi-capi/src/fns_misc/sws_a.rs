//! C functions: small stateless facts the clients used to copy (WP
//! CG-SWS-a), over `dspi_core`'s `platform_shape`, `vreg_code_*`,
//! `default_channel_names`, `enum_value_labels`, `known_feature_names`,
//! `bulk_image_extras`, `quantize_master_volume` and `link_parse_txt`.
//! `dspi_curve_sections` is in `fns_device`, beside the other curve calls.
//!
//! Replies as in the parent module's docs: JSON (camelCase), the bytes
//! needed returned, nothing written when they do not fit. None blocks.

use std::ffi::c_char;

use serde_json::Value;

use super::{c_bytes, c_str, copy_text, guard, json_reply};
use crate::{FfiCore, fns_device::text};

/// The master volume `db` as `dspi_set_master_volume` keeps it: -128 (mute)
/// at or below -128, else 0.5 dB steps within -127..=0. NaN comes back NaN.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_quantize_master_volume(db: f32) -> f32 {
    guard(f32::NAN, || dspi_core::quantize_master_volume(db))
}

/// Core-voltage code `code` in mV on platform `platform_id`, 0 for a code
/// the chip does not have.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_vreg_code_mv(code: u8, platform_id: u8) -> u16 {
    guard(0, || {
        dspi_core::vreg_code_mv(code, platform_id).unwrap_or(0)
    })
}

/// Core-voltage code `code` as shown (`1.20 V`, or `code 0x1F`), plain text.
/// Returns the bytes needed including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_vreg_code_label(
    code: u8,
    platform_id: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        copy_text(&dspi_core::vreg_code_label(code, platform_id), buf, len)
    })
}

/// Platform `platform_id`'s shape in JSON: {ok, platformId, name, known,
/// numInputChannels, numOutputChannels, numOutputSlots, pdmOutput,
/// pdmPinIndex, eqWorkerFirst, eqWorkerLast, i2sMaxPairs, multichannelI2s,
/// adat, upmixer, i2sConfig, crossfeedPairs, maxDelayMs}. Returns the bytes
/// needed including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_platform_shape(platform_id: u8, buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(
            &json_reply(Ok(dspi_core::platform_shape(platform_id))),
            buf,
            len,
        )
    })
}

/// Every feature name `dspi_feature_present` knows, in JSON: {ok, value:
/// [names]}. Returns the bytes needed including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_known_feature_names(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(&json_reply(Ok(dspi_core::known_feature_names())), buf, len)
    })
}

/// The firmware's default channel names, inputs then outputs, in JSON: {ok,
/// value: [names]}. `input_source` names the inputs (`DspState.input_source`);
/// `output_types` holds `types_len` slot types (1 is I2S); null reads as none. Returns the bytes needed including the NUL, 0 for a
/// panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_default_channel_names(
    num_inputs: u8,
    num_outputs: u8,
    input_source: u8,
    output_types: *const u8,
    types_len: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let types = c_bytes(output_types, types_len);
        let names = dspi_core::default_channel_names(num_inputs, num_outputs, input_source, types);
        copy_text(&json_reply(Ok(names)), buf, len)
    })
}

/// The values of choice setting `path` with the Console's words, in JSON:
/// {ok, value: [{value, key, label}]}, or {ok:false, code:"invalid"} for a
/// path without labels. Returns the bytes needed including the NUL, 0 for a
/// null `path` or a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_enum_value_labels(path: *const c_char, buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        let Some(path) = c_str(path) else { return 0 };
        copy_text(
            &json_reply(dspi_core::enum_value_labels(path.to_owned())),
            buf,
            len,
        )
    })
}

/// The open device's parameter-image bytes that `DspState` has no field for,
/// in JSON: {ok, decoded, i2sBckPinSlavePresent, i2sInputChannelsPresent,
/// i2sRxPinExtPresent:[3], spdifRxPinExtPresent:[3], adatOutPinPresent}.
/// Returns the bytes needed including the NUL, 0 for a null handle or a
/// panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_image_extras(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.image_extras())))
}

/// A hub's DNS-SD TXT record in JSON: {ok, hubId, name, kind, auth,
/// deviceCount, serials, path}, or {ok:false, code:"invalid"} when it is no
/// hub's. `service_name` is the instance name (null reads as empty);
/// `txt_json` a JSON object of string keys and values. Returns the bytes
/// needed including the NUL, 0 for a null `txt_json` or a panic.
#[cfg(feature = "link")]
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_parse_txt(
    service_name: *const c_char,
    txt_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let Some(txt) = c_str(txt_json) else { return 0 };
        let name = c_str(service_name).unwrap_or("").to_owned();
        let parsed = serde_json::from_str::<Value>(txt)
            .ok()
            .and_then(|v| serde_json::from_value::<serde_json::Map<String, Value>>(v).ok())
            .and_then(|m| {
                m.into_iter()
                    .map(|(key, v)| match v {
                        Value::String(value) => Some(dspi_core::LinkTxtEntry { key, value }),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| dspi_core::DspiError::InvalidArgument {
                msg: "the TXT record is not a JSON object of strings".to_owned(),
            });
        let reply = parsed.and_then(|map| dspi_core::link_parse_txt(name, map));
        copy_text(&json_reply(reply), buf, len)
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "tests_wp_cgswsa.rs"]
mod tests_wp_cgswsa;
