//! Settings > About.
//!
//! The Console's About page, minus the icon: the app name, what it is, the
//! version, and then who made what, from the one table every client shows
//! (`dspi_session::about`, WP CG15). No word of that table is written here. The
//! Console opens each link in a browser; a terminal shows the URL, which is the
//! thing a person can act on either way.
//!
//! The wording is the owner's and may change, so nothing here assumes its length or
//! what it holds. Each line of the file's text becomes its own row, folded to the
//! page's width: a word wider than the page is broken rather than running off it, and
//! a page taller than the screen scrolls a line at a time to its last line. What
//! cannot be shown, such as an escape or an order-changing mark, is dropped first.

use std::sync::OnceLock;

use dspi_session::about::{About, about};
use unicode_width::UnicodeWidthStr;

use crate::widgets::{Action, KeyHelp};

use super::{Cx, PageEvent, Row, SettingsPage};

/// The Console's paragraph above the links.
pub const SUPPORT: &str = "DSPi Firmware and Console are free, open-source software that I \
                           develop in my spare time. Contributions of any kind - code, \
                           feedback, funding, or otherwise - are always immensely appreciated.";

/// Said in place of the table if it cannot be read, which a test in `dspi-session`
/// holds cannot reach a user.
const NO_TABLE: &str = "The credits could not be read.";

/// The most of one piece of text shown. Far more than any wording needs, and it bounds
/// what a frame has to fold.
const MAX_CHARS: usize = 4000;

/// The most rows the table may add, which keeps the page's row arithmetic, in `u16`,
/// from overflowing whatever the table holds.
const MAX_ROWS: usize = 4000;

#[derive(Debug, Default)]
pub struct AboutPage {
    /// The columns the page was last drawn in.
    width: u16,
}

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("↑ ↓", "Scroll"),
    KeyHelp::new("Tab", "Sidebar, page, save bar"),
    KeyHelp::new("[ ]", "Back, forward"),
    KeyHelp::new("Esc", "Leave Settings"),
];

/// The table, read once: the page asks for its rows every frame.
fn table() -> Option<&'static About> {
    static TABLE: OnceLock<Option<About>> = OnceLock::new();
    TABLE.get_or_init(|| about().ok()).as_ref()
}

/// Would a terminal draw this at all, or draw it as something else? Controls, and the
/// marks that are invisible or reorder the text around them.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{AD}'
                | '\u{61C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// A piece of the table as one safe line of text: only what can be shown, single
/// spaces, and a bounded length.
fn clean(text: &str) -> String {
    let shown: String = text
        .chars()
        .filter(|c| c.is_whitespace() || !hidden(*c))
        .take(MAX_CHARS.saturating_add(1))
        .collect();
    let mut out = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_CHARS {
        out = out.chars().take(MAX_CHARS).collect();
        out.push('…');
    }
    out
}

/// `text` as lines of at most `width` columns. Lines break between words, and inside a
/// word only if it is wider than a line. Takes [`clean`]ed text. A character wider than
/// `width` still takes a line of its own, which the page's margin has room for.
fn fold(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() {
            if cur.width().saturating_add(1).saturating_add(word.width()) <= width {
                cur.push(' ');
                cur.push_str(word);
                continue;
            }
            lines.push(std::mem::take(&mut cur));
        }
        for c in word.chars() {
            cur.push(c);
            if cur.width() > width && cur.chars().count() > 1 {
                cur.pop();
                lines.push(std::mem::take(&mut cur));
                cur.push(c);
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// The rows of a page `width` columns wide, built for one piece of the table.
struct Writer {
    width: u16,
    rows: Vec<Row>,
}

impl Writer {
    fn new(width: u16) -> Self {
        Self {
            width,
            rows: Vec::new(),
        }
    }

    /// Text in a dim paragraph, a row a line. Nothing is added for empty text.
    fn note(&mut self, text: &str) {
        // `Row::Note` draws one column in and leaves one spare.
        let width = usize::from(self.width.saturating_sub(2).max(1));
        self.rows
            .extend(fold(&clean(text), width).into_iter().map(Row::Note));
    }

    /// A section heading, folded like a note. Nothing is added for empty text.
    fn heading(&mut self, text: &str) {
        let width = usize::from(self.width.saturating_sub(2).max(1));
        self.rows
            .extend(fold(&clean(text), width).into_iter().map(Row::Section));
    }

    /// A name and an address: on one line if they fit, the Console's layout, and
    /// otherwise each folded on lines of its own so neither is cut.
    fn pair(&mut self, label: &str, value: &str) {
        let (label, value) = (clean(label), clean(value));
        let fits = label
            .width()
            .saturating_add(value.width())
            .saturating_add(2)
            <= usize::from(self.width);
        if fits && !label.is_empty() {
            self.rows.push(Row::pair(label, value));
        } else {
            self.note(&label);
            self.note(&value);
        }
    }
}

/// The lines under the app's name.
fn credit_rows(table: Option<&About>, width: u16) -> Vec<Row> {
    let mut w = Writer::new(width);
    match table {
        Some(table) => {
            for line in &table.credit_lines {
                w.note(&line.text);
            }
        }
        None => w.note(NO_TABLE),
    }
    w.rows
}

/// Who made what, the links, and the credits, each section only if it has entries.
fn table_rows(table: Option<&About>, width: u16) -> Vec<Row> {
    let mut w = Writer::new(width);
    let Some(table) = table else {
        w.rows.push(Row::section("Links & Support"));
        w.note(SUPPORT);
        return w.rows;
    };
    if !table.makers.is_empty() {
        w.rows.push(Row::section("Made By"));
        for maker in &table.makers {
            w.pair(&maker.name, &maker.url);
            w.note(&maker.made.join(", "));
        }
        w.rows.push(Row::Blank);
    }
    // -- WP CG29 -- The heading and note are the table's: they say whose links these are.
    w.heading(&table.links_title);
    w.note(&table.links_note);
    w.note(SUPPORT);
    for link in &table.links {
        w.pair(&link.title, &link.url);
        w.note(&link.detail);
    }
    if !table.credits.is_empty() {
        w.rows.push(Row::Blank);
        w.rows.push(Row::section("Credits"));
        for credit in &table.credits {
            w.note(&format!("{}: {}", credit.name, credit.what));
            if let Some(url) = &credit.url {
                w.note(url);
            }
        }
    }
    w.rows.truncate(MAX_ROWS);
    w.rows
}

/// The credit lines as plain text, for `dspi --version`. Empty if the table cannot be
/// read: asking for the version never fails over its credits.
pub fn credit_lines() -> Vec<String> {
    table().map(plain_credits).unwrap_or_default()
}

fn plain_credits(table: &About) -> Vec<String> {
    table
        .credit_lines
        .iter()
        .map(|line| clean(&line.text))
        .filter(|line| !line.is_empty())
        .collect()
}

impl SettingsPage for AboutPage {
    fn rows(&self, cx: &Cx<'_>) -> Vec<Row> {
        let mut rows = vec![
            Row::pair("DSPi Terminal", String::new()),
            Row::note("USB Audio DSP Controller"),
            Row::note(format!("Version {}", env!("CARGO_PKG_VERSION"))),
        ];
        rows.extend(credit_rows(table(), self.width).into_iter().take(MAX_ROWS));
        rows.push(Row::Blank);
        if cx.connected {
            rows.push(Row::section("Device"));
            let caps = &cx.state.caps;
            rows.push(Row::pair("Platform", caps.platform.name()));
            rows.push(Row::pair("Firmware", caps.firmware.clone()));
            rows.push(Row::pair("Wire format", format!("V{}", caps.wire_format)));
            rows.push(Row::pair("Serial", caps.serial.clone()));
            rows.push(Row::Blank);
        }
        rows.extend(table_rows(table(), self.width));
        rows
    }

    fn act(&mut self, _index: usize, _action: Action, _cx: &Cx<'_>) -> PageEvent {
        PageEvent::Handled
    }

    fn cursor(&self) -> usize {
        0
    }

    fn set_cursor(&mut self, _i: usize) {}

    // -- WP TM22 --
    fn set_width(&mut self, width: u16) {
        self.width = width;
    }

    fn keys(&self) -> &'static [KeyHelp] {
        KEYS
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::super::Page;
    use super::super::tests::{frame, screen};
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};
    use dspi_session::about::{Credit, CreditLine, Link, Maker};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use crate::shell::screen::Screen;
    use crate::theme::{ColorDepth, Glyphs, Theme};

    /// Every line the page shows, whitespace evened out, so a test can ask for a text
    /// however it was folded. The sidebar and the border are outside `inner`.
    fn page_text(f: &str) -> String {
        f.lines()
            .map(|l| {
                l.chars()
                    .skip(20)
                    .filter(|c| !matches!(c, '│' | '|'))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The whole page, read by scrolling it from top to bottom.
    fn scrolled_to_the_end(w: u16, h: u16) -> Vec<String> {
        let (mut s, st) = screen(Page::About);
        s.focus = super::super::Focus::Page;
        let mut frames = vec![frame(&mut s, &st, w, h)];
        for _ in 0..200 {
            let _ = s.handle(KeyEvent::from(KeyCode::Down), &st);
            let f = frame(&mut s, &st, w, h);
            if frames.last() == Some(&f) {
                break;
            }
            frames.push(f);
        }
        frames
    }

    #[test]
    fn wp_tm22_about_shows_everything_the_table_holds() {
        let table = about().unwrap();
        let frames = scrolled_to_the_end(120, 40);
        let all: String = frames.iter().map(|f| page_text(f)).collect();
        let squeezed = |t: &str| t.split_whitespace().collect::<String>();
        let all = squeezed(&all);
        let mut wanted = vec![];
        wanted.extend(table.credit_lines.iter().map(|l| l.text.clone()));
        for m in &table.makers {
            wanted.extend([m.name.clone(), m.url.clone(), m.made.join(", ")]);
        }
        for l in &table.links {
            wanted.extend([l.title.clone(), l.url.clone(), l.detail.clone()]);
        }
        for c in &table.credits {
            wanted.extend([c.name.clone(), c.what.clone()]);
            wanted.extend(c.url.clone());
        }
        for text in wanted {
            assert!(all.contains(&squeezed(&text)), "missing {text}\n{all}");
        }
    }

    #[test]
    fn wp_cg29_the_links_section_is_titled_and_noted_as_weeb_labs() {
        let table = about().unwrap();
        let rows = table_rows(Some(&table), 200);
        let at = rows
            .iter()
            .position(|r| matches!(r, Row::Section(t) if t == "Weeb Labs: links and support"))
            .expect("the table's links title");
        assert!(
            matches!(&rows[at + 1], Row::Note(t) if *t == clean(&table.links_note)),
            "{:?}",
            rows[at + 1]
        );
        assert!(table.links_note.contains("not Weeb Labs"));
        let first_link = rows
            .iter()
            .position(|r| matches!(r, Row::Pair(a, _) if *a == table.links[0].title))
            .unwrap();
        assert!(first_link > at, "the links sit under the title");
        let frames = scrolled_to_the_end(120, 40);
        let all: String = frames.iter().map(|f| page_text(f)).collect();
        let squeezed = |t: &str| t.split_whitespace().collect::<String>();
        assert!(squeezed(&all).contains(&squeezed(&table.links_note)));
        // Section headings are drawn in capitals.
        assert!(squeezed(&all).contains(&squeezed(&table.links_title.to_uppercase())));
    }

    #[test]
    fn wp_tm22_about_keeps_the_app_name_version_and_device() {
        let (mut s, st) = screen(Page::About);
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("DSPi Terminal"), "{f}");
        assert!(f.contains("USB Audio DSP Controller"), "{f}");
        assert!(
            f.contains(&format!("Version {}", env!("CARGO_PKG_VERSION"))),
            "{f}"
        );
        // The probe's beta-aware version, as the Console spells it.
        assert!(f.contains("1.1.6 beta 4"), "{f}");
    }

    #[test]
    fn wp_tm22_about_fits_the_minimum_frame_and_scrolls_to_its_last_line() {
        let frames = scrolled_to_the_end(80, 24);
        assert!(frames.iter().all(|f| f.lines().count() == 24));
        assert!(frames[0].contains("DSPi Terminal"), "{}", frames[0]);
        assert!(frames.len() > 1, "the page should need scrolling");
        let last_credit = about().unwrap().credits.pop().unwrap();
        let last = frames.last().unwrap();
        assert!(last.contains(&last_credit.name), "{last}");
    }

    /// Text nobody would approve, in every field.
    fn hostile() -> About {
        let words = [
            "x".repeat(10_000),
            "寬字符".repeat(200),
            "line\nbreak\r\ttab \u{1b}[2J\u{1b}]0;title\u{7}".to_owned(),
            "\u{202E}reversed\u{2066}\u{200B}\u{FEFF}".to_owned(),
            "a\u{301}\u{302}\u{303}".repeat(100),
            "👩‍👩‍👧‍👦❤\u{FE0F}".repeat(50),
            String::new(),
            "   \n ".to_owned(),
        ];
        let mut t = About {
            draft: true,
            credit_lines: vec![],
            makers: vec![],
            links_title: String::new(),
            links_note: String::new(),
            links: vec![],
            credits: vec![],
        };
        t.links_title = words[0].clone();
        t.links_note = words[2].clone();
        for (i, w) in words.iter().enumerate() {
            let id = i.to_string();
            t.credit_lines.push(CreditLine {
                id: id.clone(),
                text: w.clone(),
            });
            t.makers.push(Maker {
                id: id.clone(),
                name: w.clone(),
                url: w.clone(),
                made: vec![w.clone(), w.clone()],
            });
            t.links.push(Link {
                id: id.clone(),
                title: w.clone(),
                detail: w.clone(),
                url: w.clone(),
            });
            t.credits.push(Credit {
                id,
                name: w.clone(),
                what: w.clone(),
                url: Some(w.clone()),
            });
        }
        t
    }

    fn theme() -> Theme {
        Theme::console(ColorDepth::TrueColor, Glyphs::Braille)
    }

    /// Draw rows stacked in a pane inside a larger buffer, as the page does, and hold
    /// that nothing lands outside the pane and every row takes the line it claims.
    fn assert_stays_inside(rows: &[Row], width: u16) {
        let pane = Rect::new(3, 0, width, 1);
        let mut y = 0u16;
        for row in rows {
            let h = row.height(width);
            assert_eq!(h, 1, "one line a row: {row:?}");
            let mut buf = Buffer::empty(Rect::new(0, 0, width + 40, 4));
            let before = buf.clone();
            let area = Rect {
                y: 1,
                height: h,
                ..pane
            };
            row.draw(area, &mut buf, &theme(), false);
            for by in 0..4u16 {
                for bx in 0..width + 40 {
                    let inside = bx >= area.x
                        && bx < area.x + area.width
                        && by >= area.y
                        && by < area.y + area.height;
                    if !inside {
                        assert_eq!(buf[(bx, by)], before[(bx, by)], "{bx},{by} {row:?}");
                    }
                }
            }
            y = y.saturating_add(h);
        }
        assert!(y > 0);
    }

    #[test]
    fn wp_tm22_hostile_text_cannot_leave_the_page_at_any_width() {
        let table = hostile();
        for width in [5, 6, 7, 21, 39, 59, 81, 200] {
            let mut rows = credit_rows(Some(&table), width);
            rows.extend(table_rows(Some(&table), width));
            assert!(rows.len() <= 2 * MAX_ROWS);
            assert_stays_inside(&rows, width);
        }
    }

    #[test]
    fn wp_tm23_a_wide_value_that_fits_stays_on_its_line() {
        let mut w = Writer::new(40);
        w.pair("寬字符", "寬字符");
        assert!(matches!(&w.rows[..], [Row::Pair(a, b)] if a == "寬字符" && b == "寬字符"));
    }

    #[test]
    fn wp_tm22_shown_text_holds_no_control_or_reordering_marks() {
        let table = hostile();
        let mut rows = credit_rows(Some(&table), 59);
        rows.extend(table_rows(Some(&table), 59));
        for row in &rows {
            let text = match row {
                Row::Note(t) | Row::Section(t) => t.clone(),
                Row::Pair(a, b) => format!("{a}{b}"),
                Row::Blank => String::new(),
                other => panic!("unexpected {other:?}"),
            };
            assert!(!text.chars().any(hidden), "{text:?}");
        }
        for line in plain_credits(&table) {
            assert!(!line.chars().any(hidden), "{line:?}");
        }
    }

    #[test]
    fn wp_tm22_any_table_size_keeps_the_row_count_bounded() {
        let mut table = hostile();
        let credit = table.credits[0].clone();
        table.credits = vec![credit; 2000];
        let rows = table_rows(Some(&table), 5);
        assert!(rows.len() <= MAX_ROWS);
        let total: u32 = rows.iter().map(|r| u32::from(r.height(5))).sum();
        assert!(total < u32::from(u16::MAX), "{total}");
    }

    #[test]
    fn wp_tm22_an_empty_or_unreadable_table_still_draws_a_page() {
        let empty = About {
            draft: false,
            credit_lines: vec![],
            makers: vec![],
            links_title: String::new(),
            links_note: String::new(),
            links: vec![],
            credits: vec![],
        };
        assert!(credit_rows(Some(&empty), 59).is_empty());
        let rows = table_rows(Some(&empty), 59);
        assert!(!rows.iter().any(|r| matches!(r, Row::Section(_))));
        assert!(matches!(&credit_rows(None, 59)[0], Row::Note(t) if t == NO_TABLE));
        assert!(!table_rows(None, 59).is_empty());
    }

    #[test]
    fn wp_tm22_fold_breaks_only_what_does_not_fit() {
        assert_eq!(fold("one two three", 7), ["one two", "three"]);
        assert_eq!(fold("abcdefgh ij", 3), ["abc", "def", "gh", "ij"]);
        assert_eq!(fold("寬寬寬", 4), ["寬寬", "寬"]);
        // A wide character takes a line of its own rather than vanishing.
        assert_eq!(fold("寬寬", 1), ["寬", "寬"]);
        assert!(fold("", 10).is_empty());
    }

    #[test]
    fn wp_tm22_clean_evens_out_whitespace_and_bounds_length() {
        assert_eq!(clean(" a\n\tb\u{1b}c\u{202E} "), "a bc");
        let long = clean(&"y".repeat(MAX_CHARS * 2));
        assert_eq!(long.chars().count(), MAX_CHARS + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn wp_tm22_version_credits_are_the_files_lines() {
        let table = about().unwrap();
        let texts: Vec<String> = table.credit_lines.iter().map(|l| l.text.clone()).collect();
        assert_eq!(credit_lines(), texts);
    }
}
