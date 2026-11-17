//! The hub's measurement documents on the web (WP P5HUB), over
//! `dspi_core::hub_server::measure` (`docs/integrator/hub-api.md` section
//! 22), through this session's DSPi Link. The web keeps no store of its own:
//! the hub's is where its documents live.
//!
//! - `hubMeasureDocs {kind?, serial?}`: the hub's documents of `kind`
//!   (`measurement`, `mic`, `target`; "" every kind), measurements of
//!   `serial` ("" the open device's): `[{id, kind, name, note, serial,
//!   savedMs, bytes}]`.
//! - `hubMeasureDocGet {id}`: `{doc, document}` (`document` the compact text;
//!   a large one read in parts and checked).
//! - `hubMeasureDocPut {text}`: `{doc, created}`; a text that is not a
//!   measurement, mic or target document is `invalid` with nothing sent.
//! - `hubMeasureDocRename {id, name}`, `hubMeasureDocNote {id, note}`: the
//!   entry; `hubMeasureDocDelete {id}` (Admin).
//! - `measureDocsMerged {kind?, serial?}`: `{entries: [{doc, where,
//!   localCopy}], hubListed, hubNoticeKey}` (on the web the hub's rows only).
//! - The hub's `measure.doc.changed` comes through `hubServerEvents` (and
//!   `linkTick`) with `measureDoc` and `measureKind`.
//!
//! Without the cap `hub_measure` every call answers `unsupported` and sends
//! nothing. The hub's refusals carry the core's `noticeKey`
//! (`dlink.not_found`, `dlink.conflict`, `dlink.busy`, ...).

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
struct List {
    kind: String,
    serial: String,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
struct Put {
    text: String,
}

#[derive(Deserialize)]
struct Rename {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Note {
    id: String,
    note: String,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "hubMeasureDocs" => {
            params::<List>(p).and_then(|x| record(core.hub_measure_docs(x.kind, x.serial)))
        }
        "hubMeasureDocGet" => params::<Id>(p).and_then(|x| record(core.hub_measure_doc_get(x.id))),
        "hubMeasureDocPut" => {
            params::<Put>(p).and_then(|x| record(core.hub_measure_doc_put(x.text)))
        }
        "hubMeasureDocRename" => {
            params::<Rename>(p).and_then(|x| record(core.hub_measure_doc_rename(x.id, x.name)))
        }
        "hubMeasureDocNote" => {
            params::<Note>(p).and_then(|x| record(core.hub_measure_doc_note(x.id, x.note)))
        }
        "hubMeasureDocDelete" => params::<Id>(p)
            .and_then(|x| record(core.hub_measure_doc_delete(x.id).map(|()| Value::Null))),
        "measureDocsMerged" => {
            params::<List>(p).and_then(|x| record(core.measure_docs_merged(x.kind, x.serial)))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p5hub {
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

    /// Off a hub: the core's own answers, on the web.
    #[test]
    fn wp_p5hub_the_hub_measure_calls_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let caps = run(h, "hubCaps", Value::Null);
        assert_eq!(caps["value"]["hubMeasure"], false, "{caps}");
        for (m, p) in [
            ("hubMeasureDocs", Value::Null),
            ("hubMeasureDocGet", json!({"id": "1-0"})),
            ("hubMeasureDocRename", json!({"id": "1-0", "name": "n"})),
            ("hubMeasureDocNote", json!({"id": "1-0", "note": "n"})),
            ("hubMeasureDocDelete", json!({"id": "1-0"})),
        ] {
            let r = run(h, m, p);
            assert_eq!(r["ok"], false, "{m}: {r}");
            assert_eq!(r["code"], "unsupported", "{m}: {r}");
        }
        let r = run(h, "hubMeasureDocPut", json!({"text": "{}"}));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "measureDocsMerged", Value::Null);
        assert_eq!(r["value"]["entries"], json!([]), "{r}");
        assert_eq!(r["value"]["hubListed"], false, "{r}");
        let r = run(h, "measureDocsMerged", json!({"kind": "nope"}));
        assert_eq!(r["code"], "invalid", "{r}");
    }
}
