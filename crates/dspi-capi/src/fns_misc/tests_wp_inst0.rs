//! WP INST0: the Installation setup through C. The hub end to end is
//! dspi-hub's `tests/wp_inst0_installation.rs`; here the shapes, the pure
//! calls and the failures off a hub.

use std::ffi::CString;
use std::ptr::null_mut;

use serde_json::json;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
    let needed = f(null_mut(), 0);
    assert!(needed > 0);
    let mut buf = vec![0u8; needed as usize];
    assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn c(v: &Value) -> CString {
    CString::new(v.to_string()).unwrap()
}

#[test]
fn wp_inst0_c_edit_validate_diff_and_save_off_a_hub() {
    let core = dspi_core_new();
    let site = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "serial": "S-TX"}]
    });
    let draft = json!({"revision": 3, "base": site, "model": site});
    let edits = json!([
        {"op": "addUnit", "serial": "S-A", "name": "Left", "room": "main", "role": "receiver", "upstream": "tx"},
        {"op": "addZone", "name": "All", "rooms": ["main"]}
    ]);
    let (d, e) = (c(&draft), c(&edits));
    let edited = reply(|b, l| dspi_installation_draft_edit(core, d.as_ptr(), e.as_ptr(), b, l));
    assert_eq!(edited["ok"], true, "{edited}");
    assert_eq!(edited["revision"], 3);
    assert_eq!(edited["model"]["units"][0]["id"], "Left");
    // The base as read, in the hub's full form; no unit in it.
    assert_eq!(edited["base"]["senders"][0]["serial"], "S-TX");
    assert_eq!(edited["base"]["units"], json!([]));

    let mut back = edited.clone();
    back.as_object_mut().unwrap().remove("ok");
    let d2 = c(&back);
    let issues = reply(|b, l| dspi_installation_validate(core, d2.as_ptr(), b, l));
    assert_eq!(issues, json!({"ok": true, "value": []}));
    let diff = reply(|b, l| dspi_installation_diff(core, d2.as_ptr(), b, l));
    assert_eq!(diff["value"], json!(["Adds zone All", "Adds unit Left"]));

    // An edit that cannot be made names itself; nothing changes.
    let bad = c(&json!([{"op": "setUnitRoom", "id": "Left", "room": "nowhere"}]));
    let r = reply(|b, l| dspi_installation_draft_edit(core, d2.as_ptr(), bad.as_ptr(), b, l));
    assert_eq!(
        (r["ok"].clone(), r["code"].clone()),
        (json!(false), json!("invalid"))
    );
    assert!(r["error"].as_str().unwrap().contains("edit 1"), "{r}");
    let junk = c(&json!([{"op": "paint"}]));
    let r = reply(|b, l| dspi_installation_draft_edit(core, d2.as_ptr(), junk.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid");

    // A draft the hub would refuse: refused here, before anything else.
    let mut broken = back.clone();
    broken["model"]["units"][0]["upstream"] = json!("ghost");
    let d3 = c(&broken);
    let issues = reply(|b, l| dspi_installation_validate(core, d3.as_ptr(), b, l));
    assert_eq!(issues["value"][0]["subject"], "Left");
    let r = reply(|b, l| dspi_installation_save(core, d3.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid", "{r}");
    // A good one off a hub; the view too.
    let r = reply(|b, l| dspi_installation_save(core, d2.as_ptr(), b, l));
    assert_eq!(r["code"], "disconnected", "{r}");
    let r = reply(|b, l| dspi_installation_view(core, b, l));
    assert_eq!(r["code"], "disconnected", "{r}");
    // Not a draft.
    let nd = c(&json!({"revision": 1, "model": {"nope": true}}));
    let r = reply(|b, l| dspi_installation_validate(core, nd.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid");
    dspi_core_free(core);

    // A stale save's failure, as the core gives it, reaches C as `link`
    // with the notice key the docs name.
    let e = DspiError::Link {
        code: "conflict".into(),
        msg: String::new(),
    };
    assert_eq!(e.notice_key(), "dlink.conflict");
    let e = DspiError::Link {
        code: "denied".into(),
        msg: String::new(),
    };
    assert_eq!(e.notice_key(), "dlink", "the hub's words name the role");
}
