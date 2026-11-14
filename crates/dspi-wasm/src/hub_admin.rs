//! The hub's admin API on the web (WP HUBADMIN), over `dspi_core::hub_admin`.
//!
//! - `installerTarget`: which copy the Installer page edits (`copy` `hub`,
//!   `console` or `none`; `editable`; `noticeKey`; `readOnlyKey`). On the web a
//!   direct device is a USB one; it has no PIN of its own (`installerPinSet`
//!   is `unsupported`), so its copy is the limits profile only.
//! - `hubLimitsGet`, `hubLimitsSet {text}`, `hubLimitsClear`, `hubPinSet
//!   {pin}` (the recovery code shown once), `hubPinClear`, `hubModePin {role,
//!   mode}` ("" unpins): the hub's `limits.*` and `mode.*` over this
//!   session's DSPi Link. Admin of the hub only for the changes; the hub says
//!   `denied` otherwise.
//! - `hubAdminEvents`: the hub's `limits.changed`, `limits.held` and
//!   `mode.changed` since the last call (also handed out by `linkTick` as
//!   `hubEvents`), each with the lock read again.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    Ok(tidy(&r?))
}

#[derive(Deserialize)]
struct Doc {
    text: String,
}

#[derive(Deserialize)]
struct Pin {
    pin: String,
}

#[derive(Deserialize)]
struct ModePin {
    role: String,
    #[serde(default)]
    mode: String,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "installerTarget" => Ok(tidy(&core.installer_target())),
        "hubLimitsGet" => record(core.hub_limits_get()),
        "hubLimitsSet" => params::<Doc>(p).and_then(|d| record(core.hub_limits_set(d.text))),
        "hubLimitsClear" => record(core.hub_limits_clear()),
        "hubPinSet" => params::<Pin>(p).and_then(|x| record(core.hub_pin_set(x.pin))),
        "hubPinClear" => record(core.hub_pin_clear()),
        "hubModePin" => {
            params::<ModePin>(p).and_then(|m| record(core.hub_mode_pin(m.role, m.mode)))
        }
        "hubAdminEvents" => Ok(tidy(&core.hub_admin_events())),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_hubadmin {
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
    fn wp_hubadmin_the_target_and_the_hub_calls_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(
            run(h, "installerTarget", Value::Null)["value"]["copy"],
            "none"
        );
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let t = run(h, "installerTarget", Value::Null);
        assert_eq!(t["value"]["copy"], "console", "{t}");
        assert_eq!(t["value"]["noticeKey"], "installer.console_copy", "{t}");
        for (m, p) in [
            ("hubLimitsGet", Value::Null),
            ("hubLimitsClear", Value::Null),
            ("hubPinClear", Value::Null),
            ("hubModePin", json!({ "role": "control", "mode": "simple" })),
        ] {
            let r = run(h, m, p);
            assert_eq!(r["ok"], false, "{m}: {r}");
            assert_eq!(r["noticeKey"], "dlink.no_limits", "{m}: {r}");
        }
        let r = run(h, "hubPinSet", json!({ "pin": "12" }));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "hubAdminEvents", Value::Null);
        assert_eq!(r["value"], json!([]), "{r}");
    }
}
