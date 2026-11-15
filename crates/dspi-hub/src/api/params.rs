//! `param.list`, `param.get`, `param.get_many`, `param.set` (WP P2PA; `hub-api.md`
//! section 9).
//!
//! Reads go through the executor's reader (`dspi_session::Session::read`: the device's
//! own answer, the layout checked); writes through the core's `write_param` (or
//! `write_param_confirmed` for a gated path, with a token the hub's core mints for an
//! Admin's `"confirm": true`), on a child session of the caller. Values are plain JSON,
//! the registry's kind decides (`hub-api.md` 1.4).

use std::sync::Arc;

use data_encoding::BASE64;
use dspi_core::{ParamValue, write_refusal};
use dspi_proto::generated::opcodes as op;
use dspi_proto::registry::{
    Hazard, Kind, ParamDesc, REGISTRY, Requires, Target, by_path, choice_alias,
};
use dspi_session::WriteError;
use dspi_session::probe::Capabilities;
use serde_json::{Value, json};

use super::exec::{Cx, Executor};
use super::{ApiError, no_foreign_guard, require, target};
use crate::hub::HubHandle;
use crate::session::{Role, Session};

/// The most items one `param.get_many` reads.
pub const MAX_GET_MANY: usize = 64;

pub(super) fn handle(
    exec: &Executor,
    hub: &HubHandle,
    session: &Arc<Session>,
    t: &str,
    msg: &Value,
) -> Result<Value, ApiError> {
    let (serial, handle) = target(hub, msg)?;
    match t {
        "param.list" => exec.run(hub, session, &serial, list),
        "param.get" => {
            let (d, idx) = path_and_idx(msg)?;
            exec.run(hub, session, &serial, |cx| get(cx, d, &idx))
        }
        "param.get_many" => {
            let items = msg
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| ApiError::bad("items is required: [{path, idx?}]"))?;
            if items.len() > MAX_GET_MANY {
                return Err(ApiError::bad(format!(
                    "at most {MAX_GET_MANY} items in one param.get_many"
                )));
            }
            exec.run(hub, session, &serial, |cx| {
                let values: Vec<Value> = items.iter().map(|item| get_item(cx, item)).collect();
                Ok(json!({"serial": cx.serial(), "values": values}))
            })
        }
        // "param.set"
        _ => {
            require(session, Role::Control)?;
            let (d, idx) = path_and_idx(msg)?;
            let value = value_in(d, msg.get("value"))?;
            let confirm = msg.get("confirm").and_then(Value::as_bool) == Some(true);
            let gated = dspi_core::confirm::gated_path(d.path);
            if gated.is_some() {
                require(session, Role::Admin)?;
                if !confirm {
                    return Err(ApiError::new(
                        "confirmation_required",
                        format!("{} is written only with \"confirm\": true", d.path),
                    ));
                }
            }
            no_foreign_guard(hub, handle, session.id())?;
            exec.run(hub, session, &serial, |cx| {
                let core = cx.core()?;
                let back = match gated {
                    Some(op) => {
                        let token = core.confirm_token(op);
                        core.write_param_confirmed(
                            d.path.to_owned(),
                            idx.clone(),
                            value,
                            Some(token),
                        )
                    }
                    None => core.write_param(d.path.to_owned(), idx.clone(), value),
                }
                .map_err(ApiError::from)?;
                let mut out = json!({"serial": cx.serial(), "path": d.path, "idx": idx});
                put_value(&mut out, d, back);
                Ok(out)
            })
        }
    }
}

/// The request's `path` (a registry path) and `idx` (as many as the path has).
fn path_and_idx(msg: &Value) -> Result<(&'static ParamDesc, Vec<u8>), ApiError> {
    let path = msg
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("path is required"))?;
    let d = by_path(path).ok_or_else(|| ApiError::bad(format!("there is no parameter {path}")))?;
    let idx = match msg.get("idx") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|i| i.as_u64().and_then(|i| u8::try_from(i).ok()))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| ApiError::bad("idx holds whole numbers 0 to 255"))?,
        Some(_) => return Err(ApiError::bad("idx is an array of whole numbers")),
    };
    let want = d.target.arity();
    if idx.len() != want {
        return Err(ApiError::bad(format!(
            "{path} takes {want} index(es) in idx, not {}",
            idx.len()
        )));
    }
    Ok((d, idx))
}

/// Why the API does not write `d`, naming what does; `None` when it does. Beyond the
/// core's own refusals ([`write_refusal`]): raw bytes, the extension opcode, and
/// `meters.clear` (the clip latch's clear, which would clear every console's clip
/// indicator: the hub never sends it, PLAN-P2 section 1).
pub(crate) fn api_refusal(d: &ParamDesc) -> Option<String> {
    if !d.is_writable() {
        return Some(format!("{} is read-only", d.path));
    }
    if let Some(why) = write_refusal(d.path) {
        return Some(why);
    }
    if d.path == "meters.clear" {
        return Some(
            "meters.clear is not written by the hub: it clears every console's clip indicator; clear them from a console"
                .into(),
        );
    }
    if d.set == Some(op::REQ_EXTENSION) {
        return Some(format!(
            "{} is an extension request, which the API does not send",
            d.path
        ));
    }
    if matches!(d.kind, Kind::Packet | Kind::Status) {
        return Some(format!(
            "{} takes raw bytes, which the API does not write",
            d.path
        ));
    }
    None
}

/// A float as a person would write it (`f32` widened gives `0.10000000149`).
fn tidy(f: f32) -> Value {
    format!("{f}")
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn choice_word(d: &ParamDesc, n: u8) -> Option<&'static str> {
    match d.kind {
        Kind::Choice(list) => list.iter().find(|(v, _)| *v == n).map(|(_, l)| *l),
        _ => None,
    }
}

/// `value` (and `choice` or `value_b64`) into `out`, as `hub-api.md` 1.4 answers them.
fn put_value(out: &mut Value, d: &ParamDesc, v: ParamValue) {
    let Some(map) = out.as_object_mut() else {
        return;
    };
    let mut set = |k: &str, v: Value| {
        map.insert(k.to_owned(), v);
    };
    match v {
        ParamValue::Bool(b) => set("value", json!(b)),
        ParamValue::Float(f) => set("value", tidy(f)),
        ParamValue::Int(i) => set("value", json!(i)),
        ParamValue::Choice(c) => {
            set("value", json!(c));
            if let Some(word) = choice_word(d, c) {
                set("choice", json!(word));
            }
        }
        ParamValue::Mask(m) => set("value", json!(m)),
        ParamValue::Text(t) => set("value", json!(t.trim_end_matches('\0'))),
        ParamValue::Bytes(b) => set("value_b64", json!(BASE64.encode(&b))),
        ParamValue::Trigger => set("value", Value::Null),
    }
}

/// A request's `value` as `d`'s kind takes it, inside the registry's range.
fn value_in(d: &ParamDesc, v: Option<&Value>) -> Result<ParamValue, ApiError> {
    if let Some(why) = api_refusal(d) {
        let e = ApiError::bad(why);
        return Err(match write_refusal(d.path) {
            Some(_) => e.with("use", json!(write_refusal(d.path))),
            None => e,
        });
    }
    let v = v.unwrap_or(&Value::Null);
    let wrong = |what: &str| ApiError::bad(format!("{} ({}) takes {what}", d.label, d.path));
    match d.kind {
        Kind::Trigger => match v {
            Value::Null => Ok(ParamValue::Trigger),
            _ => Err(wrong("no value")),
        },
        Kind::Bool => v
            .as_bool()
            .map(ParamValue::Bool)
            .ok_or_else(|| wrong("true or false")),
        Kind::Float { unit, min, max } => {
            let f = v
                .as_f64()
                .filter(|f| f.is_finite())
                .ok_or_else(|| wrong("a number"))?;
            #[allow(clippy::cast_possible_truncation)] // checked against the range next
            let f = f as f32;
            // A delay's range is the connected platform's line, which the core checks
            // (CG37); the registry's is the schema's ceiling.
            let delay = matches!(d.path, "out.delay" | "ch.delay");
            if !f.is_finite() || (!delay && !(min..=max).contains(&f)) {
                return Err(ApiError::bad(format!(
                    "{} must be between {min} and {max}{}",
                    d.label,
                    unit.suffix()
                ))
                .with("min", tidy(min))
                .with("max", tidy(max)));
            }
            Ok(ParamValue::Float(f))
        }
        Kind::Int { unit, min, max } => {
            let i = v.as_i64().ok_or_else(|| wrong("a whole number"))?;
            if !(min..=max).contains(&i) {
                return Err(ApiError::bad(format!(
                    "{} must be between {min} and {max}{}",
                    d.label,
                    unit.suffix()
                ))
                .with("min", json!(min))
                .with("max", json!(max)));
            }
            Ok(ParamValue::Int(i))
        }
        Kind::Choice(list) => {
            let n = match v {
                Value::String(word) => list
                    .iter()
                    .find(|(_, l)| l.eq_ignore_ascii_case(word))
                    .map(|(n, _)| *n)
                    .or_else(|| choice_alias(d.path, word))
                    .ok_or_else(|| ApiError::bad(format!("{} has no choice {word}", d.label)))?,
                _ => v
                    .as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| wrong("a choice's word or number"))?,
            };
            // eq.type's choices depend on the band (a crossover band takes its own);
            // the core checks those.
            if d.path != "eq.type" && !list.iter().any(|(c, _)| *c == n) {
                return Err(
                    ApiError::bad(format!("{} has no choice {n}", d.label)).with(
                        "choices",
                        json!(
                            list.iter()
                                .map(|(n, l)| json!({"value": n, "label": l}))
                                .collect::<Vec<_>>()
                        ),
                    ),
                );
            }
            Ok(ParamValue::Choice(n))
        }
        Kind::Mask => v
            .as_u64()
            .and_then(|m| u32::try_from(m).ok())
            .map(ParamValue::Mask)
            .ok_or_else(|| wrong("a whole number bit mask")),
        Kind::Text { max } => {
            let t = v.as_str().ok_or_else(|| wrong("text"))?;
            if t.len() > max {
                return Err(ApiError::bad(format!("{} is at most {max} bytes", d.label))
                    .with("max", json!(max)));
            }
            Ok(ParamValue::Text(t.to_owned()))
        }
        // Refused by `api_refusal` above.
        Kind::Packet | Kind::Status => Err(wrong("raw bytes, which the API does not write")),
    }
}

/// A read's failure, as this API's code.
fn read_error(e: WriteError) -> ApiError {
    match e {
        WriteError::Transport(t) => ApiError::transport(t),
        WriteError::Unavailable { .. } => ApiError::new("unsupported", e.to_string()),
        other => ApiError::bad(other.to_string()),
    }
}

fn get(cx: &mut Cx<'_>, d: &'static ParamDesc, idx: &[u8]) -> Result<Value, ApiError> {
    if !d.is_readable() {
        return Err(ApiError::bad(format!("{} cannot be read", d.path)));
    }
    let serial = cx.serial().to_owned();
    let reader = cx.reader()?;
    let v = reader.read(d.path, idx).map_err(read_error)?;
    let mut out = json!({"serial": serial, "path": d.path, "idx": idx});
    put_value(&mut out, d, ParamValue::from(v));
    Ok(out)
}

fn get_item(cx: &mut Cx<'_>, item: &Value) -> Value {
    let path = item.get("path").cloned().unwrap_or(Value::Null);
    let result = path_and_idx(item).and_then(|(d, idx)| get(cx, d, &idx));
    match result {
        Ok(mut v) => {
            if let Some(m) = v.as_object_mut() {
                m.remove("serial");
                m.insert("ok".into(), json!(true));
            }
            v
        }
        Err(e) => {
            let mut v = json!({"path": path, "idx": item.get("idx").cloned().unwrap_or(json!([])),
                "ok": false, "code": e.code, "msg": e.msg});
            if let Some(m) = v.as_object_mut() {
                m.extend(e.extra);
            }
            v
        }
    }
}

fn hazard_name(h: Hazard) -> &'static str {
    match h {
        Hazard::None => "none",
        Hazard::Audible => "audible",
        Hazard::Deferred => "deferred",
        Hazard::Flash => "flash",
        Hazard::Reconfig => "reconfig",
        Hazard::Irreversible => "irreversible",
    }
}

/// Whether the device offers `d`.
fn offered(caps: &Capabilities, d: &ParamDesc) -> bool {
    match d.requires {
        Requires::Always => true,
        Requires::Rp2350 => caps.platform == dspi_proto::Platform::Rp2350,
        Requires::Feature(name) => caps.features.iter().any(|f| f.name == name && f.present),
    }
}

/// The index ranges of `target` on this device; a count of `null` where the device
/// alone knows (the firmware range-checks those).
fn layout(caps: &Capabilities, target: Target) -> Value {
    let one = |name: &str, count: Option<u8>| json!({"name": name, "count": count});
    let cs = caps.cs.as_ref();
    match target {
        Target::None => json!([]),
        Target::Channel => json!([one("channel", Some(caps.num_channels))]),
        Target::Input => json!([one("input", Some(caps.num_inputs))]),
        Target::Output | Target::OutputOrAll => json!([one("output", Some(caps.num_outputs))]),
        Target::ChannelBand => json!([
            one("channel", Some(caps.num_channels)),
            one("band", Some(caps.max_bands))
        ]),
        Target::Crosspoint => json!([
            one("input", Some(caps.num_inputs)),
            one("output", Some(caps.num_outputs))
        ]),
        Target::PresetSlot => json!([one("slot", Some(10))]),
        Target::CsSlot => json!([one("binding", cs.map(|c| c.max_bindings))]),
        Target::CsIrSlot => json!([one("ir_command", cs.map(|c| c.max_ir_commands))]),
        Target::CsGroup => json!([one("group", cs.map(|c| c.max_groups))]),
        Target::CsMacro => json!([one("macro", cs.map(|c| c.max_macros))]),
        Target::CsMacroStep => json!([
            one("macro", cs.map(|c| c.max_macros)),
            one("step", cs.map(|c| c.max_macro_steps))
        ]),
        Target::CsDisplayPage => json!([one("page", Some(16))]),
        Target::InputSource => json!([one("source", Some(7))]),
        Target::SpdifInput | Target::SpdifExtraInput => json!([one("spdif_input", Some(4))]),
        Target::Slot => json!([one("slot", None)]),
        Target::PinOutput => json!([one("pin", None)]),
        Target::I2sPair => json!([one("i2s_pair", None)]),
        Target::UpmixParam => json!([one("param", None)]),
        Target::TapChannel => json!([one("channel", None)]),
    }
}

/// One `param.list` row (the web binding's `describe` row, plus the device's layout).
fn row(caps: &Capabilities, d: &ParamDesc) -> Value {
    let refusal = api_refusal(d);
    let mut v = json!({
        "path": d.path,
        "label": d.label,
        "plain": d.plain,
        "readable": d.is_readable(),
        "writable": d.is_writable(),
        "routed": d.is_writable() && refusal.is_none(),
        "indices": d.target.arity(),
        "layout": layout(caps, d.target),
        "hazard": hazard_name(d.hazard),
        "gated": dspi_core::confirm::gated_path(d.path).is_some(),
    });
    let kind = match d.kind {
        Kind::Bool => json!({"kind": "bool"}),
        Kind::Trigger => json!({"kind": "trigger"}),
        Kind::Float { unit, min, max } => json!({
            "kind": "float", "unit": unit.suffix().trim(), "min": tidy(min), "max": tidy(max),
        }),
        Kind::Int { unit, min, max } => {
            json!({"kind": "int", "unit": unit.suffix().trim(), "min": min, "max": max})
        }
        Kind::Choice(list) => json!({
            "kind": "choice",
            "choices": list.iter().map(|(n, l)| json!({"value": n, "label": l})).collect::<Vec<_>>(),
        }),
        Kind::Mask => json!({"kind": "mask"}),
        Kind::Text { max } => json!({"kind": "text", "max": max}),
        Kind::Packet => json!({"kind": "bytes"}),
        Kind::Status => json!({"kind": "status"}),
    };
    if let (Some(m), Value::Object(k)) = (v.as_object_mut(), kind) {
        m.extend(k);
        if d.is_writable()
            && let Some(why) = refusal
        {
            m.insert("use".into(), json!(why));
        }
    }
    v
}

fn list(cx: &mut Cx<'_>) -> Result<Value, ApiError> {
    let serial = cx.serial().to_owned();
    let caps = cx.reader()?.capabilities();
    let params: Vec<Value> = REGISTRY
        .iter()
        .filter(|d| offered(caps, d))
        .map(|d| row(caps, d))
        .collect();
    Ok(json!({"serial": serial, "params": params}))
}

#[cfg(test)]
mod tests_wp_p2pa {
    use super::*;

    fn d(path: &str) -> &'static ParamDesc {
        by_path(path).unwrap()
    }

    #[test]
    fn wp_p2pa_values_are_checked_by_kind_and_range() {
        let ok = |p: &str, v: Value| value_in(d(p), Some(&v));
        assert_eq!(
            ok("vol.master", json!(-20.0)).unwrap(),
            ParamValue::Float(-20.0)
        );
        let e = ok("vol.master", json!(40.0)).unwrap_err();
        assert_eq!(e.code, "bad_request");
        assert!(
            e.extra.contains_key("min") && e.extra.contains_key("max"),
            "{e:?}"
        );
        assert_eq!(ok("vol.mute", json!(true)).unwrap(), ParamValue::Bool(true));
        assert_eq!(ok("vol.mute", json!(1)).unwrap_err().code, "bad_request");
        assert_eq!(
            ok("lev.speed", json!("fast")).unwrap(),
            ParamValue::Choice(2)
        );
        assert_eq!(ok("lev.speed", json!(1)).unwrap(), ParamValue::Choice(1));
        assert_eq!(
            ok("lev.speed", json!("warp")).unwrap_err().code,
            "bad_request"
        );
        assert_eq!(
            ok("in.source", json!("link")).unwrap(),
            ParamValue::Choice(7)
        );
    }

    #[test]
    fn wp_p2pa_raw_bytes_the_extension_and_clip_clearing_are_refused() {
        for path in ["meters.clear", "bulk", "dev.reset", "mix"] {
            let e = value_in(d(path), Some(&Value::Null)).unwrap_err();
            assert_eq!(e.code, "bad_request", "{path}: {e:?}");
        }
        assert!(api_refusal(d("vol.master")).is_none());
        // No registry path reaches the core voltage at all.
        assert!(
            REGISTRY
                .iter()
                .filter(|d| d.set == Some(op::REQ_EXTENSION))
                .all(|d| api_refusal(d).is_some())
        );
    }

    #[test]
    fn wp_p2pa_idx_must_match_the_path() {
        let e = path_and_idx(&json!({"path": "out.gain"})).unwrap_err();
        assert_eq!(e.code, "bad_request");
        let (_, idx) = path_and_idx(&json!({"path": "eq.freq", "idx": [0, 4]})).unwrap();
        assert_eq!(idx, vec![0, 4]);
        assert_eq!(
            path_and_idx(&json!({"path": "no.such"})).unwrap_err().code,
            "bad_request"
        );
    }

    #[test]
    fn wp_p2pa_floats_answer_as_written() {
        assert_eq!(tidy(0.1), json!(0.1));
        assert_eq!(tidy(-12.5), json!(-12.5));
    }
}
