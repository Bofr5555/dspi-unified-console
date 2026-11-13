//! WP P1UG: undo groups for compound edits. Everything written between
//! `begin_group` and `end_group` is one undo step with the label, as a drag
//! is. All on the demo device.

use std::sync::Arc;

use super::*;
use crate::consts::NOTIFY_HISTORY;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

/// The demo with its remote, notifications read once.
fn remote_demo() -> (Arc<DspiCore>, dspi_transport::FakeRemote) {
    let dev = dspi_transport::FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.process_notifications();
    (core, remote)
}

fn band0(core: &DspiCore, b: usize) -> FilterParams {
    core.get_state().filters[0][b]
}

fn peaking(freq: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type: 1,
        bypass: false,
        freq,
        q: 1.4,
        gain,
        qp: 0.707,
    }
}

/// A band overwritten over MIDI (another writer), waited for.
fn overwrite_over_midi(core: &DspiCore, remote: &dspi_transport::FakeRemote, b: u8, hz: f32) {
    use dspi_proto::generated as g;
    let mut band = [0u8; 16];
    band[0] = 1;
    band[4..8].copy_from_slice(&hz.to_le_bytes());
    band[8..12].copy_from_slice(&0.7f32.to_le_bytes());
    band[12..].copy_from_slice(&3.0f32.to_le_bytes());
    // Channel 0: 16 bytes a band.
    let off = g::OFF_EQ + usize::from(b) * 16;
    assert!(remote.poke(off, &band, g::notify::PARAM_SRC_MIDI as u8));
    let t = dspi_transport::clock::Instant::now();
    while band0(core, usize::from(b)).freq != hz {
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "no notification"
        );
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A linked pair: both inputs' trims in one group are one undo step with
/// the label, and one redo; every value goes back and forth.
#[test]
fn wp_p1ug_a_linked_pair_is_one_step() {
    let core = demo();
    let start = core.get_state().input_preamp_db.clone();
    core.begin_group("Input trim, linked pair".into());
    core.set_input_preamp(0, -4.5).unwrap();
    core.set_input_preamp(1, -4.5).unwrap();
    core.end_group();
    let h = core.history(10);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].label.as_deref(), Some("Input trim, linked pair"));

    let m = core.undo().unwrap();
    assert_eq!(
        m.entry.unwrap().label.as_deref(),
        Some("Input trim, linked pair")
    );
    assert_eq!(core.get_state().input_preamp_db, start);
    assert!(!core.can_undo(), "one undo took both back");

    core.redo().unwrap();
    let pre = core.get_state().input_preamp_db;
    assert_eq!((pre[0], pre[1]), (-4.5, -4.5));
    assert!(!core.can_redo(), "one redo put both again");
}

/// A 10-band Clear All is one step: one undo puts every band back as it
/// was, one redo clears them all again.
#[test]
fn wp_p1ug_a_ten_band_clear_all_is_one_step() {
    let core = demo();
    for b in 0..10u8 {
        core.set_filter(0, b, peaking(100.0 + f32::from(b) * 200.0, 3.0))
            .unwrap();
    }
    let set: Vec<FilterParams> = (0..10).map(|b| band0(&core, b)).collect();
    let flat = FilterParams {
        filter_type: 0,
        bypass: false,
        freq: 1000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    };
    let n = core.history(100).len();
    core.begin_group("Clear All".into());
    for b in 0..10u8 {
        core.set_filter(0, b, flat).unwrap();
    }
    core.end_group();
    assert_eq!(core.history(100).len(), n + 1, "one entry");
    let cleared: Vec<FilterParams> = (0..10).map(|b| band0(&core, b)).collect();
    assert!(cleared.iter().all(|p| p.filter_type == 0));

    core.undo().unwrap();
    let back: Vec<FilterParams> = (0..10).map(|b| band0(&core, b)).collect();
    assert_eq!(back, set, "every band back");
    core.redo().unwrap();
    let again: Vec<FilterParams> = (0..10).map(|b| band0(&core, b)).collect();
    assert_eq!(again, cleared, "every band cleared again");
    core.undo().unwrap();
    assert_eq!(core.history(1)[0].source, HistorySource::Undo);
    // The ten set-ups before the group are still ten steps.
    assert!(core.can_undo());
}

/// Direct 1:1 routing: every crosspoint set in one group is one step; undo
/// puts the matrix back as it was, redo routes it again.
#[test]
fn wp_p1ug_direct_routing_is_one_step() {
    let core = demo();
    let st = core.get_state();
    let (ins, outs) = (st.num_input_channels, st.num_output_channels);
    // A crossed matrix to start from: input i to output (i + 1) mod outs.
    for i in 0..ins {
        for o in 0..outs {
            core.set_matrix_route(i, o, (i + 1) % outs == o, -2.0, false)
                .unwrap();
        }
    }
    let before = core.get_state();
    let n = core.history(1000).len();
    core.begin_group("Direct routing".into());
    for i in 0..ins {
        for o in 0..outs {
            core.set_matrix_route(i, o, i == o, 0.0, false).unwrap();
        }
    }
    core.end_group();
    assert_eq!(core.history(1000).len(), n + 1);
    let direct = core.get_state();
    assert!(direct.matrix_routing[0][0] && !direct.matrix_routing[0][1]);

    core.undo().unwrap();
    let s = core.get_state();
    assert_eq!(s.matrix_routing, before.matrix_routing);
    assert_eq!(s.matrix_gain, before.matrix_gain);
    core.redo().unwrap();
    let s = core.get_state();
    assert_eq!(s.matrix_routing, direct.matrix_routing);
    assert_eq!(s.matrix_gain, direct.matrix_gain);
}

/// Groups nest: only the outermost begin and end count, with the
/// outermost label; a stray end does nothing.
#[test]
fn wp_p1ug_nesting_counts() {
    let core = demo();
    let start = core.get_state().output_gain_db.clone();
    core.begin_group("Outer".into());
    core.set_output_gain(0, -1.0).unwrap();
    core.begin_group("Inner".into());
    core.set_output_gain(1, -2.0).unwrap();
    core.end_group();
    // Still inside the outer group.
    core.set_output_gain(2, -3.0).unwrap();
    // A drag inside a group is part of its step.
    core.set_dragging(true);
    core.set_output_gain(3, -4.0).unwrap();
    core.set_dragging(false);
    core.end_group();
    core.end_group(); // stray: nothing
    let h = core.history(10);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].label.as_deref(), Some("Outer"));
    core.set_output_gain(0, -9.0).unwrap();
    assert_eq!(
        core.history(10).len(),
        2,
        "after the group, a step of its own"
    );
    assert_eq!(core.history(1)[0].label, None);
    core.undo().unwrap();
    core.undo().unwrap();
    assert_eq!(core.get_state().output_gain_db, start);
    assert!(!core.can_undo());
}

/// An empty group records nothing, and leaves the step before it alone.
#[test]
fn wp_p1ug_an_empty_group_records_nothing() {
    let core = demo();
    core.set_output_gain(0, -1.0).unwrap();
    let n = core.history(10).len();
    core.process_notifications();
    core.begin_group("Nothing".into());
    core.end_group();
    assert_eq!(core.history(10).len(), n);
    assert_eq!(core.process_notifications().flags & NOTIFY_HISTORY, 0);
    // A group whose writes moved nothing records nothing either.
    core.begin_group("Same".into());
    core.set_output_gain(0, -1.0).unwrap();
    core.end_group();
    assert_eq!(core.history(10).len(), n);
}

/// An unclosed group ends with its connection: after a disconnect and the
/// device again, writes are steps of their own; the old end is harmless.
#[test]
fn wp_p1ug_an_unclosed_group_is_ended_by_a_disconnect() {
    let core = demo();
    core.begin_group("Left open".into());
    core.set_output_gain(0, -1.0).unwrap();
    core.disconnect();
    core.open_demo().unwrap();
    core.set_output_gain(0, -2.0).unwrap();
    core.set_output_gain(1, -2.0).unwrap();
    let h = core.history(10);
    assert_eq!(h.len(), 2, "{h:?}");
    assert!(h.iter().all(|e| e.label.is_none()));
    core.end_group(); // the old group's end: nothing
    core.set_output_gain(2, -2.0).unwrap();
    assert_eq!(core.history(10).len(), 3);

    // Another device opened over an open group, with no disconnect.
    core.begin_group("Left open".into());
    core.open_demo().unwrap();
    core.set_output_gain(0, -1.0).unwrap();
    core.set_output_gain(1, -1.0).unwrap();
    assert_eq!(core.history(10).len(), 2);

    // With no device the calls are ignored.
    let none = DspiCore::new();
    none.begin_group("x".into());
    none.end_group();
    none.open_demo().unwrap();
    none.set_output_gain(0, -1.0).unwrap();
    none.set_output_gain(1, -1.0).unwrap();
    assert_eq!(none.history(10).len(), 2);
}

/// A re-read (`fetch_all`) and a bulk image end a group at any depth.
#[test]
fn wp_p1ug_a_reread_and_a_bulk_image_end_the_group() {
    let core = demo();
    core.begin_group("A".into());
    core.begin_group("B".into());
    core.set_output_gain(0, -1.0).unwrap();
    core.fetch_all().unwrap();
    core.set_output_gain(1, -1.0).unwrap();
    core.set_output_gain(2, -1.0).unwrap();
    let h = core.history(10);
    assert_eq!(h.len(), 3, "{h:?}");
    assert_eq!(h[0].label, None);
    assert_eq!(h[2].label.as_deref(), Some("A"));
    core.end_group();
    core.end_group();

    let core = demo();
    core.begin_group("Import".into());
    core.set_output_gain(0, -1.0).unwrap();
    {
        let mut inner = core.lock();
        journal(&mut inner, "bulk", &[], Value::Trigger);
    }
    core.set_output_gain(1, -1.0).unwrap();
    core.set_output_gain(2, -1.0).unwrap();
    let h = core.history(10);
    assert_eq!(h.len(), 4, "{h:?}");
    assert!(h[..2].iter().all(|e| e.label.is_none()));
}

/// Another writer changes one of a group's values afterwards: undo leaves
/// it as they set it and names them, and puts the rest back.
#[test]
fn wp_p1ug_another_writer_is_left_alone_and_named() {
    let (core, remote) = remote_demo();
    let s2 = band0(&core, 2);
    core.begin_group("Two bands".into());
    core.set_filter(0, 1, peaking(500.0, 2.0)).unwrap();
    core.set_filter(0, 2, peaking(900.0, -2.0)).unwrap();
    core.end_group();
    overwrite_over_midi(&core, &remote, 1, 4321.0);

    let m = core.undo().unwrap();
    assert!(m.entry.is_some(), "{m:?}");
    assert!(m.note.as_deref().unwrap_or("").contains("MIDI"), "{m:?}");
    assert_eq!(m.skipped.len(), 1);
    assert_eq!(m.skipped[0].indices, vec![0, 1]);
    assert_eq!(m.skipped[0].label.as_deref(), Some("Two bands"));
    assert_eq!(band0(&core, 1).freq, 4321.0, "theirs stays");
    assert_eq!(band0(&core, 2), s2);
}

/// Another writer's change during a group does not split it: the group is
/// still one step, judged value by value.
#[test]
fn wp_p1ug_another_writer_inside_a_group_is_judged_per_value() {
    let (core, remote) = remote_demo();
    let (s2, s3) = (band0(&core, 2), band0(&core, 3));
    core.begin_group("Three bands".into());
    core.set_filter(0, 1, peaking(500.0, 2.0)).unwrap();
    core.set_filter(0, 2, peaking(900.0, -2.0)).unwrap();
    overwrite_over_midi(&core, &remote, 1, 4321.0);
    core.set_filter(0, 3, peaking(1500.0, 1.0)).unwrap();
    core.end_group();
    assert_eq!(core.history(10).len(), 1, "one step");

    let m = core.undo().unwrap();
    assert!(m.note.as_deref().unwrap_or("").contains("MIDI"), "{m:?}");
    assert_eq!(band0(&core, 1).freq, 4321.0, "theirs stays");
    assert_eq!((band0(&core, 2), band0(&core, 3)), (s2, s3));
    assert!(!core.can_undo());
}

/// A value the group writes again after another writer changed it starts
/// a new step, so undo goes back to their value and never past it.
#[test]
fn wp_p1ug_a_rewrite_after_another_writer_never_undoes_past_them() {
    let (core, remote) = remote_demo();
    core.begin_group("Again".into());
    core.set_filter(0, 1, peaking(500.0, 2.0)).unwrap();
    overwrite_over_midi(&core, &remote, 1, 4321.0);
    core.set_filter(0, 1, peaking(600.0, 2.0)).unwrap();
    core.set_filter(0, 2, peaking(700.0, 2.0)).unwrap();
    core.end_group();
    let h = core.history(10);
    assert_eq!(h.len(), 2, "{h:?}");
    assert!(h.iter().all(|e| e.label.as_deref() == Some("Again")));
    core.undo().unwrap();
    assert_eq!(band0(&core, 1).freq, 4321.0, "back to theirs");
}

// -- WP P1AFIX -- (review p1a F5 and the unproved hazard claim)

/// A group begun inside a drag joins the drag's step, and its end leaves
/// the step open until the drag ends: one drag is one step.
#[test]
fn wp_p1afix_a_group_inside_a_drag_joins_the_drags_step() {
    let core = demo();
    let start = core.get_state().output_gain_db.clone();
    core.set_dragging(true);
    core.set_output_gain(0, -1.0).unwrap();
    core.begin_group("Linked pair".into());
    core.set_output_gain(1, -2.0).unwrap();
    core.set_output_gain(2, -2.0).unwrap();
    core.end_group();
    core.set_output_gain(0, -3.0).unwrap();
    core.set_output_gain(3, -4.0).unwrap();
    core.set_dragging(false);
    let h = core.history(10);
    assert_eq!(h.len(), 1, "one drag, one step: {h:?}");
    // The drag's step had no label: it takes the group's.
    assert_eq!(h[0].label.as_deref(), Some("Linked pair"));
    // After the drag, a write is a step of its own.
    core.set_output_gain(0, -9.0).unwrap();
    assert_eq!(core.history(10).len(), 2);
    core.undo().unwrap();
    core.undo().unwrap();
    assert_eq!(core.get_state().output_gain_db, start);
    assert!(!core.can_undo());

    // A drag that ends while the group is still open: the group's step goes
    // on until its end.
    let core = demo();
    core.set_dragging(true);
    core.set_output_gain(0, -1.0).unwrap();
    core.begin_group("Pair".into());
    core.set_output_gain(1, -2.0).unwrap();
    core.set_dragging(false);
    core.set_output_gain(2, -2.0).unwrap();
    core.end_group();
    core.set_output_gain(3, -5.0).unwrap();
    let h = core.history(10);
    assert_eq!(h.len(), 2, "{h:?}");
    assert_eq!(h[1].label.as_deref(), Some("Pair"));
    assert_eq!(h[0].label, None);
}

/// A write undo cannot reverse, inside a group, stands apart: it is an
/// entry of its own, never undone, and the group's undoable values around
/// it each go back.
#[test]
fn wp_p1afix_a_hazard_write_inside_a_group_stands_apart() {
    let core = demo();
    let start = core.get_state().input_preamp_db.clone();
    core.begin_group("With a hazard".into());
    core.set_input_preamp(0, -3.0).unwrap();
    core.set_output_latency(1).unwrap();
    core.set_input_preamp(1, -3.0).unwrap();
    core.end_group();
    let h = core.history(10);
    let hazard: Vec<_> = h.iter().filter(|e| e.path == "out.latency").collect();
    assert_eq!(hazard.len(), 1, "{h:?}");
    assert!(!hazard[0].undoable, "{h:?}");
    assert!(
        h.iter()
            .filter(|e| e.path != "out.latency")
            .all(|e| e.undoable),
        "{h:?}"
    );
    // The hazard is not joined to a value step: the values sit in steps
    // of their own around it.
    assert_eq!(h.len(), 3, "{h:?}");
    assert_eq!(h[1].path, "out.latency", "{h:?}");
    // Undo puts the values back and never sends the hazard again.
    for _ in 0..5 {
        if !core.can_undo() {
            break;
        }
        core.undo().unwrap();
    }
    assert_eq!(core.get_state().input_preamp_db, start);
}
