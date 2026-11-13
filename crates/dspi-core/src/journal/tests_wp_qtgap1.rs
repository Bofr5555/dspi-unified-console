//! WP QTGAP1: only a continuous drag, marked by the client with
//! `set_dragging`, joins one undo step. A typed value or a discrete write is
//! a step of its own, however soon it follows. All on the demo device.

use std::sync::Arc;

use super::*;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn gain0(core: &DspiCore) -> f32 {
    core.get_state().output_gain_db[0]
}

/// Two typed values in quick succession (well within the old 1 s window)
/// are two steps: undo goes back one at a time.
#[test]
fn wp_qtgap1_typed_values_never_join() {
    let core = demo();
    let start = gain0(&core);
    core.set_output_gain(0, -3.0).unwrap();
    core.set_output_gain(0, -6.0).unwrap();
    assert_eq!(core.history(10).len(), 2);
    core.undo().unwrap();
    assert_eq!(gain0(&core), -3.0);
    core.undo().unwrap();
    assert_eq!(gain0(&core), start);
    assert!(!core.can_undo());
}

/// The writes between `set_dragging(true)` and `(false)` are one step,
/// even when the drag is held still for longer than a second.
#[test]
fn wp_qtgap1_a_drag_is_one_step_however_long() {
    let core = demo();
    let start = gain0(&core);
    core.set_dragging(true);
    core.set_output_gain(0, -1.0).unwrap();
    core.set_output_gain(0, -2.0).unwrap();
    dspi_transport::clock::sleep(std::time::Duration::from_millis(1100));
    core.set_output_gain(0, -4.0).unwrap();
    core.set_dragging(false);
    assert_eq!(core.history(10).len(), 1);
    core.undo().unwrap();
    assert_eq!(gain0(&core), start);
    assert!(!core.can_undo());
}

/// Two drags of the same control, one straight after the other, are two
/// steps: each call ends the step being gathered.
#[test]
fn wp_qtgap1_two_drags_are_two_steps() {
    let core = demo();
    for drag in [[-1.0, -2.0], [-3.0, -4.0]] {
        core.set_dragging(true);
        for db in drag {
            core.set_output_gain(0, db).unwrap();
        }
        core.set_dragging(false);
    }
    assert_eq!(core.history(10).len(), 2);
    core.undo().unwrap();
    assert_eq!(gain0(&core), -2.0);
}

/// A typed value straight after a drag, and a drag straight after a typed
/// value, stay apart from it.
#[test]
fn wp_qtgap1_a_typed_value_next_to_a_drag_stays_apart() {
    let core = demo();
    let start = gain0(&core);
    core.set_output_gain(0, -1.0).unwrap();
    core.set_dragging(true);
    core.set_output_gain(0, -2.0).unwrap();
    core.set_output_gain(0, -3.0).unwrap();
    core.set_dragging(false);
    core.set_output_gain(0, -5.0).unwrap();
    assert_eq!(core.history(10).len(), 3);
    core.undo().unwrap();
    assert_eq!(gain0(&core), -3.0, "the typed value alone");
    core.undo().unwrap();
    assert_eq!(gain0(&core), -1.0, "the drag alone");
    core.undo().unwrap();
    assert_eq!(gain0(&core), start);
}

/// Within a drag, another control's write joins the same step (WP P0W2,
/// follow-up O1, coordinator's decision: everything one drag writes is one
/// step); undo puts both back.
#[test]
fn wp_qtgap1_a_drag_joins_every_value_it_moves() {
    let core = demo();
    let start1 = core.get_state().output_gain_db[1];
    let start = gain0(&core);
    core.set_dragging(true);
    core.set_output_gain(0, -1.0).unwrap();
    core.set_output_gain(0, -2.0).unwrap();
    core.set_output_gain(1, -1.0).unwrap();
    core.set_output_gain(0, -3.0).unwrap();
    core.set_dragging(false);
    assert_eq!(core.history(10).len(), 1);
    core.undo().unwrap();
    assert_eq!(gain0(&core), start);
    assert_eq!(core.get_state().output_gain_db[1], start1);
}

/// An undo in the middle of a drag is not joined, and the drag's next
/// write starts a new step (and ends what can be redone).
#[test]
fn wp_qtgap1_an_undo_during_a_drag_is_never_joined() {
    let core = demo();
    core.set_dragging(true);
    core.set_output_gain(0, -1.0).unwrap();
    core.set_output_gain(0, -2.0).unwrap();
    core.undo().unwrap();
    assert!(core.can_redo());
    core.set_output_gain(0, -4.0).unwrap();
    core.set_dragging(false);
    assert!(!core.can_redo());
    let h = core.history(10);
    assert_eq!(h.len(), 3, "{h:?}");
    core.undo().unwrap();
    assert!(
        !core.can_undo(),
        "the earlier step was undone, and its redo dropped"
    );
}

/// The mark is harmless with no device, and joins nothing from before a
/// reconnect (WP P0W2, fixup p0w2b: it ends with its connection).
#[test]
fn wp_qtgap1_the_mark_with_no_device() {
    let core = DspiCore::new();
    core.set_dragging(true);
    core.set_dragging(false);
    core.open_demo().unwrap();
    core.set_output_gain(0, -1.0).unwrap();
    core.set_output_gain(0, -2.0).unwrap();
    assert_eq!(core.history(10).len(), 2);
}
