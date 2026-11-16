//! The measurement session (WP P5SES, `docs/core-merge/PLAN-P5.md` sections
//! 1.1, 1.2, 1.3 and 1.5): the DSPi's own signal generator plays, a client
//! pushes its microphone's frames, and the core turns them into a
//! `measurement` document. Sans-IO: no thread, no audio API, no sleep; time
//! from the core's clock seam; the client drives it with
//! [`DspiCore::measure_tick`] (about every 50 ms) and
//! [`DspiCore::measure_capture_push`].
//!
//! **Run kinds** (review RC11): **walked** (log sweeps, `SIGGEN_FLAG_WALK`,
//! one output per cycle, 2 to 8 passes), **summed** (the outputs of one
//! transition at once, the generator's `invert_mask` set to their net
//! polarity), **single live sweep** (one output, one sweep) and **finite
//! pink** (one output, at most 60 s). P5TUNE and P5LIVE start them through
//! the same calls; every one is under the same safety rules.
//!
//! **Safety** (S1 to S13, `measure/safety.rs`, `measure/preflight.rs`): no
//! host player; the session never writes a volume, a mute, a gain or anything
//! else than the generator's configuration and control; a preflight in words
//! before anything plays (S2, S6, S8, S11, S13); a closed-loop ramp per
//! output from a quiet start with the run's own stimulus, failing closed when
//! the microphone does not follow, a walked run at the minimum of its ways'
//! levels (S3); a capture clip, an output peak, the SPL cap, a limiter that
//! acts, anything foreign (a stop, a write, a volume, an input format, an
//! input stream, programme audio, a lost lock, a disconnect, a frame gap, a
//! cycle-identity mismatch, an overrun, the user's Stop) end the run with one
//! `SIGGEN_CTL_STOP_NOW` and nothing else (S4, S5); only finite generator
//! runs (S7); the device lock "Measuring" held as a dead-man lease (OD-P5n).
//!
//! **Privacy** (S12): the microphone's samples stay in this module's memory
//! for the run; they are never logged, put in an event, a document or the
//! diagnostics bundle. The result holds derived curves only.
//!
//! **Locks:** the session lives in [`MeasureSlot`], taken before `inner` and
//! never after it. The notification path (`notify.rs`, under `inner`) only
//! appends to a small tap on `Inner` ([`Tap`]); the session drains it. The
//! [`MeasureListener`] is called with every lock released.

use std::sync::{Arc, Mutex, MutexGuard};

use dspi_measure::types::{ChannelChain, Coupler, Placement, WayRole};
use dspi_session::notify::{Event, Notification, Source};
use dspi_transport::clock::Instant;
use serde::{Deserialize, Serialize};

use crate::callbacks::MeasureListener;
use crate::records::{SiggenConfig, SiggenStatus, SystemStatus};
use crate::{DspiCore, DspiError, Result};

mod plan;
mod preflight;
mod runs;
mod safety;
mod session;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5ses;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5ses_device;

pub(crate) use plan::Plan;
pub(crate) use preflight::PreflightOptions;
use session::Session;

/// Every notice key the session answers with (its rows in
/// `assets/notices/notices.json`, family `measure`).
#[cfg(test)]
pub(crate) const NOTICE_KEYS: &[&str] = &[
    "measure.no_generator",
    "measure.viewer",
    "measure.page_locked",
    "measure.guard_pending",
    "measure.ab_pending",
    "measure.volume_high",
    "measure.master_volume",
    "measure.input_audio",
    "measure.device_rate",
    "measure.too_long",
    "measure.no_output",
    "measure.output_off",
    "measure.output_muted",
    "measure.tweeter_highpass",
    "measure.crossfeed",
    "measure.bass",
    "measure.tube",
    "measure.subharmonic",
    "measure.loudness",
    "measure.limiter",
    "measure.chain_changed",
    "measure.input_eq",
    "measure.coupler",
    "measure.uncalibrated",
    "measure.muted",
    "measure.mic_not_following",
    "measure.mic_not_hearing",
    "measure.stopped",
    "measure.no_signal",
    "measure.abort.volume",
    "measure.abort.foreign",
    "measure.abort.preset",
    "measure.abort.input_format",
    "measure.abort.input_stream",
    "measure.abort.clip",
    "measure.abort.generator",
    "measure.abort.capture_rate",
    "measure.abort.frame_gap",
    "measure.abort.capture_clip",
    "measure.abort.spl",
    "measure.abort.lock",
    "measure.abort.overrun",
    "measure.abort.disconnected",
    "measure.abort.cycle_identity",
    "measure.abort.programme",
    "measure.abort.output_peak",
    "measure.abort.limiter",
];

// ── Records ─────────────────────────────────────────────────────────────

/// What a run plays (PLAN-P5 section 1.1, review RC11).
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureRunKind {
    /// Log sweeps, one output per cycle round-robin, 2 to 8 passes: responses
    /// with phase, arrivals and the timing quality gate.
    #[default]
    Walked,
    /// One sweep on several outputs at once, each with its polarity (the
    /// generator's `invert_mask`): the measured sum of a transition.
    Summed,
    /// One sweep on one output (the live view): magnitude only.
    LiveSweep,
    /// Finite pink noise on one output (at most 60 s): the microphone RTA.
    Pink,
}

/// A way of a loudspeaker, as the user confirmed the layout.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureRole {
    /// A full-range output.
    #[default]
    Full,
    /// A subwoofer.
    Sub,
    /// The lowest way of a multi-way speaker.
    Low,
    /// A middle way: needs an active high-pass to be measured (S6).
    Mid,
    /// The highest way: needs an active high-pass to be measured (S6).
    High,
}

impl MeasureRole {
    pub(crate) fn way(self) -> WayRole {
        match self {
            MeasureRole::Full => WayRole::Full,
            MeasureRole::Sub => WayRole::Sub,
            MeasureRole::Low => WayRole::Low,
            MeasureRole::Mid => WayRole::Mid,
            MeasureRole::High => WayRole::High,
        }
    }
}

/// Where the microphone is (owner requirement R3).
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasurePlacement {
    /// Speakers, the listening position.
    #[default]
    ListeningPosition,
    /// Speakers, 3 to 9 positions averaged (`count`).
    MultiPosition,
    /// Speakers, the microphone close to each driver: one run per output.
    NearField,
    /// Headphones on a coupler, 1 to 9 reseats (`count`).
    Headphones,
}

/// A headphone coupler type.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureCoupler {
    /// An IEC 60318-4 (711-type) ear simulator.
    #[default]
    Iec60318_4,
    /// A flat-plate coupler.
    FlatPlate,
    /// A head and torso simulator or another coupler with pinna.
    Hats,
}

impl MeasureCoupler {
    fn coupler(self) -> Coupler {
        match self {
            MeasureCoupler::Iec60318_4 => Coupler::Iec60318_4,
            MeasureCoupler::FlatPlate => Coupler::FlatPlate,
            MeasureCoupler::Hats => Coupler::Hats,
        }
    }
}

/// One output to measure.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureOutput {
    /// The output, 0-based.
    pub output: u8,
    /// Its way.
    pub role: MeasureRole,
    /// The speaker it belongs to (`L`, `R`, `Sub`), for the document.
    pub speaker: String,
    /// Its alignment group: the outputs of one group are measured in one
    /// walked run (PLAN-P5 section 1.4 step 4). Ignored by the other kinds.
    pub group: u8,
    /// A summed run plays this output inverted (its net programme polarity).
    pub inverted: bool,
}

/// What to measure and how (`measure_preflight`, `measure_start`). Every
/// number 0 (or absent) is the plan's default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureSetup {
    /// The run kind.
    pub kind: MeasureRunKind,
    /// The outputs, 1 to 16, each once.
    pub outputs: Vec<MeasureOutput>,
    /// Where the microphone is.
    pub placement: MeasurePlacement,
    /// Positions (3 to 9, multi-position) or reseats (1 to 9, headphones).
    pub count: u8,
    /// The coupler, for headphones.
    pub coupler: Option<MeasureCoupler>,
    /// The microphone's `mic` document's text; "" for an uncalibrated
    /// microphone (relative dB, no SPL cap).
    pub mic_document: String,
    /// One sweep, ms (1000 to 15000; 0: 5500, 2700 for a live sweep).
    pub sweep_ms: u32,
    /// The gap after each cycle, ms (300 to 2000; 0: 1000 for speakers, 300
    /// on a coupler).
    pub gap_ms: u32,
    /// Passes of a walked run (2 to 8; 0: 3).
    pub passes: u8,
    /// Pink noise, ms (1000 to 60000; 0: 10000).
    pub pink_ms: u32,
    /// The generator's ceiling, peak dBFS: lower than the mode's default, or
    /// higher up to the hard cap (-6 speakers, -20 headphones).
    pub ceiling_db: Option<f32>,
    /// The SPL cap with a calibrated microphone, dB SPL (default 95 speakers,
    /// 90 on a coupler; at most 105 and 100).
    pub spl_cap_db: Option<f32>,
    /// The capture's rate, Hz (0: 48000).
    pub capture_rate_hz: u32,
    /// The document's name ("" names it by the date).
    pub name: String,
    /// Play at the start level only (-50 dBFS speakers, -60 dBFS headphones):
    /// the ceiling is the start level, so the ramp never goes above it; every
    /// other rule unchanged (P5SPIKE's timing test).
    pub start_level_only: bool,
}

/// A walked run's timing (PLAN-P5 section 1.1 steps 6 and 7): the clock fit,
/// every cycle's residual and the quality gate's verdict. Non-finite numbers
/// read 0.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTiming {
    /// The run passed the timing quality gate.
    pub reliable: bool,
    /// The microphone's clock against the DSPi's, ppm.
    pub ratio_ppm: f64,
    /// The clock ratio was fitted (two passes or more).
    pub ratio_fitted: bool,
    /// The largest cycle residual, µs.
    pub max_residual_us: f64,
    /// The residual bound the run was held to, µs.
    pub residual_bound_us: f64,
    /// The largest phase disagreement between repeats, degrees.
    pub max_phase_spread_deg: f64,
    /// Passes in the run.
    pub passes: u16,
    /// Why not reliable, in words (empty when reliable).
    pub reasons: Vec<String>,
    /// Each cycle's output, in run order.
    pub cycle_outputs: Vec<u8>,
    /// Each cycle's residual from the fit, µs, in run order.
    pub cycle_residuals_us: Vec<f64>,
}

/// One preflight finding: a notice key and its words.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureIssue {
    /// The notice key (`notice_for_key`), e.g. `measure.volume_high`.
    pub key: String,
    /// The output it is about, if one.
    pub output: Option<u8>,
    /// What it is, in words (English; the key localises).
    pub message: String,
}

impl MeasureIssue {
    pub(crate) fn new(key: &str, output: Option<u8>, message: impl Into<String>) -> Self {
        MeasureIssue {
            key: key.to_owned(),
            output,
            message: message.into(),
        }
    }
}

/// What stands between the setup and a measurement (PLAN-P5 section 1.2).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurePreflight {
    /// Nothing blocks: `measure_start` may play.
    pub clear: bool,
    /// Every blocker, in words; nothing is played until they are clear.
    pub blockers: Vec<MeasureIssue>,
    /// What does not block but the user should know.
    pub warnings: Vec<MeasureIssue>,
}

/// Where a session stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasurePhase {
    /// No session on this core.
    #[default]
    Idle,
    /// The generator is idle; the microphone's noise floor is taken.
    PreRoll,
    /// The level ramp of one output (S3).
    Ramp,
    /// The run is playing.
    Run,
    /// The run is analysed, a slice per tick.
    Analysing,
    /// Move the microphone (or reseat the headphones), then
    /// `measure_next_position`.
    WaitPosition,
    /// The measurement is complete: `measure_result`.
    Done,
    /// Stopped on a safety rule or by the user: nothing is kept.
    Aborted,
    /// Stopped and asking the user (the microphone does not hear the output,
    /// no signal): nothing is kept.
    Failed,
}

/// One output's ramp result.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureLevel {
    /// The output.
    pub output: u8,
    /// The generator's peak level the ramp settled on, dBFS.
    pub level_db: f32,
}

/// A session's state: what the screens show and what the client must do.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureState {
    /// Where it stands.
    pub phase: MeasurePhase,
    /// The run kind.
    pub kind: MeasureRunKind,
    /// The session holds the generator or the lock: Stop is shown.
    pub active: bool,
    /// The client must push capture now (from the pre-roll to the end).
    pub wants_capture: bool,
    /// The capture rate the session expects, Hz.
    pub capture_rate_hz: u32,
    /// The position (or reseat) being measured, 1-based.
    pub position: u8,
    /// Positions (or reseats) in all.
    pub positions: u8,
    /// The run being measured, 1-based.
    pub run: u16,
    /// Runs in all.
    pub runs: u16,
    /// The output now ramping or playing alone, if one.
    pub output: Option<u8>,
    /// The generator's level now, peak dBFS, while it plays.
    pub level_db: Option<f32>,
    /// The ramps' results so far (this run).
    pub ramp_levels: Vec<MeasureLevel>,
    /// The run's level (the minimum of its ways' ramp levels).
    pub run_level_db: Option<f32>,
    /// Frames pushed since the session began.
    pub frames: u64,
    /// The capture frame at which the latest START was acknowledged.
    pub generator_start_frame: Option<u64>,
    /// STARTs sent in this session (a client's simulated microphone renders
    /// each one).
    pub generator_starts: u32,
    /// The current phase's progress, 0 to 1.
    pub progress: f32,
    /// Why it stopped, or what it asks.
    pub issue: Option<MeasureIssue>,
    /// The last run passed the timing quality gate (walked runs).
    pub timing_reliable: Option<bool>,
    /// `measure_result` has a document.
    pub result_ready: bool,
    /// The last analysed walked run's timing.
    pub timing: Option<MeasureTiming>,
    /// Gaps and repeats seen in the capture's frame indices (any one aborts
    /// the session, S5).
    pub frame_faults: u32,
}

// ── The notification tap (under `inner`) ───────────────────────────────

/// What the notification path saw that the session must judge (S5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TapEvent {
    /// A user or master volume changed (any origin).
    Volume,
    /// Another writer changed a parameter, or the whole image.
    ForeignWrite,
    /// A preset was loaded.
    PresetLoad,
    /// The input format changed.
    InputFormat,
    /// An input stream started or stopped.
    InputStream,
    /// `NOTIFY_EVT_SIGGEN_STATE`.
    Siggen {
        /// `SIGGEN_STATE_*`.
        state: u8,
        /// `SIGGEN_STOP_*`.
        reason: u8,
    },
    /// A clip event (0xE0).
    Clip,
    /// Notifications were lost: something may have changed unseen.
    Lost,
}

/// The tap the notification path fills while a session runs.
#[derive(Debug, Default)]
pub(crate) struct Tap {
    on: bool,
    events: Vec<TapEvent>,
    overflow: bool,
}

/// The most events kept between two ticks; past it the session aborts as on a
/// lost notification.
const TAP_MAX: usize = 256;

impl Tap {
    fn push(&mut self, e: TapEvent) {
        if self.events.len() >= TAP_MAX {
            self.overflow = true;
        } else {
            self.events.push(e);
        }
    }

    pub(crate) fn set(&mut self, on: bool) {
        self.on = on;
        self.events.clear();
        self.overflow = false;
    }

    pub(crate) fn drain(&mut self) -> Vec<TapEvent> {
        let mut v = std::mem::take(&mut self.events);
        if std::mem::take(&mut self.overflow) {
            v.push(TapEvent::Lost);
        }
        v
    }
}

/// The notification path's hook (`notify.rs`, under `inner`): note what a
/// running session must judge. `own` is the echo filter's verdict.
pub(crate) fn note_notification(tap: &mut Tap, note: &Notification, own: bool) {
    if !tap.on {
        return;
    }
    use dspi_proto::generated as g;
    let volume = |offset: usize| {
        let in_section =
            |base: usize, len: usize| offset >= base && offset < base.saturating_add(len);
        in_section(g::OFF_USER_VOLUME, g::LEN_USER_VOLUME)
            || in_section(g::OFF_MASTER_VOLUME, g::LEN_MASTER_VOLUME)
    };
    if note.lost {
        tap.push(TapEvent::Lost);
    }
    match &note.event {
        Event::ParamChanged { offset, .. } if volume(usize::from(*offset)) => {
            tap.push(TapEvent::Volume);
        }
        Event::ParamChanged { .. } if !own => tap.push(TapEvent::ForeignWrite),
        Event::BulkInvalidated { source } if !own => {
            if matches!(source, Source::Preset | Source::Factory) {
                tap.push(TapEvent::PresetLoad);
            } else {
                tap.push(TapEvent::ForeignWrite);
            }
        }
        Event::MasterVolume(_) => tap.push(TapEvent::Volume),
        Event::PresetLoaded { .. } => tap.push(TapEvent::PresetLoad),
        Event::InputFormat { .. } => tap.push(TapEvent::InputFormat),
        Event::SiggenState { state, reason, .. } => tap.push(TapEvent::Siggen {
            state: *state,
            reason: *reason,
        }),
        Event::I2sSlaveState { .. } | Event::AdatInputState { .. } => {
            tap.push(TapEvent::InputStream);
        }
        Event::Unknown { id, .. } if *id == dspi_proto::ext::NOTIFY_EVT_EXT_CLIP => {
            tap.push(TapEvent::Clip);
        }
        _ => {}
    }
}

// ── The slot ────────────────────────────────────────────────────────────

/// The session of one core (one device): at most one at a time.
#[derive(Default)]
pub(crate) struct MeasureSlot {
    session: Mutex<Option<Session>>,
    listener: crate::Slot<dyn MeasureListener>,
}

impl MeasureSlot {
    fn lock(&self) -> MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The device side of a session: what [`Session`] asks of the core. Every
/// write the session makes goes through here: the generator's configuration
/// and control, nothing else (S2).
pub(crate) trait Device {
    /// Stage `cfg` and play it; the reply arrived when this returns.
    fn start(&mut self, cfg: &SiggenConfig) -> Result<()>;
    /// `SIGGEN_CTL_STOP_NOW`.
    fn stop_now(&mut self) -> Result<()>;
    /// `SIGGEN_GET_STATUS`.
    fn status(&mut self) -> Result<SiggenStatus>;
    /// The meter block.
    fn meters(&mut self) -> Result<SystemStatus>;
    /// The limiter's gain reduction per output, dB.
    fn limiter_gr(&mut self) -> Result<Vec<f32>>;
    /// Whether the device lock is still ours; with `due`, its lease is
    /// renewed first (asked again of the hub over DSPi Link).
    fn renew_lock(&mut self, due: bool) -> bool;
    /// Notifications the tap holds.
    fn drain_tap(&mut self) -> Vec<TapEvent>;
}

/// [`Device`] over a [`DspiCore`]. Takes `inner` only inside each call.
struct CoreDevice<'a> {
    core: &'a DspiCore,
}

impl Device for CoreDevice<'_> {
    fn start(&mut self, cfg: &SiggenConfig) -> Result<()> {
        runs::check_config(cfg)?;
        self.core.siggen_set_config(cfg.clone())?;
        self.core.siggen_control(crate::consts::SIGGEN_CTL_START)
    }

    fn stop_now(&mut self) -> Result<()> {
        self.core.siggen_control(crate::consts::SIGGEN_CTL_STOP_NOW)
    }

    fn status(&mut self) -> Result<SiggenStatus> {
        self.core.siggen_get_status()
    }

    fn meters(&mut self) -> Result<SystemStatus> {
        self.core.fetch_status()
    }

    fn limiter_gr(&mut self) -> Result<Vec<f32>> {
        self.core.fetch_limiter_meter()
    }

    fn renew_lock(&mut self, due: bool) -> bool {
        self.core
            .measure_lock_renew(session::LOCK_REASON, session::LEASE, due)
    }

    fn drain_tap(&mut self) -> Vec<TapEvent> {
        self.core.lock().measure_tap.drain()
    }
}

// ── The calls ───────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Register (`Some`) or clear (`None`) the measurement listener: called
    /// when a session's phase changes, it stops, or it asks something. Called
    /// on the thread that pushed, ticked or stopped, with every core lock
    /// released; it may read `measure_state` but must not block.
    pub fn measure_set_listener(&self, listener: Option<Arc<dyn MeasureListener>>) {
        self.measure.listener.set(listener);
    }

    /// What stands between `setup` and a measurement, in words (PLAN-P5
    /// section 1.2): the volume (S2), the tweeter rule (S6), silence on the
    /// inputs (S8), locks and roles (S11), processing the maths does not model
    /// (S13), the plan's bounds (S7). Reads the device (the meters and the
    /// generator's caps); writes nothing.
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::InvalidArgument`] for
    /// a setup that is not one (no outputs, an output twice, a count out of
    /// range); a transfer failure of the reads.
    pub fn measure_preflight(&self, setup: MeasureSetup) -> Result<MeasurePreflight> {
        let plan = Plan::new(&setup)?;
        self.measure_preflight_plan(&plan, &PreflightOptions::default())
    }

    /// Start a measurement of `setup`: the preflight, the device lock
    /// "Measuring" (a 30 s lease, renewed by `measure_tick`), then the
    /// pre-roll. Nothing plays before the client has pushed the pre-roll and
    /// ticked. The client then pushes capture whenever `wants_capture` and
    /// calls `measure_tick` about every 50 ms until the phase is `done`,
    /// `aborted` or `failed`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a bad setup, a session
    /// already running on this core, or a blocker (named in the message);
    /// [`DspiError::ReadOnly`] (read-only mode or connection) and
    /// [`DspiError::Denied`] (a Viewer through a hub) with nothing sent;
    /// [`DspiError::Locked`] when another session holds the device lock, or
    /// this console holds it through `device_lock_acquire`;
    /// [`DspiError::Disconnected`].
    pub fn measure_start(&self, setup: MeasureSetup) -> Result<MeasureState> {
        self.measure_start_with(&setup, &PreflightOptions::default())
    }

    /// Push one block of the microphone's capture: mono samples, full scale
    /// ±1, starting at frame `first_frame` of the client's own count, at
    /// `rate_hz`, `clipped` when the converter clipped. The safety checks run
    /// first, O(block) (S4, S5): a gap or repeat in the frame indices, a
    /// clip, the SPL cap. No analysis runs here. A push with no session, or
    /// while the session wants none, is ignored.
    ///
    /// Errors: none for the capture's content (a fault ends the session and
    /// shows in the state); [`DspiError::InvalidArgument`] for more than
    /// 1 Mi samples in one block.
    pub fn measure_capture_push(
        &self,
        first_frame: u64,
        samples: Vec<f32>,
        rate_hz: u32,
        clipped: bool,
    ) -> Result<MeasureState> {
        if samples.len() > session::MAX_BLOCK {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "a capture block of {} samples (at most {})",
                    samples.len(),
                    session::MAX_BLOCK
                ),
            });
        }
        let (state, changed) = {
            let mut slot = self.measure.lock();
            let Some(s) = slot.as_mut() else {
                return Ok(MeasureState::default());
            };
            let before = s.phase_key();
            let mut dev = CoreDevice { core: self };
            s.push(
                &mut dev,
                first_frame,
                &samples,
                rate_hz,
                clipped,
                Instant::now(),
            );
            let changed = before != s.phase_key();
            let state = s.state();
            if s.finished() {
                self.measure_session_end(s);
            }
            (state, changed)
        };
        if changed {
            self.measure.listener.call(|l| l.on_measure());
        }
        Ok(state)
    }

    /// Drive the session: apply queued notifications, judge them (S5), renew
    /// the lease, poll the generator and the meters (about 4 Hz), start the
    /// next step, run one analysis slice. Call it about every 50 ms while a
    /// session is active, off the UI thread (it blocks for a few transfers).
    /// Returns the state; never fails.
    pub fn measure_tick(&self) -> MeasureState {
        // Apply what the device said first (its flags are kept for the
        // client's own `process_notifications`).
        if self.measure.lock().is_some() {
            self.settle_notifications();
        }
        let (state, changed) = {
            let mut slot = self.measure.lock();
            let Some(s) = slot.as_mut() else {
                return MeasureState::default();
            };
            let before = s.phase_key();
            let mut dev = CoreDevice { core: self };
            s.tick(&mut dev, Instant::now());
            let changed = before != s.phase_key();
            let state = s.state();
            if s.finished() {
                self.measure_session_end(s);
            }
            (state, changed)
        };
        if changed {
            self.measure.listener.call(|l| l.on_measure());
        }
        state
    }

    /// The microphone is at the next position (or the headphones reseated):
    /// go on with the next run.
    ///
    /// Errors: [`DspiError::InvalidArgument`] when the session is not waiting
    /// for a position.
    pub fn measure_next_position(&self) -> Result<MeasureState> {
        let state = {
            let mut slot = self.measure.lock();
            let s = slot.as_mut().ok_or_else(|| DspiError::InvalidArgument {
                msg: "no measurement is running".into(),
            })?;
            s.next_position(Instant::now())?;
            s.state()
        };
        self.measure.listener.call(|l| l.on_measure());
        Ok(state)
    }

    /// The user's Stop: `SIGGEN_CTL_STOP_NOW` where the device is reachable,
    /// the run discarded, the lock given back. Idempotent; never fails.
    pub fn measure_stop(&self) -> MeasureState {
        let (state, changed) = {
            let mut slot = self.measure.lock();
            let Some(s) = slot.as_mut() else {
                return MeasureState::default();
            };
            let mut dev = CoreDevice { core: self };
            let changed = s.stop_by_user(&mut dev);
            let state = s.state();
            self.measure_session_end(s);
            (state, changed)
        };
        if changed {
            self.measure.listener.call(|l| l.on_measure());
        }
        state
    }

    /// The session's state, without I/O.
    pub fn measure_state(&self) -> MeasureState {
        self.measure
            .lock()
            .as_ref()
            .map_or_else(MeasureState::default, Session::state)
    }

    /// The finished measurement as a `measurement` document's text (schema 1,
    /// derived curves only). File it with `measure_doc_put` (or the hub's).
    ///
    /// Errors: [`DspiError::InvalidArgument`] while no result is ready.
    pub fn measure_result(&self) -> Result<String> {
        self.measure
            .lock()
            .as_ref()
            .and_then(Session::result)
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: "no measurement result is ready".into(),
            })
    }

    /// Forget a finished session (its result included), so the screens show
    /// none. Refused while one is active.
    ///
    /// Errors: [`DspiError::InvalidArgument`] while a session is active.
    pub fn measure_clear(&self) -> Result<()> {
        let mut slot = self.measure.lock();
        if slot.as_ref().is_some_and(Session::active) {
            return Err(DspiError::InvalidArgument {
                msg: "a measurement is running: stop it first".into(),
            });
        }
        *slot = None;
        Ok(())
    }
}

impl DspiCore {
    /// [`DspiCore::measure_start`] with the options P5TUNE's verify runs need
    /// (their own guard allowed, the chains they were computed for).
    pub(crate) fn measure_start_with(
        &self,
        setup: &MeasureSetup,
        options: &PreflightOptions,
    ) -> Result<MeasureState> {
        let plan = Plan::new(setup)?;
        // The slot is held from the busy check to the session's install, so
        // two starts on one core cannot both pass (the slot comes before
        // `inner`, which the steps below take).
        let state = {
            let mut slot = self.measure.lock();
            if slot.as_ref().is_some_and(Session::active) {
                return Err(DspiError::InvalidArgument {
                    msg: "busy: a measurement is already running on this device".into(),
                });
            }
            let pre = self.measure_preflight_plan(&plan, options)?;
            if let Some(e) = preflight::refusal(&pre) {
                return Err(e);
            }
            let ctx = self.measure_context(&plan)?;
            // What the device already said (the echo of the user's own last
            // write, a volume lowered for this measurement) is applied before
            // the tap judges anything: only what happens from here on is
            // foreign.
            self.settle_notifications();
            // The lock before anything plays (a dead-man lease).
            let lock = self.take_device_lock(session::LOCK_REASON, session::LEASE, true)?;
            self.hold_device_lock(lock, session::LEASE, crate::presets::Holder::Measure);
            // A lock the client already holds itself (`device_lock_acquire`)
            // is not the session's to renew or give back: refused, nothing
            // played.
            if !self.measure_lock_renew(session::LOCK_REASON, session::LEASE, false) {
                return Err(DspiError::Locked);
            }
            self.lock().measure_tap.set(true);
            let session = Session::new(plan, ctx, Instant::now());
            let state = session.state();
            *slot = Some(session);
            state
        };
        self.measure.listener.call(|l| l.on_measure());
        Ok(state)
    }

    /// End a session's hold on the device: the tap off, the lock given back.
    /// Called with the slot held (the slot comes before `inner`).
    fn measure_session_end(&self, s: &mut Session) {
        if !s.release_pending() {
            return;
        }
        self.lock().measure_tap.set(false);
        self.end_held_lock(Some(crate::presets::Holder::Measure));
        s.released();
    }

    /// What the session needs of the device at its start: the chains, the
    /// serial and platform, the device's rate.
    fn measure_context(&self, plan: &Plan) -> Result<session::Context> {
        let rate = self
            .fetch_stat(crate::consts::STAT_SAMPLE_RATE)
            .unwrap_or(0);
        let mut inner = self.lock();
        let serial = inner.conn()?.serial.clone();
        let s = &inner.state;
        let platform = dspi_proto::Platform::from_id(s.platform_id)
            .name()
            .to_owned();
        let chains = plan
            .outputs
            .iter()
            .map(|o| (o.output, preflight::chain_of(s, o.output)))
            .collect();
        let limiter_outputs = plan
            .outputs
            .iter()
            .map(|o| o.output)
            .filter(|o| {
                s.limiter_enabled
                    .get(usize::from(*o))
                    .copied()
                    .unwrap_or(false)
            })
            .collect();
        Ok(session::Context {
            serial,
            platform,
            device_rate_hz: rate,
            chains,
            num_inputs: s.num_input_channels,
            limiter_outputs,
        })
    }
}

/// The measured chain of each output, as the session saw it at its start.
pub(crate) type Chains = Vec<(u8, ChannelChain)>;

/// A placement as the measurement maths takes it.
pub(crate) fn placement_of(setup: &MeasureSetup) -> Placement {
    match setup.placement {
        MeasurePlacement::ListeningPosition => Placement::ListeningPosition,
        MeasurePlacement::MultiPosition => Placement::MultiPosition { count: setup.count },
        MeasurePlacement::NearField => Placement::NearField,
        MeasurePlacement::Headphones => Placement::Headphones {
            coupler: setup.coupler.unwrap_or_default().coupler(),
            reseats: if setup.count == 0 { 1 } else { setup.count },
        },
    }
}
