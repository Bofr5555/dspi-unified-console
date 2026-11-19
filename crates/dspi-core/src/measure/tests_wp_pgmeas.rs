//! WP PGMEAS (PLAN-PG section 1.7): measurement on paged firmware. P5's
//! session and live view run, through `DspiCore` and the adapter
//! (`dspi_transport::paged::PagedTransport` made by `paged::open`), on
//! PGFAKE1/2's G2s (`Stock117`) and G2b (`Bofr117`) demo fakes, with P5's
//! simulated microphone (the scenarios of `tests_wp_p5ses_device.rs` and
//! `live/tests_wp_p5live_device.rs`, the same rig, world and assertions).
//!
//! Two recorders: [`Rec`] above the adapter sees the 1.1.6-shaped calls the
//! session makes (as P5's tests do), [`Wire`] below it sees every paged
//! request that reaches the fake. So each test shows, besides P5's own
//! outcome, that every generator START is **one** transfer on the wire
//! (`MEAS.SIGGEN_START` 0x0948, a zero-length SET: P5 section 1.1 step 5's
//! cycle identity takes the START's frame as the first cycle's), a stop is
//! one `MEAS.SIGGEN_STOP_NOW` 0x094A, a configuration one 36-byte
//! `MEAS.SIGGEN_CONFIG` 0x0940, and that the fakes saw no violation of
//! PLAN-PG section 1.1 (they would panic on one by themselves).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_measure::sim::{Burst, Capture, ProtoModel, Stimulus as SimStimulus, World, render};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_measure::types::ChannelChain;
use dspi_proto::generated::opcodes as op;
use dspi_proto::paged::cmds;
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{
    CmdId, DeviceDescriptor, FakeDevice, FakeRemote, Generation, NotificationSource, PagedRead,
    Transport, TransportCaps, paged,
};

use super::*;

const RATE: u32 = 48_000;

/// The two paged generations of PLAN-PG's table.
const PAGED: [FakeGeneration; 2] = [FakeGeneration::STOCK117, FakeGeneration::Bofr117];

// ── The two recorders ──────────────────────────────────────────────────

/// One paged request that reached the fake.
#[derive(Debug, Clone, PartialEq)]
enum Paged {
    Get {
        id: CmdId,
        value: u16,
        len: u16,
    },
    Set {
        id: CmdId,
        value: u16,
        data: Vec<u8>,
    },
}

impl Paged {
    fn is_set(&self) -> bool {
        matches!(self, Paged::Set { .. })
    }
}

/// The raw wire: the paged fake, recording every request it is sent.
struct Wire {
    dev: FakeDevice,
    log: Arc<Mutex<Vec<Paged>>>,
}

impl Transport for Wire {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        // A page-0 request: the fake counts it a violation (and panics).
        self.dev.control_in(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
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
    fn carries_pages(&self) -> bool {
        self.dev.carries_pages()
    }
    fn control_in_paged(
        &mut self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.log.lock().unwrap().push(Paged::Get {
            id,
            value,
            len: max_len,
        });
        self.dev.control_in_paged(id, value, max_len)
    }
    fn control_out_paged(&mut self, id: CmdId, value: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.log.lock().unwrap().push(Paged::Set {
            id,
            value,
            data: d.to_vec(),
        });
        self.dev.control_out_paged(id, value, d)
    }
    fn control_in_batch_paged(
        &mut self,
        reads: &[PagedRead],
    ) -> Vec<dspi_transport::Result<Vec<u8>>> {
        reads
            .iter()
            .map(|r| self.control_in_paged(r.id, r.value, r.len))
            .collect()
    }
}

/// One 1.1.6-shaped write the core made (OUT transfers and the generator's
/// control, a write-as-read), with the paged requests it became.
#[derive(Debug, Clone, PartialEq)]
struct Req {
    op: u8,
    value: u16,
    data: Vec<u8>,
    wire: Vec<Paged>,
}

#[derive(Default)]
struct Hooks {
    /// The generator's status says this output is playing (as P5SES's).
    status_channel: Option<u8>,
}

/// Above the adapter: P5's recorder, on the transport `paged::open` made.
struct Rec {
    inner: Box<dyn Transport>,
    wire: Arc<Mutex<Vec<Paged>>>,
    log: Arc<Mutex<Vec<Req>>>,
    hooks: Arc<Mutex<Hooks>>,
}

impl Rec {
    fn record<R>(&mut self, o: u8, v: u16, d: &[u8], f: impl FnOnce(&mut dyn Transport) -> R) -> R {
        let from = self.wire.lock().unwrap().len();
        let r = f(&mut *self.inner);
        let wire = self.wire.lock().unwrap()[from..].to_vec();
        self.log.lock().unwrap().push(Req {
            op: o,
            value: v,
            data: d.to_vec(),
            wire,
        });
        r
    }
}

impl Transport for Rec {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        let mut r = if o == op::REQ_SIGGEN_CONTROL {
            self.record(o, v, &[], |t| t.control_in(o, v, l))
        } else {
            self.inner.control_in(o, v, l)
        };
        if o == op::REQ_SIGGEN_GET_STATUS
            && let Ok(r) = r.as_mut()
            && let Some(ch) = self.hooks.lock().unwrap().status_channel
            && let Some(b) = r.get_mut(3)
        {
            *b = ch;
        }
        r
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.record(o, v, d, |t| t.control_out(o, v, d))
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
    fn generation(&self) -> Option<Generation> {
        self.inner.generation()
    }
}

struct Rig {
    g: FakeGeneration,
    core: Arc<DspiCore>,
    remote: FakeRemote,
    log: Arc<Mutex<Vec<Req>>>,
    wire: Arc<Mutex<Vec<Paged>>>,
    hooks: Arc<Mutex<Hooks>>,
}

impl Drop for Rig {
    /// Every test ends with the fakes' own verdict (PLAN-PG section 1.1).
    fn drop(&mut self) {
        if !std::thread::panicking() {
            no_violation(&self.remote, self.g);
        }
    }
}

fn no_violation(remote: &FakeRemote, g: FakeGeneration) {
    assert_eq!(
        remote.violations(),
        0,
        "{g:?}: {:?}",
        remote.last_violation()
    );
    assert_eq!(remote.zero_length_gets(), 0, "{g:?}");
}

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The demo device as firmware of `g`, opened by `paged::open` as every
/// construction site opens one, behind P5's recorder, made ready to measure
/// exactly as P5SES's rig does.
fn rig_as(g: FakeGeneration) -> Rig {
    let dev = FakeDevice::demo_generation(g);
    let remote = dev.remote();
    let wire = Arc::new(Mutex::new(Vec::new()));
    let opened = paged::open(Box::new(Wire {
        dev,
        log: Arc::clone(&wire),
    }))
    .unwrap();
    assert!(
        matches!(opened.generation(), Some(Generation::Paged(_))),
        "{g:?}: {:?}",
        opened.generation()
    );
    let log = Arc::new(Mutex::new(Vec::new()));
    let hooks = Arc::new(Mutex::new(Hooks::default()));
    let rec = Rec {
        inner: opened,
        wire: Arc::clone(&wire),
        log: Arc::clone(&log),
        hooks: Arc::clone(&hooks),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(rec), "PGMEAS-DEMO".into(), 0)
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
    wire.lock().unwrap().clear();
    Rig {
        g,
        core,
        remote,
        log,
        wire,
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

// ── The generator on the wire ──────────────────────────────────────────

/// What every generator write since `from` became on the wire: a START one
/// zero-length `MEAS.SIGGEN_START` SET and nothing else (one transfer), a
/// STOP_NOW one `MEAS.SIGGEN_STOP_NOW`, a configuration one 36-byte
/// `MEAS.SIGGEN_CONFIG` with the same bytes, no other write. Returns the
/// number of STARTs.
fn assert_generator_wire(r: &Rig, from: usize) -> usize {
    let log = r.log.lock().unwrap();
    let mut starts = 0;
    for q in log.iter().skip(from) {
        let want = match (q.op, q.value) {
            (op::REQ_SIGGEN_CONTROL, crate::consts::SIGGEN_CTL_START) => {
                starts += 1;
                Paged::Set {
                    id: cmds::MEAS_SIGGEN_START,
                    value: 0,
                    data: Vec::new(),
                }
            }
            (op::REQ_SIGGEN_CONTROL, crate::consts::SIGGEN_CTL_STOP_NOW) => Paged::Set {
                id: cmds::MEAS_SIGGEN_STOP_NOW,
                value: 0,
                data: Vec::new(),
            },
            (op::REQ_SIGGEN_CONTROL, crate::consts::SIGGEN_CTL_STOP) => Paged::Set {
                id: cmds::MEAS_SIGGEN_STOP,
                value: 0,
                data: Vec::new(),
            },
            (op::REQ_SIGGEN_SET_CONFIG, 0) => {
                assert_eq!(q.data.len(), 36, "{:?}: {q:?}", r.g);
                Paged::Set {
                    id: cmds::MEAS_SIGGEN_CONFIG,
                    value: 0,
                    data: q.data.clone(),
                }
            }
            _ => panic!("{:?}: a write other than the generator's: {q:?}", r.g),
        };
        assert_eq!(q.wire, vec![want], "{:?}: one transfer for {q:?}", r.g);
    }
    starts
}

/// The log since `from`: exactly one STOP_NOW (one transfer) and nothing
/// else.
fn assert_one_stop_now(r: &Rig, from: usize) {
    let log = r.log.lock().unwrap();
    let after: Vec<&Req> = log.iter().skip(from).collect();
    assert_eq!(
        after,
        vec![&Req {
            op: op::REQ_SIGGEN_CONTROL,
            value: crate::consts::SIGGEN_CTL_STOP_NOW,
            data: Vec::new(),
            wire: vec![Paged::Set {
                id: cmds::MEAS_SIGGEN_STOP_NOW,
                value: 0,
                data: Vec::new(),
            }],
        }],
        "{:?}: exactly one STOP_NOW and no other write",
        r.g
    );
}

// ── P5's simulated microphone (as in `tests_wp_p5ses_device.rs`) ─────────

struct Mic {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    world: World,
    chains: Vec<ChannelChain>,
    t0: std::time::Instant,
    first: u64,
    next: u64,
    seg: Option<(Capture, usize)>,
    seen: (u32, u32),
    seed: u64,
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
            seen: (0, 0),
            seed: 0x2545_F491_4F6C_DD1D,
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

    /// What the generator plays for the configuration the fake holds (read
    /// back from its 1.1.6 model under the paged front), from frame `at`.
    fn render(&mut self, at: u64) {
        let cfg = self.remote.measure_generator_config().unwrap();
        let amp = peak_linear(cfg.level_db);
        let mut w = self.world.clone();
        w.clock.first_frame = at;
        let walk = cfg.flags & crate::consts::SIGGEN_FLAG_WALK != 0;
        let outs: Vec<usize> = (0..16)
            .filter(|b| cfg.channel_mask & (1 << b) != 0)
            .collect();
        let capture = if cfg.signal_type == preflight::SIG_PINK {
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

    /// Push what the clock says is due.
    fn push_due(&mut self) {
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
    }

    /// The measurement session: render each start, push, tick.
    fn step(&mut self) -> MeasureState {
        let st = self.core.measure_state();
        if st.generator_starts > self.seen.1 {
            self.seen.1 = st.generator_starts;
            self.render(st.generator_start_frame.unwrap());
        }
        self.push_due();
        self.core.measure_tick()
    }

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

    /// The live view: render each of its runs' starts, push, tick.
    fn step_live(&mut self) -> MeasureLiveState {
        let st = self.core.measure_live_state();
        let key = (st.runs, st.session.generator_starts);
        if st.session.generator_starts > 0 && key != self.seen {
            self.seen = key;
            self.render(st.session.generator_start_frame.unwrap());
        }
        self.push_due();
        self.core.measure_live_tick()
    }

    fn run_live(
        &mut self,
        secs: f64,
        mut until: impl FnMut(&MeasureLiveState) -> bool,
    ) -> MeasureLiveState {
        let end = std::time::Instant::now() + Duration::from_secs_f64(secs);
        loop {
            let st = self.step_live();
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

/// `n` flat outputs in a free field (P5SES's world).
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

fn result_doc(core: &DspiCore) -> dspi_measure::doc::MeasurementDoc {
    dspi_measure::doc::MeasurementDoc::from_text(&core.measure_result().unwrap()).unwrap()
}

/// One `#[test]` per paged generation, so the two run in parallel.
macro_rules! on_paged {
    ($scenario:ident: $g2s:ident, $g2b:ident) => {
        #[test]
        fn $g2s() {
            $scenario(PAGED[0]);
        }
        #[test]
        fn $g2b() {
            $scenario(PAGED[1]);
        }
    };
}

// ── The session (P5SES) ────────────────────────────────────────────────

fn ready_demo_passes_preflight(g: FakeGeneration) {
    let r = rig_as(g);
    let p = r
        .core
        .measure_preflight(walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]))
        .unwrap();
    let keys: Vec<&str> = p.blockers.iter().map(|b| b.key.as_str()).collect();
    assert!(p.clear, "{g:?}: {keys:?}");
    assert!(
        r.log.lock().unwrap().is_empty(),
        "the preflight writes nothing"
    );
    assert!(
        !r.wire.lock().unwrap().iter().any(Paged::is_set),
        "{g:?}: no SET on the wire"
    );
}
on_paged!(ready_demo_passes_preflight:
    wp_pgmeas_g2s_the_ready_demo_passes_preflight,
    wp_pgmeas_g2b_the_ready_demo_passes_preflight);

/// P5SES's walked run, two outputs: done, the timing reliable, arrivals as
/// the world's paths; only generator writes, each START one transfer.
fn walked_run_matches_the_world(g: FakeGeneration) {
    let r = rig_as(g);
    let delays = [0.0, 0.3];
    let mut mic = Mic::new(&r, free_world(&delays));
    let st = r
        .core
        .measure_start(walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]))
        .unwrap();
    assert!(st.active && st.wants_capture);
    let st = mic.run(90.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{g:?}: {st:#?}");
    assert_eq!(st.timing_reliable, Some(true), "{g:?}");
    let t = st.timing.clone().unwrap();
    assert!(
        t.reliable && t.ratio_fitted && t.reasons.is_empty(),
        "{t:?}"
    );
    assert_eq!(t.cycle_outputs, vec![0, 1, 0, 1, 0, 1]);
    assert!(t.max_residual_us <= t.residual_bound_us);
    assert!((t.ratio_ppm - 80.0).abs() < 1.0, "{} ppm", t.ratio_ppm);
    assert_eq!(st.frame_faults, 0);
    let doc = result_doc(&r.core);
    let m = &doc.measurement;
    assert!(m.capture.timing_reliable);
    assert_eq!(m.outputs.len(), 2);
    let arr: Vec<f64> = m.outputs.iter().map(|o| o.arrival_ms.unwrap()).collect();
    let want = (delays[1] + f64::from(mic.chains[1].delay_ms))
        - (delays[0] + f64::from(mic.chains[0].delay_ms));
    assert!(
        ((arr[1] - arr[0] - want) * 1000.0).abs() <= 10.0,
        "{g:?}: {:.4} ms where {want:.4} ms",
        arr[1] - arr[0]
    );
    // The ramp's starts and the run's: every one a single transfer.
    let starts = assert_generator_wire(&r, 0);
    assert!(starts >= 2, "{g:?}: {starts} starts");
    assert_eq!(
        u32::try_from(starts).unwrap(),
        st.generator_starts,
        "{g:?}: every start the session counts went out"
    );
    assert!(r.core.held_lock_holder().is_none());
}
on_paged!(walked_run_matches_the_world:
    wp_pgmeas_g2s_a_walked_run_matches_the_world_and_start_is_one_transfer,
    wp_pgmeas_g2b_a_walked_run_matches_the_world_and_start_is_one_transfer);

/// Start a one-output session and run it into its ramp.
fn into_ramp(r: &Rig) -> Mic {
    let mut mic = Mic::new(r, free_world(&[0.0]));
    r.core
        .measure_start(walked(&[(0, MeasureRole::Full)]))
        .unwrap();
    let st = mic.run(10.0, |s| {
        s.phase == MeasurePhase::Ramp && s.level_db.is_some()
    });
    assert_eq!(st.phase, MeasurePhase::Ramp, "{:?}: {st:#?}", r.g);
    mic
}

/// P5SES's aborts that the device or the core trigger: each sends exactly
/// one STOP_NOW (one `MEAS.SIGGEN_STOP_NOW` on the wire) and nothing more.
fn every_abort_sends_one_stop_now(g: FakeGeneration) {
    type Trigger = Box<dyn Fn(&Rig, &mut Mic)>;
    let cases: Vec<(&str, Trigger)> = vec![
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
            "measure.abort.volume",
            Box::new(|r, _| assert!(r.remote.measure_host_volume(-25.0))),
        ),
        (
            "measure.abort.lock",
            Box::new(|r, _| r.core.expire_held_lock()),
        ),
    ];
    for (key, trigger) in cases {
        let r = rig_as(g);
        let mut mic = into_ramp(&r);
        let from = r.log.lock().unwrap().len();
        trigger(&r, &mut mic);
        let st = mic.run(5.0, |_| false);
        assert_eq!(st.phase, MeasurePhase::Aborted, "{g:?} {key}: {st:#?}");
        assert_eq!(st.issue.unwrap().key, key, "{g:?}");
        assert_one_stop_now(&r, from);
        mic.run(0.3, |_| false);
        assert_one_stop_now(&r, from);
        assert_generator_wire(&r, 0);
        assert!(r.core.held_lock_holder().is_none(), "{g:?} {key}");
    }
}
on_paged!(every_abort_sends_one_stop_now:
    wp_pgmeas_g2s_every_abort_sends_exactly_one_stop_now,
    wp_pgmeas_g2b_every_abort_sends_exactly_one_stop_now);

/// P5SES's cycle identity: the generator's status naming an output the run
/// does not walk aborts (the START frame and the status poll pin the cycle).
fn cycle_identity_mismatch_aborts(g: FakeGeneration) {
    let r = rig_as(g);
    let mut mic = Mic::new(&r, free_world(&[0.0, 0.1]));
    r.core
        .measure_start(walked(&[(0, MeasureRole::Full), (1, MeasureRole::Full)]))
        .unwrap();
    let st = mic.run(30.0, |s| s.phase == MeasurePhase::Run);
    assert_eq!(st.phase, MeasurePhase::Run, "{g:?}: {st:#?}");
    let from = r.log.lock().unwrap().len();
    r.hooks.lock().unwrap().status_channel = Some(5);
    let st = mic.run(5.0, |_| false);
    assert_eq!(
        st.issue.unwrap().key,
        "measure.abort.cycle_identity",
        "{g:?}"
    );
    assert_one_stop_now(&r, from);
    assert_generator_wire(&r, 0);
}
on_paged!(cycle_identity_mismatch_aborts:
    wp_pgmeas_g2s_a_cycle_identity_mismatch_aborts,
    wp_pgmeas_g2b_a_cycle_identity_mismatch_aborts);

/// P5SES's finite pink noise: a flat spectrum, every configuration finite.
fn finite_pink_noise_gives_a_spectrum(g: FakeGeneration) {
    let r = rig_as(g);
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let setup = MeasureSetup {
        kind: MeasureRunKind::Pink,
        outputs: outputs(&[(0, MeasureRole::Full)]),
        pink_ms: 2_000,
        ..MeasureSetup::default()
    };
    r.core.measure_start(setup).unwrap();
    let st = mic.run(30.0, |_| false);
    assert_eq!(st.phase, MeasurePhase::Done, "{g:?}: {st:#?}");
    let doc = result_doc(&r.core);
    assert_eq!(
        doc.measurement.stimulus.kind,
        dspi_measure::doc::RunKind::Pink
    );
    let resp = doc.measurement.outputs[0].response.to_response();
    let freqs = dspi_measure::types::LogGrid::STANDARD.freqs().unwrap();
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
    assert!(hi - lo < 6.0, "{g:?}: {lo:.1} .. {hi:.1}");
    for q in r
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
    {
        let c = dspi_proto::packets::SiggenConfig::decode(&q.data).unwrap();
        assert!(c.duration_ms >= 1 && c.duration_ms <= 60_000);
        assert_eq!(c.flags & crate::consts::SIGGEN_FLAG_RAW, 0);
    }
    assert!(assert_generator_wire(&r, 0) >= 1);
}
on_paged!(finite_pink_noise_gives_a_spectrum:
    wp_pgmeas_g2s_finite_pink_noise_gives_a_spectrum,
    wp_pgmeas_g2b_finite_pink_noise_gives_a_spectrum);

// ── The live view (P5LIVE) ─────────────────────────────────────────────

fn sweep_setup() -> MeasureLiveSetup {
    MeasureLiveSetup {
        output: 0,
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureLiveSetup::default()
    }
}

/// P5LIVE's frame per sweep: the lock held throughout, a frame per sweep,
/// only generator writes (each START one transfer), Stop gives it back.
fn live_frame_per_sweep(g: FakeGeneration) {
    let r = rig_as(g);
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let st = r.core.measure_live_start(sweep_setup()).unwrap();
    assert!(st.active && st.session.active, "{g:?}: {st:#?}");
    let mut seqs = Vec::new();
    let st = mic.run_live(60.0, |s| {
        assert_eq!(
            r.core.held_lock_holder(),
            Some(crate::presets::Holder::Measure),
            "{g:?}: the lock is held while live"
        );
        if seqs.last() != Some(&s.seq) {
            seqs.push(s.seq);
        }
        s.seq >= 2
    });
    assert!(st.active && st.seq >= 2, "{g:?}: {st:#?}");
    assert_eq!(seqs, vec![0, 1, 2], "{g:?}: one frame per sweep");
    let frame = r.core.measure_live_frame();
    assert_eq!(frame.mode, MeasureLiveMode::Sweep);
    assert!(frame.measured.is_some() && frame.target.is_some(), "{g:?}");
    let st = r.core.measure_live_stop();
    assert!(!st.active);
    assert_eq!(st.issue.unwrap().key, "measure.stopped");
    assert_eq!(r.core.held_lock_holder(), None);
    let starts = assert_generator_wire(&r, 0);
    assert!(
        starts >= 4,
        "{g:?}: a ramp step and a sweep per run: {starts}"
    );
    let n = r.log.lock().unwrap().len();
    mic.run_live(1.0, |_| false);
    assert_eq!(
        r.log.lock().unwrap().len(),
        n,
        "{g:?}: nothing after the stop"
    );
}
on_paged!(live_frame_per_sweep:
    wp_pgmeas_g2s_live_a_frame_per_sweep_with_the_lock_held,
    wp_pgmeas_g2b_live_a_frame_per_sweep_with_the_lock_held);

/// P5LIVE's Stop: the measurement's own Stop ends the live view with one
/// STOP_NOW (one transfer) and nothing else.
fn live_stop_sends_one_stop_now(g: FakeGeneration) {
    let r = rig_as(g);
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    r.core.measure_live_start(sweep_setup()).unwrap();
    let st = mic.run_live(20.0, |s| {
        s.session.phase == MeasurePhase::Run && s.session.level_db.is_some()
    });
    assert_eq!(st.session.phase, MeasurePhase::Run, "{g:?}: {st:#?}");
    let n = r.log.lock().unwrap().len();
    r.core.measure_stop();
    assert_one_stop_now(&r, n);
    assert!(!r.core.measure_live_state().active);
    assert_eq!(r.core.held_lock_holder(), None);
    let n = r.log.lock().unwrap().len();
    mic.run_live(1.5, |_| false);
    assert_eq!(r.log.lock().unwrap().len(), n);
    assert_generator_wire(&r, 0);
}
on_paged!(live_stop_sends_one_stop_now:
    wp_pgmeas_g2s_live_stop_stops_the_generator_with_one_stop_now,
    wp_pgmeas_g2b_live_stop_stops_the_generator_with_one_stop_now);

/// P5LIVE's pink RTA: re-armed at its length, every run finite.
fn live_pink_rta_is_re_armed(g: FakeGeneration) {
    let r = rig_as(g);
    let mut mic = Mic::new(&r, free_world(&[0.0]));
    let setup = MeasureLiveSetup {
        mode: MeasureLiveMode::PinkRta,
        pink_ms: 3_000,
        averaging: MeasureRtaAveraging::Infinite,
        ..sweep_setup()
    };
    r.core.measure_live_start(setup).unwrap();
    let st = mic.run_live(60.0, |s| s.runs >= 3);
    assert!(st.active && st.runs >= 3, "{g:?}: {st:#?}");
    let rta = r.core.measure_live_frame().rta.unwrap();
    assert!(rta.frames > 10, "{g:?}: {} frames", rta.frames);
    r.core.measure_live_stop();
    let pinks = r
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
        .map(|q| dspi_proto::packets::SiggenConfig::decode(&q.data).unwrap())
        .filter(|c| c.signal_type == preflight::SIG_PINK && c.duration_ms == 3_000)
        .count();
    assert!(pinks >= 2, "{g:?}: {pinks} pink runs");
    assert_generator_wire(&r, 0);
    assert_eq!(r.core.held_lock_holder(), None);
}
on_paged!(live_pink_rta_is_re_armed:
    wp_pgmeas_g2s_live_pink_rta_is_re_armed,
    wp_pgmeas_g2b_live_pink_rta_is_re_armed);

// ── The generator configuration's length (PLAN-PG section 1.2, R8b) ────

/// A generator configuration of the wrong length is refused by the adapter
/// before anything is sent (`MEAS.SIGGEN_CONFIG` is exactly 36 bytes at the
/// vendored page revision), through the core's raw path and on the opened
/// transport alike; the right length goes out as one transfer.
#[test]
fn wp_pgmeas_the_generator_config_refuses_a_wrong_length_locally() {
    for g in PAGED {
        let r = rig_as(g);
        // A configuration the session staged (valid on the device).
        let _mic = into_ramp(&r);
        r.core.measure_stop();
        let bytes = r
            .log
            .lock()
            .unwrap()
            .iter()
            .find(|q| q.op == op::REQ_SIGGEN_SET_CONFIG)
            .map(|q| q.data.clone());
        let bytes = bytes.unwrap();
        assert_eq!(bytes.len(), 36, "{g:?}");
        let sent = r.remote.paged_requests();
        for len in [0usize, 1, 35, 37, 64] {
            let mut data = bytes.clone();
            data.resize(len, 0);
            let from = r.wire.lock().unwrap().len();
            let e = r.core.raw_set(op::REQ_SIGGEN_SET_CONFIG, 0, data);
            assert!(e.is_err(), "{g:?}: {len} bytes accepted");
            assert_eq!(
                r.wire.lock().unwrap().len(),
                from,
                "{g:?}: {len} bytes reached the wire"
            );
        }
        assert_eq!(r.remote.paged_requests(), sent, "{g:?}: nothing sent");
        // The right length: one transfer.
        let from = r.log.lock().unwrap().len();
        r.core
            .raw_set(op::REQ_SIGGEN_SET_CONFIG, 0, bytes.clone())
            .unwrap();
        assert_generator_wire(&r, from);
        assert_eq!(r.remote.paged_requests(), sent + 1, "{g:?}");
    }
}

/// On a page reported above its vendored revision the adapter does not
/// judge the length: the device does (PLAN-PG OD-PGq, R8b).
#[test]
fn wp_pgmeas_a_newer_meas_page_judges_the_length_itself() {
    let g = FakeGeneration::Stock117 { revision: 200 };
    let dev = FakeDevice::demo_generation(g);
    let remote = dev.remote();
    let wire = Arc::new(Mutex::new(Vec::new()));
    let mut t = paged::open(Box::new(Wire {
        dev,
        log: Arc::clone(&wire),
    }))
    .unwrap();
    wire.lock().unwrap().clear();
    let _ = t.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &[0u8; 37]);
    // Sent; the fake's 1.1.6 model refuses the extra byte, which the
    // adapter may explain with `SYS.LAST_RESULT` (a read).
    let w = wire.lock().unwrap();
    assert!(
        matches!(w.first(), Some(Paged::Set { id, data, .. }) if *id == cmds::MEAS_SIGGEN_CONFIG && data.len() == 37),
        "{w:?}"
    );
    assert!(
        w.iter()
            .skip(1)
            .all(|q| matches!(q, Paged::Get { id, .. } if *id == cmds::SYS_LAST_RESULT)),
        "{w:?}"
    );
    drop(w);
    no_violation(&remote, g);
}

/// Through a hub the generator's class is `control` in both tables: the
/// 1.2 table by opcode (0xA4, 0xA6) and the 1.3 table by id (0x0940 SET,
/// 0x0948-0x094A) (PLAN-PG section 1.7).
#[test]
fn wp_pgmeas_the_generator_is_control_in_both_hub_tables() {
    let t12: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/link/policy/commands-1.2.json"
    )))
    .unwrap();
    let t13: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/link/policy/commands-1.3.json"
    )))
    .unwrap();
    let rows12 = t12["commands"].as_array().unwrap();
    for code in [op::REQ_SIGGEN_SET_CONFIG, op::REQ_SIGGEN_CONTROL] {
        let row = rows12
            .iter()
            .find(|r| r["code"].as_u64() == Some(u64::from(code)))
            .unwrap();
        assert_eq!(row["class"], "control", "{row}");
    }
    let rows13 = t13["commands"].as_array().unwrap();
    for (id, dir) in [
        (cmds::MEAS_SIGGEN_CONFIG, "set"),
        (cmds::MEAS_SIGGEN_START, "set"),
        (cmds::MEAS_SIGGEN_STOP, "set"),
        (cmds::MEAS_SIGGEN_STOP_NOW, "set"),
    ] {
        let row = rows13
            .iter()
            .find(|r| r["id"].as_u64() == Some(u64::from(id.0)))
            .unwrap();
        assert_eq!(row["classes"][dir], "control", "{row}");
    }
}
