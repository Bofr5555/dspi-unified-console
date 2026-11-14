//! Limits profiles on the web (WP P1LM, PRO1 S1): no files there, so the
//! page keeps the profile's document (browser storage, download, upload) and
//! hands it to `limitsApply` when a device opens. A write past a limit fails
//! with code and notice key `limit_exceeded`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

/// `limitsApply`'s parameters: a limits document's text.
#[derive(Debug, Deserialize)]
pub struct ApplyParams {
    pub text: String,
}

/// `limitsApply`: set the open device's profile and hold the device to it
/// (`DspiCore::limits_apply_profile`): `{lines, held, notes}`.
pub fn apply(core: &DspiCore, p: ApplyParams) -> Result<Value, CallError> {
    Ok(tidy(&core.limits_apply_profile(p.text)?))
}

/// `limitsProfile`: the open device's profile document, `{text}` ("" with
/// none), to keep or download.
pub fn profile(core: &DspiCore) -> Value {
    json!({ "text": core.limits_profile_document() })
}

/// `limitsClear`: remove the open device's profile.
pub fn clear(core: &DspiCore) -> Result<Value, CallError> {
    core.limits_clear()?;
    Ok(json!({}))
}

/// `limitsStatus`: `{active, serial, state, noticeKey, detail, lastHeld}`.
pub fn status(core: &DspiCore) -> Value {
    tidy(&core.limits_status())
}

#[cfg(test)]
mod tests_wp_p1lm {
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
    fn wp_p1lm_limits_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let gain = |db: f32| {
            run(
                h,
                "write",
                json!({ "path": "out.gain", "indices": [1], "value": { "float": db } }),
            )
        };
        assert_eq!(gain(8.0)["ok"], true);
        let doc = json!({
            "schemaVersion": 1,
            "meta": {"kind": "limits"},
            "limits": {"outputs": [{"output": 1, "maxGainDb": 2.0}]}
        })
        .to_string();
        let r = run(h, "limitsApply", json!({ "text": doc }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["held"][0], "out.gain[1] 8 → 2 dB", "{r}");
        let refused = gain(5.0);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "limit_exceeded", "{refused}");
        assert_eq!(refused["noticeKey"], "limit_exceeded", "{refused}");
        let st = run(h, "limitsStatus", Value::Null);
        assert_eq!(st["value"]["active"], true, "{st}");
        let p = run(h, "limitsProfile", Value::Null);
        assert!(p["value"]["text"].as_str().unwrap().contains("\"limits\""));
        assert_eq!(run(h, "limitsClear", Value::Null)["ok"], true);
        assert_eq!(gain(5.0)["ok"], true);
        let bad = run(h, "limitsApply", json!({ "text": "{" }));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }
}
