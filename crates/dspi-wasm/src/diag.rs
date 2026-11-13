//! The diagnostics bundle (WP WB8): the core's zip
//! (`DspiCore::diagnostics_zip`, what `uconsole diag` and Qt's "Save
//! Diagnostics…" write), for the page to hand the viewer as a file to save.
//! Nothing is sent anywhere.
//!
//! `diagnostics {app, files}` answers `{name, size, base64}`: the core's dated
//! file name, the zip's length and its bytes in standard base64 (the JSON
//! reply carries no raw bytes). `files` are the page's own `{name, text}`
//! (`client/<name>` in the zip). There is no hub store on the web.

use dspi_core::{DiagFile, DiagOptions, DspiCore};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

/// One file the page adds.
#[derive(Debug, Deserialize)]
pub struct FileParam {
    pub name: String,
    pub text: String,
}

/// `diagnostics`' parameters.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DiagParams {
    /// The client as it names itself.
    pub app: String,
    pub files: Vec<FileParam>,
}

/// `diagnostics`.
pub fn diagnostics(core: &DspiCore, p: DiagParams) -> Result<Value, CallError> {
    let options = DiagOptions {
        app: p.app,
        hub_store_dir: String::new(),
        files: p
            .files
            .into_iter()
            .map(|f| DiagFile {
                name: f.name,
                text: f.text,
            })
            .collect(),
    };
    let bytes = core.diagnostics_zip(options)?;
    Ok(json!({
        "name": core.diagnostics_file_name(),
        "size": bytes.len(),
        "base64": base64(&bytes),
    }))
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (RFC 4648 section 4).
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    let sym = |n: u32| char::from(ALPHABET.get((n & 63) as usize).copied().unwrap_or(b'='));
    for chunk in bytes.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(sym(n >> 18));
        out.push(sym(n >> 12));
        out.push(if chunk.len() > 1 { sym(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(n) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests_wp_wb8 {
    use serde_json::{Value, json};

    use super::base64;
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
    fn wp_wb8_base64_as_rfc_4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn wp_wb8_diagnostics_zip_of_the_demo_with_the_pages_file() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let r = run(h, "open", json!({ "kind": "demo" }));
        assert_eq!(r["ok"], true, "{r}");
        let r = run(
            h,
            "diagnostics",
            json!({ "app": "web test", "files": [{ "name": "browser.txt", "text": "agent" }] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let v = &r["value"];
        let name = v["name"].as_str().unwrap();
        assert!(
            name.starts_with("dspi-diagnostics-") && name.ends_with("Z.zip"),
            "{name}"
        );
        let b64 = v["base64"].as_str().unwrap();
        let size = v["size"].as_u64().unwrap() as usize;
        assert_eq!(b64.len(), size.div_ceil(3) * 4);
        // A zip: "PK\x03\x04" begins "UEsDB" in base64.
        assert!(b64.starts_with("UEsDB"), "{}", &b64[..12]);
        // No device: still a bundle, with the reason in notes.txt.
        run(h, "close", Value::Null);
        let r = run(h, "diagnostics", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        // Bad parameters are refused, not guessed.
        let r = run(h, "diagnostics", json!({ "files": 3 }));
        assert_eq!(r["code"], "invalid", "{r}");
    }

    #[test]
    fn wp_wb8_a_demo_clip_reaches_the_event_log() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let r = run(h, "demo", json!({ "action": "clip", "mask": 1 }));
        assert_eq!(r["ok"], true, "{r}");
        let r = run(h, "demo", json!({ "action": "clip", "mask": 0 }));
        assert_eq!(r["code"], "invalid", "{r}");
        assert_eq!(run(h, "meters", Value::Null)["ok"], true);
        let log = run(h, "eventLogRead", json!({}));
        let records = log["value"]["records"].as_array().unwrap();
        let clip = records.iter().find(|r| r["kind"] == "clip");
        assert!(clip.is_some(), "{log}");
        assert!(!clip.unwrap()["text"].as_str().unwrap().is_empty());
    }
}
