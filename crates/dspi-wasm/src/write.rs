//! `write` and `describe`: a parameter by registry path, as
//! `dspi_session::Session::write(path, indices, value)` takes it.
//!
//! The registry (`dspi_proto::registry`) checks the path, the indices and the
//! value, with its own range and wording. The change itself goes through the
//! core's setter for that path, which validates again, sends, and keeps the
//! core's mirror in step. The answer is the value read back: the device's
//! image is read again and the field taken from it.
//!
//! The setters this binding routes are listed in [`ROUTES`]; a valid path that
//! is not there yet is refused as "not available in this version yet". WB6's
//! slices add rows as their screens need them.

use dspi_core::{DspState, DspiCore, DspiError};
use dspi_proto::registry::{Kind, ParamDesc, by_path};
use serde::{Deserialize, Serialize};

use crate::calls::CallError;

/// A parameter value on the page side: `dspi_proto::value::Value`, tagged by
/// kind in lower case (`{"float": -12.5}`, `{"bool": true}`, `"trigger"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamValue {
    Bool(bool),
    Float(f32),
    Int(i64),
    Choice(u8),
    Mask(u32),
    Text(String),
    Bytes(Vec<u8>),
    Trigger,
}

/// `write`'s parameters.
#[derive(Debug, Deserialize)]
pub struct WriteParams {
    pub path: String,
    #[serde(default)]
    pub indices: Vec<u8>,
    pub value: ParamValue,
}

type Setter = fn(&DspiCore, &[u8], &ParamValue) -> Result<(), DspiError>;
type Reader = fn(&DspState, &[u8]) -> Option<ParamValue>;

/// The paths this binding writes: the core's setter and where the read-back
/// sits in `DspState`. Indices are already checked against the registry's
/// arity when these run.
const ROUTES: &[(&str, Setter, Reader)] = &[
    (
        "vol.user",
        |c, _, v| c.set_user_volume(float(v)),
        |s, _| Some(ParamValue::Float(s.user_volume_db)),
    ),
    (
        "vol.mute",
        |c, _, v| c.set_user_mute(boolean(v)),
        |s, _| Some(ParamValue::Bool(s.user_mute)),
    ),
    (
        "vol.master",
        |c, _, v| c.set_master_volume(float(v)),
        |s, _| Some(ParamValue::Float(s.master_volume_db)),
    ),
    (
        "pre",
        |c, _, v| c.set_preamp(float(v)),
        |s, _| Some(ParamValue::Float(s.preamp_db)),
    ),
    (
        "bypass",
        |c, _, v| c.set_bypass(boolean(v)),
        |s, _| Some(ParamValue::Bool(s.bypass)),
    ),
    (
        "out.gain",
        |c, i, v| c.set_output_gain(first(i), float(v)),
        |s, i| {
            s.output_gain_db
                .get(usize::from(first(i)))
                .copied()
                .map(ParamValue::Float)
        },
    ),
    (
        "out.mute",
        |c, i, v| c.set_output_mute(first(i), boolean(v)),
        |s, i| {
            s.output_muted
                .get(usize::from(first(i)))
                .copied()
                .map(ParamValue::Bool)
        },
    ),
    (
        "loud.on",
        |c, _, v| c.set_loudness(boolean(v)),
        |s, _| Some(ParamValue::Bool(s.loudness_enabled)),
    ),
    (
        "loud.ref",
        |c, _, v| c.set_loudness_ref(float(v)),
        |s, _| Some(ParamValue::Float(s.loudness_ref_spl)),
    ),
    (
        "loud.intensity",
        |c, _, v| c.set_loudness_intensity(float(v)),
        |s, _| Some(ParamValue::Float(s.loudness_intensity)),
    ),
];

fn float(v: &ParamValue) -> f32 {
    match v {
        ParamValue::Float(f) => *f,
        ParamValue::Int(i) => *i as f32,
        _ => f32::NAN,
    }
}

fn boolean(v: &ParamValue) -> bool {
    matches!(v, ParamValue::Bool(true))
}

fn first(i: &[u8]) -> u8 {
    i.first().copied().unwrap_or(0)
}

/// The registry's descriptor of a path, or the page's error for it.
fn descriptor(path: &str) -> Result<&'static ParamDesc, CallError> {
    by_path(path)
        .ok_or_else(|| CallError::new("unknown_param", format!("there is no parameter {path}")))
}

/// Check `value` against the registry's kind and range for `d`.
fn check_value(d: &ParamDesc, value: &ParamValue) -> Result<(), CallError> {
    let wrong = || {
        CallError::new(
            "invalid",
            format!("{} takes {}", d.label, kind_name(d.kind)),
        )
    };
    match (d.kind, value) {
        (Kind::Bool, ParamValue::Bool(_)) | (Kind::Trigger, ParamValue::Trigger) => Ok(()),
        (Kind::Float { unit, min, max }, ParamValue::Float(_) | ParamValue::Int(_)) => {
            let f = float(value);
            if f.is_finite() && (min..=max).contains(&f) {
                Ok(())
            } else {
                Err(CallError::new(
                    "out_of_range",
                    format!(
                        "{} must be between {min} and {max}{}",
                        d.label,
                        unit.suffix()
                    ),
                ))
            }
        }
        (Kind::Int { unit, min, max }, ParamValue::Int(i)) => {
            if (min..=max).contains(i) {
                Ok(())
            } else {
                Err(CallError::new(
                    "out_of_range",
                    format!(
                        "{} must be between {min} and {max}{}",
                        d.label,
                        unit.suffix()
                    ),
                ))
            }
        }
        (Kind::Choice(list), ParamValue::Choice(c)) => {
            if list.iter().any(|(n, _)| n == c) {
                Ok(())
            } else {
                Err(CallError::new(
                    "out_of_range",
                    format!("{} has no choice {c}", d.label),
                ))
            }
        }
        (Kind::Mask, ParamValue::Mask(_)) => Ok(()),
        (Kind::Text { max }, ParamValue::Text(t)) => {
            if t.len() <= max {
                Ok(())
            } else {
                Err(CallError::new(
                    "out_of_range",
                    format!("{} is at most {max} bytes", d.label),
                ))
            }
        }
        (Kind::Packet, ParamValue::Bytes(_)) => Ok(()),
        _ => Err(wrong()),
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Bool => "yes or no",
        Kind::Trigger => "no value",
        Kind::Float { .. } => "a number",
        Kind::Int { .. } => "a whole number",
        Kind::Choice(_) => "a choice",
        Kind::Mask => "a bit mask",
        Kind::Text { .. } => "text",
        Kind::Packet => "raw bytes",
        Kind::Status => "nothing (it is read only)",
    }
}

/// Write one parameter and answer what the device kept.
pub fn write(core: &DspiCore, p: &WriteParams) -> Result<ParamValue, CallError> {
    let d = descriptor(&p.path)?;
    if !d.is_writable() {
        return Err(CallError::new(
            "invalid",
            format!("{} cannot be changed", d.label),
        ));
    }
    let arity = d.target.arity();
    if p.indices.len() != arity {
        return Err(CallError::new(
            "invalid",
            format!(
                "{} takes {arity} index(es), not {}",
                p.path,
                p.indices.len()
            ),
        ));
    }
    check_value(d, &p.value)?;
    let Some((_, set, read)) = ROUTES.iter().find(|(path, _, _)| *path == p.path) else {
        return Err(CallError::new(
            "refused",
            format!("{} is not available in this version yet", d.label),
        ));
    };
    set(core, &p.indices, &p.value).map_err(CallError::from)?;
    // The read-back: the device's image again, not the value sent.
    core.refresh_params().map_err(CallError::from)?;
    read(&core.get_state(), &p.indices).ok_or_else(|| {
        CallError::new(
            "invalid",
            format!("{} has no index {:?}", p.path, p.indices),
        )
    })
}

/// `describe`: what the registry says of a path, for a control's range and
/// label.
pub fn describe(path: &str) -> Result<serde_json::Value, CallError> {
    let d = descriptor(path)?;
    let mut v = serde_json::json!({
        "path": d.path,
        "label": d.label,
        "plain": d.plain,
        "writable": d.is_writable(),
        "readable": d.is_readable(),
        "indices": d.target.arity(),
        "routed": ROUTES.iter().any(|(p, _, _)| *p == d.path),
    });
    let kind = match d.kind {
        Kind::Bool => serde_json::json!({ "kind": "bool" }),
        Kind::Trigger => serde_json::json!({ "kind": "trigger" }),
        Kind::Float { unit, min, max } => {
            serde_json::json!({
                "kind": "float",
                "unit": unit.suffix().trim(),
                "min": crate::calls::tidy(&min),
                "max": crate::calls::tidy(&max),
            })
        }
        Kind::Int { unit, min, max } => {
            serde_json::json!({ "kind": "int", "unit": unit.suffix().trim(), "min": min, "max": max })
        }
        Kind::Choice(list) => serde_json::json!({
            "kind": "choice",
            "choices": list.iter().map(|(n, l)| serde_json::json!({ "value": n, "label": l })).collect::<Vec<_>>(),
        }),
        Kind::Mask => serde_json::json!({ "kind": "mask" }),
        Kind::Text { max } => serde_json::json!({ "kind": "text", "max": max }),
        Kind::Packet => serde_json::json!({ "kind": "bytes" }),
        Kind::Status => serde_json::json!({ "kind": "status" }),
    };
    if let (Some(m), serde_json::Value::Object(k)) = (v.as_object_mut(), kind) {
        m.extend(k);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wb3_every_route_is_a_writable_registry_path() {
        for (path, _, _) in ROUTES {
            let d = by_path(path).unwrap_or_else(|| panic!("{path} is not in the registry"));
            assert!(d.is_writable(), "{path}");
        }
    }

    #[test]
    fn wb3_values_are_tagged_by_kind_in_lower_case() {
        let v: ParamValue = serde_json::from_str(r#"{"float": -12.5}"#).unwrap();
        assert_eq!(v, ParamValue::Float(-12.5));
        let t: ParamValue = serde_json::from_str(r#""trigger""#).unwrap();
        assert_eq!(t, ParamValue::Trigger);
        assert_eq!(
            serde_json::to_string(&ParamValue::Bool(true)).unwrap(),
            r#"{"bool":true}"#
        );
    }

    #[test]
    fn wb3_describe_gives_the_registry_range() {
        let d = describe("vol.user").unwrap();
        assert_eq!(d["kind"], "float");
        assert_eq!(d["min"], -60.0);
        assert_eq!(d["max"], 0.0);
        assert_eq!(d["unit"], "dB");
        assert_eq!(d["routed"], true);
        assert_eq!(describe("nope").unwrap_err().code, "unknown_param");
    }
}
