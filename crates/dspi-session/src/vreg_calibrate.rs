//! Core-voltage calibration: the flow behind the Advanced page, as a pure
//! state machine (WP CG11, docs/core-merge/PLAN-CLIENTS.md). The caller
//! passes each status it read and its clock, and sends the requests it gets
//! back; nothing here does I/O or keeps time by itself.
//!
//! **The limits are structural.** [`Calibrator::on_status`], the only thing a
//! poll reaches, can return nothing but a [`FlowTrial`], and only as the next
//! step of a [`Calibrator::start`] the user asked for. A `SET` comes only from
//! [`Calibrator::commit`] or [`Calibrator::manual_set`]. Every voltage this
//! module asks for lies between the build default and 1.25 V
//! ([`in_flow_range`]), and anything above 1.20 V needs the typed phrase
//! ([`typed_phrase`]). 1.30 V is not reachable from here at all; the Terminal
//! CLI has its own path.
//!
//! **What the firmware tells us, and what it does not** (`core_vreg.c`):
//!
//! - `uptime_s` counts from boot, not from the trial. A device up for hours
//!   has `uptime_s` above any soak the moment a trial starts, so the soak is
//!   measured from the uptime at which the trial was first seen running.
//! - `failed_trial_uptime_s` is the whole run's uptime at the hang, so it too
//!   is only an upper bound on how long the trial ran. It lives in RAM: a
//!   further warm reset keeps `failed_trial_vreg` and zeroes it, which reads
//!   as "how long is not known".
//! - Only a hang records a failure. A requested reboot (USB reset, BOOTSEL)
//!   ends the trial with no record, as a power cycle does. Both are "unknown".
//! - Requests are staged in one slot and applied by the main loop, so a status
//!   read right after a write can show the old state, and a second write can
//!   replace a first that was not applied. Every write here is therefore
//!   confirmed by a later status before the next one goes out, within
//!   [`VERIFY_TIMEOUT_MS`].
//! - The status reports the RAM copy of `stored_sel`, written before the flash
//!   directory, so reading it back shows the choice was taken, not that it
//!   survived a power cycle.
//!
//! **After a restart of the app** the host has nothing to go on: the firmware
//! does not say when a trial began. [`Calibrator::keep_trial`] counts the soak
//! from that moment on, which can only understate it. A pass is never
//! inferred from uptime alone.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use crate::ext::{CoreVregStatus, VregOp, vreg_mv};

/// `VREG_VOLTAGE_1_15`: the firmware's floor, the lowest build default.
pub const VREG_1_15: u8 = 0x0C;
/// `VREG_VOLTAGE_1_20`: the highest voltage offered without typed confirmation.
pub const VREG_1_20: u8 = 0x0D;
/// `VREG_VOLTAGE_1_25` (`CORE_VREG_AUTO_CAP`): the flow's ceiling.
pub const VREG_1_25: u8 = 0x0E;

/// The soak the flow suggests, seconds.
pub const DEFAULT_SOAK_S: u32 = 30 * 60;
/// The shortest soak the flow accepts, seconds.
pub const MIN_SOAK_S: u32 = 10 * 60;
/// How long a write may take to show in the status, ms. The main loop applies
/// it at once in normal running; a `SET` also takes a flash write.
pub const VERIFY_TIMEOUT_MS: u64 = 5_000;

/// The warning shown before the flow, with the limits `core_vreg.h` records.
pub const WARNING: &str = "Core voltage calibration changes the voltage of the RP2350 in this \
device. RP2350 datasheet limits: recommended up to 1.16 V, absolute maximum 1.21 V, with a \
regulator tolerance of +/-3 %. 1.20 V is already outside the recommended range and can reach \
about 1.24 V at the upper tolerance. 1.25 V is above the absolute maximum even before tolerance. \
Audio may cut out and the device may drop off USB; lower the volume.";

/// What a soak does and does not show.
pub const SOAK_NOTE: &str = "Keep audio playing during the soak: that is when hangs happen. A \
short soak proves little: one K340 hung 5 times in about 9 minutes at 1.15 V and not once in 41 \
minutes at 1.20 V.";

/// Whether the flow may use `code` on a device with this build default: from
/// the build default up to 1.25 V, never below it and never 1.30 V. A build
/// default outside 1.15-1.25 V offers nothing.
pub fn in_flow_range(code: u8, default_vreg: u8) -> bool {
    (VREG_1_15..=VREG_1_25).contains(&default_vreg) && (default_vreg..=VREG_1_25).contains(&code)
}

/// The voltages the flow offers on a device with this build default, lowest
/// first.
pub fn offered(default_vreg: u8) -> Vec<u8> {
    (VREG_1_15..=VREG_1_25)
        .filter(|c| in_flow_range(*c, default_vreg))
        .collect()
}

/// The phrase a user must type for `code`; `None` when it needs none (1.20 V
/// and below).
pub fn typed_phrase(code: u8) -> Option<String> {
    if code <= VREG_1_20 {
        return None;
    }
    let mv = vreg_mv(code).unwrap_or(0);
    Some(format!("RAISE TO {}.{:02} V", mv / 1000, (mv % 1000) / 10))
}

fn check_typed(code: u8, typed: Option<&str>) -> Result<(), FlowError> {
    match typed_phrase(code) {
        Some(phrase) if typed.map(str::trim) != Some(phrase.as_str()) => {
            Err(FlowError::ConfirmationRequired { phrase })
        }
        _ => Ok(()),
    }
}

/// `resolve()` in `core_vreg.c`: the voltage a stored selection runs at.
fn resolve(stored_sel: u8, default_vreg: u8) -> u8 {
    let v = if stored_sel == 0 {
        default_vreg
    } else {
        stored_sel
    };
    v.max(default_vreg).clamp(VREG_1_15, 0x0F)
}

/// Why the flow refused a call. Nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FlowError {
    /// The call does not apply to the flow's current phase.
    #[error("not possible while the flow is {phase}")]
    WrongPhase {
        /// [`Phase::key`].
        phase: &'static str,
    },
    /// The voltage is outside the build default to 1.25 V.
    #[error(
        "voltage 0x{code:02X} is outside what this flow offers on a device with build default 0x{default_vreg:02X}"
    )]
    OutOfRange {
        /// The voltage asked for.
        code: u8,
        /// The device's build default.
        default_vreg: u8,
    },
    /// The soak is shorter than [`MIN_SOAK_S`].
    #[error("a soak takes at least {MIN_SOAK_S} s")]
    SoakTooShort,
    /// This voltage needs the typed phrase, and it was not given.
    #[error("type \"{phrase}\" to confirm this voltage")]
    ConfirmationRequired {
        /// What to type.
        phrase: String,
    },
    /// A trial is running on the device: end or keep it first.
    #[error("a trial is running on the device: end or keep it first")]
    TrialRunning,
}

/// The one request a poll can produce: the `TRIAL` of a flow the user started,
/// checked against the status it was decided on. It cannot be a `SET`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowTrial(u8);

impl FlowTrial {
    /// The voltage to trial.
    pub fn code(self) -> u8 {
        self.0
    }

    /// The request to send.
    pub fn op(self) -> VregOp {
        VregOp::Trial(self.0)
    }
}

/// A trial that ended in a hang, as far as the device can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialFailure {
    /// The voltage that hung.
    pub vreg: u8,
    /// Seconds the trial ran before the hang: `None` when the record's uptime
    /// is gone, or when this host did not see the trial start.
    pub ran_s: Option<u32>,
    /// `failed_trial_uptime_s`: the whole run's uptime at the hang; 0 when
    /// the record's uptime is gone.
    pub uptime_at_hang_s: u32,
    /// The next voltage up the flow offers, if any.
    pub next_vreg: Option<u8>,
    /// Nothing above it to try: "this looks like a hardware problem".
    pub hardware_problem: bool,
}

impl TrialFailure {
    fn of(s: &CoreVregStatus, start_uptime_s: Option<u32>) -> Self {
        let vreg = s.failed_trial_vreg;
        let uptime = s.failed_trial_uptime_s;
        let next_vreg = offered(s.default_vreg).into_iter().find(|c| *c > vreg);
        Self {
            vreg,
            ran_s: start_uptime_s
                .filter(|start| uptime != 0 && uptime >= *start)
                .map(|start| uptime.saturating_sub(start)),
            uptime_at_hang_s: uptime,
            next_vreg,
            hardware_problem: next_vreg.is_none(),
        }
    }

    /// The failure the device has on record, if any.
    pub fn on_record(s: &CoreVregStatus) -> Option<Self> {
        (s.failed_trial_vreg != 0).then(|| Self::of(s, None))
    }
}

/// Why a result is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// The trial is gone and no hang of it is on record: a power cycle, a
    /// requested reboot, or another console.
    RecordGone,
    /// The trial was never seen running.
    NotStarted,
    /// Another voltage is on trial now, set by someone else.
    TrialChanged,
}

impl UnknownReason {
    /// A stable key, for clients that localise.
    pub fn key(self) -> &'static str {
        match self {
            UnknownReason::RecordGone => "record_gone",
            UnknownReason::NotStarted => "not_started",
            UnknownReason::TrialChanged => "trial_changed",
        }
    }
}

/// A trial being watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// The trial voltage.
    pub target: u8,
    /// The soak asked for, seconds.
    pub soak_s: u32,
    /// `failed_trial_vreg` when the watch began: 0 after the flow's `CLEAR`;
    /// a kept trial cannot clear, so an old record must not count against it.
    baseline_failed: u8,
}

/// Where the flow is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The flow is not open. Polling here never writes.
    Idle,
    /// Open: the warning, and any hang count or failure on record. Nothing
    /// has been sent.
    Review,
    /// `CLEAR` sent; waiting to see it applied before the `TRIAL`.
    Clearing {
        /// What to trial next.
        run: Run,
        /// When to give up, in the caller's clock.
        deadline_ms: u64,
    },
    /// `TRIAL` sent; waiting to see it running.
    StartingTrial {
        /// The trial.
        run: Run,
        /// The uptime at the `TRIAL`.
        start_uptime_s: u32,
        /// When to give up, in the caller's clock.
        deadline_ms: u64,
    },
    /// The trial runs; the soak is counting.
    Soaking {
        /// The trial.
        run: Run,
        /// The uptime at which the trial was first seen running.
        start_uptime_s: u32,
    },
    /// Soaked for the whole time and still running. The user chooses
    /// [`Calibrator::commit`] or [`Calibrator::end_trial`].
    Passed {
        /// The trial.
        run: Run,
        /// The uptime at which the trial was first seen running.
        start_uptime_s: u32,
    },
    /// The trial hung.
    Failed(TrialFailure),
    /// The result cannot be known: run again.
    Unknown {
        /// The voltage the run was for.
        target: u8,
        /// Why.
        reason: UnknownReason,
    },
    /// A trial is running that this flow is not watching: found on connect,
    /// or set elsewhere. The user chooses [`Calibrator::end_trial`] or
    /// [`Calibrator::keep_trial`].
    TrialFound {
        /// Its voltage.
        vreg: u8,
    },
    /// The user kept a trial outside what the flow offers (a CLI's). It is
    /// left alone and cannot pass.
    TrialLeftRunning {
        /// Its voltage.
        vreg: u8,
    },
    /// `TRIAL_END` sent; waiting for the persisted voltage.
    Ending {
        /// When to give up, in the caller's clock.
        deadline_ms: u64,
    },
    /// The trial ended; the device runs its persisted voltage.
    Ended,
    /// `SET` sent; waiting to read it back.
    Committing {
        /// The value sent; 0 is the build default.
        value: u8,
        /// When to give up, in the caller's clock.
        deadline_ms: u64,
    },
    /// The `SET` read back as asked: `stored_sel` and `live_vreg` match.
    Committed {
        /// The value sent; 0 is the build default.
        value: u8,
    },
    /// A `SET` or `TRIAL_END` did not show in the status in time. Nothing
    /// was retried; the status says where the device is.
    VerifyFailed {
        /// The `SET` value, or `None` for a `TRIAL_END`.
        set_value: Option<u8>,
    },
}

impl Phase {
    /// A stable key, for clients that localise.
    pub fn key(&self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Review => "review",
            Phase::Clearing { .. } => "clearing",
            Phase::StartingTrial { .. } => "starting_trial",
            Phase::Soaking { .. } => "soaking",
            Phase::Passed { .. } => "passed",
            Phase::Failed(_) => "failed",
            Phase::Unknown { .. } => "unknown",
            Phase::TrialFound { .. } => "trial_found",
            Phase::TrialLeftRunning { .. } => "trial_left_running",
            Phase::Ending { .. } => "ending",
            Phase::Ended => "ended",
            Phase::Committing { .. } => "committing",
            Phase::Committed { .. } => "committed",
            Phase::VerifyFailed { .. } => "verify_failed",
        }
    }

    /// The result or state, stated plainly in English.
    pub fn message(&self) -> String {
        let v = |code: u8| {
            let mv = vreg_mv(code).unwrap_or(0);
            format!("{}.{:02} V", mv / 1000, (mv % 1000) / 10)
        };
        match self {
            Phase::Idle => String::new(),
            Phase::Review => "Review the warning and what the device has on record. Starting a \
                              trial clears that record first."
                .into(),
            Phase::Clearing { .. } => "Clearing the device's record.".into(),
            Phase::StartingTrial { run, .. } => format!("Starting a trial at {}.", v(run.target)),
            Phase::Soaking { run, .. } => format!(
                "Trial at {} running. Nothing is saved; a hang or reboot returns to the saved \
                 voltage.",
                v(run.target)
            ),
            Phase::Passed { run, .. } => format!(
                "Passed: no hang at {} during this soak. The trial is still running and nothing \
                 is saved. Save this voltage, or end the trial.",
                v(run.target)
            ),
            Phase::Failed(f) => {
                let ran = match f.ran_s {
                    Some(s) => format!(" after {s} s"),
                    None => String::new(),
                };
                let what = format!("Failed: the device hung at {}{ran}.", v(f.vreg));
                match f.next_vreg {
                    Some(next) => format!("{what} You can try {} next.", v(next)),
                    None => format!("{what} This looks like a hardware problem."),
                }
            }
            Phase::Unknown { reason, .. } => match reason {
                UnknownReason::RecordGone => "Unknown: the trial is gone and no hang is on \
                                              record, as after a power cycle or a reboot. Run it \
                                              again."
                    .into(),
                UnknownReason::NotStarted => {
                    "Unknown: the trial was not seen running. Run it again.".into()
                }
                UnknownReason::TrialChanged => {
                    "Unknown: another voltage was put on trial. Run it again.".into()
                }
            },
            Phase::TrialFound { vreg } => {
                format!(
                    "A trial at {} is running on this device. End it, or keep it.",
                    v(*vreg)
                )
            }
            Phase::TrialLeftRunning { vreg } => format!(
                "The trial at {} is left running. This flow does not offer that voltage, so it \
                 cannot pass here.",
                v(*vreg)
            ),
            Phase::Ending { .. } => "Ending the trial.".into(),
            Phase::Ended => "The trial ended; the device runs its saved voltage.".into(),
            Phase::Committing { .. } => "Saving the voltage.".into(),
            Phase::Committed { value } => {
                let what = if *value == 0 {
                    "the build default".to_owned()
                } else {
                    v(*value)
                };
                format!(
                    "Saved: {what}, read back from the device. Read it again after a power cycle \
                     to be sure it reached flash."
                )
            }
            Phase::VerifyFailed { set_value } => match set_value {
                Some(_) => "The device did not read back the voltage asked for. See its status; \
                            nothing was retried."
                    .into(),
                None => "The device did not return to its saved voltage. See its status; nothing \
                         was retried."
                    .into(),
            },
        }
    }

    /// Whether a flow is under way that a manual request would disturb.
    fn busy(&self) -> bool {
        matches!(
            self,
            Phase::Clearing { .. }
                | Phase::StartingTrial { .. }
                | Phase::Soaking { .. }
                | Phase::Passed { .. }
                | Phase::Ending { .. }
                | Phase::Committing { .. }
        )
    }
}

/// The result of comparing a watched trial with a status.
enum Verdict {
    Running,
    Failed,
    Unknown(UnknownReason),
}

/// The flow. One per client; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calibrator {
    phase: Phase,
    /// The uptime in the last status, to see a reboot.
    last_uptime_s: u32,
    /// The device stopped answering since the last status.
    link_lost: bool,
}

impl Default for Calibrator {
    fn default() -> Self {
        Self::new()
    }
}

impl Calibrator {
    /// A closed flow.
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            last_uptime_s: 0,
            link_lost: false,
        }
    }

    /// Where the flow is.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The device stopped answering since the last status: a hang in
    /// progress, or a dropped link. Says nothing yet about the result.
    pub fn link_lost(&self) -> bool {
        self.link_lost
    }

    /// Seconds the current soak has counted, from `status`.
    pub fn soak_elapsed_s(&self, status: &CoreVregStatus) -> u32 {
        match self.phase {
            Phase::Soaking { start_uptime_s, .. } | Phase::Passed { start_uptime_s, .. } => {
                status.uptime_s.saturating_sub(start_uptime_s)
            }
            _ => 0,
        }
    }

    /// Whether [`Calibrator::start`] applies now.
    pub fn can_start(&self) -> bool {
        matches!(
            self.phase,
            Phase::Review | Phase::Failed(_) | Phase::Unknown { .. }
        )
    }

    /// Whether [`Calibrator::commit`] applies now.
    pub fn can_commit(&self) -> bool {
        matches!(self.phase, Phase::Passed { .. })
    }

    /// Whether [`Calibrator::keep_trial`] applies now.
    pub fn can_keep(&self) -> bool {
        matches!(self.phase, Phase::TrialFound { .. })
    }

    /// Open the flow: the user reached it, from Advanced. Sends nothing.
    pub fn open(&mut self, s: &CoreVregStatus, now_ms: u64) -> Result<(), FlowError> {
        if self.phase.busy() {
            return Err(self.wrong_phase());
        }
        self.phase = Phase::Review;
        // A running trial becomes `TrialFound`. Not busy, so no request.
        let _ = self.on_status(s, now_ms);
        Ok(())
    }

    /// Close the flow: the user left. Sends nothing, and refused while a run
    /// is under way, since the trial would go on with no one watching. A trial
    /// on the device is found again by the next status.
    pub fn close(&mut self) -> Result<(), FlowError> {
        if self.phase.busy() {
            return Err(self.wrong_phase());
        }
        self.phase = Phase::Idle;
        Ok(())
    }

    /// Start a run: the user chose `target` and a soak. Returns the `CLEAR`
    /// to send; the `TRIAL` follows from [`Calibrator::on_status`] once the
    /// device shows it applied.
    pub fn start(
        &mut self,
        s: &CoreVregStatus,
        target: u8,
        soak_s: u32,
        typed: Option<&str>,
        now_ms: u64,
    ) -> Result<VregOp, FlowError> {
        if !self.can_start() {
            return Err(self.wrong_phase());
        }
        if s.trial_active {
            return Err(FlowError::TrialRunning);
        }
        if !in_flow_range(target, s.default_vreg) {
            return Err(FlowError::OutOfRange {
                code: target,
                default_vreg: s.default_vreg,
            });
        }
        if soak_s < MIN_SOAK_S {
            return Err(FlowError::SoakTooShort);
        }
        check_typed(target, typed)?;
        self.last_uptime_s = s.uptime_s;
        self.phase = Phase::Clearing {
            run: Run {
                target,
                soak_s,
                baseline_failed: 0,
            },
            deadline_ms: now_ms.saturating_add(VERIFY_TIMEOUT_MS),
        };
        Ok(VregOp::Clear)
    }

    /// A status was read. The only request this can produce is the `TRIAL`
    /// of a run the user started.
    pub fn on_status(&mut self, s: &CoreVregStatus, now_ms: u64) -> Option<FlowTrial> {
        let phase = self.phase;
        let mut trial = None;
        match phase {
            Phase::Idle | Phase::Review | Phase::Ended | Phase::Committed { .. } => {
                if s.trial_active {
                    self.phase = Phase::TrialFound { vreg: s.trial_vreg };
                }
            }
            Phase::Failed(_) | Phase::VerifyFailed { .. } | Phase::Unknown { .. } => {
                // A failure and a failed read-back stay on show; they are
                // what the user has to act on. The status says if a trial runs.
                if matches!(phase, Phase::Unknown { .. }) && s.trial_active {
                    self.phase = Phase::TrialFound { vreg: s.trial_vreg };
                }
            }
            Phase::Clearing { run, deadline_ms } => {
                if s.trial_active {
                    self.phase = Phase::TrialFound { vreg: s.trial_vreg };
                } else if s.hang_count == 0 && s.failed_trial_vreg == 0 {
                    if in_flow_range(run.target, s.default_vreg) {
                        trial = Some(FlowTrial(run.target));
                        self.phase = Phase::StartingTrial {
                            run,
                            start_uptime_s: s.uptime_s,
                            deadline_ms: now_ms.saturating_add(VERIFY_TIMEOUT_MS),
                        };
                    } else {
                        self.unknown(run.target, UnknownReason::NotStarted);
                    }
                } else if now_ms >= deadline_ms {
                    self.unknown(run.target, UnknownReason::NotStarted);
                }
            }
            Phase::StartingTrial {
                run,
                start_uptime_s,
                deadline_ms,
            } => match self.judge(&run, s) {
                Verdict::Running => {
                    self.phase = Phase::Soaking {
                        run,
                        start_uptime_s: s.uptime_s,
                    };
                }
                Verdict::Failed => {
                    self.phase = Phase::Failed(TrialFailure::of(s, Some(start_uptime_s)));
                }
                // Not applied yet, unless the device rebooted meanwhile.
                Verdict::Unknown(UnknownReason::RecordGone)
                    if s.uptime_s >= self.last_uptime_s && now_ms < deadline_ms => {}
                Verdict::Unknown(UnknownReason::RecordGone) => {
                    self.unknown(run.target, UnknownReason::NotStarted);
                }
                Verdict::Unknown(reason) => self.unknown(run.target, reason),
            },
            Phase::Soaking {
                run,
                start_uptime_s,
            }
            | Phase::Passed {
                run,
                start_uptime_s,
            } => match self.judge(&run, s) {
                Verdict::Running => {
                    if s.uptime_s.saturating_sub(start_uptime_s) >= run.soak_s {
                        self.phase = Phase::Passed {
                            run,
                            start_uptime_s,
                        };
                    }
                }
                Verdict::Failed => {
                    self.phase = Phase::Failed(TrialFailure::of(s, Some(start_uptime_s)));
                }
                Verdict::Unknown(reason) => self.unknown(run.target, reason),
            },
            Phase::TrialFound { .. } | Phase::TrialLeftRunning { .. } => {
                if !s.trial_active {
                    self.phase = Phase::Idle;
                } else if matches!(phase, Phase::TrialFound { .. }) {
                    self.phase = Phase::TrialFound { vreg: s.trial_vreg };
                }
            }
            Phase::Ending { deadline_ms } => {
                if !s.trial_active && s.live_vreg == resolve(s.stored_sel, s.default_vreg) {
                    self.phase = Phase::Ended;
                } else if now_ms >= deadline_ms {
                    self.phase = Phase::VerifyFailed { set_value: None };
                }
            }
            Phase::Committing { value, deadline_ms } => {
                if !s.trial_active
                    && s.stored_sel == value
                    && s.live_vreg == resolve(value, s.default_vreg)
                {
                    self.phase = Phase::Committed { value };
                } else if now_ms >= deadline_ms {
                    self.phase = Phase::VerifyFailed {
                        set_value: Some(value),
                    };
                }
            }
        }
        self.last_uptime_s = s.uptime_s;
        self.link_lost = false;
        trial
    }

    /// The device stopped answering. Sends nothing, now or on return: a run
    /// whose `TRIAL` has not gone out ends here, so a reconnect cannot send it.
    pub fn on_link_lost(&mut self) {
        self.link_lost = true;
        if let Phase::Clearing { run, .. } = self.phase {
            self.unknown(run.target, UnknownReason::NotStarted);
        }
    }

    /// A request of the run failed to send. Not retried. A `SET` or
    /// `TRIAL_END` may have reached the device all the same, so those still
    /// wait for the status to say.
    pub fn on_write_failed(&mut self) {
        if let Phase::Clearing { run, .. } | Phase::StartingTrial { run, .. } = self.phase {
            self.unknown(run.target, UnknownReason::NotStarted);
        }
    }

    /// Forget everything: the device changed, or this client may no longer
    /// reach the extension.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Keep a found trial. Within the flow's range the soak counts from now,
    /// since nothing says when the trial began. Otherwise it is left alone
    /// and cannot pass. Sends nothing.
    pub fn keep_trial(&mut self, s: &CoreVregStatus, soak_s: u32) -> Result<(), FlowError> {
        if !self.can_keep() {
            return Err(self.wrong_phase());
        }
        if !s.trial_active {
            self.phase = Phase::Idle;
            return Err(self.wrong_phase());
        }
        if !in_flow_range(s.trial_vreg, s.default_vreg) {
            self.phase = Phase::TrialLeftRunning { vreg: s.trial_vreg };
            return Ok(());
        }
        if soak_s < MIN_SOAK_S {
            return Err(FlowError::SoakTooShort);
        }
        self.phase = Phase::Soaking {
            run: Run {
                target: s.trial_vreg,
                soak_s,
                baseline_failed: s.failed_trial_vreg,
            },
            start_uptime_s: s.uptime_s,
        };
        self.last_uptime_s = s.uptime_s;
        Ok(())
    }

    /// Save the voltage that passed: the only way a run leads to a `SET`.
    pub fn commit(
        &mut self,
        s: &CoreVregStatus,
        typed: Option<&str>,
        now_ms: u64,
    ) -> Result<VregOp, FlowError> {
        let Phase::Passed { run, .. } = self.phase else {
            return Err(self.wrong_phase());
        };
        if !in_flow_range(run.target, s.default_vreg) {
            return Err(FlowError::OutOfRange {
                code: run.target,
                default_vreg: s.default_vreg,
            });
        }
        check_typed(run.target, typed)?;
        self.phase = Phase::Committing {
            value: run.target,
            deadline_ms: now_ms.saturating_add(VERIFY_TIMEOUT_MS),
        };
        Ok(VregOp::Set(run.target))
    }

    /// End a trial, or abort a run, at any time but during a `SET`.
    pub fn end_trial(&mut self, now_ms: u64) -> Result<VregOp, FlowError> {
        if matches!(self.phase, Phase::Committing { .. }) {
            return Err(self.wrong_phase());
        }
        self.phase = Phase::Ending {
            deadline_ms: now_ms.saturating_add(VERIFY_TIMEOUT_MS),
        };
        Ok(VregOp::TrialEnd)
    }

    /// Manual `SET` from the Advanced page: `0` resets to the build default,
    /// otherwise the flow's range and confirmation apply.
    pub fn manual_set(
        &mut self,
        s: &CoreVregStatus,
        value: u8,
        typed: Option<&str>,
        now_ms: u64,
    ) -> Result<VregOp, FlowError> {
        if self.phase.busy() {
            return Err(self.wrong_phase());
        }
        if value != 0 {
            if !in_flow_range(value, s.default_vreg) {
                return Err(FlowError::OutOfRange {
                    code: value,
                    default_vreg: s.default_vreg,
                });
            }
            check_typed(value, typed)?;
        }
        self.phase = Phase::Committing {
            value,
            deadline_ms: now_ms.saturating_add(VERIFY_TIMEOUT_MS),
        };
        Ok(VregOp::Set(value))
    }

    /// Manual `TRIAL`: the flow's range and confirmation apply. The next
    /// status shows it as [`Phase::TrialFound`].
    pub fn manual_trial(
        &mut self,
        s: &CoreVregStatus,
        value: u8,
        typed: Option<&str>,
    ) -> Result<VregOp, FlowError> {
        if self.phase.busy() {
            return Err(self.wrong_phase());
        }
        if !in_flow_range(value, s.default_vreg) {
            return Err(FlowError::OutOfRange {
                code: value,
                default_vreg: s.default_vreg,
            });
        }
        check_typed(value, typed)?;
        self.phase = Phase::Idle;
        Ok(VregOp::Trial(value))
    }

    /// Manual `CLEAR`; refused during a run, whose result it would erase.
    pub fn manual_clear(&mut self) -> Result<VregOp, FlowError> {
        if self.phase.busy() {
            return Err(self.wrong_phase());
        }
        Ok(VregOp::Clear)
    }

    fn wrong_phase(&self) -> FlowError {
        FlowError::WrongPhase {
            phase: self.phase.key(),
        }
    }

    fn unknown(&mut self, target: u8, reason: UnknownReason) {
        self.phase = Phase::Unknown { target, reason };
    }

    /// What a status says about a watched trial. The uptime going backwards
    /// is a reboot, whatever else the status shows.
    fn judge(&self, run: &Run, s: &CoreVregStatus) -> Verdict {
        let rebooted = s.uptime_s < self.last_uptime_s;
        // A record older than the watch (only a kept trial has one) counts
        // only if a reboot wrote it: a further warm reset zeroes its uptime.
        let new_failure = s.failed_trial_vreg != 0
            && (s.failed_trial_vreg != run.baseline_failed
                || (rebooted && s.failed_trial_uptime_s != 0));
        if new_failure {
            return if s.failed_trial_vreg == run.target {
                Verdict::Failed
            } else {
                Verdict::Unknown(UnknownReason::RecordGone)
            };
        }
        if rebooted || !s.trial_active {
            return Verdict::Unknown(UnknownReason::RecordGone);
        }
        if s.trial_vreg != run.target {
            return Verdict::Unknown(UnknownReason::TrialChanged);
        }
        Verdict::Running
    }
}

#[cfg(test)]
#[path = "vreg_calibrate/tests_wp_cg11.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg11;
