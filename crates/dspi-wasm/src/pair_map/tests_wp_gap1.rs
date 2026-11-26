//! WP GAP1: the crossfeed's pair map and the outputs' polarity on the demo
//! device, as the web asks for them.

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

fn ok(h: u32, method: &str, params: Value) -> Value {
    let r = run(h, method, params);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["value"].clone()
}

fn pairs(v: &[(u8, u8)]) -> Value {
    json!(
        v.iter()
            .map(|(l, r)| json!({ "left": l, "right": r }))
            .collect::<Vec<_>>()
    )
}

#[test]
fn gap1_the_pair_map_reads_writes_and_undoes() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    ok(h, "open", json!({ "kind": "demo" }));
    let v = ok(h, "crossfeedPairMap", Value::Null);
    assert_eq!(v["supported"], true, "{v}");
    let n = v["pairCount"].as_u64().unwrap();
    assert_eq!(v["choice"], "default");
    assert_eq!(v["k340Mask"], 3);
    let default: Vec<(u8, u8)> = (0..n as u8).map(|p| (2 * p, 2 * p + 1)).collect();
    assert_eq!(v["pairs"], pairs(&default));

    // One DAC per ear: the core's map, read back into the mirror
    let v = ok(h, "setCrossfeedPairMap", json!({ "choice": "k340" }));
    assert_eq!(v["choice"], "k340");
    assert_eq!(v["pairs"][0], json!({ "left": 2, "right": 0 }));
    assert_eq!(v["pairs"][1], json!({ "left": 3, "right": 1 }));
    let st = ok(h, "state", Value::Null);
    assert_eq!(st["state"]["crossfeedPairMap"], json!([0x02, 0x13, 0, 0]));
    // One undo step
    ok(h, "undo", Value::Null);
    assert_eq!(ok(h, "crossfeedPairMap", Value::Null)["choice"], "default");

    // Custom pairs: checked first, a default pair written as 0
    let mut custom = default.clone();
    custom[0] = (1, 0);
    assert_eq!(
        ok(h, "crossfeedPairsValid", json!({ "pairs": pairs(&custom) })),
        true
    );
    let v = ok(h, "setCrossfeedPairMap", json!({ "pairs": pairs(&custom) }));
    assert_eq!(v["choice"], "custom");
    assert_eq!(v["pairs"], pairs(&custom));
    assert_eq!(
        ok(h, "state", Value::Null)["state"]["crossfeedPairMap"][0],
        0x01
    );
    assert_eq!(
        ok(h, "state", Value::Null)["state"]["crossfeedPairMap"][1],
        0
    );

    // An output in two pairs, a pair on one output, a pair too few: refused,
    // nothing sent
    let mut twice = default.clone();
    twice[1] = (0, 3);
    let mut same = default.clone();
    same[0] = (1, 1);
    for bad in [pairs(&twice), pairs(&same), pairs(&default[1..])] {
        assert_eq!(ok(h, "crossfeedPairsValid", json!({ "pairs": bad })), false);
        let r = run(h, "setCrossfeedPairMap", json!({ "pairs": bad }));
        assert_eq!(r["code"], "invalid", "{r}");
    }
    assert_eq!(
        ok(h, "crossfeedPairMap", Value::Null)["pairs"],
        pairs(&custom)
    );
    let r = run(h, "setCrossfeedPairMap", json!({ "choice": "mine" }));
    assert_eq!(r["code"], "invalid", "{r}");
    // Back to the default
    assert_eq!(
        ok(h, "setCrossfeedPairMap", json!({ "choice": "default" }))["choice"],
        "default"
    );
}

#[test]
fn gap1_the_snapshot_carries_every_outputs_polarity() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    ok(h, "open", json!({ "kind": "demo" }));
    let snap = ok(h, "state", Value::Null);
    let outs = snap["state"]["numOutputChannels"].as_u64().unwrap() as usize;
    let pol = snap["outputPolarity"].as_array().unwrap();
    assert_eq!(pol.len(), outs);
    // The demo feeds Out 1 from USB 1 only, as it comes; Out 9 from nothing
    assert_eq!(snap["state"]["matrixRouting"][0][0], true);
    assert_eq!(pol[0], "normal");
    // Inverting its one route inverts the output; a second, plain route
    // makes it mixed
    ok(
        h,
        "crosspoint",
        json!({ "input": 0, "output": 0, "enabled": true, "gain": 0.0, "invert": true }),
    );
    assert_eq!(ok(h, "state", Value::Null)["outputPolarity"][0], "inverted");
    ok(
        h,
        "crosspoint",
        json!({ "input": 1, "output": 0, "enabled": true, "gain": 0.0, "invert": false }),
    );
    assert_eq!(ok(h, "state", Value::Null)["outputPolarity"][0], "mixed");
    let last = ok(h, "state", Value::Null)["outputPolarity"][outs - 1].clone();
    assert_eq!(last, "none");
}
