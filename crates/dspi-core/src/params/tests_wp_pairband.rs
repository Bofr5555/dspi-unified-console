//! WP PAIRBAND: a linked input pair's band edit is one undo step
//! (`set_input_pair_filter`, `set_input_pair_band_bypass`): undo and redo
//! move both sides together, a graph drag of the pair is one step, and a
//! value another writer changes on one side is judged on its own.

use std::sync::Arc;
use std::time::Duration;

use dspi_proto::generated as g;
use dspi_transport::{FakeDevice, FakeRemote};

use super::PAIR_BAND_LABEL;
use crate::consts::FIRMWARE_BANDS_PER_CHANNEL;
use crate::records::FilterParams;
use crate::{DspiCore, DspiError};

fn demo() -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.process_notifications();
    (core, remote)
}

fn band(core: &DspiCore, ch: usize, b: usize) -> FilterParams {
    core.get_state().filters[ch][b]
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

/// Both sides of pair 0, band `b`.
fn pair(core: &DspiCore, b: usize) -> (FilterParams, FilterParams) {
    (band(core, 0, b), band(core, 1, b))
}

/// Band `b` of wire channel `ch` overwritten over MIDI (another writer),
/// waited for.
fn overwrite_over_midi(core: &DspiCore, remote: &FakeRemote, ch: usize, b: usize, hz: f32) {
    let mut bytes = [0u8; 16];
    bytes[0] = 1;
    bytes[4..8].copy_from_slice(&hz.to_le_bytes());
    bytes[8..12].copy_from_slice(&0.7f32.to_le_bytes());
    bytes[12..].copy_from_slice(&3.0f32.to_le_bytes());
    let off = g::OFF_EQ + (ch * FIRMWARE_BANDS_PER_CHANNEL + b) * 16;
    assert!(remote.poke(off, &bytes, g::notify::PARAM_SRC_MIDI as u8));
    let t = dspi_transport::clock::Instant::now();
    while band(core, ch, b).freq != hz {
        assert!(t.elapsed() < Duration::from_secs(5), "no notification");
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The band list: one edit of a linked pair's band is one labelled step;
/// one undo puts both sides back, one redo writes both again.
#[test]
fn wp_pairband_a_linked_pair_band_edit_is_one_step() {
    let (core, _remote) = demo();
    let before = pair(&core, 0);
    core.set_input_pair_filter(1, 0, peaking(1200.0, 4.0))
        .unwrap();
    let after = pair(&core, 0);
    assert_eq!(after.0.filter_type, 1);
    assert_eq!(after.0, after.1, "both sides written");
    let h = core.history(10);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].label.as_deref(), Some(PAIR_BAND_LABEL));

    let m = core.undo().unwrap();
    assert_eq!(m.entry.unwrap().label.as_deref(), Some(PAIR_BAND_LABEL));
    assert_eq!(pair(&core, 0), before, "one undo, both sides back");
    assert!(!core.can_undo());

    core.redo().unwrap();
    assert_eq!(pair(&core, 0), after, "one redo, both sides again");
    assert!(!core.can_redo());

    // A second edit is a second step.
    core.set_input_pair_filter(0, 0, peaking(1500.0, -2.0))
        .unwrap();
    core.undo().unwrap();
    assert_eq!(pair(&core, 0), after);
    assert!(core.can_undo(), "the first edit is still there");
}

/// The bypass of a linked pair's band: one step, both sides.
#[test]
fn wp_pairband_a_linked_pair_bypass_is_one_step() {
    let (core, _remote) = demo();
    core.set_input_pair_filter(0, 2, peaking(800.0, 3.0))
        .unwrap();
    core.set_input_pair_band_bypass(0, 2, true).unwrap();
    assert!(band(&core, 0, 2).bypass && band(&core, 1, 2).bypass);
    assert_eq!(core.history(10).len(), 2);
    core.undo().unwrap();
    assert!(!band(&core, 0, 2).bypass && !band(&core, 1, 2).bypass);
    core.redo().unwrap();
    assert!(band(&core, 0, 2).bypass && band(&core, 1, 2).bypass);
}

/// A graph drag of a linked pair: every tick goes through the pair setter;
/// the drag is one step whatever the ticks move (gain only, then frequency
/// and gain, then the type: the drag-merge rule, P0W2), and one undo puts
/// both sides back as they were when the drag began.
#[test]
fn wp_pairband_a_drag_of_a_linked_pair_is_one_step() {
    let (core, _remote) = demo();
    core.set_input_pair_filter(0, 1, peaking(1000.0, 0.0))
        .unwrap();
    let start = pair(&core, 1);
    core.set_dragging(true);
    for i in 0..5u8 {
        core.set_input_pair_filter(0, 1, peaking(1000.0, f32::from(i)))
            .unwrap();
    }
    for i in 0..5u8 {
        core.set_input_pair_filter(1, 1, peaking(1000.0 + 100.0 * f32::from(i), 5.0))
            .unwrap();
    }
    // A second band in the same drag (a pinch over two handles).
    core.set_input_pair_filter(0, 3, peaking(4000.0, -3.0))
        .unwrap();
    core.set_dragging(false);
    let end = pair(&core, 1);
    assert_eq!(end.0, end.1);
    assert_eq!(end.0.freq, 1400.0);
    let h = core.history(10);
    assert_eq!(h.len(), 2, "the edit before, then one drag: {h:?}");
    assert_eq!(h[0].label.as_deref(), Some(PAIR_BAND_LABEL));

    let m = core.undo().unwrap();
    assert!(m.skipped.is_empty(), "{m:?}");
    assert_eq!(pair(&core, 1), start, "both sides back to the drag's start");
    assert_eq!(band(&core, 0, 3).filter_type, 0);
    assert_eq!(band(&core, 1, 3).filter_type, 0);
    assert!(core.can_undo(), "only the drag was undone");
    core.redo().unwrap();
    assert_eq!(pair(&core, 1), end, "one redo, the whole drag again");
    assert_eq!(band(&core, 1, 3).freq, 4000.0);

    // After the drag, a write is a step of its own.
    core.set_input_pair_filter(0, 1, peaking(300.0, 1.0))
        .unwrap();
    core.undo().unwrap();
    assert_eq!(pair(&core, 1), end);
}

/// Another writer changes one side afterwards: undo leaves that side as they
/// set it and names them, and puts the other side back.
#[test]
fn wp_pairband_another_writer_on_one_side_is_judged_per_value() {
    let (core, remote) = demo();
    let before = pair(&core, 4);
    core.set_input_pair_filter(0, 4, peaking(600.0, 2.0))
        .unwrap();
    overwrite_over_midi(&core, &remote, 1, 4, 4321.0);

    let m = core.undo().unwrap();
    assert!(m.entry.is_some(), "{m:?}");
    assert!(m.note.as_deref().unwrap_or("").contains("MIDI"), "{m:?}");
    assert_eq!(m.skipped.len(), 1, "{m:?}");
    assert_eq!(m.skipped[0].indices, vec![1, 4]);
    assert_eq!(band(&core, 1, 4).freq, 4321.0, "theirs stays");
    assert_eq!(band(&core, 0, 4), before.0, "ours goes back");
}

/// The same within a drag: one side overwritten after the drag is left, the
/// other side's drag is undone.
#[test]
fn wp_pairband_another_writer_after_a_drag() {
    let (core, remote) = demo();
    let before = pair(&core, 5);
    core.set_dragging(true);
    for i in 1..4u8 {
        core.set_input_pair_filter(0, 5, peaking(500.0 * f32::from(i), 2.0))
            .unwrap();
    }
    core.set_dragging(false);
    overwrite_over_midi(&core, &remote, 0, 5, 777.0);
    let m = core.undo().unwrap();
    assert!(m.note.as_deref().unwrap_or("").contains("MIDI"), "{m:?}");
    assert_eq!(band(&core, 0, 5).freq, 777.0, "theirs stays");
    assert_eq!(band(&core, 1, 5), before.1, "the partner goes back");
}

/// Everything is checked before anything is sent.
#[test]
fn wp_pairband_checks_come_first() {
    let (core, _remote) = demo();
    let n = core.get_state().num_input_channels;
    let bad = |r: crate::Result<()>| matches!(r, Err(DspiError::InvalidArgument { .. }));
    assert!(bad(core.set_input_pair_filter(n, 0, peaking(1000.0, 1.0))));
    assert!(bad(core.set_input_pair_filter(0, 10, peaking(1000.0, 1.0))));
    assert!(bad(core.set_input_pair_filter(
        0,
        0,
        peaking(f32::NAN, 1.0)
    )));
    assert!(bad(core.set_input_pair_band_bypass(0, 10, true)));
    if n % 2 == 1 {
        // The last input of an odd count has no partner.
        assert!(bad(core.set_input_pair_filter(
            n - 1,
            0,
            peaking(1000.0, 1.0)
        )));
    }
    assert!(core.history(10).is_empty(), "nothing written");
    assert!(matches!(
        DspiCore::new().set_input_pair_filter(0, 0, peaking(1000.0, 1.0)),
        Err(DspiError::Disconnected)
    ));
}
