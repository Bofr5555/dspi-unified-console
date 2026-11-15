//! The preset flows that `write` does not cover (WP WB6a): the unsaved-changes
//! check, Copy to, Revert to Saved and Factory Reset, each the core's own call.
//!
//! Save, load, delete, rename and the startup slot go through `write`
//! (`preset.save`, `preset.load`, ...). Factory reset is one the core refuses
//! by path (`dspi_core::write_refusal`): the page asks the user first, then
//! calls `factoryReset`. Each flow runs under the core's device lock.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

/// A device was just opened (the demo, USB): the rest of the connect, as
/// the other clients do after opening (`fetch_all`): the reads that are not
/// in the image (the preset directory and names among them), and the
/// baseline `presetChanges` compares with. A hub device is fetched by
/// `link::open` already. On failure the device is closed again.
pub fn opened(core: &DspiCore) -> Result<(), CallError> {
    if let Err(e) = core.fetch_all() {
        core.disconnect();
        return Err(e.into());
    }
    Ok(())
}

/// `presetChanges`: whether the live state differs from the active preset,
/// and how, one sentence each in the core's wording (CG7).
pub fn changes(core: &DspiCore) -> Result<Value, CallError> {
    need_device(core)?;
    let changes = core.preset_changes();
    Ok(json!({ "dirty": !changes.is_empty(), "changes": changes }))
}

/// `presetCopy`'s parameters.
#[derive(Debug, Deserialize)]
pub struct CopyParams {
    pub source: u8,
    pub dest: u8,
    // -- WP P1TK --
    /// A confirmation token for `copy_preset` (`confirmToken`).
    #[serde(default)]
    pub token: Option<String>,
}

/// `presetCopy`: the live state into `dest`, `source` kept active
/// (`DspiCore::copy_preset`). Writes flash twice.
pub fn copy(core: &DspiCore, p: CopyParams) -> Result<Value, CallError> {
    core.copy_preset_confirmed(p.source, p.dest, p.token)?; // -- WP P1TK --
    Ok(Value::Null)
}

/// `presetRevert`: reload the active preset (`DspiCore::load_params`).
pub fn revert(core: &DspiCore) -> Result<Value, CallError> {
    core.load_params()?;
    Ok(Value::Null)
}

/// `factoryReset`: the factory defaults, after the user confirmed it
/// (`DspiCore::factory_reset`). Stored presets are kept.
pub fn factory_reset(core: &DspiCore, p: TokenParams) -> Result<Value, CallError> {
    core.factory_reset_confirmed(p.token)?; // -- WP P1TK --
    Ok(Value::Null)
}

// -- WP P1TK --
/// The parameters of a call that only takes a confirmation token: none at
/// all, or `{token}` (`confirmToken`).
#[derive(Debug, Default, Deserialize)]
pub struct TokenParams {
    #[serde(default)]
    pub token: Option<String>,
}

// -- WP P1DF --
/// `configDiff`'s parameters: two `.dspipreset` documents.
#[derive(Debug, Deserialize)]
pub struct DiffParams {
    pub before: String,
    pub after: String,
}

/// `configDiff`: what differs between two documents, with no device
/// (`dspi_core::files::config_diff`'s JSON).
pub fn diff(p: DiffParams) -> Result<Value, CallError> {
    reply(&dspi_core::files::config_diff(p.before, p.after))
}

/// `configPreview`'s parameters: a document and the import options
/// (`IMPORT_*` bits, with `IMPORT_BY_BLOCK` by block) and the EQ channels
/// (bit k: wire channel k; every one when absent).
#[derive(Debug, Deserialize)]
pub struct PreviewParams {
    pub text: String,
    #[serde(default)]
    pub options: u32,
    #[serde(default = "every_channel", rename = "eqChannels")]
    pub eq_channels: u32,
}

fn every_channel() -> u32 {
    u32::MAX
}

/// `configPreview`: what importing the document would change on the open
/// device, without writing (`DspiCore::config_preview`'s JSON).
pub fn preview(core: &DspiCore, p: PreviewParams) -> Result<Value, CallError> {
    need_device(core)?;
    reply(&core.config_preview(p.text, p.options, p.eq_channels))
}

/// A core JSON reply as the call's value; its `{ok: false, error}` as a
/// `bad_document` failure, with the core's `noticeKey` (WP P1AFIX, review
/// p1a N3).
fn reply(text: &str) -> Result<Value, CallError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| CallError::new("internal", format!("unreadable reply: {e}")))?;
    if v.get("ok") == Some(&Value::Bool(true)) {
        Ok(v)
    } else {
        let why = v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("not a DSPi configuration");
        // -- WP P1AFIX --
        let notice_key = v
            .get("noticeKey")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Err(CallError {
            notice_key,
            ..CallError::new("bad_document", why)
        })
    }
}

// -- WP P1WB1 -- a plain import and an export on the web (the page reads and
// writes the file; the web has no file system)
/// `configImport`'s parameters: [`PreviewParams`]'s, and an optional
/// confirmation token (`confirmToken` of `config_import`).
#[derive(Debug, Deserialize)]
pub struct ImportParams {
    pub text: String,
    #[serde(default)]
    pub options: u32,
    #[serde(default = "every_channel", rename = "eqChannels")]
    pub eq_channels: u32,
    #[serde(default)]
    pub token: Option<String>,
}

/// `configImport`: apply the chosen blocks of a document to the open device
/// (`DspiCore::config_import_confirmed`): its JSON reply. A text that is not
/// a document fails as `bad_document`; a missing or bad token as
/// `confirmation_required`, as every other gated call (WP P1TZF); a device
/// that cannot be written or an empty selection as `refused`; all with the
/// core's `noticeKey`.
pub fn import(core: &DspiCore, p: ImportParams) -> Result<Value, CallError> {
    need_device(core)?;
    let text = core.config_import_confirmed(p.text, p.options, p.eq_channels, p.token);
    reply(&text).map_err(|e| {
        let document = e
            .notice_key
            .as_deref()
            .is_some_and(|k| k == "config.unreadable");
        if document {
            e
        } else if e.notice_key.as_deref() == Some("confirmation_required") {
            // -- WP P1TZF -- (review p1tz F4)
            CallError {
                code: "confirmation_required",
                ..e
            }
        } else {
            CallError {
                code: "refused",
                ..e
            }
        }
    })
}

/// `configExport`'s parameters: `meta.name`, `meta.appVersion` and the
/// input-pair link mask (`global.inputPairLinked`).
#[derive(Debug, Deserialize)]
pub struct ExportParams {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "appVersion")]
    pub app_version: String,
    #[serde(default)]
    pub links: u8,
}

/// `configExport`: the open device as a `.dspipreset` document, `{text}`,
/// for the page to download (`DspiCore::config_export`).
pub fn export(core: &DspiCore, p: ExportParams) -> Result<Value, CallError> {
    let text = core.config_export(p.name, p.app_version, p.links)?;
    Ok(json!({ "text": text }))
}

// -- WP P1AFIX -- (review p1a N1)
/// `crossfeedCustomPair`: the crossfeed's custom pair as the device keeps it,
/// whatever the preset (`DspiCore::crossfeed_custom_pair`): `{freqHz,
/// feedDb}`. No transfer.
pub fn crossfeed_custom_pair(core: &DspiCore) -> Result<Value, CallError> {
    Ok(crate::calls::tidy(&core.crossfeed_custom_pair()?))
}

fn need_device(core: &DspiCore) -> Result<(), CallError> {
    if core.is_connected() {
        Ok(())
    } else {
        Err(CallError::new(
            "not_open",
            "No DSPi is open in this session",
        ))
    }
}

#[cfg(test)]
mod tests {
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

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    // -- WP P1TZ -- tokens are always required: a gated call carries one,
    // minted as the page does after its prompt.
    fn tok(h: u32, op: &str) -> Value {
        ok(h, "confirmToken", json!({ "op": op }))
    }

    // -- WP P1DF --
    #[test]
    fn p1df_config_diff_and_preview() {
        const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
        const QT: &str = include_str!("../../../tests/fixtures/preset/qt-core-v32.dspipreset");
        guard::reset_for_test();
        let h = session_new().unwrap();
        let d = ok(h, "configDiff", json!({ "before": QT, "after": MAC }));
        assert!(d["count"].as_u64().unwrap() > 0, "{d}");
        assert_eq!(
            run(h, "configDiff", json!({ "before": "{", "after": MAC }))["code"],
            "bad_document"
        );
        assert_eq!(
            run(h, "configPreview", json!({ "text": MAC }))["code"],
            "not_open"
        );
        ok(h, "open", json!({ "kind": "demo" }));
        // Volumes alone (IMPORT_BY_BLOCK | IMPORT_VOLUMES).
        let p = ok(
            h,
            "configPreview",
            json!({ "text": MAC, "options": (1 << 16) | 1 }),
        );
        assert_eq!(p["count"], 2, "{p}");
        assert_eq!(p["blocks"]["volumes"], 2);
        // EQ of wire channel 1 only.
        let p = ok(
            h,
            "configPreview",
            json!({ "text": MAC, "options": (1 << 16) | 4, "eqChannels": 2 }),
        );
        assert_eq!(p["count"], 2, "{p}");
    }

    // -- WP P1WB1 --
    #[test]
    fn wp_p1wb1_config_import_and_export() {
        const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(
            run(h, "configImport", json!({ "text": MAC }))["code"],
            "not_open"
        );
        assert_eq!(run(h, "configExport", json!({}))["ok"], false);
        ok(h, "open", json!({ "kind": "demo" }));
        let out = ok(
            h,
            "configExport",
            json!({ "name": "Here", "appVersion": "web" }),
        );
        let doc: Value = serde_json::from_str(out["text"].as_str().unwrap()).unwrap();
        assert_eq!(doc["meta"]["name"], "Here");
        // Volumes alone: what the preview said is what the import changed.
        let options = (1 << 16) | 1;
        let p = ok(
            h,
            "configPreview",
            json!({ "text": MAC, "options": options }),
        );
        assert_eq!(p["count"], 2, "{p}");
        let r = ok(
            h,
            "configImport",
            json!({ "text": MAC, "options": options, "token": tok(h, "config_import") }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let after = ok(
            h,
            "configPreview",
            json!({ "text": MAC, "options": options }),
        );
        assert_eq!(after["count"], 0, "{after}");
        let r = run(
            h,
            "configImport",
            json!({ "text": MAC, "options": 1 << 16 }),
        );
        assert_eq!(r["code"], "refused", "{r}");
        assert_eq!(r["noticeKey"], "config.nothing_selected", "{r}");
        let r = run(h, "configImport", json!({ "text": "{" }));
        assert_eq!(r["code"], "bad_document", "{r}");
    }

    // -- WP P1AFIX --
    #[test]
    fn wp_p1afix_config_refusals_carry_a_notice_key_and_the_custom_pair_reads() {
        const QT: &str = include_str!("../../../tests/fixtures/preset/qt-core-v32.dspipreset");
        guard::reset_for_test();
        let h = session_new().unwrap();
        let r = run(h, "configDiff", json!({ "before": "{", "after": QT }));
        assert_eq!(r["code"], "bad_document", "{r}");
        assert_eq!(r["noticeKey"], "config.unreadable", "{r}");
        assert_eq!(
            run(h, "crossfeedCustomPair", Value::Null)["code"],
            "not_open"
        );
        ok(h, "open", json!({ "kind": "demo" }));
        let r = run(h, "configPreview", json!({ "text": "{" }));
        assert_eq!(r["noticeKey"], "config.unreadable", "{r}");
        ok(
            h,
            "write",
            json!({ "path": "cf.preset", "value": { "choice": 3 } }),
        );
        ok(
            h,
            "write",
            json!({ "path": "cf.freq", "value": { "float": 820.0 } }),
        );
        ok(
            h,
            "write",
            json!({ "path": "cf.preset", "value": { "choice": 2 } }),
        );
        let p = ok(h, "crossfeedCustomPair", Value::Null);
        assert_eq!(p["freqHz"], 820.0, "{p}");
        assert!(p["feedDb"].is_number(), "{p}");
    }

    #[test]
    fn wb6a_changes_copy_revert_and_reset_on_the_demo() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "presetChanges", Value::Null)["code"], "not_open");
        let snap = ok(h, "open", json!({ "kind": "demo" }));
        // The open read what the image does not hold: the preset directory and names.
        assert_eq!(snap["state"]["presetOccupied"], 1, "{}", snap["state"]);
        assert_eq!(snap["state"]["presetNames"][0], "Demo");
        let clean = ok(h, "presetChanges", Value::Null);
        assert_eq!(clean, json!({ "dirty": false, "changes": [] }));

        // A preset change is reported in the core's words; the user volume is not one.
        ok(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -20.0 } }),
        );
        assert_eq!(ok(h, "presetChanges", Value::Null)["dirty"], false);
        ok(
            h,
            "write",
            json!({ "path": "out.gain", "indices": [0], "value": { "float": -6.0 } }),
        );
        let dirty = ok(h, "presetChanges", Value::Null);
        assert_eq!(dirty["dirty"], true);
        let lines: Vec<String> = serde_json::from_value(dirty["changes"].clone()).unwrap();
        assert!(lines.iter().any(|l| l.contains("-6.0 dB")), "{lines:?}");

        // Revert to Saved: the active preset again, nothing unsaved.
        ok(h, "presetRevert", Value::Null);
        assert_eq!(ok(h, "presetChanges", Value::Null)["dirty"], false);
        assert_eq!(ok(h, "state", Value::Null)["state"]["outputGainDb"][0], 0.0);

        // Copy to: the live state into slot 3, the active slot kept.
        let active = ok(h, "state", Value::Null)["state"]["activePresetSlot"].clone();
        ok(
            h,
            "write",
            json!({ "path": "out.gain", "indices": [0], "value": { "float": -4.5 } }),
        );
        ok(
            h,
            "presetCopy",
            json!({ "source": active, "dest": 3, "token": tok(h, "copy_preset") }),
        );
        let st = ok(h, "state", Value::Null)["state"].clone();
        assert_eq!(st["activePresetSlot"], active);
        assert_ne!(
            st["presetOccupied"].as_u64().unwrap() & (1 << 3),
            0,
            "slot 3 is stored"
        );
        assert_eq!(ok(h, "presetChanges", Value::Null)["dirty"], false);
        let bad = run(h, "presetCopy", json!({ "source": 0, "dest": 12 }));
        assert_eq!(bad["ok"], false, "{bad}");

        // Factory reset: the defaults, still the active preset's baseline.
        ok(
            h,
            "write",
            json!({ "path": "bypass", "value": { "bool": true } }),
        );
        ok(
            h,
            "factoryReset",
            json!({ "token": tok(h, "factory_reset") }),
        );
        let st = ok(h, "state", Value::Null)["state"].clone();
        assert_eq!(st["bypass"], false);
        assert_eq!(ok(h, "presetChanges", Value::Null)["dirty"], false);
    }
}
