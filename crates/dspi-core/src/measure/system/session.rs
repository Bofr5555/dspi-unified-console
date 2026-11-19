//! The system session's state machine (WP ATCORE, PLAN-AT 1.4, 1.5): the
//! pre-roll, each unit's ramp through the hub (S3), the timed runs, the
//! analysis in slices, the gate, the summed runs. It talks to the hub only
//! through [`super::Hub`], so its tests play the hub.
//!
//! **One timed run** (a ramp step, a walked run, a summed run): every unit's
//! config is judged (`runs::check_config`, `stimulus::check_timed_config`)
//! and staged (`system.measure.stage`), a status read gives a fresh time pin
//! and each unit's `run_id`, then `system.measure.run` places the segments
//! on the domain's counter (the hub keeps one START pending per unit). The
//! status polls (about 4 Hz, renewing the lease) judge every unit (a clip,
//! an output peak, a limiter, a time-base disturbance or loss, an event, a
//! refused or cancelled START), note a time pin per reply and every
//! segment's proof (`EXT_SIGGEN_TIMELINE` once it has ended).

use std::time::Duration;

use serde_json::{Value, json};

use dspi_measure::hearing::Listener;
use dspi_measure::ir::{AnalysisOptions, MAX_CAPTURE_SAMPLES, StatusObservation, TimingError};
use dspi_measure::stimulus::{GeneratorStatus, LogSweep, NO_CHANNEL, SweepConfig, WalkSchedule};
use dspi_measure::system::messages::{
    self as msg, AbortRequest, OpenReply, OpenRequest, OpenUnit, RunReply, RunRequest, RunSegment,
    SessionRequest, StageRequest, StatusReply, UnitStatus,
};
use dspi_measure::system::{
    DomainId, GateFinding, SegmentProof, SystemAnalyser, SystemAnalysis, SystemCapture,
    SystemSchedule, SystemStep, TimePin, UnitFacts, UnitIndex, UnitStatusObservation, check_gate,
};
use dspi_proto::ext::timebase::{
    SiggenTimeline, TB_STATE_LOCKED, TL_RUNNING, TimeBaseEntry, timeline_result,
};
use dspi_transport::clock::Instant;

use super::super::plan::PREROLL_MS;
use super::super::runs::{self, RAMP_SWEEP_MS};
use super::super::safety::{
    self, LIMITER_ACTS_DB, OUTPUT_PEAK_DBFS, Ramp, RampNext, SplMeter, StepEval,
};
use super::plan::{LEAD_MARGIN_MS, SetupPlan};
use super::proposal::{Outcome, RunDone, SumJob, Terms, Verdict};
use super::{
    Hub, MeasureSystemIssue, MeasureSystemLevel, MeasureSystemPhase, MeasureSystemPurpose,
    MeasureSystemState,
};
use crate::records::SiggenConfig;
use crate::{DspiError, Result};

/// How often the hub's status is read (renewing the lease).
const POLL_EVERY: Duration = Duration::from_millis(250);
// -- WP ATDEMO --
/// When to read the engine's terms again after `system.status` was rate
/// limited (the hub's once a second per connection).
const TERMS_RETRY: Duration = Duration::from_millis(1_100);
// -- WP ATLATE --
/// The longest a walked run waits for the engine's terms (the hub's rate
/// limit of `system.status`) before it is planned without them.
const TERMS_WAIT: Duration = Duration::from_secs(5);
/// Failed status reads in a row before the session gives up.
const MAX_POLL_FAILURES: u32 = 4;
/// Capture kept after a step's or a run's end (the input latency), ms.
const TAIL_MS: u32 = 600;
/// A run this much later than planned aborts (S5).
const OVERRUN: Duration = Duration::from_secs(5);
/// How long after a run's capture is complete its proofs may still come.
const PROOF_GRACE: Duration = Duration::from_secs(2);
/// Passes of a summed run: one, as P5's (two equal arrivals of a summed
/// cycle are two equal correlation peaks: repeats would be aligned on
/// either and their average smeared).
pub(crate) const SUM_PASSES: u16 = 1;

/// The options of every system analysis: P5's defaults, never widened
/// (review AT-A/AT-B: `delay_ms` is the envelope-peak estimator; only within
/// P5's gate does it agree with the intercepts' uncertainty).
pub(crate) fn analysis_options() -> AnalysisOptions {
    AnalysisOptions::default()
}

fn issue(serial: &str, key: &str, msg: impl Into<String>) -> MeasureSystemIssue {
    MeasureSystemIssue::new(serial, key, None, msg)
}

/// What follows a pre-roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Ramp,
    Run,
}

/// What a timed run is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimedKind {
    Ramp,
    Walk(usize),
    Sum(usize),
}

/// A timed run to start: what for, each unit's config, the segments.
type Built = (TimedKind, Vec<(usize, SiggenConfig)>, SystemSchedule);

/// A timed run in flight.
struct Timed {
    kind: TimedKind,
    schedule: SystemSchedule,
    /// `(unit, domain T minus unit T)`.
    origins: Vec<(usize, i64)>,
    /// `(segment, unit, expected run_id)`.
    expected: Vec<(usize, usize, u32)>,
    /// Proofs read, `(proof, final)`.
    proofs: Vec<(SegmentProof, bool)>,
    end_frame: u64,
    deadline: Instant,
    complete_at: Option<Instant>,
    pins: Vec<TimePin>,
    status: Vec<UnitStatusObservation>,
    /// `(unit, staged level)`.
    levels: Vec<(usize, f32)>,
}

enum Stage {
    PreRoll {
        after: After,
    },
    Ramp {
        unit: usize,
        ramp: Ramp,
        level: f32,
        timed: Option<Timed>,
    },
    Run {
        timed: Timed,
    },
    Analyse {
        analyser: Box<SystemAnalyser>,
        timed: Timed,
        facts: Vec<UnitFacts>,
    },
    Done,
    Aborted,
    Failed,
}

/// One unit during the session.
#[derive(Debug, Clone)]
struct UnitRun {
    open: OpenUnit,
    gr_base: Option<Vec<f32>>,
    entry: Option<TimeBaseEntry>,
    output_frames: u32,
    timeline: Option<SiggenTimeline>,
    events: u32,
    tick_offset: i64,
    tick_round_trip: u64,
}

/// What a finished session leaves.
pub(crate) enum SessionOutput {
    /// A measurement's outcome.
    Measured(Box<Outcome>),
    /// The summed runs' verdicts.
    Verdicts(Vec<Verdict>),
}

/// A system session.
pub(crate) struct SysSession {
    plan: SetupPlan,
    purpose: MeasureSystemPurpose,
    jobs: Vec<SumJob>,
    session: String,
    domain: DomainId,
    lead_ms: u32,
    units: Vec<UnitRun>,
    stage: Stage,
    next_frame: Option<u64>,
    frames: u64,
    buf: Vec<f32>,
    buf_first: u64,
    listener: Option<Listener>,
    spl: Option<SplMeter>,
    last_poll: Option<Instant>,
    poll_failures: u32,
    pin: Option<TimePin>,
    run_idx: usize,
    ramp_queue: Vec<(usize, u8)>,
    ramp_pos: usize,
    levels: Vec<(usize, u8, f32)>,
    terms: Vec<Option<Terms>>,
    done_runs: Vec<RunDone>,
    verdicts: Vec<Verdict>,
    issue: Option<MeasureSystemIssue>,
    progress: f32,
    output: Option<SessionOutput>,
    timing_reliable: Option<bool>,
    // -- WP ATDEMO --
    /// The engine's terms are to be read again then (`system.status` was
    /// rate limited: another view of this connection read it just before).
    terms_retry: Option<Instant>,
    // -- WP ATLATE -- until when a walked run waits for the terms.
    terms_wait_until: Option<Instant>,
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn parse<T: serde::de::DeserializeOwned>(v: Value, what: &str) -> Result<T> {
    serde_json::from_value(v).map_err(|e| DspiError::Link {
        code: "protocol".into(),
        msg: format!("the hub's {what} reply cannot be read: {e}"),
    })
}

/// A record's config as the 36 bytes `system.measure.stage` carries.
pub(crate) fn config_bytes(c: &SiggenConfig) -> Result<Vec<u8>> {
    let p = |i: usize| c.p.get(i).copied().unwrap_or(0.0);
    let w = dspi_proto::packets::SiggenConfig {
        version: 1,
        signal_type: c.signal_type,
        channel_mask: c.channel_mask,
        invert_mask: c.invert_mask,
        flags: c.flags,
        level_db: c.level_db,
        duration_ms: c.duration_ms,
        repeat_count: c.repeat,
        gap_ms: c.gap_ms,
        p1: p(0),
        p2: p(1),
        p3: p(2),
        p4: p(3),
    };
    // The one rule of op 9 (review AT-A/AT-B): a finite log sweep only.
    dspi_measure::stimulus::check_timed_config(&w, 0)
        .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
    Ok(w.encode().to_vec())
}

impl SysSession {
    /// Open the hub's session over the plan's units (`system.measure.open`)
    /// and begin with the pre-roll. `jobs`: the summed runs of a sum check
    /// or a verify (their levels come from the measurement's ramps).
    pub(crate) fn open(
        plan: SetupPlan,
        purpose: MeasureSystemPurpose,
        jobs: Option<Vec<SumJob>>,
        hub: &mut dyn Hub,
        _now: Instant,
    ) -> Result<SysSession> {
        let req = OpenRequest {
            units: plan.units.iter().map(|u| u.serial.clone()).collect(),
            lease_ms: (plan.setup.lease_ms != 0).then_some(plan.setup.lease_ms),
        };
        let reply: OpenReply = parse(hub.request(msg::t::OPEN, to_value(&req))?, "open")?;
        let close = |hub: &mut dyn Hub, s: &str| {
            let _ = hub.request(
                msg::t::CLOSE,
                to_value(&SessionRequest { session: s.into() }),
            );
        };
        if reply.units.len() != plan.units.len()
            || reply
                .units
                .iter()
                .zip(&plan.units)
                .any(|(a, b)| a.serial != b.serial)
        {
            close(hub, &reply.session);
            return Err(DspiError::Link {
                code: "protocol".into(),
                msg: "the hub opened other units than asked".into(),
            });
        }
        if let Some(u) = reply.units.iter().find(|u| u.state != TB_STATE_LOCKED) {
            close(hub, &reply.session);
            return Err(DspiError::Link {
                code: msg::code::NOT_LOCKED.into(),
                msg: format!("{}: its time base is not locked", u.serial),
            });
        }
        let arm = reply.units.iter().map(|u| u.arm_lead_ms).max().unwrap_or(0);
        let lead_ms = u32::from(arm)
            .saturating_add(LEAD_MARGIN_MS)
            .max(plan.setup.lead_ms);
        let jobs = jobs.unwrap_or_default();
        // Every session ramps (S3): a sum check or a verify its own
        // outputs again (review AT-C), not at the levels an earlier
        // session's ramps found.
        let after = After::Ramp;
        let spl = plan
            .units
            .first()
            .and_then(|u| u.plan.spl_cap_db)
            .map(|_| SplMeter::new(plan.capture_rate_hz));
        let ramp_queue: Vec<(usize, u8)> = match purpose {
            MeasureSystemPurpose::Measure => plan
                .units
                .iter()
                .enumerate()
                .flat_map(|(i, u)| u.plan.outputs.iter().map(move |o| (i, o.output)))
                .collect(),
            _ => {
                let mut q: Vec<(usize, u8)> = Vec::new();
                for j in &jobs {
                    for x in [(j.a.0, j.a.1), (j.b.0, j.b.1)] {
                        if !q.contains(&x) {
                            q.push(x);
                        }
                    }
                }
                q
            }
        };
        let n = plan.units.len();
        Ok(SysSession {
            domain: DomainId {
                time_base: reply.time_base,
                id: reply.domain,
            },
            session: reply.session,
            units: reply
                .units
                .into_iter()
                .map(|open| UnitRun {
                    open,
                    gr_base: None,
                    entry: None,
                    output_frames: 0,
                    timeline: None,
                    events: 0,
                    tick_offset: 0,
                    tick_round_trip: 0,
                })
                .collect(),
            plan,
            purpose,
            jobs,
            lead_ms,
            stage: Stage::PreRoll { after },
            next_frame: None,
            frames: 0,
            buf: Vec::new(),
            buf_first: 0,
            listener: None,
            spl,
            last_poll: None,
            poll_failures: 0,
            pin: None,
            run_idx: 0,
            ramp_queue,
            ramp_pos: 0,
            levels: Vec::new(),
            terms: vec![None; n],
            done_runs: Vec::new(),
            verdicts: Vec::new(),
            issue: None,
            progress: 0.0,
            output: None,
            timing_reliable: None,
            terms_retry: None,      // -- WP ATDEMO --
            terms_wait_until: None, // -- WP ATLATE --
        })
    }

    // ── Queries ────────────────────────────────────────────────────────

    // -- WP ATDEMO --
    /// The hub's session id ("" once closed).
    pub(crate) fn hub_session(&self) -> &str {
        &self.session
    }

    // -- WP ATDEMO --
    /// The capture frame the next push must start at (0 before the first).
    pub(crate) fn next_capture_frame(&self) -> u64 {
        self.next_frame.unwrap_or(0)
    }

    // -- WP ATDEMO --
    /// End the session with `issue` (the simulated microphone refused).
    pub(crate) fn abort_with(&mut self, hub: &mut dyn Hub, issue: MeasureSystemIssue) {
        self.abort(hub, issue);
    }

    pub(crate) fn purpose(&self) -> MeasureSystemPurpose {
        self.purpose
    }

    fn phase(&self) -> MeasureSystemPhase {
        match &self.stage {
            Stage::PreRoll { .. } => MeasureSystemPhase::PreRoll,
            Stage::Ramp { .. } => MeasureSystemPhase::Ramp,
            Stage::Run { .. } => MeasureSystemPhase::Run,
            Stage::Analyse { .. } => MeasureSystemPhase::Analysing,
            Stage::Done => MeasureSystemPhase::Done,
            Stage::Aborted => MeasureSystemPhase::Aborted,
            Stage::Failed => MeasureSystemPhase::Failed,
        }
    }

    pub(crate) fn finished(&self) -> bool {
        matches!(self.stage, Stage::Done | Stage::Aborted | Stage::Failed)
    }

    pub(crate) fn active(&self) -> bool {
        !self.finished()
    }

    pub(crate) fn phase_key(&self) -> (MeasureSystemPhase, usize, usize, bool) {
        (
            self.phase(),
            self.run_idx,
            self.ramp_pos,
            self.issue.is_some(),
        )
    }

    pub(crate) fn take_outcome(&mut self) -> Option<SessionOutput> {
        self.output.take()
    }

    pub(crate) fn wants_capture(&self) -> bool {
        matches!(
            self.stage,
            Stage::PreRoll { .. } | Stage::Ramp { .. } | Stage::Run { .. } | Stage::Analyse { .. }
        )
    }

    fn runs_total(&self) -> usize {
        match self.purpose {
            MeasureSystemPurpose::Measure => self.plan.runs.len(),
            _ => self.jobs.len(),
        }
    }

    pub(crate) fn state(&self) -> MeasureSystemState {
        let (serial, output, level_db) = match &self.stage {
            Stage::Ramp {
                unit, ramp, level, ..
            } => (
                self.serial(*unit).to_owned(),
                Some(ramp.output),
                Some(*level),
            ),
            _ => (String::new(), None, None),
        };
        MeasureSystemState {
            phase: self.phase(),
            purpose: self.purpose,
            active: self.active(),
            wants_capture: self.wants_capture(),
            capture_rate_hz: self.plan.capture_rate_hz,
            serial,
            output,
            level_db,
            ramp_levels: self
                .levels
                .iter()
                .map(|(u, o, l)| MeasureSystemLevel {
                    serial: self.serial(*u).to_owned(),
                    output: *o,
                    level_db: *l,
                })
                .collect(),
            run: u16::try_from(
                if matches!(
                    self.stage,
                    Stage::PreRoll { after: After::Ramp } | Stage::Ramp { .. }
                ) {
                    0
                } else {
                    self.run_idx.saturating_add(1).min(self.runs_total())
                },
            )
            .unwrap_or(u16::MAX),
            runs: u16::try_from(self.runs_total()).unwrap_or(u16::MAX),
            frames: self.frames,
            progress: self.progress.clamp(0.0, 1.0),
            issue: self.issue.clone(),
            result_ready: matches!(self.stage, Stage::Done)
                && self.purpose == MeasureSystemPurpose::Measure,
            timing_reliable: self.timing_reliable,
            words: self.words(),
        }
    }

    /// What the session waits for, in words: the hub holds a START back
    /// until every unit's clock has settled (up to 40 s after it first saw
    /// it locked), so a run may begin long after its lead.
    fn words(&self) -> String {
        let waiting = match (&self.stage, self.pin) {
            (Stage::Ramp { timed: Some(t), .. } | Stage::Run { timed: t }, Some(p)) => {
                let lead = self.plan.lead_samples(self.lead_ms.saturating_add(1_000));
                t.schedule.start() > p.t.saturating_add(lead)
            }
            _ => false,
        };
        if waiting {
            "waiting for the units' clocks to settle".into()
        } else {
            String::new()
        }
    }

    fn serial(&self, unit: usize) -> &str {
        self.plan.units.get(unit).map_or("", |u| u.serial.as_str())
    }

    // ── Ending ─────────────────────────────────────────────────────────

    fn end(&mut self, hub: &mut dyn Hub, issue: MeasureSystemIssue, stage: Stage) {
        if self.finished() {
            return;
        }
        log::info!("system measurement ended: {}", issue.message);
        if !self.session.is_empty() {
            let req = AbortRequest {
                session: self.session.clone(),
                reason: issue.message.clone(),
            };
            if let Err(e) = hub.request(msg::t::ABORT, to_value(&req)) {
                log::warn!("system measurement: abort failed ({e}): STOP_NOW where possible");
                let serials: Vec<String> =
                    self.plan.units.iter().map(|u| u.serial.clone()).collect();
                hub.stop_now_fallback(&serials);
            }
            self.close(hub);
        }
        self.buf = Vec::new();
        self.issue = Some(issue);
        self.stage = stage;
    }

    fn abort(&mut self, hub: &mut dyn Hub, issue: MeasureSystemIssue) {
        self.end(hub, issue, Stage::Aborted);
    }

    fn fail(&mut self, hub: &mut dyn Hub, issue: MeasureSystemIssue) {
        self.end(hub, issue, Stage::Failed);
    }

    fn close(&mut self, hub: &mut dyn Hub) {
        if self.session.is_empty() {
            return;
        }
        let req = SessionRequest {
            session: std::mem::take(&mut self.session),
        };
        if let Err(e) = hub.request(msg::t::CLOSE, to_value(&req)) {
            log::debug!("system measurement: close failed: {e}");
        }
    }

    /// The user's Stop.
    pub(crate) fn stop_by_user(&mut self, hub: &mut dyn Hub) {
        self.abort(hub, issue("", "measure.stopped", "stopped"));
    }

    // ── The push path ──────────────────────────────────────────────────

    pub(crate) fn push(
        &mut self,
        hub: &mut dyn Hub,
        first_frame: u64,
        samples: &[f32],
        rate_hz: u32,
        clipped: bool,
    ) {
        if !self.wants_capture() || samples.is_empty() {
            return;
        }
        if rate_hz != self.plan.capture_rate_hz {
            let m = format!(
                "the microphone runs at {rate_hz} Hz, the measurement at {} Hz",
                self.plan.capture_rate_hz
            );
            self.abort(hub, issue("", "measure.abort.capture_rate", m));
            return;
        }
        // S5: one stream across every run (the microphone stays open).
        if let Some(want) = self.next_frame
            && want != first_frame
        {
            let what = if first_frame > want { "gap" } else { "repeat" };
            let m =
                format!("a {what} in the capture: frame {first_frame} where {want} was expected");
            self.abort(hub, issue("", "measure.abort.frame_gap", m));
            return;
        }
        let len = samples.len() as u64;
        self.next_frame = Some(first_frame.saturating_add(len));
        self.frames = self.frames.saturating_add(len);
        if matches!(self.stage, Stage::Analyse { .. }) {
            return;
        }
        // S4: a clip.
        let clip = safety::clip_linear();
        if clipped || samples.iter().any(|s| s.is_nan() || s.abs() >= clip) {
            self.abort(
                hub,
                issue(
                    "",
                    "measure.abort.capture_clip",
                    "the microphone's capture clipped",
                ),
            );
            return;
        }
        // S3, S4: the SPL cap.
        let offset = self
            .plan
            .units
            .first()
            .and_then(|u| u.plan.mic.as_ref())
            .and_then(|m| m.spl_offset_db);
        let cap = self.plan.units.first().and_then(|u| u.plan.spl_cap_db);
        if let (Some(m), Some(off), Some(cap)) = (self.spl.as_mut(), offset, cap)
            && let Some(db) = m.push(samples)
            && db + off > f64::from(cap)
        {
            let m = format!(
                "the sound reached {:.0} dB SPL (the cap is {cap:.0})",
                db + off
            );
            self.abort(hub, issue("", "measure.abort.spl", m));
            return;
        }
        if self.buf.is_empty() {
            self.buf_first = first_frame;
        }
        if self.buf.len().saturating_add(samples.len()) > MAX_CAPTURE_SAMPLES {
            self.abort(
                hub,
                issue(
                    "",
                    "measure.system.abort.overrun",
                    "the run's capture is too long",
                ),
            );
            return;
        }
        self.buf.extend_from_slice(samples);
    }

    // ── The tick ───────────────────────────────────────────────────────

    pub(crate) fn tick(&mut self, hub: &mut dyn Hub, now: Instant) {
        if self.finished() {
            return;
        }
        // The hub's own aborts (its dead-man, a foreign write, a preset
        // load, a disturbance it saw first).
        let aborted = hub.aborted();
        if let Some(e) = aborted.iter().find(|e| e.session == self.session) {
            let serial = e
                .unit
                .map(|u| self.serial(usize::from(u)).to_owned())
                .unwrap_or_default();
            let who = if serial.is_empty() {
                String::new()
            } else {
                format!("{serial}: ")
            };
            let m = format!("{who}the hub stopped the measurement ({})", e.reason);
            // -- WP ATDEMO -- a unit's time base moved: the unit lost its lock
            let key = if e.reason == "disturbance" {
                "measure.system.abort.disturbance"
            } else {
                "measure.system.abort.hub"
            };
            self.abort(hub, issue(&serial, key, m));
            return;
        }
        let due = self
            .last_poll
            .is_none_or(|t| now.saturating_duration_since(t) >= POLL_EVERY);
        if due && !self.session.is_empty() {
            self.last_poll = Some(now);
            if !self.poll(hub, now) {
                return;
            }
        }
        let deadline = match &self.stage {
            Stage::Ramp { timed: Some(t), .. } | Stage::Run { timed: t } => Some(t.deadline),
            _ => None,
        };
        if deadline.is_some_and(|d| now > d) {
            self.abort(
                hub,
                issue(
                    "",
                    "measure.system.abort.overrun",
                    "the run took longer than planned",
                ),
            );
            return;
        }
        // -- WP ATDEMO --
        if self.terms_retry.is_some_and(|at| now >= at) {
            self.read_terms(hub, now);
        }
        self.advance(hub, now);
    }

    /// One status read and its judgement. False when the session ended.
    fn poll(&mut self, hub: &mut dyn Hub, now: Instant) -> bool {
        let sent = Instant::now();
        let req = SessionRequest {
            session: self.session.clone(),
        };
        let reply = hub
            .request(msg::t::STATUS, to_value(&req))
            // ATHUB: the body travels under "status" (the seam's `t` would
            // collide with the answer's "t": "ok"); at the top level too.
            .and_then(|mut v| {
                let body = match v.get_mut("status") {
                    Some(b) if b.is_object() => b.take(),
                    _ => v,
                };
                parse::<StatusReply>(body, "status")
            });
        let reply = match reply {
            Ok(r) => r,
            Err(e) => {
                self.poll_failures = self.poll_failures.saturating_add(1);
                // -- WP ATDEMO -- the hub answered that it stopped the session
                // itself: why, in its words, the unit named (not "the hub
                // does not answer")
                if let DspiError::Link { code, msg } = &e
                    && code == "aborted"
                {
                    let i = self.hub_stopped(msg);
                    self.abort(hub, i);
                    return false;
                }
                let gone = matches!(e, DspiError::Disconnected)
                    || matches!(&e, DspiError::Link { code, .. } if code == "not_found" || code == "closed");
                if gone || self.poll_failures >= MAX_POLL_FAILURES {
                    let m = format!("the hub does not answer: {e}");
                    self.abort(hub, issue("", "measure.system.abort.disconnected", m));
                    return false;
                }
                return true;
            }
        };
        self.poll_failures = 0;
        let rtt = Instant::now().saturating_duration_since(sent);
        let _ = now;
        let rtt_frames = (rtt.as_secs_f64() * f64::from(self.plan.capture_rate_hz)) as u64;
        let pin = TimePin {
            capture_frame: self.next_frame.unwrap_or(0),
            t: reply.t,
            round_trip_frames: rtt_frames,
        };
        self.pin = Some(pin);
        let rtt_samples = (rtt.as_secs_f64() * f64::from(self.plan.rate_hz)) as u64;
        if let Some(t) = self.timed_mut() {
            t.pins.push(pin);
        }
        for st in &reply.units {
            if let Err(i) = self.judge_unit(st, reply.t, rtt_samples) {
                self.abort(hub, i);
                return false;
            }
        }
        true
    }

    // -- WP ATDEMO --
    /// A `status` the hub answered `aborted` (it stopped the session
    /// itself): the issue by the hub's reason. A unit's time base that was
    /// disturbed or moved is `abort.disturbance`, one no longer locked
    /// `abort.time_base`, anything else `abort.hub`; the unit is named when
    /// the hub's words name one of the session's.
    fn hub_stopped(&self, words: &str) -> MeasureSystemIssue {
        let serial = self
            .plan
            .units
            .iter()
            .map(|u| u.serial.as_str())
            .filter(|s| !s.is_empty() && words.contains(*s))
            .max_by_key(|s| s.len())
            .unwrap_or("")
            .to_owned();
        let w = words.to_ascii_lowercase();
        let key =
            if w == "disturbance" || w.contains("was disturbed") || w.contains("timeline moved") {
                "measure.system.abort.disturbance"
            } else if w.contains("its time base is") || w.contains("time base entry is gone") {
                "measure.system.abort.time_base"
            } else {
                "measure.system.abort.hub"
            };
        // "unit X: why" as "X: why"; a bare reason as the hub's.
        let m = match words.strip_prefix("unit ") {
            Some(rest) if !serial.is_empty() && rest.starts_with(serial.as_str()) => {
                rest.to_owned()
            }
            _ if key == "measure.system.abort.disturbance" && !serial.is_empty() => {
                format!("{serial}: its time base was disturbed ({words})")
            }
            _ if key == "measure.system.abort.disturbance" => {
                format!(
                    "a unit's time base was disturbed: the hub stopped the measurement ({words})"
                )
            }
            _ => format!("the hub stopped the measurement ({words})"),
        };
        issue(&serial, key, m)
    }

    fn timed_mut(&mut self) -> Option<&mut Timed> {
        match &mut self.stage {
            Stage::Ramp { timed: Some(t), .. } | Stage::Run { timed: t } => Some(t),
            _ => None,
        }
    }

    /// One unit of a status reply (S4, S5 per unit, PLAN-AT 1.5).
    fn judge_unit(
        &mut self,
        st: &UnitStatus,
        t_domain: u64,
        rtt_samples: u64,
    ) -> std::result::Result<(), MeasureSystemIssue> {
        let i = usize::from(st.unit);
        let Some(pu) = self.plan.units.get(i) else {
            return Ok(());
        };
        let serial = pu.serial.clone();
        let measured: Vec<u8> = pu.plan.outputs.iter().map(|o| o.output).collect();
        let limiters = pu.ctx.limiter_outputs.clone();
        let Some(u) = self.units.get_mut(i) else {
            return Ok(());
        };
        let bad = |key: &str, m: String| Err(issue(&serial, key, format!("{serial}: {m}")));
        // The time base: locked, undisturbed, its timeline still (G2 at once).
        let Some(entry) = TimeBaseEntry::decode(&st.time_base_entry) else {
            return bad(
                "measure.system.abort.time_base",
                "its time base could not be read".into(),
            );
        };
        if entry.state.raw() != TB_STATE_LOCKED {
            return bad(
                "measure.system.abort.time_base",
                format!(
                    "its time base is no longer locked ({})",
                    entry.state.as_str()
                ),
            );
        }
        if entry.disturbances != u.open.disturbances {
            return bad(
                "measure.system.abort.disturbance",
                format!(
                    "its time base was disturbed ({} -> {})",
                    u.open.disturbances, entry.disturbances
                ),
            );
        }
        let tmo = entry.t_minus_output(st.output_frames);
        let moved = i64::from(tmo.wrapping_sub(u.open.t_minus_output) as i32);
        if moved.unsigned_abs() > 1 {
            return bad(
                "measure.system.abort.disturbance",
                format!("its timeline moved by {moved} samples"),
            );
        }
        // G1's agreement, per tick: the unit's t on the domain's numbering.
        let unit_t = i128::from(entry.t);
        let domain_t = unit_t.saturating_sub(i128::from(u.open.origin_offset));
        u.tick_offset =
            i64::try_from(domain_t.saturating_sub(i128::from(t_domain))).unwrap_or(i64::MAX);
        u.tick_round_trip = rtt_samples;
        u.entry = Some(entry);
        u.output_frames = st.output_frames;
        // Events: a clip, a time-base transition (G4).
        for e in &st.events {
            match e.event {
                dspi_proto::ext::NOTIFY_EVT_EXT_CLIP => {
                    return bad("measure.system.abort.clip", "an output clipped".into());
                }
                0xE1 | 0xE3 | 0xE4 => {
                    u.events = u.events.saturating_add(1);
                    return bad(
                        "measure.system.abort.time_base",
                        format!("its time base changed (event {:#04x})", e.event),
                    );
                }
                _ => {}
            }
        }
        if st.clipped {
            return bad("measure.system.abort.clip", "an output clipped".into());
        }
        for o in &measured {
            if st
                .output_peaks_dbfs
                .get(usize::from(*o))
                .is_some_and(|p| p.is_nan() || *p >= OUTPUT_PEAK_DBFS)
            {
                return Err(MeasureSystemIssue::new(
                    &serial,
                    "measure.system.abort.output_peak",
                    Some(*o),
                    format!(
                        "{serial}: output {} reached full scale",
                        u16::from(*o).saturating_add(1)
                    ),
                ));
            }
        }
        if !limiters.is_empty() {
            let base = u.gr_base.get_or_insert_with(|| st.limiter_gr_db.clone());
            for o in &limiters {
                let k = usize::from(*o);
                let now = st.limiter_gr_db.get(k).copied().unwrap_or(0.0);
                let was = base.get(k).copied().unwrap_or(0.0);
                if now.is_finite() && (now - was).abs() > LIMITER_ACTS_DB && now > LIMITER_ACTS_DB {
                    return Err(MeasureSystemIssue::new(
                        &serial,
                        "measure.system.abort.limiter",
                        Some(*o),
                        format!(
                            "{serial}: output {}'s limiter acted",
                            u16::from(*o).saturating_add(1)
                        ),
                    ));
                }
            }
        }
        // The generator's own word, and the proofs.
        let tl = SiggenTimeline::decode(&st.timeline);
        u.timeline = tl;
        let frame = self.next_frame.unwrap_or(0);
        let Some(timed) = (match &mut self.stage {
            Stage::Ramp { timed: Some(t), .. } | Stage::Run { timed: t } => Some(t),
            _ => None,
        }) else {
            return Ok(());
        };
        if let Some(tl) = tl {
            for &(seg, unit, rid) in &timed.expected {
                if unit != i || tl.run_id != rid {
                    continue;
                }
                let r = tl.last_result.raw();
                if matches!(
                    r,
                    timeline_result::REFUSED_LATE
                        | timeline_result::REFUSED_NOT_READY
                        | timeline_result::CANCELLED_LOSS
                        | timeline_result::CANCELLED_CONFIG
                        | timeline_result::CANCELLED_HOST
                        | timeline_result::STOPPED_LOSS
                ) {
                    return bad(
                        "measure.system.abort.generator",
                        format!(
                            "its generator did not play its segment ({})",
                            tl.last_result.as_str()
                        ),
                    );
                }
                let proof = SegmentProof {
                    segment: seg,
                    expected_run_id: rid,
                    run_id: tl.run_id,
                    start: tl.start,
                    last_result: r,
                    discontinuities: tl.discontinuities,
                    samples: tl.samples,
                };
                let fin = tl.state.raw() != TL_RUNNING;
                match timed.proofs.iter_mut().find(|(p, _)| p.segment == seg) {
                    Some(slot) if !slot.1 => *slot = (proof, fin),
                    Some(_) => {}
                    None => timed.proofs.push((proof, fin)),
                }
            }
        }
        if let Ok(s) = dspi_proto::packets::SiggenStatus::decode(&st.siggen)
            && s.active_channel != NO_CHANNEL
            && matches!(timed.kind, TimedKind::Walk(_))
        {
            timed.status.push(UnitStatusObservation {
                unit: UnitIndex(st.unit),
                observation: StatusObservation {
                    capture_frame: frame,
                    status: GeneratorStatus {
                        active_channel: s.active_channel,
                        cycles_done: s.cycles_done,
                    },
                },
            });
        }
        Ok(())
    }

    // ── The stages ─────────────────────────────────────────────────────

    fn frames_of(&self, ms: u32) -> u64 {
        u64::from(self.plan.capture_rate_hz).saturating_mul(u64::from(ms)) / 1000
    }

    /// The capture frame at which domain sample `t` is played, from the
    /// latest pin.
    fn capture_at(&self, t: u64) -> u64 {
        let Some(p) = self.pin else {
            return self.next_frame.unwrap_or(0);
        };
        let ratio = f64::from(self.plan.capture_rate_hz) / f64::from(self.plan.rate_hz.max(1));
        let f = p.capture_frame as f64 + (t as f64 - p.t as f64) * ratio;
        if f.is_finite() && f > 0.0 {
            f as u64
        } else {
            0
        }
    }

    fn advance(&mut self, hub: &mut dyn Hub, now: Instant) {
        match &self.stage {
            Stage::PreRoll { after } => {
                let after = *after;
                let want = usize::try_from(self.frames_of(PREROLL_MS)).unwrap_or(usize::MAX);
                self.progress = self.buf.len() as f32 / want.max(1) as f32;
                if self.buf.len() < want {
                    return;
                }
                match after {
                    After::Ramp => {
                        self.listener =
                            match self.plan.units.first().map(|u| {
                                safety::ramp_listener(&u.plan, self.plan.rate_hz, &self.buf)
                            }) {
                                Some(Ok(l)) => Some(l),
                                Some(Err(e)) => {
                                    log::warn!("system measurement: the ramp hears nothing: {e}");
                                    None
                                }
                                None => None,
                            };
                        self.ramp_pos = 0;
                        self.next_ramp(hub, now);
                    }
                    After::Run => self.start_run(hub, now),
                }
            }
            Stage::Ramp { .. } => self.advance_ramp(hub, now),
            Stage::Run { .. } => self.advance_run(hub, now),
            Stage::Analyse { .. } => self.analyse_slice(hub),
            Stage::Done | Stage::Aborted | Stage::Failed => {}
        }
    }

    /// Stage `configs`, read a fresh pin and the units' run ids, place
    /// `schedule0`'s segments (from domain sample 0) on the counter. `None`
    /// when the session ended.
    fn start_timed(
        &mut self,
        hub: &mut dyn Hub,
        kind: TimedKind,
        configs: &[(usize, SiggenConfig)],
        schedule0: &SystemSchedule,
        now: Instant,
    ) -> Option<Timed> {
        for (u, c) in configs {
            let serial = self.serial(*u).to_owned();
            let ceiling = self
                .plan
                .units
                .get(*u)
                .map_or(f32::NEG_INFINITY, |p| p.plan.ceiling_db);
            if c.level_db.is_nan() || c.level_db > ceiling {
                let m = format!(
                    "{serial}: refused a generator level of {} dBFS above the ceiling of {ceiling} dBFS",
                    c.level_db
                );
                self.abort(hub, issue(&serial, "measure.system.abort.generator", m));
                return None;
            }
            let bytes = runs::check_config(c).and_then(|()| config_bytes(c));
            let sent = bytes.and_then(|config| {
                let req = StageRequest {
                    session: self.session.clone(),
                    serial: serial.clone(),
                    config,
                };
                hub.request(msg::t::STAGE, to_value(&req))
            });
            if let Err(e) = sent {
                let m = format!("{serial}: its generator could not be staged: {e}");
                self.abort(hub, issue(&serial, "measure.system.abort.generator", m));
                return None;
            }
        }
        // A fresh pin and every unit's run id before the STARTs.
        if !self.poll(hub, now) {
            return None;
        }
        let base: Vec<u32> = self
            .units
            .iter()
            .map(|u| u.timeline.map_or(0, |t| t.run_id))
            .collect();
        let segments: Vec<RunSegment> = schedule0
            .segments()
            .iter()
            .map(|s| RunSegment {
                unit: s.unit.0,
                offset: s.start,
            })
            .collect();
        let req = RunRequest {
            session: self.session.clone(),
            segments: segments.clone(),
            lead_ms: self.lead_ms,
            // WP ATSUMOFF: a summed run's parts may start apart.
            summed: schedule0.is_summed(),
        };
        let reply = hub
            .request(msg::t::RUN, to_value(&req))
            .and_then(|v| parse::<RunReply>(v, "run"));
        let reply = match reply {
            Ok(r) => r,
            Err(e) => {
                let m = format!("the hub did not schedule the run: {e}");
                self.abort(hub, issue("", "measure.system.hub_refused", m));
                return None;
            }
        };
        let placed = reply.segments.len() == segments.len()
            && reply
                .segments
                .iter()
                .zip(&segments)
                .all(|(r, s)| r.unit == s.unit && Some(r.t) == reply.t0.checked_add(s.offset));
        let schedule = schedule0.offset_by(reply.t0).ok().filter(|_| placed);
        let Some(schedule) = schedule else {
            self.abort(
                hub,
                issue(
                    "",
                    "measure.system.hub_refused",
                    "the hub placed the segments elsewhere",
                ),
            );
            return None;
        };
        let mut origins: Vec<(usize, i64)> = Vec::new();
        for r in &reply.segments {
            let o =
                i64::try_from(i128::from(r.t).saturating_sub(i128::from(r.t_unit))).unwrap_or(0);
            let u = usize::from(r.unit);
            if !origins.iter().any(|(x, _)| *x == u) {
                origins.push((u, o));
            }
        }
        let mut count: Vec<u32> = vec![0; self.units.len()];
        let mut expected = Vec::new();
        for (i, s) in schedule.segments().iter().enumerate() {
            let u = usize::from(s.unit.0);
            if let Some(c) = count.get_mut(u) {
                *c = c.saturating_add(1);
                let rid = base.get(u).copied().unwrap_or(0).wrapping_add(*c);
                expected.push((i, u, rid));
            }
        }
        let gap = self
            .plan
            .units
            .first()
            .map_or(0, |u| u32::from(u.plan.sweep.gap_ms));
        let end_frame = self
            .capture_at(schedule.end())
            .saturating_add(self.frames_of(gap.saturating_add(TAIL_MS)));
        let secs = self.pin.map_or(0.0, |p| {
            (schedule.end() as f64 - p.t as f64).max(0.0) / f64::from(self.plan.rate_hz.max(1))
        });
        let planned = Duration::from_secs_f64(secs.min(1e6)).saturating_add(Duration::from_millis(
            u64::from(gap.saturating_add(TAIL_MS)),
        ));
        Some(Timed {
            kind,
            schedule,
            origins,
            expected,
            proofs: Vec::new(),
            end_frame,
            deadline: now
                .checked_add(planned.saturating_add(OVERRUN))
                .unwrap_or(now),
            complete_at: None,
            pins: self.pin.into_iter().collect(),
            status: Vec::new(),
            levels: configs.iter().map(|(u, c)| (*u, c.level_db)).collect(),
        })
    }

    // ── The ramp (S3) ──────────────────────────────────────────────────

    fn next_ramp(&mut self, hub: &mut dyn Hub, now: Instant) {
        let Some(&(unit, output)) = self.ramp_queue.get(self.ramp_pos) else {
            // Every output has its level: the engine's terms, then the run.
            self.read_terms(hub, now);
            self.buf.clear();
            self.stage = Stage::PreRoll { after: After::Run };
            return;
        };
        let Some(p) = self.plan.units.get(unit).map(|u| u.plan.clone()) else {
            return;
        };
        let ramp = Ramp::new(output, p.start_db, p.ceiling_db, p.spl_cap_db, false);
        let first = ramp.first();
        self.start_step(hub, unit, ramp, first, now);
    }

    fn start_step(&mut self, hub: &mut dyn Hub, unit: usize, ramp: Ramp, level: f32, now: Instant) {
        let Some(p) = self.plan.units.get(unit).map(|u| u.plan.clone()) else {
            return;
        };
        let cfg = runs::ramp_config(&p, ramp.output, level);
        let sweep = LogSweep::new(
            self.plan.rate_hz,
            &SweepConfig {
                duration_ms: RAMP_SWEEP_MS,
                ..p.sweep
            },
        );
        let mask = 1u16.checked_shl(u32::from(ramp.output)).unwrap_or(0);
        let schedule = sweep
            .ok()
            .and_then(|s| WalkSchedule::new(s, mask, 1).ok())
            .and_then(|w| {
                SystemSchedule::single(
                    self.domain,
                    UnitIndex(u8::try_from(unit).unwrap_or(0)),
                    0,
                    w,
                )
                .ok()
            });
        let Some(schedule) = schedule else {
            let s = self.serial(unit).to_owned();
            self.fail(
                hub,
                issue(
                    &s,
                    "measure.system.plan",
                    "the ramp's sweep cannot be built",
                ),
            );
            return;
        };
        self.buf.clear();
        self.stage = Stage::Ramp {
            unit,
            ramp,
            level,
            timed: None,
        };
        let timed = self.start_timed(hub, TimedKind::Ramp, &[(unit, cfg)], &schedule, now);
        if let (Some(t), Stage::Ramp { timed: slot, .. }) = (timed, &mut self.stage) {
            *slot = Some(t);
        }
    }

    /// The timed run's capture and proofs are complete.
    fn timed_complete(&self, t: &Timed, now: Instant) -> bool {
        if self.next_frame.unwrap_or(0) < t.end_frame {
            return false;
        }
        let proven = t
            .expected
            .iter()
            .all(|(seg, _, _)| t.proofs.iter().any(|(p, fin)| p.segment == *seg && *fin));
        proven
            || t.complete_at
                .is_some_and(|c| now.saturating_duration_since(c) >= PROOF_GRACE)
    }

    fn advance_ramp(&mut self, hub: &mut dyn Hub, now: Instant) {
        let complete = match &self.stage {
            Stage::Ramp { timed: Some(t), .. } => self.timed_complete(t, now),
            _ => return,
        };
        if !complete {
            if let Stage::Ramp { timed: Some(t), .. } = &mut self.stage
                && self.next_frame.unwrap_or(0) >= t.end_frame
                && t.complete_at.is_none()
            {
                t.complete_at = Some(now);
            }
            return;
        }
        let Stage::Ramp {
            unit,
            ramp,
            level,
            timed: Some(t),
        } = std::mem::replace(&mut self.stage, Stage::Done)
        else {
            return;
        };
        let serial = self.serial(unit).to_owned();
        // The step must have played as planned (its proof), or nothing is
        // judged from it.
        let proven = t.expected.iter().all(|(seg, _, _)| {
            t.proofs.iter().any(|(p, fin)| {
                p.segment == *seg && *fin && p.last_result == timeline_result::STARTED
            })
        });
        if !proven {
            self.stage = Stage::Ramp {
                unit,
                ramp,
                level,
                timed: None,
            };
            let m = format!("{serial}: its ramp step was not proven to play");
            self.abort(hub, issue(&serial, "measure.system.abort.generator", m));
            return;
        }
        let (snr_db, in_band_db) = safety::step_hearing(self.listener.as_ref(), &self.buf);
        log::info!(
            "system measurement: ramp step {serial} output {} at {level} dBFS: in-band SNR {snr_db:.1} dB",
            ramp.output
        );
        let spl = None;
        let mut ramp = ramp;
        let next = ramp.judge(StepEval {
            level_db: level,
            snr_db,
            in_band_db,
            clipped: false,
            spl_db: spl,
        });
        match next {
            RampNext::Step(l) => self.start_step(hub, unit, ramp, l, now),
            RampNext::Done(l) => {
                self.levels.push((unit, ramp.output, l));
                self.ramp_pos = self.ramp_pos.saturating_add(1);
                self.next_ramp(hub, now);
            }
            RampNext::Ask(i) => {
                self.stage = Stage::Ramp {
                    unit,
                    ramp,
                    level,
                    timed: None,
                };
                let i = MeasureSystemIssue::of(&serial, &i);
                self.fail(hub, i);
            }
        }
    }

    /// What the alignment engine adds per unit (WP ATHUBM's contract): the
    /// play-out (the unit's own D from `open`, the model's path and latency
    /// from `system.status`'s alignment) and each measured output's
    /// `out.delay` in effect. A unit whose terms cannot be read gets none:
    /// its arrivals are then not offered.
    fn read_terms(&mut self, hub: &mut dyn Hub, now: Instant) {
        if self.purpose != MeasureSystemPurpose::Measure {
            return;
        }
        // -- WP ATDEMO -- the hub answers `system.status` once a second per
        // connection: refused because another view read it, it is read again
        // a moment later (during the run), never taken as unreadable at once.
        let status = match hub.request("system.status", json!({})) {
            Ok(v) => Some(v),
            Err(DspiError::Link { code, .. }) if code == "rate_limited" => {
                self.terms_retry = Some(now.checked_add(TERMS_RETRY).unwrap_or(now));
                return;
            }
            Err(_) => None,
        };
        self.terms_retry = None;
        for (i, u) in self.plan.units.iter().enumerate() {
            let outputs: Vec<u8> = u.plan.outputs.iter().map(|o| o.output).collect();
            // The unit's own D (review AT-C): the hub cannot read the units
            // this session has locked, so its alignment is not asked for it.
            let own = self.units.get(i).map(|x| super::proposal::PlayOut::Own {
                d: x.open.d,
                rate_hz: self.plan.rate_hz,
            });
            let t = status
                .as_ref()
                .zip(own)
                .and_then(|(s, own)| super::proposal::playout_us(s, &u.serial, own))
                .and_then(|playout_us| {
                    let items: Vec<Value> = outputs
                        .iter()
                        .map(|o| json!({"path": "out.delay", "idx": [o]}))
                        .collect();
                    let got = hub
                        .request(
                            "param.get_many",
                            json!({"serial": u.serial, "items": items}),
                        )
                        .ok()?;
                    let delays = super::proposal::out_delays_of(&got, &outputs)?;
                    Some(Terms {
                        playout_us,
                        out_delay_ms: delays,
                    })
                });
            if t.is_none() {
                log::warn!(
                    "system measurement: the engine's terms of {} could not be read",
                    u.serial
                );
            }
            if let Some(slot) = self.terms.get_mut(i) {
                *slot = t;
            }
        }
    }

    // ── The timed runs ─────────────────────────────────────────────────

    /// The level a unit plays a run at: the minimum of its walked outputs'
    /// ramp results (suggestion 4).
    fn unit_level(&self, unit: usize, outputs: &[u8]) -> f32 {
        let ceiling = self
            .plan
            .units
            .get(unit)
            .map_or(-120.0, |u| u.plan.ceiling_db);
        self.levels
            .iter()
            .filter(|(u, o, _)| *u == unit && outputs.contains(o))
            .map(|(_, _, l)| *l)
            .fold(ceiling, f32::min)
    }

    fn start_run(&mut self, hub: &mut dyn Hub, now: Instant) {
        // -- WP ATLATE -- a walked run is planned with the engine's terms:
        // while their read waits out the hub's rate limit, the pre-roll
        // goes on (its last pre-roll's worth kept), for at most
        // TERMS_WAIT; after it a unit still without terms is expected at 0
        // and its arrivals are not offered.
        if self.purpose == MeasureSystemPurpose::Measure && self.terms_retry.is_some() {
            let until = *self
                .terms_wait_until
                .get_or_insert_with(|| now.checked_add(TERMS_WAIT).unwrap_or(now));
            if now < until {
                let want = usize::try_from(self.frames_of(PREROLL_MS)).unwrap_or(usize::MAX);
                let extra = self.buf.len().saturating_sub(want);
                if extra > 0 {
                    self.buf.drain(..extra);
                    self.buf_first = self.buf_first.saturating_add(extra as u64);
                }
                return;
            }
        }
        let built = match self.purpose {
            MeasureSystemPurpose::Measure => self.walk_run(self.run_idx),
            _ => self.sum_run(self.run_idx),
        };
        let (kind, configs, schedule) = match built {
            Ok(b) => b,
            Err(e) => {
                self.fail(hub, issue("", "measure.system.plan", e.to_string()));
                return;
            }
        };
        // The pre-roll stays in the capture: the analysis starts with it.
        self.stage = Stage::Run {
            timed: Timed {
                kind,
                schedule: schedule.clone(),
                origins: Vec::new(),
                expected: Vec::new(),
                proofs: Vec::new(),
                end_frame: u64::MAX,
                deadline: now,
                complete_at: None,
                pins: Vec::new(),
                status: Vec::new(),
                levels: Vec::new(),
            },
        };
        let Some(mut timed) = self.start_timed(hub, kind, &configs, &schedule, now) else {
            return;
        };
        if let Stage::Run { timed: old } = &mut self.stage {
            timed.pins.append(&mut old.pins);
            *old = timed;
        }
    }

    fn walk_run(&self, idx: usize) -> Result<Built> {
        let part = self
            .plan
            .runs
            .get(idx)
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: "no such run".into(),
            })?;
        // -- WP ATLATE -- the starts planned with each unit's expected
        // lateness (its play-out and output delays, read before the run).
        let lateness = self.plan.lateness(part, &self.terms);
        let s0 = self.plan.schedule_late(part, self.lead_ms, &lateness)?;
        let schedule = SystemSchedule::new(self.domain, self.plan.rate_hz, s0.segments().to_vec())
            .and_then(|s| s.with_lateness(s0.lateness()))
            .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
        let mut configs = Vec::new();
        for (u, outs) in &part.units {
            let Some(pu) = self.plan.units.get(*u) else {
                continue;
            };
            let level = self.unit_level(*u, outs);
            let spec = super::super::plan::RunSpec {
                position: 0,
                outputs: outs.clone(),
                invert_mask: 0,
            };
            let mut c = runs::run_config(&pu.plan, &spec, level);
            c.repeat = 1;
            configs.push((*u, c));
        }
        Ok((TimedKind::Walk(idx), configs, schedule))
    }

    fn sum_run(&self, idx: usize) -> Result<Built> {
        let job = self
            .jobs
            .get(idx)
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: "no such summed run".into(),
            })?;
        let sweep = self.plan.sweep()?;
        // S-AT4: 6 dB below the lower of the two levels this session's own
        // ramps found, never above the measurement's.
        let ramped = |x: (usize, u8, bool)| {
            self.levels
                .iter()
                .find(|(u, o, _)| *u == x.0 && *o == x.1)
                .map(|(_, _, l)| *l)
        };
        let (Some(la), Some(lb)) = (ramped(job.a), ramped(job.b)) else {
            return Err(DspiError::InvalidArgument {
                msg: "a summed run's output was not ramped".into(),
            });
        };
        let level_db = (la.min(lb) - super::proposal::SUM_DOUBLING_DB).min(job.level_db);
        let mut parts = Vec::new();
        let mut configs = Vec::new();
        // WP ATSUMOFF: each part at its own start, so both are heard together.
        for &((u, o, inverted), start) in &[(job.a, job.starts.0), (job.b, job.starts.1)] {
            let Some(pu) = self.plan.units.get(u) else {
                continue;
            };
            let mask = 1u16.checked_shl(u32::from(o)).unwrap_or(0);
            let walk = WalkSchedule::new(sweep, mask, SUM_PASSES)
                .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
            parts.push((UnitIndex(u8::try_from(u).unwrap_or(u8::MAX)), start, walk));
            configs.push((
                u,
                SiggenConfig {
                    signal_type: super::super::preflight::SIG_LOG_SWEEP,
                    channel_mask: mask,
                    invert_mask: if inverted { mask } else { 0 },
                    flags: 0,
                    level_db: level_db.min(pu.plan.ceiling_db),
                    duration_ms: pu.plan.sweep.duration_ms,
                    repeat: SUM_PASSES,
                    gap_ms: pu.plan.sweep.gap_ms,
                    p: vec![pu.plan.sweep.f1_hz, pu.plan.sweep.f2_hz, 0.0, 0.0],
                },
            ));
        }
        let schedule = SystemSchedule::summed_at(self.domain, self.plan.rate_hz, 0, parts)
            .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
        Ok((TimedKind::Sum(idx), configs, schedule))
    }

    fn advance_run(&mut self, hub: &mut dyn Hub, now: Instant) {
        let (complete, progress) = match &self.stage {
            Stage::Run { timed } => {
                let start = timed.schedule.start();
                let span = self
                    .capture_at(timed.schedule.end())
                    .saturating_sub(self.capture_at(start))
                    .max(1);
                let got = self
                    .next_frame
                    .unwrap_or(0)
                    .saturating_sub(self.capture_at(start));
                (self.timed_complete(timed, now), got as f32 / span as f32)
            }
            _ => return,
        };
        self.progress = progress;
        if !complete {
            if let Stage::Run { timed } = &mut self.stage
                && self.next_frame.unwrap_or(0) >= timed.end_frame
                && timed.complete_at.is_none()
            {
                timed.complete_at = Some(now);
            }
            return;
        }
        let Stage::Run { timed } = std::mem::replace(&mut self.stage, Stage::Done) else {
            return;
        };
        let facts = self.facts(&timed);
        match SystemAnalyser::new(
            &timed.schedule,
            self.plan.capture_rate_hz,
            &analysis_options(),
        ) {
            Ok(a) => {
                self.progress = 0.0;
                self.stage = Stage::Analyse {
                    analyser: Box::new(a),
                    timed,
                    facts,
                };
            }
            Err(e) => {
                self.stage = Stage::Run { timed };
                self.fail(hub, issue("", "measure.no_signal", e.to_string()));
            }
        }
    }

    /// The gate's facts of every unit of a run (G1, G2, G4).
    fn facts(&self, t: &Timed) -> Vec<UnitFacts> {
        self.units
            .iter()
            .enumerate()
            .map(|(i, u)| {
                let end = u.entry;
                UnitFacts {
                    unit: UnitIndex(u8::try_from(i).unwrap_or(u8::MAX)),
                    domain: self.domain,
                    // The hub reads the one entry of the group's domain
                    // (G1 at open, R7b); a unit it could not would not open.
                    matching_entries: 1,
                    t_offset_samples: u.tick_offset,
                    tick_round_trip_samples: u
                        .tick_round_trip
                        .max(u64::from(self.plan.rate_hz) / 100),
                    state_open: u.open.state,
                    state_end: end.map_or(0, |e| e.state.raw()),
                    disturbances_open: u.open.disturbances,
                    disturbances_end: end
                        .map_or(u.open.disturbances.wrapping_add(1), |e| e.disturbances),
                    t_minus_output_open: u.open.t_minus_output,
                    t_minus_output_end: end.map_or(u.open.t_minus_output.wrapping_add(2), |e| {
                        e.t_minus_output(u.output_frames)
                    }),
                    origin: t
                        .origins
                        .iter()
                        .find(|(x, _)| *x == i)
                        .map_or(u.open.origin_offset.saturating_neg(), |o| o.1),
                    events: u.events,
                }
            })
            .collect()
    }

    fn analyse_slice(&mut self, hub: &mut dyn Hub) {
        let Stage::Analyse {
            analyser,
            timed,
            facts,
        } = &mut self.stage
        else {
            return;
        };
        let capture = SystemCapture {
            samples: &self.buf,
            first_frame: self.buf_first,
            rate_hz: self.plan.capture_rate_hz,
            schedule: &timed.schedule,
            pins: &timed.pins,
            status: &timed.status,
            complete: true,
        };
        let step = analyser.step(&capture);
        match step {
            Ok(SystemStep::Progress { done, total }) => {
                self.progress = done as f32 / total.max(1) as f32;
            }
            Ok(SystemStep::NeedCapture) => {
                self.fail(
                    hub,
                    issue("", "measure.no_signal", "the capture ends before the run"),
                );
            }
            Ok(SystemStep::Done(a)) => {
                let facts = std::mem::take(facts);
                let proofs: Vec<SegmentProof> = timed.proofs.iter().map(|(p, _)| *p).collect();
                let schedule = timed.schedule.clone();
                let kind = timed.kind;
                let levels = timed.levels.clone();
                let gate = check_gate(&schedule, &facts, &proofs);
                self.ran(hub, kind, *a, gate, levels, &proofs);
            }
            Err(e) => {
                let key = match e {
                    TimingError::CycleIdentity(_) => "measure.abort.cycle_identity",
                    TimingError::Measure(_) => "measure.no_signal",
                };
                self.fail(hub, issue("", key, e.to_string()));
            }
        }
    }

    /// The unit a gate finding names.
    fn finding_serial(&self, g: &GateFinding, schedule: &SystemSchedule) -> String {
        let unit = match g {
            GateFinding::NoFacts { unit }
            | GateFinding::NotOneDomain { unit, .. }
            | GateFinding::EntryNotUnique { unit, .. }
            | GateFinding::TimeDisagrees { unit, .. }
            | GateFinding::NotLocked { unit, .. }
            | GateFinding::Disturbed { unit, .. }
            | GateFinding::TimelineMoved { unit, .. }
            | GateFinding::Events { unit, .. } => Some(*unit),
            GateFinding::NoProof { segment } | GateFinding::ProofMismatch { segment, .. } => {
                schedule.segments().get(*segment).map(|s| s.unit)
            }
        };
        unit.map(|u| self.serial(usize::from(u.0)).to_owned())
            .unwrap_or_default()
    }

    /// A timed run analysed.
    fn ran(
        &mut self,
        hub: &mut dyn Hub,
        kind: TimedKind,
        analysis: SystemAnalysis,
        gate: Vec<GateFinding>,
        levels: Vec<(usize, f32)>,
        proofs: &[SegmentProof],
    ) {
        let schedule = match &self.stage {
            Stage::Analyse { timed, .. } => timed.schedule.clone(),
            _ => return,
        };
        self.timing_reliable =
            Some(self.timing_reliable.unwrap_or(true) && analysis.timing_reliable);
        match kind {
            TimedKind::Walk(idx) => {
                if let Some(g) = gate.first() {
                    // G1 to G4: the run is discarded; nothing is proposed.
                    let serial = self.finding_serial(g, &schedule);
                    let m = if serial.is_empty() {
                        format!("the run is discarded: {g}")
                    } else {
                        format!("{serial}: the run is discarded: {g}")
                    };
                    self.fail(hub, issue(&serial, "measure.system.gate", m));
                    return;
                }
                self.done_runs.push(RunDone {
                    schedule,
                    analysis,
                    levels,
                    run_ids: proofs.iter().map(|p| p.run_id).collect(),
                });
                self.run_idx = idx.saturating_add(1);
                if self.run_idx < self.plan.runs.len() {
                    // The next chained run: a pre-roll of its own; the
                    // microphone stays open (one frame count).
                    self.buf.clear();
                    self.stage = Stage::PreRoll { after: After::Run };
                    return;
                }
                let outcome = Outcome::build(
                    self.plan.clone(),
                    std::mem::take(&mut self.done_runs),
                    std::mem::take(&mut self.terms),
                    self.levels.clone(),
                    self.units
                        .iter()
                        .map(|u| u.open.role == dspi_proto::ext::timebase::TB_ROLE_SOURCE)
                        .collect(),
                );
                self.close(hub);
                self.buf = Vec::new();
                match outcome {
                    Ok(o) => {
                        self.output = Some(SessionOutput::Measured(Box::new(o)));
                        self.progress = 1.0;
                        self.stage = Stage::Done;
                    }
                    Err(e) => {
                        self.stage = Stage::Done;
                        self.issue = Some(issue("", "measure.no_signal", e.to_string()));
                        self.stage = Stage::Failed;
                    }
                }
            }
            TimedKind::Sum(idx) => {
                let verdict = match self.jobs.get(idx) {
                    Some(job) => {
                        if let Some(g) = gate.first() {
                            Verdict::failed(idx, format!("the summed run is discarded: {g}"))
                        } else {
                            // The level it played at (both units the same).
                            let played =
                                levels.iter().map(|(_, l)| *l).fold(f32::INFINITY, f32::min);
                            let played = if played.is_finite() {
                                played
                            } else {
                                job.level_db
                            };
                            job.judge(idx, &analysis, &self.plan, played)
                        }
                    }
                    None => Verdict::failed(idx, "no such summed run".into()),
                };
                self.verdicts.push(verdict);
                self.run_idx = idx.saturating_add(1);
                if self.run_idx < self.jobs.len() {
                    self.buf.clear();
                    self.stage = Stage::PreRoll { after: After::Run };
                    return;
                }
                self.close(hub);
                self.buf = Vec::new();
                self.output = Some(SessionOutput::Verdicts(std::mem::take(&mut self.verdicts)));
                self.progress = 1.0;
                self.stage = Stage::Done;
            }
            TimedKind::Ramp => {}
        }
    }
}

#[cfg(test)]
impl SysSession {
    /// The lead the session uses, ms.
    pub(crate) fn lead(&self) -> u32 {
        self.lead_ms
    }
}
