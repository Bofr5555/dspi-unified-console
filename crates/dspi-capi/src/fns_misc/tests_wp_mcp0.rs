//! WP MCP0 tests: the history's ids and actors, and reverting one change
//! by id, through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_get_state, dspi_set_output_gain};
use dspi_core::actor::{ActorScope, register_actor};

/// A text call's reply as JSON.
fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0u8; 8192];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn gain(h: *mut FfiCore, o: usize) -> f32 {
    // SAFETY: the handle is live; the record stays valid until the next call.
    unsafe { (*dspi_get_state(h)).output_gain_db[o] }
}

#[test]
fn wp_mcp0_an_agents_change_is_named_and_reverted_by_id_through_c() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let agent = register_actor("AI · Claude Code");
    {
        // SAFETY: the handle is live until `dspi_core_free` below.
        let core = unsafe { &(*h).core };
        let _scope = ActorScope::enter(core, agent, None);
        core.set_output_gain(1, -5.0).unwrap();
    }
    assert!(dspi_set_output_gain(h, 0, -2.0));

    let r = reply(|b, l| dspi_history(h, 2, b, l));
    let (user, ai) = (&r["value"][0], &r["value"][1]);
    assert_eq!(user["actor"], 0, "{r}");
    assert_eq!(user["actorLabel"], Value::Null);
    assert_eq!(ai["actor"], agent, "{r}");
    assert_eq!(ai["actorLabel"], "AI · Claude Code");
    let id = ai["id"].as_u64().unwrap();
    assert_ne!(user["id"].as_u64().unwrap(), id);

    // The activity log's per-row Undo: the agent's change only.
    let r = reply(|b, l| dspi_revert_entry(h, id, b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["entry"]["id"], id);
    assert_eq!(gain(h, 1), 0.0);
    assert_eq!(gain(h, 0), -2.0);

    let r = reply(|b, l| dspi_revert_entry(h, u64::MAX, b, l));
    assert_eq!(
        (r["ok"].clone(), r["code"].clone()),
        (json!(false), json!("invalid"))
    );
    dspi_core_free(h);
}
