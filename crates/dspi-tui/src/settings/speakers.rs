//! Settings › Speakers (WP UB11, UXP B11): which output drives what. The
//! speaker layout the measurement and its safety checks take each output's
//! role from, as the Qt and web Speakers pages show it: a role per output,
//! its speaker and group, Confirm and Use the Suggestion. Kept by the
//! console core per device ([`crate::host::ProHost`]); nothing is written to
//! the DSPi and no level changes: the tweeter rule is the core's, as it was.
//!
//! The person's edits stay on the page until Confirm keeps them.

use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent};

use crate::host::{ProHost, Role, Speakers};
use crate::widgets::{Action, Dialog, DialogOutcome, KeyHelp, PopupList};

use super::{Cx, PageEvent, Row, SettingsPage};

/// What the page says without the console core.
pub const NO_HOST: &str = "The speaker layout is kept by the console core: open the device \
                           with uconsole to set it here.";

/// Under the table (the Qt page's footer, the tweeter rule in its words).
pub const FOOTER: &str = "An output with an active high-pass at 1 kHz or above is measured as \
                          a tweeter, whatever its role here.";

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("↑ ↓", "Move between outputs and buttons"),
    KeyHelp::new("← →", "Change the role"),
    KeyHelp::new("Enter", "Choose the role, or press the button"),
    KeyHelp::new("n", "Name the output's speaker (L, R, Sub …)"),
    KeyHelp::new("g", "Set the output's group"),
    KeyHelp::new("Tab", "Sidebar, page, save bar"),
    KeyHelp::new("Esc", "Leave Settings"),
];

/// With no layout shown (no core, or it could not be read).
const KEYS_EMPTY: &[KeyHelp] = &[
    KeyHelp::new("Tab", "Sidebar, page, save bar"),
    KeyHelp::new("Esc", "Leave Settings"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// The layout's row `n`.
    Role(usize),
    Buttons,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    Role(usize),
    Speaker(usize),
    Group(usize),
}

#[derive(Default)]
pub struct SpeakersPage {
    host: Option<Rc<dyn ProHost>>,
    cursor: usize,
    button: usize,
    /// The layout as shown: the core's, with the person's edits.
    shown: Option<Speakers>,
    /// Why it could not be read.
    error: Option<String>,
    /// Edited since it was read or confirmed.
    changed: bool,
    /// The last action's sentence, and whether it failed.
    said: Option<(String, bool)>,
    asking: Option<Asking>,
    // -- FIX016 -- (REVG R4)
    /// The installer's lock, in the core's words: the layout is shown and
    /// nothing on the page changes it. Asked again each time the page looks.
    locked: Option<String>,
}

impl SpeakersPage {
    pub fn new(host: Option<Rc<dyn ProHost>>) -> Self {
        Self {
            host,
            ..Self::default()
        }
    }

    /// Read the layout once, when the page is first looked at.
    fn load(&mut self) {
        if self.shown.is_some() || self.error.is_some() {
            return;
        }
        let Some(host) = &self.host else { return };
        match host.speakers() {
            Ok(s) => self.shown = Some(s),
            Err(e) => self.error = Some(e),
        }
    }

    fn label(cx: &Cx<'_>, output: u8) -> String {
        let ni = usize::from(cx.state.caps.num_inputs);
        crate::screens::channel_label(cx.state, ni + usize::from(output))
    }

    fn build(&self, cx: &Cx<'_>) -> Vec<(Option<Item>, Row)> {
        let mut out = vec![
            (None, Row::section("Speaker Layout")),
            (
                None,
                Row::note(
                    "Which output drives what? The measurement and its safety checks take \
                     each output's role from here.",
                ),
            ),
        ];
        if self.host.is_none() {
            out.push((None, Row::note(NO_HOST)));
            return out;
        }
        if let Some(e) = &self.error {
            out.push((None, Row::Status(format!("Not read: {e}"), true)));
            return out;
        }
        let Some(s) = &self.shown else {
            return out;
        };
        // -- FIX016 -- (REVG R4) under the installer's lock: the layout as it is kept, nothing to change
        if let Some(why) = &self.locked {
            return self.build_locked(cx, s, why, out);
        }
        out.push((
            None,
            if self.changed {
                Row::Status("Changed: not kept until you confirm.".into(), true)
            } else {
                Row::Status(s.stamp.clone(), !s.confirmed)
            },
        ));
        out.push((None, Row::Blank));
        let choices: Vec<String> = Role::ALL.iter().map(|r| r.words().to_owned()).collect();
        for (i, r) in s.rows.iter().enumerate() {
            let speaker = if r.speaker.is_empty() {
                "speaker not set".to_owned()
            } else {
                format!("speaker {}", r.speaker)
            };
            let source = if r.source.is_empty() {
                String::new()
            } else {
                format!(" · {}", r.source)
            };
            out.push((
                Some(Item::Role(i)),
                Row::Pick {
                    label: Self::label(cx, r.output),
                    choices: choices.clone(),
                    selected: Role::ALL.iter().position(|x| *x == r.role).unwrap_or(0),
                    caption: Some(format!("{speaker} · group {}{source}", r.group)),
                    enabled: true,
                },
            ));
        }
        if s.rows.is_empty() {
            out.push((
                None,
                Row::note("No output plays anything yet: route an input to an output first."),
            ));
        }
        out.push((None, Row::Blank));
        out.push((
            Some(Item::Buttons),
            Row::Buttons {
                label: String::new(),
                caption: None,
                buttons: vec!["Confirm".into(), "Use the Suggestion".into()],
                cursor: self.button,
                enabled: true,
            },
        ));
        if let Some((text, failed)) = &self.said {
            out.push((None, Row::Status(text.clone(), *failed)));
        }
        out.push((None, Row::Blank));
        out.push((
            None,
            Row::note(
                "n names an output's speaker, g sets its group. The outputs of one speaker \
                 share a group: they are measured in one run and aligned to each other.",
            ),
        ));
        for n in &s.notes {
            out.push((None, Row::note(n.clone())));
        }
        out.push((None, Row::note(FOOTER)));
        out
    }

    // -- FIX016 -- (REVG R4)
    /// The page under the installer's lock: why, and each output's role as
    /// words. No row to change, no Confirm, no suggestion.
    fn build_locked(
        &self,
        cx: &Cx<'_>,
        s: &Speakers,
        why: &str,
        mut out: Vec<(Option<Item>, Row)>,
    ) -> Vec<(Option<Item>, Row)> {
        out.push((None, Row::Status(s.stamp.clone(), !s.confirmed)));
        out.push((None, Row::Status(why.to_owned(), true)));
        out.push((None, Row::Blank));
        for r in &s.rows {
            let speaker = if r.speaker.is_empty() {
                "speaker not set".to_owned()
            } else {
                format!("speaker {}", r.speaker)
            };
            out.push((
                None,
                Row::pair(
                    Self::label(cx, r.output),
                    format!("{} · {speaker} · group {}", r.role.words(), r.group),
                ),
            ));
        }
        out.push((None, Row::Blank));
        out.push((None, Row::note(FOOTER)));
        out
    }

    fn item(&self, index: usize, cx: &Cx<'_>) -> Option<Item> {
        self.build(cx)
            .into_iter()
            .filter(|(_, r)| r.focusable())
            .nth(index)
            .and_then(|(i, _)| i)
    }

    fn set_role(&mut self, row: usize, choice: usize) {
        let Some(role) = Role::ALL.get(choice).copied() else {
            return;
        };
        if let Some(r) = self.shown.as_mut().and_then(|s| s.rows.get_mut(row))
            && r.role != role
        {
            r.role = role;
            r.source.clear();
            self.changed = true;
            self.said = None;
        }
    }

    fn confirm(&mut self) -> PageEvent {
        let (Some(host), Some(s)) = (self.host.clone(), self.shown.as_ref()) else {
            return PageEvent::Handled;
        };
        if self.locked.is_some() {
            return PageEvent::Handled; // -- FIX016 --
        }
        match host.speakers_confirm(&s.rows) {
            Ok(kept) => {
                self.shown = Some(kept);
                self.changed = false;
                self.said = Some((
                    "Confirmed: the measurement takes its roles from this layout.".into(),
                    false,
                ));
            }
            Err(e) => self.said = Some((format!("Not confirmed: {e}"), true)),
        }
        PageEvent::Handled
    }

    fn suggest(&mut self) -> PageEvent {
        let Some(host) = self.host.clone() else {
            return PageEvent::Handled;
        };
        match host.speakers_suggest() {
            Ok(s) => {
                self.shown = Some(s);
                self.changed = true;
                self.said = Some((
                    "The suggestion from the filters and the routing: check it, then confirm."
                        .into(),
                    false,
                ));
            }
            Err(e) => self.said = Some((format!("No suggestion: {e}"), true)),
        }
        PageEvent::Handled
    }
}

impl SettingsPage for SpeakersPage {
    fn rows(&self, cx: &Cx<'_>) -> Vec<Row> {
        self.build(cx).into_iter().map(|(_, r)| r).collect()
    }

    fn act(&mut self, index: usize, action: Action, cx: &Cx<'_>) -> PageEvent {
        match (self.item(index, cx), action) {
            (Some(Item::Role(i)), Action::Selected(choice)) => {
                self.set_role(i, choice);
                PageEvent::Handled
            }
            (Some(Item::Role(i)), Action::Open) => {
                let selected = self
                    .shown
                    .as_ref()
                    .and_then(|s| s.rows.get(i))
                    .and_then(|r| Role::ALL.iter().position(|x| *x == r.role))
                    .unwrap_or(0);
                self.asking = Some(Asking::Role(i));
                PageEvent::Popup(PopupList::new(
                    "Role",
                    Role::ALL.iter().map(|r| r.words().to_owned()).collect(),
                    selected,
                ))
            }
            (Some(Item::Buttons), Action::Selected(b)) => {
                self.button = b.min(1);
                PageEvent::Handled
            }
            (Some(Item::Buttons), Action::Open) => {
                if self.button == 0 {
                    self.confirm()
                } else {
                    self.suggest()
                }
            }
            _ => PageEvent::Handled,
        }
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, i: usize) {
        self.cursor = i;
    }

    fn keys(&self) -> &'static [KeyHelp] {
        if self.shown.is_some() && self.locked.is_none() {
            KEYS
        } else {
            KEYS_EMPTY
        }
    }

    fn popup_result(&mut self, choice: Option<usize>, _cx: &Cx<'_>) -> PageEvent {
        if let (Some(Asking::Role(i)), Some(c)) = (self.asking.take(), choice) {
            self.set_role(i, c);
        }
        PageEvent::Handled
    }

    fn dialog_result(&mut self, outcome: DialogOutcome, _cx: &Cx<'_>) -> PageEvent {
        let DialogOutcome::Text(text) = outcome else {
            self.asking = None;
            return PageEvent::Handled;
        };
        let text = text.trim().to_owned();
        let asked = self.asking.take();
        let Some(rows) = self.shown.as_mut().map(|s| &mut s.rows) else {
            return PageEvent::Handled;
        };
        match asked {
            Some(Asking::Speaker(i)) => {
                if let Some(r) = rows.get_mut(i)
                    && r.speaker != text
                {
                    r.speaker = text;
                    self.changed = true;
                    self.said = None;
                }
            }
            Some(Asking::Group(i)) => match text.parse::<u8>() {
                Ok(g) if g <= 15 => {
                    if let Some(r) = rows.get_mut(i)
                        && r.group != g
                    {
                        r.group = g;
                        self.changed = true;
                        self.said = None;
                    }
                }
                _ => self.said = Some(("A group is a number from 0 to 15.".into(), true)),
            },
            _ => {}
        }
        PageEvent::Handled
    }

    fn key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> PageEvent {
        let Some(Item::Role(i)) = self.item(self.cursor, cx) else {
            return PageEvent::Unhandled;
        };
        let Some(r) = self.shown.as_ref().and_then(|s| s.rows.get(i)) else {
            return PageEvent::Unhandled;
        };
        let label = Self::label(cx, r.output);
        match key.code {
            KeyCode::Char('n') => {
                self.asking = Some(Asking::Speaker(i));
                PageEvent::Dialog(Dialog::text(
                    "Speaker",
                    format!("The speaker {label} belongs to (L, R, Sub, or your word):"),
                    r.speaker.clone(),
                    "Not set",
                ))
            }
            KeyCode::Char('g') => {
                self.asking = Some(Asking::Group(i));
                PageEvent::Dialog(Dialog::text(
                    "Group",
                    format!("The group of {label}, 0 to 15: the outputs of one speaker share one."),
                    r.group.to_string(),
                    "0",
                ))
            }
            _ => PageEvent::Unhandled,
        }
    }

    fn observe(&mut self, _cx: &Cx<'_>) {
        self.load();
        // -- FIX016 -- (REVG R4) locked or unlocked since: the page follows
        let locked = self.host.as_ref().and_then(|h| h.speakers_locked());
        if locked.is_some() && self.locked.is_none() {
            // what the person had changed is not theirs to keep now
            self.shown = None;
            self.changed = false;
            self.said = None;
            self.load();
        }
        self.locked = locked;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AppConfig, Page, SettingsData, SettingsScreen};
    use super::*;
    use crate::host::{AbView, GuardView, Recalled, SnapshotRow, SpeakerRow};
    use crate::shell::{Screen, ScreenEvent, fixture};
    use crossterm::event::KeyModifiers;
    use std::cell::RefCell;

    /// The core's layout calls: a suggestion of two outputs, what was kept.
    #[derive(Default)]
    struct Layouts {
        kept: RefCell<Option<Speakers>>,
        /// -- FIX016 -- the installer's lock, as the core words it
        locked: Option<String>,
    }

    fn suggestion() -> Speakers {
        Speakers {
            rows: vec![
                SpeakerRow {
                    output: 0,
                    role: Role::Full,
                    speaker: "L".into(),
                    group: 0,
                    source: "suggested".into(),
                },
                SpeakerRow {
                    output: 1,
                    role: Role::Full,
                    speaker: "R".into(),
                    group: 1,
                    source: "suggested".into(),
                },
            ],
            confirmed: false,
            stamp: "Suggested from the filters and the routing: check which output drives \
                    what, then confirm."
                .into(),
            notes: vec![],
        }
    }

    impl ProHost for Layouts {
        fn snapshots(&self) -> Result<Vec<SnapshotRow>, String> {
            Ok(vec![])
        }
        fn snapshot_save(&self, _: &str) -> Result<SnapshotRow, String> {
            Err("no".into())
        }
        fn snapshot_delete(&self, _: &str) -> Result<(), String> {
            Err("no".into())
        }
        fn snapshot_recall(&self, _: &str) -> Result<Recalled, String> {
            Err("no".into())
        }
        fn guard(&self) -> GuardView {
            GuardView::default()
        }
        fn guard_keep(&self) -> Result<GuardView, String> {
            Err("no".into())
        }
        fn guard_revert(&self) -> Result<GuardView, String> {
            Err("no".into())
        }
        fn ab(&self) -> AbView {
            AbView::default()
        }
        fn ab_start(&self, _: &str) -> Result<AbView, String> {
            Err("no".into())
        }
        fn ab_toggle(&self) -> Result<AbView, String> {
            Err("no".into())
        }
        fn ab_keep(&self, _: bool) -> Result<AbView, String> {
            Err("no".into())
        }
        fn speakers(&self) -> Result<Speakers, String> {
            Ok(self.kept.borrow().clone().unwrap_or_else(suggestion))
        }
        fn speakers_suggest(&self) -> Result<Speakers, String> {
            Ok(suggestion())
        }
        fn speakers_confirm(&self, rows: &[SpeakerRow]) -> Result<Speakers, String> {
            let kept = Speakers {
                rows: rows.to_vec(),
                confirmed: true,
                stamp: "Confirmed 2026-11-26 21:40 UTC.".into(),
                notes: vec![],
            };
            *self.kept.borrow_mut() = Some(kept.clone());
            Ok(kept)
        }
        fn speakers_locked(&self) -> Option<String> {
            self.locked.clone()
        }
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn opened(host: Option<Rc<dyn ProHost>>) -> (SettingsScreen, dspi_session::DeviceState) {
        let st = fixture::state();
        let s = SettingsScreen::new(&st, SettingsData::default(), AppConfig::default())
            .host(host)
            .open(Page::Speakers, &st);
        (s, st)
    }

    fn frame(s: &mut SettingsScreen, st: &dspi_session::DeviceState) -> String {
        super::super::tests::frame(s, st, 120, 40)
    }

    #[test]
    fn ub11_speakers_shows_the_suggestion_with_a_role_per_output() {
        let host: Rc<dyn ProHost> = Rc::new(Layouts::default());
        let (mut s, st) = opened(Some(host));
        let f = frame(&mut s, &st);
        assert!(f.contains("SPEAKER LAYOUT"), "{f}");
        assert!(f.contains("Suggested from the filters"), "{f}");
        let ni = usize::from(st.caps.num_inputs);
        let out1 = crate::screens::channel_label(&st, ni);
        assert!(f.contains(&out1), "{out1}: {f}");
        assert!(f.contains("Full range"), "{f}");
        assert!(f.contains("speaker L · group 0 · suggested"), "{f}");
        assert!(
            f.contains("Confirm") && f.contains("Use the Suggestion"),
            "{f}"
        );
        assert!(
            f.contains("An output with an active high-pass at 1 kHz"),
            "{f}"
        );
        assert!(f.contains("Speakers"), "in the sidebar: {f}");
    }

    #[test]
    fn ub11_a_role_changed_is_kept_only_when_confirmed() {
        let layouts = Rc::new(Layouts::default());
        let host: Rc<dyn ProHost> = layouts.clone();
        let (mut s, st) = opened(Some(host));
        frame(&mut s, &st);
        // into the page, on Out 1's role: → four times is High (tweeter)
        s.handle(key(KeyCode::Tab), &st);
        for _ in 0..4 {
            s.handle(key(KeyCode::Right), &st);
        }
        let f = frame(&mut s, &st);
        assert!(f.contains("High (tweeter)"), "{f}");
        assert!(f.contains("Changed: not kept until you confirm."), "{f}");
        assert!(layouts.kept.borrow().is_none(), "nothing kept yet");
        // n names the speaker
        assert!(matches!(
            s.handle(key(KeyCode::Char('n')), &st),
            ScreenEvent::Dialog(_)
        ));
        s.dialog_result(DialogOutcome::Text("Left".into()), &st);
        // down to the buttons, Confirm
        s.handle(key(KeyCode::Down), &st);
        s.handle(key(KeyCode::Down), &st);
        s.handle(key(KeyCode::Enter), &st);
        let kept = layouts.kept.borrow().clone().expect("confirmed");
        assert_eq!(kept.rows[0].role, Role::High);
        assert_eq!(kept.rows[0].speaker, "Left");
        assert_eq!(kept.rows[1].role, Role::Full);
        let f = frame(&mut s, &st);
        assert!(
            f.contains("Confirmed: the measurement takes its roles"),
            "{f}"
        );
        assert!(f.contains("Confirmed 2026-11-26"), "{f}");
        assert!(!f.contains("Changed: not kept"), "{f}");
    }

    #[test]
    fn ub11_a_group_must_be_0_to_15() {
        let layouts = Rc::new(Layouts::default());
        let host: Rc<dyn ProHost> = layouts.clone();
        let (mut s, st) = opened(Some(host));
        frame(&mut s, &st);
        s.handle(key(KeyCode::Tab), &st);
        s.handle(key(KeyCode::Char('g')), &st);
        s.dialog_result(DialogOutcome::Text("40".into()), &st);
        let f = frame(&mut s, &st);
        assert!(f.contains("A group is a number from 0 to 15."), "{f}");
        s.handle(key(KeyCode::Char('g')), &st);
        s.dialog_result(DialogOutcome::Text("3".into()), &st);
        let f = frame(&mut s, &st);
        assert!(f.contains("speaker L · group 3"), "{f}");
    }

    // -- FIX016 -- (REVG R4)
    #[test]
    fn fix016_under_the_installers_lock_the_page_changes_nothing() {
        let why = "Locked by the installer: only volume, mute and presets can be changed.";
        let layouts = Rc::new(Layouts {
            locked: Some(why.into()),
            ..Layouts::default()
        });
        let host: Rc<dyn ProHost> = layouts.clone();
        let (mut s, st) = opened(Some(host));
        let f = frame(&mut s, &st);
        assert!(f.contains(why), "{f}");
        assert!(f.contains("Full range · speaker L · group 0"), "{f}");
        assert!(!f.contains("Use the Suggestion"), "{f}");
        assert!(!f.contains("Confirm"), "no Confirm: {f}");
        // what changed the role and kept it, unlocked (ub11_a_role_changed_is_kept_only_when_confirmed)
        s.handle(key(KeyCode::Tab), &st);
        for _ in 0..4 {
            s.handle(key(KeyCode::Right), &st);
        }
        assert!(!matches!(
            s.handle(key(KeyCode::Char('n')), &st),
            ScreenEvent::Dialog(_)
        ));
        assert!(!matches!(
            s.handle(key(KeyCode::Char('g')), &st),
            ScreenEvent::Dialog(_)
        ));
        s.handle(key(KeyCode::Down), &st);
        s.handle(key(KeyCode::Down), &st);
        s.handle(key(KeyCode::Enter), &st);
        assert!(layouts.kept.borrow().is_none(), "nothing kept");
        let f = frame(&mut s, &st);
        assert!(!f.contains("High (tweeter)"), "{f}");
        assert!(!f.contains("Changed: not kept"), "{f}");
    }

    #[test]
    fn ub11_without_the_core_speakers_says_where_the_layout_lives() {
        let (mut s, st) = opened(None);
        let f = frame(&mut s, &st);
        assert!(f.contains("kept by the console core"), "{f}");
    }
}
