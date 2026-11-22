//! Who writes, and what an agent's writes pass through (WP MCP0).
//!
//! The embedding app writes as itself: actor [`EMBEDDER`], 0. Anyone else
//! that writes through the same core in the same process (the MCP server's
//! AI agents) registers an actor ([`register_actor`]) and wraps its calls in
//! an [`ActorScope`]. While the scope is alive, on its thread and for its
//! core:
//!
//! - **Attribution.** Every journalled change carries the actor
//!   ([`crate::HistoryEntry::actor`] and `actor_label`), and a step gathers
//!   one actor's changes only: an agent's write never joins the user's drag
//!   or undo group, nor the user's an agent's.
//! - **The embedder hears of it.** A change by another actor raises
//!   `NOTIFY_STATE | NOTIFY_CURVES | NOTIFY_HISTORY` on the next
//!   [`DspiCore::process_notifications`], and the notification listener is
//!   woken when the scope ends. The device's echo of such a write equals the
//!   mirror and is classed as this console's own, so without this the app's
//!   view would not move.
//! - **The user is editing.** While the embedder has a drag marked
//!   ([`DspiCore::set_dragging`]), a write by another actor is refused before
//!   anything is sent: [`DspiError::ConfirmationRequired`] with code
//!   [`USER_EDITING`]. Retry once the drag ends. The two writers would
//!   otherwise interleave on one control, and the app, which coalesces its
//!   own writes, would put a stale value over the agent's.
//! - **The pre-write hook.** The scope may carry a [`WriteHook`]. Every
//!   write the core can name (a typed setter, `write_param`, a whole band,
//!   an effect, and a whole-state write such as a snapshot recall, an A/B
//!   toggle or a configuration import) shows it what changes, path by path,
//!   with the value before (from the mirror) and after, before anything is
//!   sent. The hook refuses by returning an error, which the call returns
//!   unchanged; nothing is sent then. A whole-state write is shown as one
//!   proposal of every value it changes. Undo, redo and
//!   [`DspiCore::revert_entry`] replays pass through it too: reverting a cut
//!   is a boost.
//! - **Undo by id.** [`DspiCore::revert_entry`] reverts one change, and an
//!   actor other than the embedder may revert only its own.
//! - **Not judged: writes the device applies wholesale.** A preset load and
//!   `load_params` send one command whose result is known only once the
//!   device has applied it, so the hook never sees them. Like a recall and an
//!   import they journal one `bulk` / `preset.load` entry, which resets the
//!   history: no undo, and [`DspiCore::revert_entry`] cannot undo them. A
//!   layer that lets an agent do these must class them itself (review RA F5).
//!
//! The hook runs under the core's lock: it must not call the core. Outside a
//! scope nothing of this runs beyond a thread-local look-up.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dspi_proto::FilterType;
use dspi_proto::registry::REGISTRY;
use dspi_proto::value::Value;

use crate::consts::{NOTIFY_CURVES, NOTIFY_HISTORY, NOTIFY_STATE};
use crate::records::{DspState, FilterParams};
use crate::write_param::ParamValue;
use crate::{DspiCore, DspiError, Inner, Result};

/// Who made a change: [`EMBEDDER`] or an actor from [`register_actor`].
pub type ActorId = u32;

/// The embedding app itself: every write outside an [`ActorScope`].
pub const EMBEDDER: ActorId = 0;

/// [`DspiError::ConfirmationRequired`]'s code for a write by another actor
/// while the embedder's user drags a control: nothing was sent; retry.
pub const USER_EDITING: &str = "user_editing";

/// [`DspiError::ConfirmationRequired`]'s code for a
/// [`DspiCore::revert_entry`] by an actor of a change it did not make.
pub const NOT_OWN_ENTRY: &str = "not_own_entry";

// -- WP INT1 -- (ROLE0 x MCP0)
/// [`DspiError::ConfirmationRequired`]'s code for a call only the embedder's
/// user may make, such as [`DspiCore::speaker_layout_set`]: the speaker
/// layout is the user's word on which output drives a tweeter, and the
/// safety rules trust it. Nothing was written.
pub const USER_ONLY: &str = "user_only";

/// Refuse `what` unless this thread acts as the embedder on `core`.
pub(crate) fn user_only(core: &DspiCore, what: &str) -> Result<()> {
    if current(core.lock().actor_key) == EMBEDDER {
        return Ok(());
    }
    Err(DspiError::ConfirmationRequired {
        code: USER_ONLY.into(),
        msg: format!("{what} is the user's to set, in the app; an agent cannot set it"),
    })
}

/// The longest actor label kept, in characters.
const LABEL_MAX_CHARS: usize = 64;

/// The labels of the registered actors; actor `n` is at `n - 1`. One table
/// for the process, so one agent is one actor on every core it writes to.
static LABELS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Register an actor with a label for the history ("AI · Claude Code"):
/// control and format characters (bidi overrides, zero-width ones) are
/// dropped and the label is cut to 64 characters. Each call gives a new
/// actor, never [`EMBEDDER`].
pub fn register_actor(label: &str) -> ActorId {
    let label: String = label
        .chars()
        .filter(|c| !c.is_control() && !is_format(*c))
        .take(LABEL_MAX_CHARS)
        .collect();
    let mut labels = LABELS.lock().unwrap_or_else(|p| p.into_inner());
    labels.push(label);
    // Past u32::MAX registrations the newest number is reused; a process
    // never gets there.
    ActorId::try_from(labels.len()).unwrap_or(ActorId::MAX)
}

/// Whether `c` is a Unicode format character (general category `Cf`, Unicode
/// 15.1): invisible, and some (U+202E) reorder the text around them, so an
/// agent's label could make the History read other than it is (review RA F6).
/// -- WP MCP1 -- public: `dspi-mcp`'s sanitiser of untrusted text uses the
/// same table.
pub fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// The label `actor` was registered with; `None` for [`EMBEDDER`] and for a
/// number never handed out.
pub fn actor_label(actor: ActorId) -> Option<String> {
    let at = usize::try_from(actor.checked_sub(1)?).ok()?;
    LABELS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(at)
        .cloned()
}

/// One value a write is about to change, for a [`WriteHook`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedWrite {
    /// The registry path (`out.gain`, `eq.freq`, `vol.user`, ...).
    pub path: String,
    /// Its indices (an output, a wire channel and a band, ...).
    pub indices: Vec<u8>,
    /// What the mirror holds for it now; `None` where it holds no value
    /// (an action, a value the mirror does not keep).
    pub before: Option<ParamValue>,
    /// What the write sets.
    pub after: ParamValue,
}

/// What a [`WriteHook`] judges: one call's writes.
#[derive(Debug)]
pub struct WriteProposal<'a> {
    /// Who asks.
    pub actor: ActorId,
    /// The mirror before the write: the rest of a band, an output's
    /// high-pass, the volume.
    pub state: &'a DspState,
    /// What changes. A whole band shows each field it changes (`eq.type`,
    /// `eq.freq`, `eq.q`, `eq.gain` or `eq.fp`, `eq.qp`, `eq.bypass`); a
    /// whole-state write every value of the registry it changes.
    pub writes: &'a [ProposedWrite],
    /// A whole-state write (a snapshot recall, an A/B toggle, an import):
    /// every value changes at once, or none.
    pub whole: bool,
}

/// The pre-write hook of an [`ActorScope`]: `Ok` lets the write go; an error
/// refuses it before anything is sent and is what the call returns. It runs
/// under the core's lock and must not call the core.
pub type WriteHook = Arc<dyn Fn(&WriteProposal<'_>) -> Result<()> + Send + Sync>;

/// One scope on this thread.
struct Acting {
    // -- WP MCP1 -- (review RA, MCP0 item 6) which scope, so a drop removes
    // its own entry even when scopes are dropped out of order
    serial: u64,
    core: u64,
    actor: ActorId,
    hook: Option<WriteHook>,
    // -- WP ROLL1 --
    /// Attribution only ([`JobActor::attribute`]): no drag gate, no hook.
    attribute_only: bool,
}

thread_local! {
    /// The scopes alive on this thread, oldest first.
    static ACTING: RefCell<Vec<Acting>> = const { RefCell::new(Vec::new()) };
}

/// The next core's key: what a scope names its core by.
pub(crate) fn next_core_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// While alive, the calls this thread makes on its core are `actor`'s, and
/// its writes pass `hook` (module docs). Scopes nest; the newest for a core
/// counts. Not `Send`: it belongs to the thread that made it.
///
/// Thread-scoped: work the core does on its own threads (a measurement's
/// volume and signal-generator writes, the guarded apply's revert on timeout)
/// runs outside any scope, so it is journalled as [`EMBEDDER`] and skips the
/// hook, whoever started it. -- WP ROLL1 -- A job carries its starter as a
/// [`JobActor`]: the tune's guard revert from `measure_tune_tick` is
/// attributed to the actor that applied it (never refused or judged), and
/// a session's generator writes are the embedder's, unjudged, whoever ticks
/// it.
///
/// -- WP MCP1 -- (review RA, MCP0 item 6) Also:
/// - `actor` is never [`EMBEDDER`]: a scope for the app itself would only
///   skip the drag gate's exemption and the own-entry rule's freedom; debug
///   builds assert it.
/// - A thread without a scope on a core acts as [`EMBEDDER`] there, so a
///   layer that serves an agent must enter a scope around every call it makes
///   for it (the MCP server does, in one place: `dspi-mcp`'s registry).
/// - Do not hold a scope across an `.await`: on a single-threaded executor
///   other tasks' calls on that thread would be the actor's too.
/// - Dropping scopes out of order is allowed; each removes its own entry.
#[must_use = "the scope ends when it is dropped"]
pub struct ActorScope<'a> {
    core: &'a DspiCore,
    serial: u64,
    _thread: PhantomData<*const ()>,
}

impl<'a> ActorScope<'a> {
    /// Make this thread's calls on `core` `actor`'s until the scope is
    /// dropped, each write judged by `hook` first when there is one.
    pub fn enter(core: &'a DspiCore, actor: ActorId, hook: Option<WriteHook>) -> Self {
        debug_assert!(actor != EMBEDDER, "an ActorScope is for another actor");
        let key = core.lock().actor_key;
        let serial = next_scope_serial();
        ACTING.with(|a| {
            a.borrow_mut().push(Acting {
                serial,
                core: key,
                actor,
                hook,
                attribute_only: false, // -- WP ROLL1 --
            })
        });
        Self {
            core,
            serial,
            _thread: PhantomData,
        }
    }
}

/// A new scope's serial. -- WP MCP1 --
fn next_scope_serial() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Drop for ActorScope<'_> {
    /// End the scope, and wake the core's notification listener when the
    /// actor changed something the embedder has not yet been told of.
    fn drop(&mut self) {
        ACTING.with(|a| {
            let mut a = a.borrow_mut();
            if let Some(at) = a.iter().rposition(|s| s.serial == self.serial) {
                a.remove(at);
            }
        });
        let pending = self.core.lock().foreign.flags != 0;
        if pending {
            self.core.listeners.notify.call(|l| l.on_notifications());
        }
    }
}

/// The actor of this thread's calls on the core keyed `key`.
pub(crate) fn current(key: u64) -> ActorId {
    ACTING.with(|a| {
        a.borrow()
            .iter()
            .rev()
            .find(|s| s.core == key)
            .map_or(EMBEDDER, |s| s.actor)
    })
}

// -- WP ROLL1 --
/// Whether this thread's newest scope on the core keyed `key` attributes
/// only ([`JobActor::attribute`]): its writes pass the drag gate.
fn attribute_only(key: u64) -> bool {
    ACTING.with(|a| {
        a.borrow()
            .iter()
            .rev()
            .find(|s| s.core == key)
            .is_some_and(|s| s.attribute_only)
    })
}

/// The actor and hook of this thread's newest scope on the core keyed
/// `key`, when that scope has a hook.
fn hook(key: u64) -> Option<(ActorId, WriteHook)> {
    ACTING.with(|a| {
        let a = a.borrow();
        let s = a.iter().rev().find(|s| s.core == key)?;
        Some((s.actor, s.hook.clone()?))
    })
}

/// What other actors changed since the embedder last called
/// `process_notifications`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Foreign {
    /// `NOTIFY_*` bits to report.
    pub(crate) flags: u32,
    /// One bit per wire channel whose curve moved.
    pub(crate) channels: u32,
}

/// The gate every write passes ([`Inner::writable_conn`]): a write by
/// another actor is refused while the embedder's user drags, and otherwise
/// noted for the embedder (the view moves even when the write records
/// nothing).
pub(crate) fn gate(inner: &mut Inner) -> Result<()> {
    if current(inner.actor_key) == EMBEDDER {
        return Ok(());
    }
    // -- WP ROLL1 -- a job's own write (a revert on timeout) is never refused
    if inner.dragging() && !attribute_only(inner.actor_key) {
        return Err(DspiError::ConfirmationRequired {
            code: USER_EDITING.into(),
            msg: "The user is editing a control on this device; try again when they let go.".into(),
        });
    }
    inner.foreign.flags |= NOTIFY_STATE | NOTIFY_CURVES;
    Ok(())
}

/// A change by `actor` of `path` at `indices` was journalled: tell the
/// embedder when it is not its own.
pub(crate) fn noted(inner: &mut Inner, actor: ActorId, path: &str, indices: &[u8]) {
    if actor == EMBEDDER {
        return;
    }
    inner.foreign.flags |= NOTIFY_STATE | NOTIFY_CURVES | NOTIFY_HISTORY;
    let channels = u32::from(inner.state.num_input_channels)
        .saturating_add(u32::from(inner.state.num_output_channels));
    inner.foreign.channels |= match (path, indices.first()) {
        ("bulk", _) => 1u32
            .checked_shl(channels)
            .map_or(u32::MAX, |b| b.wrapping_sub(1)),
        (p, Some(&ch)) if p.starts_with("eq.") => 1u32.checked_shl(u32::from(ch)).unwrap_or(0),
        _ => 0,
    };
}

/// Let the scope's hook judge a write before it is sent; `writes` is built
/// only when there is a hook. `Ok` without one, and for a write that changes
/// nothing.
pub(crate) fn judge(
    inner: &Inner,
    whole: bool,
    writes: impl FnOnce(&Inner) -> Vec<ProposedWrite>,
) -> Result<()> {
    let Some((actor, hook)) = hook(inner.actor_key) else {
        return Ok(());
    };
    let writes = writes(inner);
    if writes.is_empty() {
        return Ok(());
    }
    hook(&WriteProposal {
        actor,
        state: &inner.state,
        writes: &writes,
        whole,
    })
}

/// [`judge`] for one value: `path` at `indices` set to `after`.
pub(crate) fn judge_value(inner: &Inner, path: &str, indices: &[u8], after: &Value) -> Result<()> {
    judge(inner, false, |inner| {
        let before = crate::journal::mirror_value(inner, path, indices);
        if before.as_ref() == Some(after) {
            return Vec::new();
        }
        vec![ProposedWrite {
            path: path.into(),
            indices: indices.to_vec(),
            before: before.map(Into::into),
            after: after.clone().into(),
        }]
    })
}

/// [`judge`] for a whole band of wire channel `ch` (PEQ or crossover band
/// number) set to `p`: each field it changes.
pub(crate) fn judge_band(inner: &Inner, ch: u8, band: u8, p: &FilterParams) -> Result<()> {
    judge(inner, false, |inner| {
        let was = crate::write_param::band(&inner.state, ch, band);
        band_writes(ch, band, was.as_ref(), p)
    })
}

/// A band field's value.
type Field = fn(&FilterParams) -> Value;

/// The fields of band `band` of wire channel `ch` that `now` changes from
/// `was` (all of them when `was` is unknown).
fn band_writes(
    ch: u8,
    band: u8,
    was: Option<&FilterParams>,
    now: &FilterParams,
) -> Vec<ProposedWrite> {
    let linkwitz = FilterType::from_raw(now.filter_type).is_linkwitz();
    let gain = if linkwitz { "eq.fp" } else { "eq.gain" };
    let fields: [(&str, Field, bool); 6] = [
        ("eq.type", |p| Value::Choice(p.filter_type), true),
        ("eq.bypass", |p| Value::Bool(p.bypass), true),
        ("eq.freq", |p| Value::Float(p.freq), true),
        ("eq.q", |p| Value::Float(p.q), true),
        (gain, |p| Value::Float(p.gain), true),
        ("eq.qp", |p| Value::Float(p.qp), linkwitz),
    ];
    fields
        .iter()
        .filter(|(_, _, shown)| *shown)
        .filter_map(|(path, get, _)| {
            let before = was.map(get);
            let after = get(now);
            (before.as_ref() != Some(&after)).then(|| ProposedWrite {
                path: (*path).into(),
                indices: vec![ch, band],
                before: before.map(Into::into),
                after: after.into(),
            })
        })
        .collect()
}

/// The most index values tried per index of a registry row in a whole-state
/// diff: more than any device's channels, outputs or bands.
const DIFF_INDEX_SPAN: u8 = 32;

/// [`judge`] for a whole-state write that takes the mirror from `before` to
/// `after`: every registry value that differs, as one proposal.
pub(crate) fn judge_whole(inner: &Inner, before: &DspState, after: &DspState) -> Result<()> {
    judge(inner, true, |inner| state_diff(inner, before, after))
}

/// Every registry value that differs between `before` and `after`, with
/// the other values (names outside the image) as `inner` holds them.
fn state_diff(inner: &Inner, before: &DspState, after: &DspState) -> Vec<ProposedWrite> {
    let conn = inner.conn.as_ref();
    let mut out = Vec::new();
    for d in REGISTRY.iter().filter(|d| d.is_writable()) {
        let tuples: Vec<Vec<u8>> = match d.target.arity() {
            0 => vec![Vec::new()],
            1 => (0..DIFF_INDEX_SPAN).map(|i| vec![i]).collect(),
            _ => (0..DIFF_INDEX_SPAN)
                .flat_map(|i| (0..DIFF_INDEX_SPAN).map(move |j| vec![i, j]))
                .collect(),
        };
        for ix in tuples {
            let was = crate::journal::state_value(before, conn, d.path, &ix);
            let Some(now) = crate::journal::state_value(after, conn, d.path, &ix) else {
                continue;
            };
            if was.as_ref() != Some(&now) {
                out.push(ProposedWrite {
                    path: d.path.into(),
                    indices: ix,
                    before: was.map(Into::into),
                    after: now.into(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "actor/tests_wp_mcp0.rs"]
mod tests_wp_mcp0;
// -- WP MCP1 --
#[cfg(test)]
#[path = "actor/tests_wp_mcp1.rs"]
mod tests_wp_mcp1;

// -- WP ROLL1 -- (review RA item 5) a job's actor
/// Who started a job the core drives from whichever thread ticks it (a
/// measurement, the roll-off search, a tune and its guard): captured on the
/// starting call's thread, carried with the job, and entered again around
/// the job's own writes on another thread.
///
/// - [`JobActor::capture`] takes this thread's actor on a core: the newest
///   [`ActorScope`]'s, or [`EMBEDDER`] outside one. The measurement calls
///   capture it themselves at their start; a layer that starts a job on one
///   thread and drives it from another needs nothing more.
/// - [`JobActor::attribute`] enters an **attribution-only** scope: the
///   History names the actor and the embedder hears of the change, but the
///   write is neither refused while the user drags nor shown to a hook. It
///   is for the job's own safety writes (a guard's revert on timeout), which
///   must never be refused. It also works for the embedder's job: a job
///   ticked on a thread inside an agent's [`ActorScope`] (an MCP server
///   polling it) is still the embedder's, unjudged. Anything the actor
///   chooses to do goes through its own calls under its own [`ActorScope`],
///   hook and all.
/// - The measurement session's generator writes (its configuration, START
///   and STOP) run under the embedder's attribution-only scope whoever
///   ticks it: a STOP must never wait for the drag gate or a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobActor(ActorId);

impl JobActor {
    /// This thread's actor on `core` (the newest scope's, or [`EMBEDDER`]).
    /// Takes the core's lock: never call it with the lock held.
    pub fn capture(core: &DspiCore) -> JobActor {
        JobActor(current(core.lock().actor_key))
    }

    /// A job of `actor` (tests and embedders that track the actor
    /// themselves).
    pub fn of(actor: ActorId) -> JobActor {
        JobActor(actor)
    }

    /// The actor: [`EMBEDDER`] for the app's own job.
    pub fn actor(self) -> ActorId {
        self.0
    }

    /// Make this thread's writes on `core` the job actor's, for attribution
    /// only, until the scope is dropped (the embedder's too: inside an
    /// agent's scope they are the embedder's again, unjudged). Takes the
    /// core's lock: never call it with the lock held.
    pub fn attribute(self, core: &DspiCore) -> ActorScope<'_> {
        let key = core.lock().actor_key;
        let serial = next_scope_serial();
        ACTING.with(|a| {
            a.borrow_mut().push(Acting {
                serial,
                core: key,
                actor: self.0,
                hook: None,
                attribute_only: true,
            })
        });
        ActorScope {
            core,
            serial,
            _thread: PhantomData,
        }
    }
}
