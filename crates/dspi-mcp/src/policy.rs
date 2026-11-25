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
//! (-- WP ADET1 -- or, until the user confirms it, a detected one) calls a
//! tweeter (or a midrange) keeps a high-pass at 1 kHz (80 Hz) or above,
//! 12 dB/oct or steeper; and an output the speaker detection heard nothing
//! from is not routed to, enabled or unmuted. The core adds its own: the limits profile,
//! the installer lock, read-only modes, and `user_only` calls; [`hard_code`]
//! names those so no layer asks the user about them.
#![allow(clippy::module_name_repetitions)]

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering}; // -- WP FEATUI -- AtomicU32, ADET1 AtomicBool
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
    // -- WP ADET1 --
    /// The speaker detection: test signals on outputs nothing is known of,
    /// whose result the safety rules then rest on. Always asked with
    /// [`RiskKind::TestSignal`], as one grouped question ([`require_detect`]).
    Detect,
    // -- WP ROLES --
    /// Speaker roles set from the user's words (`dspi_set_speaker_roles`):
    /// the whole table in one question ([`require_roles`]).
    SpeakerRoles,
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
            Self::Source => "source",              // -- WP MCP2b --
            Self::System => "system",              // -- WP MCP2c --
            Self::Detect => "detect",              // -- WP ADET1 --
            Self::SpeakerRoles => "speaker_roles", // -- WP ROLES --
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
    // -- WP UAG -- (UXP A6)
    /// The app's words for it ([`said_of`]): `what` with the channels and
    /// presets as the screen names them ("Right tweeter: gain −5.0 dB →
    /// −2.0 dB", "preset 2 “Lunch”"), without "(flash)"; `what` when the
    /// call had no device. It may hold names the user or a device wrote, so
    /// it is never in [`Risk::to_json`], which agents read.
    pub said: String,
    /// The name of [`Risk::output`] as the screens show it ("Right
    /// tweeter"), "" for none: the sheet's heading. The app's alone, as
    /// [`Risk::said`].
    pub channel: String,
}

impl Risk {
    /// As JSON, for the app and the agent: `output` from 1, as the app shows it.
    /// Never [`Risk::said`] (-- WP UAG --).
    pub fn to_json(&self) -> Value {
        json!({"kind": self.kind.as_str(), "what": self.what, "output": self.output.map(|o| u16::from(o).saturating_add(1))})
    }
}

// -- WP UAG -- (UXP A6)
/// The names the app's screens give a device's channels and presets, for
/// the question's words ([`Risk::said`], [`Judging::rest`]): the app's alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenWords {
    /// The channels ([`DspiCore::channel_words`]).
    pub channels: dspi_core::ChannelWords,
    /// Each preset's name, "" for none (slot 1 first).
    pub presets: Vec<String>,
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
    /// Each output's role from the **trusted** speaker setup: the user's
    /// confirmed one, or (-- WP ADET1 --) one the speaker detection found.
    pub roles: Vec<Option<MeasureRole>>,
    // -- WP ADET1 --
    /// The roles were found by the speaker detection, not yet confirmed (the
    /// refusals say so).
    pub detected: bool,
    /// The outputs the speaker detection heard nothing from: an agent may
    /// not route to them, enable or unmute them ([`nothing_outputs`]).
    pub nothing: Vec<u8>,
    /// Agents may detect the speakers under "never ask" (Settings › AI
    /// Agents › Agents may detect speakers; off by default).
    pub detect_allowed: bool,
    // -- WP ROLES --
    /// The roles are an agent's, set from the user's words and not yet
    /// confirmed (the refusals say so).
    pub stated: bool,
    /// Agents may set speaker roles from the user's words under "never ask"
    /// (Settings › AI Agents › Agents may set speaker roles from my words;
    /// off by default).
    pub roles_allowed: bool,
    /// The outputs a stated setup takes for tweeters whatever the rows say
    /// ([`held_outputs`]): the refusals say why.
    pub held: Vec<Held>,
    // -- WP REVDFIX -- B2
    /// The outputs whose role is the user's own word ([`own_outputs`]): only
    /// their midrange rule stands alone; on any other row a high-pass at
    /// 1 kHz or above still shows a tweeter (owner 2026-11-23, answer 3).
    pub own: Vec<u8>,
    // -- WP FFIX --
    /// The limits profile's high-pass per output: (output, corner, slope);
    /// it raises a confirmed tweeter's or midrange's floor, never lowers it.
    pub floors: Vec<(u8, f32, u8)>,
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
    // -- WP UAG -- (UXP A6)
    /// The device's names as the app shows them, for [`Risk::said`]; `None`
    /// under a policy that does not judge on a device.
    pub words: Option<ScreenWords>,
    /// The held call's other changes, in the app's words: what the same
    /// Allow also does that is not asked about on its own ("Sub: gain 0 →
    /// −6 dB"). The app's alone, as [`Risk::said`].
    pub rest: Vec<String>,
    // -- WP UAG -- (UXP A7)
    /// Why the console refused the call, in the app's words ("Kept Right
    /// tweeter's high-pass at 1 kHz", "The installer lock is on"): the
    /// activity entry's `said`, which agents never read.
    pub refusal: Option<String>,
    /// The call's own words for the app in place of the history's, when its
    /// steps would mislead (a quiet preset load: "Master volume −127 dB;
    /// Preset 2 loaded"). The app's alone.
    pub said: Option<String>,
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
    // -- WP FUNC2a -- nothing is asked while the user holds a control of the
    // device: their Allow would meet the core's drag gate and do nothing
    if matches!(verdict, Verdict::Ask(_))
        && let Some((core, _)) = taken_back.0.as_ref()
        && core.user_editing()
    {
        return Err(core_error(
            cell,
            DspiError::ConfirmationRequired {
                code: USER_EDITING.into(),
                msg: String::new(),
            },
        ));
    }
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
            ask_all(&mut j, state, writes, risks); // -- WP UAG --
            Err(ToolError::new(HELD_TEXT))
        }
        Verdict::Refuse(why) => {
            j.refusal = Some(refusal_words(&why, j.words.as_ref())); // -- WP UAG --
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
    require_keyed(cell, kind, what, indices, None)
}

// -- WP INTI -- (review RF2)
/// The key path of a test signal across a hub's units: [`require_system_signal`].
pub const SYSTEM_SIGNAL_PATH: &str = "static.test_signal.system";

// -- WP INTI -- (review RF2)
/// [`require`] for test signals across several units of a hub (a system
/// measurement, verify or sum check): an agent's grant for test signals on
/// the call's device does not cover them, and their question grants none
/// ([`crate::grant::asks_grant`]). The key is [`SYSTEM_SIGNAL_PATH`].
pub fn require_system_signal(
    cell: &JudgingCell,
    what: &str,
    indices: &[u8],
) -> Result<(), ToolError> {
    require_keyed(
        cell,
        RiskKind::TestSignal,
        what,
        indices,
        Some(SYSTEM_SIGNAL_PATH),
    )
}

/// [`require`] with the key's path (`None`: `static.<kind>`, and an agent's
/// standing grant counts for test signals).
fn require_keyed(
    cell: &JudgingCell,
    kind: RiskKind,
    what: &str,
    indices: &[u8],
    path: Option<&str>,
) -> Result<(), ToolError> {
    let mut j = lock(cell);
    let Some(mode) = j.mode else {
        return Err(ToolError::new(
            "the server's policy does not let agents do this",
        ));
    };
    let key = WriteKey::new(
        &path.map_or_else(|| format!("static.{}", kind.as_str()), str::to_owned),
        indices,
        ParamValue::Trigger,
    );
    if mode == ConfirmMode::NeverAsk || j.grant.contains(&key) {
        return Ok(());
    }
    // -- WP ODEC -- the agent's grant for test signals on this device
    if kind == RiskKind::TestSignal
        && path.is_none()
        && j.grant.contains(&crate::grant::standing_key())
    {
        return Ok(());
    }
    push_asked(
        &mut j,
        Risk {
            said: String::new(), // -- WP UAG --
            channel: String::new(),
            kind,
            what: what.to_owned(),
            output: None,
            keys: vec![key],
        },
    );
    Err(ToolError::new(HELD_TEXT))
}

// -- WP ADET1 --
/// The refusal of a speaker detection under "never ask" without the
/// setting.
pub const DETECT_NEVER_ASK_TEXT: &str = "under \"never ask\" agents may detect the speakers only \
     when the user has switched on Settings › AI Agents › Agents may detect speakers (it is off): \
     nothing was played. Ask the user to switch it on, or to set up the speakers in Settings › \
     Speakers.";

// -- WP ADET1 --
/// The speaker detection (AUTODES 4): test signals and the detection, asked
/// as one grouped question (`signal`: what plays, `detect`: the detection
/// and its warning, in the user's words) under "ask on everything" and "ask
/// for risky changes"; not asked again while the agent holds a grant for
/// detections on the device (`crate::grant`). Under "never ask" nothing is
/// asked: refused unless [`Judging::detect_allowed`]. The grant keys are
/// `static.test_signal` and `static.detect` with `indices`.
pub fn require_detect(
    cell: &JudgingCell,
    signal: &str,
    detect: &str,
    indices: &[u8],
) -> Result<(), ToolError> {
    let mut j = lock(cell);
    let Some(mode) = j.mode else {
        return Err(ToolError::new(
            "the server's policy does not let agents do this",
        ));
    };
    if mode == ConfirmMode::NeverAsk {
        return if j.detect_allowed {
            Ok(())
        } else {
            j.refusal = Some(SWITCHED_OFF_DETECT.to_owned()); // -- WP UAG --
            Err(ToolError::new(DETECT_NEVER_ASK_TEXT))
        };
    }
    let key = |kind: RiskKind| {
        WriteKey::new(
            &format!("static.{}", kind.as_str()),
            indices,
            ParamValue::Trigger,
        )
    };
    let (signal_key, detect_key) = (key(RiskKind::TestSignal), key(RiskKind::Detect));
    if j.grant.contains(&detect_key) || j.grant.contains(&crate::grant::standing_detect_key()) {
        return Ok(());
    }
    push_asked(
        &mut j,
        Risk {
            said: String::new(), // -- WP UAG --
            channel: String::new(),
            kind: RiskKind::TestSignal,
            what: signal.to_owned(),
            output: None,
            keys: vec![signal_key],
        },
    );
    push_asked(
        &mut j,
        Risk {
            said: String::new(), // -- WP UAG --
            channel: String::new(),
            kind: RiskKind::Detect,
            what: detect.to_owned(),
            output: None,
            keys: vec![detect_key],
        },
    );
    Err(ToolError::new(HELD_TEXT))
}

// -- WP ROLES --
/// The refusal of speaker roles under "never ask" without the setting.
pub const ROLES_NEVER_ASK_TEXT: &str = "under \"never ask\" agents may set the speaker roles \
     only when the user has switched on Settings › AI Agents › Agents may set speaker roles from \
     my words (it is off): nothing was changed. Ask the user to switch it on, or to set up the \
     speakers in Settings › Speakers.";

// -- WP ROLES --
/// Speaker roles from the user's words (owner 2026-11-23): one question
/// showing the whole table (`what`) under "ask on everything" and "ask for
/// risky changes"; under "never ask" nothing is asked: refused unless
/// [`Judging::roles_allowed`], and (-- WP REVDFIX -- B1) always when
/// `demotes` says why (a statement that makes the user's own tweeter
/// something else). The grant key is `static.speaker_roles` with `indices`
/// (the table, so an Allow covers exactly the table shown).
pub fn require_roles(
    cell: &JudgingCell,
    what: &str,
    indices: &[u8],
    demotes: Option<&str>,
) -> Result<(), ToolError> {
    let mut j = lock(cell);
    let Some(mode) = j.mode else {
        return Err(ToolError::new(
            "the server's policy does not let agents do this",
        ));
    };
    if mode == ConfirmMode::NeverAsk {
        if let Some(why) = demotes {
            return Err(ToolError::new(why));
        }
        return if j.roles_allowed {
            Ok(())
        } else {
            j.refusal = Some(SWITCHED_OFF_ROLES.to_owned()); // -- WP UAG --
            Err(ToolError::new(ROLES_NEVER_ASK_TEXT))
        };
    }
    let key = WriteKey::new(
        &format!("static.{}", RiskKind::SpeakerRoles.as_str()),
        indices,
        ParamValue::Trigger,
    );
    if j.grant.contains(&key) {
        return Ok(());
    }
    push_asked(
        &mut j,
        Risk {
            said: String::new(), // -- WP UAG --
            channel: String::new(),
            kind: RiskKind::SpeakerRoles,
            what: what.to_owned(),
            output: None,
            keys: vec![key],
        },
    );
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
    push_asked(
        &mut j,
        Risk {
            said: String::new(), // -- WP UAG --
            channel: String::new(),
            kind: RiskKind::Change,
            what: what.to_owned(),
            output: None,
            keys: vec![key],
        },
    );
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

/// The installer lock's refusal: what still passes
/// (`dspi_core::limits::lock::LOCKED_USER_WRITES`, the locked-user banner's
/// "only volume, mute and presets"), where the user unlocks, and that the PIN
/// is never the agent's. -- WP FUNC2c -- (FSTORY S7: 0 of 3 agents offered the
/// mute the lock lets through when the text said only "ask the user to
/// unlock".)
pub const LOCKED_TEXT: &str = "the installer lock is on: while it is locked only the volume, \
     the mutes and a preset load pass (dspi_set_volume, dspi_set_user_mute, dspi_set_mute, \
     dspi_preset_load). For anything else ask the user to unlock it in the app (Settings › \
     Installer); no tool unlocks it, and an agent never takes or uses the PIN. Nothing was \
     changed.";

/// The installer lock as an agent reads it (`dspi_get_overview`,
/// `dspi_get_limits`): set, locked, and while locked what still passes.
/// -- WP FUNC2c --
pub fn lock_json(lock: &dspi_core::LockState) -> Value {
    if !lock.locked {
        return json!({"pin_set": lock.pin_set, "locked": false});
    }
    json!({
        "pin_set": lock.pin_set,
        "locked": true,
        "while_locked": "only the volume, the mutes and a preset load pass (dspi_set_volume, \
                         dspi_set_user_mute, dspi_set_mute, dspi_preset_load); the user unlocks \
                         in the app (Settings › Installer)",
    })
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
    // -- WP UAG -- (UXP A7) why, for the person, where the console refuses it
    if let Some(said) = refused_by_core(&e) {
        lock(cell).refusal.get_or_insert_with(|| said.to_owned());
    }
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
        DspiError::InstallerLock { .. } => ToolError::new(LOCKED_TEXT), // -- WP FUNC2c --
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
    judge_nothing(state, &after, &j.nothing, &mut refusals); // -- WP ADET1 --
    judge_values(state, &after, writes, &mut risks);
    judge_highpass(
        state,
        &after,
        writes,
        &j.roles,
        &j.floors,
        setup_word(j.detected, j.stated), // -- WP ADET1 -- ROLES
        &j.held,                          // -- WP ROLES --
        &j.own,                           // -- WP REVDFIX --
        &mut risks,
        &mut refusals,
    );
    judge_effects(state, &after, writes, &mut risks); // -- WP MCP2b --
    if let Some(why) = refusals.into_iter().next() {
        return Verdict::Refuse(why);
    }
    if mode == ConfirmMode::AskEverything && risks.is_empty() {
        risks.push(Risk {
            said: String::new(), // -- WP UAG --
            channel: String::new(),
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

// -- WP UAG -- (UXP A6) ------------------------------------ the app's words

/// The device's names as its screens show them.
fn screen_words(core: &DspiCore) -> ScreenWords {
    ScreenWords {
        channels: core.channel_words(),
        presets: core
            .get_state()
            .preset_names
            .iter()
            .map(|n| crate::untrusted::name(n))
            .collect(),
    }
}

/// Ask the user about `r`, with its words for the app.
fn push_asked(j: &mut Judging, mut r: Risk) {
    r.said = said_of(r.kind, &r.what, j.words.as_ref());
    if let (Some(o), Some(w)) = (r.output, j.words.as_ref()) {
        r.channel = w.channels.output(usize::from(o));
    }
    j.asked.push(r);
}

/// Ask the user about `risks` of a plan of `writes` on `state`, and keep the
/// plan's other changes in the app's words ([`Judging::rest`]).
fn ask_all(j: &mut Judging, state: &DspState, writes: &[WriteKey], risks: Vec<Risk>) {
    let asked: Vec<(String, Vec<u8>)> = risks
        .iter()
        .flat_map(|r| {
            r.keys
                .iter()
                .map(|k| (k.item().0.to_owned(), k.item().1.to_vec()))
        })
        .collect();
    for r in risks {
        push_asked(j, r);
    }
    let Some(words) = j.words.as_ref() else {
        return;
    };
    let mut after = state.clone();
    for w in writes {
        apply(&mut after, w);
    }
    let mut seen: Vec<(String, Vec<u8>)> = Vec::new();
    let mut rows = Vec::new();
    for w in writes {
        let item = (w.item().0.to_owned(), w.item().1.to_vec());
        if asked.contains(&item) || seen.contains(&item) {
            continue;
        }
        seen.push(item);
        // The item's last write is what it becomes
        let last = writes
            .iter()
            .rev()
            .find(|x| x.item() == w.item())
            .unwrap_or(w);
        if let Some(row) = rest_row(state, &after, last, &words.channels)
            && !rows.contains(&row)
        {
            rows.push(row);
        }
    }
    for row in rows {
        if !j.rest.contains(&row) {
            j.rest.push(row);
        }
    }
}

/// One change of a held plan in the app's words, from its value before to
/// after where both are numbers ("Sub: gain 0 → −6 dB"); `None` for a
/// write that changes nothing.
fn rest_row(
    before: &DspState,
    after: &DspState,
    w: &WriteKey,
    words: &dspi_core::ChannelWords,
) -> Option<String> {
    let (item, ix) = w.item();
    let path = if item == "eq" {
        "eq.band"
    } else {
        w.path.as_str()
    };
    let now = value_of(after, path, ix);
    let was = value_of(before, path, ix);
    if now.is_some() && now == was {
        return None;
    }
    let entry = |v: ParamValue| dspi_core::HistoryEntry {
        path: path.to_owned(),
        indices: ix.to_vec(),
        before: None,
        after: v,
        source: dspi_core::HistorySource::Local,
        at_ms: 0,
        undoable: true,
        label: None,
        id: 0,
        actor: 0,
        actor_label: None,
    };
    let text = |v| dspi_core::change_words::change_text(&entry(v), words);
    let said = text(now.unwrap_or_else(|| w.after.clone()));
    Some(match was {
        Some(v) => from_to(&text(v), &said),
        None => said,
    })
}

/// The value at `path` and `ix` on `st`, as a write of it would carry it,
/// for the paths [`apply`] models outside the effects.
fn value_of(st: &DspState, path: &str, ix: &[u8]) -> Option<ParamValue> {
    let i0 = ix.first().copied().unwrap_or(0);
    let i1 = ix.get(1).copied().unwrap_or(0);
    let band = || band_of(st, i0, i1).copied();
    Some(match path {
        "vol.user" => ParamValue::Float(st.user_volume_db),
        "vol.master" => ParamValue::Float(st.master_volume_db),
        "vol.mute" => ParamValue::Bool(st.user_mute),
        "pre" => ParamValue::Float(get(&st.input_preamp_db, i0)),
        "pre.all" => ParamValue::Float(st.preamp_db),
        "bypass" => ParamValue::Bool(st.bypass),
        "out.gain" => ParamValue::Float(get(&st.output_gain_db, i0)),
        "out.delay" => ParamValue::Float(get(&st.output_delay_ms, i0)),
        "out.mute" => ParamValue::Bool(get(&st.output_muted, i0)),
        "out.enable" => ParamValue::Bool(get(&st.output_enabled, i0)),
        "ch.delay" => ParamValue::Float(get(&st.channel_delays, i0)),
        "mix" => ParamValue::Bytes(
            MatrixRoutePacket {
                input: i0,
                output: i1,
                enabled: cell_of(&st.matrix_routing, i0, i1),
                phase_invert: cell_of(&st.matrix_invert, i0, i1),
                gain_db: cell_of(&st.matrix_gain, i0, i1),
            }
            .encode()
            .to_vec(),
        ),
        "eq.band" => ParamValue::Bytes(band_bytes(&band().unwrap_or(FLAT))),
        _ => return None,
    })
}

/// "Sub: gain 0 dB" then "Sub: gain −6 dB" as "Sub: gain 0 → −6 dB": the
/// words both share once, the values from → to; `after` alone unless both
/// differ in a number.
fn from_to(before: &str, after: &str) -> String {
    let b: Vec<&str> = before.split(' ').collect();
    let a: Vec<&str> = after.split(' ').collect();
    let head = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let (bt, at) = (
        b.get(head..).unwrap_or_default(),
        a.get(head..).unwrap_or_default(),
    );
    let digit = |w: &[&str]| w.iter().any(|t| t.chars().any(|c| c.is_ascii_digit()));
    if head == 0 || !digit(bt) || !digit(at) {
        return after.to_owned();
    }
    // A unit both end with is said once
    let unit = bt.len() > 1 && at.len() > 1 && bt.last() == at.last();
    let bt = if unit {
        bt.get(..bt.len().saturating_sub(1)).unwrap_or_default()
    } else {
        bt
    };
    format!(
        "{} {} \u{2192} {}",
        a.get(..head).unwrap_or_default().join(" "),
        bt.join(" "),
        at.join(" ")
    )
}

/// [`Risk::said`]: the server's words `what` for a risk of `kind` as the
/// app's screens say them. "(flash)" goes (the sheet says what a flash write
/// means); a raised value reads from → to and its channel comes first ("Out 4
/// gain up from −5.0 dB to −2.0 dB": "Out 4: gain −5.0 dB → −2.0 dB"); with
/// `words`, the channels and presets are named as the screens name them
/// ("Right tweeter: …", "preset 2 “Lunch”").
pub(crate) fn said_of(kind: RiskKind, what: &str, words: Option<&ScreenWords>) -> String {
    let mut s = what.replace(" (flash)", "");
    if matches!(kind, RiskKind::Louder | RiskKind::DelayStep) {
        if let Some((head, tail)) = s.rsplit_once(" from ")
            && let Some((a, b)) = tail.split_once(" to ")
        {
            let head = head.strip_suffix(" up").unwrap_or(head);
            s = format!("{head} {a} \u{2192} {b}");
        }
        if let Some(ch) = s
            .strip_prefix("Turn off ")
            .and_then(|r| r.strip_suffix("'s limiter"))
        {
            s = format!("{ch} limiter off");
        }
    }
    s = channel_first(&s);
    s = minus_signs(&s);
    match words {
        Some(w) if kind != RiskKind::SpeakerRoles => name_words(&s, w),
        _ => s,
    }
}

/// A number's "-" as the screens write it, "−".
fn minus_signs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev = ' ';
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let number = chars.peek().is_some_and(char::is_ascii_digit);
        out.push(if c == '-' && number && matches!(prev, ' ' | '(') {
            '\u{2212}'
        } else {
            c
        });
        prev = c;
    }
    out
}

/// "Out 4 gain …" as "Out 4: gain …": a channel's own value, its channel
/// first, so identical rows of several channels fold.
fn channel_first(s: &str) -> String {
    let mut t = s.splitn(4, ' ');
    let (Some(ch), Some(n), Some(field)) = (t.next(), t.next(), t.next()) else {
        return s.to_owned();
    };
    let numbered = matches!(ch, "Out" | "In") && n.chars().all(|c| c.is_ascii_digit());
    if !numbered || !matches!(field, "gain" | "trim" | "limiter" | "delay") {
        return s.to_owned();
    }
    let rest = s
        .get(ch.len().saturating_add(n.len()).saturating_add(1)..)
        .unwrap_or_default();
    format!("{ch} {n}:{rest}")
}

/// `s` with "Out N", "In N" and "preset N" named as the screens name them.
pub(crate) fn name_words(s: &str, w: &ScreenWords) -> String {
    let toks: Vec<&str> = s.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(toks.len());
    let mut i = 0usize;
    while let Some(t) = toks.get(i).copied() {
        let next = toks.get(i.saturating_add(1)).copied().unwrap_or("");
        let digits = next.chars().take_while(char::is_ascii_digit).count();
        let n = next
            .get(..digits)
            .and_then(|d| d.parse::<usize>().ok())
            .filter(|n| *n >= 1);
        let tail = next.get(digits..).unwrap_or_default();
        match (t, n) {
            ("Out", Some(n)) => {
                out.push(format!("{}{tail}", w.channels.output(n.saturating_sub(1))))
            }
            ("In", Some(n)) => out.push(format!("{}{tail}", w.channels.input(n.saturating_sub(1)))),
            ("preset", Some(n)) => {
                out.push(t.to_owned());
                let named_here = toks.get(i.saturating_add(2)) == Some(&"as");
                match w.presets.get(n.saturating_sub(1)) {
                    Some(p) if !p.trim().is_empty() && !named_here && tail.is_empty() => {
                        out.push(format!("{n} \u{201c}{p}\u{201d}"));
                    }
                    _ => out.push(next.to_owned()),
                }
            }
            _ => {
                out.push(t.to_owned());
                i = i.saturating_add(1);
                continue;
            }
        }
        i = i.saturating_add(2);
    }
    out.join(" ")
}

// -- WP UAG -- (UXP A7) ------------------------------------- the refusals

/// The refusal of a speaker detection under "never ask" with the switch off,
/// for the person.
const SWITCHED_OFF_DETECT: &str =
    "Agents may not detect speakers: switched off in Settings › AI Agents";
/// The same for speaker roles from the person's words.
const SWITCHED_OFF_ROLES: &str =
    "Agents may not set speaker roles from your words: switched off in Settings › AI Agents";

/// Why the core refused an agent's write, for the person: the rules that are
/// the person's or the installer's, not a question.
fn refused_by_core(e: &DspiError) -> Option<&'static str> {
    match e {
        DspiError::ConfirmationRequired { code, .. } if code == USER_CHANGED => {
            Some("You changed this after the agent did, so it was left as you set it")
        }
        DspiError::ConfirmationRequired { code, .. } if code == USER_ONLY => {
            Some("Only you can do this, in the app")
        }
        DspiError::ConfirmationRequired { code, .. } if code == NOT_OWN_ENTRY => {
            Some("That change is yours: only you can undo it")
        }
        DspiError::ConfirmationRequired { code, .. } if code == AGENT_READ_ONLY => {
            Some("Agents may only read on this computer")
        }
        DspiError::LimitExceeded { .. } => Some("Your limits held it"),
        DspiError::InstallerLock { .. } => Some("The installer lock is on"),
        _ => None,
    }
}

/// A frequency as the screens write it: "1 kHz", "1.2 kHz", "80 Hz".
pub(crate) fn hz_words(hz: f32) -> String {
    if hz >= 1_000.0 {
        let k = format!("{:.1}", hz / 1_000.0);
        format!("{} kHz", k.trim_end_matches(".0"))
    } else {
        format!("{hz:.0} Hz")
    }
}

/// The policy's refusal `why` (this module's words for the agent) as the
/// person reads it in the strip and the activity: what the rule kept, the
/// output named as the screens name it ("Kept Right tweeter's high-pass at
/// 1 kHz"). Never sent to an agent.
pub(crate) fn refusal_words(why: &str, words: Option<&ScreenWords>) -> String {
    // The output the rule is about leads the text ("Out 2: …", "Out 2 drives …")
    let mut t = why.split(' ');
    let out = match (t.next(), t.next()) {
        (Some("Out"), Some(n)) => {
            let n = n.trim_end_matches(':');
            n.chars()
                .all(|c| c.is_ascii_digit())
                .then(|| format!("Out {n}"))
        }
        _ => None,
    };
    let said = match out {
        Some(o) if why.contains("speaker detection heard nothing") => {
            format!("Nothing was heard from {o}: only you can include it, in Settings › Speakers")
        }
        Some(o) if why.contains("high-pass") => {
            let floor = why
                .split_once("high-pass at ")
                .and_then(|(_, r)| r.split_once(" Hz"))
                .and_then(|(n, _)| n.trim().parse::<f32>().ok());
            match floor {
                Some(hz) => format!("Kept {o}'s high-pass at {}", hz_words(hz)),
                None => format!("Kept {o}'s high-pass"),
            }
        }
        Some(o) => format!("Kept {o} safe: the speaker rules refused it"),
        None => "The speaker rules refused it".to_owned(),
    };
    match words {
        Some(w) => name_words(&said, w),
        None => said,
    }
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
                said: String::new(), // -- WP UAG --
                channel: String::new(),
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

// -- WP FFIX -- bug 1: the rule judges the state a write leaves
/// How far a hard output's filters may rise above a 12 dB/octave high-pass at
/// its floor, below the floor, dB: a boost below the floor counts by what
/// reaches the driver.
pub const BELOW_FLOOR_MARGIN_DB: f32 = 3.0;
/// The lowest frequency the below-floor response is judged at, Hz.
const ENVELOPE_LOW_HZ: f32 = 20.0;

/// The hard rule of output `o`: (corner, slope, word), from the user's
/// confirmed role and the limits profile (which may only raise it).
fn hard_rule(
    j_roles: &[Option<MeasureRole>],
    floors: &[(u8, f32, u8)],
    o: u8,
) -> Option<(f32, u8, &'static str)> {
    let (hz, slope, word) = match j_roles.get(usize::from(o)).copied().flatten() {
        Some(MeasureRole::High) => (TWEETER_FLOOR_HZ, MIN_SLOPE_DB, "a tweeter"),
        Some(MeasureRole::Mid) => (MID_FLOOR_HZ, MIN_SLOPE_DB, "a midrange"),
        _ => return None,
    };
    let (p_hz, p_slope) = floors
        .iter()
        .find(|(out, _, _)| *out == o)
        .map_or((0.0, 0), |(_, hz, s)| (*hz, *s));
    let hz = if p_hz.is_finite() { hz.max(p_hz) } else { hz };
    Some((hz, slope.max(p_slope), word))
}

/// Whether output `o` would play programme: routed from an input, on and
/// not muted (the volume is the listener's, not a protection).
pub fn plays(st: &DspState, o: u8) -> bool {
    get(&st.output_enabled, o)
        && !get(&st.output_muted, o)
        && (0..st.num_input_channels).any(|i| cell_of(&st.matrix_routing, i, o))
}

/// Whether output `o` has a high-pass at `floor` or above, `slope` steep.
fn protected_by(st: &DspState, o: u8, floor: f32, slope: u8) -> bool {
    highpasses(st, o)
        .iter()
        .any(|(hz, s)| *hz >= floor - HZ_SLACK && *s >= slope)
}

/// Output `o`'s filters (its routed inputs' PEQ, the worst of them, then its
/// own PEQ and crossover) against a 12 dB/octave high-pass at `floor`, below
/// the floor, at 1/12 octave from 20 Hz: dB above that line at each point
/// (negative: under it).
fn below_floor_curve(st: &DspState, o: u8, floor: f32) -> Vec<f32> {
    let ch = usize::from(st.num_input_channels.saturating_add(o));
    let mut own: Vec<FilterParams> = Vec::new();
    own.extend(st.filters.get(ch).into_iter().flatten().copied());
    own.extend(st.xover.get(ch).into_iter().flatten().copied());
    let mut chains: Vec<Vec<FilterParams>> = (0..st.num_input_channels)
        .filter(|i| cell_of(&st.matrix_routing, *i, o) && !st.bypass)
        .map(|i| {
            let mut c: Vec<FilterParams> = st
                .filters
                .get(usize::from(i))
                .into_iter()
                .flatten()
                .copied()
                .collect();
            c.extend(own.iter().copied());
            c
        })
        .collect();
    if chains.is_empty() {
        chains.push(own);
    }
    let mut out = Vec::new();
    let mut f = ENVELOPE_LOW_HZ;
    while f < floor * 0.97 {
        let reference = -10.0 * (1.0 + (floor / f).powi(4)).log10();
        let worst = chains
            .iter()
            .map(|c| dspi_core::compute_response(c.clone(), f))
            .filter(|db| db.is_finite())
            .fold(f32::NEG_INFINITY, f32::max);
        out.push(worst - reference);
        f *= 2f32.powf(1.0 / 12.0);
    }
    out
}

/// The most a [`below_floor_curve`] rises above its line, dB.
fn excess(curve: &[f32]) -> f32 {
    curve.iter().copied().fold(f32::NEG_INFINITY, f32::max)
}

/// Whether `after` lets more through than `before` below the floor where it
/// is above the line (by more than the margin): a boost made worse.
fn boosted(before: &[f32], after: &[f32]) -> bool {
    after
        .iter()
        .zip(before)
        .any(|(a, b)| *a > BELOW_FLOOR_MARGIN_DB && *a > b + 0.5)
}

/// The outputs `writes` can change the protection of: their filters (an
/// input's PEQ feeds every output it is routed to), routes, switch and mute.
fn touched_outputs(st: &DspState, writes: &[WriteKey]) -> Vec<u8> {
    let n_in = st.num_input_channels;
    let all = || (0..st.num_output_channels).collect::<Vec<u8>>();
    let mut out: Vec<u8> = Vec::new();
    for w in writes {
        let i0 = w.indices.first().copied().unwrap_or(0);
        match w.item().0 {
            "eq" => match i0.checked_sub(n_in) {
                Some(o) => out.push(o),
                None => out.extend(all()),
            },
            "bypass" => out.extend(all()),
            "mix" => out.push(w.indices.get(1).copied().unwrap_or(0)),
            "out.enable" | "out.mute" => out.push(i0),
            // -- WP ODEC -- a level raise feeds an unprotected output more (RE13)
            "out.gain" => out.push(i0),
            "vol.user" | "vol.master" | "vol.mute" | "pre" | "pre.all" => out.extend(all()),
            p if fx_weighed(p) => out.extend(all()),
            _ => {}
        }
    }
    out.retain(|o| *o < st.num_output_channels);
    out.sort_unstable();
    out.dedup();
    out
}

// -- WP ADET1 --
/// The speaker setup's state in a refusal: `confirmed`, detected by AI, or
/// (-- WP ROLES --) set by AI from the user's words.
fn setup_word(detected: bool, stated: bool) -> &'static str {
    // -- WP ROLES --
    if stated {
        "set by AI from the user's words, not yet confirmed"
    } else if detected {
        "detected by AI, not yet confirmed"
    } else {
        "confirmed"
    }
}

// -- WP ADET1 --
/// The `nothing` rule (AUTODES 3.2): an output the speaker detection heard
/// nothing from may not be routed to, enabled or unmuted by an agent, under
/// every setting (the user can, in the app). Judged on what the writes
/// change, so what already plays may be quietened.
fn judge_nothing(before: &DspState, after: &DspState, nothing: &[u8], refusals: &mut Vec<String>) {
    for &o in nothing {
        let what = if !get(&before.output_enabled, o) && get(&after.output_enabled, o) {
            "switch it on"
        } else if get(&before.output_muted, o) && !get(&after.output_muted, o) {
            "unmute it"
        } else if (0..before.num_input_channels)
            .any(|i| !cell_of(&before.matrix_routing, i, o) && cell_of(&after.matrix_routing, i, o))
        {
            "route an input to it"
        } else {
            continue;
        };
        refusals.push(format!(
            "{}: the speaker detection heard nothing from it, so an agent may not {what}. Ask \
             the user whether anything is wired there (Settings › Speakers); only the user can \
             include it. Nothing was changed.",
            agent_out(o)
        ));
    }
}

/// The tweeter rule on every output the writes touch. A confirmed tweeter
/// (or midrange) is judged on the state the writes leave (owner rule, MCP6
/// §4; FUNC1 bug 1): refused when it would play programme with no high-pass
/// at its floor, steep enough, or with a boost below the floor that defeats
/// the high-pass; when the writes give it a high-pass below the floor or too
/// shallow; or when they lower its protection, playing or not. An output that
/// already played unprotected (the user's doing) may be changed in ways that
/// do not make it worse. A high-pass at 1 kHz or above shows a tweeter on any
/// other output (-- WP REVDFIX -- and on a midrange that is not the user's
/// own word): lowering it is asked (refused under "never ask").
#[allow(clippy::too_many_arguments)] // the judgement's facts, each once
fn judge_highpass(
    before: &DspState,
    after: &DspState,
    writes: &[WriteKey],
    roles: &[Option<MeasureRole>],
    floors: &[(u8, f32, u8)],
    setup: &str,
    held: &[Held],
    own: &[u8],
    risks: &mut Vec<Risk>,
    refusals: &mut Vec<String>,
) {
    for o in touched_outputs(before, writes) {
        let ch = before.num_input_channels.saturating_add(o);
        let filters_touched = writes
            .iter()
            .any(|w| w.item().0 == "eq" && w.indices.first() == Some(&ch));
        if let Some((floor, slope, word)) = hard_rule(roles, floors, o) {
            if let Some(why) = hard_verdict(before, after, writes, o, (floor, slope)) {
                // -- WP ROLES -- an output the stated setup takes for a tweeter
                // whatever its row says: said why
                if let Some(Held { why: held, .. }) = held.iter().find(|h| h.output == o) {
                    refusals.push(format!(
                        "{} {held}, so it is guarded as a tweeter: it must keep a high-pass at \
                         {floor:.0} Hz or above, {slope} dB/octave or steeper, whenever it can play \
                         (routed, on and unmuted): {why}. Nothing was changed. Only the user can \
                         change its role (Settings › Speakers); tell the user.",
                        agent_out(o)
                    ));
                    continue;
                }
                refusals.push(format!(
                    "{} drives {word} in the user's speaker setup ({setup}), so it must keep a \
                     high-pass at {floor:.0} Hz or above, {slope} dB/octave or steeper, whenever it \
                     can play: {why}. This is the device's safety rule, not a question: nothing \
                     was changed. Give it such a high-pass first (dspi_set_crossover), or ask \
                     the user to change the speaker setup in the app.",
                    agent_out(o)
                ));
                continue;
            }
            // -- WP REVDFIX -- B2: a floor below 1 kHz holds alone only on the
            // user's own word; a stated or detected midrange is still judged
            // by the high-pass it has
            if floor >= TWEETER_FLOOR_HZ - HZ_SLACK || own.contains(&o) {
                continue;
            }
        }
        // The user's word holds hard; a high-pass at 1 kHz or above shows a
        // tweeter whatever the user called it (the effective role).
        let inferred = highpasses(before, o)
            .iter()
            .any(|(hz, _)| *hz >= TWEETER_FLOOR_HZ - HZ_SLACK);
        if !inferred || !filters_touched {
            continue;
        }
        let lowered = !protected(after, o, TWEETER_FLOOR_HZ)
            && (protected(before, o, TWEETER_FLOOR_HZ)
                || best_highpass(after, o) < best_highpass(before, o) - HZ_SLACK);
        if lowered {
            highpass_verdict(o, ch, writes, risks);
        }
    }
}

/// Why the writes break hard output `o`'s rule `(floor, slope)`, if they do.
fn hard_verdict(
    before: &DspState,
    after: &DspState,
    writes: &[WriteKey],
    o: u8,
    (floor, slope): (f32, u8),
) -> Option<String> {
    let ch = before.num_input_channels.saturating_add(o);
    let (prot_b, prot_a) = (
        protected_by(before, o, floor, slope),
        protected_by(after, o, floor, slope),
    );
    let (curve_b, curve_a) = (
        below_floor_curve(before, o, floor),
        below_floor_curve(after, o, floor),
    );
    let (ex_b, ex_a) = (excess(&curve_b), excess(&curve_a));
    let more = boosted(&curve_b, &curve_a);
    let safe_b = prot_b && ex_b <= BELOW_FLOOR_MARGIN_DB;
    let safe_a = prot_a && ex_a <= BELOW_FLOOR_MARGIN_DB;
    if safe_a {
        return None;
    }
    let (plays_b, plays_a) = (plays(before, o), plays(after, o));
    // A high-pass the writes put there that does not protect.
    let weak = writes
        .iter()
        .filter(|w| w.item().0 == "eq" && w.indices.first() == Some(&ch))
        .find_map(|w| {
            let b = band_of(after, ch, w.indices.get(1).copied().unwrap_or(0))?;
            let s = highpass_slope(b.filter_type)?;
            (!b.bypass && (b.freq < floor - HZ_SLACK || s < slope))
                .then(|| format!("this sets a {s} dB/octave high-pass at {:.0} Hz", b.freq))
        });
    if let (Some(w), false) = (weak, prot_a) {
        return Some(w);
    }
    if prot_b && !prot_a {
        return Some(match best_highpass(after, o) {
            hz if hz > 0.0 => format!("this lowers its high-pass to {hz:.0} Hz"),
            _ => "this removes or weakens its high-pass".to_owned(),
        });
    }
    if prot_a && more {
        return Some(format!(
            "this boosts it below {floor:.0} Hz, {ex_a:.1} dB above what a {MIN_SLOPE_DB} dB/octave \
             high-pass there lets through (at most {BELOW_FLOOR_MARGIN_DB:.0} dB): a boost below \
             the floor defeats the high-pass"
        ));
    }
    if !plays_a {
        return None;
    }
    if !plays_b || safe_b {
        let how = if !prot_a {
            "it has no such high-pass"
        } else {
            "a boost below the floor defeats its high-pass"
        };
        return Some(format!("this would let it play programme and {how}"));
    }
    // It played unprotected already: a change that adds no route and raises
    // nothing below the floor does not make it worse.
    let new_route = (0..before.num_input_channels).any(|i| {
        !cell_of(&before.matrix_routing, i, o) && cell_of(&after.matrix_routing, i, o)
            || cell_of(&after.matrix_gain, i, o)
                > cell_of(&before.matrix_gain, i, o) + LOUDER_EPS_DB
    });
    if new_route || more {
        return Some("it plays with no protection already, and this feeds it more".to_owned());
    }
    // -- WP ODEC -- a level raise feeds it more too (owner 2026-11-23, RE13)
    level_raise(before, after, writes, o).map(|what| {
        format!("it plays with no protection already, and this makes it louder ({what})")
    })
}

// -- WP ODEC --
/// A raise of output `o`'s level the writes make, in words: its gain, the
/// volumes and trims (an input's trim when that input feeds it), an unmute,
/// an effect that adds level on it, a PEQ boost on it or on an input that
/// feeds it (-- WP INTI --, review RF6).
fn level_raise(before: &DspState, after: &DspState, writes: &[WriteKey], o: u8) -> Option<String> {
    let mut risks = Vec::new();
    judge_values(before, after, writes, &mut risks);
    judge_effects(before, after, writes, &mut risks);
    let feeds =
        |i: u8| cell_of(&before.matrix_routing, i, o) || cell_of(&after.matrix_routing, i, o);
    risks
        .into_iter()
        .filter(|r| matches!(r.kind, RiskKind::Louder | RiskKind::Unmute))
        .find(|r| match r.output {
            Some(x) => x == o,
            None if r.keys.iter().all(|k| k.path == "pre") => r
                .keys
                .iter()
                .any(|k| feeds(k.indices.first().copied().unwrap_or(0))),
            None => true,
        })
        .map(|r| r.what)
        .or_else(|| eq_raise(before, after, writes, o, &feeds))
}

// -- WP INTI -- (review RF6, owner decision via the coordinator 2026-11-23)
/// A PEQ band on output `o`, or on an input that feeds it, that boosts more
/// than before (any amount) or becomes a Linkwitz transform, in words: on a
/// tweeter left playing unprotected that is louder too.
fn eq_raise(
    before: &DspState,
    after: &DspState,
    writes: &[WriteKey],
    o: u8,
    feeds: &dyn Fn(u8) -> bool,
) -> Option<String> {
    let n_in = before.num_input_channels;
    writes.iter().find_map(|w| {
        let (path, ix) = w.item();
        let (ch, band) = (*ix.first()?, *ix.get(1)?);
        if path != "eq" || band >= 20 {
            return None;
        }
        let here = if ch < n_in {
            feeds(ch)
        } else {
            ch.checked_sub(n_in) == Some(o)
        };
        if !here {
            return None;
        }
        let was = band_of(before, ch, band).copied().unwrap_or(FLAT);
        let now = band_of(after, ch, band).copied().unwrap_or(FLAT);
        let linkwitz = now.filter_type == 11 && !now.bypass && now != was;
        (linkwitz || boost(&now) > boost(&was) + LOUDER_EPS_DB).then(|| {
            format!(
                "{} EQ band {}: {}",
                channel_word(before, ch),
                u16::from(band).saturating_add(1),
                if linkwitz {
                    "a Linkwitz transform (adds bass)".to_owned()
                } else {
                    format!("a boost of {}", db(boost(&now)))
                }
            )
        })
    })
}

/// The first confirmed tweeter or midrange of `st` that plays programme
/// unprotected (no high-pass at its floor, steep enough, or one a boost below
/// the floor defeats), in words; `None` when every one is safe or silent.
pub fn unprotected_now(st: &DspState, j: &Judging) -> Option<String> {
    (0..st.num_output_channels).find_map(|o| {
        let (floor, slope, word) = hard_rule(&j.roles, &j.floors, o)?;
        let safe = protected_by(st, o, floor, slope)
            && excess(&below_floor_curve(st, o, floor)) <= BELOW_FLOOR_MARGIN_DB;
        (plays(st, o) && !safe).then(|| {
            format!(
                "{} drives {word} in the user's speaker setup ({}) and plays programme \
                 without a high-pass at {floor:.0} Hz or above, {slope} dB/octave or steeper",
                agent_out(o),
                setup_word(j.detected, j.stated) // -- WP ADET1 -- ROLES
            )
        })
    })
}

/// What a wholesale write (a preset load, a recall, an import) changed from
/// `before` to `after`, as the writes the tweeter rule weighs: each output's
/// bands, routes, switch, mute and gain, the inputs' bands and trims, and the
/// volumes.
pub fn state_writes(before: &DspState, after: &DspState) -> Vec<WriteKey> {
    let mut out = Vec::new();
    for (base, was, now) in [
        (0u8, &before.filters, &after.filters),
        (20u8, &before.xover, &after.xover),
    ] {
        for (ch, (a, b)) in (0u8..).zip(was.iter().zip(now)) {
            for (i, (x, y)) in (0u8..).zip(a.iter().zip(b)) {
                if x != y {
                    out.push(WriteKey::new(
                        "eq.band",
                        &[ch, base.saturating_add(i)],
                        ParamValue::Bytes(band_bytes(y)),
                    ));
                }
            }
        }
    }
    for i in 0..after.num_input_channels {
        for o in 0..after.num_output_channels {
            let at = |s: &DspState| {
                (
                    cell_of(&s.matrix_routing, i, o),
                    cell_of(&s.matrix_gain, i, o),
                    cell_of(&s.matrix_invert, i, o),
                )
            };
            let (on, gain_db, invert) = at(after);
            if at(before) != (on, gain_db, invert) {
                let p = MatrixRoutePacket {
                    input: i,
                    output: o,
                    enabled: on,
                    phase_invert: invert,
                    gain_db,
                };
                out.push(WriteKey::new(
                    "mix",
                    &[i, o],
                    ParamValue::Bytes(p.encode().to_vec()),
                ));
            }
        }
    }
    for o in 0..after.num_output_channels {
        for (path, a, b) in [
            ("out.enable", &before.output_enabled, &after.output_enabled),
            ("out.mute", &before.output_muted, &after.output_muted),
        ] {
            if get(a, o) != get(b, o) {
                out.push(WriteKey::new(path, &[o], ParamValue::Bool(get(b, o))));
            }
        }
    }
    // -- WP INTI -- (review RF5) the levels too: a stored preset that raises a
    // gain or a volume feeding a tweeter left playing unprotected is louder
    let changed = |a: f32, b: f32| (a - b).abs() > 0.005;
    for (o, (a, b)) in (0u8..).zip(before.output_gain_db.iter().zip(&after.output_gain_db)) {
        if changed(*a, *b) {
            out.push(WriteKey::new("out.gain", &[o], ParamValue::Float(*b)));
        }
    }
    for (i, (a, b)) in (0u8..).zip(before.input_preamp_db.iter().zip(&after.input_preamp_db)) {
        if changed(*a, *b) {
            out.push(WriteKey::new("pre", &[i], ParamValue::Float(*b)));
        }
    }
    for (path, a, b) in [
        ("pre.all", before.preamp_db, after.preamp_db),
        (
            "vol.master",
            before.master_volume_db,
            after.master_volume_db,
        ),
        ("vol.user", before.user_volume_db, after.user_volume_db),
    ] {
        if changed(a, b) {
            out.push(WriteKey::new(path, &[], ParamValue::Float(b)));
        }
    }
    out
}

/// A band as the journal's whole-band bytes ([`band_of_bytes`]).
pub(crate) fn band_bytes(f: &FilterParams) -> Vec<u8> {
    let mut b = vec![f.filter_type, u8::from(f.bypass)];
    for x in [f.freq, f.q, f.gain, f.qp] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

/// Output `o` (from 0) for the agent: "Out 3", as the tools and the app
/// count it (-- WP SHOWFIX --: one number, from 1).
pub fn agent_out(o: u8) -> String {
    format!("Out {}", u16::from(o).saturating_add(1))
}

/// A lowered high-pass protection on output `o` (wire channel `ch`) that
/// only its high-pass shows to be a tweeter: asked. (A confirmed role's is
/// [`hard_verdict`]'s.)
fn highpass_verdict(o: u8, ch: u8, writes: &[WriteKey], risks: &mut Vec<Risk>) {
    risks.push(Risk {
        said: String::new(), // -- WP UAG --
        channel: String::new(),
        kind: RiskKind::TweeterCrossing,
        what: format!(
            "{} has a high-pass at {TWEETER_FLOOR_HZ} Hz or above (a tweeter): this \
                     removes it, moves it below {TWEETER_FLOOR_HZ} Hz or makes it shallower than \
                     {MIN_SLOPE_DB} dB/oct",
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
            said: String::new(), // -- WP UAG --
            channel: String::new(),
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
            said: String::new(), // -- WP UAG --
            channel: String::new(),
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
                said: String::new(), // -- WP UAG --
                channel: String::new(),
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
            said: String::new(), // -- WP UAG --
            channel: String::new(),
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
    // -- WP FEATUI -- the app's SPL cap for agents' measurements, f32 bits (0: none)
    spl_cap_bits: AtomicU32,
    // -- WP ADET1 -- Settings › AI Agents › Agents may detect speakers
    detect: AtomicBool,
    // -- WP ROLES -- Settings › AI Agents › Agents may set speaker roles from my words
    roles: AtomicBool,
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
            spl_cap_bits: AtomicU32::new(0), // -- WP FEATUI --
            detect: AtomicBool::new(false),  // -- WP ADET1 --
            roles: AtomicBool::new(false),   // -- WP ROLES --
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

    // -- WP FEATUI --
    /// The app's SPL cap for agents' measurements (dB SPL, `None`: the
    /// placement's default). Not a question to the user: it holds under
    /// every setting, "never ask" too. A value that is no finite number is
    /// none.
    pub fn set_agent_spl_cap_db(&self, cap: Option<f32>) {
        let bits = cap
            .filter(|c| c.is_finite() && *c > 0.0)
            .map_or(0, f32::to_bits);
        self.spl_cap_bits.store(bits, Ordering::Relaxed);
    }

    // -- WP ADET1 --
    /// Settings › AI Agents › Agents may detect speakers (off by default):
    /// under "never ask" an agent's speaker detection is refused unless it is
    /// on; the other settings ask the user instead. Holds from the next call.
    pub fn set_agent_detect(&self, allowed: bool) {
        self.detect.store(allowed, Ordering::Relaxed);
    }

    // -- WP ADET1 --
    /// [`AgentPolicy::set_agent_detect`]'s switch.
    pub fn agent_detect(&self) -> bool {
        self.detect.load(Ordering::Relaxed)
    }

    // -- WP ROLES --
    /// Settings › AI Agents › Agents may set speaker roles from my words
    /// (off by default): under "never ask" `dspi_set_speaker_roles` is
    /// refused unless it is on.
    pub fn set_agent_roles(&self, allowed: bool) {
        self.roles.store(allowed, Ordering::Relaxed);
    }

    // -- WP ROLES --
    /// [`AgentPolicy::set_agent_roles`]'s switch.
    pub fn agent_roles(&self) -> bool {
        self.roles.load(Ordering::Relaxed)
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
                    ask_all(&mut j, p.state, &writes, risks); // -- WP UAG --
                    Err(DspiError::ConfirmationRequired {
                        code: AGENT_CONFIRM.into(),
                        msg: HELD_TEXT.into(),
                    })
                }
                Verdict::Refuse(why) => {
                    j.refusal = Some(refusal_words(&why, j.words.as_ref())); // -- WP UAG --
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

/// Each output's role from `core`'s speaker setup when it can be trusted:
/// confirmed by the user, or (-- WP ADET1 --, AUTODES 3.2) found by the
/// speaker detection and not yet confirmed; none otherwise (an unconfirmed or
/// imported setup is not the user's word, review RA F1).
pub fn trusted_roles(core: &DspiCore) -> Vec<Option<MeasureRole>> {
    trusted_setup(core).0
}

// -- WP ADET1 --
/// [`trusted_roles`], whether they were detected (not confirmed), and the
/// outputs the detection heard nothing from ([`nothing_outputs`]).
pub fn trusted_setup(core: &DspiCore) -> (Vec<Option<MeasureRole>>, bool, Vec<u8>) {
    let Ok(Some(layout)) = core.speaker_layout() else {
        return (Vec::new(), false, Vec::new());
    };
    let roles = dspi_core::trusted_roles(&layout).unwrap_or_default();
    let detected = layout.state() == dspi_core::LayoutState::AutoDetected;
    (roles, detected, nothing_outputs(&layout))
}

// -- WP ROLES --
/// Whether `core`'s speaker setup was set by an agent from the user's words
/// and is not yet confirmed.
pub fn stated_setup(core: &DspiCore) -> bool {
    core.speaker_layout()
        .ok()
        .flatten()
        .is_some_and(|l| l.state() == dspi_core::LayoutState::Stated)
}

// -- WP ROLES --
/// An output a stated setup takes for a tweeter whatever its row says.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    /// The output, from 0.
    pub output: u8,
    /// Why, in words that follow "Out N".
    pub why: String,
}

// -- WP ROLES --
/// The outputs `core`'s stated setup takes for tweeters whatever its rows
/// say: an output the user's words left out, a stated row whose top-fall
/// check found a tweeter's top, and (-- WP REVDFIX --) a row that was the
/// user's own tweeter before the statement. (A stated row not checked yet is trusted as
/// stated: only an agent's test signals wait for its check. ROLESFIX, owner
/// 2026-11-24.)
pub fn held_outputs(core: &DspiCore) -> Vec<Held> {
    let Ok(Some(l)) = core.speaker_layout() else {
        return Vec::new();
    };
    if l.state() != dspi_core::LayoutState::Stated {
        return Vec::new();
    }
    let n = l.stated.as_ref().map_or(0, |s| s.device_outputs);
    (0..n)
        .filter_map(|o| {
            let why = match (l.output(o), l.check_of(o)) {
                // -- WP REVDFIX -- B1, the ratchet
                (_, _) if l.user_tweeter_held(o) => {
                    "was a tweeter in the user's own speaker setup, and an agent's statement \
                     does not change that"
                        .to_owned()
                }
                (None, _) => "is not among the roles the user stated".to_owned(),
                (Some(r), Some(c)) if c.verdict == dspi_core::RoleCheckVerdict::TopHolds => {
                    format!(
                        "was stated a {} in the user's words, but its top end holds like a \
                         tweeter's",
                        dspi_core::speaker_layout::role_word(r.role)
                    )
                }
                _ => return None,
            };
            Some(Held { output: o, why })
        })
        .collect()
}

// -- WP REVDFIX -- B2
/// The outputs whose role in `core`'s speaker setup is the user's own word:
/// every row of a confirmed setup, else the rows the user set.
pub fn own_outputs(core: &DspiCore) -> Vec<u8> {
    let Ok(Some(l)) = core.speaker_layout() else {
        return Vec::new();
    };
    let confirmed = l.state() == dspi_core::LayoutState::Confirmed;
    l.outputs
        .iter()
        .filter(|r| confirmed || r.source == dspi_core::RoleSource::User)
        .map(|r| r.output)
        .collect()
}

// -- WP ADET1 --
/// The outputs `layout`'s speaker detection heard nothing from (class
/// `nothing`, heard nowhere) and the layout leaves out: no agent routes to,
/// enables or unmutes them. A row the user added for one includes it.
pub fn nothing_outputs(layout: &dspi_core::SpeakerLayout) -> Vec<u8> {
    layout
        .detected
        .iter()
        .flat_map(|d| d.outputs.iter())
        .filter(|o| {
            o.class == dspi_core::DetectClass::Nothing
                && o.heard == dspi_core::DetectHeard::Never
                && layout.output(o.output).is_none()
        })
        .map(|o| o.output)
        .collect()
}

// -- WP FFIX --
/// The open device's limits profile's high-pass per output: (output, corner,
/// slope), for the outputs that name a corner.
pub fn profile_floors(core: &DspiCore) -> Vec<(u8, f32, u8)> {
    let doc: Value = serde_json::from_str(&core.limits_profile_document()).unwrap_or(Value::Null);
    let outs = doc
        .pointer("/limits/outputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    outs.iter()
        .filter_map(|o| {
            let out = u8::try_from(o.get("output")?.as_u64()?).ok()?;
            let hz = o.get("highpassMinHz")?.as_f64()? as f32;
            let slope = o
                .get("highpassMinSlopeDb")
                .and_then(Value::as_u64)
                .and_then(|s| u8::try_from(s).ok())
                .unwrap_or(0);
            Some((out, hz, slope))
        })
        .collect()
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
        let (roles, detected, nothing) = trusted_setup(core); // -- WP ADET1 --
        let floors = profile_floors(core); // -- WP FFIX --
        {
            let mut j = lock(judging);
            j.mode = Some(self.mode());
            j.roles = roles;
            j.floors = floors;
            // -- WP ADET1 --
            j.detected = detected;
            j.nothing = nothing;
            j.detect_allowed = self.agent_detect();
            // -- WP ROLES --
            j.stated = stated_setup(core);
            j.roles_allowed = self.agent_roles();
            j.held = held_outputs(core);
            j.own = own_outputs(core); // -- WP REVDFIX --
            j.words = Some(screen_words(core)); // -- WP UAG --
        }
        (!UNJUDGED_TOOLS.contains(&call.tool)).then(|| Self::hook(Arc::clone(judging)))
    }

    // -- WP FEAT2 --
    fn setting(&self) -> Option<ConfirmMode> {
        Some(self.mode())
    }

    // -- WP FEATUI --
    fn agent_spl_cap_db(&self) -> Option<f32> {
        match self.spl_cap_bits.load(Ordering::Relaxed) {
            0 => None,
            bits => Some(f32::from_bits(bits)),
        }
    }

    fn agent_may(&self) -> Value {
        json!({
            "read": true,
            "write": true,
            "measure": true, // -- WP FEAT2 -- where the app measures (`Call::agent_may`)
            // -- WP ADET1 -- under "never ask" only with the user's switch
            "detect": self.mode() != ConfirmMode::NeverAsk || self.agent_detect(),
            // -- WP ROLES -- the same for speaker roles from the user's words
            "set_roles": self.mode() != ConfirmMode::NeverAsk || self.agent_roles(),
            "confirm": self.mode().as_str(),
            "peq_boost_step_db": PEQ_BOOST_STEP_DB,
            "delay_step_ms": DELAY_STEP_MS,
            "tweeter_floor_hz": TWEETER_FLOOR_HZ,
        })
    }
}
