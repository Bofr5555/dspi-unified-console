//! The Snapshots panel (WP UB11, UXP B11): the console core's snapshot
//! store for this DSPi inside the interface, with Recall and A/B, where
//! `uconsole snapshot` and `uconsole ab` were the only way.
//!
//! A recall is the core's: at once over USB, with a way back over DSPi Link
//! (Keep or Revert before the countdown ends, as the shell's recall). A/B
//! compares the device now (A, captured as it is) with the selected
//! snapshot (B), one key between them; `k` keeps the side the device is on.
//! Every write goes through the runner as a session request, so the
//! interface re-reads the device after it.

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent};
use dspi_session::DeviceState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::host::{AbView, GuardView, ProHost, Recalled, SnapshotRow, countdown};
use crate::shell::{Screen, ScreenEvent, SessionReply, SessionRequest};
use crate::theme::Theme;
use crate::widgets::text::truncate;
use crate::widgets::{Button, Dialog, DialogOutcome, KeyHelp};

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("↑ ↓", "Select a snapshot"),
    KeyHelp::new("Enter", "Recall it"),
    KeyHelp::new("s", "Save the device as it is now"),
    KeyHelp::new("a", "A/B: the device now (A) and the selected snapshot (B)"),
    KeyHelp::new("Space", "Switch between A and B"),
    KeyHelp::new("k", "Keep the side the device is on; keep a recall"),
    KeyHelp::new("r", "Revert a recall that waits"),
    KeyHelp::new("d", "Delete the selected snapshot"),
];

/// What the panel says without the console core (the fixture, a screenshot).
pub const NO_HOST: &str = "Snapshots are kept by the console core: open the device with \
                           uconsole, or use `uconsole snapshot` and `uconsole ab`.";

/// What the panel says with no snapshot of this DSPi yet.
pub const EMPTY: &str = "No snapshots of this DSPi yet. s saves the device as it is now.";

const TAG_RECALL: u32 = 1;
const TAG_AB_START: u32 = 2;
const TAG_AB_SWITCH: u32 = 3;
const TAG_AB_KEEP: u32 = 4;
const TAG_GUARD: u32 = 5;

/// What a session request found, kept for `session_result`.
#[derive(Debug, Clone)]
enum Found {
    Recall(Result<Recalled, String>),
    Ab(Result<AbView, String>),
    Guard(Result<GuardView, String>),
}

/// The dialog the panel has open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    Recall,
    Save,
    Delete,
}

pub struct SnapshotsPanel {
    host: Option<Rc<dyn ProHost>>,
    rows: Vec<SnapshotRow>,
    cursor: usize,
    scroll: usize,
    ab: AbView,
    guard: Option<GuardView>,
    /// One sentence on the last action, and its detail lines.
    note: String,
    detail: Vec<String>,
    asking: Option<Asking>,
    found: Rc<RefCell<Option<Found>>>,
    /// The device changed without a request (a way back that ran out).
    refresh: bool,
}

impl SnapshotsPanel {
    pub fn new(host: Option<Rc<dyn ProHost>>) -> Self {
        let mut p = Self {
            host,
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            ab: AbView::default(),
            guard: None,
            note: String::new(),
            detail: Vec::new(),
            asking: None,
            found: Rc::new(RefCell::new(None)),
            refresh: false,
        };
        p.reload();
        p
    }

    /// The store, the compare and a recall that waits, as the core has them.
    fn reload(&mut self) {
        let Some(host) = self.host.clone() else {
            return;
        };
        match host.snapshots() {
            Ok(rows) => self.rows = rows,
            Err(e) => self.say(e),
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.ab = host.ab();
        let g = host.guard();
        self.guard = g.waiting.then_some(g);
    }

    fn say(&mut self, note: impl Into<String>) {
        self.note = note.into();
        self.detail.clear();
    }

    fn selected(&self) -> Option<&SnapshotRow> {
        self.rows.get(self.cursor)
    }

    /// A write, run by the runner with the session, its result kept here.
    fn request(
        &self,
        tag: u32,
        run: impl Fn(&dyn ProHost) -> Found + 'static,
    ) -> ScreenEvent {
        let (Some(host), found) = (self.host.clone(), self.found.clone()) else {
            return ScreenEvent::Status(NO_HOST.into());
        };
        ScreenEvent::Session(SessionRequest::new(tag, move |_| {
            *found.borrow_mut() = Some(run(host.as_ref()));
            SessionReply::Ok(String::new())
        }))
    }

    fn ab_words(&self) -> String {
        if !self.ab.active {
            return "A/B: a compares the device now (A) with the selected snapshot (B)".into();
        }
        let (side, name) = if self.ab.on_b {
            ("B", self.ab.b_name.as_str())
        } else {
            ("A", "the device as it was")
        };
        format!("A/B running · on {side}: {name} · Space switches, k keeps this side")
    }

    fn guard_words(g: &GuardView) -> String {
        format!(
            "Keep these settings? They go back in {} · k Keep · r Revert",
            countdown(g.remaining_ms)
        )
    }

    /// Who keeps `k`: a recall that waits, else a running compare.
    fn keep(&mut self) -> ScreenEvent {
        if self.guard.is_some() {
            return self.request(TAG_GUARD, |h| Found::Guard(h.guard_keep()));
        }
        if self.ab.active {
            let b = self.ab.on_b;
            return self.request(TAG_AB_KEEP, move |h| Found::Ab(h.ab_keep(b)));
        }
        ScreenEvent::Handled
    }
}

impl Screen for SnapshotsPanel {
    fn title(&self) -> String {
        "Snapshots".into()
    }

    fn draw(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        _state: &DeviceState,
        focused: bool,
    ) {
        if area.height < 4 || area.width < 8 {
            return;
        }
        let w = area.width.saturating_sub(2) as usize;
        let x = area.x + 1;
        let mut y = area.y;
        let Some(_) = self.host else {
            for (i, line) in crate::widgets::text::wrap(NO_HOST, w, 4)
                .iter()
                .enumerate()
            {
                buf.set_string(x, y + i as u16, line, theme.label());
            }
            return;
        };
        let count = match self.rows.len() {
            1 => "1 snapshot".to_string(),
            n => format!("{n} snapshots"),
        };
        buf.set_string(
            x,
            y,
            truncate(&format!("Snapshots of this DSPi · {count}"), w),
            theme.title(),
        );
        y += 1;
        let ab_style = if self.ab.active {
            theme.focused()
        } else {
            theme.label()
        };
        buf.set_string(x, y, truncate(&self.ab_words(), w), ab_style);
        y += 1;
        if let Some(g) = &self.guard {
            buf.set_string(x, y, truncate(&Self::guard_words(g), w), theme.warning_style());
            y += 1;
        }
        y += 1;
        let bottom = area.y + area.height;
        // The note and its detail keep the last lines.
        let tail = if self.note.is_empty() {
            0
        } else {
            1 + self.detail.len().min(3) as u16
        };
        let list_end = bottom.saturating_sub(tail);
        if self.rows.is_empty() {
            buf.set_string(x, y, truncate(EMPTY, w), theme.label());
        } else {
            let visible = list_end.saturating_sub(y) as usize;
            self.scroll = crate::widgets::Table::scroll_to(self.cursor, self.scroll, visible);
            let name_w = self
                .rows
                .iter()
                .map(|r| unicode_width::UnicodeWidthStr::width(r.name.as_str()))
                .max()
                .unwrap_or(0)
                .clamp(8, 28);
            let when_w = self
                .rows
                .iter()
                .map(|r| unicode_width::UnicodeWidthStr::width(r.when.as_str()))
                .max()
                .unwrap_or(0);
            for (i, row) in self.rows.iter().enumerate().skip(self.scroll).take(visible) {
                let ly = y + (i - self.scroll) as u16;
                let here = i == self.cursor;
                buf.set_string(
                    area.x,
                    ly,
                    if here && focused { "▸" } else { " " },
                    theme.focused(),
                );
                let b = self.ab.active && self.ab.b_name == row.name;
                let line = format!(
                    "{:<name_w$}  {:<when_w$}  {}{}",
                    truncate(&row.name, name_w),
                    row.when,
                    if b { "B  " } else { "" },
                    row.note
                );
                let style = if here {
                    theme.value()
                } else {
                    theme.label()
                };
                buf.set_string(x, ly, truncate(&line, w), style);
            }
        }
        if !self.note.is_empty() {
            let mut ny = list_end;
            buf.set_string(x, ny, truncate(&self.note, w), theme.value());
            for d in self.detail.iter().take(3) {
                ny += 1;
                buf.set_string(x + 2, ny, truncate(d, w.saturating_sub(2)), theme.label());
            }
        }
    }

    fn handle(&mut self, key: KeyEvent, _state: &DeviceState) -> ScreenEvent {
        if self.host.is_none() {
            return ScreenEvent::Unhandled;
        }
        match key.code {
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                ScreenEvent::Handled
            }
            KeyCode::Down => {
                if self.cursor + 1 < self.rows.len() {
                    self.cursor += 1;
                }
                ScreenEvent::Handled
            }
            KeyCode::Char('k') => self.keep(),
            KeyCode::Char('r') if self.guard.is_some() => {
                self.request(TAG_GUARD, |h| Found::Guard(h.guard_revert()))
            }
            // A recall that waits is answered first
            _ if self.guard.is_some() => {
                self.say("A recall waits: k keeps it, r reverts it.");
                ScreenEvent::Handled
            }
            KeyCode::Char(' ') if self.ab.active => {
                self.request(TAG_AB_SWITCH, |h| Found::Ab(h.ab_toggle()))
            }
            KeyCode::Char('a') if self.ab.active => {
                self.request(TAG_AB_SWITCH, |h| Found::Ab(h.ab_toggle()))
            }
            KeyCode::Char('a') => match self.selected() {
                Some(r) => {
                    let id = r.id.clone();
                    self.request(TAG_AB_START, move |h| Found::Ab(h.ab_start(&id)))
                }
                None => {
                    self.say("Save a snapshot first (s): B is a snapshot.");
                    ScreenEvent::Handled
                }
            },
            _ if self.ab.active && matches!(key.code, KeyCode::Enter | KeyCode::Char('d')) => {
                self.say("An A/B compare runs: k keeps the side the device is on first.");
                ScreenEvent::Handled
            }
            KeyCode::Enter => match self.selected().cloned() {
                Some(r) => {
                    self.asking = Some(Asking::Recall);
                    ScreenEvent::Dialog(Dialog::confirm(
                        "Recall Snapshot",
                        format!(
                            "Recall “{}”? The device's settings become the snapshot's. What \
                             is on it now is not kept: save a snapshot first (s) to come back \
                             to it.",
                            r.name
                        ),
                        vec![Button::new("Recall"), Button::new("Cancel")],
                    ))
                }
                None => ScreenEvent::Handled,
            },
            KeyCode::Char('s') => {
                self.asking = Some(Asking::Save);
                ScreenEvent::Dialog(Dialog::text(
                    "Save a Snapshot",
                    "A name for the device as it is now:",
                    String::new(),
                    "Before tuning",
                ))
            }
            KeyCode::Char('d') => match self.selected().cloned() {
                Some(r) => {
                    self.asking = Some(Asking::Delete);
                    ScreenEvent::Dialog(Dialog::confirm(
                        "Delete Snapshot",
                        format!("Delete “{}”? It cannot be brought back.", r.name),
                        vec![Button::destructive("Delete"), Button::new("Cancel")],
                    ))
                }
                None => ScreenEvent::Handled,
            },
            _ => ScreenEvent::Unhandled,
        }
    }

    fn keys(&self) -> &'static [KeyHelp] {
        KEYS
    }

    fn dialog_result(&mut self, outcome: DialogOutcome, _state: &DeviceState) -> ScreenEvent {
        let Some(host) = self.host.clone() else {
            return ScreenEvent::Handled;
        };
        match (self.asking.take(), outcome) {
            (Some(Asking::Recall), DialogOutcome::Button(0)) => match self.selected() {
                Some(r) => {
                    let id = r.id.clone();
                    self.request(TAG_RECALL, move |h| Found::Recall(h.snapshot_recall(&id)))
                }
                None => ScreenEvent::Handled,
            },
            (Some(Asking::Save), DialogOutcome::Text(name)) => {
                let name = name.trim();
                if name.is_empty() {
                    self.say("Not saved: a snapshot needs a name.");
                    return ScreenEvent::Handled;
                }
                match host.snapshot_save(name) {
                    Ok(row) => {
                        self.reload();
                        self.cursor = self.rows.iter().position(|r| r.id == row.id).unwrap_or(0);
                        self.say(format!("Saved “{}”.", row.name));
                    }
                    Err(e) => self.say(format!("Not saved: {e}")),
                }
                ScreenEvent::Handled
            }
            (Some(Asking::Delete), DialogOutcome::Button(0)) => {
                if let Some(r) = self.selected().cloned() {
                    match host.snapshot_delete(&r.id) {
                        Ok(()) => {
                            self.reload();
                            self.say(format!("Deleted “{}”.", r.name));
                        }
                        Err(e) => self.say(format!("Not deleted: {e}")),
                    }
                }
                ScreenEvent::Handled
            }
            _ => ScreenEvent::Handled,
        }
    }

    fn session_result(
        &mut self,
        _tag: u32,
        reply: SessionReply,
        _state: &DeviceState,
    ) -> ScreenEvent {
        let found = self.found.borrow_mut().take();
        match found {
            Some(Found::Recall(Ok(Recalled::Done { changes, notes }))) => {
                let name = self.selected().map(|r| r.name.clone()).unwrap_or_default();
                self.note = match changes.len() {
                    0 => format!("Recalled “{name}”: nothing differed."),
                    1 => format!("Recalled “{name}”: 1 change."),
                    n => format!("Recalled “{name}”: {n} changes."),
                };
                self.detail = changes.into_iter().chain(notes).collect();
            }
            Some(Found::Recall(Ok(Recalled::Guarded(g)))) => {
                self.say(format!("Recalled “{}” with a way back.", g.label));
                self.guard = g.waiting.then_some(g);
            }
            Some(Found::Recall(Err(e))) => self.say(format!("Not recalled: {e}")),
            Some(Found::Ab(Ok(v))) => {
                let side = if v.on_b { "B" } else { "A" };
                self.note = if v.active {
                    format!("On {side}.")
                } else {
                    format!("Kept {side}; the compare has ended.")
                };
                self.detail = v.changes.clone();
                self.ab = v;
            }
            Some(Found::Ab(Err(e))) => {
                self.say(e);
                if let Some(h) = &self.host {
                    self.ab = h.ab();
                }
            }
            Some(Found::Guard(Ok(g))) => {
                self.say(if g.message.is_empty() {
                    "Kept.".to_string()
                } else {
                    g.message.clone()
                });
                self.guard = g.waiting.then_some(g);
            }
            Some(Found::Guard(Err(e))) => self.say(e),
            None => {
                if let SessionReply::Err(e) = reply {
                    self.say(e);
                }
            }
        }
        ScreenEvent::Handled
    }

    fn poll(&mut self, _session: &mut dspi_session::Session, _state: &DeviceState) {
        let (Some(host), Some(_)) = (self.host.clone(), self.guard.as_ref()) else {
            return;
        };
        let g = host.guard();
        if !g.waiting {
            self.say(if g.message.is_empty() {
                "The recall went back.".to_string()
            } else {
                g.message.clone()
            });
            self.guard = None;
            self.refresh = true;
        } else {
            self.guard = Some(g);
        }
    }

    fn wants_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::host::{Speakers, SpeakerRow};
    use crate::shell::fixture;
    use crossterm::event::KeyModifiers;
    use std::cell::Cell;

    /// A core stand-in: a store of two, a compare, a recall that waits when
    /// `guarded`; every call counted.
    #[derive(Default)]
    pub(crate) struct FakeHost {
        pub rows: RefCell<Vec<SnapshotRow>>,
        pub ab: RefCell<AbView>,
        pub guarded: bool,
        pub guard: RefCell<GuardView>,
        pub recalled: RefCell<Vec<String>>,
        pub writes: Cell<u32>,
    }

    impl FakeHost {
        pub(crate) fn two() -> Self {
            let row = |id: &str, name: &str| SnapshotRow {
                id: id.into(),
                name: name.into(),
                note: String::new(),
                when: "26 Nov 21:23".into(),
            };
            Self {
                rows: RefCell::new(vec![row("s2", "After EQ"), row("s1", "Before tuning")]),
                ..Self::default()
            }
        }
    }

    impl ProHost for FakeHost {
        fn snapshots(&self) -> Result<Vec<SnapshotRow>, String> {
            Ok(self.rows.borrow().clone())
        }
        fn snapshot_save(&self, name: &str) -> Result<SnapshotRow, String> {
            let row = SnapshotRow {
                id: format!("s{}", self.rows.borrow().len() + 1),
                name: name.into(),
                note: String::new(),
                when: "26 Nov 21:40".into(),
            };
            self.rows.borrow_mut().insert(0, row.clone());
            Ok(row)
        }
        fn snapshot_delete(&self, id: &str) -> Result<(), String> {
            self.rows.borrow_mut().retain(|r| r.id != id);
            Ok(())
        }
        fn snapshot_recall(&self, id: &str) -> Result<Recalled, String> {
            self.writes.set(self.writes.get() + 1);
            self.recalled.borrow_mut().push(id.into());
            if self.guarded {
                let g = GuardView {
                    waiting: true,
                    remaining_ms: 28_000,
                    label: id.into(),
                    message: String::new(),
                };
                *self.guard.borrow_mut() = g.clone();
                return Ok(Recalled::Guarded(g));
            }
            Ok(Recalled::Done {
                changes: vec!["Out 1 · L: gain".into(), "Volume".into()],
                notes: Vec::new(),
            })
        }
        fn guard(&self) -> GuardView {
            self.guard.borrow().clone()
        }
        fn guard_keep(&self) -> Result<GuardView, String> {
            let g = GuardView {
                message: "Kept.".into(),
                ..GuardView::default()
            };
            *self.guard.borrow_mut() = g.clone();
            Ok(g)
        }
        fn guard_revert(&self) -> Result<GuardView, String> {
            self.writes.set(self.writes.get() + 1);
            let g = GuardView {
                message: "Put back as it was.".into(),
                ..GuardView::default()
            };
            *self.guard.borrow_mut() = g.clone();
            Ok(g)
        }
        fn ab(&self) -> AbView {
            self.ab.borrow().clone()
        }
        fn ab_start(&self, b_id: &str) -> Result<AbView, String> {
            let name = self
                .rows
                .borrow()
                .iter()
                .find(|r| r.id == b_id)
                .map(|r| r.name.clone())
                .ok_or("no such snapshot")?;
            let v = AbView {
                active: true,
                on_b: false,
                a_name: "A".into(),
                b_name: name,
                changes: Vec::new(),
            };
            *self.ab.borrow_mut() = v.clone();
            Ok(v)
        }
        fn ab_toggle(&self) -> Result<AbView, String> {
            self.writes.set(self.writes.get() + 1);
            let mut v = self.ab.borrow().clone();
            v.on_b = !v.on_b;
            v.changes = vec!["Out 1 · L: gain".into()];
            *self.ab.borrow_mut() = v.clone();
            Ok(v)
        }
        fn ab_keep(&self, b: bool) -> Result<AbView, String> {
            let mut v = self.ab.borrow().clone();
            v.active = false;
            v.on_b = b;
            *self.ab.borrow_mut() = v.clone();
            Ok(v)
        }
        fn speakers(&self) -> Result<Speakers, String> {
            Err("not here".into())
        }
        fn speakers_suggest(&self) -> Result<Speakers, String> {
            Err("not here".into())
        }
        fn speakers_confirm(&self, _rows: &[SpeakerRow]) -> Result<Speakers, String> {
            Err("not here".into())
        }
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    pub(crate) fn session() -> dspi_session::Session {
        let t = dspi_transport::MockTransport::new().answering_everything(vec![0u8; 64]);
        dspi_session::Session::new(Box::new(t), fixture::caps()).expect("a session")
    }

    /// Run a session request as the runner would, and hand back its reply.
    fn run(p: &mut SnapshotsPanel, ev: ScreenEvent, state: &DeviceState) {
        let ScreenEvent::Session(req) = ev else {
            panic!("a session request, not {ev:?}");
        };
        let mut session = session();
        let reply = (req.run)(&mut session);
        p.session_result(req.tag, reply, state);
    }

    fn draw(p: &mut SnapshotsPanel, state: &DeviceState) -> String {
        let theme = crate::Theme::new(
            crate::theme::Palette::Calm,
            crate::theme::ColorDepth::Mono,
            crate::theme::Glyphs::Braille,
        );
        crate::render_frame(90, 14, |area, buf| p.draw(area, buf, &theme, state, true))
    }

    #[test]
    fn ub11_the_panel_lists_the_store_and_recalls_the_selected_after_asking() {
        let state = fixture::state();
        let host = Rc::new(FakeHost::two());
        let mut p = SnapshotsPanel::new(Some(host.clone()));
        let f = draw(&mut p, &state);
        assert!(f.contains("Snapshots of this DSPi · 2 snapshots"), "{f}");
        assert!(f.contains("▸After EQ"), "{f}");
        assert!(f.contains("Before tuning"), "{f}");
        p.handle(key(KeyCode::Down), &state);
        let ev = p.handle(key(KeyCode::Enter), &state);
        assert!(matches!(ev, ScreenEvent::Dialog(_)), "Recall asks first");
        assert!(host.recalled.borrow().is_empty(), "nothing before the answer");
        let ev = p.dialog_result(DialogOutcome::Button(0), &state);
        run(&mut p, ev, &state);
        assert_eq!(*host.recalled.borrow(), vec!["s1".to_string()]);
        let f = draw(&mut p, &state);
        assert!(f.contains("Recalled “Before tuning”: 2 changes."), "{f}");
        assert!(f.contains("Out 1 · L: gain"), "{f}");
        // Cancel recalls nothing
        p.handle(key(KeyCode::Enter), &state);
        assert_eq!(
            p.dialog_result(DialogOutcome::Button(1), &state),
            ScreenEvent::Handled
        );
        assert_eq!(host.recalled.borrow().len(), 1);
    }

    #[test]
    fn ub11_a_b_starts_on_the_selected_switches_with_space_and_k_keeps() {
        let state = fixture::state();
        let host = Rc::new(FakeHost::two());
        let mut p = SnapshotsPanel::new(Some(host.clone()));
        let ev = p.handle(key(KeyCode::Char('a')), &state);
        run(&mut p, ev, &state);
        assert!(host.ab.borrow().active);
        assert_eq!(host.ab.borrow().b_name, "After EQ");
        let f = draw(&mut p, &state);
        assert!(
            f.contains("A/B running · on A: the device as it was"),
            "{f}"
        );
        let ev = p.handle(key(KeyCode::Char(' ')), &state);
        run(&mut p, ev, &state);
        assert!(host.ab.borrow().on_b);
        let f = draw(&mut p, &state);
        assert!(f.contains("on B: After EQ"), "{f}");
        assert!(f.contains("On B."), "{f}");
        // no recall while a compare runs
        assert_eq!(p.handle(key(KeyCode::Enter), &state), ScreenEvent::Handled);
        assert!(host.recalled.borrow().is_empty());
        let ev = p.handle(key(KeyCode::Char('k')), &state);
        run(&mut p, ev, &state);
        assert!(!host.ab.borrow().active);
        assert!(host.ab.borrow().on_b, "kept the side it was on");
        let f = draw(&mut p, &state);
        assert!(f.contains("Kept B; the compare has ended."), "{f}");
    }

    #[test]
    fn ub11_a_guarded_recall_counts_down_and_k_or_r_answers_it() {
        let state = fixture::state();
        let host = Rc::new(FakeHost {
            guarded: true,
            ..FakeHost::two()
        });
        let mut p = SnapshotsPanel::new(Some(host.clone()));
        p.handle(key(KeyCode::Enter), &state);
        let ev = p.dialog_result(DialogOutcome::Button(0), &state);
        run(&mut p, ev, &state);
        let f = draw(&mut p, &state);
        assert!(
            f.contains("Keep these settings? They go back in 0:28 · k Keep · r Revert"),
            "{f}"
        );
        // other keys wait for the answer
        assert_eq!(p.handle(key(KeyCode::Char('a')), &state), ScreenEvent::Handled);
        assert!(!host.ab.borrow().active);
        let ev = p.handle(key(KeyCode::Char('r')), &state);
        run(&mut p, ev, &state);
        let f = draw(&mut p, &state);
        assert!(!f.contains("Keep these settings?"), "{f}");
        assert!(f.contains("Put back as it was."), "{f}");
    }

    #[test]
    fn ub11_a_way_back_that_runs_out_asks_the_runner_to_read_the_device_again() {
        let state = fixture::state();
        let host = Rc::new(FakeHost {
            guarded: true,
            ..FakeHost::two()
        });
        let mut p = SnapshotsPanel::new(Some(host.clone()));
        p.handle(key(KeyCode::Enter), &state);
        let ev = p.dialog_result(DialogOutcome::Button(0), &state);
        run(&mut p, ev, &state);
        assert!(!p.wants_refresh());
        *host.guard.borrow_mut() = GuardView {
            message: "Time ran out: the settings went back.".into(),
            ..GuardView::default()
        };
        let mut session = session();
        p.poll(&mut session, &state);
        assert!(p.wants_refresh());
        assert!(!p.wants_refresh(), "once");
        let f = draw(&mut p, &state);
        assert!(f.contains("Time ran out"), "{f}");
    }

    #[test]
    fn ub11_s_saves_under_a_name_and_d_deletes_after_asking() {
        let state = fixture::state();
        let host = Rc::new(FakeHost::two());
        let mut p = SnapshotsPanel::new(Some(host.clone()));
        assert!(matches!(
            p.handle(key(KeyCode::Char('s')), &state),
            ScreenEvent::Dialog(_)
        ));
        p.dialog_result(DialogOutcome::Text("Living room".into()), &state);
        let f = draw(&mut p, &state);
        assert!(f.contains("3 snapshots"), "{f}");
        assert!(f.contains("▸Living room"), "the new one selected: {f}");
        assert!(f.contains("Saved “Living room”."), "{f}");
        p.handle(key(KeyCode::Char('d')), &state);
        p.dialog_result(DialogOutcome::Button(0), &state);
        assert_eq!(host.rows.borrow().len(), 2);
        assert_eq!(host.writes.get(), 0, "saving and deleting write nothing");
    }

    #[test]
    fn ub11_without_the_core_the_panel_says_where_snapshots_live() {
        let state = fixture::state();
        let mut p = SnapshotsPanel::new(None);
        let f = draw(&mut p, &state);
        assert!(f.contains("Snapshots are kept by the console core"), "{f}");
        assert_eq!(
            p.handle(key(KeyCode::Enter), &state),
            ScreenEvent::Unhandled
        );
    }

    #[test]
    fn ub11_an_empty_store_says_how_to_save_one() {
        let state = fixture::state();
        let host = Rc::new(FakeHost::default());
        let mut p = SnapshotsPanel::new(Some(host));
        let f = draw(&mut p, &state);
        assert!(f.contains(EMPTY), "{f}");
        assert!(matches!(
            p.handle(key(KeyCode::Char('a')), &state),
            ScreenEvent::Handled
        ));
    }
}
