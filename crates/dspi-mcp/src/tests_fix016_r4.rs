//! FIX016 R4 (REVG R4): under the installer lock an agent's speaker roles
//! are refused like any other locked write: the core's layout writers check
//! the lock, so nothing an agent states reaches the roles the tweeter guards
//! trust.

use serde_json::{Value, json};

use crate::confirm::GROUP_QUIET_MS;
use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::server_of;
use crate::tests_wp_mcp5::{call_at, demo, layout, ok_at};
use crate::tools::roles::SET_ROLES_NAME;

#[test]
fn fix016_r4_an_agents_speaker_roles_are_refused_under_the_lock() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let before = core.speaker_layout().unwrap().unwrap();
    core.installer_pin_set("~fix016~r4~".into()).unwrap();
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    assert!(core.installer_lock().unwrap().locked);
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    // Out 3, the user's tweeter, stated a woofer
    let args = json!({"user_words": "out 3 is the woofer", "what_changed": "rewired",
                      "rows": [{"output": 3, "role": "woofer", "speaker": "L"}]});
    let r = call_at(&mut s, c, 0, SET_ROLES_NAME, args);
    // Refused at once, or asked and then refused on the Allow
    let result: Value = if r["structuredContent"]["status"] == json!("awaiting_confirmation") {
        let cid = r["structuredContent"]["confirmation_id"].as_u64().unwrap();
        s.answer_confirmation(cid, true, GROUP_QUIET_MS).unwrap();
        ok_at(
            &mut s,
            c,
            GROUP_QUIET_MS,
            "dspi_get_confirmation",
            json!({"confirmation_id": cid}),
        )["result"]
            .clone()
    } else {
        r
    };
    assert_eq!(result["isError"], json!(true), "{result}");
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("installer has locked"), "{result}");
    assert_eq!(
        core.speaker_layout().unwrap().unwrap().outputs,
        before.outputs
    );
}
