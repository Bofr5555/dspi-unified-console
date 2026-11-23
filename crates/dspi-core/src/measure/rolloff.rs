//! The roll-off search (WP ROLL1, owner 2026-11-22 "measuring a tweeter's low
//! end"): where an output's response drops off at the low end, measured from
//! the top down so that a tweeter is never played lower than it must be.
//!
//! **How:** a series of single sweeps on one output, each from a lower start
//! to 20 kHz: from [`ROLLOFF_START_HZ`] (or the caller's start) down in half
//! octaves. Each segment is an ordinary session of kind `liveSweep`, so every
//! rule of the session holds: the preflight (S6's high-pass, the volume,
//! the limits profile), the closed-loop ramp from a quiet start with the
//! segment's own sweep, the ceiling, the SPL cap, every abort. After each
//! segment the measured response is smoothed to a third of an octave and
//! judged:
//!
//! - **rolled off:** somewhere below its loudest point it has fallen
//!   [`ROLLOFF_STOP_DB`] below that loudest point: the search stops and
//!   reports where ([`MeasureRolloffResult::rolloff_hz`]);
//! - **the floor:** on an (effective) tweeter the search never plays below
//!   1 kHz ([`super::plan::TWEETER_STIMULUS_FLOOR_HZ`]), whatever the
//!   response looks like. A tweeter still within 15 dB of its loudest at
//!   1 kHz stops there with the note `measure.rolloff.midrange`: probably a
//!   midrange set up as a tweeter (the user's configuration; a note, not an
//!   error). Another way's floor is the plan's 20 Hz.
//!
//! The floor is held three times: the segments are planned above it, the
//! plan floors every run of a high way ([`super::plan::Plan::sweep_of`]), and
//! the session refuses any generator configuration that would play a high
//! way below it (`session::below_floor`).
//!
//! **Driving it:** [`DspiCore::measure_rolloff_start`], then
//! [`DspiCore::measure_rolloff_tick`] about every 50 ms (instead of
//! `measure_tick`) while `running`, pushing capture with
//! `measure_capture_push` whenever `session.wantsCapture` (the demo's
//! simulated microphone pushes by itself). The result holds the last
//! segment's `measurement` document for filing.
//!
//! **Locks:** the search's own mutex is taken before the session's slot,
//! never after it; what it shows is a leaf (`rolloff_view`), so the
//! measurement listener may read [`DspiCore::measure_rolloff_state`].

use dspi_measure::doc::MeasurementDoc;
use serde::{Deserialize, Serialize};

use super::plan::{Plan, stimulus_floor_hz};
use super::{
    MeasureCoupler, MeasureIssue, MeasurePhase, MeasurePlacement, MeasureRole, MeasureRunKind,
    MeasureSetup, MeasureState, PreflightOptions,
};
use crate::{DspiCore, DspiError, Result};

/// How far below its loudest point the response must fall for the search
/// to stop, dB (owner 2026-11-22: 15 dB, one constant, tuned later).
pub const ROLLOFF_STOP_DB: f32 = 15.0;
/// Where the search starts when the setup names no start, Hz.
pub const ROLLOFF_START_HZ: f32 = 8_000.0;
/// The highest start a setup may name, Hz (an octave below the sweep's top).
pub const ROLLOFF_START_MAX_HZ: f32 = 10_000.0;
/// Each segment starts this factor lower than the one before (half an
/// octave).
const STEP: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// The smoothing of the judged response, octaves (a third, each side half).
const SMOOTH_OCT: f64 = 1.0 / 3.0;
/// A segment's response is judged from this far above its start, octaves:
/// the analysis tapers the band's edge.
const EDGE_OCT: f64 = 1.0 / 6.0;
/// The notice of a tweeter still good at its floor.
pub(crate) const KEY_MIDRANGE: &str = "measure.rolloff.midrange";

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

// ── Records ─────────────────────────────────────────────────────────────

/// What to search (`measure_rolloff_start`). Every number 0 (or absent) is
/// the default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureRolloffSetup {
    /// The output, 0-based.
    pub output: u8,
    /// Its way as the caller has it; the effective role (a high-pass at
    /// 1 kHz or above makes it a tweeter) is what counts. Ignored with
    /// `roles_from_layout`.
    pub role: MeasureRole,
    /// Take the role from the device's confirmed speaker layout (agents
    /// always do; refused with `measure.layout_unconfirmed` without one).
    pub roles_from_layout: bool,
    /// The speaker it belongs to, for the document.
    pub speaker: String,
    /// Where the microphone is: near field (close to the driver, which
    /// judges a driver best) or (-- WP FEAT2 --) on a headphone coupler, with
    /// the headphones' level caps; any other placement is taken as the
    /// listening position.
    pub placement: MeasurePlacement,
    /// The microphone's `mic` document's text ("" uncalibrated).
    pub mic_document: String,
    /// The first segment's start, Hz (0: 8 kHz; at most 10 kHz; never below
    /// the output's floor).
    pub start_hz: f32,
    /// One sweep, ms (1000 to 15000; 0: 2700).
    pub sweep_ms: u32,
    /// The generator's ceiling, peak dBFS (as `MeasureSetup::ceiling_db`).
    pub ceiling_db: Option<f32>,
    /// The SPL cap with a calibrated microphone (as `MeasureSetup`).
    pub spl_cap_db: Option<f32>,
    /// The capture's rate, Hz (0: 48000).
    pub capture_rate_hz: u32,
    /// The documents' name ("" names them by the date).
    pub name: String,
    /// Play at the start level only (as `MeasureSetup::start_level_only`).
    pub start_level_only: bool,
    // -- WP FEAT2 --
    /// The coupler, for headphones (as `MeasureSetup::coupler`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub coupler: Option<MeasureCoupler>,
}

/// Where the search stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureRolloffStage {
    /// No search on this core.
    #[default]
    Idle,
    /// A segment is being measured (`session` says which phase).
    Measuring,
    /// The search ended with a result.
    Done,
    /// It stopped without a result (`issue` says why).
    Stopped,
}

/// Why the search ended.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureRolloffEnd {
    /// The response fell [`ROLLOFF_STOP_DB`] below its loudest point.
    #[default]
    RolledOff,
    /// The output's floor was reached first (1 kHz on a tweeter).
    Floor,
}

/// The search's result.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRolloffResult {
    /// The output.
    pub output: u8,
    /// Its effective role.
    pub role: MeasureRole,
    /// Why it ended.
    pub end: MeasureRolloffEnd,
    /// The threshold it stopped at, dB below the loudest point.
    pub stop_db: f32,
    /// The loudest point of the smoothed response, dB (re a 0 dBFS
    /// stimulus, or dB SPL with `spl`).
    pub reference_db: f64,
    /// Where that loudest point is, Hz.
    pub reference_hz: f64,
    /// Where the response falls `stop_db` below the loudest point, Hz;
    /// `None` when it never did above the floor.
    pub rolloff_hz: Option<f64>,
    /// The lowest frequency any stimulus played, Hz.
    pub lowest_hz: f64,
    /// The smoothed level at the lowest judged frequency, dB re the loudest
    /// point (at most 0).
    pub low_end_db: f64,
    /// The note for the user: `measure.rolloff.midrange` for a tweeter still
    /// good at its floor.
    pub note: Option<MeasureIssue>,
    /// The last segment's trusted points in its judged band, Hz.
    pub freqs_hz: Vec<f64>,
    /// The smoothed level at each of `freqs_hz`, dB (as `reference_db`).
    pub mag_db: Vec<f64>,
    /// The levels are dB SPL (a calibrated microphone).
    pub spl: bool,
    /// The last segment's `measurement` document (file it with
    /// `measure_doc_put`).
    pub document: String,
}

/// The search's state.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRolloffState {
    /// Where it stands.
    pub stage: MeasureRolloffStage,
    /// Tick it (`measure_rolloff_tick`): a segment runs or is next.
    pub running: bool,
    /// The output searched.
    pub output: Option<u8>,
    /// Its effective role.
    pub role: MeasureRole,
    /// The lowest any segment may start, Hz (1000 on a tweeter).
    pub floor_hz: f32,
    /// The segment now (or last) measured, 1-based.
    pub segment: u16,
    /// The most segments there can be (the floor reached).
    pub segments: u16,
    /// That segment's start, Hz.
    pub segment_start_hz: f32,
    /// The segment's session (push capture while it wants it).
    pub session: MeasureState,
    /// The result, once done.
    pub result: Option<MeasureRolloffResult>,
    /// Why it stopped, or what the last segment asked.
    pub issue: Option<MeasureIssue>,
    /// Who started it: 0 the app, else an agent's actor.
    pub actor: u32,
}

// ── The judgement (pure) ────────────────────────────────────────────────

/// One segment judged.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Verdict {
    pub(crate) reference_db: f64,
    pub(crate) reference_hz: f64,
    pub(crate) rolloff_hz: Option<f64>,
    /// The lowest judged point, Hz, and its level re the reference, dB.
    pub(crate) low_hz: f64,
    pub(crate) low_end_db: f64,
    pub(crate) freqs_hz: Vec<f64>,
    pub(crate) mag_db: Vec<f64>,
}

/// Judge a segment's response (`freqs`, `mag`, `valid` per point) played
/// from `f1_hz` up: smoothed to a third of an octave over the trusted points
/// of the judged band (from a sixth of an octave above `f1_hz`), the loudest
/// point, and the first point below it, going down, that is `stop_db` under
/// it (the crossing interpolated on log frequency). `None` with fewer than
/// two trusted points.
pub(crate) fn judge(
    freqs: &[f64],
    mag: &[f64],
    valid: Option<&[bool]>,
    f1_hz: f64,
    stop_db: f64,
) -> Option<Verdict> {
    let lo = f1_hz * 2f64.powf(EDGE_OCT);
    let pts: Vec<(f64, f64)> = freqs
        .iter()
        .zip(mag)
        .enumerate()
        .filter(|(i, (f, m))| {
            **f >= lo
                && f.is_finite()
                && m.is_finite()
                && valid.is_none_or(|v| v.get(*i).copied().unwrap_or(false))
        })
        .map(|(_, (f, m))| (*f, *m))
        .collect();
    if pts.len() < 2 {
        return None;
    }
    let half = SMOOTH_OCT / 2.0;
    let smooth: Vec<(f64, f64)> = pts
        .iter()
        .map(|(f, _)| {
            let (sum, n) = pts
                .iter()
                .filter(|(g, _)| (g / f).log2().abs() <= half)
                .fold((0.0, 0u32), |(s, n), (_, m)| (s + m, n.saturating_add(1)));
            (*f, sum / f64::from(n.max(1)))
        })
        .collect();
    let (top, (ref_hz, ref_db)) =
        smooth
            .iter()
            .copied()
            .enumerate()
            .fold((0, (0.0, f64::NEG_INFINITY)), |best, (i, p)| {
                if p.1 > best.1.1 { (i, p) } else { best }
            });
    let limit = ref_db - stop_db;
    let mut rolloff_hz = None;
    let mut above = (ref_hz, ref_db);
    for (f, m) in smooth.iter().take(top).rev() {
        if *m <= limit {
            // Between `f` (under) and `above` (over) on log frequency.
            let t = ((above.1 - limit) / (above.1 - m)).clamp(0.0, 1.0);
            let lf = above.0.ln() + t * (f.ln() - above.0.ln());
            rolloff_hz = Some(lf.exp());
            break;
        }
        above = (*f, *m);
    }
    let (low_hz, low_db) = smooth.first().copied().unwrap_or((ref_hz, ref_db));
    Some(Verdict {
        reference_db: ref_db,
        reference_hz: ref_hz,
        rolloff_hz,
        low_hz,
        low_end_db: (low_db - ref_db).min(0.0),
        freqs_hz: smooth.iter().map(|p| p.0).collect(),
        mag_db: smooth.iter().map(|p| p.1).collect(),
    })
}

/// The segments' starts, Hz: from `start` down in half octaves, the last at
/// `floor` (20 Hz for a way without one).
pub(crate) fn segments(start: f32, floor: f32) -> Vec<f32> {
    let floor = floor.max(20.0);
    let mut v = vec![start.max(floor)];
    while let Some(last) = v.last().copied() {
        if last <= floor || v.len() >= 64 {
            break;
        }
        let next = last * STEP;
        // A last step of less than a sixth of an octave is folded into the
        // floor.
        v.push(if next < floor * 2f32.powf(EDGE_OCT as f32) {
            floor
        } else {
            next
        });
    }
    v
}

// ── The driver ──────────────────────────────────────────────────────────

/// The search of one core.
#[derive(Debug, Default)]
pub(crate) struct Rolloff {
    setup: Option<MeasureRolloffSetup>,
    /// The resolved segment setup (one output, its effective role).
    base: Option<MeasureSetup>,
    role: MeasureRole,
    floor_hz: f32,
    starts: Vec<f32>,
    /// The segment now (or last) measured, an index into `starts`.
    at: usize,
    /// A segment's session runs.
    running: bool,
    stage: MeasureRolloffStage,
    result: Option<MeasureRolloffResult>,
    issue: Option<MeasureIssue>,
    actor: u32,
}

impl Rolloff {
    fn state(&self, session: MeasureState) -> MeasureRolloffState {
        MeasureRolloffState {
            stage: self.stage,
            running: self.stage == MeasureRolloffStage::Measuring,
            output: self.setup.as_ref().map(|s| s.output),
            role: self.role,
            floor_hz: self.floor_hz,
            segment: u16::try_from(self.at.saturating_add(1))
                .unwrap_or(u16::MAX)
                .min(u16::try_from(self.starts.len()).unwrap_or(u16::MAX)),
            segments: u16::try_from(self.starts.len()).unwrap_or(u16::MAX),
            segment_start_hz: self.starts.get(self.at).copied().unwrap_or(0.0),
            session,
            result: self.result.clone(),
            issue: self.issue.clone(),
            actor: self.actor,
        }
    }
}

impl super::MeasureSlot {
    fn rolloff_lock(&self) -> std::sync::MutexGuard<'_, Rolloff> {
        self.rolloff.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Start the roll-off search of one output (module docs): the first
    /// segment's preflight and start, as `measure_start`. On an (effective)
    /// tweeter no segment plays below 1 kHz.
    ///
    /// Errors: as `measure_start` (a blocker, read-only, a Viewer, the device
    /// lock, `measure.layout_unconfirmed`); [`DspiError::InvalidArgument`]
    /// for a setup that is not one, or while a search, a session, a tune's
    /// run or the live view runs on this core.
    pub fn measure_rolloff_start(&self, setup: MeasureRolloffSetup) -> Result<MeasureRolloffState> {
        let actor = crate::actor::JobActor::capture(self).actor();
        let mut r = self.measure.rolloff_lock();
        if r.stage == MeasureRolloffStage::Measuring {
            return Err(invalid("busy: a roll-off search is already running"));
        }
        if self.measure_tune_state().running {
            return Err(invalid("busy: a tune's run is going on this device"));
        }
        // -- WP ADET0 --
        if self.measure.detect_on() {
            return Err(invalid(
                "busy: a speaker detection is running on this device",
            ));
        }
        let start = if setup.start_hz == 0.0 {
            ROLLOFF_START_HZ
        } else if setup.start_hz.is_finite()
            && (20.0..=ROLLOFF_START_MAX_HZ).contains(&setup.start_hz)
        {
            setup.start_hz
        } else {
            return Err(invalid(format!(
                "a start of {} Hz (20 to {ROLLOFF_START_MAX_HZ} Hz)",
                setup.start_hz
            )));
        };
        let base = self.setup_roles(MeasureSetup {
            kind: MeasureRunKind::LiveSweep,
            outputs: vec![super::MeasureOutput {
                output: setup.output,
                role: setup.role,
                speaker: setup.speaker.clone(),
                group: 0,
                inverted: false,
            }],
            placement: match setup.placement {
                MeasurePlacement::NearField => MeasurePlacement::NearField,
                // -- WP FEAT2 -- headphones play under the headphones' caps
                MeasurePlacement::Headphones => MeasurePlacement::Headphones,
                _ => MeasurePlacement::ListeningPosition,
            },
            coupler: setup.coupler, // -- WP FEAT2 --
            mic_document: setup.mic_document.clone(),
            sweep_ms: setup.sweep_ms,
            ceiling_db: setup.ceiling_db,
            spl_cap_db: setup.spl_cap_db,
            capture_rate_hz: setup.capture_rate_hz,
            name: setup.name.clone(),
            start_level_only: setup.start_level_only,
            roles_from_layout: setup.roles_from_layout,
            ..MeasureSetup::default()
        })?;
        // The effective role on the device's chain decides the floor.
        let mut plan = Plan::new(&base)?;
        self.measure_resolve_roles(&mut plan)?;
        let role = plan.role_of(setup.output);
        let floor = stimulus_floor_hz(role).max(plan.sweep.f1_hz);
        let mut base = base;
        base.roles_from_layout = false;
        if let Some(o) = base.outputs.first_mut() {
            o.role = role;
        }
        *r = Rolloff {
            setup: Some(setup),
            base: Some(base),
            role,
            floor_hz: floor,
            starts: segments(start, floor),
            at: 0,
            running: false,
            stage: MeasureRolloffStage::Measuring,
            result: None,
            issue: None,
            actor,
        };
        if let Err(e) = self.rolloff_segment(&mut r) {
            *r = Rolloff::default();
            self.rolloff_publish(&r);
            return Err(e);
        }
        Ok(self.rolloff_publish(&r))
    }

    /// Drive the search: its segment's session (`measure_tick`), then the
    /// verdict and the next segment or the result. Call it about every 50 ms
    /// off the UI thread while `running`, instead of `measure_tick`. Never
    /// fails.
    pub fn measure_rolloff_tick(&self) -> MeasureRolloffState {
        let mut r = self.measure.rolloff_lock();
        if r.stage != MeasureRolloffStage::Measuring || !r.running {
            return self.rolloff_publish(&r);
        }
        let st = self.measure_tick();
        match st.phase {
            MeasurePhase::Done => {
                let text = self.measure_result().ok();
                let _ = self.measure_clear();
                r.running = false;
                match text {
                    Some(t) => self.rolloff_judge(&mut r, &t),
                    None => self.rolloff_stopped(&mut r, None),
                }
            }
            MeasurePhase::Aborted | MeasurePhase::Failed | MeasurePhase::Idle => {
                let _ = self.measure_clear();
                r.running = false;
                self.rolloff_stopped(&mut r, st.issue.clone());
            }
            _ => {}
        }
        self.rolloff_publish(&r)
    }

    /// The search's state, without I/O (safe from the measurement listener).
    pub fn measure_rolloff_state(&self) -> MeasureRolloffState {
        self.measure
            .rolloff_view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The user's Stop: the segment's session stopped (`SIGGEN_CTL_STOP_NOW`)
    /// and the search ended without a result. Idempotent; never fails.
    pub fn measure_rolloff_stop(&self) -> MeasureRolloffState {
        let mut r = self.measure.rolloff_lock();
        if r.stage == MeasureRolloffStage::Measuring {
            crate::actor::user::did(self, "measure_stop"); // -- WP AIVIS --
            if r.running {
                self.measure_stop_session();
                let _ = self.measure_clear();
                r.running = false;
            }
            self.rolloff_stopped(
                &mut r,
                Some(MeasureIssue::new(
                    "measure.stopped",
                    None,
                    "the roll-off search was stopped",
                )),
            );
        }
        self.rolloff_publish(&r)
    }

    /// Forget a finished search (its result included). Refused while one
    /// runs.
    ///
    /// Errors: [`DspiError::InvalidArgument`] while a search runs.
    pub fn measure_rolloff_clear(&self) -> Result<()> {
        let mut r = self.measure.rolloff_lock();
        if r.stage == MeasureRolloffStage::Measuring {
            return Err(invalid("a roll-off search is running: stop it first"));
        }
        *r = Rolloff::default();
        self.rolloff_publish(&r);
        Ok(())
    }
}

impl DspiCore {
    fn rolloff_publish(&self, r: &Rolloff) -> MeasureRolloffState {
        let session = if r.running {
            self.measure_state()
        } else {
            MeasureState::default()
        };
        let st = r.state(session);
        *self
            .measure
            .rolloff_view
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = st.clone();
        st
    }

    /// Start segment `r.at`: the session of its band (refused like any
    /// start).
    fn rolloff_segment(&self, r: &mut Rolloff) -> Result<()> {
        let f1 = r
            .starts
            .get(r.at)
            .copied()
            .ok_or_else(|| invalid("no segment left"))?;
        let base = r.base.as_ref().ok_or_else(|| invalid("no search"))?;
        let mut plan = Plan::new(base)?;
        // Never below the floor, whatever the list says.
        plan.set_sweep_start(f1.max(r.floor_hz))?;
        if self.measure.live_on() {
            return Err(invalid("busy: the live view is running on this device"));
        }
        self.measure_start_plan(plan, &PreflightOptions::default())?;
        // Started from a tick on any thread: still the search's starter's.
        self.measure_set_actor(r.actor);
        r.running = true;
        Ok(())
    }

    /// The segment's document judged: the result, or the next segment.
    fn rolloff_judge(&self, r: &mut Rolloff, text: &str) {
        let f1 = f64::from(r.starts.get(r.at).copied().unwrap_or(r.floor_hz));
        let verdict = MeasurementDoc::from_text(text).ok().and_then(|doc| {
            let o = doc.measurement.outputs.first()?.clone();
            let freqs = o.response.grid.freqs().ok()?;
            let spl = o.response.spl;
            judge(
                &freqs,
                &o.response.mag_db,
                o.response.valid.as_deref(),
                f1,
                f64::from(ROLLOFF_STOP_DB),
            )
            .map(|v| (v, spl))
        });
        let Some((v, spl)) = verdict else {
            self.rolloff_stopped(
                r,
                Some(MeasureIssue::new(
                    "measure.no_signal",
                    r.setup.as_ref().map(|s| s.output),
                    "the segment's response had too few trusted points to judge",
                )),
            );
            return;
        };
        let at_floor = r.at.saturating_add(1) >= r.starts.len() || f1 <= f64::from(r.floor_hz);
        if v.rolloff_hz.is_none() && !at_floor {
            r.at = r.at.saturating_add(1);
            if let Err(e) = self.rolloff_segment(r) {
                self.rolloff_stopped(
                    r,
                    Some(MeasureIssue::new(
                        "measure.stopped",
                        r.setup.as_ref().map(|s| s.output),
                        format!("the next segment could not start: {e}"),
                    )),
                );
            }
            return;
        }
        let output = r.setup.as_ref().map_or(0, |s| s.output);
        let note = (v.rolloff_hz.is_none() && r.role == MeasureRole::High).then(|| {
            MeasureIssue::new(
                KEY_MIDRANGE,
                Some(output),
                format!(
                    "Out {} still plays within {ROLLOFF_STOP_DB:.0} dB of its loudest at \
                     {:.0} Hz, where a tweeter's measurement stops: it is probably a midrange \
                     set up as a tweeter",
                    u16::from(output).saturating_add(1),
                    r.floor_hz
                ),
            )
        });
        r.result = Some(MeasureRolloffResult {
            output,
            role: r.role,
            end: if v.rolloff_hz.is_some() {
                MeasureRolloffEnd::RolledOff
            } else {
                MeasureRolloffEnd::Floor
            },
            stop_db: ROLLOFF_STOP_DB,
            reference_db: v.reference_db,
            reference_hz: v.reference_hz,
            rolloff_hz: v.rolloff_hz,
            lowest_hz: f1,
            low_end_db: v.low_end_db,
            note,
            freqs_hz: v.freqs_hz,
            mag_db: v.mag_db,
            spl,
            document: text.to_owned(),
        });
        r.issue = None;
        r.stage = MeasureRolloffStage::Done;
    }

    fn rolloff_stopped(&self, r: &mut Rolloff, issue: Option<MeasureIssue>) {
        r.stage = MeasureRolloffStage::Stopped;
        r.issue = issue.or_else(|| {
            Some(MeasureIssue::new(
                "measure.stopped",
                None,
                "the segment ended before its result",
            ))
        });
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_roll1;
