//! WP P5SES unit tests: the plan, the ramp (S3), the aborts (S4, S5) and the
//! generator configurations (S7) on the session's state machine, with a
//! played device ([`Mock`]) and a simulated clock, so every rule runs in
//! milliseconds. The demo device's tests are `tests_wp_p5ses_device.rs`.

use std::time::Duration;

use proptest::prelude::*;

use super::plan::{CEILING_SPEAKERS_DB, HARD_CAP_HEADPHONES_DB, HARD_CAP_SPEAKERS_DB, Plan};
use super::runs::check_config;
use super::safety::{Ramp, RampNext, StepEval};
use super::session::{Context, Session};
use super::*;
use crate::consts::SIGGEN_FLAG_RAW;

const RATE: u32 = 48_000;
const BLOCK: usize = 960;

/// The device as the session sees it, played by the test.
struct Mock {
    starts: Vec<SiggenConfig>,
    stops: usize,
    tap: Vec<TapEvent>,
    /// The playing configuration and the frames it has left.
    playing: Option<(SiggenConfig, u64)>,
    status: Option<SiggenStatus>,
    meters: SystemStatus,
    gr: Vec<f32>,
    lock_ok: bool,
}

impl Mock {
    fn new() -> Mock {
        Mock {
            starts: Vec::new(),
            stops: 0,
            tap: Vec::new(),
            playing: None,
            status: None,
            meters: SystemStatus {
                peaks: vec![0.0; 17],
                num_channels: 17,
                active_input_channels: 2,
                ..SystemStatus::default()
            },
            gr: vec![0.0; 16],
            lock_ok: true,
        }
    }

    /// The generator's natural length, frames at [`RATE`].
    fn length(cfg: &SiggenConfig) -> u64 {
        let ms = if cfg.signal_type == super::preflight::SIG_PINK {
            u64::from(cfg.duration_ms)
        } else {
            let n = if cfg.flags & crate::consts::SIGGEN_FLAG_WALK != 0 {
                u64::from(cfg.repeat) * u64::from(cfg.channel_mask.count_ones())
            } else {
                u64::from(cfg.repeat)
            };
            n * u64::from(cfg.duration_ms) + n.saturating_sub(1) * u64::from(cfg.gap_ms)
        };
        ms * u64::from(RATE) / 1000
    }
}

impl Device for Mock {
    fn start(&mut self, cfg: &SiggenConfig) -> Result<()> {
        check_config(cfg)?;
        self.starts.push(cfg.clone());
        self.playing = Some((cfg.clone(), Mock::length(cfg)));
        self.tap.push(TapEvent::Siggen {
            state: 2,
            reason: 0,
        });
        Ok(())
    }
    fn stop_now(&mut self) -> Result<()> {
        self.stops += 1;
        self.playing = None;
        self.tap.push(TapEvent::Siggen {
            state: 0,
            reason: 1,
        });
        Ok(())
    }
    fn status(&mut self) -> Result<SiggenStatus> {
        Ok(self.status.unwrap_or(SiggenStatus {
            active_channel: 0xFF,
            ..SiggenStatus::default()
        }))
    }
    fn meters(&mut self) -> Result<SystemStatus> {
        Ok(self.meters.clone())
    }
    fn limiter_gr(&mut self) -> Result<Vec<f32>> {
        Ok(self.gr.clone())
    }
    fn renew_lock(&mut self, _due: bool) -> bool {
        self.lock_ok
    }
    fn drain_tap(&mut self) -> Vec<TapEvent> {
        std::mem::take(&mut self.tap)
    }
}

/// A microphone in a quiet room: white noise at `noise_dbfs` (RMS), plus
/// what the generator plays at `gain(output, level)` peak amplitude: a ramp
/// step's sweep as the generator renders it (WP P5RAMPFIX: the ramp hears by
/// the sweep's matched filter), its pink noise as noise, and a run as a
/// 1 kHz tone (its analysis is not these tests').
struct Room {
    noise: f32,
    gain: Box<dyn Fn(u8, f32) -> f32>,
    seed: u64,
    frame: u64,
    phase: f64,
    /// The playing configuration's samples so far, and its sweep.
    played: u64,
    sweep: Option<(SiggenConfig, Vec<f64>)>,
    /// The simulated clock's origin: time is the frames captured.
    t0: Instant,
}

impl Room {
    fn new(noise_dbfs: f32, gain: impl Fn(u8, f32) -> f32 + 'static) -> Room {
        Room {
            noise: 10f32.powf(noise_dbfs / 20.0) * 3f32.sqrt(),
            gain: Box::new(gain),
            seed: 0x9E37_79B9_7F4A_7C15,
            frame: 1_000,
            phase: 0.0,
            played: 0,
            sweep: None,
            t0: Instant::now(),
        }
    }

    fn rnd(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        ((self.seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    /// The next block, and the device's time moved on by it.
    fn block(&mut self, dev: &mut Mock) -> (u64, Vec<f32>) {
        let playing = dev.playing.as_ref().map(|(c, _)| c.clone());
        let amp = match &playing {
            Some(cfg) => {
                let out = cfg.channel_mask.trailing_zeros() as u8;
                (self.gain)(out, cfg.level_db)
            }
            None => 0.0,
        };
        // A new configuration starts its sweep from its first sample.
        let same = matches!((&playing, &self.sweep), (Some(c), Some((s, _))) if c == s);
        if !same {
            self.played = 0;
            self.sweep = playing.as_ref().and_then(|c| {
                let one = c.signal_type == super::preflight::SIG_LOG_SWEEP
                    && c.repeat == 1
                    && c.flags & crate::consts::SIGGEN_FLAG_WALK == 0;
                one.then(|| {
                    let sc = dspi_measure::stimulus::SweepConfig {
                        f1_hz: c.p[0],
                        f2_hz: c.p[1],
                        duration_ms: c.duration_ms,
                        gap_ms: c.gap_ms,
                    };
                    let sw = dspi_measure::stimulus::LogSweep::new(RATE, &sc).unwrap();
                    (c.clone(), sw.render_cycle(true))
                })
            });
        }
        let pink = playing
            .as_ref()
            .is_some_and(|c| c.signal_type == super::preflight::SIG_PINK);
        let mut v = Vec::with_capacity(BLOCK);
        for _ in 0..BLOCK {
            self.phase += std::f64::consts::TAU * 1000.0 / f64::from(RATE);
            let sig = if playing.is_none() {
                0.0
            } else if let Some((_, sw)) = &self.sweep {
                sw.get(self.played as usize).copied().unwrap_or(0.0) as f32
            } else if pink {
                self.rnd()
            } else {
                self.phase.sin() as f32
            };
            self.played += 1;
            let s = self.noise * self.rnd() + amp * sig;
            v.push(s);
        }
        if playing.is_none() {
            self.sweep = None;
        }
        if let Some((_, left)) = dev.playing.as_mut() {
            *left = left.saturating_sub(BLOCK as u64);
            if *left == 0 {
                dev.playing = None;
                dev.tap.push(TapEvent::Siggen {
                    state: 0,
                    reason: 2,
                });
            }
        }
        let first = self.frame;
        self.frame += BLOCK as u64;
        (first, v)
    }
}

fn ctx(outputs: &[u8]) -> Context {
    Context {
        serial: "P5SES-TEST".into(),
        platform: "RP2350".into(),
        device_rate_hz: RATE,
        chains: outputs
            .iter()
            .map(|o| (*o, dspi_measure::sim::flat_chain()))
            .collect(),
        num_inputs: 8,
        limiter_outputs: Vec::new(),
    }
}

fn setup(kind: MeasureRunKind, outputs: &[u8]) -> MeasureSetup {
    MeasureSetup {
        kind,
        outputs: outputs
            .iter()
            .map(|o| MeasureOutput {
                output: *o,
                ..MeasureOutput::default()
            })
            .collect(),
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureSetup::default()
    }
}

/// Drives `s` for at most `secs` of simulated time, or until `until` holds.
fn drive(
    s: &mut Session,
    dev: &mut Mock,
    room: &mut Room,
    secs: f64,
    mut until: impl FnMut(&Session, &Mock) -> bool,
) {
    let blocks = (secs * f64::from(RATE) / BLOCK as f64) as usize;
    for _ in 0..blocks {
        if s.finished() || until(s, dev) {
            return;
        }
        let now = room.t0 + Duration::from_secs_f64(room.frame as f64 / f64::from(RATE));
        let (first, samples) = room.block(dev);
        s.push(dev, first, &samples, RATE, false, now);
        s.tick(dev, now);
    }
}

fn levels(dev: &Mock) -> Vec<f32> {
    dev.starts.iter().map(|c| c.level_db).collect()
}

fn session(setup: &MeasureSetup, outputs: &[u8]) -> Session {
    Session::new(Plan::new(setup).unwrap(), ctx(outputs), Instant::now())
}

// ── The plan ───────────────────────────────────────────────────────────

#[test]
fn wp_p5ses_the_plan_fills_in_the_defaults() {
    let p = Plan::new(&MeasureSetup {
        outputs: vec![
            MeasureOutput {
                output: 2,
                group: 1,
                ..Default::default()
            },
            MeasureOutput {
                output: 0,
                ..Default::default()
            },
        ],
        ..MeasureSetup::default()
    })
    .unwrap();
    assert_eq!(p.sweep.duration_ms, 5_500);
    assert_eq!(p.sweep.gap_ms, 1_000);
    assert_eq!(p.passes, 3);
    assert_eq!(p.start_db, -50.0);
    assert_eq!(p.ceiling_db, CEILING_SPEAKERS_DB);
    assert_eq!(p.capture_rate_hz, 48_000);
    // One run per alignment group.
    assert_eq!(p.runs.len(), 2);
    assert_eq!(p.runs[0].outputs, vec![0]);
    let hp = Plan::new(&MeasureSetup {
        placement: MeasurePlacement::Headphones,
        count: 2,
        outputs: vec![
            MeasureOutput {
                output: 0,
                ..Default::default()
            },
            MeasureOutput {
                output: 1,
                ..Default::default()
            },
        ],
        ..MeasureSetup::default()
    })
    .unwrap();
    assert_eq!(hp.sweep.gap_ms, 300);
    assert_eq!(hp.start_db, -60.0);
    assert_eq!(hp.ceiling_db, -30.0);
    assert_eq!(hp.runs.len(), 2, "a run per reseat");
}

#[test]
fn wp_p5ses_bad_setups_are_refused() {
    let one = |o: u8| MeasureOutput {
        output: o,
        ..Default::default()
    };
    let bad = [
        MeasureSetup::default(),
        MeasureSetup {
            outputs: vec![one(1), one(1)],
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(16)],
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            placement: MeasurePlacement::MultiPosition,
            count: 2,
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            passes: 1,
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            sweep_ms: 20_000,
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            ceiling_db: Some(-3.0),
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            placement: MeasurePlacement::Headphones,
            ceiling_db: Some(-12.0),
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            kind: MeasureRunKind::Pink,
            pink_ms: 61_000,
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            kind: MeasureRunKind::Summed,
            ..Default::default()
        },
        MeasureSetup {
            outputs: vec![one(0)],
            mic_document: "{\"not\": \"a mic\"}".into(),
            ..Default::default()
        },
    ];
    for s in bad {
        assert!(
            matches!(Plan::new(&s), Err(DspiError::InvalidArgument { .. })),
            "{s:?}"
        );
    }
}

#[test]
fn wp_p5ses_a_setup_past_the_capture_bound_is_refused_at_plan_time() {
    // The plan's largest default run: a stereo three-way as one group, 3
    // passes of 5.5 s sweeps with 1 s gaps, fits at 48 kHz ...
    let outputs: Vec<MeasureOutput> = (0..6)
        .map(|o| MeasureOutput {
            output: o,
            ..Default::default()
        })
        .collect();
    let s48 = MeasureSetup {
        outputs: outputs.clone(),
        ..Default::default()
    };
    assert!(Plan::new(&s48).unwrap().check_bounds(48_000).is_ok());
    // ... and is refused at 96 kHz.
    let s96 = MeasureSetup {
        outputs,
        capture_rate_hz: 96_000,
        ..Default::default()
    };
    let e = Plan::new(&s96).unwrap().check_bounds(48_000).unwrap_err();
    assert_eq!(e.key, "measure.too_long");
    // A rate the generator has no model for is refused too.
    assert_eq!(
        Plan::new(&s48)
            .unwrap()
            .check_bounds(32_000)
            .unwrap_err()
            .key,
        "measure.device_rate"
    );
}

// ── The ramp (S3) ──────────────────────────────────────────────────────

/// A step at `level` heard with an in-band SNR of `snr` dB (its in-band
/// level follows the SNR).
fn eval(level: f32, snr: f64) -> StepEval {
    StepEval {
        level_db: level,
        snr_db: snr,
        in_band_db: Some(snr),
        clipped: false,
        spl_db: None,
    }
}

#[test]
fn wp_p5ses_the_ramp_rules() {
    // Enough SNR at once.
    let mut r = Ramp::new(0, -50.0, -12.0, None, false);
    assert_eq!(r.judge(eval(-50.0, 40.0)), RampNext::Done(-50.0));
    // Up in 6 dB steps, never past the ceiling.
    let mut r = Ramp::new(0, -50.0, -12.0, None, false);
    let mut l = r.first();
    let mut seen = vec![l];
    loop {
        // Heard from -38 on, 6 dB a step, never 35 dB.
        match r.judge(eval(l, f64::from(l) + 46.0)) {
            RampNext::Step(n) => {
                l = n;
                seen.push(n);
            }
            RampNext::Done(d) => {
                assert_eq!(d, -12.0);
                break;
            }
            RampNext::Ask(i) => panic!("{i:?}"),
        }
    }
    assert_eq!(
        seen,
        vec![-50.0, -44.0, -38.0, -32.0, -26.0, -20.0, -14.0, -12.0]
    );
    // A clip ends it 12 dB below the step.
    let mut r = Ramp::new(0, -50.0, -12.0, None, false);
    let mut e = eval(-38.0, 20.0);
    e.clipped = true;
    assert_eq!(r.judge(e), RampNext::Done(-50.0));
    // The SPL cap: within a step of it stops here, past it one step down.
    let mut r = Ramp::new(0, -50.0, -12.0, Some(95.0), false);
    let mut e = eval(-30.0, 20.0);
    e.spl_db = Some(91.0);
    assert_eq!(r.judge(e), RampNext::Done(-30.0));
    e.spl_db = Some(96.0);
    assert_eq!(r.judge(e), RampNext::Done(-36.0));
    // Heard, then a step that does not raise the microphone: ask.
    let mut r = Ramp::new(3, -50.0, -12.0, None, false);
    assert_eq!(r.judge(eval(-50.0, 15.0)), RampNext::Step(-44.0));
    match r.judge(eval(-44.0, 16.0)) {
        RampNext::Ask(i) => {
            assert_eq!(i.key, "measure.mic_not_following");
            assert_eq!(i.output, Some(3));
        }
        o => panic!("{o:?}"),
    }
    // The in-band level decides, not the SNR (a sweep's SNR may stop at its
    // sidelobes while its level follows).
    let mut r = Ramp::new(0, -50.0, -12.0, None, false);
    let mut e = eval(-50.0, 30.0);
    e.in_band_db = Some(100.0);
    assert_eq!(r.judge(e), RampNext::Step(-44.0));
    let mut e = eval(-44.0, 31.0);
    e.in_band_db = Some(106.0);
    assert_eq!(r.judge(e), RampNext::Step(-38.0));
    // Not heard by start + 18 dB: ask.
    let mut r = Ramp::new(0, -50.0, -12.0, None, false);
    for l in [-50.0, -44.0, -38.0] {
        assert!(matches!(r.judge(eval(l, -10.0)), RampNext::Step(_)));
    }
    match r.judge(eval(-32.0, 5.9)) {
        RampNext::Ask(i) => {
            assert_eq!(i.key, "measure.mic_not_hearing");
            assert!(i.message.contains("-32 dBFS 5.9 dB"), "{}", i.message);
        }
        o => panic!("{o:?}"),
    }
}

/// WP P5RAMPFIX: no ramp ends on a step the microphone does not hear.
#[test]
fn wp_p5rampfix_a_ramp_never_ends_unheard() {
    // A ceiling below start + 18 dB, never heard: ask at the ceiling.
    let mut r = Ramp::new(0, -50.0, -40.0, None, false);
    assert_eq!(r.judge(eval(-50.0, 0.0)), RampNext::Step(-44.0));
    assert_eq!(r.judge(eval(-44.0, 1.0)), RampNext::Step(-40.0));
    match r.judge(eval(-40.0, 2.0)) {
        RampNext::Ask(i) => assert_eq!(i.key, "measure.mic_not_hearing"),
        o => panic!("{o:?}"),
    }
    // The start level only: its one step must be heard.
    let mut r = Ramp::new(1, -60.0, -60.0, None, true);
    match r.judge(eval(-60.0, 3.0)) {
        RampNext::Ask(i) => {
            assert_eq!(i.key, "measure.start_level_quiet");
            assert_eq!(i.output, Some(1));
            assert!(i.message.contains("ramp"), "{}", i.message);
        }
        o => panic!("{o:?}"),
    }
    let mut r = Ramp::new(1, -60.0, -60.0, None, true);
    assert_eq!(r.judge(eval(-60.0, 6.0)), RampNext::Done(-60.0));
    // Near the SPL cap but not heard in band: ask, not a run.
    let mut r = Ramp::new(0, -50.0, -12.0, Some(95.0), false);
    let mut e = eval(-30.0, 2.0);
    e.spl_db = Some(91.0);
    assert!(matches!(r.judge(e), RampNext::Ask(_)));
    // A step that could not be judged is not heard.
    let mut r = Ramp::new(0, -50.0, -50.0, None, false);
    let mut e = eval(-50.0, f64::NAN);
    e.in_band_db = None;
    assert!(matches!(r.judge(e), RampNext::Ask(_)));
}

/// A microphone that hears output `o` at `level - att(o)` dB (sine, peak).
fn hears(att: impl Fn(u8) -> f32 + 'static) -> impl Fn(u8, f32) -> f32 {
    move |o, level| 10f32.powf((level - att(o)) / 20.0)
}

#[test]
fn wp_p5ses_a_ramp_stops_at_the_ceiling_without_a_calibrated_mic() {
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    // Heard from -32 dBFS on, never 35 dB in band below the ceiling.
    let mut room = Room::new(-90.0, hears(|_| 70.0));
    drive(&mut s, &mut dev, &mut room, 30.0, |s, _| {
        !s.state().ramp_levels.is_empty()
    });
    let l = levels(&dev);
    assert_eq!(
        l,
        vec![-50.0, -44.0, -38.0, -32.0, -26.0, -20.0, -14.0, -12.0]
    );
    assert_eq!(s.state().ramp_levels[0].level_db, -12.0);
    assert!(l.iter().all(|l| *l <= -12.0));
}

fn mic_doc(spl_offset_db: f64) -> String {
    use dspi_measure::doc::{Curve, DocKind, Meta, MicCal, MicDoc, SCHEMA_VERSION};
    let grid = dspi_measure::types::LogGrid::STANDARD;
    let n = grid.freqs().unwrap().len();
    MicDoc {
        schema_version: SCHEMA_VERSION,
        meta: Meta::new(DocKind::Mic, "Test mic", "2026-11-16T00:00:00Z", "test"),
        mic: MicCal {
            id: String::new(),
            name: "Test mic".into(),
            serial: "T1".into(),
            vendor: String::new(),
            format: dspi_measure::cal::CalFormat::Rew,
            angle: 0,
            sensitivity_db: None,
            gain_db: None,
            spl_offset_db: Some(spl_offset_db),
            approximate: false,
            response: Curve::from_response(&dspi_measure::types::Response {
                grid,
                mag_db: vec![0.0; n],
                phase_deg: None,
                valid: None,
                spl: false,
            }),
            source_name: String::new(),
            source_hash: String::new(),
            note: String::new(),
            extra: Default::default(),
        },
        extra: Default::default(),
    }
    .to_text()
    .unwrap()
}

#[test]
fn wp_p5ses_a_ramp_stops_at_the_spl_cap_with_a_calibrated_mic() {
    let mut st = setup(MeasureRunKind::Walked, &[0]);
    st.mic_document = mic_doc(143.0);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    // The sweep at level - 20 dBFS peak: dB SPL = level + 123 (with the
    // noise's -57 dB beside it); noise high enough that the in-band SNR
    // (about 33 dB at -32) does not stop it first.
    let mut room = Room::new(-60.0, hears(|_| 20.0));
    drive(&mut s, &mut dev, &mut room, 30.0, |s, _| {
        !s.state().ramp_levels.is_empty()
    });
    // -50 .. -32: at -32 the next step would pass 95 dB SPL.
    assert_eq!(levels(&dev), vec![-50.0, -44.0, -38.0, -32.0]);
    assert_eq!(s.state().ramp_levels[0].level_db, -32.0);
}

#[test]
fn wp_p5ses_a_ramp_whose_mic_does_not_follow_stops_and_asks() {
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    // Heard at -50 and -44, then stuck.
    let mut room = Room::new(-70.0, move |_, l: f32| {
        10f32.powf((l.min(-44.0) - 36.0) / 20.0)
    });
    drive(&mut s, &mut dev, &mut room, 30.0, |_, _| false);
    assert_eq!(s.phase(), MeasurePhase::Failed);
    assert_eq!(s.state().issue.unwrap().key, "measure.mic_not_following");
    assert_eq!(levels(&dev), vec![-50.0, -44.0, -38.0], "nothing louder");
    // Not heard at all: asked at start + 18 dB.
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    let mut room = Room::new(-90.0, |_, _| 0.0);
    drive(&mut s, &mut dev, &mut room, 30.0, |_, _| false);
    assert_eq!(s.state().issue.unwrap().key, "measure.mic_not_hearing");
    assert_eq!(levels(&dev), vec![-50.0, -44.0, -38.0, -32.0]);
}

#[test]
fn wp_p5ses_a_walked_run_plays_at_the_minimum_of_its_ways_levels() {
    let st = setup(MeasureRunKind::Walked, &[0, 1, 2]);
    let mut s = session(&st, &[0, 1, 2]);
    let mut dev = Mock::new();
    // Output 1 is the loudest at the microphone: its ramp ends lowest.
    let mut room = Room::new(
        -90.0,
        hears(|o| match o {
            1 => 50.0,
            2 => 70.0,
            _ => 60.0,
        }),
    );
    drive(&mut s, &mut dev, &mut room, 60.0, |s, _| {
        s.phase() == MeasurePhase::Run
    });
    let st = s.state();
    let ramp: Vec<f32> = st.ramp_levels.iter().map(|l| l.level_db).collect();
    let min = ramp.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(ramp.iter().any(|l| *l > min), "the ways differ: {ramp:?}");
    let run = dev.starts.last().unwrap();
    assert_eq!(run.channel_mask, 0b111);
    assert_eq!(
        run.flags & crate::consts::SIGGEN_FLAG_WALK,
        crate::consts::SIGGEN_FLAG_WALK
    );
    assert_eq!(run.level_db, min);
    assert_eq!(st.run_level_db, Some(min));
    assert_eq!(run.repeat, 3);
}

#[test]
fn wp_p5ses_a_summed_run_plays_its_invert_mask_below_the_minimum() {
    let mut st = setup(MeasureRunKind::Summed, &[0, 1]);
    st.outputs[1].inverted = true;
    let mut s = session(&st, &[0, 1]);
    let mut dev = Mock::new();
    let mut room = Room::new(-90.0, hears(|_| 10.0));
    drive(&mut s, &mut dev, &mut room, 60.0, |s, _| {
        s.phase() == MeasurePhase::Run
    });
    let run = dev.starts.last().unwrap();
    assert_eq!(run.channel_mask, 0b11);
    assert_eq!(run.invert_mask, 0b10);
    assert_eq!(run.flags, 0);
    assert_eq!(run.repeat, 1);
    let min = s
        .state()
        .ramp_levels
        .iter()
        .map(|l| l.level_db)
        .fold(0.0, f32::min);
    assert!((run.level_db - (min - 20.0 * 2f32.log10())).abs() < 1e-4);
}

// ── The aborts (S4, S5) ────────────────────────────────────────────────

/// A session ramping output 0 (its generator playing), the mock, the room.
fn ramping() -> (Session, Mock, Room) {
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    let mut room = Room::new(-90.0, hears(|_| 60.0));
    drive(&mut s, &mut dev, &mut room, 10.0, |_, d| {
        d.playing.is_some()
    });
    assert_eq!(s.phase(), MeasurePhase::Ramp);
    (s, dev, room)
}

fn assert_aborted(s: &Session, dev: &Mock, starts: usize, key: &str) {
    assert_eq!(s.phase(), MeasurePhase::Aborted, "{:?}", s.state().issue);
    assert_eq!(s.state().issue.as_ref().unwrap().key, key);
    assert_eq!(dev.stops, 1, "exactly one STOP_NOW");
    assert_eq!(dev.starts.len(), starts, "nothing started after the abort");
}

#[test]
fn wp_p5ses_each_tapped_event_aborts_with_one_stop_now() {
    let cases = [
        (TapEvent::Volume, "measure.abort.volume"),
        (TapEvent::ForeignWrite, "measure.abort.foreign"),
        (TapEvent::PresetLoad, "measure.abort.preset"),
        (TapEvent::InputFormat, "measure.abort.input_format"),
        (TapEvent::InputStream, "measure.abort.input_stream"),
        (TapEvent::Clip, "measure.abort.clip"),
        (TapEvent::Lost, "measure.abort.foreign"),
        (
            TapEvent::Siggen {
                state: 0,
                reason: 3,
            },
            "measure.abort.generator",
        ),
        (
            TapEvent::Siggen {
                state: 0,
                reason: 1,
            },
            "measure.abort.generator",
        ),
    ];
    for (e, key) in cases {
        let (mut s, mut dev, mut room) = ramping();
        let n = dev.starts.len();
        dev.tap.push(e);
        drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
        assert_aborted(&s, &dev, n, key);
        // Ended for good: a later push or tick sends nothing.
        drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
        assert_eq!(dev.stops, 1);
    }
}

#[test]
fn wp_p5ses_a_foreign_start_while_idle_aborts() {
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    dev.tap.push(TapEvent::Siggen {
        state: 2,
        reason: 0,
    });
    let mut room = Room::new(-90.0, |_, _| 0.0);
    drive(&mut s, &mut dev, &mut room, 0.5, |_, _| false);
    assert_aborted(&s, &dev, 0, "measure.abort.generator");
}

#[test]
fn wp_p5ses_push_path_faults_abort() {
    // A gap in the frame indices.
    let (mut s, mut dev, mut room) = ramping();
    let n = dev.starts.len();
    let (first, samples) = room.block(&mut dev);
    s.push(&mut dev, first + 7, &samples, RATE, false, Instant::now());
    assert_aborted(&s, &dev, n, "measure.abort.frame_gap");
    // A repeated block.
    let (mut s, mut dev, mut room) = ramping();
    let (first, samples) = room.block(&mut dev);
    s.push(&mut dev, first, &samples, RATE, false, Instant::now());
    s.push(&mut dev, first, &samples, RATE, false, Instant::now());
    assert_aborted(&s, &dev, 1, "measure.abort.frame_gap");
    // A clip in the pre-roll.
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    s.push(&mut dev, 0, &[0.0, 0.95, 0.0], RATE, false, Instant::now());
    assert_aborted(&s, &dev, 0, "measure.abort.capture_clip");
    // The converter's clip flag.
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    s.push(&mut dev, 0, &[0.0; 16], RATE, true, Instant::now());
    assert_aborted(&s, &dev, 0, "measure.abort.capture_clip");
    // Another rate.
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    s.push(&mut dev, 0, &[0.0; 16], 44_100, false, Instant::now());
    assert_aborted(&s, &dev, 0, "measure.abort.capture_rate");
}

#[test]
fn wp_p5ses_a_clip_in_a_ramp_step_ends_the_ramp_12_db_below() {
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut s = session(&st, &[0]);
    let mut dev = Mock::new();
    // Quiet until -38, then the microphone clips.
    let mut room = Room::new(-90.0, |_, l: f32| {
        if l >= -38.0 {
            0.99
        } else {
            10f32.powf((l - 60.0) / 20.0)
        }
    });
    drive(&mut s, &mut dev, &mut room, 30.0, |s, _| {
        !s.state().ramp_levels.is_empty()
    });
    assert_eq!(levels(&dev), vec![-50.0, -44.0, -38.0], "{:?}", s.state());
    assert_eq!(
        s.state().ramp_levels.first().map(|l| l.level_db),
        Some(-50.0),
        "{:?}",
        s.state()
    );
    assert_eq!(dev.stops, 1, "the clipping step was stopped at once");
    assert!(s.active());
}

#[test]
fn wp_p5ses_meter_and_lock_faults_abort() {
    // An output at full scale.
    let (mut s, mut dev, mut room) = ramping();
    dev.meters.peaks[8] = 1.0;
    drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
    assert_aborted(&s, &dev, 1, "measure.abort.output_peak");
    // Programme audio on an input.
    let (mut s, mut dev, mut room) = ramping();
    dev.meters.peaks[1] = 0.01;
    drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
    assert_aborted(&s, &dev, 1, "measure.abort.programme");
    // An input stream starting.
    let (mut s, mut dev, mut room) = ramping();
    dev.meters.active_input_channels = 4;
    drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
    assert_aborted(&s, &dev, 1, "measure.abort.input_stream");
    // The lock lost.
    let (mut s, mut dev, mut room) = ramping();
    dev.lock_ok = false;
    drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
    assert_aborted(&s, &dev, 1, "measure.abort.lock");
    // A limiter acting on a measured output.
    let st = setup(MeasureRunKind::Walked, &[0]);
    let mut c = ctx(&[0]);
    c.limiter_outputs = vec![0];
    let mut s = Session::new(Plan::new(&st).unwrap(), c, Instant::now());
    let mut dev = Mock::new();
    let mut room = Room::new(-90.0, hears(|_| 60.0));
    drive(&mut s, &mut dev, &mut room, 0.6, |_, _| false);
    assert!(s.active());
    dev.gr[0] = 2.0;
    drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
    assert_aborted(&s, &dev, 0, "measure.abort.limiter");
}

#[test]
fn wp_p5ses_an_overrun_aborts() {
    let (mut s, mut dev, _room) = ramping();
    // Stop the device's time: the step never ends.
    dev.playing = Some((dev.starts[0].clone(), u64::MAX));
    let late = Instant::now() + Duration::from_secs(600);
    s.tick(&mut dev, late);
    assert_aborted(&s, &dev, 1, "measure.abort.overrun");
}

#[test]
fn wp_p5ses_a_cycle_identity_mismatch_aborts() {
    let st = setup(MeasureRunKind::Walked, &[0, 1, 2]);
    let mut s = session(&st, &[0, 1, 2]);
    let mut dev = Mock::new();
    let mut room = Room::new(-90.0, hears(|_| 10.0));
    drive(&mut s, &mut dev, &mut room, 60.0, |s, _| {
        s.phase() == MeasurePhase::Run
    });
    let n = dev.starts.len();
    // Right after the start the generator says it is on the third output.
    dev.status = Some(SiggenStatus {
        state: 2,
        active_channel: 2,
        cycles_done: 2,
        ..SiggenStatus::default()
    });
    drive(&mut s, &mut dev, &mut room, 1.0, |_, _| false);
    assert_aborted(&s, &dev, n, "measure.abort.cycle_identity");
    // The right output at the start is not a mismatch.
    let mut s = session(&st, &[0, 1, 2]);
    let mut dev = Mock::new();
    drive(&mut s, &mut dev, &mut room, 60.0, |s, _| {
        s.phase() == MeasurePhase::Run
    });
    dev.status = Some(SiggenStatus {
        state: 2,
        active_channel: 0,
        cycles_done: 0,
        ..SiggenStatus::default()
    });
    drive(&mut s, &mut dev, &mut room, 0.6, |_, _| false);
    assert_eq!(s.phase(), MeasurePhase::Run);
}

#[test]
fn wp_p5ses_the_users_stop_sends_one_stop_now() {
    let (mut s, mut dev, _) = ramping();
    assert!(s.stop_by_user(&mut dev));
    assert!(!s.stop_by_user(&mut dev));
    assert_aborted(&s, &dev, 1, "measure.stopped");
    assert!(s.release_pending());
}

// ── The notification tap ──────────────────────────────────────────────

#[test]
fn wp_p5ses_the_tap_sees_only_what_it_must() {
    use dspi_session::notify::{Event, Notification, Source};
    let note = |event: Event| Notification {
        seq: 0,
        event,
        lost: false,
        origin: None,
        received: Instant::now(),
    };
    let mut tap = Tap::default();
    // Off: nothing.
    note_notification(&mut tap, &note(Event::PresetLoaded { slot: 1 }), false);
    assert!(tap.drain().is_empty());
    tap.set(true);
    let uv = dspi_proto::generated::OFF_USER_VOLUME as u16;
    let eq = dspi_proto::generated::OFF_EQ as u16;
    let changed = |offset: u16, source| Event::ParamChanged {
        offset,
        source,
        bytes: vec![0; 4],
    };
    // Our own volume echo still counts; our own EQ echo does not.
    note_notification(&mut tap, &note(changed(uv, Source::HostSet)), true);
    note_notification(&mut tap, &note(changed(eq, Source::HostSet)), true);
    note_notification(&mut tap, &note(changed(eq, Source::Gpio)), false);
    note_notification(&mut tap, &note(Event::InputFormat { channels: 2 }), false);
    note_notification(&mut tap, &note(Event::Idle), false);
    assert_eq!(
        tap.drain(),
        vec![
            TapEvent::Volume,
            TapEvent::ForeignWrite,
            TapEvent::InputFormat
        ]
    );
    for _ in 0..300 {
        note_notification(&mut tap, &note(Event::InputFormat { channels: 2 }), false);
    }
    assert_eq!(tap.drain().last(), Some(&TapEvent::Lost));
}

// ── S7: every configuration finite, never RAW, within the cap ─────────

fn kind_of(k: u8) -> MeasureRunKind {
    match k % 4 {
        0 => MeasureRunKind::Walked,
        1 => MeasureRunKind::Summed,
        2 => MeasureRunKind::LiveSweep,
        _ => MeasureRunKind::Pink,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn proptest_wp_p5ses_no_configuration_is_raw_infinite_or_too_loud(
        kind in 0u8..4,
        n in 1usize..4,
        headphones in any::<bool>(),
        sweep_ms in prop_oneof![Just(0u32), 1_000u32..3_000],
        gap_ms in prop_oneof![Just(0u32), 300u32..2_000],
        passes in 0u8..9,
        pink_ms in prop_oneof![Just(0u32), 1_000u32..61_000],
        ceiling in proptest::option::of(-130.0f32..0.0),
        att in proptest::collection::vec(-20.0f32..80.0, 3),
        noise in -100.0f32..-40.0,
    ) {
        let kind = kind_of(kind);
        let n = match kind {
            MeasureRunKind::LiveSweep | MeasureRunKind::Pink => 1,
            MeasureRunKind::Summed => n.max(2),
            MeasureRunKind::Walked => n,
        };
        let outs: Vec<u8> = (0..n as u8).collect();
        let mut st = setup(kind, &outs);
        st.sweep_ms = sweep_ms;
        st.gap_ms = gap_ms;
        st.passes = passes;
        st.pink_ms = pink_ms;
        st.ceiling_db = ceiling;
        if headphones {
            st.placement = MeasurePlacement::Headphones;
        }
        let Ok(plan) = Plan::new(&st) else {
            return Ok(());
        };
        let cap = plan.ceiling_db;
        let hard = if headphones { HARD_CAP_HEADPHONES_DB } else { HARD_CAP_SPEAKERS_DB };
        prop_assert!(cap <= hard);
        let mut s = Session::new(plan, ctx(&outs), Instant::now());
        let mut dev = Mock::new();
        let att2 = att.clone();
        let mut room = Room::new(noise, move |o, l: f32| {
            10f32.powf((l - att2[usize::from(o) % 3]) / 20.0).min(0.99)
        });
        // Until the run starts (its analysis is not this test's).
        drive(&mut s, &mut dev, &mut room, 150.0, |s, _| s.phase() == MeasurePhase::Run);
        s.stop_by_user(&mut dev);
        for c in &dev.starts {
            prop_assert!(check_config(c).is_ok(), "{:?}", c);
            prop_assert_eq!(c.flags & SIGGEN_FLAG_RAW, 0);
            prop_assert!(c.repeat >= 1);
            if c.signal_type == super::preflight::SIG_PINK {
                prop_assert!(c.duration_ms >= 1 && c.duration_ms <= 60_000);
            } else {
                prop_assert!(c.duration_ms >= 1_000 && c.duration_ms <= 15_000 && c.repeat <= 8);
            }
            prop_assert!(c.level_db <= cap && c.level_db >= -120.0, "{} > {}", c.level_db, cap);
        }
    }

    #[test]
    fn proptest_wp_p5ses_the_push_path_never_panics(
        blocks in proptest::collection::vec(
            (0u64..10_000, proptest::collection::vec(proptest::num::f32::ANY, 0..64), any::<bool>()),
            1..20,
        ),
    ) {
        let st = setup(MeasureRunKind::Walked, &[0]);
        let mut s = session(&st, &[0]);
        let mut dev = Mock::new();
        for (first, samples, clipped) in blocks {
            s.push(&mut dev, first, &samples, RATE, clipped, Instant::now());
            s.tick(&mut dev, Instant::now());
        }
        prop_assert!(dev.stops <= 1);
    }
}

#[test]
fn wp_p5ses_check_config_refuses_what_the_rules_forbid() {
    let ok = SiggenConfig {
        signal_type: super::preflight::SIG_LOG_SWEEP,
        channel_mask: 1,
        invert_mask: 0,
        flags: 0,
        level_db: -40.0,
        duration_ms: 1_000,
        repeat: 1,
        gap_ms: 300,
        p: vec![20.0, 20_000.0, 0.0, 0.0],
    };
    assert!(check_config(&ok).is_ok());
    let bad = [
        SiggenConfig {
            flags: SIGGEN_FLAG_RAW,
            ..ok.clone()
        },
        SiggenConfig {
            repeat: 0,
            ..ok.clone()
        },
        SiggenConfig {
            level_db: -5.0,
            ..ok.clone()
        },
        SiggenConfig {
            level_db: f32::NAN,
            ..ok.clone()
        },
        SiggenConfig {
            signal_type: 0,
            ..ok.clone()
        },
        SiggenConfig {
            signal_type: super::preflight::SIG_PINK,
            duration_ms: 0,
            ..ok.clone()
        },
        SiggenConfig {
            signal_type: super::preflight::SIG_PINK,
            duration_ms: 60_001,
            ..ok.clone()
        },
    ];
    for c in bad {
        assert!(check_config(&c).is_err(), "{c:?}");
    }
}

#[test]
fn wp_p5ses_utc_text_is_iso() {
    assert_eq!(super::runs::utc_text(0), "1970-01-01T00:00:00Z");
    assert_eq!(super::runs::utc_text(1_794_869_295), "2026-11-16T22:48:15Z");
}

#[test]
fn wp_p5ses_every_issue_key_has_a_notice_row() {
    let sources = [
        include_str!("../measure.rs"),
        include_str!("plan.rs"),
        include_str!("preflight.rs"),
        include_str!("runs.rs"),
        include_str!("safety.rs"),
        include_str!("session.rs"),
    ];
    let mut keys = Vec::new();
    for src in sources {
        for part in src.split('"').skip(1).step_by(2) {
            if part.starts_with("measure.")
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
            {
                keys.push(part.to_owned());
            }
        }
    }
    keys.sort();
    keys.dedup();
    assert!(keys.len() > 40, "{keys:?}");
    for k in &keys {
        assert!(
            NOTICE_KEYS.contains(&k.as_str()),
            "{k} is not in NOTICE_KEYS"
        );
    }
    for k in keys
        .iter()
        .chain(&["read_only".to_owned(), "read_only.client".to_owned()])
    {
        assert!(crate::notice_text::has_row(k), "no notice row for {k}");
        assert_eq!(crate::notice_for_key(k.clone(), String::new()).key, *k);
    }
}

#[test]
fn wp_p5ses_start_level_only_never_plays_above_the_start() {
    for (placement, start) in [
        (MeasurePlacement::ListeningPosition, -50.0),
        (MeasurePlacement::Headphones, -60.0),
    ] {
        let mut st = setup(MeasureRunKind::Walked, &[0, 1]);
        st.placement = placement;
        st.start_level_only = true;
        let mut s = session(&st, &[0, 1]);
        let mut dev = Mock::new();
        // Heard at the start level, but a normal ramp would climb.
        let mut room = Room::new(-90.0, hears(|_| 40.0));
        drive(&mut s, &mut dev, &mut room, 60.0, |s, _| {
            s.phase() == MeasurePhase::Run
        });
        assert_eq!(s.phase(), MeasurePhase::Run);
        assert!(
            levels(&dev).iter().all(|l| *l == start),
            "{:?}",
            levels(&dev)
        );
        assert_eq!(dev.starts.len(), 3, "one step per output, then the run");
    }
}

// ── Review P5-B ─────────────────────────────────────────────────────────

/// Review P5-B: the start level only never raises a ceiling the setup names
/// below it, and still refuses a ceiling that is not one.
#[test]
fn wp_p5b_start_level_only_keeps_a_lower_ceiling() {
    let mut st = setup(MeasureRunKind::Walked, &[0, 1]);
    st.start_level_only = true;
    st.ceiling_db = Some(-70.0);
    let mut s = session(&st, &[0, 1]);
    let mut dev = Mock::new();
    let mut room = Room::new(-90.0, hears(|_| 20.0));
    drive(&mut s, &mut dev, &mut room, 60.0, |s, _| {
        s.phase() == MeasurePhase::Run
    });
    assert_eq!(s.phase(), MeasurePhase::Run);
    assert!(
        levels(&dev).iter().all(|l| *l == -70.0),
        "{:?}",
        levels(&dev)
    );
    for bad in [f32::NAN, 0.0, -3.0, f32::INFINITY] {
        st.ceiling_db = Some(bad);
        assert!(
            Plan::new(&st).is_err(),
            "a ceiling of {bad} with the start level only"
        );
    }
    // Above the start level it is the start level.
    st.ceiling_db = Some(-12.0);
    assert_eq!(Plan::new(&st).unwrap().ceiling_db, -50.0);
}

/// Review P5-B: a ramp step that ends by itself just before the push path's
/// STOP_NOW reaches it sends its own "completed" and nothing for the stop
/// (`siggen.c`: STOP_NOW on an idle generator announces nothing). That is the
/// step's end, not someone else's stop.
#[test]
fn wp_p5b_a_step_completing_under_its_stop_is_its_end() {
    use super::safety::{GenView, STOP_COMPLETED, STOP_HOST, judge_event};
    let mut g = GenView {
        playing: false,
        stopping: true,
        starts_pending: 0,
    };
    let done = TapEvent::Siggen {
        state: 0,
        reason: STOP_COMPLETED,
    };
    assert_eq!(judge_event(done, &mut g), Ok(true));
    assert!(!g.stopping);
    // Nothing is awaited any more: a stray echo is ignored, and a stop while
    // we play and did not ask is still foreign.
    assert_eq!(judge_event(done, &mut g), Ok(false));
    let mut g = GenView {
        playing: true,
        stopping: false,
        starts_pending: 0,
    };
    let host = TapEvent::Siggen {
        state: 0,
        reason: STOP_HOST,
    };
    assert_eq!(
        judge_event(host, &mut g).unwrap_err().key,
        "measure.abort.generator"
    );
}
