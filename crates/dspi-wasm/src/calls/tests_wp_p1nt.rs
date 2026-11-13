//! WP P1NT: a failure reply's `noticeKey` and `noticeFor` give the core's notice.

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
fn wp_p1nt_wasm_gives_the_cores_notice_for_a_failure_reply() {
    guard::reset_for_test();
    let h = session_new().unwrap();
    for e in [
        DspiError::Disconnected,
        DspiError::Locked,
        DspiError::Refused { status: 4 },
        DspiError::Link {
            code: "wrong_pin".into(),
            msg: "the hub said no".into(),
        },
        DspiError::OvervoltWordRequired {
            msg: "type OVERVOLT".into(),
        },
    ] {
        let failure: Value = serde_json::from_slice(&reply(Err(e.clone().into()))).unwrap();
        assert_eq!(failure["ok"], false);
        let key = failure["noticeKey"].as_str().unwrap();
        let r = run(
            h,
            "noticeFor",
            json!({ "key": key, "detail": e.notice_detail() }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let want = serde_json::to_value(dspi_core::notice_for(e.clone())).unwrap();
        assert_eq!(r["value"], want, "{e:?}");
    }
    // The binding's own refusals have no notice key.
    let own: Value = serde_json::from_slice(&reply(Err(CallError::new("busy", "x")))).unwrap();
    assert!(own.get("noticeKey").is_none());
    let table = run(h, "noticeTexts", Value::Null);
    let rows = table["value"].as_array().unwrap();
    assert_eq!(rows.len(), dspi_core::notice_texts().len());
    assert!(session_free(h).unwrap());
}
