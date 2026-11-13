//! WP SWAPFIX: what a re-read of the device (`fetch_all`, on a refresh or a
//! reconnect) does to undo and to the unsaved-changes baseline, and the
//! changes list's one line per output delay. All on the demo device.

use std::sync::Arc;

use super::*;
use crate::state::{decode_bulk, default_state, encode_bulk};

fn demo() -> (Arc<DspiCore>, dspi_transport::FakeRemote) {
    let dev = dspi_transport::FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.fetch_all().unwrap();
    (core, remote)
}

/// Change the device's image the way `edit` says, with no notification this
/// client reads: only a re-read shows it.
fn change_unheard(remote: &dspi_transport::FakeRemote, edit: impl FnOnce(&mut DspState)) {
    let mut s = default_state();
    decode_bulk(&remote.image(), &mut s).unwrap();
    edit(&mut s);
    remote.load_image(encode_bulk(&s), 0);
}

fn delay0(core: &DspiCore) -> f32 {
    core.get_state().output_delay_ms[0]
}

// ── F1: a re-read and undo ──────────────────────────────────────────

/// QP22b's recipe (F1): writes and their reverts back to back, re-reads in
/// between, then a new delay. Undo puts back what the re-read found (0), not
/// the value from before it (1.0), and history says so.
#[test]
fn wp_swapfix_f1_undo_after_a_reread_writes_what_the_reread_found() {
    let (core, remote) = demo();
    core.set_output_delay(0, 1.0).unwrap();
    core.fetch_all().unwrap();
    core.set_output_gain(0, -3.0).unwrap();
    core.fetch_all().unwrap();
    core.set_output_gain(0, 0.0).unwrap();
    core.set_output_delay(0, 0.0).unwrap();
    core.fetch_all().unwrap();
    core.set_output_delay(0, 1.5).unwrap();

    let newest = core.history(1).pop().unwrap();
    assert_eq!(newest.path, "out.delay");
    assert_eq!(newest.before, Some(ParamValue::Float(0.0)), "{newest:?}");
    assert_eq!(newest.after, ParamValue::Float(1.5));

    let m = core.undo().unwrap();
    assert_eq!(m.note, None, "{m:?}");
    assert_eq!(delay0(&core), 0.0);
    let mut dev = default_state();
    decode_bulk(&remote.image(), &mut dev).unwrap();
    assert_eq!(dev.output_delay_ms[0], 0.0, "the device too");
}

/// A drag that goes on across a re-read is two steps: the first undo goes
/// back to the value the re-read found, the second to the start.
#[test]
fn wp_swapfix_f1_a_reread_ends_the_step_a_drag_gathers() {
    let (core, _) = demo();
    core.set_output_delay(0, 1.0).unwrap();
    core.set_output_delay(0, 1.25).unwrap();
    assert_eq!(core.history(10).len(), 1, "one step before the re-read");
    core.fetch_all().unwrap();
    core.set_output_delay(0, 1.5).unwrap();
    assert_eq!(core.history(10).len(), 2);
    core.undo().unwrap();
    assert_eq!(delay0(&core), 1.25);
    core.undo().unwrap();
    assert_eq!(delay0(&core), 0.0);
}

/// A value someone changed unheard (no notification reached this client)
/// shows in the re-read: undo leaves it as they set it, drops the entry and
/// names another writer (P0W's rule).
#[test]
fn wp_swapfix_f1_a_value_the_reread_contradicts_is_skipped_and_named() {
    let (core, remote) = demo();
    core.set_output_delay(0, 1.0).unwrap();
    change_unheard(&remote, |s| {
        s.output_delay_ms[0] = 2.0;
        let ch = usize::from(s.num_input_channels);
        s.channel_delays[ch] = 2.0;
    });
    core.fetch_all().unwrap();
    assert_eq!(delay0(&core), 2.0);
    let m = core.undo().unwrap();
    assert!(m.entry.is_none(), "{m:?}");
    assert_eq!(m.skipped.len(), 1);
    let note = m.note.unwrap();
    assert!(note.contains("another writer"), "{note}");
    assert_eq!(delay0(&core), 2.0, "theirs stays");
    assert!(!core.can_undo());
}

/// The same with a change after the re-read: undo of the new change puts
/// back the other writer's value, and the entry from before is skipped.
#[test]
fn wp_swapfix_f1_a_change_after_the_reread_goes_back_to_the_other_writers_value() {
    let (core, remote) = demo();
    core.set_output_delay(0, 1.0).unwrap();
    change_unheard(&remote, |s| {
        s.output_delay_ms[0] = 2.0;
        let ch = usize::from(s.num_input_channels);
        s.channel_delays[ch] = 2.0;
    });
    core.fetch_all().unwrap();
    core.set_output_delay(0, 3.0).unwrap();
    core.undo().unwrap();
    assert_eq!(delay0(&core), 2.0);
    let m = core.undo().unwrap();
    assert!(m.entry.is_none() && m.note.is_some(), "{m:?}");
    assert_eq!(delay0(&core), 2.0);
}

/// Another writer heard through a notification also ends the step: undo
/// of the change after theirs goes back to their value, then names them.
#[test]
fn wp_swapfix_f1_another_writer_ends_the_step() {
    let (core, remote) = demo();
    core.set_master_volume(-10.0).unwrap();
    let src = dspi_proto::generated::notify::PARAM_SRC_UART as u8;
    let off = dspi_proto::generated::OFF_MASTER_VOLUME;
    assert!(remote.poke(off, &(-12.0f32).to_le_bytes(), src));
    let t = dspi_transport::clock::Instant::now();
    while core.get_state().master_volume_db != -12.0 {
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        core.process_notifications();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    core.set_master_volume(-13.0).unwrap();
    core.undo().unwrap();
    assert_eq!(core.get_state().master_volume_db, -12.0);
    let m = core.undo().unwrap();
    assert!(m.note.unwrap().contains("the UART"));
    assert_eq!(core.get_state().master_volume_db, -12.0);
}

// ── F2: a re-read keeps the unsaved-changes baseline ────────────────

/// Owner decision (2026-11-13): a refresh keeps unsaved edits unsaved.
#[test]
fn wp_swapfix_f2_a_refresh_keeps_unsaved_changes() {
    let (core, _) = demo();
    assert!(!core.preset_dirty());
    core.set_output_delay(0, 1.5).unwrap();
    let changes = core.preset_changes();
    assert!(!changes.is_empty());
    core.fetch_all().unwrap();
    assert!(core.preset_dirty());
    assert_eq!(core.preset_changes(), changes);
}

/// A reconnect to the same device on the same preset keeps them too; the
/// edit the device still runs is unsaved.
#[test]
fn wp_swapfix_f2_a_reconnect_keeps_unsaved_changes() {
    let (core, remote) = demo();
    core.set_output_delay(0, 1.5).unwrap();
    let changes = core.preset_changes();
    let image = remote.image();
    core.disconnect();
    assert!(!core.preset_dirty(), "nothing to compare without a device");
    let dev = dspi_transport::FakeDevice::demo();
    dev.remote().load_image(image, 0);
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    core.fetch_all().unwrap();
    assert!(core.preset_dirty());
    assert_eq!(core.preset_changes(), changes);
    // Reverting by hand clears it: the baseline is the preset's.
    core.set_output_delay(0, 0.0).unwrap();
    assert!(!core.preset_dirty());
}

/// A change someone else made is compared with the same baseline.
#[test]
fn wp_swapfix_f2_someone_elses_change_is_unsaved_after_a_reread() {
    let (core, remote) = demo();
    change_unheard(&remote, |s| s.output_gain_db[1] = -6.0);
    core.fetch_all().unwrap();
    assert!(core.preset_dirty());
    assert!(
        core.preset_changes().iter().any(|c| c.contains("gain")),
        "{:?}",
        core.preset_changes()
    );
}

/// Another device, or another preset active, has a baseline of its own.
#[test]
fn wp_swapfix_f2_another_device_or_preset_captures_a_new_baseline() {
    let (core, _) = demo();
    core.set_output_delay(0, 1.5).unwrap();
    let dev = dspi_transport::FakeDevice::demo();
    let other = dev.remote();
    change_unheard(&other, |s| s.output_gain_db[1] = -6.0);
    core.disconnect();
    core.attach_probed(Box::new(dev), "OTHER".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    assert!(!core.preset_dirty(), "{:?}", core.preset_changes());

    core.set_output_delay(0, 1.5).unwrap();
    assert!(core.preset_dirty());
    // The baseline was taken on another preset than the one active now.
    {
        let mut inner = core.lock();
        let slot = inner.state.active_preset_slot;
        if let Some(of) = inner.baseline_of.as_mut() {
            of.1 = slot.wrapping_add(1);
        }
    }
    core.fetch_all().unwrap();
    assert!(!core.preset_dirty());
}

/// A factory reset still makes what the device runs the baseline.
#[test]
fn wp_swapfix_f2_a_factory_reset_captures_the_baseline() {
    let (core, _) = demo();
    core.set_output_delay(0, 1.5).unwrap();
    core.factory_reset().unwrap();
    assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
}

// ── F3: an output's delay is listed once ────────────────────────────

#[test]
fn wp_swapfix_f3_an_output_delay_is_one_line() {
    let (core, _) = demo();
    core.set_output_delay(0, 1.5).unwrap();
    let changes = core.preset_changes();
    let delays: Vec<_> = changes.iter().filter(|c| c.contains("delay")).collect();
    assert_eq!(delays.len(), 1, "{changes:?}");
    assert!(delays[0].ends_with("delay: 0 ms → 1.5 ms"), "{changes:?}");
}

/// An input's delay is still listed (it has one copy).
#[test]
fn wp_swapfix_f3_an_input_delay_is_listed() {
    let (core, _) = demo();
    core.set_delay(0, 2.0).unwrap();
    let changes = core.preset_changes();
    let delays: Vec<_> = changes.iter().filter(|c| c.contains("delay")).collect();
    assert_eq!(delays.len(), 1, "{changes:?}");
}
