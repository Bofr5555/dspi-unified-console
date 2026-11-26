//! WP P5DEMO tests: the demo's simulated microphone through `DspiCore`.
//! The sessions run in real time on the core's clock (the demo device's
//! generator runs on it too); nothing is played anywhere.

use std::sync::Arc;
use std::time::Duration;

use dspi_transport::{
    DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps, TransportKind,
};

use super::world::SimMic;
use super::*;
use crate::measure::{
    MeasureOutput, MeasurePhase, MeasurePlacement, MeasureRole, MeasureSetup, MeasureState,
};
use crate::records::SiggenConfig;

const SEED: u64 = 0x5EED_D3E0;

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Make an open demo device ready to measure as a user would: the volume at
/// -20 dB, the processing the maths does not model off, outputs on.
pub(super) fn make_ready(core: &DspiCore) {
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    let s = core.get_state();
    if s.loudness_enabled {
        core.set_loudness(false).unwrap();
    }
    if s.crossfeed_enabled {
        core.set_crossfeed(false).unwrap();
    }
    if s.psybass_enabled {
        core.set_psybass_enabled(false).unwrap();
    }
    for o in 0..s.num_output_channels.min(4) {
        if !s.output_enabled[usize::from(o)] {
            core.set_output_enable(o, true).unwrap();
        }
        if s.output_muted[usize::from(o)] {
            core.set_output_mute(o, false).unwrap();
        }
    }
    settle(core);
}

pub(super) fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo_blank().unwrap(); // -- WP UB12 --
    make_ready(&core);
    core
}

pub(super) fn setup(outputs: &[u8]) -> MeasureSetup {
    MeasureSetup {
        outputs: outputs
            .iter()
            .map(|o| MeasureOutput {
                output: *o,
                role: MeasureRole::Full,
                speaker: "L".into(),
                ..MeasureOutput::default()
            })
            .collect(),
        placement: MeasurePlacement::ListeningPosition,
        sweep_ms: 1_000,
        gap_ms: 300,
        start_level_only: true,
        name: "P5DEMO".into(),
        ..MeasureSetup::default()
    }
}

/// The client's loop with the simulated microphone: tick, nothing pushed.
fn run(core: &DspiCore, secs: u64) -> MeasureState {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let st = core.measure_tick();
        if !st.active || std::time::Instant::now() > end {
            return st;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The done criterion: a demo-device session produces a measurement without
/// a microphone, with the timing of the world (each way's acoustic offset).
#[test]
fn wp_p5demo_a_demo_session_measures_with_the_simulated_microphone() {
    let core = demo();
    let info = core.measure_sim_set(true, SEED).unwrap();
    assert!(info.available && info.on, "{info:?}");
    assert_eq!(info.label, "Simulated microphone");
    assert_eq!(info.world, SIM_WORLD);
    assert_eq!(info.seed, SEED);
    let st = core.measure_start(setup(&[0, 1, 2])).unwrap();
    assert!(st.active && st.wants_capture);
    let st = run(&core, 120);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    assert_eq!(st.frame_faults, 0);
    let t = st.timing.clone().unwrap();
    assert!(t.reliable, "{t:#?}");
    assert_eq!(t.cycle_outputs, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
    // The world's microphone runs 37.5 ppm fast.
    assert!((t.ratio_ppm - 37.5).abs() < 2.0, "{} ppm", t.ratio_ppm);
    let doc =
        dspi_measure::doc::MeasurementDoc::from_text(&core.measure_result().unwrap()).unwrap();
    let m = &doc.measurement;
    assert!(m.capture.timing_reliable);
    assert_eq!(m.outputs.len(), 3);
    // Arrivals: the woofer's acoustic centre 0.25 ms and the midrange's 0.12
    // ms behind the tweeter's, plus each chain's own delay. The onset also
    // carries each driver's own filters (the midrange's 120 Hz high-pass, the
    // woofer's 3.5 kHz roll-off), a few tens of µs: within 50 µs.
    let s = core.get_state();
    let arr: Vec<f64> = m.outputs.iter().map(|o| o.arrival_ms.unwrap()).collect();
    let offset = [0.25, 0.12, 0.0];
    for i in 1..3 {
        let want = (offset[i] + f64::from(s.output_delay_ms[i]))
            - (offset[0] + f64::from(s.output_delay_ms[0]));
        let got = arr[i] - arr[0];
        eprintln!("P5DEMO arrival of output {i}: {got:.4} ms, the world's offset {want:.4} ms");
        assert!(
            ((got - want) * 1000.0).abs() <= 50.0,
            "output {i}: {got:.4} ms where {want:.4} ms"
        );
    }
    assert!(core.held_lock_holder().is_none(), "the lock given back");
    // A second session on the same core runs again from a fresh pacing.
    core.measure_clear().unwrap();
    let mut short = setup(&[0]);
    short.kind = crate::measure::MeasureRunKind::LiveSweep;
    core.measure_start(short).unwrap();
    let st = run(&core, 60);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    core.measure_sim_set(false, 0).unwrap();
    assert!(!core.measure_sim_info().on);
}

/// While it is on, a client's own pushes are ignored (they would collide
/// with the simulated microphone's frames).
#[test]
fn wp_p5demo_a_clients_push_is_ignored_while_it_is_on() {
    let core = demo();
    core.measure_sim_set(true, SEED).unwrap();
    core.measure_start(setup(&[0])).unwrap();
    for _ in 0..5 {
        let st = core
            .measure_capture_push(999_999, vec![0.5; 960], 48_000, true)
            .unwrap();
        assert!(
            st.active,
            "a clipped block of the client's would abort: {st:?}"
        );
        assert_eq!(st.frames, 0, "nothing of the client's was taken");
    }
    // Refused while running, either way.
    assert!(matches!(
        core.measure_sim_set(false, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    let st = core.measure_stop();
    assert_eq!(st.phase, MeasurePhase::Aborted);
    assert!(core.measure_sim_info().on);
}

/// The demo device behind a transport that says it is a DSPi on USB.
pub(super) struct UsbKind(pub(super) FakeDevice);

impl Transport for UsbKind {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.0.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.0.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.0.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.0.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.0.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.0.notifications()
    }
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Usb,
            ..self.0.caps()
        }
    }
}

/// The done criterion: a real (fake-USB-kind) connection refuses it, and a
/// microphone turned on for the demo turns itself off when the connection
/// becomes a real one.
#[test]
fn wp_p5demo_a_real_connection_refuses_the_simulated_microphone() {
    let core = DspiCore::new();
    assert!(matches!(
        core.measure_sim_set(true, SEED),
        Err(DspiError::Disconnected)
    ));
    core.attach_probed(
        Box::new(UsbKind(FakeDevice::demo())),
        "P5DEMO-USB".into(),
        0,
    )
    .unwrap();
    assert!(matches!(
        core.measure_sim_set(true, SEED),
        Err(DspiError::Unsupported)
    ));
    let info = core.measure_sim_info();
    assert!(!info.available && !info.on, "{info:?}");
    assert!(
        core.measure_sim_set(false, 0).is_ok(),
        "off is always allowed"
    );

    // On for the demo, then the demo replaced by a "USB" DSPi: off at the
    // next tick, and the client's pushes are the microphone again.
    core.open_demo_blank().unwrap(); // -- WP UB12 --
    make_ready(&core);
    assert!(core.measure_sim_set(true, SEED).unwrap().on);
    core.attach_probed(
        Box::new(UsbKind(FakeDevice::demo())),
        "P5DEMO-USB".into(),
        0,
    )
    .unwrap();
    make_ready(&core);
    core.measure_tick();
    let info = core.measure_sim_info();
    assert!(!info.on && !info.available, "{info:?}");
    core.measure_start(setup(&[0])).unwrap();
    let st = core
        .measure_capture_push(0, vec![0.0; 960], 48_000, false)
        .unwrap();
    assert_eq!(st.frames, 960, "the client's push is taken");
    core.measure_stop();
}

fn walked_config() -> SiggenConfig {
    SiggenConfig {
        signal_type: 4,
        channel_mask: 0b11,
        invert_mask: 0,
        flags: crate::consts::SIGGEN_FLAG_WALK,
        level_db: -40.0,
        duration_ms: 1_000,
        repeat: 2,
        gap_ms: 300,
        p: vec![20.0, 20_000.0],
    }
}

fn blocks(mic: &mut SimMic, from: u64, n: usize) -> Vec<(u64, Vec<f32>, bool)> {
    let mut next = from;
    (0..n)
        .map(|_| {
            let b = mic.block(next);
            next = b.0 + b.1.len() as u64;
            b
        })
        .collect()
}

/// Deterministic from the seed: the same configuration at the same frame
/// gives the same samples; another seed another room noise. The blocks are
/// contiguous from the START frame, 20 ms each.
#[test]
fn wp_p5demo_the_microphone_is_deterministic_from_its_seed() {
    let chains = dspi_measure::sim::flat_chains(8);
    let play = |seed: u64| {
        let mut mic = SimMic::new(seed, 8, 48_000);
        let mut out = blocks(&mut mic, 0, 10);
        mic.play(&walked_config(), &chains, 48_000, 9_600);
        assert!(mic.issue().is_none(), "{:?}", mic.issue());
        out.extend(blocks(&mut mic, 9_600, 200));
        out
    };
    let a = play(SEED);
    assert_eq!(a, play(SEED));
    assert_ne!(a, play(SEED + 1));
    let mut next = 0;
    for (first, samples, clipped) in &a {
        assert_eq!(*first, next, "contiguous frame indices");
        assert_eq!(samples.len(), 960);
        assert!(!clipped);
        next += 960;
    }
    // The sweep is heard: well above the room's -85 dBFS.
    let peak = a
        .iter()
        .skip(10)
        .flat_map(|b| b.1.iter())
        .fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak > 10f32.powf(-70.0 / 20.0), "peak {peak}");
}

/// A configuration it cannot render leaves the room's noise and says why.
#[test]
fn wp_p5demo_an_unrenderable_configuration_is_an_issue_not_a_panic() {
    let chains = dspi_measure::sim::flat_chains(2);
    let mut mic = SimMic::new(1, 2, 48_000);
    let mut sine = walked_config();
    sine.signal_type = 0;
    mic.play(&sine, &chains, 48_000, 0);
    assert!(mic.issue().is_some());
    let mut none = walked_config();
    none.channel_mask = 0;
    mic.play(&none, &chains, 48_000, 0);
    assert!(mic.issue().is_some());
    let mut bad = walked_config();
    bad.level_db = f32::NAN;
    bad.duration_ms = u32::MAX;
    mic.play(&bad, &chains, 12_345, 0);
    assert!(mic.issue().is_some());
    let (first, s, _) = mic.block(0);
    assert_eq!((first, s.len()), (0, 960));
    assert!(s.iter().all(|v| v.abs() < 1e-3));
}

/// A generator stopped before its end: after the sound already on its way
/// (the microphone's 50 ms latency) and a short fade, the room's noise only.
#[test]
fn wp_p5demo_a_stop_before_the_end_silences_the_sweep() {
    let chains = dspi_measure::sim::flat_chains(2);
    let mut mic = SimMic::new(SEED, 2, 48_000);
    mic.play(&walked_config(), &chains, 48_000, 0);
    assert!(mic.uncut());
    let before = blocks(&mut mic, 0, 25);
    let loud = |b: &(u64, Vec<f32>, bool)| b.1.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(
        before.iter().skip(5).all(|b| loud(b) > 1e-3),
        "the sweep plays"
    );
    mic.stopped_at(24_000);
    assert!(!mic.uncut());
    // 24000 + 50 ms + 10 ms = 26880: the block from 26880 on is noise.
    let after = blocks(&mut mic, 24_000, 20);
    assert_eq!(after[0].0, 24_000, "contiguous");
    assert!(
        loud(&after[0]) > 1e-3,
        "the sound on its way is still heard"
    );
    assert!(after.iter().skip(3).all(|b| loud(b) < 1e-3), "then silence");
    let mut next = 24_000;
    for (first, s, _) in &after {
        assert_eq!(*first, next);
        next += s.len() as u64;
    }
}

/// The spike front ends' setup (two outputs walked, on a coupler, the start
/// level only: -60 dBFS) reaches a timing verdict whatever the seed: four
/// demo cores measured side by side.
#[test]
fn wp_p5demo_the_spike_setup_is_timing_reliable_for_any_seed() {
    let runs: Vec<_> = [1u64, 7, 42, 0xDEAD_BEEF]
        .into_iter()
        .map(|seed| {
            std::thread::spawn(move || {
                let core = demo();
                core.measure_sim_set(true, seed).unwrap();
                let mut s = setup(&[0, 1]);
                s.placement = MeasurePlacement::Headphones;
                s.coupler = Some(crate::measure::MeasureCoupler::FlatPlate);
                core.measure_start(s).unwrap();
                (seed, run(&core, 120))
            })
        })
        .collect();
    for r in runs {
        let (seed, st) = r.join().unwrap();
        assert_eq!(st.phase, MeasurePhase::Done, "seed {seed}: {:?}", st.issue);
        let t = st.timing.clone().unwrap();
        assert!(t.reliable, "seed {seed}: {t:?}");
        assert_eq!(t.cycle_residuals_us.len(), 6);
        assert!(st.ramp_levels.iter().all(|l| l.level_db == -60.0), "{st:?}");
    }
}
