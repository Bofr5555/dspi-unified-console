//! WP P2CC3: the hub's device and name writes without a hub. The calls
//! against the real hub (with and without the caps, the roles, read-only
//! mode) are in `crates/dspi-hub/tests/wp_p2cc3_core_calls.rs`.

use super::*;

#[test]
fn wp_p2cc3_off_a_hub_every_call_is_unsupported() {
    let core = DspiCore::new();
    let u = DspiError::Unsupported;
    assert_eq!(
        core.hub_inventory_set("S1".into(), Some("Hall".into()), None, None)
            .unwrap_err(),
        u
    );
    assert_eq!(
        core.hub_inventory_forget("S1".into(), false).unwrap_err(),
        u
    );
    assert_eq!(core.hub_rename("Studio".into()).unwrap_err(), u);
    assert_eq!(
        core.hub_identify_serial("S1".into(), 5, None).unwrap_err(),
        u
    );
    // Forget names its device: an empty serial is refused before anything.
    assert!(matches!(
        core.hub_inventory_forget(String::new(), true),
        Err(DspiError::InvalidArgument { .. })
    ));
}
