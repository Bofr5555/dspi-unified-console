//! FIX016 R3 (REVG R3): on side A of "as saved" against "now", B's edits
//! count as unsaved (preset_dirty, preset_changes: the one rule every
//! client's unsaved question reads); a save while on A saves B; a load (a
//! switch or a revert the person said yes to) drops B as asked.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;
use crate::confirm::ConfirmOp;
use crate::records::FilterParams;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "FIX016-R3".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

/// The reviewer's probe's edit: output 1's first band +3 dB.
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

#[test]
fn fix016_r3_on_a_bs_edit_is_unsaved_and_listed() {
    let core = demo();
    let saved = band(&core);
    edit(&core);
    let on_b = core.preset_changes();
    assert!(!on_b.is_empty());
    core.ab_start_saved().unwrap();
    assert_eq!(core.ab_state().side, Some(AbSide::A));
    assert_eq!(band(&core), saved, "the device plays A");
    assert!(core.preset_dirty(), "B's edit is unsaved on A");
    let on_a = core.preset_changes();
    for c in &on_b {
        assert!(on_a.contains(c), "{c:?} not in {on_a:?}");
    }
    // An edit made on A is listed too, each change once
    core.set_output_gain(1, core.get_state().output_gain_db[1] - 2.0)
        .unwrap();
    let both = core.preset_changes();
    assert!(both.len() > on_a.len(), "{both:?}");
    // Kept on A by hand: B goes, and with it its change
    core.ab_keep(AbSide::A).unwrap();
    assert_eq!(core.preset_changes().len(), both.len() - on_b.len());
}

#[test]
fn fix016_r3_a_save_on_a_saves_b() {
    for to_other in [false, true] {
        let core = demo();
        let saved = band(&core);
        edit(&core);
        let now = band(&core);
        let other = (core.get_state().active_preset_slot + 1) % 8;
        core.ab_start_saved().unwrap();
        assert_eq!(band(&core), saved);
        if to_other {
            let token = core.confirm_token(ConfirmOp::SavePreset);
            core.save_preset_confirmed(other, Some(token)).unwrap();
        } else {
            let token = core.confirm_token(ConfirmOp::SaveParams);
            core.save_params_confirmed(Some(token)).unwrap();
        }
        assert!(!core.ab_state().active, "the compare ended");
        assert_eq!(band(&core), now, "B is back on the device");
        assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
        // And B is what the slot holds
        core.load_params().unwrap();
        assert_eq!(band(&core), now, "saved {to_other}");
    }
}

#[test]
fn fix016_r3_a_load_after_the_question_drops_b() {
    let core = demo();
    let first = core.get_state().active_preset_slot;
    let saved = band(&core);
    // Another slot to switch to, holding the saved settings
    let other = (first + 1) % 8;
    let token = core.confirm_token(ConfirmOp::CopyPreset);
    core.copy_preset_confirmed(first, other, Some(token))
        .unwrap();
    edit(&core);
    core.ab_start_saved().unwrap();
    assert!(core.preset_dirty(), "the switch asks");
    // The person said "discard": the load drops B
    core.load_preset(other).unwrap();
    assert!(!core.ab_state().active);
    assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
    core.load_preset(first).unwrap();
    assert_eq!(band(&core), saved, "B's edit is gone, as asked");
    // A revert on A drops it the same way
    edit(&core);
    core.ab_start_saved().unwrap();
    core.load_params().unwrap();
    assert!(!core.ab_state().active, "a revert on A ends it");
    assert!(!core.preset_dirty());
    assert_eq!(band(&core), saved);
}
