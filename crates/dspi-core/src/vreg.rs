//! Core-voltage calibration and the Advanced page's manual requests (WP
//! CG11): `dspi_session::vreg_calibrate`'s flow, on this core's connection.
//! The limits, and what the firmware does and does not report, are documented
//! there.
//!
//! **Nothing here starts by itself.** A client calls
//! [`DspiCore::core_vreg_cal_poll`] on every connect and about once a second
//! while the page shows; a poll writes only the next step of a run the user
//! started, and never a `SET`. A trial found on the device is offered as end
//! or keep. **No write is replayed:** a request that fails stays failed, and a
//! run whose `TRIAL` had not gone out when the connection changed ends as
//! unknown.
//!
//! **Hidden over a hub:** a viewer's read is refused, so the extension counts
//! as absent; a refused write hides the flow until the next connection.
//!
//! `core_vreg_op` stays as it was for the Terminal CLI, with the firmware's
//! limits. A GUI uses the calls here.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::MutexGuard;
use std::time::Instant;

use dspi_proto::generated::opcodes as op;
use dspi_session::ext::{CoreVregStatus, VregOp, vreg_mv};
use dspi_session::vreg_calibrate::{self as cal, Calibrator, FlowError, Phase, TrialFailure};
use dspi_transport::TransportKind;
use dspi_transport::fake::{EXT_FAKE_VREG_SIM, VregSim};

use crate::records::CoreVregInfo;
use crate::{DspiCore, DspiError, Result};

/// Where a run is: [`VregCalView::phase`]. The details are in the view.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VregCalPhase {
    /// The flow is not open.
    #[default]
    Idle,
    /// Open: show the warning and what the device has on record.
    Review,
    /// Clearing the device's record before the trial.
    Clearing,
    /// The trial was sent and is not seen running yet.
    StartingTrial,
    /// The trial runs; the soak is counting. Abort is available.
    Soaking,
    /// Passed: the trial still runs and nothing is saved. Commit or end.
    Passed,
    /// The trial hung: `failed_vreg`, `ran_s`, `next_vreg`, `hardware_problem`.
    Failed,
    /// The result cannot be known: run again.
    Unknown,
    /// A trial nobody here started is running: end or keep.
    TrialFound,
    /// A kept trial outside the flow's range: left alone, cannot pass.
    TrialLeftRunning,
    /// Ending the trial.
    Ending,
    /// The trial ended.
    Ended,
    /// Saving: waiting to read the voltage back.
    Committing,
    /// Saved and read back.
    Committed,
    /// A save or an end did not show in time. Nothing was retried.
    VerifyFailed,
}

/// A voltage the flow offers on the connected device.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VregChoice {
    /// VREG code.
    pub code: u8,
    /// The voltage, mV.
    pub millivolts: u16,
    /// What the user must type for it; `None` at 1.20 V and below.
    pub typed_phrase: Option<String>,
}

/// The flow as a client shows it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VregCalView {
    /// The connection can use the flow: `EXT_CORE_VREG` present, writable,
    /// and not refused by a hub. While false with phase `Idle`, hide the
    /// flow. While false in another phase the device is away: show the phase.
    pub available: bool,
    /// Where the flow is.
    pub phase: VregCalPhase,
    /// A stable key for `message`, for clients that localise.
    pub message_key: String,
    /// The state or result, stated plainly in English.
    pub message: String,
    /// The warning to show before the flow, with the datasheet limits.
    pub warning: String,
    /// What a soak does and does not show.
    pub soak_note: String,
    /// The voltages offered, lowest first. Empty on a build default the flow
    /// does not cover, or before a first status.
    pub choices: Vec<VregChoice>,
    /// The soak to suggest, seconds.
    pub default_soak_s: u32,
    /// The shortest soak accepted, seconds.
    pub min_soak_s: u32,
    /// The run's or found trial's voltage; 0 if none.
    pub target_vreg: u8,
    /// The soak asked for, seconds; 0 if none.
    pub soak_s: u32,
    /// Seconds of soak counted so far.
    pub elapsed_s: u32,
    /// The failure shown or on record: its voltage; 0 if none.
    pub failed_vreg: u8,
    /// Seconds the failed trial ran, if this host saw it start and the record
    /// still has the time.
    pub ran_s: Option<u32>,
    /// The device's uptime when that trial hung; 0 if the record lost it. Its
    /// whole run, so at most an upper bound on the trial.
    pub uptime_at_hang_s: u32,
    /// The next voltage up to offer after that failure; 0 if none.
    pub next_vreg: u8,
    /// Nothing above that failure to try: a hardware problem.
    pub hardware_problem: bool,
    /// Why the result is unknown (`record_gone`, `not_started`,
    /// `trial_changed`); `None` unless `phase` is `Unknown`.
    pub unknown_reason: Option<String>,
    /// The device stopped answering since the last status: a hang in
    /// progress, or a dropped link. The result is still open.
    pub link_lost: bool,
    /// `core_vreg_cal_start` applies now.
    pub can_start: bool,
    /// `core_vreg_cal_keep` applies now.
    pub can_keep: bool,
    /// `core_vreg_cal_commit` applies now.
    pub can_commit: bool,
    /// The last status read from the device.
    pub status: Option<CoreVregInfo>,
    /// Why the last call or read failed, in a sentence; `None` once one
    /// succeeded.
    pub last_error: Option<String>,
}

/// What can happen to the demo device: [`DspiCore::core_vreg_demo_event`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VregDemoEvent {
    /// The watchdog fires.
    Hang,
    /// A requested reboot: no failure on record.
    Reboot,
    /// Power lost: every record gone.
    PowerCycle,
    /// `seconds` pass on the device's clock.
    AdvanceUptime,
}

/// What `DspiCore::vreg_cal` holds.
pub(crate) struct CalState {
    machine: Calibrator,
    /// The clock the deadlines run on.
    origin: Instant,
    /// The connection the flow last saw, and the device's serial. Kept across
    /// a drop, so a device coming back is known as the same one.
    generation: Option<u64>,
    serial: Option<String>,
    /// The last status read.
    status: Option<CoreVregStatus>,
    /// A hub refused a write on this connection.
    denied: bool,
    /// The extension is usable on this connection, as far as last read.
    usable: bool,
    last_error: Option<String>,
}

impl CalState {
    pub(crate) fn new() -> Self {
        Self {
            machine: Calibrator::new(),
            origin: Instant::now(),
            generation: None,
            serial: None,
            status: None,
            denied: false,
            usable: false,
            last_error: None,
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The flow is out of this client's reach: nothing to show or to do.
    fn hide(&mut self) {
        self.machine.reset();
        self.status = None;
        self.usable = false;
    }

    fn view(&self) -> VregCalView {
        let phase = *self.machine.phase();
        let (target_vreg, soak_s) = match phase {
            Phase::Clearing { run, .. }
            | Phase::StartingTrial { run, .. }
            | Phase::Soaking { run, .. }
            | Phase::Passed { run, .. } => (run.target, run.soak_s),
            Phase::Unknown { target, .. } => (target, 0),
            Phase::TrialFound { vreg } | Phase::TrialLeftRunning { vreg } => (vreg, 0),
            Phase::Committing { value, .. } | Phase::Committed { value } => (value, 0),
            _ => (0, 0),
        };
        let failure = match phase {
            Phase::Failed(f) => Some(f),
            _ => self.status.as_ref().and_then(TrialFailure::on_record),
        };
        let choices = match &self.status {
            Some(s) => cal::offered(s.default_vreg)
                .into_iter()
                .map(|code| VregChoice {
                    code,
                    millivolts: vreg_mv(code).unwrap_or(0),
                    typed_phrase: cal::typed_phrase(code),
                })
                .collect(),
            None => Vec::new(),
        };
        VregCalView {
            available: self.usable && !self.denied,
            phase: flat_phase(&phase),
            message_key: phase.key().to_owned(),
            message: phase.message(),
            warning: cal::WARNING.to_owned(),
            soak_note: cal::SOAK_NOTE.to_owned(),
            choices,
            default_soak_s: cal::DEFAULT_SOAK_S,
            min_soak_s: cal::MIN_SOAK_S,
            target_vreg,
            soak_s,
            elapsed_s: self
                .status
                .as_ref()
                .map_or(0, |s| self.machine.soak_elapsed_s(s)),
            failed_vreg: failure.map_or(0, |f| f.vreg),
            ran_s: failure.and_then(|f| f.ran_s),
            uptime_at_hang_s: failure.map_or(0, |f| f.uptime_at_hang_s),
            next_vreg: failure.and_then(|f| f.next_vreg).unwrap_or(0),
            hardware_problem: failure.is_some_and(|f| f.hardware_problem),
            unknown_reason: match phase {
                Phase::Unknown { reason, .. } => Some(reason.key().to_owned()),
                _ => None,
            },
            link_lost: self.machine.link_lost(),
            can_start: self.machine.can_start() && self.usable && !self.denied,
            can_keep: self.machine.can_keep() && self.usable && !self.denied,
            can_commit: self.machine.can_commit() && self.usable && !self.denied,
            status: self.status.as_ref().map(info_of),
            last_error: self.last_error.clone(),
        }
    }
}

fn flat_phase(p: &Phase) -> VregCalPhase {
    match p {
        Phase::Idle => VregCalPhase::Idle,
        Phase::Review => VregCalPhase::Review,
        Phase::Clearing { .. } => VregCalPhase::Clearing,
        Phase::StartingTrial { .. } => VregCalPhase::StartingTrial,
        Phase::Soaking { .. } => VregCalPhase::Soaking,
        Phase::Passed { .. } => VregCalPhase::Passed,
        Phase::Failed(_) => VregCalPhase::Failed,
        Phase::Unknown { .. } => VregCalPhase::Unknown,
        Phase::TrialFound { .. } => VregCalPhase::TrialFound,
        Phase::TrialLeftRunning { .. } => VregCalPhase::TrialLeftRunning,
        Phase::Ending { .. } => VregCalPhase::Ending,
        Phase::Ended => VregCalPhase::Ended,
        Phase::Committing { .. } => VregCalPhase::Committing,
        Phase::Committed { .. } => VregCalPhase::Committed,
        Phase::VerifyFailed { .. } => VregCalPhase::VerifyFailed,
    }
}

fn info_of(s: &CoreVregStatus) -> CoreVregInfo {
    CoreVregInfo {
        live_vreg: s.live_vreg,
        millivolts: vreg_mv(s.live_vreg).unwrap_or(0),
        stored_sel: s.stored_sel,
        default_vreg: s.default_vreg,
        auto_raised: s.flags & CoreVregStatus::FLAG_AUTO_RAISED != 0,
        auto_disabled: s.flags & CoreVregStatus::FLAG_AUTO_DISABLED != 0,
        hang_count: s.hang_count,
        trial_active: s.trial_active,
        trial_vreg: s.trial_vreg,
        failed_trial_vreg: s.failed_trial_vreg,
        failed_trial_uptime_s: s.failed_trial_uptime_s,
        uptime_s: s.uptime_s,
    }
}

fn status_of(i: &CoreVregInfo) -> CoreVregStatus {
    let flag = |on: bool, bit: u8| if on { bit } else { 0 };
    CoreVregStatus {
        live_vreg: i.live_vreg,
        stored_sel: i.stored_sel,
        default_vreg: i.default_vreg,
        flags: flag(i.auto_raised, CoreVregStatus::FLAG_AUTO_RAISED)
            | flag(i.auto_disabled, CoreVregStatus::FLAG_AUTO_DISABLED),
        hang_count: i.hang_count,
        trial_active: i.trial_active,
        trial_vreg: i.trial_vreg,
        failed_trial_vreg: i.failed_trial_vreg,
        failed_trial_uptime_s: i.failed_trial_uptime_s,
        uptime_s: i.uptime_s,
    }
}

fn flow_error(e: FlowError) -> DspiError {
    DspiError::InvalidArgument { msg: e.to_string() }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The calibration flow as it stands, from memory: no traffic. It waits
    /// for a poll or request in flight, so a slow link can hold it up as long
    /// as that read: call it off the UI thread too.
    pub fn core_vreg_cal_view(&self) -> VregCalView {
        self.vreg_cal_lock().view()
    }

    /// Read the device and advance the flow; never fails, the view says.
    /// Call on every connect and about once a second while the page shows.
    /// The only write that can go out is the next step of a run the user
    /// started; never a `SET`, and none after the connection changed. Blocks
    /// for a read: call it off the UI thread.
    pub fn core_vreg_cal_poll(&self) -> VregCalView {
        let mut cal = self.vreg_cal_lock();
        // Errors are in the view.
        let _ = self.vreg_read_and_advance(&mut cal);
        cal.view()
    }

    /// Open the flow: the user reached it, from Advanced. Shows the warning
    /// and what is on record; sends no write.
    ///
    /// `Disconnected`; `Unsupported` without a usable `EXT_CORE_VREG`;
    /// `Denied` after a hub refused a write on this connection; `ReadOnly`;
    /// `InvalidArgument` during a run.
    pub fn core_vreg_cal_open(&self) -> Result<()> {
        self.vreg_call(|_, cal, s, now| cal.machine.open(s, now).map_err(flow_error))
    }

    /// Close the flow: the user left. No traffic. `InvalidArgument` during a
    /// run, which goes on until ended.
    pub fn core_vreg_cal_close(&self) -> Result<()> {
        let mut cal = self.vreg_cal_lock();
        let r = cal.machine.close().map_err(flow_error);
        note(&mut cal, r)
    }

    /// Start a run: the current record is cleared, `target` goes on trial
    /// once the device shows that, and the soak counts for `soak_s` seconds.
    /// `typed` is what the user typed for a voltage above 1.20 V.
    ///
    /// `InvalidArgument`, with no write, for a voltage outside the build
    /// default to 1.25 V, a soak under the minimum, a missing or wrong typed
    /// phrase, a trial already running, or the wrong phase. `Disconnected`,
    /// `Unsupported`, `ReadOnly`; `Refused`, `Denied`, `Locked`, `Transfer` for
    /// the write, which is not retried.
    pub fn core_vreg_cal_start(
        &self,
        target: u8,
        soak_s: u32,
        typed: Option<String>,
    ) -> Result<()> {
        self.vreg_call(|core, cal, s, now| {
            let op = cal
                .machine
                .start(s, target, soak_s, typed.as_deref(), now)
                .map_err(flow_error)?;
            core.vreg_send(cal, op)
        })
    }

    /// Keep a trial found on the device: the soak counts from now, for
    /// `soak_s` seconds. A trial outside the flow's range is left alone and
    /// cannot pass. Sends nothing. Errors as for opening.
    pub fn core_vreg_cal_keep(&self, soak_s: u32) -> Result<()> {
        self.vreg_call(|_, cal, s, _| cal.machine.keep_trial(s, soak_s).map_err(flow_error))
    }

    /// Save the voltage that passed, with a `SET`: the one explicit step from
    /// a pass to flash. The status is read first, so a hang since the last
    /// poll takes the pass back. Read back by later polls: `Committed` or
    /// `VerifyFailed`. `typed` as for starting. Errors as for starting.
    pub fn core_vreg_cal_commit(&self, typed: Option<String>) -> Result<()> {
        self.vreg_call(|core, cal, s, now| {
            let op = cal
                .machine
                .commit(s, typed.as_deref(), now)
                .map_err(flow_error)?;
            core.vreg_send(cal, op)
        })
    }

    /// End a trial, or abort a run, with `TRIAL_END`, at any time but while a
    /// save is being read back. Works without a fresh status, so a run can be
    /// aborted while reads are failing. `Disconnected` and `ReadOnly` change
    /// nothing. A write that fails otherwise is not retried: later polls end
    /// in `Ended`, or in `VerifyFailed`, from which it can be asked for again.
    pub fn core_vreg_trial_end(&self) -> Result<()> {
        let mut cal = self.vreg_cal_lock();
        let r = self.vreg_trial_end_locked(&mut cal);
        note(&mut cal, r)
    }

    /// Manual `SET` from the Advanced page. `value` 0 is the reset to the
    /// build default; anything else has the flow's limits and confirmation.
    /// Read back by later polls, as a commit. Errors as for starting.
    pub fn core_vreg_manual_set(&self, value: u8, typed: Option<String>) -> Result<()> {
        self.vreg_call(|core, cal, s, now| {
            let op = cal
                .machine
                .manual_set(s, value, typed.as_deref(), now)
                .map_err(flow_error)?;
            core.vreg_send(cal, op)
        })
    }

    /// Manual `TRIAL`, with the flow's limits and confirmation. It shows as a
    /// found trial on the next poll. Errors as for starting.
    pub fn core_vreg_manual_trial(&self, value: u8, typed: Option<String>) -> Result<()> {
        self.vreg_call(|core, cal, s, _| {
            let op = cal
                .machine
                .manual_trial(s, value, typed.as_deref())
                .map_err(flow_error)?;
            core.vreg_send(cal, op)
        })
    }

    /// Manual `AUTO`: the firmware's automatic raising after repeated hangs.
    /// Errors as for starting.
    pub fn core_vreg_manual_auto(&self, on: bool) -> Result<()> {
        self.vreg_call(|core, cal, _, _| core.vreg_send(cal, VregOp::Auto(on)))
    }

    /// Manual `CLEAR`: forget the hang count and the last trial failure.
    /// `InvalidArgument` during a run, whose result it would erase. Errors
    /// otherwise as for starting.
    pub fn core_vreg_manual_clear(&self) -> Result<()> {
        self.vreg_call(|core, cal, _, _| {
            let op = cal.machine.manual_clear().map_err(flow_error)?;
            core.vreg_send(cal, op)
        })
    }

    /// Play an event on the demo device, for clients' tests and demo mode.
    /// `seconds` is for [`VregDemoEvent::AdvanceUptime`]. `Unsupported`, with
    /// no traffic, on anything but a simulated device.
    pub fn core_vreg_demo_event(&self, event: VregDemoEvent, seconds: u32) -> Result<()> {
        let sim = match event {
            VregDemoEvent::Hang => VregSim::Hang,
            VregDemoEvent::Reboot => VregSim::Reboot,
            VregDemoEvent::PowerCycle => VregSim::PowerCycle,
            VregDemoEvent::AdvanceUptime => VregSim::Advance(seconds),
        };
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        if w.conn().caps.kind != TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        w.conn()
            .transport()
            .control_out(op::REQ_EXTENSION, EXT_FAKE_VREG_SIM, &sim.encode())
            .map_err(DspiError::from)
    }
}

/// Record a call's outcome for the view.
fn note(cal: &mut CalState, r: Result<()>) -> Result<()> {
    cal.last_error = r.as_ref().err().map(ToString::to_string);
    r
}

impl DspiCore {
    fn vreg_cal_lock(&self) -> MutexGuard<'_, CalState> {
        // Every update is a plain assignment between calls, so a panic leaves
        // nothing half-written that matters.
        self.vreg_cal.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Note a change of connection. Back on the same device the flow goes on,
    /// but it has lost its link; on another device it starts over.
    fn vreg_sync_connection(&self, cal: &mut CalState) {
        let now = self
            .lock()
            .conn()
            .ok()
            .map(|c| (c.generation, c.serial.clone()));
        let Some((generation, serial)) = now else {
            if cal.generation.take().is_some() {
                cal.machine.on_link_lost();
            }
            cal.usable = false;
            cal.denied = false;
            return;
        };
        if cal.generation == Some(generation) {
            return;
        }
        cal.generation = Some(generation);
        cal.denied = false;
        match &cal.serial {
            Some(known) if *known != serial => {
                cal.machine.reset();
                cal.status = None;
                cal.last_error = None;
            }
            _ => cal.machine.on_link_lost(),
        }
        cal.serial = Some(serial);
    }

    /// Read the status, and advance the flow with it. On a failed read the
    /// flow has lost its link, or is hidden if the extension is out of reach.
    fn vreg_read_and_advance(&self, cal: &mut CalState) -> Result<CoreVregStatus> {
        self.vreg_sync_connection(cal);
        if cal.denied {
            return Err(DspiError::Denied);
        }
        let writable = self.lock().conn().map(|c| c.writable());
        if writable == Ok(false) {
            cal.hide();
            return Err(DspiError::ReadOnly);
        }
        let status = match self.fetch_core_vreg() {
            Ok(info) => status_of(&info),
            Err(DspiError::Unsupported) => {
                cal.hide();
                return Err(DspiError::Unsupported);
            }
            Err(e) => {
                cal.machine.on_link_lost();
                cal.usable = false;
                cal.last_error = Some(e.to_string());
                return Err(e);
            }
        };
        cal.usable = true;
        cal.last_error = None;
        cal.status = Some(status.clone());
        let now = cal.now_ms();
        if let Some(trial) = cal.machine.on_status(&status, now) {
            let _ = self.vreg_send(cal, trial.op());
        }
        Ok(status)
    }

    /// One user request: on a fresh status, with the outcome noted.
    fn vreg_call<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&Self, &mut CalState, &CoreVregStatus, u64) -> Result<()>,
    {
        let mut cal = self.vreg_cal_lock();
        let r = self.vreg_read_and_advance(&mut cal).and_then(|s| {
            let now = cal.now_ms();
            f(self, &mut cal, &s, now)
        });
        note(&mut cal, r)
    }

    /// Send one request, once. A failure is the flow's to know about; a hub's
    /// refusal hides it for this connection.
    fn vreg_send(&self, cal: &mut CalState, request: VregOp) -> Result<()> {
        let [code, value] = request.encode();
        let r = self.core_vreg_op(code, value);
        if let Err(e) = &r {
            cal.last_error = Some(e.to_string());
            if *e == DspiError::Denied {
                cal.denied = true;
                cal.machine.reset();
            } else {
                cal.machine.on_write_failed();
            }
        }
        r
    }

    fn vreg_trial_end_locked(&self, cal: &mut CalState) -> Result<()> {
        self.vreg_sync_connection(cal);
        // Nothing can be sent: the run is left as it is, for the status to judge.
        if !self.lock().conn()?.writable() {
            return Err(DspiError::ReadOnly);
        }
        let now = cal.now_ms();
        let request = cal.machine.end_trial(now).map_err(flow_error)?;
        self.vreg_send(cal, request)
    }
}

#[cfg(test)]
#[path = "vreg/tests_wp_cg11.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg11;
