//! WP ROLES: Settings › AI Agents › Agents may set speaker roles from my
//! words through C.

use super::*;
use crate::fns_core_set::{dspi_core_set_free, dspi_core_set_new};

#[test]
fn wp_roles_the_roles_switch_is_set_through_c() {
    let set = dspi_core_set_new();
    let mcp = dspi_mcp_new(set);
    assert!(!mcp.is_null());
    // SAFETY: the handle made above, alive here.
    let policy = unsafe { &*mcp }.policy.clone();
    assert!(!policy.agent_roles(), "off by default");
    assert!(dspi_mcp_set_confirm(mcp, c"never_ask".as_ptr()));
    let may = || dspi_mcp::Policy::agent_may(policy.as_ref());
    assert_eq!(
        may()["set_roles"],
        json!(false),
        "never ask without the switch"
    );
    assert!(dspi_mcp_set_agent_roles(mcp, true));
    assert!(policy.agent_roles());
    assert_eq!(may()["set_roles"], json!(true));
    // The detection's switch is its own.
    assert_eq!(may()["detect"], json!(false));
    assert!(dspi_mcp_set_agent_roles(mcp, false));
    assert!(!policy.agent_roles());
    assert!(dspi_mcp_set_confirm(mcp, c"ask_risky".as_ptr()));
    assert_eq!(may()["set_roles"], json!(true), "asked instead");
    assert!(!dspi_mcp_set_agent_roles(std::ptr::null_mut(), true));
    dspi_mcp_free(mcp);
    dspi_core_set_free(set);
}
