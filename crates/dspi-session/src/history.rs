//! The journal's one door, and the stacks undo and redo walk (WP P0W).
//!
//! Every change a [`Session`] knows of enters its journal through
//! [`Session::record_change`]: [`Session::write`] records there after its
//! confirming read, and the embedder API in `dspi-core`, which sends on the
//! fast path and keeps its own mirror, records there after each typed setter
//! with the value the mirror held before. One journal, one set of rules:
//!
//! - **What can be undone is the registry's word.** Only a parameter with no
//!   hazard (`Hazard::None`) goes back. A flash write, an audible one, a
//!   reconfiguration, a deferred control-surface write and an irreversible
//!   one are recorded and never sent again by undo or redo (see
//!   [`undoable`]).
//! - **A change that replaces the whole state ends the history before it**
//!   ([`RESETS_HISTORY`]: a preset load, a bulk image, a factory reset). What
//!   came before described a state that is gone, so writing it back would set
//!   values nobody saw. The entry itself stays on the stack, so the next undo
//!   names it instead of saying nothing.
//! - **A drag is one step.** A change marked `coalesce` that follows one
//!   (from the same source, itself marked) joins that entry instead of
//!   adding one: a value it already holds takes the new value after, any
//!   other value (another band, another control) is added to the step with
//!   its value before ([`JournalEntry::more`]). A slider or a graph handle
//!   sends a write per tick, a multi-band drag one per band per tick, and
//!   undo should take back the drag, every value it moved. The caller marks
//!   only the writes of a continuous drag, and ends the step when the drag
//!   ends ([`Session::end_step`]); there is no time window, so a drag held
//!   still stays one step and a typed value never joins (WP QTGAP1, WP
//!   P0W2). A caller that journals one value under several paths (a band's
//!   field, or the whole band) widens that value to the group's path
//!   ([`Session::widen_step`]) when the drag moves another of them, so it
//!   is held once.
//! - **A re-read or another writer ends a step** ([`Session::end_step`]):
//!   a change after it starts a new entry, whatever it repeats, so undo never
//!   goes back past what the re-read or the other writer left (WP SWAPFIX).
//! - **Bounded.** The journal keeps the newest [`JOURNAL_CAP`] entries, the
//!   undo stack the newest [`UNDO_CAP`].
//! - **Per connection.** The journal belongs to the session, and the session
//!   to one connection: a reconnect starts with none. A device that went
//!   away may have been power-cycled (its live values back at the startup
//!   preset) or changed by someone else meanwhile, so an old value written
//!   back would land on a state the history never described.
//!
//! Each entry carries who made it ([`ChangeSource`]) and when (milliseconds
//! since the Unix epoch, from the clock seam).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::registry::{Hazard, by_path};
use dspi_proto::value::Value;

use crate::write::{Companion, JournalEntry, Outcome, Session, StepValue};

/// The most entries the journal keeps; the oldest go first.
pub const JOURNAL_CAP: usize = 1000;
/// The most steps undo can go back.
pub const UNDO_CAP: usize = 500;

/// The changes that replace the whole live state: the history before one
/// describes a state that is gone (module docs).
pub const RESETS_HISTORY: &[&str] = &["preset.load", "bulk", "bulk.chunk", "dev.reset"];

/// Who made a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ChangeSource {
    /// A write this program asked for.
    Local,
    /// The reversal of an earlier change, sent by undo.
    Undo,
    /// The repeat of an undone change, sent by redo.
    Redo,
}

/// A change to put in the journal: what [`Session::record_change`] takes.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// The registry path, or a name of the caller's own for a change the
    /// registry has no path for (the caller then gives its hazard).
    pub path: &'static str,
    pub indices: Vec<u8>,
    /// The value before, if known: undo writes it back.
    pub before: Option<Value>,
    /// The value after: redo writes it again.
    pub after: Value,
    /// What the device moved besides this value, as it was before.
    pub companions: Vec<Companion>,
    /// Decides whether undo and redo may send it again ([`undoable`]).
    pub hazard: Hazard,
    pub source: ChangeSource,
    /// Join the last entry when this repeats it (module docs).
    pub coalesce: bool,
}

impl Change {
    /// A change of registry `path`, its hazard the registry's
    /// ([`Hazard::Irreversible`] for a path the registry does not know, so a
    /// stray name is never replayed).
    pub fn of(path: &'static str, indices: Vec<u8>, before: Option<Value>, after: Value) -> Self {
        Self {
            path,
            indices,
            before,
            after,
            companions: Vec::new(),
            hazard: hazard_of(path),
            source: ChangeSource::Local,
            coalesce: false,
        }
    }
}

/// The registry's hazard for `path`; [`Hazard::Irreversible`] for a path it
/// does not have.
pub fn hazard_of(path: &str) -> Hazard {
    by_path(path).map_or(Hazard::Irreversible, |d| d.hazard)
}

/// Whether undo and redo may send a change of this hazard again: only one
/// with none. An audible change is an action rather than a value to put back
/// (stepping back through `sig.control` would start a generator the person
/// had stopped); a flash, deferred or reconfiguring one is confirmed before
/// the fact instead; an irreversible one has nothing to put back.
pub fn undoable(hazard: Hazard) -> bool {
    matches!(hazard, Hazard::None)
}

/// What [`Session::take_undo`] found.
#[derive(Debug, Clone, Default)]
pub struct UndoStep {
    /// The change to reverse: its `before` (then each companion's) is to be
    /// written back, then the entry handed to [`Session::finish_undo`] (or to
    /// [`Session::restore_undo`] if that failed). `None` when nothing is left
    /// to reverse, or when an entry with nothing to put back stopped the walk
    /// (it is then the last of `skipped`).
    pub entry: Option<JournalEntry>,
    /// Entries stepped over on the way, newest first: they cannot be undone.
    pub skipped: Vec<JournalEntry>,
}

/// Milliseconds since the Unix epoch, from the clock seam.
fn now_ms() -> u64 {
    dspi_transport::clock::system_time()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Drop the oldest entries of `v` beyond `cap`.
fn cap(v: &mut Vec<JournalEntry>, cap: usize) {
    if let Some(excess) = v.len().checked_sub(cap).filter(|n| *n > 0) {
        v.drain(..excess);
    }
}

// -- WP P0W2 (O1) --
/// Join `change` to the step `e`: a value it already holds takes the new
/// value after; another is added with its value before.
fn join(e: &mut JournalEntry, change: &Change) {
    if e.path == change.path && e.indices == change.indices {
        e.after = change.after.clone();
        return;
    }
    if let Some(v) = e
        .more
        .iter_mut()
        .find(|v| v.path == change.path && v.indices == change.indices)
    {
        v.after = change.after.clone();
        return;
    }
    e.more.push(StepValue {
        path: change.path,
        indices: change.indices.clone(),
        before: change.before.clone(),
        after: change.after.clone(),
        companions: change.companions.clone(),
    });
}

impl Session {
    /// Put `change` in the journal, and on the undo stack unless it is a
    /// replay by undo or redo (module docs for the rules). The outcome
    /// recorded is [`Outcome::Accepted`]: the caller's word that it was sent.
    pub fn record_change(&mut self, change: Change) {
        self.record_with_outcome(change, Outcome::Accepted);
    }

    /// [`Session::record_change`] with the outcome the write came back with.
    pub(crate) fn record_with_outcome(&mut self, change: Change, outcome: Outcome) {
        let at_ms = now_ms();
        self.history_version = self.history_version.wrapping_add(1);
        // An action has no value to put back, whatever its hazard.
        let reversible = undoable(change.hazard) && change.after != Value::Trigger;
        if change.coalesce && reversible && self.gathering(change.source).is_some() {
            for e in [self.journal.last_mut(), self.undo_stack.last_mut()]
                .into_iter()
                .flatten()
            {
                join(e, &change);
                e.outcome = outcome.clone();
                e.at_ms = at_ms;
            }
            return;
        }
        let entry = JournalEntry {
            path: change.path,
            indices: change.indices,
            before: change.before,
            after: change.after,
            outcome,
            undoable: reversible,
            companions: change.companions,
            source: change.source,
            at_ms,
            // -- WP P0W2 (O1) -- nothing joins a change undo cannot reverse
            coalesce: change.coalesce && reversible,
            more: Vec::new(),
        };
        self.journal.push(entry.clone());
        cap(&mut self.journal, JOURNAL_CAP);

        // A replay is already accounted for by the stack it came from. A
        // fresh change is a new branch of history, so anything that had been
        // undone is no longer reachable.
        if entry.source != ChangeSource::Local {
            return;
        }
        self.redo_stack.clear();
        if RESETS_HISTORY.contains(&entry.path) {
            self.undo_stack.clear();
        }
        self.undo_stack.push(entry);
        cap(&mut self.undo_stack, UNDO_CAP);
    }

    // -- WP P0W2 (O1) --
    /// The step a change from `source` marked to coalesce joins, whatever it
    /// moves: the newest of the journal and of the undo stack are that same
    /// entry, marked to coalesce, from that source; nothing has been undone
    /// since. One drag is one step, across bands and controls. A caller that
    /// journals one value under several paths (one field of a band, or the
    /// whole band) reads it to find the value a change belongs with.
    pub fn gathering(&self, source: ChangeSource) -> Option<&JournalEntry> {
        let (last, top) = (self.journal.last()?, self.undo_stack.last()?);
        (source == ChangeSource::Local
            && self.redo_stack.is_empty()
            && last.coalesce
            && last.source == source
            && top.path == last.path
            && top.indices == last.indices
            && top.at_ms == last.at_ms)
            .then_some(last)
    }

    /// Widen the value `from` at `indices` of the step being gathered
    /// ([`Session::gathering`]) to `path`, with `before` as the value undo
    /// restores: the caller's word that `before` is what `path` held when
    /// the step began. A change of `path` at `indices` then joins that
    /// value. Nothing happens when no step is being gathered or it holds no
    /// such value.
    pub fn widen_step(
        &mut self,
        from: &str,
        indices: &[u8],
        path: &'static str,
        before: Option<Value>,
    ) {
        let gathering = self
            .journal
            .last()
            .is_some_and(|l| self.gathering(l.source).is_some());
        if !gathering {
            return;
        }
        self.history_version = self.history_version.wrapping_add(1);
        for e in [self.journal.last_mut(), self.undo_stack.last_mut()]
            .into_iter()
            .flatten()
        {
            let mut values = e.values();
            for v in values
                .iter_mut()
                .filter(|v| v.path == from && v.indices == indices)
            {
                v.path = path;
                v.before = before.clone();
            }
            e.set_values(values);
        }
    }

    // -- WP SWAPFIX --
    /// End the step the newest entry is gathering: the next change starts an
    /// entry of its own even when it repeats that one (a re-read of the
    /// device, or another writer's change, came between them).
    pub fn end_step(&mut self) {
        if let Some(e) = self.journal.last_mut() {
            e.coalesce = false;
        }
    }

    /// Mark the [`Session::write`]s that follow as replays by `source` (an
    /// embedder's undo or redo), or, with `None`, as fresh changes again: a
    /// replay is journalled under its source and is not a new undo step.
    pub fn replaying_as(&mut self, source: Option<ChangeSource>) {
        self.replaying = source.is_some();
        if let Some(s) = source {
            self.replay_source = s;
        }
    }

    /// A counter that moves whenever the journal or either stack changes, so
    /// an embedder can tell its user's history view is out of date.
    pub fn history_version(&self) -> u64 {
        self.history_version
    }

    /// The newest `n` journal entries, newest first.
    pub fn history(&self, n: usize) -> Vec<JournalEntry> {
        self.journal.iter().rev().take(n).cloned().collect()
    }

    /// Is there a change left to step back through?
    pub fn can_undo(&self) -> bool {
        self.undo_stack.iter().any(|e| e.undoable && e.restorable())
    }

    /// Is there a change left to re-apply?
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Take the newest change undo can reverse off the stack, stepping over
    /// (and dropping) the ones it cannot. An entry with nothing to put back
    /// stops the walk: stepping over it to an older, unrelated change would
    /// undo something the person did not ask about.
    pub fn take_undo(&mut self) -> UndoStep {
        let mut step = UndoStep::default();
        while let Some(entry) = self.undo_stack.pop() {
            self.history_version = self.history_version.wrapping_add(1);
            if !entry.undoable {
                step.skipped.push(entry);
                continue;
            }
            if !entry.restorable() {
                step.skipped.push(entry);
                return step;
            }
            step.entry = Some(entry);
            return step;
        }
        step
    }

    /// `entry`, taken by [`Session::take_undo`], was reversed: redo may
    /// apply it again.
    pub fn finish_undo(&mut self, entry: JournalEntry) {
        self.history_version = self.history_version.wrapping_add(1);
        self.redo_stack.push(entry);
    }

    /// `entry`, taken by [`Session::take_undo`], could not be reversed: the
    /// change still stands, so it goes back on the stack to be tried again.
    pub fn restore_undo(&mut self, entry: JournalEntry) {
        self.history_version = self.history_version.wrapping_add(1);
        self.undo_stack.push(entry);
    }

    /// Take the change undo most recently reversed off the redo stack: its
    /// `after` is to be written again, then the entry handed to
    /// [`Session::finish_redo`] (or [`Session::restore_redo`]).
    pub fn take_redo(&mut self) -> Option<JournalEntry> {
        let entry = self.redo_stack.pop()?;
        self.history_version = self.history_version.wrapping_add(1);
        Some(entry)
    }

    /// `entry`, taken by [`Session::take_redo`], was applied again.
    pub fn finish_redo(&mut self, entry: JournalEntry) {
        self.history_version = self.history_version.wrapping_add(1);
        self.undo_stack.push(entry);
    }

    /// `entry`, taken by [`Session::take_redo`], could not be applied again.
    pub fn restore_redo(&mut self, entry: JournalEntry) {
        self.history_version = self.history_version.wrapping_add(1);
        self.redo_stack.push(entry);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0w {
    use super::*;
    use crate::probe::{Capabilities, ChannelInfo};
    use dspi_proto::Platform;
    use dspi_transport::MockTransport;

    fn session() -> Session {
        let caps = Capabilities {
            serial: "TEST".into(),
            platform: Platform::Rp2350,
            firmware: "1.1.6".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 6, 0),
            build_info: None,
            wire_format: dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8,
            num_channels: 17,
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
            band_storage: 12,
            channels: (0..17)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= 8,
                })
                .collect(),
            features: Vec::new(),
            cs: None,
            siggen: None,
            active_preset: None,
        };
        Session::new(Box::new(MockTransport::new()), caps).unwrap()
    }

    fn gain(before: f32, after: f32) -> Change {
        Change::of(
            "out.gain",
            vec![0],
            Some(Value::Float(before)),
            Value::Float(after),
        )
    }

    #[test]
    fn wp_p0w_a_recorded_change_can_be_taken_back_and_forth() {
        let mut s = session();
        let v0 = s.history_version();
        s.record_change(gain(0.0, -3.0));
        assert!(s.history_version() != v0);
        assert!(s.can_undo() && !s.can_redo());

        let step = s.take_undo();
        let e = step.entry.unwrap();
        assert_eq!(e.before, Some(Value::Float(0.0)));
        assert_eq!(e.source, ChangeSource::Local);
        assert!(e.at_ms > 0, "stamped from the clock");
        s.finish_undo(e);
        assert!(!s.can_undo() && s.can_redo());

        let e = s.take_redo().unwrap();
        assert_eq!(e.after, Value::Float(-3.0));
        s.finish_redo(e);
        assert!(s.can_undo() && !s.can_redo());
    }

    #[test]
    fn wp_p0w_a_hazard_is_recorded_and_stepped_over() {
        let mut s = session();
        s.record_change(gain(0.0, -3.0));
        // A flash write: in the journal, never undone.
        s.record_change(Change::of("dev.save", vec![], None, Value::Trigger));
        assert_eq!(s.history(10).len(), 2);
        assert!(!s.history(1)[0].undoable);
        let step = s.take_undo();
        assert_eq!(step.skipped.len(), 1);
        assert_eq!(step.skipped[0].path, "dev.save");
        assert_eq!(step.entry.unwrap().path, "out.gain");
    }

    #[test]
    fn wp_p0w_a_preset_load_ends_the_history_before_it() {
        let mut s = session();
        s.record_change(gain(0.0, -3.0));
        s.record_change(Change::of("preset.load", vec![2], None, Value::Trigger));
        assert!(
            !s.can_undo(),
            "the gain change described a state that is gone"
        );
        let step = s.take_undo();
        assert!(step.entry.is_none());
        assert_eq!(step.skipped.len(), 1, "the load is named");
        assert!(s.take_undo().skipped.is_empty());
        assert_eq!(s.history(10).len(), 2, "the journal keeps both");
    }

    #[test]
    fn wp_p0w_a_drag_is_one_step_and_a_replay_never_joins() {
        let mut s = session();
        for v in [-1.0, -2.0, -3.0] {
            let mut c = gain(0.0, v);
            c.before = Some(Value::Float(v + 1.0));
            c.coalesce = true;
            s.record_change(c);
        }
        assert_eq!(s.history(10).len(), 1);
        let e = s.take_undo().entry.unwrap();
        assert_eq!(e.before, Some(Value::Float(0.0)), "the drag's start");
        assert_eq!(e.after, Value::Float(-3.0), "the drag's end");
        s.finish_undo(e);

        // The reversal is journalled, not joined, and not an undo step.
        let mut c = gain(-3.0, 0.0);
        c.source = ChangeSource::Undo;
        c.coalesce = true;
        s.record_change(c);
        assert_eq!(s.history(10).len(), 2);
        assert!(!s.can_undo() && s.can_redo());

        // Another index joins the same drag (WP P0W2, O1).
        let mut a = gain(0.0, -1.0);
        a.coalesce = true;
        s.record_change(a);
        let mut b = gain(0.0, -1.0);
        b.indices = vec![1];
        b.coalesce = true;
        s.record_change(b);
        assert_eq!(s.history(10).len(), 3);
        assert_eq!(s.history(1)[0].more.len(), 1);
    }

    // -- WP SWAPFIX --
    #[test]
    fn wp_swapfix_an_ended_step_gathers_nothing_more() {
        let mut s = session();
        let mut c = gain(0.0, -1.0);
        c.coalesce = true;
        s.record_change(c);
        s.end_step();
        let mut c = gain(-1.0, -2.0);
        c.coalesce = true;
        s.record_change(c);
        assert_eq!(s.history(10).len(), 2);
        let e = s.take_undo().entry.unwrap();
        assert_eq!(
            e.before,
            Some(Value::Float(-1.0)),
            "back to the end of the step before"
        );
    }

    // -- WP P0W2 --
    fn dragged(path: &'static str, ix: Vec<u8>, before: Value, after: Value) -> Change {
        let mut c = Change::of(path, ix, Some(before), after);
        c.coalesce = true;
        if path == "eq.band" {
            // The core's own path, undoable (dspi-core journal.rs).
            c.hazard = Hazard::None;
        }
        c
    }

    #[test]
    fn wp_p0w2_a_widened_value_takes_the_groups_changes() {
        let mut s = session();
        s.record_change(dragged(
            "eq.freq",
            vec![0, 1],
            Value::Float(100.0),
            Value::Float(110.0),
        ));
        s.record_change(dragged(
            "eq.freq",
            vec![0, 2],
            Value::Float(200.0),
            Value::Float(210.0),
        ));
        assert_eq!(s.gathering(ChangeSource::Local).unwrap().path, "eq.freq");
        assert!(s.gathering(ChangeSource::Undo).is_none(), "a replay");

        s.widen_step("eq.freq", &[0, 1], "eq.band", Some(Value::Bytes(vec![1])));
        s.record_change(dragged(
            "eq.band",
            vec![0, 1],
            Value::Bytes(vec![2]),
            Value::Bytes(vec![3]),
        ));
        assert_eq!(s.history(10).len(), 1);
        let e = s.take_undo().entry.unwrap();
        let v = e.values();
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(
            (v[0].path, v[0].indices.as_slice()),
            ("eq.band", &[0, 1][..])
        );
        assert_eq!(
            v[0].before,
            Some(Value::Bytes(vec![1])),
            "the start of the step"
        );
        assert_eq!(v[0].after, Value::Bytes(vec![3]));
        assert_eq!(
            (v[1].path, v[1].after.clone()),
            ("eq.freq", Value::Float(210.0))
        );

        // An ended step is not widened.
        s.finish_undo(e);
        s.record_change(dragged(
            "eq.q",
            vec![0, 1],
            Value::Float(1.0),
            Value::Float(2.0),
        ));
        s.end_step();
        s.widen_step("eq.q", &[0, 1], "eq.band", None);
        assert_eq!(s.history(1)[0].path, "eq.q");
        assert!(s.gathering(ChangeSource::Local).is_none());
    }

    // -- WP P0W2 (O1) --
    /// One drag is one step across values: each keeps its own value before,
    /// a value moved again takes the newest value after; a typed value and
    /// the next drag stay apart.
    #[test]
    fn wp_p0w2_o1_one_drag_is_one_step_across_values() {
        let mut s = session();
        for tick in 1..=4 {
            let t = tick as f32;
            for out in [0u8, 1] {
                s.record_change(dragged(
                    "out.gain",
                    vec![out],
                    Value::Float(-(t - 1.0)),
                    Value::Float(-t),
                ));
            }
        }
        assert_eq!(s.history(10).len(), 1);
        let e = &s.history(1)[0];
        assert_eq!(e.more.len(), 1);
        assert_eq!(e.before, Some(Value::Float(0.0)));
        assert_eq!(e.after, Value::Float(-4.0));
        assert_eq!(e.more[0].before, Some(Value::Float(0.0)));
        assert_eq!(e.more[0].after, Value::Float(-4.0));

        s.record_change(gain(-4.0, -5.0)); // typed
        s.end_step();
        s.record_change(dragged(
            "out.gain",
            vec![1],
            Value::Float(-4.0),
            Value::Float(-6.0),
        ));
        assert_eq!(s.history(10).len(), 3);
        // An action never joins, and ends nothing it is not part of.
        s.record_change(dragged("dev.save", vec![], Value::Trigger, Value::Trigger));
        assert_eq!(s.history(10).len(), 4);
    }

    #[test]
    fn wp_p0w_the_journal_and_the_stack_are_bounded() {
        let mut s = session();
        for i in 0..(JOURNAL_CAP + 10) {
            s.record_change(gain(i as f32, i as f32 + 1.0));
        }
        assert_eq!(s.journal.len(), JOURNAL_CAP);
        assert_eq!(s.undo_stack.len(), UNDO_CAP);
        assert_eq!(
            s.history(1)[0].after,
            Value::Float((JOURNAL_CAP + 10) as f32)
        );
    }

    #[test]
    fn wp_p0w_a_path_the_registry_does_not_know_is_never_replayed() {
        assert_eq!(hazard_of("no.such.path"), Hazard::Irreversible);
        assert!(!undoable(hazard_of("no.such.path")));
        assert!(undoable(hazard_of("out.gain")));
        assert!(!undoable(hazard_of("out.enable")), "audible");
        assert!(!undoable(hazard_of("out.pin")), "reconfiguration");
        assert!(!undoable(hazard_of("preset.name")), "flash");
    }
}
