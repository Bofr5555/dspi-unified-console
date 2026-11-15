//! The client read-only mode on the web (WP ROMODE), over
//! `dspi_core::read_only`.
//!
//! - `setClientReadOnly {on}`: turn it on or off for this session's core; it
//!   stays for every device the session opens. Answers `{on}`. Turning it on
//!   while a guarded apply waits fails with code `invalid` and notice key
//!   `read_only.guard_pending` (keep or revert first).
//! - `clientReadOnly`: `{on, refusals}`.
//!
//! While it is on, every write fails with code `read_only` and notice key
//! `read_only.client` before anything is sent; reads, notifications, the
//! meters and the analyser go on. A client-side diagnostic, not a security
//! boundary (the page's `?readonly`).

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

type CallResult = Result<Value, CallError>;

#[derive(Deserialize)]
struct On {
    on: bool,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "setClientReadOnly" => serde_json::from_value::<On>(p)
            .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
            .and_then(|o| {
                core.set_client_read_only(o.on).map_err(|e| {
                    let key = core.notice_for_error(e.clone()).key;
                    CallError {
                        notice_key: Some(key),
                        ..CallError::from(e)
                    }
                })?;
                Ok(json!({ "on": core.client_read_only() }))
            }),
        "clientReadOnly" => Ok(json!({
            "on": core.client_read_only(),
            "refusals": core.client_read_only_refusals(),
        })),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_romode {
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
    fn wp_romode_the_page_turns_the_mode_on_and_a_write_is_refused_in_its_words() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "clientReadOnly", Value::Null)["value"]["on"], false);
        let r = run(h, "setClientReadOnly", json!({ "on": true }));
        assert_eq!(r["value"]["on"], true, "{r}");
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let before = run(h, "state", Value::Null)["value"]["state"]["masterVolumeDb"].clone();

        let r = run(
            h,
            "write",
            json!({ "path": "vol.master", "value": { "float": -7.0 } }),
        );
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "read_only", "{r}");
        assert_eq!(r["noticeKey"], "read_only.client", "{r}");
        let n = run(
            h,
            "noticeFor",
            json!({ "key": "read_only.client", "detail": "" }),
        );
        assert_eq!(n["value"]["title"], "Read-only mode", "{n}");

        // Reads go on, and the device did not change.
        let s = run(h, "state", Value::Null);
        assert_eq!(s["ok"], true, "{s}");
        assert_eq!(s["value"]["state"]["masterVolumeDb"], before);
        assert_eq!(run(h, "meters", Value::Null)["ok"], true);
        let st = run(h, "clientReadOnly", Value::Null);
        assert_eq!(st["value"]["refusals"], 1, "{st}");

        // Off again: the write goes out.
        let r = run(h, "setClientReadOnly", json!({ "on": false }));
        assert_eq!(r["value"]["on"], false);
        let r = run(
            h,
            "write",
            json!({ "path": "vol.master", "value": { "float": -7.0 } }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let r = run(h, "setClientReadOnly", json!({ "on": "yes" }));
        assert_eq!(r["code"], "invalid", "{r}");

        // While a guarded apply waits, the mode is refused with its row.
        assert_eq!(
            run(h, "applyGuardBegin", json!({ "label": "x" }))["ok"],
            true
        );
        let r = run(h, "setClientReadOnly", json!({ "on": true }));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "invalid", "{r}");
        assert_eq!(r["noticeKey"], "read_only.guard_pending", "{r}");
        assert_eq!(run(h, "clientReadOnly", Value::Null)["value"]["on"], false);
        assert_eq!(run(h, "applyRevert", json!({}))["ok"], true);
        let r = run(h, "setClientReadOnly", json!({ "on": true }));
        assert_eq!(r["value"]["on"], true, "{r}");
    }
}
