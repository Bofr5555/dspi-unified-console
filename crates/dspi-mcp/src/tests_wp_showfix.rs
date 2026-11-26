//! WP SHOWFIX (SHOW2 finding 9): an agent said "these settings are saved to
//! slot 0, 'Movie night'" while the screen said preset 1, and explained "the
//! tools count outputs from 0". Every number an agent gives and reads now
//! counts from 1, as the screen does: preset 1 is the app's preset 1, output
//! 1 is Out 1. The wire keeps counting from 0 inside.

use serde_json::json;

use crate::confirm::GROUP_QUIET_MS; // -- FIX016 -- (R2) an answer waits for the sheet
use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{call, demo, err, ok, server_of};

#[test]
fn wp_showfix_a_preset_saves_lists_and_loads_as_the_screen_numbers_it() {
    let (set, core) = demo("showfix-preset");
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let held = call(
        &mut s,
        c,
        "dspi_preset_save",
        json!({"slot": 1, "name": "Movie night"}),
    );
    assert_eq!(
        held["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{held}"
    );
    let id = held["structuredContent"]["confirmation_id"]
        .as_u64()
        .unwrap();
    let what = s.confirmation(id).unwrap().risks[0].what.clone();
    assert!(
        what.starts_with("Save the settings to preset 1 as \u{201c}Movie night\u{201d}"),
        "{what}"
    );
    s.answer_confirmation(id, true, GROUP_QUIET_MS).unwrap();
    let r = ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(r["status"], json!("allowed"), "{r}");
    assert_eq!(r["result"]["structuredContent"]["slot"], json!(1), "{r}");
    // On the wire it is slot 0
    assert_eq!(core.get_state().active_preset_slot, 0);
    assert_eq!(core.get_preset_directory().unwrap().occupied_mask & 1, 1);

    // The list says preset 1 too, and so does the overview
    let list = ok(&mut s, c, "dspi_list_presets", json!({}));
    assert_eq!(
        list["presets"][0],
        json!({"slot": 1, "preset_name": "Movie night"}),
        "{list}"
    );
    assert_eq!(list["last_active"], json!(1), "{list}");
    let over = ok(&mut s, c, "dspi_get_overview", json!({}));
    assert_eq!(over["preset"]["last_active"], json!(1), "{over}");

    // Loaded back by the same number (held: a recall), the sheet says preset 1
    core.set_output_gain(0, -9.0).unwrap();
    let held = call(&mut s, c, "dspi_preset_load", json!({"slot": 1}));
    let id = held["structuredContent"]["confirmation_id"]
        .as_u64()
        .unwrap();
    let what = s.confirmation(id).unwrap().risks[0].what.clone();
    assert!(what.starts_with("Load preset 1"), "{what}");
    s.answer_confirmation(id, true, GROUP_QUIET_MS).unwrap();
    let r = ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    assert_eq!(r["result"]["structuredContent"]["slot"], json!(1), "{r}");
    assert_eq!(core.get_state().active_preset_slot, 0);
    assert!(core.get_state().output_gain_db[0].abs() < 0.01);

    // Slot 0 is no preset, nor is one past the last (the schema says so)
    for (slot, says) in [(0, "at least 1"), (11, "at most 10")] {
        let e = err(&mut s, c, "dspi_preset_load", json!({"slot": slot}));
        assert!(e.contains(says), "{slot}: {e}");
    }
    let schema = crate::tools::library::slot_prop();
    assert_eq!(
        (&schema["minimum"], &schema["maximum"]),
        (&json!(1), &json!(10))
    );
}

#[test]
fn wp_showfix_output_1_is_out_1_on_the_screen_and_wire_output_0() {
    let (set, core) = demo("showfix-output");
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let n_out = core.get_state().num_output_channels;
    let n_in = core.get_state().num_input_channels;

    let r = ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -7.0}),
    );
    assert!((core.get_state().output_gain_db[0] + 7.0).abs() < 0.01);
    assert_eq!(r["changes"][0]["output"], json!(1), "{r}");
    let over = ok(&mut s, c, "dspi_get_overview", json!({}));
    assert_eq!(over["outputs"][0]["output"], json!(1));
    assert_eq!(over["outputs"][0]["gain_db"], json!(-7.0));
    assert_eq!(over["inputs"][0]["input"], json!(1));
    assert_eq!(over["outputs"][0]["channel"], json!(n_in + 1));
    // In 1 and band 1 too
    let r = ok(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"input": 1, "slot": 1, "type": "peaking", "freq_hz": 300, "gain_db": -2, "q": 1}),
    );
    assert_eq!(core.get_state().filters[0][0].filter_type, 1);
    assert_eq!(r["changes"][0]["slot"], json!(1), "{r}");
    assert_eq!(r["changes"][0]["channel"], json!(1), "{r}");

    // What is out of range says the range, from 1
    let e = err(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": n_out + 1, "gain_db": -7.0}),
    );
    assert!(
        e.contains(&format!("no output {}", n_out + 1))
            && e.contains(&format!("outputs 1..{n_out}")),
        "{e}"
    );
    let e = err(&mut s, c, "dspi_get_channel", json!({"input": n_in + 1}));
    assert!(e.contains(&format!("inputs 1..{n_in}")), "{e}");
    let e = err(
        &mut s,
        c,
        "dspi_set_mute",
        json!({"output": 0, "muted": true}),
    );
    assert!(e.contains("at least 1"), "{e}");
    let e = err(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"output": 1, "slot": 11, "type": "peaking", "freq_hz": 300}),
    );
    assert!(e.contains("10"), "{e}");
    // A batch names its changes from 1
    let e = err(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 1, "gain_db": -8},
                           {"op": "set_gain", "output": n_out + 1, "gain_db": -8}]}),
    );
    assert!(
        e.starts_with(&format!("change 2: no output {}", n_out + 1)),
        "{e}"
    );
}

// -- WP FIN015 -- (REVF S3, probe P3) a preset copy echoes the presets as the
// agent named them, and the core's refusal names the preset from 1.
#[test]
fn wp_fin015_a_preset_copy_counts_from_1() {
    let (set, core) = demo("fin015-copy");
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok(&mut s, c, "dspi_preset_save", json!({"slot": 1}));
    assert_eq!(core.get_state().active_preset_slot, 0);
    let r = ok(
        &mut s,
        c,
        "dspi_preset_copy",
        json!({"source": 1, "dest": 6}),
    );
    assert_eq!((&r["source"], &r["dest"]), (&json!(1), &json!(6)), "{r}");
    // On the wire it is slot 5
    assert_ne!(
        core.get_preset_directory().unwrap().occupied_mask & (1 << 5),
        0
    );
    let e = err(
        &mut s,
        c,
        "dspi_preset_copy",
        json!({"source": 3, "dest": 6}),
    );
    assert!(e.contains("preset 3 is not the active preset"), "{e}");
}

// -- WP FIN015 -- (REVF S3, probe P4) the limits profile's outputs count from
// 1 in dspi_get_limits, as everything else the agent reads does.
#[test]
fn wp_fin015_the_limits_profile_names_its_outputs_from_1() {
    let (set, core) = demo("fin015-limits");
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    core.limits_apply_profile(
        json!({"schemaVersion": 1, "meta": {"kind": "limits", "name": "Limits profile"},
               "limits": {"outputs": [{"output": 2, "highpassMinHz": 2000.0}]}})
        .to_string(),
    )
    .unwrap();
    let r = ok(&mut s, c, "dspi_get_limits", json!({}));
    let row = &r["profile"]["limits"]["outputs"][0];
    assert_eq!(row["output"], json!(3), "{r}");
    assert_eq!(row["highpassMinHz"], json!(2000.0), "{r}");
    // The core keeps it on wire output 2 (Out 3)
    assert!(core.limits_profile_document().contains("\"output\": 2"));
}

// -- WP FIN015 -- (REVF S6) the user's changes keep codes as codes: an input
// source's name gives its code, a physical output slot's pin its slot code.
#[test]
fn wp_fin015_a_users_change_keeps_codes_as_codes() {
    let mut m = serde_json::Map::new();
    let words = crate::user::place("in.name", &[2], 4, &mut m);
    assert_eq!((words.as_str(), &m["source"]), ("", &json!(2)), "{m:?}");
    assert!(
        !m.contains_key("indices") && !m.contains_key("input"),
        "{m:?}"
    );
    let mut m = serde_json::Map::new();
    let words = crate::user::place("out.pin", &[1], 4, &mut m);
    assert_eq!((words.as_str(), &m["pin_slot"]), ("", &json!(1)), "{m:?}");
    assert!(!m.contains_key("output"), "{m:?}");
    // An output's own setting is still Out N from 1.
    let mut m = serde_json::Map::new();
    assert_eq!(crate::user::place("out.gain", &[1], 4, &mut m), "Out 2");
}

// -- WP FIN015 -- (REVF S9) a slot the device names past the last is none
// known (null), never preset 256; (REVF S10) the guide names the documents
// that keep their file's numbers.
#[test]
fn wp_fin015_an_unknown_slot_is_null_and_the_guide_names_the_file_numbers() {
    use crate::tools::read::shown_slot;
    assert_eq!(shown_slot(0), Some(1));
    assert_eq!(shown_slot(9), Some(10));
    assert_eq!(shown_slot(10), None);
    assert_eq!(shown_slot(255), None);
    let g = crate::resources::GUIDE;
    assert!(
        g.contains("keeps that format's fields, which count from 0")
            && g.contains("`dspi_snapshot_export`")
            && g.contains("(`pin_slot`)"),
        "the guide's numbers"
    );
}
