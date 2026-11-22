//! WP ROLL1 tests: the tweeter's stimulus floor (the plan, the ramp, the
//! interlock, single and walked runs) and the roll-off search, pure and on
//! the demo DSPi with the simulated microphone (`measure_sim`). The demo
//! runs play in real time on the core's clock; nothing is played anywhere.

use std::sync::Arc;
use std::time::Duration;

use dspi_measure::doc::MeasurementDoc;

use super::super::plan::{Plan, TWEETER_STIMULUS_FLOOR_HZ};
use super::super::preflight::{SIG_LOG_SWEEP, SIG_PINK};
use super::super::runs::{ramp_config, run_config};
use super::super::session::below_floor;
use super::super::{
    MeasureOutput, MeasurePhase, MeasurePlacement, MeasureRole, MeasureRunKind, MeasureSetup,
    MeasureState,
};
use super::*;
use crate::DspiCore;
use crate::records::{FilterParams, SiggenConfig};

const SEED: u64 = 0x0011_F1F1;
/// The crossover's LR4 (24 dB/octave) high-pass.
const LR4_HP: u8 = 35;
/// The demo world's outputs: 0 a woofer, 1 a midrange, 2 a tweeter.
const WOOFER: u8 = 0;
const MID: u8 = 1;
const TWEETER: u8 = 2;

fn out(output: u8, role: MeasureRole, group: u8) -> MeasureOutput {
    MeasureOutput {
        output,
        role,
        speaker: "L".into(),
        group,
        inverted: false,
    }
}

fn setup(kind: MeasureRunKind, outputs: Vec<MeasureOutput>) -> MeasureSetup {
    MeasureSetup {
        kind,
        outputs,
        placement: MeasurePlacement::ListeningPosition,
        sweep_ms: 1_000,
        gap_ms: 300,
        start_level_only: true,
        name: "ROLL1".into(),
        ..MeasureSetup::default()
    }
}

fn flat_chain() -> dspi_measure::types::ChannelChain {
    dspi_measure::types::ChannelChain {
        peq: Vec::new(),
        crossover: Vec::new(),
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: dspi_measure::types::ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

fn starts_at(cfg: &SiggenConfig) -> f32 {
    cfg.p.first().copied().unwrap_or(0.0)
}

// ── The plan (pure) ─────────────────────────────────────────────────────

#[test]
fn wp_roll1_a_tweeters_single_run_and_its_ramp_start_at_1khz() {
    for kind in [MeasureRunKind::LiveSweep, MeasureRunKind::Walked] {
        let p = Plan::new(&setup(kind, vec![out(3, MeasureRole::High, 0)])).unwrap();
        assert_eq!(p.runs.len(), 1);
        let run = &p.runs[0];
        assert_eq!(run.f1_hz, TWEETER_STIMULUS_FLOOR_HZ);
        assert_eq!(starts_at(&run_config(&p, run, -40.0)), 1_000.0);
        assert_eq!(starts_at(&ramp_config(&p, run, 3, -50.0)), 1_000.0);
        assert_eq!(p.sweep_of(run).f2_hz, 20_000.0);
        let s = p.schedule(run, 48_000).unwrap();
        assert!(s.planned_capture_frames(48_000, 1_000) > 0);
    }
    // A non-tweeter keeps the full 20 Hz sweep.
    let p = Plan::new(&setup(
        MeasureRunKind::LiveSweep,
        vec![out(0, MeasureRole::Low, 0)],
    ))
    .unwrap();
    assert_eq!(starts_at(&run_config(&p, &p.runs[0], -40.0)), 20.0);
    assert_eq!(starts_at(&ramp_config(&p, &p.runs[0], 0, -50.0)), 20.0);
}

#[test]
fn wp_roll1_a_walked_group_of_a_woofer_and_a_tweeter_is_two_runs() {
    let mut s = setup(
        MeasureRunKind::Walked,
        vec![
            out(0, MeasureRole::Low, 0),
            out(1, MeasureRole::High, 0),
            out(4, MeasureRole::Sub, 1),
        ],
    );
    s.placement = MeasurePlacement::MultiPosition;
    s.count = 3;
    let p = Plan::new(&s).unwrap();
    // Per position: the group band-limited (its timing), its woofer full
    // band, the sub's own group.
    assert_eq!(p.runs.len(), 9);
    for pos in 0..3u8 {
        let mine: Vec<_> = p.runs.iter().filter(|r| r.position == pos).collect();
        assert_eq!(mine[0].outputs, vec![0, 1]);
        assert_eq!(mine[0].f1_hz, 1_000.0);
        assert!(mine[0].aligns);
        assert_eq!(mine[1].outputs, vec![0]);
        assert_eq!(mine[1].f1_hz, 20.0);
        assert!(!mine[1].aligns);
        assert_eq!(mine[2].outputs, vec![4]);
        assert_eq!(mine[2].f1_hz, 20.0);
    }
    for run in &p.runs {
        let cfg = run_config(&p, run, -40.0);
        let tweeter = run.outputs.contains(&1);
        assert_eq!(starts_at(&cfg) >= 1_000.0, tweeter, "{run:?}");
        assert!(below_floor(&p, &cfg).is_none());
        for o in &run.outputs {
            let ramp = ramp_config(&p, run, *o, -50.0);
            assert_eq!(
                starts_at(&ramp),
                starts_at(&cfg),
                "the ramp plays the run's sweep"
            );
        }
    }
    // Near field: one run per output, the tweeter's from 1 kHz.
    s.placement = MeasurePlacement::NearField;
    let p = Plan::new(&s).unwrap();
    let f1: Vec<f32> = p.runs.iter().map(|r| r.f1_hz).collect();
    assert_eq!(f1, vec![20.0, 1_000.0, 20.0]);
    // A summed run with a tweeter starts at 1 kHz.
    let p = Plan::new(&setup(
        MeasureRunKind::Summed,
        vec![out(0, MeasureRole::Low, 0), out(1, MeasureRole::High, 0)],
    ))
    .unwrap();
    assert_eq!(starts_at(&run_config(&p, &p.runs[0], -40.0)), 1_000.0);
}

#[test]
fn wp_roll1_the_effective_role_is_resolved_into_the_plan() {
    let mut p = Plan::new(&setup(
        MeasureRunKind::Walked,
        vec![out(0, MeasureRole::Low, 0), out(1, MeasureRole::Full, 0)],
    ))
    .unwrap();
    assert_eq!(p.runs.len(), 1);
    let mut chain = flat_chain();
    chain.crossover = vec![dspi_measure::types::BandSpec {
        filter_type: LR4_HP,
        bypass: false,
        freq: 3_000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }];
    p.resolve_roles(&vec![(1, chain)]).unwrap();
    assert_eq!(
        p.role_of(1),
        MeasureRole::High,
        "a 3 kHz high-pass is a tweeter"
    );
    assert_eq!(p.role_of(0), MeasureRole::Low);
    assert_eq!(p.runs.len(), 2);
    assert_eq!(p.runs[0].f1_hz, 1_000.0);
}

#[test]
fn wp_roll1_a_three_way_group_plays_the_tweeter_with_the_way_that_plays_highest() {
    let band = |filter_type: u8, freq: f32| dspi_measure::types::BandSpec {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    };
    // LR4 low-pass 34, high-pass 35 (the crossover's types).
    let with = |bands: Vec<dspi_measure::types::BandSpec>| {
        let mut c = flat_chain();
        c.crossover = bands;
        c
    };
    let mut p = Plan::new(&setup(
        MeasureRunKind::Walked,
        vec![
            out(0, MeasureRole::Low, 0),
            out(1, MeasureRole::Mid, 0),
            out(2, MeasureRole::High, 0),
        ],
    ))
    .unwrap();
    p.resolve_roles(&vec![
        (0, with(vec![band(34, 400.0)])),
        (1, with(vec![band(35, 400.0), band(34, 3_000.0)])),
        (2, with(vec![band(LR4_HP, 3_000.0)])),
    ])
    .unwrap();
    // The woofer, low-passed at 400 Hz, would not be heard above 1 kHz: the
    // midrange is the tweeter's reference.
    assert_eq!(p.runs.len(), 2);
    assert_eq!(p.runs[0].outputs, vec![1, 2]);
    assert_eq!(p.runs[0].f1_hz, 1_000.0);
    assert_eq!(p.runs[1].outputs, vec![0, 1]);
    assert_eq!(p.runs[1].f1_hz, 20.0);
}

#[test]
fn wp_roll1_pink_noise_never_plays_a_tweeter() {
    let e = Plan::new(&setup(
        MeasureRunKind::Pink,
        vec![out(2, MeasureRole::High, 0)],
    ))
    .unwrap_err()
    .to_string();
    assert!(e.contains("measure.tweeter_stimulus"), "{e}");
    let mut p = Plan::new(&setup(
        MeasureRunKind::Pink,
        vec![out(2, MeasureRole::Full, 0)],
    ))
    .unwrap();
    let mut chain = flat_chain();
    chain.peq = vec![dspi_measure::types::BandSpec {
        filter_type: LR4_HP,
        bypass: false,
        freq: 2_000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }];
    assert!(p.resolve_roles(&vec![(2, chain)]).is_err());
}

#[test]
fn wp_roll1_the_interlock_refuses_anything_below_the_floor() {
    let p = Plan::new(&setup(
        MeasureRunKind::Walked,
        vec![out(0, MeasureRole::Low, 0), out(1, MeasureRole::High, 0)],
    ))
    .unwrap();
    let sweep = |mask: u16, f1: f32| SiggenConfig {
        signal_type: SIG_LOG_SWEEP,
        channel_mask: mask,
        invert_mask: 0,
        flags: 0,
        level_db: -40.0,
        duration_ms: 1_000,
        repeat: 1,
        gap_ms: 300,
        p: vec![f1, 20_000.0, 0.0, 0.0],
    };
    assert!(below_floor(&p, &sweep(0b10, 20.0)).is_some());
    assert!(below_floor(&p, &sweep(0b11, 999.0)).is_some());
    assert!(below_floor(&p, &sweep(0b10, f32::NAN)).is_some());
    assert!(below_floor(&p, &sweep(0b10, 1_000.0)).is_none());
    assert!(below_floor(&p, &sweep(0b01, 20.0)).is_none());
    let pink = SiggenConfig {
        signal_type: SIG_PINK,
        channel_mask: 0b10,
        p: vec![0.0; 4],
        ..sweep(0b10, 1_000.0)
    };
    assert!(below_floor(&p, &pink).is_some());
    // A reversed sweep that ends low is below the floor too.
    let mut down = sweep(0b10, 20_000.0);
    down.p[1] = 500.0;
    assert!(below_floor(&p, &down).is_some());
}

// ── The judgement (pure) ────────────────────────────────────────────────

/// A curve on the standard grid: `f(hz)` dB.
fn curve(f: impl Fn(f64) -> f64) -> (Vec<f64>, Vec<f64>) {
    let freqs = dspi_measure::types::LogGrid::STANDARD.freqs().unwrap();
    let mag = freqs.iter().map(|x| f(*x)).collect();
    (freqs, mag)
}

/// An LR4 high-pass at `fc`, dB.
fn lr4(f: f64, fc: f64) -> f64 {
    let r4 = (f / fc).powi(4);
    2.0 * 10.0 * (r4 / (1.0 + r4)).log10()
}

#[test]
fn wp_roll1_judge_finds_the_15_db_point() {
    // A tweeter rolling off with an LR4 at 3 kHz, played from 1 kHz: -15 dB
    // lies near 2.1 kHz.
    let (f, m) = curve(|x| lr4(x, 3_000.0) - 20.0);
    let v = judge(&f, &m, None, 1_000.0, 15.0).unwrap();
    let hz = v.rolloff_hz.unwrap();
    assert!((1_900.0..2_300.0).contains(&hz), "{hz}");
    assert!(v.reference_db > -21.0 && v.reference_db < -19.0);
    assert!(v.low_end_db < -15.0);
    // Played from 4 kHz only, it has not rolled off yet.
    let v = judge(&f, &m, None, 4_000.0, 15.0).unwrap();
    assert!(v.rolloff_hz.is_none());
    assert!(v.low_end_db > -3.0);
    // Flat: never.
    let (f, m) = curve(|_| -10.0);
    let v = judge(&f, &m, None, 1_000.0, 15.0).unwrap();
    assert!(v.rolloff_hz.is_none());
    // A narrow notch deeper than 15 dB is smoothed away.
    let (f, m) = curve(|x| {
        if (x / 5_000.0).log2().abs() < 0.01 {
            -40.0
        } else {
            0.0
        }
    });
    assert!(
        judge(&f, &m, None, 1_000.0, 15.0)
            .unwrap()
            .rolloff_hz
            .is_none()
    );
    // Untrusted points are not judged; too few trusted: no verdict.
    let (f, m) = curve(|x| lr4(x, 3_000.0));
    let valid: Vec<bool> = f.iter().map(|x| *x >= 2_500.0).collect();
    let v = judge(&f, &m, Some(&valid), 1_000.0, 15.0).unwrap();
    assert!(v.rolloff_hz.is_none() && v.freqs_hz[0] >= 2_500.0);
    assert!(judge(&f, &m, Some(&vec![false; f.len()]), 1_000.0, 15.0).is_none());
}

#[test]
fn wp_roll1_the_segments_come_down_to_the_floor_and_no_lower() {
    let s = segments(8_000.0, 1_000.0);
    assert_eq!(s.first(), Some(&8_000.0));
    assert_eq!(s.last(), Some(&1_000.0));
    assert!(s.windows(2).all(|w| w[1] < w[0]));
    assert!(s.iter().all(|f| *f >= 1_000.0));
    assert_eq!(s.len(), 7, "{s:?}");
    // A start below the floor is the floor.
    assert_eq!(segments(500.0, 1_000.0), vec![1_000.0]);
    // A way without a floor goes to 20 Hz.
    assert_eq!(segments(8_000.0, 0.0).last(), Some(&20.0));
}

// ── The demo DSPi and the simulated microphone ──────────────────────────

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn hp(freq: f32) -> FilterParams {
    FilterParams {
        filter_type: LR4_HP,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

/// The demo DSPi ready to measure, its simulated microphone on: the volume
/// at -20 dB, the processing the maths does not model off, outputs 0 to 2
/// on, the midrange's and the tweeter's crossover high-passes as given (an
/// LR4; `None` leaves a flat crossover).
fn demo(mid_hp: Option<f32>, tweeter_hp: Option<f32>) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
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
    let n_in = s.num_input_channels;
    for o in 0..3u8 {
        if !s.output_enabled[usize::from(o)] {
            core.set_output_enable(o, true).unwrap();
        }
        if s.output_muted[usize::from(o)] {
            core.set_output_mute(o, false).unwrap();
        }
        let hp_hz = match o {
            MID => mid_hp,
            TWEETER => tweeter_hp,
            _ => None,
        };
        let bands = core.get_state().xover[usize::from(n_in + o)].len() as u8;
        for b in 0..bands {
            let p = match (b, hp_hz) {
                (0, Some(hz)) => hp(hz),
                _ => FilterParams {
                    filter_type: 0,
                    bypass: true,
                    freq: 1_000.0,
                    q: 0.707,
                    gain: 0.0,
                    qp: 0.0,
                },
            };
            core.set_crossover(n_in + o, b, p).unwrap();
        }
    }
    settle(&core);
    let info = core.measure_sim_set(true, SEED).unwrap();
    assert!(info.on, "{info:?}");
    core
}

/// Every generator configuration the demo DSPi was given, read back after
/// each tick (each plays a second or more, many ticks).
#[derive(Default)]
struct Starts(Vec<SiggenConfig>);

impl Starts {
    fn see(&mut self, core: &DspiCore) {
        if let Ok(c) = core.siggen_get_config()
            && c.channel_mask != 0
            && self.0.last() != Some(&c)
        {
            self.0.push(c);
        }
    }

    /// Every configuration that plays `output` starts at `f1` or above.
    fn floor_holds(&self, output: u8, f1: f32) {
        let bit = 1u16 << output;
        let on: Vec<&SiggenConfig> = self
            .0
            .iter()
            .filter(|c| c.channel_mask & bit != 0)
            .collect();
        assert!(!on.is_empty(), "output {output} was never played");
        for c in on {
            assert_eq!(c.signal_type, SIG_LOG_SWEEP, "{c:?}");
            assert!(
                starts_at(c) >= f1,
                "output {output} played from {} Hz: {c:?}",
                starts_at(c)
            );
        }
    }
}

/// A session's client loop: tick, nothing pushed (the simulated microphone).
fn run(core: &DspiCore, starts: &mut Starts, secs: u64) -> MeasureState {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let st = core.measure_tick();
        starts.see(core);
        if !st.active || std::time::Instant::now() > end {
            return st;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn rolloff(core: &DspiCore, starts: &mut Starts, secs: u64) -> MeasureRolloffState {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let st = core.measure_rolloff_tick();
        starts.see(core);
        if !st.running || std::time::Instant::now() > end {
            return st;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wp_roll1_sim_a_single_sweep_on_a_tweeter_starts_at_1khz_and_a_woofer_at_20hz() {
    // The tweeter called Full: its 3 kHz high-pass makes it one.
    let core = demo(None, Some(3_000.0));
    let mut starts = Starts::default();
    let st = core
        .measure_start(setup(
            MeasureRunKind::LiveSweep,
            vec![out(TWEETER, MeasureRole::Full, 0)],
        ))
        .unwrap();
    assert!(st.active);
    let st = run(&core, &mut starts, 60);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    starts.floor_holds(TWEETER, 1_000.0);
    let doc = MeasurementDoc::from_text(&core.measure_result().unwrap()).unwrap();
    let o = &doc.measurement.outputs[0];
    let freqs = o.response.grid.freqs().unwrap();
    let valid = o.response.valid.clone().unwrap();
    assert!(
        freqs.iter().zip(&valid).all(|(f, v)| !*v || *f >= 1_000.0),
        "nothing below 1 kHz is trusted"
    );
    assert!(
        valid.iter().any(|v| *v),
        "the band-limited sweep was analysed"
    );
    assert_eq!(doc.measurement.extra.len(), 0);
    assert_eq!(
        doc.measurement.stimulus.extra["tweeterFloor"]["outputs"],
        serde_json::json!([TWEETER])
    );
    core.measure_clear().unwrap();
    // A woofer still gets the full 20 Hz sweep.
    let mut starts = Starts::default();
    core.measure_start(setup(
        MeasureRunKind::LiveSweep,
        vec![out(WOOFER, MeasureRole::Low, 0)],
    ))
    .unwrap();
    let st = run(&core, &mut starts, 60);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    let woofer: Vec<_> = starts
        .0
        .iter()
        .filter(|c| c.channel_mask == 1 << WOOFER)
        .collect();
    assert!(!woofer.is_empty());
    assert!(woofer.iter().all(|c| starts_at(c) == 20.0), "{woofer:?}");
    let doc = MeasurementDoc::from_text(&core.measure_result().unwrap()).unwrap();
    let o = &doc.measurement.outputs[0];
    let freqs = o.response.grid.freqs().unwrap();
    let valid = o.response.valid.clone().unwrap();
    assert!(
        freqs.iter().zip(&valid).any(|(f, v)| *v && *f < 100.0),
        "the woofer is measured down low"
    );
    // Pink noise on the tweeter is refused before anything plays.
    core.measure_clear().unwrap();
    let e = core
        .measure_start(setup(
            MeasureRunKind::Pink,
            vec![out(TWEETER, MeasureRole::Full, 0)],
        ))
        .unwrap_err()
        .to_string();
    assert!(e.contains("measure.tweeter_stimulus"), "{e}");
    assert!(!core.measure_state().active);
}

#[test]
fn wp_roll1_sim_a_walked_run_gives_the_tweeter_its_floor_and_the_woofer_its_low_end() {
    let core = demo(None, Some(3_000.0));
    let mut starts = Starts::default();
    core.measure_start(setup(
        MeasureRunKind::Walked,
        vec![
            out(WOOFER, MeasureRole::Low, 0),
            out(TWEETER, MeasureRole::High, 0),
        ],
    ))
    .unwrap();
    let st = run(&core, &mut starts, 120);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    assert_eq!(
        st.runs, 2,
        "the group band-limited, then the woofer full band"
    );
    starts.floor_holds(TWEETER, 1_000.0);
    assert!(
        starts
            .0
            .iter()
            .any(|c| c.channel_mask == 1 << WOOFER && starts_at(c) == 20.0),
        "the woofer's full-band run: {:?}",
        starts.0
    );
    let doc = MeasurementDoc::from_text(&core.measure_result().unwrap()).unwrap();
    let m = &doc.measurement;
    assert_eq!(m.outputs.len(), 2);
    let trusted_low = |o: &dspi_measure::doc::OutputMeasurement| {
        let freqs = o.response.grid.freqs().unwrap();
        let valid = o.response.valid.clone().unwrap();
        freqs
            .iter()
            .zip(&valid)
            .filter(|(_, v)| **v)
            .map(|(f, _)| *f)
            .fold(f64::INFINITY, f64::min)
    };
    let w = m.outputs.iter().find(|o| o.output == WOOFER).unwrap();
    let t = m.outputs.iter().find(|o| o.output == TWEETER).unwrap();
    assert!(trusted_low(w) < 100.0, "woofer from {}", trusted_low(w));
    assert!(trusted_low(t) >= 1_000.0, "tweeter from {}", trusted_low(t));
    // Both arrivals from the one band-limited run (one timing origin).
    if m.capture.timing_reliable {
        let (aw, at) = (w.arrival_ms.unwrap(), t.arrival_ms.unwrap());
        let s = core.get_state();
        let want = f64::from(s.output_delay_ms[usize::from(TWEETER)])
            - (0.25 + f64::from(s.output_delay_ms[usize::from(WOOFER)]));
        eprintln!(
            "ROLL1 arrival tweeter - woofer: {:.4} ms, the world's {want:.4} ms",
            at - aw
        );
        assert!(((at - aw) - want).abs() < 0.1, "{} vs {want}", at - aw);
    }
}

#[test]
fn wp_roll1_sim_the_search_stops_15_db_down_on_a_tweeter() {
    let core = demo(None, Some(3_000.0));
    let mut starts = Starts::default();
    let st = core
        .measure_rolloff_start(MeasureRolloffSetup {
            output: TWEETER,
            role: MeasureRole::Full,
            speaker: "L".into(),
            placement: MeasurePlacement::NearField,
            sweep_ms: 1_000,
            start_level_only: true,
            name: "ROLL1 tweeter".into(),
            ..MeasureRolloffSetup::default()
        })
        .unwrap();
    assert!(st.running);
    assert_eq!(
        st.role,
        MeasureRole::High,
        "the 3 kHz high-pass makes it a tweeter"
    );
    assert_eq!(st.floor_hz, 1_000.0);
    assert_eq!(st.segment_start_hz, ROLLOFF_START_HZ);
    let st = rolloff(&core, &mut starts, 240);
    assert_eq!(st.stage, MeasureRolloffStage::Done, "{st:#?}");
    let r = st.result.clone().unwrap();
    eprintln!(
        "ROLL1 tweeter: {} segments, the reference {:.1} dB at {:.0} Hz, -15 dB at {:?} Hz",
        st.segment, r.reference_db, r.reference_hz, r.rolloff_hz
    );
    assert_eq!(r.end, MeasureRolloffEnd::RolledOff);
    assert!(r.note.is_none());
    assert_eq!(r.stop_db, ROLLOFF_STOP_DB);
    let hz = r.rolloff_hz.unwrap();
    assert!((1_200.0..3_000.0).contains(&hz), "{hz}");
    assert!(
        r.lowest_hz >= 1_000.0 && r.lowest_hz < hz,
        "{}",
        r.lowest_hz
    );
    assert!(r.low_end_db <= -15.0, "{}", r.low_end_db);
    assert!(
        st.segment >= 2 && st.segment < st.segments,
        "stopped before the floor"
    );
    starts.floor_holds(TWEETER, 1_000.0);
    // The segments came down from the top, each start lower.
    let f1: Vec<f32> = starts.0.iter().map(starts_at).collect();
    assert!(f1.windows(2).all(|w| w[1] <= w[0]), "{f1:?}");
    assert!(MeasurementDoc::from_text(&r.document).is_ok());
    assert!(core.held_lock_holder().is_none(), "the lock given back");
    assert!(!core.measure_state().active);
    core.measure_rolloff_clear().unwrap();
    assert_eq!(
        core.measure_rolloff_state().stage,
        MeasureRolloffStage::Idle
    );
}

#[test]
fn wp_roll1_sim_a_midrange_set_up_as_a_tweeter_stops_at_1khz_with_the_note() {
    // The midrange with a 1 kHz high-pass: a tweeter for safety.
    let core = demo(Some(1_000.0), None);
    let mut starts = Starts::default();
    let st = core
        .measure_rolloff_start(MeasureRolloffSetup {
            output: MID,
            role: MeasureRole::High,
            speaker: "L".into(),
            placement: MeasurePlacement::NearField,
            start_hz: 4_000.0,
            sweep_ms: 1_000,
            start_level_only: true,
            ..MeasureRolloffSetup::default()
        })
        .unwrap();
    assert_eq!(st.segments, 5, "4 kHz to 1 kHz in half octaves");
    let st = rolloff(&core, &mut starts, 240);
    assert_eq!(st.stage, MeasureRolloffStage::Done, "{st:#?}");
    let r = st.result.clone().unwrap();
    eprintln!(
        "ROLL1 midrange: the reference {:.1} dB at {:.0} Hz, {:.1} dB at the low end",
        r.reference_db, r.reference_hz, r.low_end_db
    );
    assert_eq!(r.end, MeasureRolloffEnd::Floor);
    assert!(r.rolloff_hz.is_none());
    assert_eq!(r.lowest_hz, 1_000.0);
    let note = r.note.unwrap();
    assert_eq!(note.key, KEY_MIDRANGE);
    assert_eq!(note.output, Some(MID));
    assert!(
        note.message.contains("probably a midrange"),
        "{}",
        note.message
    );
    starts.floor_holds(MID, 1_000.0);
    assert!(
        starts.0.iter().any(|c| starts_at(c) == 1_000.0),
        "the floor was reached"
    );
}

#[test]
fn wp_roll1_sim_the_search_is_refused_and_stopped_cleanly() {
    let core = demo(None, Some(3_000.0));
    // A start out of range.
    let bad = MeasureRolloffSetup {
        output: TWEETER,
        start_hz: 15_000.0,
        ..MeasureRolloffSetup::default()
    };
    assert!(core.measure_rolloff_start(bad).is_err());
    // An unconfirmed layout.
    let e = core
        .measure_rolloff_start(MeasureRolloffSetup {
            output: TWEETER,
            roles_from_layout: true,
            ..MeasureRolloffSetup::default()
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("measure.layout_unconfirmed"), "{e}");
    assert_eq!(
        core.measure_rolloff_state().stage,
        MeasureRolloffStage::Idle
    );
    // The preflight still holds: a loud volume is refused before anything
    // plays.
    core.set_user_volume(0.0).unwrap();
    settle(&core);
    let setup = MeasureRolloffSetup {
        output: TWEETER,
        sweep_ms: 1_000,
        start_level_only: true,
        ..MeasureRolloffSetup::default()
    };
    assert!(core.measure_rolloff_start(setup.clone()).is_err());
    assert!(!core.measure_state().active);
    core.set_user_volume(-20.0).unwrap();
    settle(&core);
    // Started, then stopped: no result, nothing left playing or held.
    let st = core.measure_rolloff_start(setup.clone()).unwrap();
    assert!(st.running);
    assert!(core.measure_rolloff_start(setup).is_err(), "one at a time");
    for _ in 0..10 {
        core.measure_rolloff_tick();
        std::thread::sleep(Duration::from_millis(20));
    }
    let st = core.measure_rolloff_stop();
    assert_eq!(st.stage, MeasureRolloffStage::Stopped);
    assert!(st.result.is_none());
    assert_eq!(st.issue.unwrap().key, "measure.stopped");
    assert!(!core.measure_state().active);
    assert!(core.held_lock_holder().is_none());
}

#[test]
fn wp_roll1_a_job_carries_its_actor() {
    use crate::actor::{ActorScope, JobActor, register_actor};
    let core = demo(None, Some(3_000.0));
    assert_eq!(JobActor::capture(&core).actor(), crate::actor::EMBEDDER);
    let agent = register_actor("ROLL1 agent");
    let st = {
        let _scope = ActorScope::enter(&core, agent, None);
        assert_eq!(JobActor::capture(&core).actor(), agent);
        core.measure_rolloff_start(MeasureRolloffSetup {
            output: TWEETER,
            sweep_ms: 1_000,
            start_level_only: true,
            ..MeasureRolloffSetup::default()
        })
        .unwrap()
    };
    assert_eq!(st.actor, agent);
    // Ticked from another thread (outside any scope): still the agent's.
    let c2 = Arc::clone(&core);
    let st = std::thread::spawn(move || c2.measure_rolloff_tick())
        .join()
        .unwrap();
    assert_eq!(st.actor, agent);
    assert_eq!(st.session.actor, agent, "the segment's session too");
    core.measure_rolloff_stop();
    // An attribution-only scope journals as the actor and passes the drag
    // gate; the agent's own scope is refused while the user drags.
    core.set_dragging(true);
    {
        let _scope = ActorScope::enter(&core, agent, None);
        assert!(
            core.set_output_gain(0, -3.0).is_err(),
            "the user is editing"
        );
    }
    {
        let _scope = JobActor::of(agent).attribute(&core);
        core.set_output_gain(0, -3.0).unwrap();
    }
    let newest = core.history(1).first().cloned().unwrap();
    assert_eq!(newest.actor, agent, "{newest:?}");
    // The embedder's job ticked inside an agent's scope (an MCP server
    // polling it) is the embedder's again: not gated, not attributed.
    {
        let _agent = ActorScope::enter(&core, agent, None);
        let _job = JobActor::of(crate::actor::EMBEDDER).attribute(&core);
        core.set_output_gain(0, -4.0).unwrap();
        // The generator's STOP passes while the user drags.
        core.siggen_control(crate::consts::SIGGEN_CTL_STOP_NOW)
            .unwrap();
    }
    {
        let _agent = ActorScope::enter(&core, agent, None);
        assert!(
            core.siggen_control(crate::consts::SIGGEN_CTL_STOP_NOW)
                .is_err(),
            "outside the job's scope the agent's write waits for the drag"
        );
    }
    core.set_dragging(false);
    let newest = core.history(1).first().cloned().unwrap();
    assert_eq!(newest.actor, crate::actor::EMBEDDER, "{newest:?}");
}
