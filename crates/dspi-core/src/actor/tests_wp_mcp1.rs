//! WP MCP1 (review RA, MCP0 item 6): a scope dropped out of order removes
//! its own entry, not the newest one for its core.

use super::*;

#[test]
fn wp_mcp1_a_scope_dropped_out_of_order_removes_its_own_entry() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let key = core.lock().actor_key;
    let outer_actor = register_actor("outer");
    let inner_actor = register_actor("inner");
    let outer = ActorScope::enter(&core, outer_actor, None);
    let inner = ActorScope::enter(&core, inner_actor, None);
    assert_eq!(current(key), inner_actor);
    // The older scope ends first: the newer one still counts.
    drop(outer);
    assert_eq!(current(key), inner_actor);
    drop(inner);
    assert_eq!(current(key), EMBEDDER);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "an ActorScope is for another actor")]
fn wp_mcp1_a_scope_for_the_embedder_is_a_bug() {
    let core = DspiCore::new();
    let _scope = ActorScope::enter(&core, EMBEDDER, None);
}
