//! `write` and `describe`: a parameter by registry path, as
//! `dspi_session::Session::write(path, indices, value)` takes it.
//!
//! The registry (`dspi_proto::registry`) checks the path, the indices and the
//! value here first, with its own range and wording. The change itself is
//! the core's `DspiCore::write_param` (WP CG32), which checks again (each
//! index against the device too), goes through the core's typed setter for
//! the path and answers the value read back from the core's mirror. A path
//! the core writes only through a flow of its own (`dspi_core::write_refusal`)
//! is refused in the core's words; `describe.routed` says which.

use dspi_core::{DspiCore, write_refusal};
use dspi_proto::registry::{Kind, ParamDesc, by_path};
use serde::Deserialize;

use crate::calls::CallError;

/// A parameter value on the page side: the core's own, tagged by kind in
/// lower case (`{"float": -12.5}`, `{"bool": true}`, `"trigger"`).
pub use dspi_core::ParamValue;

/// `write`'s parameters.
#[derive(Debug, Deserialize)]
pub struct WriteParams {
    pub path: String,
    #[serde(default)]
    pub indices: Vec<u8>,
    pub value: ParamValue,
}

fn float(v: &ParamValue) -> f32 {
    match v {
        ParamValue::Float(f) => *f,
        ParamValue::Int(i) => *i as f32,
        _ => f32::NAN,
    }
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
    if let Some(why) = write_refusal(d.path) {
        return Err(CallError::new("refused", why));
    }
    // The read-back is the core's: the mirror once its setter is done.
    core.write_param(p.path.clone(), p.indices.clone(), p.value.clone())
        .map_err(CallError::from)
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
        "routed": d.is_writable() && write_refusal(d.path).is_none(),
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
        // -- WP CG32 -- the routes are the core's: every writable path but
        // the ones a flow of their own writes.
        assert_eq!(describe("vol.user").unwrap()["routed"], true);
        assert_eq!(describe("tube.drive").unwrap()["routed"], true);
        assert_eq!(describe("dev.reset").unwrap()["routed"], false);
        assert_eq!(describe("cs.binding").unwrap()["routed"], false);
        assert_eq!(describe("meters").unwrap()["routed"], false, "read only");
    }

    // -- WP CG32 --
    fn put(
        core: &DspiCore,
        path: &str,
        indices: &[u8],
        value: ParamValue,
    ) -> Result<ParamValue, CallError> {
        write(
            core,
            &WriteParams {
                path: path.into(),
                indices: indices.to_vec(),
                value,
            },
        )
    }

    #[test]
    fn wp_cg32_paths_the_table_did_not_route_are_written_and_read_back() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        // An effect, through its table and the setter's clamp.
        assert_eq!(
            put(&core, "tube.drive", &[], ParamValue::Float(6.0)).unwrap(),
            ParamValue::Float(6.0)
        );
        // One field of a band: the rest of the band stays.
        let before = core.get_state().filters[2][3];
        assert_eq!(
            put(&core, "eq.freq", &[2, 3], ParamValue::Float(1234.0)).unwrap(),
            ParamValue::Float(1234.0)
        );
        let after = core.get_state().filters[2][3];
        assert_eq!(
            (after.freq, after.q, after.filter_type),
            (1234.0, before.q, before.filter_type)
        );
        // A choice and a crosspoint packet.
        assert_eq!(
            put(&core, "lev.speed", &[], ParamValue::Choice(2)).unwrap(),
            ParamValue::Choice(2)
        );
        let packet = dspi_proto::packets::MatrixRoutePacket {
            input: 0,
            output: 1,
            enabled: true,
            phase_invert: true,
            gain_db: -6.0,
        };
        let back = put(
            &core,
            "mix",
            &[0, 1],
            ParamValue::Bytes(packet.encode().to_vec()),
        )
        .unwrap();
        assert_eq!(back, ParamValue::Bytes(packet.encode().to_vec()));
        assert!(core.get_state().matrix_invert[0][1]);
        // A path a flow of its own writes is refused in the core's words.
        let e = put(&core, "dev.reset", &[], ParamValue::Trigger).unwrap_err();
        assert_eq!(e.code, "refused");
        assert!(e.message.contains("factory_reset"), "{}", e.message);
        // An index the device lacks: the core's check, not the setter's.
        let e = put(&core, "out.mute", &[200], ParamValue::Bool(true)).unwrap_err();
        assert_eq!(e.code, "invalid");
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

    // -- WP CG34 --
    #[test]
    fn wp_cg34_a_linkwitz_band_is_written_by_path_from_the_page() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        put(&core, "eq.type", &[2, 4], ParamValue::Choice(11)).unwrap();
        assert_eq!(
            put(&core, "eq.fp", &[2, 4], ParamValue::Float(28.0)).unwrap(),
            ParamValue::Float(28.0)
        );
        assert_eq!(
            put(&core, "eq.qp", &[2, 4], ParamValue::Float(0.6)).unwrap(),
            ParamValue::Float(307.0 / 512.0)
        );
        let b = core.get_state().filters[2][4];
        assert_eq!((b.filter_type, b.gain, b.qp), (11, 28.0, 307.0 / 512.0));
        // The page's own range check names the firmware's limits first.
        let e = put(&core, "eq.fp", &[2, 4], ParamValue::Float(5.0)).unwrap_err();
        assert_eq!(e.code, "out_of_range");
        // A dB value cannot land in the target frequency.
        let e = put(&core, "eq.gain", &[2, 4], ParamValue::Float(3.0)).unwrap_err();
        assert!(e.message.contains("eq.fp"), "{}", e.message);
        assert_eq!(core.get_state().filters[2][4].gain, 28.0);
    }
}
