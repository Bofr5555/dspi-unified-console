//! WP P2WB2: a snapshot document captured with no store (the web's) gets the
//! id a store would give it before it is filed on a hub; one with an id keeps
//! it.

use serde_json::json;

use super::*;

#[test]
fn wp_p2wb2_a_document_without_a_store_id_gets_its_capture_times() {
    let web = json!({"meta": {"kind": "snapshot"},
        "snapshot": {"serial": "S1", "savedMs": 1_763_200_000_123_i64}});
    let doc = with_store_id(web);
    assert_eq!(doc["snapshot"]["id"], "1763200000123", "{doc}");
    assert_eq!(
        doc["snapshot"]["id"],
        dspi_session::snapshot_store::id_for(1_763_200_000_123, 0)
    );
    // An id the store made stays; one that is not a store id is replaced.
    let kept = with_store_id(json!({"snapshot": {"id": "0000000000200-1", "savedMs": 5}}));
    assert_eq!(kept["snapshot"]["id"], "0000000000200-1");
    let bad = with_store_id(json!({"snapshot": {"id": "../x", "savedMs": 7}}));
    assert_eq!(bad["snapshot"]["id"], "0000000000007");
    // No block: as it was (the hub says why).
    let none = with_store_id(json!({"meta": {"kind": "snapshot"}}));
    assert_eq!(none, json!({"meta": {"kind": "snapshot"}}));
    // A capture of the demo is such a document.
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let text = core
        .snapshot_capture_document("Now".into(), String::new())
        .unwrap();
    let captured: Value = serde_json::from_str(&text).unwrap();
    assert!(captured["snapshot"].get("id").is_none(), "{captured}");
    let filed = with_store_id(captured.clone());
    assert!(
        dspi_session::snapshot_store::valid_id(filed["snapshot"]["id"].as_str().unwrap()),
        "{filed}"
    );
}
