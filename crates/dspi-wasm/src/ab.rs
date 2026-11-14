//! A/B compare on the web (WP P1AB, PRO1 W4). No snapshot store there, so a
//! compare starts from snapshot documents the page holds (OD-P1b); the rest
//! is the core's calls.

use dspi_core::{AbSide, DspiCore};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

/// `abStart`'s parameters: B's snapshot document and, optionally, A's
/// (absent or "": the device is captured as A).
#[derive(Debug, Deserialize)]
pub struct StartParams {
    #[serde(default)]
    pub a: String,
    pub b: String,
}

/// `abStart`: start a compare (`DspiCore::ab_start_documents`):
/// `{active, side, aName, bName, recall}`.
pub fn start(core: &DspiCore, p: StartParams) -> Result<Value, CallError> {
    Ok(tidy(&core.ab_start_documents(p.a, p.b)?))
}

/// `abToggle`: recall the other side (`DspiCore::ab_toggle`).
pub fn toggle(core: &DspiCore) -> Result<Value, CallError> {
    Ok(tidy(&core.ab_toggle()?))
}

/// `abDiff`: what differs from A to B, `configDiff`'s reply for the pair.
pub fn diff(core: &DspiCore) -> Result<Value, CallError> {
    let text = core.ab_diff()?;
    serde_json::from_str(&text)
        .map_err(|e| CallError::new("internal", format!("unreadable reply: {e}")))
}

/// `abKeep`'s parameters: the side to keep, `"a"` or `"b"`.
#[derive(Debug, Deserialize)]
pub struct KeepParams {
    pub side: String,
}

/// `abKeep`: end the compare on a side (`DspiCore::ab_keep`).
pub fn keep(core: &DspiCore, p: KeepParams) -> Result<Value, CallError> {
    let side = AbSide::from_key(&p.side).ok_or_else(|| {
        CallError::new(
            "invalid",
            format!("side is \"a\" or \"b\", not \"{}\"", p.side),
        )
    })?;
    Ok(tidy(&core.ab_keep(side)?))
}

/// `abState`: the running compare, `active: false` with none.
pub fn state(core: &DspiCore) -> Value {
    tidy(&core.ab_state())
}

#[cfg(test)]
mod tests_wp_p1ab {
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

    #[test]
    fn wp_p1ab_compare_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "abToggle", json!({}))["code"], "invalid");
        assert_eq!(ok(h, "abState", json!({}))["active"], false);
        assert_eq!(run(h, "abStart", json!({ "b": "{" }))["code"], "not_open");
        ok(h, "open", json!({ "kind": "demo" }));

        let capture = |name: &str| {
            ok(h, "snapshotCapture", json!({ "name": name }))["text"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        // How many values differ from document `a` to the device now.
        let differ = |a: &str| {
            let now = capture("now");
            ok(h, "configDiff", json!({ "before": a, "after": now }))["count"]
                .as_u64()
                .unwrap()
        };
        let a = capture("Start");
        ok(
            h,
            "write",
            json!({ "path": "out.gain", "indices": [0], "value": { "float": -4.0 } }),
        );
        let b = capture("Bright");
        ok(h, "snapshotRecall", json!({ "text": a }));
        assert_eq!(differ(&a), 0);

        let s = ok(h, "abStart", json!({ "b": b }));
        assert_eq!(
            (s["active"].clone(), s["side"].clone()),
            (json!(true), json!("a"))
        );
        let t = ok(h, "abToggle", json!({}));
        assert_eq!(t["side"], "b", "{t}");
        assert_eq!(t["recall"]["image"], true);
        assert_eq!(differ(&b), 0);
        assert_ne!(differ(&a), 0);
        let t = ok(h, "abToggle", json!({}));
        assert_eq!(t["side"], "a");
        assert_eq!(differ(&a), 0);

        let d = ok(h, "abDiff", json!({}));
        assert_eq!(d["ok"], true, "{d}");
        assert!(d["count"].as_u64().unwrap() >= 1, "{d}");

        assert_eq!(run(h, "abKeep", json!({ "side": "x" }))["code"], "invalid");
        let k = ok(h, "abKeep", json!({ "side": "b" }));
        assert_eq!(
            (k["active"].clone(), k["side"].clone()),
            (json!(false), json!("b"))
        );
        assert_eq!(ok(h, "abState", json!({}))["active"], false);
    }
}
