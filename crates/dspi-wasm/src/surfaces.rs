//! WP WB6l: the control-surface editor and the quick strip on the web, over
//! the core's control-surface calls (WP C14, CG20: `DspiCore::cs_*`).
//!
//! The editor's calls are the core's JSON, passed through unchanged:
//!
//! - `csFetch`: read the whole configuration from the device, then answer
//!   `csSnapshot`. Reads only: allowed while read only.
//! - `csSnapshot`: the last read (`{supported, ...}`, `DspiCore::cs_snapshot`).
//! - `csApply {op}`: one operation (`DspiCore::cs_apply`): `{ok, status}` or
//!   `{ok: false, error}`. A write on a read-only connection, in the client
//!   read-only mode or under the installer's lock is refused there, before
//!   anything is sent; that refusal is the reply, as every other.
//! - `csDescribe {kind, slot, draft}`, `csEdit {kind, slot, draft, change}`,
//!   `csNewBinding {slot, component}`, `csNewRecord {kind}`: the editing rules
//!   (CG20). No device I/O.
//! - `csStatusTexts`: every status code's sentence, `{code: text}` (1..0x3F).
//! - `csDemoLearn {protocol, code}`, `csDemoBusy {on}`: the demo device's
//!   remote press and held revert; `unsupported` on any other device.
//!
//! The quick strip (M60): `quickStrip` answers, for each effect switch the
//! Console's strip has, its registry path, whether the open device has it
//! (the path's requirement in the registry: a probed feature name or the
//! platform) and whether it is on, `[{id, path, present, on}]`. The page
//! switches one by `write` of that path.

use dspi_core::DspiCore;
use dspi_proto::registry::{Requires, by_path};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

type CallResult = Result<Value, CallError>;

/// The status codes `csStatusTexts` names (`CS_STATUS_*` stay under 0x40).
const STATUS_CODES: std::ops::Range<u8> = 1..0x40;

/// The quick strip's effect switches, in the Console's order
/// (`ContentView.swift:575-602`): its id and its registry path.
pub const STRIP: [(&str, &str); 5] = [
    ("crossfeed", "cf.on"),
    ("loudness", "loud.on"),
    ("leveller", "lev.on"),
    ("psybass", "bass.on"),
    ("bypass", "bypass"),
];

#[derive(Deserialize)]
struct ApplyParams {
    op: Value,
}

#[derive(Deserialize)]
struct DescribeParams {
    kind: String,
    slot: u32,
    draft: Value,
}

#[derive(Deserialize)]
struct EditParams {
    kind: String,
    slot: u32,
    draft: Value,
    change: Value,
}

#[derive(Deserialize)]
struct NewBindingParams {
    slot: u32,
    component: u8,
}

#[derive(Deserialize)]
struct NewRecordParams {
    kind: String,
}

#[derive(Deserialize)]
struct LearnParams {
    protocol: u8,
    code: u32,
}

#[derive(Deserialize)]
struct On {
    on: bool,
}

fn params<T: for<'de> Deserialize<'de>>(p: Value) -> Result<T, CallError> {
    serde_json::from_value(p).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// The core's JSON reply as a value; text that is not JSON (never expected)
/// is the binding's own error.
fn parsed(text: &str) -> CallResult {
    serde_json::from_str(text)
        .map_err(|e| CallError::new("internal", format!("the core's reply is not JSON: {e}")))
}

/// Whether the open device has what the registry says `path` needs.
fn present(core: &DspiCore, path: &str) -> bool {
    let Some(d) = by_path(path) else {
        return false;
    };
    if !core.is_connected() {
        return false;
    }
    match d.requires {
        Requires::Always => true,
        Requires::Rp2350 => core.get_state().platform_id == 1,
        Requires::Feature(name) => core.feature_present(name),
    }
}

/// `quickStrip`: the strip's switches on the open device.
pub fn quick_strip(core: &DspiCore) -> Value {
    let s = core.get_state();
    let rows: Vec<Value> = STRIP
        .iter()
        .map(|&(id, path)| {
            let on = match id {
                "crossfeed" => s.crossfeed_enabled,
                "loudness" => s.loudness_enabled,
                "leveller" => s.leveller_enabled,
                "psybass" => s.psybass_enabled,
                _ => s.bypass,
            };
            json!({ "id": id, "path": path, "present": present(core, path), "on": on })
        })
        .collect();
    Value::Array(rows)
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "csFetch" => core
            .cs_fetch()
            .map_err(CallError::from)
            .and_then(|()| parsed(&core.cs_snapshot())),
        "csSnapshot" => parsed(&core.cs_snapshot()),
        "csApply" => params::<ApplyParams>(p).and_then(|a| {
            if !a.op.is_object() {
                return Err(CallError::new("invalid", "an operation is a JSON object"));
            }
            parsed(&core.cs_apply(a.op.to_string()))
        }),
        "csDescribe" => params::<DescribeParams>(p)
            .and_then(|d| parsed(&core.cs_describe(d.kind, d.slot, d.draft.to_string()))),
        "csEdit" => params::<EditParams>(p).and_then(|e| {
            parsed(&core.cs_edit(e.kind, e.slot, e.draft.to_string(), e.change.to_string()))
        }),
        "csNewBinding" => params::<NewBindingParams>(p)
            .and_then(|b| parsed(&core.cs_new_binding(b.slot, b.component))),
        "csNewRecord" => {
            params::<NewRecordParams>(p).and_then(|r| parsed(&core.cs_new_record(r.kind)))
        }
        "csStatusTexts" => {
            let mut out = serde_json::Map::new();
            for code in STATUS_CODES {
                out.insert(code.to_string(), Value::String(core.cs_status_text(code)));
            }
            Ok(Value::Object(out))
        }
        "csDemoLearn" => params::<LearnParams>(p).and_then(|l| {
            core.cs_demo_learn(l.protocol, l.code)
                .map(Value::Bool)
                .map_err(Into::into)
        }),
        "csDemoBusy" => params::<On>(p).and_then(|o| {
            core.cs_demo_busy(o.on)
                .map(|()| Value::Null)
                .map_err(Into::into)
        }),
        "quickStrip" => Ok(quick_strip(core)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_wb6l {
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

    fn demo() -> u32 {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        h
    }

    #[test]
    fn wp_wb6l_fetch_describe_edit_and_apply_a_binding_on_the_demo() {
        let h = demo();
        let snap = run(h, "csFetch", Value::Null);
        assert_eq!(snap["value"]["supported"], true, "{snap}");
        let b0 = snap["value"]["bindings"][0].clone();
        let d = run(
            h,
            "csDescribe",
            json!({ "kind": "binding", "slot": 0, "draft": b0 }),
        );
        assert_eq!(d["value"]["ok"], true, "{d}");
        assert!(d["value"]["rows"].as_array().is_some_and(|r| !r.is_empty()));

        // A fresh button in the first free slot, applied through the core.
        let free = snap["value"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .position(|b| b["type"] == 0)
            .unwrap();
        let made = run(h, "csNewBinding", json!({ "slot": free, "component": 1 }));
        assert_eq!(made["value"]["ok"], true, "{made}");
        let draft = made["value"]["draft"].clone();
        let r = run(
            h,
            "csApply",
            json!({ "op": { "op": "binding", "index": free, "binding": draft } }),
        );
        assert_eq!(r["value"]["ok"], true, "{r}");
        let after = run(h, "csSnapshot", Value::Null);
        assert_eq!(after["value"]["bindings"][free]["type"], 1);
        assert_eq!(after["value"]["status"]["dirty"], true);

        // A bad draft is the core's refusal, in its JSON.
        let bad = run(
            h,
            "csDescribe",
            json!({ "kind": "nope", "slot": 0, "draft": {} }),
        );
        assert_eq!(bad["value"]["ok"], false, "{bad}");
        let texts = run(h, "csStatusTexts", Value::Null);
        assert!(texts["value"]["30"].as_str().is_some_and(|t| !t.is_empty()));
    }

    #[test]
    fn wp_wb6l_a_write_in_the_read_only_mode_is_refused_before_anything_is_sent() {
        let h = demo();
        run(h, "csFetch", Value::Null);
        assert_eq!(
            run(h, "setClientReadOnly", json!({ "on": true }))["ok"],
            true
        );
        let r = run(h, "csApply", json!({ "op": { "op": "save" } }));
        assert_eq!(r["value"]["ok"], false, "{r}");
        assert!(r["value"]["error"].is_string());
        // Reads go on.
        assert_eq!(
            run(h, "csApply", json!({ "op": { "op": "refreshStatus" } }))["value"]["ok"],
            true
        );
        let strip = run(h, "quickStrip", Value::Null);
        assert_eq!(strip["value"].as_array().map(Vec::len), Some(5));
        // The strip's switch is a write of its path, which the mode refuses too.
        let w = run(
            h,
            "write",
            json!({ "path": "cf.on", "value": { "bool": true } }),
        );
        assert_eq!(w["code"], "read_only", "{w}");
    }

    #[test]
    fn wp_wb6l_the_quick_strip_follows_the_registry_paths() {
        let h = demo();
        let strip = run(h, "quickStrip", Value::Null)["value"].clone();
        let row = |id: &str| {
            strip
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(row("psybass")["path"], "bass.on");
        assert_eq!(
            row("psybass")["present"],
            true,
            "the demo has psychoacoustic_bass"
        );
        assert_eq!(row("bypass")["present"], true);
        let was = row("crossfeed")["on"].as_bool().unwrap();
        let w = run(
            h,
            "write",
            json!({ "path": "cf.on", "value": { "bool": !was } }),
        );
        assert_eq!(w["ok"], true, "{w}");
        let strip = run(h, "quickStrip", Value::Null)["value"].clone();
        let now = strip
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "crossfeed")
            .unwrap()["on"]
            .clone();
        assert_eq!(now, !was);
        // Nothing open: nothing present.
        guard::reset_for_test();
        let none = session_new().unwrap();
        let strip = run(none, "quickStrip", Value::Null)["value"].clone();
        assert!(
            strip
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["present"] == false)
        );
    }

    #[test]
    fn wp_wb6l_a_demo_learn_ends_with_the_pressed_code() {
        let h = demo();
        run(h, "csFetch", Value::Null);
        let arm = run(h, "csApply", json!({ "op": { "op": "learnArm" } }));
        assert_eq!(arm["value"]["ok"], true, "{arm}");
        assert_eq!(
            run(
                h,
                "csDemoLearn",
                json!({ "protocol": 1, "code": 0x00FF_A25D_u32 })
            )["value"],
            true
        );
        let read = run(h, "csApply", json!({ "op": { "op": "learnRead" } }));
        assert_eq!(read["value"]["state"], 2, "{read}");
        assert_eq!(read["value"]["code"], 0x00FF_A25D_u32);
    }
}
