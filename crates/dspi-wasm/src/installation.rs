//! Setting up the installation the hub keeps on the web (WP INST0, MPLAN
//! 1.4): the core's `installation_*` calls as JSON, for the System screen's
//! Installation tab (INSTW).
//!
//! A draft is `{revision, base, model}` (the hub's model as read and with the
//! edits, both the hub's JSON); the page keeps it between calls. Edits are
//! `{op, ...}` objects (`dspi_core::link::InstallationEdit`, camelCase, as the
//! C ABI's `fns_misc/installation.rs` lists them).
//!
//! - `installationView` → `{revision, role, mayEdit, draft, unseen: [{serial,
//!   name, online, fw, outputs}], inventory}`; asks the hub.
//! - `installationDraftEdit {draft, edits}` → the new draft; sends nothing.
//! - `installationValidate {draft}` → `[{subject, message}]`; sends nothing.
//! - `installationDiff {draft}` → `[sentence]`; sends nothing.
//! - `installationSave {draft}` → `{revision, warnings}`; one `system.set`.
//!
//! Failures keep the hub's meaning as `hubSystem`'s do: `conflict` (the
//! installation changed on the hub meanwhile), `denied` (the role), `invalid`
//! (a draft the hub would refuse; nothing sent), `read_only`, `not_open`.

use dspi_core::link::{InstallationDraft, InstallationEdit};
use dspi_core::{DspiCore, DspiError};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// The parameters of every call but the view.
#[derive(Debug, Deserialize)]
struct Params {
    draft: Value,
    #[serde(default)]
    edits: Vec<InstallationEdit>,
}

fn params(v: Value) -> Result<(InstallationDraft, Vec<InstallationEdit>), CallError> {
    let p: Params = serde_json::from_value(v)
        .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))?;
    Ok((InstallationDraft::from_json(&p.draft)?, p.edits))
}

/// The page's error: the hub's `conflict` and `denied` by name.
fn page_error(e: DspiError) -> CallError {
    let page = match &e {
        DspiError::Link { code, .. } if code == "conflict" => "conflict",
        DspiError::Link { code, .. } if code == "denied" => "denied",
        _ => return e.into(),
    };
    CallError {
        notice_key: Some(e.notice_key()),
        ..CallError::new(page, e.to_string())
    }
}

/// The call `method` names, if it is one of these.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "installationView" => core
            .installation_view()
            .map(|v| {
                let mut out = tidy(&v);
                if let Some(m) = out.as_object_mut() {
                    m.insert("draft".into(), v.draft.to_json());
                }
                out
            })
            .map_err(page_error),
        "installationDraftEdit" => params(p).and_then(|(d, edits)| {
            core.installation_draft_edit(d, edits)
                .map(|d| d.to_json())
                .map_err(CallError::from)
        }),
        "installationValidate" => params(p).and_then(|(d, _)| {
            core.installation_validate(d)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "installationDiff" => params(p).and_then(|(d, _)| {
            core.installation_diff(d)
                .map(|v| tidy(&v))
                .map_err(CallError::from)
        }),
        "installationSave" => params(p).and_then(|(d, _)| {
            core.installation_save(d)
                .map(|v| tidy(&v))
                .map_err(page_error)
        }),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests_wp_inst0 {
    use serde_json::{Value, json};

    use crate::calls::{call, session_new};
    use crate::guard;

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn wp_inst0_web_edit_validate_diff_and_off_a_hub() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let site = json!({"rooms": [{"id": "main"}], "senders": [{"id": "tx", "serial": "S-TX"}]});
        let draft = json!({"revision": 2, "base": site, "model": site});
        let r = run(
            h,
            "installationDraftEdit",
            json!({"draft": draft, "edits": [
                {"op": "addUnit", "serial": "S-A", "role": "repeater", "upstream": "tx", "room": "main"},
                {"op": "addGroup", "name": "Everything", "units": ["S-A"]}
            ]}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let d = r["value"].clone();
        assert_eq!(d["revision"], 2);
        assert_eq!(d["model"]["units"][0]["repeater"], true);
        let v = run(h, "installationValidate", json!({ "draft": d }));
        assert_eq!(v["value"], json!([]), "{v}");
        let v = run(h, "installationDiff", json!({ "draft": d }));
        assert_eq!(
            v["value"],
            json!(["Adds group Everything", "Adds unit S-A"])
        );
        // A refused edit and a bad draft: `invalid`.
        let r = run(
            h,
            "installationDraftEdit",
            json!({"draft": d, "edits": [{"op": "removeElement", "id": "ghost"}]}),
        );
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "installationValidate", json!({ "draft": {"model": 3} }));
        assert_eq!(r["code"], "invalid", "{r}");
        // No hub: not open, nothing sent.
        assert_eq!(run(h, "installationView", Value::Null)["code"], "not_open");
        assert_eq!(
            run(h, "installationSave", json!({ "draft": d }))["code"],
            "not_open"
        );
        let mut bad = d.clone();
        bad["model"]["units"][0]["upstream"] = json!("ghost");
        assert_eq!(
            run(h, "installationSave", json!({ "draft": bad }))["code"],
            "invalid"
        );
        // The hub's `conflict` keeps its name.
        let e = super::page_error(dspi_core::DspiError::Link {
            code: "conflict".into(),
            msg: "changed".into(),
        });
        assert_eq!(e.code, "conflict");
    }
}
