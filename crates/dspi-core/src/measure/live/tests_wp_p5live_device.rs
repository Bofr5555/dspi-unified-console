//! WP P5LIVE tests on the demo device: the live view through `DspiCore`, with
//! P5SIM's world rendered from each generator configuration the live view's
//! runs staged (read back from the fake device) and pushed as capture with
//! frame indices, paced by the clock as a real microphone is. The world is
//! rendered through the device's chain **as it is at each start**, so a
//! change the live view writes is heard by the next run. Every device write
//! is recorded.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_measure::sim::{
    Burst, Capture, ProtoModel, SimFilter, Stimulus as SimStimulus, World, render,
};
use dspi_measure::stimulus::{LogSweep, SweepConfig, peak_linear};
use dspi_measure::types::{ChannelChain, SectionModel};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

use super::super::preflight;
use super::*;
use crate::presets::Holder;

const RATE: u32 = 48_000;

#[derive(Debug, Clone, PartialEq)]
struct Req {
    op: u8,
    value: u16,
    data: Vec<u8>,
}

/// The demo device, recording every write.
struct Rec {
    dev: FakeDevice,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Transport for Rec {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if o == op::REQ_SIGGEN_CONTROL {
            self.log.lock().unwrap().push(Req {
                op: o,
                value: v,
                data: Vec::new(),
            });
        }
        self.dev.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.dev.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.log.lock().unwrap().push(Req {
            op: o,
            value: v,
            data: d.to_vec(),
        });
        self.dev.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

struct Rig {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    log: Arc<Mutex<Vec<Req>>>,
}

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The demo device behind a recorder, ready to measure (as P5SES's rig).
fn rig() -> Rig {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec {
        dev,
        log: Arc::clone(&log),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(rec), "P5LIVE-DEMO".into(), 0)
        .unwrap();
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
    for o in 0..s.num_output_channels {
        if s.limiter_enabled
            .get(usize::from(o))
            .copied()
            .unwrap_or(false)
        {
            core.set_limiter_param(o, 0, 0.0).unwrap();
        }
    }
    remote.measure_set_peaks(-200.0, &[], 2);
    settle(&core);
    log.lock().unwrap().clear();
    Rig { core, remote, log }
}

/// One flat output in a free field: the driver at -6 dB, a microphone 80 ppm
/// fast with 50 ms of latency, noise at -90 dBFS.
fn free_world() -> World {
    let mut w = World::flat(1, RATE);
    w.noise_dbfs = Some(-90.0);
    w.clock.ppm = 80.0;
    w.clock.latency_ms = 50.0;
    w.clock.block_frames = 960;
    w.clock.first_frame = 123_456;
    w.outputs[0].driver.sensitivity_db = -6.0;
    w
}

/// A short live sweep of output 0.
fn sweep_setup() -> MeasureLiveSetup {
    MeasureLiveSetup {
        output: 0,
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureLiveSetup::default()
    }
}

/// The microphone: pushes what the clock says is due, rendering each
/// generator start through the device's chain at that moment, and ticks the
/// live view.
struct Mic {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    world: World,
    t0: std::time::Instant,
    first: u64,
    next: u64,
    seg: Option<(Capture, usize)>,
    seen: (u32, u32),
    seed: u64,
}

impl Mic {
    fn new(r: &Rig, world: World) -> Mic {
        Mic {
            core: Arc::clone(&r.core),
            remote: r.remote.clone(),
            first: world.clock.first_frame,
            next: world.clock.first_frame,
            world,
            t0: std::time::Instant::now(),
            seg: None,
            seen: (0, 0),
            seed: 0x2545_F491_4F6C_DD1D,
        }
    }

    fn chains(&self) -> Vec<ChannelChain> {
        let s = self.core.get_state();
        (0..self.world.outputs.len() as u8)
            .map(|o| preflight::chain_of(&s, o))
            .collect()
    }

    fn noise_block(&mut self, n: usize) -> Vec<f32> {
        let amp = self
            .world
            .noise_dbfs
            .map_or(0.0, |db| 10f32.powf(db as f32 / 20.0) * 3f32.sqrt());
        (0..n)
            .map(|_| {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                amp * (((self.seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0)
            })
            .collect()
    }

    fn render(&mut self, at: u64) {
        let cfg = self.remote.measure_generator_config().unwrap();
        let amp = peak_linear(cfg.level_db);
        let chains = self.chains();
        let mut w = self.world.clone();
        w.clock.first_frame = at;
        let out = (0..16).find(|b| cfg.channel_mask & (1 << b) != 0).unwrap();
        let capture = if cfg.signal_type == preflight::SIG_PINK {
            let n = (u64::from(cfg.duration_ms) * u64::from(RATE) / 1000) as usize;
            let pink = pink_noise(n, amp / 8.0, &mut self.seed);
            let stim = SimStimulus {
                bursts: vec![Burst {
                    output: out,
                    start_frame: 0,
                    samples: &pink,
                    gain: 1.0,
                }],
                frames: (n + RATE as usize / 2) as u64,
            };
            render(&w, &stim, &chains, &ProtoModel, RATE).unwrap()
        } else {
            let sc = SweepConfig {
                f1_hz: cfg.p1,
                f2_hz: cfg.p2,
                duration_ms: cfg.duration_ms,
                gap_ms: cfg.gap_ms,
            };
            let sweep = LogSweep::new(RATE, &sc).unwrap();
            let first: Vec<f32> = sweep
                .render_cycle(true)
                .iter()
                .map(|v| *v as f32 * amp)
                .collect();
            let stim = SimStimulus::summed(
                &first,
                &[(out, false)],
                0,
                u64::from(sweep.gap_samples) + 4800,
            )
            .unwrap();
            render(&w, &stim, &chains, &ProtoModel, RATE).unwrap()
        };
        self.seg = Some((capture, 0));
    }

    /// Push what is due, then tick the live view.
    fn step(&mut self) -> MeasureLiveState {
        let st = self.core.measure_live_state();
        let key = (st.runs, st.session.generator_starts);
        if st.session.generator_starts > 0 && key != self.seen {
            self.seen = key;
            self.render(st.session.generator_start_frame.unwrap());
        }
        let due = self.first + (self.t0.elapsed().as_secs_f64() * f64::from(RATE)) as u64;
        while self.next < due {
            let block = match self.seg.as_mut() {
                Some((cap, i)) if *i < cap.blocks.len() => {
                    let b = cap.blocks[*i];
                    *i += 1;
                    Some((b.first_frame, cap.block_samples(&b).to_vec(), b.clipped))
                }
                _ => None,
            };
            let (first, samples, clipped) = match block {
                Some(b) => b,
                None => {
                    self.seg = None;
                    (self.next, self.noise_block(960), false)
                }
            };
            self.next = first + samples.len() as u64;
            self.core
                .measure_capture_push(first, samples, RATE, clipped)
                .unwrap();
        }
        self.core.measure_live_tick()
    }

    /// Run until `until` holds or the live view ends, at most `secs`.
    fn run(
        &mut self,
        secs: f64,
        mut until: impl FnMut(&MeasureLiveState) -> bool,
    ) -> MeasureLiveState {
        let end = std::time::Instant::now() + Duration::from_secs_f64(secs);
        loop {
            let st = self.step();
            if !st.active || until(&st) || std::time::Instant::now() > end {
                return st;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Pink noise of `n` samples about `amp` peak (Paul Kellet's filter).
fn pink_noise(n: usize, amp: f32, seed: &mut u64) -> Vec<f32> {
    let mut b = [0f32; 7];
    (0..n)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            let w = ((*seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
            b[0] = 0.99886 * b[0] + w * 0.055_517_9;
            b[1] = 0.99332 * b[1] + w * 0.075_075_9;
            b[2] = 0.96900 * b[2] + w * 0.153_852;
            b[3] = 0.86650 * b[3] + w * 0.310_485_6;
            b[4] = 0.55000 * b[4] + w * 0.532_952_2;
            b[5] = -0.7616 * b[5] - w * 0.016_898;
            let p = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
            b[6] = w * 0.115_926;
            amp * p / 3.5
        })
        .collect()
}

/// The world's truth for output 0 at `f` through `chain` (a flat driver, an
/// optional breakup peak), dB re a 0 dBFS stimulus.
fn truth_db(world: &World, chain: &ChannelChain, f: f64) -> f64 {
    let d = &world.outputs[0].driver;
    let mut db = f64::from(chain.gain_db) + d.sensitivity_db + world.outputs[0].gain_db;
    for band in chain.peq.iter().chain(&chain.crossover) {
        for c in ProtoModel.sections(band) {
            db += biquad_db(&c, f, f64::from(RATE));
        }
    }
    if let Some(SimFilter::Peak { freq, q, gain_db }) = d.breakup {
        db += rbj_peak_db(freq, q, gain_db, f, f64::from(RATE));
    }
    db
}

fn rbj_peak_db(f0: f64, q: f64, gain_db: f64, f: f64, fs: f64) -> f64 {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = std::f64::consts::TAU * f0 / fs;
    let alpha = w0.sin() / (2.0 * q);
    let a0 = 1.0 + alpha / a;
    let c = dspi_measure::types::Coeffs {
        b0: (1.0 + alpha * a) / a0,
        b1: -2.0 * w0.cos() / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: -2.0 * w0.cos() / a0,
        a2: (1.0 - alpha / a) / a0,
    };
    biquad_db(&c, f, fs)
}

/// The points of `c` in `lo..=hi` Hz that are trusted, as (f, dB).
fn points(c: &MeasureLiveCurve, lo: f64, hi: f64) -> Vec<(f64, f64)> {
    c.freqs_hz
        .iter()
        .zip(&c.mag_db)
        .zip(&c.valid)
        .filter(|((f, _), v)| **v && **f >= lo && **f <= hi)
        .map(|((f, m), _)| (*f, *m))
        .collect()
}

/// `c` at the grid point nearest `f`.
fn at(c: &MeasureLiveCurve, f: f64) -> f64 {
    let i = c
        .freqs_hz
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 / f).ln().abs().total_cmp(&(b.1 / f).ln().abs()))
        .unwrap()
        .0;
    c.mag_db[i]
}

/// The first PEQ band of output 0 that is free (flat or bypassed).
fn free_band(r: &Rig) -> u8 {
    let s = r.core.get_state();
    let ch = usize::from(s.num_input_channels);
    s.filters[ch]
        .iter()
        .position(|b| b.filter_type == 0 || b.bypass)
        .unwrap() as u8
}

fn configs(r: &Rig) -> Vec<dspi_proto::packets::SiggenConfig> {
    r.log
        .lock()
        .unwrap()
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
        .map(|q| dspi_proto::packets::SiggenConfig::decode(&q.data).unwrap())
        .collect()
}

/// Every generator configuration the live view sent is finite, never `RAW`,
/// a pink run at most 60 s.
fn assert_configs_safe(r: &Rig) {
    for c in configs(r) {
        assert_eq!(c.flags & crate::consts::SIGGEN_FLAG_RAW, 0);
        assert!(c.repeat_count >= 1);
        assert!(c.duration_ms >= 1 && c.duration_ms <= 60_000, "{c:?}");
        assert!(c.level_db <= -6.0, "{c:?}");
    }
}

/// Writes other than the generator's since `from`.
fn other_writes(r: &Rig, from: usize) -> usize {
    r.log
        .lock()
        .unwrap()
        .iter()
        .skip(from)
        .filter(|q| q.op != op::REQ_SIGGEN_SET_CONFIG && q.op != op::REQ_SIGGEN_CONTROL)
        .count()
}

fn first_frame(mic: &mut Mic, core: &DspiCore) -> MeasureLiveFrame {
    let st = mic.run(40.0, |s| s.seq >= 1);
    assert!(st.active && st.seq >= 1, "{st:#?}");
    core.measure_live_frame()
}

#[test]
fn wp_p5live_device_a_frame_per_sweep_with_the_lock_held_throughout() {
    let r = rig();
    let world = free_world();
    let mut mic = Mic::new(&r, world.clone());
    let st = r.core.measure_live_start(sweep_setup()).unwrap();
    assert!(st.active && st.session.active, "{st:#?}");
    assert_eq!(st.runs, 1);
    // A measurement and a second live view are busy meanwhile.
    assert!(
        r.core
            .measure_start(MeasureSetup {
                outputs: vec![MeasureOutput::default()],
                ..MeasureSetup::default()
            })
            .is_err()
    );
    assert!(r.core.measure_live_start(sweep_setup()).is_err());
    let mut seqs = Vec::new();
    let st = mic.run(60.0, |s| {
        // Held between runs too.
        assert_eq!(
            r.core.held_lock_holder(),
            Some(Holder::Measure),
            "the lock is held while live: {s:#?}"
        );
        if seqs.last() != Some(&s.seq) {
            seqs.push(s.seq);
        }
        s.seq >= 3
    });
    assert!(st.active && st.seq >= 3, "{st:#?}");
    assert_eq!(seqs, vec![0, 1, 2, 3], "one frame per sweep");
    assert!(st.runs >= 3);
    let frame = r.core.measure_live_frame();
    assert_eq!(frame.mode, MeasureLiveMode::Sweep);
    assert!(frame.rta.is_none() && frame.predicted.is_none());
    let m = frame.measured.clone().unwrap();
    let chain = preflight::chain_of(&r.core.get_state(), 0);
    let pts = points(&m, 200.0, 5_000.0);
    assert!(pts.len() > 100, "{} trusted points", pts.len());
    for (f, db) in &pts {
        let e = db - truth_db(&world, &chain, *f);
        assert!(e.abs() < 1.0, "{f:.0} Hz: {e:.2} dB off the truth");
    }
    // The target is levelled to the curve.
    let t = frame.target.unwrap();
    assert!((at(&t, 1_000.0) - at(&m, 1_000.0)).abs() < 3.0);
    // A later run's ramp starts at the level the run before settled on, and
    // nothing is ever louder than it.
    let level = st.level_db.unwrap();
    let cfgs = configs(&r);
    assert!(cfgs.iter().all(|c| c.level_db <= level + 1e-3), "{cfgs:?}");
    assert!(cfgs.len() >= 6, "a ramp step and a sweep per run: {cfgs:?}");
    assert_configs_safe(&r);
    // Stop: the lock given back, nothing plays.
    let st = r.core.measure_live_stop();
    assert!(!st.active);
    assert_eq!(st.issue.unwrap().key, "measure.stopped");
    assert_eq!(r.core.held_lock_holder(), None);
    assert!(!r.core.measure_state().active);
    let n = r.log.lock().unwrap().len();
    mic.run(1.0, |_| false);
    assert_eq!(r.log.lock().unwrap().len(), n, "nothing after the stop");
    // Only the generator was written.
    assert_eq!(other_writes(&r, 0), 0);
}

#[test]
fn wp_p5live_device_an_edit_is_predicted_at_once_and_measured_after_the_next_sweep() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world());
    r.core.measure_live_start(sweep_setup()).unwrap();
    let frame = first_frame(&mut mic, &r.core);
    let measured = frame.measured.unwrap();
    let band = free_band(&r);
    let n = r.log.lock().unwrap().len();
    let p = r
        .core
        .measure_live_predict(
            0,
            vec![MeasureLiveBand {
                band,
                filter_type: 1,
                freq: 2_000.0,
                q: 2.0,
                gain: -6.0,
                ..MeasureLiveBand::default()
            }],
        )
        .unwrap();
    assert_eq!(p.adjust, MeasureLiveAdjust::Edit);
    assert_eq!(p.changes.len(), 1);
    assert!(
        p.changes[0].words.contains("peak -6.0 dB at 2.00 kHz"),
        "{}",
        p.changes[0].words
    );
    // At once: the predicted curve, nothing written.
    let f = r.core.measure_live_frame();
    let predicted = f.predicted.clone().unwrap();
    assert_eq!(f.pending.as_ref().unwrap().id, p.id);
    let drop_db = at(&measured, 2_000.0) - at(&predicted, 2_000.0);
    assert!((drop_db - 6.0).abs() < 0.3, "{drop_db:.2} dB");
    assert_eq!(other_writes(&r, n), 0, "no write without the apply");
    // Applied: one undo step, the band written.
    let st = r.core.measure_live_apply(p.id).unwrap();
    assert!(st.active && st.pending.is_none());
    let s = r.core.get_state();
    let written = s.filters[usize::from(s.num_input_channels)][usize::from(band)];
    assert_eq!((written.filter_type, written.gain), (1, -6.0));
    let steps = r
        .core
        .history(100)
        .iter()
        .filter(|h| h.label.as_deref() == Some("Live EQ change"))
        .count();
    assert_eq!(steps, 1);
    // The change shows as predicted until a sweep measures it.
    let seq = st.seq;
    assert!(r.core.measure_live_frame().predicted.is_some());
    let st = mic.run(40.0, |s| s.seq > seq);
    assert!(st.seq > seq, "{st:#?}");
    let f = r.core.measure_live_frame();
    assert!(f.predicted.is_none(), "measured now");
    let m = f.measured.unwrap();
    let pts = points(&m, 200.0, 5_000.0);
    assert!(pts.len() > 100);
    for (fq, db) in pts {
        let e = db - at(&predicted, fq);
        assert!(
            e.abs() < 1.0,
            "{fq:.0} Hz: measured {e:.2} dB off the prediction"
        );
    }
    // The same id again writes nothing.
    let n = r.log.lock().unwrap().len();
    assert!(r.core.measure_live_apply(p.id).is_err());
    assert_eq!(other_writes(&r, n), 0);
    r.core.measure_live_stop();
    assert_configs_safe(&r);
}

#[test]
fn wp_p5live_device_cut_this_peak_gives_one_band_and_one_undo_step() {
    let r = rig();
    let mut world = free_world();
    world.outputs[0].driver.breakup = Some(SimFilter::Peak {
        freq: 1_000.0,
        q: 4.0,
        gain_db: 6.0,
    });
    let mut mic = Mic::new(&r, world);
    r.core.measure_live_start(sweep_setup()).unwrap();
    let frame = first_frame(&mut mic, &r.core);
    let m = frame.measured.unwrap();
    let peak = at(&m, 1_000.0) - at(&m, 500.0);
    assert!(peak > 4.5, "the world's peak: {peak:.2} dB");
    let before = r.core.get_state();
    let n = r.log.lock().unwrap().len();
    let p = r.core.measure_live_cut_peak(0, 1_000.0).unwrap();
    assert!(p.issue.is_none(), "{p:#?}");
    assert_eq!(p.adjust, MeasureLiveAdjust::CutPeak);
    assert_eq!(p.changes.len(), 1, "{p:#?}");
    let c = p.changes[0].clone();
    assert_eq!(c.before.filter_type, 0, "a free band");
    assert_eq!(c.after.filter_type, 1);
    assert!(c.after.gain < -3.0 && c.after.gain >= -15.0, "{c:?}");
    assert!(
        (f64::from(c.after.freq) / 1_000.0).log2().abs() < 1.0 / 6.0,
        "{c:?}"
    );
    assert_eq!(p.gain_after_db, p.gain_before_db, "no gain change");
    assert!(p.predicted.is_some());
    assert_eq!(other_writes(&r, n), 0, "previewed only");
    // Applied as one undo step.
    r.core.measure_live_apply(p.id).unwrap();
    assert_eq!(p.label, "Cut the peak at 1.00 kHz");
    let groups: Vec<_> = r
        .core
        .history(100)
        .into_iter()
        .filter(|h| h.label.as_deref() == Some(p.label.as_str()))
        .collect();
    assert_eq!(groups.len(), 1, "{groups:#?}");
    // The next sweep measures the peak cut.
    let seq = r.core.measure_live_state().seq;
    let st = mic.run(40.0, |s| s.seq > seq);
    assert!(st.seq > seq);
    let m = r.core.measure_live_frame().measured.unwrap();
    let left = at(&m, 1_000.0) - at(&m, 500.0);
    assert!(left < 2.0, "{left:.2} dB of the peak left");
    // One undo puts the band back.
    r.core.measure_live_stop();
    r.core.undo().unwrap();
    let ch = usize::from(before.num_input_channels);
    assert_eq!(
        r.core.get_state().filters[ch][usize::from(c.band)],
        before.filters[ch][usize::from(c.band)]
    );
    assert_configs_safe(&r);
}

#[test]
fn wp_p5live_device_fit_this_range_is_previewed_then_written_as_one_step() {
    let r = rig();
    let mut world = free_world();
    world.outputs[0].driver.breakup = Some(SimFilter::Peak {
        freq: 800.0,
        q: 2.0,
        gain_db: 7.0,
    });
    let mut mic = Mic::new(&r, world);
    r.core.measure_live_start(sweep_setup()).unwrap();
    first_frame(&mut mic, &r.core);
    let before = r.core.get_state();
    let n = r.log.lock().unwrap().len();
    // The span is held to the trusted range: asked from 10 Hz, no band
    // lands below a third of an octave above the sweep's 20 Hz.
    let p = r.core.measure_live_fit_range(0, 10.0, 3_000.0).unwrap();
    assert!(p.issue.is_none(), "{p:#?}");
    assert_eq!(p.adjust, MeasureLiveAdjust::FitRange);
    assert!(!p.changes.is_empty());
    assert!(p.changes.iter().all(|c| c.after.freq >= 25.0), "{p:#?}");
    // The peak at 800 Hz is cut.
    assert!(
        p.changes
            .iter()
            .any(|c| c.after.gain < -4.0 && (f64::from(c.after.freq) / 800.0).log2().abs() < 0.25),
        "{p:#?}"
    );
    assert!(p.rms_after_db < p.rms_before_db);
    assert!(p.changes.iter().all(|c| c.after.gain <= 3.0 + 1e-3));
    assert_eq!(other_writes(&r, n), 0, "previewed only");
    r.core.measure_live_apply(p.id).unwrap();
    // Every band the proposal lists is written, and its gain.
    let s = r.core.get_state();
    let ch = usize::from(s.num_input_channels);
    for c in &p.changes {
        let w = s.filters[ch][usize::from(c.band)];
        assert_eq!(MeasureLiveBand::of(c.band, &w), c.after, "{c:?}");
    }
    assert_eq!(s.output_gain_db[0], p.gain_after_db);
    let steps = r
        .core
        .history(100)
        .iter()
        .filter(|h| h.label.as_deref() == Some(p.label.as_str()))
        .count();
    assert_eq!(steps, 1);
    r.core.measure_live_stop();
    r.core.undo().unwrap();
    let s = r.core.get_state();
    assert_eq!(
        s.filters[ch], before.filters[ch],
        "one undo puts every band back"
    );
    assert_eq!(s.output_gain_db[0], before.output_gain_db[0]);
}

#[test]
fn wp_p5live_device_pink_rta_reads_the_world_and_is_re_armed() {
    let r = rig();
    // A broad feature on the chain: the RTA must read its shape.
    let n_in = r.core.get_state().num_input_channels;
    let band = free_band(&r);
    r.core
        .set_filter(
            n_in,
            band,
            FilterParams {
                filter_type: 1,
                bypass: false,
                freq: 1_000.0,
                q: 0.7,
                gain: 6.0,
                qp: 0.0,
            },
        )
        .unwrap();
    settle(&r.core);
    r.log.lock().unwrap().clear();
    let world = free_world();
    let mut mic = Mic::new(&r, world.clone());
    let setup = MeasureLiveSetup {
        mode: MeasureLiveMode::PinkRta,
        pink_ms: 3_000,
        averaging: MeasureRtaAveraging::Infinite,
        ..sweep_setup()
    };
    r.core.measure_live_start(setup).unwrap();
    // Two pink runs at least: re-armed.
    let st = mic.run(60.0, |s| s.runs >= 3);
    assert!(st.active && st.runs >= 3, "{st:#?}");
    let frame = r.core.measure_live_frame();
    let rta = frame.rta.clone().unwrap();
    assert!(rta.frames > 10, "{} frames", rta.frames);
    let cur = rta.current.unwrap();
    let chain = preflight::chain_of(&r.core.get_state(), 0);
    // 1/3-octave band centres from 200 Hz to 5 kHz: the shape within 1 dB.
    let centres: Vec<f64> = (-7..=7)
        .map(|k| 1_000.0 * 2f64.powf(f64::from(k) / 3.0))
        .collect();
    let errs: Vec<f64> = centres
        .iter()
        .map(|f| at(&cur, *f) - truth_db(&world, &chain, *f))
        .collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    for (f, e) in centres.iter().zip(&errs) {
        assert!((e - mean).abs() < 1.0, "{f:.0} Hz: {:.2} dB", e - mean);
    }
    // The measured curve follows it, and the target is shown.
    assert!(frame.measured.is_some() && frame.target.is_some());
    // Hold and compare.
    r.core.measure_live_hold(true).unwrap();
    let f = r.core.measure_live_frame();
    let cmp = f.rta.unwrap().compare.unwrap();
    assert!(
        points(&cmp, 200.0, 5_000.0)
            .iter()
            .all(|(_, d)| d.abs() < 0.5)
    );
    r.core
        .measure_live_averaging(MeasureRtaAveraging::Fast)
        .unwrap();
    assert!(
        r.core.measure_live_frame().rta.unwrap().held.is_some(),
        "the hold is kept"
    );
    r.core.measure_live_stop();
    // Every pink run finite, at most 60 s; re-armed at its length.
    let pinks: Vec<_> = configs(&r)
        .into_iter()
        .filter(|c| c.signal_type == preflight::SIG_PINK && c.duration_ms == 3_000)
        .collect();
    assert!(pinks.len() >= 2, "{pinks:?}");
    assert_configs_safe(&r);
    assert_eq!(r.core.held_lock_holder(), None);
}

#[test]
fn wp_p5live_device_stop_stops_the_generator_and_ends_the_live_view() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world());
    r.core.measure_live_start(sweep_setup()).unwrap();
    let st = mic.run(20.0, |s| {
        s.session.phase == MeasurePhase::Run && s.session.level_db.is_some()
    });
    assert_eq!(st.session.phase, MeasurePhase::Run, "{st:#?}");
    let n = r.log.lock().unwrap().len();
    // The measurement's own Stop ends the live view.
    r.core.measure_stop();
    let log = r.log.lock().unwrap();
    let after: Vec<&Req> = log.iter().skip(n).collect();
    assert_eq!(
        after,
        vec![&Req {
            op: op::REQ_SIGGEN_CONTROL,
            value: crate::consts::SIGGEN_CTL_STOP_NOW,
            data: Vec::new(),
        }],
        "one STOP_NOW and nothing else"
    );
    drop(log);
    let st = r.core.measure_live_state();
    assert!(!st.active);
    assert_eq!(r.core.held_lock_holder(), None);
    // Ticking further starts nothing.
    let n = r.log.lock().unwrap().len();
    mic.run(1.5, |_| false);
    assert_eq!(r.log.lock().unwrap().len(), n);
    assert!(!r.core.measure_live_tick().active);
}

#[test]
fn wp_p5live_device_no_write_without_a_preview_of_the_chain_it_has() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world());
    // Read-only: no live view, nothing sent.
    r.core.set_client_read_only(true).unwrap();
    assert!(matches!(
        r.core.measure_live_start(sweep_setup()),
        Err(DspiError::ReadOnly)
    ));
    assert!(r.log.lock().unwrap().is_empty(), "nothing sent");
    r.core.set_client_read_only(false).unwrap();
    r.core.measure_live_start(sweep_setup()).unwrap();
    // Before a curve: nothing to cut, nothing pending.
    let p = r.core.measure_live_cut_peak(0, 1_000.0).unwrap();
    assert_eq!(p.issue.unwrap().key, NOTICE_NO_FRAME);
    assert!(p.changes.is_empty());
    assert!(r.core.measure_live_state().pending.is_none());
    // Another output than the live one.
    assert!(r.core.measure_live_cut_peak(1, 1_000.0).is_err());
    first_frame(&mut mic, &r.core);
    let band = free_band(&r);
    let edit = MeasureLiveBand {
        band,
        filter_type: 1,
        freq: 300.0,
        q: 1.0,
        gain: -3.0,
        ..MeasureLiveBand::default()
    };
    let p = r.core.measure_live_predict(0, vec![edit]).unwrap();
    // No pending change by that id: nothing written.
    let n = r.log.lock().unwrap().len();
    assert!(r.core.measure_live_apply(p.id + 1).is_err());
    assert_eq!(other_writes(&r, n), 0);
    // The band changed since the preview: refused, nothing written by it.
    let n_in = r.core.get_state().num_input_channels;
    r.core
        .set_filter(
            n_in,
            band,
            FilterParams {
                filter_type: 1,
                bypass: false,
                freq: 400.0,
                q: 1.0,
                gain: -2.0,
                qp: 0.0,
            },
        )
        .unwrap();
    let n = r.log.lock().unwrap().len();
    let e = r.core.measure_live_apply(p.id).unwrap_err();
    assert!(e.to_string().contains(NOTICE_CHAIN_CHANGED), "{e}");
    assert_eq!(other_writes(&r, n), 0);
    assert!(r.core.measure_live_state().pending.is_none());
    // Discard: nothing pending, nothing written.
    let p = r.core.measure_live_predict(0, vec![edit]).unwrap();
    assert!(p.issue.is_none());
    r.core.measure_live_discard();
    assert!(r.core.measure_live_state().pending.is_none());
    assert_eq!(other_writes(&r, n), 0);
    // A band past the PEQ, a value that is not a number: refused.
    assert!(
        r.core
            .measure_live_predict(0, vec![MeasureLiveBand { band: 200, ..edit }])
            .is_err()
    );
    assert!(
        r.core
            .measure_live_predict(
                0,
                vec![MeasureLiveBand {
                    gain: f32::NAN,
                    ..edit
                }]
            )
            .is_err()
    );
    r.core.measure_live_stop();
}
