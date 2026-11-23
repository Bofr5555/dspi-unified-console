//! The auto-tune (WP P5TUNE, PLAN-P5 section 1.4, T10, R5).
//!
//! A multi-way speaker (or a headphone side) is tuned from a `measurement`
//! document in steps the user sees one at a time; nothing is written until
//! the user applies a proposal:
//!
//! 1. **Layout** ([`DspiCore::measure_layout_proposal`]): speakers and ways
//!    from the device's state (an output with only a low-pass is a low way,
//!    a high-pass and a low-pass a mid, only a high-pass a high; a sub by name
//!    or by a low-pass at or below 120 Hz; the ways of one speaker share
//!    their programme source) with each output's programme feed (RC1). The
//!    user confirms or edits it, then measures it with `measure_start` (one
//!    walked run per alignment group).
//! 2. **Proposal** ([`DspiCore::measure_tune_propose`], the maths in
//!    `tune/maths.rs`): de-embedding, delay and polarity, crossover corner and
//!    levels **only from a run that passed the timing quality gate** (RC5d:
//!    otherwise the magnitudes stay and those writes are withdrawn with the
//!    reason), then the target fit. Every write is judged before it is offered
//!    (S9): the registry's range, the limits profile, the platform's delay
//!    range, no gain raised; a write the core would refuse withdraws its kind
//!    for that speaker and the proposal is made again without it. A
//!    measurement of another device is refused; one older than 30 days, or
//!    of another placement, is warned; one whose chains differ from the
//!    device's is re-predicted against the chains now (OD-P5o).
//! 3. **The summed check** ([`DspiCore::measure_tune_sum_check`], mandatory
//!    for any delay, polarity, crossover or level write, RC5f): one summed run
//!    per transition with the **current** chains and polarities, through the
//!    measurement session's own run kind (every S-rule holds); the measured
//!    sum against the sum predicted from the walked run, at most 1.5 dB RMS
//!    one octave each side. A speaker that fails gets its timing writes
//!    withdrawn and is proposed again without them (the EQ stays). A speaker
//!    is confirmed only when every one of its transitions' runs passed; these
//!    runs, and the verify's, never play louder than the measurement was made
//!    (its level is their ceiling; review P5-C2).
//! 4. **Apply** ([`DspiCore::measure_tune_apply`], S10): the preflight
//!    **again** with the chains the proposal was computed for (silence, locks,
//!    roles, read-only, the role through a hub; every crosspoint that feeds a
//!    tuned output as it was, review P5-C2), the "before" snapshot filed
//!    (the hub's store through the hub, else this console's; on the web it is
//!    kept as text), `apply_guard_begin("Auto-tune")`, then every write through
//!    `write_param` in **one undo group**. A refused write ends the group and
//!    the guard reverts.
//! 5. **Verify** ([`DspiCore::measure_tune_verify`]): inside the guard, a
//!    walked run of the tuned outputs and a summed run per transition with the
//!    new polarities, each measured against the prediction.
//! 6. **Keep or revert** ([`DspiCore::measure_tune_keep`],
//!    [`DspiCore::measure_tune_revert`]): Keep confirms the guard, files the
//!    "after" snapshot with its `tune` block and adds the tune's row to the
//!    measurement (updated in this console's store; a new copy on the hub,
//!    which never overwrites; the updated text for a measurement given as
//!    text); [`DspiCore::measure_tune_ab`] then compares the two. Revert or the
//!    guard's timeout put the "before" state back; nothing else changes.
//!
//! The runs are driven by [`DspiCore::measure_tune_tick`] (about every 50 ms,
//! off the UI thread, instead of `measure_tick`; the client pushes capture
//! as for any measurement). It also drives the guard's timer.
//!
//! **Locks** (in this order): the tune's state, then the session's slot, then
//! `inner`; the tune's view is a leaf. The measurement listener is called
//! while the tune's state is held: from a listener, read
//! ([`DspiCore::measure_tune_state`], `measure_state`), never drive the tune.

use std::collections::BTreeSet;
use std::sync::MutexGuard;

use dspi_measure::doc::{MeasurementDoc, TargetDoc, TuneRecord};
use dspi_measure::target::{
    DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse, default_target,
};
use dspi_measure::types::{ChannelChain, LogGrid, Placement, Response};
use dspi_measure::xover::{
    Driver, HighpassOverride, HighpassRule, SumMode, predict_sum, summed_check,
};
use dspi_session::output_label; // -- WP ROLE0 --
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::live::{MeasureLiveCurve, curve, levelled_target, repredict};
use super::plan::Plan;
use super::preflight::{self, PreflightOptions, chain_of};
use super::safety;
use super::{
    MeasureCoupler, MeasureIssue, MeasureOutput, MeasurePhase, MeasurePlacement, MeasureRole,
    MeasureRunKind, MeasureSetup, MeasureSlot, MeasureState,
};
use crate::ab::AbCompare;
use crate::guarded::{GuardPhase, GuardedApply};
use crate::measure_doc::CoreSectionModel;
use crate::write_param::ParamValue;
use crate::{DspiCore, DspiError, Result};

mod maths;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5tune;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5tune_device;

use maths::{Allow, Bounds, Computed, WayIn};

/// The label of the guard and the undo group.
pub(crate) const TUNE_LABEL: &str = "Auto-tune";
/// The guard's timeout when the caller gives 0 and the verify's length is
/// not known, s (S10: room for the verify runs).
pub(crate) const TUNE_GUARD_DEFAULT_S: u32 = 300;
// -- WP P5TRUST -- coordinator decision (c): the guard's length scales with
// the planned verify (a stereo three-way at 5.5 s sweeps verifies in about
// 175 s plus its analysis, close to the flat 300 s).
/// The guard's time beyond the planned verify, s: the analysis, and the
/// user reading the verify and choosing Keep or Revert.
pub(crate) const TUNE_GUARD_MARGIN_S: u32 = 120;
/// The planned verify's length is taken this much longer (the analysis
/// between runs, a slow client's ticks), percent.
pub(crate) const TUNE_GUARD_SLACK_PCT: u64 = 25;
// -- review P5TRUST --
/// The second Keep after a failed verify counts only this long after the
/// first was refused, ms: a double click, or a client that sends a refused
/// call again, is not the user's second confirmation.
pub(crate) const TUNE_KEEP_CONFIRM_MIN_MS: u64 = 1_000;
/// Each ramp step of a planned run, ms: the pre-roll, the short sweep and
/// its analysis.
const RAMP_STEP_MS: u64 = 2_600;
/// A measurement older than this is warned, ms (30 days).
const OLD_MS: i64 = 30 * 24 * 3600 * 1000;
/// What the tune's algorithm calls itself in the "after" snapshot.
pub(crate) const ALGORITHM: &str = "dspi-core p5tune 1";
/// The verify's per-way comparison: points within this of the way's loudest
/// predicted level, dB.
const VERIFY_SPAN_DB: f64 = 30.0;
// -- WP P5TRUST2 -- (P5TM's finding) each verified output is judged too.
/// A verified output whose measured response is further than this from its
/// prediction (RMS, dB, 100 Hz to 10 kHz within 30 dB of its loudest) fails
/// the verify. RC5f's bound for a summed run against its prediction: the same
/// question (does the run confirm what the maths predicted?) for one way.
/// Before, a layout without transitions (no summed run, no verified sum)
/// always passed, even 6.4 dB off (P5TM). The simulated rooms' verifies read
/// 0.003 to 0.18 dB per output.
pub(crate) const VERIFY_OUTPUT_MAX_RMS_DB: f64 = dspi_measure::xover::SUMMED_CHECK_MAX_RMS_DB;
/// The verify's whole-speaker comparison band, Hz.
const VERIFY_SUM_HZ: (f64, f64) = (200.0, 10_000.0);
/// The graph's smoothing of the proposal's curves, 1/N octave.
const CURVE_SMOOTHING: u8 = 24;

/// The tune's notice keys (`assets/notices/notices.json`, family `measure`).
#[cfg(test)]
pub(crate) const NOTICE_KEYS: &[&str] = &[
    maths::KEY_UNRELIABLE,
    maths::KEY_SUM_FAILED,
    maths::KEY_NO_TIMING,
    maths::KEY_LIMIT,
    maths::KEY_NO_FIT,
    KEY_OTHER_DEVICE,
    KEY_OLD,
    KEY_PLACEMENT,
    KEY_CHAINS,
    KEY_SUM_NEEDED,
    KEY_MIC,
    KEY_NOT_KEPT,
    KEY_KEEP_UNVERIFIED,
    // -- WP P5TRUST2 --
    KEY_GUARD_CAPPED,
    // -- WP P5HOLD --
    KEY_KEEP_DURING_VERIFY,
];
const KEY_OTHER_DEVICE: &str = "measure.tune.other_device";
const KEY_OLD: &str = "measure.tune.old";
const KEY_PLACEMENT: &str = "measure.tune.placement";
const KEY_CHAINS: &str = "measure.tune.chains_changed";
const KEY_SUM_NEEDED: &str = "measure.tune.sum_check_needed";
const KEY_MIC: &str = "measure.tune.mic";
const KEY_NOT_KEPT: &str = "measure.tune.not_kept";
// -- WP P5TRUST -- coordinator decision (b)
const KEY_KEEP_UNVERIFIED: &str = "measure.tune.keep_unverified";
// -- WP P5TRUST2 -- (review P5TRUST N7)
const KEY_GUARD_CAPPED: &str = "measure.tune.guard_capped";
// -- WP P5HOLD -- (review P5-D D3)
const KEY_KEEP_DURING_VERIFY: &str = "measure.tune.keep_during_verify";

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

// ── Records ─────────────────────────────────────────────────────────────

/// One output of a layout (PLAN-P5 section 1.4 step 1).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureLayoutOutput {
    /// The output, 0-based.
    pub output: u8,
    /// Its way.
    pub role: MeasureRole,
    /// The speaker it belongs to (`L`, `R`, `Sub`, ...): the ways of one
    /// speaker are tuned together.
    pub speaker: String,
    /// Its alignment group: one walked run measures a group.
    pub group: u8,
    /// The output's user name, if it has one.
    pub name: String,
    /// The input that feeds it most (its programme source), if one.
    pub feed_input: Option<u8>,
    /// The net crosspoint gain of that source, dB.
    pub feed_gain_db: f32,
    /// The net polarity of its feeding crosspoints is inverted.
    pub feed_inverted: bool,
}

/// The speakers and ways the tune works on (`measure_layout_proposal`;
/// confirmed or edited by the user).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureLayout {
    /// The outputs, each once.
    pub outputs: Vec<MeasureLayoutOutput>,
    /// The layout recognised (`3way`, `2way`, `headphones`, `full range`),
    /// "" when none.
    pub template: String,
    /// Remarks in words.
    pub notes: Vec<String>,
}

/// What the tune may change (`measure_tune_propose`). Every number 0 is the
/// default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureTuneOptions {
    /// A `target` document's text; "" for the placement's default target.
    pub target_document: String,
    /// The measurement's microphone's `mic` document; "" looks it up by the
    /// id the measurement names (needed for the summed and verify runs).
    pub mic_document: String,
    /// "Keep the crossover, align only": no corner or level change.
    pub keep_crossover: bool,
    /// Keep the current delays (polarity is still judged).
    pub keep_delays: bool,
    /// Keep every output gain of the crossover.
    pub keep_levels: bool,
    /// No EQ fit.
    pub keep_eq: bool,
    /// The corner search's window each side, octaves (0: 0.5; at most 2).
    pub window_octaves: f64,
    /// The fit's range, Hz (0: 20 Hz to 300 Hz for speakers, 20 Hz to 10 kHz
    /// for headphones).
    pub fit_min_hz: f64,
    /// See `fit_min_hz`.
    pub fit_max_hz: f64,
    // -- WP ROLE0 --
    /// Take the layout's roles, speakers and groups from the device's stored
    /// speaker layout (MPLAN 2.2, `roles: "layout"`); a layout with no
    /// outputs then means every output of it. Refused
    /// (`measure.layout_unconfirmed`) while the user has not confirmed it.
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub roles_from_layout: bool,
}

/// What a write changes.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureTuneKind {
    /// A crossover corner.
    #[default]
    Crossover,
    /// An output delay.
    Delay,
    /// The net polarity of an output's feeding crosspoints.
    Polarity,
    /// An output gain (a way's level, cut only).
    Level,
    /// An output gain cut for the EQ's boost.
    Headroom,
    /// A PEQ band.
    Eq,
}

/// One parameter of a write: what `write_param` gets.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneParam {
    /// The registry path (`out.delay`, `eq.freq`, `mix`, ...).
    pub path: String,
    /// Its indices.
    pub indices: Vec<u8>,
    /// The value written.
    pub value: ParamValue,
}

/// One change of a proposal, in words, with the parameters it writes.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneWrite {
    /// The output it changes.
    pub output: u8,
    /// What it changes.
    pub kind: MeasureTuneKind,
    /// A delay, polarity, crossover or level write: offered only from a
    /// timing-reliable run, applied only after the summed check passed.
    pub timing: bool,
    /// The change in words ("Output 3 (L high): delay 0.0000 → 0.2100 ms").
    pub words: String,
    /// What is written, in order.
    pub params: Vec<MeasureTuneParam>,
}

/// A speaker's curves: before, predicted, the target levelled to before.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneCurves {
    /// The speaker.
    pub speaker: String,
    /// The ways' sum with phase (a timing-reliable run); else their power sum.
    pub coherent: bool,
    /// The speaker as it is now (programme audio).
    pub before: MeasureLiveCurve,
    /// The speaker with the proposal.
    pub predicted: MeasureLiveCurve,
    /// The target, levelled to `before`.
    pub target: MeasureLiveCurve,
}

/// One transition of a proposal.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneTransition {
    /// The speaker.
    pub speaker: String,
    /// The lower way's output.
    pub low_output: u8,
    /// The upper way's output.
    pub high_output: u8,
    /// The corner now, Hz.
    pub corner_before_hz: f64,
    /// The proposed corner, Hz.
    pub corner_hz: f64,
    /// The upper way's delay minus the lower way's, as proposed, ms.
    pub relative_delay_ms: f64,
    /// The upper way's polarity is flipped relative to the lower way's.
    pub flipped: bool,
    /// The summed check's RMS difference, dB, once run.
    pub sum_check_rms_db: Option<f64>,
    /// The verify's summed RMS difference, dB, once run.
    pub verify_rms_db: Option<f64>,
}

/// Where the summed check stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureTuneCheck {
    /// The proposal has no timing write: nothing to check.
    #[default]
    NotNeeded,
    /// Not run yet: the timing writes cannot be applied.
    Pending,
    /// Every checked transition passed.
    Passed,
    /// A transition failed: its speaker's timing writes are withdrawn.
    Failed,
}

/// A proposal: nothing is written until it is applied.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneProposal {
    /// Its id, for `measure_tune_apply` (a new id after a summed check).
    pub id: u32,
    /// The device it is for.
    pub serial: String,
    /// The measurement's id ("" for one given as text).
    pub measurement_id: String,
    /// The measurement's name.
    pub measurement_name: String,
    /// Every write, in the order it is applied.
    pub writes: Vec<MeasureTuneWrite>,
    /// Each speaker's curves.
    pub curves: Vec<MeasureTuneCurves>,
    /// Each transition.
    pub transitions: Vec<MeasureTuneTransition>,
    /// The limits it was held to, in words.
    pub held: Vec<String>,
    /// What is not offered, and why.
    pub withdrawn: Vec<MeasureIssue>,
    /// What the user should know (an old measurement, another placement).
    pub warnings: Vec<MeasureIssue>,
    /// Remarks of the maths, in words.
    pub notes: Vec<String>,
    /// The measurement passed the timing quality gate.
    pub timing_reliable: bool,
    /// The summed check.
    pub sum_check: MeasureTuneCheck,
}

/// Where a tune stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureTuneStage {
    /// No tune on this core.
    #[default]
    Idle,
    /// A proposal waits to be checked or applied.
    Proposed,
    /// The summed runs play.
    SumCheck,
    /// Applied inside the guard: verify, Keep or Revert.
    Applied,
    /// The verify runs play (inside the guard).
    Verifying,
    /// Verified (inside the guard): Keep or Revert.
    Verified,
    /// Kept: the A/B compare may be started.
    Kept,
    /// Reverted (by the user, the timeout or a failed write).
    Reverted,
    // -- WP REVERTW --
    /// The guard ended without putting everything back (part of it, nothing,
    /// or it is not known): `issue` holds the `revert.*` notice and what was
    /// not put back. The user must check the settings or restore a backup
    /// (a snapshot): the "before" snapshot is `snapshot_before_id`.
    RevertIncomplete,
}

// -- WP REVERTW --
/// A Revert's end: as [`tune_end_of`], but a guard left kept (another
/// console took the device over first) did not put anything back.
fn tune_revert_end(g: &GuardedApply) -> (MeasureTuneStage, Option<MeasureIssue>) {
    if g.phase == GuardPhase::Kept {
        let words = if g.message.is_empty() {
            "the change stays: nothing was written back".to_owned()
        } else {
            g.message.clone()
        };
        return (
            MeasureTuneStage::RevertIncomplete,
            Some(MeasureIssue::new(
                crate::guarded::NOTICE_REVERT_NOT_WRITTEN,
                None,
                words,
            )),
        );
    }
    tune_end_of(g)
}

/// Where a tune stands once its guard ended as `g` says, and the issue to
/// show for a revert that was not fully successful.
fn tune_end_of(g: &GuardedApply) -> (MeasureTuneStage, Option<MeasureIssue>) {
    match g.phase {
        GuardPhase::Kept => (MeasureTuneStage::Kept, None),
        GuardPhase::Reverted => (MeasureTuneStage::Reverted, None),
        _ => {
            let key = if g.revert_notice.is_empty() {
                crate::guarded::NOTICE_REVERT_UNKNOWN
            } else {
                g.revert_notice.as_str()
            };
            let mut words = vec![if g.message.is_empty() {
                "the guard ended without putting the settings from before back".to_owned()
            } else {
                g.message.clone()
            }];
            words.extend(g.notes.iter().cloned());
            (
                MeasureTuneStage::RevertIncomplete,
                Some(MeasureIssue::new(key, None, words.join(" "))),
            )
        }
    }
}

/// One output's verify.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneVerifyOutput {
    /// The output.
    pub output: u8,
    /// RMS difference of the measured response from the predicted one, dB,
    /// over the points both trust within 30 dB of the loudest; `None` when
    /// too few.
    pub rms_db: Option<f64>,
}

/// One speaker's verified sum.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneVerifySum {
    /// The speaker.
    pub speaker: String,
    /// RMS difference of the verified programme sum (the verify run's ways
    /// summed with their phase) from the predicted one, 200 Hz to 10 kHz,
    /// dB; `None` without timing.
    pub rms_db: Option<f64>,
}

/// What the verify runs found.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneVerify {
    /// Each tuned output.
    pub outputs: Vec<MeasureTuneVerifyOutput>,
    /// Each speaker with timing.
    pub sums: Vec<MeasureTuneVerifySum>,
    /// Each transition with its summed run.
    pub transitions: Vec<MeasureTuneTransition>,
    /// The verify's walked run passed the timing quality gate.
    pub timing_reliable: bool,
    /// Every summed run, verified sum and verified output within 1.5 dB RMS
    /// of the prediction.
    pub passed: bool,
    /// Remarks in words.
    pub notes: Vec<String>,
}

/// A tune's state (`measure_tune_tick`, `measure_tune_state`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneState {
    /// Where it stands.
    pub stage: MeasureTuneStage,
    /// A summed or verify run is under way: tick and push capture.
    pub running: bool,
    /// The current proposal's id.
    pub proposal_id: Option<u32>,
    /// The run under way, 1-based, and the runs of this step.
    pub run: u16,
    /// See `run`.
    pub runs: u16,
    /// The measurement session under the run.
    pub session: MeasureState,
    /// The guard, once applied.
    pub guard: GuardedApply,
    /// The verify's result.
    pub verify: Option<MeasureTuneVerify>,
    /// Why the last step stopped, if it did.
    pub issue: Option<MeasureIssue>,
    /// The "before" snapshot's id ("" when not filed: its text is kept).
    pub snapshot_before_id: String,
    /// The "after" snapshot's id, once kept.
    pub snapshot_after_id: String,
}

/// What Keep did (`measure_tune_keep`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureTuneKeep {
    /// The guard's end: `kept`, or the revert's when the timeout came first.
    pub guard: GuardedApply,
    /// The "before" snapshot's id and text.
    pub snapshot_before_id: String,
    /// See `snapshot_before_id`.
    pub snapshot_before: String,
    /// The "after" snapshot's id ("" when not filed) and text (with its
    /// `tune` block).
    pub snapshot_after_id: String,
    /// See `snapshot_after_id`.
    pub snapshot_after: String,
    /// The measurement's id with the tune's row: the same one in this
    /// console's store, a new copy's on the hub, "" for a measurement given
    /// as text.
    pub measurement_id: String,
    /// The measurement's text with the tune's row (file it when it was
    /// given as text).
    pub measurement: String,
    /// What could not be filed, in words.
    pub notes: Vec<String>,
}

// ── The state ───────────────────────────────────────────────────────────

/// Where the measurement came from.
#[derive(Debug, Clone, PartialEq)]
enum Source {
    Text,
    Local(String),
    Hub(String),
}

/// What a proposal was made from.
#[derive(Debug, Clone)]
struct Input {
    doc: MeasurementDoc,
    source: Source,
    layout: MeasureLayout,
    options: MeasureTuneOptions,
    ways: Vec<WayIn>,
    bounds: Bounds,
    target: Response,
    mic_document: String,
    warnings: Vec<MeasureIssue>,
    serial: String,
}

/// One run of a step.
#[derive(Debug, Clone)]
enum Purpose {
    /// The summed check of a speaker's transition (indices into the
    /// computed speakers and their transitions).
    SumCheck { speaker: usize, transition: usize },
    /// The verify's walked run.
    VerifyWalked,
    /// The verify's summed run of a transition.
    VerifySummed { speaker: usize, transition: usize },
}

#[derive(Debug, Clone)]
struct RunItem {
    purpose: Purpose,
    setup: MeasureSetup,
}

#[derive(Debug, Clone, Default)]
struct Runs {
    queue: Vec<RunItem>,
    current: Option<RunItem>,
    done: Vec<(RunItem, String)>,
    total: u16,
}

/// The tune of one core.
#[derive(Debug, Default)]
pub(crate) struct Tune {
    next_id: u32,
    stage: MeasureTuneStage,
    input: Option<Input>,
    proposal: Option<MeasureTuneProposal>,
    computed: Option<Computed>,
    allow: Allow,
    /// Speakers whose summed check passed.
    confirmed: BTreeSet<String>,
    runs: Option<Runs>,
    issue: Option<MeasureIssue>,
    guard_id: u64,
    before: Option<(String, String)>,
    after: Option<(String, String)>,
    applied: Vec<ChannelChain>,
    verify: Option<MeasureTuneVerify>,
    // -- WP P5TRUST --
    /// Keep after a verify that did not pass was refused once: the next Keep
    /// is the user's second, explicit confirmation.
    keep_armed: bool,
    // -- review P5TRUST --
    /// The last verify of the applied tune did not pass. It stays through a
    /// Stop, and through a new verify that ends without a verdict, until a
    /// verify passes or another apply: Keep asks for the second confirmation
    /// in every stage, not only while the failed verdict is on show.
    verify_failed: bool,
    /// When the first Keep was refused (the guard's clock): the second one
    /// counts from [`TUNE_KEEP_CONFIRM_MIN_MS`] later, so a double click is
    /// one press.
    keep_refused_at: Option<dspi_transport::clock::Instant>,
    // -- WP ROLL1 -- (review RA item 5)
    /// Who last started the tune's runs or applied it: its runs started from
    /// `measure_tune_tick` (any thread) are theirs, and its guard's revert on
    /// timeout is attributed to them.
    actor: crate::actor::JobActor,
}

impl MeasureSlot {
    fn tune_lock(&self) -> MutexGuard<'_, Tune> {
        self.tune.lock().unwrap_or_else(|p| p.into_inner())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn placement_setup(p: &Placement) -> (MeasurePlacement, Option<MeasureCoupler>, u8) {
    match p {
        Placement::Headphones { coupler, .. } => (
            MeasurePlacement::Headphones,
            Some(match coupler {
                dspi_measure::types::Coupler::Iec60318_4 => MeasureCoupler::Iec60318_4,
                dspi_measure::types::Coupler::FlatPlate => MeasureCoupler::FlatPlate,
                dspi_measure::types::Coupler::Hats => MeasureCoupler::Hats,
            }),
            1,
        ),
        _ => (MeasurePlacement::ListeningPosition, None, 0),
    }
}

/// -- review P5-C2 -- The ceiling of the tune's own runs (the summed check,
/// the verify), peak dBFS: the level the measurement was made at, at most the
/// placement's default ceiling. The user's lowered ceiling or an SPL stop is
/// in that level, and the setup of a document does not say them: a tune
/// never plays louder than its measurement did (S3; before the review these
/// runs ramped to the default -12 dBFS whatever the measurement's ceiling).
fn run_ceiling(doc: &MeasurementDoc) -> f32 {
    let default = match doc.measurement.placement {
        Placement::Headphones { .. } => super::plan::CEILING_HEADPHONES_DB,
        _ => super::plan::CEILING_SPEAKERS_DB,
    };
    let level = doc.measurement.stimulus.level_db;
    if level.is_finite() {
        (level as f32).clamp(-120.0, default)
    } else {
        // No level in the document: the quietest start level there is.
        super::plan::START_HEADPHONES_DB
    }
}

fn role_of(r: dspi_measure::types::WayRole) -> MeasureRole {
    use dspi_measure::types::WayRole as W;
    match r {
        W::Full => MeasureRole::Full,
        W::Sub => MeasureRole::Sub,
        W::Low => MeasureRole::Low,
        W::Mid => MeasureRole::Mid,
        W::High => MeasureRole::High,
    }
}

/// RMS of `a - b` over `lo..=hi` on the points both trust and `keep` passes.
fn rms_between(
    a: &Response,
    b: &Response,
    lo: f64,
    hi: f64,
    keep: &dyn Fn(usize) -> bool,
) -> Option<f64> {
    if a.grid != b.grid || a.mag_db.len() != b.mag_db.len() {
        return None;
    }
    let freqs = a.grid.freqs().ok()?;
    let ok = |r: &Response, i: usize| {
        r.valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false))
    };
    let (mut ss, mut n) = (0.0f64, 0u32);
    for (i, f) in freqs.iter().enumerate() {
        if *f < lo || *f > hi || !ok(a, i) || !ok(b, i) || !keep(i) {
            continue;
        }
        if let (Some(x), Some(y)) = (a.mag_db.get(i), b.mag_db.get(i)) {
            let d = x - y;
            if d.is_finite() {
                ss += d * d;
                n = n.saturating_add(1);
            }
        }
    }
    (n >= 12).then(|| (ss / f64::from(n)).sqrt())
}

// -- WP P5TRUST2 --, review P5-D
/// Why a verify without any sum (no summed run, no verified sum: a layout
/// without transitions) confirms nothing, or `None`: it compared no output
/// at all, or (review P5-D) one of its outputs could not be compared with its
/// prediction (too few trusted points). Before the review only "no output at
/// all" failed, so of two full-range outputs one could go unjudged, whatever
/// it played, and the verify still passed on the other. With a sum the
/// speaker's sum judges its ways, and a way too narrow to compare alone (a
/// subwoofer below 100 Hz) does not fail it.
fn verify_unjudged(v: &MeasureTuneVerify) -> Option<String> {
    if !v.sums.is_empty() || !v.transitions.is_empty() {
        return None;
    }
    let missing: Vec<String> = v
        .outputs
        .iter()
        .filter(|o| o.rms_db.is_none())
        .map(|o| format!("output {}", u16::from(o.output).saturating_add(1)))
        .collect();
    if v.outputs.is_empty() || missing.len() == v.outputs.len() {
        Some("the verify could compare nothing with its prediction".into())
    } else if missing.is_empty() {
        None
    } else {
        Some(format!(
            "the verify could not compare {} with its prediction",
            missing.join(", ")
        ))
    }
}

fn invert_mask(outputs: &[(u8, bool)]) -> u32 {
    outputs
        .iter()
        .filter(|(_, inv)| *inv)
        .fold(0u32, |m, (o, _)| {
            m | 1u32.checked_shl(u32::from(*o)).unwrap_or(0)
        })
}

fn date_words() -> String {
    let secs = u64::try_from(crate::events::now_ms() / 1000).unwrap_or(0);
    super::runs::utc_text(secs)
}

impl Tune {
    fn state(&self, core: &DspiCore) -> MeasureTuneState {
        let (run, runs) = self.runs.as_ref().map_or((0, 0), |r| {
            (
                u16::try_from(r.done.len().saturating_add(1)).unwrap_or(u16::MAX),
                r.total,
            )
        });
        MeasureTuneState {
            stage: self.stage,
            running: self.runs.is_some(),
            proposal_id: self.proposal.as_ref().map(|p| p.id),
            run,
            runs,
            session: core.measure_state(),
            guard: core.apply_status(),
            verify: self.verify.clone(),
            issue: self.issue.clone(),
            snapshot_before_id: self
                .before
                .as_ref()
                .map(|b| b.0.clone())
                .unwrap_or_default(),
            snapshot_after_id: self.after.as_ref().map(|a| a.0.clone()).unwrap_or_default(),
        }
    }

    fn forget(&mut self) {
        let next = self.next_id;
        *self = Tune {
            next_id: next,
            ..Tune::default()
        };
    }
}

// ── The core ────────────────────────────────────────────────────────────

impl DspiCore {
    /// Publish the tune's state to the view (a leaf the listener may read).
    fn tune_publish(&self, t: &Tune) -> MeasureTuneState {
        let st = t.state(self);
        *self
            .measure
            .tune_view
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = st.clone();
        st
    }

    /// The measurement's text and where it came from: text as it is, else a
    /// filed id (the hub's copy first, it wins), else this console's.
    fn tune_load(&self, measurement: &str) -> Result<(MeasurementDoc, Source)> {
        let m = measurement.trim();
        if m.starts_with('{') {
            let doc = MeasurementDoc::from_text(m)
                .map_err(|e| invalid(format!("the measurement: {e}")))?;
            return Ok((doc, Source::Text));
        }
        if let Ok(h) = self.hub_measure_doc_get(m.to_owned()) {
            let doc = MeasurementDoc::from_text(&h.document)
                .map_err(|e| invalid(format!("the measurement: {e}")))?;
            return Ok((doc, Source::Hub(m.to_owned())));
        }
        let text = self.measure_doc_get(m.to_owned())?;
        let doc = MeasurementDoc::from_text(&text)
            .map_err(|e| invalid(format!("the measurement: {e}")))?;
        Ok((doc, Source::Local(m.to_owned())))
    }

    /// The microphone's document the measurement names: the caller's, else
    /// the one filed under its id (the hub's, then this console's); its
    /// calibration's hash must be the measurement's.
    fn tune_mic(&self, doc: &MeasurementDoc, given: &str) -> std::result::Result<String, String> {
        let Some(mic) = &doc.measurement.mic else {
            return Ok(String::new());
        };
        let text = if given.trim().is_empty() {
            self.hub_measure_doc_get(mic.id.clone())
                .map(|h| h.document)
                .or_else(|_| self.measure_doc_get(mic.id.clone()))
                .map_err(|_| {
                    format!(
                        "the measurement's microphone ({}) is not filed here: give its \
                         calibration to check and verify",
                        mic.name
                    )
                })?
        } else {
            given.to_owned()
        };
        let m = dspi_measure::doc::MicDoc::from_text(&text)
            .map_err(|e| format!("the microphone document: {e}"))?;
        if !mic.cal_hash.is_empty() && m.mic.source_hash != mic.cal_hash {
            return Err(format!(
                "the microphone's calibration is not the one the measurement was made with ({})",
                mic.name
            ));
        }
        Ok(text)
    }

    /// The layout's ways from the document and the device now.
    fn tune_ways(&self, doc: &MeasurementDoc, layout: &MeasureLayout) -> Result<Vec<WayIn>> {
        if layout.outputs.is_empty() {
            return Err(invalid("the layout has no output"));
        }
        let s = self.get_state();
        let mut seen = BTreeSet::new();
        let mut ways = Vec::new();
        for lo in &layout.outputs {
            if !seen.insert(lo.output) {
                return Err(invalid(format!("output {} twice in the layout", lo.output)));
            }
            if lo.output >= s.num_output_channels {
                return Err(invalid(format!(
                    "the DSPi has no output {}",
                    u16::from(lo.output).saturating_add(1)
                )));
            }
            let om = doc
                .measurement
                .outputs
                .iter()
                .find(|o| o.output == lo.output)
                .ok_or_else(|| {
                    invalid(format!(
                        "the measurement has no output {}",
                        u16::from(lo.output).saturating_add(1)
                    ))
                })?;
            let response = om.response.to_response();
            let n = response.mag_db.len();
            let o = usize::from(lo.output);
            let feeds = s
                .matrix_routing
                .iter()
                .enumerate()
                .filter(|(_, r)| r.get(o).copied().unwrap_or(false))
                .filter_map(|(i, _)| {
                    let input = u8::try_from(i).ok()?;
                    let gain = s.matrix_gain.get(i).and_then(|g| g.get(o)).copied()?;
                    let inv = s
                        .matrix_invert
                        .get(i)
                        .and_then(|g| g.get(o))
                        .copied()
                        .unwrap_or(false);
                    Some((input, gain, inv))
                })
                .collect();
            ways.push(WayIn {
                output: lo.output,
                role: lo.role,
                speaker: lo.speaker.clone(),
                measured_chain: om.chain.chain.clone(),
                chain_now: chain_of(&s, lo.output),
                snr_db: maths::stored_snr(om, &response.grid, n),
                arrival_ms: om.arrival_ms,
                spread_db: om.response.spread_db.clone(),
                response,
                feeds,
                channel: s.num_input_channels.saturating_add(lo.output),
            });
        }
        Ok(ways)
    }

    /// The bounds of the device and its limits profile for `ways`.
    fn tune_bounds(&self, ways: &[WayIn], headphones: bool) -> Bounds {
        let mut inner = self.lock();
        let max = f64::from(crate::state::max_delay_ms_for(&inner.state));
        let profile = crate::limits::profile(&mut inner);
        let mut min_d = 0.0f64;
        let mut max_d = max;
        let mut highpass = Vec::new();
        for w in ways {
            // -- WP ROLE0 -- a high pass at 1 kHz or above keeps the tweeter's
            // rule, whatever the way is called (MPLAN 2.2, the effective role)
            let effective = super::effective_role(w.role, &w.chain_now);
            let has_own = profile
                .as_ref()
                .and_then(|p| p.output(w.output))
                .is_some_and(|o| o.highpass_min_hz.is_some());
            if effective != w.role && !has_own {
                highpass.extend(HighpassRule::default_for(effective.way()).map(|rule| {
                    HighpassOverride {
                        output: w.output,
                        rule,
                    }
                }));
            }
            if let Some(o) = profile.as_ref().and_then(|p| p.output(w.output)) {
                if let Some(d) = o.delay_min_ms {
                    min_d = min_d.max(f64::from(d));
                }
                if let Some(d) = o.delay_max_ms {
                    max_d = max_d.min(f64::from(d));
                }
                if let Some(hz) = o.highpass_min_hz {
                    let base = HighpassRule::default_for(effective.way()); // -- WP ROLE0 --
                    let slope = o
                        .highpass_min_slope_db
                        .or(base.map(|b| b.min_slope_db))
                        .unwrap_or(preflight::MIN_SLOPE_DB);
                    // -- WP ROLE0 -- a profile never lowers a tweeter's floor
                    let (min_hz, min_slope_db) = preflight::tweeter_floor(effective, hz, slope);
                    highpass.push(HighpassOverride {
                        output: w.output,
                        rule: HighpassRule {
                            min_hz,
                            min_slope_db,
                        },
                    });
                }
            }
        }
        let placement_cap = if headphones { 6.0 } else { 3.0 };
        let cap = profile
            .as_ref()
            .and_then(|p| p.max_peq_boost_db)
            .map(|c| f64::from(c.max(0.0)));
        Bounds {
            min_delay_ms: min_d,
            max_delay_ms: max_d.max(min_d),
            highpass,
            max_boost_db: cap.map_or(placement_cap, |c| c.min(placement_cap)),
            boost_from_profile: cap.is_some_and(|c| c < placement_cap),
            headphones,
        }
    }

    /// Every write of `c` judged as the core would judge it (S9): `Err` with
    /// the first refusal's output, timing flag and words.
    fn tune_judge(&self, input: &Input, c: &Computed) -> Option<(u8, bool, String)> {
        let mut inner = self.lock();
        for w in &c.writes {
            let way_i = input.ways.iter().position(|x| x.output == w.output)?;
            let way = input.ways.get(way_i)?;
            let after = c.chains.get(way_i)?;
            for p in &w.params {
                let v: dspi_proto::value::Value = p.value.clone().into();
                if let Some(d) = dspi_proto::registry::by_path(&p.path)
                    && !matches!(v, dspi_proto::value::Value::Bytes(_))
                    && let Err(e) = d.kind.validate(&v)
                {
                    return Some((w.output, w.timing, format!("{}: {e}", w.words)));
                }
                let refused = match p.path.as_str() {
                    "out.delay" => v
                        .as_f32()
                        .ok_or_else(|| invalid("no delay"))
                        .and_then(|ms| crate::params::check_delay(&inner.state, "out.delay", ms))
                        .and_then(|()| {
                            crate::limits::check_write(&mut inner, &p.path, &p.indices, &v)
                        })
                        .err(),
                    "out.gain" => {
                        let raised = v.as_f32().is_none_or(|g| g > way.chain_now.gain_db + 0.049);
                        if raised {
                            Some(invalid("an output gain would be raised"))
                        } else {
                            crate::limits::check_write(&mut inner, &p.path, &p.indices, &v).err()
                        }
                    }
                    "eq.type" | "eq.freq" | "eq.q" | "eq.gain" | "eq.bypass" => {
                        let band = p.indices.get(1).copied().unwrap_or(0);
                        let spec = match band.checked_sub(maths::XOVER_BASE) {
                            Some(x) => after.crossover.get(usize::from(x)),
                            None => after.peq.get(usize::from(band)),
                        };
                        spec.and_then(|b| {
                            crate::limits::check_band(
                                &mut inner,
                                way.channel,
                                band,
                                &maths::params_of(b),
                            )
                            .err()
                        })
                    }
                    _ => None,
                };
                if let Some(e) = refused {
                    return Some((w.output, w.timing, format!("{}: {e}", w.words)));
                }
            }
        }
        None
    }

    /// -- review P5-C2 -- Withdraw the timing and the fit of every speaker
    /// (a refusal no single speaker's withdrawal removes).
    fn tune_withdraw_all(input: &Input, allow: &mut Allow, why: &str) {
        for w in &input.ways {
            allow.no_timing.entry(w.speaker.clone()).or_insert_with(|| {
                (
                    maths::KEY_LIMIT.to_owned(),
                    format!("a write the core would refuse: {why}"),
                )
            });
            allow
                .no_fit
                .entry(w.speaker.clone())
                .or_insert_with(|| format!("a write the core would refuse: {why}"));
        }
    }

    /// The proposal of `input` with `allow`, judged (a refused write
    /// withdraws its kind for that speaker and the proposal is made again).
    fn tune_make(
        &self,
        t: &mut Tune,
        input: &Input,
        check: MeasureTuneCheck,
    ) -> MeasureTuneProposal {
        let mut allow = t.allow.clone();
        let mut c = maths::compute(
            &input.doc,
            &input.ways,
            &input.options,
            &input.bounds,
            &input.target,
            &allow,
            &CoreSectionModel,
        );
        // -- review P5-C2 -- S9: a proposal with a write the core would refuse
        // is never offered. Each round withdraws the refused write's kind for
        // its speaker; a refusal that withdrawing does not remove (the common
        // headroom cut on an output of another speaker), or one left when the
        // rounds are spent, withdraws everything.
        let rounds = input.ways.len().saturating_mul(2).saturating_add(2);
        let mut settled = false;
        for _ in 0..rounds {
            let Some((output, timing, why)) = self.tune_judge(input, &c) else {
                settled = true;
                break;
            };
            let speaker = input
                .ways
                .iter()
                .find(|w| w.output == output)
                .map(|w| w.speaker.clone())
                .unwrap_or_default();
            let before = allow.clone();
            if timing {
                allow.no_timing.entry(speaker).or_insert_with(|| {
                    (
                        maths::KEY_LIMIT.to_owned(),
                        format!("a write the core would refuse: {why}"),
                    )
                });
            } else {
                allow
                    .no_fit
                    .entry(speaker)
                    .or_insert_with(|| format!("a write the core would refuse: {why}"));
            }
            if allow == before {
                Self::tune_withdraw_all(input, &mut allow, &why);
            }
            c = maths::compute(
                &input.doc,
                &input.ways,
                &input.options,
                &input.bounds,
                &input.target,
                &allow,
                &CoreSectionModel,
            );
        }
        if !settled && let Some((_, _, why)) = self.tune_judge(input, &c) {
            Self::tune_withdraw_all(input, &mut allow, &why);
            c = maths::compute(
                &input.doc,
                &input.ways,
                &input.options,
                &input.bounds,
                &input.target,
                &allow,
                &CoreSectionModel,
            );
            if self.tune_judge(input, &c).is_some() {
                c.writes.clear();
            }
        }
        t.allow = allow;
        t.next_id = t.next_id.wrapping_add(1).max(1);
        let mut curves = Vec::new();
        let mut transitions = Vec::new();
        for sp in &c.speakers {
            if let (Some(b), Some(p)) = (&sp.before, &sp.predicted) {
                let tgt = levelled_target(&input.target, b);
                if let (Some(before), Some(predicted), Some(target)) = (
                    curve(b, CURVE_SMOOTHING),
                    curve(p, CURVE_SMOOTHING),
                    curve(&tgt, CURVE_SMOOTHING),
                ) {
                    curves.push(MeasureTuneCurves {
                        speaker: sp.name.clone(),
                        coherent: sp.coherent,
                        before,
                        predicted,
                        target,
                    });
                }
            }
            for (k, tr) in sp.transitions.iter().enumerate() {
                let (Some(lo), Some(hi)) = (
                    sp.ways.get(tr.low).and_then(|i| input.ways.get(*i)),
                    sp.ways.get(tr.high).and_then(|i| input.ways.get(*i)),
                ) else {
                    continue;
                };
                let now = |w: &WayIn| maths::lowpass_corner(&w.chain_now).map(f64::from);
                let fallback = now(lo).unwrap_or(0.0);
                let (before_hz, after_hz, rel, flipped) = sp
                    .xo
                    .get(k)
                    .copied()
                    .unwrap_or((fallback, fallback, 0.0, false));
                transitions.push(MeasureTuneTransition {
                    speaker: sp.name.clone(),
                    low_output: lo.output,
                    high_output: hi.output,
                    corner_before_hz: before_hz,
                    corner_hz: after_hz,
                    relative_delay_ms: rel,
                    flipped,
                    sum_check_rms_db: None,
                    verify_rms_db: None,
                });
            }
        }
        let has_timing = c.writes.iter().any(|w| w.timing);
        let proposal = MeasureTuneProposal {
            id: t.next_id,
            serial: input.serial.clone(),
            measurement_id: match &input.source {
                Source::Text => String::new(),
                Source::Local(id) | Source::Hub(id) => id.clone(),
            },
            measurement_name: input.doc.meta.name.clone(),
            writes: c.writes.clone(),
            curves,
            transitions,
            held: c.held.clone(),
            withdrawn: c.withdrawn.clone(),
            // -- WP P5TRUST2 -- the guard's cap said (review P5TRUST N7).
            warnings: input
                .warnings
                .iter()
                .cloned()
                .chain(Self::tune_guard_warning(input, &c))
                .collect(),
            notes: c.notes.clone(),
            timing_reliable: input.doc.measurement.capture.timing_reliable,
            sum_check: if has_timing {
                check
            } else if check == MeasureTuneCheck::Failed {
                MeasureTuneCheck::Failed
            } else {
                MeasureTuneCheck::NotNeeded
            },
        };
        t.computed = Some(c);
        t.proposal = Some(proposal.clone());
        proposal
    }

    /// A summed run's setup for outputs `pair` (way, its polarity).
    fn tune_summed_setup(input: &Input, pair: &[(&WayIn, bool)], name: &str) -> MeasureSetup {
        let (placement, coupler, count) = placement_setup(&input.doc.measurement.placement);
        let st = &input.doc.measurement.stimulus;
        MeasureSetup {
            kind: MeasureRunKind::Summed,
            outputs: pair
                .iter()
                .map(|(w, inv)| MeasureOutput {
                    output: w.output,
                    role: w.role,
                    speaker: w.speaker.clone(),
                    group: 0,
                    inverted: *inv,
                })
                .collect(),
            placement,
            count,
            coupler,
            mic_document: input.mic_document.clone(),
            sweep_ms: st.duration_ms,
            gap_ms: st.gap_ms,
            capture_rate_hz: input.doc.measurement.capture.rate_hz,
            ceiling_db: Some(run_ceiling(&input.doc)),
            name: name.to_owned(),
            ..MeasureSetup::default()
        }
    }

    // -- WP P5TRUST -- the verify's runs, for the verify and for the guard's
    // length.
    /// The verify's runs of an applied tune: a walked run of the tuned
    /// outputs, then a summed run per transition with the polarities of
    /// `applied` (the chains after the apply; the proposal's where empty).
    fn tune_verify_queue(input: &Input, c: &Computed, applied: &[ChannelChain]) -> Vec<RunItem> {
        let (placement, coupler, count) = placement_setup(&input.doc.measurement.placement);
        let st = &input.doc.measurement.stimulus;
        let walked = MeasureSetup {
            kind: MeasureRunKind::Walked,
            outputs: input
                .ways
                .iter()
                .map(|w| MeasureOutput {
                    output: w.output,
                    role: w.role,
                    speaker: w.speaker.clone(),
                    group: input
                        .layout
                        .outputs
                        .iter()
                        .find(|o| o.output == w.output)
                        .map_or(0, |o| o.group),
                    inverted: false,
                })
                .collect(),
            placement,
            count,
            coupler,
            mic_document: input.mic_document.clone(),
            sweep_ms: st.duration_ms,
            gap_ms: st.gap_ms,
            passes: st.repeat.max(3),
            capture_rate_hz: input.doc.measurement.capture.rate_hz,
            ceiling_db: Some(run_ceiling(&input.doc)),
            name: "Auto-tune verify".into(),
            ..MeasureSetup::default()
        };
        let mut queue = vec![RunItem {
            purpose: Purpose::VerifyWalked,
            setup: walked,
        }];
        for (si, sp) in c.speakers.iter().enumerate() {
            if sp.drivers.is_none() {
                continue;
            }
            for (k, tr) in sp.transitions.iter().enumerate() {
                let (Some(lo), Some(hi)) = (
                    sp.ways.get(tr.low).and_then(|i| input.ways.get(*i)),
                    sp.ways.get(tr.high).and_then(|i| input.ways.get(*i)),
                ) else {
                    continue;
                };
                let inv = |w: &WayIn| {
                    input
                        .ways
                        .iter()
                        .position(|x| x.output == w.output)
                        .and_then(|i| applied.get(i))
                        .map_or(w.chain_now.feed.inverted, |c| c.feed.inverted)
                };
                queue.push(RunItem {
                    purpose: Purpose::VerifySummed {
                        speaker: si,
                        transition: k,
                    },
                    setup: Self::tune_summed_setup(
                        input,
                        &[(lo, inv(lo)), (hi, inv(hi))],
                        "Auto-tune verify (summed)",
                    ),
                });
            }
        }
        queue
    }

    /// The guard's length for applying `c` (coordinator decision (c)): the
    /// planned verify, [`TUNE_GUARD_SLACK_PCT`] longer, plus
    /// [`TUNE_GUARD_MARGIN_S`]; at most the guard's longest. The planned
    /// verify is each run's capture plus its ramp's steps from the start
    /// level to the ceiling. [`TUNE_GUARD_DEFAULT_S`] when a run cannot be
    /// planned.
    fn tune_guard_s(input: &Input, c: &Computed) -> u32 {
        match Self::tune_guard_planned_s(input, c) {
            Some(s) => u32::try_from(s)
                .unwrap_or(u32::MAX)
                .min(crate::guarded::APPLY_GUARD_MAX_TIMEOUT_S),
            None => TUNE_GUARD_DEFAULT_S,
        }
    }

    // -- WP P5TRUST2 -- (review P5TRUST N7) the planned length before the
    // guard's cap, so a proposal can say when the cap cuts it.
    /// The guard's planned length before the guard's longest is applied, s;
    /// `None` when a run cannot be planned.
    fn tune_guard_planned_s(input: &Input, c: &Computed) -> Option<u64> {
        let rate = match input.doc.measurement.stimulus.device_rate_hz {
            r @ (44_100 | 48_000 | 96_000) => r,
            _ => 48_000,
        };
        let mut ms: u64 = 0;
        for item in Self::tune_verify_queue(input, c, &c.chains) {
            let Ok(plan) = Plan::new(&item.setup) else {
                return None;
            };
            let span = f64::from(plan.ceiling_db - plan.start_db).max(0.0);
            let steps = ((span / f64::from(safety::RAMP_STEP_DB)).ceil() as u64).saturating_add(1);
            for run in &plan.runs {
                let Ok(run_ms) = plan.run_ms(run, rate) else {
                    return None;
                };
                let ramps = (run.outputs.len() as u64).saturating_mul(steps);
                ms = ms
                    .saturating_add(run_ms)
                    .saturating_add(ramps.saturating_mul(RAMP_STEP_MS));
            }
        }
        Some(
            ms.saturating_mul(100 + TUNE_GUARD_SLACK_PCT)
                .div_ceil(100_000)
                .saturating_add(u64::from(TUNE_GUARD_MARGIN_S)),
        )
    }

    // -- WP P5TRUST2 -- (review P5TRUST N7)
    /// The proposal's warning when the planned verify needs a longer guard
    /// than the guard's longest: the guard is still capped, and would revert
    /// the tune during or right after the verify.
    fn tune_guard_warning(input: &Input, c: &Computed) -> Option<MeasureIssue> {
        let planned = Self::tune_guard_planned_s(input, c)?;
        let cap = crate::guarded::APPLY_GUARD_MAX_TIMEOUT_S;
        (planned > u64::from(cap)).then(|| {
            MeasureIssue::new(
                KEY_GUARD_CAPPED,
                None,
                format!(
                    "the verify needs a guard of about {planned} s with its margin, longer \
                     than the guard's longest ({cap} s): the guard reverts the tune after {cap} s, \
                     perhaps before the verify ends"
                ),
            )
        })
    }

    /// Start the next run of `t`, or end the step (`finish`).
    fn tune_next(&self, t: &mut Tune) {
        let Some(runs) = t.runs.as_ref() else {
            return;
        };
        if runs.queue.is_empty() {
            self.tune_finish(t);
            return;
        }
        let options = match t.stage {
            MeasureTuneStage::Verifying => PreflightOptions {
                allow_guard: Some(t.guard_id),
                expect_chains: None,
            },
            _ => PreflightOptions {
                allow_guard: None,
                expect_chains: t.input.as_ref().map(|i| {
                    i.ways
                        .iter()
                        .map(|w| (w.output, w.chain_now.clone()))
                        .collect()
                }),
            },
        };
        let Some(item) = t.runs.as_mut().map(|r| r.queue.remove(0)) else {
            return;
        };
        let _ = self.measure_clear();
        match self.measure_start_with(&item.setup, &options) {
            Ok(_) => {
                // -- WP ROLL1 -- the run is the tune's starter's, whichever
                // thread ticked it on
                self.measure_set_actor(t.actor.actor());
                if let Some(r) = t.runs.as_mut() {
                    r.current = Some(item);
                }
            }
            Err(e) => {
                t.runs = None;
                t.issue = Some(MeasureIssue::new("measure.stopped", None, e.to_string()));
                t.stage = if t.stage == MeasureTuneStage::Verifying {
                    MeasureTuneStage::Applied
                } else {
                    MeasureTuneStage::Proposed
                };
            }
        }
    }

    /// Every run of the step is done: judge them.
    fn tune_finish(&self, t: &mut Tune) {
        let Some(runs) = t.runs.take() else {
            return;
        };
        // -- WP P5HOLD -- a Keep refused during the verify is answered by
        // its verdict.
        if t.stage == MeasureTuneStage::Verifying
            && t.issue
                .as_ref()
                .is_some_and(|i| i.key == KEY_KEEP_DURING_VERIFY)
        {
            t.issue = None;
        }
        match t.stage {
            MeasureTuneStage::SumCheck => self.tune_sum_done(t, &runs.done),
            MeasureTuneStage::Verifying => self.tune_verify_done(t, &runs.done),
            _ => {}
        }
    }

    /// The summed check's verdicts (RC5f).
    fn tune_sum_done(&self, t: &mut Tune, done: &[(RunItem, String)]) {
        let (Some(input), Some(c)) = (t.input.clone(), t.computed.clone()) else {
            t.stage = MeasureTuneStage::Proposed;
            return;
        };
        let mut failed: Vec<(String, String)> = Vec::new();
        let mut results: Vec<(String, u8, u8, Option<f64>)> = Vec::new();
        // -- review P5-C2 -- a speaker is confirmed only when every one of
        // its transitions' runs passed (a run whose result could not be read
        // leaves the check incomplete: fail closed).
        let mut passes: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for (item, text) in done {
            let Purpose::SumCheck {
                speaker,
                transition,
            } = item.purpose
            else {
                continue;
            };
            let Some(sp) = c.speakers.get(speaker) else {
                continue;
            };
            let verdict = sum_verdict(&input, sp, transition, text, &|w: &WayIn| {
                w.chain_now.clone()
            });
            match verdict {
                Ok((lo, hi, rms, passed, corner)) => {
                    results.push((sp.name.clone(), lo, hi, Some(rms)));
                    if passed {
                        let n = passes.entry(sp.name.clone()).or_insert(0);
                        *n = n.saturating_add(1);
                    } else {
                        failed.push((
                            sp.name.clone(),
                            format!(
                                "the summed run of outputs {} and {} differs from the prediction by \
                                 {rms:.2} dB RMS through {corner:.0} Hz (at most 1.5 dB): the walked \
                                 run's timing is not trusted; delay, polarity and crossover changes \
                                 are withdrawn",
                                u16::from(lo).saturating_add(1),
                                u16::from(hi).saturating_add(1)
                            ),
                        ));
                    }
                }
                Err(why) => failed.push((sp.name.clone(), format!("the summed check: {why}"))),
            }
        }
        for (name, why) in &failed {
            t.allow.no_timing.insert(
                name.clone(),
                (maths::KEY_SUM_FAILED.to_owned(), why.clone()),
            );
        }
        let need: Vec<(String, usize)> = c
            .speakers
            .iter()
            .filter(|sp| sp.timing && !sp.transitions.is_empty())
            .map(|sp| (sp.name.clone(), sp.transitions.len()))
            .collect();
        let failed_names: Vec<String> = failed.iter().map(|(n, _)| n.clone()).collect();
        t.confirmed = fully_checked(&need, &passes, &failed_names);
        let check = if !failed.is_empty() {
            MeasureTuneCheck::Failed
        } else if need.iter().all(|(n, _)| t.confirmed.contains(n)) {
            MeasureTuneCheck::Passed
        } else {
            t.issue = Some(MeasureIssue::new(
                "measure.stopped",
                None,
                "the summed check is not complete: run it again",
            ));
            MeasureTuneCheck::Pending
        };
        let mut p = self.tune_make(t, &input, check);
        for tr in &mut p.transitions {
            tr.sum_check_rms_db = results
                .iter()
                .find(|(s, lo, hi, _)| {
                    *s == tr.speaker && *lo == tr.low_output && *hi == tr.high_output
                })
                .and_then(|r| r.3);
        }
        t.proposal = Some(p);
        t.stage = MeasureTuneStage::Proposed;
    }

    /// The verify's verdicts.
    fn tune_verify_done(&self, t: &mut Tune, done: &[(RunItem, String)]) {
        let (Some(input), Some(c)) = (t.input.clone(), t.computed.clone()) else {
            t.stage = MeasureTuneStage::Applied;
            return;
        };
        let applied = t.applied.clone();
        let chain_after = |w: &WayIn| -> ChannelChain {
            input
                .ways
                .iter()
                .position(|x| x.output == w.output)
                .and_then(|i| applied.get(i).cloned())
                .unwrap_or_else(|| w.chain_now.clone())
        };
        let mut v = MeasureTuneVerify::default();
        let model = CoreSectionModel;
        let mut passed = true;
        for (item, text) in done {
            match item.purpose {
                Purpose::VerifyWalked => {
                    let Ok(doc) = MeasurementDoc::from_text(text) else {
                        v.notes
                            .push("the verify's measurement could not be read".into());
                        passed = false;
                        continue;
                    };
                    v.timing_reliable = doc.measurement.capture.timing_reliable;
                    for w in &input.ways {
                        let Some(om) = doc
                            .measurement
                            .outputs
                            .iter()
                            .find(|o| o.output == w.output)
                        else {
                            continue;
                        };
                        let measured = om.response.to_response();
                        let rms =
                            repredict(&w.response, &w.measured_chain, &chain_after(w), &model)
                                .ok()
                                .and_then(|pred| {
                                    let top = pred
                                        .mag_db
                                        .iter()
                                        .copied()
                                        .filter(|m| m.is_finite())
                                        .fold(f64::NEG_INFINITY, f64::max);
                                    let keep = |i: usize| {
                                        pred.mag_db
                                            .get(i)
                                            .is_some_and(|m| *m >= top - VERIFY_SPAN_DB)
                                    };
                                    rms_between(&measured, &pred, 100.0, 10_000.0, &keep)
                                });
                        // -- WP P5TRUST2 --
                        if let Some(r) = rms
                            && r > VERIFY_OUTPUT_MAX_RMS_DB
                        {
                            passed = false;
                            v.notes.push(format!(
                                "{} is {r:.2} dB RMS off its prediction (at most \
                                 {VERIFY_OUTPUT_MAX_RMS_DB:.1} dB)",
                                maths::who(w)
                            ));
                        }
                        v.outputs.push(MeasureTuneVerifyOutput {
                            output: w.output,
                            rms_db: rms,
                        });
                    }
                    for sp in &c.speakers {
                        let Some(drivers) = &sp.drivers else {
                            continue;
                        };
                        let rms = if v.timing_reliable {
                            verified_sum(&input, sp, drivers, &doc, &chain_after)
                        } else {
                            None
                        };
                        if rms.is_none_or(|r| r > dspi_measure::xover::SUMMED_CHECK_MAX_RMS_DB) {
                            passed = false;
                        }
                        v.sums.push(MeasureTuneVerifySum {
                            speaker: sp.name.clone(),
                            rms_db: rms,
                        });
                    }
                }
                Purpose::VerifySummed {
                    speaker,
                    transition,
                } => {
                    let Some(sp) = c.speakers.get(speaker) else {
                        continue;
                    };
                    match sum_verdict(&input, sp, transition, text, &chain_after) {
                        Ok((lo, hi, rms, ok, corner)) => {
                            passed &= ok;
                            v.transitions.push(MeasureTuneTransition {
                                speaker: sp.name.clone(),
                                low_output: lo,
                                high_output: hi,
                                corner_before_hz: corner,
                                corner_hz: corner,
                                verify_rms_db: Some(rms),
                                ..MeasureTuneTransition::default()
                            });
                        }
                        Err(why) => {
                            passed = false;
                            v.notes.push(format!("{}: {why}", sp.name));
                        }
                    }
                }
                Purpose::SumCheck { .. } => {}
            }
        }
        // -- WP P5TRUST2 -- a verify that judged nothing confirms nothing.
        // Review P5-D: nor does it confirm an output it could not compare
        // when no sum judges that output for it.
        if let Some(why) = verify_unjudged(&v) {
            passed = false;
            v.notes.push(why);
        }
        v.passed = passed;
        t.verify_failed = !passed;
        t.verify = Some(v);
        t.stage = MeasureTuneStage::Verified;
    }

    // -- WP P5TRUST, review P5TRUST --
    /// Coordinator decision (b): after a verify that did not pass, the first
    /// Keep is refused (`measure.tune.keep_unverified`, the state's `issue`
    /// says it, nothing changes), and so is any Keep within
    /// [`TUNE_KEEP_CONFIRM_MIN_MS`] of that refusal; a later one is the
    /// user's second confirmation. The failed verdict holds in every stage of
    /// the applied tune (after a Stop, during a new verify, after one that
    /// ended without a verdict).
    ///
    /// WP P5HOLD (review P5-D D3): **no Keep while a verify runs**
    /// (`measure.tune.keep_during_verify`, nothing changes, the guard and the
    /// verify run on): a Keep then would keep before the verdict, in one
    /// press. Keep before any verify has started keeps its rule (one press;
    /// the proposal said "Not verified").
    ///
    /// WP RCFIX (review RC, RC1): the second confirmation is **the user's
    /// only**. Another actor's Keep of a tune whose verify failed is refused
    /// however often it asks (the same key), and leaves the two-step Keep as
    /// the user left it: it neither arms nor uses it.
    fn tune_keep_gate(&self, t: &mut Tune) -> Result<()> {
        if t.stage == MeasureTuneStage::Verifying {
            // The issue's words follow its notice's title ("The verify is
            // still running"); the error carries the key.
            let what = "wait for its verdict, then Keep or Revert";
            t.issue = Some(MeasureIssue::new(KEY_KEEP_DURING_VERIFY, None, what));
            self.tune_publish(t);
            return Err(invalid(format!(
                "{KEY_KEEP_DURING_VERIFY}: the verify is still running; {what}"
            )));
        }
        if !t.verify_failed {
            return Ok(());
        }
        // -- WP RCFIX --
        if crate::actor::JobActor::capture(self).actor() != crate::actor::EMBEDDER {
            return Err(invalid(format!(
                "{KEY_KEEP_UNVERIFIED}: the verify did not confirm the tune; only the user may \
                 keep it, in the app"
            )));
        }
        let now = crate::guarded::now();
        let confirmed = t.keep_armed
            && t.keep_refused_at.is_some_and(|at| {
                now.saturating_duration_since(at)
                    >= std::time::Duration::from_millis(TUNE_KEEP_CONFIRM_MIN_MS)
            });
        if confirmed {
            return Ok(());
        }
        if !t.keep_armed {
            t.keep_armed = true;
            t.keep_refused_at = Some(now);
        }
        let why = format!(
            "{KEY_KEEP_UNVERIFIED}: the verify did not confirm the tune; Keep it only if \
             you are sure (call Keep again to keep it anyway, or Revert)"
        );
        t.issue = Some(MeasureIssue::new(KEY_KEEP_UNVERIFIED, None, why.clone()));
        self.tune_publish(t);
        Err(invalid(why))
    }

    /// The gate of [`DspiCore::apply_confirm`]: when the waiting guard is an
    /// applied tune's, its Keep rule ([`Self::tune_keep_gate`]) holds for the
    /// generic Keep too.
    pub(crate) fn tune_confirm_gate(&self) -> Result<()> {
        let mut t = self.measure.tune_lock();
        if !matches!(
            t.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verifying | MeasureTuneStage::Verified
        ) {
            return Ok(());
        }
        let g = self.apply_status();
        if g.id != t.guard_id || g.phase != GuardPhase::Waiting {
            return Ok(());
        }
        self.tune_keep_gate(&mut t)
    }

    /// The guard of an applied tune ended other than kept: the tune is
    /// reverted (its runs stopped).
    fn tune_guard_ended(&self, t: &mut Tune) {
        if !matches!(
            t.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verifying | MeasureTuneStage::Verified
        ) {
            return;
        }
        let g = self.apply_status();
        if g.id == t.guard_id && g.phase == GuardPhase::Waiting {
            return;
        }
        if t.runs.take().is_some() && self.measure_state().active {
            self.measure_stop_session();
        }
        let _ = self.measure_clear();
        // -- WP REVERTW --
        let (stage, issue) = tune_end_of(&g);
        t.stage = stage;
        if issue.is_some() {
            t.issue = issue;
        }
    }

    /// File a snapshot document's text: on the hub through the hub, else in
    /// this console's store; "" with a note when neither keeps it.
    fn tune_file_snapshot(&self, text: &str, notes: &mut Vec<String>) -> String {
        match self.hub_snapshots_put(String::new(), text.to_owned()) {
            Ok(p) => return p.snapshot.id,
            Err(DspiError::Unsupported) => {}
            Err(e) => notes.push(format!("the hub did not file the snapshot: {e}")),
        }
        let serial = self
            .lock()
            .conn
            .as_ref()
            .map(|c| c.serial.clone())
            .unwrap_or_default();
        match self.snapshot_file_copy(&serial, text) {
            Ok(i) => i.id,
            Err(e) => {
                notes.push(format!(
                    "the snapshot is not filed ({e}): keep its text to go back"
                ));
                String::new()
            }
        }
    }
}

/// -- review P5-C2 -- The speakers whose summed check is complete: every one
/// of their transitions (`need`: speaker, transitions) has a run that passed
/// (`passes`), and none failed.
fn fully_checked(
    need: &[(String, usize)],
    passes: &std::collections::BTreeMap<String, usize>,
    failed: &[String],
) -> BTreeSet<String> {
    need.iter()
        .filter(|(name, n)| {
            *n > 0 && passes.get(name).copied().unwrap_or(0) >= *n && !failed.contains(name)
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// The summed check (or the verify's summed run) of `sp`'s transition `k`:
/// the measured sum in `text` against the drivers' prediction through
/// `chain` (the chains now, or the applied ones). Ok: (low, high, RMS,
/// passed, corner).
fn sum_verdict(
    input: &Input,
    sp: &maths::SpeakerOut,
    k: usize,
    text: &str,
    chain: &dyn Fn(&WayIn) -> ChannelChain,
) -> std::result::Result<(u8, u8, f64, bool, f64), String> {
    let drivers = sp.drivers.as_ref().ok_or("no timing for this speaker")?;
    let tr = sp.transitions.get(k).ok_or("no such transition")?;
    let (Some(lo), Some(hi)) = (
        sp.ways.get(tr.low).and_then(|i| input.ways.get(*i)),
        sp.ways.get(tr.high).and_then(|i| input.ways.get(*i)),
    ) else {
        return Err("no such transition".into());
    };
    let (Some(dl), Some(dh)) = (drivers.get(tr.low), drivers.get(tr.high)) else {
        return Err("no such transition".into());
    };
    let doc = MeasurementDoc::from_text(text).map_err(|e| e.to_string())?;
    let measured = doc
        .measurement
        .outputs
        .first()
        .ok_or("the summed run has no response")?
        .response
        .to_response();
    let (cl, ch) = (chain(lo), chain(hi));
    let mask = invert_mask(&[(lo.output, cl.feed.inverted), (hi.output, ch.feed.inverted)]);
    let predicted = predict_sum(
        &[dl.clone(), dh.clone()],
        &[cl.clone(), ch.clone()],
        SumMode::Generator { invert_mask: mask },
        &CoreSectionModel,
    )
    .map_err(|e| e.to_string())?;
    let corner = match (maths::lowpass_corner(&cl), highpass_corner(&ch)) {
        (Some(a), Some(b)) => (f64::from(a) * f64::from(b)).sqrt(),
        (Some(a), None) | (None, Some(a)) => f64::from(a),
        _ => return Err("no crossover corner".into()),
    };
    let r = summed_check(&measured, &predicted, corner, 0.0).map_err(|e| e.to_string())?;
    Ok((lo.output, hi.output, r.rms_db, r.passed, corner))
}

fn highpass_corner(c: &ChannelChain) -> Option<f32> {
    c.crossover
        .iter()
        .find(|b| !b.bypass && dspi_proto::xover::meta(b.filter_type).is_some_and(|m| m.high_pass))
        .map(|b| b.freq)
}

/// The verify run's ways summed with their phase and their feeds, against
/// the drivers' prediction with the applied chains: RMS 200 Hz to 10 kHz.
fn verified_sum(
    input: &Input,
    sp: &maths::SpeakerOut,
    drivers: &[Driver],
    doc: &MeasurementDoc,
    chain: &dyn Fn(&WayIn) -> ChannelChain,
) -> Option<f64> {
    let model = CoreSectionModel;
    let mut measured = Vec::new();
    let mut flat = Vec::new();
    let mut after = Vec::new();
    for i in &sp.ways {
        let w = input.ways.get(*i)?;
        let om = doc
            .measurement
            .outputs
            .iter()
            .find(|o| o.output == w.output)?;
        let c = chain(w);
        measured.push(Driver {
            output: w.output,
            role: w.role.way(),
            chain: c.clone(),
            response: om.response.to_response(),
            arrival_ms: 0.0,
        });
        flat.push(ChannelChain {
            peq: Vec::new(),
            crossover: Vec::new(),
            gain_db: 0.0,
            delay_ms: 0.0,
            feed: c.feed,
        });
        after.push(c);
    }
    let got = predict_sum(&measured, &flat, SumMode::Programme, &model).ok()?;
    let want = predict_sum(drivers, &after, SumMode::Programme, &model).ok()?;
    rms_between(&got, &want, VERIFY_SUM_HZ.0, VERIFY_SUM_HZ.1, &|_| true)
}

/// The layout the device's state suggests (PLAN-P5 section 1.4 step 1).
pub(crate) fn layout_of(s: &crate::records::DspState) -> MeasureLayout {
    let n_in = s.num_input_channels;
    let labels = crate::output_labels::source(s); // -- WP ROLE0 --
    let mut outputs = Vec::new();
    let mut notes = Vec::new();
    for o in 0..s.num_output_channels {
        let oi = usize::from(o);
        if !s.output_enabled.get(oi).copied().unwrap_or(false) {
            continue;
        }
        let c = chain_of(s, o);
        // The programme source: the routed input with the highest gain.
        let mut feed: Option<(u8, f32)> = None;
        for (i, r) in s.matrix_routing.iter().enumerate() {
            if !r.get(oi).copied().unwrap_or(false) {
                continue;
            }
            let g = s
                .matrix_gain
                .get(i)
                .and_then(|g| g.get(oi))
                .copied()
                .unwrap_or(0.0);
            if let Ok(input) = u8::try_from(i)
                && feed.is_none_or(|(_, best)| g > best)
            {
                feed = Some((input, g));
            }
        }
        let Some((input, _)) = feed else {
            notes.push(format!(
                "output {} plays no input: left out",
                u16::from(o).saturating_add(1)
            ));
            continue;
        };
        let name = s
            .channel_names
            .get(usize::from(n_in.saturating_add(o)))
            .cloned()
            .unwrap_or_default();
        // -- WP ROLE0 -- the labels' inference, thresholds and all
        let filters = labels.filters.get(oi).map(Vec::as_slice).unwrap_or(&[]);
        let role = MeasureRole::from_driver(output_label::driver_role(filters, &name));
        let speaker = if role == MeasureRole::Sub {
            "Sub".to_owned()
        } else {
            match input {
                0 => "L".to_owned(),
                1 => "R".to_owned(),
                i => format!("In {}", u16::from(i).saturating_add(1)),
            }
        };
        outputs.push(MeasureLayoutOutput {
            output: o,
            role,
            speaker,
            group: 0,
            name,
            feed_input: Some(input),
            feed_gain_db: c.feed.gain_db,
            feed_inverted: c.feed.inverted,
        });
    }
    // One alignment group per speaker, in order.
    let mut speakers: Vec<String> = Vec::new();
    for o in &mut outputs {
        let g = match speakers.iter().position(|s| *s == o.speaker) {
            Some(g) => g,
            None => {
                speakers.push(o.speaker.clone());
                speakers.len().saturating_sub(1)
            }
        };
        o.group = u8::try_from(g).unwrap_or(u8::MAX);
    }
    let per: Vec<Vec<MeasureRole>> = speakers
        .iter()
        .map(|s| {
            let mut r: Vec<MeasureRole> = outputs
                .iter()
                .filter(|o| o.speaker == *s)
                .map(|o| o.role)
                .collect();
            r.sort_by_key(|r| *r as u8);
            r
        })
        .collect();
    let mains: Vec<&Vec<MeasureRole>> = per
        .iter()
        .filter(|r| !r.iter().all(|x| *x == MeasureRole::Sub))
        .collect();
    let all = |want: &[MeasureRole]| {
        !mains.is_empty()
            && mains.iter().all(|r| {
                let mut w = want.to_vec();
                w.sort_by_key(|r| *r as u8);
                **r == w
            })
    };
    let template = if all(&[MeasureRole::Low, MeasureRole::Mid, MeasureRole::High]) {
        "3way"
    } else if all(&[MeasureRole::Low, MeasureRole::High]) {
        "2way"
    } else if all(&[MeasureRole::Full]) {
        "full range"
    } else {
        ""
    };
    MeasureLayout {
        outputs,
        template: template.to_owned(),
        notes,
    }
}

// ── The calls ───────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The layout the device's state suggests (PLAN-P5 section 1.4 step 1):
    /// each enabled output that plays an input, its way from its filters
    /// (only a low-pass: low, or sub at or below 120 Hz or by name; a
    /// high-pass and a low-pass: mid; only a high-pass: high; none: full
    /// range), its speaker from its programme source (input 1 `L`, input 2
    /// `R`), one alignment group per speaker, and its programme feed. Reads
    /// the mirror only.
    ///
    /// Errors: [`DspiError::Disconnected`].
    pub fn measure_layout_proposal(&self) -> Result<MeasureLayout> {
        self.lock().conn()?;
        Ok(layout_of(&self.get_state()))
    }

    /// Propose a tune of `layout` from `measurement` (a filed measurement's
    /// id, the hub's first, or a `measurement` document's text) with
    /// `options` (PLAN-P5 section 1.4 steps 3 to 8). Nothing is written.
    /// Replaces any proposal not applied yet.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a measurement of another
    /// device (`measure.tune.other_device`), one without an output of the
    /// layout, a bad layout, options or target, or while a tune is applied
    /// (Keep or Revert it first); [`DspiError::Disconnected`]; as
    /// `measure_doc_get` for an id.
    pub fn measure_tune_propose(
        &self,
        measurement: String,
        layout: MeasureLayout,
        options: MeasureTuneOptions,
    ) -> Result<MeasureTuneProposal> {
        let serial = self.lock().conn()?.serial.clone();
        let mut t = self.measure.tune_lock();
        if matches!(
            t.stage,
            MeasureTuneStage::Applied
                | MeasureTuneStage::Verifying
                | MeasureTuneStage::Verified
                | MeasureTuneStage::SumCheck
        ) {
            return Err(invalid(
                "a tune is applied or checking: Keep, Revert or stop it first",
            ));
        }
        if !(options.window_octaves.is_finite()
            && (0.0..=dspi_measure::xover::MAX_WINDOW_OCTAVES).contains(&options.window_octaves)
            && options.fit_min_hz.is_finite()
            && options.fit_max_hz.is_finite()
            && options.fit_min_hz >= 0.0
            && options.fit_max_hz >= 0.0)
        {
            return Err(invalid("tune options out of range"));
        }
        let layout = self.layout_roles(layout, options.roles_from_layout)?; // -- WP ROLE0 --
        let (doc, source) = self.tune_load(&measurement)?;
        if doc.measurement.serial != serial {
            return Err(invalid(format!(
                "{KEY_OTHER_DEVICE}: the measurement is of DSPi {}, not of this one ({serial})",
                doc.measurement.serial
            )));
        }
        let headphones = matches!(doc.measurement.placement, Placement::Headphones { .. });
        let mut warnings = Vec::new();
        let age = crate::events::now_ms().saturating_sub(doc.measurement.saved_ms);
        if age > OLD_MS {
            warnings.push(MeasureIssue::new(
                KEY_OLD,
                None,
                format!(
                    "the measurement is {} days old: measure again if the room or the speakers \
                     changed",
                    age / (24 * 3600 * 1000)
                ),
            ));
        }
        if matches!(
            doc.measurement.placement,
            Placement::NearField | Placement::MultiPosition { .. }
        ) {
            warnings.push(MeasureIssue::new(
                KEY_PLACEMENT,
                None,
                match doc.measurement.placement {
                    Placement::NearField => {
                        "a near-field measurement: the tune expects the listening position"
                    }
                    _ => "several positions: delays and polarity come from position 1 only",
                },
            ));
        }
        let ways = self.tune_ways(&doc, &layout)?;
        for w in &ways {
            if w.chain_now != w.measured_chain {
                warnings.push(MeasureIssue::new(
                    KEY_CHAINS,
                    Some(w.output),
                    format!(
                        "{}'s settings changed since it was measured: predicted against the \
                         settings now",
                        maths::who(w)
                    ),
                ));
            }
            if let Some(m) = doc
                .measurement
                .outputs
                .iter()
                .find(|o| o.output == w.output)
                && role_of(m.role) != w.role
            {
                warnings.push(MeasureIssue::new(
                    KEY_PLACEMENT,
                    Some(w.output),
                    format!("{} was measured as another way", maths::who(w)),
                ));
            }
        }
        let target_use = if headphones {
            TargetUse::Headphones
        } else {
            TargetUse::Speakers
        };
        let coupler = match doc.measurement.placement {
            Placement::Headphones { coupler, .. } => Some(coupler),
            _ => None,
        };
        let target = if options.target_document.trim().is_empty() {
            Target {
                target_use,
                coupler,
                definition: TargetDefinition::Builtin {
                    builtin: default_target(target_use, coupler),
                },
                reference: DEFAULT_REFERENCE_BAND,
            }
        } else {
            TargetDoc::from_text(&options.target_document)
                .and_then(|d| d.target.to_target())
                .map_err(|e| invalid(format!("the target document: {e}")))?
        }
        .evaluate(&LogGrid::STANDARD)
        .map_err(|e| invalid(format!("the target: {e}")))?;
        let mic_document = match self.tune_mic(&doc, &options.mic_document) {
            Ok(m) => m,
            Err(why) => {
                warnings.push(MeasureIssue::new(KEY_MIC, None, why));
                String::new()
            }
        };
        let bounds = self.tune_bounds(&ways, headphones);
        let input = Input {
            doc,
            source,
            layout,
            options,
            ways,
            bounds,
            target,
            mic_document,
            warnings,
            serial,
        };
        let next = t.next_id;
        *t = Tune {
            next_id: next,
            stage: MeasureTuneStage::Proposed,
            ..Tune::default()
        };
        let p = self.tune_make(&mut t, &input, MeasureTuneCheck::Pending);
        t.input = Some(input);
        self.tune_publish(&t);
        Ok(p)
    }

    /// The current proposal (a new one after a summed check), if any. No
    /// I/O.
    pub fn measure_tune_proposal(&self) -> Option<MeasureTuneProposal> {
        self.measure.tune_lock().proposal.clone()
    }

    /// Run the mandatory summed check of proposal `proposal_id` (RC5f): one
    /// summed run per transition with timing writes, with the chains and
    /// polarities **now**, each a measurement session under every S-rule
    /// (the preflight, the ramp, the aborts). Drive it with
    /// [`DspiCore::measure_tune_tick`]; the proposal is then made again
    /// ([`DspiCore::measure_tune_proposal`]): a speaker whose summed run
    /// differs from the prediction by more than 1.5 dB RMS gets its timing
    /// writes withdrawn, with the reason.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another proposal's id, a
    /// tune not waiting with a proposal, a measurement microphone that is not
    /// at hand, or `measure_start`'s refusals ([`DspiError::ReadOnly`],
    /// [`DspiError::Denied`], [`DspiError::Locked`]).
    pub fn measure_tune_sum_check(&self, proposal_id: u32) -> Result<MeasureTuneState> {
        let actor = crate::actor::JobActor::capture(self); // -- WP ROLL1 --
        let mut t = self.measure.tune_lock();
        t.actor = actor;
        if t.stage != MeasureTuneStage::Proposed
            || t.proposal.as_ref().map(|p| p.id) != Some(proposal_id)
        {
            return Err(invalid(format!(
                "no proposal {proposal_id} waits to be checked"
            )));
        }
        let (Some(input), Some(c)) = (t.input.clone(), t.computed.clone()) else {
            return Err(invalid("no proposal waits to be checked"));
        };
        if input.doc.measurement.mic.is_some() && input.mic_document.is_empty() {
            return Err(invalid(format!(
                "{KEY_MIC}: the measurement's microphone document is needed for the summed check"
            )));
        }
        let mut queue = Vec::new();
        for (si, sp) in c.speakers.iter().enumerate() {
            if !sp.timing {
                continue;
            }
            for (k, tr) in sp.transitions.iter().enumerate() {
                let (Some(lo), Some(hi)) = (
                    sp.ways.get(tr.low).and_then(|i| input.ways.get(*i)),
                    sp.ways.get(tr.high).and_then(|i| input.ways.get(*i)),
                ) else {
                    continue;
                };
                queue.push(RunItem {
                    purpose: Purpose::SumCheck {
                        speaker: si,
                        transition: k,
                    },
                    setup: Self::tune_summed_setup(
                        &input,
                        &[
                            (lo, lo.chain_now.feed.inverted),
                            (hi, hi.chain_now.feed.inverted),
                        ],
                        "Auto-tune summed check",
                    ),
                });
            }
        }
        if queue.is_empty() {
            if let Some(p) = t.proposal.as_mut() {
                p.sum_check = MeasureTuneCheck::NotNeeded;
            }
            return Ok(self.tune_publish(&t));
        }
        let total = u16::try_from(queue.len()).unwrap_or(u16::MAX);
        t.runs = Some(Runs {
            queue,
            total,
            ..Runs::default()
        });
        t.stage = MeasureTuneStage::SumCheck;
        t.issue = None;
        self.tune_next(&mut t);
        if t.runs.is_none() {
            let why = t
                .issue
                .as_ref()
                .map(|i| i.message.clone())
                .unwrap_or_default();
            self.tune_publish(&t);
            return Err(invalid(why));
        }
        Ok(self.tune_publish(&t))
    }

    /// Drive the tune: its summed or verify runs (`measure_tick`, then the
    /// next run or the verdict), and the guard's timer while applied. Call it
    /// about every 50 ms off the UI thread while `running` (and while a tune
    /// is applied), instead of `measure_tick`; push capture whenever the
    /// session wants it. Never fails.
    pub fn measure_tune_tick(&self) -> MeasureTuneState {
        let mut t = self.measure.tune_lock();
        if matches!(
            t.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verifying | MeasureTuneStage::Verified
        ) {
            // -- WP ROLL1 -- a revert on timeout is the applier's in the History
            let _scope = t.actor.attribute(self);
            self.apply_poll();
            self.tune_guard_ended(&mut t);
        }
        if t.runs.as_ref().is_some_and(|r| r.current.is_some()) {
            let st = self.measure_tick();
            match st.phase {
                MeasurePhase::Done => {
                    let text = self.measure_result().ok();
                    let _ = self.measure_clear();
                    if let Some(r) = t.runs.as_mut()
                        && let Some(item) = r.current.take()
                    {
                        match text {
                            Some(x) => r.done.push((item, x)),
                            None => r.queue.clear(),
                        }
                    }
                    self.tune_next(&mut t);
                }
                MeasurePhase::Aborted | MeasurePhase::Failed | MeasurePhase::Idle => {
                    let _ = self.measure_clear();
                    t.runs = None;
                    t.issue = st.issue.clone().or_else(|| {
                        Some(MeasureIssue::new(
                            "measure.stopped",
                            None,
                            "the run ended before its result",
                        ))
                    });
                    t.stage = match t.stage {
                        MeasureTuneStage::Verifying => MeasureTuneStage::Applied,
                        MeasureTuneStage::SumCheck => MeasureTuneStage::Proposed,
                        s => s,
                    };
                }
                _ => {}
            }
        }
        self.tune_publish(&t)
    }

    /// The tune's state, without I/O (safe from the measurement listener).
    pub fn measure_tune_state(&self) -> MeasureTuneState {
        self.measure
            .tune_view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Apply proposal `proposal_id` (PLAN-P5 section 1.4 step 9, S10): the
    /// preflight **again** with the chains it was computed for, the "before"
    /// snapshot filed, the guard begun (`guard_s` seconds; 0: the planned
    /// verify plus a margin, at most 590 s), then every write through
    /// `write_param` in one undo group "Auto-tune". A refused write ends the
    /// group and the guard reverts. Returns the guard.
    ///
    /// Errors, with nothing written: [`DspiError::InvalidArgument`] for
    /// another proposal's id, a proposal with timing writes whose summed check
    /// has not passed (`measure.tune.sum_check_needed`), a measurement or a
    /// live view running, or a blocker of the preflight (programme audio, a
    /// chain changed since, ...); [`DspiError::ReadOnly`] (read-only mode or
    /// connection); [`DspiError::Denied`] (a Viewer through a hub); the
    /// guard's refusals. After the guard began: the refused write's error
    /// (the guard has reverted).
    pub fn measure_tune_apply(&self, proposal_id: u32, guard_s: u32) -> Result<GuardedApply> {
        let actor = crate::actor::JobActor::capture(self); // -- WP ROLL1 --
        let mut t = self.measure.tune_lock();
        t.actor = actor;
        let p = t
            .proposal
            .clone()
            .filter(|p| p.id == proposal_id && t.stage == MeasureTuneStage::Proposed)
            .ok_or_else(|| invalid(format!("no proposal {proposal_id} waits to be applied")))?;
        let input = t
            .input
            .clone()
            .ok_or_else(|| invalid("no proposal waits to be applied"))?;
        let unconfirmed = p.writes.iter().any(|w| {
            w.timing
                && input
                    .ways
                    .iter()
                    .find(|x| x.output == w.output)
                    .is_none_or(|x| !t.confirmed.contains(&x.speaker))
        });
        if unconfirmed {
            return Err(invalid(format!(
                "{KEY_SUM_NEEDED}: run the summed check first: delay, polarity and crossover \
                 changes are applied only after it passed"
            )));
        }
        if self.measure.live_on() || self.measure_state().active {
            return Err(invalid("busy: a measurement or the live view is running"));
        }
        let serial = self.lock().conn()?.serial.clone();
        if serial != input.serial {
            return Err(invalid(format!(
                "{KEY_OTHER_DEVICE}: the proposal is for DSPi {}",
                input.serial
            )));
        }
        // S10: the preflight again, with the chains it was computed for.
        self.settle_notifications();
        let (placement, coupler, count) = placement_setup(&input.doc.measurement.placement);
        let setup = MeasureSetup {
            outputs: input
                .ways
                .iter()
                .map(|w| MeasureOutput {
                    output: w.output,
                    role: w.role,
                    speaker: w.speaker.clone(),
                    ..MeasureOutput::default()
                })
                .collect(),
            placement,
            coupler,
            count,
            ..MeasureSetup::default()
        };
        let mut plan = Plan::new(&setup)?;
        self.measure_resolve_roles(&mut plan)?; // -- WP RCFIX --
        let pre = self.measure_preflight_plan(
            &plan,
            &PreflightOptions {
                allow_guard: None,
                expect_chains: Some(
                    input
                        .ways
                        .iter()
                        .map(|w| (w.output, w.chain_now.clone()))
                        .collect(),
                ),
            },
        )?;
        if let Some(e) = preflight::refusal(&pre) {
            return Err(e);
        }
        // -- review P5-C2 -- a polarity write carries every feeding
        // crosspoint as it was read for the proposal (its gain too), and the
        // preflight's chain holds only the loudest one: all of them must
        // still be what the proposal was computed for, or a stale crosspoint
        // gain would be written back.
        let ways_now = self.tune_ways(&input.doc, &input.layout)?;
        if ways_now.len() != input.ways.len()
            || ways_now
                .iter()
                .zip(&input.ways)
                .any(|(a, b)| a.output != b.output || a.feeds != b.feeds)
        {
            return Err(invalid(format!(
                "{KEY_CHAINS}: a crosspoint that feeds a tuned output changed since the \
                 proposal: propose again"
            )));
        }
        self.lock().writable_conn()?;
        // The "before" snapshot, then the guard, then one undo group.
        let mut notes = Vec::new();
        let before_text = self.snapshot_capture_document(
            format!("Before auto-tune {}", date_words()),
            format!(
                "Before applying the auto-tune of \"{}\"",
                input.doc.meta.name
            ),
        )?;
        let before_id = self.tune_file_snapshot(&before_text, &mut notes);
        let guard_len = if guard_s == 0 {
            match t.computed.as_ref() {
                Some(c) => Self::tune_guard_s(&input, c),
                None => TUNE_GUARD_DEFAULT_S,
            }
        } else {
            guard_s
        };
        let guard = self.apply_guard_begin(TUNE_LABEL.to_owned(), guard_len)?;
        self.begin_group(TUNE_LABEL.to_owned());
        let mut failed = None;
        'all: for w in &p.writes {
            for q in &w.params {
                if let Err(e) = self.write_param(q.path.clone(), q.indices.clone(), q.value.clone())
                {
                    failed = Some(match e {
                        DspiError::ReadOnly | DspiError::Denied => e,
                        e => DspiError::InvalidArgument {
                            msg: format!("{}: {e}", w.words),
                        },
                    });
                    break 'all;
                }
            }
        }
        self.end_group();
        t.guard_id = guard.id;
        t.before = Some((before_id, before_text));
        if let Some(e) = failed {
            // -- WP REVERTW -- a revert that did not put everything back says so
            let g = self.apply_revert().unwrap_or_else(|_| self.apply_status());
            let (stage, issue) = tune_revert_end(&g);
            t.stage = stage;
            t.issue =
                Some(issue.unwrap_or_else(|| MeasureIssue::new(KEY_NOT_KEPT, None, e.to_string())));
            self.tune_publish(&t);
            return Err(e);
        }
        let s = self.get_state();
        t.applied = input.ways.iter().map(|w| chain_of(&s, w.output)).collect();
        t.stage = MeasureTuneStage::Applied;
        t.issue = None;
        t.verify = None;
        t.keep_armed = false;
        t.verify_failed = false;
        t.keep_refused_at = None;
        if !notes.is_empty() {
            t.issue = Some(MeasureIssue::new(KEY_NOT_KEPT, None, notes.join("; ")));
        }
        self.tune_publish(&t);
        Ok(self.apply_status())
    }

    /// Verify the applied tune inside its guard (PLAN-P5 section 1.4 step
    /// 10): a walked run of the tuned outputs and a summed run per transition
    /// with the new polarities, each measured against the prediction. Drive
    /// it with [`DspiCore::measure_tune_tick`]; the result is the state's
    /// `verify`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] when no tune is applied, the
    /// measurement's microphone is not at hand, or `measure_start` refuses.
    pub fn measure_tune_verify(&self) -> Result<MeasureTuneState> {
        let actor = crate::actor::JobActor::capture(self); // -- WP ROLL1 --
        let mut t = self.measure.tune_lock();
        t.actor = actor;
        self.tune_guard_ended(&mut t);
        if !matches!(
            t.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verified
        ) {
            return Err(invalid("no applied tune waits to be verified"));
        }
        let (Some(input), Some(c)) = (t.input.clone(), t.computed.clone()) else {
            return Err(invalid("no applied tune waits to be verified"));
        };
        if input.doc.measurement.mic.is_some() && input.mic_document.is_empty() {
            return Err(invalid(format!(
                "{KEY_MIC}: the measurement's microphone document is needed to verify"
            )));
        }
        let queue = Self::tune_verify_queue(&input, &c, &t.applied);
        let total = u16::try_from(queue.len()).unwrap_or(u16::MAX);
        t.runs = Some(Runs {
            queue,
            total,
            ..Runs::default()
        });
        t.stage = MeasureTuneStage::Verifying;
        t.issue = None;
        t.verify = None;
        t.keep_armed = false;
        t.keep_refused_at = None;
        self.tune_next(&mut t);
        if t.runs.is_none() {
            let why = t
                .issue
                .as_ref()
                .map(|i| i.message.clone())
                .unwrap_or_default();
            self.tune_publish(&t);
            return Err(invalid(why));
        }
        Ok(self.tune_publish(&t))
    }

    /// Keep the applied tune (PLAN-P5 section 1.4 step 11): `apply_confirm`,
    /// the "after" snapshot filed with its `tune` block (the measurement, the
    /// algorithm, the options, the writes), and the tune's row added to the
    /// measurement.
    ///
    /// **While a verify runs** Keep is refused with
    /// `measure.tune.keep_during_verify` (WP P5HOLD, review P5-D D3; nothing
    /// changes, the verify and the guard run on): wait for the verdict.
    /// Before any verify has started one Keep keeps (the proposal is "Not
    /// verified").
    ///
    /// **After a verify that did not pass** the first Keep is refused with
    /// `measure.tune.keep_unverified` (nothing changes, the guard runs on;
    /// the state's `issue` says it): the client shows why and asks the user
    /// again, and only a second Keep, on the user's explicit confirmation,
    /// keeps it. A new verify, Revert or another apply ask again. Another
    /// actor than the embedder ([`crate::actor::ActorScope`]) is refused every
    /// time (WP RCFIX).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when no tune is applied, while
    /// a verify runs, or the first Keep after a failed verify;
    /// `apply_confirm`'s. Filing failures are notes, not errors.
    pub fn measure_tune_keep(&self) -> Result<MeasureTuneKeep> {
        let mut t = self.measure.tune_lock();
        self.tune_guard_ended(&mut t);
        if !matches!(
            t.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verifying | MeasureTuneStage::Verified
        ) {
            return Err(invalid("no applied tune waits for Keep"));
        }
        // -- WP P5TRUST -- coordinator decision (b): a failed verify needs a
        // second, explicit Keep.
        self.tune_keep_gate(&mut t)?;
        if t.runs.take().is_some() && self.measure_state().active {
            self.measure_stop_session();
        }
        let _ = self.measure_clear();
        let guard = self.apply_confirm_ungated()?;
        let (before_id, before_text) = t.before.clone().unwrap_or_default();
        let mut keep = MeasureTuneKeep {
            guard: guard.clone(),
            snapshot_before_id: before_id.clone(),
            snapshot_before: before_text,
            ..MeasureTuneKeep::default()
        };
        if guard.phase != GuardPhase::Kept {
            // -- WP REVERTW --
            let (stage, issue) = tune_end_of(&guard);
            t.stage = stage;
            if issue.is_some() {
                t.issue = issue;
            }
            self.tune_publish(&t);
            return Ok(keep);
        }
        t.stage = MeasureTuneStage::Kept;
        let Some(input) = t.input.clone() else {
            self.tune_publish(&t);
            return Ok(keep);
        };
        let at = crate::events::now_ms();
        let mut notes = Vec::new();
        let after = self
            .snapshot_capture_document(
                format!("After auto-tune {}", date_words()),
                format!("The auto-tune of \"{}\", kept", input.doc.meta.name),
            )
            .ok()
            .and_then(|text| {
                let mut v: Value = serde_json::from_str(&text).ok()?;
                let block = serde_json::json!({
                    "measurement": t.proposal.as_ref().map(|p| p.measurement_id.clone()).unwrap_or_default(),
                    "measurementName": input.doc.meta.name,
                    "algorithm": ALGORITHM,
                    "options": serde_json::to_value(&input.options).unwrap_or(Value::Null),
                    "proposal": t.proposal.as_ref().map(|p| p.id).unwrap_or(0),
                    "writes": t.proposal.as_ref().map(|p| p.writes.iter().map(|w| w.words.clone()).collect::<Vec<_>>()).unwrap_or_default(),
                    "snapshotBefore": before_id,
                    "at": at,
                });
                v.as_object_mut()?.insert("tune".into(), block);
                serde_json::to_string_pretty(&v).ok()
            });
        let Some(after_text) = after else {
            notes.push("the \"after\" snapshot could not be taken".into());
            keep.notes = notes;
            self.tune_publish(&t);
            return Ok(keep);
        };
        let after_id = self.tune_file_snapshot(&after_text, &mut notes);
        t.after = Some((after_id.clone(), after_text.clone()));
        keep.snapshot_after_id = after_id.clone();
        keep.snapshot_after = after_text;
        let store_id = |s: &str| {
            if dspi_session::measure_store::valid_id(s) {
                s.to_owned()
            } else {
                String::new()
            }
        };
        let record = TuneRecord {
            at,
            snapshot_before: store_id(&before_id),
            snapshot_after: store_id(&after_id),
            kept: true,
            extra: Default::default(),
        };
        let mut doc = input.doc.clone();
        while doc.measurement.tunes.len() >= dspi_measure::doc::MAX_TUNES {
            doc.measurement.tunes.remove(0);
        }
        doc.measurement.tunes.push(record.clone());
        match &input.source {
            Source::Local(id) => match self.measure_doc_add_tune(id, record) {
                Ok(i) => {
                    keep.measurement_id = i.id;
                    keep.measurement = self.measure_doc_get(id.clone()).unwrap_or_default();
                }
                Err(e) => notes.push(format!("the measurement's tune row is not filed: {e}")),
            },
            Source::Hub(_) => {
                // The hub never overwrites: a new copy with the row.
                doc.measurement.id = String::new();
                match doc.to_text() {
                    Ok(text) => {
                        match self.hub_measure_doc_put(text.clone()) {
                            Ok(p) => keep.measurement_id = p.doc.id,
                            Err(e) => notes.push(format!(
                                "the hub did not file the measurement with its tune row: {e}"
                            )),
                        }
                        keep.measurement = text;
                    }
                    Err(e) => notes.push(format!("the measurement: {e}")),
                }
            }
            Source::Text => match doc.to_text() {
                Ok(text) => keep.measurement = text,
                Err(e) => notes.push(format!("the measurement: {e}")),
            },
        }
        keep.notes = notes;
        self.tune_publish(&t);
        Ok(keep)
    }

    /// Revert the applied tune: the guard puts the "before" state back (a
    /// verify still running is stopped first). Returns the guard.
    ///
    /// Errors: [`DspiError::InvalidArgument`] when no tune is applied;
    /// `apply_revert`'s.
    pub fn measure_tune_revert(&self) -> Result<GuardedApply> {
        let mut t = self.measure.tune_lock();
        self.tune_guard_ended(&mut t);
        if !matches!(
            t.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verifying | MeasureTuneStage::Verified
        ) {
            return Err(invalid("no applied tune waits for Revert"));
        }
        if t.runs.take().is_some() && self.measure_state().active {
            self.measure_stop_session();
        }
        let _ = self.measure_clear();
        let g = self.apply_revert()?;
        // -- WP REVERTW --
        let (stage, issue) = tune_revert_end(&g);
        t.stage = stage;
        if issue.is_some() {
            t.issue = issue;
        }
        self.tune_publish(&t);
        Ok(g)
    }

    /// Start the A/B compare of a kept tune: the device now ("A", the tune)
    /// against the "before" snapshot ("B"). Toggle with `ab_toggle`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] when no tune was kept;
    /// `ab_start_documents`'s.
    pub fn measure_tune_ab(&self) -> Result<AbCompare> {
        let t = self.measure.tune_lock();
        if t.stage != MeasureTuneStage::Kept {
            return Err(invalid("no kept tune to compare"));
        }
        let before = t
            .before
            .as_ref()
            .map(|b| b.1.clone())
            .ok_or_else(|| invalid("the \"before\" snapshot is not at hand"))?;
        drop(t);
        self.ab_start_documents(String::new(), before)
    }

    /// Stop the tune's runs and forget it, unless it is applied (then only
    /// its runs stop: Keep or Revert it). Never fails.
    pub fn measure_tune_stop(&self) -> MeasureTuneState {
        let mut t = self.measure.tune_lock();
        if t.runs.take().is_some() && self.measure_state().active {
            crate::actor::user::did(self, "measure_stop"); // -- WP AIVIS --
            self.measure_stop_session();
        }
        let _ = self.measure_clear();
        match t.stage {
            MeasureTuneStage::Applied
            | MeasureTuneStage::Verifying
            | MeasureTuneStage::Verified => {
                t.stage = MeasureTuneStage::Applied;
                // Review P5-E: no verify runs any more, so "the verify is
                // still running" is no longer true (it was cleared only by a
                // verdict, and every client went on saying it after a Stop).
                if t.issue
                    .as_ref()
                    .is_some_and(|i| i.key == KEY_KEEP_DURING_VERIFY)
                {
                    t.issue = None;
                }
            }
            _ => t.forget(),
        }
        self.tune_publish(&t)
    }
}

// -- WP REVERTW --
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_revertw {
    use super::*;

    fn ended(phase: GuardPhase, notice: &str) -> GuardedApply {
        GuardedApply {
            phase,
            message: "the hub could not put it back".into(),
            notes: vec!["Output 2: not written".into()],
            revert_notice: notice.into(),
            ..GuardedApply::default()
        }
    }

    /// A tune's guard that ended without putting everything back is
    /// `RevertIncomplete` with the guard's `revert.*` notice and its words;
    /// a clean revert and a Keep are not.
    #[test]
    fn wp_revertw_a_tune_guard_not_wholly_put_back_has_its_own_stage() {
        use crate::guarded::{
            NOTICE_REVERT_FAILED, NOTICE_REVERT_INCOMPLETE, NOTICE_REVERT_NOT_WRITTEN,
            NOTICE_REVERT_UNKNOWN,
        };
        for (phase, key) in [
            (GuardPhase::RevertIncomplete, NOTICE_REVERT_INCOMPLETE),
            (GuardPhase::RevertFailed, NOTICE_REVERT_FAILED),
            (GuardPhase::NotWritten, NOTICE_REVERT_NOT_WRITTEN),
        ] {
            let (stage, issue) = tune_end_of(&ended(phase, key));
            assert_eq!(stage, MeasureTuneStage::RevertIncomplete, "{phase:?}");
            let i = issue.unwrap();
            assert_eq!(i.key, key);
            assert!(i.message.contains("Output 2: not written"), "{}", i.message);
        }
        // Lost, its end not known: the unknown notice.
        let (stage, issue) = tune_end_of(&ended(GuardPhase::Lost, ""));
        assert_eq!(stage, MeasureTuneStage::RevertIncomplete);
        assert_eq!(issue.unwrap().key, NOTICE_REVERT_UNKNOWN);
        assert_eq!(
            tune_end_of(&ended(GuardPhase::Reverted, "")),
            (MeasureTuneStage::Reverted, None)
        );
        assert_eq!(
            tune_end_of(&ended(GuardPhase::Kept, "")),
            (MeasureTuneStage::Kept, None)
        );
        // A Revert another console's takeover left kept put nothing back.
        let (stage, issue) = tune_revert_end(&ended(GuardPhase::Kept, ""));
        assert_eq!(stage, MeasureTuneStage::RevertIncomplete);
        assert_eq!(issue.unwrap().key, NOTICE_REVERT_NOT_WRITTEN);
        assert_eq!(
            serde_json::to_value(MeasureTuneStage::RevertIncomplete).unwrap(),
            serde_json::json!("revertIncomplete")
        );
    }
}
