//! The Crossfeed panel: `CrossfeedView.swift` as a tool panel.
//!
//! The Console's four voicings are a radio group over one device parameter,
//! and the two sliders beneath them belong to the Custom voicing: editing
//! either one while a named voicing is selected switches to Custom first, the
//! same way clicking the Console's slider does, so what is on screen never
//! claims to be a preset it no longer is.
//!
//! Firmware with the extension channel also gets a PAIR MAP row (WP TM1,
//! `CrossfeedPairMapView.swift`): which two outputs each crossfeed pair
//! processes. It is a different thing from the OUTPUT PAIRS mask, which says
//! which of those pairs crossfeed runs on, so it has its own section and its
//! writes never touch the mask. Stock firmware never sees the row.

use crossterm::event::KeyEvent;
use dspi_proto::generated::SECTIONS;
use dspi_session::ext::{self, PairMap};
use dspi_session::ext_state::ExtState;
use dspi_session::{DeviceState, Session};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::number;
use super::panel::{self, Body, ChipSpec, Header, PanelGraph, Param, Row};
use crate::curves;
use crate::shell::{Screen, ScreenEvent, SessionReply, SessionRequest};
use crate::theme::Theme;
use crate::widgets::{Action, ChipState, KeyHelp, PopupList, StatusTone};

/// The Console's voicings, `crossfeedPresets` in `CrossfeedView.swift:52-57`.
/// The em-dash of the Console's own strings is a hyphen here, as the house
/// style requires.
const PRESETS: [(&str, &str, &str); 4] = [
    (
        "Default",
        "700 Hz / 4.5 dB - Balanced, most popular",
        "default",
    ),
    (
        "Chu Moy",
        "700 Hz / 6.0 dB - Stronger spatial effect",
        "chumoy",
    ),
    (
        "Jan Meier",
        "650 Hz / 9.5 dB - Natural speaker-like",
        "meier",
    ),
    ("Custom", "User-defined parameters", "custom"),
];

/// The Console's output-pair presets menu.
const PAIR_PRESETS: [&str; 3] = ["All pairs", "Pair 1 only (Headphones)", "None"];

const CUSTOM: u8 = 3;

/// `pair_map`'s place in `WireCrossfeedParams` (bulk_params.h:80).
const PAIR_MAP_OFFSET: usize = 12;

/// The Console's "Outputs" menu, in [`MapChoice`]'s order.
const MAP_CHOICES: [&str; 3] = [
    "Default (1/2, 3/4, ...)",
    "One DAC per ear (K340)",
    "Custom...",
];

const NOT_NOW: &str = "Not Now";
const ENABLE_BOTH: &str = "Enable Pairs 1 and 2";
const CANCEL: &str = "Cancel";
const APPLY: &str = "Apply";
/// Short enough for the editor's one status line at any panel width.
const DRAFT_WARNING: &str = "Each output can be used once only.";
const INVALID_DRAFT: &str =
    "Each pair needs two different outputs, and no output can be in two pairs.";

/// Which pick a pair-map write came from, as its session tag.
const TAG_MAP_DEFAULT: u32 = 1;
const TAG_MAP_K340: u32 = 2;
const TAG_MAP_CUSTOM: u32 = 3;

/// The pair map's menu row, in the focus order: the header is 0.
const MAP_FOCUS: usize = 1;

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("↑ ↓", "Move"),
    KeyHelp::new("← →", "Adjust"),
    KeyHelp::new("Enter", "Edit or choose"),
    KeyHelp::new("Space", "Toggle"),
    KeyHelp::new("p", "Presets"),
    KeyHelp::new("Backspace", "Reset"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Pairs,
    /// The pair map's menu.
    Map,
    /// One output of the custom editor's pair.
    Output {
        pair: usize,
        right: bool,
    },
}

/// What a map is, for the menu: `CrossfeedPairChoice`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapChoice {
    Default = 0,
    K340 = 1,
    Custom = 2,
}

impl MapChoice {
    fn of(map: &PairMap) -> Self {
        let map = map.normalized();
        if map == PairMap([0; PairMap::SIZE]) {
            Self::Default
        } else if map == PairMap::K340.normalized() {
            Self::K340
        } else {
            Self::Custom
        }
    }
}

pub struct CrossfeedPanel {
    body: Body,
    chip: usize,
    pending: Option<Pending>,
    /// Whether this device has a usable pair map: `None` until the first poll
    /// after a connect has asked. The map itself is never kept here; it is in
    /// the bulk image, as everything else this panel draws.
    has_map: Option<bool>,
    /// The custom editor's pairs, `(left, right)`, while it is open.
    draft: Option<Vec<(u8, u8)>>,
    /// The K340 preset was just picked, so offer the mask that goes with it.
    offer_mask: bool,
    /// The column of the pair map's button rows.
    button: usize,
}

impl Default for CrossfeedPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl CrossfeedPanel {
    pub fn new() -> Self {
        Self {
            body: Body::default(),
            chip: 0,
            pending: None,
            has_map: None,
            draft: None,
            offer_mask: false,
            button: 0,
        }
    }

    /// How many pairs the map covers. A panel with none shows no row.
    fn map_pairs(state: &DeviceState) -> usize {
        Self::pairs(state).min(PairMap::PAIRS)
    }

    fn shows_map(&self, state: &DeviceState) -> bool {
        self.has_map == Some(true) && Self::map_pairs(state) > 0
    }

    /// `Out n` and its channel name, or just `Out n` before names arrive.
    fn output_label(state: &DeviceState, output: u8) -> String {
        let number = u16::from(output) + 1;
        let name = state.channel_name(usize::from(state.caps.num_inputs) + usize::from(output));
        if name.is_empty() {
            format!("{number} - Out {number}")
        } else {
            format!("{number} - {name}")
        }
    }

    /// Whether the mask offer is on screen: `CrossfeedPairMapControls.body`.
    fn shows_offer(&self, state: &DeviceState) -> bool {
        self.offer_mask
            && MapChoice::of(&pair_map(state)) == MapChoice::K340
            && state.crossfeed().output_pair_mask & PairMap::K340_MASK != PairMap::K340_MASK
    }

    /// The device has gone: what was learned about it goes too, because the
    /// next one may be stock, or a firmware update of this one.
    fn forget_device(&mut self) {
        if self.has_map == Some(true) {
            self.body.focus = self.body.focus.saturating_sub(1);
        }
        self.has_map = None;
        self.draft = None;
        self.offer_mask = false;
    }

    /// The pair map's section, under its own title so that it does not read
    /// as the OUTPUT PAIRS mask.
    fn push_map_rows(&self, rows: &mut Vec<Row>, state: &DeviceState) {
        let choice = MapChoice::of(&pair_map(state));
        let shown = match choice {
            MapChoice::Custom => "Custom",
            c => MAP_CHOICES[c as usize],
        };
        rows.push(Row::Blank);
        rows.push(Row::Menu {
            title: "Pair map".into(),
            action: format!("{shown} ▾"),
        });
        rows.push(Row::Caption(
            "Which two outputs each pair crossfeeds. An output can belong to one pair only.".into(),
        ));
        if self.shows_offer(state) {
            rows.push(Row::Caption(
                "Crossfeed the woofers and the tweeters?".into(),
            ));
            rows.push(self.button_row("", &[NOT_NOW, ENABLE_BOTH]));
        }
        if let Some(draft) = &self.draft {
            for (p, (l, r)) in draft.iter().enumerate() {
                rows.push(Row::Buttons {
                    label: format!("Pair {}", p + 1),
                    buttons: vec![
                        format!("L Out {}", u16::from(*l) + 1),
                        format!("R Out {}", u16::from(*r) + 1),
                    ],
                    cursor: self.button.min(1),
                });
            }
            if validated(draft, Self::map_pairs(state)).is_none() {
                rows.push(Row::Status {
                    text: DRAFT_WARNING.into(),
                    tone: StatusTone::Warning,
                });
            }
            rows.push(self.button_row("", &[CANCEL, APPLY]));
        }
    }

    fn button_row(&self, label: &str, buttons: &[&str]) -> Row {
        Row::Buttons {
            label: label.into(),
            buttons: buttons.iter().map(|b| (*b).to_string()).collect(),
            cursor: self.button.min(buttons.len().saturating_sub(1)),
        }
    }

    /// A chip's words: its number, and with a map, the outputs it processes
    /// (`1: 3/1`; the Console's `1: Out 3 / Out 1`, short enough for a row).
    fn chip_label(&self, state: &DeviceState, p: usize) -> String {
        if self.shows_map(state) {
            let (l, r) = pair_map(state).pair(p);
            format!("{}: {}/{}", p + 1, u16::from(l) + 1, u16::from(r) + 1)
        } else {
            (p + 1).to_string()
        }
    }

    /// One press on one of the pair map's button rows.
    fn press(&mut self, label: &str, button: &str, b: usize, state: &DeviceState) -> ScreenEvent {
        match button {
            NOT_NOW => {
                self.offer_mask = false;
                self.body.focus = MAP_FOCUS;
                ScreenEvent::Handled
            }
            ENABLE_BOTH => {
                self.offer_mask = false;
                self.body.focus = MAP_FOCUS;
                let mask = state.crossfeed().output_pair_mask | PairMap::K340_MASK;
                ScreenEvent::Command(format!("cf.outputs 0x{mask:X}"))
            }
            CANCEL => {
                self.draft = None;
                self.body.focus = MAP_FOCUS;
                ScreenEvent::Handled
            }
            APPLY => {
                let wanted = self
                    .draft
                    .as_deref()
                    .and_then(|d| validated(d, Self::map_pairs(state)));
                match wanted {
                    Some(map) => write_map(TAG_MAP_CUSTOM, map),
                    // Nothing leaves: the firmware would ACK this and ignore it.
                    None => ScreenEvent::Status(INVALID_DRAFT.into()),
                }
            }
            _ => self.pick_output(label, b, state),
        }
    }

    /// Enter on a `Pair n` row: choose that side's output from a list.
    fn pick_output(&mut self, label: &str, b: usize, state: &DeviceState) -> ScreenEvent {
        let pair = label
            .strip_prefix("Pair ")
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| n.checked_sub(1));
        let (Some(pair), Some(draft)) = (pair, self.draft.as_ref()) else {
            return ScreenEvent::Handled;
        };
        let Some((l, r)) = draft.get(pair) else {
            return ScreenEvent::Handled;
        };
        let right = b == 1;
        let selected = usize::from(if right { *r } else { *l });
        self.pending = Some(Pending::Output { pair, right });
        // Two stereo outputs per pair.
        let items = (0..Self::map_pairs(state) * 2)
            .map(|o| Self::output_label(state, o as u8))
            .collect();
        let side = if right { "right" } else { "left" };
        ScreenEvent::Popup(PopupList::new(
            format!("Pair {} {side} output", pair + 1),
            items,
            selected,
        ))
    }

    /// The menu's answer: a preset writes now, Custom opens the editor on what
    /// the device reports.
    fn pick_map(&mut self, choice: usize, state: &DeviceState) -> ScreenEvent {
        match choice {
            0 => {
                self.draft = None;
                self.offer_mask = false;
                write_map(TAG_MAP_DEFAULT, PairMap([0; PairMap::SIZE]))
            }
            1 => {
                self.draft = None;
                let pairs: Vec<(u8, u8)> = (0..Self::map_pairs(state))
                    .map(|p| PairMap::K340.pair(p))
                    .collect();
                match validated(&pairs, Self::map_pairs(state)) {
                    Some(map) => write_map(TAG_MAP_K340, map),
                    None => ScreenEvent::Status(
                        "This device has too few outputs for one DAC per ear.".into(),
                    ),
                }
            }
            _ => {
                let map = pair_map(state);
                self.draft = Some((0..Self::map_pairs(state)).map(|p| map.pair(p)).collect());
                self.offer_mask = false;
                self.button = 0;
                self.body.focus = MAP_FOCUS + 1;
                ScreenEvent::Handled
            }
        }
    }

    /// Stereo output pairs: the S/PDIF instances, which is every output slot
    /// except the mono PDM sub. Two on RP2040, four on RP2350.
    fn pairs(state: &DeviceState) -> usize {
        (state.caps.num_outputs as usize).saturating_sub(1) / 2
    }

    /// The per-pair mask shipped in wire format V20 (`REQ_SET_CROSSFEED_
    /// OUTPUTS`, config.h:499-517); older firmware crossfeeds a fixed set and
    /// the Console hides the selector.
    fn shows_pairs(state: &DeviceState) -> bool {
        state.caps.wire_format >= 20 && Self::pairs(state) > 0
    }

    fn all_pairs(state: &DeviceState) -> u8 {
        let n = Self::pairs(state);
        if n >= 8 { 0xFF } else { (1u16 << n) as u8 - 1 }
    }

    fn rows(&self, state: &DeviceState, theme: &Theme) -> Vec<Row> {
        let cf = state.crossfeed();
        let mut rows = vec![Row::Section {
            title: "Frequency response".into(),
            action: None,
        }];
        rows.push(Row::Graph(if cf.enabled {
            let (cross, direct) =
                curves::crossfeed_curves(cf.custom_fc as f64, cf.custom_feed_db as f64);
            PanelGraph::Curves {
                series: vec![
                    ("Crossfeed".into(), theme.warning, cross),
                    ("Direct".into(), theme.accent, direct),
                ],
                legend: vec![
                    ("Direct".into(), theme.accent),
                    ("Crossfeed".into(), theme.warning),
                ],
            }
        } else {
            PanelGraph::Disabled
        }));

        if self.shows_map(state) {
            self.push_map_rows(&mut rows, state);
        }

        if Self::shows_pairs(state) {
            rows.push(Row::Blank);
            rows.push(Row::Section {
                title: "Output pairs".into(),
                action: Some("Presets ▾".into()),
            });
            let chips = (0..Self::pairs(state))
                .map(|p| ChipSpec {
                    label: self.chip_label(state, p),
                    state: if cf.output_pair_mask & (1 << p) != 0 {
                        ChipState::On
                    } else {
                        ChipState::Off
                    },
                    color: theme.role_color(crate::theme::ChannelRole::Output(2 * p as u8)),
                    enabled: true,
                    dimmed: false,
                })
                .collect();
            // `DESIGN.md` 7.8's template is header, chips, caption.
            rows.push(Row::Chips {
                chips,
                cursor: self.chip,
                polarity: false,
            });
            rows.push(Row::Caption(
                "Crossfeed only the stereo output pairs feeding headphones. Speaker pairs stay \
                 bit-accurate. The mono sub is never crossfed."
                    .into(),
            ));
        }

        rows.push(Row::Blank);
        rows.push(Row::Section {
            title: "Preset".into(),
            action: None,
        });
        for (i, (name, detail, _)) in PRESETS.iter().enumerate() {
            rows.push(Row::Radio {
                label: (*name).into(),
                detail: (*detail).into(),
                on: cf.preset as usize == i,
            });
        }

        rows.push(Row::Blank);
        rows.push(Row::Section {
            title: "Parameters".into(),
            action: None,
        });
        // `CrossfeedView.swift:301`: the whole section is at 0.5 opacity unless
        // the voicing is Custom, and it stays live, because an edit is what
        // switches the voicing.
        let custom = cf.preset == CUSTOM;
        rows.push(Row::Param(
            Param::new("Cutoff Frequency", cf.custom_fc as f64, 500.0, 2000.0, "Hz")
                .step(10.0)
                .decimals(0)
                .dimmed(!custom)
                .caption(
                    "Simulates head shadow lowpass cutoff. Lower = more bass crossfeed. \
                     Typical: 650-700 Hz.",
                ),
        ));
        rows.push(Row::Param(
            Param::new("Feed Level", cf.custom_feed_db as f64, 0.0, 15.0, "dB")
                .step(0.5)
                .decimals(1)
                .dimmed(!custom)
                .caption(
                    "Crossfeed attenuation below direct signal. Higher = more crossfeed. \
                     Typical: 4.5-9.5 dB.",
                ),
        ));
        rows.push(Row::Blank);
        rows.push(Row::Toggle {
            label: "Interaural Time Delay".into(),
            on: cf.itd_enabled,
            caption: Some("Simulates ~220 us path difference via all-pass filter".into()),
            enabled: true,
        });
        rows
    }

    /// What one row's action means, as a command.
    fn act(&mut self, row: &Row, action: Action, state: &DeviceState) -> ScreenEvent {
        match (row, action) {
            (Row::Chips { .. }, Action::Selected(i)) => {
                self.chip = i;
                ScreenEvent::Handled
            }
            (Row::Chips { .. }, Action::Chip(i, next)) => {
                let bit = 1u8 << i;
                let mask = state.crossfeed().output_pair_mask;
                let mask = if next == ChipState::On {
                    mask | bit
                } else {
                    mask & !bit
                };
                ScreenEvent::Command(format!("cf.outputs 0x{mask:X}"))
            }
            (Row::Chips { .. }, Action::Open) => {
                self.pending = Some(Pending::Pairs);
                ScreenEvent::Popup(PopupList::new(
                    "Output pairs",
                    PAIR_PRESETS.iter().map(|s| s.to_string()).collect(),
                    0,
                ))
            }
            (Row::Menu { .. }, Action::Open) => {
                self.pending = Some(Pending::Map);
                ScreenEvent::Popup(PopupList::new(
                    "Pair map",
                    MAP_CHOICES.iter().map(|s| s.to_string()).collect(),
                    MapChoice::of(&pair_map(state)) as usize,
                ))
            }
            (Row::Buttons { .. }, Action::Selected(b)) => {
                self.button = b;
                ScreenEvent::Handled
            }
            (Row::Buttons { label, buttons, .. }, Action::Button(b)) => match buttons.get(b) {
                Some(button) => self.press(label, button, b, state),
                None => ScreenEvent::Handled,
            },
            (Row::Radio { label, .. }, Action::Toggled(_)) => {
                let token = PRESETS
                    .iter()
                    .find(|(n, _, _)| n == label)
                    .map(|(_, _, t)| *t)
                    .unwrap_or("default");
                ScreenEvent::Command(format!("cf.preset {token}"))
            }
            (Row::Toggle { .. }, Action::Toggled(next)) => {
                ScreenEvent::Command(format!("cf.itd {}", if next { "on" } else { "off" }))
            }
            (Row::Param(p), Action::Changed(v)) => self.param_command(state, &p.label, v),
            (Row::Param(p), Action::Reset) => {
                let v = if p.label.starts_with("Cutoff") {
                    700.0
                } else {
                    4.5
                };
                self.param_command(state, &p.label, v)
            }
            _ => ScreenEvent::Handled,
        }
    }

    /// A parameter edit belongs to the Custom voicing, so it carries the
    /// voicing change with it.
    fn param_command(&self, state: &DeviceState, label: &str, value: f64) -> ScreenEvent {
        let path = if label.starts_with("Cutoff") {
            "cf.freq"
        } else {
            "cf.feed"
        };
        let mut lines = Vec::new();
        if state.crossfeed().preset != CUSTOM {
            lines.push("cf.preset custom".to_string());
        }
        lines.push(format!("{path} {}", number(value as f32)));
        ScreenEvent::Command(lines.join("\n"))
    }
}

impl Screen for CrossfeedPanel {
    fn quick(&self, line: &str, _state: &DeviceState) -> Option<crate::shell::Quick> {
        Some(super::quick::toggle(line, "cf.on", "Crossfeed"))
    }

    fn title(&self) -> String {
        "Crossfeed".into()
    }

    fn draw(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        state: &DeviceState,
        focused: bool,
    ) {
        if !state.connected {
            self.forget_device();
        }
        let rows = self.rows(state, theme);
        self.body.clamp(&rows);
        let header =
            Header::new("BS2B Bauer Stereophonic-to-Binaural").toggle(state.crossfeed().enabled);
        self.body.draw(area, buf, theme, &header, &rows, focused);
    }

    fn handle(&mut self, key: KeyEvent, state: &DeviceState) -> ScreenEvent {
        let rows = self.rows(state, panel::key_theme());
        self.body.clamp(&rows);
        match panel::dispatch(&mut self.body, &rows, key) {
            panel::Step::Done(ev) => ev,
            panel::Step::Header => ScreenEvent::Command(format!(
                "cf.on {}",
                if state.crossfeed().enabled {
                    "off"
                } else {
                    "on"
                }
            )),
            panel::Step::Commit(i, v) => match &rows[i] {
                Row::Param(p) => self.param_command(state, &p.label, v.clamp(p.min, p.max)),
                _ => ScreenEvent::Handled,
            },
            panel::Step::Row(i, action) => self.act(&rows[i], action, state),
        }
    }

    fn keys(&self) -> &'static [KeyHelp] {
        KEYS
    }

    fn popup_result(&mut self, choice: Option<usize>, state: &DeviceState) -> ScreenEvent {
        let pending = self.pending.take();
        match (pending, choice) {
            (Some(Pending::Pairs), Some(i)) => {
                let mask = match i {
                    0 => Self::all_pairs(state),
                    1 => 0x01,
                    _ => 0x00,
                };
                ScreenEvent::Command(format!("cf.outputs 0x{mask:X}"))
            }
            (Some(Pending::Map), Some(i)) => self.pick_map(i, state),
            (Some(Pending::Output { pair, right }), Some(i)) => {
                if let (Some((l, r)), Ok(output)) = (
                    self.draft.as_mut().and_then(|d| d.get_mut(pair)),
                    u8::try_from(i),
                ) {
                    *(if right { r } else { l }) = output;
                }
                ScreenEvent::Handled
            }
            _ => ScreenEvent::Handled,
        }
    }

    /// Ask once per connect whether this firmware has a pair map. A stall, or
    /// a DSPi Link hub refusing this client's role, hides the row; a transport
    /// failure leaves the question open for the next poll.
    fn poll(&mut self, session: &mut Session, state: &DeviceState) {
        if self.has_map.is_some() || !state.connected {
            return;
        }
        if let Ok(found) = ExtState::probe(session.transport_mut()) {
            let has = found.pair_map.is_some();
            self.has_map = Some(has);
            // The row arrives above whatever has focus: stay on that row.
            if has && self.body.focus >= MAP_FOCUS {
                self.body.focus = self.body.focus.saturating_add(1);
            }
        }
    }

    fn session_result(
        &mut self,
        tag: u32,
        reply: SessionReply,
        state: &DeviceState,
    ) -> ScreenEvent {
        match reply {
            SessionReply::Ok(line) => {
                match tag {
                    TAG_MAP_K340 => {
                        self.offer_mask = true;
                        if self.shows_offer(state) {
                            self.button = 0;
                            self.body.focus = MAP_FOCUS + 1;
                        }
                    }
                    TAG_MAP_CUSTOM => {
                        self.draft = None;
                        self.body.focus = MAP_FOCUS;
                    }
                    _ => {}
                }
                ScreenEvent::Status(line)
            }
            // Never retried: the editor stays open with what was asked for, and
            // the row keeps showing the map the device reports.
            SessionReply::Err(why) => ScreenEvent::Status(why),
            SessionReply::Bytes(_) => ScreenEvent::Handled,
        }
    }
}

/// The map in the bulk image, as stored: all zeros (the default pairing) on a
/// firmware that leaves the bytes reserved. This is the map the device reports;
/// a preset load or another client changes it here too.
fn pair_map(state: &DeviceState) -> PairMap {
    SECTIONS
        .iter()
        .find(|(name, _, _)| *name == "crossfeed")
        .map(|(_, offset, _)| offset.saturating_add(PAIR_MAP_OFFSET))
        .and_then(|at| {
            state
                .bulk
                .as_bytes()
                .get(at..at.checked_add(PairMap::SIZE)?)
        })
        .and_then(|b| <[u8; PairMap::SIZE]>::try_from(b).ok())
        .map(PairMap)
        .unwrap_or(PairMap([0; PairMap::SIZE]))
}

/// The map for explicit pairs (one per platform pair, pair 0 first) when the
/// firmware would accept it: `CrossfeedPairMap.validated`, which is
/// [`PairMap::is_valid`] plus a check of the pairs themselves, since a pair on
/// output 1 twice would encode as 0x00, the default pair.
fn validated(pairs: &[(u8, u8)], pair_count: usize) -> Option<PairMap> {
    if pair_count > PairMap::PAIRS || pairs.len() != pair_count {
        return None;
    }
    let mut map = [0u8; PairMap::SIZE];
    for (byte, (l, r)) in map.iter_mut().zip(pairs) {
        // A nibble each, or the encoding below would lose bits.
        if l == r || *l > 0x0F || *r > 0x0F {
            return None;
        }
        *byte = (*r << 4) | *l;
    }
    let map = PairMap(map);
    map.is_valid(pair_count).then(|| map.normalized())
}

/// Send a whole map and confirm it: the firmware ACKs an invalid map and
/// ignores it, so `write_pair_map`'s read-back is the only word on it. The
/// runner re-reads the bulk image afterwards, which is what the row draws.
fn write_map(tag: u32, map: PairMap) -> ScreenEvent {
    ScreenEvent::Session(SessionRequest::new(
        tag,
        move |session| match ext::write_pair_map(session.transport_mut(), map) {
            Ok(_) => SessionReply::Ok("Pair map updated".into()),
            Err(e) => SessionReply::Err(format!("Pair map not changed: {e}")),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::panel::testing;
    use crate::shell::Tool;
    use crate::widgets::testing::key;
    use crossterm::event::KeyCode;

    fn panel() -> (CrossfeedPanel, DeviceState) {
        let mut state = testing::state();
        // The fixture leaves crossfeed off; the panel is more interesting on.
        let (_, cf, _) = dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == "crossfeed")
            .copied()
            .unwrap();
        state.bulk.patch(cf, &[1, 0, 1, 0x03]);
        state.bulk.patch(cf + 4, &700.0f32.to_le_bytes());
        state.bulk.patch(cf + 8, &4.5f32.to_le_bytes());
        (CrossfeedPanel::new(), state)
    }

    #[test]
    fn the_panel_carries_every_section_the_console_has() {
        let (mut p, state) = panel();
        let f = testing::draw(&mut p, &state, 100, 40);
        assert!(f.contains("BS2B Bauer Stereophonic-to-Binaural"), "{f}");
        assert!(f.contains("● On"), "the master toggle: {f}");
        assert!(f.contains("FREQUENCY RESPONSE"), "{f}");
        assert!(f.contains("OUTPUT PAIRS") && f.contains("Presets ▾"), "{f}");
        assert!(f.contains("PRESET"), "{f}");
        assert!(
            f.contains("700 Hz / 4.5 dB - Balanced, most popular"),
            "{f}"
        );
        assert!(f.contains("Jan Meier"), "{f}");
        assert!(f.contains("Cutoff Frequency"), "{f}");
        assert!(f.contains("700 Hz"), "{f}");
    }

    /// D24: `CrossfeedView.swift:301` draws PARAMETERS at 0.5 opacity outside
    /// the Custom voicing, and `survey-console.md` 2.14 says the same. The rows
    /// stay live, because editing one is what switches the voicing.
    #[test]
    fn the_parameters_dim_outside_custom_but_still_take_an_edit() {
        let (mut p, mut state) = panel();
        let theme = panel::key_theme();
        let (_, cf, _) = dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == "crossfeed")
            .copied()
            .unwrap();

        let dimmed = |p: &CrossfeedPanel, state: &DeviceState| -> Vec<bool> {
            p.rows(state, theme)
                .iter()
                .filter_map(|r| match r {
                    Row::Param(param) => Some(param.dimmed),
                    _ => None,
                })
                .collect()
        };

        // The fixture is on the Default voicing.
        assert_eq!(state.crossfeed().preset, 0);
        assert_eq!(dimmed(&p, &state), vec![true, true]);
        assert_eq!(
            testing::fg_of(&mut p, &state, 100, 40, "Cutoff Frequency"),
            theme.dim,
            "a dimmed parameter is drawn in the caption colour"
        );

        // Custom (wire 3) brings them back up.
        state.bulk.patch(cf + 1, &[3]);
        assert_eq!(state.crossfeed().preset, CUSTOM);
        assert_eq!(dimmed(&p, &state), vec![false, false]);
        assert_eq!(
            testing::fg_of(&mut p, &state, 100, 40, "Cutoff Frequency"),
            theme.fg
        );
    }

    /// The dim is presentation only: a nudge on the Default voicing still
    /// writes, and carries `cf.preset custom` with it.
    #[test]
    fn a_dimmed_parameter_is_still_editable_and_switches_the_voicing() {
        let (mut p, state) = panel();
        let rows = p.rows(&state, panel::key_theme());
        let param = panel::focus_rows(&rows)
            .iter()
            .position(|r| matches!(rows[*r], Row::Param(_)))
            .expect("a parameter row")
            + 1;
        p.body.focus = param;
        assert_eq!(
            p.handle(key(KeyCode::Right), &state),
            ScreenEvent::Command("cf.preset custom\ncf.freq 710".into())
        );
    }

    /// D46: `DESIGN.md` 7.8 draws a PRESET row as one line,
    /// `● Default    700 Hz / 4.5 dB - Balanced, most popular`. Four rows were
    /// costing eight, with the width to spare.
    #[test]
    fn a_preset_row_is_one_line_where_it_fits_and_two_where_it_does_not() {
        let (mut p, state) = panel();
        let f = testing::draw(&mut p, &state, 100, 40);
        let line = f
            .lines()
            .find(|l| l.contains("Default"))
            .expect("the Default row");
        assert!(
            line.contains("700 Hz / 4.5 dB - Balanced, most popular"),
            "label and detail on one line: {line}"
        );
        // The four voicings cost four rows, not eight.
        for name in ["Default", "Chu Moy", "Jan Meier", "Custom"] {
            assert_eq!(
                f.lines().filter(|l| l.contains(name)).count(),
                1,
                "{name}:\n{f}"
            );
        }

        // Too narrow for both, and the detail goes back under its label.
        let f = testing::draw(&mut p, &state, 46, 40);
        let line = f
            .lines()
            .find(|l| l.contains("Default"))
            .expect("the Default row");
        assert!(!line.contains("Balanced"), "{f}");
        assert!(f.contains("Balanced"), "but it is still drawn:\n{f}");
    }

    /// D45: `DESIGN.md` 7.8's template is header, chips, caption. The caption
    /// was drawn above the chips it describes.
    #[test]
    fn the_caption_sits_under_the_chip_row() {
        let (p, state) = panel();
        let rows = p.rows(&state, panel::key_theme());
        let chips = rows
            .iter()
            .position(|r| matches!(r, Row::Chips { .. }))
            .expect("a chip row");
        assert!(
            matches!(rows.get(chips + 1), Some(Row::Caption(_))),
            "{rows:?}"
        );
    }

    #[test]
    fn the_graph_says_disabled_when_the_master_switch_is_off() {
        let (mut p, mut state) = panel();
        let (_, cf, _) = dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == "crossfeed")
            .copied()
            .unwrap();
        state.bulk.patch(cf, &[0]);
        let f = testing::draw(&mut p, &state, 100, 40);
        assert!(f.contains("Disabled"), "{f}");
        assert!(f.contains("○ Off"), "{f}");
    }

    #[test]
    fn golden_frames_at_both_sizes() {
        let (_, state) = panel();
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = testing::frame(
                Tool::Crossfeed,
                Box::new(CrossfeedPanel::new()),
                &state,
                w,
                h,
            );
            assert_eq!(f.lines().count(), h as usize, "{w}x{h}");
            assert!(f.contains("Crossfeed"), "{w}x{h}:\n{f}");
            assert!(f.contains("X closes"), "{w}x{h}:\n{f}");
            assert!(f.contains("INPUTS"), "the sidebar stays: {w}x{h}");
        }
    }

    #[test]
    fn the_header_toggle_is_first_in_focus_order() {
        let (mut p, state) = panel();
        assert_eq!(p.body.focus, 0);
        assert_eq!(
            p.handle(key(KeyCode::Char(' ')), &state),
            ScreenEvent::Command("cf.on off".into())
        );
        p.handle(key(KeyCode::Down), &state);
        assert_eq!(p.body.focus, 1);
    }

    #[test]
    fn the_chips_write_the_pair_mask_and_the_menu_sets_it_wholesale() {
        let (mut p, state) = panel();
        // Focus 1 is the chip row: the graph and the section headers do not
        // take focus.
        p.body.focus = 1;
        assert_eq!(
            p.handle(key(KeyCode::Char(' ')), &state),
            ScreenEvent::Command("cf.outputs 0x2".into()),
            "pair 1 was on, so Space clears it"
        );
        assert_eq!(p.handle(key(KeyCode::Right), &state), ScreenEvent::Handled);
        assert_eq!(p.chip, 1);
        assert_eq!(
            p.handle(key(KeyCode::Char(' ')), &state),
            ScreenEvent::Command("cf.outputs 0x1".into())
        );
        match p.handle(key(KeyCode::Char('p')), &state) {
            ScreenEvent::Popup(list) => assert_eq!(list.items[1], "Pair 1 only (Headphones)"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            p.popup_result(Some(0), &state),
            ScreenEvent::Command("cf.outputs 0xF".into())
        );
        assert_eq!(
            p.popup_result(Some(2), &state),
            ScreenEvent::Handled,
            "a popup result with nothing pending is ignored"
        );
    }

    #[test]
    fn a_voicing_is_one_write_and_a_slider_edit_switches_to_custom() {
        let (mut p, state) = panel();
        let rows = p.rows(&state, panel::key_theme());
        let radios: Vec<usize> = panel::focus_rows(&rows)
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(rows[**r], Row::Radio { .. }))
            .map(|(i, _)| i + 1)
            .collect();
        p.body.focus = radios[1];
        assert_eq!(
            p.handle(key(KeyCode::Enter), &state),
            ScreenEvent::Command("cf.preset chumoy".into())
        );
        // The cutoff slider: the preset is Default, so a nudge says Custom
        // first and then the value.
        p.body.focus = radios[3] + 1;
        assert_eq!(
            p.handle(key(KeyCode::Right), &state),
            ScreenEvent::Command("cf.preset custom\ncf.freq 710".into())
        );
        assert_eq!(
            p.handle(key(KeyCode::Backspace), &state),
            ScreenEvent::Command("cf.preset custom\ncf.freq 700".into())
        );
    }

    #[test]
    fn the_time_delay_toggle_is_last() {
        let (mut p, state) = panel();
        let rows = p.rows(&state, panel::key_theme());
        p.body.focus = panel::focus_rows(&rows).len();
        assert_eq!(
            p.handle(key(KeyCode::Char(' ')), &state),
            ScreenEvent::Command("cf.itd off".into())
        );
    }

    #[test]
    fn every_advertised_key_is_handled_somewhere() {
        let (_, state) = panel();
        for help in KEYS {
            for k in crate::screens::tests::keys_for(help.key) {
                let mut hit = false;
                for focus in 0..12 {
                    let mut p = CrossfeedPanel::new();
                    p.body.focus = focus;
                    let before = (p.body.focus, p.chip);
                    let ev = p.handle(k, &state);
                    if ev != ScreenEvent::Unhandled || (p.body.focus, p.chip) != before {
                        hit = true;
                        break;
                    }
                }
                assert!(hit, "{:?} from {:?} is not bound", k.code, help.key);
            }
        }
    }

    // ------------------------------------------------------------ WP TM1

    mod wp_tm1 {
        use super::*;
        use crate::live::{ConsoleScreens, Live};
        use crate::perf::Performance;
        use crate::theme::{ColorDepth, Glyphs};
        use dspi_session::ext::read_pair_map;
        use dspi_transport::{Capture, FakeDevice, FakeOptions, Transport};

        fn open(fake: FakeDevice) -> (Session, DeviceState) {
            let mut t: Box<dyn Transport> = Box::new(fake);
            let caps = dspi_session::probe(t.as_mut()).unwrap();
            let mut session = Session::new(t, caps.clone()).unwrap();
            let bulk = session.snapshot().unwrap();
            (session, DeviceState::new(caps, bulk))
        }

        fn demo() -> (CrossfeedPanel, Session, DeviceState) {
            let (mut session, state) = open(FakeDevice::demo());
            let mut p = CrossfeedPanel::new();
            p.poll(&mut session, &state);
            assert_eq!(p.has_map, Some(true));
            (p, session, state)
        }

        /// What the runner does with a screen's event: run the request,
        /// re-read the image, hand the reply back.
        fn run(
            p: &mut CrossfeedPanel,
            session: &mut Session,
            state: &mut DeviceState,
            ev: ScreenEvent,
        ) -> ScreenEvent {
            match ev {
                ScreenEvent::Session(req) => {
                    let reply = (req.run)(session);
                    state.replace_bulk(session.snapshot().unwrap());
                    p.session_result(req.tag, reply, state)
                }
                ev => ev,
            }
        }

        fn pick(
            p: &mut CrossfeedPanel,
            session: &mut Session,
            state: &mut DeviceState,
            choice: usize,
        ) -> ScreenEvent {
            p.body.focus = MAP_FOCUS;
            assert!(matches!(
                p.handle(key(KeyCode::Enter), state),
                ScreenEvent::Popup(_)
            ));
            let ev = p.popup_result(Some(choice), state);
            run(p, session, state, ev)
        }

        #[test]
        fn wp_tm1_the_row_is_hidden_on_stock_firmware_and_before_the_probe() {
            // The stock fake stalls EXT_LIST.
            let (mut session, state) =
                open(FakeDevice::new(Capture::demo(), FakeOptions::default()));
            let mut p = CrossfeedPanel::new();
            assert!(!testing::draw(&mut p, &state, 100, 60).contains("PAIR MAP"));
            p.poll(&mut session, &state);
            assert_eq!(p.has_map, Some(false));
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(!f.contains("PAIR MAP"), "{f}");
            assert!(f.contains("OUTPUT PAIRS"), "the mask stays: {f}");
            assert!(f.contains("[1]") || f.contains("1]"), "plain chips: {f}");
            assert!(!f.contains("1: 1/2"), "{f}");
            // Focus is where it always was: the chips straight after the header.
            p.body.focus = 1;
            assert!(matches!(
                p.handle(key(KeyCode::Char(' ')), &state),
                ScreenEvent::Command(c) if c.starts_with("cf.outputs")
            ));
        }

        #[test]
        fn wp_tm1_the_row_has_its_own_section_above_the_mask_on_the_demo_device() {
            let (mut p, _session, state) = demo();
            let f = testing::draw(&mut p, &state, 100, 60);
            let map = f.find("PAIR MAP").expect(&f);
            let mask = f.find("OUTPUT PAIRS").expect(&f);
            assert!(map < mask, "{f}");
            assert!(f.contains("Default (1/2, 3/4, ...) ▾"), "{f}");
            assert!(f.contains("Which two outputs each pair"), "{f}");
            assert!(f.contains("1: 1/2") && f.contains("4: 7/8"), "{f}");
        }

        #[test]
        fn wp_tm1_k340_and_default_write_the_map_and_read_back() {
            let (mut p, mut session, mut state) = demo();
            let mask_before = state.crossfeed().output_pair_mask;

            let ev = pick(&mut p, &mut session, &mut state, 1);
            assert!(matches!(ev, ScreenEvent::Status(_)), "{ev:?}");
            assert_eq!(
                read_pair_map(session.transport_mut()).unwrap().0,
                PairMap::K340.0
            );
            assert_eq!(
                pair_map(&state).0,
                PairMap::K340.0,
                "the row reads the image"
            );
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(f.contains("One DAC per ear (K340) ▾"), "{f}");
            assert!(f.contains("1: 3/1") && f.contains("2: 4/2"), "{f}");
            assert!(f.contains("3: 5/6"), "the other pairs stay default: {f}");
            assert_eq!(
                state.crossfeed().output_pair_mask,
                mask_before,
                "the map never touches the mask"
            );

            let ev = pick(&mut p, &mut session, &mut state, 0);
            assert!(matches!(ev, ScreenEvent::Status(_)), "{ev:?}");
            assert_eq!(read_pair_map(session.transport_mut()).unwrap().0, [0; 4]);
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(f.contains("Default (1/2, 3/4, ...) ▾"), "{f}");
        }

        /// The offer: only after picking K340, only while the mask lacks a
        /// pair, and an explicit press, never automatic.
        #[test]
        fn wp_tm1_the_k340_preset_offers_the_mask_and_only_a_press_writes_it() {
            let (mut p, mut session, mut state) = demo();
            assert_eq!(
                state.crossfeed().output_pair_mask & PairMap::K340_MASK,
                0x01
            );
            pick(&mut p, &mut session, &mut state, 1);
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(f.contains("Crossfeed the woofers and the tweeters?"), "{f}");
            assert!(f.contains(ENABLE_BOTH) && f.contains(NOT_NOW), "{f}");

            // Focus lands on the offer's Not Now.
            assert_eq!(p.body.focus, MAP_FOCUS + 1);
            assert_eq!(p.handle(key(KeyCode::Enter), &state), ScreenEvent::Handled);
            assert!(!testing::draw(&mut p, &state, 100, 60).contains(ENABLE_BOTH));

            pick(&mut p, &mut session, &mut state, 1);
            p.handle(key(KeyCode::Right), &state);
            let mask = state.crossfeed().output_pair_mask | PairMap::K340_MASK;
            assert_eq!(
                p.handle(key(KeyCode::Enter), &state),
                ScreenEvent::Command(format!("cf.outputs 0x{mask:X}"))
            );
            assert!(!p.offer_mask);
        }

        #[test]
        fn wp_tm1_custom_edits_a_draft_and_apply_writes_it() {
            let (mut p, mut session, mut state) = demo();
            pick(&mut p, &mut session, &mut state, 2);
            assert_eq!(
                p.draft.as_deref(),
                Some(&[(0, 1), (2, 3), (4, 5), (6, 7)][..]),
                "the editor opens on the device's map"
            );
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(f.contains("Pair 1") && f.contains("L Out 1"), "{f}");
            assert!(f.contains(APPLY) && f.contains(CANCEL), "{f}");
            assert_eq!(
                read_pair_map(session.transport_mut()).unwrap().0,
                [0; 4],
                "nothing is sent until Apply"
            );

            // Swap pair 1's sides: left to output 2.
            assert_eq!(p.body.focus, MAP_FOCUS + 1);
            match p.handle(key(KeyCode::Enter), &state) {
                ScreenEvent::Popup(list) => {
                    assert_eq!(list.title, "Pair 1 left output");
                    assert_eq!(list.items.len(), 8);
                    assert!(list.items[0].starts_with("1 - "), "{:?}", list.items);
                }
                other => panic!("{other:?}"),
            }
            p.popup_result(Some(1), &state);
            p.handle(key(KeyCode::Right), &state);
            assert!(matches!(
                p.handle(key(KeyCode::Enter), &state),
                ScreenEvent::Popup(_)
            ));
            p.popup_result(Some(0), &state);
            assert_eq!(p.draft.as_ref().unwrap()[0], (1, 0));

            // Down to Apply: pairs 2, 3 and 4, then the buttons.
            for _ in 0..4 {
                p.handle(key(KeyCode::Down), &state);
            }
            let ev = p.handle(key(KeyCode::Enter), &state);
            assert!(matches!(ev, ScreenEvent::Session(_)), "{ev:?}");
            run(&mut p, &mut session, &mut state, ev);
            assert_eq!(
                read_pair_map(session.transport_mut()).unwrap().0,
                [0x01, 0, 0, 0]
            );
            assert_eq!(p.draft, None, "a good write closes the editor");
            assert_eq!(p.body.focus, MAP_FOCUS);
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(f.contains("Custom ▾"), "{f}");
            assert!(f.contains("1: 2/1"), "{f}");
        }

        #[test]
        fn wp_tm1_an_invalid_draft_is_refused_and_nothing_is_sent() {
            let (mut p, mut session, mut state) = demo();
            pick(&mut p, &mut session, &mut state, 2);
            // Pair 1's left onto output 3, which pair 2 already has.
            p.handle(key(KeyCode::Enter), &state);
            p.popup_result(Some(2), &state);
            let f = testing::draw(&mut p, &state, 46, 60);
            assert!(f.contains(DRAFT_WARNING), "{f}");

            for _ in 0..4 {
                p.handle(key(KeyCode::Down), &state);
            }
            p.handle(key(KeyCode::Right), &state);
            let ev = p.handle(key(KeyCode::Enter), &state);
            assert_eq!(ev, ScreenEvent::Status(INVALID_DRAFT.into()));
            assert!(p.draft.is_some(), "the draft stays to be fixed");
            assert_eq!(read_pair_map(session.transport_mut()).unwrap().0, [0; 4]);

            // Cancel is the column left of Apply.
            p.handle(key(KeyCode::Left), &state);
            assert_eq!(p.handle(key(KeyCode::Enter), &state), ScreenEvent::Handled);
            assert_eq!(p.draft, None);
            assert_eq!(read_pair_map(session.transport_mut()).unwrap().0, [0; 4]);
        }

        #[test]
        fn wp_tm1_a_failed_write_keeps_the_editor_and_is_not_retried() {
            let (mut p, _session, state) = demo();
            p.draft = Some(vec![(1, 0), (2, 3), (4, 5), (6, 7)]);
            let ev = p.session_result(TAG_MAP_CUSTOM, SessionReply::Err("no".into()), &state);
            assert_eq!(ev, ScreenEvent::Status("no".into()));
            assert!(p.draft.is_some());
            assert_eq!(pair_map(&state).0, [0; 4], "the row keeps the device's map");
        }

        /// The two rows side by side: the chips still write the mask alone, and
        /// the menu writes the map alone.
        #[test]
        fn wp_tm1_the_map_is_not_confused_with_the_mask() {
            let (mut p, mut session, mut state) = demo();
            let rows = p.rows(&state, panel::key_theme());
            let focus = panel::focus_rows(&rows);
            assert!(matches!(
                rows[focus[0]],
                Row::Menu { ref title, .. } if title == "Pair map"
            ));
            assert!(matches!(rows[focus[1]], Row::Chips { .. }));
            assert!(
                rows.iter()
                    .any(|r| matches!(r, Row::Section { title, .. } if title == "Output pairs"))
            );

            p.body.focus = MAP_FOCUS + 1;
            assert_eq!(
                p.handle(key(KeyCode::Char(' ')), &state),
                ScreenEvent::Command("cf.outputs 0x0".into()),
                "a chip is the mask"
            );
            // `p` on the chips is still the mask's presets, on the map's row
            // the map's.
            match p.handle(key(KeyCode::Char('p')), &state) {
                ScreenEvent::Popup(list) => assert_eq!(list.title, "Output pairs"),
                other => panic!("{other:?}"),
            }
            assert_eq!(p.pending, Some(Pending::Pairs));
            p.popup_result(None, &state);
            p.body.focus = MAP_FOCUS;
            match p.handle(key(KeyCode::Char('p')), &state) {
                ScreenEvent::Popup(list) => assert_eq!(list.title, "Pair map"),
                other => panic!("{other:?}"),
            }
            assert_eq!(p.pending, Some(Pending::Map));
            p.popup_result(None, &state);

            let before = state.crossfeed().output_pair_mask;
            pick(&mut p, &mut session, &mut state, 1);
            assert_eq!(state.crossfeed().output_pair_mask, before);
        }

        /// The row arrives on the first poll, above what the person may be on.
        #[test]
        fn wp_tm1_focus_stays_on_its_row_when_the_map_row_arrives() {
            let (mut session, state) = open(FakeDevice::demo());
            let mut p = CrossfeedPanel::new();
            p.body.focus = 1;
            let before = p.rows(&state, panel::key_theme());
            assert!(matches!(p.body.row(&before), Some(Row::Chips { .. })));
            p.poll(&mut session, &state);
            let after = p.rows(&state, panel::key_theme());
            assert!(matches!(p.body.row(&after), Some(Row::Chips { .. })));

            // The header stays the header.
            let (mut session, state) = open(FakeDevice::demo());
            let mut p = CrossfeedPanel::new();
            p.poll(&mut session, &state);
            assert_eq!(p.body.focus, 0);
        }

        #[test]
        fn wp_tm1_a_lost_device_forgets_the_row_and_the_next_one_is_asked_again() {
            let (mut p, mut session, mut state) = demo();
            pick(&mut p, &mut session, &mut state, 2);
            state.connected = false;
            let f = testing::draw(&mut p, &state, 100, 60);
            assert!(!f.contains("PAIR MAP"), "{f}");
            assert_eq!((p.has_map, p.draft.is_none()), (None, true));
            // No traffic for a device that is not there.
            p.poll(&mut session, &state);
            assert_eq!(p.has_map, None);

            state.connected = true;
            p.poll(&mut session, &state);
            assert_eq!(p.has_map, Some(true));
        }

        /// `validated`'s encoding against the fake's answer on the wire, on
        /// every pairing of the first two pairs over all eight outputs. Both
        /// use [`PairMap::is_valid`], which dspi-proto holds to the firmware.
        #[test]
        fn wp_tm1_validation_agrees_with_the_firmware_model() {
            let (_p, mut session, _state) = demo();
            for l0 in 0..8u8 {
                for r0 in 0..8u8 {
                    for l1 in 0..8u8 {
                        for r1 in 0..8u8 {
                            let pairs = [(l0, r0), (l1, r1), (4, 5), (6, 7)];
                            // A valid map in the same place, so a refused one
                            // shows up as the old map still being there.
                            ext::write_pair_map(session.transport_mut(), PairMap([0; 4])).unwrap();
                            let raw = PairMap([(r0 << 4) | l0, (r1 << 4) | l1, 0, 0]);
                            // (l, r) with l == r == 0 encodes as the default
                            // pair; the wire cannot say it, so only `validated`
                            // can refuse it.
                            if l0 == r0 || l1 == r1 {
                                assert_eq!(validated(&pairs, 4), None, "{pairs:?}");
                                continue;
                            }
                            let accepted =
                                ext::write_pair_map(session.transport_mut(), raw).is_ok();
                            assert_eq!(validated(&pairs, 4).is_some(), accepted, "{pairs:?}");
                        }
                    }
                }
            }
            assert_eq!(validated(&[(0, 1)], 4), None, "one pair per platform pair");
            assert_eq!(validated(&[], 0), None);
            assert_eq!(
                validated(&[(2, 0), (3, 1)], 2),
                Some(PairMap::K340),
                "valid on RP2040 too"
            );
            assert_eq!(validated(&[(4, 0), (3, 1)], 2), None, "past two pairs");
        }

        #[test]
        fn wp_tm1_an_explicit_default_byte_reads_as_default() {
            assert_eq!(MapChoice::of(&PairMap([0x10, 0, 0, 0])), MapChoice::Default);
            assert_eq!(MapChoice::of(&PairMap::K340), MapChoice::K340);
            assert_eq!(MapChoice::of(&PairMap([0x01, 0, 0, 0])), MapChoice::Custom);
        }

        #[test]
        fn wp_tm1_the_frame_fits_both_sizes_with_the_row() {
            let (_p, mut session, state) = demo();
            for (w, h) in [(120u16, 40u16), (80, 24)] {
                let mut p = CrossfeedPanel::new();
                p.poll(&mut session, &state);
                let f = testing::frame(Tool::Crossfeed, Box::new(p), &state, w, h);
                assert_eq!(f.lines().count(), h as usize, "{w}x{h}");
                assert!(f.contains("PAIR MAP"), "{w}x{h}:\n{f}");
            }
        }

        /// The whole path on the demo device: the real runner, its keys, its
        /// popup and its refresh.
        #[test]
        fn wp_tm1_the_runner_drives_default_k340_and_custom_end_to_end() {
            let (mut session, state) = open(FakeDevice::demo());
            let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
            let mut live = Live::new(
                state,
                theme,
                Performance::default(),
                Box::new(ConsoleScreens::new()),
            );
            assert!(live.show(&mut session, "crossfeed"));
            let (_, screen) = live.shell.tool.as_mut().unwrap();
            screen.poll(&mut session, &live.state);

            let press = |live: &mut Live, session: &mut Session, code: KeyCode| {
                live.handle_key(session, key(code));
            };
            let map = |session: &mut Session| read_pair_map(session.transport_mut()).unwrap().0;

            // Header, then down to the menu; K340 is one down in its list.
            press(&mut live, &mut session, KeyCode::Down);
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Down);
            press(&mut live, &mut session, KeyCode::Enter);
            assert_eq!(map(&mut session), PairMap::K340.0);
            assert_eq!(
                pair_map(&live.state).0,
                PairMap::K340.0,
                "the runner refreshed"
            );

            // The offer has focus: Enable, through the shared grammar.
            let mask = live.state.crossfeed().output_pair_mask;
            press(&mut live, &mut session, KeyCode::Right);
            press(&mut live, &mut session, KeyCode::Enter);
            assert_eq!(
                live.state.crossfeed().output_pair_mask,
                mask | PairMap::K340_MASK,
                "both pairs"
            );

            // Back on the menu: the list opens on K340, Default is one up.
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Up);
            press(&mut live, &mut session, KeyCode::Enter);
            assert_eq!(map(&mut session), [0; 4]);

            // Custom: pair 1's left onto output 2, its right onto output 1.
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Down);
            press(&mut live, &mut session, KeyCode::Down);
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Down);
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Right);
            press(&mut live, &mut session, KeyCode::Enter);
            press(&mut live, &mut session, KeyCode::Up);
            press(&mut live, &mut session, KeyCode::Enter);
            for _ in 0..4 {
                press(&mut live, &mut session, KeyCode::Down);
            }
            press(&mut live, &mut session, KeyCode::Enter);
            assert_eq!(map(&mut session), [0x01, 0, 0, 0]);
            assert_eq!(pair_map(&live.state).0, [0x01, 0, 0, 0]);
        }
    }
}
