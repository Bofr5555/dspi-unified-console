//! The Console's core screens: the dashboard overview, the input page and the
//! output page, plus the filter list they share.
//!
//! Every screen here draws from `&DeviceState` each frame and keeps only its
//! own cursor and edit state, so a change made anywhere (a knob, a remote,
//! another host) is on screen the next tick without the screen being told. A
//! write is never issued from a screen: it is asked for as a
//! [`ScreenEvent::Command`](crate::shell::ScreenEvent::Command) in the shared
//! command grammar, which is also what the echo line shows, so anything the
//! interface can do a person can type.
//!
//! What the device does not store lives in [`SharedState`]: which input pairs
//! are linked (the Console's `linkedInputPairs`), the preset names read back
//! from the device, and the channel clipboard. The screen factory owns one and
//! hands every screen a handle to it.

pub mod autoeq;
pub mod clipboard;
pub mod crossfeed;
pub mod filters;
pub mod input;
pub mod leveller;
pub mod limiter;
pub mod linkwitz;
pub mod loudness;
pub mod matrix;
pub mod monitor;
pub mod output;
pub mod overview;
pub mod panel;
pub mod presets;
pub mod psybass;
pub mod signals;
pub mod spectrum;
pub mod stats;
pub mod subharm;
pub mod tube;
pub mod upmixer;

use std::cell::RefCell;
use std::rc::Rc;

use dspi_proto::FilterType;
use dspi_proto::value::EqParamPacket;
use dspi_proto::xover;
use dspi_session::DeviceState;
// Crossover gate and the Console's cleared band: shared with every GUI (WP S5).
pub use dspi_session::clipboard::{cleared_band, supports_crossover};

pub use autoeq::AutoEqPanel;
pub use clipboard::ChannelClipboard;
pub use crossfeed::CrossfeedPanel;
pub use filters::{FilterList, FilterMode};
pub use input::InputPage;
pub use leveller::LevellerPanel;
pub use linkwitz::LinkwitzPanel;
pub use loudness::LoudnessPanel;
pub use matrix::MatrixPanel;
pub use monitor::MonitorPanel;
pub use output::OutputPage;
pub use overview::Overview;
pub mod quick;
pub use presets::{PresetChoice, PresetMenu};
pub use psybass::PsybassPanel;
pub use signals::SignalsPanel;
pub use spectrum::SpectrumPanel;
pub use stats::StatsPanel;
pub use subharm::SubharmPanel;
pub use tube::TubePanel;
pub use upmixer::UpmixerPanel;

/// The application-side state the screens share.
#[derive(Debug, Default, Clone)]
pub struct SharedState {
    /// Linked input pairs, by pair index (0 is inputs 1 and 2). The Console
    /// keeps this in the app, not on the device, and so do we.
    pub linked_pairs: [bool; 4],
    /// Preset slot names, read back with `preset.name <slot>` and refreshed
    /// after a rename. Empty means the slot has no name.
    pub preset_names: Vec<String>,
    /// Bit N set means slot N holds a preset, from `REQ_PRESET_GET_DIR`
    /// (the Console's `isPresetOccupied`).
    pub occupied: u16,
    /// The slot the device loads at power on, when the startup mode is
    /// "specified".
    pub default_slot: Option<u8>,
    pub clipboard: Option<ChannelClipboard>,
    /// The polled diagnostics behind the Stats panel. The runner refreshes it
    /// on the Console's two-second cadence while that panel is open; nothing
    /// in it is notified, so there is no other way for it to move.
    pub stats: crate::actions::Stats,
    /// Every notification seen, for the Interrupt Monitor. The runner fills it
    /// from the same drain that keeps the device state current, so the monitor
    /// does not need a second reader on the endpoint.
    pub log: crate::actions::EventLog,
    /// The AutoEQ database, loaded once and kept: it is several megabytes of
    /// JSON, and re-reading it every time the browser opens would be felt.
    pub autoeq: Option<Rc<dspi_session::autoeq::Database>>,
    /// Favourite profile ids, mirrored from the favourites file.
    pub favourites: Vec<String>,
    /// The main graph's window and zoom, mirrored by the runner each frame so
    /// the overview's cells draw on the same axes (DESIGN 12.2).
    pub graph: crate::graph::GraphSettings,
    /// The spectrum analyser: its engine, its Settings values and the
    /// channels chosen. The runner ticks the engine; views subscribe to it.
    pub spectrum: spectrum::SpectrumState,
}

impl SharedState {
    /// Is this input's adjacent pair linked?
    pub fn input_linked(&self, input: usize) -> bool {
        self.linked_pairs.get(input / 2).copied().unwrap_or(false)
    }

    /// The channel an edit on `input` mirrors onto, if the pair is linked and
    /// the partner is live.
    pub fn linked_partner(&self, input: usize, num_inputs: usize) -> Option<usize> {
        let partner = input ^ 1;
        // A pair is only available while both of its channels are live, which
        // is the Console's `pairAvailable`.
        (self.input_linked(input) && input.max(partner) < num_inputs).then_some(partner)
    }

    pub fn set_linked(&mut self, pair: usize, on: bool) {
        if let Some(slot) = self.linked_pairs.get_mut(pair) {
            *slot = on;
        }
    }
}

/// A handle every screen the factory makes holds a clone of.
pub type Shared = Rc<RefCell<SharedState>>;

pub fn shared() -> Shared {
    Rc::new(RefCell::new(SharedState::default()))
}

// ---------------------------------------------------------------------------
// What the device can do
// ---------------------------------------------------------------------------

/// A firmware version string as a comparable triple; an unparseable version
/// reads as `(0, 0, 0)`, which is the Console's conservative default.
pub fn firmware_triple(version: &str) -> (u32, u32, u32) {
    let mut parts = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Per-band bypass shipped in firmware 1.1.4 (`DSPViewModel.firmwareSupports
/// BandBypass`); older firmware stalls the opcode, so the disc is hidden.
pub fn supports_bypass(state: &DeviceState) -> bool {
    firmware_triple(&state.caps.firmware) >= (1, 1, 4)
}

/// Notch and all-pass shipped in the same release as bypass.
fn supports_notch(state: &DeviceState) -> bool {
    supports_bypass(state)
}

/// The Linkwitz Transform arrived at V22, and the Console offers it on outputs
/// only.
pub fn supports_linkwitz(state: &DeviceState) -> bool {
    state.caps.wire_format >= 22
}

/// The PEQ types this firmware will accept, in the Console's menu order.
///
/// `availableFilterTypes` in `ContentView.swift`: every gate is a firmware
/// version or a wire-format version, never a compiled-in list.
pub fn available_types(state: &DeviceState, include_linkwitz: bool) -> Vec<FilterType> {
    let wire = state.caps.wire_format;
    let mut out = vec![
        FilterType::Flat,
        FilterType::Peaking,
        FilterType::LowShelf,
        FilterType::HighShelf,
        FilterType::LowPass,
        FilterType::HighPass,
    ];
    if supports_notch(state) {
        out.push(FilterType::Notch);
        out.push(FilterType::AllPass);
    }
    if wire >= 13 {
        out.push(FilterType::AllPass1);
    }
    if wire >= 14 {
        out.push(FilterType::LowShelf1);
        out.push(FilterType::HighShelf1);
    }
    if wire >= 28 {
        out.push(FilterType::LowPass1);
        out.push(FilterType::HighPass1);
    }
    if include_linkwitz && supports_linkwitz(state) {
        out.push(FilterType::LinkwitzTransform);
    }
    out
}

/// The longest delay this platform's line holds, in ms: 21 on RP2040, 42 on
/// RP2350 (-- WP CG37 -- `Platform::max_delay_ms`, the session's and the
/// core's limit; the Console's 42/85 is more than the line delivers).
pub fn max_delay_ms(state: &DeviceState) -> f64 {
    f64::from(state.caps.platform.max_delay_ms())
}

// -- WP CG37 --
/// A delay as the line delivers it. A value stored past the line (another
/// app can set one: the firmware keeps the ms and caps the samples) shows as
/// the line's longest.
pub fn delivered_delay_ms(state: &DeviceState, ms: f32) -> f64 {
    f64::from(ms).min(max_delay_ms(state))
}

// ---------------------------------------------------------------------------
// Names and numbers
// ---------------------------------------------------------------------------

/// The Console's full type name, `FilterType.name` in `DSPMath.swift`: the
/// core's one table (WP CG34, `FilterType::display_name`).
pub fn type_name(t: FilterType) -> String {
    t.display_name()
}

/// The dashboard's compact code, `DashboardRow.typeCode`
/// (`DashboardView.swift:370-384`), which reads `OFF` for an unset band.
///
/// PEQ passes read as cuts, as their full names do: a low pass is a high cut
/// (`HC`) and a high pass a low cut (`LC`), after `FilterType.shortLabel`
/// (`DSPMath.swift:168-179`). Crossovers keep `LP` and `HP`, and filter files
/// keep REW's pass codes (`fileCode`, `DSPMath.swift:285-297`), which is why
/// `dspi_session::filterfile` keeps a table of its own.
pub fn type_code(t: FilterType) -> String {
    match t {
        FilterType::Flat => "OFF".into(),
        FilterType::Peaking => "PK".into(),
        FilterType::LowShelf => "LS".into(),
        FilterType::HighShelf => "HS".into(),
        FilterType::LowPass => "HC".into(),
        FilterType::HighPass => "LC".into(),
        FilterType::Notch => "NO".into(),
        FilterType::AllPass => "AP".into(),
        FilterType::AllPass1 => "AP1".into(),
        FilterType::LowShelf1 => "LS1".into(),
        FilterType::HighShelf1 => "HS1".into(),
        FilterType::LowPass1 => "HC1".into(),
        FilterType::HighPass1 => "LC1".into(),
        FilterType::LinkwitzTransform => "LT".into(),
        other => match xover::meta(other.to_raw()) {
            Some(m) => format!(
                "{}{}{}",
                m.family.short().to_uppercase(),
                m.order,
                if m.high_pass { "HP" } else { "LP" }
            ),
            None => format!("{}", other.to_raw()),
        },
    }
}

/// The token the command grammar takes for a filter type: the registry's name
/// for a PEQ type, the file code (`lr4lp`) for a crossover.
pub fn type_token(t: FilterType) -> String {
    if let Some(m) = xover::meta(t.to_raw()) {
        return format!(
            "{}{}{}",
            m.family.short(),
            m.order,
            if m.high_pass { "hp" } else { "lp" }
        );
    }
    match t {
        FilterType::Flat => "flat".into(),
        FilterType::Peaking => "peak".into(),
        FilterType::LowShelf => "lowshelf".into(),
        FilterType::HighShelf => "highshelf".into(),
        FilterType::LowPass => "lowpass".into(),
        FilterType::HighPass => "highpass".into(),
        FilterType::Notch => "notch".into(),
        FilterType::AllPass => "allpass".into(),
        FilterType::AllPass1 => "allpass1".into(),
        FilterType::LowShelf1 => "lowshelf1".into(),
        FilterType::HighShelf1 => "highshelf1".into(),
        FilterType::LinkwitzTransform => "linkwitz".into(),
        FilterType::LowPass1 => "lowpass1".into(),
        FilterType::HighPass1 => "highpass1".into(),
        other => other.to_raw().to_string(),
    }
}

/// How a command names a channel: the device's own slug, so a line copied off
/// the echo line works in a shell against this device.
pub fn channel_token(state: &DeviceState, channel: usize) -> String {
    if let Some(c) = state.caps.channels.get(channel)
        && !c.slug.is_empty()
    {
        return c.slug.clone();
    }
    let ni = state.caps.num_inputs as usize;
    if channel < ni {
        format!("in.{}", channel + 1)
    } else {
        format!("out.{}", channel - ni + 1)
    }
}

/// The channel a sidebar output index equalises. Outputs follow the inputs in
/// the unified channel space.
pub fn output_channel(state: &DeviceState, output: usize) -> usize {
    state.caps.num_inputs as usize + output
}

/// The Console's `formatTrimmed`: a fixed number of decimals with trailing
/// zeros stripped, but never down to a bare integer.
pub fn trimmed(value: f64, decimals: usize, signed: bool) -> String {
    let full = if signed {
        format!("{value:+.d$}", d = decimals)
    } else {
        format!("{value:.d$}", d = decimals)
    };
    let Some((head, tail)) = full.split_once('.') else {
        return full;
    };
    let tail = tail.trim_end_matches('0');
    if tail.is_empty() {
        format!("{head}.0")
    } else {
        format!("{head}.{tail}")
    }
}

/// A number as a command line would carry it: enough precision for every field
/// the device has, with no trailing noise.
pub fn number(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// How a command names a band: 1-based for the PEQ bank, and the firmware's
/// own 20 to 23 for the crossover bank.
pub fn display_band(band: u8) -> String {
    if (20..24).contains(&band) {
        band.to_string()
    } else {
        (band + 1).to_string()
    }
}

/// The commands that write one whole band.
///
/// The firmware stores a band as one 16-byte packet and the grammar's `eq`
/// verb sends exactly that, so a single field edit is still a whole-band
/// write. The packet's bypass byte is always sent clear by `eq`, so a bypassed
/// band needs the flag put back.
pub fn band_command(
    state: &DeviceState,
    channel: usize,
    band: u8,
    p: &EqParamPacket,
) -> Vec<String> {
    let ch = channel_token(state, channel);
    let n = display_band(band);
    let mut line = format!(
        "eq {ch} {n} {} {} {} {}",
        type_token(p.filter_type),
        number(p.freq),
        number(p.q),
        number(p.gain_db),
    );
    if let Some(qp) = p.qp.filter(|_| p.filter_type.is_linkwitz()) {
        line.push(' ');
        line.push_str(&number(qp));
    }
    let mut out = vec![line];
    if p.bypass && p.filter_type != FilterType::Flat && supports_bypass(state) {
        out.push(format!("eq.bypass {ch} {n} on"));
    }
    out
}

// -- WP TM18 --
/// The Console's Identify, as commands (`Commands.swift:3688-3704`): the
/// channel-ID blip melody on one output at -12 dB, two passes and then idle,
/// so a listener can tell which speaker is which.
///
/// The output is clamped to the generator's valid-channel mask, or left as it
/// is when the probe has no mask to go by. The firmware walks a channel-ID
/// signal whatever the flags say (`siggen.c:1088-1089`), so no flag is sent.
///
/// `None` when the firmware has no signal generator to play it with, which is
/// the same gate the sidebar's `i` applies, or when this is not an output the
/// generator can drive.
pub fn identify_command(state: &DeviceState, output: usize) -> Option<String> {
    if !panel::has_feature(state, "test_signals") {
        return None;
    }
    let bit = 1u16.checked_shl(u32::try_from(output).ok()?)?;
    let valid = match &state.caps.siggen {
        Some(c) if c.valid_channel_mask != 0 => c.valid_channel_mask,
        _ => bit,
    };
    let mask = bit & valid;
    (mask != 0).then(|| {
        format!(
            "sig.config type=channel-id channels=0x{mask:X} invert=0x0 level=-12 duration=0 \
             repeat=2 gap=0 p1=120\nsig.control start"
        )
    })
}

/// The Console's name for a channel, falling back to its descriptor when the
/// device has none.
pub fn channel_name(state: &DeviceState, channel: usize) -> String {
    let name = state.channel_name(channel);
    if !name.is_empty() {
        return name;
    }
    crate::theme::ChannelRole::of(channel as u8, state.caps.num_inputs, state.caps.num_outputs)
        .descriptor(state.caps.num_outputs)
}

// -- BUG1 --
/// How one output is shown: its label, its slot's name, and the label cut
/// short for the sidebar.
#[derive(Debug, Clone, Default)]
struct Shown {
    label: String,
    slot: String,
    short: String,
}

/// The bulk image the labels were worked out from, and the labels.
#[derive(Default)]
struct LabelCache {
    key: Vec<u8>,
    outputs: Vec<Shown>,
}

thread_local! {
    /// [`channel_label`] is called while drawing; the labels change only
    /// with the image.
    static LABELS: std::cell::RefCell<LabelCache> = std::cell::RefCell::new(LabelCache::default());
}

/// `f` of output `output`'s [`Shown`] for `state`, the labels read again only
/// when its bulk image changed; `None` past the outputs.
fn with_shown<T>(state: &DeviceState, output: usize, f: impl FnOnce(&Shown) -> T) -> Option<T> {
    use dspi_session::output_label::{Band, LabelSource, output_labels};
    LABELS.with_borrow_mut(|c| {
        if c.key.as_slice() != state.bulk.as_bytes() {
            c.outputs = output_labels(&LabelSource::from_state(state))
                .into_iter()
                .map(|l| {
                    // -- WP REVEFIX -- (REVE C5) an unused output is the core's "None"
                    // beside its OUTn (owner 2026-11-24: "3 · None"), never "None·Lo"
                    let short = match (l.user_named, l.band) {
                        (true, _) => l.label.clone(),
                        (false, _) if l.unused => l.short_label.clone(),
                        (false, Band::Full) => l.channel.clone(),
                        (false, Band::Low) => format!("{}·Lo", l.channel),
                        (false, Band::High) => format!("{}·Hi", l.channel),
                        (false, Band::Mid) => format!("{}·Mid", l.channel),
                    };
                    Shown {
                        label: l.label,
                        slot: l.slot_name,
                        short,
                    }
                })
                .collect();
            c.key = state.bulk.as_bytes().to_vec();
        }
        c.outputs.get(output).map(f)
    })
}

/// What to show for a channel (BUG1): an input's name; for an output, the
/// name the user set, else the core's label from the routing and the filters
/// ("Out 1 · R · Low", dspi-session's `output_label`). [`channel_name`] stays
/// the stored name, for a rename and the filter file.
pub fn channel_label(state: &DeviceState, channel: usize) -> String {
    let ni = usize::from(state.caps.num_inputs);
    match channel.checked_sub(ni) {
        Some(o) if o < usize::from(state.caps.num_outputs) => {
            with_shown(state, o, |s| s.label.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| channel_name(state, channel))
        }
        _ => channel_name(state, channel),
    }
}

/// [`channel_label`] for the sidebar's five columns beside the output's
/// `OUTn`: the user's name, else the channel with its band cut short ("R·Lo",
/// "R·Hi", "L·Mid"; a full-range output just "L", "R" or "L+R").
pub fn channel_short_label(state: &DeviceState, channel: usize) -> String {
    let ni = usize::from(state.caps.num_inputs);
    match channel.checked_sub(ni) {
        Some(o) if o < usize::from(state.caps.num_outputs) => {
            with_shown(state, o, |s| s.short.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| channel_name(state, channel))
        }
        _ => channel_name(state, channel),
    }
}

/// The firmware's name for output `output`'s slot ("I2S 1 L", "PDM"), for a
/// detail line; empty past the outputs.
pub fn slot_name(state: &DeviceState, output: usize) -> String {
    with_shown(state, output, |s| s.slot.clone()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::shell::{Shell, fixture};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// What Identify asks the generator for, as the command parser reads it.
    fn tm18_identify_config(
        state: &DeviceState,
        output: usize,
    ) -> dspi_proto::packets::SiggenConfig {
        let cmds = identify_command(state, output).expect("identify offered");
        let lines: Vec<&str> = cmds.lines().collect();
        assert_eq!(lines.len(), 2, "{cmds}");
        assert_eq!(lines[1], "sig.control start");
        let fields: Vec<(&str, &str)> = lines[0]
            .split_whitespace()
            .skip(1)
            .map(|f| f.split_once('=').expect("key=value"))
            .collect();
        dspi_proto::packets::SiggenConfig::from_fields(&fields).expect("parses")
    }

    #[test]
    fn tm18_identify_is_the_consoles_two_passes_at_minus_12() {
        let state = panel::testing::state();
        let c = tm18_identify_config(&state, 2);
        assert_eq!(c.signal_type, 14, "SIGGEN_CHANNEL_ID");
        assert_eq!(c.channel_mask, 0x4, "the third output alone");
        assert_eq!(c.level_db, -12.0);
        assert_eq!(c.repeat_count, 2);
        assert_eq!(c.p1, 120.0, "120 ms blips");
        assert_eq!(
            (c.invert_mask, c.flags, c.duration_ms, c.gap_ms),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn tm18_identify_is_clamped_to_the_valid_channel_mask() {
        let mut state = panel::testing::state();
        // 0x1FF: the ninth output is the last the generator can drive.
        assert_eq!(tm18_identify_config(&state, 8).channel_mask, 0x100);
        assert_eq!(identify_command(&state, 9), None);
        // Past the mask's sixteen bits is refused rather than shifted away.
        assert_eq!(identify_command(&state, 16), None);
        assert_eq!(identify_command(&state, usize::MAX), None);
        // A mask with a hole in it is what the generator said, so it holds.
        if let Some(c) = state.caps.siggen.as_mut() {
            c.valid_channel_mask = 0b101;
        }
        assert_eq!(identify_command(&state, 1), None);
        assert_eq!(tm18_identify_config(&state, 2).channel_mask, 0b100);
        // No mask yet: the raw bit, as the Console falls back before its first
        // caps fetch.
        if let Some(c) = state.caps.siggen.as_mut() {
            c.valid_channel_mask = 0;
        }
        assert_eq!(tm18_identify_config(&state, 12).channel_mask, 1 << 12);
        state.caps.siggen = None;
        assert_eq!(tm18_identify_config(&state, 12).channel_mask, 1 << 12);
    }

    #[test]
    fn tm18_identify_needs_a_generator() {
        assert_eq!(identify_command(&fixture::state(), 0), None);
    }

    /// Turn a key-line token into the key events it stands for, so a screen
    /// can prove that every key it advertises is bound. The shell has the same
    /// helper for its own regions.
    pub(crate) fn keys_for(token: &str) -> Vec<KeyEvent> {
        let plain = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        token
            .split_whitespace()
            .map(|k| match k {
                "↑" => plain(KeyCode::Up),
                "↓" => plain(KeyCode::Down),
                "←" => plain(KeyCode::Left),
                "→" => plain(KeyCode::Right),
                "Enter" => plain(KeyCode::Enter),
                "Space" => plain(KeyCode::Char(' ')),
                "Backspace" => plain(KeyCode::Backspace),
                "Esc" => plain(KeyCode::Esc),
                "PgUp" => plain(KeyCode::PageUp),
                "End" => plain(KeyCode::End),
                "Home" => plain(KeyCode::Home),
                "PgDn" => plain(KeyCode::PageDown),
                "1-9,0" | "1-9" => plain(KeyCode::Char('4')),
                other => {
                    let c = other.chars().next().unwrap();
                    assert_eq!(other.chars().count(), 1, "unknown key token {other}");
                    KeyEvent::new(
                        KeyCode::Char(c),
                        if c.is_ascii_uppercase() {
                            KeyModifiers::SHIFT
                        } else {
                            KeyModifiers::NONE
                        },
                    )
                }
            })
            .collect()
    }

    /// A whole shell with one of the Console's screens in its detail region,
    /// so the golden frames are what a person actually sees.
    fn shell(detail: Box<dyn crate::shell::Screen>, selection: crate::shell::Selection) -> Shell {
        let theme = crate::theme::Theme::console(
            crate::theme::ColorDepth::TrueColor,
            crate::theme::Glyphs::Braille,
        );
        let mut model = fixture::rp2350(&theme);
        model.selection = selection;
        let mut shell = Shell::new(model, theme, detail);
        shell.focus = crate::shell::Focus::Screen;
        shell
    }

    fn frame(shell: &mut Shell, w: u16, h: u16) -> String {
        let state = fixture::state();
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).expect("backend");
        term.draw(|f| shell.draw(f.area(), f.buffer_mut(), &state))
            .expect("draw");
        let buf = term.backend().buffer();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn screens_under_test() -> Vec<(
        &'static str,
        Box<dyn crate::shell::Screen>,
        crate::shell::Selection,
    )> {
        let state = fixture::state();
        vec![
            (
                "overview",
                Box::new(Overview::new(shared())) as Box<dyn crate::shell::Screen>,
                crate::shell::Selection::Overview,
            ),
            (
                "input",
                Box::new(InputPage::new(0, shared(), &state)),
                crate::shell::Selection::Input(0),
            ),
            (
                "output",
                Box::new(OutputPage::new(0, shared(), &state)),
                crate::shell::Selection::Output(0),
            ),
        ]
    }

    #[test]
    fn every_screen_fills_the_detail_region_at_both_sizes() {
        for (name, detail, selection) in screens_under_test() {
            let mut s = shell(detail, selection);
            for (w, h) in [(120u16, 40u16), (80, 24)] {
                let f = frame(&mut s, w, h);
                assert_eq!(f.lines().count(), h as usize, "{name} at {w}x{h}");
                assert!(
                    f.contains("INPUTS") || f.contains("OUTPUTS"),
                    "the sidebar is still there: {name}"
                );
                assert!(
                    !f.contains("arrives in Phase"),
                    "{name} at {w}x{h} is still a placeholder:\n{f}"
                );
            }
        }
    }

    #[test]
    fn the_overview_draws_its_grid_through_the_shell() {
        let state = fixture::state();
        let _ = &state;
        let mut s = shell(
            Box::new(Overview::new(shared())),
            crate::shell::Selection::Overview,
        );
        let f = frame(&mut s, 120, 40);
        assert!(f.contains("╭ FL FR "), "{f}");
        assert!(f.contains("LP 80 Hz · -3.0 dB"), "{f}");
        assert!(
            !f.contains("Filter Response"),
            "the grid carries the curves: {f}"
        );
        assert!(f.contains("Move between cells"), "the key line: {f}");
        let f = frame(&mut s, 80, 24);
        assert!(f.contains("╭ FL FR "), "{f}");
    }

    #[test]
    fn the_input_page_draws_its_header_and_list_through_the_shell() {
        let state = fixture::state();
        let mut s = shell(
            Box::new(InputPage::new(0, shared(), &state)),
            crate::shell::Selection::Input(0),
        );
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = frame(&mut s, w, h);
            assert!(f.contains("Link 1/2"), "{w}x{h}: {f}");
            assert!(f.contains("Input trim"), "{w}x{h}: {f}");
            assert!(f.contains("Clear PEQ"), "{w}x{h}: {f}");
            assert!(f.contains("TYPE"), "{w}x{h}: {f}");
            assert!(
                f.contains("Filter Response · FL"),
                "the pane title names the channel, once: {f}"
            );
            assert!(!f.contains("● FL"), "{f}");
        }
        // The wider frame has room for the type names in full, and for the
        // whole key line.
        let f = frame(&mut s, 120, 40);
        assert!(f.contains("Low Shelf (12dB)"), "{f}");
        assert!(f.contains("105 Hz"), "{f}");
        assert!(f.contains("Enable All"), "the key line: {f}");
    }

    #[test]
    fn the_output_page_draws_routing_gain_delay_mute_and_the_tabs() {
        let state = fixture::state();
        let mut s = shell(
            Box::new(OutputPage::new(0, shared(), &state)),
            crate::shell::Selection::Output(0),
        );
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = frame(&mut s, w, h);
            assert!(f.contains("GAIN"), "{w}x{h}: {f}");
            assert!(f.contains("DELAY"), "{w}x{h}: {f}");
            assert!(f.contains("MUTE"), "{w}x{h}: {f}");
            assert!(f.contains("PEQ") && f.contains("XO"), "{w}x{h}: {f}");
            assert!(f.contains("INV"), "{w}x{h}: {f}");
        }
    }

    /// The shell hands the key to the focused screen before it looks at it
    /// itself, so a screen's own letters have to survive the trip, and the
    /// tool keys a band list gave up (PLAN-beta4 decision 2) have to pass
    /// through it to the shell.
    #[test]
    fn a_screens_keys_reach_it_and_the_band_list_lets_s_d_and_a_through() {
        use crate::shell::{Selection, ShellEvent, Tool};
        let state = fixture::state();
        let pages: [(Box<dyn crate::shell::Screen>, Selection); 2] = [
            (
                Box::new(InputPage::new(0, shared(), &state)),
                Selection::Input(0),
            ),
            (
                Box::new(OutputPage::new(0, shared(), &state)),
                Selection::Output(0),
            ),
        ];
        for (page, selection) in pages {
            let mut s = shell(page, selection);
            // Down into the band list, where `A` and `D` used to be bound.
            s.handle(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &state);
            let events = s.handle(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                &state,
            );
            assert!(events.is_empty(), "`a` is the list's Enable All");
            for (c, tool) in [
                ('S', Tool::Subharm),
                ('D', Tool::Tube),
                ('A', Tool::Spectrum),
            ] {
                let events = s.handle(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT), &state);
                assert_eq!(events, vec![ShellEvent::OpenTool(tool)], "{c}");
                assert!(s.dialog.is_none(), "{c} opened a dialog");
            }
        }
    }

    #[test]
    fn firmware_and_wire_gates_follow_the_console() {
        let s = fixture::state();
        assert_eq!(firmware_triple("1.1.6"), (1, 1, 6));
        assert_eq!(firmware_triple(""), (0, 0, 0));
        assert!(supports_bypass(&s));
        assert!(supports_crossover(&s));
        assert!(supports_linkwitz(&s));
        let types = available_types(&s, false);
        assert!(!types.contains(&FilterType::LinkwitzTransform));
        assert!(
            types.contains(&FilterType::LowPass1),
            "V28 first-order pass"
        );
        assert!(available_types(&s, true).contains(&FilterType::LinkwitzTransform));

        let mut old = fixture::state();
        old.caps.firmware = "1.1.3".into();
        old.caps.wire_format = 10;
        assert!(!supports_bypass(&old));
        assert!(!supports_crossover(&old));
        let types = available_types(&old, true);
        assert!(!types.contains(&FilterType::Notch));
        assert!(!types.contains(&FilterType::LowPass1));
        assert!(!types.contains(&FilterType::LinkwitzTransform));
    }

    #[test]
    fn names_codes_and_tokens_match_the_consoles_tables() {
        assert_eq!(type_name(FilterType::LowShelf), "Low Shelf (12dB)");
        assert_eq!(type_name(FilterType::from_raw(34)), "LR4 Low Pass");
        assert_eq!(type_code(FilterType::Notch), "NO");
        assert_eq!(type_code(FilterType::from_raw(34)), "LR4LP");
        assert_eq!(type_token(FilterType::Peaking), "peak");
        assert_eq!(type_token(FilterType::from_raw(35)), "lr4hp");
        assert_eq!(type_token(FilterType::from_raw(56)), "bes2lp");
    }

    /// PEQ passes read as cuts in the interface; crossovers keep LP and HP,
    /// and a filter file keeps REW's pass codes (DSPMath.swift:168-179,
    /// 285-297).
    #[test]
    fn peq_passes_read_as_cuts_and_crossovers_and_files_keep_passes() {
        let codes: Vec<String> = [
            FilterType::LowPass,
            FilterType::HighPass,
            FilterType::LowPass1,
            FilterType::HighPass1,
        ]
        .into_iter()
        .map(type_code)
        .collect();
        assert_eq!(codes, ["HC", "LC", "HC1", "LC1"]);
        assert_eq!(type_name(FilterType::LowPass), "High Cut (12dB)");
        assert_eq!(type_name(FilterType::HighPass1), "Low Cut (6dB)");
        // An LR4 low pass and high pass.
        assert_eq!(type_code(FilterType::from_raw(34)), "LR4LP");
        assert_eq!(type_code(FilterType::from_raw(35)), "LR4HP");
        assert_eq!(type_name(FilterType::from_raw(35)), "LR4 High Pass");
        use dspi_session::filterfile::{Bank, format_band};
        let band = |t: FilterType| dspi_proto::dsp::Band {
            filter_type: t,
            freq: 80.0,
            q: 0.707,
            gain_db: 0.0,
            bypass: false,
        };
        let line = format_band(Bank::Peq, 1, &band(FilterType::LowPass), None);
        assert!(line.contains(" LP "), "{line}");
        let line = format_band(Bank::Peq, 1, &band(FilterType::HighPass1), None);
        assert!(line.contains(" HP1 "), "{line}");
    }

    #[test]
    fn trimmed_keeps_one_decimal_and_drops_the_rest() {
        assert_eq!(trimmed(-8.6, 2, true), "-8.6");
        assert_eq!(trimmed(8.0, 2, true), "+8.0");
        assert_eq!(trimmed(3.58, 3, false), "3.58");
        assert_eq!(trimmed(0.707, 3, false), "0.707");
        assert_eq!(trimmed(1.0, 3, false), "1.0");
    }

    #[test]
    fn channel_tokens_come_from_the_device() {
        let s = fixture::state();
        assert_eq!(channel_token(&s, 0), "in.1");
        assert_eq!(channel_token(&s, 8), "out.1");
        assert_eq!(output_channel(&s, 8), 16);
        assert_eq!(channel_name(&s, 0), "FL");
        assert_eq!(channel_name(&s, 16), "Sub");
    }

    #[test]
    fn a_linked_pair_names_its_partner_only_while_it_is_live() {
        let mut st = SharedState::default();
        assert_eq!(st.linked_partner(0, 8), None);
        st.set_linked(0, true);
        assert_eq!(st.linked_partner(0, 8), Some(1));
        assert_eq!(st.linked_partner(1, 8), Some(0));
        assert_eq!(st.linked_partner(1, 1), None, "partner is not live");
    }

    // -- BUG1 --
    /// The K340's own configuration (tests/fixtures/preset/k340-bug1.dspipreset)
    /// on the demo device, applied as the Terminal's `import` does.
    fn k340() -> (DeviceState, dspi_session::Session) {
        use dspi_session::preset_file::{ApplyOptions, apply, parse};
        use dspi_transport::Transport;
        let mut t: Box<dyn Transport> = Box::new(dspi_transport::FakeDevice::demo());
        let caps = dspi_session::probe(t.as_mut()).unwrap();
        let mut session = dspi_session::Session::new(t, caps.clone()).unwrap();
        let doc = parse(include_str!(
            "../../../../tests/fixtures/preset/k340-bug1.dspipreset"
        ))
        .unwrap();
        let options = ApplyOptions {
            hardware_io: true,
            ..ApplyOptions::default()
        };
        let _ = apply(&mut session, &doc, options);
        let bulk = session.snapshot().unwrap();
        (DeviceState::new(caps, bulk), session)
    }

    #[test]
    fn bug1_k340_outputs_show_their_labels_and_slots() {
        let (s, mut session) = k340();
        let ni = s.caps.num_inputs as usize;
        let shown: Vec<String> = (0..4).map(|o| channel_label(&s, ni + o)).collect();
        assert_eq!(
            shown,
            [
                "Out 1 · R · Low",
                "Out 2 · R · High",
                "Out 3 · L · Low",
                "Out 4 · L · High"
            ]
        );
        let slots: Vec<String> = (0..4).map(|o| slot_name(&s, o)).collect();
        assert_eq!(slots, ["I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"]);
        // The stored name stays what a rename and a filter file read
        assert_eq!(channel_name(&s, ni), "I2S 1 L");
        // An input keeps its name
        assert_eq!(channel_label(&s, 0), channel_name(&s, 0));

        // A user's name wins, read again once the image changed
        session
            .write(
                "ch.name",
                &[ni as u8],
                dspi_proto::value::Value::Text("Right woofer".into()),
            )
            .unwrap();
        let s = DeviceState::new(s.caps.clone(), session.snapshot().unwrap());
        assert_eq!(channel_label(&s, ni), "Right woofer");
        assert_eq!(channel_label(&s, ni + 1), "Out 2 · R · High");
    }

    #[test]
    fn bug1_the_fixture_state_is_labelled_too() {
        let s = fixture::state();
        let ni = s.caps.num_inputs as usize;
        for o in 0..s.caps.num_outputs as usize {
            let l = channel_label(&s, ni + o);
            let stored = channel_name(&s, ni + o);
            // -- WP INTD -- (W12); -- WP REVEFIX -- an unused output is "N · None"
            assert!(
                l == stored
                    || l == format!("{} · None", o + 1)
                    || l.starts_with(&format!("Out {} · ", o + 1)),
                "{o}: {l} / {stored}"
            );
        }
    }

    /// The Terminal's main screen on the K340: the sidebar rows carry the
    /// labels. Prints the frame (`--nocapture`) for the BUG1 report.
    #[test]
    fn bug1_k340_sidebar_frame() {
        let (s, mut session) = k340();
        // -- WP REVEFIX -- (REVE C5) the first unused output low-passed: still "None"
        let ni = usize::from(s.caps.num_inputs);
        let spare = (0..usize::from(s.caps.num_outputs))
            .find(|&o| channel_label(&s, ni + o) == format!("{} · None", o + 1))
            .expect("an unused output");
        session
            .write_band(&dspi_proto::value::EqParamPacket {
                channel: (ni + spare) as u8,
                band: 0,
                filter_type: dspi_proto::enums::FilterType::LowPass,
                bypass: false,
                freq: 300.0,
                q: 0.707,
                gain_db: 0.0,
                qp: None,
            })
            .unwrap();
        let s = DeviceState::new(s.caps.clone(), session.snapshot().unwrap());
        let theme = crate::Theme::console(
            crate::theme::ColorDepth::TrueColor,
            crate::theme::Glyphs::Braille,
        );
        let mut live = crate::live::Live::new(
            s,
            theme,
            crate::perf::Performance::lite(),
            Box::new(crate::live::ConsoleScreens::new()),
        );
        live.tick(&mut session, None);
        let frame = crate::render_frame(110, 30, |a, b| live.draw(a, b));
        println!("{frame}");
        for l in ["R·Lo", "R·Hi", "L·Lo", "L·Hi", "Out 1 · R · Low"] {
            assert!(frame.contains(l), "{l} not in\n{frame}");
        }
        assert_eq!(channel_short_label(&live.state, 8), "R·Lo");
        // -- WP REVEFIX -- (REVE C5) the unused outputs read the core's "None" in
        // the sidebar and "N · None" elsewhere, never a band word after "None"
        assert!(!frame.contains("None·"), "{frame}");
        assert_eq!(channel_short_label(&live.state, ni + spare), "None");
        let unused: Vec<usize> = (0..usize::from(live.state.caps.num_outputs))
            .filter(|&o| channel_label(&live.state, ni + o) == format!("{} · None", o + 1))
            .collect();
        assert!(!unused.is_empty());
        for o in unused {
            assert_eq!(channel_short_label(&live.state, ni + o), "None");
        }
    }
}
