//! BUG1 tests: output labels and cards through C, on the K340's own
//! configuration (tests/fixtures/preset/k340-bug1.dspipreset).

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

const K340: &str = include_str!("../../../../tests/fixtures/preset/k340-bug1.dspipreset");

fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 8192];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

#[test]
fn bug1_labels_and_cards_through_c() {
    assert_eq!(
        dspi_output_labels(std::ptr::null_mut(), std::ptr::null_mut(), 0),
        0
    );
    assert_eq!(
        dspi_output_cards(std::ptr::null_mut(), std::ptr::null_mut(), 0),
        0
    );
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let doc = CString::new(K340).unwrap();
    let r = reply(|b, l| dspi_config_import(h, doc.as_ptr(), 2, b, l));
    assert_eq!(r["ok"], true, "{r}");

    let r = reply(|b, l| dspi_output_labels(h, b, l));
    assert_eq!(r["ok"], true, "{r}");
    let labels: Vec<&str> = r["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels[..4],
        [
            "Out 1 · R · Low",
            "Out 2 · R · High",
            "Out 3 · L · Low",
            "Out 4 · L · High"
        ]
    );
    assert_eq!(r["value"][0]["slotName"], "I2S 1 L");
    assert_eq!(r["value"][0]["userNamed"], false);

    let r = reply(|b, l| dspi_output_cards(h, b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        r["value"],
        serde_json::json!([
            { "left": 2, "right": 0, "title": "Woofers", "leftName": "Woofers L", "rightName": "Woofers R" },
            { "left": 3, "right": 1, "title": "Tweeters", "leftName": "Tweeters L", "rightName": "Tweeters R" },
        ])
    );
    dspi_core_free(h);
}
