//! The mode manifest and the installer's lock on the web (WP P1LK, PRO1 S4
//! and U7), over `dspi_core::modes` and `dspi_core::limits`.
//!
//! - `modeManifest`: the manifest, in the shape of `assets/modes/modes.json`.
//! - `modeResolve {platform, chosen?}`: the mode to show, and why.
//! - `modeSetPinned {mode}` ("" clears), `modePinned`: the hub's pinned mode.
//! - `installerLockState`, `installerUnlock {pin}`, `installerLock`,
//!   `installerPinClear`, `installerLockCover {pages}`.
//! - `installerPinSet {pin}` fails with `unsupported`: the web keeps no PIN
//!   (no files, no random source in the core); a hub keeps a managed
//!   device's lock (P1HL).
//!
//! A write refused by the lock fails with code `installer_lock` and notice
//! key `installer_lock.locked`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    Ok(tidy(&r?))
}

#[derive(Deserialize)]
struct Resolve {
    platform: String,
    #[serde(default)]
    chosen: String,
}

#[derive(Deserialize)]
struct Mode {
    #[serde(default)]
    mode: String,
}

#[derive(Deserialize)]
struct Pin {
    pin: String,
}

#[derive(Deserialize)]
struct Cover {
    pages: Vec<String>,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "modeManifest" => Ok(tidy(&dspi_core::mode_manifest())),
        "modeResolve" => {
            params::<Resolve>(p).and_then(|r| record(core.mode_resolve(r.platform, r.chosen)))
        }
        "modeSetPinned" => params::<Mode>(p).and_then(|m| {
            core.mode_set_pinned(m.mode)?;
            Ok(json!({}))
        }),
        "modePinned" => Ok(json!({ "mode": core.mode_pinned() })),
        "installerLockState" => Ok(tidy(&core.installer_lock_state())),
        "installerUnlock" => params::<Pin>(p).and_then(|x| record(core.installer_unlock(x.pin))),
        "installerLock" => record(core.installer_lock()),
        "installerPinSet" => params::<Pin>(p).and_then(|x| record(core.installer_pin_set(x.pin))),
        "installerPinClear" => record(core.installer_pin_clear()),
        "installerLockCover" => {
            params::<Cover>(p).and_then(|c| record(core.installer_lock_cover(c.pages)))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p1lk {
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
    fn wp_p1lk_the_manifest_round_trips_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let m = run(h, "modeManifest", Value::Null);
        assert_eq!(m["ok"], true, "{m}");
        let back: dspi_core::ModeManifest = serde_json::from_value(m["value"].clone()).unwrap();
        assert_eq!(back, dspi_core::mode_manifest());
    }

    #[test]
    fn wp_p1lk_modes_and_the_lock_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let c = run(h, "modeResolve", json!({ "platform": "web" }));
        assert_eq!(c["value"]["mode"], "advanced", "{c}");
        let c = run(
            h,
            "modeResolve",
            json!({ "platform": "web", "chosen": "installer" }),
        );
        assert_eq!(c["value"]["needsPin"], false, "{c}");
        let st = run(h, "installerLockState", Value::Null);
        assert_eq!(st["value"]["pinSet"], false, "{st}");
        assert_eq!(
            st["value"]["scopeNoticeKey"], "installer_lock.scope",
            "{st}"
        );
        // No PIN: nothing asked, nothing to lock.
        assert_eq!(run(h, "installerUnlock", json!({ "pin": "x" }))["ok"], true);
        let r = run(h, "installerLock", Value::Null);
        assert_eq!(r["noticeKey"], "installer_lock.no_pin", "{r}");
        assert_eq!(r["code"], "installer_lock", "{r}");
        let r = run(h, "modeSetPinned", json!({ "mode": "simple" }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(run(h, "modePinned", Value::Null)["value"]["mode"], "simple");
        let c = run(
            h,
            "modeResolve",
            json!({ "platform": "web", "chosen": "advanced" }),
        );
        assert_eq!(c["value"]["reason"], "pinned", "{c}");
        let bad = run(h, "modeResolve", json!({ "platform": "tablet" }));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }
}
