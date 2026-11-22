//! C functions: the volume of a room or a zone of the installation (WP
//! FEAT2), over `dspi_core::link::system_volume` (feature `link`).
//!
//! An ask is JSON `{room or zone, volume: "user"|"master", levelDb or
//! stepDb}`. Both calls answer JSON {ok, ask, name, number, of, units:
//! [{unitId, name, serial, beforeDb, targetDb, afterDb, louder, state:
//! "ready"|"unchanged"|"refused"|"done", code, message}]}. Show the plan's
//! `louder` units before writing it. Replies as in the parent module's docs;
//! 64 units are about 16 KiB.

use std::ffi::c_char;

use dspi_core::link::{SystemVolumeAsk, SystemVolumePlan};
use dspi_core::{DspiError, Result};

use super::{arg, json_reply, text};

/// What `ask_json` would do to each unit's volume: the plan, nothing written.
/// Every role. Blocks on the network: off the UI thread. Failures: `invalid`
/// (an ask that is not one, a room or zone the installation lacks),
/// `unsupported` (the hub has no fan-out), `disconnected`, `link`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_system_volume_plan(
    core: *mut crate::FfiCore,
    ask_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let ask = || -> Result<SystemVolumeAsk> {
            serde_json::from_str(arg(ask_json, "ask_json")?).map_err(|e| {
                DspiError::InvalidArgument {
                    msg: format!("ask_json: {e}"),
                }
            })
        };
        json_reply(ask().and_then(|a| c.system_volume_plan(a)))
    })
}

/// Write `plan_json` (`dspi_system_volume_plan`'s answer): its ready units,
/// each to its `targetDb`, through the hub. The plan again with each unit's
/// row settled; a refused unit stops no other. Control or Admin. A call whose
/// reply did not fit **has written**: plan again to see the levels rather than
/// writing again. Blocks: off the UI thread. Failures: `invalid` and
/// `read_only` (nothing sent), `denied`, `disconnected`, `link`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_system_volume_set(
    core: *mut crate::FfiCore,
    plan_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let plan = || -> Result<SystemVolumePlan> {
            serde_json::from_str(arg(plan_json, "plan_json")?).map_err(|e| {
                DspiError::InvalidArgument {
                    msg: format!("plan_json: {e}"),
                }
            })
        };
        json_reply(plan().and_then(|p| c.system_volume_set(p)))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_feat2 {
    use std::ffi::{CStr, CString};

    use serde_json::{Value, json};

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let n = f(std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; n as usize];
        f(buf.as_mut_ptr().cast(), n);
        serde_json::from_str(CStr::from_bytes_with_nul(&buf).unwrap().to_str().unwrap()).unwrap()
    }

    #[test]
    fn wp_feat2_c_volume_calls_off_a_hub() {
        let core = dspi_core_new();
        let plan = |ask: Value| {
            let ask = CString::new(ask.to_string()).unwrap();
            reply(|b, l| dspi_system_volume_plan(core, ask.as_ptr(), b, l))
        };
        let r = plan(json!({"zone": "inside", "stepDb": -3.0}));
        assert_eq!(r["code"], "disconnected", "{r}");
        for bad in [
            json!({}),
            json!({"zone": "inside", "stepDb": -3.0, "levelDb": -20.0}),
            json!({"zone": "inside", "volume": "treble", "stepDb": -3.0}),
            json!([]),
        ] {
            assert_eq!(plan(bad)["code"], "invalid");
        }
        let nothing = CString::new(
            json!({"ask": {"room": "bar", "levelDb": -20.0}, "units": []}).to_string(),
        )
        .unwrap();
        let r = reply(|b, l| dspi_system_volume_set(core, nothing.as_ptr(), b, l));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["units"], json!([]));
        dspi_core_free(core);
    }
}
