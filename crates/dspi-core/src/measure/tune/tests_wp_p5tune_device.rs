//! WP P5TUNE tests on the demo device: the tune through `DspiCore`, with
//! P5SIM's stereo three-way world rendered from each generator configuration
//! the runs staged (read back from the fake device) through the device's
//! chains **as they are at each start**, and pushed as capture with frame
//! indices at the clock's pace. Every device write is recorded.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_measure::doc::MeasurementDoc;
use dspi_measure::sim::{Capture, ProtoModel, Stimulus as SimStimulus, World, render};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_measure::types::ChannelChain;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

use super::tests_wp_p5tune::{
    COMMON_MS, F1, F2, RATE, SERIAL, bass_held, layout, offline_doc, role, speakers_target,
    start_chains, world,
};
use super::*;
use crate::records::FilterParams;

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
    dir: std::path::PathBuf,
}

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn params(b: &dspi_measure::types::BandSpec) -> FilterParams {
    FilterParams {
        filter_type: b.filter_type,
        bypass: b.bypass,
        freq: b.freq,
        q: b.q,
        gain: b.gain,
        qp: b.qp,
    }
}

/// The demo device behind a recorder, set up as the stereo three-way of the
/// test world (`start_chains`: LR4 corners off the design, a common 3 ms
/// delay; input 1 feeds outputs 1 to 3, input 2 outputs 4 to 6), ready to
/// measure, with stores for snapshots and measurements.
fn rig(name: &str) -> Rig {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec {
        dev,
        log: Arc::clone(&log),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(rec), SERIAL.into(), 0).unwrap();
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    let s = core.get_state();
    assert!(
        s.num_output_channels >= 6,
        "{} outputs",
        s.num_output_channels
    );
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
    let chains = start_chains();
    for o in 0..s.num_output_channels {
        let oi = usize::from(o);
        let on = o < 6;
        if s.output_enabled[oi] != on {
            let _ = core.set_output_enable(o, on);
        }
        if s.output_muted[oi] {
            core.set_output_mute(o, false).unwrap();
        }
        if s.limiter_enabled.get(oi).copied().unwrap_or(false) {
            core.set_limiter_param(o, 0, 0.0).unwrap();
        }
        for i in 0..n_in {
            let route = on && i == u8::from(o >= 3);
            core.set_matrix_route(i, o, route, 0.0, false).unwrap();
        }
        if let Some(c) = chains.get(oi) {
            let ch = n_in + o;
            for (b, p) in c.peq.iter().enumerate() {
                core.set_filter(ch, b as u8, params(p)).unwrap();
            }
            for x in 0..s.xover[usize::from(ch)].len() {
                let p = c.crossover.get(x).map(params).unwrap_or(FilterParams {
                    filter_type: 0,
                    bypass: false,
                    freq: 1000.0,
                    q: 0.707,
                    gain: 0.0,
                    qp: 0.0,
                });
                core.set_crossover(ch, x as u8, p).unwrap();
            }
            core.set_output_gain(o, 0.0).unwrap();
            core.set_output_delay(o, COMMON_MS).unwrap();
        }
    }
    let dir = std::env::temp_dir().join("wp-p5tune").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    core.measure_set_dir(dir.join("measure").display().to_string())
        .unwrap();
    core.snapshot_set_dir(dir.join("snapshots").display().to_string())
        .unwrap();
    remote.measure_set_peaks(-200.0, &[], 2);
    settle(&core);
    log.lock().unwrap().clear();
    Rig {
        core,
        remote,
        log,
        dir,
    }
}

impl Rig {
    /// The device's chains of outputs 0 to 5 now.
    fn chains(&self) -> Vec<ChannelChain> {
        let s = self.core.get_state();
        (0..6).map(|o| chain_of(&s, o)).collect()
    }

    /// Writes other than the generator's and the meters' since `from`.
    fn writes_since(&self, from: usize) -> Vec<Req> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .filter(|q| {
                q.op != op::REQ_SIGGEN_SET_CONFIG
                    && q.op != op::REQ_SIGGEN_CONTROL
                    && q.op != op::REQ_CLEAR_CLIPS
            })
            .cloned()
            .collect()
    }

    fn log_len(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The microphone: pushes what the clock says is due, rendering each
/// generator start through the device's chains at that moment, and ticks.
/// `summed_extra` moves output `.0` `.1` ms later in the summed runs only
/// (a walked run whose timing the gate missed).
struct Mic {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    world: World,
    t0: std::time::Instant,
    first: u64,
    next: u64,
    seg: Option<(Capture, usize)>,
    seen: (u32, Option<u64>),
    seed: u64,
    summed_extra: Option<(usize, f64)>,
    // -- WP P5TRUST2 -- an output louder in the walked runs, dB.
    walked_extra_db: Option<(usize, f64)>,
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
            seen: (0, None),
            seed: 0x2545_F491_4F6C_DD1D,
            summed_extra: None,
            walked_extra_db: None,
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

    fn render(&mut self, at: u64) {
        let cfg = self.remote.measure_generator_config().unwrap();
        let amp = peak_linear(cfg.level_db);
        let s = self.core.get_state();
        let chains: Vec<ChannelChain> = (0..self.world.outputs.len() as u8)
            .map(|o| chain_of(&s, o))
            .collect();
        let mut w = self.world.clone();
        w.clock.first_frame = at;
        let walk = cfg.flags & crate::consts::SIGGEN_FLAG_WALK != 0;
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
        let tail = u64::from(sweep.gap_samples) + 4800;
        let capture = if walk {
            if let Some((o, db)) = self.walked_extra_db {
                w.outputs[o].gain_db += db;
            }
            let schedule = WalkSchedule::new(sweep, cfg.channel_mask, cfg.repeat_count).unwrap();
            let bursts = schedule
                .cycles()
                .iter()
                .map(|c| dspi_measure::sim::Burst {
                    output: usize::from(c.output),
                    start_frame: c.start,
                    samples: if c.index == 0 { &first } else { &regular },
                    gain: 1.0,
                })
                .collect();
            let stim = SimStimulus {
                bursts,
                frames: schedule.total_samples() + tail,
            };
            render(&w, &stim, &chains, &ProtoModel, RATE).unwrap()
        } else {
            if let Some((o, ms)) = self.summed_extra {
                w.outputs[o].delay_ms += ms;
            }
            let outs: Vec<(usize, bool)> = (0..16)
                .filter(|b| cfg.channel_mask & (1 << b) != 0)
                .map(|o| (o, cfg.invert_mask & (1 << o) != 0))
                .collect();
            let stim = SimStimulus::summed(&first, &outs, 0, tail).unwrap();
            render(&w, &stim, &chains, &ProtoModel, RATE).unwrap()
        };
        self.seg = Some((capture, 0));
    }

    /// Push what is due, then `tick`.
    fn step<T>(&mut self, tick: &dyn Fn(&DspiCore) -> T) -> T {
        let st = self.core.measure_state();
        let key = (st.generator_starts, st.generator_start_frame);
        if st.generator_starts > 0 && key != self.seen {
            self.seen = key;
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
            self.core
                .measure_capture_push(first, samples, RATE, clipped)
                .unwrap();
        }
        tick(&self.core)
    }

    /// A measurement of `setup`, to its end (at most `secs`).
    fn measure(&mut self, setup: MeasureSetup, secs: f64) -> String {
        self.core.measure_start(setup).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs_f64(secs);
        loop {
            let st = self.step(&|c| c.measure_tick());
            if !st.active {
                assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
                let text = self.core.measure_result().unwrap();
                self.core.measure_clear().unwrap();
                return text;
            }
            assert!(
                std::time::Instant::now() < end,
                "the measurement took too long"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Tick the tune until its runs end (at most `secs`).
    fn tune(&mut self, secs: f64) -> MeasureTuneState {
        let end = std::time::Instant::now() + Duration::from_secs_f64(secs);
        loop {
            let st = self.step(&|c| c.measure_tune_tick());
            if !st.running {
                return st;
            }
            assert!(
                std::time::Instant::now() < end,
                "the tune's runs took too long: {st:#?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The world as the device hears it: the test world 10 dB louder at the
/// microphone, in a quiet room (-110 dBFS). The ramp settles every way at its
/// start level (-50 dBFS: heard at once). In P5SIM's own -85 dBFS the run
/// itself is fine since P5ARRIVAL (`review_p5c2_the_ramps_level_in_p5sims_noise`
/// below), but the drivers' de-embedded responses are then not trusted one
/// octave past their corners (RC12's mask and the trust mask's repeat
/// spread there: review P5-C2's follow-up), so the transitions are kept;
/// this world shows the whole flow.
fn heard() -> World {
    let mut w = world(11, 0.3);
    w.noise_dbfs = Some(-110.0);
    for p in &mut w.outputs {
        p.gain_db += 10.0;
    }
    w
}

/// A walked measurement setup of the layout's outputs.
fn walked(outputs: &[u8]) -> MeasureSetup {
    MeasureSetup {
        outputs: layout(outputs)
            .outputs
            .iter()
            .map(|o| MeasureOutput {
                output: o.output,
                role: o.role,
                speaker: o.speaker.clone(),
                group: o.group,
                inverted: false,
            })
            .collect(),
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureSetup::default()
    }
}

/// An EQ-only proposal of the left speaker from a document made offline
/// (timing unreliable: no summed check is needed to apply it).
fn eq_only(r: &Rig) -> MeasureTuneProposal {
    let text = offline_doc(&world(7, 0.3), &r.chains(), &[0, 1, 2], SERIAL);
    let mut doc = MeasurementDoc::from_text(&text).unwrap();
    doc.measurement.capture.timing_reliable = false;
    let p = r
        .core
        .measure_tune_propose(
            doc.to_text().unwrap(),
            layout(&[0, 1, 2]),
            MeasureTuneOptions::default(),
        )
        .unwrap();
    assert!(!p.writes.is_empty() && p.writes.iter().all(|w| !w.timing));
    assert_eq!(p.sum_check, MeasureTuneCheck::NotNeeded);
    p
}

/// The parameters of `p`'s writes.
fn param_count(p: &MeasureTuneProposal) -> usize {
    p.writes.iter().map(|w| w.params.len()).sum()
}

#[test]
fn wp_p5tune_device_the_layout_proposal_reads_the_device() {
    let r = rig("layout");
    let l = r.core.measure_layout_proposal().unwrap();
    assert_eq!(l.template, "3way", "{l:#?}");
    assert_eq!(l.outputs.len(), 6, "{l:#?}");
    for o in &l.outputs {
        assert_eq!(o.role, role(o.output));
        assert_eq!(o.speaker, if o.output < 3 { "L" } else { "R" });
        assert_eq!(o.feed_input, Some(u8::from(o.output >= 3)));
        assert_eq!(o.group, u8::from(o.output >= 3));
    }
}

#[test]
fn wp_p5tune_device_the_whole_flow_from_layout_to_keep() {
    let r = rig("flow");
    let mut mic = Mic::new(&r, heard());
    // 1. The layout; 2. the measurement (one walked run per speaker).
    let lay = r.core.measure_layout_proposal().unwrap();
    let text = mic.measure(walked(&[0, 1, 2, 3, 4, 5]), 300.0);
    let filed = r.core.measure_doc_put(text).unwrap();
    // 3. The proposal, from the filed measurement.
    let p = r
        .core
        .measure_tune_propose(filed.id.clone(), lay, MeasureTuneOptions::default())
        .unwrap();
    for w in &p.writes {
        println!("{}", w.words);
    }
    assert!(
        p.timing_reliable && p.withdrawn.is_empty(),
        "{:#?}",
        p.withdrawn
    );
    assert_eq!(p.sum_check, MeasureTuneCheck::Pending);
    assert_eq!(p.curves.len(), 2);
    assert_eq!(p.transitions.len(), 4);
    println!("{:#?}", p.notes);
    assert!(
        !p.notes.iter().any(|n| n.contains("are kept")),
        "{:#?}",
        p.notes
    );
    for side in [0u8, 3] {
        let flip = |o: u8| {
            p.writes
                .iter()
                .any(|w| w.output == o && w.kind == MeasureTuneKind::Polarity)
        };
        // The inverted midrange is corrected (it, or both others, flipped).
        assert_ne!(flip(side + 1), flip(side), "side {side}");
        assert_eq!(flip(side + 2), flip(side), "side {side}");
    }
    // Applying before the summed check is refused, nothing written.
    let n = r.log_len();
    let e = r.core.measure_tune_apply(p.id, 0).unwrap_err();
    assert!(
        e.to_string().contains("measure.tune.sum_check_needed"),
        "{e}"
    );
    assert!(r.writes_since(n).is_empty());
    // 4. The summed check: it passes in this world.
    r.core.measure_tune_sum_check(p.id).unwrap();
    let st = mic.tune(180.0);
    assert_eq!(st.stage, MeasureTuneStage::Proposed, "{st:#?}");
    let p = r.core.measure_tune_proposal().unwrap();
    assert_eq!(p.sum_check, MeasureTuneCheck::Passed, "{:#?}", p.withdrawn);
    for t in &p.transitions {
        println!("summed check {t:?}");
        assert!(t.sum_check_rms_db.unwrap() <= 1.5);
    }
    // 5. Apply: one undo group inside the guard.
    let before = r.core.history(1000).len();
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.phase, GuardPhase::Waiting);
    assert_eq!(g.label, TUNE_LABEL);
    let h = r.core.history(1000);
    let new = h.len() - before;
    assert!(new > 0 && new <= param_count(&p), "{new} history entries");
    assert!(
        h.iter()
            .take(new)
            .all(|e| e.label.as_deref() == Some(TUNE_LABEL)),
        "every write in the one Auto-tune step"
    );
    let after = r.chains();
    let min = |c: &[ChannelChain]| c.iter().map(|c| c.delay_ms).fold(f32::MAX, f32::min);
    assert!(
        (min(&after[..3]) - COMMON_MS).abs() < 1e-3,
        "the common delay kept"
    );
    assert!(
        (min(&after[3..]) - COMMON_MS).abs() < 1e-3,
        "the common delay kept"
    );
    // 6. Verify inside the guard: the predicted and the verified sums agree.
    r.core.measure_tune_verify().unwrap();
    let st = mic.tune(400.0);
    assert_eq!(st.stage, MeasureTuneStage::Verified, "{st:#?}");
    let v = st.verify.clone().unwrap();
    println!("verify {v:#?}");
    assert!(v.timing_reliable);
    assert_eq!(v.sums.len(), 2);
    for s in &v.sums {
        assert!(s.rms_db.unwrap() <= 1.5, "{s:?}");
    }
    assert_eq!(v.transitions.len(), 4);
    for t in &v.transitions {
        assert!(t.verify_rms_db.unwrap() <= 1.5, "{t:?}");
    }
    assert!(v.passed);
    // The bass as proposed: the modes within 2 dB of the target.
    for c in &p.curves {
        let resp = dspi_measure::types::Response {
            grid: LogGrid {
                min_hz: dspi_measure::types::GRID_MIN_HZ,
                max_hz: dspi_measure::types::GRID_MAX_HZ,
                points_per_octave: 48,
            },
            mag_db: c.predicted.mag_db.clone(),
            phase_deg: None,
            valid: Some(c.predicted.valid.clone()),
            spl: false,
        };
        let on = dspi_measure::response::resample(&resp, &LogGrid::STANDARD).unwrap();
        let (modes, peak) = bass_held(&on, &speakers_target());
        println!(
            "{}: bass modes within {modes:.2} dB, peak {peak:+.2} dB",
            c.speaker
        );
        assert!(modes <= 2.0 && peak <= 2.0);
    }
    // 7. Keep: the after snapshot with its tune block, the measurement's row.
    let k = r.core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept, "{k:#?}");
    assert!(!k.snapshot_before_id.is_empty() && !k.snapshot_after_id.is_empty());
    let after_doc: serde_json::Value = serde_json::from_str(&k.snapshot_after).unwrap();
    assert_eq!(after_doc["tune"]["algorithm"], ALGORITHM);
    assert_eq!(after_doc["tune"]["measurement"], filed.id.as_str());
    assert_eq!(k.measurement_id, filed.id, "updated in place");
    let m = MeasurementDoc::from_text(&r.core.measure_doc_get(filed.id.clone()).unwrap()).unwrap();
    assert_eq!(m.measurement.tunes.len(), 1);
    assert_eq!(m.measurement.tunes[0].snapshot_before, k.snapshot_before_id);
    assert_eq!(m.measurement.tunes[0].snapshot_after, k.snapshot_after_id);
    assert!(m.measurement.tunes[0].kept);
    let names: Vec<String> = r
        .core
        .snapshot_list(String::new())
        .iter()
        .map(|s| s.name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("Before auto-tune")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("After auto-tune")),
        "{names:?}"
    );
    // 8. A/B: the before state and the tune toggle.
    let tuned = r.remote.image();
    let ab = r.core.measure_tune_ab().unwrap();
    assert!(ab.active);
    r.core.ab_toggle().unwrap();
    settle(&r.core);
    assert_ne!(r.remote.image(), tuned, "B is the before state");
    r.core.ab_toggle().unwrap();
    settle(&r.core);
    assert_eq!(r.chains(), after, "A is the tune");
}

#[test]
fn wp_p5tune_device_a_summed_run_that_disagrees_withdraws_the_timing() {
    let r = rig("summed");
    let mut mic = Mic::new(&r, heard());
    // The summed runs hear the tweeter 0.25 ms later than the walked run
    // did: a slip the timing gate missed.
    mic.summed_extra = Some((2, 0.25));
    let text = mic.measure(walked(&[0, 1, 2]), 150.0);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    assert!(p.writes.iter().any(|w| w.timing));
    r.core.measure_tune_sum_check(p.id).unwrap();
    let st = mic.tune(120.0);
    assert_eq!(st.stage, MeasureTuneStage::Proposed, "{st:#?}");
    let q = r.core.measure_tune_proposal().unwrap();
    println!("{:#?}", q.withdrawn);
    assert_ne!(q.id, p.id);
    assert_eq!(q.sum_check, MeasureTuneCheck::Failed);
    assert!(q.writes.iter().all(|w| !w.timing), "timing withdrawn");
    assert!(
        q.writes.iter().any(|w| w.kind == MeasureTuneKind::Eq),
        "the EQ kept"
    );
    assert!(
        q.withdrawn
            .iter()
            .any(|i| i.key == super::maths::KEY_SUM_FAILED)
    );
    // The EQ alone may be applied (no timing write is left).
    let n = r.log_len();
    r.core.measure_tune_apply(q.id, 0).unwrap();
    assert!(!r.writes_since(n).is_empty());
    r.core.measure_tune_revert().unwrap();
}

#[test]
fn wp_p5tune_device_one_undo_puts_every_value_back() {
    let r = rig("undo");
    settle(&r.core);
    let before = r.remote.image();
    let state = r.core.get_state();
    let p = eq_only(&r);
    let n = r.core.history(1000).len();
    r.core.measure_tune_apply(p.id, 0).unwrap();
    settle(&r.core);
    assert_ne!(r.remote.image(), before);
    let h = r.core.history(1000);
    assert!(
        h.iter()
            .take(h.len() - n)
            .all(|e| e.label.as_deref() == Some(TUNE_LABEL))
    );
    r.core.undo().unwrap();
    settle(&r.core);
    assert_eq!(r.remote.image(), before, "one undo, every value back");
    assert_eq!(r.core.get_state().filters, state.filters);
    assert_eq!(r.core.get_state().output_gain_db, state.output_gain_db);
    r.core.measure_tune_revert().unwrap();
}

#[test]
fn wp_p5tune_device_revert_and_the_timeout_restore_the_before_state_byte_for_byte() {
    let r = rig("revert");
    settle(&r.core);
    let before = r.remote.image();
    let p = eq_only(&r);
    r.core.measure_tune_apply(p.id, 0).unwrap();
    settle(&r.core);
    assert_ne!(r.remote.image(), before);
    let g = r.core.measure_tune_revert().unwrap();
    assert_eq!(g.phase, GuardPhase::Reverted);
    settle(&r.core);
    assert_eq!(r.remote.image(), before, "Revert: byte for byte");
    assert_eq!(
        r.core.measure_tune_state().stage,
        MeasureTuneStage::Reverted
    );
    // The guard's timeout.
    let p = eq_only(&r);
    r.core.measure_tune_apply(p.id, 30).unwrap();
    settle(&r.core);
    assert_ne!(r.remote.image(), before);
    crate::guarded::skew::advance(Duration::from_secs(31));
    let st = r.core.measure_tune_tick();
    assert_eq!(st.stage, MeasureTuneStage::Reverted, "{st:#?}");
    settle(&r.core);
    assert_eq!(r.remote.image(), before, "the timeout: byte for byte");
    // Keep after the revert is refused.
    assert!(r.core.measure_tune_keep().is_err());
}

#[test]
fn wp_p5tune_device_an_apply_after_programme_audio_started_is_refused_with_nothing_written() {
    let r = rig("programme");
    let p = eq_only(&r);
    r.remote.measure_set_peaks(-40.0, &[], 2);
    settle(&r.core);
    let n = r.log_len();
    let e = r.core.measure_tune_apply(p.id, 0).unwrap_err();
    assert!(e.to_string().contains("stop playback"), "{e}");
    assert!(r.writes_since(n).is_empty(), "nothing written");
    assert_eq!(r.core.apply_status().id, 0, "no guard begun");
    // A chain changed since the proposal: refused the same way.
    r.remote.measure_set_peaks(-200.0, &[], 2);
    let ch = r.core.get_state().num_input_channels;
    r.core
        .set_filter(
            ch,
            9,
            FilterParams {
                filter_type: 1,
                bypass: false,
                freq: 500.0,
                q: 1.0,
                gain: -2.0,
                qp: 0.0,
            },
        )
        .unwrap();
    settle(&r.core);
    let n = r.log_len();
    let e = r.core.measure_tune_apply(p.id, 0).unwrap_err();
    assert!(e.to_string().contains("changed since"), "{e}");
    assert!(r.writes_since(n).is_empty());
}

#[test]
fn wp_p5tune_device_a_limits_profile_is_never_exceeded_and_named() {
    let r = rig("limits");
    let dir = r.dir.join("limits");
    r.core.limits_set_dir(dir.display().to_string()).unwrap();
    let profile = dspi_session::limits::LimitsProfile {
        outputs: vec![dspi_session::limits::OutputLimits {
            output: 1,
            max_gain_db: None,
            highpass_min_hz: Some(300.0),
            highpass_min_slope_db: Some(24),
            delay_min_ms: None,
            delay_max_ms: None,
            limiter_ceiling_db: None,
        }],
        max_peq_boost_db: Some(3.0),
        ..dspi_session::limits::LimitsProfile::default()
    };
    r.core.limits_apply_profile(profile.to_document()).unwrap();
    let text = offline_doc(&world(7, 0.3), &r.chains(), &[0, 1, 2], SERIAL);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    for w in &p.writes {
        println!("{}", w.words);
    }
    println!("{:#?}", p.held);
    assert!(
        p.held
            .iter()
            .any(|h| h.contains("PEQ boost at most +3.0 dB")),
        "{:?}",
        p.held
    );
    assert!(
        p.held
            .iter()
            .any(|h| h.contains("output 2: a high-pass at or above 300 Hz")),
        "{:?}",
        p.held
    );
    let mut corner = None;
    for w in &p.writes {
        for q in &w.params {
            if q.path == "eq.gain"
                && let ParamValue::Float(g) = q.value
            {
                assert!(g <= 3.0, "{}", w.words);
            }
            if q.path == "eq.freq"
                && w.output == 1
                && q.indices[1] == super::maths::XOVER_BASE
                && let ParamValue::Float(f) = q.value
            {
                corner = Some(f);
                assert!(f >= 300.0, "{}", w.words);
            }
        }
    }
    println!("the mid's high-pass: {corner:?}");
}

#[test]
fn wp_p5tune_device_read_only_and_another_device_get_nothing_applied() {
    let r = rig("readonly");
    let p = eq_only(&r);
    r.core.set_client_read_only(true).unwrap();
    let n = r.log_len();
    assert!(matches!(
        r.core.measure_tune_apply(p.id, 0),
        Err(DspiError::ReadOnly)
    ));
    assert!(r.writes_since(n).is_empty(), "nothing sent");
    r.core.set_client_read_only(false).unwrap();
    // A measurement of another device is refused.
    let text = offline_doc(&world(7, 0.3), &r.chains(), &[0, 1, 2], "ANOTHER-DSPI");
    let e = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap_err();
    assert!(e.to_string().contains("measure.tune.other_device"), "{e}");
}

#[cfg(feature = "link")]
#[test]
fn wp_p5tune_device_a_viewer_through_the_hub_is_denied() {
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
        "p5tune".into(),
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
    core.link_select_device(d.handle, d.serial.clone()).unwrap();
    core.ensure_probed();
    // A measurement of the hub's device, its output 1 as a full-range way.
    let s = core.get_state();
    let chains: Vec<ChannelChain> = (0..s.num_output_channels.min(6))
        .map(|o| chain_of(&s, o))
        .collect();
    let mut w = World::flat(chains.len(), RATE);
    w.noise_dbfs = Some(-90.0);
    w.outputs[0].driver.breakup = Some(dspi_measure::sim::SimFilter::Peak {
        freq: 80.0,
        q: 4.0,
        gain_db: 6.0,
    });
    let text = offline_doc(&w, &chains, &[0], &d.serial);
    let lay = MeasureLayout {
        outputs: vec![MeasureLayoutOutput {
            output: 0,
            role: MeasureRole::Full,
            speaker: "L".into(),
            ..MeasureLayoutOutput::default()
        }],
        ..MeasureLayout::default()
    };
    let p = core
        .measure_tune_propose(text, lay, MeasureTuneOptions::default())
        .unwrap();
    assert!(!p.writes.is_empty(), "{p:#?}");
    let before = hub.log().len();
    let r = core.measure_tune_apply(p.id, 0);
    assert!(matches!(r, Err(DspiError::Denied)), "{r:?}");
    let writes = hub.log().iter().skip(before).filter(|e| e.1 == 0).count();
    assert_eq!(writes, 0, "nothing sent");
}

#[test]
fn wp_p5tune_device_the_demo_simulated_microphone_measures_and_checks() {
    // P5DEMO's simulated microphone: the demo device's own room (P5SIM's
    // three-way room), heard without a client pushing anything.
    // The whole left speaker: since P5ARRIVAL the woofer behind its LR4
    // low-pass is identified at the ramp's level in the demo room's noise
    // (review P5-C2; P5TIME's whole-band search aborted this run).
    let r = rig("demo-sim");
    r.core.measure_sim_set(true, 5).unwrap();
    r.core.measure_start(walked(&[0, 1, 2])).unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(150);
    let st = loop {
        let st = r.core.measure_tick();
        if !st.active {
            break st;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(st.phase, MeasurePhase::Done, "{st:#?}");
    let text = r.core.measure_result().unwrap();
    r.core.measure_clear().unwrap();
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    assert!(p.timing_reliable, "{p:#?}");
    assert_eq!(p.transitions.len(), 2);
    if !p.writes.iter().any(|w| w.timing) {
        // At the ramp's level in the demo room the drivers are not trusted
        // one octave past the corner (see `heard`): the transition is kept,
        // and said so; nothing needs the summed check.
        assert!(
            p.notes.iter().any(|n| n.contains("are kept")),
            "{:#?}",
            p.notes
        );
        assert_eq!(p.sum_check, MeasureTuneCheck::NotNeeded);
        r.core.measure_sim_set(false, 0).unwrap();
        return;
    }
    r.core.measure_tune_sum_check(p.id).unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(120);
    let st = loop {
        let st = r.core.measure_tune_tick();
        if !st.running {
            break st;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(st.stage, MeasureTuneStage::Proposed, "{st:#?}");
    let q = r.core.measure_tune_proposal().unwrap();
    assert_eq!(q.sum_check, MeasureTuneCheck::Passed, "{:#?}", q.withdrawn);
    r.core.measure_sim_set(false, 0).unwrap();
}

// ── Review P5-C2 ────────────────────────────────────────────────────────

/// P5TUNE's follow-up 1, on its own rig: P5SIM's three-way room as it is
/// (-85 dBFS of noise, nothing made louder), the ramp stopping where it hears
/// 35 dB. The woofer behind its LR4 low-pass is identified at -50 dBFS (the
/// run aborted on "not heard in the window" before P5ARRIVAL) and the run is
/// timing reliable.
#[test]
fn review_p5c2_the_ramps_level_in_p5sims_noise_identifies_every_way() {
    let r = rig("p5c2-noise");
    let w = world(11, 0.3);
    assert_eq!(w.noise_dbfs, Some(-85.0), "P5SIM's own noise");
    let mut mic = Mic::new(&r, w);
    let text = mic.measure(walked(&[0, 1, 2]), 300.0);
    let doc = MeasurementDoc::from_text(&text).unwrap();
    let c = &doc.measurement.capture;
    assert!(c.timing_reliable, "{text}");
    assert!(
        doc.measurement.stimulus.level_db <= -44.0,
        "the ramp's level"
    );
    assert_eq!(doc.measurement.outputs.len(), 3);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    assert!(p.timing_reliable && p.withdrawn.is_empty(), "{p:#?}");
    // Whatever is offered at this level, nothing of the timing is applied
    // before its summed check.
    if p.writes.iter().any(|w| w.timing) {
        let e = r.core.measure_tune_apply(p.id, 0).unwrap_err();
        assert!(e.to_string().contains("measure.tune.sum_check_needed"));
    }
}

/// S9: a refusal that withdrawing the refused speaker's own writes does not
/// remove (the common headroom cut on an output already at the registry's
/// lowest gain, for another speaker's boost) is never offered: before the
/// review the proposal kept the write `out.gain` -61.3 dB after its eight
/// rounds.
#[test]
fn review_p5c2_a_refusal_no_withdrawal_removes_is_never_offered() {
    let r = rig("p5c2-stall");
    let outs = [0, 1, 2, 3, 4, 5];
    let text = offline_doc(&world(7, 0.3), &r.chains(), &outs, SERIAL);
    // Since the measurement: the right tweeter turned down to the lowest
    // gain there is (the proposal is made for the chains now, OD-P5o).
    r.core.set_output_gain(5, -60.0).unwrap();
    settle(&r.core);
    let mut doc = MeasurementDoc::from_text(&text).unwrap();
    doc.measurement.capture.timing_reliable = false;
    let p = r
        .core
        .measure_tune_propose(
            doc.to_text().unwrap(),
            layout(&outs),
            MeasureTuneOptions::default(),
        )
        .unwrap();
    for w in &p.writes {
        for q in &w.params {
            if q.path == "out.gain"
                && let ParamValue::Float(g) = q.value
            {
                assert!(g >= -60.0, "{}", w.words);
            }
        }
    }
    assert!(
        p.withdrawn
            .iter()
            .any(|i| i.message.contains("a write the core would refuse")),
        "{:#?}",
        p.withdrawn
    );
    // What is offered can be applied.
    if !p.writes.is_empty() {
        r.core.measure_tune_apply(p.id, 0).unwrap();
        r.core.measure_tune_revert().unwrap();
    }
}

/// RC5f, fail closed: a summed check whose runs did not all give a result
/// confirms nothing (before the review a speaker with one of its two
/// transitions checked was confirmed, and a check with no result at all read
/// "passed").
#[test]
fn review_p5c2_an_incomplete_summed_check_confirms_nothing() {
    use std::collections::BTreeMap;
    let need = vec![("L".to_owned(), 2), ("R".to_owned(), 2)];
    let passes: BTreeMap<String, usize> = [("L".to_owned(), 1), ("R".to_owned(), 2)].into();
    let ok = super::fully_checked(&need, &passes, &[]);
    assert!(!ok.contains("L") && ok.contains("R"), "{ok:?}");
    assert!(super::fully_checked(&need, &passes, &["R".to_owned()]).is_empty());
    assert!(super::fully_checked(&need, &BTreeMap::new(), &[]).is_empty());
    // On the device: the check's runs ended without a result.
    let r = rig("p5c2-partial");
    let text = offline_doc(&world(7, 0.3), &r.chains(), &[0, 1, 2], SERIAL);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    assert!(p.writes.iter().any(|w| w.timing), "{p:#?}");
    {
        let mut t = r.core.measure.tune_lock();
        t.stage = MeasureTuneStage::SumCheck;
        r.core.tune_sum_done(&mut t, &[]);
        assert!(t.confirmed.is_empty());
        assert_eq!(t.stage, MeasureTuneStage::Proposed);
    }
    let q = r.core.measure_tune_proposal().unwrap();
    assert_eq!(q.sum_check, MeasureTuneCheck::Pending, "not passed");
    let n = r.log_len();
    let e = r.core.measure_tune_apply(q.id, 0).unwrap_err();
    assert!(
        e.to_string().contains("measure.tune.sum_check_needed"),
        "{e}"
    );
    assert!(r.writes_since(n).is_empty(), "nothing written");
}

/// S10: a crosspoint that feeds a tuned output but is not its loudest (the
/// preflight's chain holds the loudest only) changed since the proposal: the
/// apply is refused with nothing written (a polarity write would have put
/// the crosspoint's old gain back).
#[test]
fn review_p5c2_a_changed_secondary_crosspoint_refuses_the_apply() {
    let r = rig("p5c2-feeds");
    let p = eq_only(&r);
    let s = r.core.get_state();
    let fed = (0..s.num_input_channels)
        .find(|i| s.matrix_routing[usize::from(*i)][0])
        .expect("output 1 is fed");
    let other = (0..s.num_input_channels)
        .find(|i| *i != fed && !s.matrix_routing[usize::from(*i)][0])
        .expect("an input that does not feed output 1");
    r.core
        .set_matrix_route(other, 0, true, -30.0, false)
        .unwrap();
    settle(&r.core);
    let n = r.log_len();
    let e = r.core.measure_tune_apply(p.id, 0).unwrap_err();
    assert!(e.to_string().contains("crosspoint"), "{e}");
    assert!(r.writes_since(n).is_empty(), "nothing written");
    assert_eq!(r.core.apply_status().id, 0, "no guard begun");
}

/// S3: the tune's own runs (the summed check, the verify) never play louder
/// than the measurement was made: its level is their ceiling. Before the
/// review they ramped to the default ceiling (-12 dBFS for speakers) whatever
/// ceiling the user had measured with.
#[test]
fn review_p5c2_the_tunes_runs_never_play_louder_than_the_measurement() {
    let r = rig("p5c2-ceiling");
    let text = offline_doc(&world(7, 0.3), &r.chains(), &[0, 1, 2], SERIAL);
    let mut doc = MeasurementDoc::from_text(&text).unwrap();
    assert_eq!(super::run_ceiling(&doc), -20.0, "the document's level");
    doc.measurement.stimulus.level_db = -3.0;
    assert_eq!(
        super::run_ceiling(&doc),
        crate::measure::plan::CEILING_SPEAKERS_DB,
        "never above the default ceiling"
    );
    doc.measurement.stimulus.level_db = -36.0;
    assert_eq!(super::run_ceiling(&doc), -36.0);
    let p = r
        .core
        .measure_tune_propose(
            doc.to_text().unwrap(),
            layout(&[0, 1, 2]),
            MeasureTuneOptions::default(),
        )
        .unwrap();
    assert!(p.writes.iter().any(|w| w.timing), "{p:#?}");
    // The summed check's runs carry it.
    r.core.measure_tune_sum_check(p.id).unwrap();
    {
        let t = r.core.measure.tune_lock();
        let runs = t.runs.as_ref().expect("the check runs");
        let all = runs.current.iter().chain(&runs.queue);
        assert!(all.clone().count() >= 2);
        for item in all {
            assert_eq!(item.setup.ceiling_db, Some(-36.0), "{:?}", item.setup.name);
        }
    }
    let st = r.core.measure_state();
    assert!(st.active, "{st:#?}");
    r.core.measure_tune_stop();
    assert!(!r.core.measure_state().active);
}

// ── WP P5TRUST ──────────────────────────────────────────────────────────

/// Review P5-C2's follow-up 2 as a done criterion: P5TUNE's rig in P5SIM's
/// own room (-85 dBFS of noise, nothing made louder), the ramp's own level.
/// Before the trust rules were fixed every transition was kept (EQ only);
/// now the proposal holds delays, the midrange's polarity and the crossover,
/// the corners near the design, and the summed checks pass within RC5f (the
/// reviewer's probe read 0.25 and 0.09 dB RMS at -50 dBFS).
#[test]
fn wp_p5trust_the_rig_at_realistic_noise_gives_a_full_timing_proposal() {
    let r = rig("p5trust-noise");
    let w = world(11, 0.3);
    assert_eq!(w.noise_dbfs, Some(-85.0), "P5SIM's own noise");
    let mut mic = Mic::new(&r, w);
    let text = mic.measure(walked(&[0, 1, 2]), 300.0);
    let doc = MeasurementDoc::from_text(&text).unwrap();
    assert!(doc.measurement.capture.timing_reliable, "{text}");
    let level = doc.measurement.stimulus.level_db;
    println!("measured at {level:.1} dBFS");
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    for w in &p.writes {
        println!("{}", w.words);
    }
    for n in &p.notes {
        println!("note: {n}");
    }
    assert!(p.timing_reliable && p.withdrawn.is_empty(), "{p:#?}");
    assert!(
        !p.notes.iter().any(|n| n.contains("trusted range")),
        "no transition is kept for want of trust: {:#?}",
        p.notes
    );
    let has = |o: u8, k: MeasureTuneKind| p.writes.iter().any(|w| w.output == o && w.kind == k);
    assert!(
        (0..3).any(|o| has(o, MeasureTuneKind::Delay)),
        "a delay is proposed"
    );
    assert_ne!(
        has(1, MeasureTuneKind::Polarity),
        has(0, MeasureTuneKind::Polarity),
        "the inverted midrange is corrected"
    );
    assert!(
        (0..3).any(|o| has(o, MeasureTuneKind::Crossover)),
        "a corner is proposed"
    );
    assert_eq!(p.transitions.len(), 2);
    for (t, design) in p.transitions.iter().zip([F1, F2]) {
        let off = |f: f64| (f / design).log2().abs();
        println!(
            "transition {}-{}: {:.0} -> {:.0} Hz (design {design:.0}), delay {:.4} ms, flipped {}",
            t.low_output,
            t.high_output,
            t.corner_before_hz,
            t.corner_hz,
            t.relative_delay_ms,
            t.flipped
        );
        // Never further from the design than the corner was, and within a
        // quarter of an octave of it.
        assert!(off(t.corner_hz) <= off(t.corner_before_hz) + 1e-6);
        assert!(off(t.corner_hz) <= 0.25 + 1e-3, "{t:?}");
    }
    // The tweeter's high-pass is never below its protective floor.
    let hp = p
        .transitions
        .iter()
        .find(|t| t.high_output == 2)
        .unwrap()
        .corner_hz;
    assert!(hp >= f64::from(HighpassRule::HIGH.min_hz));
    // The summed check passes, within RC5f.
    r.core.measure_tune_sum_check(p.id).unwrap();
    let st = mic.tune(180.0);
    assert_eq!(st.stage, MeasureTuneStage::Proposed, "{st:#?}");
    let p = r.core.measure_tune_proposal().unwrap();
    assert_eq!(p.sum_check, MeasureTuneCheck::Passed, "{:#?}", p.withdrawn);
    for t in &p.transitions {
        let rms = t.sum_check_rms_db.unwrap();
        println!(
            "summed check {}-{}: {rms:.2} dB RMS (review P5-C2's probe: 0.25 and 0.09)",
            t.low_output, t.high_output
        );
        assert!(rms <= dspi_measure::xover::SUMMED_CHECK_MAX_RMS_DB);
    }
    assert!(
        p.writes.iter().any(|w| w.timing),
        "the timing stays offered"
    );
}

/// Coordinator decisions (b) and (c): the guard of an applied tune lasts the
/// planned verify plus a margin (not a flat 300 s), and Keep after a verify
/// that did not pass is refused with `measure.tune.keep_unverified` until it
/// is asked a second time.
#[test]
fn wp_p5trust_keep_after_a_failed_verify_needs_a_second_confirmation() {
    let r = rig("p5trust-keep");
    let mut mic = Mic::new(&r, heard());
    let text = mic.measure(walked(&[0, 1, 2]), 150.0);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0, 1, 2]), MeasureTuneOptions::default())
        .unwrap();
    assert!(p.writes.iter().any(|w| w.timing), "{p:#?}");
    // Every crossover write is a timing write: applied only after the
    // summed check passed (decision (a)).
    assert!(
        p.writes
            .iter()
            .filter(|w| w.kind == MeasureTuneKind::Crossover)
            .all(|w| w.timing)
    );
    r.core.measure_tune_sum_check(p.id).unwrap();
    let st = mic.tune(180.0);
    assert_eq!(st.stage, MeasureTuneStage::Proposed, "{st:#?}");
    let p = r.core.measure_tune_proposal().unwrap();
    assert_eq!(p.sum_check, MeasureTuneCheck::Passed);
    // (c) The guard's length: the planned verify plus the margin.
    let planned = {
        let t = r.core.measure.tune_lock();
        DspiCore::tune_guard_s(t.input.as_ref().unwrap(), t.computed.as_ref().unwrap())
    };
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.phase, GuardPhase::Waiting);
    assert_eq!(g.timeout_ms, planned * 1000, "{g:#?}");
    assert_ne!(planned, TUNE_GUARD_DEFAULT_S);
    assert!(planned > TUNE_GUARD_MARGIN_S && planned <= 590, "{planned}");
    // The verify hears the tweeter 0.25 ms later than everything before:
    // its summed run disagrees with the prediction.
    mic.summed_extra = Some((2, 0.25));
    let started = std::time::Instant::now();
    r.core.measure_tune_verify().unwrap();
    let st = mic.tune(400.0);
    let took = started.elapsed().as_secs_f64();
    println!("the verify took {took:.0} s of a {planned} s guard");
    assert!(took < f64::from(planned - TUNE_GUARD_MARGIN_S), "{took}");
    assert_eq!(st.stage, MeasureTuneStage::Verified, "{st:#?}");
    let v = st.verify.clone().unwrap();
    assert!(!v.passed, "{v:#?}");
    // -- review P5TRUST -- the failed verdict holds after a Stop (the stage
    // goes back to Applied): Keep still asks.
    let st = r.core.measure_tune_stop();
    assert_eq!(st.stage, MeasureTuneStage::Applied, "{st:#?}");
    // (b) The first Keep is refused, typed; nothing changes.
    let image = r.remote.image();
    let e = r.core.measure_tune_keep().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_UNVERIFIED), "{e}");
    let st = r.core.measure_tune_state();
    assert_eq!(st.stage, MeasureTuneStage::Applied);
    assert_eq!(st.issue.as_ref().unwrap().key, KEY_KEEP_UNVERIFIED);
    assert_eq!(st.guard.phase, GuardPhase::Waiting, "the guard runs on");
    assert_eq!(r.remote.image(), image);
    // -- review P5TRUST -- a double click is one press: a Keep right after
    // the refusal is refused too, through the tune's Keep and through the
    // guard's own (the generic Keep of a client's guard banner).
    let e = r.core.measure_tune_keep().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_UNVERIFIED), "{e}");
    let e = r.core.apply_confirm().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_UNVERIFIED), "{e}");
    assert_eq!(r.core.apply_status().phase, GuardPhase::Waiting);
    assert_eq!(r.remote.image(), image);
    // The second, explicit Keep keeps it.
    crate::guarded::skew::advance(std::time::Duration::from_millis(TUNE_KEEP_CONFIRM_MIN_MS));
    let k = r.core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept, "{k:#?}");
    assert_eq!(r.core.measure_tune_state().stage, MeasureTuneStage::Kept);
}

/// Decision (c), the length alone: longer sweeps, a longer guard; never past
/// the guard's longest.
#[test]
fn wp_p5trust_the_guard_scales_with_the_planned_verify() {
    let r = rig("p5trust-guard");
    let _ = eq_only(&r);
    let (input, computed) = {
        let t = r.core.measure.tune_lock();
        (t.input.clone().unwrap(), t.computed.clone().unwrap())
    };
    let short = DspiCore::tune_guard_s(&input, &computed);
    let mut long = input.clone();
    long.doc.measurement.stimulus.duration_ms = 5_500;
    let longer = DspiCore::tune_guard_s(&long, &computed);
    println!("guard {short} s at 1 s sweeps, {longer} s at 5.5 s");
    assert!(
        short > TUNE_GUARD_MARGIN_S && longer > short,
        "{short} {longer}"
    );
    long.doc.measurement.stimulus.duration_ms = 15_000;
    long.doc.measurement.stimulus.repeat = 8;
    assert!(DspiCore::tune_guard_s(&long, &computed) <= crate::guarded::APPLY_GUARD_MAX_TIMEOUT_S);
    // -- WP P5TRUST2 -- (review P5TRUST N7) a verify longer than the guard's
    // longest: the guard is still capped, and the proposal says so.
    let planned = DspiCore::tune_guard_planned_s(&long, &computed).unwrap();
    assert!(
        planned > u64::from(crate::guarded::APPLY_GUARD_MAX_TIMEOUT_S),
        "{planned}"
    );
    assert_eq!(
        DspiCore::tune_guard_s(&long, &computed),
        crate::guarded::APPLY_GUARD_MAX_TIMEOUT_S
    );
    let w = DspiCore::tune_guard_warning(&long, &computed).unwrap();
    assert_eq!(w.key, KEY_GUARD_CAPPED);
    assert!(w.message.contains(&planned.to_string()), "{w:?}");
    assert!(DspiCore::tune_guard_warning(&input, &computed).is_none());
    assert_eq!(
        DspiCore::tune_guard_planned_s(&input, &computed),
        Some(u64::from(short))
    );
    // The apply with 0 takes it; an explicit length is the caller's.
    let p = eq_only(&r);
    assert!(
        p.warnings.iter().all(|w| w.key != KEY_GUARD_CAPPED),
        "{:?}",
        p.warnings
    );
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.timeout_ms, short * 1000);
    r.core.measure_tune_revert().unwrap();
    let p = eq_only(&r);
    let g = r.core.measure_tune_apply(p.id, 45).unwrap();
    assert_eq!(g.timeout_ms, 45_000);
    r.core.measure_tune_revert().unwrap();
}

// -- WP P5TRUST2 -- (P5TM's finding) a layout without transitions: the
// verify judges each output against its prediction too.
#[test]
fn wp_p5trust2_a_verify_without_transitions_fails_on_one_bad_output() {
    let r = rig("p5trust2-verify");
    let mut mic = Mic::new(&r, heard());
    let text = mic.measure(walked(&[0]), 150.0);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0]), MeasureTuneOptions::default())
        .unwrap();
    assert!(p.transitions.is_empty(), "{p:#?}");
    assert!(!p.writes.is_empty(), "{p:#?}");
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.phase, GuardPhase::Waiting);
    // As measured: the verify passes, each output near its prediction.
    r.core.measure_tune_verify().unwrap();
    let st = mic.tune(120.0);
    let v = st.verify.clone().unwrap();
    println!("verify as measured: {:?}", v.outputs);
    assert!(v.sums.is_empty() && v.transitions.is_empty(), "{v:#?}");
    assert!(v.passed, "{v:#?}");
    // The output plays 4 dB louder than predicted: the verify fails on it.
    mic.walked_extra_db = Some((0, 4.0));
    r.core.measure_tune_verify().unwrap();
    let st = mic.tune(120.0);
    let v = st.verify.clone().unwrap();
    println!("verify 4 dB off: {:?}", v.outputs);
    assert!(!v.passed, "{v:#?}");
    let o = v.outputs.iter().find(|o| o.output == 0).unwrap();
    assert!(o.rms_db.unwrap() > VERIFY_OUTPUT_MAX_RMS_DB, "{o:?}");
    assert!(
        v.notes.iter().any(|n| n.contains("its prediction")),
        "{v:#?}"
    );
    // The two-step Keep is armed.
    let e = r.core.measure_tune_keep().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_UNVERIFIED), "{e}");
    assert_eq!(r.core.apply_status().phase, GuardPhase::Waiting);
    crate::guarded::skew::advance(std::time::Duration::from_millis(TUNE_KEEP_CONFIRM_MIN_MS));
    let k = r.core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept, "{k:#?}");
}

// -- WP P5HOLD -- (review P5-D D3) Keep is refused while a verify runs; Keep
// before any verify keeps its one press.
#[test]
fn wp_p5hold_keep_is_refused_while_the_verify_runs() {
    // Before any verify has started: one Keep keeps (the proposal is "Not
    // verified" in every client's words).
    let r = rig("p5hold-keep-unverified");
    let p = eq_only(&r);
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.phase, GuardPhase::Waiting);
    let k = r.core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept, "{k:#?}");
    assert_eq!(r.core.measure_tune_state().stage, MeasureTuneStage::Kept);

    // While the verify runs: refused, typed, through the tune's Keep and the
    // guard's own; nothing changes, the verify and the guard run on.
    let r = rig("p5hold-keep-verifying");
    let mut mic = Mic::new(&r, heard());
    let text = mic.measure(walked(&[0]), 150.0);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0]), MeasureTuneOptions::default())
        .unwrap();
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.phase, GuardPhase::Waiting);
    r.core.measure_tune_verify().unwrap();
    assert_eq!(
        r.core.measure_tune_state().stage,
        MeasureTuneStage::Verifying
    );
    let image = r.remote.image();
    let e = r.core.measure_tune_keep().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_DURING_VERIFY), "{e}");
    let st = r.core.measure_tune_state();
    assert_eq!(st.stage, MeasureTuneStage::Verifying, "{st:#?}");
    assert!(st.running, "the verify runs on");
    assert_eq!(st.issue.as_ref().unwrap().key, KEY_KEEP_DURING_VERIFY);
    let e = r.core.apply_confirm().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_DURING_VERIFY), "{e}");
    assert_eq!(r.core.apply_status().phase, GuardPhase::Waiting);
    assert_eq!(r.remote.image(), image);
    // The verdict answers it: the issue goes, and one Keep keeps a verify
    // that passed.
    let st = mic.tune(120.0);
    assert_eq!(st.stage, MeasureTuneStage::Verified, "{st:#?}");
    assert!(st.verify.as_ref().unwrap().passed, "{:#?}", st.verify);
    assert!(
        st.issue
            .as_ref()
            .is_none_or(|i| i.key != KEY_KEEP_DURING_VERIFY),
        "{st:#?}"
    );
    let k = r.core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept, "{k:#?}");
}

// Review P5-E: a Stop of the verify ends "the verify is still running".
#[test]
fn review_p5e_a_stopped_verify_no_longer_says_it_is_running() {
    let r = rig("p5e-keep-verifying-stop");
    let mut mic = Mic::new(&r, heard());
    let text = mic.measure(walked(&[0]), 150.0);
    let p = r
        .core
        .measure_tune_propose(text, layout(&[0]), MeasureTuneOptions::default())
        .unwrap();
    let g = r.core.measure_tune_apply(p.id, 0).unwrap();
    assert_eq!(g.phase, GuardPhase::Waiting);
    r.core.measure_tune_verify().unwrap();
    let e = r.core.measure_tune_keep().unwrap_err();
    assert!(e.to_string().contains(KEY_KEEP_DURING_VERIFY), "{e}");
    assert_eq!(
        r.core.measure_tune_state().issue.unwrap().key,
        KEY_KEEP_DURING_VERIFY
    );
    let st = r.core.measure_tune_stop();
    assert_eq!(st.stage, MeasureTuneStage::Applied, "{st:#?}");
    assert!(!st.running);
    assert!(
        st.issue
            .as_ref()
            .is_none_or(|i| i.key != KEY_KEEP_DURING_VERIFY),
        "{st:#?}"
    );
    assert_eq!(r.core.apply_status().phase, GuardPhase::Waiting);
    // No verify had its verdict: the core's rule is one Keep (the screens
    // that run the verify by themselves ask first).
    let k = r.core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept, "{k:#?}");
}
