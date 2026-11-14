//! One write path through the session journal: undo, redo and history for
//! every binding (WP P0W).
//!
//! Every typed setter records the change it made in the session's journal
//! (`dspi_session::history`), the one the Terminal's `Session::write` fills:
//!
//! 1. under the core lock, before it sends, the setter notes what the mirror
//!    holds for its registry path ([`Inner::journal_begin`]), and what the
//!    device moves with it (`dspi_session::write::side_effects`: the tube's
//!    voicing, a limiter link group's settings);
//! 2. it sends and patches the mirror exactly as before: no read is added,
//!    so a slider still costs one transfer;
//! 3. once the mirror is patched it records the change with the mirror's new
//!    value ([`Pending::done`]). A change that moved nothing is not recorded.
//!
//! Undo and redo ([`DspiCore::undo`], [`DspiCore::redo`]) replay an entry
//! through [`DspiCore::write_param`], so a reversal takes the typed setter's
//! checks, clamps, device lock and mirror update, and is journalled in turn
//! (source `undo` or `redo`) without becoming a new step.
//!
//! What may be undone is the registry's word (`history::undoable`): only a
//! parameter with no hazard. A flash write, an audible one (an output enable,
//! the signal generator, a preset load), a reconfiguration (pins, clocks, the
//! input source, a bulk image), a deferred control-surface write and an
//! irreversible one are recorded and stepped over. A preset load, a bulk
//! image and a factory reset also end the history before them.
//!
//! The journal belongs to the connection's session: a reconnect (an unplug,
//! a DSPi Link re-attach, another device) starts with none, and a replay that
//! finds a different connection than the one its entry came from is refused
//! ([`DspiError::Disconnected`]) before anything is sent. Nothing is replayed
//! by itself, ever: a write lost with a link is gone, and undo only acts when
//! called.
//!
//! Over DSPi Link an undo is a write like any other: it goes to the hub on
//! the client's session, the hub orders and attributes it to that session as
//! it does every write, and other clients see the device's change as a
//! foreign one (their own journals do not hold it). The journal records this
//! client's changes only.
//!
//! When someone else has changed a value since (another Link client, a knob,
//! the UART, the computer's volume), the mirror no longer holds what this
//! client left there. Undo and redo then leave their value alone, drop the
//! entry and name the other writer in [`HistoryMove::note`], from the
//! notification's source and Link origin (owner, 2026-11-12). A value the
//! mirror holds no single copy of (a limiter write to every output) is not
//! judged.
//!
//! A re-read of the device ([`DspiCore::fetch_all`], on a refresh or a
//! reconnect of a Link session) puts the device's values in the mirror.
//! Entries it contradicts are judged by the same rule when undo or redo
//! reaches them: left alone, dropped, the writer named ("another writer" when
//! only the re-read saw it). The re-read also ends the step a drag gathers,
//! so a change after it is undone to the value the re-read found, never to
//! one from before it (WP SWAPFIX).
//!
//! Only a continuous drag gathers a step: the client marks it with
//! [`DspiCore::set_dragging`], and a typed value or a discrete write is a
//! step of its own however soon it follows another (WP QTGAP1). Everything
//! one drag writes is one step, across bands and controls, and undo puts
//! every value it moved back as it was when the drag began; a band is held
//! once, whatever fields the drag moves (WP P0W2). A value someone else has
//! changed since is judged on its own: it is left alone and its writer
//! named, the others go back.
//!
//! A compound edit that is not a drag (a linked pair's two trims, a Clear
//! All, a routing preset) is one step through an undo group (WP P1UG):
//! [`DspiCore::begin_group`] with a label, then the writes, then
//! [`DspiCore::end_group`]. Everything written between them that undo can
//! reverse is one step carrying the label, as a drag is; groups nest, and
//! only the outermost begin and end count. A group ends with its
//! connection (a disconnect, another device opened, a re-attach), with a
//! re-read ([`DspiCore::fetch_all`]) and with a change that ends the history
//! (a preset load, a bulk image, a factory reset), as a drag does. An empty
//! group records nothing. A write undo cannot reverse stands apart and ends
//! the step, as in a drag; the group's later writes gather a step of their
//! own. Another writer's change during a group does not split it; a value
//! the group writes again after another writer's change starts a new step,
//! so undo never goes back past their value. Undo judges each value of the
//! step on its own: one someone else has changed since is left alone and
//! its writer named, the others go back.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::cell::Cell;

use dspi_proto::FilterType;
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::registry::{ALL_OUTPUTS, Hazard, by_path};
use dspi_proto::value::Value;
use dspi_session::history::{Change, ChangeSource, hazard_of};
use dspi_session::write::{Companion, JournalEntry, StepValue, side_effects};

use crate::records::{DspState, FilterParams};
use crate::write_param::{ParamValue, band, fx_mirror};
use crate::{DspiCore, DspiError, Inner, Result};

/// The journal's name for a whole band written at once (a type change, an
/// import): its value is [`band_bytes`]. The registry addresses a band's
/// fields one by one; a change of one field is journalled under that field's
/// path.
pub(crate) const EQ_BAND: &str = "eq.band";

/// The journal's name for the extension channel's crossfeed pair map
/// (`set_crossfeed_pair_map`): four bytes, as the binding takes them.
pub(crate) const EXT_PAIRMAP: &str = "ext.pairmap";

// -- WP SW1 (fixup sw1, F1) -- the audio link's writes (`alink.rs`)
/// The audio link slots, 8 bytes (`set_alink_slots`): undoable.
pub(crate) const ALINK_SLOTS: &str = "ext.alink.slots";
/// The phase-lock mode and delay, `[mode, d_local LE, hops_offset LE]`
/// (`set_alink_phase`): recorded, never undone (it re-anchors).
pub(crate) const ALINK_PHASE: &str = "ext.alink.phase";
/// An apply-at-frame op (`apply_at_frame`): op and arg as indices, the
/// request's 16 bytes as the value. An action.
pub(crate) const ALINK_APPLY: &str = "ext.alink.apply";
/// The counters cleared (`reset_alink_stats`). An action.
pub(crate) const ALINK_STATS_RESET: &str = "ext.alink.stats_reset";
// -- WP AUSB1 -- the USB-direct block's writes (`alink_usb.rs`)
/// The USB-direct mode and D, `[mode, flags, d LE]` (`set_alink_usb`):
/// recorded, never undone (it re-anchors).
pub(crate) const ALINK_USB: &str = "ext.alink.usb";
/// The USB-direct diagnostics cleared (`clear_alink_usb_diag`). An action.
pub(crate) const ALINK_USB_DIAG_CLEAR: &str = "ext.alink.usb.diag_clear";

/// The first crossover band on the wire (`XOVER_BAND_BASE`, 20).
const XOVER_BASE: u8 = {
    assert!(dspi_proto::generated::filter::XOVER_BAND_BASE <= u8::MAX as u16);
    dspi_proto::generated::filter::XOVER_BAND_BASE as u8
};

/// The journal's own names for changes the registry has no path for, and
/// their hazards. Only a whole band is undone; the rest are recorded.
const OWN_PATHS: &[(&str, Hazard)] = &[
    (EQ_BAND, Hazard::None),
    // The extension channel's crossfeed pair map: live routing, as a
    // crossfeed setting is.
    (EXT_PAIRMAP, Hazard::None),
    // The core-voltage requests: never sent again.
    ("ext.vreg", Hazard::Irreversible),
    // A raw vendor request: nobody knows what it moved.
    ("raw", Hazard::Irreversible),
    // -- WP SW1 (fixup sw1, F1) -- the audio link: slots are live routing;
    // a phase change re-anchors (a short silence); apply and the counter
    // reset are actions.
    (ALINK_SLOTS, Hazard::None),
    (ALINK_PHASE, Hazard::Audible),
    (ALINK_APPLY, Hazard::Irreversible),
    (ALINK_STATS_RESET, Hazard::Irreversible),
    // -- WP AUSB1 -- a USB-direct mode or D change re-anchors; a clear is an action
    (ALINK_USB, Hazard::Audible),
    (ALINK_USB_DIAG_CLEAR, Hazard::Irreversible),
];

/// The hazard of a journal path: the registry's, or one of [`OWN_PATHS`].
/// The control surfaces' paths count as deferred whatever their row says:
/// their editor writes them (`cs_apply`), and `write_param` refuses them.
fn hazard(path: &str) -> Hazard {
    if path.starts_with("cs.") {
        return Hazard::Deferred;
    }
    OWN_PATHS
        .iter()
        .find(|(p, _)| *p == path)
        .map_or_else(|| hazard_of(path), |(_, h)| *h)
}

/// A replay in progress on this thread: undo or redo, and the connection
/// its entry came from.
#[derive(Debug, Clone, Copy)]
struct Replay {
    source: ChangeSource,
    generation: u64,
}

thread_local! {
    /// Set by [`DspiCore::undo`] and [`DspiCore::redo`] around the setter
    /// call that replays an entry. The setter runs on the same thread, so
    /// the record it makes knows itself for a replay, and a write from
    /// another thread meanwhile does not.
    static REPLAY: Cell<Option<Replay>> = const { Cell::new(None) };
}

/// The replay mark for the life of a replay step: cleared on drop, so a
/// panic inside the setter cannot leave the thread marked.
struct ReplayMark;

impl ReplayMark {
    fn set(r: Replay) -> Self {
        REPLAY.set(Some(r));
        Self
    }
}

impl Drop for ReplayMark {
    fn drop(&mut self) {
        REPLAY.set(None);
    }
}

/// Who last changed the device besides this client, for undo's
/// foreign-change rule (owner 2026-11-12: a value someone else has
/// overwritten since is left as they set it, the entry dropped and the
/// writer named): milliseconds since the Unix epoch and the writer in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignWrite {
    pub(crate) at_ms: u64,
    pub(crate) writer: String,
}

/// Note a change another writer made (a notification that was not this
/// client's echo), on the connection.
pub(crate) fn note_foreign(
    conn: &mut crate::Connection,
    source: dspi_session::Source,
    origin: Option<u16>,
) {
    use dspi_session::Source as S;
    let writer = match (origin, source) {
        (Some(sid), _) if sid != 0 => format!("DSPi Link session {sid}"),
        (_, S::Gpio) => "a control surface".into(),
        // -- WP QPI4 -- no host-device noun (GLOSSARY.md §1)
        (_, S::Uac1) => "the USB audio volume control".into(),
        (_, S::Uart) => "the UART".into(),
        (_, S::I2c) => "the I2C bus".into(),
        (_, S::Midi) => "MIDI".into(),
        (_, S::Preset) => "a preset load".into(),
        (_, S::Factory) => "a factory reset".into(),
        (_, S::HostSet | S::BulkSet) => "another app".into(),
        _ => "another writer".into(),
    };
    let at_ms = now_ms();
    conn.last_foreign = Some(ForeignWrite { at_ms, writer });
    // -- WP P1UG -- inside a group the step goes on; a value the group
    // writes again after this starts a new one (`split_regroup`)
    if conn.undo_group.is_open() {
        conn.undo_group.foreign_since = true;
        return;
    }
    // -- WP SWAPFIX -- a change of ours after theirs is a step of its own
    if let Ok(session) = conn.session() {
        session.end_step();
    }
}

// -- WP P1UG --
/// The undo group open on a connection ([`DspiCore::begin_group`]): how
/// deep, the outermost label, and whether another writer changed the device
/// since the step began.
#[derive(Debug, Default)]
pub(crate) struct UndoGroup {
    depth: u32,
    label: String,
    foreign_since: bool,
}

impl UndoGroup {
    /// Whether a group is open.
    pub(crate) fn is_open(&self) -> bool {
        self.depth > 0
    }
}

/// End the group open on `conn`, whatever its depth, and the step it
/// gathers. Nothing when none is open.
fn end_undo_group(conn: &mut crate::Connection) {
    if !conn.undo_group.is_open() {
        return;
    }
    conn.undo_group = UndoGroup::default();
    if let Ok(session) = conn.session() {
        session.end_step();
    }
}

/// Whether journal paths `a` and `b` name one value: the same path, or two
/// of a band's ([`BAND_FIELDS`] and [`EQ_BAND`]).
fn same_value(a: &str, b: &str) -> bool {
    let of_band = |p: &str| p == EQ_BAND || BAND_FIELDS.contains(&p);
    a == b || (of_band(a) && of_band(b))
}

/// Inside a group, after another writer's change: when `change` writes a
/// value the step being gathered already holds, end the step first, so
/// undo of the new step goes back to the other writer's value and never
/// past it (WP SWAPFIX's rule).
fn split_regroup(conn: &mut crate::Connection, change: &Change) {
    if !conn.undo_group.foreign_since {
        return;
    }
    let Ok(session) = conn.session() else {
        return;
    };
    // No step gathered: this change starts one after theirs anyway.
    let held = session.gathering(change.source).map(|step| {
        step.values()
            .iter()
            .any(|v| v.indices == change.indices && same_value(v.path, change.path))
    });
    match held {
        Some(false) => {}
        Some(true) => {
            session.end_step();
            conn.undo_group.foreign_since = false;
        }
        None => conn.undo_group.foreign_since = false,
    }
}

// -- WP SWAPFIX --
/// A re-read of the device ([`DspiCore::fetch_all`]): the journal's step
/// ends, so a change after it never joins one from before it; and when the
/// device held something other than the mirror (`changed`), someone this
/// client did not hear from changed it, named as "another writer" for
/// undo's foreign-change rule.
pub(crate) fn note_reread(conn: &mut crate::Connection, changed: bool) {
    if changed {
        conn.last_foreign = Some(ForeignWrite {
            at_ms: now_ms(),
            writer: "another writer".into(),
        });
    }
    // -- WP P1UG -- a re-read ends a group as it ends a drag
    end_undo_group(conn);
    if let Ok(session) = conn.session() {
        session.end_step();
    }
}

/// Whether someone else changed `path` at `indices` since this client left
/// it at `expected` (in a step begun at `at_ms`: WP P1UG, a group's step
/// goes on after another writer's change), and who, in words;
/// `None` when the mirror still holds it (or holds no single value for it).
fn overwritten(
    inner: &Inner,
    path: &'static str,
    indices: &[u8],
    at_ms: u64,
    expected: &Value,
) -> Option<String> {
    let now = mirror_value(inner, path, indices)?;
    if &now == expected {
        return None;
    }
    // -- WP P0WECHO -- a value this session wrote itself not long ago: a late
    // own echo patched it back into the mirror, not another writer.
    if inner
        .conn
        .as_ref()
        .is_some_and(|c| c.own_writes.explains(path, indices, &now, now_ms()))
    {
        return None;
    }
    let who = inner
        .conn
        .as_ref()
        .and_then(|c| c.last_foreign.as_ref())
        .filter(|f| f.at_ms >= at_ms)
        .map_or("another writer", |f| f.writer.as_str());
    // -- WP QPI4 -- the registry's label, not the path in backticks; -- WP
    // P0W2 (O1) -- a whole band by the name the clients give it
    let what = match path {
        EQ_BAND => "Filter band",
        _ => by_path(path).map_or(path, |d| d.label),
    };
    Some(format!(
        "{what} was changed by {who} since; it was left as they set it"
    ))
}

// -- WP P0WECHO --
/// How long a value this session wrote can still come back as its own echo.
/// Echoes come within milliseconds; a loaded host or a slow Link can hold
/// them for seconds, so this is generous.
const OWN_ECHO_MS: u64 = 30_000;

/// The most own writes remembered; the oldest go first.
const OWN_WRITES_CAP: usize = 64;

/// Milliseconds since the Unix epoch, from the clock seam.
fn now_ms() -> u64 {
    dspi_transport::clock::system_time()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The values this session wrote recently, per journal path and indices:
/// what its own echoes, which the mirror takes in on purpose (they correct a
/// value the firmware adjusted), can still bring back. A difference between
/// the mirror and the journal that one of these explains is not another
/// writer's (undo's foreign-change rule). Bounded by count and age; the
/// firmware merges echoes, so they are not counted off one by one.
#[derive(Debug, Default)]
pub(crate) struct OwnWrites(std::collections::VecDeque<OwnWrite>);

#[derive(Debug)]
struct OwnWrite {
    path: &'static str,
    indices: Vec<u8>,
    value: Value,
    at_ms: u64,
}

impl OwnWrites {
    /// Note that this session wrote `value` to `path` at `indices`.
    fn note(&mut self, path: &'static str, indices: &[u8], value: &Value, at_ms: u64) {
        if self.0.len() >= OWN_WRITES_CAP {
            self.0.pop_front();
        }
        self.0.push_back(OwnWrite {
            path,
            indices: indices.to_vec(),
            value: value.clone(),
            at_ms,
        });
    }

    /// Whether `value` at `path`/`indices` is one this session wrote within
    /// [`OWN_ECHO_MS`] of `now_ms` and has written over since: a late echo
    /// brings back an older value. The newest one is what this session
    /// left there, by a setter or by undo putting someone else's value back
    /// (WP SWAPFIX), so it explains no difference.
    fn explains(&self, path: &str, indices: &[u8], value: &Value, now_ms: u64) -> bool {
        let mut mine = self
            .0
            .iter()
            .rev()
            .filter(|w| w.path == path && w.indices == indices);
        if mine.next().is_none_or(|newest| &newest.value == value) {
            return false;
        }
        mine.any(|w| &w.value == value && now_ms.saturating_sub(w.at_ms) <= OWN_ECHO_MS)
    }
}

/// Refuse a replay on a connection other than the one its entry came from.
/// `Ok` outside a replay.
pub(crate) fn check_replay(inner: &Inner) -> Result<()> {
    let Some(r) = REPLAY.get() else {
        return Ok(());
    };
    match &inner.conn {
        Some(c) if c.generation == r.generation => Ok(()),
        _ => Err(DspiError::Disconnected),
    }
}

/// The source of the replay in progress on this thread, if any.
pub(crate) fn replay_source() -> Option<ChangeSource> {
    REPLAY.get().map(|r| r.source)
}

/// A change a setter is making: what the mirror held before.
#[must_use = "a change is recorded by `done`"]
pub(crate) struct Pending {
    path: &'static str,
    indices: Vec<u8>,
    before: Option<Value>,
    companions: Vec<Companion>,
}

impl Inner {
    // -- WP P0W2 (fixup p0w2b) --
    /// Whether a drag is marked on the connection open now: a mark made on
    /// another (one that has gone, or before a re-attach) counts for nothing.
    pub(crate) fn dragging(&self) -> bool {
        self.dragging
            .is_some_and(|g| self.conn.as_ref().is_some_and(|c| c.generation == g))
    }

    /// Begin a journalled change of `path` at `indices`: note what the
    /// mirror holds for it and for what the device moves with it.
    /// [`DspiError::Disconnected`] for a replay whose connection has gone.
    pub(crate) fn journal_begin(&self, path: &'static str, indices: &[u8]) -> Result<Pending> {
        check_replay(self)?;
        let (inputs, outputs) = (
            self.state.num_input_channels,
            self.state.num_output_channels,
        );
        let companions = side_effects(path, indices, inputs, outputs)
            .into_iter()
            .filter_map(|(path, indices)| {
                let before = mirror_value(self, path, &indices)?;
                Some(Companion {
                    path,
                    indices,
                    before,
                })
            })
            .collect();
        Ok(Pending {
            path,
            indices: indices.to_vec(),
            before: mirror_value(self, path, indices),
            companions,
        })
    }
}

impl Pending {
    /// A change of `path` at `indices` whose value before the caller read
    /// itself, with nothing moving besides it.
    pub(crate) fn of(path: &'static str, indices: &[u8], before: Option<Value>) -> Self {
        Self {
            path,
            indices: indices.to_vec(),
            before,
            companions: Vec::new(),
        }
    }

    /// Record the change with the mirror's value now, or `after` where the
    /// mirror holds none.
    pub(crate) fn done(self, inner: &mut Inner, after: Value) {
        let after = mirror_value(inner, self.path, &self.indices).unwrap_or(after);
        self.done_with(inner, after);
    }

    /// Record the change with `after` as its new value. Nothing is recorded
    /// when nothing moved (neither the value nor what moves with it), except
    /// for an undo's or a redo's replay.
    pub(crate) fn done_with(self, inner: &mut Inner, after: Value) {
        let moved = self.before.as_ref() != Some(&after)
            || self
                .companions
                .iter()
                .any(|c| mirror_value(inner, c.path, &c.indices).as_ref() != Some(&c.before));
        // -- WP P0WECHO -- a replay is recorded even when the mirror already
        // showed its value (a late own echo can put it there first): it was
        // sent, and the history says so.
        if !moved && replay_source().is_none() {
            return;
        }
        // Only a client's continuous drag joins: a typed value or a key step
        // is a step of its own however soon it follows (WP QTGAP1). Within
        // one drag every value undo can put back joins, whatever it is (WP
        // P0W2, O1); one undo cannot reverse (a hazard, an action) stays
        // apart.
        // -- WP P1UG -- an undo group gathers as a drag does
        let gathering =
            inner.dragging() || inner.conn.as_ref().is_some_and(|c| c.undo_group.is_open());
        let coalesce = gathering && hazard(self.path) == Hazard::None && after != Value::Trigger;
        record(
            inner,
            Change {
                path: self.path,
                indices: self.indices,
                before: self.before,
                after,
                companions: self.companions,
                hazard: hazard(self.path),
                source: replay_source().unwrap_or(ChangeSource::Local),
                coalesce,
            },
        );
    }
}

/// Record a change with no value before it to restore (an action, a flash
/// write, a reconfiguration): history, never undone.
pub(crate) fn journal(inner: &mut Inner, path: &'static str, indices: &[u8], after: Value) {
    record(
        inner,
        Change {
            path,
            indices: indices.to_vec(),
            before: None,
            after,
            companions: Vec::new(),
            hazard: hazard(path),
            source: replay_source().unwrap_or(ChangeSource::Local),
            coalesce: false,
        },
    );
}

/// Put `change` in the open session's journal; nowhere without one.
fn record(inner: &mut Inner, change: Change) {
    // -- WP P1UG -- a change that ends the history ends the group too; inside
    // a group, a value written again after another writer's change starts a
    // new step (before a band's paths are joined, so the new step holds it)
    if let Some(conn) = inner.conn.as_mut() {
        if dspi_session::history::RESETS_HISTORY.contains(&change.path) {
            end_undo_group(conn);
        }
        if conn.undo_group.is_open() && change.source == ChangeSource::Local && change.coalesce {
            split_regroup(conn, &change);
        }
    }
    let change = join_band_step(inner, change);
    let Some(conn) = inner.conn.as_mut() else {
        return;
    };
    // -- WP P0WECHO -- its echo may still be on the way
    conn.own_writes
        .note(change.path, &change.indices, &change.after, now_ms());
    // -- WP P1UG -- a fresh change inside a group carries its label
    let label = (conn.undo_group.is_open() && change.source == ChangeSource::Local)
        .then(|| conn.undo_group.label.clone());
    // An empty label names nothing; the writes still form one step.
    let label = label.filter(|l| !l.is_empty());
    if let Ok(session) = conn.session() {
        match label {
            Some(l) => session.record_in_group(change, &l),
            None => session.record_change(change),
        }
    }
}

// -- WP P0W2 --
/// A band's live fields as the journal names them: what a graph handle or a
/// slider drags. A change of one is journalled under its own path; while a
/// drag is marked, one that joins a step of another field (or of the whole
/// band) turns the step into a whole-band one ([`join_band_step`]).
const BAND_FIELDS: &[&str] = &["eq.freq", "eq.q", "eq.gain", "eq.fp", "eq.qp"];

/// `p` with field `path` set to `v`; `None` for anything but a band field's
/// number.
fn with_field(mut p: FilterParams, path: &str, v: &Value) -> Option<FilterParams> {
    let Value::Float(x) = *v else {
        return None;
    };
    match path {
        "eq.freq" => p.freq = x,
        "eq.q" => p.q = x,
        "eq.gain" | "eq.fp" => p.gain = x,
        "eq.qp" => p.qp = x,
        _ => return None,
    }
    Some(p)
}

/// A band is held once in a drag's step, whatever fields the drag moves (WP
/// P0W2). A whole-band write is journalled under the one field it moved, or
/// as [`EQ_BAND`] when it moved more (`band_change`), so a graph drag that
/// alternates one-axis and two-axis moves alternates paths. When `change`,
/// marked to coalesce, is of a band the step being gathered already holds
/// under another of the band's paths, it is journalled as [`EQ_BAND`] (the
/// band before and after it), and a value still under one field is widened
/// to [`EQ_BAND`] with the band as it was when the drag began: that field's
/// value before, the rest as they were before this change (only that field
/// moved during the step, or it would not be one field's). Anything else,
/// and anything it cannot rebuild, is left as it was.
fn join_band_step(inner: &mut Inner, change: Change) -> Change {
    let of_band = |p: &str| p == EQ_BAND || BAND_FIELDS.contains(&p);
    if !change.coalesce || !of_band(change.path) {
        return change;
    }
    let (Some(&ch), Some(&b), 2) = (
        change.indices.first(),
        change.indices.get(1),
        change.indices.len(),
    ) else {
        return change;
    };
    let Some(now) = band(&inner.state, ch, b) else {
        return change;
    };
    let Some(session) = inner.conn.as_mut().and_then(|c| c.session().ok()) else {
        return change;
    };
    let Some(held) = session.gathering(change.source).and_then(|step| {
        step.values()
            .into_iter()
            .find(|v| v.indices == change.indices && of_band(v.path))
    }) else {
        return change;
    };
    if held.path == change.path {
        return change;
    }
    // The band before this change.
    let was = match (change.path, &change.before) {
        (EQ_BAND, Some(Value::Bytes(bytes))) => band_from_bytes(bytes),
        (field, Some(v)) => with_field(now, field, v),
        _ => None,
    };
    let Some(was) = was else {
        return change;
    };
    if held.path != EQ_BAND {
        let Some(start) = held
            .before
            .as_ref()
            .and_then(|v| with_field(was, held.path, v))
        else {
            return change;
        };
        session.widen_step(
            held.path,
            &change.indices,
            EQ_BAND,
            Some(Value::Bytes(band_bytes(&start))),
        );
    }
    Change {
        path: EQ_BAND,
        before: Some(Value::Bytes(band_bytes(&was))),
        after: Value::Bytes(band_bytes(&now)),
        ..change
    }
}

/// The journal's path and values for a whole-band write, from the band as it
/// was and as it is: one field's path when only that field moved, otherwise
/// [`EQ_BAND`] with [`band_bytes`].
pub(crate) fn band_change(was: &FilterParams, now: &FilterParams) -> &'static str {
    let fields = [
        (was.bypass != now.bypass, "eq.bypass"),
        (was.freq != now.freq, "eq.freq"),
        (was.q != now.q, "eq.q"),
        (
            was.gain != now.gain,
            if FilterType::from_raw(now.filter_type).is_linkwitz() {
                "eq.fp"
            } else {
                "eq.gain"
            },
        ),
        (
            was.qp != now.qp && FilterType::from_raw(now.filter_type).is_linkwitz(),
            "eq.qp",
        ),
    ];
    let mut moved = fields.iter().filter(|(m, _)| *m).map(|(_, p)| *p);
    match (
        was.filter_type == now.filter_type,
        moved.next(),
        moved.next(),
    ) {
        (true, Some(one), None) => one,
        _ => EQ_BAND,
    }
}

/// A band as the journal holds it: type, bypass, then freq, Q, gain and the
/// Linkwitz target Q as little-endian `f32`s (18 bytes).
pub(crate) fn band_bytes(p: &FilterParams) -> Vec<u8> {
    let mut v = vec![p.filter_type, u8::from(p.bypass)];
    for f in [p.freq, p.q, p.gain, p.qp] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v
}

/// [`band_bytes`] read back; `None` for anything else.
pub(crate) fn band_from_bytes(b: &[u8]) -> Option<FilterParams> {
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

fn at<T: Clone>(v: &[T], i: u8) -> Option<T> {
    v.get(usize::from(i)).cloned()
}

/// What the mirror holds for journal `path` at `indices`, as the registry's
/// kind; `None` for an action, a value the mirror does not hold, or an index
/// it does not have. "Every output" (`ALL_OUTPUTS`) has no single value.
#[allow(clippy::too_many_lines)] // one arm per path, nothing to factor
pub(crate) fn mirror_value(inner: &Inner, path: &str, ix: &[u8]) -> Option<Value> {
    let s: &DspState = &inner.state;
    let i0 = ix.first().copied().unwrap_or(0);
    let i1 = ix.get(1).copied().unwrap_or(0);
    let f = |x: f32| Some(Value::Float(x));
    let b = |x: bool| Some(Value::Bool(x));
    let c = |x: u8| Some(Value::Choice(x));
    let n = |x: u8| Some(Value::Int(x.into()));
    let m = |x: u32| Some(Value::Mask(x));
    if let Some((fx, id)) = crate::params_fx::fx_param(path) {
        if i0 == ALL_OUTPUTS && path.starts_with("limit.") {
            return None;
        }
        let x = fx_mirror(s, fx, id, i0)?;
        let kind = by_path(path)?.kind;
        return Some(crate::write_param::as_kind(kind, x).into());
    }
    match path {
        "vol.user" => f(s.user_volume_db),
        "vol.mute" => b(s.user_mute),
        "vol.master" => f(s.master_volume_db),
        "vol.master.mode" => c(s.master_volume_mode),
        "pre" => at(&s.input_preamp_db, i0).map(Value::Float),
        "pre.all" => f(s.preamp_db),
        "bypass" => b(s.bypass),
        "eq.type" => band(s, i0, i1).map(|p| Value::Choice(p.filter_type)),
        "eq.freq" => band(s, i0, i1).map(|p| Value::Float(p.freq)),
        "eq.q" => band(s, i0, i1).map(|p| Value::Float(p.q)),
        "eq.gain" | "eq.fp" => band(s, i0, i1).map(|p| Value::Float(p.gain)),
        "eq.qp" => band(s, i0, i1).map(|p| Value::Float(p.qp)),
        "eq.bypass" => band(s, i0, i1).map(|p| Value::Bool(p.bypass)),
        EQ_BAND => band(s, i0, i1).map(|p| Value::Bytes(band_bytes(&p))),
        "ch.delay" => at(&s.channel_delays, i0).map(Value::Float),
        "ch.name" => at(&s.channel_names, i0).map(Value::Text),
        "mix" => {
            let cell = |v: &Vec<Vec<bool>>| v.get(usize::from(i0))?.get(usize::from(i1)).copied();
            let gain = s
                .matrix_gain
                .get(usize::from(i0))?
                .get(usize::from(i1))
                .copied()?;
            let p = MatrixRoutePacket {
                input: i0,
                output: i1,
                enabled: cell(&s.matrix_routing)?,
                phase_invert: cell(&s.matrix_invert)?,
                gain_db: gain,
            };
            Some(Value::Bytes(p.encode().to_vec()))
        }
        "out.enable" => at(&s.output_enabled, i0).map(Value::Bool),
        "out.gain" => at(&s.output_gain_db, i0).map(Value::Float),
        "out.mute" => at(&s.output_muted, i0).map(Value::Bool),
        "out.delay" => at(&s.output_delay_ms, i0).map(Value::Float),
        "out.pin" => at(&s.output_pins, i0).map(|p| Value::Int(p.into())),
        "out.type" => at(&s.output_types, i0).map(Value::Choice),
        "loud.on" => b(s.loudness_enabled),
        "loud.ref" => f(s.loudness_ref_spl),
        "loud.intensity" => f(s.loudness_intensity),
        "loud.mask" => m(s.loudness_output_mask.into()),
        "lev.on" => b(s.leveller_enabled),
        "lev.amount" => f(s.leveller_amount),
        "lev.speed" => c(s.leveller_speed),
        "lev.maxgain" => f(s.leveller_max_gain_db),
        "lev.lookahead" => b(s.leveller_lookahead),
        "lev.gate" => f(s.leveller_gate_db),
        "lev.masks" => {
            m(u32::from(s.leveller_detector_mask)
                | u32::from(s.leveller_apply_mask).wrapping_shl(8))
        }
        "cf.on" => b(s.crossfeed_enabled),
        "cf.preset" => c(s.crossfeed_preset),
        "cf.freq" => f(s.crossfeed_freq),
        "cf.feed" => f(s.crossfeed_feed),
        "cf.itd" => b(s.crossfeed_itd),
        "cf.outputs" => m(s.crossfeed_output_pair_mask.into()),
        "bass.on" => b(s.psybass_enabled),
        "bass.mask" => m(s.psybass_output_mask.into()),
        "sub.solo" => b(s.subharm_solo),
        "in.source" => c(s.input_source),
        "in.name" => inner
            .conn
            .as_ref()
            .and_then(|c| at(&c.source_names, i0))
            .map(Value::Text),
        "in.rate" => Some(Value::Int(match s.i2s_input_rate {
            0 => 44_100,
            2 => 96_000,
            _ => 48_000,
        })),
        "in.spdif.pin" => at(&s.spdif_rx_pins, i0).map(|p| Value::Int(p.into())),
        "in.spdif.enable" => {
            let bit = 1u8.checked_shl(u32::from(i0))?;
            b(s.spdif_inputs_enabled & bit != 0)
        }
        "in.i2s.pin" => at(&s.i2s_rx_pins, i0).map(|p| Value::Int(p.into())),
        "in.i2s.channels" => n(s.i2s_input_channels),
        "in.i2s.clock" => c(s.i2s_clock_mode),
        "in.adat.enable" => b(s.adat_input_enabled),
        "in.adat.pin" => n(s.adat_input_pin),
        "in.adat.clock" => c(s.adat_input_clock_mode),
        "in.lg" => b(s.lg_sound_sync_enabled),
        "i2s.bck" => n(s.i2s_bck_pin),
        "i2s.bck.slave" => n(s.i2s_bck_pin_slave),
        "i2s.clockpins" => c(s.i2s_clock_pin_mode),
        "i2s.mck" => b(s.mck_enabled),
        "i2s.mck.pin" => n(s.mck_pin),
        "i2s.mck.mult" => c(s.mck_multiplier),
        "adat.enable" => b(s.adat_out_enabled),
        "adat.pin" => n(s.adat_out_pin),
        "dev.name" => inner
            .conn
            .as_ref()
            .map(|c| Value::Text(c.device_name.clone())),
        "preset.name" => at(&s.preset_names, i0).map(Value::Text),
        "preset.startup" => Some(Value::Bytes(vec![
            s.preset_startup_mode,
            s.preset_default_slot,
        ])),
        "preset.iomode" => c(s.output_config_mode),
        EXT_PAIRMAP => inner
            .conn
            .as_ref()
            .and_then(|c| c.ext.pair_map)
            .map(|m| Value::Bytes(m.0.to_vec())),
        _ => None,
    }
}

// ── The bindings' view ──────────────────────────────────────────────

/// Who made a change in the history.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistorySource {
    /// A write this program asked for.
    Local,
    /// The reversal of an earlier change, sent by [`DspiCore::undo`].
    Undo,
    /// The repeat of an undone change, sent by [`DspiCore::redo`].
    Redo,
}

impl From<ChangeSource> for HistorySource {
    fn from(s: ChangeSource) -> Self {
        match s {
            ChangeSource::Local => Self::Local,
            ChangeSource::Undo => Self::Undo,
            ChangeSource::Redo => Self::Redo,
        }
    }
}

/// One change in the history ([`DspiCore::history`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// The registry path (`out.gain`, `eq.freq`, ...), or one of the
    /// journal's own: `eq.band` (a whole band: type, bypass, then freq, Q,
    /// gain and the Linkwitz target Q as little-endian `f32`s), `ext.pairmap`
    /// (the crossfeed pair map's four bytes), `ext.vreg` (a core-voltage
    /// request: op and value), `raw` (a raw request: opcode and `wValue` as
    /// indices, the payload as the value), `ext.alink.slots` (the 8
    /// slot bytes), `ext.alink.phase` (mode, then the delay and the
    /// hops offset as little-endian 16-bit values), `ext.alink.apply`
    /// (op and arg as indices, the 16-byte request as the value) and
    /// `ext.alink.stats_reset`. A control-surface operation is
    /// under its `cs.*` path with the operation's JSON as text.
    pub path: String,
    /// The path's indices (an output, a channel and a band, ...).
    pub indices: Vec<u8>,
    /// The value before, when known; undo writes it back.
    pub before: Option<ParamValue>,
    /// The value after (`trigger` for an action).
    pub after: ParamValue,
    /// Who made it.
    pub source: HistorySource,
    /// When, in milliseconds since the Unix epoch (a drag: its last step).
    pub at_ms: u64,
    /// Whether undo may reverse it. The registry decides: only a parameter
    /// with no hazard (not a flash write, an audible one, a reconfiguration,
    /// a deferred or an irreversible one).
    pub undoable: bool,
    // -- WP P1UG --
    /// The undo group's label ([`DspiCore::begin_group`]) when the change
    /// was written inside one; `None` otherwise.
    pub label: Option<String>,
}

impl From<&JournalEntry> for HistoryEntry {
    fn from(e: &JournalEntry) -> Self {
        Self {
            label: e.label.clone(),
            path: e.path.into(),
            indices: e.indices.clone(),
            before: e.before.clone().map(Into::into),
            after: e.after.clone().into(),
            source: e.source.into(),
            at_ms: e.at_ms,
            undoable: e.undoable,
        }
    }
}

// -- WP P1EFIX --
/// [`HistoryMove::note`] after undoing [`DspiCore::protect_all_outputs`]: the
/// thresholds, releases and links go back, the limiters stay on.
pub const PROTECT_UNDONE_NOTE: &str =
    "The limiters stay on: undo puts back their settings but never switches a limiter off.";

/// What [`DspiCore::undo`] or [`DspiCore::redo`] did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMove {
    /// The change reversed (undo) or applied again (redo); `None` when
    /// nothing moved.
    pub entry: Option<HistoryEntry>,
    /// The changes stepped over on the way, newest first, which cannot be
    /// undone (see [`HistoryEntry::undoable`]). An entry here that is
    /// undoable had no value before to restore, or someone else has changed
    /// it since (`note` says who); the walk stopped there. A drag's step of
    /// several values lists here each value someone else changed since, one
    /// entry each, also when its other values moved.
    pub skipped: Vec<HistoryEntry>,
    /// Why a value was left alone, in words, when another writer changed it
    /// since (only where the core leaves such a change alone): nothing moved,
    /// or, for a drag's step of several values, the others did. After an
    /// undo of "Protect all outputs" it also says the limiters stay on
    /// ([`PROTECT_UNDONE_NOTE`], WP P1EFIX). `None` otherwise.
    pub note: Option<String>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Step back: write back the value the newest undoable change replaced
    /// (then what the device moved with it), through the typed setter for
    /// its path. Changes that cannot be undone are stepped over and named in
    /// [`HistoryMove::skipped`]. Nothing to undo is not an error: `entry` is
    /// then `None`.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device, or when the
    /// connection changed during the call (nothing is sent to another
    /// device); the setter's errors, after which the change stays to be
    /// undone.
    pub fn undo(&self) -> Result<HistoryMove> {
        self.undo_step()
    }
    /// Step forward: write again the change [`DspiCore::undo`] most recently
    /// reversed. A new change since then ends what can be redone.
    ///
    /// Errors: as [`DspiCore::undo`]; after a setter's error the change
    /// stays to be redone.
    pub fn redo(&self) -> Result<HistoryMove> {
        self.redo_step()
    }

    // -- WP QTGAP1 --
    /// Mark a continuous drag (a slider or a graph handle held and moved):
    /// `true` when it starts, `false` when it ends. Between the two, every
    /// write undo can reverse joins a single undo step, however long the
    /// drag lasts and however many values it moves (several bands, a band's
    /// fields, a slider per tick); undo puts each back as it was when the
    /// drag began. A write outside a drag (a typed value, a key step, a
    /// click) is always a step of its own, and one undo cannot reverse ends
    /// the step. Each call ends the step being gathered, so two drags are two
    /// steps. A drag ends with its connection: after a disconnect, another
    /// device opened or a re-attach, writes are steps of their own until the
    /// next `true`; with no device the mark is ignored. A client that never
    /// calls this gets one step per write.
    pub fn set_dragging(&self, dragging: bool) {
        let mut inner = self.lock();
        // -- WP P0W2 (fixup p0w2b) -- pinned to the open connection
        inner.dragging = inner
            .conn
            .as_ref()
            .map(|c| c.generation)
            .filter(|_| dragging);
        // -- WP P1UG -- a drag inside an undo group is part of its step
        if inner.conn.as_ref().is_some_and(|c| c.undo_group.is_open()) {
            return;
        }
        if let Some(session) = inner.conn.as_mut().and_then(|c| c.session().ok()) {
            session.end_step();
        }
    }

    // -- WP P1UG --
    /// Begin an undo group: everything written until the matching
    /// [`DspiCore::end_group`] that undo can reverse is one undo step,
    /// carrying `label` ([`HistoryEntry::label`]), as a drag is. Use it for
    /// one action that writes several values (a linked pair, a Clear All, a
    /// routing preset). Groups nest: only the outermost begin starts the
    /// step and takes its label, and only the matching end ends it. The
    /// group belongs to the connection: a disconnect, another device
    /// opened, a re-attach, a re-read ([`DspiCore::fetch_all`]) and a change
    /// that ends the history (a preset load, a bulk image, a factory reset)
    /// end it at any depth, and an `end_group` after that does nothing.
    /// With no device the call is ignored. An empty group records nothing.
    /// A write undo cannot reverse stands apart, as in a drag. The group is
    /// the connection's, not the thread's: call it on the thread that sends
    /// the group's writes, in order with them.
    ///
    /// A group begun while a drag is marked ([`DspiCore::set_dragging`])
    /// joins the drag's step instead of ending it (the step takes the
    /// group's label if it had none), and its `end_group` leaves that step
    /// open until the drag ends: one drag is one step.
    // -- WP P1AFIX -- (review p1a F5) a group inside a drag joins its step
    pub fn begin_group(&self, label: String) {
        let mut inner = self.lock();
        let dragging = inner.dragging();
        let Some(conn) = inner.conn.as_mut() else {
            return;
        };
        if conn.undo_group.is_open() {
            conn.undo_group.depth = conn.undo_group.depth.saturating_add(1);
            return;
        }
        conn.undo_group = UndoGroup {
            depth: 1,
            label,
            foreign_since: false,
        };
        if dragging {
            return;
        }
        if let Ok(session) = conn.session() {
            session.end_step();
        }
    }

    // -- WP P1UG --
    /// End the undo group [`DspiCore::begin_group`] opened: the outermost
    /// end ends the step, so the next write is a step of its own. Nothing
    /// when no group is open (one a disconnect or a re-read ended, or none
    /// at all). Inside a drag the step goes on until `set_dragging(false)`.
    // -- WP P1AFIX -- (review p1a F5)
    pub fn end_group(&self) {
        let mut inner = self.lock();
        let dragging = inner.dragging();
        let Some(conn) = inner.conn.as_mut() else {
            return;
        };
        match conn.undo_group.depth {
            0 => {}
            1 if dragging => conn.undo_group = UndoGroup::default(),
            1 => end_undo_group(conn),
            d => conn.undo_group.depth = d.saturating_sub(1),
        }
    }

    /// Whether [`DspiCore::undo`] has a change to reverse. False with no
    /// device.
    pub fn can_undo(&self) -> bool {
        self.lock()
            .conn
            .as_mut()
            .and_then(|c| c.session().ok())
            .is_some_and(|s| s.can_undo())
    }

    /// Whether [`DspiCore::redo`] has a change to apply again. False with no
    /// device.
    pub fn can_redo(&self) -> bool {
        self.lock()
            .conn
            .as_mut()
            .and_then(|c| c.session().ok())
            .is_some_and(|s| s.can_redo())
    }

    /// The newest `n` changes of this connection, newest first: every typed
    /// setter's, undo's and redo's, with the value before and after, who
    /// made it and when. The connection's session keeps the newest 1000; a
    /// reconnect starts with none. Empty with no device.
    pub fn history(&self, n: u32) -> Vec<HistoryEntry> {
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        self.lock()
            .conn
            .as_mut()
            .and_then(|c| c.session().ok())
            .map(|s| s.history(n).iter().map(HistoryEntry::from).collect())
            .unwrap_or_default()
    }
}

impl DspiCore {
    /// [`DspiCore::undo`]. A change someone else has overwritten since (the
    /// mirror no longer holds the value this client left) is not reversed:
    /// theirs stays, the entry is dropped and [`HistoryMove::note`] names
    /// them (owner 2026-11-12). In a drag's step of several values (WP
    /// P0W2, O1) this is judged value by value: the others are put back, and
    /// the ones left alone leave the step.
    fn undo_step(&self) -> Result<HistoryMove> {
        let (step, generation, note, left) = {
            let mut inner = self.lock();
            let conn = inner.conn()?;
            let generation = conn.generation;
            let mut step = conn.session()?.take_undo();
            let (mut note, mut left) = (None, Vec::new());
            if let Some(entry) = step.entry.as_mut() {
                let judged = judge(&inner, entry, |v| Some(&v.after));
                (note, left) = (judged.note, judged.left);
                if judged.keep.is_empty() {
                    // Theirs now: the entry describes values that are gone.
                    step.entry = None;
                } else {
                    entry.set_values(judged.keep);
                }
            }
            (step, generation, note, left)
        };
        // -- WP P0W2 (fixup p0w2b, F1) -- each value left as another writer
        // set it, after the entries stepped over
        let skipped = step
            .skipped
            .iter()
            .map(HistoryEntry::from)
            .chain(left)
            .collect();
        let Some(entry) = step.entry else {
            return Ok(HistoryMove {
                entry: None,
                skipped,
                note,
            });
        };
        // Newest value first, each before what the device moved with it.
        let steps: Vec<(&'static str, Vec<u8>, Value)> = entry
            .values()
            .into_iter()
            .rev()
            .flat_map(|v| {
                v.before.map(|b| (v.path, v.indices, b)).into_iter().chain(
                    v.companions
                        .into_iter()
                        .map(|c| (c.path, c.indices, c.before)),
                )
            })
            .collect();
        let replay = Replay {
            source: ChangeSource::Undo,
            generation,
        };
        for (path, ix, value) in steps {
            if let Err(e) = self.replay(replay, path, &ix, value) {
                self.with_session(generation, |s| s.restore_undo(entry));
                return Err(e);
            }
        }
        let moved = HistoryEntry::from(&entry);
        self.with_session(generation, |s| s.finish_undo(entry));
        // -- WP P1EFIX -- undoing "Protect all outputs" never switches a
        // limiter off (`limit.on` is audible): say so in every client
        let note = if moved.label.as_deref() == Some(crate::params_fx::PROTECT_LABEL) {
            Some(match note {
                Some(n) => format!("{n} {PROTECT_UNDONE_NOTE}"),
                None => PROTECT_UNDONE_NOTE.to_owned(),
            })
        } else {
            note
        };
        Ok(HistoryMove {
            entry: Some(moved),
            skipped,
            note,
        })
    }

    /// [`DspiCore::redo`]: as [`DspiCore::undo`], a change someone else has
    /// overwritten since undo put it back is left as they set it, value by
    /// value in a drag's step.
    fn redo_step(&self) -> Result<HistoryMove> {
        let (entry, generation, note, left) = {
            let mut inner = self.lock();
            let conn = inner.conn()?;
            let generation = conn.generation;
            let mut entry = conn.session()?.take_redo();
            let mut note = None;
            let mut left = Vec::new();
            if let Some(e) = entry.as_mut() {
                let judged = judge(&inner, e, |v| v.before.as_ref());
                (note, left) = (judged.note, judged.left);
                if judged.keep.is_empty() {
                    return Ok(HistoryMove {
                        entry: None,
                        skipped: left,
                        note,
                    });
                }
                e.set_values(judged.keep);
            }
            (entry, generation, note, left)
        };
        let Some(entry) = entry else {
            return Ok(HistoryMove::default());
        };
        let replay = Replay {
            source: ChangeSource::Redo,
            generation,
        };
        for v in entry.values() {
            if let Err(e) = self.replay(replay, v.path, &v.indices, v.after) {
                self.with_session(generation, |s| s.restore_redo(entry));
                return Err(e);
            }
        }
        let moved = HistoryEntry::from(&entry);
        self.with_session(generation, |s| s.finish_redo(entry));
        Ok(HistoryMove {
            entry: Some(moved),
            skipped: left,
            note,
        })
    }

    /// Write one step of a replay through the typed setter for `path`, the
    /// thread marked as replaying so the record knows it for one.
    fn replay(&self, replay: Replay, path: &str, ix: &[u8], value: Value) -> Result<()> {
        let _mark = ReplayMark::set(replay);
        self.write_journal_value(path, ix, value)
    }

    /// A journal path's value through its writer: a whole band through
    /// `set_filter` or `set_crossover`, anything else through
    /// [`DspiCore::write_param`].
    fn write_journal_value(&self, path: &str, ix: &[u8], value: Value) -> Result<()> {
        // -- WP SW1 (fixup sw1, F1) --
        if path == ALINK_SLOTS {
            return match value {
                Value::Bytes(slots) => self.set_alink_slots(slots),
                _ => Err(DspiError::InvalidArgument {
                    msg: format!("{ALINK_SLOTS}: not 8 slots"),
                }),
            };
        }
        if path == EXT_PAIRMAP {
            return match value {
                Value::Bytes(map) => self.set_crossfeed_pair_map(map),
                _ => Err(DspiError::InvalidArgument {
                    msg: format!("{EXT_PAIRMAP}: not a pair map"),
                }),
            };
        }
        if path != EQ_BAND {
            return self
                .write_param(path.into(), ix.to_vec(), value.into())
                .map(drop);
        }
        let (Some(&ch), Some(&b)) = (ix.first(), ix.get(1)) else {
            return Err(DspiError::InvalidArgument {
                msg: format!("{EQ_BAND} takes a channel and a band, not {ix:?}"),
            });
        };
        let p = match &value {
            Value::Bytes(bytes) => band_from_bytes(bytes),
            _ => None,
        }
        .ok_or_else(|| DspiError::InvalidArgument {
            msg: format!("{EQ_BAND} [{ch}, {b}]: not a band"),
        })?;
        match b.checked_sub(XOVER_BASE) {
            Some(x) => self.set_crossover(ch, x, p),
            None => self.set_filter(ch, b, p),
        }
    }

    /// Run `f` on the session of connection `generation`, if it is still
    /// the open one.
    fn with_session(&self, generation: u64, f: impl FnOnce(&mut dspi_session::Session)) {
        let mut inner = self.lock();
        if let Some(c) = inner.conn.as_mut().filter(|c| c.generation == generation)
            && let Ok(s) = c.session()
        {
            f(s);
        }
    }
}

// -- WP P0W2 (O1) --
/// What [`judge`] found in a step.
struct Judged {
    /// The values nobody else has changed since: these move.
    keep: Vec<StepValue>,
    /// The others, one history entry each (fixup p0w2b, F1): left as their
    /// writer set them, named in `note`.
    left: Vec<HistoryEntry>,
    /// The others' writers in words, one sentence each.
    note: Option<String>,
}

/// Judge the values of `entry` against what this client left them at
/// (`expected`: their value after, for undo; before, for redo). A value with
/// no expected value (nothing before, for redo) is kept unjudged.
fn judge(
    inner: &Inner,
    entry: &JournalEntry,
    expected: impl Fn(&StepValue) -> Option<&Value>,
) -> Judged {
    let (mut keep, mut left) = (Vec::new(), Vec::new());
    let mut notes: Vec<String> = Vec::new();
    for v in entry.values() {
        match expected(&v).and_then(|x| overwritten(inner, v.path, &v.indices, entry.begun_ms, x)) {
            Some(n) => {
                if !notes.contains(&n) {
                    notes.push(n);
                }
                left.push(HistoryEntry {
                    path: v.path.into(),
                    indices: v.indices,
                    before: v.before.map(Into::into),
                    after: v.after.into(),
                    source: entry.source.into(),
                    at_ms: entry.at_ms,
                    undoable: entry.undoable,
                    label: entry.label.clone(), // -- WP P1UG --
                });
            }
            None => keep.push(v),
        }
    }
    Judged {
        keep,
        left,
        note: (!notes.is_empty()).then(|| notes.join(" ")),
    }
}

/// The history's state for `NOTIFY_HISTORY`: the connection and its
/// journal's version, `None` while there is no history at all.
pub(crate) fn history_mark(inner: &mut Inner) -> Option<(u64, u64)> {
    let conn = inner.conn.as_mut()?;
    let generation = conn.generation;
    let version = conn.session().ok()?.history_version();
    (version != 0).then_some((generation, version))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "journal/tests_wp_p0w.rs"]
mod tests_wp_p0w;

// -- WP SWAPFIX --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "journal/tests_wp_swapfix.rs"]
mod tests_wp_swapfix;

// -- WP QTGAP1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "journal/tests_wp_qtgap1.rs"]
mod tests_wp_qtgap1;

// -- WP P0W2 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "journal/tests_wp_p0w2.rs"]
mod tests_wp_p0w2;

// -- WP P1UG --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "journal/tests_wp_p1ug.rs"]
mod tests_wp_p1ug;
