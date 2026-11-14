//! What a DSPi does at power-on, for the web's Installer page (WP P1WB2, PRO1
//! S5). The startup preset, the master volume's and the output
//! configuration's storage are `write`s (`preset.startup`, `vol.master.mode`,
//! `preset.iomode`); the DAC's hardware mute is a packet the registry leaves
//! to its typed setter, so it has these two calls:
//!
//! - `dacMuteSet {enabled, activeLow, pin, holdMs, releaseMs}`: write the
//!   config (`DspiCore::set_dac_mute`). The firmware applies it later in its
//!   main loop; read it back with `dacMuteFetch` about 150 ms after.
//! - `dacMuteFetch`: re-read it into the state (`dacMute*` fields), answering
//!   them: `{supported, enabled, activeLow, pin, holdMs, releaseMs}`.
//!
//! The pulse is `write` of `dev.dacmute.test`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

type CallResult = Result<Value, CallError>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DacMute {
    enabled: bool,
    active_low: bool,
    pin: u8,
    hold_ms: u16,
    release_ms: u16,
}

fn dac_mute(core: &DspiCore) -> Value {
    let s = core.get_state();
    json!({
        "supported": s.dac_mute_supported,
        "enabled": s.dac_mute_enabled,
        "activeLow": s.dac_mute_active_low,
        "pin": s.dac_mute_pin,
        "holdMs": s.dac_mute_hold_ms,
        "releaseMs": s.dac_mute_release_ms,
    })
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "dacMuteSet" => serde_json::from_value::<DacMute>(p)
            .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
            .and_then(|d| {
                core.set_dac_mute(d.enabled, d.active_low, d.pin, d.hold_ms, d.release_ms)?;
                Ok(Value::Null)
            }),
        "dacMuteFetch" => core
            .fetch_dac_mute()
            .map(|()| dac_mute(core))
            .map_err(Into::into),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p1wb2 {
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
    fn wp_p1wb2_power_on_fields_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        // The startup preset: slot 3, as two bytes; read back in the state.
        let r = run(
            h,
            "write",
            json!({ "path": "preset.startup", "value": { "bytes": [0, 2] } }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let st = run(h, "state", Value::Null);
        assert_eq!(st["value"]["state"]["presetDefaultSlot"], 2, "{st}");
        // The DAC mute: written, then read back.
        let r = run(
            h,
            "dacMuteSet",
            json!({ "enabled": true, "activeLow": false, "pin": 11, "holdMs": 20, "releaseMs": 40 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let r = run(h, "dacMuteFetch", Value::Null);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["holdMs"], 20, "{r}");
        assert_eq!(r["value"]["releaseMs"], 40, "{r}");
        assert_eq!(r["value"]["enabled"], true, "{r}");
        let bad = run(h, "dacMuteSet", json!({ "enabled": true }));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }
}
