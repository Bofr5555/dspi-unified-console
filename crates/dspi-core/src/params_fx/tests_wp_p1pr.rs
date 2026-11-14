//! WP P1PR: "protect all outputs" in the core, the warning list, and a
//! linked input pair's preamp as one undo step. All on the demo device.

use std::sync::Arc;

use super::*;

const ALL: u8 = limiter::LIMITER_ALL_OUTPUTS as u8;
const ON: u8 = limiter::LIMITER_PARAM_ENABLED as u8;
const THR: u8 = limiter::LIMITER_PARAM_THRESHOLD_DB as u8;
const REL: u8 = limiter::LIMITER_PARAM_RELEASE_MS as u8;
const LINK: u8 = limiter::LIMITER_PARAM_LINK_GROUP as u8;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

/// Every output's limiter: (enabled, threshold, release, group).
fn limiter_rows(core: &DspiCore) -> Vec<(bool, f32, f32, u8)> {
    let s = core.get_state();
    (0..usize::from(s.num_output_channels))
        .map(|k| {
            (
                s.limiter_enabled[k],
                s.limiter_threshold_db[k],
                s.limiter_release_ms[k],
                s.limiter_link_group[k],
            )
        })
        .collect()
}

/// A messy start: no output linked, every limiter off at -6 dB, each output
/// its own release, and output 2 alone in group 4.
fn messy(core: &DspiCore) {
    core.set_limiter_param(ALL, LINK, 0.0).unwrap();
    core.set_limiter_param(ALL, ON, 0.0).unwrap();
    core.set_limiter_param(ALL, THR, -6.0).unwrap();
    let n = core.get_state().num_output_channels;
    for o in 0..n {
        core.set_limiter_param(o, REL, 50.0 + f32::from(o) * 10.0)
            .unwrap();
    }
    core.set_limiter_param(2, LINK, 4.0).unwrap();
}

#[test]
fn wp_p1pr_the_groups_pair_outputs_below_the_pdm_sub() {
    // RP2350: 8 outputs and the PDM sub
    assert_eq!(protect_groups(9), [1, 1, 2, 2, 3, 3, 4, 4, 0]);
    // RP2040: 4 outputs and the PDM sub
    assert_eq!(protect_groups(5), [1, 1, 2, 2, 0]);
    // no lone output in a group, no group past 4
    assert_eq!(protect_groups(4), [1, 1, 0, 0]);
    assert_eq!(protect_groups(12), [1, 1, 2, 2, 3, 3, 4, 4, 0, 0, 0, 0]);
    assert!(protect_groups(0).is_empty());
    assert_eq!(protect_groups(1), [0]);
}

/// One call: every limiter on at the ceiling, the stereo pairs linked. One
/// undo, labelled, puts every link group, threshold and release back; the
/// limiters stay on (`limit.on` is audible: the registry never undoes it),
/// and the enable is listed apart. One redo protects again.
#[test]
fn wp_p1pr_protect_all_outputs_is_one_undo_step() {
    let core = demo();
    messy(&core);
    let before = limiter_rows(&core);
    assert!(before.iter().all(|r| !r.0), "all off to begin with");
    let n = core.history(1000).len();

    core.protect_all_outputs(-1.0).unwrap();
    let s = core.get_state();
    let outputs = s.num_output_channels;
    let groups = protect_groups(outputs);
    let after = limiter_rows(&core);
    for (k, &(on, thr, _, group)) in after.iter().enumerate() {
        assert!(on, "output {k} on");
        assert_eq!(thr, -1.0, "output {k} at the ceiling");
        assert_eq!(group, groups[k], "output {k}'s group");
    }
    // a pair shares its release, its lower output's
    for p in 0..usize::from(outputs) / 2 {
        if groups[2 * p] != 0 {
            assert_eq!(after[2 * p].2, after[2 * p + 1].2, "pair {p}");
            assert_eq!(after[2 * p].2, before[2 * p].2, "pair {p}");
        }
    }
    // the step, then the enable apart (newest first)
    let h = core.history(1000);
    assert_eq!(h.len(), n + 2, "{h:?}");
    assert_eq!(h[0].label.as_deref(), Some(PROTECT_LABEL));
    assert!(h[0].undoable);
    assert_eq!(h[1].path, "limit.on");
    assert_eq!(h[1].label.as_deref(), Some(PROTECT_LABEL));
    assert!(!h[1].undoable, "switching a limiter is never undone");

    let m = core.undo().unwrap();
    assert_eq!(m.entry.unwrap().label.as_deref(), Some(PROTECT_LABEL));
    // -- WP P1EFIX -- every client shows the note: the limiters stay on
    assert_eq!(m.note.as_deref(), Some(crate::journal::PROTECT_UNDONE_NOTE));
    let undone = limiter_rows(&core);
    let kept_on: Vec<_> = before.iter().map(|&(_, t, r, g)| (true, t, r, g)).collect();
    assert_eq!(
        undone, kept_on,
        "one undo: groups, thresholds, releases back"
    );

    core.redo().unwrap();
    assert_eq!(limiter_rows(&core), after, "one redo protects again");
    assert!(!core.can_redo());
}

/// From a device with every limiter on already, one undo restores all of it.
#[test]
fn wp_p1pr_with_the_limiters_on_one_undo_restores_everything() {
    let core = demo();
    messy(&core);
    core.set_limiter_param(ALL, ON, 1.0).unwrap();
    let before = limiter_rows(&core);
    let n = core.history(1000).len();
    core.protect_all_outputs(-2.0).unwrap();
    assert_eq!(core.history(1000).len(), n + 1, "one step");
    assert_ne!(limiter_rows(&core), before);
    // -- WP P1EFIX -- the note also when they were on before
    let m = core.undo().unwrap();
    assert_eq!(m.note.as_deref(), Some(crate::journal::PROTECT_UNDONE_NOTE));
    assert_eq!(limiter_rows(&core), before);
    // and only for that step: another undo has no such note
    core.set_output_gain(0, -3.0).unwrap();
    assert_eq!(core.undo().unwrap().note, None);
}

#[test]
fn wp_p1pr_protect_checks_the_ceiling_first_and_clamps_it() {
    let core = demo();
    messy(&core);
    let before = limiter_rows(&core);
    let n = core.history(1000).len();
    assert!(matches!(
        core.protect_all_outputs(f32::NAN),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(limiter_rows(&core), before, "nothing written");
    assert_eq!(core.history(1000).len(), n, "nothing recorded");

    core.protect_all_outputs(5.0).unwrap();
    assert!(limiter_rows(&core).iter().all(|r| r.0 && r.1 == 0.0));
    core.protect_all_outputs(-40.0).unwrap();
    assert!(limiter_rows(&core).iter().all(|r| r.0 && r.1 == -30.0));
}

/// Protecting twice records nothing the second time (the group is empty).
#[test]
fn wp_p1pr_protecting_a_protected_device_records_nothing() {
    let core = demo();
    core.protect_all_outputs(-1.0).unwrap();
    let n = core.history(1000).len();
    core.protect_all_outputs(-1.0).unwrap();
    assert_eq!(core.history(1000).len(), n);
}

#[test]
fn wp_p1pr_protect_without_a_writable_device_is_refused() {
    let core = DspiCore::new();
    assert_eq!(core.protect_all_outputs(-1.0), Err(DspiError::Disconnected));
    // V17: no limiter, and read-only in this build
    core.open_demo_wire(17).unwrap();
    assert!(core.protect_all_outputs(-1.0).is_err());
    assert!(core.history(10).is_empty());
}

/// The warning list: outputs on, above the gain threshold, limiter off.
#[test]
fn wp_p1pr_outputs_without_a_limiter_above_the_threshold() {
    let core = demo();
    messy(&core);
    let s = core.get_state();
    let n = s.num_output_channels;
    for o in 0..n {
        core.set_output_gain(o, -20.0).unwrap();
    }
    assert!(core.outputs_without_limiter(-10.0).is_empty(), "all quiet");

    // output 0 loud and on; output 1 loud with its limiter; output 3 loud
    // but switched off
    core.set_output_gain(0, -5.0).unwrap();
    core.set_output_gain(1, 0.0).unwrap();
    core.set_limiter_param(1, ON, 1.0).unwrap();
    core.set_output_gain(3, 6.0).unwrap();
    let s = core.get_state();
    assert!(s.output_enabled[0] && s.output_enabled[1]);
    if s.output_enabled[3] {
        core.set_output_enable(3, false).unwrap();
    }
    assert!(!core.get_state().output_enabled[3]);
    assert_eq!(core.outputs_without_limiter(-10.0), [0]);
    // -10 itself is not above -10
    core.set_output_gain(0, -10.0).unwrap();
    assert!(core.outputs_without_limiter(-10.0).is_empty());
    assert_eq!(core.outputs_without_limiter(-15.0), [0]);
    assert!(core.outputs_without_limiter(f32::NAN).is_empty());

    // protected: nothing to warn about
    core.set_output_gain(0, 0.0).unwrap();
    core.protect_all_outputs(-1.0).unwrap();
    assert!(core.outputs_without_limiter(-60.0).is_empty());
}

#[test]
fn wp_p1pr_no_warning_without_a_device_or_a_limiter() {
    let core = DspiCore::new();
    assert!(core.outputs_without_limiter(-60.0).is_empty());
    core.open_demo_wire(17).unwrap();
    assert!(core.outputs_without_limiter(-60.0).is_empty());
}

/// A linked pair's preamp: both inputs in one call, one undo step with the
/// label, one redo; whichever side moved.
#[test]
fn wp_p1pr_a_linked_pair_preamp_change_is_one_undo_step() {
    let core = demo();
    let start = core.get_state().input_preamp_db;
    let n = core.history(1000).len();

    core.set_input_pair_preamp(1, -4.5).unwrap();
    let pre = core.get_state().input_preamp_db;
    assert_eq!((pre[0], pre[1]), (-4.5, -4.5));
    assert_eq!(
        core.get_state().preamp_db,
        -4.5,
        "input 0 is the legacy preamp"
    );
    let h = core.history(1000);
    assert_eq!(h.len(), n + 1, "{h:?}");
    assert_eq!(
        h[0].label.as_deref(),
        Some(crate::params::PAIR_PREAMP_LABEL)
    );
    assert!(h[0].undoable);

    core.undo().unwrap();
    assert_eq!(
        core.get_state().input_preamp_db,
        start,
        "one undo, both back"
    );
    assert!(!core.can_undo());
    core.redo().unwrap();
    let pre = core.get_state().input_preamp_db;
    assert_eq!((pre[0], pre[1]), (-4.5, -4.5), "one redo, both again");

    // the other side, a second step
    core.set_input_pair_preamp(0, 2.04).unwrap();
    let pre = core.get_state().input_preamp_db;
    assert_eq!(
        (pre[0], pre[1]),
        (2.0, 2.0),
        "quantized as set_input_preamp"
    );
    core.undo().unwrap();
    let pre = core.get_state().input_preamp_db;
    assert_eq!((pre[0], pre[1]), (-4.5, -4.5), "one undo, the second step");
}

#[test]
fn wp_p1pr_a_pair_preamp_checks_both_inputs_first() {
    let core = demo();
    let inputs = core.get_state().num_input_channels;
    let start = core.get_state().input_preamp_db;
    assert!(matches!(
        core.set_input_pair_preamp(0, f32::NAN),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.set_input_pair_preamp(inputs, 0.0),
        Err(DspiError::InvalidArgument { .. })
    ));
    if inputs % 2 == 1 {
        // the last input of an odd count has no partner
        assert!(matches!(
            core.set_input_pair_preamp(inputs - 1, 0.0),
            Err(DspiError::InvalidArgument { .. })
        ));
    }
    assert_eq!(core.get_state().input_preamp_db, start, "nothing written");
    assert!(core.history(10).is_empty());
    assert_eq!(
        DspiCore::new().set_input_pair_preamp(0, 0.0),
        Err(DspiError::Disconnected),
        "no device"
    );
}
