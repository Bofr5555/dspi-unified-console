//! WP UB34 tests (UXP B3, owner D7): the A/B compare of "as saved on the
//! device" against "now", on the demo device.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;
use crate::confirm::ConfirmOp;
use crate::records::FilterParams;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "UB34-DEMO".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

/// An edit a preset holds: output 1's first band a 3 dB peak at 2.5 kHz.
fn edit(core: &DspiCore) {
    core.set_filter(
        0,
        0,
        FilterParams {
            filter_type: 1,
            bypass: false,
            freq: 2500.0,
            q: 1.5,
            gain: 3.0,
            ..core.get_state().filters[0][0]
        },
    )
    .unwrap();
}

fn band(core: &DspiCore) -> f32 {
    core.get_state().filters[0][0].gain
}

fn save(core: &DspiCore) {
    let token = core.confirm_token(ConfirmOp::SaveParams);
    core.save_params_confirmed(Some(token)).unwrap();
}

/// A is the device as saved, B is now: the start lands on A with nothing
/// unsaved, a toggle brings the edit back, and B again is what was there.
#[test]
fn wp_ub34_a_is_as_saved_and_b_is_now() {
    let core = demo();
    let saved = band(&core);
    edit(&core);
    assert!(core.preset_dirty());
    let now = band(&core);
    assert_ne!(saved, now);

    let c = core.ab_start_saved().unwrap();
    assert!(c.active);
    assert_eq!(c.side, Some(AbSide::A));
    assert_eq!((c.a_name.as_str(), c.b_name.as_str()), ("As saved", "Now"));
    assert!(core.ab_is_saved());
    assert_eq!(band(&core), saved, "A is the saved band");
    assert!(
        core.preset_changes().is_empty(),
        "{:?}",
        core.preset_changes()
    );

    let c = core.ab_toggle().unwrap();
    assert_eq!(c.side, Some(AbSide::B));
    assert_eq!(band(&core), now, "B is the edit");
    assert!(core.preset_dirty());

    // Keeping B ends it with the edit in place.
    let c = core.ab_keep(AbSide::B).unwrap();
    assert!(!c.active);
    assert!(!core.ab_is_saved());
    assert_eq!(band(&core), now);
}

/// The listening volume is not part of a preset: starting, toggling and
/// keeping leave it where the person put it last, on either side.
#[test]
fn wp_ub34_the_listening_volume_never_jumps() {
    let core = demo();
    core.set_user_volume(-20.0).unwrap();
    save(&core);
    core.set_user_volume(-12.0).unwrap();
    edit(&core);

    core.ab_start_saved().unwrap();
    assert_eq!(
        core.get_state().user_volume_db,
        -12.0,
        "A keeps the live volume"
    );
    core.set_user_volume(-30.0).unwrap();
    core.ab_toggle().unwrap();
    assert_eq!(core.get_state().user_volume_db, -30.0, "B keeps it too");
    core.set_user_volume(-40.0).unwrap();
    core.ab_toggle().unwrap();
    assert_eq!(core.get_state().user_volume_db, -40.0);
    core.set_user_volume(-35.0).unwrap();
    core.ab_keep(AbSide::B).unwrap();
    assert_eq!(
        core.get_state().user_volume_db,
        -35.0,
        "so does a keep of the other side"
    );
}

/// A plain compare (two snapshots) is not one of "as saved": its sides
/// still carry their own volume, as before.
#[test]
fn wp_ub34_a_plain_compare_is_not_as_saved() {
    let core = demo();
    core.ab_start_documents(
        String::new(),
        core.snapshot_capture_document("B".into(), String::new())
            .unwrap(),
    )
    .unwrap();
    assert!(core.ab_state().active);
    assert!(!core.ab_is_saved());
}

/// A save on B makes B what is saved: the compare ends there, the edit kept
/// and nothing unsaved.
#[test]
fn wp_ub34_a_save_on_b_ends_it() {
    let core = demo();
    edit(&core);
    core.ab_start_saved().unwrap();
    core.ab_toggle().unwrap();
    let now = band(&core);
    save(&core);
    assert!(!core.ab_state().active);
    assert!(!core.ab_is_saved());
    assert_eq!(band(&core), now);
    assert!(!core.preset_dirty());
}

/// A save (or a revert) of the same preset while on A keeps the compare:
/// A is still what is saved, and B's edit comes back on the next toggle.
#[test]
fn wp_ub34_a_save_on_a_keeps_b() {
    let core = demo();
    let saved = band(&core);
    edit(&core);
    let now = band(&core);
    core.ab_start_saved().unwrap();
    save(&core);
    assert!(core.ab_is_saved(), "still comparing");
    core.load_params().unwrap();
    assert!(core.ab_is_saved(), "a revert on A keeps it too");
    assert_eq!(band(&core), saved);
    core.ab_toggle().unwrap();
    assert_eq!(band(&core), now, "B's edit is not lost");
}

/// Another preset loaded, or a revert on B, makes "as saved" another thing:
/// the compare ends.
#[test]
fn wp_ub34_a_load_ends_it() {
    let core = demo();
    edit(&core);
    core.ab_start_saved().unwrap();
    core.ab_toggle().unwrap();
    core.load_params().unwrap();
    assert!(!core.ab_state().active, "a revert on B ends it");

    edit(&core);
    core.ab_start_saved().unwrap();
    let other = (core.get_state().active_preset_slot + 1) % 8;
    let token = core.confirm_token(ConfirmOp::SavePreset);
    core.save_preset_confirmed(other, Some(token)).unwrap();
    assert!(!core.ab_state().active, "a save to another slot ends it");
}

/// With nothing changed A and B are the same; the start still works (the
/// clients only offer it after an edit) and changes nothing.
#[test]
fn wp_ub34_nothing_changed_is_no_change() {
    let core = demo();
    let before = core.get_state();
    core.ab_start_saved().unwrap();
    assert!(core.preset_changes().is_empty());
    assert_eq!(core.get_state().filters, before.filters);
    assert_eq!(core.get_state().user_volume_db, before.user_volume_db);
}

/// No device: nothing to compare.
#[test]
fn wp_ub34_no_device_is_refused() {
    let core = DspiCore::new();
    assert!(core.ab_start_saved().is_err());
    assert!(!core.ab_is_saved());
}
