//! WP CG-SWS-a tests: each call answers what the core's function does.

use crate::calls::{call, session_new};
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

fn ok(h: u32, method: &str, params: Value) -> Value {
    let r = run(h, method, params);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["value"].clone()
}

#[test]
fn cgswsa_wasm_stateless_calls_match_the_core() {
    let h = session_new().unwrap();
    assert_eq!(ok(h, "quantizeMasterVolume", json!({ "db": -3.26 })), -3.5);
    assert_eq!(ok(h, "quantizeMasterVolume", json!({ "db": -300 })), -128.0);
    let band = json!({ "filterType": 1, "freq": 1000.0, "q": 1.0, "gain": 6.0 });
    let s = ok(h, "curveSections", json!({ "filters": [band] }));
    assert_eq!(s.as_array().unwrap().len(), 1);
    assert!(s[0]["b0"].as_f64().unwrap() > 1.0);
    let many: Vec<Value> = (0..65).map(|_| band.clone()).collect();
    assert_eq!(
        run(h, "curveSections", json!({ "filters": many }))["code"],
        "invalid"
    );
    assert_eq!(
        ok(h, "vregCode", json!({ "code": 13, "platformId": 1 })),
        json!({ "mv": 1200, "label": "1.20 V" })
    );
    assert_eq!(
        ok(h, "vregCode", json!({ "code": 30, "platformId": 0 })),
        json!({ "mv": null, "label": "code 0x1E" })
    );
    let shape = ok(h, "platformShape", json!({ "platformId": 0 }));
    assert_eq!(shape["numInputChannels"], 2);
    assert_eq!(shape["maxDelayMs"], 21.0);
    assert_eq!(
        ok(h, "knownFeatureNames", Value::Null),
        json!(dspi_core::known_feature_names())
    );
    assert_eq!(
        ok(
            h,
            "defaultChannelNames",
            json!({ "numInputs": 2, "numOutputs": 3, "outputTypes": [1] })
        ),
        json!(["USB L", "USB R", "I2S 1 L", "I2S 1 R", "PDM"])
    );
    let l = ok(h, "enumValueLabels", json!({ "path": "up.center_mode" }));
    assert_eq!(l[2], json!({ "value": 2, "key": "off", "label": "Off" }));
    assert_eq!(
        run(h, "enumValueLabels", json!({ "path": "x" }))["code"],
        "invalid"
    );
    let t = ok(
        h,
        "linkParseTxt",
        json!({ "serviceName": "Hub", "txt": { "v": "1", "hid": "AB" } }),
    );
    assert_eq!(t["hubId"], "ab");
    assert_eq!(
        run(h, "linkParseTxt", json!({ "txt": { "hid": "AB" } }))["code"],
        "invalid"
    );
    assert_eq!(run(h, "vregCode", json!({}))["code"], "invalid");
}

#[test]
fn cgswsa_wasm_image_extras_follow_the_device() {
    let h = session_new().unwrap();
    assert_eq!(ok(h, "imageExtras", Value::Null)["decoded"], false);
    ok(h, "open", json!({ "kind": "demo" }));
    let e = ok(h, "imageExtras", Value::Null);
    assert_eq!(e["decoded"], true);
    assert_eq!(e["i2sRxPinExtPresent"].as_array().unwrap().len(), 3);
}
