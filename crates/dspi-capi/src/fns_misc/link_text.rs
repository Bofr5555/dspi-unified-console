//! C functions: the wording clients share (WP CG10), over `dspi_core::link_text`.
//!
//! None takes a core handle, needs a feature or blocks. Replies as in the parent
//! module's docs. A reply too long for `buf` writes nothing and returns the length
//! needed, so ask with a null `buf` and call again.

use std::ffi::c_char;

use dspi_core::{DspiError, LinkPhaseText, Result};
use serde_json::{Value, json};

use super::{c_str, copy_text, guard, json_reply};

fn text_json(t: LinkPhaseText) -> Value {
    json!({
        "key": t.key,
        "phase": t.phase,
        "text": t.text,
        "actionKey": t.action_key,
        "action": t.action,
    })
}

/// What to tell the user about a Link phase, in JSON: {ok, key, phase, text,
/// actionKey, action}, or {ok:false, code:"invalid", error} for a `phase` that is
/// no phase. `phase` is the name `dspi_link_status` reports. `pin_source` is its
/// `pinSource`; it matters only to `needs_pairing`, and null reads as `window`.
/// `key` is what to localise by: the phase's name, or `needs_pairing.window` or
/// `needs_pairing.fixed`. `actionKey` and `action` are null, or a button the phase
/// must show: `reconnecting`'s calls `dspi_link_close`.
/// Returns the bytes needed including the NUL, 0 for a null `phase` or a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_phase_text(
    phase: *const c_char,
    pin_source: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        if phase.is_null() {
            return 0;
        }
        let reply: Result<Value> = c_str(phase)
            .and_then(dspi_core::link_phase_from_key)
            .map(|p| {
                let source = c_str(pin_source).unwrap_or("").to_owned();
                text_json(dspi_core::link_phase_text(p, source))
            })
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: "not a Link phase".to_owned(),
            });
        copy_text(&json_reply(reply), buf, len)
    })
}

/// The whole wording table in JSON, every key once: {ok, value:[{key, phase, text,
/// actionKey, action}]}. Returns the bytes needed including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_link_phase_texts(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        let table: Vec<Value> = dspi_core::link_phase_texts()
            .into_iter()
            .map(text_json)
            .collect();
        copy_text(&json_reply(Ok(table)), buf, len)
    })
}

/// The release notes, as the JSON document they are: [{version, headline, items}],
/// newest first; no `ok` wrapper. Returns the bytes needed including the NUL, 0 for
/// a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_onboarding_whats_new_json(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(&dspi_core::onboarding_whats_new_json(), buf, len)
    })
}

/// The Getting Started wizard's data, as the JSON document it is; no `ok` wrapper.
/// `dspi_core::link_text` describes it. Returns the bytes needed including the NUL,
/// 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_onboarding_getting_started_json(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(&dspi_core::onboarding_getting_started_json(), buf, len)
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg10 {
    use std::ffi::{CString, c_char};
    use std::ptr;

    use serde_json::Value;

    use super::*;

    /// Everything a function replies, by asking for the length first.
    fn read(f: impl Fn(*mut c_char, u32) -> u32) -> String {
        let needed = f(ptr::null_mut(), 0);
        assert!(needed > 0);
        let mut buf = vec![0u8; needed as usize];
        assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
        assert_eq!(buf.pop(), Some(0));
        String::from_utf8(buf).unwrap()
    }

    fn text(phase: &str, pin_source: Option<&str>) -> Value {
        let phase = CString::new(phase).unwrap();
        let source = pin_source.map(|s| CString::new(s).unwrap());
        let source = source.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        let reply = read(|buf, len| dspi_link_phase_text(phase.as_ptr(), source, buf, len));
        serde_json::from_str(&reply).unwrap()
    }

    const PHASES: [&str; 8] = [
        "idle",
        "needs_pairing",
        "no_device",
        "choose_device",
        "bound",
        "reconnecting",
        "closed",
        "refused",
    ];

    #[test]
    fn wp_cg10_every_phase_has_text_over_c() {
        for phase in PHASES {
            let t = text(phase, None);
            assert_eq!(t["ok"], true, "{phase}");
            assert_eq!(t["phase"], phase);
            assert!(!t["text"].as_str().unwrap().is_empty(), "{phase}");
        }
        assert_eq!(text("needs_pairing", None)["key"], "needs_pairing.window");
        assert_eq!(
            text("needs_pairing", Some("fixed"))["key"],
            "needs_pairing.fixed"
        );
        let lost = text("reconnecting", Some("fixed"));
        assert_eq!(lost["key"], "reconnecting");
        assert_eq!(lost["text"], "Connection lost, reconnecting");
        assert_eq!(lost["actionKey"], "reconnecting.disconnect");
        assert_eq!(lost["action"], "Disconnect");
        let no_device = text("no_device", None);
        assert_eq!(no_device["text"], "Connected to hub, no DSPi found");
        assert_eq!(no_device["actionKey"], Value::Null);
    }

    #[test]
    fn wp_cg10_bad_arguments_over_c() {
        assert_eq!(
            dspi_link_phase_text(ptr::null(), ptr::null(), ptr::null_mut(), 0),
            0
        );
        let t = text("connecting", None);
        assert_eq!(t["ok"], false);
        assert_eq!(t["code"], "invalid");
    }

    #[test]
    fn wp_cg10_short_buffer_writes_nothing() {
        let mut buf = [0x55u8; 8];
        let needed = dspi_onboarding_whats_new_json(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize > buf.len());
        assert_eq!(buf, [0x55u8; 8]);
    }

    #[test]
    fn wp_cg10_table_and_onboarding_over_c() {
        let table: Value = serde_json::from_str(&read(|b, l| dspi_link_phase_texts(b, l))).unwrap();
        assert_eq!(table["ok"], true);
        let keys: Vec<&str> = table["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys.len(), PHASES.len() + 1);
        assert!(keys.contains(&"needs_pairing.fixed"));

        let notes: Value =
            serde_json::from_str(&read(|b, l| dspi_onboarding_whats_new_json(b, l))).unwrap();
        assert!(notes[0]["version"].is_string());
        let wizard: Value =
            serde_json::from_str(&read(|b, l| dspi_onboarding_getting_started_json(b, l))).unwrap();
        assert_eq!(wizard["stages"][0]["id"], "welcome");
    }

    #[test]
    fn wp_cg10_the_header_declares_them() {
        let header = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/dspi_core.h");
        let header = std::fs::read_to_string(header).unwrap();
        for f in [
            "dspi_link_phase_text(",
            "dspi_link_phase_texts(",
            "dspi_onboarding_whats_new_json(",
            "dspi_onboarding_getting_started_json(",
        ] {
            assert!(header.contains(f), "{f} is not in the header");
        }
    }
}
