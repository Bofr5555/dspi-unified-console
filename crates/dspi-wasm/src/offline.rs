//! Offline editing and templates (WP P1OF), each the core's own call
//! (`dspi_core::offline`). `documentOpen` lives in `calls.rs`, beside `open`,
//! since it replaces the session's device as `open` does.
//!
//! The web has no file system: `templateList` lists the templates this build
//! ships, and the page reads and writes the user's files itself.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

/// `documentOpen`'s parameters: the document's text and its file's name.
#[derive(Debug, Deserialize)]
pub struct OpenParams {
    pub text: String,
    #[serde(default, rename = "fileName")]
    pub file_name: String,
}

/// `documentSave`'s parameters (`links`: the input-pair link mask).
#[derive(Debug, Deserialize)]
pub struct SaveParams {
    #[serde(default, rename = "appVersion")]
    pub app_version: String,
    #[serde(default)]
    pub links: u8,
}

/// `templateSave`'s parameters: [`SaveParams`] and the template's name.
#[derive(Debug, Deserialize)]
pub struct TemplateParams {
    pub name: String,
    #[serde(default, rename = "appVersion")]
    pub app_version: String,
    #[serde(default)]
    pub links: u8,
}

/// `templateText`'s parameters: an id from `templateList`.
#[derive(Debug, Deserialize)]
pub struct TextParams {
    pub id: String,
}

/// A core JSON reply as the call's value.
fn value(text: &str) -> Result<Value, CallError> {
    serde_json::from_str(text)
        .map_err(|e| CallError::new("internal", format!("unreadable reply: {e}")))
}

/// `documentOpen`'s core half: the core's reply
/// (`DspiCore::open_document`).
pub fn open(core: &DspiCore, p: OpenParams) -> Result<Value, CallError> {
    value(&core.open_document(p.text, p.file_name)?)
}

/// `documentInfo`: whether the session is on a document, and which
/// (`DspiCore::offline_document`).
pub fn info(core: &DspiCore) -> Result<Value, CallError> {
    value(&core.offline_document())
}

/// `documentSave`: the open document as it now stands, `{text}`
/// (`DspiCore::save_document`).
pub fn save(core: &DspiCore, p: SaveParams) -> Result<Value, CallError> {
    Ok(json!({ "text": core.save_document(p.app_version, p.links)? }))
}

/// `templateSave`: the open device or document as a template, `{text}`
/// (`DspiCore::save_as_template`).
pub fn save_template(core: &DspiCore, p: TemplateParams) -> Result<Value, CallError> {
    Ok(json!({ "text": core.save_as_template(p.name, p.app_version, p.links)? }))
}

/// `templateList`: the templates this build ships (`dspi_core::template_list`).
pub fn list() -> Result<Value, CallError> {
    value(&dspi_core::template_list(String::new()))
}

/// `templateText`: a shipped template's text, `{text}`
/// (`dspi_core::template_text`).
pub fn text(p: TextParams) -> Result<Value, CallError> {
    Ok(json!({ "text": dspi_core::template_text(p.id, String::new())? }))
}

#[cfg(test)]
mod tests_wp_p1of {
    use crate::calls::{call, session_new};
    use serde_json::{Value, json};

    const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

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
    fn wp_p1of_documents_and_templates_through_wasm() {
        let h = session_new().unwrap();
        let r = run(h, "documentInfo", Value::Null);
        assert_eq!(r["value"]["offline"], false, "{r}");
        let r = run(
            h,
            "documentOpen",
            json!({ "text": MAC, "fileName": "hall.dspipreset" }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["document"]["name"], "Mac reference");
        assert_eq!(r["value"]["device"]["demo"], false);
        assert_eq!(r["value"]["state"]["numOutputChannels"], 9);
        let r = run(h, "documentInfo", Value::Null);
        assert_eq!(r["value"]["fileName"], "hall.dspipreset");
        let r = run(h, "documentSave", json!({ "appVersion": "web" }));
        let saved: Value = serde_json::from_str(r["value"]["text"].as_str().unwrap()).unwrap();
        assert_eq!(saved["meta"]["name"], "Mac reference");
        let r = run(h, "templateSave", json!({ "name": "Hall" }));
        let t: Value = serde_json::from_str(r["value"]["text"].as_str().unwrap()).unwrap();
        assert_eq!(t["meta"]["kind"], "template");
        let r = run(h, "templateList", Value::Null);
        // -- WP P1EFIX -- since TPL1 the generic templates come first; K340 is listed
        assert!(
            r["value"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == "builtin:k340"),
            "{r}"
        );
        let r = run(h, "templateText", json!({ "id": "builtin:k340" }));
        assert!(
            r["value"]["text"]
                .as_str()
                .unwrap()
                .contains("\"template\"")
        );
        let r = run(h, "documentOpen", json!({ "text": "nope" }));
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "invalid");
        let r = run(h, "templateText", json!({ "id": "../x.dspipreset" }));
        assert_eq!(r["code"], "invalid");
    }
}
