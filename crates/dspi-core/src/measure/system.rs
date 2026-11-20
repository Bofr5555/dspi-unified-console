//! The system measurement across units (WP ATCORE, `docs/core-merge/PLAN-AT.md`
//! sections 1.4 and 1.5): several DSPi units on one time base, one
//! microphone, the client's core owning the microphone, the plan, the
//! analysis, the per-unit safety judgement and the documents; **our hub**
//! owning the units' side (`system.measure.*`, the seam's
//! [`dspi_measure::system::messages`]): it reads the time bases, stages the
//! configs, schedules every START (op 9) on the shared counter, watches the
//! units and aborts on its own (a dead-man lease).
//!
//! **The calls.** Each unit's own core judges that unit first
//! ([`DspiCore::measure_system_unit_check`]: P5's preflight, S2, S6, S8,
//! S11, S13, on that unit's mirror); the core on the hub session takes those
//! checks ([`DspiCore::measure_system_preflight`],
//! [`DspiCore::measure_system_start`]) and runs the session: a pre-roll, a
//! closed-loop ramp per output of every unit (S3, P5's ramp: each step a
//! one-segment run at the next T through the hub), then the timed run
//! (interleaved passes, `dspi_measure::system::interleave`; runs that do not
//! fit one capture are split and chained through an anchor output), the
//! analysis in slices ([`dspi_measure::system::SystemAnalyser`], always with
//! P5's default options), the gate (G1 to G4, `check_gate`, plus G5), one
//! `measurement` document per unit with a `system` block, the summed check
//! per inter-unit transition (RC5f), the proposal and its apply (Admin:
//! measured arrivals into the hub's model, `system.align`, the revision and
//! every unit's output delays filed first; the summed runs repeated; Keep or
//! Revert).
//!
//! **Safety.** P5's S1 to S13 per unit: nothing but the generator's
//! configuration and control is written by the session (through the hub's
//! `stage` and `run`); a capture clip, the SPL cap, a frame gap, a unit's
//! clip, output peak, limiter, time-base disturbance or loss, a failed proof,
//! a lost lease or the hub's own abort end the run with one
//! `system.measure.abort` (and a `STOP_NOW` on this core's own device when
//! the abort cannot be sent), naming the unit, nothing proposed. Foreign
//! writes, preset loads and volume changes on a unit are the hub's to see
//! (it aborts on them; PLAN-AT 1.5). Only finite log sweeps start on the time
//! base (`stimulus::check_timed_config` before every `stage`).
//!
//! **The microphone across chained runs** (review AT-A/AT-B): it **stays
//! open**. A group split into runs is captured as one stream with one frame
//! count (a gap or a repeat in the frame indices aborts, as P5's S5), so the
//! runs share one capture clock: `ChainOptions::one_capture_clock` is set and
//! the anchor's arrival check (20 us after the drift fit) runs with three
//! runs or more, beside the magnitude check. A client that stops its
//! microphone between runs ends the session (a frame gap): the arrival check
//! is never silently skipped.
//!
//! **Arrivals** (WP ATHUBM's contract): `measuredArrivalUs` is the acoustic
//! part only, relative to the group's latest output. The session takes out
//! what the alignment engine adds itself: the unit's play-out on the time
//! base (the path and the latency of the alignment the hub reports in
//! `system.status`, which are the model's, plus the unit's own play-out
//! delay D, `EXT_TIME_BASE.d` at `open`, which holds the lip-sync term; a
//! source plays T at once, D = 0, its lag is in its output delays) and the
//! output delay in effect during the run (read with `param.get_many` before
//! it). Review AT-C: never the alignment's `effective` during a session;
//! its locks keep the hub from reading the units it is computed from.
//!
//! Privacy (S12): the microphone's samples stay in this module's memory for
//! the run; the documents hold derived curves only.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use dspi_measure::system::messages::AbortedEvent;
use dspi_transport::clock::Instant;

use super::{MeasureIssue, MeasureOutput, MeasurePreflight};
use crate::{DspiCore, DspiError, Result};

pub(crate) mod plan;
pub(crate) mod proposal;
pub(crate) mod session;
// -- WP ATDEMO -- the demo hub's simulated microphone
pub(crate) mod sim;
pub use sim::MeasureSystemSimInfo;

pub(crate) use session::{SessionOutput, SysSession};

// ── Records ─────────────────────────────────────────────────────────────

/// One unit of a system measurement and the outputs measured on it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureSystemUnit {
    /// Its USB serial (as the hub knows it).
    pub serial: String,
    /// Its outputs to measure, 1 to 16, each once (`group` is ignored: the
    /// group is the whole setup).
    pub outputs: Vec<MeasureOutput>,
}

/// What to measure across units and how (PLAN-AT 1.4). Every number 0 (or
/// absent) is the plan's default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureSystemSetup {
    /// The group's units, 2 to 16, in the order the hub opens them.
    pub units: Vec<MeasureSystemUnit>,
    /// Passes, 2 to 8; 0: 5 where the run fits one capture, else 3.
    pub passes: u8,
    /// The anchor output's unit for runs chained through it; "": the first
    /// unit.
    pub anchor_serial: String,
    /// The anchor output; `None`: the anchor unit's first output.
    pub anchor_output: Option<u8>,
    /// The microphone's `mic` document's text; "" for an uncalibrated one.
    pub mic_document: String,
    /// One sweep, ms (1000 to 15000; 0: 5500).
    pub sweep_ms: u32,
    /// The gap after each cycle, ms (300 to 2000; 0: 1000).
    pub gap_ms: u32,
    /// The generator's ceiling, peak dBFS (as P5's).
    pub ceiling_db: Option<f32>,
    /// The SPL cap with a calibrated microphone, dB SPL (as P5's).
    pub spl_cap_db: Option<f32>,
    /// The capture's rate, Hz (0: 48000).
    pub capture_rate_hz: u32,
    /// The lead of every START, ms; 0 or less than the units' arm lead plus
    /// 100 ms: that (S-AT3).
    pub lead_ms: u32,
    /// The hub's lease, ms (0: the hub's default, 2000; clamped 500..5000).
    pub lease_ms: u32,
    /// The documents' name ("" names them by the date).
    pub name: String,
}

/// One unit judged by its own core (P5's preflight on that unit's mirror),
/// for the core on the hub session.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureSystemUnitCheck {
    /// The unit's serial.
    pub serial: String,
    /// P5's preflight of its outputs.
    pub preflight: MeasurePreflight,
    /// What the session needs of the unit (its platform, rate, inputs,
    /// limiters, the measured chains), as JSON. Opaque to clients.
    pub context: String,
}

/// An issue of a system measurement, naming its unit.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemIssue {
    /// The unit's serial; "" when it concerns no one unit.
    pub serial: String,
    /// The notice key (`measure.*`, `measure.system.*`).
    pub key: String,
    /// The output it concerns, if one.
    pub output: Option<u8>,
    /// What happened, in words (the unit named).
    pub message: String,
}

impl MeasureSystemIssue {
    pub(crate) fn new(serial: &str, key: &str, output: Option<u8>, msg: impl Into<String>) -> Self {
        MeasureSystemIssue {
            serial: serial.to_owned(),
            key: key.to_owned(),
            output,
            message: msg.into(),
        }
    }

    fn of(serial: &str, i: &MeasureIssue) -> Self {
        let msg = if serial.is_empty() {
            i.message.clone()
        } else {
            format!("{serial}: {}", i.message)
        };
        MeasureSystemIssue::new(serial, &i.key, i.output, msg)
    }
}

/// What stands between a setup and a system measurement.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemPreflight {
    /// Nothing blocks.
    pub clear: bool,
    /// What must change first, each naming its unit.
    pub blockers: Vec<MeasureSystemIssue>,
    /// What to know.
    pub warnings: Vec<MeasureSystemIssue>,
}

/// Where a system measurement is.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureSystemPhase {
    /// No session.
    #[default]
    Idle,
    /// The noise floor is captured (nothing plays).
    PreRoll,
    /// One output's ramp step plays (S3).
    Ramp,
    /// The timed run plays.
    Run,
    /// The capture is analysed.
    Analysing,
    /// Finished: a result (or a summed check's verdicts) is ready.
    Done,
    /// Stopped: by the user, a safety rule or the hub; nothing kept.
    Aborted,
    /// Could not finish (no signal, the gate, the ramp); nothing kept.
    Failed,
}

/// What a session's runs are for.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureSystemPurpose {
    /// The walked measurement.
    #[default]
    Measure,
    /// The summed check of every inter-unit transition (RC5f).
    SumCheck,
    /// The summed runs repeated after an apply (P5's verify).
    Verify,
}

/// One output's level after its ramp.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemLevel {
    /// The unit.
    pub serial: String,
    /// The output.
    pub output: u8,
    /// Its level, peak dBFS.
    pub level_db: f32,
}

/// A system session's state (no I/O to read it).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemState {
    /// The phase.
    pub phase: MeasureSystemPhase,
    /// What the runs are for.
    pub purpose: MeasureSystemPurpose,
    /// Holds the hub's session or a stage in progress.
    pub active: bool,
    /// Push the microphone's capture now.
    pub wants_capture: bool,
    /// The capture's rate, Hz.
    pub capture_rate_hz: u32,
    /// The unit playing (a ramp step), or "".
    pub serial: String,
    /// The output playing (a ramp step).
    pub output: Option<u8>,
    /// Its level, peak dBFS.
    pub level_db: Option<f32>,
    /// Every level the ramps found so far.
    pub ramp_levels: Vec<MeasureSystemLevel>,
    /// The timed run in progress, 1-based (0 before the first).
    pub run: u16,
    /// Timed runs in all.
    pub runs: u16,
    /// Capture frames pushed.
    pub frames: u64,
    /// Progress of the stage, 0 to 1.
    pub progress: f32,
    /// Why it stopped or what it asks.
    pub issue: Option<MeasureSystemIssue>,
    /// A result is ready (`measure_system_result`).
    pub result_ready: bool,
    /// G5: the timing of every run was reliable.
    pub timing_reliable: Option<bool>,
    /// What the session waits for, in words ("waiting for the units' clocks
    /// to settle"), or "".
    pub words: String,
}

/// A measured output's arrival, as the proposal would write it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemArrival {
    /// The unit.
    pub serial: String,
    /// The output.
    pub output: u8,
    /// Its acoustic arrival relative to the group's latest output, us (0 for
    /// that one, negative for an earlier one): WP ATHUBM's
    /// `measuredArrivalUs`.
    pub arrival_us: f64,
    /// The arrival as measured on the timeline, before the engine's terms
    /// were taken out, relative to the same reference, us.
    pub measured_us: f64,
    /// The uncertainty of the arrival against the latest output, us (one
    /// standard error; `None` without two clear passes).
    pub uncertainty_us: Option<f64>,
    /// May be proposed: the gate passed and the delay to the latest output
    /// is offered (within 10 us).
    pub offered: bool,
    /// Why not (empty when offered).
    pub reason: String,
}

/// A delay between two outputs on different units of one run.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemDelay {
    /// The earlier output's unit.
    pub from_serial: String,
    /// Its output.
    pub from_output: u8,
    /// The other output's unit.
    pub to_serial: String,
    /// Its output.
    pub to_output: u8,
    /// `to`'s arrival minus `from`'s, us.
    pub delay_us: f64,
    /// Its uncertainty, us.
    pub uncertainty_us: Option<f64>,
    /// The analyser offers it.
    pub offered: bool,
    /// Why not ("timing unreliable: ..." with the wander).
    pub reason: String,
}

/// An inter-unit transition and its summed check (RC5f, R8d).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemTransition {
    /// One output's unit.
    pub a_serial: String,
    /// Its output.
    pub a_output: u8,
    /// The other output's unit.
    pub b_serial: String,
    /// Its output.
    pub b_output: u8,
    /// The check band, Hz.
    pub band_lo_hz: f64,
    /// The check band's top, Hz.
    pub band_hi_hz: f64,
    /// The summed run was measured and judged.
    pub checked: bool,
    /// Within 1.5 dB RMS of the prediction.
    pub passed: bool,
    /// The RMS difference, dB.
    pub rms_db: Option<f64>,
    /// What happened, in words.
    pub message: String,
}

/// One unit's `measurement` document.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemDocument {
    /// The unit.
    pub serial: String,
    /// The document's text (schema 1 with a `system` block). File it with
    /// `measure_doc_put` (or the hub's).
    pub document: String,
}

/// A finished system measurement.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemResult {
    /// One document per unit.
    pub documents: Vec<MeasureSystemDocument>,
    /// Every measured output's arrival.
    pub arrivals: Vec<MeasureSystemArrival>,
    /// Every cross-unit delay of every run.
    pub delays: Vec<MeasureSystemDelay>,
    /// The inter-unit transitions.
    pub transitions: Vec<MeasureSystemTransition>,
    /// G5 for every run.
    pub timing_reliable: bool,
    /// Why not reliable, what was not offered, a refused chain.
    pub reasons: Vec<String>,
    /// The generator runs (`EXT_SIGGEN_TIMELINE.run_id`) the arrivals came
    /// from.
    pub run_ids: Vec<u32>,
    /// Timed runs chained through the anchor (1: none).
    pub runs: u16,
    // -- WP ATOVL --
    /// Which plan each run used: "overlapped starts, 50 ms apart" (the
    /// default where it is safe) or "interleaved passes" with why
    /// overlapped starts were not used; "run 1: ...; run 2: ..." for
    /// chained runs.
    #[serde(default)]
    pub plan: String,
}

/// The alignment a system measurement proposes (OD-ATl): measured arrivals
/// into the hub's model, applied by `system.align`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemProposal {
    /// Its id (the apply names it).
    pub id: u32,
    /// The arrivals to write: offered ones only.
    pub arrivals: Vec<MeasureSystemArrival>,
    /// Arrivals not offered, with why.
    pub withheld: Vec<MeasureSystemArrival>,
    /// The transitions and their summed checks.
    pub transitions: Vec<MeasureSystemTransition>,
    /// A summed check is still to run before anything may be applied.
    pub sum_check_needed: bool,
    /// Every cross-unit delay was withdrawn (a summed check failed).
    pub withdrawn: bool,
    /// Something can be applied (after the summed check).
    pub offered: bool,
    /// This session's role may apply it (Admin).
    pub may_apply: bool,
    /// What stands in the way, if anything.
    pub notice: Option<MeasureSystemIssue>,
    // -- WP ATGUARD --
    /// The hub's guard the apply begins, s: the hub reverts the apply by
    /// itself unless it is kept within this long (and when this console's
    /// connection is lost). 0: the hub has no guard for a system apply.
    #[serde(default)]
    pub guard_s: u32,
    /// The guard in words: how long, or that nothing reverts by itself.
    #[serde(default)]
    pub guard_note: String,
}

/// One unit's output delays as filed before an apply.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemSnapshot {
    /// The unit.
    pub serial: String,
    /// `(output, out.delay ms)` read before the apply.
    pub out_delays_ms: Vec<MeasureSystemOutDelay>,
}

/// One output's delay.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemOutDelay {
    /// The output.
    pub output: u8,
    /// Its delay, ms.
    pub delay_ms: f32,
}

/// Where an applied system alignment stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureSystemApplyStage {
    /// Nothing applied.
    #[default]
    None,
    /// Applied; the summed runs are to be repeated (`measure_system_verify`).
    Applied,
    /// Verified (or not): Keep or Revert.
    Verified,
    /// Kept.
    Kept,
    /// Reverted: the model and the units are as filed.
    Reverted,
    // -- WP REVERTW --
    /// The revert (the user's, or the hub's guard's) did not put everything
    /// back: the model was not set back, a unit's write failed, a delay
    /// reads back other than filed, or what the hub did is not known.
    /// `revert_notice` is the `revert.*` notice and `message` names what;
    /// the user must check the settings or restore a backup (a snapshot).
    RevertIncomplete,
}

/// What an apply, a verify, a Keep or a Revert did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSystemApplied {
    /// Where it stands.
    pub stage: MeasureSystemApplyStage,
    /// The hub model's revision filed before the apply.
    pub revision_before: u64,
    /// Its revision after the apply (or the revert).
    pub revision_after: u64,
    /// Every unit's output delays filed before the apply.
    pub snapshots: Vec<MeasureSystemSnapshot>,
    /// `system.align`'s writes that were `ok`.
    pub writes_ok: u32,
    /// Writes that failed, in words.
    pub write_failures: Vec<String>,
    /// The verify's summed runs agreed with the new delays (`None` before).
    pub verified: Option<bool>,
    /// What to know, in words.
    pub message: String,
    // -- WP ATGUARD --
    /// The hub's guard of this apply, s (0: none, nothing reverts by
    /// itself): the hub sets the filed model back and aligns at the
    /// deadline unless Keep confirms it, and when the connection is lost.
    #[serde(default)]
    pub guard_s: u32,
    // -- WP REVERTW --
    /// The notice key when the revert was not fully successful (stage
    /// `RevertIncomplete`): `revert.incomplete`, `revert.failed`,
    /// `revert.not_written` or `revert.unknown`; "" otherwise. `message`
    /// names what was not put back.
    #[serde(default)]
    pub revert_notice: String,
}

// ── The hub, as the session sees it ────────────────────────────────────

/// What the system session asks of the hub (one DSPi Link session). The
/// core's is [`CoreHub`]; tests play a scripted hub.
pub(crate) trait Hub: Send {
    /// The hub's caps; `None` with no hub session.
    fn caps(&self) -> Option<Vec<String>>;
    /// This session's role (`viewer`, `control`, `admin`; "" with none).
    fn role(&self) -> String;
    /// Send `t` with `body` (an object) and wait for the answer.
    fn request(&mut self, t: &str, body: Value) -> Result<Value>;
    /// `system.measure.aborted` events since the last call.
    fn aborted(&mut self) -> Vec<AbortedEvent>;
    /// The client's fallback when `abort` fails (PLAN-AT 1.5): `STOP_NOW`
    /// through any unit connection it has. True when one was sent.
    fn stop_now_fallback(&mut self, serials: &[String]) -> bool;
}

/// [`Hub`] over this core's DSPi Link session.
struct CoreHub<'a> {
    core: &'a DspiCore,
}

impl Hub for CoreHub<'_> {
    fn caps(&self) -> Option<Vec<String>> {
        #[cfg(feature = "link")]
        {
            self.core.link_hub_caps().flatten()
        }
        #[cfg(not(feature = "link"))]
        {
            None
        }
    }

    fn role(&self) -> String {
        #[cfg(feature = "link")]
        {
            self.core.link_hub_role()
        }
        #[cfg(not(feature = "link"))]
        {
            String::new()
        }
    }

    fn request(&mut self, t: &str, body: Value) -> Result<Value> {
        #[cfg(feature = "link")]
        {
            let fields = match body {
                Value::Object(m) => m,
                _ => serde_json::Map::new(),
            };
            self.core
                .link_hub_request_within(t, fields, std::time::Duration::from_secs(15))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (t, body);
            Err(DspiError::Disconnected)
        }
    }

    fn aborted(&mut self) -> Vec<AbortedEvent> {
        std::mem::take(
            &mut *self
                .core
                .measure
                .system_aborted
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        )
    }

    fn stop_now_fallback(&mut self, serials: &[String]) -> bool {
        let own = self.core.lock().conn().ok().map(|c| c.serial.clone());
        match own {
            Some(s) if serials.contains(&s) => self
                .core
                .siggen_control(crate::consts::SIGGEN_CTL_STOP_NOW)
                .is_ok(),
            _ => false,
        }
    }
}

/// The system session's slot on a core: taken before `inner`, never after.
#[derive(Default)]
pub(crate) struct SystemSlot {
    pub(crate) session: Option<SysSession>,
    /// The last measurement's outcome, kept for the proposal and its apply.
    pub(crate) outcome: Option<proposal::Outcome>,
    // -- WP ATDEMO --
    /// The demo hub's simulated microphone.
    pub(crate) sim: sim::SystemSim,
    /// A test's scripted hub in place of the core's.
    #[cfg(test)]
    pub(crate) hub: Option<Box<dyn Hub>>,
}

/// The unit's context as the check carries it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct UnitContext {
    pub(crate) serial: String,
    pub(crate) platform: String,
    pub(crate) device_rate_hz: u32,
    pub(crate) num_inputs: u8,
    pub(crate) limiter_outputs: Vec<u8>,
    pub(crate) chains: Vec<(u8, dspi_measure::types::ChannelChain)>,
}

// ── The calls ───────────────────────────────────────────────────────────

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Judge this core's own device as one unit of a system measurement:
    /// P5's preflight of `unit`'s outputs on this device's mirror (S2, S6,
    /// S8, S11, S13, S7's bounds) and what the system session needs of it.
    /// Call it on each unit's own core and hand the checks to the core on
    /// the hub session. Reads the device; writes nothing.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a unit that is not this
    /// device, or a setup that is not one; [`DspiError::Disconnected`]; a
    /// transfer failure of the reads.
    pub fn measure_system_unit_check(
        &self,
        setup: MeasureSystemSetup,
        unit: MeasureSystemUnit,
    ) -> Result<MeasureSystemUnitCheck> {
        let serial = self.lock().conn()?.serial.clone();
        if serial != unit.serial {
            return Err(invalid(format!(
                "this core's device is {serial}, not {}",
                unit.serial
            )));
        }
        let p = plan::unit_plan(&setup, &unit)?;
        let preflight = self.measure_preflight_plan(&p, &super::PreflightOptions::default())?;
        let rate = self
            .fetch_stat(crate::consts::STAT_SAMPLE_RATE)
            .unwrap_or(0);
        let ctx = {
            let inner = self.lock();
            let s = &inner.state;
            UnitContext {
                serial: serial.clone(),
                platform: dspi_proto::Platform::from_id(s.platform_id)
                    .name()
                    .to_owned(),
                device_rate_hz: rate,
                num_inputs: s.num_input_channels,
                limiter_outputs: p
                    .outputs
                    .iter()
                    .map(|o| o.output)
                    .filter(|o| {
                        s.limiter_enabled
                            .get(usize::from(*o))
                            .copied()
                            .unwrap_or(false)
                    })
                    .collect(),
                chains: p
                    .outputs
                    .iter()
                    .map(|o| (o.output, super::preflight::chain_of(s, o.output)))
                    .collect(),
            }
        };
        Ok(MeasureSystemUnitCheck {
            serial,
            preflight,
            context: serde_json::to_string(&ctx).unwrap_or_default(),
        })
    }

    /// What stands between `setup` and a system measurement, each item
    /// naming its unit: the hub (`system_measure`, a role above Viewer),
    /// every unit's own check (`checks`, one per unit), one rate, a plan
    /// that fits. Talks to no unit.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a setup that is not one.
    pub fn measure_system_preflight(
        &self,
        setup: MeasureSystemSetup,
        checks: Vec<MeasureSystemUnitCheck>,
    ) -> Result<MeasureSystemPreflight> {
        let mut slot = self.system_slot();
        let pre = self.with_hub(&mut slot, |hub| plan::preflight(&setup, &checks, hub))?;
        Ok(pre.0)
    }

    /// Start a system measurement: the preflight, then `system.measure.open`
    /// (the hub takes every unit's device lock and checks one domain), then
    /// the pre-roll. Nothing plays before the client has pushed the pre-roll
    /// (`measure_system_capture_push`) and ticked. Tick about every 50 ms
    /// (`measure_system_tick`) until the phase is `done`, `aborted` or
    /// `failed`.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a bad setup, a blocker
    /// (named with its unit), a session already running, or an applied
    /// alignment still waiting for Keep or Revert (nothing sent);
    /// [`DspiError::Unsupported`] for a hub without `system_measure` ("measuring
    /// several units together needs the DSPi hub"); [`DspiError::Denied`]
    /// for a Viewer; [`DspiError::Link`] with the hub's refusal
    /// (`not_one_domain`, `not_locked`, `no_timeline`, `locked`, ...).
    pub fn measure_system_start(
        &self,
        setup: MeasureSystemSetup,
        checks: Vec<MeasureSystemUnitCheck>,
    ) -> Result<MeasureSystemState> {
        let state = {
            let mut slot = self.system_slot();
            if slot.session.as_ref().is_some_and(SysSession::active) {
                return Err(invalid(
                    "busy: a system measurement is already running on this console",
                ));
            }
            // -- review AT-D2 -- a new measurement would replace the outcome
            // that holds what was filed (and the hub's guard): the applied
            // alignment could then be neither kept nor reverted from here,
            // and the hub would put it back at its deadline unseen
            if slot
                .outcome
                .as_ref()
                .is_some_and(proposal::Outcome::awaits_keep)
            {
                return Err(invalid(
                    "an applied alignment across units waits for Keep or Revert: keep or revert \
                     it before measuring again",
                ));
            }
            let (pre, units) =
                self.with_hub(&mut slot, |hub| plan::preflight(&setup, &checks, hub))?;
            if let Some(e) = plan::refusal(&pre) {
                return Err(e);
            }
            // -- WP ATDEMO -- the simulated microphone hears every unit, or nothing starts
            let serials: Vec<String> = setup.units.iter().map(|u| u.serial.clone()).collect();
            self.system_sim_probe(&mut slot, &serials)?;
            let session = self.with_hub(&mut slot, |hub| {
                SysSession::open(
                    plan::SetupPlan::new(&setup, units)?,
                    MeasureSystemPurpose::Measure,
                    None,
                    hub,
                    Instant::now(),
                )
            })?;
            let state = session.state();
            slot.session = Some(session);
            slot.outcome = None;
            state
        };
        self.measure.listener.call(|l| l.on_measure());
        Ok(state)
    }

    /// Push one block of the microphone's capture (as
    /// `measure_capture_push`): the frame index, a clip and the SPL cap are
    /// checked at once (S4, S5); nothing is analysed here. Ignored with no
    /// session.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for more than 1 Mi samples.
    pub fn measure_system_capture_push(
        &self,
        first_frame: u64,
        samples: Vec<f32>,
        rate_hz: u32,
        clipped: bool,
    ) -> Result<MeasureSystemState> {
        if samples.len() > super::session::MAX_BLOCK {
            return Err(invalid(format!(
                "a capture block of {} samples (at most {})",
                samples.len(),
                super::session::MAX_BLOCK
            )));
        }
        let (state, changed) = {
            let mut slot = self.system_slot();
            // -- WP ATDEMO -- the simulated microphone is the capture: a client's is ignored
            if slot.sim.is_on() {
                return Ok(slot
                    .session
                    .as_ref()
                    .map_or_else(MeasureSystemState::default, SysSession::state));
            }
            let Some(mut s) = slot.session.take() else {
                return Ok(MeasureSystemState::default());
            };
            let before = s.phase_key();
            self.with_hub(&mut slot, |hub| {
                s.push(hub, first_frame, &samples, rate_hz, clipped);
            });
            let changed = before != s.phase_key();
            let state = s.state();
            self.system_settle(&mut slot, s);
            (state, changed)
        };
        if changed {
            self.measure.listener.call(|l| l.on_measure());
        }
        Ok(state)
    }

    /// Drive the system session: the hub's status (about 4 Hz, which renews
    /// its lease), the per-unit judgements, the next step, one analysis
    /// slice. Call it about every 50 ms while active, off the UI thread.
    pub fn measure_system_tick(&self) -> MeasureSystemState {
        let (state, changed) = {
            let mut slot = self.system_slot();
            let Some(mut s) = slot.session.take() else {
                return MeasureSystemState::default();
            };
            let before = s.phase_key();
            self.system_sim_feed(&mut slot, &mut s); // -- WP ATDEMO --
            self.with_hub(&mut slot, |hub| s.tick(hub, Instant::now()));
            let changed = before != s.phase_key();
            let state = s.state();
            self.system_settle(&mut slot, s);
            (state, changed)
        };
        if changed {
            self.measure.listener.call(|l| l.on_measure());
        }
        state
    }

    /// The system session's state, without I/O.
    pub fn measure_system_state(&self) -> MeasureSystemState {
        self.system_slot()
            .session
            .as_ref()
            .map_or_else(MeasureSystemState::default, SysSession::state)
    }

    /// The user's Stop: `system.measure.abort` (every unit stopped, every
    /// START cancelled), the hub's session closed, the run discarded.
    /// Idempotent; never fails.
    pub fn measure_system_stop(&self) -> MeasureSystemState {
        let state = {
            let mut slot = self.system_slot();
            let Some(mut s) = slot.session.take() else {
                return MeasureSystemState::default();
            };
            self.with_hub(&mut slot, |hub| s.stop_by_user(hub));
            let state = s.state();
            slot.session = Some(s);
            state
        };
        self.measure.listener.call(|l| l.on_measure());
        state
    }

    /// The finished measurement: per unit its document, every arrival and
    /// cross-unit delay with its uncertainty, the transitions.
    ///
    /// Errors: [`DspiError::InvalidArgument`] while no result is ready.
    pub fn measure_system_result(&self) -> Result<MeasureSystemResult> {
        self.system_slot()
            .outcome
            .as_ref()
            .map(|o| o.result.clone())
            .ok_or_else(|| invalid("no system measurement result is ready"))
    }

    /// The alignment the last result proposes: the offered arrivals (the
    /// gate G1 to G4 passed and each one's delay to the group's latest
    /// output within 10 us), what is withheld and why, whether the summed
    /// check is still needed. No I/O but the role.
    ///
    /// Errors: [`DspiError::InvalidArgument`] while no result is ready.
    pub fn measure_system_proposal(&self) -> Result<MeasureSystemProposal> {
        let mut slot = self.system_slot();
        let role = self.with_hub(&mut slot, |hub| hub.role());
        // -- WP ATGUARD -- whether the hub guards the apply
        let guarded = self.with_hub(&mut slot, |hub| proposal::hub_guards(hub));
        let o = slot
            .outcome
            .as_ref()
            .ok_or_else(|| invalid("no system measurement result is ready"))?;
        Ok(o.proposal_for(&role, guarded))
    }

    /// Start the summed check of every inter-unit transition (RC5f): one
    /// summed run per transition, the two units started at one T, compared
    /// with the sum the walked run predicts. **A session under every S-rule
    /// of its own** (review AT-C, as P5's summed check): `checks` are each
    /// unit's fresh [`DspiCore::measure_system_unit_check`] (the preflight
    /// S2, S6, S8, S11, S13 is judged again, a blocker names its unit and
    /// nothing plays), and each output of a transition is ramped again (S3)
    /// before the summed run plays 6 dB below the lower of the two levels
    /// (S-AT4), never above the measurement's.
    ///
    /// Errors: as [`DspiCore::measure_system_start`]; [`DspiError::
    /// InvalidArgument`] with no result or no transition.
    pub fn measure_system_sum_check(
        &self,
        checks: Vec<MeasureSystemUnitCheck>,
    ) -> Result<MeasureSystemState> {
        self.system_runs_start(MeasureSystemPurpose::SumCheck, &checks)
    }

    /// Apply proposal `proposal_id` (Admin): the hub's model revision and
    /// every measured unit's output delays are filed, the offered arrivals
    /// are written into the model (`system.set`, guarded by the revision),
    /// then `system.align` writes the units. Repeat the summed runs next
    /// (`measure_system_verify`), then Keep or Revert.
    ///
    /// WP ATGUARD: on a hub with the cap `system_apply_guard` the hub's
    /// guard is begun first (`system.guard.begin`, the proposal's
    /// `guard_s`: the planned verify, 25 % longer, plus 120 s, at most
    /// 590 s): the hub reverts by itself at the deadline unless Keep
    /// confirms it, and when this console's connection is lost. Without the
    /// cap nothing reverts by itself (the proposal says so).
    ///
    /// Errors: [`DspiError::Denied`] for a role below Admin, nothing sent;
    /// [`DspiError::InvalidArgument`] for another id, a summed check not
    /// passed, nothing offered; [`DspiError::Link`] with the hub's refusal
    /// (`conflict` when the model changed meanwhile).
    pub fn measure_system_apply(&self, proposal_id: u32) -> Result<MeasureSystemApplied> {
        let mut slot = self.system_slot();
        if slot.session.as_ref().is_some_and(SysSession::active) {
            return Err(invalid("a system measurement is running: wait or stop it"));
        }
        let Some(mut o) = slot.outcome.take() else {
            return Err(invalid("no system measurement result is ready"));
        };
        let r = self.with_hub(&mut slot, |hub| o.apply(hub, proposal_id));
        slot.outcome = Some(o);
        r
    }

    /// Repeat the summed runs after an apply (P5's verify step), predicted
    /// with each output's applied change. A session under every S-rule of
    /// its own, as [`DspiCore::measure_system_sum_check`]: `checks` are each
    /// unit's fresh check, and each output is ramped again.
    ///
    /// Errors: as [`DspiCore::measure_system_sum_check`]; [`DspiError::
    /// InvalidArgument`] with nothing applied.
    pub fn measure_system_verify(
        &self,
        checks: Vec<MeasureSystemUnitCheck>,
    ) -> Result<MeasureSystemState> {
        self.system_runs_start(MeasureSystemPurpose::Verify, &checks)
    }

    /// Where the applied alignment stands.
    pub fn measure_system_applied(&self) -> MeasureSystemApplied {
        self.system_slot()
            .outcome
            .as_ref()
            .map(|o| o.applied.clone())
            .unwrap_or_default()
    }

    /// Keep the applied alignment.
    ///
    /// **After a verify that did not pass** the first Keep is refused
    /// (nothing changes; the applied record's `message` says why): the
    /// client shows it and asks the user again, and only a second Keep, at
    /// least a second later, keeps it (as `measure_tune_keep`). A new
    /// verify asks again.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with nothing applied, or the
    /// first Keep after a failed verify.
    ///
    /// WP ATGUARD: a guarded apply is confirmed on the hub
    /// (`system.guard.confirm`); one the hub has reverted already (its
    /// deadline passed) is shown as reverted, with the hub's words.
    pub fn measure_system_keep(&self) -> Result<MeasureSystemApplied> {
        let mut slot = self.system_slot();
        let Some(mut o) = slot.outcome.take() else {
            return Err(invalid("nothing is applied"));
        };
        let r = self.with_hub(&mut slot, |hub| o.keep(hub));
        slot.outcome = Some(o);
        r
    }

    /// Revert the applied alignment (Admin): the filed model is set again
    /// (`system.set`) and `system.align` writes the units back; every unit's
    /// output delays are read back against the filed ones.
    ///
    /// WP ATGUARD: a guarded apply is reverted by the hub
    /// (`system.guard.cancel`: the same set and alignment), which ends the
    /// guard.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with nothing applied;
    /// [`DspiError::Denied`] below Admin; the hub's refusals.
    pub fn measure_system_revert(&self) -> Result<MeasureSystemApplied> {
        let mut slot = self.system_slot();
        let Some(mut o) = slot.outcome.take() else {
            return Err(invalid("nothing is applied"));
        };
        let r = self.with_hub(&mut slot, |hub| o.revert(hub));
        slot.outcome = Some(o);
        r
    }
}

impl DspiCore {
    pub(crate) fn system_slot(&self) -> std::sync::MutexGuard<'_, SystemSlot> {
        self.measure
            .system
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Run `f` with this core's hub (or a test's scripted one).
    fn with_hub<R>(&self, slot: &mut SystemSlot, f: impl FnOnce(&mut dyn Hub) -> R) -> R {
        #[cfg(test)]
        if let Some(h) = slot.hub.as_mut() {
            return f(h.as_mut());
        }
        let _ = &slot;
        let mut hub = CoreHub { core: self };
        f(&mut hub)
    }

    // -- WP ATDEMO --
    /// Before a session with the simulated microphone on: the hub's room
    /// hears every one of `serials`, or the session does not start.
    fn system_sim_probe(&self, slot: &mut SystemSlot, serials: &[String]) -> Result<()> {
        let mut sim = std::mem::take(&mut slot.sim);
        let r = self.with_hub(slot, |hub| {
            if sim.on(hub) {
                sim.probe(hub, serials)
            } else {
                Ok(())
            }
        });
        slot.sim = sim;
        r
    }

    // -- WP ATDEMO --
    /// The simulated microphone's capture since the last tick, pushed.
    fn system_sim_feed(&self, slot: &mut SystemSlot, s: &mut SysSession) {
        let mut sim = std::mem::take(&mut slot.sim);
        self.with_hub(slot, |hub| {
            if sim.on(hub) {
                sim.feed(hub, s);
            }
        });
        slot.sim = sim;
    }

    /// Put a session back; a finished one leaves its outcome.
    fn system_settle(&self, slot: &mut SystemSlot, mut s: SysSession) {
        match s.take_outcome() {
            Some(SessionOutput::Measured(o)) => slot.outcome = Some(*o),
            Some(SessionOutput::Verdicts(v)) => {
                if let Some(o) = slot.outcome.as_mut() {
                    o.absorb_verdicts(s.purpose(), v);
                }
            }
            None => {}
        }
        slot.session = Some(s);
    }

    fn system_runs_start(
        &self,
        purpose: MeasureSystemPurpose,
        checks: &[MeasureSystemUnitCheck],
    ) -> Result<MeasureSystemState> {
        let state = {
            let mut slot = self.system_slot();
            if slot.session.as_ref().is_some_and(SysSession::active) {
                return Err(invalid(
                    "busy: a system measurement is already running on this console",
                ));
            }
            // -- WP ATGUARD -- a verify runs inside the hub's guard: one
            // the hub has ended (its deadline) is not verified
            if purpose == MeasureSystemPurpose::Verify
                && let Some(mut o) = slot.outcome.take()
            {
                let alive = self.with_hub(&mut slot, |hub| o.guard_alive(hub));
                slot.outcome = Some(o);
                alive?;
            }
            let Some(o) = slot.outcome.as_ref() else {
                return Err(invalid("no system measurement result is ready"));
            };
            let jobs = o.sum_jobs(purpose)?;
            let mut setup = o.setup.clone();
            // Review AT-C: the per-unit preflight again, on fresh checks
            // (S2, S6, S8, S11, S13 as they are now, not as they were when
            // the walked run was measured); what the limiters are now.
            let (pre, units) =
                self.with_hub(&mut slot, |hub| plan::preflight(&setup.setup, checks, hub))?;
            if let Some(e) = plan::refusal(&pre) {
                return Err(e);
            }
            if units.len() != setup.units.len()
                || units.iter().any(|u| u.ctx.device_rate_hz != setup.rate_hz)
            {
                return Err(invalid(
                    "the units changed since the measurement (their rate): measure again",
                ));
            }
            for (old, new) in setup.units.iter_mut().zip(units) {
                old.ctx.limiter_outputs = new.ctx.limiter_outputs;
            }
            // -- WP ATDEMO -- the simulated microphone hears every unit, or nothing starts
            let serials: Vec<String> = setup.units.iter().map(|u| u.serial.clone()).collect();
            self.system_sim_probe(&mut slot, &serials)?;
            let session = self.with_hub(&mut slot, |hub| {
                SysSession::open(setup, purpose, Some(jobs), hub, Instant::now())
            })?;
            let state = session.state();
            slot.session = Some(session);
            state
        };
        self.measure.listener.call(|l| l.on_measure());
        Ok(state)
    }

    /// The hub said it aborted a system session on its own (the link's
    /// supervisor, with no lock held).
    #[cfg(feature = "link")]
    pub(crate) fn measure_system_on_aborted(&self, e: AbortedEvent) {
        let mut q = self
            .measure
            .system_aborted
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if q.len() < 64 {
            q.push(e);
        }
    }

    /// Install a scripted hub for a test.
    #[cfg(test)]
    pub(crate) fn measure_system_set_hub(&self, hub: Box<dyn Hub>) {
        self.system_slot().hub = Some(hub);
    }
}

/// The slot's aborted-event queue type.
pub(crate) type AbortedQueue = Mutex<Vec<AbortedEvent>>;
