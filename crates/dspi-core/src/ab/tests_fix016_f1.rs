//! FIX016 F1 (REVG F1, reviewer A's A1): "as saved" against "now" never
//! jumps the master volume, also when it travels with presets ("with
//! preset" mode): the master volume is not compared.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;
use crate::confirm::ConfirmOp;
use crate::presets::WITH_PRESET;
use crate::records::FilterParams;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "FIX016-F1".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

fn master(core: &DspiCore) -> f32 {
    core.get_state().master_volume_db
}

#[test]
fn fix016_f1_a_saved_compare_keeps_the_live_master_volume_with_preset() {
    let core = demo();
    core.set_master_volume_mode(WITH_PRESET).unwrap();
    core.set_master_volume(-10.0).unwrap();
    let token = core.confirm_token(ConfirmOp::SaveParams);
    core.save_params_confirmed(Some(token)).unwrap();
    assert!(!core.preset_dirty(), "{:?}", core.preset_changes());
    // The reviewer's probe: the master turned down to -40 dB, and an edit
    core.set_master_volume(-40.0).unwrap();
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
    let c = core.ab_start_saved().unwrap();
    assert_eq!(c.side, Some(AbSide::A));
    assert!(
        (master(&core) + 40.0).abs() < 0.01,
        "A keeps the live master: {}",
        master(&core)
    );
    assert!(
        core.get_state().filters[0][0].gain.abs() < 0.01,
        "A is as saved"
    );
    core.ab_toggle().unwrap();
    assert!(
        (master(&core) + 40.0).abs() < 0.01,
        "B too: {}",
        master(&core)
    );
    assert!((core.get_state().filters[0][0].gain - 3.0).abs() < 0.01);
    core.ab_toggle().unwrap();
    assert!((master(&core) + 40.0).abs() < 0.01, "and A again");
    // The compare says so
    assert!(SAVED_A_NOTE.contains("Master volume is not compared"));
}
