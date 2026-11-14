//! Safe apply with rollback on the web (WP P1SA): the core's guarded apply
//! (`DspiCore::apply_guarded_*`). Snapshots are documents the page holds, so
//! the guarded recall takes a snapshot document's text. Each call answers the
//! state `{id, phase, label, hub, timeoutMs, remainingMs, reason, message,
//! changes, notes}`. The page polls `applyPoll` (the timeout's revert runs
//! there, no timer in the core) and draws the countdown from `applyStatus`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

fn every_channel() -> u32 {
    u32::MAX
}

/// `applyGuardedImport`'s parameters: the document, the blocks (`IMPORT_*`
/// bits, as `configPreview` takes them), the EQ channels (every one when
/// absent) and the timeout in seconds (0 or absent: 30).
#[derive(Debug, Deserialize)]
pub struct ImportParams {
    pub text: String,
    #[serde(default)]
    pub options: u32,
    #[serde(default = "every_channel", rename = "eqChannels")]
    pub eq_channels: u32,
    #[serde(default, rename = "timeoutS")]
    pub timeout_s: u32,
}

/// `applyGuardedImport`: import with a way back
/// (`DspiCore::apply_guarded_import`).
pub fn import(core: &DspiCore, p: ImportParams) -> Result<Value, CallError> {
    Ok(tidy(&core.apply_guarded_import(
        p.text,
        p.options,
        p.eq_channels,
        p.timeout_s,
    )?))
}

/// `applyGuardedSnapshot`'s parameters: a snapshot document's text and the
/// timeout in seconds (0 or absent: 30).
#[derive(Debug, Deserialize)]
pub struct SnapshotParams {
    pub text: String,
    #[serde(default, rename = "timeoutS")]
    pub timeout_s: u32,
}

/// `applyGuardedSnapshot`: recall an uploaded snapshot with a way back
/// (`DspiCore::apply_guarded_snapshot_document`).
pub fn snapshot(core: &DspiCore, p: SnapshotParams) -> Result<Value, CallError> {
    Ok(tidy(
        &core.apply_guarded_snapshot_document(p.text, p.timeout_s)?,
    ))
}

/// `applyGuardBegin`'s parameters: the flow's label and the timeout.
#[derive(Debug, Deserialize)]
pub struct BeginParams {
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "timeoutS")]
    pub timeout_s: u32,
}

/// `applyGuardBegin`: guard a flow the page writes itself
/// (`DspiCore::apply_guard_begin`).
pub fn begin(core: &DspiCore, p: BeginParams) -> Result<Value, CallError> {
    Ok(tidy(&core.apply_guard_begin(p.label, p.timeout_s)?))
}

/// `applyConfirm`: Keep.
pub fn confirm(core: &DspiCore) -> Result<Value, CallError> {
    Ok(tidy(&core.apply_confirm()?))
}

/// `applyRevert`: Revert.
pub fn revert(core: &DspiCore) -> Result<Value, CallError> {
    Ok(tidy(&core.apply_revert()?))
}

/// `applyPoll`: drive the guard (the timeout's revert, a lost connection).
pub fn poll(core: &DspiCore) -> Value {
    tidy(&core.apply_poll())
}

/// `applyStatus`: the state, no I/O.
pub fn status(core: &DspiCore) -> Value {
    tidy(&core.apply_status())
}

/// `applyGuardDefault`: whether imports and recalls are guarded by default
/// here (over DSPi Link to a hub with the guard).
pub fn default_on(core: &DspiCore) -> Value {
    Value::Bool(core.apply_guard_default())
}

#[cfg(test)]
mod tests_wp_p1sa {
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
    fn wp_p1sa_guarded_apply_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "applyStatus", json!({}))["value"]["phase"], "idle");
        assert_eq!(
            run(h, "applyGuardBegin", json!({ "label": "x" }))["code"],
            "not_open"
        );
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        assert_eq!(run(h, "applyGuardDefault", json!({}))["value"], false);
        let snap = run(h, "snapshotCapture", json!({ "name": "Före" }));
        let text = snap["value"]["text"].as_str().unwrap().to_owned();

        let s = run(
            h,
            "applyGuardedSnapshot",
            json!({ "text": text, "timeoutS": 5 }),
        );
        assert_eq!(s["ok"], true, "{s}");
        assert_eq!(s["value"]["phase"], "waiting");
        assert_eq!(s["value"]["label"], "Före");
        assert_eq!(s["value"]["timeoutMs"], 5000);
        let k = run(h, "applyConfirm", json!({}));
        assert_eq!(k["value"]["phase"], "kept", "{k}");
        assert_eq!(run(h, "applyConfirm", json!({}))["code"], "invalid");

        let b = run(h, "applyGuardBegin", json!({ "label": "Flow" }));
        assert_eq!(b["value"]["timeoutMs"], 30_000, "{b}");
        let r = run(h, "applyRevert", json!({}));
        assert_eq!(r["value"]["phase"], "reverted", "{r}");
        assert_eq!(run(h, "applyPoll", json!({}))["value"], r["value"]);

        let i = run(h, "applyGuardedImport", json!({ "text": text }));
        assert_eq!(i["value"]["phase"], "waiting", "{i}");
        assert_eq!(run(h, "applyRevert", json!({}))["value"]["phase"], "reverted");
        assert_eq!(
            run(h, "applyGuardedImport", json!({ "text": "{" }))["code"],
            "invalid"
        );
    }
}
