//! The interface before there is a device (WP UTM, UXP A13).
//!
//! `uconsole` with nothing plugged in used to print "no DSPi device found"
//! and end, with no word about the demo, the doctor or DSPi Link, although
//! the interface finds a device by itself once it runs. It now opens on this
//! page: it waits for a board and opens it the moment it is there, and offers
//! the demo, a hub and the doctor's checks meanwhile. The one-shot commands
//! keep their exit code.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use crate::theme::{Glyphs, Theme};
use crate::widgets::text::fit_left;

/// How often the bus is looked at for a device.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// The page's first line: what is happening, and the one thing a board in
/// its bootloader looks like instead.
pub const WAITING: &str = "No DSPi found · waiting for one (a board in BOOTSEL shows as RPI-RP2)";

/// What the person chose, or what arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// A DSPi is on the bus now.
    Device,
    /// `d`: the built-in demo device.
    Demo,
    /// `l`: a hub at this address; empty looks for one on the network.
    Link(String),
    /// `q`, Escape or Ctrl-C.
    Quit,
}

/// The page's state.
#[derive(Debug, Clone)]
pub struct Waiting {
    pub theme: Theme,
    /// The doctor's report, once `?` asked for it.
    pub doctor: Option<String>,
    /// The hub address being typed, after `l`.
    pub address: Option<String>,
    /// Why the last choice did not open, said on the page.
    pub note: Option<String>,
}

impl Waiting {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            doctor: None,
            address: None,
            note: None,
        }
    }

    /// One key. `doctor` writes the doctor's report, run only when asked.
    pub fn handle(&mut self, key: KeyEvent, doctor: &dyn Fn() -> String) -> Option<Choice> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Choice::Quit);
        }
        if let Some(address) = &mut self.address {
            match key.code {
                KeyCode::Esc => self.address = None,
                KeyCode::Enter => {
                    let a = address.trim().to_string();
                    self.address = None;
                    return Some(Choice::Link(a));
                }
                KeyCode::Backspace => {
                    address.pop();
                }
                KeyCode::Char(c) => address.push(c),
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Char('d') => Some(Choice::Demo),
            KeyCode::Char('l') => {
                self.address = Some(String::new());
                None
            }
            KeyCode::Char('?') => {
                self.doctor = match self.doctor {
                    Some(_) => None,
                    None => Some(doctor()),
                };
                None
            }
            KeyCode::Char('q') | KeyCode::Esc => Some(Choice::Quit),
            _ => None,
        }
    }

    pub fn draw(&self, area: Rect, buf: &mut Buffer) {
        let t = &self.theme;
        let w = area.width as usize;
        buf.set_string(
            area.x + 1,
            area.y,
            fit_left(
                &format!("{} · no device", dspi_cmd::COMMAND),
                w.saturating_sub(2),
            ),
            t.title(),
        );
        let card_w = area.width.saturating_sub(4).min(84);
        let x = area.x + (area.width - card_w) / 2;
        let y = area.y + 2;
        let card = Rect::new(x, y, card_w, 5.min(area.height.saturating_sub(2)));
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(if t.glyphs == Glyphs::Ascii {
                BorderType::Plain
            } else {
                BorderType::Rounded
            })
            .border_style(Style::default().fg(t.accent))
            .title(format!(" {WAITING} "))
            .title_style(t.title());
        let inner = block.inner(card);
        block.render(card, buf);
        let iw = inner.width as usize;
        let mut lx = inner.x + 1;
        for (key, does) in [
            ("[d]", "Try the demo"),
            ("[l]", "Connect over DSPi Link…"),
            ("[?]", "Why can't it see my device?"),
        ] {
            let room = (inner.x + inner.width).saturating_sub(lx) as usize;
            if room < key.len() + 2 {
                break;
            }
            buf.set_string(lx, inner.y, key, t.focused());
            lx += key.len() as u16 + 1;
            let text = fit_left(does, room.saturating_sub(key.len() + 1));
            buf.set_string(lx, inner.y, &text, t.value());
            lx += text.trim_end().chars().count() as u16 + 3;
        }
        if inner.height > 1 {
            buf.set_string(
                inner.x + 1,
                inner.y + 1,
                fit_left(
                    "Plug the DSPi in: this page opens it as soon as it is there.",
                    iw.saturating_sub(2),
                ),
                t.label(),
            );
        }
        if inner.height > 2 {
            buf.set_string(
                inner.x + 1,
                inner.y + 2,
                fit_left("q quits", iw.saturating_sub(2)),
                t.label(),
            );
        }
        let mut row = card.y + card.height + 1;
        let bottom = area.y + area.height;
        if let Some(address) = &self.address
            && row < bottom
        {
            let label = "Hub address (HOST[:PORT], empty looks on the network): ";
            buf.set_string(x + 1, row, fit_left(label, card_w as usize), t.label());
            let ax = x + 1 + label.chars().count() as u16;
            if ax < area.x + area.width {
                buf.set_string(ax, row, format!("{address}▏"), t.focused());
            }
            row += 2;
        }
        if let Some(note) = &self.note
            && row < bottom
        {
            buf.set_string(
                x + 1,
                row,
                fit_left(note, card_w as usize),
                t.warning_style(),
            );
            row += 2;
        }
        if let Some(report) = &self.doctor {
            for line in report.lines() {
                if row >= bottom {
                    break;
                }
                buf.set_string(x + 1, row, fit_left(line, card_w as usize), t.value());
                row += 1;
            }
        }
    }
}

/// Show the page until a device arrives or the person chooses. `found` looks
/// for a device (about once a second); `doctor` writes the doctor's report.
/// The terminal is the page's only while it runs.
pub fn run(
    page: &mut Waiting,
    found: &mut dyn FnMut() -> bool,
    doctor: &dyn Fn() -> String,
) -> io::Result<Choice> {
    let mut terminal = ratatui::try_init()
        .map_err(|e| io::Error::new(e.kind(), format!("cannot open the terminal: {e}")))?;
    let mut looked = Instant::now();
    let result = (|| -> io::Result<Choice> {
        loop {
            terminal.draw(|f| {
                let area = f.area();
                page.draw(area, f.buffer_mut());
            })?;
            if event::poll(Duration::from_millis(250))?
                && let TermEvent::Key(key) = event::read()?
                && key.kind == event::KeyEventKind::Press
                && let Some(choice) = page.handle(key, doctor)
            {
                return Ok(choice);
            }
            if page.address.is_none() && looked.elapsed() >= LOOK_EVERY {
                looked = Instant::now();
                if found() {
                    return Ok(Choice::Device);
                }
            }
        }
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorDepth;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn page() -> Waiting {
        Waiting::new(Theme::console(ColorDepth::TrueColor, Glyphs::Braille))
    }

    fn frame(p: &Waiting) -> String {
        crate::render_frame(100, 30, |area, buf| p.draw(area, buf))
    }

    /// UXP A13: the page says what is happening and offers the three ways on.
    #[test]
    fn utm_the_page_waits_and_offers_the_demo_a_hub_and_the_doctor() {
        let f = frame(&page());
        assert!(f.contains("No DSPi found · waiting for one"), "{f}");
        assert!(f.contains("RPI-RP2"), "{f}");
        assert!(f.contains("[d] Try the demo"), "{f}");
        assert!(f.contains("[l] Connect over DSPi Link…"), "{f}");
        assert!(f.contains("[?] Why can't it see my device?"), "{f}");
    }

    #[test]
    fn utm_d_opens_the_demo_and_q_quits() {
        let doctor = || String::new();
        assert_eq!(
            page().handle(key(KeyCode::Char('d')), &doctor),
            Some(Choice::Demo)
        );
        assert_eq!(
            page().handle(key(KeyCode::Char('q')), &doctor),
            Some(Choice::Quit)
        );
        assert_eq!(
            page().handle(key(KeyCode::Esc), &doctor),
            Some(Choice::Quit)
        );
    }

    #[test]
    fn utm_l_takes_a_hub_address() {
        let doctor = || String::new();
        let mut p = page();
        assert_eq!(p.handle(key(KeyCode::Char('l')), &doctor), None);
        for c in "hub.local".chars() {
            assert_eq!(
                p.handle(key(KeyCode::Char(c)), &doctor),
                None,
                "typed, not keys"
            );
        }
        assert!(frame(&p).contains("hub.local▏"), "{}", frame(&p));
        assert_eq!(
            p.handle(key(KeyCode::Enter), &doctor),
            Some(Choice::Link("hub.local".into()))
        );
        // Escape leaves the field, not the page.
        p.handle(key(KeyCode::Char('l')), &doctor);
        assert_eq!(p.handle(key(KeyCode::Esc), &doctor), None);
        assert!(p.address.is_none());
    }

    #[test]
    fn utm_question_mark_shows_the_doctors_report_on_the_page() {
        let doctor = || "  FAIL  no DSPi found (looking for 2E8B:FEAA)".to_string();
        let mut p = page();
        assert_eq!(p.handle(key(KeyCode::Char('?')), &doctor), None);
        let f = frame(&p);
        assert!(f.contains("FAIL  no DSPi found"), "{f}");
        p.handle(key(KeyCode::Char('?')), &doctor);
        assert!(p.doctor.is_none(), "? again hides it");
    }
}
