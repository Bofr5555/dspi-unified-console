//! Review P5-C1: the simulated microphone with the live view (P5DEMO with
//! P5LIVE, as the demo's screens will use them), and a simulated microphone
//! left on when the demo is replaced by a real connection.

use super::tests_wp_p5demo::{UsbKind, demo, make_ready, setup};
use super::*;
use crate::measure::{MeasureLiveMode, MeasureLiveSetup, MeasureLiveState, MeasurePhase};
use dspi_transport::FakeDevice;
use std::time::Duration;

fn live(core: &DspiCore, secs: u64, until: impl Fn(&MeasureLiveState) -> bool) -> MeasureLiveState {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let st = core.measure_live_tick();
        if !st.active || until(&st) || std::time::Instant::now() > end {
            return st;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn review_p5c1_the_live_view_runs_on_the_simulated_microphone() {
    let core = demo();
    core.measure_sim_set(true, 7).unwrap();
    core.measure_live_start(MeasureLiveSetup {
        output: 0,
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureLiveSetup::default()
    })
    .unwrap();
    // Two sweeps: the second session is heard from a fresh pacing.
    let st = live(&core, 90, |s| s.seq >= 2);
    assert!(st.active && st.seq >= 2, "{st:#?}");
    let frame = core.measure_live_frame();
    let m = frame.measured.expect("a measured curve");
    assert!(m.valid.iter().filter(|v| **v).count() > 100);
    assert!(core.measure_sim_info().issue.is_none());
    let st = core.measure_live_stop();
    assert!(!st.active);
    assert!(core.held_lock_holder().is_none());
    assert_eq!(core.siggen_get_status().unwrap().state, 0);

    // Pink noise with the RTA.
    core.measure_live_start(MeasureLiveSetup {
        output: 0,
        mode: MeasureLiveMode::PinkRta,
        pink_ms: 2_000,
        ..MeasureLiveSetup::default()
    })
    .unwrap();
    let st = live(&core, 90, |s| s.seq >= 1 && s.runs >= 2);
    assert!(st.active && st.seq >= 1 && st.runs >= 2, "{st:#?}");
    let rta = core.measure_live_frame().rta.expect("the RTA");
    assert!(rta.frames > 0 && rta.current.is_some());
    core.measure_stop();
    assert!(!core.measure_live_state().active);
    assert_eq!(core.siggen_get_status().unwrap().state, 0);
}

#[test]
fn review_p5c1_a_simulated_microphone_left_on_is_off_at_once_on_a_real_connection() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    make_ready(&core);
    assert!(core.measure_sim_set(true, 7).unwrap().on);
    core.attach_probed(Box::new(UsbKind(FakeDevice::demo())), "P5C1-USB".into(), 0)
        .unwrap();
    make_ready(&core);
    // No tick in between: the screens read the info first.
    let info = core.measure_sim_info();
    assert!(!info.on && !info.available, "{info:?}");
    core.measure_start(setup(&[0])).unwrap();
    let st = core
        .measure_capture_push(0, vec![0.0; 960], 48_000, false)
        .unwrap();
    assert_eq!(st.frames, 960, "the client's first push is taken");
    assert_eq!(st.phase, MeasurePhase::PreRoll);
    core.measure_stop();
}
