//! WP WB6F: `limiterMeter`, the limiters' gain reduction for the status rail's
//! health panel: one figure in dB per output slot, 0 where nothing limits.

use super::*;

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
fn wp_wb6f_limiter_meter_on_the_demo() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    // Nothing open: refused in the core's words, not a panic.
    let r = run(h, "limiterMeter", Value::Null);
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "not_open", "{r}");
    assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
    let r = run(h, "limiterMeter", Value::Null);
    assert_eq!(r["ok"], true, "{r}");
    let gr = r["value"].as_array().unwrap();
    assert_eq!(gr.len(), dspi_core::consts::MAX_OUTPUTS);
    // The demo limits nothing.
    assert!(gr.iter().all(|v| v.as_f64() == Some(0.0)), "{r}");
    session_free(h).unwrap();
}
