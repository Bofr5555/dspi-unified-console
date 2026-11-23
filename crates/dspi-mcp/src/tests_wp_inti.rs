//! WP INTI: FUNC2b's and FUNC2c's preset-load fixes together. Under an
//! installer's limits profile, a load the tweeter rule refuses goes back to
//! the slot active before (FUNC2b) and tells nothing of the profile's hold;
//! a load the profile held says so (FUNC2c `limits_held`), and a later load
//! the profile held nothing of carries no older rows. The load guard's
//! silent master (-127 dB) is back each time.

use serde_json::json;

use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{LR4_HP, blank, confirm, demo, err, hp, n_in, ok, server_of};

#[test]
fn wp_inti_a_refused_load_and_a_held_load_under_a_limits_profile() {
    let (set, core) = demo("inti-load");
    let tweeter = n_in(&core) + 1;
    let save = |slot: u8, name: &str| {
        let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
        core.save_preset_confirmed(slot, Some(token)).unwrap();
        core.set_preset_name(slot, name.into()).unwrap();
    };
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    core.set_output_gain(0, 6.0).unwrap();
    // Slot 9 (8): the tweeter at 850 Hz and Out 1 at +6 dB: refused.
    core.set_crossover(tweeter, 0, hp(LR4_HP, 850.0)).unwrap();
    save(8, "Test");
    // Slot 3 (2): the tweeter protected, Out 1 at +6 dB: the profile holds it.
    core.set_crossover(tweeter, 0, hp(LR4_HP, 3_000.0)).unwrap();
    save(2, "Lunch");
    // Slot 2 (1): nothing for the profile to hold, the active one.
    core.set_output_gain(0, 0.0).unwrap();
    save(1, "Dinner");
    core.set_master_volume(-12.0).unwrap();
    core.limits_apply_profile(
        json!({"schemaVersion": 1, "meta": {"kind": "limits", "name": "Limits profile"},
               "limits": {"outputs": [{"output": 0, "maxGainDb": 0.0}]}})
        .to_string(),
    )
    .unwrap();
    assert!(core.limits_status().active);
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let level = |core: &dspi_core::DspiCore| core.get_state().master_volume_db;
    assert!((level(&core) - -12.0).abs() < 0.01);

    // Refused: back on Dinner, its settings, the master as it was, no hold told.
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 8}));
    assert!(
        e.contains("drives a tweeter") && e.contains("put back"),
        "{e}"
    );
    assert!(!e.contains("limits_held") && !e.contains("still 9"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 1, "left on the refused slot");
    assert!((st.xover[usize::from(tweeter)][0].freq - 3_000.0).abs() < 1.0);
    assert!(
        st.output_gain_db[0].abs() < 0.01,
        "{}",
        st.output_gain_db[0]
    );
    assert!((level(&core) - -12.0).abs() < 0.01, "{}", level(&core));

    // Held: Lunch is loaded, its +6 dB held to 0 dB, and the agent is told.
    let r = ok(&mut s, c, "dspi_preset_load", json!({"slot": 2}));
    let held = r["limits_held"].as_array().cloned().unwrap_or_default();
    assert!(
        held.iter()
            .any(|x| x.as_str().is_some_and(|x| x.starts_with("Out 1 gain"))),
        "{r}"
    );
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 2);
    assert!(st.output_gain_db[0] <= 0.01, "{}", st.output_gain_db[0]);
    assert!((level(&core) - -12.0).abs() < 0.5, "{}", level(&core));

    // Refused again from Lunch: back on Lunch, still held.
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 8}));
    assert!(e.contains("put back") && !e.contains("limits_held"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 2, "left on the refused slot");
    assert!(st.output_gain_db[0] <= 0.01, "{}", st.output_gain_db[0]);

    // Dinner holds nothing: no older rows.
    let r = ok(&mut s, c, "dspi_preset_load", json!({"slot": 1}));
    assert!(r.get("limits_held").is_none(), "{r}");
    assert_eq!(core.get_state().active_preset_slot, 1);
    assert!((level(&core) - -12.0).abs() < 0.5, "{}", level(&core));
}
