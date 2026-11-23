//! WP FEATUI: the app's SPL cap for agents' measurements and the host audio
//! reading through C.

use super::*;
use crate::fns_core_set::{dspi_core_set_free, dspi_core_set_new};

#[test]
fn wp_featui_the_spl_cap_is_set_and_the_host_audio_is_read() {
    let set = dspi_core_set_new();
    let mcp = dspi_mcp_new(set);
    assert!(!mcp.is_null());
    assert!(!dspi_mcp_set_agent_spl_cap(mcp, f32::NAN), "no number");
    assert!(dspi_mcp_set_agent_spl_cap(mcp, 80.0));
    // SAFETY: the handle made above, alive here.
    let policy = unsafe { &*mcp }.policy.clone();
    assert_eq!(
        dspi_mcp::Policy::agent_spl_cap_db(policy.as_ref()),
        Some(80.0)
    );
    assert!(dspi_mcp_set_agent_spl_cap(mcp, 0.0), "0: none");
    assert_eq!(dspi_mcp::Policy::agent_spl_cap_db(policy.as_ref()), None);
    assert!(!dspi_mcp_set_agent_spl_cap(std::ptr::null_mut(), 80.0));

    let mut buf = vec![0u8; 1 << 16];
    let needed = dspi_mcp_host_audio(mcp, buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed > 0 && needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    let v: Value = serde_json::from_slice(&buf[..end]).unwrap();
    assert_eq!(v["ok"], json!(true), "{v}");
    assert!(
        v["status"] == json!("known") || v["status"] == json!("unknown"),
        "{v}"
    );
    assert_eq!(v["devices"], json!([]), "nothing open");
    assert!(v["how_to_set_default"].is_string(), "{v}");
    dspi_mcp_free(mcp);
    dspi_core_set_free(set);
}
