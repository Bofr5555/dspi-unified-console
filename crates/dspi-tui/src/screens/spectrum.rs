//! The Spectrum Analyser panel: the Console's analyser window
//! (`SpectrumAnalyserView.swift:1488-1902`) as a tool panel on `A`.
//!
//! The Console's window mirrors a channel chooser that lives in the response
//! graph's gear popover (`:820-960`). The Terminal has no gear, so the panel
//! carries the chooser itself: an Inputs / Outputs switch, one chip per
//! channel in its colour, the Console's summary and Clear. Beneath it the
//! picture as Curves, Bars or Both, and the window's status line at the foot.
//!
//! The engine that talks to the device ([`dspi_session::rta::RtaEngine`])
//! lives in the shared state, not here: the runner ticks it inside its own
//! loop and budget. [`SpectrumState`] holds one subscription for everything
//! that draws the selection: this panel while it is open, the overlay on the
//! response graph while that is on screen, and the bar strip above the page
//! while that is. Nothing drawing, or no channel chosen, releases it, which is what makes the device stop the analyser.
//! No channel chosen while following another console still draws and polls
//! that console's choice (WP CG30).
//!
//! The device has one analyser and other consoles may be using it. The
//! Terminal does not link `dspi-core`, so it reaches the following state
//! through [`dspi_session::rta_sharing::Sharing`], which holds the same rules
//! for a client that drives the engine itself. While following, [`selection`]
//! is what the device analyses and every display draws that; a choice made
//! here, or Take control, ends it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use dspi_proto::dsp;
use dspi_proto::packets::RtaConfig;
use dspi_session::DeviceState;
use dspi_session::notify::{Event, Notification};
use dspi_session::rta::{
    Bins, Budget, Request, RtaEngine, TAP_INPUT, TAP_OUTPUT, TickReport, ViewId,
};
use dspi_session::rta_sharing::{Followed, Sharing};
use dspi_transport::{Transport, TransportKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::Shared;
use super::panel::{self, Body, ChipSpec, Row};
use crate::graph::{Graph, GraphCurve, GraphSettings, SpectrumOverlay};
use crate::settings::config::Spectrum as Settings;
use crate::shell::{Screen, ScreenEvent};
use crate::theme::{ChannelRole, Glyphs, Theme};
use crate::widgets::bar_strip;
use crate::widgets::text::{fit_centre, truncate, wrap};
use crate::widgets::{Action, BarChannel, BarPicture, ChipState, KeyHelp};

// ---------------------------------------------------------------------------
// What the panels share
// ---------------------------------------------------------------------------

/// The analyser's half of the shared state: the engine, the Settings page's
/// values, and the channels chosen, which outlive the panel as the Console's
/// dashboard selection outlives its window.
#[derive(Debug, Clone, Default)]
pub struct SpectrumState {
    pub engine: RtaEngine,
    pub settings: Settings,
    /// The chosen side and channels; `None` until someone chooses.
    pub selection: Option<Selection>,
    /// The side not showing, as it was left, so switching back restores it
    /// (`switchingSides`, SpectrumAnalyserView.swift:674-688).
    pub other_side: Option<Selection>,
    /// Driving the device's analyser, or following another console.
    pub sharing: Sharing,
    /// The one subscription for everything that draws the selection.
    view: Option<ViewId>,
    panel_open: bool,
    /// The spectrum on the response graph: `f`, off until asked for, since the
    /// analyser is the one feature that adds continuous traffic.
    overlay: bool,
    /// The response graph is on screen, which is when the overlay watches.
    overlay_shown: bool,
    overlay_glide: Glide,
    // -- WP TM6 --
    /// The bars above the page: `F`, off until asked for like the overlay.
    bars: bool,
    /// The strip has room on screen, which is when it watches.
    bars_shown: bool,
    bars_glide: Glide,
}

impl SpectrumState {
    /// Take the Settings page's values, pushing the device's half on the
    /// next tick. A change takes the analyser back from a console being
    /// followed, when something here is watching.
    pub fn adopt_settings(&mut self, s: &Settings) {
        self.settings = s.clone();
        if self.sharing.following().is_none() {
            self.engine.set_options(s.engine_options());
        }
    }

    fn watching(&self) -> bool {
        self.panel_open || (self.overlay && self.overlay_shown) || (self.bars && self.bars_shown) // -- WP TM6 --
    }

    /// What this console asks of the device; `None` while nothing here
    /// watches, the selection is empty or the caps are unknown.
    fn own_config(&self, state: &DeviceState) -> Option<RtaConfig> {
        let caps = self.engine.caps()?;
        if !self.watching() || !state.connected {
            return None;
        }
        let sel = own_selection(state, self);
        let fft_order = self
            .settings
            .transform_order
            .filter(|o| (caps.order_min..=caps.order_max).contains(o))
            .unwrap_or(caps.order_default);
        (sel.mask() != 0).then_some(RtaConfig {
            tap: sel.tap,
            channel_mask: sel.mask(),
            fft_order,
            avg_ms: self.settings.averaging_ms,
            peak_decay_db_s: self.settings.peak_decay_db_s,
            flags: 0,
        })
    }

    /// Keep the engine in step with what is drawn: the sharing rules judge
    /// this console's choice, and the engine then polls the device's
    /// configuration while following and this console's otherwise. Runs
    /// before every poll and after every choice.
    pub fn sync(&mut self, state: &DeviceState) {
        self.sharing.judge_own(self.own_config(state));
        self.engine.set_options(match self.sharing.following() {
            Some(f) => f.options(),
            None => self.settings.engine_options(),
        });
        let sel = selection(state, self);
        if !self.watching()
            || sel.channels.is_empty()
            || !self.engine.supported()
            || !state.connected
        {
            self.release();
            return;
        }
        let request = Request {
            tap: sel.tap,
            mask: sel.mask(),
            wants_bins: sel.channels.len() == 1,
        };
        match self.view {
            Some(id) => self.engine.update(id, request),
            None => self.view = Some(self.engine.subscribe(request)),
        }
    }

    fn release(&mut self) {
        if let Some(id) = self.view.take() {
            self.engine.release(id);
        }
    }

    /// One poll, under the sharing rules. The caller has just called
    /// [`Self::sync`].
    pub fn tick(&mut self, t: &mut dyn Transport, now: Instant) -> TickReport {
        self.sharing
            .tick(&mut self.engine, t, now, Budget::default())
    }

    /// Take a notification, if it says who holds the analyser.
    pub fn note(
        &mut self,
        note: &Notification,
        kind: TransportKind,
        session: Option<u16>,
        state: &DeviceState,
    ) {
        if matches!(note.event, Event::Unknown { .. }) {
            self.sharing.judge_own(self.own_config(state));
            self.sharing.note(note, kind, session, self.engine.caps());
        }
    }

    /// Another device, or the same one back: the following state belongs to
    /// the connection. What watches stays, so the next poll starts the
    /// analyser again.
    pub fn new_connection(&mut self) {
        self.sharing.reset();
    }

    /// Whether the response graph is on screen, from the runner.
    pub fn set_overlay_shown(&mut self, shown: bool) {
        self.overlay_shown = shown;
    }

    /// `f`: show or hide the spectrum on the response graph, and say which.
    /// With no channel chosen yet, showing it starts from the graphed
    /// channel, `page` (its tap, its channel at that tap).
    pub fn toggle_overlay(&mut self, page: Option<(u8, u8)>) -> bool {
        self.overlay = !self.overlay;
        if self.overlay {
            self.start_from(page);
        }
        self.overlay
    }

    /// With no channel chosen yet, a display starts from the page's channel.
    fn start_from(&mut self, page: Option<(u8, u8)>) {
        if self
            .selection
            .as_ref()
            .is_none_or(|c| c.channels.is_empty())
            && let Some((tap, channel)) = page
        {
            self.selection = Some(Selection::new(tap, vec![channel]));
        }
    }

    // -- WP TM6 --
    /// Whether the bar strip has room on screen, from the runner.
    pub fn set_bars_shown(&mut self, shown: bool) {
        self.bars_shown = shown;
    }

    /// `F`: show or hide the bars above the page, and say which. Starts as
    /// the overlay does.
    pub fn toggle_bars(&mut self, page: Option<(u8, u8)>) -> bool {
        self.bars = !self.bars;
        if self.bars {
            self.start_from(page);
        }
        self.bars
    }
}

/// The words for following, naming the session when the hub does
/// (SpectrumAnalyserView.swift:1995).
fn following_words(f: &Followed) -> String {
    match f.origin {
        0 => "Following another console".into(),
        o => format!("Following another console (session {o})"),
    }
}

/// Some channels at one tap: the device has one FFT engine listening at one
/// tap at a time, so inputs and outputs never mix (`RtaChannelSelection`,
/// SpectrumAnalyserView.swift:615-660).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub tap: u8,
    /// Channels at `tap` (input row, or output index), ascending.
    pub channels: Vec<u8>,
}

impl Selection {
    pub fn new(tap: u8, mut channels: Vec<u8>) -> Self {
        channels.sort_unstable();
        channels.dedup();
        Self { tap, channels }
    }

    pub fn mask(&self) -> u16 {
        self.channels
            .iter()
            .filter(|c| **c < 16)
            .fold(0, |m, c| m | 1 << c)
    }

    pub fn toggling(&self, channel: u8) -> Self {
        let mut channels = self.channels.clone();
        match channels.iter().position(|c| *c == channel) {
            Some(i) => {
                channels.remove(i);
            }
            None => channels.push(channel),
        }
        Self::new(self.tap, channels)
    }

    /// Only the channels in `live`, keeping the tap.
    pub fn restricted(&self, live: &[u8]) -> Self {
        Self::new(
            self.tap,
            self.channels
                .iter()
                .copied()
                .filter(|c| live.contains(c))
                .collect(),
        )
    }
}

/// The channels the analyser can show at `tap`: every live input row, or
/// every enabled output. Clamped to what the caps report, because a mask bit
/// for a channel the device lacks is a refused configuration (`rtaChannels`,
/// SpectrumAnalyserView.swift:691-699).
///
/// The live input rows are the device's: the active input count, and past
/// the stereo pair the rows the upmixer derives while it runs on one
/// (rta.c:95-106). The Console lists its own input count there; the
/// Terminal has no host-side count, and the device analyses no other rows.
pub fn channels(state: &DeviceState, engine: &RtaEngine, tap: u8) -> Vec<u8> {
    let reported = engine
        .caps()
        .map(|c| {
            if tap == TAP_INPUT {
                c.input_channels
            } else {
                c.output_channels
            }
        })
        .filter(|n| *n > 0)
        .unwrap_or(16)
        .min(16);
    if tap == TAP_INPUT {
        // Zero until the first meter read says otherwise.
        let live = match state.meters.active_inputs {
            0 => state.caps.num_inputs,
            n => n.saturating_add(super::matrix::derived_rows(state) as u8),
        };
        (0..live.min(state.caps.num_inputs).min(reported)).collect()
    } else {
        (0..state.caps.num_outputs.min(reported))
            .filter(|o| state.output(*o as usize).enabled)
            .collect()
    }
}

// -- WP INTE -- (owner 2026-11-24) the chooser is not an edit view: an output
// nothing is routed to leaves it, unless it is analysed, so it can still be
// switched off
/// The chips the chooser offers for `sel`: [`channels`] at its tap, less the
/// outputs nothing is routed to that `sel` does not analyse.
fn chooser_channels(state: &DeviceState, engine: &RtaEngine, sel: &Selection) -> Vec<u8> {
    let mut available = channels(state, engine, sel.tap);
    if sel.tap != TAP_INPUT {
        available.retain(|c| {
            sel.channels.contains(c)
                || !super::channel_unused(state, global_channel(state, sel.tap, *c))
        });
    }
    available
}

/// The selection on show: this console's, and while it follows another one
/// the device's tap and channels, so every display draws what the analyser is
/// doing (`effectiveSelection`, SpectrumAnalyser.swift:456), also with no
/// channel chosen here (`drawn(own:followed:)`, SpectrumAnalyserView.swift:682;
/// WP CG30). Nothing is asked for then: [`SpectrumState::own_config`] is
/// `None`, and the poll holds every push back while following.
pub fn selection(state: &DeviceState, s: &SpectrumState) -> Selection {
    let own = own_selection(state, s);
    match s.sharing.following() {
        Some(f) => {
            let mask = f.config.channel_mask;
            Selection::new(
                f.config.tap,
                (0..16).filter(|c| mask >> c & 1 != 0).collect(),
            )
        }
        _ => own,
    }
}

/// This console's selection: the chosen one, limited to channels live on this
/// device. Never chosen, or every chosen channel gone, falls back to the
/// first enabled output and then input 1; a chosen empty selection stays
/// empty (`dashboardRtaSelection`, SpectrumAnalyserView.swift:710-724).
fn own_selection(state: &DeviceState, s: &SpectrumState) -> Selection {
    if let Some(chosen) = &s.selection {
        if chosen.channels.is_empty() {
            return chosen.clone();
        }
        let live = chosen.restricted(&channels(state, &s.engine, chosen.tap));
        if !live.channels.is_empty() {
            return live;
        }
    }
    match channels(state, &s.engine, TAP_OUTPUT).first() {
        Some(first) => Selection::new(TAP_OUTPUT, vec![*first]),
        None => Selection::new(TAP_INPUT, vec![0]),
    }
}

/// A channel at a tap as the rest of the interface numbers it.
fn global_channel(state: &DeviceState, tap: u8, channel: u8) -> usize {
    if tap == TAP_INPUT {
        channel as usize
    } else {
        state.caps.num_inputs as usize + channel as usize
    }
}

fn channel_color(state: &DeviceState, theme: &Theme, tap: u8, channel: u8) -> Color {
    let ch = global_channel(state, tap, channel);
    theme.role_color(ChannelRole::of(
        ch as u8,
        state.caps.num_inputs,
        state.caps.num_outputs,
    ))
}

fn channel_label(state: &DeviceState, tap: u8, channel: u8) -> String {
    if tap == TAP_INPUT {
        // An upmixer row by what it carries, as the matrix names it.
        return super::matrix::row_name(state, channel as usize);
    }
    super::channel_label(state, global_channel(state, tap, channel))
}

/// The Console's summary under the chips (SpectrumAnalyserView.swift:936-942).
fn summary(sel: &Selection) -> String {
    match sel.channels.len() {
        0 => "Spectrum hidden".into(),
        1 => "1 channel".into(),
        n => format!("{n} channels"),
    }
}

/// `rtaShortHz` (SpectrumAnalyserView.swift:21-23).
fn short_hz(hz: u16) -> String {
    if hz >= 1000 {
        format!("{}k", (hz as f64 / 1000.0).round() as u32)
    } else {
        hz.to_string()
    }
}

// ---------------------------------------------------------------------------
// Curves
// ---------------------------------------------------------------------------

/// One channel's band levels as a curve on the graph's frequency grid,
/// joined in log frequency between the bands that have a reading. Bands the
/// transform cannot resolve are left out rather than drawn at the floor, so
/// the curve starts where the measurement does (`bandPoints`,
/// GraphSpectrumOverlay.swift:344-400); outside the measured span the curve
/// is absent.
pub fn band_curve(levels: &[f64], centres: &[u16], populated: &dyn Fn(usize) -> bool) -> Vec<f64> {
    let points: Vec<(f64, f64)> = levels
        .iter()
        .zip(centres)
        .enumerate()
        .filter(|(i, (v, c))| populated(*i) && v.is_finite() && **c > 0)
        .map(|(_, (v, c))| ((*c as f64).log10(), *v))
        .collect();
    dsp::frequencies()
        .iter()
        .map(|f| interpolate(&points, f.log10()))
        .collect()
}

fn interpolate(points: &[(f64, f64)], x: f64) -> f64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return f64::NAN;
    };
    if x < first.0 || x > last.0 {
        return f64::NAN;
    }
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
            return y0 + (y1 - y0) * t;
        }
    }
    last.1
}

/// One channel from both products: the bands below the top of the bass
/// bank, where the bins are too coarse to resolve a third-octave, and the
/// bins above it, crossfaded over the next three bands so there is no step
/// (`blend`, GraphSpectrumOverlay.swift:397-438). Where only one product has
/// data it is used alone.
pub fn blend(bands: &[f64], bins: &Bins, centres: &[u16], bass_bands: usize) -> Vec<f64> {
    let (lo, hi) = if bass_bands > 0 && bass_bands <= centres.len() {
        (
            (centres[bass_bands - 1] as f64).log10(),
            (centres[(bass_bands + 2).min(centres.len() - 1)] as f64).log10(),
        )
    } else {
        (f64::NEG_INFINITY, f64::NEG_INFINITY)
    };
    dsp::frequencies()
        .iter()
        .zip(bands)
        .map(|(f, band)| {
            let bin = bins.level_at(*f);
            match (band.is_finite(), bin) {
                (true, Some(b)) => {
                    let w = if hi > lo {
                        ((f.log10() - lo) / (hi - lo)).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    band + (b - band) * w
                }
                (true, None) => *band,
                (false, Some(b)) => b,
                (false, None) => f64::NAN,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Smoothing between frames
// ---------------------------------------------------------------------------

/// Glides the picture between device frames, one pole per value, stepped by
/// real elapsed time (`RtaBarSmoother`, SpectrumAnalyserView.swift:128-190).
/// The fall is slower than the rise, and a peak cap never eases upward. A
/// new identity (another channel, another band count) snaps rather than
/// sliding across from the old picture.
#[derive(Debug, Clone, Default)]
struct Glide {
    series: HashMap<(u8, u8, u8), (Vec<f64>, Instant)>,
}

impl Glide {
    fn step(
        &mut self,
        key: (u8, u8, u8),
        target: Vec<f64>,
        now: Instant,
        rise: Duration,
        fall: Duration,
    ) -> Vec<f64> {
        let entry = self.series.entry(key).or_insert((Vec::new(), now));
        if entry.0.len() != target.len() {
            *entry = (target.clone(), now);
            return target;
        }
        let dt = now.saturating_duration_since(entry.1).as_secs_f64();
        if dt <= 0.0 {
            return entry.0.clone();
        }
        entry.1 = now;
        let k = |tau: Duration| {
            if tau.is_zero() {
                1.0
            } else {
                1.0 - (-dt / tau.as_secs_f64()).exp()
            }
        };
        let (rise_k, fall_k) = (k(rise), k(fall));
        for (v, t) in entry.0.iter_mut().zip(&target) {
            if !v.is_finite() || !t.is_finite() {
                *v = *t;
            } else {
                *v += (t - *v) * if t > v { rise_k } else { fall_k };
            }
        }
        entry.0.clone()
    }
}

/// The fall time for the Smoothing setting: about one refresh interval, so a
/// bar is still moving when the next frame lands, clamped at both ends
/// (`rtaFallTau` with the Console's 0.35, SpectrumAnalyserView.swift:
/// 201-210 and DSPi_ConsoleApp.swift:118).
fn fall_tau(refresh: Duration) -> Duration {
    Duration::from_secs_f64((refresh.as_secs_f64() * 0.35).clamp(0.035, 0.40))
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// How the window draws the spectrum (`DisplayMode`,
/// SpectrumAnalyserView.swift:1576-1581).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Curves,
    Bars,
    Both,
}

const MODES: [&str; 3] = ["Curves", "Bars", "Both"];

/// SpectrumAnalyserView.swift:1046.
const TAKE_CONTROL: &str = "Take control";

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("↑ ↓", "Move"),
    KeyHelp::new("← →", "Choose"),
    KeyHelp::new("Space", "Show or hide a channel"),
    KeyHelp::new("Enter", "Clear"),
];

/// One channel's picture, gathered for drawing.
struct Trace {
    color: Color,
    name: String,
    avg: Vec<f64>,
    peak: Option<Vec<f64>>,
    /// The curve on the graph's frequency grid.
    curve: Vec<f64>,
    peak_curve: Option<Vec<f64>>,
}

/// Every chosen channel's picture, glided when Smoothing is on.
fn traces(
    s: &SpectrumState,
    state: &DeviceState,
    theme: &Theme,
    glide: &mut Glide,
    now: Instant,
) -> Vec<Trace> {
    let engine = &s.engine;
    let sel = selection(state, s);
    let single = sel.channels.len() == 1;
    let populated = |i: usize| engine.band_populated(i);
    let fall = fall_tau(engine.refresh_interval());
    let rise = fall.mul_f64(0.4);
    let mut out = Vec::new();
    for ch in &sel.channels {
        let Some(frame) = engine.frame(sel.tap, *ch) else {
            out.push(Trace {
                color: channel_color(state, theme, sel.tap, *ch),
                name: channel_label(state, sel.tap, *ch),
                avg: Vec::new(),
                peak: None,
                curve: Vec::new(),
                peak_curve: None,
            });
            continue;
        };
        let avg: Vec<f64> = frame
            .avg_levels()
            .iter()
            .map(|v| engine.level_db(*v))
            .collect();
        let peak: Vec<f64> = frame
            .peak_levels()
            .iter()
            .map(|v| engine.level_db(*v))
            .collect();
        let (avg, peak) = if s.settings.smoothing {
            (
                glide.step((sel.tap, *ch, 0), avg, now, rise, fall),
                glide.step((sel.tap, *ch, 1), peak, now, Duration::ZERO, fall),
            )
        } else {
            (avg, peak)
        };
        let bands = band_curve(&avg, engine.centres(), &populated);
        let curve = match engine.bins(sel.tap, *ch).filter(|_| single) {
            Some(bins) => {
                let blended = blend(&bands, bins, engine.centres(), engine.bass_bands());
                if s.settings.smoothing {
                    glide.step((sel.tap, *ch, 2), blended, now, rise, fall)
                } else {
                    blended
                }
            }
            None => bands,
        };
        let peak_on = s.settings.peak_hold;
        out.push(Trace {
            color: channel_color(state, theme, sel.tap, *ch),
            name: channel_label(state, sel.tap, *ch),
            peak_curve: peak_on.then(|| band_curve(&peak, engine.centres(), &populated)),
            peak: peak_on.then_some(peak),
            avg,
            curve,
        });
    }
    out
}

/// The traces as the graph takes them: peak contours first, in grey, so every
/// channel's own curve sits on top of them.
fn graph_curves(traces: &[Trace], theme: &Theme) -> Vec<GraphCurve> {
    let mut curves: Vec<GraphCurve> = traces
        .iter()
        .filter_map(|t| t.peak_curve.as_ref())
        .map(|p| GraphCurve {
            descriptor: String::new(),
            color: theme.dim,
            magnitude: p.clone(),
            phase: None,
            selected: false,
        })
        .collect();
    curves.extend(
        traces
            .iter()
            .filter(|t| !t.curve.is_empty())
            .map(|t| GraphCurve {
                descriptor: t.name.clone(),
                color: t.color,
                magnitude: t.curve.clone(),
                phase: None,
                selected: true,
            }),
    );
    curves
}

/// What the response graph draws behind its curves; `None` while the overlay
/// is off or has nothing to show (`GraphSpectrumOverlay.swift`).
pub fn overlay_picture(
    shared: &Shared,
    state: &DeviceState,
    theme: &Theme,
    now: Instant,
) -> Option<SpectrumOverlay> {
    let mut app = shared.try_borrow_mut().ok()?;
    let s = &mut app.spectrum;
    if !(s.overlay && s.overlay_shown && state.connected && s.engine.supported()) {
        return None;
    }
    let mut glide = std::mem::take(&mut s.overlay_glide);
    let picture = traces(s, state, theme, &mut glide, now);
    s.overlay_glide = glide;
    Some(SpectrumOverlay {
        floor_db: s.settings.floor_db as f64,
        ceiling_db: s.settings.ceiling_db as f64,
        curves: graph_curves(&picture, theme),
        note: s
            .sharing
            .following()
            .map(|f| format!("{}  t takes control", following_words(&f))),
    })
}

// -- WP TM6 --
/// What the bar strip above the page draws; `None` while it is off, covered,
/// or has no channel to show (`SpectrumBarStrip`, SpectrumAnalyserView.swift:
/// 1446-1447). Following shows the device's channels, as every display does.
pub fn bars_picture(
    shared: &Shared,
    state: &DeviceState,
    theme: &Theme,
    now: Instant,
) -> Option<BarPicture> {
    let mut app = shared.try_borrow_mut().ok()?;
    let s = &mut app.spectrum;
    if !(s.bars && s.bars_shown && state.connected && s.engine.supported()) {
        return None;
    }
    let mut glide = std::mem::take(&mut s.bars_glide);
    let picture = traces(s, state, theme, &mut glide, now);
    s.bars_glide = glide;
    if picture.is_empty() {
        return None;
    }
    let bands = picture.iter().map(|t| t.avg.len()).max().unwrap_or(0);
    Some(BarPicture {
        floor_db: s.settings.floor_db as f64,
        ceiling_db: s.settings.ceiling_db as f64,
        centres: s.engine.centres().to_vec(),
        populated: (0..bands).map(|b| s.engine.band_populated(b)).collect(),
        channels: picture
            .into_iter()
            .map(|t| BarChannel {
                color: t.color,
                name: t.name,
                levels: t.avg,
                peaks: t.peak,
            })
            .collect(),
    })
}

pub struct SpectrumPanel {
    shared: Shared,
    body: Body,
    chip: usize,
    mode: Mode,
    glide: Glide,
}

impl SpectrumPanel {
    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            body: Body::headless(),
            chip: 0,
            mode: Mode::Curves,
            glide: Glide::default(),
        }
    }

    /// Open and start watching at once, so the next tick already reads.
    pub fn open(shared: Shared, state: &DeviceState) -> Self {
        let mut p = Self::new(shared);
        p.sync(state);
        p
    }

    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Keep the engine's subscription in step with what is shown: watching
    /// while a channel is chosen, released when none is.
    pub fn sync(&mut self, state: &DeviceState) {
        let Ok(mut app) = self.shared.try_borrow_mut() else {
            return;
        };
        app.spectrum.panel_open = true;
        app.spectrum.sync(state);
    }

    fn supported(&self, state: &DeviceState) -> bool {
        state.connected && self.shared.borrow().spectrum.engine.supported()
    }

    fn rows(&self, state: &DeviceState, theme: &Theme) -> Vec<Row> {
        let app = self.shared.borrow();
        let sel = selection(state, &app.spectrum);
        let available = chooser_channels(state, &app.spectrum.engine, &sel);
        let mut rows = Vec::new();
        if let Some(f) = app.spectrum.sharing.following() {
            rows.push(Row::Buttons {
                label: following_words(&f),
                buttons: vec![TAKE_CONTROL.into()],
                cursor: 0,
            });
        }
        rows.push(Row::Segmented {
            label: "Spectrum".into(),
            choices: vec!["Inputs".into(), "Outputs".into()],
            selected: usize::from(sel.tap == TAP_OUTPUT),
            enabled: true,
        });
        if available.is_empty() {
            rows.push(Row::Caption(
                if sel.tap == TAP_INPUT {
                    "No active inputs."
                } else {
                    "No enabled outputs."
                }
                .into(),
            ));
        } else {
            rows.push(Row::Chips {
                chips: available
                    .iter()
                    .map(|ch| ChipSpec {
                        label: channel_label(state, sel.tap, *ch),
                        state: if sel.channels.contains(ch) {
                            ChipState::On
                        } else {
                            ChipState::Off
                        },
                        color: channel_color(state, theme, sel.tap, *ch),
                        enabled: true,
                        dimmed: false,
                    })
                    .collect(),
                cursor: self.chip.min(available.len() - 1),
                polarity: false,
            });
        }
        rows.push(Row::Buttons {
            label: summary(&sel),
            buttons: if sel.channels.is_empty() {
                Vec::new()
            } else {
                vec!["Clear".into()]
            },
            cursor: 0,
        });
        rows.push(Row::Segmented {
            label: "Display".into(),
            choices: MODES.iter().map(|m| m.to_string()).collect(),
            selected: self.mode as usize,
            enabled: true,
        });
        rows
    }

    fn act(&mut self, row: &Row, action: Action, state: &DeviceState) -> ScreenEvent {
        let mut app = self.shared.borrow_mut();
        let sel = selection(state, &app.spectrum);
        let available = chooser_channels(state, &app.spectrum.engine, &sel);
        let own = own_selection(state, &app.spectrum);
        let s = &mut app.spectrum;
        match (row, action) {
            (Row::Segmented { label, .. }, Action::Selected(i)) if label == "Display" => {
                self.mode = [Mode::Curves, Mode::Bars, Mode::Both][i.min(2)];
            }
            (Row::Segmented { .. }, Action::Selected(i)) => {
                // Choosing a side brings back what was checked there when it
                // was last left, or nothing. The side on show may be another
                // console's: what is put away is this console's.
                let tap = if i == 0 { TAP_INPUT } else { TAP_OUTPUT };
                if tap != sel.tap {
                    if tap != own.tap {
                        let restored = s
                            .other_side
                            .take()
                            .filter(|o| o.tap == tap)
                            .unwrap_or_else(|| Selection::new(tap, Vec::new()));
                        s.other_side = Some(s.selection.take().unwrap_or(own));
                        s.selection = Some(restored);
                    }
                    s.sharing.take_control();
                    self.chip = 0;
                }
            }
            (Row::Chips { .. }, Action::Selected(i)) => self.chip = i,
            (Row::Chips { .. }, Action::Chip(i, _)) => {
                if let Some(ch) = available.get(i) {
                    // A choice made here takes the analyser back even when it
                    // leaves this console's selection as it was (`userChose`,
                    // SpectrumAnalyserView.swift:929-932).
                    s.selection = Some(sel.toggling(*ch));
                    s.sharing.take_control();
                }
            }
            (Row::Buttons { buttons, .. }, Action::Button(_))
                if buttons.first().is_some_and(|b| b == TAKE_CONTROL) =>
            {
                s.sharing.take_control();
            }
            (Row::Buttons { buttons, .. }, Action::Button(_)) if !buttons.is_empty() => {
                s.selection = Some(Selection::new(sel.tap, Vec::new()));
                s.sharing.take_control();
            }
            _ => {}
        }
        drop(app);
        self.sync(state);
        ScreenEvent::Handled
    }

    fn draw_curves(&self, area: Rect, buf: &mut Buffer, theme: &Theme, traces: &[Trace]) {
        let app = self.shared.borrow();
        let s = &app.spectrum.settings;
        let settings = GraphSettings {
            min_hz: app.graph.min_hz,
            max_hz: app.graph.max_hz,
            db_center: (s.floor_db + s.ceiling_db) as f64 / 2.0,
            db_range: (s.ceiling_db - s.floor_db) as f64,
            show_phase: false,
            unwrap_phase: false,
            freq_grid: true,
            freq_labels: true,
            db_grid: true,
            db_labels: true,
            grid: app.graph.grid,
            ..GraphSettings::default()
        };
        let curves = graph_curves(traces, theme);
        let graph = Graph::new(&curves, &settings, theme);
        let plot = graph.plot_area(area);
        ratatui::widgets::Widget::render(graph, area, buf);
        // A cell has no opacity: a strength in the lower half of the
        // Console's slider draws the curves dim instead.
        if s.strength_pct < Settings::DIM_BELOW {
            for y in plot.y..plot.y + plot.height {
                for x in plot.x..plot.x + plot.width {
                    let cell = &mut buf[(x, y)];
                    if cell
                        .symbol()
                        .chars()
                        .next()
                        .is_some_and(|c| ('\u{2800}'..='\u{28FF}').contains(&c))
                    {
                        cell.modifier.insert(Modifier::DIM);
                    }
                }
            }
        }
    }

    fn draw_bars(&self, area: Rect, buf: &mut Buffer, theme: &Theme, traces: &[Trace]) {
        let n = traces.len();
        if n == 0 || area.height < 2 || area.width < 8 {
            return;
        }
        // The fewest columns that give every cell a title and four rows of
        // bars; failing that, as many columns as the Console allows.
        let fits = |c: usize| {
            let rows = n.div_ceil(c) as u16;
            (area.height / rows, area.width / c as u16)
        };
        let cols = (1..=n.min(4))
            .find(|c| fits(*c).0 >= 5 && fits(*c).1 >= 16)
            .unwrap_or(n.min(4));
        let (cell_h, cell_w) = fits(cols);
        let app = self.shared.borrow();
        let s = &app.spectrum;
        for (i, t) in traces.iter().enumerate() {
            let (r, c) = ((i / cols) as u16, (i % cols) as u16);
            if cell_h < 2 || (r + 1) * cell_h > area.height {
                break;
            }
            let cell = Rect::new(
                area.x + c * cell_w,
                area.y + r * cell_h,
                cell_w.saturating_sub(u16::from(c + 1 < cols as u16)),
                cell_h,
            );
            draw_bar_cell(cell, buf, theme, t, s, cell_h >= 6);
        }
    }

    fn draw_status(&self, area: Rect, buf: &mut Buffer, theme: &Theme, shows_bars: bool) {
        if area.height == 0 {
            return;
        }
        let app = self.shared.borrow();
        let engine = &app.spectrum.engine;
        let st = engine.status().copied().unwrap_or_default();
        let running = engine.running();
        let ascii = theme.glyphs == Glyphs::Ascii;
        let dot = match (running, ascii) {
            (true, true) => "*",
            (false, true) => "o",
            (true, false) => "●",
            (false, false) => "○",
        };
        buf.set_string(
            area.x + 1,
            area.y,
            dot,
            Style::default().fg(if running { theme.ok } else { theme.dim }),
        );
        let mut parts = vec![
            if running { "Running" } else { "Idle" }.to_string(),
            engine.refresh_description(),
        ];
        if st.frames_per_s > 0 {
            parts.push(format!("{} frames/s", st.frames_per_s));
        }
        if st.last_frame_us > 0 {
            parts.push(format!(
                "transform {} {}",
                st.last_frame_us,
                if ascii { "us" } else { "µs" }
            ));
        }
        if st.busy_us_per_s > 0 {
            // Ten thousand microseconds a second is one percent.
            parts.push(format!(
                "main loop {:.1}%",
                st.busy_us_per_s as f64 / 10_000.0
            ));
        }
        if st.bass_busy_us_per_s > 0 {
            let saturated = st.bass_busy_us_per_s == u16::MAX;
            parts.push(format!(
                "bass {}{:.2}%",
                match (saturated, ascii) {
                    (false, _) => "",
                    (true, true) => ">=",
                    (true, false) => "≥",
                },
                st.bass_busy_us_per_s as f64 / 10_000.0
            ));
        }
        // Whole items only: a telemetry figure cut in half reads as another
        // number. What does not fit the first line starts the second.
        let room = area.width.saturating_sub(3) as usize;
        let mut lines = [String::new(), String::new()];
        let mut row = 0;
        for p in parts {
            let next = if lines[row].is_empty() {
                p.clone()
            } else {
                format!("{}  {p}", lines[row])
            };
            if next.chars().count() <= room {
                lines[row] = next;
            } else if row == 0 && area.height >= 2 {
                row = 1;
                lines[1] = p;
            } else {
                break;
            }
        }
        let [line, rest] = lines;
        buf.set_string(area.x + 3, area.y, &line, theme.label());

        // The notice: a refusal, the remedy for shaded bands, or the range
        // this device can be trusted over (SpectrumAnalyserView.swift:1874-1885).
        let caps = engine.caps().copied().unwrap_or_default();
        let low_band = (shows_bars && running && engine.fft_order() < caps.order_max)
            .then(|| engine.lowest_measurable_centre())
            .flatten();
        let following = app.spectrum.sharing.following();
        let (notice, style) = if engine.rejected() && following.is_none() {
            (
                "Device refused this configuration".to_string(),
                theme.warning_style(),
            )
        } else if let Some(f) = following {
            (following_words(&f), theme.label())
        } else if let Some(hz) = low_band {
            (
                format!(
                    "shaded bands below {} Hz need a larger transform size in Settings",
                    short_hz(hz)
                ),
                theme.warning_style(),
            )
        } else if caps.dynamic_range_db > 0 {
            (
                format!(
                    "{}/{} dB range",
                    caps.dynamic_range_db, caps.bass_dynamic_range_db
                ),
                theme.label(),
            )
        } else {
            (String::new(), theme.label())
        };
        if notice.is_empty() {
            if area.height >= 2 {
                buf.set_string(area.x + 3, area.y + 1, &rest, theme.label());
            }
            return;
        }
        if area.height >= 2 {
            // The notice keeps the right of the second line; telemetry that
            // spilled over gives way to it when both will not fit.
            let w = notice.chars().count() as u16;
            let spill = rest.chars().count() as u16;
            if spill > 0 && 3 + spill + 2 + w <= area.width {
                buf.set_string(area.x + 3, area.y + 1, &rest, theme.label());
                buf.set_string(area.x + area.width - w - 1, area.y + 1, &notice, style);
            } else {
                buf.set_string(area.x + 3, area.y + 1, truncate(&notice, room), style);
            }
        } else {
            let w = notice.chars().count() as u16;
            let used = 3 + line.chars().count() as u16 + 2;
            if used + w <= area.width {
                buf.set_string(area.x + area.width - w, area.y, &notice, style);
            }
        }
    }

    fn draw_notice(area: Rect, buf: &mut Buffer, theme: &Theme, title: &str, body: &str) {
        if area.height == 0 {
            return;
        }
        let mid = area.y + area.height.saturating_sub(2) / 2;
        buf.set_string(
            area.x,
            mid,
            fit_centre(title, area.width as usize),
            theme.title(),
        );
        for (i, line) in wrap(body, area.width.saturating_sub(4) as usize, 3)
            .iter()
            .enumerate()
        {
            let y = mid + 1 + i as u16;
            if y >= area.y + area.height {
                break;
            }
            buf.set_string(
                area.x,
                y,
                fit_centre(line, area.width as usize),
                theme.label(),
            );
        }
    }
}

impl Drop for SpectrumPanel {
    /// Closing the panel releases its subscription; when it was the last,
    /// the engine stops the device on the next tick.
    fn drop(&mut self) {
        if let Ok(mut app) = self.shared.try_borrow_mut() {
            app.spectrum.panel_open = false;
            if !app.spectrum.watching() {
                app.spectrum.release();
            }
        }
    }
}

/// One channel's bars, the same as the strip's cells but in blocks on any
/// terminal that has them: a panel has the room to want their finer height.
fn draw_bar_cell(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    t: &Trace,
    s: &SpectrumState,
    labels: bool,
) {
    let populated = |b: usize| s.engine.band_populated(b);
    bar_strip::draw_cell(
        area,
        buf,
        theme,
        &bar_strip::Cell {
            color: t.color,
            name: &t.name,
            levels: &t.avg,
            peaks: t.peak.as_deref(),
        },
        &bar_strip::Scale {
            floor_db: s.settings.floor_db as f64,
            ceiling_db: s.settings.ceiling_db as f64,
            centres: s.engine.centres(),
            populated: &populated,
        },
        match theme.glyphs {
            Glyphs::Ascii => Glyphs::Ascii,
            _ => Glyphs::Blocks,
        },
        labels,
    );
}

impl Screen for SpectrumPanel {
    fn title(&self) -> String {
        "Spectrum Analyser".into()
    }

    fn draw(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        state: &DeviceState,
        focused: bool,
    ) {
        if area.height == 0 {
            return;
        }
        self.sync(state);
        if !self.supported(state) {
            panel::draw_unsupported(
                area,
                buf,
                theme,
                "Spectrum analyser unavailable",
                if state.connected {
                    "The connected firmware does not provide a compatible analyser. Update the \
                     firmware to use it."
                } else {
                    "Connect a DSPi to use the analyser."
                },
            );
            return;
        }
        let rows = self.rows(state, theme);
        self.body.clamp(&rows);
        let heights: Vec<u16> = rows.iter().map(|r| r.height(area.width)).collect();
        let controls_h = heights.iter().sum::<u16>().min(area.height);
        let focus_row = self.body.focused_row(&rows);
        panel::draw_rows(
            Rect::new(area.x, area.y, area.width, controls_h),
            buf,
            theme,
            &rows,
            0,
            focus_row,
            focused,
            None,
        );
        let status_h = if area.height >= controls_h + 10 { 2 } else { 1 };
        let top = area.y + controls_h + 1;
        let bottom = (area.y + area.height).saturating_sub(status_h);
        if bottom <= top {
            return;
        }
        let display = Rect::new(area.x + 1, top, area.width.saturating_sub(2), bottom - top);
        let sel = selection(state, &self.shared.borrow().spectrum);
        let shows_bars = !sel.channels.is_empty() && self.mode != Mode::Curves;
        if sel.channels.is_empty() {
            Self::draw_notice(
                display,
                buf,
                theme,
                "No channels selected",
                "Choose channels above to see their spectrum. The analyser runs only while \
                 something is watching it.",
            );
        } else {
            let traces = {
                let app = self.shared.borrow();
                traces(&app.spectrum, state, theme, &mut self.glide, Instant::now())
            };
            match self.mode {
                Mode::Curves => self.draw_curves(display, buf, theme, &traces),
                Mode::Bars => self.draw_bars(display, buf, theme, &traces),
                Mode::Both => {
                    let upper = display.height / 2;
                    self.draw_curves(
                        Rect::new(display.x, display.y, display.width, upper),
                        buf,
                        theme,
                        &traces,
                    );
                    self.draw_bars(
                        Rect::new(
                            display.x,
                            display.y + upper,
                            display.width,
                            display.height - upper,
                        ),
                        buf,
                        theme,
                        &traces,
                    );
                }
            }
        }
        self.draw_status(
            Rect::new(area.x, bottom, area.width, status_h),
            buf,
            theme,
            shows_bars,
        );
    }

    fn handle(&mut self, key: KeyEvent, state: &DeviceState) -> ScreenEvent {
        if !self.supported(state) {
            return ScreenEvent::Unhandled;
        }
        let rows = self.rows(state, panel::key_theme());
        self.body.clamp(&rows);
        match panel::dispatch(&mut self.body, &rows, key) {
            panel::Step::Done(ev) => ev,
            panel::Step::Row(i, action) => self.act(&rows[i], action, state),
            panel::Step::Header | panel::Step::Commit(..) => ScreenEvent::Handled,
        }
    }

    fn keys(&self) -> &'static [KeyHelp] {
        KEYS
    }
}

// ---------------------------------------------------------------------------
// A picture without a device
// ---------------------------------------------------------------------------

/// Fill the shared analyser with an RP2350's caps and a plausible picture,
/// for the gallery and the golden frames: a pink-ish slope with a bump in
/// the bass on every channel at `tap`, bins for the first, and a running
/// status.
pub mod demo {
    use std::time::Instant;

    use dspi_proto::packets::{RtaBandFrame, RtaBinFrame, RtaBinHeader, RtaCaps, RtaStatus};
    use dspi_session::rta::TAP_OUTPUT;

    use super::{Selection, Shared};

    /// `rta_band_centre_tab` (rta_tables.h:818-822).
    pub const CENTRES: [u16; 37] = [
        10, 13, 16, 20, 25, 32, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800,
        1000, 1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000,
        25000, 31500, 40000,
    ];

    /// The RP2350's caps (survey-firmware-beta4 3.2).
    pub fn caps() -> RtaCaps {
        RtaCaps {
            input_channels: 8,
            output_channels: 9,
            order_min: 8,
            order_max: 10,
            order_default: 10,
            bass_bands: 14,
            max_bands: 37,
            level_zero: 243,
            dynamic_range_db: 120,
            idle_timeout_ms: 5000,
            max_bin_frame: 529,
            bass_dynamic_range_db: 70,
        }
    }

    /// A level byte for `db` dBFS (rta_fft.h:44-45).
    fn level(db: f64) -> u8 {
        (243.0 + db * 2.0).round().clamp(0.0, 255.0) as u8
    }

    /// Channel `ch`'s band picture: -18 dBFS in the bass falling 3 dB an
    /// octave, a little different per channel.
    pub fn frame(ch: u8) -> RtaBandFrame {
        let mut avg = [0u8; RtaBandFrame::MAX_BANDS];
        let mut peak = [0u8; RtaBandFrame::MAX_BANDS];
        for (i, (a, p)) in avg.iter_mut().zip(peak.iter_mut()).enumerate() {
            let octaves = (i as f64 - 8.0) / 3.0;
            let bump = if (5..=10).contains(&i) { 6.0 } else { 0.0 };
            let db = -24.0 - 3.0 * octaves.max(0.0) + bump - 2.0 * ch as f64;
            *a = level(db);
            *p = level(db + 6.0);
        }
        RtaBandFrame {
            channel: ch,
            seq: 1,
            n_bands: 34,
            age_ms: 12,
            avg,
            peak,
        }
    }

    /// The same shape at 1024 points, with a tone at 1 kHz.
    pub fn bins(ch: u8) -> RtaBinFrame {
        let levels = (0..512)
            .map(|k| {
                let hz = k as f64 * 48_000.0 / 1024.0;
                let octaves = (hz.max(20.0) / 100.0).log2();
                let tone = if (21..=22).contains(&k) { 30.0 } else { 0.0 };
                level(-30.0 - 3.0 * octaves.max(0.0) + tone)
            })
            .collect();
        RtaBinFrame {
            header: RtaBinHeader {
                channel: ch,
                seq: 1,
                fft_order: 10,
                sample_rate_hz: 48_000,
                n_bins: 512,
            },
            levels,
        }
    }

    pub fn status(live: u8) -> RtaStatus {
        RtaStatus {
            state: 1,
            tap: TAP_OUTPUT,
            channel: 0,
            live_count: live,
            live_mask: (1u16 << live) - 1,
            frames_per_s: 47,
            busy_us_per_s: 43_000,
            last_frame_us: 850,
            idle_ms: 20,
            sample_rate_hz: 48_000,
            first_band: 0,
            bass_busy_us_per_s: 1_200,
        }
    }

    /// Load the analyser with `channels` chosen at `tap`, each with a
    /// picture, and bins for a one-channel choice.
    pub fn load(shared: &Shared, tap: u8, channels: &[u8]) {
        let mut app = shared.borrow_mut();
        let s = &mut app.spectrum;
        s.engine.load(caps(), CENTRES.to_vec());
        let now = Instant::now();
        s.engine
            .accept_frames(tap, channels.iter().map(|c| frame(*c)).collect(), now);
        if let [one] = channels {
            s.engine.accept_bins(tap, &bins(*one), now);
        }
        s.engine.accept_status(status(channels.len().max(1) as u8));
        s.selection = Some(Selection::new(tap, channels.to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::panel::testing;
    use crate::shell::Tool;
    use crate::widgets::testing::key;
    use crossterm::event::KeyCode;

    fn shared_with(tap: u8, channels: &[u8]) -> Shared {
        let shared = crate::screens::shared();
        demo::load(&shared, tap, channels);
        shared
    }

    fn frame_of(shared: &Shared, state: &DeviceState, w: u16, h: u16) -> String {
        testing::frame(
            Tool::Spectrum,
            Box::new(SpectrumPanel::new(shared.clone())),
            state,
            w,
            h,
        )
    }

    fn both_sizes(shared: &Shared, state: &DeviceState, check: impl Fn(&str, u16, u16)) {
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = frame_of(shared, state, w, h);
            assert_eq!(f.lines().count(), h as usize, "{w}x{h}");
            assert!(f.contains("Spectrum Analyser"), "{w}x{h}:\n{f}");
            assert!(f.contains("A closes"), "{w}x{h}:\n{f}");
            check(&f, w, h);
        }
    }

    #[test]
    fn golden_frames_unavailable() {
        let state = testing::state();
        // A firmware without the analyser: caps never read.
        let shared = crate::screens::shared();
        both_sizes(&shared, &state, |f, w, h| {
            assert!(f.contains("Spectrum analyser unavailable"), "{w}x{h}:\n{f}");
            assert!(f.contains("does not provide a compatible"), "{w}x{h}:\n{f}");
            assert!(!f.contains("Outputs"), "no controls: {f}");
        });
        let mut gone = state.clone();
        gone.connected = false;
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let f = frame_of(&shared, &gone, 120, 40);
        assert!(f.contains("Connect a DSPi to use the analyser."), "{f}");
    }

    #[test]
    fn golden_frames_no_channels() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[]);
        both_sizes(&shared, &state, |f, w, h| {
            assert!(f.contains("No channels selected"), "{w}x{h}:\n{f}");
            assert!(f.contains("Spectrum hidden"), "{w}x{h}:\n{f}");
            let summary = f.lines().find(|l| l.contains("Spectrum hidden")).unwrap();
            assert!(!summary.contains("Clear"), "nothing to clear: {summary}");
            assert!(f.contains("Inputs") && f.contains("Outputs"), "{f}");
        });
        // Nothing chosen means nothing watched.
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        assert!(!shared.borrow().spectrum.engine.watching());
    }

    #[test]
    fn golden_frames_curves_one_channel() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        both_sizes(&shared, &state, |f, w, h| {
            assert!(f.contains("1 channel"), "{w}x{h}:\n{f}");
            assert!(f.contains("Clear"), "{f}");
            assert!(f.contains("Curves"), "{f}");
            assert!(f.contains("-40"), "the dB axis: {f}");
            assert!(f.contains("1k"), "the frequency axis: {f}");
            assert!(
                f.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
                "a curve: {w}x{h}:\n{f}"
            );
            assert!(f.contains("Running"), "{f}");
            assert!(f.contains("1 channel, each refreshed every 21 ms"), "{f}");
        });
        let f = frame_of(&shared, &state, 120, 40);
        assert!(f.contains("47 frames/s"), "{f}");
        assert!(f.contains("transform 850 µs"), "{f}");
        assert!(f.contains("main loop 4.3%"), "{f}");
        assert!(f.contains("bass 0.12%"), "{f}");
        assert!(f.contains("120/70 dB range"), "{f}");
    }

    #[test]
    fn golden_frames_curves_several_channels() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0, 1, 8]);
        both_sizes(&shared, &state, |f, w, h| {
            assert!(f.contains("3 channels"), "{w}x{h}:\n{f}");
            assert!(
                f.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
                "curves: {w}x{h}:\n{f}"
            );
        });
        // Band curves only: several channels rotate, so no bins are wanted.
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        let want = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!(want.channel_mask, 0b1_0000_0011);
    }

    #[test]
    fn golden_frames_bars() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0, 1]);
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = testing::frame(
                Tool::Spectrum,
                Box::new(SpectrumPanel::new(shared.clone()).mode(Mode::Bars)),
                &state,
                w,
                h,
            );
            assert_eq!(f.lines().count(), h as usize);
            assert!(f.contains('█'), "bars: {w}x{h}:\n{f}");
            assert!(f.contains("2 channels"), "{f}");
            // Each cell names its channel.
            let names = [0usize, 1].map(|o| channel_name_of(&state, TAP_OUTPUT, o as u8));
            for n in names {
                assert!(f.contains(&n), "{n} in {w}x{h}:\n{f}");
            }
        }
    }

    fn channel_name_of(state: &DeviceState, tap: u8, ch: u8) -> String {
        channel_label(state, tap, ch)
    }

    #[test]
    fn both_draws_the_curves_over_the_bars() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let f = testing::frame(
            Tool::Spectrum,
            Box::new(SpectrumPanel::new(shared.clone()).mode(Mode::Both)),
            &state,
            120,
            40,
        );
        assert!(f.contains('█'), "{f}");
        assert!(
            f.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
            "{f}"
        );
    }

    #[test]
    fn a_refusal_and_the_small_transform_hint_use_the_consoles_words() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        {
            let mut app = shared.borrow_mut();
            let mut settings = app.spectrum.settings.clone();
            settings.transform_order = Some(8);
            app.spectrum.adopt_settings(&settings);
            let mut s = demo::status(1);
            s.first_band = 15;
            app.spectrum.engine.accept_status(s);
        }
        let f = testing::frame(
            Tool::Spectrum,
            Box::new(SpectrumPanel::new(shared.clone()).mode(Mode::Bars)),
            &state,
            120,
            40,
        );
        assert!(
            f.contains("shaded bands below 315 Hz need a larger transform size in Settings"),
            "{f}"
        );
        assert!(f.contains('░'), "the shaded bands: {f}");
    }

    #[test]
    fn choosing_channels_and_sides_drives_the_engine() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        let wanted = || shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!((wanted().tap, wanted().channel_mask), (TAP_OUTPUT, 1));

        // The chips: move right and show output 2 as well.
        p.body.focus = 2;
        p.handle(key(KeyCode::Right), &state);
        assert_eq!(p.chip, 1);
        p.handle(key(KeyCode::Char(' ')), &state);
        assert_eq!(wanted().channel_mask, 0b11);

        // Inputs: nothing chosen there yet, so nothing is watched.
        p.body.focus = 1;
        p.handle(key(KeyCode::Left), &state);
        let sel = selection(&state, &shared.borrow().spectrum);
        assert_eq!(sel, Selection::new(TAP_INPUT, vec![]));
        assert!(!shared.borrow().spectrum.engine.watching());
        // And back: the outputs come back as they were left.
        p.handle(key(KeyCode::Right), &state);
        assert_eq!(wanted().channel_mask, 0b11);

        // Clear.
        p.body.focus = 3;
        p.handle(key(KeyCode::Enter), &state);
        assert_eq!(
            selection(&state, &shared.borrow().spectrum).channels,
            Vec::<u8>::new()
        );
        assert!(!shared.borrow().spectrum.engine.watching());

        // The display switch.
        p.body.focus = 4;
        p.handle(key(KeyCode::Right), &state);
        assert_eq!(p.mode, Mode::Bars);
    }

    /// `A` is a global tool key (PLAN-beta4 decision 2): it opens the panel
    /// from the overview and closes it again from the panel itself.
    #[test]
    fn a_opens_and_closes_the_panel() {
        use crate::shell::{Placeholder, Shell, ShellEvent};
        let state = testing::state();
        let theme = crate::theme::Theme::console(
            crate::theme::ColorDepth::TrueColor,
            crate::theme::Glyphs::Braille,
        );
        let model = crate::shell::fixture::rp2350(&theme);
        let mut shell = Shell::new(model, theme, Box::new(Placeholder::new("Overview", "")));
        let a = crossterm::event::KeyEvent::new(
            KeyCode::Char('A'),
            crossterm::event::KeyModifiers::SHIFT,
        );
        assert_eq!(
            shell.handle(a, &state),
            vec![ShellEvent::OpenTool(Tool::Spectrum)]
        );
        let shared = shared_with(TAP_OUTPUT, &[0]);
        shell.open_tool(Tool::Spectrum, Box::new(SpectrumPanel::new(shared)));
        assert_eq!(shell.handle(a, &state), vec![ShellEvent::CloseTool]);
    }

    #[test]
    fn closing_the_panel_releases_the_analyser() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let p = SpectrumPanel::open(shared.clone(), &state);
        assert!(shared.borrow().spectrum.engine.watching());
        drop(p);
        assert!(!shared.borrow().spectrum.engine.watching());
    }

    /// The input chips are the rows the device is analysing: the active
    /// inputs, and the upmixer's derived rows while it runs on the stereo
    /// pair (rta.c:95-106).
    #[test]
    fn the_input_chips_are_the_live_rows() {
        let mut state = testing::state();
        let shared = crate::screens::shared();
        demo::load(&shared, TAP_INPUT, &[0]);
        let engine = &shared.borrow().spectrum.engine;
        let n = |state: &DeviceState| channels(state, engine, TAP_INPUT).len();
        state.meters.active_inputs = 0;
        assert_eq!(n(&state), 8, "unknown: every input");
        state.meters.active_inputs = 4;
        assert_eq!(n(&state), 4);
        state.meters.active_inputs = 2;
        assert_eq!(n(&state), 2, "the upmixer is off");
        state.caps.features.push(dspi_session::probe::Feature {
            name: "upmixer".into(),
            present: true,
            evidence: "test".into(),
        });
        let o = dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == "upmix")
            .map(|(_, o, _)| *o)
            .unwrap();
        state.bulk.patch(o, &[1]);
        assert!(state.upmix().enabled);
        let rows = 2 + super::super::matrix::derived_rows(&state);
        assert!(rows == 3 || rows == 5, "{rows}");
        assert_eq!(n(&state), rows, "the derived rows are live");
        assert_eq!(channel_label(&state, TAP_INPUT, 2), "C");
    }

    #[test]
    fn the_default_is_the_first_enabled_output() {
        let state = testing::state();
        let shared = crate::screens::shared();
        demo::load(&shared, TAP_OUTPUT, &[]);
        shared.borrow_mut().spectrum.selection = None;
        let sel = selection(&state, &shared.borrow().spectrum);
        let first = channels(&state, &shared.borrow().spectrum.engine, TAP_OUTPUT)[0];
        assert_eq!(sel, Selection::new(TAP_OUTPUT, vec![first]));
    }

    // -- WP INTE -- (owner 2026-11-24) the chooser is not an edit view: an
    // output nothing is routed to has no chip, unless it is analysed
    #[test]
    fn inte_an_unused_output_has_no_chip_unless_it_is_analysed() {
        let state = testing::state();
        let sub = 8u8;
        let ni = usize::from(state.caps.num_inputs);
        assert!(crate::screens::channel_unused(
            &state,
            ni + usize::from(sub)
        ));
        let offered = |shared: &Shared| {
            let app = shared.borrow();
            chooser_channels(
                &state,
                &app.spectrum.engine,
                &selection(&state, &app.spectrum),
            )
        };
        // Not analysed: no chip, though the analyser could show it
        let shared = shared_with(TAP_OUTPUT, &[0]);
        assert_eq!(offered(&shared), (0..8).collect::<Vec<u8>>());
        assert!(channels(&state, &shared.borrow().spectrum.engine, TAP_OUTPUT).contains(&sub));
        // Analysed: it keeps its chip, the last, so it can be switched off
        let shared = shared_with(TAP_OUTPUT, &[0, sub]);
        assert_eq!(offered(&shared), (0..9).collect::<Vec<u8>>());
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        p.body.focus = 2;
        for _ in 0..8 {
            p.handle(key(KeyCode::Right), &state);
        }
        assert_eq!(p.chip, 8);
        p.handle(key(KeyCode::Char(' ')), &state);
        assert_eq!(
            selection(&state, &shared.borrow().spectrum).channels,
            vec![0]
        );
        assert_eq!(offered(&shared), (0..8).collect::<Vec<u8>>());
    }

    #[test]
    fn a_band_curve_leaves_out_what_was_not_measured() {
        let levels: Vec<f64> = (0..34).map(|i| -20.0 - i as f64).collect();
        let curve = band_curve(&levels, &demo::CENTRES, &|i| i >= 14);
        let f = dsp::frequencies();
        let at = |hz: f64| {
            let i = f.iter().position(|x| *x >= hz).unwrap();
            curve[i]
        };
        assert!(at(100.0).is_nan(), "below the first measured band");
        assert!(at(1000.0).is_finite());
        // 1 kHz is band 20: -40 dBFS, give or take the grid.
        assert!((at(1000.0) - -40.0).abs() < 1.0, "{}", at(1000.0));
    }

    #[test]
    fn one_channel_blends_bands_into_bins_above_the_bass_bank() {
        let bands = vec![-20.0; dsp::POINTS];
        let bins = Bins {
            channel: 0,
            fft_order: 10,
            sample_rate_hz: 48_000,
            levels_db: vec![-60.0; 512],
            smoothed_db: vec![-60.0; 512],
        };
        let c = blend(&bands, &bins, &demo::CENTRES, 14);
        let f = dsp::frequencies();
        let at = |hz: f64| c[f.iter().position(|x| *x >= hz).unwrap()];
        assert_eq!(at(20.0), -20.0, "below bin 1 the bands alone");
        assert!((at(100.0) - -20.0).abs() < 1e-9, "the bass bank");
        assert!((at(2000.0) - -60.0).abs() < 1e-9, "the bins");
        let mid = at(300.0);
        assert!(mid < -20.0 && mid > -60.0, "crossfaded: {mid}");
    }

    #[test]
    fn a_glide_snaps_first_and_falls_slower_than_it_rises() {
        let mut g = Glide::default();
        let t0 = Instant::now();
        let rise = Duration::from_millis(40);
        let fall = Duration::from_millis(100);
        assert_eq!(g.step((1, 0, 0), vec![-60.0], t0, rise, fall), vec![-60.0]);
        let up = g.step(
            (1, 0, 0),
            vec![0.0],
            t0 + Duration::from_millis(40),
            rise,
            fall,
        )[0];
        let mut h = Glide::default();
        h.step((1, 0, 0), vec![0.0], t0, rise, fall);
        let down = h.step(
            (1, 0, 0),
            vec![-60.0],
            t0 + Duration::from_millis(40),
            rise,
            fall,
        )[0];
        assert!(up > -60.0 && up < 0.0);
        assert!((up - -60.0).abs() > (down - 0.0).abs(), "{up} {down}");
    }

    /// The `RTA_CONFIG` event for `config`, attributed to `origin`.
    fn rta_event(config: RtaConfig, origin: u16) -> Notification {
        let mut bytes = config.encode().to_vec();
        bytes.push(1);
        Notification {
            seq: 1,
            event: Event::Unknown {
                id: dspi_proto::generated::notify::NOTIFY_EVT_RTA_CONFIG as u8,
                bytes,
            },
            lost: false,
            origin: Some(origin),
            received: Instant::now(),
        }
    }

    /// Another console's configuration, as `Sharing` hears of it. -- WP ANLAST --
    /// A pushed choice of ours that is still pending comes back first: their
    /// push is a later one (the last choice made wins, and one of ours whose
    /// echo is still to come would drive).
    fn follow(shared: &Shared, mask: u16, origin: u16) {
        let mut app = shared.borrow_mut();
        let s = &mut app.spectrum;
        let caps = s.engine.caps().copied();
        if s.sharing.pending()
            && s.sharing.following().is_none()
            && let Some(ours) = s.engine.wanted()
        {
            s.sharing.note(
                &rta_event(ours, 1),
                TransportKind::Link,
                Some(1),
                caps.as_ref(),
            );
        }
        let note = rta_event(
            RtaConfig {
                tap: TAP_INPUT,
                channel_mask: mask,
                fft_order: 10,
                avg_ms: 300,
                peak_decay_db_s: 12,
                flags: 0,
            },
            origin,
        );
        s.sharing
            .note(&note, TransportKind::Link, Some(1), caps.as_ref());
    }

    #[test]
    fn wp_tm5_following_shows_the_devices_channels_and_says_so() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        follow(&shared, 0b0110, 9);
        p.sync(&state);
        let w = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!((w.tap, w.channel_mask), (TAP_INPUT, 0b0110), "polls theirs");
        let sel = selection(&state, &shared.borrow().spectrum);
        assert_eq!(sel, Selection::new(TAP_INPUT, vec![1, 2]));
        // The registered choice is still ours.
        assert_eq!(
            shared.borrow().spectrum.selection.as_ref().unwrap().tap,
            TAP_OUTPUT
        );
        let out = frame_of(&shared, &state, 100, 30);
        assert!(
            out.contains("Following another console (session 9)"),
            "{out}"
        );
        assert!(out.contains("Take control"), "{out}");
    }

    #[test]
    fn wp_tm5_the_take_control_row_and_a_chip_both_take_it_back() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        let following = || shared.borrow().spectrum.sharing.following().is_some();

        follow(&shared, 0b0110, 9);
        p.sync(&state);
        assert!(following());
        p.body.focus = 1;
        p.handle(key(KeyCode::Enter), &state);
        assert!(!following());
        let w = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!((w.tap, w.channel_mask), (TAP_OUTPUT, 1), "ours again");

        // A chip toggles what is on show, and makes that our choice.
        follow(&shared, 0b0110, 9);
        p.sync(&state);
        assert!(following());
        p.body.focus = 3;
        p.chip = 3;
        p.handle(key(KeyCode::Char(' ')), &state);
        assert!(!following());
        let w = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!((w.tap, w.channel_mask), (TAP_INPUT, 0b1110));

        // The side switch, to this console's own side.
        shared.borrow_mut().spectrum.selection = Some(Selection::new(TAP_OUTPUT, vec![0]));
        p.sync(&state);
        follow(&shared, 0b0110, 9);
        p.sync(&state);
        p.body.focus = 2;
        p.handle(key(KeyCode::Right), &state);
        assert!(!following());
        let w = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!((w.tap, w.channel_mask), (TAP_OUTPUT, 1), "kept as it was");
    }

    /// The device's writes during polls: every `control_out` opcode. Reads
    /// stall, which a poll survives.
    struct Pushes(Vec<u8>, dspi_transport::DeviceDescriptor);

    impl Transport for Pushes {
        fn control_in(&mut self, opcode: u8, _: u16, _: u16) -> dspi_transport::Result<Vec<u8>> {
            Err(dspi_transport::TransportError::Stalled { opcode })
        }
        fn control_out(&mut self, opcode: u8, _: u16, _: &[u8]) -> dspi_transport::Result<()> {
            self.0.push(opcode);
            Ok(())
        }
        fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
            &self.1
        }
    }

    /// WP CG30 (was TM5's "a display hidden here stays hidden"): with no
    /// channel chosen here, the panel draws what another console drives, as
    /// the Console's `drawn(own:followed:)` does (SpectrumAnalyserView.swift:682),
    /// and asks the device for nothing of its own.
    #[test]
    fn wp_cg30_an_empty_selection_draws_what_is_followed_and_pushes_nothing() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[]);
        let mut p = SpectrumPanel::new(shared.clone());
        let following = || shared.borrow().spectrum.sharing.following().is_some();
        let watching = || shared.borrow().spectrum.engine.watching();
        // Never chosen anything: another console's choice is not judged
        // against anything (`lastOwn`, SpectrumAnalyser.swift:683).
        p.sync(&state);
        follow(&shared, 0b0110, 9);
        p.sync(&state);
        assert!(!following() && !watching());

        // Chosen, then cleared while driving: released.
        shared.borrow_mut().spectrum.selection = Some(Selection::new(TAP_OUTPUT, vec![0]));
        p.sync(&state);
        assert!(watching());
        shared.borrow_mut().spectrum.selection = Some(Selection::new(TAP_OUTPUT, vec![]));
        p.sync(&state);
        assert!(!watching());

        // Another console drives: its channels are polled and drawn.
        follow(&shared, 0b0110, 9);
        p.sync(&state);
        assert!(following());
        assert_eq!(
            selection(&state, &shared.borrow().spectrum),
            Selection::new(TAP_INPUT, vec![1, 2])
        );
        let w = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!((w.tap, w.channel_mask), (TAP_INPUT, 0b0110), "polls theirs");
        let out = frame_of(&shared, &state, 100, 30);
        assert!(
            out.contains("Following another console (session 9)"),
            "{out}"
        );
        // Nothing of ours goes to the device, and our choice stays empty.
        let mut t = Pushes(
            Vec::new(),
            dspi_transport::DeviceDescriptor {
                serial: "CG30".into(),
                bus_id: String::new(),
                address: 0,
                ..Default::default()
            },
        );
        for _ in 0..3 {
            p.sync(&state);
            shared.borrow_mut().spectrum.tick(&mut t, Instant::now());
        }
        assert_eq!(t.0, Vec::<u8>::new(), "no push over the other console");
        assert!(following());
        assert_eq!(
            shared.borrow().spectrum.selection,
            Some(Selection::new(TAP_OUTPUT, vec![]))
        );
    }

    #[test]
    fn wp_tm5_the_overlay_watches_only_while_on_screen_and_outlives_the_panel() {
        let state = testing::state();
        let theme = Theme::console(crate::theme::ColorDepth::TrueColor, Glyphs::Braille);
        let shared = shared_with(TAP_OUTPUT, &[0]);
        let picture = || overlay_picture(&shared, &state, &theme, Instant::now());
        let sync = |shown: bool| {
            let mut app = shared.borrow_mut();
            app.spectrum.set_overlay_shown(shown);
            app.spectrum.sync(&state);
        };
        let watching = || shared.borrow().spectrum.engine.watching();

        sync(true);
        assert!(!watching() && picture().is_none(), "off until asked for");

        assert!(shared.borrow_mut().spectrum.toggle_overlay(None));
        sync(false);
        assert!(!watching() && picture().is_none(), "the graph is covered");

        sync(true);
        assert!(watching());
        let o = picture().unwrap();
        assert_eq!((o.floor_db, o.ceiling_db), (-90.0, 6.0));
        assert!(
            o.curves
                .iter()
                .any(|c| c.descriptor == channel_name_of(&state, TAP_OUTPUT, 0))
        );
        assert_eq!(o.note, None);

        // One subscription between them: closing the panel keeps it.
        let mut p = SpectrumPanel::new(shared.clone());
        p.sync(&state);
        drop(p);
        assert!(watching());

        follow(&shared, 0b0110, 9);
        sync(true);
        let note = picture().unwrap().note.unwrap();
        assert!(
            note.contains("(session 9)") && note.contains("t takes control"),
            "{note}"
        );

        assert!(!shared.borrow_mut().spectrum.toggle_overlay(None));
        sync(true);
        assert!(!watching() && picture().is_none());
    }

    #[test]
    fn wp_tm5_the_overlay_starts_from_the_graphed_channel_when_none_was_chosen() {
        let mut s = SpectrumState::default();
        s.toggle_overlay(Some((TAP_INPUT, 2)));
        assert_eq!(s.selection, Some(Selection::new(TAP_INPUT, vec![2])));
        s.toggle_overlay(Some((TAP_INPUT, 3)));
        s.toggle_overlay(Some((TAP_INPUT, 3)));
        assert_eq!(
            s.selection,
            Some(Selection::new(TAP_INPUT, vec![2])),
            "a choice stays"
        );
    }

    // -- WP TM6 --
    #[test]
    fn wp_tm6_the_strip_watches_only_while_on_screen_and_shares_the_subscription() {
        let state = testing::state();
        let theme = Theme::console(crate::theme::ColorDepth::TrueColor, Glyphs::Braille);
        let shared = shared_with(TAP_OUTPUT, &[0, 2]);
        let picture = || bars_picture(&shared, &state, &theme, Instant::now());
        let sync = |shown: bool| {
            let mut app = shared.borrow_mut();
            app.spectrum.set_bars_shown(shown);
            app.spectrum.sync(&state);
        };
        let watching = || shared.borrow().spectrum.engine.watching();

        sync(true);
        assert!(!watching() && picture().is_none(), "off until asked for");

        assert!(shared.borrow_mut().spectrum.toggle_bars(None));
        sync(false);
        assert!(!watching() && picture().is_none(), "no room on screen");

        sync(true);
        assert!(watching());
        let p = picture().unwrap();
        assert_eq!((p.floor_db, p.ceiling_db), (-90.0, 6.0));
        assert_eq!(p.channels.len(), 2);
        assert_eq!(p.channels[1].name, channel_name_of(&state, TAP_OUTPUT, 2));
        assert_eq!(p.channels[0].levels.len(), 34, "the device's bands");
        assert!(p.channels[0].peaks.is_some(), "Peak Hold is on by default");
        assert_eq!(p.populated.len(), 34);

        // The channels are the device's, and one subscription serves all.
        let want = shared.borrow().spectrum.engine.wanted().unwrap();
        assert_eq!(want.channel_mask, 0b101);
        assert!(shared.borrow_mut().spectrum.toggle_overlay(None));
        shared.borrow_mut().spectrum.set_overlay_shown(true);
        let mut panel = SpectrumPanel::new(shared.clone());
        panel.sync(&state);
        drop(panel);
        assert!(!shared.borrow_mut().spectrum.toggle_overlay(None));
        sync(true);
        assert!(watching(), "the strip keeps it");

        // Peak Hold off is honoured.
        {
            let mut app = shared.borrow_mut();
            let mut settings = app.spectrum.settings.clone();
            settings.peak_hold = false;
            app.spectrum.adopt_settings(&settings);
        }
        assert!(picture().unwrap().channels[0].peaks.is_none());

        assert!(!shared.borrow_mut().spectrum.toggle_bars(None));
        sync(true);
        assert!(!watching() && picture().is_none());
    }

    #[test]
    fn wp_tm6_following_draws_the_devices_channels_also_with_none_chosen_here() {
        let state = testing::state();
        let theme = Theme::console(crate::theme::ColorDepth::TrueColor, Glyphs::Braille);
        let shared = shared_with(TAP_OUTPUT, &[0]);
        {
            let mut app = shared.borrow_mut();
            app.spectrum.toggle_bars(None);
            app.spectrum.set_bars_shown(true);
            app.spectrum.sync(&state);
        }
        follow(&shared, 0b0110, 9);
        let p = bars_picture(&shared, &state, &theme, Instant::now()).unwrap();
        let names: Vec<_> = p.channels.iter().map(|c| c.name.clone()).collect();
        assert_eq!(
            names,
            [1, 2].map(|c| channel_name_of(&state, TAP_INPUT, c)),
            "the device's channels, not this console's"
        );

        // -- WP CG30 -- none chosen here: still the device's channels (was: hidden)
        shared.borrow_mut().spectrum.selection = Some(Selection::new(TAP_OUTPUT, vec![]));
        let p = bars_picture(&shared, &state, &theme, Instant::now()).unwrap();
        assert_eq!(
            p.channels.len(),
            2,
            "the device's channels with none of ours"
        );
    }

    #[test]
    fn wp_tm6_the_bars_start_from_the_page_when_none_was_chosen() {
        let mut s = SpectrumState::default();
        s.toggle_bars(Some((TAP_INPUT, 2)));
        assert_eq!(s.selection, Some(Selection::new(TAP_INPUT, vec![2])));
        s.toggle_bars(Some((TAP_INPUT, 3)));
        s.toggle_bars(Some((TAP_INPUT, 3)));
        assert_eq!(
            s.selection,
            Some(Selection::new(TAP_INPUT, vec![2])),
            "a choice stays"
        );
    }

    #[test]
    fn every_advertised_key_is_handled_somewhere() {
        let state = testing::state();
        let shared = shared_with(TAP_OUTPUT, &[0]);
        for help in KEYS {
            for k in crate::screens::tests::keys_for(help.key) {
                let mut hit = false;
                for focus in 1..6 {
                    let mut p = SpectrumPanel::new(shared.clone());
                    p.body.focus = focus;
                    let before = (p.body.focus, p.chip, p.mode);
                    let ev = p.handle(k, &state);
                    if ev != ScreenEvent::Unhandled || (p.body.focus, p.chip, p.mode) != before {
                        hit = true;
                        break;
                    }
                }
                assert!(hit, "{:?} from {:?} is not bound", k.code, help.key);
            }
        }
    }
}
