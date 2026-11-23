//! WP FUNC2b: what the function-test stories S12 (presets, snapshots, A/B) and
//! S15 (a long evening) found in the MCP server.

use serde_json::json;

use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{LR4_HP, blank, confirm, demo, err, hp, n_in, ok, server_of};

/// A refused preset load leaves the device on the slot that was active before,
/// so a Save (or dspi_commit) afterwards cannot overwrite the refused preset.
#[test]
fn wp_func2b_a_refused_preset_load_leaves_the_device_on_its_own_slot() {
    let (set, core) = demo("refused-slot");
    let tweeter = n_in(&core) + 1;
    // Slot 9 (8): the user's old "Test", the tweeter at 850 Hz (below 1 kHz).
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    core.set_crossover(tweeter, 0, hp(LR4_HP, 850.0)).unwrap();
    let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
    core.save_preset_confirmed(8, Some(token)).unwrap();
    core.set_preset_name(8, "Test".into()).unwrap();
    // Slot 2 (1): protected, the active one.
    core.set_crossover(tweeter, 0, hp(LR4_HP, 3_000.0)).unwrap();
    let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
    core.save_preset_confirmed(1, Some(token)).unwrap();
    assert_eq!(core.get_state().active_preset_slot, 1);
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 8}));
    assert!(
        e.contains("drives a tweeter") && e.contains("put back") && !e.contains(".."),
        "{e}"
    );
    assert!(!e.contains("still 9"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 1, "left on the refused slot");
    assert!((st.xover[usize::from(tweeter)][0].freq - 3_000.0).abs() < 1.0);

    // With unsaved changes before the load: its own slot again, and those changes kept.
    core.set_output_gain(0, -3.0).unwrap();
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 8}));
    assert!(e.contains("put back"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 1);
    assert!(
        (st.output_gain_db[0] - -3.0).abs() < 0.01,
        "{}",
        st.output_gain_db[0]
    );
    assert!((st.xover[usize::from(tweeter)][0].freq - 3_000.0).abs() < 1.0);
}

/// "Undo the last change" after a limiter set on every output at once: that
/// change cannot be undone (no value before it), and an older one is not undone
/// in its place.
#[test]
fn wp_func2b_undo_never_undoes_an_older_change_in_place_of_the_newest() {
    let (set, core) = demo("undo-all");
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok(&mut s, c, "dspi_set_leveller", json!({"enabled": true}));
    ok(
        &mut s,
        c,
        "dspi_set_limiter",
        json!({"output": "all", "enabled": true, "threshold_db": -6}),
    );
    let newest = core
        .history(10)
        .into_iter()
        .find(|e| e.actor != 0)
        .expect("the agent's limiter change");
    // The core's journal keeps no value from before a write to every output
    assert!(newest.before.is_none(), "{newest:?}");
    let e = err(&mut s, c, "dspi_undo", json!({}));
    assert!(
        e.contains("cannot be undone") && e.contains("Nothing was undone"),
        "{e}"
    );
    assert!(
        core.get_state().leveller_enabled,
        "the leveller was undone in its place"
    );
}
