//! WP GAP1: `curves` with `phase` and `groupDelay`: the edited channel's
//! phase is the core's (`compute_phase_curve`, which Qt's graph draws through
//! `dspi_compute_phase_curve`), checked against the recorded reference
//! outputs every client's curves are held to
//! (`tests/fixtures/curves/responses.jsonl`), to the call's rounding.

use super::*;
use crate::calls::{call, session_new};
use crate::guard;

fn run(h: u32, params: Value) -> Value {
    let out = call(
        h,
        json!({ "method": "curves", "params": params })
            .to_string()
            .as_bytes(),
    );
    serde_json::from_slice(&out).unwrap()
}

/// The fixture's bands (written with `{:?}`) as the page sends them.
fn bands_of(text: &str) -> Vec<Value> {
    text.split("FilterParams {")
        .skip(1)
        .map(|b| {
            let field = |name: &str| -> &str {
                let at = b.find(&format!(" {name}: ")).unwrap() + name.len() + 3;
                let rest = &b[at..];
                rest[..rest.find([',', ' ', '}']).unwrap()].trim()
            };
            json!({
                "filterType": field("filter_type").parse::<u8>().unwrap(),
                "bypass": field("bypass") == "true",
                "freq": field("freq").parse::<f32>().unwrap(),
                "q": field("q").parse::<f32>().unwrap(),
                "gain": field("gain").parse::<f32>().unwrap(),
                "qp": field("qp").parse::<f32>().unwrap(),
            })
        })
        .collect()
}

#[test]
fn gap1_curves_phase_is_the_recorded_reference_phase() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/curves/responses.jsonl"
    );
    let mut cases = 0;
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let rec: Value = serde_json::from_str(line).unwrap();
        let Some(text) = rec["filters"].as_str() else {
            continue;
        };
        let bands = bands_of(text);
        // Values a JSON number cannot carry (NaN, the infinities) are the
        // core's own tests'; the page never sends them.
        if bands
            .iter()
            .any(|b| b.as_object().unwrap().values().any(Value::is_null))
        {
            continue;
        }
        for (unwrap, key) in [(false, "phase"), (true, "phase_unwrapped")] {
            let r = run(
                h,
                json!({ "chains": [bands], "phase": true, "unwrap": unwrap }),
            );
            assert_eq!(r["ok"], true, "{r}");
            let got = r["value"]["phase"].as_array().unwrap();
            let want = rec[key].as_array().unwrap();
            assert_eq!(got.len(), want.len(), "{key} {text}");
            for (i, (g, w)) in got.iter().zip(want).enumerate() {
                let (g, w) = (g.as_f64().unwrap(), w.as_f64().unwrap());
                assert!(
                    (g - w).abs() <= 0.0005 + 1e-9 * w.abs(),
                    "{key}[{i}] {g} vs {w} for {text}"
                );
            }
        }
        cases += 1;
    }
    assert_eq!(cases, 188, "the recorded filter sets with numbers only");
}

#[test]
fn gap1_curves_adds_phase_and_group_delay_only_when_asked() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    let lr4 = json!({ "filterType": 34, "bypass": false, "freq": 1000.0, "q": 0.707, "gain": 0.0, "qp": 0.0 });
    let plain = run(h, json!({ "chains": [[lr4]] }));
    assert_eq!(plain["ok"], true, "{plain}");
    assert!(plain["value"].get("phase").is_none());
    assert!(plain["value"].get("groupDelay").is_none());

    let r = run(
        h,
        json!({ "chains": [[lr4], []], "phase": true, "groupDelay": true }),
    );
    let v = &r["value"];
    let core = compute_group_delay_curve(vec![FilterParams {
        filter_type: 34,
        bypass: false,
        freq: 1000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }]);
    let gd = v["groupDelay"].as_array().unwrap();
    assert_eq!(gd.len(), CURVE_POINTS);
    for (a, b) in gd.iter().zip(&core) {
        assert!((a.as_f64().unwrap() - b).abs() <= 0.00005);
    }
    // An LR4 low pass is 360 degrees behind at the top, wrapped near 0 and
    // unwrapped near -360 just below 20 kHz; the first chain's only
    let wrapped = v["phase"].as_array().unwrap();
    assert_eq!(wrapped.len(), CURVE_POINTS);
    assert!(wrapped.iter().all(|d| d.as_f64().unwrap().abs() <= 180.0));
    let r = run(
        h,
        json!({ "chains": [[lr4]], "phase": true, "unwrap": true }),
    );
    let last = r["value"]["phase"][CURVE_POINTS - 1].as_f64().unwrap();
    assert!(last < -300.0, "{last}");
    // No chain: flat
    let r = run(h, json!({ "phase": true, "groupDelay": true }));
    assert!(
        r["value"]["phase"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d.as_f64() == Some(0.0))
    );
    assert!(
        r["value"]["groupDelay"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d.as_f64() == Some(0.0))
    );
}
