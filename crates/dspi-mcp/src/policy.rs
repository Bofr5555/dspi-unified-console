//! The agent policy (WP MCP2a; MPLAN 3.1 to 3.3, the owner's decisions of
//! 2026-11-21 and 2026-11-22).
//!
//! **One judgement.** [`judge`] takes the device's state and the writes a
//! call will make, and says whether they pass, need the user's word, or are
//! refused. It is the only place that knows what is risky. Two callers use
//! it:
//! - an edit tool judges its whole plan first ([`prejudge`]), so a call of
//!   several writes is held or refused before the first is sent, never
//!   halfway;
//! - the core's pre-write hook ([`AgentPolicy`]'s) judges every write the
//!   plan did not name: an undo's replay, a recall, a later tool that has no
//!   plan of its own. A write the plan named passes the hook, as the plan
//!   was judged whole (a high-pass moved from one slot to another is not a
//!   removal).
//!
//! **What is risky** (asked under the default setting):
//! - louder: a gain, trim or volume raised, an unmute, an output enabled, a
//!   new route or a route's gain raised;
//! - a PEQ boost above +[`PEQ_BOOST_STEP_DB`] dB that rose, or a Linkwitz
//!   transform changed (it adds bass);
//! - a delay step over [`DELAY_STEP_MS`] ms;
//! - lowering the protection of an output that has a high-pass at 1 kHz or
//!   above (it drives a tweeter, whatever the user called it): removing that
//!   high-pass, moving it below 1 kHz or making it shallower than 12 dB/oct;
//! - the static classes later tools name ([`require`]): test signals, a
//!   preset load, a flash write, an installation save.
//!
//! **The setting** ([`ConfirmMode`]): ask on everything, ask for risky changes
//! (the default), or never ask. "Never ask" never asks (owner 2026-11-22):
//! everything passes but a lowered tweeter protection, which is refused.
//!
//! **Refused whatever the setting** (the device's safety configuration, not
//! a question to the user): an output the user's confirmed speaker setup
//! calls a tweeter (or a midrange) keeps a high-pass at 1 kHz (80 Hz) or
//! above, 12 dB/oct or steeper. The core adds its own: the limits profile,
//! the installer lock, read-only modes, and `user_only` calls; [`hard_code`]
//! names those so no layer asks the user about them.
#![allow(clippy::module_name_repetitions)]

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dspi_core::actor::user::{self as user_changes, USER_CHANGED}; // -- WP AIVIS --
use dspi_core::actor::{NOT_OWN_ENTRY, USER_EDITING, USER_ONLY, WriteHook, WriteProposal};
use dspi_core::measure::MeasureRole;
use dspi_core::{DspState, DspiCore, DspiError, FilterParams, ParamValue};
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::xover;
use serde_json::{Value, json};

use crate::registry::{AGENT_READ_ONLY, CallInfo, Policy, ToolError};

/// A PEQ band may boost up to this without asking (MCPDES 3.3).
pub const PEQ_BOOST_STEP_DB: f32 = 6.0;
/// A delay may move by up to this without asking (MCPDES 3.3).
pub const DELAY_STEP_MS: f32 = 20.0;
/// A flat band (a free slot), where the state holds none.
pub const FLAT: FilterParams = FilterParams {
    filter_type: 0,
    bypass: false,
    freq: 1_000.0,
    q: 0.707,
    gain: 0.0,
    qp: 0.707,
};
/// A tweeter's high-pass corner at least (the measurement's S6 number).
pub const TWEETER_FLOOR_HZ: f32 = 1_000.0;
/// A midrange's high-pass corner at least (S6).
pub const MID_FLOOR_HZ: f32 = 80.0;
/// A protecting high-pass is at least this steep, dB/octave.
pub const MIN_SLOPE_DB: u8 = 12;
/// Corner frequencies this close count as equal (the core's slack).
const HZ_SLACK: f32 = 0.5;
/// A level change smaller than this is no change (dB).
const LOUDER_EPS_DB: f32 = 0.05;

/// [`DspiError::ConfirmationRequired`]'s code from the hook: the write needs
/// the user's word; the server holds the call for it.
pub const AGENT_CONFIRM: &str = "agent_confirm";
/// [`DspiError::ConfirmationRequired`]'s code from the hook: the policy
/// refuses the write whatever the user says.
pub const AGENT_REFUSED: &str = "agent_refused";

// ------------------------------------------------------------- the setting

/// When the user is asked (Settings › AI agents).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmMode {
    /// Every change.
    AskEverything,
    /// Risky changes only (the default).
    #[default]
    AskRisky,
    /// Never: everything the safety rules allow passes.
    NeverAsk,
}

impl ConfirmMode {
    /// The word for settings and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskEverything => "ask_everything",
            Self::AskRisky => "ask_risky",
            Self::NeverAsk => "never_ask",
        }
    }

    /// From [`ConfirmMode::as_str`]'s word.
    pub fn parse(word: &str) -> Option<Self> {
        [Self::AskEverything, Self::AskRisky, Self::NeverAsk]
            .into_iter()
            .find(|m| m.as_str() == word)
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::AskEverything => 0,
            Self::AskRisky => 1,
            Self::NeverAsk => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::AskEverything,
            2 => Self::NeverAsk,
            _ => Self::AskRisky,
        }
    }
}

// --------------------------------------------------------------- the words

/// Why a write needs the user's word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskKind {
    /// A gain, trim or volume raised.
    Louder,
    /// An output or the volume unmuted.
    Unmute,
    /// An output enabled.
    Enable,
    /// An input routed to an output that it did not feed.
    NewRoute,
    /// A PEQ boost above the step, or a Linkwitz transform.
    Boost,
    /// A delay moved by more than the step.
    DelayStep,
    /// A tweeter's high-pass protection lowered (an output with a high-pass
    /// at 1 kHz or above).
    TweeterCrossing,
    /// Any change, under "ask on everything".
    Change,
    /// Test signals (a measurement start). -- for MCP5 --
    TestSignal,
    /// A preset load or a wholesale recall. -- for MCP2b --
    Recall,
    /// A write to the device's flash. -- for MCP2b --
    FlashWrite,
    /// The installation saved (every unit). -- for MCP2c --
    InstallationSave,
    // -- WP MCP2b --
    /// The input source changed (another signal plays).
    Source,
    // -- WP MCP2c --
    /// A change on the hub that every unit of the installation hears: an
    /// alignment (each unit re-anchors), a scene recall, USB-direct modes.
    System,
}

impl RiskKind {
    /// The word in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Louder => "louder",
            Self::Unmute => "unmute",
            Self::Enable => "enable",
            Self::NewRoute => "new_route",
            Self::Boost => "boost",
            Self::DelayStep => "delay_step",
            Self::TweeterCrossing => "tweeter_crossing",
            Self::Change => "change",
            Self::TestSignal => "test_signal",
            Self::Recall => "recall",
            Self::FlashWrite => "flash_write",
            Self::InstallationSave => "installation_save",
            Self::Source => "source", // -- WP MCP2b --
            Self::System => "system", // -- WP MCP2c --
        }
    }
}

/// One value a call writes: a registry path, its indices and the new value
/// (as the core's hook shows it).
#[derive(Debug, Clone, PartialEq)]
pub struct WriteKey {
    /// `out.gain`, `eq.freq`, `mix`, ...
    pub path: String,
    /// The path's indices.
    pub indices: Vec<u8>,
    /// The value set.
    pub after: ParamValue,
}

impl WriteKey {
    /// `path` at `indices` set to `after`.
    pub fn new(path: &str, indices: &[u8], after: ParamValue) -> Self {
        Self {
            path: path.to_owned(),
            indices: indices.to_vec(),
            after,
        }
    }

    /// The value it names: a band's fields are one value.
    fn item(&self) -> (&str, &[u8]) {
        if self.path.starts_with("eq.") {
            ("eq", &self.indices)
        } else {
            (&self.path, &self.indices)
        }
    }
}

/// One thing the user is asked about.
#[derive(Debug, Clone, PartialEq)]
pub struct Risk {
    /// What kind.
    pub kind: RiskKind,
    /// In the user's words, for the confirmation sheet ("Out 3 gain from
    /// −6.0 to −2.0 dB"). Numbers only, never a name.
    pub what: String,
    /// The output it is about, from 0.
    pub output: Option<u8>,
    /// The writes that make it so; an Allow covers exactly these.
    pub keys: Vec<WriteKey>,
}

impl Risk {
    /// As JSON, for the app and the agent.
    pub fn to_json(&self) -> Value {
        json!({"kind": self.kind.as_str(), "what": self.what, "output": self.output})
    }
}

/// [`judge`]'s answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The writes may go.
    Pass,
    /// They need the user's word on these.
    Ask(Vec<Risk>),
    /// They are refused, whatever the user says; the text says why.
    Refuse(String),
}

// ------------------------------------------------------------- one call

/// What one tool call's judgement knows and found. The server makes one per
/// call (`registry::run`); the hook and the tool share it.
#[derive(Debug, Default, Clone)]
pub struct Judging {
    /// The setting, when the policy is [`AgentPolicy`]; `None` under another
    /// policy, which then judges in its own hook alone.
    pub mode: Option<ConfirmMode>,
    /// Each output's role from the user's **confirmed** speaker setup.
    pub roles: Vec<Option<MeasureRole>>,
    /// The writes the user allowed for this call (a re-run after Allow).
    pub grant: Vec<WriteKey>,
    /// The values the tool's judged plan writes: the hook lets these pass.
    pub plan: Vec<(String, Vec<u8>)>,
    /// What the user must be asked (the call is then held).
    pub asked: Vec<Risk>,
    /// The policy refused (the text).
    pub refused: Option<String>,
    /// The core refused because the user is dragging a control.
    pub editing: bool,
    /// The guarded apply the call began (`dspi_apply_changes`).
    pub guard_begun: Option<u64>,
    /// The guarded apply the call ended (keep or revert).
    pub guard_ended: Option<u64>,
    // -- WP MCP2b --
    /// A wholesale write the user allowed as a whole (a recall, a preset
    /// load, an import: [`require`]'s static class): the hook asks nothing
    /// more of it, but still refuses, and refuses a lowered tweeter
    /// protection too ([`umbrella`]).
    pub wholesale: bool,
    // -- WP AIVIS --
    /// The device and the actor, for the rule on what the user took back.
    pub taken_back: TakenBack,
}

// -- WP AIVIS --
/// What [`prejudge`] asks the core before a plan's first write: has the user
/// taken one of its controls back from this agent (`dspi_core::actor::user`)?
/// The core's own gate asks the same at each write.
#[derive(Clone, Default)]
pub struct TakenBack(pub Option<(Arc<DspiCore>, dspi_core::actor::ActorId)>);

impl std::fmt::Debug for TakenBack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TakenBack({})", self.0.is_some())
    }
}

/// The agent's answer when the user took a control back.
pub const USER_CHANGED_TEXT: &str = "the user changed this after you set it, so nothing was \
     changed: ask the user before changing it again (if they have just asked you for it, call \
     again in 5 s). dspi_get_user_activity says what they did.";

/// A call's [`Judging`], shared by the tool and the hook.
pub type JudgingCell = Arc<Mutex<Judging>>;

/// Lock a [`JudgingCell`] (a poisoned lock still holds the facts).
pub fn lock(cell: &JudgingCell) -> MutexGuard<'_, Judging> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Judging {
    fn planned(&self, w: &WriteKey) -> bool {
        let (path, ix) = w.item();
        self.plan.iter().any(|(p, i)| {
            let p = if p.starts_with("eq.") {
                "eq"
            } else {
                p.as_str()
            };
            p == path && i.as_slice() == ix
        })
    }
}

/// Judge a tool's whole plan before anything is sent. `Ok` lets it go and
/// marks its writes for the hook; an error is the tool's answer: a refusal,
/// or "held for the user" with [`Judging::asked`] filled for the server.
pub fn prejudge(
    cell: &JudgingCell,
    state: &DspState,
    writes: &[WriteKey],
) -> Result<(), ToolError> {
    let verdict = judge(state, writes, &lock(cell));
    // -- WP AIVIS -- a control the user took back, before anything of the plan
    // is sent (what is held for the user's word is asked there)
    let taken_back = lock(cell).taken_back.clone();
    if verdict == Verdict::Pass
        && let Some((core, actor)) = taken_back.0
    {
        let writes = writes
            .iter()
            .map(|w| (w.path.as_str(), w.indices.as_slice()));
        user_changes::check_plan(&core, actor, writes).map_err(|e| core_error(cell, e))?;
    }
    let mut j = lock(cell);
    match verdict {
        Verdict::Pass => {
            j.plan
                .extend(writes.iter().map(|w| (w.path.clone(), w.indices.clone())));
            Ok(())
        }
        Verdict::Ask(risks) => {
            j.asked.extend(risks);
            Err(ToolError::new(HELD_TEXT))
        }
        Verdict::Refuse(why) => {
            j.refused = Some(why.clone());
            Err(ToolError::new(why))
        }
    }
}

/// A held call's text when the transport cannot hold it (replaced by the
/// server).
const HELD_TEXT: &str = "this change needs the user's confirmation in the app";

/// A static class for a write the hook cannot see (a preset load, test
/// signals, an installation save, a flash write): asked under "ask on
/// everything" and "ask for risky changes", passed under "never ask",
/// refused under a policy that is not [`AgentPolicy`]. `what` is the user's
/// words; the grant key is `static.<kind>` with `indices`.
pub fn require(
    cell: &JudgingCell,
    kind: RiskKind,
    what: &str,
    indices: &[u8],
) -> Result<(), ToolError> {
    let mut j = lock(cell);
    let Some(mode) = j.mode else {
        return Err(ToolError::new(
            "the server's policy does not let agents do this",
        ));
    };
    let key = WriteKey::new(
        &format!("static.{}", kind.as_str()),
        indices,
        ParamValue::Trigger,
    );
    if mode == ConfirmMode::NeverAsk || j.grant.contains(&key) {
        return Ok(());
    }
    j.asked.push(Risk {
        kind,
        what: what.to_owned(),
        output: None,
        keys: vec![key],
    });
    Err(ToolError::new(HELD_TEXT))
}

// -- WP MCP2b --
/// A static class for a quiet write the hook cannot see (a crossfeed pair
/// map, a snapshot's name): asked under "ask on everything" only, refused
/// under a policy that is not [`AgentPolicy`]. The grant key is
/// `static.change` with `indices`.
pub fn require_change(cell: &JudgingCell, what: &str, indices: &[u8]) -> Result<(), ToolError> {
    let mut j = lock(cell);
    let Some(mode) = j.mode else {
        return Err(ToolError::new(
            "the server's policy does not let agents do this",
        ));
    };
    let key = WriteKey::new("static.change", indices, ParamValue::Trigger);
    if mode != ConfirmMode::AskEverything || j.grant.contains(&key) {
        return Ok(());
    }
    j.asked.push(Risk {
        kind: RiskKind::Change,
        what: what.to_owned(),
        output: None,
        keys: vec![key],
    });
    Err(ToolError::new(HELD_TEXT))
}

/// A wholesale write's verdict once the user allowed it whole
/// ([`Judging::wholesale`]): what it would ask passes, but a lowered tweeter
/// protection is refused, as the user was not told of it. -- WP MCP2b --
pub fn umbrella(verdict: Verdict) -> Verdict {
    match verdict {
        Verdict::Ask(risks) => match risks.iter().find(|r| r.kind == RiskKind::TweeterCrossing) {
            Some(r) => Verdict::Refuse(format!(
                "{}: a wholesale change (a recall, a preset load, an import) may not remove or \
                 lower a tweeter's high-pass; the user can do it in the app.",
                r.output.map_or_else(|| "an output".to_owned(), out_word)
            )),
            None => Verdict::Pass,
        },
        other => other,
    }
}

/// Whether `code` (a [`DspiError::ConfirmationRequired`]'s) is a refusal no
/// confirmation can lift: never retried, never asked (review RA, MCP0 item
/// 7). `user_editing` is not one (retry), nor is [`AGENT_CONFIRM`] (asked).
pub fn hard_code(code: &str) -> bool {
    matches!(
        code,
        USER_ONLY | AGENT_READ_ONLY | NOT_OWN_ENTRY | AGENT_REFUSED
    )
}

/// A core error as the agent reads it, noting in `cell` what the server
/// acts on: a drag (retry, with the server's staleness bound) and a hold.
pub fn core_error(cell: &JudgingCell, e: DspiError) -> ToolError {
    match &e {
        DspiError::ConfirmationRequired { code, .. } if code == USER_EDITING => {
            lock(cell).editing = true;
            ToolError::new(
                "the user is editing a control on this device; nothing was changed. \
                 Try again in a second or two.",
            )
        }
        // -- WP AIVIS --
        DspiError::ConfirmationRequired { code, .. } if code == USER_CHANGED => {
            ToolError::new(USER_CHANGED_TEXT)
        }
        DspiError::ConfirmationRequired { code, .. } if code == AGENT_CONFIRM => {
            ToolError::new(HELD_TEXT)
        }
        DspiError::ConfirmationRequired { code, msg } if code == AGENT_REFUSED => {
            ToolError::new(crate::untrusted::note(msg))
        }
        DspiError::ConfirmationRequired { code, .. } if code == USER_ONLY => ToolError::new(
            "only the user may do that, in the app; an agent never can. Do not retry.",
        ),
        DspiError::ConfirmationRequired { code, .. } if code == NOT_OWN_ENTRY => ToolError::new(
            "that change is not this agent's; only the user can undo it. Do not retry.",
        ),
        DspiError::ConfirmationRequired { code, .. } if code == AGENT_READ_ONLY => {
            ToolError::new("agents may only read on this computer. Do not retry.")
        }
        DspiError::LimitExceeded { .. } => ToolError::new(format!(
            "{} This is the installer's limits profile: it is not asked about and an \
             agent cannot lift it.",
            ToolError::from(e).0
        )),
        DspiError::InstallerLock { .. } => ToolError::new(
            "the installer lock is on: ask the user to unlock in the app. Nothing was changed.",
        ),
        _ => ToolError::from(e),
    }
}

// ----------------------------------------------------------- the judgement

/// Judge `writes` on `state` under `j`'s setting, roles and grant (module
/// docs). Writes that change nothing count for nothing.
pub fn judge(state: &DspState, writes: &[WriteKey], j: &Judging) -> Verdict {
    let Some(mode) = j.mode else {
        return Verdict::Pass;
    };
    let mut after = state.clone();
    for w in writes {
        apply(&mut after, w);
    }
    if after == *state && writes.iter().all(|w| weighed(&w.path)) {
        return Verdict::Pass;
    }
    let mut risks = Vec::new();
    let mut refusals = Vec::new();
    judge_values(state, &after, writes, &mut risks);
    judge_highpass(state, &after, writes, &j.roles, &mut risks, &mut refusals);
    judge_effects(state, &after, writes, &mut risks); // -- WP MCP2b --
    if let Some(why) = refusals.into_iter().next() {
        return Verdict::Refuse(why);
    }
    if mode == ConfirmMode::AskEverything && risks.is_empty() {
        risks.push(Risk {
            kind: RiskKind::Change,
            what: summary(state, writes),
            output: None,
            keys: writes.to_vec(),
        });
    }
    if mode == ConfirmMode::NeverAsk {
        if let Some(r) = risks.iter().find(|r| r.kind == RiskKind::TweeterCrossing) {
            return Verdict::Refuse(format!(
                "{}. Under \"never ask\" a change that lowers a tweeter's protection is \
                 refused; the user can make it in the app.",
                r.what
            ));
        }
        return Verdict::Pass;
    }
    risks.retain(|r| !r.keys.iter().all(|k| j.grant.contains(k)));
    if risks.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Ask(risks)
    }
}

/// A path [`apply`] puts into the state, so the judgement weighs it. Any
/// other is a change all the same, asked about under "ask on everything"
/// (MCP2b adds the effects' rules).
fn weighed(path: &str) -> bool {
    fx_weighed(path) // -- WP MCP2b --
        || path.starts_with("eq.")
        || matches!(
            path,
            "vol.user"
                | "vol.master"
                | "vol.mute"
                | "pre"
                | "pre.all"
                | "bypass"
                | "out.gain"
                | "out.delay"
                | "out.mute"
                | "out.enable"
                | "ch.delay"
                | "mix"
        )
}

/// "Out 3" or "In 1" for wire channel `ch`.
fn channel_word(st: &DspState, ch: u8) -> String {
    match ch.checked_sub(st.num_input_channels) {
        Some(o) => format!("Out {}", u16::from(o).saturating_add(1)),
        None => format!("In {}", u16::from(ch).saturating_add(1)),
    }
}

fn out_word(o: u8) -> String {
    format!("Out {}", u16::from(o).saturating_add(1))
}

fn in_word(i: u8) -> String {
    format!("In {}", u16::from(i).saturating_add(1))
}

/// A short account of `writes` for "ask on everything".
fn summary(st: &DspState, writes: &[WriteKey]) -> String {
    let mut items: Vec<String> = Vec::new();
    for w in writes {
        let ch = w.indices.first().copied().unwrap_or(0);
        let word = match w.item().0 {
            "eq" => format!("{} filters", channel_word(st, ch)),
            "out.gain" => format!("{} gain", out_word(ch)),
            "out.mute" => format!("{} mute", out_word(ch)),
            "out.enable" => format!("{} on/off", out_word(ch)),
            "out.delay" | "ch.delay" => format!("{} delay", channel_word(st, w_ch(st, w))),
            "mix" => format!(
                "routing {} to {}",
                in_word(ch),
                out_word(w.indices.get(1).copied().unwrap_or(0))
            ),
            "vol.user" | "vol.mute" => "volume".to_owned(),
            "vol.master" => "master volume".to_owned(),
            "pre" | "pre.all" => "input trim".to_owned(),
            "bypass" => "EQ bypass".to_owned(),
            other => other.to_owned(),
        };
        if !items.contains(&word) {
            items.push(word);
        }
    }
    format!("change {}", items.join(", "))
}

/// The wire channel a delay write is about.
fn w_ch(st: &DspState, w: &WriteKey) -> u8 {
    let i = w.indices.first().copied().unwrap_or(0);
    if w.path == "out.delay" {
        st.num_input_channels.saturating_add(i)
    } else {
        i
    }
}

fn float(v: &ParamValue) -> Option<f32> {
    match v {
        ParamValue::Float(f) => Some(*f),
        _ => None,
    }
}

fn boolean(v: &ParamValue) -> Option<bool> {
    match v {
        ParamValue::Bool(b) => Some(*b),
        _ => None,
    }
}

fn set<T: Copy>(v: &mut [T], i: u8, x: T) {
    if let Some(slot) = v.get_mut(usize::from(i)) {
        *slot = x;
    }
}

fn get<T: Copy + Default>(v: &[T], i: u8) -> T {
    v.get(usize::from(i)).copied().unwrap_or_default()
}

fn cell_of<T: Copy + Default>(v: &[Vec<T>], i: u8, o: u8) -> T {
    v.get(usize::from(i))
        .and_then(|r| r.get(usize::from(o)))
        .copied()
        .unwrap_or_default()
}

fn set_cell<T: Copy>(v: &mut [Vec<T>], i: u8, o: u8, x: T) {
    if let Some(slot) = v
        .get_mut(usize::from(i))
        .and_then(|r| r.get_mut(usize::from(o)))
    {
        *slot = x;
    }
}

/// Band `band` of wire channel `ch`: PEQ 0..9, crossover 20..23.
pub fn band_of(st: &DspState, ch: u8, band: u8) -> Option<&FilterParams> {
    let ch = usize::from(ch);
    match band.checked_sub(20) {
        Some(x) => st.xover.get(ch)?.get(usize::from(x)),
        None => st.filters.get(ch)?.get(usize::from(band)),
    }
}

fn band_mut(st: &mut DspState, ch: u8, band: u8) -> Option<&mut FilterParams> {
    let ch = usize::from(ch);
    match band.checked_sub(20) {
        Some(x) => st.xover.get_mut(ch)?.get_mut(usize::from(x)),
        None => st.filters.get_mut(ch)?.get_mut(usize::from(band)),
    }
}

/// The journal's whole-band bytes (`eq.band`, in a history entry): type,
/// bypass, then freq, Q, gain and qp as little-endian `f32`s.
/// -- WP AIVIS -- also for the user's changes in words.
pub(crate) fn band_of_bytes(b: &[u8]) -> Option<FilterParams> {
    band_from_bytes(b)
}

fn band_from_bytes(b: &[u8]) -> Option<FilterParams> {
    let f = |k: usize| -> Option<f32> {
        let at = k.checked_mul(4)?.checked_add(2)?;
        Some(f32::from_le_bytes(
            b.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    (b.len() == 18).then_some(())?;
    Some(FilterParams {
        filter_type: *b.first()?,
        bypass: *b.get(1)? != 0,
        freq: f(0)?,
        q: f(1)?,
        gain: f(2)?,
        qp: f(3)?,
    })
}

/// Put `w` into `st`, as the device would hold it. Paths the judgement does
/// not weigh are left out.
pub fn apply(st: &mut DspState, w: &WriteKey) {
    let i0 = w.indices.first().copied().unwrap_or(0);
    let i1 = w.indices.get(1).copied().unwrap_or(0);
    let f = float(&w.after);
    let b = boolean(&w.after);
    match (w.path.as_str(), f, b) {
        ("vol.user", Some(x), _) => st.user_volume_db = x,
        ("vol.master", Some(x), _) => st.master_volume_db = x,
        ("vol.mute", _, Some(x)) => st.user_mute = x,
        ("pre", Some(x), _) => set(&mut st.input_preamp_db, i0, x),
        ("pre.all", Some(x), _) => st.preamp_db = x,
        ("bypass", _, Some(x)) => st.bypass = x,
        ("out.gain", Some(x), _) => set(&mut st.output_gain_db, i0, x),
        ("out.delay", Some(x), _) => set(&mut st.output_delay_ms, i0, x),
        ("out.mute", _, Some(x)) => set(&mut st.output_muted, i0, x),
        ("out.enable", _, Some(x)) => set(&mut st.output_enabled, i0, x),
        ("ch.delay", Some(x), _) => set(&mut st.channel_delays, i0, x),
        ("mix", _, _) => {
            if let ParamValue::Bytes(bytes) = &w.after
                && let Ok(p) = MatrixRoutePacket::decode(bytes)
            {
                set_cell(&mut st.matrix_routing, i0, i1, p.enabled);
                set_cell(&mut st.matrix_gain, i0, i1, p.gain_db);
                set_cell(&mut st.matrix_invert, i0, i1, p.phase_invert);
            }
        }
        (path, _, _) if path.starts_with("eq.") => {
            let Some(band) = band_mut(st, i0, i1) else {
                return;
            };
            match (path, &w.after) {
                ("eq.type", ParamValue::Choice(t)) => band.filter_type = *t,
                ("eq.bypass", ParamValue::Bool(x)) => band.bypass = *x,
                ("eq.freq", ParamValue::Float(x)) => band.freq = *x,
                ("eq.q", ParamValue::Float(x)) => band.q = *x,
                ("eq.gain" | "eq.fp", ParamValue::Float(x)) => band.gain = *x,
                ("eq.qp", ParamValue::Float(x)) => band.qp = *x,
                ("eq.band", ParamValue::Bytes(bytes)) => {
                    if let Some(p) = band_from_bytes(bytes) {
                        *band = p;
                    }
                }
                _ => {}
            }
        }
        _ => apply_fx(st, w), // -- WP MCP2b --
    }
}

/// A risk from the writes in `keys` matching `item`.
fn keys_of(writes: &[WriteKey], path: &str, ix: &[u8]) -> Vec<WriteKey> {
    writes
        .iter()
        .filter(|w| w.item() == (path, ix))
        .cloned()
        .collect()
}

fn db(x: f32) -> String {
    format!("{x:+.1} dB")
}

/// Louder, unmute, enable, routes, PEQ boosts and delay steps.
#[allow(clippy::too_many_lines)] // one arm per kind of value
fn judge_values(before: &DspState, after: &DspState, writes: &[WriteKey], risks: &mut Vec<Risk>) {
    let mut seen: Vec<(String, Vec<u8>)> = Vec::new();
    for w in writes {
        let (path, ix) = w.item();
        if seen.iter().any(|(p, i)| p == path && i.as_slice() == ix) {
            continue;
        }
        seen.push((path.to_owned(), ix.to_vec()));
        let i0 = ix.first().copied().unwrap_or(0);
        let i1 = ix.get(1).copied().unwrap_or(0);
        let keys = || keys_of(writes, path, ix);
        let mut push = |kind, what: String, output| {
            risks.push(Risk {
                kind,
                what,
                output,
                keys: keys(),
            });
        };
        let louder = |a: f32, b: f32| b > a + LOUDER_EPS_DB;
        match path {
            "vol.user" | "vol.master" | "pre.all" => {
                let (a, b, word) = match path {
                    "vol.user" => (before.user_volume_db, after.user_volume_db, "Volume"),
                    "vol.master" => (
                        before.master_volume_db,
                        after.master_volume_db,
                        "Master volume",
                    ),
                    _ => (before.preamp_db, after.preamp_db, "Input trim (all)"),
                };
                if louder(a, b) {
                    push(
                        RiskKind::Louder,
                        format!("{word} up from {} to {}", db(a), db(b)),
                        None,
                    );
                }
            }
            "pre" => {
                let (a, b) = (
                    get(&before.input_preamp_db, i0),
                    get(&after.input_preamp_db, i0),
                );
                if louder(a, b) {
                    push(
                        RiskKind::Louder,
                        format!("{} trim up from {} to {}", in_word(i0), db(a), db(b)),
                        None,
                    );
                }
            }
            "out.gain" => {
                let (a, b) = (
                    get(&before.output_gain_db, i0),
                    get(&after.output_gain_db, i0),
                );
                if louder(a, b) {
                    push(
                        RiskKind::Louder,
                        format!("{} gain up from {} to {}", out_word(i0), db(a), db(b)),
                        Some(i0),
                    );
                }
            }
            "vol.mute" if before.user_mute && !after.user_mute => {
                push(RiskKind::Unmute, "Unmute the volume".to_owned(), None);
            }
            "out.mute" if get(&before.output_muted, i0) && !get(&after.output_muted, i0) => {
                push(
                    RiskKind::Unmute,
                    format!("Unmute {}", out_word(i0)),
                    Some(i0),
                );
            }
            "out.enable" if !get(&before.output_enabled, i0) && get(&after.output_enabled, i0) => {
                push(
                    RiskKind::Enable,
                    format!("Turn on {}", out_word(i0)),
                    Some(i0),
                );
            }
            "mix" => {
                let on = (
                    cell_of(&before.matrix_routing, i0, i1),
                    cell_of(&after.matrix_routing, i0, i1),
                );
                let (a, b) = (
                    cell_of(&before.matrix_gain, i0, i1),
                    cell_of(&after.matrix_gain, i0, i1),
                );
                if !on.0 && on.1 {
                    push(
                        RiskKind::NewRoute,
                        format!("Route {} to {} at {}", in_word(i0), out_word(i1), db(b)),
                        Some(i1),
                    );
                } else if on.1 && louder(a, b) {
                    push(
                        RiskKind::Louder,
                        format!(
                            "Route {} to {} up from {} to {}",
                            in_word(i0),
                            out_word(i1),
                            db(a),
                            db(b)
                        ),
                        Some(i1),
                    );
                }
            }
            "out.delay" | "ch.delay" => {
                let (a, b) = if path == "out.delay" {
                    (
                        get(&before.output_delay_ms, i0),
                        get(&after.output_delay_ms, i0),
                    )
                } else {
                    (
                        get(&before.channel_delays, i0),
                        get(&after.channel_delays, i0),
                    )
                };
                if (b - a).abs() > DELAY_STEP_MS {
                    let ch = w_ch(before, w);
                    push(
                        RiskKind::DelayStep,
                        format!(
                            "{} delay from {a:.2} to {b:.2} ms (a step over {DELAY_STEP_MS} ms)",
                            channel_word(before, ch)
                        ),
                        ch.checked_sub(before.num_input_channels),
                    );
                }
            }
            "eq" if i1 < 20 => {
                let was = band_of(before, i0, i1).copied().unwrap_or(FLAT);
                let now = band_of(after, i0, i1).copied().unwrap_or(FLAT);
                let output = i0.checked_sub(before.num_input_channels);
                let slot = u16::from(i1).saturating_add(1);
                if now.filter_type == 11 && !now.bypass && now != was {
                    push(
                        RiskKind::Boost,
                        format!(
                            "{} EQ band {slot}: a Linkwitz transform (adds bass)",
                            channel_word(before, i0)
                        ),
                        output,
                    );
                } else if boost(&now) > PEQ_BOOST_STEP_DB
                    && boost(&now) > boost(&was) + LOUDER_EPS_DB
                {
                    push(
                        RiskKind::Boost,
                        format!(
                            "{} EQ band {slot}: a boost of {} (above {})",
                            channel_word(before, i0),
                            db(boost(&now)),
                            db(PEQ_BOOST_STEP_DB)
                        ),
                        output,
                    );
                }
            }
            _ => {}
        }
    }
}

/// A band's boost in dB (0 for a cut, a filter that does not boost, or a
/// bypassed band).
fn boost(p: &FilterParams) -> f32 {
    let boosting = matches!(p.filter_type, 1 | 2 | 3 | 9 | 10);
    if boosting && !p.bypass && p.gain.is_finite() {
        p.gain.max(0.0)
    } else {
        0.0
    }
}

/// A high-pass type's slope in dB/octave; `None` for any other type.
pub fn highpass_slope(filter_type: u8) -> Option<u8> {
    match filter_type {
        5 => Some(12),
        13 => Some(6),
        t => xover::meta(t)
            .filter(|m| m.high_pass)
            .map(|m| m.order.saturating_mul(6)),
    }
}

/// Output `o`'s active high-passes: (corner, slope), PEQ and crossover.
fn highpasses(st: &DspState, o: u8) -> Vec<(f32, u8)> {
    let ch = usize::from(st.num_input_channels.saturating_add(o));
    let peq = st.filters.get(ch).map(Vec::as_slice).unwrap_or(&[]);
    let xo = st.xover.get(ch).map(Vec::as_slice).unwrap_or(&[]);
    peq.iter()
        .chain(xo)
        .filter(|b| !b.bypass && b.freq.is_finite())
        .filter_map(|b| highpass_slope(b.filter_type).map(|s| (b.freq, s)))
        .collect()
}

/// Whether output `o` has a high-pass at `floor` or above, steep enough.
fn protected(st: &DspState, o: u8, floor: f32) -> bool {
    highpasses(st, o)
        .iter()
        .any(|(hz, s)| *hz >= floor - HZ_SLACK && *s >= MIN_SLOPE_DB)
}

/// Output `o`'s highest steep-enough high-pass corner, 0 for none.
fn best_highpass(st: &DspState, o: u8) -> f32 {
    highpasses(st, o)
        .iter()
        .filter(|(_, s)| *s >= MIN_SLOPE_DB)
        .map(|(hz, _)| *hz)
        .fold(0.0, f32::max)
}

/// The tweeter rule on every output whose filters the writes touch.
fn judge_highpass(
    before: &DspState,
    after: &DspState,
    writes: &[WriteKey],
    roles: &[Option<MeasureRole>],
    risks: &mut Vec<Risk>,
    refusals: &mut Vec<String>,
) {
    let mut outputs: Vec<u8> = writes
        .iter()
        .filter(|w| w.item().0 == "eq")
        .filter_map(|w| w.indices.first()?.checked_sub(before.num_input_channels))
        .filter(|o| *o < before.num_output_channels)
        .collect();
    outputs.sort_unstable();
    outputs.dedup();
    for o in outputs {
        let role = roles.get(usize::from(o)).copied().flatten();
        // The user's word holds hard; a high-pass at 1 kHz or above shows a
        // tweeter whatever the user called it (the effective role).
        let inferred = highpasses(before, o)
            .iter()
            .any(|(hz, _)| *hz >= TWEETER_FLOOR_HZ - HZ_SLACK);
        let mut rules: Vec<(f32, bool)> = Vec::new();
        match role {
            Some(MeasureRole::High) => rules.push((TWEETER_FLOOR_HZ, true)),
            Some(MeasureRole::Mid) => rules.push((MID_FLOOR_HZ, true)),
            _ => {}
        }
        if inferred && role != Some(MeasureRole::High) {
            rules.push((TWEETER_FLOOR_HZ, false));
        }
        let ch = before.num_input_channels.saturating_add(o);
        for (floor, hard) in rules {
            let lowered = !protected(after, o, floor)
                && (protected(before, o, floor)
                    || best_highpass(after, o) < best_highpass(before, o) - HZ_SLACK);
            if !lowered {
                continue;
            }
            highpass_verdict(o, ch, floor, hard, writes, risks, refusals);
        }
    }
}

/// A lowered high-pass protection on output `o` (wire channel `ch`): refused
/// by the user's setup (`hard`), else asked.
fn highpass_verdict(
    o: u8,
    ch: u8,
    floor: f32,
    hard: bool,
    writes: &[WriteKey],
    risks: &mut Vec<Risk>,
    refusals: &mut Vec<String>,
) {
    if hard {
        let kind = if floor >= TWEETER_FLOOR_HZ {
            "a tweeter"
        } else {
            "a midrange"
        };
        refusals.push(format!(
            "{} drives {kind} in the user's speaker setup: its high-pass must stay at \
                 {floor} Hz or above, {MIN_SLOPE_DB} dB/octave or steeper. This is the device's \
                 safety rule, not a question: only the user can change the setup, in the app. \
                 Nothing was changed.",
            out_word(o)
        ));
    } else {
        risks.push(Risk {
            kind: RiskKind::TweeterCrossing,
            what: format!(
                "{} has a high-pass at {TWEETER_FLOOR_HZ} Hz or above (a tweeter): this \
                     removes it, moves it lower or makes it shallower than {MIN_SLOPE_DB} dB/oct",
                out_word(o)
            ),
            output: Some(o),
            keys: writes
                .iter()
                .filter(|w| w.item().0 == "eq" && w.indices.first() == Some(&ch))
                .cloned()
                .collect(),
        });
    }
}

// -- WP MCP2b --
// ------------------------------------------------------------- the effects

/// The effect values [`apply_fx`] models: the ones that can add level, and
/// the limiter that holds it.
const FX_WEIGHED: &[&str] = &[
    "loud.on",
    "loud.ref",
    "loud.intensity",
    "lev.on",
    "lev.maxgain",
    "bass.on",
    "bass.harmonics",
    "bass.drive",
    "bass.original",
    "sub.on",
    "sub.low",
    "sub.high",
    "sub.top",
    "sub.boost",
    "sub.ceiling",
    "tube.on",
    "tube.drive",
    "tube.trim",
    "limit.on",
    "limit.threshold",
    // -- WP MCP2c -- the outputs an effect plays on (review RB E2), and the
    // upmixer's switch (E3)
    "loud.mask",
    "lev.masks",
    "bass.mask",
    "sub.mask",
    "tube.mask",
    "up.on",
];

fn fx_weighed(path: &str) -> bool {
    FX_WEIGHED.contains(&path)
}

/// A value as a number, whatever its kind.
fn number(v: &ParamValue) -> Option<f32> {
    match v {
        ParamValue::Float(f) => Some(*f),
        ParamValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        ParamValue::Int(i) => Some(*i as f32),
        ParamValue::Choice(c) => Some(f32::from(*c)),
        ParamValue::Mask(m) => Some(*m as f32),
        _ => None,
    }
}

/// The subharmonic synth's and the tube's values by parameter id (their
/// paths carry no index).
fn fx_slot(path: &str) -> Option<(bool, u8)> {
    Some(match path {
        "sub.on" => (true, 0),
        "sub.low" => (true, 2),
        "sub.high" => (true, 3),
        "sub.top" => (true, 4),
        "sub.boost" => (true, 5),
        "sub.ceiling" => (true, 9),
        "sub.mask" => (true, 1), // -- WP MCP2c --
        "tube.on" => (false, 0),
        "tube.mask" => (false, 1), // -- WP MCP2c --
        "tube.drive" => (false, 3),
        "tube.trim" => (false, 13),
        _ => return None,
    })
}

/// Put an effect write into `st` (the paths in [`FX_WEIGHED`]). A limiter
/// write to output 0xFF is every output's.
fn apply_fx(st: &mut DspState, w: &WriteKey) {
    let Some(x) = number(&w.after) else {
        return;
    };
    let on = x != 0.0;
    match w.path.as_str() {
        "loud.on" => st.loudness_enabled = on,
        "loud.ref" => st.loudness_ref_spl = x,
        "loud.intensity" => st.loudness_intensity = x,
        "lev.on" => st.leveller_enabled = on,
        "lev.maxgain" => st.leveller_max_gain_db = x,
        "bass.on" => st.psybass_enabled = on,
        "bass.harmonics" => st.psybass_harmonics_db = x,
        "bass.drive" => st.psybass_drive_db = x,
        "bass.original" => st.psybass_original_db = x,
        // -- WP MCP2c --
        "loud.mask" => st.loudness_output_mask = x as u16,
        "bass.mask" => st.psybass_output_mask = x as u16,
        "lev.masks" => {
            // Detector in the low byte, apply in the high one (write_param's).
            let [det, app, ..] = (x as u32).to_le_bytes();
            st.leveller_detector_mask = det;
            st.leveller_apply_mask = app;
        }
        "up.on" => set(&mut st.upmix, 0, if on { 1.0 } else { 0.0 }),
        "limit.on" | "limit.threshold" => {
            let o = w.indices.first().copied().unwrap_or(0);
            let n = st.num_output_channels;
            let outs: Vec<u8> = if o == 0xFF { (0..n).collect() } else { vec![o] };
            for o in outs {
                if w.path == "limit.on" {
                    set(&mut st.limiter_enabled, o, on);
                } else {
                    set(&mut st.limiter_threshold_db, o, x);
                }
            }
        }
        path => {
            if let Some((sub, i)) = fx_slot(path) {
                let v = if sub { &mut st.subharm } else { &mut st.tube };
                set(v, i, x);
            }
        }
    }
}

/// One effect's switch and levels, before and after: (path prefix, word,
/// on before, on after, [(level, before, after)]).
type FxLevels<'a> = (&'a str, &'a str, bool, bool, Vec<(&'a str, f32, f32)>);

/// The effects that can add level: turned on, or a level raised while on;
/// a limiter turned off or its ceiling raised.
fn judge_effects(before: &DspState, after: &DspState, writes: &[WriteKey], risks: &mut Vec<Risk>) {
    let touched = |prefix: &str| writes.iter().any(|w| w.path.starts_with(prefix));
    let keys = |prefix: &str| -> Vec<WriteKey> {
        writes
            .iter()
            .filter(|w| w.path.starts_with(prefix))
            .cloned()
            .collect()
    };
    let louder = |a: f32, b: f32| b > a + LOUDER_EPS_DB;
    let fx: [FxLevels<'_>; 5] = [
        (
            "loud.",
            "Loudness",
            before.loudness_enabled,
            after.loudness_enabled,
            vec![
                ("reference", before.loudness_ref_spl, after.loudness_ref_spl),
                (
                    "intensity",
                    before.loudness_intensity,
                    after.loudness_intensity,
                ),
            ],
        ),
        (
            "lev.",
            "Leveller",
            before.leveller_enabled && before.leveller_max_gain_db > 0.0,
            after.leveller_enabled && after.leveller_max_gain_db > 0.0,
            vec![(
                "max gain",
                before.leveller_max_gain_db,
                after.leveller_max_gain_db,
            )],
        ),
        (
            "bass.",
            "Psychoacoustic bass",
            before.psybass_enabled,
            after.psybass_enabled,
            vec![
                (
                    "harmonics",
                    before.psybass_harmonics_db,
                    after.psybass_harmonics_db,
                ),
                ("drive", before.psybass_drive_db, after.psybass_drive_db),
                (
                    "original bass",
                    before.psybass_original_db,
                    after.psybass_original_db,
                ),
            ],
        ),
        (
            "sub.",
            "Subharmonic synth",
            get(&before.subharm, 0) != 0.0,
            get(&after.subharm, 0) != 0.0,
            [
                ("low", 2),
                ("high", 3),
                ("top", 4),
                ("boost", 5),
                ("ceiling", 9),
            ]
            .iter()
            .map(|(w, i)| (*w, get(&before.subharm, *i), get(&after.subharm, *i)))
            .collect(),
        ),
        (
            "tube.",
            "Tube",
            get(&before.tube, 0) != 0.0,
            get(&after.tube, 0) != 0.0,
            vec![
                ("drive", get(&before.tube, 3), get(&after.tube, 3)),
                ("trim", get(&before.tube, 13), get(&after.tube, 13)),
            ],
        ),
    ];
    for (prefix, word, was_on, is_on, levels) in fx {
        if !touched(prefix) || !is_on {
            continue;
        }
        let what = if was_on {
            let raised: Vec<String> = levels
                .iter()
                .filter(|(_, a, b)| louder(*a, *b))
                .map(|(w, a, b)| format!("{w} from {a:.1} to {b:.1}"))
                .collect();
            if raised.is_empty() {
                continue;
            }
            format!("{word} raised: {}", raised.join(", "))
        } else {
            format!("Turn on {word} (it adds level)")
        };
        risks.push(Risk {
            kind: RiskKind::Louder,
            what,
            output: None,
            keys: keys(prefix),
        });
    }
    judge_masks(before, after, writes, risks); // -- WP MCP2c --
    if !touched("limit.") {
        return;
    }
    for o in 0..before.num_output_channels {
        if !get(&before.limiter_enabled, o) {
            continue;
        }
        let (a, b) = (
            get(&before.limiter_threshold_db, o),
            get(&after.limiter_threshold_db, o),
        );
        let what = if !get(&after.limiter_enabled, o) {
            format!("Turn off {}'s limiter", out_word(o))
        } else if louder(a, b) {
            format!(
                "{} limiter ceiling up from {} to {}",
                out_word(o),
                db(a),
                db(b)
            )
        } else {
            continue;
        };
        risks.push(Risk {
            kind: RiskKind::Louder,
            what,
            output: Some(o),
            keys: keys("limit."),
        });
    }
}

// -- WP MCP2c --
/// An effect that adds level, already on, now also on an output it did not
/// play on: louder for that output (review RB E2). Turned on in the same
/// call, the turn-on is asked already. The upmixer turned on feeds the
/// centre and surround outputs: an output enabled, in effect (E3).
fn judge_masks(before: &DspState, after: &DspState, writes: &[WriteKey], risks: &mut Vec<Risk>) {
    let keys = |path: &str| -> Vec<WriteKey> {
        writes.iter().filter(|w| w.path == path).cloned().collect()
    };
    let masks: [(&str, &str, bool, u32, u32); 5] = [
        (
            "loud.mask",
            "Loudness",
            before.loudness_enabled && after.loudness_enabled,
            u32::from(before.loudness_output_mask),
            u32::from(after.loudness_output_mask),
        ),
        (
            "lev.masks",
            "Leveller",
            before.leveller_enabled && after.leveller_enabled && after.leveller_max_gain_db > 0.0,
            u32::from(before.leveller_apply_mask),
            u32::from(after.leveller_apply_mask),
        ),
        (
            "bass.mask",
            "Psychoacoustic bass",
            before.psybass_enabled && after.psybass_enabled,
            u32::from(before.psybass_output_mask),
            u32::from(after.psybass_output_mask),
        ),
        (
            "sub.mask",
            "Subharmonic synth",
            get(&before.subharm, 0) != 0.0 && get(&after.subharm, 0) != 0.0,
            get(&before.subharm, 1) as u32,
            get(&after.subharm, 1) as u32,
        ),
        (
            "tube.mask",
            "Tube",
            get(&before.tube, 0) != 0.0 && get(&after.tube, 0) != 0.0,
            get(&before.tube, 1) as u32,
            get(&after.tube, 1) as u32,
        ),
    ];
    for (path, word, on, was, now) in masks {
        if !on || !writes.iter().any(|w| w.path == path) {
            continue;
        }
        let added = now & !was;
        for o in 0..after.num_output_channels.min(16) {
            if added & 1u32.wrapping_shl(u32::from(o)) == 0 {
                continue;
            }
            risks.push(Risk {
                kind: RiskKind::Louder,
                what: format!("{word} now also on {} (it adds level)", out_word(o)),
                output: Some(o),
                keys: keys(path),
            });
        }
    }
    if writes.iter().any(|w| w.path == "up.on")
        && get(&before.upmix, 0) == 0.0
        && get(&after.upmix, 0) != 0.0
    {
        risks.push(Risk {
            kind: RiskKind::Enable,
            what: "Turn on the upmixer (it plays on the centre and surround outputs)".to_owned(),
            output: None,
            keys: keys("up.on"),
        });
    }
}

// ------------------------------------------------------------- the policy

/// The policy the app runs agents under: the setting, the judgement and the
/// hook. The app keeps an `Arc` to change the setting while it serves.
#[derive(Debug, Default)]
pub struct AgentPolicy {
    mode: AtomicU8,
}

/// Tools whose writes are never judged: a revert is never refused
/// (PLAN-CLIENTS 5.9). Each puts back only what this agent applied itself.
// -- WP FEAT2 -- dspi_tune_revert, as its text says (and AIVIS: it promises the same)
const UNJUDGED_TOOLS: &[&str] = &["dspi_revert_changes", "dspi_tune_revert"];

impl AgentPolicy {
    /// A policy with `mode`.
    pub fn new(mode: ConfirmMode) -> Self {
        Self {
            mode: AtomicU8::new(mode.to_u8()),
        }
    }

    /// The setting.
    pub fn mode(&self) -> ConfirmMode {
        ConfirmMode::from_u8(self.mode.load(Ordering::Relaxed))
    }

    /// Change the setting; calls from then on are judged by it.
    pub fn set_mode(&self, mode: ConfirmMode) {
        self.mode.store(mode.to_u8(), Ordering::Relaxed);
    }

    fn hook(cell: JudgingCell) -> WriteHook {
        Arc::new(move |p: &WriteProposal<'_>| {
            let mut j = lock(&cell);
            let writes: Vec<WriteKey> = p
                .writes
                .iter()
                .map(|w| WriteKey {
                    path: w.path.clone(),
                    indices: w.indices.clone(),
                    after: w.after.clone(),
                })
                .filter(|w| !j.planned(w))
                .collect();
            if writes.is_empty() {
                return Ok(());
            }
            let verdict = judge(p.state, &writes, &j);
            // -- WP MCP2b -- a wholesale write the user allowed whole
            let verdict = if j.wholesale {
                umbrella(verdict)
            } else {
                verdict
            };
            match verdict {
                Verdict::Pass => Ok(()),
                Verdict::Ask(risks) => {
                    j.asked.extend(risks);
                    Err(DspiError::ConfirmationRequired {
                        code: AGENT_CONFIRM.into(),
                        msg: HELD_TEXT.into(),
                    })
                }
                Verdict::Refuse(why) => {
                    j.refused = Some(why.clone());
                    Err(DspiError::ConfirmationRequired {
                        code: AGENT_REFUSED.into(),
                        msg: why,
                    })
                }
            }
        })
    }
}

/// Each output's role from `core`'s speaker setup, when the user confirmed
/// it; none otherwise (an unconfirmed or imported setup is not the user's
/// word, review RA F1).
pub fn confirmed_roles(core: &DspiCore) -> Vec<Option<MeasureRole>> {
    let Ok(Some(layout)) = core.speaker_layout() else {
        return Vec::new();
    };
    if !layout.confirmed_by_user {
        return Vec::new();
    }
    let mut out = Vec::new();
    for o in &layout.outputs {
        let at = usize::from(o.output);
        if out.len() <= at {
            out.resize(at.saturating_add(1), None);
        }
        if let Some(slot) = out.get_mut(at) {
            *slot = Some(o.role);
        }
    }
    out
}

impl Policy for AgentPolicy {
    fn write_hook(&self, call: &CallInfo<'_>) -> Option<WriteHook> {
        let cell = JudgingCell::default();
        lock(&cell).mode = Some(self.mode());
        (!UNJUDGED_TOOLS.contains(&call.tool)).then(|| Self::hook(cell))
    }

    fn judging_hook(
        &self,
        call: &CallInfo<'_>,
        core: &DspiCore,
        judging: &JudgingCell,
    ) -> Option<WriteHook> {
        let roles = confirmed_roles(core);
        {
            let mut j = lock(judging);
            j.mode = Some(self.mode());
            j.roles = roles;
        }
        (!UNJUDGED_TOOLS.contains(&call.tool)).then(|| Self::hook(Arc::clone(judging)))
    }

    // -- WP FEAT2 --
    fn setting(&self) -> Option<ConfirmMode> {
        Some(self.mode())
    }

    fn agent_may(&self) -> Value {
        json!({
            "read": true,
            "write": true,
            "measure": true, // -- WP FEAT2 -- where the app measures (`Call::agent_may`)
            "confirm": self.mode().as_str(),
            "peq_boost_step_db": PEQ_BOOST_STEP_DB,
            "delay_step_ms": DELAY_STEP_MS,
            "tweeter_floor_hz": TWEETER_FLOOR_HZ,
        })
    }
}
