//! WP P5DEMO: the Terminal's build carries the demo's simulated microphone
//! (dspi-core's `measure-sim`), so its demo mode (P5TM's `measure ... --demo`)
//! measures without a microphone: a live sweep of the demo device, through
//! the core the Terminal links, heard by P5SIM's room. Nothing is played.

use std::time::{Duration, Instant};

use dspi_core::{
    DspiCore, MeasureOutput, MeasurePhase, MeasureRole, MeasureRunKind, MeasureSetup, SIM_MIC_LABEL,
};

#[test]
fn wp_p5demo_the_terminals_demo_core_measures_with_the_simulated_microphone() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    let info = core.measure_sim_set(true, 3).unwrap();
    assert!(info.available && info.on, "{info:?}");
    assert_eq!(info.label, SIM_MIC_LABEL);
    let setup = MeasureSetup {
        kind: MeasureRunKind::LiveSweep,
        outputs: vec![MeasureOutput {
            output: 0,
            role: MeasureRole::Full,
            ..MeasureOutput::default()
        }],
        sweep_ms: 1_000,
        start_level_only: true,
        ..MeasureSetup::default()
    };
    let pre = core.measure_preflight(setup.clone()).unwrap();
    assert!(pre.clear, "{pre:?}");
    core.measure_start(setup).unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    let st = loop {
        let st = core.measure_tick();
        if !st.active || Instant::now() > end {
            break st;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(st.phase, MeasurePhase::Done, "{st:?}");
    assert!(core.measure_result().unwrap().contains("\"measurement\""));
}
