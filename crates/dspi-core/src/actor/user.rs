//! What the embedder's user changed, for the actors that work beside them
//! (WP AIVIS): an AI agent is told on its next call, and does not write over
//! a control the user has just taken back from it.
//!
//! - **The log.** Every change the embedder's user makes is noted here as it
//!   is journalled, whether or not the history keeps it (it outlives a preset
//!   load and a reconnect), and so is what another writer changed (a knob on
//!   the device, another console). Not the embedder's own jobs: a
//!   measurement's generator writes, a guard's revert at its timeout.
//!   [`changes_since`] reads it by cursor.
//! - **The contest rule**, beside the drag gate ([`super::USER_EDITING`]):
//!   when the user changes a control an actor wrote within [`TUNING_MS`],
//!   that actor's next write to it is refused before anything is sent, with
//!   code [`USER_CHANGED`], and so is every retry within [`ASK_FIRST_MS`] of
//!   that refusal: an agent that retries blindly does not get through. After
//!   that its writes pass again (it has had the time to ask). Only judged
//!   writes are held to it (a scope with a [`super::WriteHook`]); a revert,
//!   which no hook judges, always passes.
//!
//! In memory, per core, bounded.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use dspi_proto::generated::SECTIONS;
use dspi_proto::registry::by_path;
use dspi_proto::value::Value;

use super::{ActorId, EMBEDDER, ProposedWrite, attribute_only, current};
use crate::write_param::ParamValue;
use crate::{DspiCore, DspiError, Inner, Result};

/// [`DspiError::ConfirmationRequired`]'s code for an actor's write to a
/// control the user has changed since the actor set it: nothing was sent; ask
/// the user first.
pub const USER_CHANGED: &str = "user_changed";

/// How long after an actor's write to a control the user's change of it
/// counts as taking it back, and how long that holds the actor off.
pub const TUNING_MS: u64 = 600_000;

/// How long after a [`USER_CHANGED`] refusal the actor's retries are refused
/// too: the time to ask the user.
pub const ASK_FIRST_MS: u64 = 5_000;

/// The most entries of the log.
const LOG_CAP: usize = 256;
/// The most controls remembered as an actor's, and as contested.
const CONTROLS_CAP: usize = 256;

/// Who made a [`UserChange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangedBy {
    /// The app's user.
    User,
    /// Another writer: a control on the device, another console, the host's
    /// volume control.
    Elsewhere,
}

/// One change.
#[derive(Debug, Clone, PartialEq)]
pub struct UserChange {
    /// Its number, rising within the process; a control changed again gets a
    /// new one.
    pub seq: u64,
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// Who.
    pub by: ChangedBy,
    /// The journal's path (`out.gain`, `eq.freq`, `preset.load`, `bulk`, ...);
    /// for [`ChangedBy::Elsewhere`] the image section's name; for an act that
    /// writes no parameter `did.<act>` ([`did`]).
    pub path: String,
    /// The path's indices.
    pub indices: Vec<u8>,
    /// The value before (a drag: before it began), when known.
    pub before: Option<ParamValue>,
    /// The value after; for [`ChangedBy::Elsewhere`] who, in words.
    pub after: ParamValue,
    /// It is the user's undo (or redo) of a change.
    pub undo: bool,
    /// One step of a drag.
    drag: bool,
}

impl UserChange {
    /// The registry's name for what changed, "" for a path it does not know.
    pub fn label(&self) -> &'static str {
        by_path(&self.path).map_or("", |d| d.label)
    }
}

/// A control: a band is one, whatever field of it changes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Control {
    path: String,
    indices: Vec<u8>,
}

impl Control {
    fn of(path: &str, indices: &[u8]) -> Self {
        let path = if path.starts_with("eq.") { "eq" } else { path };
        Self {
            path: path.to_owned(),
            indices: indices.to_vec(),
        }
    }
}

/// A write of everything: every control an actor set is another since.
fn wholesale(path: &str) -> bool {
    dspi_session::history::RESETS_HISTORY.contains(&path)
}

#[derive(Debug, Clone)]
struct Touched {
    control: Control,
    actor: ActorId,
    at_ms: u64,
}

#[derive(Debug, Clone)]
struct Contest {
    control: Control,
    actor: ActorId,
    /// When the user changed it.
    at_ms: u64,
    /// When the actor was first refused for it.
    told_ms: Option<u64>,
}

/// One core's log ([`Inner`]).
#[derive(Debug, Default)]
pub(crate) struct UserLog {
    changes: VecDeque<UserChange>,
    /// The connection `touched` and `contests` are of.
    generation: u64,
    /// The log's number when a measurement last ended ([`measured_until`]).
    measure_ended: u64,
    touched: VecDeque<Touched>,
    /// Behind the core's lock like the rest; a cell because the judgement
    /// reads the core and notes only here that the actor was told.
    contests: RefCell<Vec<Contest>>,
}

/// Milliseconds since the Unix epoch, from the clock seam.
fn now_ms() -> u64 {
    dspi_transport::clock::system_time()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn next_seq() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl UserLog {
    /// Add `c`; it takes the place of the control's earlier one, and in a drag
    /// (`c.drag`) the value from before the drag.
    fn push(&mut self, mut c: UserChange) {
        let joins = c.drag;
        let same = self
            .changes
            .iter()
            .position(|o| o.by == c.by && o.path == c.path && o.indices == c.indices);
        if let Some(old) = same.and_then(|at| self.changes.remove(at))
            && joins
            && old.drag
        {
            c.before = old.before;
        }
        if self.changes.len() >= LOG_CAP {
            self.changes.pop_front();
        }
        self.changes.push_back(c);
    }

    fn touch(&mut self, control: Control, actor: ActorId, at_ms: u64) {
        self.touched
            .retain(|t| t.actor != actor || t.control != control);
        if self.touched.len() >= CONTROLS_CAP {
            self.touched.pop_front();
        }
        self.touched.push_back(Touched {
            control,
            actor,
            at_ms,
        });
    }

    /// The user changed `path` at `indices`: what an actor set there is
    /// contested from now on.
    fn contest(&mut self, path: &str, indices: &[u8], at_ms: u64) {
        let control = Control::of(path, indices);
        let all = wholesale(path);
        let contests = self.contests.get_mut();
        contests.retain(|c| at_ms.saturating_sub(c.at_ms) <= TUNING_MS);
        for t in &self.touched {
            // An actor that replaced everything set this control too
            let whole = wholesale(&t.control.path);
            let same = all || whole || t.control == control;
            if at_ms.saturating_sub(t.at_ms) > TUNING_MS || !same {
                continue;
            }
            let theirs = if whole && !all { &control } else { &t.control };
            contests.retain(|c| c.actor != t.actor || c.control != *theirs);
            if contests.len() >= CONTROLS_CAP {
                contests.remove(0);
            }
            contests.push(Contest {
                control: theirs.clone(),
                actor: t.actor,
                at_ms,
                told_ms: None,
            });
        }
        // It is the user's now; the actor's next write makes it its own again
        self.touched.retain(|t| !(all || t.control == control));
    }

    /// Whether `actor` may write `controls` at `now_ms`: `false` when one is
    /// contested, which it is told now. An expired contest is dropped.
    fn may_write(&self, actor: ActorId, controls: &[Control], now_ms: u64) -> bool {
        let mut ok = true;
        self.contests.borrow_mut().retain_mut(|c| {
            if now_ms.saturating_sub(c.at_ms) > TUNING_MS {
                return false;
            }
            if c.actor != actor || !controls.contains(&c.control) {
                return true;
            }
            match c.told_ms {
                Some(t) if now_ms.saturating_sub(t) >= ASK_FIRST_MS => false,
                told => {
                    c.told_ms = Some(told.unwrap_or(now_ms));
                    ok = false;
                    true
                }
            }
        });
        ok
    }

    /// What was set or taken back on another connection counts for nothing.
    fn on_connection(&mut self, generation: u64) {
        if self.generation != generation {
            self.generation = generation;
            self.touched.clear();
            self.contests.get_mut().clear();
        }
    }

    fn contested_by(&self, actor: ActorId) -> bool {
        self.contests.borrow().iter().any(|c| c.actor == actor)
    }
}

/// A journalled change of this thread's actor: the user's goes in the log,
/// another actor's marks the control as its.
pub(crate) fn journalled(
    inner: &mut Inner,
    path: &str,
    indices: &[u8],
    before: Option<&Value>,
    after: &Value,
    undo: bool,
) {
    if attribute_only(inner.actor_key) {
        return; // a job's own write
    }
    let at_ms = now_ms();
    let actor = current(inner.actor_key);
    let generation = inner.conn.as_ref().map_or(0, |c| c.generation);
    inner.user_log.on_connection(generation);
    if actor != EMBEDDER {
        inner
            .user_log
            .touch(Control::of(path, indices), actor, at_ms);
        return;
    }
    inner.user_log.contest(path, indices, at_ms);
    let drag = inner.dragging();
    inner.user_log.push(UserChange {
        seq: next_seq(),
        at_ms,
        by: ChangedBy::User,
        path: path.to_owned(),
        indices: indices.to_vec(),
        before: before.cloned().map(Into::into),
        after: after.clone().into(),
        undo,
        drag,
    });
}

/// Another writer (`who`, in words) changed the image at `offset`, or all of
/// it (`None`).
pub(crate) fn elsewhere(log: &mut UserLog, offset: Option<usize>, who: &str) {
    let section = offset.map_or("everything", |at| {
        SECTIONS
            .iter()
            .find(|(_, base, len)| at >= *base && at < base.saturating_add(*len))
            .map_or("a setting", |(name, _, _)| *name)
    });
    log.push(UserChange {
        seq: next_seq(),
        at_ms: now_ms(),
        by: ChangedBy::Elsewhere,
        path: section.to_owned(),
        indices: Vec::new(),
        before: None,
        after: ParamValue::Text(who.to_owned()),
        undo: false,
        drag: false,
    });
}

/// The embedder's user did `act`, which writes no parameter: `speakers` (the
/// speaker layout saved), `measure_stop`, ... Nothing when another actor, or a
/// job of the embedder's, is acting on this thread. Takes the core's lock.
pub fn did(core: &DspiCore, act: &str) {
    let mut inner = core.lock();
    if current(inner.actor_key) != EMBEDDER || attribute_only(inner.actor_key) {
        return;
    }
    inner.user_log.push(UserChange {
        seq: next_seq(),
        at_ms: now_ms(),
        by: ChangedBy::User,
        path: format!("did.{act}"),
        indices: Vec::new(),
        before: None,
        after: ParamValue::Trigger,
        undo: false,
        drag: false,
    });
}

// -- WP ADET0 --
/// [`did`] with what it changed in words (numbers and role words only, no
/// names): `after` holds the text, "" for none.
pub fn did_with(core: &DspiCore, act: &str, words: &str) {
    let mut inner = core.lock();
    if current(inner.actor_key) != EMBEDDER || attribute_only(inner.actor_key) {
        return;
    }
    push_act(&mut inner, act, words);
}

// -- WP ADET0 --
/// A job the embedder's user started did `act` (the speaker detection
/// ended), whichever thread ticks it: noted only when `actor` is the
/// embedder (an agent learns of its own job from the job).
pub(crate) fn did_as(core: &DspiCore, actor: ActorId, act: &str, words: &str) {
    if actor != EMBEDDER {
        return;
    }
    push_act(&mut core.lock(), act, words);
}

fn push_act(inner: &mut Inner, act: &str, words: &str) {
    inner.user_log.push(UserChange {
        seq: next_seq(),
        at_ms: now_ms(),
        by: ChangedBy::User,
        path: format!("did.{act}"),
        indices: Vec::new(),
        before: None,
        after: if words.is_empty() {
            ParamValue::Trigger
        } else {
            ParamValue::Text(words.to_owned())
        },
        undo: false,
        drag: false,
    });
}

/// The changes after `seq`, oldest first (at most the newest 256 are kept).
pub fn changes_since(core: &DspiCore, seq: u64) -> Vec<UserChange> {
    core.lock()
        .user_log
        .changes
        .iter()
        .filter(|c| c.seq > seq)
        .cloned()
        .collect()
}

/// A measurement session ended: what is changed from here on disturbs it no
/// more.
pub(crate) fn measure_ended(inner: &mut Inner) {
    inner.user_log.measure_ended = next_seq();
}

/// The first change of the user's that a job which began at `since` (a
/// [`latest`]) was running through, if `matters` says it disturbs the job: on
/// the same core, after `since` and before the last measurement session
/// ended, if one has since.
pub fn disturbed(
    core: &DspiCore,
    since: u64,
    matters: impl Fn(&UserChange) -> bool,
) -> Option<UserChange> {
    let running = core.measure_state().active;
    let inner = core.lock();
    let log = &inner.user_log;
    let until = if !running && log.measure_ended > since {
        log.measure_ended
    } else {
        u64::MAX
    };
    log.changes
        .iter()
        .find(|c| c.by == ChangedBy::User && c.seq > since && c.seq < until && matters(c))
        .cloned()
}

/// Whether `actor` may keep what it set: refused as [`check_plan`] is when
/// the user took back any control of it.
///
/// Errors: [`DspiError::ConfirmationRequired`] with code [`USER_CHANGED`].
pub fn check_keep(core: &DspiCore, actor: ActorId) -> Result<()> {
    let inner = core.lock();
    let controls: Vec<Control> = inner
        .user_log
        .contests
        .borrow()
        .iter()
        .filter(|c| c.actor == actor)
        .map(|c| c.control.clone())
        .collect();
    refuse(&inner, actor, &controls, now_ms())
}

/// The newest change's number on any core so far: a cursor that skips what
/// was changed before now.
pub fn latest() -> u64 {
    next_seq()
}

/// Whether `actor` may write these values (`path`, indices) now, as far as
/// the contest rule goes: an actor's plan of several writes is checked
/// whole, before the first is sent. A refusal counts as having been told.
///
/// Errors: [`DspiError::ConfirmationRequired`] with code [`USER_CHANGED`].
pub fn check_plan<'a>(
    core: &DspiCore,
    actor: ActorId,
    writes: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Result<()> {
    let controls: Vec<Control> = writes.map(|(p, ix)| Control::of(p, ix)).collect();
    let inner = core.lock();
    refuse(&inner, actor, &controls, now_ms())
}

/// The user said yes to what `actor` asked for: nothing of it is contested
/// any more.
pub fn allowed(core: &DspiCore, actor: ActorId) {
    core.lock()
        .user_log
        .contests
        .get_mut()
        .retain(|c| c.actor != actor);
}

fn refuse(inner: &Inner, actor: ActorId, controls: &[Control], now_ms: u64) -> Result<()> {
    let log = &inner.user_log;
    let generation = inner.conn.as_ref().map_or(0, |c| c.generation);
    if log.generation != generation || log.may_write(actor, controls, now_ms) {
        return Ok(());
    }
    Err(DspiError::ConfirmationRequired {
        code: USER_CHANGED.into(),
        msg: "The user changed this after you set it; ask before changing it again.".into(),
    })
}

/// The contest rule for a judged write ([`super::judge`]).
pub(super) fn contested(inner: &Inner) -> bool {
    inner.user_log.contested_by(current(inner.actor_key))
}

/// See [`contested`].
pub(super) fn gate(inner: &Inner, writes: &[ProposedWrite]) -> Result<()> {
    let controls: Vec<Control> = writes
        .iter()
        .map(|w| Control::of(&w.path, &w.indices))
        .collect();
    refuse(inner, current(inner.actor_key), &controls, now_ms())
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "tests_wp_aivis.rs"]
mod tests_wp_aivis;
