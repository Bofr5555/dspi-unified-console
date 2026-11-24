//! WP ADET1: Settings › AI Agents › Agents may detect speakers through C.

use super::*;
use crate::fns_core_set::{dspi_core_set_free, dspi_core_set_new};

#[test]
fn wp_adet1_the_detect_switch_is_set_through_c() {
    let set = dspi_core_set_new();
    let mcp = dspi_mcp_new(set);
    assert!(!mcp.is_null());
    // SAFETY: the handle made above, alive here.
    let policy = unsafe { &*mcp }.policy.clone();
    assert!(!policy.agent_detect(), "off by default");
    assert!(dspi_mcp_set_confirm(mcp, c"never_ask".as_ptr()));
    assert_eq!(
        dspi_mcp::Policy::agent_may(policy.as_ref())["detect"],
        json!(false),
        "never ask without the switch"
    );
    assert!(dspi_mcp_set_agent_detect(mcp, true));
    assert!(policy.agent_detect());
    assert_eq!(
        dspi_mcp::Policy::agent_may(policy.as_ref())["detect"],
        json!(true)
    );
    assert!(dspi_mcp_set_agent_detect(mcp, false));
    assert!(!policy.agent_detect());
    assert!(!dspi_mcp_set_agent_detect(std::ptr::null_mut(), true));
    dspi_mcp_free(mcp);
    dspi_core_set_free(set);
}
