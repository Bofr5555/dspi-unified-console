//! The session's state machine (WP P5SES, PLAN-P5 sections 1.1 to 1.3): the
//! pre-roll, the ramp of each output, the run, the analysis in slices, the
//! positions, the result. It talks to the device only through
//! [`super::Device`], so its tests can play the device; the core plays it in
//! `measure.rs`.
//!
//! **The push path** ([`Session::push`]) checks the frame index, a clip and
//! the SPL cap, O(block), then stores the samples: no analysis there (RC8).
//! **The tick** ([`Session::tick`]) judges what the notification path saw,
//! renews the lease, polls the generator and the meters at about 4 Hz, moves
//! the stages on, and runs one analysis slice (a cycle's FFT at most).

use std::time::Duration;

use dspi_measure::ir::{AnalysisOptions, MAX_CAPTURE_SAMPLES, Step, WalkedAnalyser, WalkedCapture};
use dspi_measure::rta::{Averaging, MicRta, RtaConfig};
use dspi_measure::stimulus::{GeneratorStatus, NO_CHANNEL, WalkSchedule};
use dspi_transport::clock::Instant;

use super::plan::{PINK_POST_MS, PREROLL_MS, Plan, RunSpec};
use super::runs::{self, RAMP_PINK_MS, RAMP_SWEEP_MS, RunResult};
use super::safety::{
    self, GenView, LIMITER_ACTS_DB, OUTPUT_PEAK_DBFS, Ramp, RampNext, SplMeter, StepEval,
    StepRecord,
};
use super::{
    Chains, Device, MeasureIssue, MeasureLevel, MeasurePhase, MeasureRunKind, MeasureState,
    TapEvent,
};
use crate::DspiError;
use crate::records::{SiggenConfig, SiggenStatus};
use dspi_measure::hearing::Listener;

/// The device lock's lease (OD-P5n): renewed while the session is active.
pub(crate) const LEASE: Duration = Duration::from_secs(30);
/// How often the lease is renewed.
const RENEW_EVERY: Duration = Duration::from_secs(10);
/// The lock's reason, as other sessions see it.
pub(crate) const LOCK_REASON: &str = "Measuring";
/// The largest capture block accepted, samples.
pub(crate) const MAX_BLOCK: usize = 1 << 20;
/// How often the generator and the meters are polled while playing.
const POLL_EVERY: Duration = Duration::from_millis(250);
/// Capture kept after a ramp step's stimulus (the input latency), ms.
const RAMP_TAIL_MS: u32 = 600;
/// A run or a step this much later than planned aborts (S5).
const OVERRUN: Duration = Duration::from_secs(5);
/// How far a status poll may be from the capture's time, ms.
const STATUS_SLACK_MS: f64 = 250.0;
/// Failed polls in a row before the session gives up.
const MAX_POLL_FAILURES: u32 = 4;
/// Pink noise analysed per tick, ms of capture.
const PINK_SLICE_MS: u32 = 1_000;
/// Pink noise skipped at its start (the input latency), ms.
const PINK_SKIP_MS: u32 = 300;

/// What the session needs of the device, read at its start.
#[derive(Debug, Clone)]
pub(crate) struct Context {
    pub(crate) serial: String,
    pub(crate) platform: String,
    pub(crate) device_rate_hz: u32,
    pub(crate) chains: Chains,
    /// The device's inputs (the meter block lists them first).
    pub(crate) num_inputs: u8,
    /// The measured outputs whose limiter is on (its gain reduction is
    /// watched, S13).
    pub(crate) limiter_outputs: Vec<u8>,
}

/// What follows a pre-roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Ramp,
    Run,
}

/// The analysis of a run.
enum Analysis {
    Walked(Box<WalkedAnalyser>, (usize, usize)),
    Pink { rta: Box<MicRta>, fed: usize },
}

enum Stage {
    /// The generator idle: `buf` fills with the noise floor.
    PreRoll {
        after: After,
    },
    /// One ramp step: `level` on `ramp.output`.
    RampStep {
        ramp: Ramp,
        level: f32,
        clipped: bool,
        loud: bool,
    },
    /// The run plays.
    Run,
    /// The run is analysed.
    Analyse(Analysis),
    /// Waiting for the next position.
    WaitPosition,
    Done,
    Aborted,
    Failed,
}

/// A generator run in flight: where it started and should end.
#[derive(Debug, Clone, Copy)]
struct Playing {
    /// The capture frame at the START's reply.
    start: u64,
    /// The capture frame by which it is over (its tail included).
    end: u64,
    deadline: Instant,
    /// The generator's end was seen.
    ended: bool,
    /// Its schedule's cycle identity is checked (walked runs).
    walked: bool,
}

/// A measurement session.
pub(crate) struct Session {
    plan: Plan,
    ctx: Context,
    stage: Stage,
    /// The run in progress, an index into `plan.runs`.
    run: usize,
    /// The ramp's output, an index into the run's outputs.
    ramp_idx: usize,
    levels: Vec<MeasureLevel>,
    run_level: Option<f32>,
    /// The frame the next block must start at (none before the first, and
    /// after a new position).
    next_frame: Option<u64>,
    frames: u64,
    /// The capture of the current stage and its first frame.
    buf: Vec<f32>,
    buf_first: u64,
    /// The pre-roll's listener of the ramp's steps (WP P5RAMPFIX); `None`
    /// hears nothing (fail closed).
    listener: Option<Listener>,
    /// The ramp's steps of this run, judged.
    ramp_steps: Vec<StepRecord>,
    spl: Option<SplMeter>,
    generator: GenView,
    playing: Option<Playing>,
    schedule: Option<WalkSchedule>,
    starts: u32,
    start_frame: Option<u64>,
    last_poll: Option<Instant>,
    last_renew: Instant,
    poll_failures: u32,
    active_inputs: Option<u8>,
    limiter_outputs: Vec<u8>,
    gr_base: Option<Vec<f32>>,
    issue: Option<MeasureIssue>,
    results: Vec<RunResult>,
    document: Option<String>,
    progress: f32,
    timing_reliable: Option<bool>,
    timing: Option<super::MeasureTiming>,
    frame_faults: u32,
    /// The lock and the tap are still held: the core gives them back.
    release_pending: bool,
    // -- WP ROLL1 --
    /// Who started it ([`crate::actor::JobActor`]).
    actor: crate::actor::ActorId,
}

fn frames_of(ms: u32, rate: u32) -> u64 {
    u64::from(rate).saturating_mul(u64::from(ms)) / 1000
}

fn issue(key: &str, msg: impl Into<String>) -> MeasureIssue {
    MeasureIssue::new(key, None, msg)
}

// -- WP ROLL1 --
/// Why `cfg` would play below the stimulus floor of an output it plays
/// (pink noise on a high way, a sweep from below 1 kHz), or `None`.
pub(crate) fn below_floor(plan: &Plan, cfg: &SiggenConfig) -> Option<String> {
    let outs: Vec<u8> = (0u8..16)
        .filter(|o| {
            1u16.checked_shl(u32::from(*o))
                .is_some_and(|b| cfg.channel_mask & b != 0)
        })
        .collect();
    let floor = plan.floor_of(&outs);
    if floor <= 0.0 {
        return None;
    }
    let f1 = match cfg.signal_type {
        super::preflight::SIG_LOG_SWEEP => cfg.p.first().copied().unwrap_or(0.0),
        _ => 0.0,
    };
    let f2 = cfg.p.get(1).copied().unwrap_or(0.0);
    if cfg.signal_type == super::preflight::SIG_LOG_SWEEP
        && f1.is_finite()
        && f2.is_finite()
        && f1.min(f2) >= floor
    {
        return None;
    }
    Some(format!(
        "refused a stimulus that plays a tweeter below {floor:.0} Hz"
    ))
}

impl Session {
    pub(crate) fn new(plan: Plan, ctx: Context, now: Instant) -> Session {
        let limiter_outputs = ctx.limiter_outputs.clone();
        Session {
            spl: plan
                .spl_cap_db
                .is_some()
                .then(|| SplMeter::new(plan.capture_rate_hz)),
            plan,
            ctx,
            stage: Stage::PreRoll { after: After::Ramp },
            run: 0,
            ramp_idx: 0,
            levels: Vec::new(),
            run_level: None,
            next_frame: None,
            frames: 0,
            buf: Vec::new(),
            buf_first: 0,
            listener: None,
            ramp_steps: Vec::new(),
            generator: GenView::default(),
            playing: None,
            schedule: None,
            starts: 0,
            start_frame: None,
            last_poll: None,
            last_renew: now,
            poll_failures: 0,
            active_inputs: None,
            limiter_outputs,
            gr_base: None,
            issue: None,
            results: Vec::new(),
            document: None,
            progress: 0.0,
            timing_reliable: None,
            timing: None,
            frame_faults: 0,
            release_pending: true,
            actor: crate::actor::EMBEDDER,
        }
    }

    // ── Queries ────────────────────────────────────────────────────────

    pub(crate) fn phase(&self) -> MeasurePhase {
        match &self.stage {
            Stage::PreRoll { .. } => MeasurePhase::PreRoll,
            Stage::RampStep { .. } => MeasurePhase::Ramp,
            Stage::Run => MeasurePhase::Run,
            Stage::Analyse(_) => MeasurePhase::Analysing,
            Stage::WaitPosition => MeasurePhase::WaitPosition,
            Stage::Done => MeasurePhase::Done,
            Stage::Aborted => MeasurePhase::Aborted,
            Stage::Failed => MeasurePhase::Failed,
        }
    }

    /// Ended, one way or another.
    pub(crate) fn finished(&self) -> bool {
        matches!(self.stage, Stage::Done | Stage::Aborted | Stage::Failed)
    }

    /// Holds the generator, the lock or a stage in progress.
    pub(crate) fn active(&self) -> bool {
        !self.finished()
    }

    /// What a listener must hear about.
    pub(crate) fn phase_key(&self) -> (MeasurePhase, usize, Option<u8>, bool) {
        let out = match &self.stage {
            Stage::RampStep { ramp, .. } => Some(ramp.output),
            _ => None,
        };
        (self.phase(), self.run, out, self.issue.is_some())
    }

    pub(crate) fn release_pending(&self) -> bool {
        self.release_pending && self.finished()
    }

    pub(crate) fn released(&mut self) {
        self.release_pending = false;
    }

    pub(crate) fn result(&self) -> Option<String> {
        self.document.clone()
    }

    fn wants_capture(&self) -> bool {
        matches!(
            self.stage,
            Stage::PreRoll { .. } | Stage::RampStep { .. } | Stage::Run | Stage::Analyse(_)
        )
    }

    fn current_run(&self) -> Option<&RunSpec> {
        self.plan.runs.get(self.run)
    }

    pub(crate) fn state(&self) -> MeasureState {
        let (output, level_db) = match &self.stage {
            Stage::RampStep { ramp, level, .. } if self.generator.playing => {
                (Some(ramp.output), Some(*level))
            }
            Stage::RampStep { ramp, .. } => (Some(ramp.output), None),
            Stage::Run if self.generator.playing => (
                self.current_run()
                    .filter(|r| r.outputs.len() == 1)
                    .and_then(|r| r.outputs.first().copied()),
                self.run_level,
            ),
            _ => (None, None),
        };
        let run = self.current_run();
        MeasureState {
            phase: self.phase(),
            kind: self.plan.kind,
            active: self.active(),
            wants_capture: self.wants_capture(),
            capture_rate_hz: self.plan.capture_rate_hz,
            position: run.map_or(1, |r| r.position.saturating_add(1)),
            positions: self.plan.positions.max(1),
            run: u16::try_from(self.run.saturating_add(1)).unwrap_or(u16::MAX),
            runs: u16::try_from(self.plan.runs.len()).unwrap_or(u16::MAX),
            output,
            level_db,
            ramp_levels: self.levels.clone(),
            run_level_db: self.run_level,
            frames: self.frames,
            generator_start_frame: self.start_frame,
            generator_starts: self.starts,
            progress: self.progress.clamp(0.0, 1.0),
            issue: self.issue.clone(),
            timing_reliable: self.timing_reliable,
            result_ready: self.document.is_some(),
            timing: self.timing.clone(),
            frame_faults: self.frame_faults,
            actor: self.actor,
        }
    }

    // -- WP ROLL1 --
    /// Who started the session (shown in its state).
    pub(crate) fn set_actor(&mut self, actor: crate::actor::ActorId) {
        self.actor = actor;
    }

    // ── Ending ─────────────────────────────────────────────────────────

    /// Abort (S4, S5): one `STOP_NOW` where the device is reachable, the run
    /// discarded, nothing else sent.
    fn abort(&mut self, dev: &mut dyn Device, issue: MeasureIssue) {
        if self.finished() {
            return;
        }
        if let Err(e) = dev.stop_now() {
            log::debug!("measurement abort: STOP_NOW failed: {e}");
        }
        self.generator = GenView::default();
        self.playing = None;
        self.buf = Vec::new();
        self.issue = Some(issue);
        self.stage = Stage::Aborted;
    }

    /// Stop and ask (the ramp failed closed, no signal): the generator is
    /// stopped if it plays; nothing is kept.
    fn fail(&mut self, dev: &mut dyn Device, issue: MeasureIssue) {
        if self.finished() {
            return;
        }
        if self.generator.playing && dev.stop_now().is_err() {
            log::debug!("measurement: STOP_NOW failed");
        }
        self.generator = GenView::default();
        self.playing = None;
        self.buf = Vec::new();
        self.issue = Some(issue);
        self.stage = Stage::Failed;
    }

    /// The user's Stop. True when it changed anything.
    pub(crate) fn stop_by_user(&mut self, dev: &mut dyn Device) -> bool {
        if self.finished() {
            return false;
        }
        self.abort(dev, issue("measure.stopped", "stopped"));
        true
    }

    // ── The push path ──────────────────────────────────────────────────

    /// One block of capture (see `DspiCore::measure_capture_push`).
    pub(crate) fn push(
        &mut self,
        dev: &mut dyn Device,
        first_frame: u64,
        samples: &[f32],
        rate_hz: u32,
        clipped: bool,
        _now: Instant,
    ) {
        if !self.wants_capture() || samples.is_empty() {
            return;
        }
        if rate_hz != self.plan.capture_rate_hz {
            self.abort(
                dev,
                issue(
                    "measure.abort.capture_rate",
                    format!(
                        "the microphone runs at {rate_hz} Hz, the measurement at {} Hz",
                        self.plan.capture_rate_hz
                    ),
                ),
            );
            return;
        }
        // S5: frame continuity.
        if let Some(want) = self.next_frame
            && want != first_frame
        {
            let what = if first_frame > want { "gap" } else { "repeat" };
            self.frame_faults = self.frame_faults.saturating_add(1);
            self.abort(
                dev,
                issue(
                    "measure.abort.frame_gap",
                    format!(
                        "a {what} in the capture: frame {first_frame} where {want} was expected"
                    ),
                ),
            );
            return;
        }
        let len = samples.len() as u64;
        self.next_frame = Some(first_frame.saturating_add(len));
        self.frames = self.frames.saturating_add(len);
        if matches!(self.stage, Stage::Analyse(_)) {
            // The generator is idle; the frames only keep the count.
            return;
        }
        // S4: a clip.
        let clip = safety::clip_linear();
        let clipping = clipped || samples.iter().any(|s| s.is_nan() || s.abs() >= clip);
        // S3, S4: the SPL cap.
        let spl = match (
            self.spl.as_mut(),
            self.plan.mic.as_ref().and_then(|m| m.spl_offset_db),
        ) {
            (Some(meter), Some(offset)) => meter.push(samples).map(|db| db + offset),
            _ => None,
        };
        let over_cap = match (spl, self.plan.spl_cap_db) {
            (Some(db), Some(cap)) => db >= f64::from(cap),
            _ => false,
        };
        if let Stage::RampStep {
            clipped: c, loud, ..
        } = &mut self.stage
        {
            if (clipping || over_cap) && self.generator.playing {
                *c |= clipping;
                *loud |= over_cap;
                if dev.stop_now().is_ok() {
                    self.generator.playing = false;
                    self.generator.stopping = true;
                }
            } else {
                *c |= clipping;
                *loud |= over_cap;
            }
        } else if clipping {
            self.abort(
                dev,
                issue(
                    "measure.abort.capture_clip",
                    "the microphone's capture clipped",
                ),
            );
            return;
        } else if over_cap {
            self.abort(
                dev,
                issue(
                    "measure.abort.spl",
                    format!(
                        "the level reached the SPL cap of {:.0} dB SPL",
                        self.plan.spl_cap_db.unwrap_or_default()
                    ),
                ),
            );
            return;
        }
        if self.buf.is_empty() {
            self.buf_first = first_frame;
        }
        if self.buf.len().saturating_add(samples.len()) > MAX_CAPTURE_SAMPLES {
            self.abort(
                dev,
                issue("measure.too_long", "the capture passed its bound"),
            );
            return;
        }
        self.buf.extend_from_slice(samples);
    }

    // ── The tick ───────────────────────────────────────────────────────

    pub(crate) fn tick(&mut self, dev: &mut dyn Device, now: Instant) {
        if self.finished() {
            return;
        }
        // S5: what the notification path saw.
        for e in dev.drain_tap() {
            match safety::judge_event(e, &mut self.generator) {
                Ok(true) => {
                    if let Some(p) = self.playing.as_mut() {
                        p.ended = true;
                    }
                }
                Ok(false) => {}
                Err(i) => {
                    self.abort(dev, i);
                    return;
                }
            }
            if e == TapEvent::Lost {
                // judge_event aborted already.
                return;
            }
        }
        // The lease (a dead-man switch: it runs out if the client dies).
        if !dev.renew_lock(now.saturating_duration_since(self.last_renew) >= RENEW_EVERY) {
            self.abort(dev, issue("measure.abort.lock", "the device lock was lost"));
            return;
        }
        if now.saturating_duration_since(self.last_renew) >= RENEW_EVERY {
            self.last_renew = now;
        }
        // S5: the run taking 5 s longer than planned.
        if let Some(p) = self.playing
            && now > p.deadline
        {
            self.abort(
                dev,
                issue("measure.abort.overrun", "the run took longer than planned"),
            );
            return;
        }
        // About 4 Hz: the generator, the meters.
        let due = self
            .last_poll
            .is_none_or(|t| now.saturating_duration_since(t) >= POLL_EVERY);
        if due
            && matches!(
                self.stage,
                Stage::PreRoll { .. } | Stage::RampStep { .. } | Stage::Run
            )
        {
            self.last_poll = Some(now);
            if !self.poll(dev) {
                return;
            }
        }
        self.advance(dev, now);
    }

    /// The polls; false when the session ended.
    fn poll(&mut self, dev: &mut dyn Device) -> bool {
        if self.generator.playing {
            match dev.status() {
                Ok(st) => {
                    self.poll_failures = 0;
                    if let Err(i) = self.check_status(&st) {
                        self.abort(dev, i);
                        return false;
                    }
                }
                Err(e) => {
                    if !self.poll_failed(dev, &e) {
                        return false;
                    }
                }
            }
        }
        match dev.meters() {
            Ok(m) => {
                self.poll_failures = 0;
                if let Err(i) = self.check_meters(&m) {
                    self.abort(dev, i);
                    return false;
                }
            }
            Err(e) => {
                if !self.poll_failed(dev, &e) {
                    return false;
                }
            }
        }
        if !self.limiter_outputs.is_empty() {
            match dev.limiter_gr() {
                Ok(gr) => {
                    if let Err(i) = self.check_limiters(&gr) {
                        self.abort(dev, i);
                        return false;
                    }
                }
                Err(e) => {
                    if !self.poll_failed(dev, &e) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn poll_failed(&mut self, dev: &mut dyn Device, e: &DspiError) -> bool {
        self.poll_failures = self.poll_failures.saturating_add(1);
        if matches!(e, DspiError::Disconnected) || self.poll_failures >= MAX_POLL_FAILURES {
            self.abort(
                dev,
                issue(
                    "measure.abort.disconnected",
                    format!("the DSPi does not answer: {e}"),
                ),
            );
            return false;
        }
        true
    }

    /// Cycle identity during a walked run (RC5b): the polled walked output and
    /// cycle count must agree with where the capture is.
    fn check_status(&self, st: &SiggenStatus) -> Result<(), MeasureIssue> {
        let (Some(p), Some(schedule), Some(next)) =
            (self.playing, self.schedule.as_ref(), self.next_frame)
        else {
            return Ok(());
        };
        if !p.walked || st.active_channel == NO_CHANNEL {
            // An idle generator is judged by its stop event.
            return Ok(());
        }
        let ratio =
            f64::from(self.ctx.device_rate_hz) / f64::from(self.plan.capture_rate_hz.max(1));
        let est = next.saturating_sub(p.start) as f64 * ratio;
        let slack = STATUS_SLACK_MS * f64::from(self.ctx.device_rate_hz) / 1000.0;
        // The firmware counts a cycle and steps the walk at the cycle's end
        // (`siggen.c` `segment_finished`), as the schedule models it and the
        // demo device plays it (review P5-B, decision b): no further slack.
        let observed = GeneratorStatus {
            active_channel: st.active_channel,
            cycles_done: st.cycles_done,
        };
        if schedule.status_consistent(est - slack, est + slack, observed) {
            Ok(())
        } else {
            Err(issue(
                "measure.abort.cycle_identity",
                format!(
                    "the generator plays output {} after {} cycles where the capture expects \
                     another: the capture does not line up with the run",
                    st.active_channel.saturating_add(1),
                    st.cycles_done
                ),
            ))
        }
    }

    fn check_meters(&mut self, m: &crate::records::SystemStatus) -> Result<(), MeasureIssue> {
        let n_inputs = usize::from(self.ctx.num_inputs);
        // S8 during the run: programme audio on an input.
        let silence = safety::linear(super::preflight::INPUT_SILENCE_DBFS);
        if let Some((i, _)) = m
            .peaks
            .iter()
            .take(n_inputs)
            .enumerate()
            .find(|(_, p)| p.is_nan() || **p > silence)
        {
            return Err(issue(
                "measure.abort.programme",
                format!("programme audio appeared on input {}", i.saturating_add(1)),
            ));
        }
        // An input stream starting or stopping.
        match self.active_inputs {
            Some(a) if a != m.active_input_channels => {
                return Err(issue(
                    "measure.abort.input_stream",
                    "an input stream started or stopped",
                ));
            }
            _ => self.active_inputs = Some(m.active_input_channels),
        }
        // S4: a measured output's peak.
        let peak = safety::linear(OUTPUT_PEAK_DBFS);
        for spec in &self.plan.outputs {
            let at = n_inputs.saturating_add(usize::from(spec.output));
            if m.peaks.get(at).is_some_and(|p| p.is_nan() || *p >= peak) {
                return Err(MeasureIssue::new(
                    "measure.abort.output_peak",
                    Some(spec.output),
                    format!(
                        "output {} reached full scale",
                        spec.output.saturating_add(1)
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_limiters(&mut self, gr: &[f32]) -> Result<(), MeasureIssue> {
        let base = self.gr_base.get_or_insert_with(|| gr.to_vec());
        for o in &self.limiter_outputs {
            let i = usize::from(*o);
            let now = gr.get(i).copied().unwrap_or(0.0);
            let was = base.get(i).copied().unwrap_or(0.0);
            if now.is_finite() && (now - was).abs() > LIMITER_ACTS_DB && now > LIMITER_ACTS_DB {
                return Err(MeasureIssue::new(
                    "measure.abort.limiter",
                    Some(*o),
                    format!("output {}'s limiter acted", o.saturating_add(1)),
                ));
            }
        }
        Ok(())
    }
}

// ── The stages ─────────────────────────────────────────────────────────

impl Session {
    fn rate(&self) -> u32 {
        self.plan.capture_rate_hz
    }

    /// Stage `cfg` and play it; the capture frame of the reply is the run's
    /// timing anchor (RC5b). False when the session ended.
    fn start_gen(
        &mut self,
        dev: &mut dyn Device,
        cfg: &SiggenConfig,
        after_ms: u64,
        walked: bool,
        now: Instant,
    ) -> bool {
        // Never above the plan's ceiling, whatever built `cfg` (S3; the
        // device's own check only knows the speakers' hard cap).
        if cfg.level_db.is_nan() || cfg.level_db > self.plan.ceiling_db {
            self.abort(
                dev,
                issue(
                    "measure.abort.generator",
                    format!(
                        "refused a generator level of {} dBFS above the ceiling of {} dBFS",
                        cfg.level_db, self.plan.ceiling_db
                    ),
                ),
            );
            return false;
        }
        // -- WP ROLL1 -- the hard interlock (owner 2026-11-22): nothing below
        // 1 kHz reaches a high way, whatever built `cfg`.
        if let Some(why) = below_floor(&self.plan, cfg) {
            self.abort(dev, issue("measure.abort.generator", why));
            return false;
        }
        if let Err(e) = dev.start(cfg) {
            let key = if matches!(e, DspiError::Disconnected) {
                "measure.abort.disconnected"
            } else {
                "measure.abort.generator"
            };
            self.abort(dev, issue(key, format!("the generator did not start: {e}")));
            return false;
        }
        let start = self.next_frame.unwrap_or(0);
        self.starts = self.starts.saturating_add(1);
        self.start_frame = Some(start);
        self.generator = GenView {
            playing: true,
            stopping: false,
            starts_pending: self.generator.starts_pending.saturating_add(1),
        };
        let frames = after_ms.saturating_mul(u64::from(self.rate())) / 1000;
        let planned = Duration::from_millis(after_ms);
        self.playing = Some(Playing {
            start,
            end: start.saturating_add(frames),
            deadline: now
                .checked_add(planned.saturating_add(OVERRUN))
                .unwrap_or(now),
            ended: false,
            walked,
        });
        if let Some(m) = self.spl.as_mut() {
            m.reset_max();
        }
        true
    }

    /// A ramp step of `ramp.output` at `level`.
    fn start_step(&mut self, dev: &mut dyn Device, ramp: Ramp, level: f32, now: Instant) {
        let Some(run) = self.current_run().cloned() else {
            self.finish(dev);
            return;
        };
        let cfg = runs::ramp_config(&self.plan, &run, ramp.output, level); // -- WP ROLL1 --
        let stim_ms = if self.plan.kind == MeasureRunKind::Pink {
            RAMP_PINK_MS
        } else {
            RAMP_SWEEP_MS
        };
        self.buf.clear();
        self.stage = Stage::RampStep {
            ramp,
            level,
            clipped: false,
            loud: false,
        };
        let after = u64::from(stim_ms.saturating_add(RAMP_TAIL_MS));
        self.start_gen(dev, &cfg, after, false, now);
    }

    /// The ramp of the run's output `idx`, or the run's pre-roll when every
    /// output has its level.
    fn next_ramp(&mut self, dev: &mut dyn Device, now: Instant) {
        let Some(run) = self.current_run().cloned() else {
            self.finish(dev);
            return;
        };
        match run.outputs.get(self.ramp_idx).copied() {
            Some(output) => {
                let ramp = Ramp::new(
                    output,
                    self.plan.start_db,
                    self.plan.ceiling_db,
                    self.plan.spl_cap_db,
                    self.plan.start_level_only,
                );
                let first = ramp.first();
                self.start_step(dev, ramp, first, now);
            }
            None => {
                let levels: Vec<f32> = self.levels.iter().map(|l| l.level_db).collect();
                self.run_level = Some(runs::run_level(&self.plan, &levels, run.outputs.len()));
                self.buf.clear();
                self.stage = Stage::PreRoll { after: After::Run };
            }
        }
    }

    fn start_run(&mut self, dev: &mut dyn Device, now: Instant) {
        let Some(run) = self.current_run().cloned() else {
            self.finish(dev);
            return;
        };
        let level = self.run_level.unwrap_or(self.plan.start_db);
        let cfg = runs::run_config(&self.plan, &run, level);
        let rate = self.ctx.device_rate_hz;
        let after_ms = if self.plan.kind == MeasureRunKind::Pink {
            u64::from(self.plan.pink_ms.saturating_add(PINK_POST_MS))
        } else {
            match self.plan.schedule(&run, rate) {
                Ok(s) => {
                    let ms = self
                        .plan
                        .run_ms(&run, rate)
                        .unwrap_or(0)
                        .saturating_sub(u64::from(PREROLL_MS));
                    self.schedule = Some(s);
                    ms
                }
                Err(e) => {
                    self.abort(dev, issue("measure.abort.generator", e.to_string()));
                    return;
                }
            }
        };
        let walked = self.plan.kind == MeasureRunKind::Walked;
        self.stage = Stage::Run;
        self.start_gen(dev, &cfg, after_ms, walked, now);
    }

    /// Move the stages on: whatever is complete is judged and the next one
    /// begins; at most one analysis slice.
    fn advance(&mut self, dev: &mut dyn Device, now: Instant) {
        let rate = self.rate();
        match &mut self.stage {
            Stage::PreRoll { after } => {
                let after = *after;
                let want = usize::try_from(frames_of(PREROLL_MS, rate)).unwrap_or(usize::MAX);
                self.progress = self.buf.len() as f32 / want.max(1) as f32;
                if self.buf.len() < want {
                    return;
                }
                match after {
                    After::Ramp => {
                        // -- WP ROLL1 -- the run's own band
                        let heard = match self.plan.runs.get(self.run) {
                            Some(run) => safety::ramp_listener(
                                &self.plan,
                                run,
                                self.ctx.device_rate_hz,
                                &self.buf,
                            ),
                            None => Err("no run".to_owned()),
                        };
                        self.listener = match heard {
                            Ok(l) => Some(l),
                            Err(e) => {
                                log::warn!("measure: the ramp hears nothing: {e}");
                                None
                            }
                        };
                        self.ramp_idx = 0;
                        self.levels.clear();
                        self.ramp_steps.clear();
                        self.next_ramp(dev, now);
                    }
                    After::Run => self.start_run(dev, now),
                }
            }
            Stage::RampStep { .. } => self.advance_ramp(dev, now),
            Stage::Run => {
                let Some(p) = self.playing else {
                    return;
                };
                let span = p.end.saturating_sub(p.start).max(1);
                let got = self.next_frame.unwrap_or(0).saturating_sub(p.start);
                self.progress = got as f32 / span as f32;
                if !p.ended || self.next_frame.unwrap_or(0) < p.end {
                    return;
                }
                self.playing = None;
                self.begin_analysis(dev);
            }
            Stage::Analyse(_) => self.analyse_slice(dev),
            Stage::WaitPosition | Stage::Done | Stage::Aborted | Stage::Failed => {}
        }
    }

    fn advance_ramp(&mut self, dev: &mut dyn Device, now: Instant) {
        let Some(p) = self.playing else {
            return;
        };
        let complete = p.ended && self.next_frame.unwrap_or(0) >= p.end;
        let Stage::RampStep {
            ramp,
            level,
            clipped,
            loud,
        } = &mut self.stage
        else {
            return;
        };
        // A step the push path stopped (a clip, the SPL cap) is judged once
        // its stop is confirmed; any other once its tail is captured.
        if !p.ended || (!complete && !(*clipped || *loud)) {
            return;
        }
        let spl = match (self.plan.spl_cap_db, *loud) {
            (Some(cap), true) => Some(f64::from(cap)),
            _ => self
                .spl
                .as_ref()
                .and_then(|m| m.max_db)
                .zip(self.plan.mic.as_ref().and_then(|m| m.spl_offset_db))
                .map(|(db, off)| db + off),
        };
        let (snr_db, in_band_db) = safety::step_hearing(self.listener.as_ref(), &self.buf);
        log::info!(
            "measure: ramp step output {} at {} dBFS: in-band SNR {snr_db:.1} dB",
            ramp.output,
            *level
        );
        let eval = StepEval {
            level_db: *level,
            snr_db,
            in_band_db,
            clipped: *clipped,
            spl_db: spl,
        };
        self.ramp_steps.push(StepRecord {
            output: ramp.output,
            level_db: *level,
            snr_db,
        });
        let mut ramp = ramp.clone();
        let next = ramp.judge(eval);
        self.playing = None;
        match next {
            RampNext::Step(l) => self.start_step(dev, ramp, l, now),
            RampNext::Done(l) => {
                self.levels.push(MeasureLevel {
                    output: ramp.output,
                    level_db: l,
                });
                self.ramp_idx = self.ramp_idx.saturating_add(1);
                self.next_ramp(dev, now);
            }
            RampNext::Ask(i) => self.fail(dev, i),
        }
    }

    fn begin_analysis(&mut self, dev: &mut dyn Device) {
        let rate = self.rate();
        if self.plan.kind == MeasureRunKind::Pink {
            let mic = self.plan.mic.as_ref().map(|m| m.correction.clone());
            let offset = self.plan.mic.as_ref().and_then(|m| m.spl_offset_db);
            let cfg = RtaConfig {
                sample_rate: f64::from(rate),
                fft_len: 16_384,
                bands_per_octave: 12,
                averaging: Averaging::Infinite,
            };
            match MicRta::new(cfg, mic.as_ref(), offset) {
                Ok(rta) => {
                    let skip = usize::try_from(
                        self.start_frame
                            .unwrap_or(self.buf_first)
                            .saturating_sub(self.buf_first)
                            .saturating_add(frames_of(PINK_SKIP_MS, rate)),
                    )
                    .unwrap_or(usize::MAX);
                    self.stage = Stage::Analyse(Analysis::Pink {
                        rta: Box::new(rta),
                        fed: skip,
                    });
                }
                Err(e) => self.fail(dev, issue("measure.no_signal", e.to_string())),
            }
            return;
        }
        let Some(schedule) = self.schedule.as_ref() else {
            self.fail(dev, issue("measure.no_signal", "no schedule"));
            return;
        };
        match WalkedAnalyser::new(schedule, rate, &AnalysisOptions::default()) {
            Ok(a) => self.stage = Stage::Analyse(Analysis::Walked(Box::new(a), (0, 1))),
            Err(e) => self.fail(dev, issue("measure.no_signal", e.to_string())),
        }
    }

    fn analyse_slice(&mut self, dev: &mut dyn Device) {
        let rate = self.rate();
        let position = self.current_run().map_or(0, |r| r.position);
        let level = self.run_level.unwrap_or(self.plan.start_db);
        let Stage::Analyse(analysis) = &mut self.stage else {
            return;
        };
        let outcome: Option<std::result::Result<RunResult, MeasureIssue>> = match analysis {
            Analysis::Walked(a, progress) => {
                let capture = WalkedCapture {
                    samples: &self.buf,
                    first_frame: self.buf_first,
                    rate_hz: rate,
                    start_frame: self.start_frame.unwrap_or(self.buf_first),
                    status: &[],
                    complete: true,
                };
                match a.step(&capture) {
                    Ok(Step::Progress { done, total }) => {
                        *progress = (done, total.max(1));
                        None
                    }
                    Ok(Step::NeedCapture) => Some(Err(issue(
                        "measure.no_signal",
                        "the capture ended before the run",
                    ))),
                    Ok(Step::Done(r)) => Some(
                        runs::walked_result(&r, position, level, &self.plan)
                            .map_err(|e| issue("measure.no_signal", e.to_string())),
                    ),
                    // A single sweep not heard where its START puts it is
                    // a sweep not heard; a walked run's cycles are its
                    // identity (RC5b).
                    Err(dspi_measure::ir::TimingError::CycleIdentity(m)) => {
                        Some(Err(if self.plan.kind == MeasureRunKind::Walked {
                            issue(
                                "measure.abort.cycle_identity",
                                format!("the capture does not line up with the run: {m}"),
                            )
                        } else {
                            issue("measure.no_signal", format!("the sweep was not heard: {m}"))
                        }))
                    }
                    Err(e) => Some(Err(issue("measure.no_signal", e.to_string()))),
                }
            }
            Analysis::Pink { rta, fed } => {
                let end = usize::try_from(
                    self.start_frame
                        .unwrap_or(self.buf_first)
                        .saturating_sub(self.buf_first)
                        .saturating_add(frames_of(self.plan.pink_ms, rate)),
                )
                .unwrap_or(usize::MAX)
                .min(self.buf.len());
                let slice = usize::try_from(frames_of(PINK_SLICE_MS, rate)).unwrap_or(1);
                let to = fed.saturating_add(slice).min(end);
                if let Some(chunk) = self.buf.get(*fed..to)
                    && let Err(e) = rta.push(chunk)
                {
                    log::debug!("pink analysis: {e}");
                }
                *fed = to;
                if *fed < end {
                    None
                } else {
                    let output = self
                        .plan
                        .runs
                        .get(self.run)
                        .and_then(|r| r.outputs.first().copied())
                        .unwrap_or(0);
                    Some(match rta.spectrum() {
                        Some(s) => runs::pink_result(&s, output, level)
                            .map_err(|e| issue("measure.no_signal", e.to_string())),
                        None => Err(issue("measure.no_signal", "no pink noise was captured")),
                    })
                }
            }
        };
        if let Stage::Analyse(Analysis::Walked(_, (d, t))) = &self.stage {
            self.progress = *d as f32 / (*t).max(1) as f32;
        }
        match outcome {
            None => {}
            Some(Ok(mut r)) => {
                r.ramp_steps.clone_from(&self.ramp_steps);
                // -- WP ROLL1 --
                if let Some(run) = self.plan.runs.get(self.run) {
                    r.f1_hz = if self.plan.kind == MeasureRunKind::Pink {
                        0.0
                    } else {
                        self.plan.sweep_of(run).f1_hz
                    };
                    r.aligns = run.aligns;
                }
                if self.run == 0 {
                    self.timing_reliable = Some(r.timing_reliable);
                }
                // -- WP ROLL1 -- a non-tweeters' full-band run beside its
                // group's is not the group's timing
                if r.timing.is_some() && r.aligns {
                    self.timing.clone_from(&r.timing);
                }
                self.results.push(r);
                self.buf = Vec::new();
                self.schedule = None;
                self.next_run(dev);
            }
            Some(Err(i)) if i.key == "measure.abort.cycle_identity" => self.abort(dev, i),
            Some(Err(i)) => self.fail(dev, i),
        }
    }

    /// After a run: the next one (its own ramp), a position change, or the
    /// document.
    fn next_run(&mut self, dev: &mut dyn Device) {
        let here = self.current_run().map(|r| r.position);
        self.run = self.run.saturating_add(1);
        self.ramp_idx = 0;
        self.levels.clear();
        self.ramp_steps.clear();
        self.run_level = None;
        match self.current_run().map(|r| r.position) {
            None => self.finish(dev),
            Some(p) if Some(p) != here => {
                self.stage = Stage::WaitPosition;
                // The microphone may be stopped while it is moved.
                self.next_frame = None;
            }
            Some(_) => self.stage = Stage::PreRoll { after: After::Ramp },
        }
    }

    /// The document of every run.
    fn finish(&mut self, dev: &mut dyn Device) {
        let secs = dspi_transport::clock::system_time()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ctx = runs::DocContext {
            serial: &self.ctx.serial,
            platform: &self.ctx.platform,
            device_rate_hz: self.ctx.device_rate_hz,
            chains: &self.ctx.chains,
            saved_ms: i64::try_from(secs.saturating_mul(1000)).unwrap_or(i64::MAX),
            saved_utc: runs::utc_text(secs),
        };
        match runs::document(&self.plan, &self.results, &ctx) {
            Ok(text) => {
                self.document = Some(text);
                self.progress = 1.0;
                self.stage = Stage::Done;
            }
            Err(e) => self.fail(dev, issue("measure.no_signal", e.to_string())),
        }
    }

    /// The user moved the microphone (or reseated the headphones).
    pub(crate) fn next_position(&mut self, _now: Instant) -> crate::Result<()> {
        if !matches!(self.stage, Stage::WaitPosition) {
            return Err(DspiError::InvalidArgument {
                msg: "the measurement is not waiting for a position".into(),
            });
        }
        self.buf.clear();
        self.next_frame = None;
        self.active_inputs = None;
        self.stage = Stage::PreRoll { after: After::Ramp };
        Ok(())
    }
}
