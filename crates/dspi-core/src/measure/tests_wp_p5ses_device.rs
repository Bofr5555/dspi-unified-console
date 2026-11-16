//! WP P5SES tests on the demo device: the session through `DspiCore`, with
//! P5SIM's world rendered from the generator configuration the session staged
//! (read back from the fake device) and pushed as capture with frame indices,
//! paced by the clock as a real microphone is (the fake device's generator
//! runs on the same clock). Every device write is recorded ([`Rec`]), so an
//! abort is shown to send exactly one `STOP_NOW` and nothing else.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

use super::*;

const RATE: u32 = 48_000;

/// One request the core sent.
#[derive(Debug, Clone, PartialEq)]
struct Req {
    op: u8,
    value: u16,
    data: Vec<u8>,
}

/// What a test makes the transport do beyond the fake device.
#[derive(Default)]
struct Hooks {
    /// The limiter meter's reply, dB per output.
    gr: Option<Vec<f32>>,
    /// The generator's status says this output is playing.
    status_channel: Option<u8>,
}

/// The demo device, recording every write (OUT transfers and the generator's
/// control, a write-as-read).
struct Rec {
    dev: FakeDevice,
    log: Arc<Mutex<Vec<Req>>>,
    hooks: Arc<Mutex<Hooks>>,
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
        if o == op::REQ_LIMITER
            && v & 0xFF == dspi_proto::generated::limiter::LIMITER_GET_METER
            && let Some(gr) = self.hooks.lock().unwrap().gr.clone()
        {
            let mut b: Vec<u8> = gr
                .iter()
                .flat_map(|g| ((g * 100.0) as i16).to_le_bytes())
                .collect();
            b.resize(usize::from(l), 0);
            return Ok(b);
        }
        let mut r = self.dev.control_in(o, v, l)?;
        if o == op::REQ_SIGGEN_GET_STATUS
            && let Some(ch) = self.hooks.lock().unwrap().status_channel
            && let Some(b) = r.get_mut(3)
        {
            // `SiggenStatus`: version, state, signal type, active channel.
            *b = ch;
        }
        Ok(r)
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
    hooks: Arc<Mutex<Hooks>>,
}

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The demo device behind a recorder, made ready to measure: the volume at
/// -20 dB, processing the maths does not model off, silence on the inputs.
fn rig() -> Rig {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let log = Arc::new(Mutex::new(Vec::new()));
    let hooks = Arc::new(Mutex::new(Hooks::default()));
    let rec = Rec {
        dev,
        log: Arc::clone(&log),
        hooks: Arc::clone(&hooks),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(rec), "P5SES-DEMO".into(), 0)
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
    Rig {
        core,
        remote,
        log,
        hooks,
    }
}

fn outputs(list: &[(u8, MeasureRole)]) -> Vec<MeasureOutput> {
    list.iter()
        .map(|(o, r)| MeasureOutput {
            output: *o,
            role: *r,
            ..MeasureOutput::default()
        })
        .collect()
}

fn walked(list: &[(u8, MeasureRole)]) -> MeasureSetup {
    MeasureSetup {
        outputs: outputs(list),
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureSetup::default()
    }
}

fn keys(p: &MeasurePreflight) -> Vec<String> {
    p.blockers.iter().map(|b| b.key.clone()).collect()
}

#[test]
fn wp_p5ses_device_the_ready_demo_passes_preflight() {
    let r = rig();
    let p = r
        .core
        .measure_preflight(walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]))
        .unwrap();
    assert!(p.clear, "{p:#?} {:#?}", keys(&p));
    assert!(
        r.log.lock().unwrap().is_empty(),
        "the preflight writes nothing"
    );
    let _ = (&r.remote, &r.hooks);
}

// ── The simulated microphone ───────────────────────────────────────────

use dspi_measure::sim::{
    Burst, Capture, CaptureFault, ProtoModel, Stimulus as SimStimulus, World, render,
};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_measure::types::{ChannelChain, SectionModel};

/// A microphone in P5SIM's world, at the listening position: it pushes
/// capture as the clock passes (blocks of 20 ms), renders what the generator
/// plays from the configuration the session staged, and ticks the session.
struct Mic {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    world: World,
    chains: Vec<ChannelChain>,
    t0: std::time::Instant,
    first: u64,
    next: u64,
    seg: Option<(Capture, usize)>,
    seen: u32,
    /// Faults played into walked runs only.
    run_faults: Vec<CaptureFault>,
    seed: u64,
    /// Pushes so far.
    pushes: u64,
}

impl Mic {
    fn new(r: &Rig, world: World) -> Mic {
        let s = r.core.get_state();
        let chains = (0..world.outputs.len() as u8)
            .map(|o| preflight::chain_of(&s, o))
            .collect();
        Mic {
            core: Arc::clone(&r.core),
            remote: r.remote.clone(),
            first: world.clock.first_frame,
            next: world.clock.first_frame,
            world,
            chains,
            t0: std::time::Instant::now(),
            seg: None,
            seen: 0,
            run_faults: Vec::new(),
            seed: 0x2545_F491_4F6C_DD1D,
            pushes: 0,
        }
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

    /// What the generator plays for `cfg`, rendered from capture frame `at`.
    fn render(&mut self, at: u64) {
        let cfg = self.remote.measure_generator_config().unwrap();
        let amp = peak_linear(cfg.level_db);
        let mut w = self.world.clone();
        w.clock.first_frame = at;
        let walk = cfg.flags & crate::consts::SIGGEN_FLAG_WALK != 0;
        w.faults = if walk {
            self.run_faults.clone()
        } else {
            Vec::new()
        };
        let outs: Vec<usize> = (0..16)
            .filter(|b| cfg.channel_mask & (1 << b) != 0)
            .collect();
        let capture = if cfg.signal_type == super::preflight::SIG_PINK {
            let n = (u64::from(cfg.duration_ms) * u64::from(RATE) / 1000) as usize;
            let pink = pink_noise(n, amp / 8.0, &mut self.seed);
            let stim = SimStimulus {
                bursts: vec![Burst {
                    output: outs[0],
                    start_frame: 0,
                    samples: &pink,
                    gain: 1.0,
                }],
                frames: (n + RATE as usize / 2) as u64,
            };
            render(&w, &stim, &self.chains, &ProtoModel, RATE).unwrap()
        } else {
            let sc = SweepConfig {
                f1_hz: cfg.p1,
                f2_hz: cfg.p2,
                duration_ms: cfg.duration_ms,
                gap_ms: cfg.gap_ms,
            };
            let sweep = LogSweep::new(RATE, &sc).unwrap();
            let scaled = |first: bool| -> Vec<f32> {
                sweep
                    .render_cycle(first)
                    .iter()
                    .map(|v| *v as f32 * amp)
                    .collect()
            };
            let (first, regular) = (scaled(true), scaled(false));
            if walk {
                let schedule =
                    WalkSchedule::new(sweep, cfg.channel_mask, cfg.repeat_count).unwrap();
                let bursts = schedule
                    .cycles()
                    .iter()
                    .map(|c| Burst {
                        output: usize::from(c.output),
                        start_frame: c.start,
                        samples: if c.index == 0 { &first } else { &regular },
                        gain: 1.0,
                    })
                    .collect();
                let stim = SimStimulus {
                    bursts,
                    frames: schedule.total_samples() + u64::from(sweep.gap_samples) + 4800,
                };
                render(&w, &stim, &self.chains, &ProtoModel, RATE).unwrap()
            } else {
                let o: Vec<(usize, bool)> = outs
                    .iter()
                    .map(|o| (*o, cfg.invert_mask & (1 << o) != 0))
                    .collect();
                let stim = SimStimulus::summed(&first, &o, 0, u64::from(sweep.gap_samples) + 4800)
                    .unwrap();
                render(&w, &stim, &self.chains, &ProtoModel, RATE).unwrap()
            }
        };
        self.seg = Some((capture, 0));
    }

    /// Push what the clock says is due, then tick.
    fn step(&mut self) -> MeasureState {
        let st = self.core.measure_state();
        if st.generator_starts > self.seen {
            self.seen = st.generator_starts;
            self.render(st.generator_start_frame.unwrap());
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
            self.pushes += 1;
            self.core
                .measure_capture_push(first, samples, RATE, clipped)
                .unwrap();
        }
        self.core.measure_tick()
    }

    /// Run until the session ends or `until` holds, at most `secs`.
    fn run(&mut self, secs: f64, mut until: impl FnMut(&MeasureState) -> bool) -> MeasureState {
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

/// `n` flat outputs in a free field: drivers at -6 dB, the path delays
/// given, a microphone 80 ppm fast with 50 ms of latency, noise at -90 dBFS.
fn free_world(delays_ms: &[f64]) -> World {
    let mut w = World::flat(delays_ms.len(), RATE);
    w.noise_dbfs = Some(-90.0);
    w.clock.ppm = 80.0;
    w.clock.latency_ms = 50.0;
    w.clock.block_frames = 960;
    w.clock.first_frame = 123_456;
    for (p, d) in w.outputs.iter_mut().zip(delays_ms) {
        p.driver.sensitivity_db = -6.0;
        p.delay_ms = *d;
    }
    w
}

/// A biquad's magnitude at `f`, dB.
fn biquad_db(c: &dspi_measure::types::Coeffs, f: f64, fs: f64) -> f64 {
    let w = std::f64::consts::TAU * f / fs;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let (nr, ni) = (c.b0 + c.b1 * c1 + c.b2 * c2, -(c.b1 * s1 + c.b2 * s2));
    let (dr, di) = (1.0 + c.a1 * c1 + c.a2 * c2, -(c.a1 * s1 + c.a2 * s2));
    10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
}

/// The world's truth for output `o` at `f` (flat drivers): the chain
/// through the model, the driver's sensitivity and the path, dB.
fn truth_db(world: &World, chain: &ChannelChain, o: usize, f: f64) -> f64 {
    assert!(world.outputs[o].driver.filters().is_empty());
    let mut db = f64::from(chain.gain_db)
        + world.outputs[o].driver.sensitivity_db
        + world.outputs[o].gain_db;
    for band in chain.peq.iter().chain(&chain.crossover) {
        for c in ProtoModel.sections(band) {
            db += biquad_db(&c, f, f64::from(RATE));
        }
    }
    db
}

fn result_doc(core: &DspiCore) -> dspi_measure::doc::MeasurementDoc {
    dspi_measure::doc::MeasurementDoc::from_text(&core.measure_result().unwrap()).unwrap()
}

#[test]
fn wp_p5ses_device_a_three_output_walked_run_matches_the_world() {
    let r = rig();
    // A peak on output 0's first PEQ band: the chain is in what is measured.
    let n_in = r.core.get_state().num_input_channels;
    r.core
        .set_filter(
            n_in,
            0,
            crate::records::FilterParams {
                filter_type: 1,
                bypass: false,
                freq: 1_000.0,
                q: 1.0,
                gain: 6.0,
                qp: 0.0,
            },
        )
        .unwrap();
    settle(&r.core);
    r.log.lock().unwrap().clear();
    let delays = [0.0, 0.3, 1.234];
    let mut mic = Mic::new(&r, free_world(&delays));
    let st = r
        .core
        .measure_start(walked(&[
            (0, MeasureRole::Full),
            (1, MeasureRole::Full),
            (2, MeasureRole::Full),
        ]))
        .unwrap();
    assert!(st.active && st.wants_capture);
    let st = mic.run(90.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    assert_eq!(st.timing_reliable, Some(true));
    // The timing in detail (P5SPIKE): every cycle's residual, the clock.
    let t = st.timing.clone().unwrap();
    assert!(t.reliable && t.ratio_fitted && t.reasons.is_empty());
    assert_eq!(t.cycle_residuals_us.len(), 9);
    assert_eq!(t.cycle_outputs, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
    assert!(t.max_residual_us <= t.residual_bound_us);
    assert!((t.ratio_ppm - 80.0).abs() < 1.0, "{} ppm", t.ratio_ppm);
    assert_eq!(st.frame_faults, 0);
    let doc = result_doc(&r.core);
    let m = &doc.measurement;
    assert!(m.capture.timing_reliable);
    assert_eq!(
        m.capture.extra["cycleResidualsUs"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    assert_eq!(m.outputs.len(), 3);
    let freqs = dspi_measure::types::LogGrid::STANDARD.freqs().unwrap();
    for (i, o) in m.outputs.iter().enumerate() {
        let resp = o.response.to_response();
        let valid = resp.valid.clone().unwrap();
        let mut checked = 0;
        let mut worst: f64 = 0.0;
        for (k, f) in freqs.iter().enumerate() {
            if *f < 100.0 || *f > 10_000.0 || !valid[k] {
                continue;
            }
            let e = resp.mag_db[k] - truth_db(&mic.world, &mic.chains[i], i, *f);
            worst = worst.max(e.abs());
            checked += 1;
        }
        assert!(checked > 500, "output {i}: {checked} trusted points");
        assert!(worst <= 0.5, "output {i}: {worst:.3} dB off the truth");
    }
    // Arrivals: the differences are the paths' (and the chains' delays).
    let arr: Vec<f64> = m.outputs.iter().map(|o| o.arrival_ms.unwrap()).collect();
    for i in 1..3 {
        let want = (delays[i] + f64::from(mic.chains[i].delay_ms))
            - (delays[0] + f64::from(mic.chains[0].delay_ms));
        let got = arr[i] - arr[0];
        assert!(
            ((got - want) * 1000.0).abs() <= 10.0,
            "output {i}: {got:.4} ms where {want:.4} ms"
        );
    }
    // Every generator configuration finite and below the ceiling; the lock
    // given back; only generator writes.
    let log = r.log.lock().unwrap();
    assert!(
        log.iter()
            .all(|q| q.op == op::REQ_SIGGEN_SET_CONFIG || q.op == op::REQ_SIGGEN_CONTROL)
    );
    assert!(r.core.held_lock_holder().is_none());
}

// ── Faults in a run ────────────────────────────────────────────────────

/// The log since `from`: exactly one STOP_NOW and nothing else.
fn assert_one_stop_now(r: &Rig, from: usize) {
    let log = r.log.lock().unwrap();
    let after: Vec<&Req> = log.iter().skip(from).collect();
    assert_eq!(
        after,
        vec![&Req {
            op: op::REQ_SIGGEN_CONTROL,
            value: crate::consts::SIGGEN_CTL_STOP_NOW,
            data: Vec::new(),
        }],
        "exactly one STOP_NOW and no other write"
    );
}

#[test]
fn wp_p5ses_device_a_dropped_block_aborts() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0, 0.2]));
    // Early in the walked run's second cycle.
    mic.run_faults = vec![CaptureFault::DroppedBlock { at_frame: 70_000 }];
    r.core
        .measure_start(walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]))
        .unwrap();
    let st = mic.run(60.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Aborted, "{st:#?}");
    assert_eq!(st.issue.unwrap().key, "measure.abort.frame_gap");
    assert_eq!(st.frame_faults, 1);
    let log = r.log.lock().unwrap();
    let stops = log
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_CONTROL && q.value == crate::consts::SIGGEN_CTL_STOP_NOW)
        .count();
    assert_eq!(stops, 1);
    assert_eq!(
        log.last().unwrap().value,
        crate::consts::SIGGEN_CTL_STOP_NOW
    );
    assert!(r.core.measure_result().is_err(), "nothing kept");
    assert!(r.core.held_lock_holder().is_none(), "the lock given back");
}

#[test]
fn wp_p5ses_device_a_slow_ratio_wander_is_timing_unreliable() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0, 0.25, 0.5]));
    mic.run_faults = vec![CaptureFault::RatioWander {
        amplitude_ppm: 40.0,
        period_s: 6.0,
    }];
    r.core
        .measure_start(walked(&[
            (0, MeasureRole::Full),
            (1, MeasureRole::Full),
            (2, MeasureRole::Full),
        ]))
        .unwrap();
    let st = mic.run(90.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    assert_eq!(st.timing_reliable, Some(false));
    assert!(!st.timing.as_ref().unwrap().reasons.is_empty());
    let doc = result_doc(&r.core);
    assert!(!doc.measurement.capture.timing_reliable);
    assert!(
        doc.measurement
            .outputs
            .iter()
            .all(|o| o.arrival_ms.is_none()),
        "no arrival from an unreliable run"
    );
}

/// Start a one-output session and run it into its ramp.
fn into_ramp(r: &Rig, setup: MeasureSetup) -> Mic {
    let mut mic = Mic::new(r, free_world(&[0.0]));
    r.core.measure_start(setup).unwrap();
    let st = mic.run(10.0, |s| {
        s.phase == MeasurePhase::Ramp && s.level_db.is_some()
    });
    assert_eq!(st.phase, MeasurePhase::Ramp, "{st:#?}");
    mic
}

#[test]
fn wp_p5ses_device_every_abort_sends_exactly_one_stop_now() {
    type Trigger = Box<dyn Fn(&Rig, &mut Mic)>;
    let cases: Vec<(&str, Trigger)> = vec![
        (
            "measure.abort.capture_clip",
            Box::new(|r, m| {
                let at = m.next;
                r.core
                    .measure_capture_push(at, vec![0.99; 960], RATE, false)
                    .unwrap();
                m.next = at + 960;
            }),
        ),
        (
            "measure.abort.frame_gap",
            Box::new(|r, m| {
                let at = m.next + 4_800;
                r.core
                    .measure_capture_push(at, vec![0.0; 960], RATE, false)
                    .unwrap();
                m.next = at + 960;
            }),
        ),
        (
            "measure.abort.output_peak",
            Box::new(|r, _| r.remote.measure_set_peaks(-200.0, &[(0, 0.0)], 2)),
        ),
        (
            "measure.abort.programme",
            Box::new(|r, _| r.remote.measure_set_peaks(-40.0, &[], 2)),
        ),
        (
            "measure.abort.preset",
            Box::new(|r, _| r.remote.measure_preset_load_elsewhere(0)),
        ),
        (
            "measure.abort.foreign",
            Box::new(|r, _| assert!(r.remote.measure_foreign_output_write(0, -3.0))),
        ),
        (
            "measure.abort.volume",
            Box::new(|r, _| assert!(r.remote.measure_host_volume(-25.0))),
        ),
        (
            "measure.abort.input_format",
            Box::new(|r, _| r.remote.measure_input_format(4)),
        ),
        (
            "measure.abort.lock",
            Box::new(|r, _| r.core.expire_held_lock()),
        ),
    ];
    for (key, trigger) in cases {
        let r = rig();
        // A clip in a ramp step only ends the ramp (S4): the capture clip is
        // played in the pre-roll, where it aborts.
        let mut mic = if key == "measure.abort.capture_clip" {
            let mut mic = Mic::new(&r, free_world(&[0.0]));
            r.core
                .measure_start(walked(&[(0, MeasureRole::Full)]))
                .unwrap();
            assert_eq!(mic.step().phase, MeasurePhase::PreRoll);
            mic
        } else {
            into_ramp(&r, walked(&[(0, MeasureRole::Full)]))
        };
        let from = r.log.lock().unwrap().len();
        trigger(&r, &mut mic);
        let st = mic.run(5.0, |_| false);
        assert_eq!(st.phase, MeasurePhase::Aborted, "{key}: {st:#?}");
        // A preset load stops the generator with its own reason; whichever
        // is judged first names the abort.
        let got = st.issue.unwrap().key;
        assert!(
            got == key || (key == "measure.abort.preset" && got == "measure.abort.generator"),
            "{key}: {got}"
        );
        assert_one_stop_now(&r, from);
        // Ended for good: more capture and ticks send nothing.
        mic.run(0.3, |_| false);
        assert_one_stop_now(&r, from);
        assert!(
            r.core.held_lock_holder().is_none(),
            "{key}: the lock given back"
        );
    }
}

#[test]
fn wp_p5ses_device_a_limiter_that_acts_aborts() {
    let r = rig();
    // A limiter on output 0 with its threshold above the ceiling passes the
    // preflight; its gain reduction is then watched.
    r.core.set_limiter_param(0, 1, -1.0).unwrap();
    r.core.set_limiter_param(0, 0, 1.0).unwrap();
    settle(&r.core);
    r.hooks.lock().unwrap().gr = Some(vec![0.0; 9]);
    r.log.lock().unwrap().clear();
    let mut mic = into_ramp(&r, walked(&[(0, MeasureRole::Full)]));
    let from = r.log.lock().unwrap().len();
    r.hooks.lock().unwrap().gr = Some(vec![3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let st = mic.run(5.0, |_| false);
    assert_eq!(st.issue.unwrap().key, "measure.abort.limiter");
    assert_one_stop_now(&r, from);
}

#[test]
fn wp_p5ses_device_a_cycle_identity_mismatch_aborts() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0, 0.1]));
    r.core
        .measure_start(walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]))
        .unwrap();
    let st = mic.run(30.0, |s| s.phase == MeasurePhase::Run);
    assert_eq!(st.phase, MeasurePhase::Run, "{st:#?}");
    let from = r.log.lock().unwrap().len();
    // The generator reports an output the run does not walk.
    r.hooks.lock().unwrap().status_channel = Some(5);
    let st = mic.run(5.0, |_| false);
    assert_eq!(st.issue.unwrap().key, "measure.abort.cycle_identity");
    assert_one_stop_now(&r, from);
}

// ── The preflight's refusals ───────────────────────────────────────────

fn band(filter_type: u8, freq: f32) -> crate::records::FilterParams {
    crate::records::FilterParams {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

#[test]
fn wp_p5ses_device_preflight_refuses_what_the_rules_forbid() {
    let full = walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]);
    let blockers = |r: &Rig, s: &MeasureSetup| keys(&r.core.measure_preflight(s.clone()).unwrap());
    // S2: the volume at -6 dB.
    let r = rig();
    r.core.set_user_volume(-6.0).unwrap();
    assert!(blockers(&r, &full).contains(&"measure.volume_high".to_owned()));
    // S6: a high way with a 500 Hz high-pass, then a 6 dB/octave one.
    let r = rig();
    let n_in = r.core.get_state().num_input_channels;
    let high = walked(&[(0, MeasureRole::High)]);
    assert!(blockers(&r, &high).contains(&"measure.tweeter_highpass".to_owned()));
    r.core.set_crossover(n_in, 0, band(35, 500.0)).unwrap(); // LR4 high-pass
    assert!(blockers(&r, &high).contains(&"measure.tweeter_highpass".to_owned()));
    r.core.set_crossover(n_in, 0, band(0, 1_000.0)).unwrap();
    r.core.set_filter(n_in, 0, band(13, 2_000.0)).unwrap(); // a first-order PEQ high-pass
    assert!(blockers(&r, &high).contains(&"measure.tweeter_highpass".to_owned()));
    r.core.set_filter(n_in, 0, band(0, 1_000.0)).unwrap();
    r.core.set_crossover(n_in, 0, band(35, 2_000.0)).unwrap();
    assert!(!blockers(&r, &high).contains(&"measure.tweeter_highpass".to_owned()));
    // S13: crossfeed on a measured pair, loudness on.
    let r = rig();
    r.core.set_crossfeed(true).unwrap();
    assert!(blockers(&r, &full).contains(&"measure.crossfeed".to_owned()));
    let r = rig();
    r.core.set_loudness_mask(0xFFFF).unwrap();
    r.core.set_loudness(true).unwrap();
    assert!(blockers(&r, &full).contains(&"measure.loudness".to_owned()));
    // S8: input audio at -40 dBFS.
    let r = rig();
    r.remote.measure_set_peaks(-40.0, &[], 2);
    assert!(blockers(&r, &full).contains(&"measure.input_audio".to_owned()));
    // S11: a pending guard.
    let r = rig();
    r.core.apply_guard_begin("other".into(), 60).unwrap();
    assert!(blockers(&r, &full).contains(&"measure.guard_pending".to_owned()));
    // S11: the read-only mode: refused, nothing sent.
    let r = rig();
    r.core.set_client_read_only(true).unwrap();
    assert!(blockers(&r, &full).contains(&"read_only.client".to_owned()));
    assert!(matches!(
        r.core.measure_start(full.clone()),
        Err(DspiError::ReadOnly)
    ));
    assert!(r.log.lock().unwrap().is_empty());
    assert!(r.core.held_lock_holder().is_none());
    // S11: the measure page locked by the installer.
    let r = rig();
    r.core.installer_pin_set("~p5ses~2468~".into()).unwrap();
    r.core.installer_lock_cover(vec!["measure".into()]).unwrap();
    r.core.installer_lock().unwrap();
    assert!(blockers(&r, &full).contains(&"measure.page_locked".to_owned()));
    assert!(r.core.measure_start(full.clone()).is_err());
    assert!(
        r.log
            .lock()
            .unwrap()
            .iter()
            .all(|q| q.op != op::REQ_SIGGEN_SET_CONFIG)
    );
}

#[cfg(feature = "link")]
#[test]
fn wp_p5ses_device_a_viewer_through_the_hub_is_denied() {
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed("123456".into()),
        pin_role: "viewer".into(),
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "p5ses".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core.link_pair("123456".into()).unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(8);
    while core.link_devices().is_empty() {
        assert!(std::time::Instant::now() < until, "no device");
        std::thread::sleep(Duration::from_millis(5));
    }
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.ensure_probed();
    let setup = walked(&[(0, MeasureRole::Full)]);
    let p = core.measure_preflight(setup.clone()).unwrap();
    assert!(keys(&p).contains(&"measure.viewer".to_owned()), "{p:#?}");
    assert!(matches!(core.measure_start(setup), Err(DspiError::Denied)));
    let sets = hub
        .log()
        .iter()
        .filter(|e| e.2 == op::REQ_SIGGEN_SET_CONFIG || e.2 == op::REQ_SIGGEN_CONTROL)
        .count();
    assert_eq!(sets, 0, "nothing sent");
}

// ── Lock, listener, positions, pink ────────────────────────────────────

#[test]
fn wp_p5ses_device_the_lock_is_held_renewed_and_released() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    r.core
        .measure_start(walked(&[(0, MeasureRole::Full)]))
        .unwrap();
    assert_eq!(
        r.core.held_lock_holder(),
        Some(crate::presets::Holder::Measure)
    );
    assert!(
        r.core
            .measure_lock_renew(session::LOCK_REASON, session::LEASE, true)
    );
    mic.run(0.5, |_| false);
    // A second session on the same core is busy.
    assert!(matches!(
        r.core.measure_start(walked(&[(0, MeasureRole::Full)])),
        Err(DspiError::InvalidArgument { .. })
    ));
    let st = r.core.measure_stop();
    assert_eq!(st.phase, MeasurePhase::Aborted);
    assert!(r.core.held_lock_holder().is_none());
    assert!(
        !r.core
            .measure_lock_renew(session::LOCK_REASON, session::LEASE, true)
    );
    // A finished session can be cleared and a new one started.
    r.core.measure_clear().unwrap();
    assert_eq!(r.core.measure_state().phase, MeasurePhase::Idle);
}

/// A listener that calls back into the core from its callback.
struct Reentrant {
    core: std::sync::Weak<DspiCore>,
    calls: std::sync::atomic::AtomicUsize,
}

impl MeasureListener for Reentrant {
    fn on_measure(&self) {
        if let Some(c) = self.core.upgrade() {
            let _ = c.measure_state();
            let _ = c.get_state();
            let _ = c.siggen_get_status();
        }
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn wp_p5ses_device_a_listener_calling_back_into_the_core_does_not_deadlock() {
    let r = rig();
    let l = Arc::new(Reentrant {
        core: Arc::downgrade(&r.core),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    r.core.measure_set_listener(Some(l.clone()));
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    r.core
        .measure_start(walked(&[(0, MeasureRole::Full)]))
        .unwrap();
    mic.run(3.0, |s| s.phase == MeasurePhase::Ramp);
    r.core.measure_stop();
    assert!(l.calls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    r.core.measure_set_listener(None);
}

#[test]
fn wp_p5ses_device_finite_pink_noise_gives_a_spectrum() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let setup = MeasureSetup {
        kind: MeasureRunKind::Pink,
        outputs: outputs(&[(0, MeasureRole::Full)]),
        pink_ms: 2_000,
        ..MeasureSetup::default()
    };
    r.core.measure_start(setup).unwrap();
    let st = mic.run(30.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    let doc = result_doc(&r.core);
    assert_eq!(
        doc.measurement.stimulus.kind,
        dspi_measure::doc::RunKind::Pink
    );
    let resp = doc.measurement.outputs[0].response.to_response();
    let freqs = dspi_measure::types::LogGrid::STANDARD.freqs().unwrap();
    // Pink through a flat world reads flat (pink-referenced), within a few dB
    // in 200 Hz to 5 kHz.
    let band: Vec<f64> = freqs
        .iter()
        .zip(&resp.mag_db)
        .filter(|(f, _)| (200.0..=5_000.0).contains(*f))
        .map(|(_, m)| *m)
        .collect();
    let (lo, hi) = band
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(*v), b.max(*v))
        });
    assert!(hi - lo < 6.0, "{lo:.1} .. {hi:.1}");
    // Every configuration finite: the pink runs at most 60 s.
    let log = r.log.lock().unwrap();
    for q in log.iter().filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG) {
        let c = dspi_proto::packets::SiggenConfig::decode(&q.data).unwrap();
        assert!(c.duration_ms >= 1 && c.duration_ms <= 60_000);
        assert_eq!(c.flags & crate::consts::SIGGEN_FLAG_RAW, 0);
    }
}

#[test]
fn wp_p5ses_device_positions_wait_for_the_user() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let setup = MeasureSetup {
        placement: MeasurePlacement::NearField,
        ..walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)])
    };
    r.core.measure_start(setup).unwrap();
    let st = mic.run(40.0, |s| s.phase == MeasurePhase::WaitPosition);
    assert_eq!(st.phase, MeasurePhase::WaitPosition, "{st:#?}");
    assert!(!st.wants_capture);
    assert_eq!((st.position, st.positions), (2, 2));
    // Nothing plays while waiting.
    let n = r.log.lock().unwrap().len();
    mic.run(1.0, |_| false);
    assert_eq!(r.log.lock().unwrap().len(), n);
    r.core.measure_stop();
}

// ── The other run kinds and placements ─────────────────────────────────

#[test]
fn wp_p5ses_device_a_summed_run_with_an_invert_mask() {
    let r = rig();
    // Two outputs at one distance, the second 6 dB quieter at the
    // microphone and played inverted: the sum is the difference, 0.5.
    let mut world = free_world(&[0.0, 0.0]);
    world.outputs[1].gain_db = 20.0 * 0.5f64.log10();
    let mut mic = Mic::new(&r, world);
    let mut setup = walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]);
    setup.kind = MeasureRunKind::Summed;
    setup.outputs[1].inverted = true;
    r.core.measure_start(setup).unwrap();
    let st = mic.run(40.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    let log = r.log.lock().unwrap();
    let run = log
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
        .map(|q| dspi_proto::packets::SiggenConfig::decode(&q.data).unwrap())
        .next_back()
        .unwrap();
    assert_eq!(
        (run.channel_mask, run.invert_mask, run.repeat_count),
        (0b11, 0b10, 1)
    );
    drop(log);
    let doc = result_doc(&r.core);
    assert_eq!(
        doc.measurement.stimulus.kind,
        dspi_measure::doc::RunKind::Summed
    );
    let resp = doc.measurement.outputs[0].response.to_response();
    let freqs = dspi_measure::types::LogGrid::STANDARD.freqs().unwrap();
    let mid: Vec<f64> = freqs
        .iter()
        .zip(&resp.mag_db)
        .filter(|(f, _)| (500.0..=2_000.0).contains(*f))
        .map(|(_, m)| *m)
        .collect();
    let mean = mid.iter().sum::<f64>() / mid.len() as f64;
    let want = -6.0 + 20.0 * 0.5f64.log10() + f64::from(mic.chains[0].gain_db);
    assert!(
        (mean - want).abs() < 1.0,
        "the inverted sum: {mean:.2} dB, {want:.2} wanted"
    );
}

#[test]
fn wp_p5ses_device_a_live_sweep_is_magnitude_only() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let mut setup = walked(&[(0, MeasureRole::Full)]);
    setup.kind = MeasureRunKind::LiveSweep;
    r.core.measure_start(setup).unwrap();
    let st = mic.run(30.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    assert_eq!(st.timing_reliable, Some(false));
    let doc = result_doc(&r.core);
    assert_eq!(
        doc.measurement.stimulus.kind,
        dspi_measure::doc::RunKind::LiveSweep
    );
    assert!(doc.measurement.outputs[0].arrival_ms.is_none());
}

#[test]
fn wp_p5ses_device_headphone_reseats_are_averaged() {
    let r = rig();
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let setup = MeasureSetup {
        placement: MeasurePlacement::Headphones,
        coupler: Some(MeasureCoupler::Iec60318_4),
        count: 2,
        outputs: outputs(&[(0, MeasureRole::Full)]),
        sweep_ms: 1_000,
        ..MeasureSetup::default()
    };
    r.core.measure_start(setup).unwrap();
    let st = mic.run(40.0, |s| s.phase == MeasurePhase::WaitPosition);
    assert_eq!(st.phase, MeasurePhase::WaitPosition, "{st:#?}");
    // Every ramp level on a coupler starts at -60 dBFS and stays at or below
    // -30.
    {
        let log = r.log.lock().unwrap();
        let levels: Vec<f32> = log
            .iter()
            .filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
            .map(|q| {
                dspi_proto::packets::SiggenConfig::decode(&q.data)
                    .unwrap()
                    .level_db
            })
            .collect();
        assert_eq!(levels[0], -60.0);
        assert!(levels.iter().all(|l| *l <= -30.0), "{levels:?}");
    }
    // The headphones reseated; the microphone (and its count) starts anew.
    mic.first = 900_000;
    mic.next = 900_000;
    mic.t0 = std::time::Instant::now();
    r.core.measure_next_position().unwrap();
    let st = mic.run(40.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    let doc = result_doc(&r.core);
    assert_eq!(doc.measurement.positions, 2);
    assert!(doc.measurement.outputs[0].response.spread_db.is_some());
}

#[test]
fn wp_p5ses_device_the_second_preflight_checks_chains_and_its_own_guard() {
    let r = rig();
    let setup = walked(&[(0, MeasureRole::Full)]);
    let plan = Plan::new(&setup).unwrap();
    let s = r.core.get_state();
    let mut chains: Chains = vec![(0, preflight::chain_of(&s, 0))];
    let same = PreflightOptions {
        expect_chains: Some(chains.clone()),
        allow_guard: None,
    };
    assert!(r.core.measure_preflight_plan(&plan, &same).unwrap().clear);
    chains[0].1.gain_db -= 3.0;
    let changed = PreflightOptions {
        expect_chains: Some(chains),
        allow_guard: None,
    };
    let p = r.core.measure_preflight_plan(&plan, &changed).unwrap();
    assert_eq!(keys(&p), vec!["measure.chain_changed".to_owned()]);
    // A tune's own guard does not block its verify runs; another does.
    let g = r.core.apply_guard_begin("Auto-tune".into(), 300).unwrap();
    let own = PreflightOptions {
        expect_chains: None,
        allow_guard: Some(g.id),
    };
    assert!(r.core.measure_preflight_plan(&plan, &own).unwrap().clear);
    let p = r
        .core
        .measure_preflight_plan(&plan, &PreflightOptions::default())
        .unwrap();
    assert_eq!(keys(&p), vec!["measure.guard_pending".to_owned()]);
    r.core.apply_revert().unwrap();
}

#[test]
fn wp_p5ses_device_a_lock_the_client_holds_refuses_the_start() {
    let r = rig();
    r.core
        .device_lock_acquire("Restoring".into(), 10_000)
        .unwrap();
    assert!(matches!(
        r.core.measure_start(walked(&[(0, MeasureRole::Full)])),
        Err(DspiError::Locked)
    ));
    assert!(r.log.lock().unwrap().is_empty(), "nothing played");
    assert_eq!(
        r.core.held_lock_holder(),
        Some(crate::presets::Holder::Caller)
    );
    assert_eq!(r.core.measure_state().phase, MeasurePhase::Idle);
    r.core.device_lock_release();
}

// ── Review P5-B ─────────────────────────────────────────────────────────

/// Review P5-B (S2): a volume the host raised a moment before the start, its
/// notification received but not yet applied by the client, is the volume the
/// preflight judges: the start is refused and nothing is sent.
#[test]
fn wp_p5b_device_a_volume_raised_just_before_the_start_is_seen() {
    let r = rig();
    assert!(r.remote.measure_host_volume(-3.0));
    // The notification reaches the core's queue; nobody pumps it.
    std::thread::sleep(Duration::from_millis(100));
    let e = r
        .core
        .measure_start(walked(&[(0, MeasureRole::Full)]))
        .unwrap_err();
    assert!(
        matches!(&e, DspiError::InvalidArgument { msg } if msg.contains("volume")),
        "{e:?}"
    );
    assert!(r.log.lock().unwrap().is_empty(), "nothing was sent");
    assert_eq!(r.core.measure_state().phase, MeasurePhase::Idle);
}

/// Review P5-B, decision (a): what the spike front ends read. A walked run
/// at the start level only, through the state as JSON (the C and wasm
/// bindings serialise this record): the level never above the start, every
/// timing field and the verdict; then a second run stopped by the user sends
/// exactly one `STOP_NOW` and nothing else.
#[test]
fn wp_p5b_device_a_start_level_only_run_gives_the_timing_and_one_stop() {
    let r = rig();
    let mut setup = walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]);
    setup.start_level_only = true;
    setup.placement = MeasurePlacement::Headphones;
    setup.coupler = Some(MeasureCoupler::FlatPlate);
    let mut mic = Mic::new(&r, free_world(&[0.0, 0.25]));
    r.core.measure_start(setup.clone()).unwrap();
    let st = mic.run(120.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{:?}", st.issue);
    let v = serde_json::to_value(&st).unwrap();
    assert_eq!(v["phase"], "done");
    assert_eq!(v["resultReady"], true);
    assert_eq!(v["frameFaults"], 0);
    assert_eq!(v["generatorStarts"], 3, "a step per output, then the run");
    let t = &v["timing"];
    for key in [
        "reliable",
        "ratioPpm",
        "ratioFitted",
        "maxResidualUs",
        "residualBoundUs",
        "maxPhaseSpreadDeg",
        "passes",
        "reasons",
        "cycleOutputs",
        "cycleResidualsUs",
    ] {
        assert!(!t[key].is_null(), "timing.{key} in {t}");
    }
    assert_eq!(t["passes"], 3);
    assert_eq!(t["cycleOutputs"], serde_json::json!([0, 1, 0, 1, 0, 1]));
    assert_eq!(t["cycleResidualsUs"].as_array().unwrap().len(), 6);
    assert_eq!(v["timingReliable"], t["reliable"]);
    assert!((t["ratioPpm"].as_f64().unwrap() - 80.0).abs() < 5.0, "{t}");
    // Every level the generator was given is the headphones' start level.
    let staged: Vec<f32> = r
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
        .map(|q| {
            dspi_proto::packets::SiggenConfig::decode(&q.data)
                .unwrap()
                .level_db
        })
        .collect();
    assert_eq!(staged, vec![-60.0; 3], "{staged:?}");
    let doc: serde_json::Value = serde_json::from_str(&r.core.measure_result().unwrap()).unwrap();
    assert_eq!(doc["measurement"]["stimulus"]["levelDb"], -60.0);
    assert!(doc["measurement"]["capture"]["cycleResidualsUs"].is_array());

    // The user's Stop, mid-run.
    r.core.measure_clear().unwrap();
    r.log.lock().unwrap().clear();
    let mut mic = Mic::new(&r, free_world(&[0.0, 0.25]));
    r.core.measure_start(setup).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while mic.step().phase != MeasurePhase::Run {
        assert!(std::time::Instant::now() < deadline, "the run never began");
    }
    r.log.lock().unwrap().clear();
    let st = r.core.measure_stop();
    assert_eq!(st.phase, MeasurePhase::Aborted);
    assert_eq!(st.issue.as_ref().unwrap().key, "measure.stopped");
    // Stop again, tick, push: nothing more is sent.
    r.core.measure_stop();
    r.core.measure_tick();
    r.core
        .measure_capture_push(mic.next, vec![0.0; 960], RATE, false)
        .unwrap();
    let log = r.log.lock().unwrap();
    let stops = log
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_CONTROL && q.value == crate::consts::SIGGEN_CTL_STOP_NOW)
        .count();
    assert_eq!(stops, 1, "{log:?}");
    assert!(
        log.iter().all(|q| q.op == op::REQ_SIGGEN_CONTROL),
        "nothing but the generator's control: {log:?}"
    );
}
