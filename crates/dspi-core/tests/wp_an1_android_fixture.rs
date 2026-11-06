//! WP AN1: `apps/android/fixtures/dspstate_*.json`, the state the Android demo
//! mode and its tests start from, were written by the old core's decoder. This
//! holds the new one to them: the same capture must give the same state, or the
//! fixtures must be regenerated on purpose (`DSPI_WRITE_ANDROID_FIXTURE=1`).

use std::path::PathBuf;

use dspi_core::DspiCore;
use dspi_core::consts::COMPAT_OK;
use dspi_core::records::DspState;
use dspi_core::state::{apply_platform, decode_bulk};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/android/fixtures")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Every difference between two JSON values, floats compared as `f32`, which is
/// what both sides are.
fn differences(path: &str, old: &Value, new: &Value, out: &mut Vec<String>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                match b.get(k) {
                    Some(w) => differences(&format!("{path}.{k}"), v, w, out),
                    None => out.push(format!("{path}.{k}: gone")),
                }
            }
            for k in b.keys().filter(|k| !a.contains_key(*k)) {
                out.push(format!("{path}.{k}: new"));
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}[{i}]"), v, w, out);
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            if a.as_f64().map(|x| x as f32) != b.as_f64().map(|x| x as f32) {
                out.push(format!("{path}: {a} -> {b}"));
            }
        }
        (a, b) if a == b => {}
        (a, b) => out.push(format!("{path}: {a} -> {b}")),
    }
}

/// Compare with the fixture `name`, or rewrite it on purpose.
fn check(name: &str, state: &DspState) {
    let new = serde_json::to_value(state).expect("json");
    if std::env::var_os("DSPI_WRITE_ANDROID_FIXTURE").is_some() {
        let text = serde_json::to_string_pretty(&new).expect("json");
        std::fs::write(fixtures().join(name), text + "\n").expect("write");
        return;
    }
    let old: Value = serde_json::from_slice(&read(name)).expect("fixture");
    let mut diffs = Vec::new();
    differences("", &old, &new, &mut diffs);
    assert!(
        diffs.is_empty(),
        "{name}: {} differences:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

#[test]
fn wp_an1_new_decoder_matches_the_android_capture_fixture() {
    let mut state = DspiCore::new().get_state();
    apply_platform(&read("platform.bin"), &mut state).expect("platform");
    decode_bulk(&read("bulk_v32.bin"), &mut state).expect("bulk");
    state.compat = COMPAT_OK; // the caller's to set (`apply_platform`)
    check("dspstate_v32.json", &state);
}

#[test]
fn wp_an1_state_before_any_device_matches_the_android_fixture() {
    check("dspstate_default.json", &DspiCore::new().get_state());
}
