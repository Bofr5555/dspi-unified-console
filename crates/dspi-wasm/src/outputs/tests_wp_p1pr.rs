//! WP P1PR: `protectAllOutputs`, `outputsWithoutLimiter` and `inputPairPreamp`
//! on the demo, each one undo step.

use serde_json::{Value, json};

use crate::calls::{call, session_new};
use crate::guard;

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

fn limiters(st: &Value) -> Value {
    // the enable is left out: switching a limiter is never undone
    json!([
        st["limiterThresholdDb"].clone(),
        st["limiterLinkGroup"].clone()
    ])
}

#[test]
fn wp_p1pr_protect_all_outputs_through_wasm() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    let st = ok(h, "open", json!({ "kind": "demo" }))["state"].clone();
    let out1 = st["numInputChannels"].as_u64().unwrap();
    let w = |path: &str, ix: Value, v: Value| {
        ok(
            h,
            "write",
            json!({ "path": path, "indices": ix, "value": v }),
        );
    };
    // every limiter off, outputs 0 and 1 loud
    w("limit.on", json!([255]), json!({ "bool": false }));
    w("limit.threshold", json!([255]), json!({ "float": -6.0 }));
    w("out.gain", json!([0]), json!({ "float": 0.0 }));
    w("out.gain", json!([1]), json!({ "float": -30.0 }));
    let warn = ok(h, "outputsWithoutLimiter", Value::Null);
    assert!(warn.as_array().unwrap().contains(&json!(0)), "{warn}");
    assert!(!warn.as_array().unwrap().contains(&json!(1)), "{warn}");
    assert_eq!(
        ok(h, "outputsWithoutLimiter", json!({ "thresholdDb": 6.0 })),
        json!([])
    );
    let before = limiters(&ok(h, "state", Value::Null)["state"]);

    let bad = run(h, "protectAllOutputs", json!({ "ceilingDbfs": "loud" }));
    assert_eq!(bad["ok"], false, "{bad}");
    assert_eq!(ok(h, "protectAllOutputs", Value::Null), Value::Null);
    let st = ok(h, "state", Value::Null)["state"].clone();
    let n = st["numOutputChannels"].as_u64().unwrap() as usize;
    for k in 0..n {
        assert_eq!(st["limiterEnabled"][k], true, "output {k}");
        assert_eq!(st["limiterThresholdDb"][k], -1.0, "output {k}");
    }
    assert_eq!(st["limiterLinkGroup"][0], 1);
    assert_eq!(st["limiterLinkGroup"][1], 1);
    assert_eq!(ok(h, "outputsWithoutLimiter", Value::Null), json!([]));
    let hist = ok(h, "history", json!({ "n": 1 }));
    assert_eq!(hist[0]["label"], "Protect all outputs", "{hist}");

    ok(h, "undo", Value::Null);
    assert_eq!(
        limiters(&ok(h, "state", Value::Null)["state"]),
        before,
        "one undo"
    );

    ok(h, "protectAllOutputs", json!({ "ceilingDbfs": -3.0 }));
    let st = ok(h, "state", Value::Null)["state"].clone();
    assert_eq!(st["limiterThresholdDb"][0], -3.0);

    // a linked pair's preamp: both inputs, one undo
    let pre = |st: &Value| {
        json!([
            st["inputPreampDb"][0].clone(),
            st["inputPreampDb"][1].clone()
        ])
    };
    let start = pre(&ok(h, "state", Value::Null)["state"]);
    ok(h, "inputPairPreamp", json!({ "input": 1, "db": -4.5 }));
    assert_eq!(
        pre(&ok(h, "state", Value::Null)["state"]),
        json!([-4.5, -4.5])
    );
    ok(h, "undo", Value::Null);
    assert_eq!(pre(&ok(h, "state", Value::Null)["state"]), start);
    let r = run(h, "inputPairPreamp", json!({ "input": out1, "db": 0.0 }));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "invalid", "{r}");
}
