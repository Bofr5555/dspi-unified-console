//! WP P0W2: one drag of one band is one undo step, whatever fields it
//! moves. A graph drag sends the band whole (`set_filter`); the journal
//! names a write by the one field it moved, or `eq.band` when it moved more,
//! so a drag mixing one-axis and two-axis moves used to split into several
//! steps. All on the demo device.

use std::sync::Arc;

use super::*;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn band0(core: &DspiCore, b: usize) -> FilterParams {
    core.get_state().filters[0][b]
}

/// A graph drag: horizontal, diagonal, vertical, horizontal again.
fn mixed_drag(core: &DspiCore, b: u8) {
    let mut p = band0(core, usize::from(b));
    core.set_dragging(true);
    for (df, dg) in [(10.0, 0.0), (20.0, 1.0), (0.0, 1.5), (5.0, 0.0)] {
        p.freq += df;
        p.gain += dg;
        core.set_filter(0, b, p).unwrap();
    }
    core.set_dragging(false);
}

#[test]
fn wp_p0w2_a_graph_drag_mixing_axes_is_one_step() {
    let core = demo();
    let start = band0(&core, 1);
    mixed_drag(&core, 1);
    let end = band0(&core, 1);
    assert_ne!(start, end);
    let h = core.history(10);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].path, "eq.band");
    assert_eq!(
        h[0].before,
        Some(ParamValue::Bytes(band_bytes(&start))),
        "the band before the drag"
    );
    assert_eq!(h[0].after, ParamValue::Bytes(band_bytes(&end)));

    let m = core.undo().unwrap();
    assert!(m.entry.is_some() && m.note.is_none(), "{m:?}");
    assert_eq!(band0(&core, 1), start, "the whole band as it was");
    assert!(!core.can_undo());
    core.redo().unwrap();
    assert_eq!(band0(&core, 1), end);
}

/// A drag that starts on two axes and goes on along one joins as well.
#[test]
fn wp_p0w2_a_two_axis_start_then_one_axis_is_one_step() {
    let core = demo();
    let start = band0(&core, 2);
    let mut p = start;
    core.set_dragging(true);
    for (df, dq) in [(10.0, 0.1), (10.0, 0.0), (0.0, 0.2)] {
        p.freq += df;
        p.q += dq;
        core.set_filter(0, 2, p).unwrap();
    }
    core.set_dragging(false);
    assert_eq!(core.history(10).len(), 1);
    core.undo().unwrap();
    assert_eq!(band0(&core, 2), start);
}

/// Field-by-field writes of one band during one drag (a pinch, a client
/// sending `eq.freq` and `eq.gain` through `write_param`) are one step too.
#[test]
fn wp_p0w2_field_writes_of_one_band_in_one_drag_are_one_step() {
    let core = demo();
    let start = band0(&core, 3);
    core.set_dragging(true);
    for (path, v) in [
        ("eq.freq", start.freq + 100.0),
        ("eq.gain", start.gain - 2.0),
        ("eq.q", start.q + 0.3),
        ("eq.freq", start.freq + 150.0),
    ] {
        core.write_param(path.into(), vec![0, 3], ParamValue::Float(v))
            .unwrap();
    }
    core.set_dragging(false);
    let h = core.history(10);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].path, "eq.band");
    core.undo().unwrap();
    assert_eq!(band0(&core, 3), start);
}

/// A drag along one axis keeps that field's name in the history.
#[test]
fn wp_p0w2_a_one_axis_drag_keeps_the_fields_name() {
    let core = demo();
    let start = band0(&core, 1);
    let mut p = start;
    core.set_dragging(true);
    for _ in 0..3 {
        p.freq += 10.0;
        core.set_filter(0, 1, p).unwrap();
    }
    core.set_dragging(false);
    let h = core.history(10);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].path, "eq.freq");
    assert_eq!(h[0].before, Some(ParamValue::Float(start.freq)));
}

/// Typed values never join, whatever band field they set.
#[test]
fn wp_p0w2_typed_band_values_never_join() {
    let core = demo();
    let start = band0(&core, 1);
    let mut p = start;
    p.freq += 100.0;
    core.set_filter(0, 1, p).unwrap();
    p.gain += 1.0;
    core.set_filter(0, 1, p).unwrap();
    p.freq += 1.0;
    p.q += 0.1;
    core.set_filter(0, 1, p).unwrap();
    assert_eq!(core.history(10).len(), 3);
}

/// Two drags of one band are two steps.
#[test]
fn wp_p0w2_two_drags_stay_apart() {
    let core = demo();
    let start = band0(&core, 1);
    mixed_drag(&core, 1);
    let after_first = band0(&core, 1);
    mixed_drag(&core, 1);
    assert_eq!(core.history(10).len(), 2);
    core.undo().unwrap();
    assert_eq!(band0(&core, 1), after_first);
    core.undo().unwrap();
    assert_eq!(band0(&core, 1), start);
}

/// Another writer during the drag ends the step: the session's rule, the
/// widened step included. Afterwards undo leaves their band and names them.
#[test]
fn wp_p0w2_another_writer_is_skipped_and_named_after_a_mixed_drag() {
    use dspi_proto::generated as g;
    let dev = dspi_transport::FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.process_notifications();
    mixed_drag(&core, 1);
    assert_eq!(core.history(10).len(), 1);

    let mut band = [0u8; 16];
    band[0] = 1;
    band[4..8].copy_from_slice(&4321.0f32.to_le_bytes());
    band[8..12].copy_from_slice(&0.7f32.to_le_bytes());
    band[12..].copy_from_slice(&3.0f32.to_le_bytes());
    // Channel 0, band 1: 16 bytes a band.
    let off = g::OFF_EQ + 16;
    let midi = g::notify::PARAM_SRC_MIDI as u8;
    assert!(remote.poke(off, &band, midi));
    let t = dspi_transport::clock::Instant::now();
    while band0(&core, 1).freq != 4321.0 {
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "no notification"
        );
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let m = core.undo().unwrap();
    assert!(m.entry.is_none(), "{m:?}");
    assert_eq!(m.skipped.len(), 1);
    assert!(m.note.unwrap().contains("MIDI"));
    assert_eq!(band0(&core, 1).freq, 4321.0, "theirs stays");
}

// ── Follow-up O1: one drag is one step across bands and controls ─────

/// The web's multi-selection drag: every selected band written whole per
/// tick, two bands over four ticks, mixed axes.
fn multi_band_drag(core: &DspiCore, bands: &[u8]) {
    core.set_dragging(true);
    for (df, dg) in [(10.0, 0.0), (20.0, 1.0), (0.0, 1.5), (5.0, 0.0)] {
        for &b in bands {
            let mut p = band0(core, usize::from(b));
            p.freq += df;
            p.gain += dg;
            core.set_filter(0, b, p).unwrap();
        }
    }
    core.set_dragging(false);
}

#[test]
fn wp_p0w2_o1_a_multi_band_drag_is_one_step() {
    let core = demo();
    let (s1, s2) = (band0(&core, 1), band0(&core, 2));
    multi_band_drag(&core, &[1, 2]);
    let (e1, e2) = (band0(&core, 1), band0(&core, 2));
    assert!(e1 != s1 && e2 != s2);
    let h = core.history(10);
    assert_eq!(h.len(), 1, "{h:?}");

    let m = core.undo().unwrap();
    assert!(m.entry.is_some() && m.note.is_none(), "{m:?}");
    assert_eq!((band0(&core, 1), band0(&core, 2)), (s1, s2));
    assert!(!core.can_undo());
    core.redo().unwrap();
    assert_eq!((band0(&core, 1), band0(&core, 2)), (e1, e2));
    core.undo().unwrap();
    assert_eq!((band0(&core, 1), band0(&core, 2)), (s1, s2));
}

/// Across parameter groups too: a band, an output gain and a delay moved in
/// one drag go back together.
#[test]
fn wp_p0w2_o1_a_drag_across_groups_is_one_step() {
    let core = demo();
    let s = core.get_state();
    let (b1, g0, d0) = (s.filters[0][1], s.output_gain_db[0], s.output_delay_ms[0]);
    core.set_dragging(true);
    for t in 1..=3 {
        let mut p = band0(&core, 1);
        p.freq += 10.0;
        core.set_filter(0, 1, p).unwrap();
        core.set_output_gain(0, -(t as f32)).unwrap();
        core.set_output_delay(0, t as f32 * 0.5).unwrap();
    }
    core.set_dragging(false);
    assert_eq!(core.history(10).len(), 1);
    core.undo().unwrap();
    let s = core.get_state();
    assert_eq!(
        (s.filters[0][1], s.output_gain_db[0], s.output_delay_ms[0]),
        (b1, g0, d0)
    );
}

/// Typed values next to a multi-band drag stay apart from it.
#[test]
fn wp_p0w2_o1_typed_values_beside_a_multi_band_drag_stay_apart() {
    let core = demo();
    let mut p = band0(&core, 1);
    p.freq += 100.0;
    core.set_filter(0, 1, p).unwrap();
    multi_band_drag(&core, &[1, 2]);
    let mut p = band0(&core, 2);
    p.gain -= 1.0;
    core.set_filter(0, 2, p).unwrap();
    assert_eq!(core.history(10).len(), 3);
}

/// Another writer changes one of the dragged bands afterwards: undo leaves
/// that band as they set it and names them, and puts the other back; redo
/// then moves only the band undo put back.
#[test]
fn wp_p0w2_o1_another_writer_is_judged_value_by_value() {
    use dspi_proto::generated as g;
    let dev = dspi_transport::FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.process_notifications();
    let s2 = band0(&core, 2);
    multi_band_drag(&core, &[1, 2]);
    let e2 = band0(&core, 2);

    let mut band = [0u8; 16];
    band[0] = 1;
    band[4..8].copy_from_slice(&4321.0f32.to_le_bytes());
    band[8..12].copy_from_slice(&0.7f32.to_le_bytes());
    band[12..].copy_from_slice(&3.0f32.to_le_bytes());
    // Channel 0, band 1: 16 bytes a band.
    assert!(remote.poke(g::OFF_EQ + 16, &band, g::notify::PARAM_SRC_MIDI as u8));
    let t = dspi_transport::clock::Instant::now();
    while band0(&core, 1).freq != 4321.0 {
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "no notification"
        );
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let m = core.undo().unwrap();
    assert!(m.entry.is_some(), "the other band went back: {m:?}");
    let note = m.note.unwrap();
    assert!(
        note.contains("MIDI") && note.starts_with("Filter band"),
        "{note}"
    );
    assert_eq!(band0(&core, 1).freq, 4321.0, "theirs stays");
    assert_eq!(band0(&core, 2), s2);

    let m = core.redo().unwrap();
    assert!(m.entry.is_some() && m.note.is_none(), "{m:?}");
    assert_eq!(band0(&core, 2), e2);
    assert_eq!(band0(&core, 1).freq, 4321.0, "still theirs");
}
