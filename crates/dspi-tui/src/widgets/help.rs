//! The `?` overlay: the focused region's keys, then the global ones.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use unicode_width::UnicodeWidthStr;

use super::text::fit_left;
use crate::theme::{Glyphs, Theme};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHelp {
    pub key: &'static str,
    pub does: &'static str,
}

impl KeyHelp {
    pub const fn new(key: &'static str, does: &'static str) -> Self {
        Self { key, does }
    }
}

pub struct HelpOverlay<'a> {
    pub title: &'a str,
    pub sections: Vec<(&'a str, &'a [KeyHelp])>,
    pub theme: &'a Theme,
}

impl<'a> HelpOverlay<'a> {
    pub fn new(title: &'a str, theme: &'a Theme) -> Self {
        Self {
            title,
            sections: Vec::new(),
            theme,
        }
    }
    pub fn section(mut self, name: &'a str, keys: &'a [KeyHelp]) -> Self {
        self.sections.push((name, keys));
        self
    }

    pub fn size(&self, max: Rect) -> Rect {
        let rows: usize = self
            .sections
            .iter()
            .map(|(_, k)| k.len() + 2)
            .sum::<usize>()
            + 1;
        // -- WP UTM -- (UXP A14) the widest key and the widest words, as the
        // columns are drawn: a long key on one row cut another row's words
        let widest = self.key_width()
            + self
                .sections
                .iter()
                .flat_map(|(_, k)| k.iter().map(|h| h.does.width()))
                .max()
                .unwrap_or(16)
            + 4;
        let w = (widest as u16 + 4).clamp(24, max.width.saturating_sub(4));
        let h = (rows as u16 + 2).min(max.height.saturating_sub(2));
        Rect::new(
            max.x + (max.width - w) / 2,
            max.y + (max.height - h) / 2,
            w,
            h,
        )
    }
}

impl HelpOverlay<'_> {
    // -- WP UTM --
    /// The key column's width, in cells.
    fn key_width(&self) -> usize {
        self.sections
            .iter()
            .flat_map(|(_, k)| k.iter().map(|h| h.key.width()))
            .max()
            .unwrap_or(4)
    }
}

impl Widget for HelpOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        Clear.render(area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(if t.glyphs == Glyphs::Ascii {
                BorderType::Plain
            } else {
                BorderType::Rounded
            })
            .border_style(Style::default().fg(t.accent))
            .title(format!(" {} ", self.title))
            .title_style(t.title());
        let inner = block.inner(area);
        block.render(area, buf);
        let key_w = self.key_width(); // -- WP UTM --
        let mut y = inner.y;
        for (name, keys) in &self.sections {
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_string(inner.x + 1, y, name.to_uppercase(), t.section());
            y += 1;
            for h in keys.iter() {
                if y >= inner.y + inner.height {
                    break;
                }
                buf.set_string(inner.x + 1, y, fit_left(h.key, key_w), t.focused());
                buf.set_string(
                    inner.x + 2 + key_w as u16,
                    y,
                    fit_left(h.does, (inner.width as usize).saturating_sub(key_w + 3)),
                    t.value(),
                );
                y += 1;
            }
            y += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::render;
    use super::*;
    use crate::theme::ColorDepth;

    #[test]
    fn help_lists_sections_of_keys() {
        let t = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        const KEYS: &[KeyHelp] = &[
            KeyHelp::new("↑ ↓", "Move between bands"),
            KeyHelp::new("Space", "Bypass band"),
        ];
        const GLOBAL: &[KeyHelp] = &[KeyHelp::new("Ctrl-P", "Search everything")];
        let s = render(
            HelpOverlay::new("Filters", &t)
                .section("Filters", KEYS)
                .section("Everywhere", GLOBAL),
            44,
            9,
        );
        assert!(s.contains(" Filters "), "{s}");
        // -- WP UTM -- the key column in cells: "↑ ↓" is three, not seven bytes
        assert!(s.contains("Space  Bypass band"), "{s}");
        assert!(s.contains("EVERYWHERE"), "{s}");
    }

    // -- WP UTM -- (UXP A14) "Graph height, pop-out, phase, p…" was cut
    #[test]
    fn utm_a_long_key_does_not_cut_the_words_beside_the_others() {
        let t = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        const KEYS: &[KeyHelp] = &[
            KeyHelp::new("M L X P S D U V G A T I B", "Open a tool"),
            KeyHelp::new("= g p . + -", "Graph height, pop-out, phase, partner, zoom"),
            KeyHelp::new("← →", "Change"),
        ];
        let max = Rect::new(0, 0, 120, 40);
        let o = HelpOverlay::new("Help", &t).section("Everywhere", KEYS);
        let r = o.size(max);
        let s = render(
            HelpOverlay::new("Help", &t).section("Everywhere", KEYS),
            r.width,
            r.height,
        );
        assert!(
            s.contains("Graph height, pop-out, phase, partner, zoom"),
            "{s}"
        );
    }
}
