//! Snapshots on the web (WP P1SN): no files there, so a snapshot is a
//! document the page downloads and uploads (OD-P1b). The two calls are the
//! core's document calls; the store calls are not offered.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

/// `snapshotCapture`'s parameters.
#[derive(Debug, Deserialize)]
pub struct CaptureParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub note: String,
}

/// `snapshotCapture`: the open device as a snapshot document
/// (`DspiCore::snapshot_capture_document`): `{text}`, to download.
pub fn capture(core: &DspiCore, p: CaptureParams) -> Result<Value, CallError> {
    let text = core.snapshot_capture_document(p.name, p.note)?;
    Ok(serde_json::json!({ "text": text }))
}

/// `snapshotRecall`'s parameters: a snapshot document's text.
#[derive(Debug, Deserialize)]
pub struct RecallParams {
    pub text: String,
}

/// `snapshotRecall`: recall an uploaded snapshot document
/// (`DspiCore::snapshot_recall_document`): `{name, image, changes, notes}`.
pub fn recall(core: &DspiCore, p: RecallParams) -> Result<Value, CallError> {
    Ok(tidy(&core.snapshot_recall_document(p.text)?))
}

#[cfg(test)]
mod tests_wp_p1sn {
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

    #[test]
    fn wp_p1sn_capture_and_recall_a_document() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "snapshotCapture", json!({}))["code"], "not_open");
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let c = run(
            h,
            "snapshotCapture",
            json!({ "name": "Kväll", "note": "n" }),
        );
        assert_eq!(c["ok"], true, "{c}");
        let text = c["value"]["text"].as_str().unwrap().to_owned();
        assert!(text.contains("\"kind\": \"snapshot\""));
        let r = run(h, "snapshotRecall", json!({ "text": text }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["name"], "Kväll");
        assert_eq!(r["value"]["image"], true);
        assert_eq!(r["value"]["changes"], json!([]));
        let bad = run(h, "snapshotRecall", json!({ "text": "{" }));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }
}
