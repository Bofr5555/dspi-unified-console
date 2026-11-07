//! The analyser's third-octave bars: the cell the Spectrum Analyser panel
//! draws, and the strip above a page (`SpectrumBarStrip`,
//! SpectrumAnalyserView.swift:1390-1560).
//!
//! The strip is one row of cells per channel, side by side before wrapping,
//! each naming its channel and carrying its own frequency labels since cells
//! in a row share nothing below them. The terminal has no drag handle or gear,
//! so [`BarPicture::layout`] picks the height and the columns from the room
//! it is given.
//!
//! Three inks, by [`Glyphs`]. Braille has two dot columns and four dot rows to
//! a cell, which fits a whole third-octave bank in a narrow cell; Blocks has
//! one bar to a cell in eighths of its height; ASCII has `#`, `-` for the
//! peak hold and `.` where the transform has no bin.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use crate::graph::{BRAILLE_BASE, BRAILLE_DOTS};
use crate::theme::{Glyphs, Theme};
use crate::widgets::text::truncate;

const BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Rows of a cell with the channel name and the labels: a lone channel gets
/// four rows of bars and several get three, near the Console's three quarters.
const CELL_ONE: u16 = 6;
const CELL_SEVERAL: u16 = 5;
/// A name and two rows of bars; below this a cell says nothing.
const CELL_MIN: u16 = 3;
/// Too narrow to read below this, so more columns are not worth having.
const CELL_MIN_WIDTH: u16 = 12;
/// The Console's most columns (`chosenColumns`, :1421).
const MAX_COLUMNS: usize = 4;
/// A cell shows its labels from this height.
const LABELS_FROM: u16 = 5;

/// What every cell is drawn against.
pub struct Scale<'a> {
    pub floor_db: f64,
    pub ceiling_db: f64,
    /// The device's band centres, for the labels.
    pub centres: &'a [u16],
    /// Whether the transform has a bin for this band.
    pub populated: &'a dyn Fn(usize) -> bool,
}

impl Scale<'_> {
    fn norm(&self, db: f64) -> f64 {
        ((db - self.floor_db) / (self.ceiling_db - self.floor_db).max(1.0)).clamp(0.0, 1.0)
    }
}

/// One channel's bars.
pub struct Cell<'a> {
    pub color: Color,
    pub name: &'a str,
    /// dBFS per band; empty until the first frame.
    pub levels: &'a [f64],
    /// The peak hold per band, when on.
    pub peaks: Option<&'a [f64]>,
}

/// How the bars take a plot's columns: `count` columns (dot columns in
/// braille) for `bands` bands.
#[derive(Clone, Copy)]
struct Columns {
    count: usize,
    bands: usize,
}

impl Columns {
    /// A wide plot spreads the bands across its width, band b from column
    /// b w / n.
    fn first(self, band: usize) -> usize {
        (band * self.count).div_ceil(self.bands)
    }

    /// With two columns a band or more, every bar is the same width with a
    /// gap after it, and with fewer the bars touch rather than gap at random.
    fn bar_width(self) -> usize {
        match self.count / self.bands {
            ..2 => self.count,
            w => w - 1,
        }
    }

    /// The bands column `x` draws, or `None` in a gap. A narrow plot shows in
    /// each column the loudest band that lands in it, so a tone is kept rather
    /// than averaged away.
    fn bands_at(self, x: usize) -> Option<std::ops::Range<usize>> {
        let b0 = x * self.bands / self.count;
        if self.count >= self.bands {
            return (x - self.first(b0) < self.bar_width()).then_some(b0..b0 + 1);
        }
        let b1 = ((x + 1) * self.bands / self.count).max(b0 + 1);
        Some(b0..b1.min(self.bands))
    }

    /// The column a band's label centres on.
    fn label_at(self, band: usize) -> usize {
        match self.count / self.bands {
            ..2 => self.first(band),
            _ => self.first(band) + self.bar_width() / 2,
        }
    }
}

/// The level and peak of column `x`, or what is drawn instead of them.
enum Column {
    Gap,
    Shaded,
    Bar { level: f64, peak: Option<f64> },
}

fn column(x: usize, columns: Columns, cell: &Cell, scale: &Scale) -> Column {
    let Some(bands) = columns.bands_at(x) else {
        return Column::Gap;
    };
    if bands.clone().all(|b| !(scale.populated)(b)) {
        return Column::Shaded;
    }
    let loudest = |values: &[f64]| {
        bands
            .clone()
            .filter_map(|b| values.get(b).copied())
            .fold(f64::MIN, f64::max)
    };
    Column::Bar {
        level: loudest(cell.levels),
        peak: cell.peaks.map(loudest),
    }
}

/// One channel's third-octave bars: a title, equal-width bars on the device's
/// own band centres, the peak hold as a cap, and shading where the transform
/// has no bin (`RtaBandsView`, SpectrumAnalyserView.swift:236-420).
pub fn draw_cell(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    cell: &Cell,
    scale: &Scale,
    glyphs: Glyphs,
    labels: bool,
) {
    if area.height < 2 || area.width < 4 {
        return;
    }
    buf.set_string(
        area.x,
        area.y,
        if glyphs == Glyphs::Ascii { "*" } else { "●" },
        Style::default().fg(cell.color),
    );
    buf.set_string(
        area.x + 2,
        area.y,
        truncate(cell.name, area.width.saturating_sub(3) as usize),
        theme.section(),
    );
    let plot_h = area.height - 1 - u16::from(labels);
    let plot = Rect::new(area.x, area.y + 1, area.width, plot_h);
    let bands = cell.levels.len();
    if bands == 0 || plot_h == 0 {
        return;
    }
    let per_cell = if glyphs == Glyphs::Braille { 2 } else { 1 };
    let columns = Columns {
        count: plot.width as usize * per_cell,
        bands,
    };
    match glyphs {
        Glyphs::Braille => draw_braille(plot, buf, theme, cell, scale, columns),
        Glyphs::Blocks | Glyphs::Ascii => draw_solid(
            plot,
            buf,
            theme,
            cell,
            scale,
            columns,
            glyphs == Glyphs::Ascii,
        ),
    }
    if labels {
        draw_labels(plot, buf, theme, scale, columns, per_cell);
    }
}

/// A bar to a column, in eighths of a cell or, in ASCII, half cells.
fn draw_solid(
    plot: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    cell: &Cell,
    scale: &Scale,
    columns: Columns,
    ascii: bool,
) {
    let plot_h = plot.height;
    for x in 0..columns.count {
        let px = plot.x + x as u16;
        let (level, peak) = match column(x, columns, cell, scale) {
            Column::Gap => continue,
            Column::Shaded => {
                for y in 0..plot_h {
                    buf[(px, plot.y + y)]
                        .set_symbol(if ascii { "." } else { "░" })
                        .set_style(Style::default().fg(theme.chrome_faint));
                }
                continue;
            }
            Column::Bar { level, peak } => (level, peak),
        };
        let eighths = (scale.norm(level) * plot_h as f64 * 8.0).round() as u16;
        for r in 0..plot_h {
            let fill = eighths.saturating_sub(r * 8).min(8);
            if fill == 0 {
                break;
            }
            let sym = match (ascii, fill) {
                (true, 4..) => "#",
                (true, _) => continue,
                (false, _) => BLOCKS[(fill - 1) as usize],
            };
            buf[(px, plot.y + plot_h - 1 - r)]
                .set_symbol(sym)
                .set_style(Style::default().fg(cell.color));
        }
        if let Some(p) = peak {
            let row = ((scale.norm(p) * plot_h as f64).ceil() as u16).min(plot_h);
            // A cap only where it clears the bar.
            if row > 0 && row * 8 > eighths + 4 {
                buf[(px, plot.y + plot_h - row)]
                    .set_symbol(if ascii { "-" } else { "▔" })
                    .set_style(Style::default().fg(cell.color));
            }
        }
    }
}

/// Bars of dots: two to a cell, four dots to a row.
fn draw_braille(
    plot: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    cell: &Cell,
    scale: &Scale,
    columns: Columns,
) {
    let (w, h) = (plot.width as usize, plot.height as usize);
    let rows = h * BRAILLE_DOTS.len();
    let mut dots = vec![0u16; w * h];
    let mut shaded = vec![false; w];
    for x in 0..columns.count {
        let (level, peak) = match column(x, columns, cell, scale) {
            Column::Gap => continue,
            Column::Shaded => {
                shaded[x / 2] = true;
                continue;
            }
            Column::Bar { level, peak } => (level, peak),
        };
        // Dot `d` up from the bottom of the plot.
        let mut set = |d: usize| {
            let (cy, dy) = ((rows - 1 - d) / 4, (rows - 1 - d) % 4);
            dots[cy * w + x / 2] |= BRAILLE_DOTS[dy][x % 2];
        };
        let height = ((scale.norm(level) * rows as f64).round() as usize).min(rows);
        (0..height).for_each(&mut set);
        if let Some(p) = peak {
            let top = ((scale.norm(p) * rows as f64).ceil() as usize).min(rows);
            // A cap only where a dot of air separates it from the bar.
            if top > height + 1 {
                set(top - 1);
            }
        }
    }
    for cy in 0..h {
        for cx in 0..w {
            let pos = (plot.x + cx as u16, plot.y + cy as u16);
            let bits = dots[cy * w + cx];
            if bits != 0 {
                let ch = char::from_u32((BRAILLE_BASE | bits) as u32).unwrap_or('?');
                buf[pos]
                    .set_char(ch)
                    .set_style(Style::default().fg(cell.color));
            } else if shaded[cx] {
                buf[pos]
                    .set_symbol("░")
                    .set_style(Style::default().fg(theme.chrome_faint));
            }
        }
    }
}

/// The frequency labels, on the row under the plot.
fn draw_labels(
    plot: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    scale: &Scale,
    columns: Columns,
    per_cell: usize,
) {
    let y = plot.y + plot.height;
    let mut last_end = 0u16;
    for (hz, label) in [(100u16, "100"), (1000, "1k"), (10_000, "10k")] {
        let Some(b) = scale
            .centres
            .iter()
            .position(|c| *c == hz)
            .filter(|b| *b < columns.bands)
        else {
            continue;
        };
        let x = (columns.label_at(b) / per_cell) as u16;
        let start = plot.x + x.saturating_sub(label.len() as u16 / 2);
        if start < last_end || start + label.len() as u16 > plot.x + plot.width {
            continue;
        }
        buf.set_string(start, y, label, theme.label());
        last_end = start + label.len() as u16 + 1;
    }
}

/// One channel in the strip.
#[derive(Debug, Clone, PartialEq)]
pub struct BarChannel {
    pub color: Color,
    pub name: String,
    pub levels: Vec<f64>,
    pub peaks: Option<Vec<f64>>,
}

/// What the strip shows; the runner gathers it for each frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarPicture {
    pub floor_db: f64,
    pub ceiling_db: f64,
    pub centres: Vec<u16>,
    /// Per band, as [`Scale::populated`].
    pub populated: Vec<bool>,
    pub channels: Vec<BarChannel>,
}

/// How the cells sit in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarLayout {
    pub columns: u16,
    pub cell_height: u16,
    /// The strip's rows in all.
    pub height: u16,
}

/// The tallest cells that fit `channels` channels in `width` by `max_rows`,
/// side by side as far as they go (`columnCount`, :1425); `None` when no
/// readable cell fits.
pub fn layout_for(channels: usize, width: u16, max_rows: u16) -> Option<BarLayout> {
    let want = if channels == 1 {
        CELL_ONE
    } else {
        CELL_SEVERAL
    };
    let mut best: Option<BarLayout> = None;
    for c in 1..=channels.min(MAX_COLUMNS) as u16 {
        if c > 1 && width / c < CELL_MIN_WIDTH {
            break;
        }
        let rows = channels.div_ceil(c as usize).min(u16::MAX as usize) as u16;
        let cell_height = want.min(max_rows / rows);
        if cell_height >= CELL_MIN && best.is_none_or(|b| cell_height >= b.cell_height) {
            best = Some(BarLayout {
                columns: c,
                cell_height,
                height: rows * cell_height,
            });
        }
    }
    best
}

impl BarPicture {
    /// How the strip sits in `width` by `max_rows`.
    pub fn layout(&self, width: u16, max_rows: u16) -> Option<BarLayout> {
        layout_for(self.channels.len(), width, max_rows)
    }
}

/// The strip, in the theme's glyphs. Draws nothing where no cell fits.
pub struct BarStrip<'a> {
    picture: &'a BarPicture,
    theme: &'a Theme,
}

impl<'a> BarStrip<'a> {
    pub fn new(picture: &'a BarPicture, theme: &'a Theme) -> Self {
        Self { picture, theme }
    }
}

impl Widget for BarStrip<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let p = self.picture;
        let Some(layout) = p.layout(area.width, area.height) else {
            return;
        };
        let populated = |b: usize| p.populated.get(b).copied().unwrap_or(true);
        let scale = Scale {
            floor_db: p.floor_db,
            ceiling_db: p.ceiling_db,
            centres: &p.centres,
            populated: &populated,
        };
        let cell_w = area.width / layout.columns;
        for (i, ch) in p.channels.iter().enumerate() {
            let (row, col) = (i as u16 / layout.columns, i as u16 % layout.columns);
            let rect = Rect::new(
                area.x + col * cell_w,
                area.y + row * layout.cell_height,
                // The gap between cells; the last column takes what is left.
                match col + 1 == layout.columns {
                    true => area.width - col * cell_w,
                    false => cell_w - 1,
                },
                layout.cell_height,
            );
            draw_cell(
                rect,
                buf,
                self.theme,
                &Cell {
                    color: ch.color,
                    name: &ch.name,
                    levels: &ch.levels,
                    peaks: ch.peaks.as_deref(),
                },
                &scale,
                self.theme.glyphs,
                layout.cell_height >= LABELS_FROM,
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm6 {
    use super::*;
    use crate::theme::ColorDepth;
    use crate::widgets::testing::render;

    /// IEC third-octave centres 12.5 Hz to 20 kHz: 33 bands.
    const CENTRES: [u16; 33] = [
        13, 16, 20, 25, 32, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000,
        1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000,
    ];

    fn theme(g: Glyphs) -> Theme {
        match g {
            Glyphs::Ascii => Theme::mono(g),
            _ => Theme::console(ColorDepth::TrueColor, g),
        }
    }

    fn channel(name: &str, levels: Vec<f64>, peaks: Option<Vec<f64>>) -> BarChannel {
        BarChannel {
            color: Color::Cyan,
            name: name.into(),
            levels,
            peaks,
        }
    }

    /// `channels` channels on a -90..+6 scale, all bands measured.
    fn picture(channels: Vec<BarChannel>) -> BarPicture {
        BarPicture {
            floor_db: -90.0,
            ceiling_db: 6.0,
            centres: CENTRES.to_vec(),
            populated: vec![true; CENTRES.len()],
            channels,
        }
    }

    /// A slope from the floor to the ceiling across the bands.
    fn ramp() -> Vec<f64> {
        (0..CENTRES.len())
            .map(|b| -90.0 + 96.0 * b as f64 / (CENTRES.len() - 1) as f64)
            .collect()
    }

    fn draw(p: &BarPicture, g: Glyphs, w: u16, h: u16) -> String {
        let t = theme(g);
        render(BarStrip::new(p, &t), w, h)
    }

    fn is_braille(c: char) -> bool {
        ('\u{2801}'..='\u{28FF}').contains(&c)
    }

    fn is_block(c: char) -> bool {
        ('▁'..='█').contains(&c)
    }

    #[test]
    fn wp_tm6_a_third_octave_strip_renders_in_ascii() {
        let p = picture(vec![channel("Out 1", ramp(), None)]);
        let out = draw(&p, Glyphs::Ascii, 66, 6);
        assert_eq!(
            out,
            [
                "* Out 1",
                "                                                        # # # # #",
                "                                        # # # # # # # # # # # # #",
                "                        # # # # # # # # # # # # # # # # # # # # #",
                "        # # # # # # # # # # # # # # # # # # # # # # # # # # # # #",
                "                 100                 1k                  10k",
            ]
            .join("\n")
        );
        assert!(out.is_ascii());
    }

    #[test]
    fn wp_tm6_a_third_octave_strip_renders_in_braille() {
        let p = picture(vec![channel("Out 1", ramp(), None)]);
        // 33 bands in 66 dot columns: a dot column for each and a gap after.
        let out = draw(&p, Glyphs::Braille, 33, 6);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "● Out 1");
        assert!(lines[1..5].iter().any(|l| l.chars().any(is_braille)));
        assert!(!out.chars().any(is_block), "dots, not blocks:\n{out}");
        assert!(lines[5].contains("1k"), "{out}");
        // The ramp rises left to right: the bottom row starts empty and fills,
        // and the top row only has dots at the right.
        assert!(lines[4].ends_with('⡇') || lines[4].ends_with('⣀'), "{out}");
        assert!(
            lines[1].trim_start().len() < lines[4].trim_start().len(),
            "{out}"
        );
    }

    #[test]
    fn wp_tm6_braille_fits_the_whole_bank_where_blocks_merge_bands() {
        // One loud band among silence, in a cell of 17 columns: braille has 34
        // dot columns, so neighbouring loud bands stay apart.
        let mut levels = vec![-90.0; CENTRES.len()];
        levels[10] = 0.0;
        levels[12] = 0.0;
        let p = picture(vec![channel("x", levels, None)]);
        let dots = draw(&p, Glyphs::Braille, 17, 6);
        let bottom = dots.lines().nth(4).unwrap();
        // Bands 10 and 12 land in separate dot columns of the same cell or in
        // neighbouring cells, with empty dots between them.
        let bits: Vec<u32> = bottom
            .chars()
            .filter(|c| is_braille(*c))
            .map(|c| c as u32 - 0x2800)
            .collect();
        let columns: usize = bits
            .iter()
            .map(|b| usize::from(b & 0x47 != 0) + usize::from(b & 0xB8 != 0))
            .sum();
        assert_eq!(columns, 2, "two separate bars:\n{dots}");
        // In blocks the same cell can only give them a column each or merge.
        assert!(draw(&p, Glyphs::Blocks, 17, 6).contains('█'));
    }

    #[test]
    fn wp_tm6_the_peak_hold_shows_as_a_cap_in_every_ink() {
        let levels = vec![-60.0; CENTRES.len()];
        let peaks = Some(vec![-12.0; CENTRES.len()]);
        let with = picture(vec![channel("x", levels.clone(), peaks)]);
        let without = picture(vec![channel("x", levels, None)]);
        assert!(draw(&with, Glyphs::Ascii, 66, 6).contains('-'));
        assert!(!draw(&without, Glyphs::Ascii, 66, 6).contains('-'));
        assert!(draw(&with, Glyphs::Blocks, 66, 6).contains('▔'));
        assert!(!draw(&without, Glyphs::Blocks, 66, 6).contains('▔'));
        // In braille the cap is the only thing in the upper rows.
        let top = |p: &BarPicture| {
            draw(p, Glyphs::Braille, 66, 6)
                .lines()
                .nth(1)
                .map(str::to_string)
        };
        assert!(top(&with).unwrap().chars().any(is_braille));
        assert!(!top(&without).unwrap_or_default().chars().any(is_braille));
    }

    #[test]
    fn wp_tm6_a_peak_at_the_bar_draws_no_cap() {
        let levels = vec![-42.0; CENTRES.len()];
        let at_bar = picture(vec![channel("x", levels.clone(), Some(levels.clone()))]);
        let without = picture(vec![channel("x", levels, None)]);
        for g in [Glyphs::Ascii, Glyphs::Blocks, Glyphs::Braille] {
            assert_eq!(draw(&at_bar, g, 66, 6), draw(&without, g, 66, 6), "{g:?}");
        }
    }

    #[test]
    fn wp_tm6_bands_with_no_bin_are_shaded_not_drawn_at_the_floor() {
        let mut p = picture(vec![channel("x", ramp(), None)]);
        for b in 14..20 {
            p.populated[b] = false;
        }
        let ascii = draw(&p, Glyphs::Ascii, 66, 6);
        assert!(ascii.lines().nth(1).unwrap().contains(". ."), "{ascii}");
        assert!(draw(&p, Glyphs::Braille, 66, 6).contains('░'));
        assert!(draw(&p, Glyphs::Blocks, 66, 6).contains('░'));
    }

    #[test]
    fn wp_tm6_a_narrow_cell_keeps_a_tone() {
        let mut levels = vec![-90.0; CENTRES.len()];
        levels[19] = 6.0;
        let p = picture(vec![channel("x", levels, None)]);
        for (g, mark) in [(Glyphs::Ascii, '#'), (Glyphs::Blocks, '█')] {
            let out = draw(&p, g, 12, 6);
            assert!(out.lines().nth(1).unwrap().contains(mark), "{g:?}\n{out}");
        }
        assert!(draw(&p, Glyphs::Braille, 12, 6).chars().any(is_braille));
    }

    #[test]
    fn wp_tm6_channels_sit_side_by_side_before_wrapping() {
        let at = |n, w, h| layout_for(n, w, h).map(|l| (l.columns, l.cell_height, l.height));
        assert_eq!(at(1, 80, 20), Some((1, 6, 6)));
        assert_eq!(at(1, 80, 5), Some((1, 5, 5)));
        assert_eq!(at(2, 80, 20), Some((2, 5, 5)));
        assert_eq!(at(4, 80, 20), Some((4, 5, 5)));
        assert_eq!(at(8, 80, 20), Some((4, 5, 10)));
        assert_eq!(at(4, 30, 20), Some((2, 5, 10)), "too narrow for four");
        assert_eq!(at(1, 80, 2), None);
        assert_eq!(at(8, 80, 5), None);
        assert_eq!(at(0, 80, 20), None);
    }

    #[test]
    fn wp_tm6_every_cell_names_its_channel() {
        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];
        let p = picture(names.iter().map(|n| channel(n, ramp(), None)).collect());
        for g in [Glyphs::Ascii, Glyphs::Blocks, Glyphs::Braille] {
            let out = draw(&p, g, 100, 10);
            for n in names {
                assert!(out.contains(n), "{n} {g:?}\n{out}");
            }
        }
    }

    #[test]
    fn wp_tm6_a_channel_without_a_frame_is_a_name_alone() {
        let p = picture(vec![channel("Waiting", vec![], None)]);
        assert_eq!(draw(&p, Glyphs::Braille, 40, 6).trim_end(), "● Waiting");
    }

    #[test]
    fn wp_tm6_odd_data_and_tiny_areas_do_not_panic() {
        let odd = vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300];
        let short_peaks = Some(vec![0.0]);
        let mut p = picture(vec![channel("x", odd, short_peaks)]);
        p.populated.clear();
        for g in [Glyphs::Ascii, Glyphs::Blocks, Glyphs::Braille] {
            for w in [1u16, 2, 3, 4, 5, 13, 40, 200] {
                for h in [1u16, 2, 3, 4, 7] {
                    draw(&p, g, w, h);
                    let t = theme(g);
                    let area = Rect::new(0, 0, w, h);
                    let mut buf = Buffer::empty(area);
                    let populated = |_: usize| false;
                    let scale = Scale {
                        floor_db: 0.0,
                        ceiling_db: 0.0,
                        centres: &[],
                        populated: &populated,
                    };
                    let levels = [0.0; 37];
                    let cell = Cell {
                        color: Color::Red,
                        name: "y",
                        levels: &levels,
                        peaks: None,
                    };
                    draw_cell(area, &mut buf, &t, &cell, &scale, g, true);
                }
            }
        }
    }
}
