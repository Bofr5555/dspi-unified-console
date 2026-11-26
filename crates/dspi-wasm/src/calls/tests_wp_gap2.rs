//! WP GAP2: solo one output from the web page.

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

fn ok(h: u32, method: &str, params: Value) -> Value {
    let r = run(h, method, params);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["value"].clone()
}

#[test]
fn gap2_solo_on_the_web() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    assert_eq!(ok(h, "soloState", json!({}))["output"], Value::Null);
    assert_eq!(run(h, "soloOutput", json!({ "output": 1 }))["ok"], false);
    ok(h, "open", json!({ "kind": "demo" }));

    assert_eq!(ok(h, "soloOutput", json!({ "output": 2 }))["output"], 2);
    assert_eq!(ok(h, "soloState", json!({}))["output"], 2);
    // The page's state keeps the user's mutes: none
    let s = ok(h, "state", json!({}));
    let muted = s["state"]["outputMuted"].as_array().unwrap();
    assert!(muted.iter().all(|m| m == false), "{muted:?}");
    assert_eq!(
        run(h, "soloOutput", json!({ "output": 99 }))["code"],
        "invalid"
    );
    assert_eq!(ok(h, "soloEnd", json!({}))["output"], Value::Null);
    assert_eq!(ok(h, "soloState", json!({}))["output"], Value::Null);
}
