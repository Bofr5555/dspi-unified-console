//! WP P5DOCK tests: the same text through the web binding as from the core.

use crate::calls::{call, session_new};
use serde_json::{Value, json};

const UMIK: &str = include_str!("../../../dspi-measure/testdata/p5/cal/umik1_7000001.txt");
const TARGET: &str = include_str!("../../../dspi-measure/testdata/p5/cal/target_rew.txt");

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

/// `savedUtc` holds the second the document was made: equal apart from it.
fn without_time(text: &str) -> Value {
    let mut v: Value = serde_json::from_str(text).unwrap();
    v["meta"]["savedUtc"] = Value::Null;
    v
}

#[test]
fn wp_p5dock_the_same_text_through_the_web_binding() {
    crate::guard::reset_for_test();
    let h = session_new().unwrap();
    let mic = ok(
        h,
        "micImport",
        json!({ "name": "Mic", "fileName": "umik1_7000001.txt", "text": UMIK }),
    );
    let core =
        dspi_core::mic_import("Mic".into(), "umik1_7000001.txt".into(), UMIK.into()).unwrap();
    let mic_text = mic["text"].as_str().unwrap().to_owned();
    assert_eq!(without_time(&mic_text), without_time(&core));

    let t = ok(
        h,
        "targetImport",
        json!({ "name": "T", "text": TARGET, "use": "headphones", "coupler": "iec60318_4" }),
    );
    let core_t = dspi_core::target_import(
        "T".into(),
        TARGET.into(),
        "headphones".into(),
        "iec60318_4".into(),
    )
    .unwrap();
    assert_eq!(
        without_time(t["text"].as_str().unwrap()),
        without_time(&core_t)
    );

    assert_eq!(
        ok(h, "targetBuiltins", json!({})),
        serde_json::to_value(dspi_core::target_builtins()).unwrap()
    );
    let info = ok(h, "measureDocInfo", json!({ "text": mic_text }));
    assert_eq!(
        (info["kind"].clone(), info["name"].clone()),
        (json!("mic"), json!("Mic"))
    );

    // Refusals.
    assert_eq!(
        run(h, "micImport", json!({ "text": "no numbers" }))["code"],
        "invalid"
    );
    assert_eq!(
        run(h, "targetImport", json!({ "text": TARGET, "use": "room" }))["code"],
        "invalid"
    );
    assert_eq!(
        run(h, "measureDocInfo", json!({ "text": "{}" }))["code"],
        "invalid"
    );
    // An export takes a measurement's text, never an id.
    assert_eq!(
        run(
            h,
            "measurementExportRew",
            json!({ "text": "1763287200000", "output": 0 })
        )["code"],
        "invalid"
    );
    let r = run(
        h,
        "measurementExportRew",
        json!({ "text": mic_text, "output": 0 }),
    );
    assert_eq!(r["code"], "invalid", "{r}");
}
